use crate::app_log;
use crate::error::AppResult;
use crate::runtime::{ProxyStatus, Runtime};
use crate::storage::{default_store_path, AppStore};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

pub struct AppState {
    pub app_data_dir: PathBuf,
    /// Tauri resource dir (bundled assets); used to scan `resources/rules/`.
    pub resource_dir: Option<PathBuf>,
    pub store_path: PathBuf,
    pub store: Mutex<AppStore>,
    pub runtime: Mutex<Runtime>,
    /// Main WebView is visible (affects journal sampling rate).
    pub ui_visible: AtomicBool,
    /// Only true when user explicitly quits (tray Quit / close without tray).
    /// Destroying the last WebView would otherwise kill tray + sing-box.
    pub exit_allowed: AtomicBool,
    /// One-click subscribe deep links waiting for the add-subscription UI.
    /// Cleared when the user closes the modal (not sticky across intentional dismiss).
    pending_import_urls: Mutex<Option<Vec<String>>>,
}

/// Recover from a poisoned mutex so one panic cannot brick the whole app.
fn recover_lock<'a, T>(mutex: &'a Mutex<T>, name: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            app_log::error(
                "lock",
                format!(
                    "{name} lock was poisoned — recovering (previous panic left the mutex tainted)"
                ),
            );
            poisoned.into_inner()
        }
    }
}

impl AppState {
    pub fn load(app_data_dir: PathBuf, resource_dir: Option<PathBuf>) -> AppResult<Self> {
        let store_path = default_store_path(&app_data_dir);
        let store = AppStore::load(&store_path, resource_dir.as_deref())?;
        Ok(Self {
            app_data_dir,
            resource_dir,
            store_path,
            store: Mutex::new(store),
            runtime: Mutex::new(Runtime::new()),
            ui_visible: AtomicBool::new(true),
            exit_allowed: AtomicBool::new(false),
            pending_import_urls: Mutex::new(None),
        })
    }

    /// Queue deep-link URLs for the frontend add-subscription form.
    pub fn set_pending_import_urls(&self, urls: Vec<String>) {
        *recover_lock(&self.pending_import_urls, "pending_import") = Some(urls);
    }

    /// Read pending import URLs without clearing (UI may remount before user closes).
    pub fn peek_pending_import_urls(&self) -> Option<Vec<String>> {
        recover_lock(&self.pending_import_urls, "pending_import").clone()
    }

    /// Drop pending import after user closes / finishes the add form.
    pub fn clear_pending_import_urls(&self) {
        *recover_lock(&self.pending_import_urls, "pending_import") = None;
    }

    /// Lock order rule: **never** hold `store` while acquiring `runtime`.
    /// Prefer `runtime` then `store` when both are needed.
    pub fn lock_runtime(&self) -> MutexGuard<'_, Runtime> {
        recover_lock(&self.runtime, "runtime")
    }

    pub fn lock_store(&self) -> MutexGuard<'_, AppStore> {
        recover_lock(&self.store, "store")
    }

    pub fn set_ui_visible(&self, visible: bool) {
        self.ui_visible.store(visible, Ordering::Relaxed);
    }

    pub fn is_ui_visible(&self) -> bool {
        self.ui_visible.load(Ordering::Relaxed)
    }

    pub fn allow_exit(&self) {
        self.exit_allowed.store(true, Ordering::SeqCst);
    }

    pub fn is_exit_allowed(&self) -> bool {
        self.exit_allowed.load(Ordering::SeqCst)
    }

    pub fn unload_ui_on_tray(&self) -> bool {
        self.with_store(|s| Ok(s.settings.unload_ui_on_tray))
            .unwrap_or(false)
    }

    pub fn with_store_mut<F, T>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&mut AppStore) -> AppResult<T>,
    {
        let mut guard = self.lock_store();
        let result = f(&mut guard)?;
        guard.save(&self.store_path)?;
        Ok(result)
    }

    pub fn with_store<F, T>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&AppStore) -> AppResult<T>,
    {
        let guard = self.lock_store();
        f(&guard)
    }

    pub fn start_proxy(
        &self,
        resource_dir: Option<&Path>,
        enable_system_proxy: bool,
    ) -> AppResult<ProxyStatus> {
        let mut runtime = self.lock_runtime();
        let mut store = self.lock_store();
        let stored_capture = store.settings.capture_mode;
        let enable_system_proxy = match stored_capture {
            crate::domain::CaptureMode::System => true,
            crate::domain::CaptureMode::Tun => false,
            crate::domain::CaptureMode::Off => enable_system_proxy,
        };
        // Preserve compatibility with callers that explicitly request system
        // proxy on first start, while never overriding a saved TUN preference.
        if enable_system_proxy && stored_capture == crate::domain::CaptureMode::Off {
            store.settings.capture_mode = crate::domain::CaptureMode::System;
            store.settings.tun_enabled = false;
        }
        let mut status = runtime.start_proxy(
            &self.app_data_dir,
            resource_dir,
            &mut store,
            enable_system_proxy,
        )?;
        if runtime.system_proxy_on != enable_system_proxy {
            status = runtime.set_system_proxy(&store, enable_system_proxy)?;
        }
        store.save(&self.store_path)?;
        Ok(status)
    }

    pub fn stop_proxy(&self) -> AppResult<ProxyStatus> {
        let mut runtime = self.lock_runtime();
        let store = self.lock_store();
        runtime.stop_proxy(&store)
    }

    pub fn restart_proxy(&self, resource_dir: Option<&Path>) -> AppResult<ProxyStatus> {
        let mut runtime = self.lock_runtime();
        let mut store = self.lock_store();
        let want_system = store.settings.capture_mode == crate::domain::CaptureMode::System;
        let mut status = runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
        if runtime.system_proxy_on != want_system {
            status = runtime.set_system_proxy(&store, want_system)?;
        }
        store.save(&self.store_path)?;
        Ok(status)
    }

    /// If core is running, regenerate config and restart so settings take effect.
    pub fn restart_if_running(
        &self,
        resource_dir: Option<&Path>,
    ) -> AppResult<Option<crate::runtime::ProxyStatus>> {
        if !self.is_core_running() {
            return Ok(None);
        }
        Ok(Some(self.restart_proxy(resource_dir)?))
    }

    pub fn proxy_status(&self) -> AppResult<ProxyStatus> {
        // Kernel urltest: mirror selected tag → current_node_id so UI / nodes stay accurate.
        self.sync_kernel_selection();
        let mut runtime = self.lock_runtime();
        let store = self.lock_store();
        Ok(runtime.status(&store))
    }

    /// When auto_select=kernel, read Clash API group `now` and persist as current_node_id.
    fn sync_kernel_selection(&self) {
        use crate::config::outbound_tag;
        use crate::domain::AutoSelectMode;

        let mode = match self.with_store(|s| Ok(s.settings.auto_select)) {
            Ok(m) => m,
            Err(_) => return,
        };
        if mode != AutoSelectMode::Kernel {
            return;
        }

        let now_tag = {
            let mut runtime = self.lock_runtime();
            runtime.core.poll();
            if !runtime.core.is_running() {
                return;
            }
            let Some(api) = runtime.api.as_ref() else {
                return;
            };
            match api.proxy_group_now("proxy") {
                Ok(t) => t,
                Err(_) => return,
            }
        };
        let Some(tag) = now_tag else {
            return;
        };

        let node_id = match self.with_store(|store| {
            Ok(store
                .nodes
                .iter()
                .find(|n| outbound_tag(&n.node) == tag)
                .map(|n| n.node.id.clone()))
        }) {
            Ok(id) => id,
            Err(_) => return,
        };
        let Some(node_id) = node_id else {
            return;
        };

        let changed = self
            .with_store(|s| Ok(s.settings.current_node_id.as_deref() != Some(node_id.as_str())))
            .unwrap_or(false);
        if !changed {
            return;
        }

        if let Err(e) = self.with_store_mut(|store| {
            store.settings.current_node_id = Some(node_id.clone());
            Ok(())
        }) {
            app_log::warn(
                "auto_select",
                format!("persist kernel selection failed: {e}"),
            );
            return;
        }
        app_log::info(
            "auto_select",
            format!("kernel urltest now → node {node_id} ({tag})"),
        );
    }

    pub fn shutdown_runtime(&self) {
        let mut runtime = self.lock_runtime();
        runtime.shutdown();
    }

    pub fn is_core_running(&self) -> bool {
        let mut r = self.lock_runtime();
        r.core.poll();
        r.core.is_running()
    }

    pub fn set_system_proxy(
        &self,
        enabled: bool,
        resource_dir: Option<&Path>,
    ) -> AppResult<ProxyStatus> {
        self.set_capture_mode(if enabled { "system" } else { "off" }, resource_dir)
    }

    /// Toggle TUN mode. When core is running, regenerate config and restart.
    pub fn set_tun_enabled(
        &self,
        enabled: bool,
        resource_dir: Option<&Path>,
    ) -> AppResult<ProxyStatus> {
        self.set_capture_mode(if enabled { "tun" } else { "off" }, resource_dir)
    }

    /// Traffic capture mode (mutually exclusive): `off` | `system` | `tun`.
    ///
    /// - off: system proxy off, TUN off  
    /// - system: TUN off, system proxy on  
    /// - tun: system proxy off, TUN on  
    pub fn set_capture_mode(
        &self,
        mode: &str,
        resource_dir: Option<&Path>,
    ) -> AppResult<ProxyStatus> {
        let mode = crate::domain::CaptureMode::parse(mode).ok_or_else(|| {
            crate::error::AppError::Core("capture mode must be off | system | tun".into())
        })?;
        let mut runtime = self.lock_runtime();
        let mut store = self.lock_store();
        runtime.core.poll();

        let want_tun = mode == crate::domain::CaptureMode::Tun;
        let want_sys = mode == crate::domain::CaptureMode::System;
        let tun_now = store.settings.tun_enabled;
        let sys_now = runtime.system_proxy_on;

        if tun_now == want_tun && sys_now == want_sys && store.settings.capture_mode == mode {
            return Ok(runtime.status(&store));
        }

        store.settings.capture_mode = mode;

        // 1) TUN setting / restart first (heavier).
        if tun_now != want_tun {
            store.settings.tun_enabled = want_tun;
            store.save(&self.store_path)?;
            if runtime.core.is_running() {
                runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
                store.save(&self.store_path)?;
            }
        }

        // 2) System proxy: always align with mode (TUN implies proxy off).
        if runtime.system_proxy_on != want_sys {
            runtime.set_system_proxy(&store, want_sys)?;
        }

        store.save(&self.store_path)?;

        Ok(runtime.status(&store))
    }

    /// Clash-style rule / global / direct. Restarts core when running.
    pub fn set_outbound_mode(
        &self,
        mode: crate::domain::OutboundMode,
        resource_dir: Option<&Path>,
    ) -> AppResult<ProxyStatus> {
        let mut runtime = self.lock_runtime();
        let mut store = self.lock_store();

        if store.settings.outbound_mode == mode {
            return Ok(runtime.status(&store));
        }
        store.settings.outbound_mode = mode;
        store.save(&self.store_path)?;

        runtime.core.poll();
        if runtime.core.is_running() {
            let status = runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
            store.save(&self.store_path)?;
            Ok(status)
        } else {
            Ok(runtime.status(&store))
        }
    }
}

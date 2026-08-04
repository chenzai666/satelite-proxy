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
}

/// Recover from a poisoned mutex so one panic cannot brick the whole app.
fn recover_lock<'a, T>(mutex: &'a Mutex<T>, name: &str) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            app_log::error(
                "lock",
                format!("{name} lock was poisoned — recovering (previous panic left the mutex tainted)"),
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
        })
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
            .unwrap_or(true)
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
        let status =
            runtime.start_proxy(&self.app_data_dir, resource_dir, &mut store, enable_system_proxy)?;
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
        let status = runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
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
        let mut runtime = self.lock_runtime();
        let store = self.lock_store();
        Ok(runtime.status(&store))
    }

    pub fn shutdown_runtime(&self) {
        let ports: Vec<u16> = {
            let s = self.lock_store();
            vec![s.settings.mixed_port, s.settings.api_port]
        };
        let mut runtime = self.lock_runtime();
        runtime.shutdown_with_ports(&ports);
    }

    pub fn is_core_running(&self) -> bool {
        let mut r = self.lock_runtime();
        r.core.poll();
        r.core.is_running()
    }

    pub fn set_system_proxy(&self, enabled: bool) -> AppResult<ProxyStatus> {
        let mut runtime = self.lock_runtime();
        let store = self.lock_store();
        runtime.set_system_proxy(&store, enabled)
    }

    /// Toggle TUN mode. When core is running, regenerate config and restart.
    pub fn set_tun_enabled(
        &self,
        enabled: bool,
        resource_dir: Option<&Path>,
    ) -> AppResult<ProxyStatus> {
        let mut runtime = self.lock_runtime();
        let mut store = self.lock_store();

        if store.settings.tun_enabled == enabled {
            return Ok(runtime.status(&store));
        }
        store.settings.tun_enabled = enabled;
        store.save(&self.store_path)?;

        runtime.core.poll();
        if runtime.core.is_running() {
            // Apply new inbound set via full restart
            let status = runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
            store.save(&self.store_path)?;
            Ok(status)
        } else {
            Ok(runtime.status(&store))
        }
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

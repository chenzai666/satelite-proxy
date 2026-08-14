use crate::app_log;
use crate::error::AppResult;
use crate::runtime::{ProxyStatus, Runtime};
use crate::storage::{default_store_path, AppStore};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

const KERNEL_SELECTION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const KERNEL_SELECTION_HTTP_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Default)]
struct KernelSelectionPoll {
    in_flight: bool,
    last_started: Option<Instant>,
}

impl KernelSelectionPoll {
    fn try_start(&mut self, now: Instant) -> bool {
        if self.in_flight
            || self.last_started.is_some_and(|last| {
                now.saturating_duration_since(last) < KERNEL_SELECTION_POLL_INTERVAL
            })
        {
            return false;
        }
        self.in_flight = true;
        self.last_started = Some(now);
        true
    }

    fn finish(&mut self) {
        self.in_flight = false;
    }
}

#[cfg(test)]
mod kernel_selection_poll_tests {
    use super::*;

    #[test]
    fn suppresses_concurrent_and_recent_status_polls() {
        let mut poll = KernelSelectionPoll::default();
        let start = Instant::now();
        assert!(poll.try_start(start));
        assert!(!poll.try_start(start + Duration::from_secs(10)));

        poll.finish();
        assert!(!poll.try_start(start + Duration::from_millis(500)));
        assert!(poll.try_start(start + KERNEL_SELECTION_POLL_INTERVAL));
    }
}

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
    /// True while the managed core is being started, stopped, or replaced.
    /// Background samplers must not contend for Runtime during this window.
    core_transitioning: AtomicBool,
    /// One-click subscribe deep links waiting for the add-subscription UI.
    /// Cleared when the user closes the modal (not sticky across intentional dismiss).
    pending_import_urls: Mutex<Option<Vec<String>>>,
    /// One global debounced apply queue for toggles and remote-rule updates.
    rule_apply_queue: Mutex<crate::rule_apply::RuleApplyQueue>,
    kernel_selection_poll: Mutex<KernelSelectionPoll>,
}

struct CoreTransitionGuard<'a> {
    flag: &'a AtomicBool,
}

impl Drop for CoreTransitionGuard<'_> {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
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
            core_transitioning: AtomicBool::new(false),
            pending_import_urls: Mutex::new(None),
            rule_apply_queue: Mutex::new(crate::rule_apply::RuleApplyQueue::default()),
            kernel_selection_poll: Mutex::new(KernelSelectionPoll::default()),
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

    /// Short-lived bookkeeping lock; unrelated to the runtime/store lock order.
    pub(crate) fn lock_rule_apply_queue(
        &self,
    ) -> MutexGuard<'_, crate::rule_apply::RuleApplyQueue> {
        recover_lock(&self.rule_apply_queue, "rule_apply_queue")
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

    pub fn is_core_transitioning(&self) -> bool {
        self.core_transitioning.load(Ordering::Acquire)
    }

    fn begin_core_transition(&self) -> AppResult<CoreTransitionGuard<'_>> {
        self.core_transitioning
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| crate::error::AppError::Core("内核正在切换，请稍候".into()))?;
        Ok(CoreTransitionGuard {
            flag: &self.core_transitioning,
        })
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
        let _transition = self.begin_core_transition()?;
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
        let _transition = self.begin_core_transition()?;
        let mut runtime = self.lock_runtime();
        let store = self.lock_store();
        runtime.stop_proxy(&store)
    }

    pub fn restart_proxy(&self, resource_dir: Option<&Path>) -> AppResult<ProxyStatus> {
        let _transition = self.begin_core_transition()?;
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
        let mut runtime = self.lock_runtime();
        let store = self.lock_store();
        Ok(runtime.status(&store))
    }

    /// When auto_select=kernel, read Clash API group `now` and persist as current_node_id.
    pub fn schedule_kernel_selection_sync(app: tauri::AppHandle) {
        use tauri::Manager;

        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let kernel_mode = state
            .with_store(|store| {
                Ok(store.settings.auto_select == crate::domain::AutoSelectMode::Kernel)
            })
            .unwrap_or(false);
        if !kernel_mode
            || state.is_core_transitioning()
            || !recover_lock(&state.kernel_selection_poll, "kernel_selection_poll")
                .try_start(Instant::now())
        {
            return;
        }

        tauri::async_runtime::spawn_blocking(move || {
            if let Some(state) = app.try_state::<AppState>() {
                state.sync_kernel_selection_outside_runtime_lock();
                recover_lock(&state.kernel_selection_poll, "kernel_selection_poll").finish();
            }
        });
    }

    /// Mirror the kernel urltest selection without holding Runtime during HTTP.
    fn sync_kernel_selection_outside_runtime_lock(&self) {
        use crate::config::outbound_tag;
        use crate::domain::AutoSelectMode;

        let mode = match self.with_store(|s| Ok(s.settings.auto_select)) {
            Ok(m) => m,
            Err(_) => return,
        };
        if mode != AutoSelectMode::Kernel {
            return;
        }

        let api = {
            let mut runtime = self.lock_runtime();
            runtime.core.poll();
            if !runtime.core.is_running() {
                return;
            }
            runtime.api_clone()
        };
        let Some(api) = api else { return };
        let now_tag = match api.proxy_group_now_with_timeout("proxy", KERNEL_SELECTION_HTTP_TIMEOUT)
        {
            Ok(tag) => tag,
            Err(_) => return,
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
        let _transition = self.begin_core_transition()?;
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
        let _transition = self.begin_core_transition()?;
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

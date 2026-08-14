use crate::app_log;
use crate::core::manager::CoreState;
use crate::error::AppResult;
use crate::runtime::{ConnectionView, ProxyStatus, Runtime};
use crate::storage::{default_store_path, AppStore};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

const KERNEL_SELECTION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const KERNEL_SELECTION_HTTP_TIMEOUT: Duration = Duration::from_millis(800);

#[derive(Default)]
struct KernelSelectionPoll {
    in_flight: bool,
    last_started: Option<Instant>,
}

#[derive(Default)]
struct QueryViewCache {
    query: String,
    limit: Option<usize>,
    rows: Vec<ConnectionView>,
}

#[derive(Default)]
struct TrafficViewCache {
    live: Vec<ConnectionView>,
    requests: QueryViewCache,
    failures: QueryViewCache,
}

fn apply_selected_node(
    settings: &mut crate::domain::AppSettings,
    node_id: String,
    manual: bool,
) -> bool {
    let was_kernel = settings.auto_select.is_kernel();
    settings.current_node_id = Some(node_id);
    if manual {
        settings.auto_select = crate::domain::AutoSelectMode::Off;
        settings.smart_switch = false;
    }
    was_kernel
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::sync::Arc;

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

    #[test]
    fn manual_node_selection_disables_every_auto_select_mode() {
        for mode in [
            crate::domain::AutoSelectMode::Smart,
            crate::domain::AutoSelectMode::Kernel,
        ] {
            let mut settings = crate::domain::AppSettings {
                auto_select: mode,
                smart_switch: true,
                ..crate::domain::AppSettings::default()
            };
            let was_kernel = apply_selected_node(&mut settings, "manual-node".into(), true);
            assert_eq!(settings.auto_select, crate::domain::AutoSelectMode::Off);
            assert!(!settings.smart_switch);
            assert_eq!(settings.current_node_id.as_deref(), Some("manual-node"));
            assert_eq!(was_kernel, mode.is_kernel());
        }
    }

    #[test]
    fn status_uses_cache_instead_of_waiting_for_runtime_during_transition() {
        let test_dir = std::env::temp_dir().join(format!(
            "satelite-status-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let state = Arc::new(AppState::load(test_dir.clone(), None).expect("load test state"));
        state.core_transitioning.store(true, Ordering::Release);
        state.mark_cached_core_state(CoreState::Starting);

        let runtime = state.lock_runtime();
        let (tx, rx) = mpsc::channel();
        let query_state = Arc::clone(&state);
        let query = std::thread::spawn(move || {
            tx.send(query_state.proxy_status()).expect("send status");
        });

        let result = rx.recv_timeout(Duration::from_millis(200));
        drop(runtime);
        query.join().expect("status query thread");
        let status = result
            .expect("status query must not wait for the runtime lock")
            .expect("status query");
        assert_eq!(status.core_state, CoreState::Starting);

        let _ = std::fs::remove_dir_all(test_dir);
    }

    #[test]
    fn traffic_views_use_cache_instead_of_waiting_during_transition() {
        let test_dir = std::env::temp_dir().join(format!(
            "satelite-traffic-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let state = Arc::new(AppState::load(test_dir.clone(), None).expect("load test state"));
        state.core_transitioning.store(true, Ordering::Release);

        let runtime = state.lock_runtime();
        let (tx, rx) = mpsc::channel();
        let query_state = Arc::clone(&state);
        let query = std::thread::spawn(move || {
            let live = query_state.live_connection_views();
            let requests = query_state.request_views(None, Some(800), false);
            let failures = query_state.request_views(None, Some(800), true);
            tx.send((live, requests, failures))
                .expect("send traffic views");
        });

        let result = rx.recv_timeout(Duration::from_millis(200));
        drop(runtime);
        query.join().expect("traffic query thread");
        let (live, requests, failures) =
            result.expect("traffic queries must not wait for the runtime lock");
        assert!(live.is_empty());
        assert!(requests.is_empty());
        assert!(failures.is_empty());

        let _ = std::fs::remove_dir_all(test_dir);
    }

    #[test]
    fn journal_never_waits_for_runtime_and_rejects_stale_sessions() {
        let test_dir = std::env::temp_dir().join(format!(
            "satelite-journal-session-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let state = Arc::new(AppState::load(test_dir.clone(), None).expect("load test state"));
        let current = crate::api::ClashApi::new("127.0.0.1", 19090, "current");
        state.lock_runtime().api = Some(current.clone());

        let runtime = state.lock_runtime();
        let (tx, rx) = mpsc::channel();
        let journal_state = Arc::clone(&state);
        let query = std::thread::spawn(move || {
            tx.send(journal_state.try_clash_api_clone())
                .expect("send journal API");
        });
        let busy_result = rx.recv_timeout(Duration::from_millis(200));
        drop(runtime);
        query.join().expect("journal query thread");
        assert!(busy_result.expect("journal query must not wait").is_none());

        let stale = crate::api::ClashApi::new("127.0.0.1", 19090, "stale");
        let snapshot = |upload_total| crate::api::ConnectionsSnapshot {
            upload_total,
            download_total: 0,
            connections: Vec::new(),
        };
        assert!(!state.try_apply_connection_snapshot(&stale, snapshot(3)));
        assert!(state.try_apply_connection_snapshot(&current, snapshot(7)));
        assert_eq!(state.proxy_status().expect("status").upload_total, 7);

        let _ = std::fs::remove_dir_all(test_dir);
    }

    #[test]
    fn live_selection_does_not_hold_runtime_lock_during_http() {
        let test_dir = std::env::temp_dir().join(format!(
            "satelite-live-select-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let state = Arc::new(AppState::load(test_dir.clone(), None).expect("load test state"));
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake clash api");
        let port = listener.local_addr().expect("fake api address").port();
        state.lock_runtime().api = Some(crate::api::ClashApi::new("127.0.0.1", port, "test"));

        let (seen_tx, seen_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept selector request");
            let mut request = [0u8; 4096];
            socket.read(&mut request).expect("read selector request");
            seen_tx.send(()).expect("signal request received");
            release_rx.recv().expect("release fake response");
            socket
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .expect("write selector response");
        });

        let operation_state = Arc::clone(&state);
        let operation = std::thread::spawn(move || {
            operation_state.select_group_live_serialized("proxy", "node-a", false)
        });
        seen_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("selector request must reach fake api");
        let runtime_was_available = state.runtime.try_lock().is_ok();
        release_tx.send(()).expect("release selector response");
        server.join().expect("fake api server");
        assert!(operation
            .join()
            .expect("selector thread")
            .expect("selection"));
        assert!(
            runtime_was_available,
            "runtime lock must be released before selector HTTP waits"
        );

        let _ = std::fs::remove_dir_all(test_dir);
    }

    #[test]
    fn core_running_check_never_waits_for_runtime_lock() {
        let test_dir = std::env::temp_dir().join(format!(
            "satelite-running-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let state = Arc::new(AppState::load(test_dir.clone(), None).expect("load test state"));
        {
            let mut cached = recover_lock(&state.status_cache, "status_cache");
            cached.running = true;
            cached.core_state = CoreState::Running;
        }

        let runtime = state.lock_runtime();
        let (tx, rx) = mpsc::channel();
        let query_state = Arc::clone(&state);
        let cached_query = std::thread::spawn(move || {
            tx.send(query_state.is_core_running())
                .expect("send cached running state");
        });
        let cached_running = rx.recv_timeout(Duration::from_millis(200));
        drop(runtime);
        cached_query.join().expect("cached running query");

        state.core_transitioning.store(true, Ordering::Release);
        let runtime = state.lock_runtime();
        let (tx, rx) = mpsc::channel();
        let query_state = Arc::clone(&state);
        let transition_query = std::thread::spawn(move || {
            tx.send(query_state.is_core_running())
                .expect("send transition running state");
        });
        let transition_running = rx.recv_timeout(Duration::from_millis(200));
        drop(runtime);
        transition_query.join().expect("transition running query");

        assert!(cached_running.expect("lock contention must use cached state"));
        assert!(!transition_running.expect("transition check must not wait"));
        let _ = std::fs::remove_dir_all(test_dir);
    }
}

pub struct AppState {
    pub app_data_dir: PathBuf,
    /// Tauri resource dir (bundled assets); used to scan `resources/rules/`.
    pub resource_dir: Option<PathBuf>,
    pub store_path: PathBuf,
    pub store: Mutex<AppStore>,
    pub runtime: Mutex<Runtime>,
    /// Last complete status snapshot. Status IPC reads this while a long core
    /// transition owns `runtime`, so the WebView never queues behind startup.
    status_cache: Mutex<ProxyStatus>,
    /// Last rendered traffic rows. Traffic pages keep showing these while a
    /// core transition owns `runtime` instead of queuing another IPC request.
    traffic_view_cache: Mutex<TrafficViewCache>,
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
        let mut runtime = Runtime::new();
        let status_cache = runtime.status(&store);
        Ok(Self {
            app_data_dir,
            resource_dir,
            store_path,
            store: Mutex::new(store),
            runtime: Mutex::new(runtime),
            status_cache: Mutex::new(status_cache),
            traffic_view_cache: Mutex::new(TrafficViewCache::default()),
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

    /// The connection journal is best-effort and high-frequency. It must never
    /// queue behind a core transition; another snapshot will arrive shortly.
    pub fn try_clash_api_clone(&self) -> Option<crate::api::ClashApi> {
        if self.is_core_transitioning() {
            return None;
        }
        match self.runtime.try_lock() {
            Ok(runtime) => runtime.clash_api_clone(),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Poisoned(poisoned)) => {
                app_log::error("lock", "runtime lock was poisoned — recovering");
                poisoned.into_inner().clash_api_clone()
            }
        }
    }

    /// Apply only a snapshot from the currently active core session. If the
    /// runtime is busy, dropping one frame is safer than delaying a restart or
    /// applying stale data after it completes.
    pub fn try_apply_connection_snapshot(
        &self,
        api: &crate::api::ClashApi,
        snapshot: crate::api::ConnectionsSnapshot,
    ) -> bool {
        if self.is_core_transitioning() || !api.is_active() {
            return false;
        }
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => return false,
            Err(TryLockError::Poisoned(poisoned)) => {
                app_log::error("lock", "runtime lock was poisoned — recovering");
                poisoned.into_inner()
            }
        };
        if self.is_core_transitioning()
            || !api.is_active()
            || !runtime
                .clash_api_clone()
                .is_some_and(|current| current.same_session(api))
        {
            return false;
        }
        runtime.apply_snapshot(snapshot);
        true
    }

    fn begin_core_transition(&self) -> AppResult<CoreTransitionGuard<'_>> {
        self.core_transitioning
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| crate::error::AppError::Core("内核正在切换，请稍候".into()))?;
        Ok(CoreTransitionGuard {
            flag: &self.core_transitioning,
        })
    }

    fn cache_status(&self, status: &ProxyStatus) {
        *recover_lock(&self.status_cache, "status_cache") = status.clone();
    }

    fn cached_status(&self) -> ProxyStatus {
        recover_lock(&self.status_cache, "status_cache").clone()
    }

    fn mark_cached_core_state(&self, core_state: CoreState) {
        let mut status = recover_lock(&self.status_cache, "status_cache");
        status.core_state = core_state;
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
        self.mark_cached_core_state(CoreState::Starting);
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
        self.cache_status(&status);
        Ok(status)
    }

    pub fn stop_proxy(&self) -> AppResult<ProxyStatus> {
        let _transition = self.begin_core_transition()?;
        self.mark_cached_core_state(CoreState::Stopping);
        let mut runtime = self.lock_runtime();
        let store = self.lock_store();
        let status = runtime.stop_proxy(&store)?;
        self.cache_status(&status);
        Ok(status)
    }

    pub fn restart_proxy(&self, resource_dir: Option<&Path>) -> AppResult<ProxyStatus> {
        let _transition = self.begin_core_transition()?;
        self.mark_cached_core_state(CoreState::Starting);
        let mut runtime = self.lock_runtime();
        let mut store = self.lock_store();
        let want_system = store.settings.capture_mode == crate::domain::CaptureMode::System;
        let mut status = runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
        if runtime.system_proxy_on != want_system {
            status = runtime.set_system_proxy(&store, want_system)?;
        }
        store.save(&self.store_path)?;
        self.cache_status(&status);
        Ok(status)
    }

    /// If core is running, regenerate config and restart so settings take effect.
    pub fn restart_if_running(
        &self,
        resource_dir: Option<&Path>,
    ) -> AppResult<Option<crate::runtime::ProxyStatus>> {
        if self.is_core_transitioning() {
            return Err(crate::error::AppError::Core("内核正在切换，请稍候".into()));
        }
        if !self.is_core_running() {
            return Ok(None);
        }
        Ok(Some(self.restart_proxy(resource_dir)?))
    }

    pub fn proxy_status(&self) -> AppResult<ProxyStatus> {
        if self.is_core_transitioning() {
            return Ok(self.cached_status());
        }

        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => return Ok(self.cached_status()),
            Err(TryLockError::Poisoned(poisoned)) => {
                app_log::error("lock", "runtime lock was poisoned — recovering");
                poisoned.into_inner()
            }
        };
        let store = match self.store.try_lock() {
            Ok(store) => store,
            Err(TryLockError::WouldBlock) => return Ok(self.cached_status()),
            Err(TryLockError::Poisoned(poisoned)) => {
                app_log::error("lock", "store lock was poisoned — recovering");
                poisoned.into_inner()
            }
        };
        let status = runtime.status(&store);
        self.cache_status(&status);
        Ok(status)
    }

    pub fn live_connection_views(&self) -> Vec<ConnectionView> {
        if self.is_core_transitioning() {
            return recover_lock(&self.traffic_view_cache, "traffic_view_cache")
                .live
                .clone();
        }
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => {
                return recover_lock(&self.traffic_view_cache, "traffic_view_cache")
                    .live
                    .clone()
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let store = match self.store.try_lock() {
            Ok(store) => store,
            Err(TryLockError::WouldBlock) => {
                return recover_lock(&self.traffic_view_cache, "traffic_view_cache")
                    .live
                    .clone()
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let rows = runtime.live_connections(&store);
        recover_lock(&self.traffic_view_cache, "traffic_view_cache").live = rows.clone();
        rows
    }

    pub fn request_views(
        &self,
        query: Option<&str>,
        limit: Option<usize>,
        failures_only: bool,
    ) -> Vec<ConnectionView> {
        let query = query.unwrap_or("").trim().to_string();
        let cached = || {
            let cache = recover_lock(&self.traffic_view_cache, "traffic_view_cache");
            let entry = if failures_only {
                &cache.failures
            } else {
                &cache.requests
            };
            if entry.query == query && entry.limit == limit {
                entry.rows.clone()
            } else {
                Vec::new()
            }
        };
        if self.is_core_transitioning() {
            return cached();
        }
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => return cached(),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let store = match self.store.try_lock() {
            Ok(store) => store,
            Err(TryLockError::WouldBlock) => return cached(),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        let rows = if failures_only {
            runtime.request_failures(&store, Some(&query), limit)
        } else {
            runtime.request_history(&store, Some(&query), limit)
        };
        let mut cache = recover_lock(&self.traffic_view_cache, "traffic_view_cache");
        let entry = if failures_only {
            &mut cache.failures
        } else {
            &mut cache.requests
        };
        entry.query = query;
        entry.limit = limit;
        entry.rows = rows.clone();
        rows
    }

    pub fn clear_request_history_nonblocking(&self) -> AppResult<()> {
        if self.is_core_transitioning() {
            return Err(crate::error::AppError::Core("内核正在切换，请稍候".into()));
        }
        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => {
                return Err(crate::error::AppError::Core("内核正忙，请稍候".into()))
            }
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        runtime.clear_request_history();
        let mut cache = recover_lock(&self.traffic_view_cache, "traffic_view_cache");
        cache.requests.rows.clear();
        cache.failures.rows.clear();
        Ok(())
    }

    /// Run a Clash selector update without holding `runtime` across HTTP I/O.
    /// The transition guard prevents a core restart from replacing the API
    /// endpoint between cloning the handle and applying the selection.
    pub fn select_group_live_serialized(
        &self,
        group: &str,
        node_tag: &str,
        close_connections: bool,
    ) -> AppResult<bool> {
        let _operation = self.begin_core_transition()?;
        let api = {
            let runtime = self.lock_runtime();
            runtime.clash_api_clone()
        };
        let Some(api) = api else {
            return Ok(false);
        };

        api.select_proxy(group, node_tag)?;
        if close_connections {
            let _ = api.close_all_connections();
        }
        Ok(true)
    }

    /// Select the main proxy node and persist it under the same operation
    /// guard, so a manual click and a smart-switch apply cannot overwrite one
    /// another mid-flight.
    pub fn select_current_node_serialized(
        &self,
        node_id: &str,
        manual: bool,
        close_if_enabled: bool,
    ) -> AppResult<(crate::domain::AppSettings, bool, bool)> {
        let _operation = self.begin_core_transition()?;
        let (tag, should_close) = self.with_store(|store| {
            if !manual && !store.settings.auto_select.is_smart() {
                return Err(crate::error::AppError::Core("智能切换已关闭".into()));
            }
            let node = store
                .find_node(node_id)
                .ok_or_else(|| crate::error::AppError::NotFound(node_id.to_string()))?;
            Ok((
                crate::config::outbound_tag(node),
                close_if_enabled && store.settings.close_connections_on_switch,
            ))
        })?;
        let api = {
            let runtime = self.lock_runtime();
            runtime.clash_api_clone()
        };
        let selected_live = if let Some(api) = api {
            api.select_proxy("proxy", &tag)?;
            if should_close {
                let _ = api.close_all_connections();
            }
            true
        } else {
            false
        };

        let node_id = node_id.to_string();
        let (settings, was_kernel) = self.with_store_mut(|store| {
            let was_kernel = apply_selected_node(&mut store.settings, node_id, manual);
            Ok((store.settings.clone(), was_kernel))
        })?;
        Ok((settings, was_kernel, selected_live))
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
        // Background schedulers must skip work while the endpoint is being
        // replaced; waiting here can occupy an async worker for 6–10 seconds.
        if self.is_core_transitioning() {
            return false;
        }

        let mut runtime = match self.runtime.try_lock() {
            Ok(runtime) => runtime,
            Err(TryLockError::WouldBlock) => return self.cached_status().running,
            Err(TryLockError::Poisoned(poisoned)) => {
                app_log::error("lock", "runtime lock was poisoned — recovering");
                poisoned.into_inner()
            }
        };
        runtime.core.poll();
        let running = runtime.core.is_running();
        let core_state = runtime.core.state();
        drop(runtime);

        let mut cached = recover_lock(&self.status_cache, "status_cache");
        cached.running = running;
        cached.core_state = core_state;
        running
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
            let status = runtime.status(&store);
            self.cache_status(&status);
            return Ok(status);
        }

        store.settings.capture_mode = mode;

        // 1) TUN setting / restart first (heavier).
        if tun_now != want_tun {
            store.settings.tun_enabled = want_tun;
            store.save(&self.store_path)?;
            if runtime.core.is_running() {
                self.mark_cached_core_state(CoreState::Starting);
                runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
                store.save(&self.store_path)?;
            }
        }

        // 2) System proxy: always align with mode (TUN implies proxy off).
        if runtime.system_proxy_on != want_sys {
            runtime.set_system_proxy(&store, want_sys)?;
        }

        store.save(&self.store_path)?;

        let status = runtime.status(&store);
        self.cache_status(&status);
        Ok(status)
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
            let status = runtime.status(&store);
            self.cache_status(&status);
            return Ok(status);
        }
        store.settings.outbound_mode = mode;
        store.save(&self.store_path)?;

        runtime.core.poll();
        if runtime.core.is_running() {
            self.mark_cached_core_state(CoreState::Starting);
            let status = runtime.restart_core(&self.app_data_dir, resource_dir, &mut store)?;
            store.save(&self.store_path)?;
            self.cache_status(&status);
            Ok(status)
        } else {
            let status = runtime.status(&store);
            self.cache_status(&status);
            Ok(status)
        }
    }
}

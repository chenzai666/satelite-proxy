//! Network diagnostics surfaced to the UI. Detection only — never mutates
//! system network settings. See `core::macos_net` for the rationale.

use crate::services::exit_ip::{probe, ExitIpInfo};
use crate::state::AppState;
use tauri::State;

#[derive(Debug, Clone, serde::Serialize)]
pub struct NetworkDiagnosticsResult {
    /// Empty when nothing was detected (or the platform isn't supported).
    pub issues: Vec<DiagnosticIssue>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticIssue {
    pub id: String,
    pub issue: String,
    pub suggestion: String,
}

/// Run best-effort network diagnostics (currently: TUN-bypassing system DNS,
/// macOS only). Never modifies system settings — UI-facing detection only.
#[tauri::command]
pub fn diagnose_network() -> NetworkDiagnosticsResult {
    #[cfg(target_os = "macos")]
    let issues = crate::core::macos_net::diagnose_system_dns_bypass()
        .into_iter()
        .map(|diag| DiagnosticIssue {
            id: "dns-bypasses-tun".into(),
            issue: diag.issue,
            suggestion: diag.suggestion,
        })
        .collect();

    #[cfg(not(target_os = "macos"))]
    let issues = Vec::new();

    NetworkDiagnosticsResult { issues }
}

/// 探测当前实际出口 IP。核心运行且非直连模式时走 Satelite mixed 入站，
/// 这样可以验证浏览器/CLIProxy 等应用看到的是否为代理出口。
#[tauri::command]
pub async fn check_exit_ip(state: State<'_, AppState>) -> Result<ExitIpInfo, String> {
    let status = state.proxy_status().map_err(|error| error.to_string())?;
    let via_proxy = status.running && status.outbound_mode != "direct";
    probe(status.mixed_port, via_proxy).await
}

/// Detect persistent pending public TCP connections that are owned by another
/// process instead of Satelite's core. This is a read-only hint for
/// applications that do not consume the Windows system proxy (for example
/// some updaters and games). It never changes the capture mode or any OS
/// network setting.
#[tauri::command(async)]
pub async fn detect_proxy_bypasses(
    state: State<'_, AppState>,
) -> Result<crate::proxy_bypass::ProxyBypassReport, String> {
    if state.is_core_transitioning() {
        return Ok(crate::proxy_bypass::empty_report(false));
    }

    let status = state.proxy_status().map_err(|error| error.to_string())?;
    let managed_pids = state.lock_runtime().managed_process_ids();
    // The runtime flag can be stale when another proxy manager, a script, or
    // a previous crash changed WinINet after Satelite last updated its state.
    // Only run the probe when the actual Windows proxy still points at this
    // instance's mixed port.
    #[cfg(windows)]
    let system_proxy_matches = crate::proxy::create_system_proxy()
        .detect_owned("127.0.0.1", status.mixed_port)
        .map(|snapshot| snapshot.is_some())
        .unwrap_or(false);
    #[cfg(not(windows))]
    let system_proxy_matches = true;
    tauri::async_runtime::spawn_blocking(move || {
        crate::proxy_bypass::detect(&status, &managed_pids, system_proxy_matches)
    })
    .await
    .map_err(|error| format!("proxy bypass detection task: {error}"))
}

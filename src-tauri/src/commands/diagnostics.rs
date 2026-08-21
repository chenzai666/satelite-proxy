//! Network diagnostics surfaced to the UI. Detection only — never mutates
//! system network settings. See `core::macos_net` for the rationale.

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
    let mut issues = Vec::new();

    #[cfg(target_os = "macos")]
    if let Some(diag) = crate::core::macos_net::diagnose_system_dns_bypass() {
        issues.push(DiagnosticIssue {
            id: "dns-bypasses-tun".into(),
            issue: diag.issue,
            suggestion: diag.suggestion,
        });
    }

    NetworkDiagnosticsResult { issues }
}

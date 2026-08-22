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

/// Aggregate memory of the app's WebView process tree (Windows only).
/// `None` on platforms without a supported reader — the UI hides the row.
#[tauri::command]
pub fn get_webview_memory() -> Option<crate::core::memory::WebViewTreeMemory> {
    crate::core::memory::read_webview_tree_memory()
}

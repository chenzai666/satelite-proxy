//! DNS settings commands (docs/dns.md).

use crate::domain::DnsSettings;
use crate::state::AppState;
use serde::Serialize;
use std::net::ToSocketAddrs;
use std::time::Instant;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn get_dns_settings(state: State<'_, AppState>) -> Result<DnsSettings, String> {
    state
        .with_store(|store| Ok(store.dns.clone()))
        .map_err(|e| e.to_string())
}

/// Replace full DNS settings. Optionally restart core when `apply` is true and running.
#[tauri::command]
pub fn update_dns_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: DnsSettings,
    apply: Option<bool>,
) -> Result<DnsSettings, String> {
    let apply = apply.unwrap_or(true);
    state
        .with_store_mut(|store| {
            store.dns = settings;
            Ok(store.dns.clone())
        })
        .map_err(|e| e.to_string())?;

    let dns = state
        .with_store(|s| Ok(s.dns.clone()))
        .map_err(|e| e.to_string())?;

    if apply && state.is_core_running() {
        let resource_dir = app.path().resource_dir().ok();
        // Restart so DNS is rewritten into active.json
        state
            .restart_if_running(resource_dir.as_deref())
            .map_err(|e| format!("DNS 已保存，但重启内核失败: {e}"))?;
    }

    Ok(dns)
}

#[derive(Debug, Serialize)]
pub struct DnsTestResult {
    pub domain: String,
    pub ok: bool,
    pub addrs: Vec<String>,
    pub elapsed_ms: u64,
    pub error: Option<String>,
    /// Hint only — OS resolve does not reveal which sing-box server answered.
    pub note: String,
}

/// Resolve a domain via the OS (diagnostics). When proxy+hijack is on, OS may still
/// use system DNS; this is a connectivity smoke test, not full FakeIP inspection.
#[tauri::command]
pub fn test_dns_lookup(domain: String) -> Result<DnsTestResult, String> {
    let domain = domain.trim().to_string();
    if domain.is_empty() {
        return Err("domain is empty".into());
    }
    // strip scheme/path if pasted as URL
    let host = domain
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(&domain)
        .split(':')
        .next()
        .unwrap_or(&domain)
        .to_string();

    let start = Instant::now();
    let query = format!("{host}:0");
    match query.to_socket_addrs() {
        Ok(iter) => {
            let mut addrs: Vec<String> = iter.map(|a| a.ip().to_string()).collect();
            addrs.sort();
            addrs.dedup();
            Ok(DnsTestResult {
                domain: host,
                ok: !addrs.is_empty(),
                addrs,
                elapsed_ms: start.elapsed().as_millis() as u64,
                error: None,
                note: "系统解析结果（非 sing-box 查询路径）".into(),
            })
        }
        Err(e) => Ok(DnsTestResult {
            domain: host,
            ok: false,
            addrs: vec![],
            elapsed_ms: start.elapsed().as_millis() as u64,
            error: Some(e.to_string()),
            note: "系统解析失败".into(),
        }),
    }
}

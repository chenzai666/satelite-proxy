//! DNS settings commands (docs/dns.md).

use crate::config::{dump_dns_rules_file, lookup_hosts};
use crate::domain::{read_system_hosts_entries, DnsSettings, HostsEntry};
use crate::state::AppState;
use serde::Serialize;
use std::net::ToSocketAddrs;
use std::time::Instant;
use tauri::{AppHandle, State};

/// Export the current DNS rules to `{app_data}/data/dns/user-dns-rules.list`.
fn dump_dns_rules(state: &AppState) {
    let rules = state
        .with_store(|s| Ok(s.dns.enabled_dns_rules()))
        .unwrap_or_default();
    if let Err(e) = dump_dns_rules_file(&state.app_data_dir, &rules) {
        eprintln!("[satelite] dump dns rules: {e}");
    }
}

#[tauri::command]
pub fn get_dns_settings(state: State<'_, AppState>) -> Result<DnsSettings, String> {
    state
        .with_store(|store| Ok(store.dns.clone()))
        .map_err(|e| e.to_string())
}

/// Read the OS hosts file into a read-only entry list (for the Hosts UI).
#[tauri::command]
pub fn read_system_hosts() -> Vec<HostsEntry> {
    read_system_hosts_entries()
}

/// Replace full DNS settings. Optionally restart core when `apply` is true and running.
#[tauri::command]
pub fn update_dns_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: DnsSettings,
    apply: Option<bool>,
) -> Result<DnsSettings, String> {
    let apply = apply.unwrap_or(true);
    settings.ensure_rule_sets();
    state
        .with_store_mut(|store| {
            store.dns = settings;
            Ok(store.dns.clone())
        })
        .map_err(|e| e.to_string())?;

    let dns = state
        .with_store(|s| Ok(s.dns.clone()))
        .map_err(|e| e.to_string())?;

    // Export user DNS rules to disk (mirror routing rule export).
    dump_dns_rules(&state);

    if apply {
        crate::rule_apply::request_restart(app, Vec::new());
    }

    Ok(dns)
}

/// Reset DNS rules to factory defaults (other fields unchanged).
/// Rules reset reloads `resources/dns/builtin-dns-rules.list`.
#[tauri::command]
pub fn reset_dns_defaults(
    app: AppHandle,
    state: State<'_, AppState>,
    section: String,
    apply: Option<bool>,
) -> Result<DnsSettings, String> {
    let apply = apply.unwrap_or(true);
    let section = section.trim().to_ascii_lowercase();

    let dns = state
        .with_store_mut(|store| {
            match section.as_str() {
                "rules" => {
                    store.dns.reset_builtin_dns_set();
                }
                other => {
                    return Err(crate::error::AppError::Config(format!(
                        "unknown DNS reset section: {other} (use rules)"
                    )));
                }
            }
            Ok(store.dns.clone())
        })
        .map_err(|e| e.to_string())?;

    dump_dns_rules(&state);

    if apply {
        crate::rule_apply::request_restart(app, Vec::new());
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

/// Resolve a domain for diagnostics. Enabled application Hosts entries are checked
/// first; unmatched names fall back to the OS resolver.
#[tauri::command]
pub fn test_dns_lookup(
    state: State<'_, AppState>,
    domain: String,
) -> Result<DnsTestResult, String> {
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
    let hosts = state
        .with_store(|store| Ok(store.dns.effective_hosts()))
        .map_err(|e| e.to_string())?;
    let host_addrs = lookup_hosts(&hosts, &host);
    if !host_addrs.is_empty() {
        return Ok(DnsTestResult {
            domain: host,
            ok: true,
            addrs: host_addrs,
            elapsed_ms: start.elapsed().as_millis() as u64,
            error: None,
            note: "应用 Hosts 命中（精确域名匹配）".into(),
        });
    }

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

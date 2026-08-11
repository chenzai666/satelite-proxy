//! Dump user DNS rules to an on-disk list under app data:
//! - `user-dns-rules.list`  Clash-style DNS rules (DOMAIN-SUFFIX,payload,ACTION)
//!
//! Mirrors [`crate::config::rule_files`]: `store.json` (`dns.rules`) is the source
//! of truth; this file is an export copy written on every save.

use crate::domain::{DnsAction, DnsRule, DomainMatcher};
use crate::error::{AppError, AppResult};
use std::fs;
use std::path::{Path, PathBuf};

/// Directory holding exported DNS rule lists: `{app_data}/data/dns`.
fn dns_export_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("data").join("dns")
}

/// Path to the user DNS rules export file.
fn user_dns_rules_path(app_data_dir: &Path) -> PathBuf {
    dns_export_dir(app_data_dir).join("user-dns-rules.list")
}

fn matcher_token(m: DomainMatcher) -> &'static str {
    match m {
        DomainMatcher::Domain => "DOMAIN",
        DomainMatcher::DomainSuffix => "DOMAIN-SUFFIX",
        DomainMatcher::DomainKeyword => "DOMAIN-KEYWORD",
    }
}

fn action_token(a: DnsAction) -> &'static str {
    match a {
        DnsAction::Local => "LOCAL",
        DnsAction::Domestic => "DOMESTIC",
        DnsAction::Remote => "REMOTE",
        DnsAction::Block => "BLOCK",
    }
}

/// Render DNS rules as a Clash-style list. Header documents the format.
pub fn format_dns_rules_list(rules: &[DnsRule]) -> String {
    let mut lines = Vec::new();
    lines.push("# name: user-dns-rules".into());
    lines.push("# User DNS rules (exported from DNS settings).".into());
    lines.push("# Format: MATCHER,payload,ACTION  (ACTION: LOCAL | DOMESTIC | REMOTE)".into());
    lines.push(String::new());
    for r in rules.iter().filter(|r| r.enabled) {
        let payload = r.payload.trim();
        if payload.is_empty() {
            continue;
        }
        lines.push(format!(
            "{},{},{}",
            matcher_token(r.matcher),
            payload,
            action_token(r.action)
        ));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Write the user DNS rules list. Removes the file when there are no enabled rules.
pub fn dump_dns_rules_file(app_data_dir: &Path, rules: &[DnsRule]) -> AppResult<()> {
    let dir = dns_export_dir(app_data_dir);
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::Storage(format!("create dns export dir {}: {e}", dir.display())))?;

    let path = user_dns_rules_path(app_data_dir);
    let has_enabled = rules
        .iter()
        .any(|r| r.enabled && !r.payload.trim().is_empty());
    if !has_enabled {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    let body = format_dns_rules_list(rules);
    fs::write(&path, body)
        .map_err(|e| AppError::Storage(format!("write {}: {e}", path.display())))?;
    Ok(())
}

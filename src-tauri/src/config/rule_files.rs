//! Dump user rule sets to dual on-disk lists under app data:
//! - `{set_id}.list`      Clash-style routing rules
//! - `{set_id}.dns.list`  SYSTEM DNS projections (optional)

use crate::domain::{format_clash_rules_list, format_dns_sidecar_list, RuleSet};
use crate::error::{AppError, AppResult};
use std::fs;
use std::path::{Path, PathBuf};

pub fn rules_export_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("data").join("rules")
}

fn safe_stem(set_id: &str) -> String {
    set_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn clash_list_path(app_data_dir: &Path, set_id: &str) -> PathBuf {
    rules_export_dir(app_data_dir).join(format!("{}.list", safe_stem(set_id)))
}

pub fn dns_sidecar_path(app_data_dir: &Path, set_id: &str) -> PathBuf {
    rules_export_dir(app_data_dir).join(format!("{}.dns.list", safe_stem(set_id)))
}

/// Write Clash + DNS sidecar for one set. Built-in sets: routing list only (no DNS file).
pub fn dump_rule_set_files(app_data_dir: &Path, set: &RuleSet) -> AppResult<()> {
    let dir = rules_export_dir(app_data_dir);
    fs::create_dir_all(&dir).map_err(|e| {
        AppError::Storage(format!("create rules export dir {}: {e}", dir.display()))
    })?;

    let clash_path = clash_list_path(app_data_dir, &set.id);
    let clash_body = format_clash_rules_list(&set.name, &set.rules);
    fs::write(&clash_path, clash_body).map_err(|e| {
        AppError::Storage(format!("write {}: {e}", clash_path.display()))
    })?;

    // Sidecar only when some rules request system DNS (user overrides, any set).
    let dns_path = dns_sidecar_path(app_data_dir, &set.id);
    let dns_body = format_dns_sidecar_list(&set.name, &set.rules);
    if dns_body.trim().is_empty() {
        let _ = fs::remove_file(&dns_path);
    } else {
        fs::write(&dns_path, dns_body).map_err(|e| {
            AppError::Storage(format!("write {}: {e}", dns_path.display()))
        })?;
    }
    Ok(())
}

/// Dump all rule sets (best-effort log on individual failure).
pub fn dump_all_rule_sets(app_data_dir: &Path, sets: &[RuleSet]) -> AppResult<()> {
    for set in sets {
        dump_rule_set_files(app_data_dir, set)?;
    }
    Ok(())
}

pub fn remove_rule_set_files(app_data_dir: &Path, set_id: &str) {
    let _ = fs::remove_file(clash_list_path(app_data_dir, set_id));
    let _ = fs::remove_file(dns_sidecar_path(app_data_dir, set_id));
}

//! Dump user rule sets to on-disk Clash-style lists under app data:
//! - `{set_id}.list`  Clash-style routing rules

use crate::domain::{format_clash_rules_list, RuleSet};
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

/// Write the Clash-style routing list for one set.
pub fn dump_rule_set_files(app_data_dir: &Path, set: &RuleSet) -> AppResult<()> {
    let dir = rules_export_dir(app_data_dir);
    fs::create_dir_all(&dir).map_err(|e| {
        AppError::Storage(format!("create rules export dir {}: {e}", dir.display()))
    })?;

    let clash_path = clash_list_path(app_data_dir, &set.id);
    let clash_body = format_clash_rules_list(&set.name, &set.rules);
    fs::write(&clash_path, clash_body)
        .map_err(|e| AppError::Storage(format!("write {}: {e}", clash_path.display())))?;
    Ok(())
}

pub fn remove_rule_set_files(app_data_dir: &Path, set_id: &str) {
    let _ = fs::remove_file(clash_list_path(app_data_dir, set_id));
}

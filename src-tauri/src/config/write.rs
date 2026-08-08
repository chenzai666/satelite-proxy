use crate::config::builder::BuiltConfig;
use crate::error::{AppError, AppResult};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn config_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("config")
}

pub fn active_config_path(app_data_dir: &Path) -> PathBuf {
    config_dir(app_data_dir).join("active.json")
}

/// Write active.json and a timestamped backup. Returns active path.
pub fn write_active_config(app_data_dir: &Path, built: &BuiltConfig) -> AppResult<PathBuf> {
    let dir = config_dir(app_data_dir);
    let backup_dir = dir.join("backup");
    fs::create_dir_all(&backup_dir)?;

    let raw = serde_json::to_string_pretty(&built.value)
        .map_err(|e| AppError::Config(format!("serialize config: {e}")))?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = backup_dir.join(format!("{ts}.json"));
    fs::write(&backup, &raw)?;

    let active = active_config_path(app_data_dir);
    let tmp = dir.join("active.json.tmp");
    fs::write(&tmp, &raw)?;
    fs::rename(&tmp, &active)?;

    // Keep at most 20 backups
    prune_backups(&backup_dir, 20)?;

    Ok(active)
}

fn prune_backups(dir: &Path, keep: usize) -> AppResult<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    for e in entries.into_iter().skip(keep) {
        let _ = fs::remove_file(e.path());
    }
    Ok(())
}

//! Shared hourly log naming and bounded retention.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const RETENTION_DAYS: u64 = 7;
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
pub const APP_ACTIVE_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const CORE_ACTIVE_MAX_BYTES: u64 = MAX_TOTAL_BYTES - APP_ACTIVE_MAX_BYTES;

pub fn current_hour() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() / 3600)
        .unwrap_or(0)
}

pub fn hourly_path(dir: &Path, prefix: &str) -> PathBuf {
    hourly_path_for(dir, prefix, current_hour())
}

pub fn hourly_path_for(dir: &Path, prefix: &str, hour: u64) -> PathBuf {
    dir.join(format!("{prefix}-{hour:010}.log"))
}

/// Remove `.log` files older than seven days, then oldest files until the
/// directory is at most 1 GiB. The actively written files are protected.
pub fn cleanup(dir: &Path, protected: &[&Path]) -> std::io::Result<()> {
    cleanup_with_limits(
        dir,
        protected,
        SystemTime::now(),
        Duration::from_secs(RETENTION_DAYS * 24 * 60 * 60),
        MAX_TOTAL_BYTES,
    )
}

pub fn cleanup_current_hour(dir: &Path) -> std::io::Result<()> {
    let hour = current_hour();
    let app = hourly_path_for(dir, "app", hour);
    let core = hourly_path_for(dir, "sing-box", hour);
    cleanup(dir, &[app.as_path(), core.as_path()])
}

fn cleanup_with_limits(
    dir: &Path,
    protected: &[&Path],
    now: SystemTime,
    max_age: Duration,
    max_total_bytes: u64,
) -> std::io::Result<()> {
    let mut files = Vec::new();

    for entry in match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    } {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("log") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        let is_protected = protected.iter().any(|active| path == *active);
        if !is_protected
            && now
                .duration_since(modified)
                .map(|age| age > max_age)
                .unwrap_or(false)
        {
            let _ = fs::remove_file(&path);
            continue;
        }
        files.push((path, metadata.len(), modified, is_protected));
    }

    let mut total: u64 = files.iter().map(|(_, len, _, _)| *len).sum();
    if total <= max_total_bytes {
        return Ok(());
    }
    files.sort_by_key(|(_, _, modified, _)| *modified);
    for (path, len, _, is_protected) in files {
        if total <= max_total_bytes {
            break;
        }
        if is_protected {
            continue;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn hourly_names_are_stable_within_an_hour() {
        let dir = Path::new("logs");
        assert_eq!(hourly_path(dir, "app"), hourly_path(dir, "app"));
        assert!(hourly_path(dir, "app")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("app-"));
    }

    #[test]
    fn cleanup_never_removes_active_file() {
        let dir = std::env::temp_dir().join(format!(
            "satelite-log-retention-{}-{}",
            std::process::id(),
            current_hour()
        ));
        fs::create_dir_all(&dir).unwrap();
        let active = dir.join("app-active.log");
        File::create(&active)
            .unwrap()
            .set_len(MAX_TOTAL_BYTES + 1)
            .unwrap();

        cleanup(&dir, &[active.as_path()]).unwrap();

        assert!(active.exists());
        let _ = fs::remove_file(active);
        let _ = fs::remove_dir(dir);
    }

    #[test]
    fn cleanup_removes_files_older_than_retention_window() {
        let dir = test_dir("age");
        fs::create_dir_all(&dir).unwrap();
        let old = dir.join("app-old.log");
        let fresh = dir.join("app-fresh.log");
        File::create(&old).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        File::create(&fresh).unwrap();
        let now = fs::metadata(&fresh).unwrap().modified().unwrap();
        cleanup_with_limits(&dir, &[], now, Duration::from_millis(10), u64::MAX).unwrap();
        assert!(!old.exists());
        assert!(fresh.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_evicts_oldest_files_until_under_size_limit() {
        let dir = test_dir("size");
        fs::create_dir_all(&dir).unwrap();
        let oldest = dir.join("app-1.log");
        let middle = dir.join("app-2.log");
        let newest = dir.join("app-3.log");
        File::create(&oldest).unwrap().set_len(8).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        File::create(&middle).unwrap().set_len(8).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        File::create(&newest).unwrap().set_len(8).unwrap();
        cleanup_with_limits(&dir, &[], SystemTime::now(), Duration::from_secs(3600), 16).unwrap();
        assert!(!oldest.exists());
        assert!(middle.exists());
        assert!(newest.exists());
        let _ = fs::remove_dir_all(dir);
    }

    fn test_dir(label: &str) -> PathBuf {
        dir_for_test(label, current_hour())
    }

    fn dir_for_test(label: &str, suffix: u64) -> PathBuf {
        std::env::temp_dir().join(format!(
            "satelite-log-retention-{label}-{}-{suffix}",
            std::process::id()
        ))
    }
}

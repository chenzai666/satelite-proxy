//! Shared install/reset coordination and bounded Windows sharing retries.

use super::CoreKind;
use crate::error::{AppError, AppResult};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

static INSTALL_LOCKS: [Mutex<()>; 3] = [Mutex::new(()), Mutex::new(()), Mutex::new(())];
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const SHARING_RETRY_BUDGET: Duration = Duration::from_secs(5);

// File transactions never acquire runtime/store locks. Resolution may already
// hold those locks and wait here; installation/reset must not wait back on them.
pub(super) fn lock_install(kind: CoreKind) -> AppResult<MutexGuard<'static, ()>> {
    try_lock_install(install_mutex(kind), kind)
}

fn install_mutex(kind: CoreKind) -> &'static Mutex<()> {
    let index = match kind {
        CoreKind::SingBox => 0,
        CoreKind::Xray => 1,
        CoreKind::Mihomo => 2,
    };
    &INSTALL_LOCKS[index]
}

fn try_lock_install(mutex: &Mutex<()>, kind: CoreKind) -> AppResult<MutexGuard<'_, ()>> {
    mutex.try_lock().map_err(|_| {
        AppError::Core(format!(
            "{} 正在安装或恢复，请稍后重试",
            kind.display_name()
        ))
    })
}

fn is_sharing_conflict(error: &io::Error) -> bool {
    cfg!(target_os = "windows") && matches!(error.raw_os_error(), Some(32 | 33))
}

pub(super) fn lock_resolution(kind: CoreKind) -> AppResult<MutexGuard<'static, ()>> {
    install_mutex(kind).lock().map_err(|_| {
        AppError::Core(format!(
            "{} 安装锁异常，请重启应用后重试",
            kind.display_name()
        ))
    })
}

fn retry_sharing<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let started = Instant::now();
    loop {
        match operation() {
            Err(error)
                if is_sharing_conflict(&error) && started.elapsed() < SHARING_RETRY_BUDGET =>
            {
                std::thread::sleep(
                    Duration::from_millis(100)
                        .min(SHARING_RETRY_BUDGET.saturating_sub(started.elapsed())),
                );
            }
            result => return result,
        }
    }
}

pub(super) fn rename_file(from: &Path, to: &Path) -> io::Result<()> {
    retry_sharing(|| fs::rename(from, to))
}

pub(super) fn remove_file(path: &Path) -> io::Result<()> {
    retry_sharing(|| match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    })
}

pub(super) fn cleanup_file(path: &Path) {
    if let Err(error) = remove_file(path) {
        crate::app_log::warn("core", format!("cleanup {}: {error}", path.display()));
    }
}

pub(super) fn preserve_previous(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    if !path.is_file() {
        return Err(AppError::Core(format!(
            "invalid previous core path: {}",
            path.display()
        )));
    }
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let extension = path.extension().unwrap_or_default().to_string_lossy();
    let recovery = path.with_extension(format!(
        "recovery-{}-{nonce}-{sequence}.{extension}",
        std::process::id()
    ));
    if recovery.exists() {
        return Err(AppError::Core(format!(
            "recovery path exists: {}",
            recovery.display()
        )));
    }
    rename_file(path, &recovery).map_err(|error| {
        AppError::Core(format!(
            "preserve previous core {}: {error}",
            path.display()
        ))
    })?;
    crate::app_log::warn(
        "core",
        format!(
            "previous core retained for recovery: {}",
            recovery.display()
        ),
    );
    Ok(())
}

// A failed metadata commit must leave the previous version text intact. Unique
// sibling files also avoid collisions with bundled-core staging/version writes.
pub(super) fn write_version_atomically(path: &Path, version: &str) -> io::Result<()> {
    write_atomically(path, |file| file.write_all(version.as_bytes()))
}

pub(super) fn write_atomically(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        write(&mut file)?;
        file.sync_all()
    })();
    drop(file);
    let result = result.and_then(|()| rename_file(&temporary, path));
    if result.is_err() {
        cleanup_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permanent_io_errors_are_not_retried() {
        let mut attempts = 0;
        let result: io::Result<()> = retry_sharing(|| {
            attempts += 1;
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
        });
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }

    #[test]
    fn install_and_reset_reject_same_kind_but_other_kinds_are_independent() {
        let locks = [Mutex::new(()), Mutex::new(()), Mutex::new(())];
        let kinds = [CoreKind::SingBox, CoreKind::Xray, CoreKind::Mihomo];
        for (index, kind) in kinds.iter().copied().enumerate() {
            let guard = try_lock_install(&locks[index], kind).unwrap();
            assert!(try_lock_install(&locks[index], kind).is_err());
            for (other, other_kind) in kinds.iter().copied().enumerate() {
                assert_eq!(
                    std::ptr::eq(install_mutex(kind), install_mutex(other_kind)),
                    index == other
                );
                if other != index {
                    assert!(try_lock_install(&locks[other], other_kind).is_ok());
                }
            }
            drop(guard);
            assert!(try_lock_install(&locks[index], kind).is_ok());
        }
    }

    #[test]
    fn only_windows_sharing_and_lock_errors_are_retryable() {
        for code in [32, 33] {
            assert_eq!(
                is_sharing_conflict(&io::Error::from_raw_os_error(code)),
                cfg!(windows)
            );
        }
        for code in [2, 5, 80] {
            assert!(!is_sharing_conflict(&io::Error::from_raw_os_error(code)));
        }
    }
}

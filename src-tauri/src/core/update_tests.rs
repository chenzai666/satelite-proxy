use super::*;
use crate::core::paths::version_file_path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const KINDS: [CoreKind; 3] = [CoreKind::SingBox, CoreKind::Xray, CoreKind::Mihomo];
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    dest: PathBuf,
    staged: PathBuf,
    previous: PathBuf,
    version: PathBuf,
    dependency: PathBuf,
}

impl Fixture {
    fn new(kind: CoreKind) -> Self {
        let root = std::env::temp_dir().join(format!(
            "satelite-update-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(core_dir(&root)).unwrap();
        let dest = core_bin_path(&root, kind);
        Self {
            staged: staged_core_path(kind, &dest),
            previous: previous_core_path(kind, &dest),
            version: version_file_path(&root, kind),
            dependency: core_dir(&root).join("dependency.dat"),
            root,
            dest,
        }
    }

    fn seed(&self) {
        fs::write(&self.dest, b"old executable").unwrap();
        fs::write(&self.staged, b"new executable").unwrap();
    }

    fn assert_no_temporary_files(&self) {
        let unexpected: Vec<_> = fs::read_dir(core_dir(&self.root))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path != &self.dest
                    && path != &self.staged
                    && path != &self.previous
                    && path != &self.version
                    && path != &self.dependency
            })
            .collect();
        assert!(unexpected.is_empty(), "unexpected files: {unexpected:?}");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for path in [
            &self.dest,
            &self.staged,
            &self.previous,
            &self.version,
            &self.dependency,
        ] {
            if let Err(error) = fs::remove_file(path) {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!("test cleanup {}: {error}", path.display());
                }
            }
        }
        for dir in [core_dir(&self.root), self.root.clone()] {
            if let Err(error) = fs::remove_dir(&dir) {
                eprintln!("test cleanup {}: {error}", dir.display());
            }
        }
    }
}

#[test]
fn replacement_and_failed_activation_preserve_the_old_core_for_all_kinds() {
    for kind in KINDS {
        let f = Fixture::new(kind);
        f.seed();
        assert!(replace_installed_core(kind, &f.staged, &f.dest, &f.previous).unwrap());
        assert_eq!(fs::read(&f.dest).unwrap(), b"new executable");
        assert_eq!(fs::read(&f.previous).unwrap(), b"old executable");
        assert!(!f.staged.exists());

        fs::remove_file(&f.previous).unwrap();
        fs::write(&f.dest, b"old executable").unwrap();
        let error = replace_installed_core(kind, &f.staged, &f.dest, &f.previous).unwrap_err();
        assert!(error.to_string().contains("activate downloaded core"));
        assert_eq!(fs::read(&f.dest).unwrap(), b"old executable");
        assert!(!f.previous.exists());
    }
}

#[test]
fn only_previous_copy_is_never_discarded() {
    for kind in KINDS {
        let f = Fixture::new(kind);
        fs::write(&f.previous, b"only old executable").unwrap();
        fs::write(&f.staged, b"new executable").unwrap();
        let error = replace_installed_core(kind, &f.staged, &f.dest, &f.previous).unwrap_err();
        let message = error.to_string();
        assert!(message.contains(&f.previous.display().to_string()));
        assert!(message.contains(&f.dest.display().to_string()));
        assert_eq!(fs::read(&f.previous).unwrap(), b"only old executable");
        assert_eq!(fs::read(&f.staged).unwrap(), b"new executable");
        assert!(!f.dest.exists());
    }
}

#[test]
fn rollback_failure_reports_both_recovery_paths() {
    for kind in KINDS {
        let f = Fixture::new(kind);
        fs::write(&f.dest, b"new executable").unwrap();
        let error = rollback_installed_core(
            &f.dest,
            &f.previous,
            true,
            AppError::Core("metadata failed".into()),
        );
        let message = error.to_string();
        assert!(message.contains("metadata failed"));
        assert!(message.contains("rollback failed"));
        assert!(message.contains(&f.dest.display().to_string()));
        assert!(message.contains(&f.previous.display().to_string()));
        assert!(!f.dest.exists());
    }
}

#[test]
fn later_update_preserves_an_uncommitted_previous_backup() {
    for kind in KINDS {
        let f = Fixture::new(kind);
        fs::write(&f.dest, b"uncommitted executable").unwrap();
        fs::write(&f.previous, b"old committed executable").unwrap();
        fs::write(&f.staged, b"latest executable").unwrap();
        assert!(replace_installed_core(kind, &f.staged, &f.dest, &f.previous).unwrap());
        assert_eq!(fs::read(&f.dest).unwrap(), b"latest executable");
        assert_eq!(fs::read(&f.previous).unwrap(), b"uncommitted executable");
        let recoveries: Vec<_> = fs::read_dir(core_dir(&f.root))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".recovery-")
            })
            .collect();
        assert_eq!(recoveries.len(), 1);
        assert_eq!(
            fs::read(&recoveries[0]).unwrap(),
            b"old committed executable"
        );
        fs::remove_file(&recoveries[0]).unwrap();
        f.assert_no_temporary_files();
    }
}

#[test]
fn interrupted_atomic_dependency_write_keeps_old_content_and_removes_temporary() {
    for kind in KINDS {
        let f = Fixture::new(kind);
        fs::write(&f.dependency, b"old dependency").unwrap();
        let error = update::write_atomically(&f.dependency, |file| {
            file.write_all(b"partial new dependency")?;
            Err(io::Error::new(
                io::ErrorKind::Other,
                "injected write failure",
            ))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected write failure"));
        assert_eq!(fs::read(&f.dependency).unwrap(), b"old dependency");
        f.assert_no_temporary_files();
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{Duration, Instant};

    fn lock(path: &Path) -> File {
        fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(path)
            .unwrap()
    }

    #[test]
    fn short_file_locks_are_retried_for_all_kinds() {
        for kind in KINDS {
            for lock_staged in [false, true] {
                let f = Fixture::new(kind);
                f.seed();
                let handle = lock(if lock_staged { &f.staged } else { &f.dest });
                let releaser = std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(250));
                    drop(handle);
                });
                let result = replace_installed_core(kind, &f.staged, &f.dest, &f.previous);
                releaser.join().unwrap();
                assert!(result.unwrap());
                assert_eq!(fs::read(&f.dest).unwrap(), b"new executable");
                assert_eq!(fs::read(&f.previous).unwrap(), b"old executable");
            }
        }
    }

    #[test]
    fn persistent_file_locks_fail_within_budget_and_preserve_old_core() {
        for kind in KINDS {
            for lock_staged in [false, true] {
                let f = Fixture::new(kind);
                f.seed();
                let handle = lock(if lock_staged { &f.staged } else { &f.dest });
                let start = Instant::now();
                let result = replace_installed_core(kind, &f.staged, &f.dest, &f.previous);
                let elapsed = start.elapsed();
                drop(handle);
                assert!(result.is_err());
                assert!(
                    elapsed >= Duration::from_secs(4),
                    "retry ended early: {elapsed:?}"
                );
                assert!(
                    elapsed < Duration::from_secs(15),
                    "retry exceeded budget: {elapsed:?}"
                );
                assert_eq!(fs::read(&f.dest).unwrap(), b"old executable");
                assert_eq!(fs::read(&f.staged).unwrap(), b"new executable");
                assert!(!f.previous.exists());
            }
        }
    }

    #[test]
    fn rollback_locked_new_core_preserves_previous_and_allows_later_recovery() {
        for kind in KINDS {
            let f = Fixture::new(kind);
            f.seed();
            let had_previous =
                replace_installed_core(kind, &f.staged, &f.dest, &f.previous).unwrap();
            let handle = lock(&f.dest);
            let error = rollback_installed_core(
                &f.dest,
                &f.previous,
                had_previous,
                AppError::Core("version commit failed".into()),
            );
            let message = error.to_string();
            assert!(message.contains("version commit failed"));
            assert!(message.contains("rollback failed"));
            assert!(message.contains(&f.previous.display().to_string()));
            assert_eq!(fs::read(&f.previous).unwrap(), b"old executable");
            assert_eq!(fs::read(&f.dest).unwrap(), b"new executable");
            drop(handle);
            let recovered = rollback_installed_core(
                &f.dest,
                &f.previous,
                true,
                AppError::Core("original error".into()),
            );
            assert!(!recovered.to_string().contains("rollback failed"));
            assert_eq!(fs::read(&f.dest).unwrap(), b"old executable");
            assert!(!f.previous.exists());
        }
    }

    #[test]
    fn failed_version_commit_preserves_metadata_and_rolls_back_core() {
        for kind in KINDS {
            let f = Fixture::new(kind);
            f.seed();
            fs::write(&f.version, b"v-old").unwrap();
            let handle = lock(&f.version);
            let had_previous =
                replace_installed_core(kind, &f.staged, &f.dest, &f.previous).unwrap();
            let error = write_version_file(&f.root, kind, "1.0.46").unwrap_err();
            let error = rollback_installed_core(&f.dest, &f.previous, had_previous, error);
            drop(handle);
            assert!(!error.to_string().is_empty());
            assert_eq!(fs::read(&f.dest).unwrap(), b"old executable");
            assert_eq!(fs::read(&f.version).unwrap(), b"v-old");
            assert!(!f.previous.exists());
            f.assert_no_temporary_files();
        }
    }
}

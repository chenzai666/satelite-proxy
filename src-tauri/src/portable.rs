//! Portable (便携版) mode.
//!
//! When the marker file `portable.flag` sits next to the running exe, the exe
//! directory becomes the data root: `data/store.json`, `config/`, `bin/`
//! (staged cores), `logs/`, `mihomo/`, `remote-rule-sets/` and the WebView2
//! profile (`webview/`) all live next to the exe instead of the OS app-data
//! dir — extract-and-run, nothing is written to `%APPDATA%`.
//!
//! Rules (AGENTS.md §9):
//! - Never call `app.path().app_data_dir()` directly; resolve through
//!   [`resolve_app_data_dir`] so portable mode stays in effect everywhere.
//! - The WebView2 user-data folder must be redirected on BOTH window creation
//!   paths: config windows are auto-disabled via [`patch_context`] and rebuilt
//!   here in setup, and `window_ctrl::show_main` recreates with
//!   [`webview_data_dir`]. Tauri otherwise forces `%LOCALAPPDATA%/<id>`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager, Runtime};

/// Marker file that enables portable mode when present next to the exe.
pub const MARKER_FILE: &str = "portable.flag";

static PORTABLE_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Pure marker check: `Some(exe_dir)` when `exe_dir/portable.flag` exists.
fn detect_in(exe_dir: &Path) -> Option<PathBuf> {
    if exe_dir.join(MARKER_FILE).is_file() {
        Some(exe_dir.to_path_buf())
    } else {
        None
    }
}

fn detect() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .and_then(|dir| detect_in(&dir))
}

/// Portable root (the exe directory) when the marker is present.
pub fn root() -> Option<&'static Path> {
    PORTABLE_ROOT.get_or_init(detect).as_deref()
}

pub fn is_portable() -> bool {
    root().is_some()
}

/// Portable data root: the exe directory itself (no extra nesting — keeps the
/// `<app_data>/data/…` sub-layout, `config/`, `bin/`, `logs/`, `mihomo/` and
/// the config-relative mihomo home derivation in `core/kind.rs` intact).
pub fn app_data_dir() -> Option<PathBuf> {
    root().map(Path::to_path_buf)
}

/// App data dir with portable override; drop-in replacement for
/// `app.path().app_data_dir()`.
pub fn resolve_app_data_dir<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<PathBuf> {
    match app_data_dir() {
        Some(dir) => Ok(dir),
        None => app.path().app_data_dir(),
    }
}

fn webview_data_dir_in(root: &Path) -> PathBuf {
    root.join("webview")
}

/// WebView2 user-data folder (`<exe_dir>/webview`) in portable mode.
pub fn webview_data_dir() -> Option<PathBuf> {
    root().map(webview_data_dir_in)
}

/// Portable: stop Tauri from auto-creating config windows. They are rebuilt in
/// setup instead so the WebView2 data dir can be redirected — config windows
/// are created *before* the setup hook runs, and config `dataDirectory`
/// values are always anchored under `%LOCALAPPDATA%`, hence this dance.
pub fn patch_context<R: Runtime>(context: &mut tauri::Context<R>) {
    if is_portable() {
        for window in &mut context.config_mut().app.windows {
            window.create = false;
        }
    }
}

/// Build the config-declared main window with the portable WebView2 data
/// directory. Runs first thing in `setup` so the rest of startup (silent-start
/// hide, tray, deep links) sees the same window as the installed build.
/// A failure here only logs: tray reopen rebuilds via `window_ctrl::show_main`.
pub fn build_main_window<R: Runtime>(app: &tauri::App<R>) {
    let Some(data_dir) = webview_data_dir() else {
        return;
    };
    for window in &app.config().app.windows {
        if window.create {
            // Not disabled by `patch_context` (non-portable path) — already
            // auto-created by Tauri before setup; never reached in portable.
            continue;
        }
        match tauri::WebviewWindowBuilder::from_config(app.handle(), window) {
            Ok(builder) => {
                if let Err(error) = builder.data_directory(data_dir.clone()).build() {
                    crate::app_log::error("portable", format!("build main window failed: {error}"));
                }
            }
            Err(error) => {
                crate::app_log::error("portable", format!("main window config unusable: {error}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "satelite-portable-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ))
    }

    #[test]
    fn detects_marker_next_to_exe_dir() {
        let root = temp_root("marker");
        std::fs::create_dir_all(&root).expect("create temp dir");
        assert_eq!(detect_in(&root), None, "no marker -> not portable");

        std::fs::write(root.join(MARKER_FILE), b"").expect("write marker");
        assert_eq!(detect_in(&root), Some(root.clone()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn marker_must_be_a_file_not_a_directory() {
        let root = temp_root("dir-marker");
        std::fs::create_dir_all(root.join(MARKER_FILE)).expect("create marker dir");
        assert_eq!(detect_in(&root), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn webview_data_dir_sits_next_to_exe() {
        let root = Path::new("X:/Satelite-Portable");
        assert_eq!(webview_data_dir_in(root), root.join("webview"));
    }
}

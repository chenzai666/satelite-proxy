//! Main window show / hide / destroy for tray memory management.
//!
//! Destroying the last WebView triggers Tauri `ExitRequested`. Callers must
//! keep `AppState::exit_allowed == false` so the run loop calls `prevent_exit`
//! and tray + sing-box stay alive.

use crate::state::AppState;
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Matches frontend `windowLayout.ts` (logical px).
const PRO_SIZE: (f64, f64) = (1024.0, 760.0);
const SIMPLE_SIZE: (f64, f64) = (420.0, 760.0);

fn ui_mode_file(app_data_dir: &std::path::Path) -> PathBuf {
    app_data_dir.join("data").join("ui_mode")
}

/// Persist UI mode so the next WebView recreate uses the correct window size.
pub fn write_ui_mode(app_data_dir: &std::path::Path, mode: &str) {
    let path = ui_mode_file(app_data_dir);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let v = match mode.trim().to_ascii_lowercase().as_str() {
        "simple" => "simple",
        _ => "pro",
    };
    let _ = fs::write(path, v);
}

pub fn read_ui_mode(app_data_dir: &std::path::Path) -> &'static str {
    let path = ui_mode_file(app_data_dir);
    match fs::read_to_string(path) {
        Ok(s) if s.trim().eq_ignore_ascii_case("simple") => "simple",
        _ => "pro",
    }
}

fn size_for_ui_mode(mode: &str) -> (f64, f64) {
    if mode == "simple" {
        SIMPLE_SIZE
    } else {
        PRO_SIZE
    }
}

/// macOS: show Dock icon (foreground app). No-op on other platforms.
#[cfg(target_os = "macos")]
pub fn set_dock_visible<R: Runtime>(app: &AppHandle<R>, visible: bool) {
    let policy = if visible {
        tauri::ActivationPolicy::Regular
    } else {
        // Accessory ≈ menu-bar / tray-only; Dock icon is hidden.
        tauri::ActivationPolicy::Accessory
    };
    if let Err(e) = app.set_activation_policy(policy) {
        eprintln!("[satelite] set_activation_policy failed: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_dock_visible<R: Runtime>(_app: &AppHandle<R>, _visible: bool) {}

/// Show main UI; recreate WebView if it was destroyed on tray.
///
/// Called from tray menu/click and from macOS Dock reopen (`RunEvent::Reopen`).
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    // Restore Dock icon before showing so the window can become key.
    set_dock_visible(app, true);

    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    } else {
        // Use last persisted UI mode so we don't flash pro (1024) then shrink to simple.
        let mode = app
            .try_state::<AppState>()
            .map(|s| read_ui_mode(&s.app_data_dir).to_string())
            .unwrap_or_else(|| "pro".into());
        let (w, h) = size_for_ui_mode(&mode);
        let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("Satelite")
            .inner_size(w, h)
            .resizable(false)
            .fullscreen(false)
            // Important on macOS: without activation policy / visible, Dock reopen
            // can recreate a window that never becomes key.
            .visible(true)
            .focused(true);
        match builder.build() {
            Ok(win) => {
                let _ = win.show();
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
            Err(e) => eprintln!("[satelite] recreate main window failed: {e}"),
        }
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.set_ui_visible(true);
    }
}

/// Soft-hide only (keep WebView process). Safe at app launch for silent_start.
pub fn soft_hide_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>() {
        state.set_ui_visible(false);
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
    // Silent / tray-only: hide Dock icon on macOS.
    set_dock_visible(app, false);
}

/// Hide to tray. Optionally destroy WebView (low-memory mode).
/// Default is hide-only; destroy is opt-in via `unload_ui_on_tray`.
/// Does **not** allow process exit — tray and core keep running.
pub fn hide_main_to_tray<R: Runtime>(app: &AppHandle<R>) {
    let unload = app
        .try_state::<AppState>()
        .map(|s| s.unload_ui_on_tray())
        .unwrap_or(false);

    if let Some(state) = app.try_state::<AppState>() {
        state.set_ui_visible(false);
        // Critical: destroy() may fire ExitRequested; stay alive unless tray Quit.
        // exit_allowed stays false.
    }

    // Hide Dock icon before (or with) hide — matches close-to-tray-and-dock.md.
    set_dock_visible(app, false);

    if let Some(w) = app.get_webview_window("main") {
        if unload {
            // hide first so user doesn't see a flash; then drop WKWebView
            let _ = w.hide();
            if let Err(e) = w.destroy() {
                eprintln!("[satelite] destroy main window: {e}");
                // fallback: already hidden
            }
        } else {
            let _ = w.hide();
        }
    }
}

/// Explicit full quit: allow exit, stop core, exit process.
pub fn quit_app<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>() {
        state.allow_exit();
        state.shutdown_runtime();
    }
    app.exit(0);
}

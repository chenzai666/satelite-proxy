//! Main window show / hide / destroy for tray memory management.
//!
//! Destroying the last WebView triggers Tauri `ExitRequested`. Callers must
//! keep `AppState::exit_allowed == false` so the run loop calls `prevent_exit`
//! and tray + sing-box stay alive.

use crate::state::AppState;
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Show main UI; recreate WebView if it was destroyed on tray.
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    } else {
        let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("Satelite")
            .inner_size(1024.0, 760.0)
            .resizable(false)
            .fullscreen(false);
        match builder.build() {
            Ok(w) => {
                let _ = w.show();
                let _ = w.set_focus();
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
}

/// Hide to tray. Optionally destroy WebView to free GPU/JS memory.
/// Does **not** allow process exit — tray and core keep running.
pub fn hide_main_to_tray<R: Runtime>(app: &AppHandle<R>) {
    let unload = app
        .try_state::<AppState>()
        .map(|s| s.unload_ui_on_tray())
        .unwrap_or(true);

    if let Some(state) = app.try_state::<AppState>() {
        state.set_ui_visible(false);
        // Critical: destroy() may fire ExitRequested; stay alive unless tray Quit.
        // exit_allowed stays false.
    }

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

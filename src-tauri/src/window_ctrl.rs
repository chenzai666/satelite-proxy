//! Main window show / hide / destroy for tray memory management.
//!
//! Destroying the last WebView triggers Tauri `ExitRequested`. Callers must
//! keep `AppState::exit_allowed == false` so the run loop calls `prevent_exit`
//! and tray + sing-box stay alive.

use crate::state::AppState;
use std::fs;
use std::path::PathBuf;
use tauri::{
    image::Image, window::Color, AppHandle, Manager, Runtime, Theme, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

#[cfg(windows)]
mod windows_taskbar_icon {
    use std::os::windows::ffi::OsStrExt;
    use std::sync::OnceLock;
    use tauri::{Runtime, WebviewWindow};

    const WM_SETICON: u32 = 0x0080;
    const ICON_BIG: usize = 1;

    #[link(name = "shell32")]
    extern "system" {
        fn ExtractIconExW(
            file: *const u16,
            icon_index: i32,
            large_icon: *mut isize,
            small_icon: *mut isize,
            icon_count: u32,
        ) -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageW(window: isize, message: u32, wparam: usize, lparam: isize) -> isize;
    }

    // Keep the extracted HICON alive for the process lifetime. Windows expects
    // a WM_SETICON handle to remain valid while the window uses it.
    static LARGE_ICON: OnceLock<usize> = OnceLock::new();

    fn executable_large_icon() -> Option<isize> {
        if let Some(icon) = LARGE_ICON.get() {
            return Some(*icon as isize);
        }

        let executable = std::env::current_exe().ok()?;
        let mut path: Vec<u16> = executable.as_os_str().encode_wide().collect();
        path.push(0);
        let mut icon = 0isize;
        let extracted =
            unsafe { ExtractIconExW(path.as_ptr(), 0, &mut icon, std::ptr::null_mut(), 1) };
        if extracted == 0 || icon == 0 {
            return None;
        }
        let _ = LARGE_ICON.set(icon as usize);
        Some(icon)
    }

    pub fn apply<R: Runtime>(window: &WebviewWindow<R>) {
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let Some(icon) = executable_large_icon() else {
            eprintln!("[satelite] extract Windows taskbar icon failed");
            return;
        };
        unsafe {
            SendMessageW(hwnd.0 as isize, WM_SETICON, ICON_BIG, icon);
        }
    }
}

/// Matches frontend `windowLayout.ts` (logical px).
const PRO_SIZE: (f64, f64) = (960.0, 720.0);
const SIMPLE_SIZE: (f64, f64) = (420.0, 720.0);
/// Simple mode lets the user shrink the window; content scrolls below this.
const SIMPLE_MIN: (f64, f64) = (320.0, 480.0);
/// …but never grow past the default simple strip.
const SIMPLE_MAX: (f64, f64) = SIMPLE_SIZE;
const BG_AEROSPACE: (u8, u8, u8) = (0x11, 0x14, 0x1c);
const BG_DAY: (u8, u8, u8) = (0xee, 0xf0, 0xf4);

fn is_dark_theme<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<AppState>()
        .and_then(|state| {
            state
                .with_store(|store| {
                    Ok(store
                        .settings
                        .theme
                        .trim()
                        .eq_ignore_ascii_case("aerospace"))
                })
                .ok()
        })
        .unwrap_or(false)
}

fn theme_bg_color<R: Runtime>(app: &AppHandle<R>) -> Color {
    let (r, g, b) = if is_dark_theme(app) {
        BG_AEROSPACE
    } else {
        BG_DAY
    };
    Color(r, g, b, 255)
}

/// 将原生窗口标题栏固定为应用主题，而不是让它跟随系统明暗模式漂移。
pub fn apply_window_theme<R: Runtime>(app: &AppHandle<R>) {
    let theme = if is_dark_theme(app) {
        Theme::Dark
    } else {
        Theme::Light
    };
    if let Some(window) = app.get_webview_window("main") {
        if let Err(error) = window.set_theme(Some(theme)) {
            eprintln!("[satelite] set native window theme failed: {error}");
        }
    }
}

fn set_main_window_icon<R: Runtime>(window: &WebviewWindow<R>) {
    let icon = match Image::from_bytes(include_bytes!("../icons/128x128.png")) {
        Ok(icon) => icon,
        Err(error) => {
            eprintln!("[satelite] decode main window icon failed: {error}");
            return;
        }
    };
    if let Err(error) = window.set_icon(icon) {
        eprintln!("[satelite] set main window icon failed: {error}");
    }
    #[cfg(windows)]
    windows_taskbar_icon::apply(window);
}

/// Explicitly set the top-level window icon. On Windows this stabilizes the
/// taskbar icon when the tray icon is refreshed.
pub fn apply_main_window_icon<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        set_main_window_icon(&window);
    }
}

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
        set_main_window_icon(&w);
        let _ = w.set_theme(Some(if is_dark_theme(app) {
            Theme::Dark
        } else {
            Theme::Light
        }));
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    } else {
        // Use last persisted UI mode so we don't flash pro (960) then shrink to simple.
        let mode = app
            .try_state::<AppState>()
            .map(|s| read_ui_mode(&s.app_data_dir).to_string())
            .unwrap_or_else(|| "pro".into());
        let (w, h) = size_for_ui_mode(&mode);
        let builder = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("Satelite")
            .inner_size(w, h)
            .fullscreen(false)
            .background_color(theme_bg_color(app))
            .theme(Some(if is_dark_theme(app) {
                Theme::Dark
            } else {
                Theme::Light
            }))
            // Important on macOS: without activation policy / visible, Dock reopen
            // can recreate a window that never becomes key.
            .visible(true)
            .focused(true);
        let builder = match Image::from_bytes(include_bytes!("../icons/128x128.png"))
            .and_then(|icon| builder.icon(icon))
        {
            Ok(builder) => builder,
            Err(error) => {
                eprintln!("[satelite] configure main window icon failed: {error}");
                return;
            }
        };
        let builder = match crate::portable::webview_data_dir() {
            Some(dir) => builder.data_directory(dir),
            None => builder,
        };
        // Simple mode: user-resizable strip, shrink-only (frontend restores size).
        let builder = if mode == "simple" {
            builder
                .resizable(true)
                .min_inner_size(SIMPLE_MIN.0, SIMPLE_MIN.1)
                .max_inner_size(SIMPLE_MAX.0, SIMPLE_MAX.1)
        } else {
            builder.resizable(false)
        };
        match builder.build() {
            Ok(win) => {
                set_main_window_icon(&win);
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

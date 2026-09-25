//! Main window show / hide / destroy for tray memory management.
//!
//! Destroying the last WebView triggers Tauri `ExitRequested`. Callers must
//! keep `AppState::exit_allowed == false` so the run loop calls `prevent_exit`
//! and tray + sing-box stay alive.

use crate::state::AppState;
use std::fs;
use std::path::PathBuf;
use tauri::{
    image::Image, window::Color, AppHandle, LogicalPosition, LogicalSize, Manager, Runtime, Theme, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

/// Matches frontend `windowLayout.ts` (logical px).
const PRO_SIZE: (f64, f64) = (960.0, 720.0);
const SIMPLE_SIZE: (f64, f64) = (420.0, 720.0);
/// Simple mode lets the user shrink the window; content scrolls below this.
const SIMPLE_MIN: (f64, f64) = (320.0, 480.0);
const BG_AEROSPACE: (u8, u8, u8) = (0x11, 0x14, 0x1c);
const BG_DAY: (u8, u8, u8) = (0xee, 0xf0, 0xf4);

/// Accent presets mirrored from `src/theme/accents.ts` — (aerospace, day).
#[cfg(target_os = "windows")]
const ACCENT_PRESETS: &[(&str, (u8, u8, u8), (u8, u8, u8))] = &[
    ("green", (0x55, 0xc8, 0x9a), (0x1f, 0x9a, 0x72)),
    ("blue", (0x6b, 0xb6, 0xe8), (0x2e, 0x86, 0xc8)),
    ("purple", (0xb1, 0x9c, 0xd9), (0x8e, 0x5b, 0xb8)),
    ("pink", (0xf4, 0xa6, 0xb8), (0xd6, 0x5a, 0x7e)),
    ("orange", (0xf5, 0xb9, 0x7a), (0xd8, 0x8a, 0x3d)),
    ("cyan", (0x7a, 0xd7, 0xd7), (0x2f, 0xa9, 0xa9)),
];
#[cfg(target_os = "windows")]
const DEFAULT_ACCENT_RGB: (u8, u8, u8) = (0x55, 0xc8, 0x9a);

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
    apply_titlebar_accent(app);
}

/// Resolve a stored glow/accent id to an RGB value for the active theme.
#[cfg(target_os = "windows")]
fn resolve_glow_rgb(id: &str, dark: bool) -> (u8, u8, u8) {
    let id = id.trim();
    if id.len() == 7 && id.starts_with('#') {
        if let Ok(n) = u32::from_str_radix(&id[1..], 16) {
            return (
                ((n >> 16) & 0xff) as u8,
                ((n >> 8) & 0xff) as u8,
                (n & 0xff) as u8,
            );
        }
    }
    ACCENT_PRESETS
        .iter()
        .find(|(preset, _, _)| *preset == id)
        .map(|(_, dark_rgb, light_rgb)| if dark { *dark_rgb } else { *light_rgb })
        .unwrap_or(DEFAULT_ACCENT_RGB)
}

#[cfg(target_os = "windows")]
fn blend_over(bg: (u8, u8, u8), glow: (u8, u8, u8), alpha: f64) -> (u8, u8, u8) {
    let mix = |base: u8, overlay: u8| -> u8 {
        (base as f64 * (1.0 - alpha) + overlay as f64 * alpha).round() as u8
    };
    (mix(bg.0, glow.0), mix(bg.1, glow.1), mix(bg.2, glow.2))
}

#[cfg(target_os = "windows")]
fn titlebar_accent_color<R: Runtime>(app: &AppHandle<R>) -> (u8, u8, u8) {
    let dark = is_dark_theme(app);
    let (glow_id, accent_id) = app
        .try_state::<AppState>()
        .and_then(|s| {
            s.with_store(|st| Ok((st.settings.glow_color.clone(), st.settings.accent.clone())))
                .ok()
        })
        .unwrap_or_else(|| ("accent".to_string(), "green".to_string()));
    let effective_id = if glow_id.trim() == "accent" {
        accent_id
    } else {
        glow_id
    };
    let glow_rgb = resolve_glow_rgb(&effective_id, dark);
    let bg = if dark { BG_AEROSPACE } else { BG_DAY };
    let alpha = if dark { 0.12 } else { 0.10 };
    blend_over(bg, glow_rgb, alpha)
}

/// Windows 11 (build 22000+) title-bar tint matching the dashboard glow.
/// Older Windows versions and non-Windows targets silently keep native chrome.
#[cfg(target_os = "windows")]
pub fn apply_titlebar_accent<R: Runtime>(app: &AppHandle<R>) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR};

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let Ok(hwnd) = window.hwnd() else {
        return;
    };
    let (r, g, b) = titlebar_accent_color(app);
    let colorref = COLORREF((b as u32) << 16 | (g as u32) << 8 | r as u32);
    unsafe {
        let _ = DwmSetWindowAttribute(
            HWND(hwnd.0),
            DWMWA_CAPTION_COLOR,
            &colorref as *const _ as *const _,
            std::mem::size_of::<COLORREF>() as u32,
        );
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_titlebar_accent<R: Runtime>(_app: &AppHandle<R>) {}

fn set_main_window_icon<R: Runtime>(window: &WebviewWindow<R>) {
    #[cfg(windows)]
    {
        crate::window_icon::apply_to(window);
        return;
    }
    #[cfg(not(windows))]
    {
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
    }
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

fn min_for_ui_mode(mode: &str) -> (f64, f64) {
    if mode == "simple" {
        SIMPLE_MIN
    } else {
        PRO_SIZE
    }
}

fn window_size_file(app_data_dir: &std::path::Path, mode: &str) -> PathBuf {
    app_data_dir
        .join("data")
        .join(format!("window_size_{mode}"))
}

/// Persisted per-mode window layout (logical px, `"<w> <h> [x y]"`) so a
/// recreated or restarted window is born directly at its final size and
/// position — resizing or moving after the WebView paints reads as an
/// animation to the user. The position tokens are optional so size-only
/// files from older versions keep parsing.
struct WindowLayout {
    size: (f64, f64),
    position: Option<(f64, f64)>,
}

fn read_window_layout(app_data_dir: &std::path::Path, mode: &str) -> Option<WindowLayout> {
    let raw = fs::read_to_string(window_size_file(app_data_dir, mode)).ok()?;
    let mut parts = raw.split_whitespace();
    let w: f64 = parts.next()?.parse().ok()?;
    let h: f64 = parts.next()?.parse().ok()?;
    if !w.is_finite() || !h.is_finite() {
        return None;
    }
    let (min_w, min_h) = min_for_ui_mode(mode);
    let size = (w.clamp(min_w, 8192.0), h.clamp(min_h, 8192.0));
    let x: Option<f64> = parts.next().and_then(|p| p.parse().ok());
    let y: Option<f64> = parts.next().and_then(|p| p.parse().ok());
    let position = match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some((x, y)),
        _ => None,
    };
    Some(WindowLayout { size, position })
}

/// True when a `w×h` window at logical `(x, y)` overlaps some connected
/// monitor's work area by enough to grab — restoring onto an unplugged
/// monitor or a shrunken desktop would strand the window off-screen. Each
/// work area is converted with its own monitor's scale factor, an
/// approximation on mixed-DPI setups that errs toward "reachable".
fn position_reachable<R: Runtime>(app: &AppHandle<R>, x: f64, y: f64, w: f64, h: f64) -> bool {
    let Ok(monitors) = app.available_monitors() else {
        return false;
    };
    monitors.iter().any(|m| {
        let scale = m.scale_factor();
        if !scale.is_finite() || scale <= 0.0 {
            return false;
        }
        let wa = m.work_area();
        let left = wa.position.x as f64 / scale;
        let top = wa.position.y as f64 / scale;
        let right = left + wa.size.width as f64 / scale;
        let bottom = top + wa.size.height as f64 / scale;
        titlebar_reachable(x, y, w, h, (left, top, right, bottom))
    })
}

fn titlebar_reachable(x: f64, y: f64, w: f64, h: f64, area: (f64, f64, f64, f64)) -> bool {
    if ![x, y, w, h, area.0, area.1, area.2, area.3].iter().all(|v| v.is_finite()) {
        return false;
    }
    // Seeing only the lower content is not enough: the title bar must remain
    // reachable after a monitor above the primary has been disconnected.
    let overlap_w = (x + w).min(area.2) - x.max(area.0);
    let overlap_h = (y + h.min(40.0)).min(area.3) - y.max(area.1);
    overlap_w >= 80.0 && overlap_h >= 24.0
}

/// Save the main window's current logical size and position for its UI mode.
/// Called when hiding to tray / quitting — the moments the WebView may be
/// destroyed. Maximized snapshots are skipped: restoring one would produce a
/// full-screen window that is not actually maximized.
fn persist_main_window_layout<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let mode = read_ui_mode(&state.app_data_dir);
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_maximized().unwrap_or(false) {
        return;
    }
    let Ok(size) = win.inner_size() else {
        return;
    };
    let Ok(pos) = win.outer_position() else {
        return;
    };
    let scale = win.scale_factor().unwrap_or(1.0);
    if scale <= 0.0 {
        return;
    }
    let path = window_size_file(&state.app_data_dir, mode);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(
        path,
        format!(
            "{} {} {} {}",
            size.width as f64 / scale,
            size.height as f64 / scale,
            pos.x as f64 / scale,
            pos.y as f64 / scale
        ),
    );
}

/// Resize and reposition the just-created main window (born centered at the
/// design size from config) to the persisted layout before the WebView
/// paints — cold-start companion to the tray-recreate sizing in `show_main`.
/// Also lowers the config min (960x720) to the simple-mode floor so a simple
/// window can shrink here. Without a usable position the window re-centers:
/// config centers the 960x720 design size, so a simple-mode resize would
/// otherwise sit off-center.
pub fn restore_main_window_layout<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let mode = read_ui_mode(&state.app_data_dir);
    let Some(layout) = read_window_layout(&state.app_data_dir, mode) else {
        return;
    };
    let (w, h) = layout.size;
    if let Some(win) = app.get_webview_window("main") {
        let (min_w, min_h) = min_for_ui_mode(mode);
        let _ = win.set_min_size(Some(LogicalSize::new(min_w, min_h)));
        let _ = win.set_size(LogicalSize::new(w, h));
        match layout.position {
            Some((x, y)) if position_reachable(app, x, y, w, h) => {
                let _ = win.set_position(LogicalPosition::new(x, y));
            }
            _ => {
                let _ = win.center();
            }
        }
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
        apply_titlebar_accent(app);
        let _ = w.show();
        let _ = w.unminimize();
        if let (Ok(pos), Ok(size), Ok(scale)) = (w.outer_position(), w.inner_size(), w.scale_factor()) {
            if scale.is_finite() && scale > 0.0 && !position_reachable(app,
                pos.x as f64 / scale, pos.y as f64 / scale,
                size.width as f64 / scale, size.height as f64 / scale) {
                let _ = w.center();
            }
        }
        let _ = w.set_focus();
    } else {
        // Use last persisted UI mode so we don't flash pro (960) then shrink to simple.
        let mode = app
            .try_state::<AppState>()
            .map(|s| read_ui_mode(&s.app_data_dir).to_string())
            .unwrap_or_else(|| "pro".into());
        // Born at the persisted layout (persist_main_window_layout) so waking
        // from tray shows the final size and position directly — no grow
        // animation, no OS cascade placement.
        let layout = app
            .try_state::<AppState>()
            .and_then(|s| read_window_layout(&s.app_data_dir, &mode));
        let (w, h) = layout
            .as_ref()
            .map(|l| l.size)
            .unwrap_or_else(|| size_for_ui_mode(&mode));
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
        let builder = match layout.as_ref().and_then(|l| l.position) {
            Some((x, y)) if position_reachable(app, x, y, w, h) => builder.position(x, y),
            _ => builder.center(),
        };
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
        // Both modes resizable; the frontend restores the persisted exact
        // size (and the simple 320x480 floor) right after the WebView mounts.
        let builder = if mode == "simple" {
            builder
                .resizable(true)
                .min_inner_size(SIMPLE_MIN.0, SIMPLE_MIN.1)
        } else {
            builder
                .resizable(true)
                .min_inner_size(PRO_SIZE.0, PRO_SIZE.1)
        };
        match builder.build() {
            Ok(win) => {
                set_main_window_icon(&win);
                apply_titlebar_accent(app);
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

    // Capture the size and position while the window still exists — destroy
    // below may drop it, and the next recreate needs them at build time.
    persist_main_window_layout(app);

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
    // Keep the window layout file fresh for the next launch (no-op when the
    // WebView was already destroyed — hide_main_to_tray persisted then).
    persist_main_window_layout(app);
    if let Some(state) = app.try_state::<AppState>() {
        state.allow_exit();
        state.shutdown_runtime();
    }
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_requires_reachable_titlebar() {
        let primary = (0.0, 0.0, 1920.0, 1040.0);
        assert!(titlebar_reachable(100.0, 100.0, 960.0, 720.0, primary));
        assert!(!titlebar_reachable(2000.0, 100.0, 960.0, 720.0, primary));
        assert!(!titlebar_reachable(100.0, -600.0, 960.0, 720.0, primary));
        assert!(!titlebar_reachable(1900.0, 100.0, 960.0, 720.0, primary));
        assert!(titlebar_reachable(-1800.0, 50.0, 960.0, 720.0, (-1920.0, 0.0, 0.0, 1080.0)));
        assert!(!titlebar_reachable(f64::NAN, 0.0, 960.0, 720.0, primary));
    }

    fn temp_root(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("satelite-window-ctrl-{tag}-{}", std::process::id()))
    }

    fn write_layout(tag: &str, contents: &str) -> std::path::PathBuf {
        let root = temp_root(tag);
        std::fs::create_dir_all(root.join("data")).expect("create temp data dir");
        std::fs::write(window_size_file(&root, "pro"), contents).expect("write layout file");
        root
    }

    #[test]
    fn parses_size_and_position() {
        let root = write_layout("full", "1100 800 240 130\n");
        let l = read_window_layout(&root, "pro").expect("layout");
        assert_eq!(l.size, (1100.0, 800.0));
        assert_eq!(l.position, Some((240.0, 130.0)));
    }

    #[test]
    fn parses_legacy_size_only() {
        let root = write_layout("legacy", "1280 800");
        let l = read_window_layout(&root, "pro").expect("layout");
        assert_eq!(l.size, (1280.0, 800.0));
        assert_eq!(
            l.position, None,
            "no position tokens -> None, not a failure"
        );
    }

    #[test]
    fn parses_negative_positions() {
        // Monitor left of / above the primary has negative coordinates.
        let root = write_layout("negative", "960 720 -1920.5 -8");
        let l = read_window_layout(&root, "pro").expect("layout");
        assert_eq!(l.position, Some((-1920.5, -8.0)));
    }

    #[test]
    fn clamps_size_to_mode_floor() {
        let root = write_layout("clamp", "100 100 0 0");
        let l = read_window_layout(&root, "pro").expect("layout");
        assert_eq!(l.size, (960.0, 720.0), "pro floor is the design size");
    }

    #[test]
    fn rejects_malformed() {
        let root = write_layout("garbage", "not-a-number");
        assert!(read_window_layout(&root, "pro").is_none());
        let root = write_layout("height-missing", "960");
        assert!(read_window_layout(&root, "pro").is_none());
        let root = temp_root("absent");
        let _ = std::fs::remove_dir_all(&root);
        assert!(read_window_layout(&root, "pro").is_none());
    }
}

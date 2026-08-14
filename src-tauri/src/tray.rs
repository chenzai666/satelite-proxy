//! System tray: open window, start/stop, quit (with cleanup).

use crate::domain::TrayIconStyle;
use crate::state::AppState;
use crate::window_ctrl;
use std::io::Write;
use std::process::{Command, Stdio};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime as TauriRuntime,
};

/// Same shell line as Dashboard “复制环境变量”.
fn proxy_env_export(mixed_port: u16) -> String {
    let proxy_url = format!("http://127.0.0.1:{mixed_port}");
    if cfg!(target_os = "windows") {
        format!(r#"$env:ALL_PROXY = "{proxy_url}""#)
    } else {
        format!("export all_proxy={proxy_url}")
    }
}

#[cfg(test)]
mod tests {
    use super::proxy_env_export;

    #[test]
    fn proxy_env_matches_current_platform_shell() {
        let text = proxy_env_export(2080);
        if cfg!(target_os = "windows") {
            assert_eq!(text, r#"$env:ALL_PROXY = "http://127.0.0.1:2080""#);
        } else {
            assert_eq!(text, "export all_proxy=http://127.0.0.1:2080");
        }
    }
}

fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("pbcopy: {e}"))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| "pbcopy stdin unavailable".to_string())?
            .write_all(text.as_bytes())
            .map_err(|e| format!("pbcopy write: {e}"))?;
        let status = child.wait().map_err(|e| format!("pbcopy wait: {e}"))?;
        if !status.success() {
            return Err("pbcopy failed".into());
        }
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let mut child = Command::new("cmd")
            .args(["/C", "clip"])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("clip: {e}"))?;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| "clip stdin unavailable".to_string())?
            .write_all(text.as_bytes())
            .map_err(|e| format!("clip write: {e}"))?;
        let status = child.wait().map_err(|e| format!("clip wait: {e}"))?;
        if !status.success() {
            return Err("clip failed".into());
        }
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for (bin, args) in [
            ("wl-copy", &[][..]),
            ("xclip", &["-selection", "clipboard"][..]),
        ] {
            let Ok(mut child) = Command::new(bin).args(args).stdin(Stdio::piped()).spawn() else {
                continue;
            };
            if let Some(stdin) = child.stdin.as_mut() {
                if stdin.write_all(text.as_bytes()).is_err() {
                    continue;
                }
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return Ok(());
            }
        }
        return Err("no clipboard tool (wl-copy / xclip)".into());
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    {
        let _ = text;
        Err("clipboard unsupported on this platform".into())
    }
}

fn copy_proxy_env(app: &AppHandle<impl TauriRuntime>) {
    let port = app
        .try_state::<AppState>()
        .and_then(|s| s.with_store(|st| Ok(st.settings.mixed_port)).ok())
        .unwrap_or(2080);
    let text = proxy_env_export(port);
    if let Err(e) = copy_text_to_clipboard(&text) {
        eprintln!("[satelite] tray copy env failed: {e}");
    }
}

const TRAY_ID: &str = "main";

fn tray_png(style: TrayIconStyle, running: bool) -> (&'static [u8], bool) {
    match (style, running) {
        (TrayIconStyle::Badge, true) => {
            (include_bytes!("../icons/tray/badge-on.png"), false)
        }
        (TrayIconStyle::Badge, false) => {
            (include_bytes!("../icons/tray/badge-off.png"), false)
        }
        (TrayIconStyle::Mark, true) => (include_bytes!("../icons/tray/mark-on.png"), false),
        // Transparent flat mark: black silhouette, macOS tints for the menu bar.
        (TrayIconStyle::Mark, false) => (include_bytes!("../icons/tray/mark-off.png"), true),
        (TrayIconStyle::Ghost, true) => {
            (include_bytes!("../icons/tray/ghost-on.png"), false)
        }
        (TrayIconStyle::Ghost, false) => {
            (include_bytes!("../icons/tray/ghost-off.png"), false)
        }
        (TrayIconStyle::Buddy, true) => {
            (include_bytes!("../icons/tray/buddy-on.png"), false)
        }
        (TrayIconStyle::Buddy, false) => {
            (include_bytes!("../icons/tray/buddy-off.png"), false)
        }
    }
}

fn current_style(app: &AppHandle<impl TauriRuntime>) -> TrayIconStyle {
    app.try_state::<AppState>()
        .and_then(|s| s.with_store(|st| Ok(st.settings.tray_icon)).ok())
        .unwrap_or_default()
}

/// Re-select and apply the tray icon based on current core run state.
/// Safe to call frequently; `is_core_running()` is a short-lived lock.
pub fn refresh_icon<R: TauriRuntime>(app: &AppHandle<R>) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let running = app
        .try_state::<AppState>()
        .map(|s| s.is_core_running())
        .unwrap_or(false);
    let (bytes, as_template) = tray_png(current_style(app), running);
    let Ok(icon) = Image::from_bytes(bytes) else {
        return;
    };
    let _ = tray.set_icon_with_as_template(Some(icon), as_template);
}

pub fn setup_tray<R: TauriRuntime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)?;
    let start_i = MenuItem::with_id(app, "start", "启动代理", true, None::<&str>)?;
    let stop_i = MenuItem::with_id(app, "stop", "停止代理", true, None::<&str>)?;
    let copy_env_i = MenuItem::with_id(app, "copy_env", "复制环境变量", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&show_i, &sep, &start_i, &stop_i, &copy_env_i, &sep, &quit_i],
    )?;

    // Prefer app icon; fall back to default tray without custom image if load fails.
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Satelite")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                window_ctrl::show_main(app);
            }
            "start" => {
                if let Some(state) = app.try_state::<AppState>() {
                    let res = app.path().resource_dir().ok();
                    let _ = state.start_proxy(res.as_deref(), true);
                }
                refresh_icon(app);
            }
            "stop" => {
                if let Some(state) = app.try_state::<AppState>() {
                    let _ = state.stop_proxy();
                }
                refresh_icon(app);
            }
            "copy_env" => {
                copy_proxy_env(app);
            }
            "quit" => {
                window_ctrl::quit_app(app);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window_ctrl::show_main(tray.app_handle());
            }
        });

    // Placeholder; refresh_icon applies the user's chosen set + run state.
    let (bytes, as_template) = tray_png(current_style(app), false);
    if let Ok(icon) = Image::from_bytes(bytes) {
        builder = builder.icon(icon).icon_as_template(as_template);
    } else if let Ok(icon) = Image::from_bytes(include_bytes!("../icons/tray-icon.png")) {
        builder = builder.icon(icon);
    } else if let Ok(icon) = Image::from_bytes(include_bytes!("../icons/32x32.png")) {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    refresh_icon(app);
    Ok(())
}

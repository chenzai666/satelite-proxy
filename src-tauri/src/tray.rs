//! System tray: open window, start/stop, quit (with cleanup).

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
    format!("export all_proxy=http://127.0.0.1:{mixed_port}")
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
        for (bin, args) in [("wl-copy", &[][..]), ("xclip", &["-selection", "clipboard"][..])] {
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
    let mut builder = TrayIconBuilder::new()
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
            }
            "stop" => {
                if let Some(state) = app.try_state::<AppState>() {
                    let _ = state.stop_proxy();
                }
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

    // Monochrome satellite mark for menu bar (black silhouette = macOS template).
    // Template mode lets the system recolor for light/dark menu bars.
    if let Ok(icon) = Image::from_bytes(include_bytes!("../icons/tray-icon-template.png")) {
        builder = builder.icon(icon).icon_as_template(true);
    } else if let Ok(icon) = Image::from_bytes(include_bytes!("../icons/tray-icon.png")) {
        builder = builder.icon(icon);
    } else if let Ok(icon) = Image::from_bytes(include_bytes!("../icons/32x32.png")) {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

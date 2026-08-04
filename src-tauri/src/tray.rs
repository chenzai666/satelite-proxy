//! System tray: open window, start/stop, quit (with cleanup).

use crate::state::AppState;
use crate::window_ctrl;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime as TauriRuntime,
};

pub fn setup_tray<R: TauriRuntime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "打开主界面", true, None::<&str>)?;
    let start_i = MenuItem::with_id(app, "start", "启动代理", true, None::<&str>)?;
    let stop_i = MenuItem::with_id(app, "stop", "停止代理", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&show_i, &sep, &start_i, &stop_i, &sep, &quit_i])?;

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

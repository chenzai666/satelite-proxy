mod api;
mod app_log;
mod autostart;
mod commands;
mod config;
mod conn_journal;
mod core;
mod domain;
mod error;
mod proxy;
mod runtime;
mod services;
mod state;
mod storage;
mod subscription;
mod smart_switch;
mod subscription_auto;
mod tray;
mod window_ctrl;

use state::AppState;
use tauri::Manager;

pub use domain::{
    AppSettings, ParseResult as SubscriptionParseResult, Protocol, ProtocolConfig, ProxyNode,
    SkippedProxy, Subscription, SubscriptionFormat, SubscriptionSource, SubscriptionView,
    TlsConfig, Transport,
};
pub use subscription::parse_subscription;

pub async fn download_core_to(
    app_data_dir: &std::path::Path,
    tag: Option<String>,
) -> Result<core::CoreDownloadResult, String> {
    core::download_latest_core(app_data_dir, tag)
        .await
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("resolve app data dir");
            std::fs::create_dir_all(&dir).ok();
            let resource_dir = app.path().resource_dir().ok();
            let app_state = AppState::load(dir, resource_dir).expect("load app store");

            // Snapshot app prefs before move into managed state
            let silent = app_state
                .with_store(|s| Ok(s.settings.silent_start))
                .unwrap_or(false);
            let auto_proxy = app_state
                .with_store(|s| Ok(s.settings.auto_start_proxy))
                .unwrap_or(false);
            // Keep LaunchAgent in sync with stored preference
            let launch = app_state
                .with_store(|s| Ok(s.settings.launch_at_login))
                .unwrap_or(false);
            let _ = autostart::set_launch_at_login(launch);

            app.manage(app_state);
            app_log::info("app", "Satelite started");

            if let Err(e) = tray::setup_tray(app.handle()) {
                app_log::error("tray", format!("setup failed: {e}"));
            }

            // Connection journal: WebSocket snapshots @100ms + ring history.
            // Clash API only yields live sockets; low-interval stream reduces misses.
            conn_journal::spawn_connection_journal(app.handle().clone());

            // Profile auto-update (per-subscription interval, default 1440 min).
            subscription_auto::spawn(app.handle().clone());

            // Smart node switch (docs/auto.md): passive + on-demand probe.
            smart_switch::spawn(app.handle().clone());

            // Silent start: hide only (do not destroy at launch — that can exit the app).
            if silent {
                window_ctrl::soft_hide_main(app.handle());
            }

            // Auto-run proxy after launch
            if auto_proxy {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    // slight delay so tray / window settle
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    if let Some(state) = handle.try_state::<AppState>() {
                        let res = handle.path().resource_dir().ok();
                        if let Err(e) = state.start_proxy(res.as_deref(), false) {
                            app_log::error("app", format!("auto_start_proxy failed: {e}"));
                        } else {
                            app_log::info("app", "auto_start_proxy ok");
                        }
                    }
                });
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let close_to_tray = window
                        .app_handle()
                        .try_state::<AppState>()
                        .and_then(|s| s.with_store(|st| Ok(st.settings.close_to_tray)).ok())
                        .unwrap_or(true);
                    if close_to_tray {
                        // Keep Rust + tray + core; optionally destroy WebView for memory.
                        api.prevent_close();
                        window_ctrl::hide_main_to_tray(window.app_handle());
                    } else {
                        // Real quit from window close
                        api.prevent_close();
                        window_ctrl::quit_app(window.app_handle());
                    }
                }
                tauri::WindowEvent::Focused(true) => {
                    if let Some(state) = window.app_handle().try_state::<AppState>() {
                        state.set_ui_visible(true);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_subscriptions,
            commands::get_subscription,
            commands::add_subscription_url,
            commands::add_subscription_file,
            commands::update_subscription,
            commands::refresh_subscription,
            commands::activate_subscription,
            commands::set_mix_mode,
            commands::remove_subscription,
            commands::list_subscription_nodes,
            commands::list_all_nodes,
            commands::get_settings,
            commands::update_settings,
            commands::set_current_node,
            commands::generate_singbox_config,
            commands::preview_singbox_config,
            commands::get_active_config_path,
            commands::get_core_info,
            commands::check_core_update,
            commands::download_core,
            commands::fetch_core_latest,
            commands::test_nodes_latency,
            commands::get_proxy_status,
            commands::start_proxy,
            commands::stop_proxy,
            commands::restart_proxy,
            commands::set_system_proxy,
            commands::set_tun_enabled,
            commands::set_outbound_mode,
            commands::get_dns_settings,
            commands::update_dns_settings,
            commands::test_dns_lookup,
            commands::set_current_node_live,
            commands::smart_switch_now,
            commands::list_rule_sets,
            commands::get_rule_set,
            commands::set_active_rule_set,
            commands::set_rule_set_enabled,
            commands::create_rule_set,
            commands::reorder_rule_sets,
            commands::delete_rule_set,
            commands::reset_rule_set,
            commands::reset_builtin_rule_set,
            commands::list_rules,
            commands::save_rule,
            commands::remove_rule,
            commands::set_rule_enabled,
            commands::list_connections,
            commands::list_requests,
            commands::clear_request_history,
            commands::list_app_logs,
            commands::clear_app_logs,
            parse_subscription_text,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            match event {
                // Destroying the last WebView triggers ExitRequested. Stay in tray
                // unless the user explicitly quit (exit_allowed).
                tauri::RunEvent::ExitRequested { api, .. } => {
                    let allow = app_handle
                        .try_state::<AppState>()
                        .map(|s| s.is_exit_allowed())
                        .unwrap_or(false);
                    if !allow {
                        api.prevent_exit();
                        return;
                    }
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        state.shutdown_runtime();
                    }
                }
                tauri::RunEvent::Exit => {
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        if state.is_exit_allowed() {
                            state.shutdown_runtime();
                        }
                    }
                }
                _ => {}
            }
        });
}

#[tauri::command]
fn parse_subscription_text(content: String) -> Result<domain::ParseResult, String> {
    parse_subscription(&content).map_err(|e| e.to_string())
}

use crate::config::{
    active_config_path, build_singbox_config, generate_api_secret, write_active_config,
    BuildOptions,
};
use crate::domain::{AppSettings, ProxyNode};
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, State};

#[derive(Debug, Serialize)]
pub struct GenerateConfigResult {
    pub path: String,
    pub selected_tag: String,
    pub outbound_count: usize,
    pub mixed_port: u16,
    pub api_port: u16,
    /// Pretty JSON for UI preview (may be large).
    pub preview: String,
}

/// Node list item for UI: ProxyNode fields + owning subscription (mix mode label).
#[derive(Debug, Serialize)]
pub struct ListedNode {
    #[serde(flatten)]
    pub node: ProxyNode,
    pub subscription_id: String,
    pub subscription_name: String,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    state
        .with_store(|store| Ok(store.settings.clone()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mixed_port: Option<u16>,
    api_port: Option<u16>,
    probe_url: Option<String>,
    tun_enabled: Option<bool>,
    tun_stack: Option<String>,
    close_to_tray: Option<bool>,
    launch_at_login: Option<bool>,
    silent_start: Option<bool>,
    auto_start_proxy: Option<bool>,
    close_connections_on_switch: Option<bool>,
    locale: Option<String>,
    theme: Option<String>,
    accent: Option<String>,
    tray_icon: Option<String>,
    unload_ui_on_tray: Option<bool>,
    smart_switch: Option<bool>,
    auto_select: Option<String>, // off | smart | kernel
    route_final: Option<String>, // proxy | direct | block (Rule mode)
    find_process: Option<bool>,
) -> Result<AppSettings, String> {
    let mut launch_changed: Option<bool> = None;
    let mut auto_select_changed: Option<(
        crate::domain::AutoSelectMode,
        crate::domain::AutoSelectMode,
    )> = None;
    let mut route_final_changed = false;
    let mut find_process_changed = false;
    let settings = state
        .with_store_mut(|store| {
            if let Some(p) = mixed_port {
                store.settings.mixed_port = p;
            }
            if let Some(p) = api_port {
                store.settings.api_port = p;
            }
            if let Some(u) = probe_url {
                if !u.trim().is_empty() {
                    store.settings.probe_url = u;
                }
            }
            if let Some(t) = tun_enabled {
                store.settings.tun_enabled = t;
                if t {
                    store.settings.capture_mode = crate::domain::CaptureMode::Tun;
                } else if store.settings.capture_mode == crate::domain::CaptureMode::Tun {
                    store.settings.capture_mode = crate::domain::CaptureMode::Off;
                }
            }
            if let Some(s) = tun_stack {
                let s = s.trim().to_ascii_lowercase();
                if matches!(s.as_str(), "system" | "gvisor" | "mixed") {
                    store.settings.tun_stack = s;
                }
            }
            if let Some(v) = close_to_tray {
                store.settings.close_to_tray = v;
            }
            if let Some(v) = launch_at_login {
                if store.settings.launch_at_login != v {
                    launch_changed = Some(v);
                }
                store.settings.launch_at_login = v;
            }
            if let Some(v) = silent_start {
                store.settings.silent_start = v;
            }
            if let Some(v) = auto_start_proxy {
                store.settings.auto_start_proxy = v;
            }
            if let Some(v) = close_connections_on_switch {
                store.settings.close_connections_on_switch = v;
            }
            if let Some(loc) = locale {
                let loc = loc.trim().to_ascii_lowercase();
                if matches!(loc.as_str(), "zh" | "en") {
                    store.settings.locale = loc;
                }
            }
            if let Some(th) = theme {
                let th = th.trim().to_ascii_lowercase();
                if matches!(th.as_str(), "aerospace" | "day") {
                    store.settings.theme = th;
                }
            }
            if let Some(ac) = accent {
                let ac = ac.trim().to_ascii_lowercase();
                if matches!(
                    ac.as_str(),
                    "green" | "blue" | "purple" | "pink" | "orange" | "cyan"
                ) {
                    store.settings.accent = ac;
                }
            }
            if let Some(raw) = tray_icon {
                if let Some(style) = crate::domain::TrayIconStyle::parse(&raw) {
                    store.settings.tray_icon = style;
                }
            }
            if let Some(v) = unload_ui_on_tray {
                store.settings.unload_ui_on_tray = v;
            }
            if let Some(rf) = route_final {
                let rf = rf.trim().to_ascii_lowercase();
                if matches!(rf.as_str(), "proxy" | "direct" | "block") {
                    if store.settings.route_final != rf {
                        route_final_changed = true;
                        store.settings.route_final = rf;
                    }
                }
            }
            // Prefer explicit auto_select; legacy smart_switch maps to off/smart.
            if let Some(v) = find_process {
                if store.settings.find_process != v {
                    find_process_changed = true;
                    store.settings.find_process = v;
                }
            }
            if let Some(raw) = auto_select {
                if let Some(mode) = crate::domain::AutoSelectMode::parse(&raw) {
                    let prev = store.settings.auto_select;
                    if prev != mode {
                        auto_select_changed = Some((prev, mode));
                        store.settings.auto_select = mode;
                        store.settings.smart_switch = mode.is_smart();
                        crate::app_log::info(
                            "settings",
                            format!("auto_select {} → {}", prev.as_str(), mode.as_str()),
                        );
                    }
                }
            } else if let Some(v) = smart_switch {
                let mode = if v {
                    crate::domain::AutoSelectMode::Smart
                } else {
                    crate::domain::AutoSelectMode::Off
                };
                let prev = store.settings.auto_select;
                // Don't clobber kernel via legacy bool unless turning smart on/off from non-kernel.
                if prev.is_kernel() && !v {
                    // off from UI that still sends smartSwitch:false while on kernel → treat as off
                    auto_select_changed = Some((prev, crate::domain::AutoSelectMode::Off));
                    store.settings.auto_select = crate::domain::AutoSelectMode::Off;
                    store.settings.smart_switch = false;
                } else if prev != mode && !prev.is_kernel() {
                    auto_select_changed = Some((prev, mode));
                    store.settings.auto_select = mode;
                    store.settings.smart_switch = mode.is_smart();
                } else if prev.is_kernel() && v {
                    auto_select_changed = Some((prev, crate::domain::AutoSelectMode::Smart));
                    store.settings.auto_select = crate::domain::AutoSelectMode::Smart;
                    store.settings.smart_switch = true;
                }
                crate::app_log::info(
                    "settings",
                    format!(
                        "smart_switch legacy → auto_select {}",
                        store.settings.auto_select.as_str()
                    ),
                );
            }
            Ok(store.settings.clone())
        })
        .map_err(|e| e.to_string())?;

    if let Some(enabled) = launch_changed {
        crate::autostart::set_launch_at_login(enabled).map_err(|e| e.to_string())?;
    }
    crate::tray::refresh_icon(&app);

    // route.final must restart: sing-box Clash PUT /configs often returns OK without
    // re-applying route.final (file updates, process keeps old final).
    // selector ↔ urltest also needs a full restart (outbound type changes).
    let need_restart = route_final_changed
        || find_process_changed
        || auto_select_changed
            .map(|(prev, next)| prev.is_kernel() != next.is_kernel())
            .unwrap_or(false);
    if need_restart {
        crate::rule_apply::request_restart(app, Vec::new());
    }

    Ok(settings)
}

#[tauri::command]
pub fn set_current_node(
    state: State<'_, AppState>,
    node_id: String,
) -> Result<AppSettings, String> {
    let (settings, close_conns) = state
        .with_store_mut(|store| {
            if store.find_node(&node_id).is_none() {
                return Err(crate::error::AppError::NotFound(node_id.clone()));
            }
            store.settings.current_node_id = Some(node_id.clone());
            Ok((
                store.settings.clone(),
                store.settings.close_connections_on_switch,
            ))
        })
        .map_err(|e| e.to_string())?;

    // Hot-switch selector when core is running (no restart).
    if state.is_core_running() {
        if let Ok(tag) = state.with_store(|store| {
            store
                .find_node(&node_id)
                .map(crate::config::outbound_tag)
                .ok_or_else(|| crate::error::AppError::NotFound(node_id.clone()))
        }) {
            let runtime = state.lock_runtime();
            let _ = runtime.select_node_live(&tag);
            if close_conns {
                if let Some(api) = runtime.clash_api_clone() {
                    let _ = api.close_all_connections();
                }
            }
        }
    }

    Ok(settings)
}

#[tauri::command]
pub fn list_all_nodes(state: State<'_, AppState>) -> Result<Vec<ListedNode>, String> {
    state
        .with_store(|store| {
            let names: HashMap<&str, &str> = store
                .subscriptions
                .iter()
                .map(|s| (s.id.as_str(), s.name.as_str()))
                .collect();
            let enabled: std::collections::HashSet<&str> = store
                .subscriptions
                .iter()
                .filter(|s| s.enabled)
                .map(|s| s.id.as_str())
                .collect();
            Ok(store
                .nodes
                .iter()
                .filter(|n| enabled.contains(n.subscription_id.as_str()))
                .map(|n| ListedNode {
                    node: n.node.clone(),
                    subscription_id: n.subscription_id.clone(),
                    subscription_name: names
                        .get(n.subscription_id.as_str())
                        .copied()
                        .unwrap_or("")
                        .to_string(),
                })
                .collect())
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn generate_singbox_config(state: State<'_, AppState>) -> Result<GenerateConfigResult, String> {
    let secret = generate_api_secret();

    let (nodes, settings, rules, remote_rule_sets, dns) = state
        .with_store(|store| {
            Ok((
                store.enabled_nodes(),
                store.settings.clone(),
                store.enabled_rules_sorted(),
                store.enabled_rule_sets(),
                store.dns.clone(),
            ))
        })
        .map_err(|e| e.to_string())?;

    let built = build_singbox_config(
        &nodes,
        &BuildOptions {
            mixed_port: settings.mixed_port,
            api_port: settings.api_port,
            api_secret: secret.clone(),
            current_node_id: settings.current_node_id.clone(),
            log_level: "info".into(),
            rules,
            rule_sets: remote_rule_sets,
            tun_enabled: settings.tun_enabled,
            tun_stack: settings.tun_stack.clone(),
            dns,
            outbound_mode: settings.outbound_mode,
            route_final: settings.route_final.clone(),
            auto_select: settings.auto_select,
            probe_url: settings.probe_url.clone(),
            find_process: settings.find_process,
        },
    )
    .map_err(|e| e.to_string())?;

    let path = write_active_config(&state.app_data_dir, &built).map_err(|e| e.to_string())?;

    // persist secret + ensure current node set if missing
    state
        .with_store_mut(|store| {
            store.settings.clash_api_secret = Some(secret);
            if store.settings.current_node_id.is_none() {
                if let Some(first) = store.enabled_nodes().first() {
                    store.settings.current_node_id = Some(first.id.clone());
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;

    let preview = serde_json::to_string_pretty(&built.value).unwrap_or_default();

    Ok(GenerateConfigResult {
        path: path.display().to_string(),
        selected_tag: built.selected_tag,
        outbound_count: built.outbound_tags.len(),
        mixed_port: settings.mixed_port,
        api_port: settings.api_port,
        preview,
    })
}

#[tauri::command]
pub fn get_active_config_path(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let path = active_config_path(&state.app_data_dir);
    if path.exists() {
        Ok(Some(path.display().to_string()))
    } else {
        Ok(None)
    }
}

#[tauri::command]
pub fn preview_singbox_config(state: State<'_, AppState>) -> Result<GenerateConfigResult, String> {
    let (nodes, settings, rules, remote_rule_sets, dns) = state
        .with_store(|store| {
            Ok((
                store.enabled_nodes(),
                store.settings.clone(),
                store.enabled_rules_sorted(),
                store.enabled_rule_sets(),
                store.dns.clone(),
            ))
        })
        .map_err(|e| e.to_string())?;

    let secret = settings
        .clash_api_secret
        .clone()
        .unwrap_or_else(generate_api_secret);

    let built = build_singbox_config(
        &nodes,
        &BuildOptions {
            mixed_port: settings.mixed_port,
            api_port: settings.api_port,
            api_secret: secret,
            current_node_id: settings.current_node_id.clone(),
            log_level: "info".into(),
            rules,
            rule_sets: remote_rule_sets,
            tun_enabled: settings.tun_enabled,
            tun_stack: settings.tun_stack.clone(),
            dns,
            outbound_mode: settings.outbound_mode,
            route_final: settings.route_final.clone(),
            auto_select: settings.auto_select,
            probe_url: settings.probe_url.clone(),
            find_process: settings.find_process,
        },
    )
    .map_err(|e| e.to_string())?;

    let path = active_config_path(&state.app_data_dir);
    let preview = serde_json::to_string_pretty(&built.value).unwrap_or_default();

    Ok(GenerateConfigResult {
        path: path.display().to_string(),
        selected_tag: built.selected_tag,
        outbound_count: built.outbound_tags.len(),
        mixed_port: settings.mixed_port,
        api_port: settings.api_port,
        preview,
    })
}

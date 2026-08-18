use crate::config::{
    active_config_path, build_singbox_config, generate_api_secret, write_active_config,
    BuildOptions,
};
use crate::domain::{AppSettings, ProxyNode};
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Manager, State};

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

#[derive(Debug, Serialize)]
pub struct NodePage {
    pub nodes: Vec<ListedNode>,
    pub total: usize,
    pub offset: usize,
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
    hero_style: Option<String>,
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
            if let Some(hs) = hero_style {
                let hs = hs.trim().to_ascii_lowercase();
                if matches!(hs.as_str(), "particle" | "classic") {
                    store.settings.hero_style = hs;
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
pub async fn set_current_node(app: AppHandle, node_id: String) -> Result<AppSettings, String> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker_app
            .try_state::<AppState>()
            .ok_or_else(|| "app state unavailable".to_string())?;
        let (settings, was_kernel, _) = state
            .select_current_node_serialized(&node_id, true, true)
            .map_err(|e| e.to_string())?;
        if was_kernel {
            crate::rule_apply::request_restart(worker_app.clone(), Vec::new());
        }
        Ok(settings)
    })
    .await
    .map_err(|e| format!("select node task: {e}"))?
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
pub fn list_nodes_page(
    state: State<'_, AppState>,
    query: Option<String>,
    sort_mode: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<NodePage, String> {
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
            let query = query.unwrap_or_default().trim().to_lowercase();
            let mut nodes: Vec<ListedNode> = store
                .nodes
                .iter()
                .filter(|n| enabled.contains(n.subscription_id.as_str()))
                .filter(|n| {
                    query.is_empty()
                        || n.node.name.to_lowercase().contains(&query)
                        || n.node.server.to_lowercase().contains(&query)
                        || n.node.protocol.as_str().to_lowercase().contains(&query)
                        || names
                            .get(n.subscription_id.as_str())
                            .is_some_and(|name| name.to_lowercase().contains(&query))
                })
                .map(|n| ListedNode {
                    node: n.node.clone(),
                    subscription_id: n.subscription_id.clone(),
                    subscription_name: names
                        .get(n.subscription_id.as_str())
                        .copied()
                        .unwrap_or("")
                        .to_string(),
                })
                .collect();
            match sort_mode.as_deref() {
                Some("name") => nodes.sort_by_cached_key(|n| n.node.name.to_lowercase()),
                Some("latency") => nodes.sort_by(|a, b| {
                    let score = |n: &ListedNode| match n.node.latency_ms {
                        Some(ms) => (0u8, ms as u64),
                        None if n.node.latency_at.is_some() => (1, 0),
                        None => (2, 0),
                    };
                    score(a)
                        .cmp(&score(b))
                        .then_with(|| a.node.name.to_lowercase().cmp(&b.node.name.to_lowercase()))
                }),
                _ => {}
            }
            let total = nodes.len();
            let offset = offset.unwrap_or(0).min(total);
            let limit = limit.unwrap_or(200).clamp(1, 500);
            let nodes = nodes.into_iter().skip(offset).take(limit).collect();
            Ok(NodePage {
                nodes,
                total,
                offset,
            })
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_node_ids(
    state: State<'_, AppState>,
    query: Option<String>,
) -> Result<Vec<String>, String> {
    state
        .with_store(|store| {
            let enabled: std::collections::HashSet<&str> = store
                .subscriptions
                .iter()
                .filter(|s| s.enabled)
                .map(|s| s.id.as_str())
                .collect();
            let names: HashMap<&str, &str> = store
                .subscriptions
                .iter()
                .map(|s| (s.id.as_str(), s.name.as_str()))
                .collect();
            let query = query.unwrap_or_default().trim().to_lowercase();
            Ok(store
                .nodes
                .iter()
                .filter(|n| enabled.contains(n.subscription_id.as_str()))
                .filter(|n| {
                    query.is_empty()
                        || n.node.name.to_lowercase().contains(&query)
                        || n.node.server.to_lowercase().contains(&query)
                        || n.node.protocol.as_str().to_lowercase().contains(&query)
                        || names
                            .get(n.subscription_id.as_str())
                            .is_some_and(|name| name.to_lowercase().contains(&query))
                })
                .map(|n| n.node.id.clone())
                .collect())
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn generate_singbox_config(
    state: State<'_, AppState>,
) -> Result<GenerateConfigResult, String> {
    let secret = generate_api_secret();
    let app_data_dir = state.app_data_dir.clone();

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

    let worker_secret = secret.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: settings.mixed_port,
                api_port: settings.api_port,
                api_secret: worker_secret,
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
        let path = write_active_config(&app_data_dir, &built).map_err(|e| e.to_string())?;
        let preview = serde_json::to_string_pretty(&built.value).unwrap_or_default();
        Ok::<_, String>(GenerateConfigResult {
            path: path.display().to_string(),
            selected_tag: built.selected_tag,
            outbound_count: built.outbound_tags.len(),
            mixed_port: settings.mixed_port,
            api_port: settings.api_port,
            preview,
        })
    })
    .await
    .map_err(|e| format!("generate config task: {e}"))??;

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

    Ok(result)
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
pub async fn preview_singbox_config(
    state: State<'_, AppState>,
) -> Result<GenerateConfigResult, String> {
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

    let path = active_config_path(&state.app_data_dir);
    tauri::async_runtime::spawn_blocking(move || {
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
        let preview = serde_json::to_string_pretty(&built.value).unwrap_or_default();
        Ok::<_, String>(GenerateConfigResult {
            path: path.display().to_string(),
            selected_tag: built.selected_tag,
            outbound_count: built.outbound_tags.len(),
            mixed_port: settings.mixed_port,
            api_port: settings.api_port,
            preview,
        })
    })
    .await
    .map_err(|e| format!("preview config task: {e}"))?
}

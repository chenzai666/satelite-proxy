use crate::domain::{ProxyNode, SubscriptionDetail, SubscriptionView};
use crate::services::import::{
    import_from_file, import_from_file_with_id, import_from_url_with_id,
};
use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub subscription: SubscriptionView,
    pub node_count: u32,
    pub skipped_count: u32,
}

#[tauri::command]
pub fn list_subscriptions(state: State<'_, AppState>) -> Result<Vec<SubscriptionView>, String> {
    state
        .with_store(|store| Ok(store.subscriptions.iter().map(|s| s.to_view()).collect()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_subscription(
    state: State<'_, AppState>,
    id: String,
) -> Result<SubscriptionDetail, String> {
    state
        .with_store(|store| {
            store
                .get_subscription(&id)
                .map(|s| s.to_detail())
                .ok_or_else(|| crate::error::AppError::NotFound(id))
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_subscription_url(
    state: State<'_, AppState>,
    name: Option<String>,
    url: String,
    via_proxy: Option<bool>,
    auto_update: Option<bool>,
    auto_update_interval_min: Option<u32>,
) -> Result<ImportResult, String> {
    let via = via_proxy.unwrap_or(false);
    let mixed_port = state
        .with_store(|s| Ok(s.settings.mixed_port))
        .map_err(|e| e.to_string())?;
    let mut outcome = import_from_url_with_id(name, url, None, via, Some(mixed_port))
        .await
        .map_err(|e| e.to_string())?;
    apply_auto_update_prefs(
        &mut outcome.subscription,
        auto_update.unwrap_or(false),
        auto_update_interval_min.unwrap_or(1440),
    );
    persist_import(&state, outcome)
}

#[tauri::command]
pub fn add_subscription_file(
    state: State<'_, AppState>,
    name: Option<String>,
    path: String,
    auto_update: Option<bool>,
    auto_update_interval_min: Option<u32>,
) -> Result<ImportResult, String> {
    let mut outcome = import_from_file(name, &PathBuf::from(&path)).map_err(|e| e.to_string())?;
    apply_auto_update_prefs(
        &mut outcome.subscription,
        auto_update.unwrap_or(false),
        auto_update_interval_min.unwrap_or(1440),
    );
    persist_import(&state, outcome)
}

/// Update existing subscription. Keeps stable id. Re-imports nodes.
#[tauri::command]
pub async fn update_subscription(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    kind: String,
    url: Option<String>,
    path: Option<String>,
    via_proxy: Option<bool>,
    auto_update: Option<bool>,
    auto_update_interval_min: Option<u32>,
) -> Result<ImportResult, String> {
    let existing = state
        .with_store(|store| {
            store
                .get_subscription(&id)
                .cloned()
                .ok_or_else(|| crate::error::AppError::NotFound(id.clone()))
        })
        .map_err(|e| e.to_string())?;

    let display_name = name
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| existing.name.clone());

    let via = via_proxy.unwrap_or(existing.via_proxy);
    let mixed_port = state
        .with_store(|s| Ok(s.settings.mixed_port))
        .map_err(|e| e.to_string())?;

    let kind = kind.to_ascii_lowercase();
    let outcome = match kind.as_str() {
        "url" => {
            let url = url
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .ok_or_else(|| "url is required".to_string())?;
            import_from_url_with_id(
                Some(display_name),
                url,
                Some(id.clone()),
                via,
                Some(mixed_port),
            )
            .await
            .map_err(|e| e.to_string())?
        }
        "file" => {
            let path = path
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .ok_or_else(|| "path is required".to_string())?;
            let mut o = import_from_file_with_id(
                Some(display_name),
                &PathBuf::from(&path),
                Some(id.clone()),
            )
            .map_err(|e| e.to_string())?;
            o.subscription.via_proxy = false;
            o
        }
        _ => return Err("kind must be url or file".into()),
    };

    let mut outcome = outcome;
    outcome.subscription.enabled = existing.enabled;
    outcome.subscription.id = id;
    apply_auto_update_prefs(
        &mut outcome.subscription,
        auto_update.unwrap_or(existing.auto_update),
        auto_update_interval_min.unwrap_or(existing.auto_update_interval_min),
    );

    persist_import(&state, outcome)
}

#[tauri::command]
pub async fn refresh_subscription(
    state: State<'_, AppState>,
    id: String,
    via_proxy: Option<bool>,
) -> Result<ImportResult, String> {
    refresh_subscription_inner(&state, id, via_proxy).await
}

fn apply_auto_update_prefs(
    sub: &mut crate::domain::Subscription,
    auto_update: bool,
    interval_min: u32,
) {
    sub.auto_update = auto_update;
    sub.auto_update_interval_min = interval_min.max(1);
}

/// Internal refresh used by the auto-update scheduler (no Tauri State).
pub async fn refresh_subscription_by_id(
    state: &AppState,
    id: &str,
) -> Result<ImportResult, String> {
    refresh_subscription_inner(state, id.to_string(), None).await
}

async fn refresh_subscription_inner(
    state: &AppState,
    id: String,
    via_proxy: Option<bool>,
) -> Result<ImportResult, String> {
    let existing = state
        .with_store(|store| {
            store
                .get_subscription(&id)
                .cloned()
                .ok_or_else(|| crate::error::AppError::NotFound(id.clone()))
        })
        .map_err(|e| e.to_string())?;

    let via = via_proxy.unwrap_or(existing.via_proxy);
    let mixed_port = state
        .with_store(|s| Ok(s.settings.mixed_port))
        .map_err(|e| e.to_string())?;

    let mut outcome = match &existing.source {
        crate::domain::SubscriptionSource::Url { url } => import_from_url_with_id(
            Some(existing.name.clone()),
            url.clone(),
            Some(id.clone()),
            via,
            Some(mixed_port),
        )
        .await
        .map_err(|e| e.to_string())?,
        crate::domain::SubscriptionSource::File { path } => import_from_file_with_id(
            Some(existing.name.clone()),
            &PathBuf::from(path),
            Some(id.clone()),
        )
        .map_err(|e| e.to_string())?,
    };
    outcome.subscription.enabled = existing.enabled;
    outcome.subscription.id = id;
    apply_auto_update_prefs(
        &mut outcome.subscription,
        existing.auto_update,
        existing.auto_update_interval_min,
    );
    persist_import(state, outcome)
}

#[tauri::command]
pub fn remove_subscription(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .with_store_mut(|store| store.remove_subscription(&id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_subscription_nodes(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<ProxyNode>, String> {
    state
        .with_store(|store| {
            Ok(store
                .nodes
                .iter()
                .filter(|n| n.subscription_id == id)
                .map(|n| n.node.clone())
                .collect())
        })
        .map_err(|e| e.to_string())
}

fn persist_import(
    state: &AppState,
    outcome: crate::services::import::ImportOutcome,
) -> Result<ImportResult, String> {
    let node_count = outcome.subscription.node_count;
    let skipped_count = outcome.subscription.skipped_count;
    let sub_id = outcome.subscription.id.clone();
    let view = state
        .with_store_mut(|store| {
            let mut outcome = outcome;
            let is_new = !store
                .subscriptions
                .iter()
                .any(|s| s.id == outcome.subscription.id);
            if is_new {
                store.prepare_new_subscription_enabled(&mut outcome.subscription);
            }
            store.upsert_subscription(outcome.subscription, outcome.nodes)?;
            store.ensure_current_node_valid();
            let view = store
                .get_subscription(&sub_id)
                .map(|s| s.to_view())
                .ok_or_else(|| crate::error::AppError::NotFound(sub_id.clone()))?;
            Ok(view)
        })
        .map_err(|e| e.to_string())?;
    Ok(ImportResult {
        subscription: view,
        node_count,
        skipped_count,
    })
}

/// Click a config card: exclusive enable (default) or Mix toggle.
#[tauri::command]
pub fn activate_subscription(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<SubscriptionView>, String> {
    state
        .with_store_mut(|store| {
            store.activate_subscription(&id)?;
            Ok(store.subscriptions.iter().map(|s| s.to_view()).collect())
        })
        .map_err(|e| e.to_string())
}

/// Toggle Mix mode (multi-subscription enable). Turning off keeps first enabled only.
#[tauri::command]
pub fn set_mix_mode(
    state: State<'_, AppState>,
    mix: bool,
) -> Result<crate::domain::AppSettings, String> {
    state
        .with_store_mut(|store| {
            store.set_mix_mode(mix)?;
            Ok(store.settings.clone())
        })
        .map_err(|e| e.to_string())
}

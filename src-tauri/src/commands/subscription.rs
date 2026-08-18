use crate::domain::{ProxyNode, SubscriptionDetail, SubscriptionSource, SubscriptionView};
use crate::services::import::{
    canonical_subscription_url, import_from_file, import_from_file_with_id, import_from_url_with_id,
};
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tauri::State;
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub subscription: SubscriptionView,
    pub node_count: u32,
    pub skipped_count: u32,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionUrlEntry {
    pub id: String,
    pub url: String,
}

type SharedRefreshResult = Result<ImportResult, String>;
type RefreshSender = watch::Sender<Option<SharedRefreshResult>>;

static REFRESH_FLIGHTS: OnceLock<Mutex<HashMap<String, RefreshSender>>> = OnceLock::new();

enum RefreshFlight {
    Leader(RefreshLeader),
    Follower(watch::Receiver<Option<SharedRefreshResult>>),
}

struct RefreshLeader {
    id: String,
    sender: RefreshSender,
    finished: bool,
}

impl RefreshLeader {
    fn finish(mut self, result: SharedRefreshResult) -> SharedRefreshResult {
        self.sender.send_replace(Some(result.clone()));
        remove_refresh_flight(&self.id, &self.sender);
        self.finished = true;
        result
    }
}

impl Drop for RefreshLeader {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.sender
            .send_replace(Some(Err("订阅更新任务已取消".into())));
        remove_refresh_flight(&self.id, &self.sender);
    }
}

fn refresh_flights() -> &'static Mutex<HashMap<String, RefreshSender>> {
    REFRESH_FLIGHTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn begin_refresh_flight(id: &str) -> Result<RefreshFlight, String> {
    let mut flights = refresh_flights()
        .lock()
        .map_err(|_| "subscription refresh lock poisoned".to_string())?;
    if let Some(sender) = flights.get(id) {
        return Ok(RefreshFlight::Follower(sender.subscribe()));
    }
    let (sender, _) = watch::channel(None);
    flights.insert(id.to_string(), sender.clone());
    Ok(RefreshFlight::Leader(RefreshLeader {
        id: id.to_string(),
        sender,
        finished: false,
    }))
}

fn remove_refresh_flight(id: &str, sender: &RefreshSender) {
    let Ok(mut flights) = refresh_flights().lock() else {
        return;
    };
    if flights
        .get(id)
        .is_some_and(|current| current.same_channel(sender))
    {
        flights.remove(id);
    }
}

async fn wait_for_refresh(
    mut receiver: watch::Receiver<Option<SharedRefreshResult>>,
) -> SharedRefreshResult {
    loop {
        if let Some(result) = receiver.borrow().clone() {
            return result;
        }
        receiver
            .changed()
            .await
            .map_err(|_| "订阅更新任务已取消".to_string())?;
    }
}

#[tauri::command]
pub fn list_subscriptions(state: State<'_, AppState>) -> Result<Vec<SubscriptionView>, String> {
    state
        .with_store(|store| Ok(store.subscriptions.iter().map(|s| s.to_view()).collect()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_subscription_urls(
    state: State<'_, AppState>,
) -> Result<Vec<SubscriptionUrlEntry>, String> {
    state
        .with_store(|store| {
            Ok(store
                .subscriptions
                .iter()
                .filter_map(|subscription| match &subscription.source {
                    SubscriptionSource::Url { url } => Some(SubscriptionUrlEntry {
                        id: subscription.id.clone(),
                        url: url.clone(),
                    }),
                    SubscriptionSource::File { .. } => None,
                })
                .collect())
        })
        .map_err(|error| error.to_string())
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
    let canonical = canonical_subscription_url(&url);
    let existing_id = state
        .with_store(|store| {
            Ok(store
                .subscriptions
                .iter()
                .find_map(|subscription| match &subscription.source {
                    SubscriptionSource::Url { url: existing_url }
                        if canonical.is_some()
                            && canonical_subscription_url(existing_url) == canonical =>
                    {
                        Some(subscription.id.clone())
                    }
                    _ => None,
                }))
        })
        .map_err(|e| e.to_string())?;
    let mixed_port = state
        .with_store(|s| Ok(s.settings.mixed_port))
        .map_err(|e| e.to_string())?;
    let mut outcome = import_from_url_with_id(name, url, existing_id, via, Some(mixed_port))
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
pub async fn add_subscription_file(
    state: State<'_, AppState>,
    name: Option<String>,
    path: String,
    auto_update: Option<bool>,
    auto_update_interval_min: Option<u32>,
) -> Result<ImportResult, String> {
    let mut outcome = import_file_blocking(name, PathBuf::from(path), None).await?;
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
    let (outcome, replaced_id, replaced_enabled) = match kind.as_str() {
        "url" => {
            let url = url
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .ok_or_else(|| "url is required".to_string())?;
            let duplicate = state
                .with_store(|store| {
                    Ok(store.subscriptions.iter().find_map(|subscription| {
                        if subscription.id == id {
                            return None;
                        }
                        match &subscription.source {
                            SubscriptionSource::Url { url: existing_url }
                                if canonical_subscription_url(existing_url)
                                    == canonical_subscription_url(&url) =>
                            {
                                Some((subscription.id.clone(), subscription.enabled))
                            }
                            _ => None,
                        }
                    }))
                })
                .map_err(|e| e.to_string())?;
            let target_id = duplicate
                .as_ref()
                .map(|(duplicate_id, _)| duplicate_id.clone())
                .unwrap_or_else(|| id.clone());
            let outcome = import_from_url_with_id(
                Some(display_name),
                url,
                Some(target_id),
                via,
                Some(mixed_port),
            )
            .await
            .map_err(|e| e.to_string())?;
            let replaced_enabled = duplicate.as_ref().is_some_and(|(_, enabled)| *enabled);
            (outcome, duplicate.map(|_| id.clone()), replaced_enabled)
        }
        "file" => {
            let path = path
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .ok_or_else(|| "path is required".to_string())?;
            let mut o =
                import_file_blocking(Some(display_name), PathBuf::from(path), Some(id.clone()))
                    .await?;
            o.subscription.via_proxy = false;
            (o, None, false)
        }
        _ => return Err("kind must be url or file".into()),
    };

    let mut outcome = outcome;
    outcome.subscription.enabled = existing.enabled || replaced_enabled;
    apply_auto_update_prefs(
        &mut outcome.subscription,
        auto_update.unwrap_or(existing.auto_update),
        auto_update_interval_min.unwrap_or(existing.auto_update_interval_min),
    );

    persist_import_replacing(&state, outcome, replaced_id.as_deref())
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
    match begin_refresh_flight(&id)? {
        RefreshFlight::Follower(receiver) => wait_for_refresh(receiver).await,
        RefreshFlight::Leader(leader) => {
            let result = refresh_subscription_once(state, id, via_proxy).await;
            leader.finish(result)
        }
    }
}

async fn refresh_subscription_once(
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
        crate::domain::SubscriptionSource::File { path } => {
            import_file_blocking(
                Some(existing.name.clone()),
                PathBuf::from(path),
                Some(id.clone()),
            )
            .await?
        }
    };
    let latest = state
        .with_store(|store| {
            store
                .get_subscription(&id)
                .cloned()
                .ok_or_else(|| crate::error::AppError::NotFound(id.clone()))
        })
        .map_err(|error| error.to_string())?;
    if latest.source != existing.source {
        return Err("订阅地址或文件已在更新期间改变，已丢弃旧结果".into());
    }
    outcome.subscription.name = latest.name;
    outcome.subscription.enabled = latest.enabled;
    outcome.subscription.via_proxy = via_proxy.unwrap_or(latest.via_proxy);
    outcome.subscription.id = id;
    apply_auto_update_prefs(
        &mut outcome.subscription,
        latest.auto_update,
        latest.auto_update_interval_min,
    );
    persist_import(state, outcome)
}

async fn import_file_blocking(
    name: Option<String>,
    path: PathBuf,
    existing_id: Option<String>,
) -> Result<crate::services::import::ImportOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if existing_id.is_some() {
            import_from_file_with_id(name, &path, existing_id)
        } else {
            import_from_file(name, &path)
        }
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("subscription file task: {error}"))?
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
    persist_import_replacing(state, outcome, None)
}

fn persist_import_replacing(
    state: &AppState,
    outcome: crate::services::import::ImportOutcome,
    remove_id: Option<&str>,
) -> Result<ImportResult, String> {
    let node_count = outcome.subscription.node_count;
    let skipped_count = outcome.subscription.skipped_count;
    let sub_id = outcome.subscription.id.clone();
    let view = state
        .with_store_mut(|store| {
            let mut outcome = outcome;
            if let Some(remove_id) = remove_id.filter(|remove_id| *remove_id != sub_id) {
                store
                    .subscriptions
                    .retain(|subscription| subscription.id != remove_id);
                store.nodes.retain(|node| node.subscription_id != remove_id);
            }
            let is_new = !store
                .subscriptions
                .iter()
                .any(|s| s.id == outcome.subscription.id);
            if is_new {
                store.prepare_new_subscription_enabled(&mut outcome.subscription);
            }
            store.upsert_subscription(outcome.subscription, outcome.nodes)?;
            store.ensure_subscription_enable_policy();
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

#[cfg(test)]
mod refresh_flight_tests {
    use super::*;

    fn unique_id(name: &str) -> String {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!(
            "subscription-flight-{name}-{}-{}",
            std::process::id(),
            nonce
        )
    }

    #[tokio::test]
    async fn duplicate_refreshes_share_the_leader_result() {
        let id = unique_id("shared");
        let leader = match begin_refresh_flight(&id).unwrap() {
            RefreshFlight::Leader(leader) => leader,
            RefreshFlight::Follower(_) => panic!("first refresh must lead"),
        };
        let follower = match begin_refresh_flight(&id).unwrap() {
            RefreshFlight::Follower(receiver) => receiver,
            RefreshFlight::Leader(_) => panic!("duplicate refresh must follow"),
        };

        let result = leader.finish(Err("shared result".into()));
        assert_eq!(result.unwrap_err(), "shared result");
        assert_eq!(
            wait_for_refresh(follower).await.unwrap_err(),
            "shared result"
        );
        assert!(matches!(
            begin_refresh_flight(&id).unwrap(),
            RefreshFlight::Leader(_)
        ));
    }

    #[tokio::test]
    async fn cancelled_leader_releases_waiters_and_registry() {
        let id = unique_id("cancelled");
        let leader = match begin_refresh_flight(&id).unwrap() {
            RefreshFlight::Leader(leader) => leader,
            RefreshFlight::Follower(_) => panic!("first refresh must lead"),
        };
        let follower = match begin_refresh_flight(&id).unwrap() {
            RefreshFlight::Follower(receiver) => receiver,
            RefreshFlight::Leader(_) => panic!("duplicate refresh must follow"),
        };

        drop(leader);
        assert!(wait_for_refresh(follower)
            .await
            .unwrap_err()
            .contains("取消"));
        assert!(matches!(
            begin_refresh_flight(&id).unwrap(),
            RefreshFlight::Leader(_)
        ));
    }
}

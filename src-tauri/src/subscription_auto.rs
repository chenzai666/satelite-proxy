//! Periodically refresh subscriptions with `auto_update` enabled.

use crate::commands::subscription::refresh_subscription_by_id;
use crate::state::AppState;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const TICK_SECS: u64 = 60;
const AUTO_UPDATE_CONCURRENCY: usize = 3;

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // First check after a short delay so app finishes setup.
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            run_due_updates(&app).await;
            tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
        }
    });
}

async fn run_due_updates(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let due_ids: Vec<String> = state
        .with_store(|store| {
            Ok(store
                .subscriptions
                .iter()
                .filter(|s| s.is_auto_update_due(now))
                .map(|s| s.id.clone())
                .collect())
        })
        .unwrap_or_default();

    let mut pending = due_ids.into_iter();
    let mut updates = tokio::task::JoinSet::new();
    for _ in 0..AUTO_UPDATE_CONCURRENCY {
        if let Some(id) = pending.next() {
            spawn_auto_update(&mut updates, app.clone(), id);
        }
    }
    while let Some(joined) = updates.join_next().await {
        match joined {
            Ok((id, Ok(result))) => crate::app_log::info(
                "subscription_auto",
                format!("updated {id} ({} nodes)", result.node_count),
            ),
            Ok((id, Err(error))) => {
                crate::app_log::warn("subscription_auto", format!("update {id} failed: {error}"))
            }
            Err(error) => {
                crate::app_log::warn("subscription_auto", format!("update task failed: {error}"))
            }
        }
        if let Some(id) = pending.next() {
            spawn_auto_update(&mut updates, app.clone(), id);
        }
    }
}

fn spawn_auto_update(
    updates: &mut tokio::task::JoinSet<(
        String,
        Result<crate::commands::subscription::ImportResult, String>,
    )>,
    app: AppHandle,
    id: String,
) {
    updates.spawn(async move {
        let result = match app.try_state::<AppState>() {
            Some(state) => refresh_subscription_by_id(&state, &id).await,
            None => Err("app state unavailable".into()),
        };
        (id, result)
    });
}

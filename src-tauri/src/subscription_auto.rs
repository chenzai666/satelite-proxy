//! Periodically refresh subscriptions with `auto_update` enabled.

use crate::commands::subscription::refresh_subscription_by_id;
use crate::state::AppState;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const TICK_SECS: u64 = 60;

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // First check after a short delay so app finishes setup.
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            if let Some(state) = app.try_state::<AppState>() {
                run_due_updates(&state).await;
            }
            tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
        }
    });
}

async fn run_due_updates(state: &AppState) {
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

    for id in due_ids {
        match refresh_subscription_by_id(state, &id).await {
            Ok(r) => {
                eprintln!("[satelite] auto-update ok: {} ({} nodes)", id, r.node_count);
            }
            Err(e) => {
                eprintln!("[satelite] auto-update failed {id}: {e}");
            }
        }
    }
}

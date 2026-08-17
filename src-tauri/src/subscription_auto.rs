//! Periodically refresh subscriptions with `auto_update` enabled.

use crate::commands::subscription::refresh_subscription_by_id;
use crate::state::AppState;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

const TICK_SECS: u64 = 60;
const AUTO_UPDATE_CONCURRENCY: usize = 3;

#[derive(Default)]
struct RetryState {
    failures: u32,
    retry_at: i64,
    subscription_last_update: i64,
}

impl RetryState {
    fn record_failure(&mut self, now: i64, subscription_last_update: i64) -> u64 {
        self.failures = self.failures.saturating_add(1);
        let delay = retry_delay_secs(self.failures);
        self.retry_at = now.saturating_add(delay as i64);
        self.subscription_last_update = subscription_last_update;
        delay
    }
}

static RETRIES: LazyLock<Mutex<HashMap<String, RetryState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
    let now = now_secs();

    let (mut due, active_ids): (Vec<(String, i64)>, HashSet<String>) = state
        .with_store(|store| {
            let active_ids = store
                .subscriptions
                .iter()
                .filter(|subscription| subscription.auto_update)
                .map(|s| s.id.clone())
                .collect();
            let due = store
                .subscriptions
                .iter()
                .filter(|subscription| subscription.is_auto_update_due(now))
                .map(|subscription| (subscription.id.clone(), subscription.last_update))
                .collect();
            Ok((due, active_ids))
        })
        .unwrap_or_default();
    {
        let mut retries = RETRIES.lock().unwrap_or_else(|p| p.into_inner());
        retries.retain(|id, _| active_ids.contains(id));
        due.retain(|(id, last_update)| {
            retries.get(id).is_none_or(|retry| {
                retry.subscription_last_update != *last_update || retry.retry_at <= now
            })
        });
    }

    let mut pending = due.into_iter();
    let mut updates = tokio::task::JoinSet::new();
    for _ in 0..AUTO_UPDATE_CONCURRENCY {
        if let Some((id, last_update)) = pending.next() {
            spawn_auto_update(&mut updates, app.clone(), id, last_update);
        }
    }
    while let Some(joined) = updates.join_next().await {
        match joined {
            Ok((id, _, Ok(result))) => {
                RETRIES
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                crate::app_log::info(
                    "subscription_auto",
                    format!("updated {id} ({} nodes)", result.node_count),
                );
            }
            Ok((id, last_update, Err(error))) => {
                let delay = RETRIES
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .entry(id.clone())
                    .or_default()
                    .record_failure(now_secs(), last_update);
                crate::app_log::warn(
                    "subscription_auto",
                    format!(
                        "update {id} failed: {error}; retry in {} minutes",
                        delay / 60
                    ),
                );
            }
            Err(error) => {
                crate::app_log::warn("subscription_auto", format!("update task failed: {error}"))
            }
        }
        if let Some((id, last_update)) = pending.next() {
            spawn_auto_update(&mut updates, app.clone(), id, last_update);
        }
    }
}

fn retry_delay_secs(failures: u32) -> u64 {
    match failures {
        0 | 1 => 5 * 60,
        2 => 15 * 60,
        3 => 30 * 60,
        _ => 60 * 60,
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn spawn_auto_update(
    updates: &mut tokio::task::JoinSet<(
        String,
        i64,
        Result<crate::commands::subscription::ImportResult, String>,
    )>,
    app: AppHandle,
    id: String,
    last_update: i64,
) {
    updates.spawn(async move {
        let result = match app.try_state::<AppState>() {
            Some(state) => refresh_subscription_by_id(&app, &state, &id).await,
            None => Err("app state unavailable".into()),
        };
        (id, last_update, result)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_updates_back_off_up_to_one_hour() {
        let mut retry = RetryState::default();
        let delays: Vec<_> = (0..5)
            .map(|index| retry.record_failure(1_000 + index, 42))
            .collect();
        assert_eq!(delays, vec![300, 900, 1800, 3600, 3600]);
        assert_eq!(retry.failures, 5);
        assert_eq!(retry.retry_at, 4_604);
        assert_eq!(retry.subscription_last_update, 42);
    }
}

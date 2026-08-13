//! Async apply-and-restart for rule-set enable/disable toggles.
//!
//! Persisting a toggle is synchronous and fast; restarting sing-box so the
//! change actually takes routing effect is not (stop + start + health-check,
//! several seconds). This module runs that restart in the background and
//! reports progress/result via the `rule-set-apply-status` event, so the
//! calling command can return to the frontend immediately.
//!
//! Rapid repeated toggles of the *same* rule set are coalesced: only one
//! restart worker runs per id at a time. If the target value changes again
//! while a restart is in flight, the worker loops once more after finishing,
//! rather than spawning a second overlapping restart.

use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

const EVENT: &str = "rule-set-apply-status";

#[derive(Clone, Serialize)]
struct ApplyStatusEvent {
    id: String,
    enabled: bool,
    status: &'static str,
    error: Option<String>,
}

fn emit(app: &AppHandle, id: &str, enabled: bool, status: &'static str, error: Option<String>) {
    let _ = app.emit(
        EVENT,
        ApplyStatusEvent {
            id: id.to_string(),
            enabled,
            status,
            error,
        },
    );
}

/// Record the latest requested value for `id` and, if no restart worker is
/// already handling it, spawn one. Called right after the store write
/// succeeds; never blocks.
pub fn request_apply(app: AppHandle, id: String, enabled: bool) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let had_pending = {
        let mut pending = state.lock_pending_rule_set_toggle();
        pending.insert(id.clone(), enabled).is_some()
    };
    if !had_pending {
        spawn_worker(app, id);
    }
}

fn spawn_worker(app: AppHandle, id: String) {
    tauri::async_runtime::spawn_blocking(move || loop {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let target = match state.lock_pending_rule_set_toggle().get(&id) {
            Some(v) => *v,
            None => return, // shouldn't happen: we only remove after this round completes
        };

        emit(&app, &id, target, "restarting", None);

        let resource_dir = app.path().resource_dir().ok();
        let result = state.restart_if_running(resource_dir.as_deref());

        // Converge: drop the pending entry only if no newer request arrived
        // while this restart was running. Otherwise loop once more with the
        // latest target instead of leaving it stale.
        let still_current = {
            let mut pending = state.lock_pending_rule_set_toggle();
            if pending.get(&id) == Some(&target) {
                pending.remove(&id);
                true
            } else {
                false
            }
        };

        match result {
            Ok(_) => emit(&app, &id, target, "ready", None),
            Err(e) => emit(
                &app,
                &id,
                target,
                "error",
                Some(format!("已保存，但重启内核失败: {e}")),
            ),
        }

        if still_current {
            break;
        }
    });
}

use crate::app_log::{self, LogBatch, LogLevel};
use crate::state::AppState;
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub async fn list_app_logs(
    min_level: Option<String>,
    limit: Option<usize>,
    query: Option<String>,
    after_id: Option<u64>,
) -> Result<LogBatch, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let level = min_level
            .as_deref()
            .and_then(LogLevel::parse)
            .unwrap_or(LogLevel::Info);
        let limit = limit.unwrap_or(500).clamp(1, 2_000);
        Ok(app_log::list(level, limit, query.as_deref(), after_id))
    })
    .await
    .map_err(|e| format!("list logs task: {e}"))?
}

#[tauri::command]
pub fn clear_app_logs() -> Result<(), String> {
    app_log::clear();
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct CoreLogTail {
    /// Absolute path of the log file (current or last core session).
    pub path: Option<String>,
    pub lines: Vec<String>,
}

/// Tail of the active core's hourly log file. Xray has no per-connection
/// API, so the Xray-mode traffic page streams the core log instead — at
/// `info` level Xray logs accepted connections and routing decisions there.
#[tauri::command]
pub fn get_core_log_tail(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<CoreLogTail, String> {
    let limit = limit.unwrap_or(300).clamp(1, 1_000);
    let tail = state.lock_runtime().core.core_log_tail(limit);
    Ok(match tail {
        Some((path, lines)) => CoreLogTail {
            path: Some(path.display().to_string()),
            lines,
        },
        None => CoreLogTail {
            path: None,
            lines: Vec::new(),
        },
    })
}

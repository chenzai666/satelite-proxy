use crate::app_log::{self, LogBatch, LogLevel};

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

use crate::app_log::{self, LogEntry, LogLevel};

#[tauri::command]
pub fn list_app_logs(
    min_level: Option<String>,
    limit: Option<usize>,
    query: Option<String>,
) -> Result<Vec<LogEntry>, String> {
    let level = min_level
        .as_deref()
        .and_then(LogLevel::parse)
        .unwrap_or(LogLevel::Info);
    let limit = limit.unwrap_or(500).clamp(1, 2_000);
    Ok(app_log::list(level, limit, query.as_deref()))
}

#[tauri::command]
pub fn clear_app_logs() -> Result<(), String> {
    app_log::clear();
    Ok(())
}

use crate::runtime::ConnectionView;
use crate::state::AppState;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub async fn list_connections(app: AppHandle) -> Result<Vec<ConnectionView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| "app state unavailable".to_string())?;
        Ok(state.live_connection_views())
    })
    .await
    .map_err(|e| format!("list connections task: {e}"))?
}

#[tauri::command]
pub async fn list_requests(
    app: AppHandle,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ConnectionView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| "app state unavailable".to_string())?;
        Ok(state.request_views(query.as_deref(), limit, false))
    })
    .await
    .map_err(|e| format!("list requests task: {e}"))?
}

#[tauri::command]
pub async fn list_request_failures(
    app: AppHandle,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ConnectionView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app
            .try_state::<AppState>()
            .ok_or_else(|| "app state unavailable".to_string())?;
        Ok(state.request_views(query.as_deref(), limit, true))
    })
    .await
    .map_err(|e| format!("list request failures task: {e}"))?
}

#[tauri::command]
pub fn clear_request_history(state: State<'_, AppState>) -> Result<(), String> {
    state
        .clear_request_history_nonblocking()
        .map_err(|error| error.to_string())
}

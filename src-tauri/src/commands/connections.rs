use crate::runtime::ConnectionView;
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn list_connections(state: State<'_, AppState>) -> Result<Vec<ConnectionView>, String> {
    Ok(state.live_connection_views())
}

#[tauri::command]
pub fn list_requests(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ConnectionView>, String> {
    Ok(state.request_views(query.as_deref(), limit, false))
}

#[tauri::command]
pub fn list_request_failures(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ConnectionView>, String> {
    Ok(state.request_views(query.as_deref(), limit, true))
}

#[tauri::command]
pub fn clear_request_history(state: State<'_, AppState>) -> Result<(), String> {
    state
        .clear_request_history_nonblocking()
        .map_err(|error| error.to_string())
}

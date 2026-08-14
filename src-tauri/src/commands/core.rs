use crate::core::{
    active_core_version, bundled_core_version, detect_platform, download_latest_core_with_progress,
    fetch_latest_release_with_proxy, inspect_core_bin, CoreDownloadResult, CoreSource,
};
use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

const CORE_DOWNLOAD_EVENT: &str = "core-download-progress";

#[derive(Debug, Serialize)]
pub struct CoreInfo {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub platform: String,
    /// Filled only when check_update=true (network). Otherwise null for instant UI.
    pub latest_version: Option<String>,
    pub update_available: bool,
    /// `bundled` | `downloaded` | `missing`
    pub source: String,
    pub bundled_version: Option<String>,
}

/// Local core status only (no network). Prefer this for page load.
#[tauri::command]
pub fn get_core_info(app: AppHandle, state: State<'_, AppState>) -> Result<CoreInfo, String> {
    let platform = detect_platform().map_err(|e| e.to_string())?;
    let resource_dir = app.path().resource_dir().ok();
    let res = resource_dir.as_deref();

    let (path, source) = inspect_core_bin(&state.app_data_dir, res);
    // Metadata-only inspection: do not stage/copy the bundled core during page load.
    let version = active_core_version(&state.app_data_dir, res);
    let bundled_version = bundled_core_version(res);

    Ok(CoreInfo {
        installed: path.is_some(),
        version,
        path: path.map(|p| p.display().to_string()),
        platform: platform.asset_suffix.to_string(),
        latest_version: None,
        update_available: false,
        source: match source {
            CoreSource::Bundled => "bundled".into(),
            CoreSource::Downloaded => "downloaded".into(),
            CoreSource::Missing => "missing".into(),
        },
        bundled_version,
    })
}

/// Remote latest version only (network). Call after local info is shown.
#[tauri::command]
pub async fn check_core_update(
    state: State<'_, AppState>,
    local_version: Option<String>,
) -> Result<CoreUpdateInfo, String> {
    let proxy_url = current_download_proxy(&state)?;
    let latest = fetch_latest_release_with_proxy(proxy_url.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    let update_available = match &local_version {
        Some(local) => normalize_cmp(local) != normalize_cmp(&latest.version),
        None => true,
    };
    Ok(CoreUpdateInfo {
        latest_version: latest.version,
        update_available,
        asset_name: latest.asset_name,
        size: latest.size,
    })
}

#[derive(Debug, Serialize)]
pub struct CoreUpdateInfo {
    pub latest_version: String,
    pub update_available: bool,
    pub asset_name: String,
    pub size: u64,
}

#[tauri::command]
pub async fn download_core(
    app: AppHandle,
    state: State<'_, AppState>,
    tag: Option<String>,
) -> Result<CoreDownloadResult, String> {
    let proxy_url = current_download_proxy(&state)?;
    let progress_app = app.clone();
    download_latest_core_with_progress(&state.app_data_dir, tag, proxy_url, move |progress| {
        let _ = progress_app.emit(CORE_DOWNLOAD_EVENT, progress);
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn fetch_core_latest(
    state: State<'_, AppState>,
) -> Result<crate::core::LatestReleaseInfo, String> {
    let proxy_url = current_download_proxy(&state)?;
    fetch_latest_release_with_proxy(proxy_url.as_deref())
        .await
        .map_err(|e| e.to_string())
}

fn current_download_proxy(state: &AppState) -> Result<Option<String>, String> {
    if !state.is_core_running() {
        return Ok(None);
    }
    let mixed_port = state
        .with_store(|store| Ok(store.settings.mixed_port))
        .map_err(|error| error.to_string())?;
    Ok(Some(format!("http://127.0.0.1:{mixed_port}")))
}

fn normalize_cmp(v: &str) -> String {
    v.trim().trim_start_matches('v').to_string()
}

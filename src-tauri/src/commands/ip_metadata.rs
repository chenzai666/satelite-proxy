use crate::services::{
    ip_metadata::{self, IpMetadata},
    network_probe,
};
use crate::state::AppState;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::State;

type Cache = Vec<(String, Instant, IpMetadata)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[tauri::command]
pub async fn check_ip_metadata(
    state: State<'_, AppState>,
    ip: String,
) -> Result<IpMetadata, String> {
    let ip = ip_metadata::validate_ip(&ip)?;
    let key = ip.to_string();
    if state.is_core_transitioning() {
        return Err("内核正在切换，请稍后检测".into());
    }
    let (version, before) = state
        .probe_snapshot()
        .map_err(|_| "无法获取稳定的检测路径")?;
    let route = network_probe::route_for_status(&before)?;
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let cached = cache
        .lock()
        .map_err(|_| "IP 信息缓存不可用")?
        .iter()
        .find(|(address, time, _)| address == &key && time.elapsed() < Duration::from_secs(300))
        .map(|(_, _, value)| value.clone());
    if let Some(value) = cached {
        let (after_version, after) = state.probe_snapshot().map_err(|_| "检测路径正在变化")?;
        if version != after_version || !network_probe::same_session(&before, &after) {
            return Err("检测期间代理路径已变化，请重新检测".into());
        }
        return Ok(value);
    }
    let _permit = GATE
        .try_acquire()
        .map_err(|_| "IP 信息正在检测，请稍后重试")?;
    let value = ip_metadata::query(ip, route).await?;
    let (after_version, after) = state.probe_snapshot().map_err(|_| "无法确认检测路径")?;
    if version != after_version
        || state.is_core_transitioning()
        || !network_probe::same_session(&before, &after)
    {
        return Err("检测期间代理路径已变化，请重新检测".into());
    }
    let mut entries = cache.lock().map_err(|_| "IP 信息缓存不可用")?;
    entries
        .retain(|(address, time, _)| address != &key && time.elapsed() < Duration::from_secs(300));
    if entries.len() >= 64 {
        entries.remove(0);
    }
    entries.push((key, Instant::now(), value.clone()));
    Ok(value)
}

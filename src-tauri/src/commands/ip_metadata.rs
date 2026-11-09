use crate::services::{
    ip_metadata::{self, IpMetadata},
    network_probe,
};
use crate::state::AppState;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::State;

type Cache = Vec<(String, Instant, IpMetadata)>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
static LOOKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn cache_ttl(value: &IpMetadata) -> Duration {
    Duration::from_secs(if matches!(value.network_type, "unknown" | "isp") {
        30
    } else {
        300
    })
}

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
        .find(|(address, time, value)| address == &key && time.elapsed() < cache_ttl(value))
        .map(|(_, _, value)| value.clone());
    if let Some(value) = cached {
        let (after_version, after) = state.probe_snapshot().map_err(|_| "检测路径正在变化")?;
        if version != after_version || !network_probe::same_session(&before, &after) {
            return Err("检测期间代理路径已变化，请重新检测".into());
        }
        return Ok(value);
    }
    // 切换路线的旧请求会在下方取消，短暂等待许可而非立即返回“忙碌”。
    let lookup = LOOKUP_SEQUENCE
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    let _permit = tokio::time::timeout(Duration::from_millis(500), GATE.acquire())
        .await
        .map_err(|_| "IP 信息正在检测，请稍后重试")?
        .map_err(|_| "IP 信息正在检测，请稍后重试")?;
    if LOOKUP_SEQUENCE.load(Ordering::Acquire) != lookup {
        return Err("出口信息已更新，取消旧查询".into());
    }
    let value = tokio::select! {
        result = ip_metadata::query(ip, route) => result?,
        _ = async {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if LOOKUP_SEQUENCE.load(Ordering::Acquire) != lookup
                    || state.is_core_transitioning() || state.probe_snapshot().is_ok_and(|(current, _)| current != version) {
                    break;
                }
            }
        } => return Err("检测期间代理路径已变化，请重新检测".into()),
    };
    let (after_version, after) = state.probe_snapshot().map_err(|_| "无法确认检测路径")?;
    if LOOKUP_SEQUENCE.load(Ordering::Acquire) != lookup
        || version != after_version
        || state.is_core_transitioning()
        || !network_probe::same_session(&before, &after)
    {
        return Err("检测期间代理路径已变化，请重新检测".into());
    }
    let mut entries = cache.lock().map_err(|_| "IP 信息缓存不可用")?;
    entries.retain(|(address, time, value)| address != &key && time.elapsed() < cache_ttl(value));
    if entries.len() >= 64 {
        entries.remove(0);
    }
    entries.push((key, Instant::now(), value.clone()));
    Ok(value)
}

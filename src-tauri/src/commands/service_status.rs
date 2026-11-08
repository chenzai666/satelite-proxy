//! 服务状态检测命令：单批互斥，并拒绝内核切换期间或路线已过期的结果。

use crate::services::network_probe::{route_for_status, same_session};
use crate::services::service_status::{probe_services, ServiceStatusBatch};
use crate::state::AppState;
use tauri::State;
use tokio::sync::Semaphore;

static BATCH_GATE: Semaphore = Semaphore::const_new(1);
const TRANSITION_ERROR: &str = "内核正在切换，请稍后检测服务状态";
const STALE_ERROR: &str = "代理状态或检测路线已变化，服务检测结果已过期，请重新检测";

#[tauri::command]
pub async fn check_service_status(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<ServiceStatusBatch, String> {
    if state.is_core_transitioning() {
        return Err(TRANSITION_ERROR.into());
    }
    // 不排队：重复点击立即拒绝，许可覆盖开始快照、请求和最终校验。
    let _permit = BATCH_GATE
        .try_acquire()
        .map_err(|_| "服务状态检测正在进行，请等待本批完成".to_string())?;
    let (version, before) = state
        .probe_snapshot()
        .map_err(|_| "无法读取当前代理状态".to_string())?;
    if state.is_core_transitioning() {
        return Err(TRANSITION_ERROR.into());
    }
    let route = route_for_status(&before)
        .map_err(|_| "当前配置没有可供检测使用的 HTTP/mixed 入站".to_string())?;
    let batch = probe_services(&ids, route).await?;
    if state.is_core_transitioning() {
        return Err(STALE_ERROR.into());
    }
    let (after_version, after) = state
        .probe_snapshot()
        .map_err(|_| STALE_ERROR.to_string())?;
    let after_route = route_for_status(&after).map_err(|_| STALE_ERROR.to_string())?;
    if version != after_version
        || state.is_core_transitioning()
        || !same_session(&before, &after)
        || route != after_route
    {
        return Err(STALE_ERROR.into());
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_gate_rejects_duplicates_and_releases_on_scope_exit() {
        {
            let _permit = BATCH_GATE.try_acquire().unwrap();
            assert!(BATCH_GATE.try_acquire().is_err());
        }
        assert!(BATCH_GATE.try_acquire().is_ok());
    }
}

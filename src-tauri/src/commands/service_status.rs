//! 服务状态检测命令：单批互斥，并拒绝内核切换期间或路线已过期的结果。

use crate::runtime::ProxyStatus;
use crate::services::network_probe::{route_for_status, same_session};
use crate::services::service_status::{probe_services, ServiceStatusBatch};
use crate::state::AppState;
use std::time::Duration;
use tauri::State;
use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::timeout;

static BATCH_GATE: Semaphore = Semaphore::const_new(1);
// 只允许一个排队者，避免会话刷新积累大量待执行批次。
static WAIT_GATE: Semaphore = Semaphore::const_new(1);
const GATE_WAIT: Duration = Duration::from_secs(8);
const BUSY_ERROR: &str = "服务状态检测正在进行，请等待本批完成";

async fn acquire_batch<'a>(
    gate: &'a Semaphore,
    waiters: &Semaphore,
    wait: Duration,
) -> Result<SemaphorePermit<'a>, String> {
    if let Ok(permit) = gate.try_acquire() {
        return Ok(permit);
    }
    let _waiting = waiters.try_acquire().map_err(|_| BUSY_ERROR.to_string())?;
    timeout(wait, gate.acquire())
        .await
        .map_err(|_| BUSY_ERROR.to_string())?
        .map_err(|_| BUSY_ERROR.to_string())
}

const TRANSITION_ERROR: &str = "内核正在切换，请稍后检测服务状态";
const STALE_ERROR: &str = "代理状态或检测路线已变化，服务检测结果已过期，请重新检测";

fn same_snapshot(before: &(u64, ProxyStatus), after: &(u64, ProxyStatus)) -> bool {
    before.0 == after.0 && same_session(&before.1, &after.1)
}

#[tauri::command]
pub async fn check_service_status(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<ServiceStatusBatch, String> {
    if state.is_core_transitioning() {
        return Err(TRANSITION_ERROR.into());
    }
    // probe_snapshot 内的 runtime/store try-lock 在返回时已释放；等待期间只持值。
    let queued = state
        .probe_snapshot()
        .map_err(|_| "无法读取当前代理状态，请稍后重试".to_string())?;
    let _permit = acquire_batch(&BATCH_GATE, &WAIT_GATE, GATE_WAIT).await?;
    // 必须重新采集真实快照，禁止沿用等待前的 route。
    let fresh = state
        .probe_snapshot()
        .map_err(|_| STALE_ERROR.to_string())?;
    if state.is_core_transitioning() || !same_snapshot(&queued, &fresh) {
        return Err(STALE_ERROR.into());
    }
    let (version, before) = fresh;
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
    fn snapshot_check_rejects_session_changes_and_epoch_aba() {
        let status = crate::runtime::Runtime::new().status(&crate::storage::AppStore::default());
        let before = (2, status.clone());
        assert!(same_snapshot(&before, &(2, status.clone())));
        assert!(!same_snapshot(&before, &(4, status.clone())));
        let mut changed = status;
        changed.core_started_at = Some(123);
        assert!(!same_snapshot(&before, &(2, changed)));
    }

    #[tokio::test]
    async fn waits_for_old_batch_and_bounds_queue() {
        let gate = &BATCH_GATE;
        let waiters = &WAIT_GATE;
        let old = gate.acquire().await.unwrap();
        let next = acquire_batch(&gate, &waiters, GATE_WAIT);
        tokio::pin!(next);
        // 实际轮询等待 future，使其进入队列。
        assert!(timeout(Duration::from_millis(10), &mut next).await.is_err());
        assert_eq!(waiters.available_permits(), 0);
        assert_eq!(
            acquire_batch(&gate, &waiters, GATE_WAIT).await.unwrap_err(),
            BUSY_ERROR
        );
        drop(old);
        let permit = timeout(Duration::from_secs(1), &mut next)
            .await
            .unwrap()
            .unwrap();
        assert!(gate.try_acquire().is_err());
        assert_eq!(waiters.available_permits(), 1);
        drop(permit);
        assert!(gate.try_acquire().is_ok());
    }

    #[tokio::test]
    async fn wait_timeout_and_cancellation_release_queue_slot() {
        let gate = Semaphore::new(1);
        let waiters = Semaphore::new(1);
        let old = gate.acquire().await.unwrap();
        assert_eq!(
            acquire_batch(&gate, &waiters, Duration::from_millis(10))
                .await
                .unwrap_err(),
            BUSY_ERROR
        );
        assert_eq!(waiters.available_permits(), 1);
        assert!(timeout(
            Duration::from_millis(10),
            acquire_batch(&gate, &waiters, GATE_WAIT)
        )
        .await
        .is_err());
        assert_eq!(waiters.available_permits(), 1);
        drop(old);
        assert!(acquire_batch(&gate, &waiters, GATE_WAIT).await.is_ok());
    }
}

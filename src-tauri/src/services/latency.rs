//! Latency probe helpers.
//!
//! - **UI 测速** (`test_nodes_latency`): always direct TCP to `server:port` (no proxy).
//! - **Smart switch**: may pass clash API for through-outbound delay when core is up.
//!
//! Clash path uses **unified delay** (like mihomo / FlClash): probe twice and
//! report the second RTT so handshake / cold-connect bias is reduced.

use crate::api::ClashApi;
use crate::config::outbound_tag;
use crate::domain::ProxyNode;
use crate::error::AppResult;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::time::timeout;

const DEFAULT_TIMEOUT_MS: u64 = 5000;
const DEFAULT_CONCURRENCY: usize = 30;
const GLOBAL_CONCURRENCY: usize = 30;
const CACHE_TTL: Duration = Duration::from_secs(90);
const FAILURE_CACHE_TTL: Duration = Duration::from_secs(15);

static GLOBAL_SEMAPHORE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(GLOBAL_CONCURRENCY)));
static PROBE_CACHE: LazyLock<Mutex<HashMap<String, (Instant, LatencyResult)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static PROBE_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Serialize)]
pub struct LatencyResult {
    pub id: String,
    pub name: String,
    /// None means timeout / unreachable
    pub latency_ms: Option<u32>,
    pub error: Option<String>,
    pub tested_at: i64,
    /// `clash_api` | `tcp`
    pub method: String,
}

pub async fn probe_nodes(
    nodes: &[ProxyNode],
    timeout_ms: Option<u64>,
    concurrency: Option<usize>,
    clash: Option<ClashApi>,
    probe_url: String,
) -> AppResult<Vec<LatencyResult>> {
    let timeout_ms = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let concurrency = concurrency.unwrap_or(DEFAULT_CONCURRENCY).max(1);
    let batch_sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(nodes.len());

    for node in nodes {
        let id = node.id.clone();
        let name = node.name.clone();
        let server = node.server.clone();
        let port = node.port;
        let tag = outbound_tag(node);
        let batch_sem = Arc::clone(&batch_sem);
        let clash = clash.clone();
        let probe_url = probe_url.clone();
        handles.push(tokio::spawn(async move {
            let _batch_permit = batch_sem.acquire().await.expect("batch semaphore");
            let key = if let Some(api) = &clash {
                format!(
                    "clash|{}|{}|{id}|{tag}|{probe_url}|{timeout_ms}",
                    api.base, api.secret
                )
            } else {
                format!("tcp|{id}|{server}|{port}|{timeout_ms}")
            };
            probe_coalesced(key, move || async move {
                if let Some(api) = clash {
                    probe_clash(api, id, name, tag, probe_url, timeout_ms).await
                } else {
                    probe_tcp(id, name, &server, port, timeout_ms).await
                }
            })
            .await
        }));
    }

    let mut results = Vec::with_capacity(handles.len());
    for h in handles {
        match h.await {
            Ok(r) => results.push(r),
            Err(e) => results.push(LatencyResult {
                id: String::new(),
                name: String::new(),
                latency_ms: None,
                error: Some(format!("join error: {e}")),
                tested_at: now_secs(),
                method: "error".into(),
            }),
        }
    }
    Ok(results)
}

async fn probe_coalesced<F, Fut>(key: String, probe: F) -> LatencyResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = LatencyResult>,
{
    if let Some(result) = cached_result(&key) {
        return result;
    }

    let probe_lock = {
        let mut map = PROBE_LOCKS.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(
            map.entry(key.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    };
    let _key_guard = probe_lock.lock().await;
    if let Some(result) = cached_result(&key) {
        return result;
    }

    let _global_permit = Arc::clone(&GLOBAL_SEMAPHORE)
        .acquire_owned()
        .await
        .expect("global probe semaphore");
    let result = probe().await;
    {
        let mut cache = PROBE_CACHE.lock().unwrap_or_else(|p| p.into_inner());
        cache.retain(|_, (at, result)| at.elapsed() < cache_ttl(result));
        cache.insert(key.clone(), (Instant::now(), result.clone()));
    }
    let mut locks = PROBE_LOCKS.lock().unwrap_or_else(|p| p.into_inner());
    if locks
        .get(&key)
        .map(|current| Arc::ptr_eq(current, &probe_lock))
        .unwrap_or(false)
    {
        locks.remove(&key);
    }
    result
}

fn cached_result(key: &str) -> Option<LatencyResult> {
    let mut cache = PROBE_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    match cache.get(key) {
        Some((at, result)) if at.elapsed() < cache_ttl(result) => Some(result.clone()),
        Some(_) => {
            cache.remove(key);
            None
        }
        None => None,
    }
}

fn cache_ttl(result: &LatencyResult) -> Duration {
    if result.latency_ms.is_some() {
        CACHE_TTL
    } else {
        FAILURE_CACHE_TTL
    }
}

async fn probe_clash(
    api: ClashApi,
    id: String,
    name: String,
    tag: String,
    probe_url: String,
    timeout_ms: u64,
) -> LatencyResult {
    let tested_at = now_secs();
    // Unified delay: two sequential URL tests; prefer the second (warm path).
    // Mirrors mihomo `unified-delay` / FlClash default.
    let result = tokio::task::spawn_blocking(move || {
        let first = api.delay(&tag, &probe_url, timeout_ms);
        let second = api.delay(&tag, &probe_url, timeout_ms);
        match (first, second) {
            (_, Ok(ms2)) => Ok(ms2),
            (Ok(ms1), Err(_)) => Ok(ms1),
            (Err(e1), Err(e2)) => Err(format!("{e1}; retry: {e2}")),
        }
    })
    .await;

    match result {
        Ok(Ok(ms)) => LatencyResult {
            id,
            name,
            latency_ms: Some(ms),
            error: None,
            tested_at,
            method: "clash_api".into(),
        },
        Ok(Err(e)) => LatencyResult {
            id,
            name,
            latency_ms: None,
            error: Some(e),
            tested_at,
            method: "clash_api".into(),
        },
        Err(e) => LatencyResult {
            id,
            name,
            latency_ms: None,
            error: Some(format!("join: {e}")),
            tested_at,
            method: "clash_api".into(),
        },
    }
}

async fn probe_tcp(
    id: String,
    name: String,
    server: &str,
    port: u16,
    timeout_ms: u64,
) -> LatencyResult {
    let tested_at = now_secs();
    let addr = format!("{server}:{port}");
    let start = Instant::now();

    match timeout(
        Duration::from_millis(timeout_ms),
        TcpStream::connect(addr.as_str()),
    )
    .await
    {
        Ok(Ok(_stream)) => {
            let ms = start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
            LatencyResult {
                id,
                name,
                latency_ms: Some(ms),
                error: None,
                tested_at,
                method: "tcp".into(),
            }
        }
        Ok(Err(e)) => LatencyResult {
            id,
            name,
            latency_ms: None,
            error: Some(e.to_string()),
            tested_at,
            method: "tcp".into(),
        },
        Err(_) => LatencyResult {
            id,
            name,
            latency_ms: None,
            error: Some("timeout".into()),
            tested_at,
            method: "tcp".into(),
        },
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn result(ms: Option<u32>) -> LatencyResult {
        LatencyResult {
            id: "test-node".into(),
            name: "test".into(),
            latency_ms: ms,
            error: ms.is_none().then(|| "failed".into()),
            tested_at: now_secs(),
            method: "test".into(),
        }
    }

    fn unique_key(label: &str) -> String {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        format!("test|{label}|{}", NEXT.fetch_add(1, Ordering::Relaxed))
    }

    #[tokio::test]
    async fn identical_in_flight_probes_are_coalesced_and_cached() {
        let key = unique_key("coalesce");
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let key = key.clone();
            let calls = Arc::clone(&calls);
            tasks.push(tokio::spawn(async move {
                probe_coalesced(key, || async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    result(Some(42))
                })
                .await
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap().latency_ms, Some(42));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let cached = probe_coalesced(key, || async {
            panic!("fresh successful result must be reused");
        })
        .await;
        assert_eq!(cached.latency_ms, Some(42));
    }

    #[tokio::test]
    async fn global_probe_concurrency_never_exceeds_thirty() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for i in 0..45 {
            let active = Arc::clone(&active);
            let peak = Arc::clone(&peak);
            tasks.push(tokio::spawn(async move {
                probe_coalesced(unique_key(&format!("global-{i}")), || async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(15)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    result(Some(10))
                })
                .await
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= GLOBAL_CONCURRENCY);
    }

    #[test]
    fn failures_use_shorter_cache_ttl() {
        assert_eq!(cache_ttl(&result(Some(1))), CACHE_TTL);
        assert_eq!(cache_ttl(&result(None)), FAILURE_CACHE_TTL);
    }
}

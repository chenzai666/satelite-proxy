//! Latency probe helpers.
//!
//! - **UI 测速** (`test_nodes_latency`): direct TCP to `server:port` for
//!   TCP-based protocols. UDP-only protocols (hysteria/hysteria2/tuic) never
//!   accept a plain TCP connect, so they're routed through the clash delay
//!   API instead when the core is running; otherwise they report timeout by
//!   design rather than a misleading raw-reachability failure.
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
const MAX_CACHE_ENTRIES: usize = 4096;
const CACHE_TRIM_TO: usize = 3072;

static GLOBAL_SEMAPHORE: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(GLOBAL_CONCURRENCY)));
struct ProbeCache {
    entries: HashMap<String, (Instant, LatencyResult)>,
    last_prune: Instant,
}

static PROBE_CACHE: LazyLock<Mutex<ProbeCache>> = LazyLock::new(|| {
    Mutex::new(ProbeCache {
        entries: HashMap::new(),
        last_prune: Instant::now(),
    })
});
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
    let mut pending = nodes.iter().cloned().enumerate();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..concurrency.min(nodes.len()) {
        if let Some((index, node)) = pending.next() {
            spawn_probe_task(
                &mut tasks,
                index,
                node,
                timeout_ms,
                clash.clone(),
                probe_url.clone(),
            );
        }
    }

    let mut indexed_results = Vec::with_capacity(nodes.len());
    let mut task_errors = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(result) => indexed_results.push(result),
            Err(e) => task_errors.push(LatencyResult {
                id: String::new(),
                name: String::new(),
                latency_ms: None,
                error: Some(format!("join error: {e}")),
                tested_at: now_secs(),
                method: "error".into(),
            }),
        }
        if let Some((index, node)) = pending.next() {
            spawn_probe_task(
                &mut tasks,
                index,
                node,
                timeout_ms,
                clash.clone(),
                probe_url.clone(),
            );
        }
    }
    indexed_results.sort_unstable_by_key(|(index, _)| *index);
    let mut results: Vec<_> = indexed_results
        .into_iter()
        .map(|(_, result)| result)
        .collect();
    results.append(&mut task_errors);
    Ok(results)
}

fn spawn_probe_task(
    tasks: &mut tokio::task::JoinSet<(usize, LatencyResult)>,
    index: usize,
    node: ProxyNode,
    timeout_ms: u64,
    clash: Option<ClashApi>,
    probe_url: String,
) {
    tasks.spawn(async move {
        let id = node.id.clone();
        let name = node.name.clone();
        let server = node.server.clone();
        let port = node.port;
        let tag = outbound_tag(&node);
        // UDP-only protocols (hysteria/hysteria2/tuic) never accept a plain
        // TCP connect, so a direct-TCP probe always times out regardless of
        // node health. Route those through the clash delay API instead when
        // the core is up. Without the core there's no way to speak the
        // protocol at all, so report that explicitly instead of running a
        // TCP probe that can only ever time out and looks like a dead node.
        let use_clash = node.protocol.is_udp_only() && clash.is_some();
        if node.protocol.is_udp_only() && clash.is_none() {
            return (
                index,
                LatencyResult {
                    id,
                    name,
                    latency_ms: None,
                    error: Some("core not running: start the proxy to test this protocol".into()),
                    tested_at: now_secs(),
                    method: "unsupported".into(),
                },
            );
        }
        let key = if use_clash {
            let api = clash.as_ref().expect("checked by use_clash");
            format!(
                "clash|{}|{}|{id}|{tag}|{probe_url}|{timeout_ms}",
                api.base, api.secret
            )
        } else {
            format!("tcp|{id}|{server}|{port}|{timeout_ms}")
        };
        let result = probe_coalesced(key, move || async move {
            if use_clash {
                probe_clash(
                    clash.expect("checked by use_clash"),
                    id,
                    name,
                    tag,
                    probe_url,
                    timeout_ms,
                )
                .await
            } else {
                probe_tcp(id, name, &server, port, timeout_ms).await
            }
        })
        .await;
        (index, result)
    });
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
    cache_result(key.clone(), result.clone());
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
    match cache.entries.get(key) {
        Some((at, result)) if at.elapsed() < cache_ttl(result) => Some(result.clone()),
        Some(_) => {
            cache.entries.remove(key);
            None
        }
        None => None,
    }
}

fn cache_result(key: String, result: LatencyResult) {
    let mut cache = PROBE_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    if cache.last_prune.elapsed() >= FAILURE_CACHE_TTL
        || (cache.entries.len() >= MAX_CACHE_ENTRIES && !cache.entries.contains_key(&key))
    {
        cache
            .entries
            .retain(|_, (at, cached)| at.elapsed() < cache_ttl(cached));
        cache.last_prune = now;
    }
    if cache.entries.len() >= MAX_CACHE_ENTRIES && !cache.entries.contains_key(&key) {
        let remove_count = cache.entries.len().saturating_sub(CACHE_TRIM_TO) + 1;
        let mut oldest: Vec<_> = cache
            .entries
            .iter()
            .map(|(entry_key, (at, _))| (entry_key.clone(), *at))
            .collect();
        oldest.sort_unstable_by_key(|(_, at)| *at);
        for (entry_key, _) in oldest.into_iter().take(remove_count) {
            cache.entries.remove(&entry_key);
        }
    }
    cache.entries.insert(key, (now, result));
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

    fn node(protocol: crate::domain::Protocol) -> ProxyNode {
        use crate::domain::ProtocolConfig;
        ProxyNode {
            id: unique_key("node"),
            name: "test-node".into(),
            protocol,
            server: "127.0.0.1".into(),
            // Nothing listens here; TCP connect fails fast (connection refused)
            // instead of waiting out the timeout.
            port: 1,
            tls: None,
            transport: None,
            udp: None,
            config: ProtocolConfig::Hysteria2 {
                password: "x".into(),
                up_mbps: None,
                down_mbps: None,
                obfs: None,
                obfs_password: None,
            },
            source: None,
            latency_ms: None,
            latency_at: None,
        }
    }

    // Hysteria2/Hysteria/Tuic are QUIC-only: a plain TCP connect to their port
    // always fails regardless of node health, so probe_nodes must route them
    // through the clash delay API (when available) instead of TCP. This is
    // the behavior the UI "测速" bug report depended on — without it, every
    // hy2 node reports a spurious timeout even when the node is reachable.
    #[tokio::test]
    async fn udp_only_protocols_use_clash_api_when_available_not_tcp() {
        use crate::domain::Protocol;

        // Nothing is listening on this port; ClashApi::delay will fail, but
        // the point under test is *which* probe path ran, recorded in
        // LatencyResult::method regardless of success.
        let clash = crate::api::ClashApi::new("127.0.0.1", 1, "secret");

        for protocol in [Protocol::Hysteria2, Protocol::Hysteria, Protocol::Tuic] {
            let nodes = vec![node(protocol)];
            let results = probe_nodes(
                &nodes,
                Some(200),
                Some(1),
                Some(clash.clone()),
                String::new(),
            )
            .await
            .unwrap();
            assert_eq!(
                results[0].method, "clash_api",
                "{protocol:?} must probe via clash_api, not raw TCP"
            );
        }

        // A TCP-based protocol must keep using the direct-TCP probe even
        // when a clash API is available (unchanged prior behavior).
        let nodes = vec![node(Protocol::Shadowsocks)];
        let results = probe_nodes(&nodes, Some(200), Some(1), Some(clash), String::new())
            .await
            .unwrap();
        assert_eq!(results[0].method, "tcp");
    }

    // Without a running core there is no way to speak QUIC-only protocols at
    // all — a raw TCP probe would always time out and look like a dead node,
    // so probe_nodes must report "unsupported" explicitly instead of running
    // a doomed TCP probe (the bug this behavior fixes: hy2 nodes always
    // showing timeout even when perfectly reachable).
    #[tokio::test]
    async fn udp_only_protocols_report_unsupported_without_clash_api() {
        use crate::domain::Protocol;
        let nodes = vec![node(Protocol::Hysteria2)];
        let results = probe_nodes(&nodes, Some(200), Some(1), None, String::new())
            .await
            .unwrap();
        assert_eq!(results[0].method, "unsupported");
        assert!(results[0].latency_ms.is_none());
        assert!(results[0].error.is_some());
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

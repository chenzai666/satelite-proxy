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
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio::time::timeout;

const DEFAULT_TIMEOUT_MS: u64 = 5000;
const DEFAULT_CONCURRENCY: usize = 30;

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
    let sem = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::with_capacity(nodes.len());

    for node in nodes {
        let id = node.id.clone();
        let name = node.name.clone();
        let server = node.server.clone();
        let port = node.port;
        let tag = outbound_tag(node);
        let sem = Arc::clone(&sem);
        let clash = clash.clone();
        let probe_url = probe_url.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore");
            if let Some(api) = clash {
                probe_clash(api, id, name, tag, probe_url, timeout_ms).await
            } else {
                probe_tcp(id, name, &server, port, timeout_ms).await
            }
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

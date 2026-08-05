use crate::services::latency::{probe_nodes, LatencyResult};
use crate::state::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Serialize)]
pub struct LatencyBatchResult {
    pub results: Vec<LatencyResult>,
    pub tested: usize,
    pub ok: usize,
    pub failed: usize,
    /// `clash_api` or `tcp`
    pub method: String,
}

/// Test latency via **direct TCP connect** to each node (never via proxy / clash delay API).
///
/// Previously used clash_api `/proxies/{tag}/delay` when the core was running, which measures
/// through the outbound and is skewed by system proxy / current route. UI “测速” should
/// report raw reachability to the node address instead.
#[tauri::command]
pub async fn test_nodes_latency(
    state: State<'_, AppState>,
    ids: Option<Vec<String>>,
    timeout_ms: Option<u64>,
) -> Result<LatencyBatchResult, String> {
    let nodes = state
        .with_store(|store| {
            let all = store.enabled_nodes();
            let filtered = if let Some(ids) = &ids {
                let set: std::collections::HashSet<_> = ids.iter().cloned().collect();
                all.into_iter().filter(|n| set.contains(&n.id)).collect()
            } else {
                all
            };
            Ok(filtered)
        })
        .map_err(|e| e.to_string())?;

    if nodes.is_empty() {
        return Ok(LatencyBatchResult {
            results: vec![],
            tested: 0,
            ok: 0,
            failed: 0,
            method: "none".into(),
        });
    }

    // Always TCP — do not pass clash API even when core is running.
    let results = probe_nodes(&nodes, timeout_ms, Some(30), None, String::new())
        .await
        .map_err(|e| e.to_string())?;

    state
        .with_store_mut(|store| {
            for r in &results {
                if r.id.is_empty() {
                    continue;
                }
                store.update_node_latency(&r.id, r.latency_ms, r.tested_at);
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;

    let ok = results.iter().filter(|r| r.latency_ms.is_some()).count();
    let failed = results.len() - ok;
    Ok(LatencyBatchResult {
        tested: results.len(),
        ok,
        failed,
        results,
        method: "tcp".into(),
    })
}

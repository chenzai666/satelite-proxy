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

/// Test latency: clash_api delay when core running, else TCP connect.
#[tauri::command]
pub async fn test_nodes_latency(
    state: State<'_, AppState>,
    ids: Option<Vec<String>>,
    timeout_ms: Option<u64>,
) -> Result<LatencyBatchResult, String> {
    let (nodes, probe_url) = state
        .with_store(|store| {
            let all = store.enabled_nodes();
            let filtered = if let Some(ids) = &ids {
                let set: std::collections::HashSet<_> = ids.iter().cloned().collect();
                all.into_iter().filter(|n| set.contains(&n.id)).collect()
            } else {
                all
            };
            Ok((filtered, store.settings.probe_url.clone()))
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

    let clash = {
        let r = state.lock_runtime();
        r.clash_api_clone()
    };

    // clash_api path uses unified delay (two probes, report second).
    let method = if clash.is_some() {
        "clash_api_unified"
    } else {
        "tcp"
    };

    let results = probe_nodes(&nodes, timeout_ms, Some(30), clash, probe_url)
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
        method: method.into(),
    })
}

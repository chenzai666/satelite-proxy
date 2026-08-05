//! Smart node auto-switch (docs/auto.md).
//!
//! Architecture: passive connection journal → degradation check → on-demand
//! active URL probe of top-K candidates → hysteretic switch with cooldown.
//! Uses Selector + Clash API; does not scan the full node list continuously.
//!
//! Lock rule: never hold `store` while acquiring `runtime` (see AppState).

use crate::app_log;
use crate::config::{outbound_tag, smart_pool_nodes};
use crate::domain::{ProxyNode, Rule, RuleTarget};
use crate::services::latency::probe_nodes;
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const TICK: Duration = Duration::from_secs(20);
const MIN_DWELL: Duration = Duration::from_secs(120);
const COOLDOWN: Duration = Duration::from_secs(90);
const PROBE_TIMEOUT_MS: u64 = 2500;
const TOP_K: usize = 4;
const MIN_IMPROVEMENT_MS: u32 = 100;
const MIN_IMPROVEMENT_RATIO: f64 = 0.25;
const PASSIVE_LOOKBACK_MS: i64 = 30_000;
const PASSIVE_MIN_SAMPLES: u32 = 8;
const PASSIVE_FAIL_RATE: f64 = 0.25;
const CONSECUTIVE_PROBE_FAILS: u32 = 2;
/// On enable: probe this many candidates first (auto.md Level 2–3).
const BOOTSTRAP_BATCH: usize = 8;
const BOOTSTRAP_MAX: usize = 24;
const BOOTSTRAP_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Serialize)]
pub struct SmartSwitchNowResult {
    pub switched: bool,
    pub from_id: Option<String>,
    pub to_id: Option<String>,
    pub to_name: Option<String>,
    pub latency_ms: Option<u32>,
    pub probed: u32,
    pub message: String,
}

#[derive(Debug, Default)]
struct Controller {
    last_switch: Option<Instant>,
    consecutive_probe_fails: u32,
    /// node_id → eject until
    ejected: HashMap<String, Instant>,
    eject_counts: HashMap<String, u32>,
}

impl Controller {
    fn in_dwell(&self) -> bool {
        self.last_switch
            .map(|t| t.elapsed() < MIN_DWELL)
            .unwrap_or(false)
    }

    fn in_cooldown(&self) -> bool {
        self.last_switch
            .map(|t| t.elapsed() < MIN_DWELL + COOLDOWN)
            .unwrap_or(false)
    }

    fn eject(&mut self, id: &str) {
        let n = self.eject_counts.entry(id.to_string()).or_insert(0);
        *n = n.saturating_add(1);
        let secs = match *n {
            1 => 30,
            2 => 120,
            3 => 600,
            _ => 1800,
        };
        self.ejected
            .insert(id.to_string(), Instant::now() + Duration::from_secs(secs));
    }

    fn clear_eject_if_expired(&mut self) {
        let now = Instant::now();
        self.ejected.retain(|_, until| *until > now);
    }
}

static CTRL: LazyLock<Mutex<Controller>> =
    LazyLock::new(|| Mutex::new(Controller::default()));

fn ctrl() -> std::sync::MutexGuard<'static, Controller> {
    CTRL.lock().unwrap_or_else(|p| p.into_inner())
}

/// Per smart-rule selector: last switch time (reuse dwell/cooldown).
static RULE_LAST: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            if let Some(state) = app.try_state::<AppState>() {
                if let Err(e) = tick(&state).await {
                    app_log::warn("smart_switch", format!("tick: {e}"));
                }
                if let Err(e) = tick_smart_rules(&state).await {
                    app_log::warn("smart_switch", format!("smart_rules: {e}"));
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// User just enabled smart switch: probe candidates and pick the best node once.
/// Bypasses passive trigger / hysteresis (still respects circuit-breaker ejection).
pub async fn select_best_now(state: &AppState) -> Result<SmartSwitchNowResult, String> {
    app_log::info("smart_switch", "bootstrap probe started");

    if !state.is_core_running() {
        app_log::warn("smart_switch", "bootstrap skipped: core not running");
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: None,
            to_id: None,
            to_name: None,
            latency_ms: None,
            probed: 0,
            message: "core not running".into(),
        });
    }

    {
        let mut c = ctrl();
        c.clear_eject_if_expired();
    }

    // Separate locks — never nest store under runtime or reverse.
    let (current_id, mut nodes, probe_url) = {
        let store = state.lock_store();
        (
            store.settings.current_node_id.clone(),
            store.enabled_nodes(),
            store.settings.probe_url.clone(),
        )
    };
    let clash = {
        let rt = state.lock_runtime();
        rt.clash_api_clone()
    };

    if nodes.is_empty() {
        app_log::warn("smart_switch", "bootstrap: no nodes");
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: current_id,
            to_id: None,
            to_name: None,
            latency_ms: None,
            probed: 0,
            message: "no nodes".into(),
        });
    }

    let Some(api) = clash else {
        app_log::warn("smart_switch", "bootstrap: clash api unavailable");
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: current_id,
            to_id: None,
            to_name: None,
            latency_ms: None,
            probed: 0,
            message: "clash api unavailable".into(),
        });
    };

    let ejected: Vec<String> = {
        let c = ctrl();
        c.ejected
            .iter()
            .filter(|(_, until)| Instant::now() < **until)
            .map(|(id, _)| id.clone())
            .collect()
    };

    nodes.retain(|n| !ejected.iter().any(|e| e == &n.id));
    nodes.sort_by(|a, b| {
        let la = a.latency_ms.unwrap_or(u32::MAX / 4);
        let lb = b.latency_ms.unwrap_or(u32::MAX / 4);
        la.cmp(&lb).then_with(|| a.name.cmp(&b.name))
    });

    let mut probed: u32 = 0;
    let mut best: Option<(String, String, u32)> = None; // id, name, ms
    let limit = nodes.len().min(BOOTSTRAP_MAX);
    let pool = &nodes[..limit];

    app_log::debug(
        "smart_switch",
        format!("bootstrap pool size={limit} (max {BOOTSTRAP_MAX})"),
    );

    for (batch_idx, batch) in pool.chunks(BOOTSTRAP_BATCH).enumerate() {
        // User may disable smart switch mid-probe; stop without applying a switch.
        let still_on = state
            .with_store(|s| Ok(s.settings.smart_switch))
            .unwrap_or(false);
        if !still_on {
            app_log::info("smart_switch", "bootstrap cancelled (smart_switch off)");
            return Ok(SmartSwitchNowResult {
                switched: false,
                from_id: current_id,
                to_id: None,
                to_name: None,
                latency_ms: None,
                probed,
                message: "cancelled".into(),
            });
        }

        let results = probe_nodes(
            batch,
            Some(PROBE_TIMEOUT_MS),
            Some(BOOTSTRAP_CONCURRENCY),
            Some(api.clone()),
            probe_url.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        probed = probed.saturating_add(results.len() as u32);

        let _ = state.with_store_mut(|store| {
            for r in &results {
                if !r.id.is_empty() {
                    store.update_node_latency(&r.id, r.latency_ms, r.tested_at);
                }
            }
            Ok(())
        });

        for r in results {
            if let Some(ms) = r.latency_ms {
                let better = best.as_ref().map(|(_, _, b)| ms < *b).unwrap_or(true);
                if better {
                    best = Some((r.id, r.name, ms));
                }
            }
        }

        app_log::trace(
            "smart_switch",
            format!(
                "bootstrap batch {} done, probed={}, best={}",
                batch_idx + 1,
                probed,
                best.as_ref()
                    .map(|(id, _, ms)| format!("{id}:{ms}ms"))
                    .unwrap_or_else(|| "none".into())
            ),
        );

        if best.is_some() && batch_idx >= 1 {
            break;
        }
    }

    let still_on = state
        .with_store(|s| Ok(s.settings.smart_switch))
        .unwrap_or(false);
    if !still_on {
        app_log::info("smart_switch", "bootstrap cancelled before apply");
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: current_id,
            to_id: None,
            to_name: None,
            latency_ms: None,
            probed,
            message: "cancelled".into(),
        });
    }

    let Some((best_id, best_name, best_ms)) = best else {
        app_log::warn(
            "smart_switch",
            format!("bootstrap: all probes failed (probed={probed})"),
        );
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: current_id,
            to_id: None,
            to_name: None,
            latency_ms: None,
            probed,
            message: "all probes failed".into(),
        });
    };

    if current_id.as_ref() == Some(&best_id) {
        {
            let mut c = ctrl();
            c.last_switch = Some(Instant::now());
            c.consecutive_probe_fails = 0;
        }
        app_log::info(
            "smart_switch",
            format!("bootstrap: already best {best_name} ({best_ms}ms)"),
        );
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: current_id,
            to_id: Some(best_id),
            to_name: Some(best_name),
            latency_ms: Some(best_ms),
            probed,
            message: "already best".into(),
        });
    }

    apply_switch(state, &best_id, false)?;
    {
        let mut c = ctrl();
        c.last_switch = Some(Instant::now());
        c.consecutive_probe_fails = 0;
    }

    app_log::info(
        "smart_switch",
        format!(
            "bootstrap: {} → {} ({}ms, probed={})",
            current_id.as_deref().unwrap_or("—"),
            best_name,
            best_ms,
            probed
        ),
    );

    Ok(SmartSwitchNowResult {
        switched: true,
        from_id: current_id,
        to_id: Some(best_id),
        to_name: Some(best_name),
        latency_ms: Some(best_ms),
        probed,
        message: "switched".into(),
    })
}

/// Hot-select first; only then persist current_node_id (avoids half-applied state).
fn apply_switch(state: &AppState, best_id: &str, hard_fail: bool) -> Result<(), String> {
    let (tag, name) = {
        let store = state.lock_store();
        let node = store
            .find_node(best_id)
            .ok_or_else(|| format!("node {best_id} missing"))?;
        (outbound_tag(node), node.name.clone())
    };

    let close_conns = state
        .with_store(|s| Ok(s.settings.close_connections_on_switch))
        .unwrap_or(true);

    {
        let runtime = state.lock_runtime();
        if let Err(e) = runtime.select_node_live(&tag) {
            app_log::error(
                "smart_switch",
                format!("select_node_live failed for {name} ({tag}): {e}"),
            );
            return Err(e.to_string());
        }
        if close_conns && hard_fail {
            if let Some(api) = runtime.clash_api_clone() {
                let _ = api.close_all_connections();
            }
        }
    }

    state
        .with_store_mut(|store| {
            store.settings.current_node_id = Some(best_id.to_string());
            Ok(())
        })
        .map_err(|e| e.to_string())?;

    app_log::debug(
        "smart_switch",
        format!("applied switch → {name} (hard_fail={hard_fail})"),
    );
    Ok(())
}

async fn tick(state: &AppState) -> Result<(), String> {
    let enabled = state
        .with_store(|s| Ok(s.settings.smart_switch))
        .unwrap_or(false);
    if !enabled || !state.is_core_running() {
        return Ok(());
    }

    {
        let mut c = ctrl();
        c.clear_eject_if_expired();
        if c.in_dwell() {
            return Ok(());
        }
    }

    let (current_id, nodes, probe_url) = {
        let store = state.lock_store();
        (
            store.settings.current_node_id.clone(),
            store.enabled_nodes(),
            store.settings.probe_url.clone(),
        )
    };
    let clash = {
        let rt = state.lock_runtime();
        rt.clash_api_clone()
    };

    let Some(current_id) = current_id else {
        return Ok(());
    };
    let Some(current) = nodes.iter().find(|n| n.id == current_id).cloned() else {
        return Ok(());
    };
    let current_tag = outbound_tag(&current);

    // —— Level 0: passive observation (connection journal) ——
    let (sus, total) = {
        let rt = state.lock_runtime();
        rt.passive_close_stats(&current_tag, PASSIVE_LOOKBACK_MS)
    };
    let passive_bad =
        total >= PASSIVE_MIN_SAMPLES && (sus as f64 / total as f64) >= PASSIVE_FAIL_RATE;

    let follow_up = {
        let c = ctrl();
        c.consecutive_probe_fails > 0
    };
    if !passive_bad && !follow_up {
        return Ok(());
    }

    app_log::debug(
        "smart_switch",
        format!(
            "degrade signal: passive_bad={passive_bad} sus={sus}/{total} follow_up={follow_up}"
        ),
    );

    // —— Level 1: active probe confirms current node ——
    let Some(api) = clash.clone() else {
        return Ok(());
    };
    let cur_results = probe_nodes(
        &[current.clone()],
        Some(PROBE_TIMEOUT_MS),
        Some(1),
        Some(api.clone()),
        probe_url.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let cur_ms = cur_results.first().and_then(|r| r.latency_ms);
    let cur_fail = cur_ms.is_none();

    {
        let mut c = ctrl();
        if cur_fail {
            c.consecutive_probe_fails = c.consecutive_probe_fails.saturating_add(1);
        } else if !passive_bad {
            c.consecutive_probe_fails = 0;
            if let Some(ms) = cur_ms {
                drop(c);
                let _ = state.with_store_mut(|store| {
                    store.update_node_latency(&current_id, Some(ms), now_secs());
                    Ok(())
                });
                let peers: Vec<u32> = nodes
                    .iter()
                    .filter(|n| n.id != current_id)
                    .filter_map(|n| n.latency_ms)
                    .collect();
                if peers.len() >= 2 {
                    let mut sorted = peers;
                    sorted.sort_unstable();
                    let median = sorted[sorted.len() / 2];
                    if ms <= median.saturating_mul(2).saturating_add(150) {
                        return Ok(());
                    }
                } else {
                    return Ok(());
                }
            }
        }
    }

    let hard_fail = {
        let c = ctrl();
        cur_fail && c.consecutive_probe_fails >= CONSECUTIVE_PROBE_FAILS
    };
    let soft_degrade = !cur_fail && (passive_bad || cur_ms.is_some());
    if !hard_fail && !soft_degrade && !passive_bad {
        let c = ctrl();
        if c.consecutive_probe_fails < CONSECUTIVE_PROBE_FAILS && !passive_bad {
            return Ok(());
        }
    }

    {
        let c = ctrl();
        if !passive_bad && c.consecutive_probe_fails < CONSECUTIVE_PROBE_FAILS && cur_fail {
            return Ok(());
        }
        if c.in_cooldown() && !hard_fail {
            return Ok(());
        }
    }

    // —— Level 2: probe top-K candidates ——
    let ejected: Vec<String> = {
        let c = ctrl();
        c.ejected
            .iter()
            .filter(|(_, until)| Instant::now() < **until)
            .map(|(id, _)| id.clone())
            .collect()
    };

    let mut candidates: Vec<ProxyNode> = nodes
        .into_iter()
        .filter(|n| n.id != current_id)
        .filter(|n| !ejected.iter().any(|e| e == &n.id))
        .collect();
    candidates.sort_by(|a, b| {
        let la = a.latency_ms.unwrap_or(u32::MAX / 4);
        let lb = b.latency_ms.unwrap_or(u32::MAX / 4);
        la.cmp(&lb).then_with(|| a.name.cmp(&b.name))
    });
    candidates.truncate(TOP_K);
    if candidates.is_empty() {
        if cur_fail {
            let mut c = ctrl();
            c.eject(&current_id);
        }
        return Ok(());
    }

    let cand_results = probe_nodes(
        &candidates,
        Some(PROBE_TIMEOUT_MS),
        Some(3),
        Some(api),
        probe_url,
    )
    .await
    .map_err(|e| e.to_string())?;

    let _ = state.with_store_mut(|store| {
        for r in &cand_results {
            if !r.id.is_empty() {
                store.update_node_latency(&r.id, r.latency_ms, r.tested_at);
            }
        }
        if let Some(ms) = cur_ms {
            store.update_node_latency(&current_id, Some(ms), now_secs());
        }
        Ok(())
    });

    let mut ok_cands: Vec<(String, u32)> = cand_results
        .into_iter()
        .filter_map(|r| r.latency_ms.map(|ms| (r.id, ms)))
        .collect();
    ok_cands.sort_by_key(|(_, ms)| *ms);

    if ok_cands.is_empty() {
        app_log::warn(
            "smart_switch",
            "all candidates failed (possible local network issue)",
        );
        if cur_fail {
            let mut c = ctrl();
            c.eject(&current_id);
        }
        return Ok(());
    }

    let (best_id, best_ms) = ok_cands[0].clone();

    let should_switch = if hard_fail || (cur_fail && passive_bad) {
        true
    } else if let Some(cms) = cur_ms {
        let better_ratio = (best_ms as f64) <= (cms as f64) * (1.0 - MIN_IMPROVEMENT_RATIO);
        let better_abs = cms.saturating_sub(best_ms) >= MIN_IMPROVEMENT_MS;
        better_ratio || better_abs
    } else {
        true
    };

    if !should_switch {
        return Ok(());
    }

    let best_name = {
        let store = state.lock_store();
        store
            .find_node(&best_id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| best_id.clone())
    };

    apply_switch(state, &best_id, hard_fail)?;

    {
        let mut c = ctrl();
        c.last_switch = Some(Instant::now());
        c.consecutive_probe_fails = 0;
        if cur_fail {
            c.eject(&current_id);
        }
    }

    app_log::info(
        "smart_switch",
        format!(
            "{} → {} ({}ms{})",
            current.name,
            best_name,
            best_ms,
            if hard_fail { ", hard fail" } else { "" }
        ),
    );
    Ok(())
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn collect_enabled_smart_rules(state: &AppState) -> Vec<Rule> {
    state
        .with_store(|store| {
            let mut out = Vec::new();
            for set in store.rule_sets.iter().filter(|s| s.enabled) {
                for r in set.rules.iter().filter(|r| {
                    r.enabled && matches!(r.target, RuleTarget::Smart)
                }) {
                    out.push(r.clone());
                }
            }
            Ok(out)
        })
        .unwrap_or_default()
}

/// Maintain keyword-filtered smart rule selectors (independent of global smart_switch toggle).
async fn tick_smart_rules(state: &AppState) -> Result<(), String> {
    if !state.is_core_running() {
        return Ok(());
    }
    let rules = collect_enabled_smart_rules(state);
    if rules.is_empty() {
        return Ok(());
    }

    let (nodes, probe_url) = {
        let store = state.lock_store();
        (store.enabled_nodes(), store.settings.probe_url.clone())
    };
    let clash = {
        let rt = state.lock_runtime();
        rt.clash_api_clone()
    };
    let Some(api) = clash else {
        return Ok(());
    };

    for rule in rules {
        if let Err(e) = maintain_smart_rule(state, &rule, &nodes, &probe_url, api.clone()).await {
            app_log::debug(
                "smart_switch",
                format!("smart rule {}: {e}", rule.id),
            );
        }
    }
    Ok(())
}

async fn maintain_smart_rule(
    state: &AppState,
    rule: &Rule,
    nodes: &[ProxyNode],
    probe_url: &str,
    api: crate::api::ClashApi,
) -> Result<(), String> {
    let group = rule.smart_outbound_tag();
    {
        let map = RULE_LAST.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(t) = map.get(&rule.id) {
            if t.elapsed() < MIN_DWELL + COOLDOWN {
                return Ok(());
            }
        }
    }

    let mut pool = smart_pool_nodes(rule, nodes);
    if pool.is_empty() {
        return Ok(());
    }
    pool.sort_by(|a, b| {
        let la = a.latency_ms.unwrap_or(u32::MAX / 4);
        let lb = b.latency_ms.unwrap_or(u32::MAX / 4);
        la.cmp(&lb).then_with(|| a.name.cmp(&b.name))
    });
    pool.truncate(BOOTSTRAP_MAX.min(TOP_K.max(8)));

    let results = probe_nodes(
        &pool,
        Some(PROBE_TIMEOUT_MS),
        Some(BOOTSTRAP_CONCURRENCY),
        Some(api),
        probe_url.to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;

    let _ = state.with_store_mut(|store| {
        for r in &results {
            if !r.id.is_empty() {
                store.update_node_latency(&r.id, r.latency_ms, r.tested_at);
            }
        }
        Ok(())
    });

    let mut ok: Vec<(String, String, u32)> = results
        .into_iter()
        .filter_map(|r| {
            let ms = r.latency_ms?;
            Some((r.id, r.name, ms))
        })
        .collect();
    ok.sort_by_key(|(_, _, ms)| *ms);
    let Some((best_id, best_name, best_ms)) = ok.into_iter().next() else {
        return Ok(());
    };

    let tag = {
        let store = state.lock_store();
        store
            .find_node(&best_id)
            .map(outbound_tag)
            .ok_or_else(|| format!("node {best_id} missing"))?
    };

    {
        let runtime = state.lock_runtime();
        runtime
            .select_group_live(&group, &tag)
            .map_err(|e| e.to_string())?;
    }

    {
        let mut map = RULE_LAST.lock().unwrap_or_else(|p| p.into_inner());
        map.insert(rule.id.clone(), Instant::now());
    }

    app_log::info(
        "smart_switch",
        format!(
            "smart rule {} → {} ({}ms, group={})",
            rule.payload, best_name, best_ms, group
        ),
    );
    Ok(())
}

/// Immediate probe for one smart rule (e.g. after save). Best-effort.
pub async fn refresh_smart_rule_now(state: &AppState, rule: &Rule) -> Result<(), String> {
    if !matches!(rule.target, RuleTarget::Smart) || !rule.enabled {
        return Ok(());
    }
    if !state.is_core_running() {
        return Ok(());
    }
    let (nodes, probe_url) = {
        let store = state.lock_store();
        (store.enabled_nodes(), store.settings.probe_url.clone())
    };
    let api = {
        let rt = state.lock_runtime();
        rt.clash_api_clone()
    };
    let Some(api) = api else {
        return Ok(());
    };
    // Bypass dwell so new rules get a pick quickly.
    {
        let mut map = RULE_LAST.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&rule.id);
    }
    maintain_smart_rule(state, rule, &nodes, &probe_url, api).await
}

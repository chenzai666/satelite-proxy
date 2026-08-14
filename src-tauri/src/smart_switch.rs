//! Smart node auto-switch (docs/auto.md).
//!
//! Architecture:
//!   passive connection journal → degradation / healthy re-probe
//!   → on-demand active URL probe of top-K candidates
//!   → light score + Clash-style tolerance + dwell / cooldown / eject
//!
//! Not a pure Clash `url-test` (no continuous full-list interval).
//! Healthy periods still get a low-frequency re-probe for drift correction.
//!
//! Lock rule: never hold `store` while acquiring `runtime` (see AppState).

use crate::app_log;
use crate::config::{outbound_tag, smart_pool_nodes};
use crate::domain::{ProxyNode, Rule, RuleSetStrategy, RuleTarget};
use crate::services::latency::probe_nodes;
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

// —— Schedule ——
const TICK: Duration = Duration::from_secs(20);
/// After a switch, refuse further switches for this long.
const MIN_DWELL: Duration = Duration::from_secs(120);
/// After dwell, soft switches wait this extra window (hard fail may skip).
const COOLDOWN: Duration = Duration::from_secs(90);
/// When healthy, re-probe current + top-K at most this often (url-test-like drift fix).
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(600);
/// Smart-rule selectors use the same low-frequency refresh cadence.
const RULE_PROBE_INTERVAL: Duration = Duration::from_secs(600);
const RULE_FAILURE_RETRY_BASE: Duration = Duration::from_secs(60);

// —— Active probe ——
const PROBE_TIMEOUT_MS: u64 = 2500;
const TOP_K: usize = 4;
const CANDIDATE_CONCURRENCY: usize = 3;
const BOOTSTRAP_BATCH: usize = 8;
const BOOTSTRAP_MAX: usize = 24;
const BOOTSTRAP_CONCURRENCY: usize = 4;

// —— Hysteresis (Clash url-test `tolerance` style) ——
/// Only switch when `best + TOLERANCE_MS < current`.
const TOLERANCE_MS: u32 = 50;
/// Secondary: large relative improvement also qualifies if abs ≥ TOLERANCE_MS.
const MIN_IMPROVEMENT_RATIO: f64 = 0.25;

// —— Passive (connection journal) ——
const PASSIVE_LOOKBACK_MS: i64 = 30_000;
const PASSIVE_MIN_SAMPLES: u32 = 8;
const PASSIVE_FAIL_RATE: f64 = 0.25;
const CONSECUTIVE_PROBE_FAILS: u32 = 2;

// —— Score weights (lower is better) ——
const SCORE_FAIL_PENALTY: f64 = 200.0;
const SCORE_EJECT_PENALTY: f64 = 5_000.0;
const SCORE_UNKNOWN_LATENCY: f64 = 8_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No degrade signal; optional periodic health probe.
    Ok,
    /// Passive journal looks bad; awaiting / running confirm probe.
    Suspect,
    /// Active probe in progress (logical marker for logs).
    Probing,
    /// Post-switch dwell / soft cooldown.
    Cooldown,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Suspect => "suspect",
            Self::Probing => "probing",
            Self::Cooldown => "cooldown",
        }
    }
}

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

#[derive(Debug)]
struct Controller {
    phase: Phase,
    last_switch: Option<Instant>,
    last_health_probe: Option<Instant>,
    consecutive_probe_fails: u32,
    /// node_id → eject until
    ejected: HashMap<String, Instant>,
    eject_counts: HashMap<String, u32>,
}

impl Default for Controller {
    fn default() -> Self {
        Self {
            phase: Phase::Ok,
            last_switch: None,
            last_health_probe: None,
            consecutive_probe_fails: 0,
            ejected: HashMap::new(),
            eject_counts: HashMap::new(),
        }
    }
}

impl Controller {
    fn set_phase(&mut self, p: Phase) {
        if self.phase != p {
            app_log::debug(
                "smart_switch",
                format!("phase {} → {}", self.phase.as_str(), p.as_str()),
            );
            self.phase = p;
        }
    }

    fn in_dwell(&self) -> bool {
        self.last_switch
            .map(|t| t.elapsed() < MIN_DWELL)
            .unwrap_or(false)
    }

    fn in_soft_cooldown(&self) -> bool {
        self.last_switch
            .map(|t| t.elapsed() < MIN_DWELL + COOLDOWN)
            .unwrap_or(false)
    }

    fn health_probe_due(&self) -> bool {
        self.last_health_probe
            .map(|t| t.elapsed() >= HEALTH_PROBE_INTERVAL)
            .unwrap_or(true)
    }

    fn mark_switched(&mut self) {
        self.last_switch = Some(Instant::now());
        self.consecutive_probe_fails = 0;
        self.set_phase(Phase::Cooldown);
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

    fn ejected_ids(&self) -> Vec<String> {
        let now = Instant::now();
        self.ejected
            .iter()
            .filter(|(_, until)| now < **until)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

static CTRL: LazyLock<Mutex<Controller>> = LazyLock::new(|| Mutex::new(Controller::default()));

fn ctrl() -> std::sync::MutexGuard<'static, Controller> {
    CTRL.lock().unwrap_or_else(|p| p.into_inner())
}

/// Per smart-rule: last switch + last measured latency of the selected leaf.
#[derive(Debug, Clone)]
struct RuleState {
    last_switch: Option<Instant>,
    last_probe: Instant,
    consecutive_probe_fails: u32,
    last_node_id: Option<String>,
    last_latency_ms: Option<u32>,
}

static RULE_STATE: LazyLock<Mutex<HashMap<String, RuleState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// —— Shared decision helpers ——

/// Clash url-test style: switch only if best is better by more than `TOLERANCE_MS`,
/// or by a large relative margin (≥25% and at least TOLERANCE_MS absolute).
fn should_prefer(best_ms: u32, cur_ms: u32) -> bool {
    if best_ms.saturating_add(TOLERANCE_MS) < cur_ms {
        return true;
    }
    let better_ratio = (best_ms as f64) <= (cur_ms as f64) * (1.0 - MIN_IMPROVEMENT_RATIO);
    better_ratio && cur_ms.saturating_sub(best_ms) >= TOLERANCE_MS
}

fn should_switch(cur_ms: Option<u32>, best_ms: u32, hard_fail: bool) -> bool {
    if hard_fail {
        return true;
    }
    match cur_ms {
        None => true,
        Some(cms) => should_prefer(best_ms, cms),
    }
}

/// Lower is better. Uses probe latency + optional passive fail rate + eject.
fn score_node(latency_ms: Option<u32>, fail_rate: f64, ejected: bool) -> f64 {
    let lat = latency_ms
        .map(|m| m as f64)
        .unwrap_or(SCORE_UNKNOWN_LATENCY);
    let fail = fail_rate.clamp(0.0, 1.0) * SCORE_FAIL_PENALTY;
    let ej = if ejected { SCORE_EJECT_PENALTY } else { 0.0 };
    lat + fail + ej
}

fn sort_candidates_by_score(nodes: &mut [ProxyNode], ejected: &[String]) {
    nodes.sort_by(|a, b| {
        let ea = ejected.iter().any(|e| e == &a.id);
        let eb = ejected.iter().any(|e| e == &b.id);
        let sa = score_node(a.latency_ms, 0.0, ea);
        let sb = score_node(b.latency_ms, 0.0, eb);
        sa.partial_cmp(&sb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            if let Some(state) = app.try_state::<AppState>() {
                if state.is_core_transitioning() {
                    tokio::time::sleep(TICK).await;
                    continue;
                }
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
        c.set_phase(Phase::Probing);
    }

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
        ctrl().set_phase(Phase::Ok);
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
        ctrl().set_phase(Phase::Ok);
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

    let ejected = ctrl().ejected_ids();
    nodes.retain(|n| !ejected.iter().any(|e| e == &n.id));
    sort_candidates_by_score(&mut nodes, &ejected);

    let mut probed: u32 = 0;
    let mut best: Option<(String, String, u32)> = None;
    let limit = nodes.len().min(BOOTSTRAP_MAX);
    let pool = &nodes[..limit];

    app_log::debug(
        "smart_switch",
        format!("bootstrap pool size={limit} (max {BOOTSTRAP_MAX})"),
    );

    for (batch_idx, batch) in pool.chunks(BOOTSTRAP_BATCH).enumerate() {
        let still_on = state
            .with_store(|s| Ok(s.settings.auto_select.is_smart()))
            .unwrap_or(false);
        if !still_on {
            ctrl().set_phase(Phase::Ok);
            app_log::info(
                "smart_switch",
                "bootstrap cancelled (auto_select not smart)",
            );
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
        .with_store(|s| Ok(s.settings.auto_select.is_smart()))
        .unwrap_or(false);
    if !still_on {
        ctrl().set_phase(Phase::Ok);
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
        ctrl().set_phase(Phase::Ok);
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
            c.last_health_probe = Some(Instant::now());
            c.set_phase(Phase::Ok);
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
        c.mark_switched();
        c.last_health_probe = Some(Instant::now());
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
        .with_store(|s| Ok(s.settings.auto_select.is_smart()))
        .unwrap_or(false);
    if !enabled || !state.is_core_running() {
        return Ok(());
    }

    {
        let mut c = ctrl();
        c.clear_eject_if_expired();
        if c.in_dwell() {
            c.set_phase(Phase::Cooldown);
            return Ok(());
        }
        if c.phase == Phase::Cooldown && !c.in_soft_cooldown() {
            c.set_phase(Phase::Ok);
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

    // —— Level 0: passive observation ——
    let passive = {
        let rt = state.lock_runtime();
        rt.passive_node_stats(&current_tag, PASSIVE_LOOKBACK_MS)
    };
    let passive_soft = passive.soft_degraded(PASSIVE_MIN_SAMPLES, PASSIVE_FAIL_RATE);
    let passive_hard = passive.hard_degraded();
    let passive_bad = passive_soft || passive_hard;

    let (follow_up, health_due, soft_cd) = {
        let c = ctrl();
        (
            c.consecutive_probe_fails > 0,
            c.health_probe_due(),
            c.in_soft_cooldown(),
        )
    };

    // Nothing to do: healthy and health re-probe not due.
    if !passive_bad && !follow_up && !health_due {
        ctrl().set_phase(Phase::Ok);
        return Ok(());
    }

    // Soft cooldown: block health re-probe and soft passive; allow hard passive / fail streak.
    if soft_cd && !passive_hard && !follow_up {
        return Ok(());
    }

    if passive_bad {
        ctrl().set_phase(Phase::Suspect);
    }

    app_log::debug(
        "smart_switch",
        format!(
            "signal phase={} passive_soft={} passive_hard={} sus={}/{} dests={}/{} consec={} follow_up={} health_due={}",
            ctrl().phase.as_str(),
            passive_soft,
            passive_hard,
            passive.suspicious,
            passive.total,
            passive.sus_dests,
            passive.dests,
            passive.consecutive_recent_sus,
            follow_up,
            health_due
        ),
    );

    let Some(api) = clash else {
        return Ok(());
    };

    ctrl().set_phase(Phase::Probing);

    // —— Level 1: confirm current node ——
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
        } else {
            // Probe succeeded: clear fail streak when not in passive hard mode.
            if !passive_hard {
                c.consecutive_probe_fails = 0;
            }
            if let Some(ms) = cur_ms {
                let _ = state.with_store_mut(|store| {
                    store.update_node_latency(&current_id, Some(ms), now_secs());
                    Ok(())
                });
            }
        }
    }

    let hard_fail = {
        let c = ctrl();
        (cur_fail && c.consecutive_probe_fails >= CONSECUTIVE_PROBE_FAILS)
            || (cur_fail && passive_hard)
            || (cur_fail && passive_soft && c.consecutive_probe_fails >= 1)
    };

    // Healthy re-probe path: current OK and no passive bad — only switch if candidate much better.
    let health_only = !passive_bad && !hard_fail && !cur_fail && health_due;

    if !cur_fail && !passive_bad && !health_only {
        // Soft passive cleared by successful probe, or follow-up resolved.
        ctrl().set_phase(Phase::Ok);
        return Ok(());
    }

    // Successful current + soft passive only: if latency not worse than peer cache, skip expand.
    if !cur_fail && passive_soft && !passive_hard && !hard_fail {
        if let Some(ms) = cur_ms {
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
                    app_log::debug(
                        "smart_switch",
                        format!("soft passive but cur {ms}ms within peer median band; skip"),
                    );
                    ctrl().set_phase(Phase::Suspect);
                    return Ok(());
                }
            }
        }
    }

    if !hard_fail {
        let c = ctrl();
        if c.in_soft_cooldown() && !passive_hard {
            ctrl().set_phase(Phase::Cooldown);
            return Ok(());
        }
    }

    // —— Level 2: probe top-K candidates ——
    let ejected = ctrl().ejected_ids();
    let mut candidates: Vec<ProxyNode> = nodes
        .iter()
        .filter(|n| n.id != current_id)
        .filter(|n| !ejected.iter().any(|e| e == &n.id))
        .cloned()
        .collect();
    sort_candidates_by_score(&mut candidates, &ejected);
    candidates.truncate(TOP_K);

    if candidates.is_empty() {
        if cur_fail {
            let mut c = ctrl();
            c.eject(&current_id);
        }
        if health_only {
            ctrl().last_health_probe = Some(Instant::now());
            ctrl().set_phase(Phase::Ok);
        }
        return Ok(());
    }

    let cand_results = probe_nodes(
        &candidates,
        Some(PROBE_TIMEOUT_MS),
        Some(CANDIDATE_CONCURRENCY),
        Some(api),
        probe_url,
    )
    .await
    .map_err(|e| e.to_string())?;

    if health_only {
        ctrl().last_health_probe = Some(Instant::now());
    }

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

    // Rank by live score (latency only post-probe; fail_rate 0 for successful probes).
    let mut ranked: Vec<(String, u32, f64)> = cand_results
        .into_iter()
        .filter_map(|r| {
            let ms = r.latency_ms?;
            let sc = score_node(Some(ms), 0.0, false);
            Some((r.id, ms, sc))
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.2.partial_cmp(&b.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
    });

    if ranked.is_empty() {
        app_log::warn(
            "smart_switch",
            "all candidates failed (possible local network issue)",
        );
        if cur_fail {
            let mut c = ctrl();
            c.eject(&current_id);
        }
        ctrl().set_phase(if passive_bad {
            Phase::Suspect
        } else {
            Phase::Ok
        });
        return Ok(());
    }

    let (best_id, best_ms, _) = ranked[0].clone();

    if !should_switch(cur_ms, best_ms, hard_fail) {
        app_log::debug(
            "smart_switch",
            format!(
                "keep current (cur={:?} best={best_ms} hard={hard_fail} tol={TOLERANCE_MS})",
                cur_ms
            ),
        );
        ctrl().set_phase(if passive_bad {
            Phase::Suspect
        } else {
            Phase::Ok
        });
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
        c.mark_switched();
        c.last_health_probe = Some(Instant::now());
        if cur_fail {
            c.eject(&current_id);
        }
    }

    app_log::info(
        "smart_switch",
        format!(
            "{} → {} ({}ms{}, score-based)",
            current.name,
            best_name,
            best_ms,
            if hard_fail {
                ", hard fail"
            } else if health_only {
                ", health re-probe"
            } else {
                ""
            }
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
            for set in store
                .rule_sets
                .iter()
                .filter(|s| s.enabled && s.strategy == RuleSetStrategy::Smart)
            {
                for r in set
                    .rules
                    .iter()
                    .filter(|r| r.enabled && matches!(r.target, RuleTarget::Smart))
                {
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
            app_log::debug("smart_switch", format!("smart rule {}: {e}", rule.id));
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
        let map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(st) = map.get(&rule.id) {
            let retry = rule_probe_interval(st.consecutive_probe_fails);
            let in_switch_cooldown = st
                .last_switch
                .map(|at| at.elapsed() < MIN_DWELL + COOLDOWN)
                .unwrap_or(false);
            if in_switch_cooldown || st.last_probe.elapsed() < retry {
                return Ok(());
            }
        }
    }

    let ejected = ctrl().ejected_ids();
    let mut pool = smart_pool_nodes(rule, nodes);
    if pool.is_empty() {
        return Ok(());
    }
    pool.retain(|n| !ejected.iter().any(|e| e == &n.id));
    sort_candidates_by_score(&mut pool, &ejected);
    pool.truncate(BOOTSTRAP_MAX.min(TOP_K.max(8)));

    let results = match probe_nodes(
        &pool,
        Some(PROBE_TIMEOUT_MS),
        Some(BOOTSTRAP_CONCURRENCY),
        Some(api.clone()),
        probe_url.to_string(),
    )
    .await
    {
        Ok(results) => results,
        Err(e) => {
            record_rule_probe_failure(&rule.id);
            return Err(e.to_string());
        }
    };

    let _ = state.with_store_mut(|store| {
        for r in &results {
            if !r.id.is_empty() {
                store.update_node_latency(&r.id, r.latency_ms, r.tested_at);
            }
        }
        Ok(())
    });

    let mut ranked: Vec<(String, String, u32, f64)> = results
        .into_iter()
        .filter_map(|r| {
            let ms = r.latency_ms?;
            let sc = score_node(Some(ms), 0.0, false);
            Some((r.id, r.name, ms, sc))
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.3.partial_cmp(&b.3)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2))
    });
    let Some((best_id, best_name, best_ms, _)) = ranked.into_iter().next() else {
        record_rule_probe_failure(&rule.id);
        return Ok(());
    };

    // Same hysteresis as global path when we know previous pick latency.
    let prev = {
        let map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        map.get(&rule.id).cloned()
    };
    if let Some(st) = &prev {
        if st.last_node_id.as_ref() == Some(&best_id) {
            // Refresh latency bookkeeping only.
            let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
            map.insert(
                rule.id.clone(),
                RuleState {
                    last_switch: st.last_switch,
                    last_probe: Instant::now(),
                    consecutive_probe_fails: 0,
                    last_node_id: Some(best_id),
                    last_latency_ms: Some(best_ms),
                },
            );
            return Ok(());
        }
        if let Some(cur_ms) = st.last_latency_ms {
            if !should_prefer(best_ms, cur_ms) {
                let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(current) = map.get_mut(&rule.id) {
                    current.last_probe = Instant::now();
                    current.consecutive_probe_fails = 0;
                    current.last_latency_ms = Some(cur_ms);
                }
                app_log::debug(
                    "smart_switch",
                    format!(
                        "smart rule {} keep {} (cur={cur_ms} best={best_ms} tol={TOLERANCE_MS})",
                        rule.payload,
                        st.last_node_id.as_deref().unwrap_or("?")
                    ),
                );
                return Ok(());
            }
        }
    }

    let tag = {
        let store = state.lock_store();
        store
            .find_node(&best_id)
            .map(outbound_tag)
            .ok_or_else(|| format!("node {best_id} missing"))?
    };

    let selected = {
        let runtime = state.lock_runtime();
        runtime
            .select_group_live(&group, &tag)
            .map_err(|e| e.to_string())
    };
    if let Err(e) = selected {
        record_rule_probe_failure(&rule.id);
        return Err(e);
    }

    {
        let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        map.insert(
            rule.id.clone(),
            RuleState {
                last_switch: Some(Instant::now()),
                last_probe: Instant::now(),
                consecutive_probe_fails: 0,
                last_node_id: Some(best_id.clone()),
                last_latency_ms: Some(best_ms),
            },
        );
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

fn rule_probe_interval(consecutive_fails: u32) -> Duration {
    if consecutive_fails == 0 {
        return RULE_PROBE_INTERVAL;
    }
    let shift = consecutive_fails.saturating_sub(1).min(4);
    RULE_FAILURE_RETRY_BASE
        .checked_mul(1u32 << shift)
        .unwrap_or(RULE_PROBE_INTERVAL)
        .min(RULE_PROBE_INTERVAL)
}

fn record_rule_probe_failure(rule_id: &str) {
    let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
    let previous = map.get(rule_id).cloned();
    map.insert(
        rule_id.to_string(),
        RuleState {
            last_switch: previous.as_ref().and_then(|st| st.last_switch),
            last_probe: Instant::now(),
            consecutive_probe_fails: previous
                .as_ref()
                .map(|st| st.consecutive_probe_fails.saturating_add(1))
                .unwrap_or(1),
            last_node_id: previous.as_ref().and_then(|st| st.last_node_id.clone()),
            last_latency_ms: previous.and_then(|st| st.last_latency_ms),
        },
    );
}

#[cfg(test)]
mod probe_schedule_tests {
    use super::*;

    #[test]
    fn healthy_smart_rules_probe_every_ten_minutes() {
        assert_eq!(rule_probe_interval(0), Duration::from_secs(600));
    }

    #[test]
    fn smart_rule_failures_back_off_up_to_ten_minutes() {
        let seconds: Vec<u64> = (1..=7)
            .map(|fails| rule_probe_interval(fails).as_secs())
            .collect();
        assert_eq!(seconds, vec![60, 120, 240, 480, 600, 600, 600]);
    }
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
        let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&rule.id);
    }
    maintain_smart_rule(state, rule, &nodes, &probe_url, api).await
}

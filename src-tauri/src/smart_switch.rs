//! Smart node auto-switch.
//!
//! Architecture (2026-09 rework — "must beat the manual ping→try→retry loop"):
//!   every tick: patrol the CURRENT exit with a through-kernel URL probe
//!     (cache-backed, generous-bar confirm on failure so a transient blip
//!     or a merely-slow node never triggers a scan)
//!   → dead exit: recovery bypasses dwell/cooldown (a bad pick must not
//!     blind the engine for MIN_DWELL). Ping-rank candidates in batches,
//!     then URL-VERIFY the top few through the kernel delay API — the
//!     manual loop's "open a page through it and see", done without moving
//!     the selector — and switch to the fastest verified node. Ping/verify
//!     failures get escalating ejection, so scanning a broken pool makes
//!     strict forward progress across rounds.
//!   → healthy exit: soft paths stay dwell-guarded — passive-journal
//!     degrade signals and a 10-min drift re-probe may switch only when a
//!     verified candidate beats the current URL latency by tolerance.
//!
//! URL delay is the single comparison currency (current and candidates
//! alike); TCP ping is only the cheap ordering pre-filter.
//!
//! Lock rule: never hold `store` while acquiring `runtime` (see AppState).

use crate::app_log;
use crate::config::outbound_tag;
use crate::domain::{ProxyNode, Rule, RuleSetStrategy, RuleTarget};
use crate::runtime::PassiveNodeStats;
use crate::services::latency::{probe_nodes, probe_nodes_ranked, LatencyResult};
use crate::state::AppState;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

// —— Schedule ——
const TICK: Duration = Duration::from_secs(20);
/// After a soft (optimization) switch, refuse further soft switches this
/// long. Recovery switches (probe-confirmed dead exit) bypass dwell — that
/// blindness window was the "switched to a corpse and sat there" bug.
const MIN_DWELL: Duration = Duration::from_secs(120);
/// After dwell, soft switches wait this extra window.
const COOLDOWN: Duration = Duration::from_secs(90);
/// When healthy, re-optimize at most this often (url-test-like drift fix).
const HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(600);
/// Smart-rule selectors use the same low-frequency refresh cadence.
const RULE_PROBE_INTERVAL: Duration = Duration::from_secs(600);
const RULE_FAILURE_RETRY_BASE: Duration = Duration::from_secs(60);

// —— Active probe ——
/// TCP-ping timeout — the cheap ranking pre-filter.
const PROBE_TIMEOUT_MS: u64 = 2500;
/// Through-node URL probe timeout (patrol confirm + candidate verify) —
/// the "open a page through this node and see" measurement.
const VERIFY_TIMEOUT_MS: u64 = 5000;

// —— Candidate scan ——
/// Candidates pinged per scan step (also the ping wave width).
const SCAN_BATCH: usize = 8;
/// Per-round candidate budget (prefix of the score-ordered pool).
const SCAN_MAX: usize = 24;
/// Ping-passers per batch that get a real-path URL verification.
const SCAN_VERIFY_TOP: usize = 3;

// —— Smart-pool maintenance ——
const BOOTSTRAP_MAX: usize = 24;
const BOOTSTRAP_CONCURRENCY: usize = 4;

// —— Passive (connection journal) ——
const PASSIVE_LOOKBACK_MS: i64 = 20_000;
const PASSIVE_MIN_SAMPLES: u32 = 5;
const PASSIVE_FAIL_RATE: f64 = 0.15;

// —— Hysteresis (Clash url-test `tolerance` style) ——
/// Only switch when `best + TOLERANCE_MS < current`.
const TOLERANCE_MS: u32 = 50;
/// Secondary: large relative improvement also qualifies if abs ≥ TOLERANCE_MS.
const MIN_IMPROVEMENT_RATIO: f64 = 0.25;

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
        self.set_phase(Phase::Cooldown);
    }

    /// Eject a node, returning the escalation count and the ejection seconds
    /// (for activity logging at the call sites, where the node name is known).
    fn eject(&mut self, id: &str) -> (u32, u64) {
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
        (*n, secs)
    }

    fn clear_eject_if_expired(&mut self) {
        let now = Instant::now();
        let expired: Vec<_> = self
            .ejected
            .iter()
            .filter(|(_, until)| **until <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.ejected.remove(&id);
            self.eject_counts.remove(&id);
        }
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
    sort_candidates_by_score_with_fail_rate(nodes, ejected, |_| 0.0);
}

/// Same ranking, but `fail_rate_of` supplies each node's recent passive fail
/// rate (e.g. from the connection journal) so chronically-flaky nodes sort
/// behind merely-slower ones even before an active probe confirms it.
fn sort_candidates_by_score_with_fail_rate(
    nodes: &mut [ProxyNode],
    ejected: &[String],
    mut fail_rate_of: impl FnMut(&ProxyNode) -> f64,
) {
    nodes.sort_by(|a, b| {
        let ea = ejected.iter().any(|e| e == &a.id);
        let eb = ejected.iter().any(|e| e == &b.id);
        let sa = score_node(a.latency_ms, fail_rate_of(a), ea);
        let sb = score_node(b.latency_ms, fail_rate_of(b), eb);
        sa.partial_cmp(&sb)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Poll cadence while the core is down — quick to notice a start so smart
/// mode engages promptly instead of waiting a full TICK.
const IDLE_POLL: Duration = Duration::from_secs(5);
/// Re-log an idle reason at most this often while it persists.
const IDLE_NOTE_INTERVAL: Duration = Duration::from_secs(60);
/// Re-log a skipped round (lock contention) at most this often.
const SKIP_NOTE_INTERVAL: Duration = Duration::from_secs(60);

/// Hard wall for one round of engine work. Generous enough for a full
/// bootstrap scan (24 nodes, ping + verify ≈ 15s observed); anything past it
/// is a stuck round — abandon it and keep looping. A silently-dead engine is
/// exactly the "node died and nothing switched" failure mode (2026-09-13:
/// the engine froze three times and the dead exit was never recovered), so
/// the loop must never die with its work.
const WORK_HARD_LIMIT: Duration = Duration::from_secs(120);

/// Tracks observed core-running samples to detect a real Stopped→Running
/// edge. `None` samples (core transitioning — the stop/start of a restart
/// hides inside the transition) are no-observations: a restart therefore
/// reads as continuous running and never fabricates an edge.
#[derive(Debug, Default)]
struct RunningEdge {
    was_running: bool,
}

impl RunningEdge {
    fn observe(&mut self, sample: Option<bool>) -> bool {
        match sample {
            Some(running) => {
                let edge = running && !self.was_running;
                self.was_running = running;
                edge
            }
            None => false,
        }
    }
}

/// Last time we logged a skipped round (lock contention) — throttled so a
/// stuck lock holder stays visible without flooding the log.
static SKIP_NOTE: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));

fn note_round_skipped(what: &str) {
    let mut last = SKIP_NOTE.lock().unwrap_or_else(|p| p.into_inner());
    let now = Instant::now();
    if last.map(|at| now.duration_since(at) < SKIP_NOTE_INTERVAL) == Some(true) {
        return;
    }
    app_log::info(
        "smart_switch",
        format!("round skipped: {what} lock busy — will retry next tick"),
    );
    *last = Some(now);
}

/// Throttled liveness note for why the engine isn't doing rounds. A dangling
/// core-transition flag or a stopped core used to be indistinguishable from
/// a dead engine (total log silence) — now every idle state is visible.
fn note_engine_idle(last: &mut Option<(&'static str, Instant)>, reason: &'static str) {
    let now = Instant::now();
    if let Some((seen, at)) = *last {
        if seen == reason && now.duration_since(at) < IDLE_NOTE_INTERVAL {
            return;
        }
    }
    app_log::info("smart_switch", format!("engine idle: {reason}"));
    *last = Some((reason, now));
}

/// Run one engine round as a supervised CHILD task and await it under the
/// hard limit. A round that blocks mid-poll on a worker (sync lock queue,
/// hung syscall) or panics cannot take the engine loop down with it: tokio
/// timeouts only fire at await points, so a sync block inside the SAME
/// task's future would starve its own timeout — as a detached child the
/// block only strands that child, the supervisor's timer still fires, the
/// round is abandoned with a visible log, and the loop keeps running.
async fn supervise_round<F, Fut>(app: &AppHandle, label: &'static str, make: F)
where
    F: FnOnce(AppHandle) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
{
    let started = Instant::now();
    // tokio::spawn (not tauri::async_runtime::spawn) so the handle survives
    // the timeout wrapper and can be aborted afterwards.
    let mut handle = tokio::spawn(make(app.clone()));
    match tokio::time::timeout(WORK_HARD_LIMIT, &mut handle).await {
        Ok(Ok(Ok(()))) => {
            app_log::debug(
                "smart_switch",
                format!("{label} round done in {:?}", started.elapsed()),
            );
        }
        Ok(Ok(Err(e))) => app_log::warn("smart_switch", format!("{label}: {e}")),
        Ok(Err(join)) => app_log::error(
            "smart_switch",
            format!("{label} round failed to join: {join}"),
        ),
        Err(_) => {
            // Abort cancels the child at its next await point (a child stuck
            // mid-poll in sync code cannot be cancelled, but one parked on an
            // await — e.g. a saturated probe semaphore — is). Either way the
            // abandoned child can no longer pile up unnoticed.
            handle.abort();
            app_log::warn(
                "smart_switch",
                format!(
                    "{label} round exceeded hard limit ({:?} elapsed) — aborted, engine keeps running",
                    started.elapsed()
                ),
            );
        }
    }
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        // A fresh app session that starts the core while auto_select=smart
        // never passes through the UI's enable-time bootstrap
        // (`smart_switch_now` is only invoked on the toggle) — the engine
        // must engage itself on the core's Stopped→Running edge.
        let mut edge = RunningEdge::default();
        let mut idle_note: Option<(&'static str, Instant)> = None;
        loop {
            if let Some(state) = app.try_state::<AppState>() {
                if state.is_core_transitioning() {
                    edge.observe(None);
                    note_engine_idle(&mut idle_note, "core transitioning");
                    tokio::time::sleep(IDLE_POLL).await;
                    continue;
                }
                let running = state.is_core_running();
                if edge.observe(Some(running)) {
                    supervise_round(&app, "bootstrap", |app| async move {
                        let state = app.state::<AppState>();
                        bootstrap_if_smart(&state).await;
                        Ok(())
                    })
                    .await;
                }
                if running {
                    idle_note = None;
                    supervise_round(&app, "tick", |app| async move {
                        let state = app.state::<AppState>();
                        tick(&state).await
                    })
                    .await;
                    supervise_round(&app, "smart rules", |app| async move {
                        let state = app.state::<AppState>();
                        tick_smart_rules(&state).await
                    })
                    .await;
                    tokio::time::sleep(TICK).await;
                    continue;
                }
                note_engine_idle(&mut idle_note, "core not running");
            }
            tokio::time::sleep(IDLE_POLL).await;
        }
    });
}

/// The core just came up: if smart mode is on, run the same verified
/// bootstrap the UI triggers when toggling smart on, so the engine actually
/// engages (probe the pool, pick the best verified node) instead of idling
/// until the next degrade/drift tick.
async fn bootstrap_if_smart(state: &AppState) {
    let smart_on = state
        .with_store(|s| Ok(s.settings.auto_select.is_smart()))
        .unwrap_or(false);
    if !smart_on {
        return;
    }
    app_log::info(
        "smart_switch",
        "core started with smart mode on — bootstrap",
    );
    if let Err(e) = select_best_now(state).await {
        app_log::warn("smart_switch", format!("start bootstrap: {e}"));
    }
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

    // Custom sing-box configs manage their own outbounds — nothing to switch.
    if state
        .with_store(|s| Ok(s.settings.runtime_source().is_custom()))
        .unwrap_or(false)
    {
        app_log::warn("smart_switch", "bootstrap skipped: custom runtime mode");
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: None,
            to_id: None,
            to_name: None,
            latency_ms: None,
            probed: 0,
            message: "custom runtime mode".into(),
        });
    }

    // Smart switch leans on the Clash API connection journal for passive
    // health and for live group selection — neither exists under Xray.
    if state
        .with_store(|s| {
            Ok(crate::core::CoreKind::parse(&s.settings.core_type) == crate::core::CoreKind::Xray)
        })
        .unwrap_or(false)
    {
        app_log::warn(
            "smart_switch",
            "bootstrap skipped: Xray core (no connection journal)",
        );
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: None,
            to_id: None,
            to_name: None,
            latency_ms: None,
            probed: 0,
            message: "smart switch requires the sing-box core".into(),
        });
    }

    {
        let mut c = ctrl();
        c.clear_eject_if_expired();
        c.set_phase(Phase::Probing);
    }

    let (current_id, nodes, probe_url) = {
        let store = state.lock_store();
        (
            store.settings.current_node_id.clone(),
            store.enabled_nodes(),
            store.settings.probe_url.clone(),
        )
    };
    let (clash, core_kind) = {
        let rt = state.lock_runtime();
        (rt.clash_api_clone(), rt.core.kind())
    };
    // Main-node candidates must be servable by the running core (a core may drop
    // REALITY nodes — they pass TCP probes and would win the race, then the
    // pick is rejected by the switch guard).
    let nodes: Vec<_> = nodes
        .into_iter()
        .filter(|n| core_kind.supports_node(n))
        .collect();

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

    // Scan the pool and pick the best VERIFIED node — a ping-lowest pick
    // that cannot carry real traffic is never applied. The current node
    // competes too (winning keeps it, "already best").
    let (picked, probed) = run_scan(
        state,
        &nodes,
        current_id.as_deref().unwrap_or(""),
        ScanGoal::BestOverall,
        &api,
        &probe_url,
        "bootstrap",
    )
    .await?;

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

    let Some(pick) = picked else {
        ctrl().set_phase(Phase::Ok);
        app_log::warn(
            "smart_switch",
            format!("bootstrap: no verified node (probed={probed})"),
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

    if current_id.as_ref() == Some(&pick.id) {
        {
            let mut c = ctrl();
            c.last_switch = Some(Instant::now());
            c.last_health_probe = Some(Instant::now());
            c.set_phase(Phase::Ok);
        }
        app_log::info(
            "smart_switch",
            format!(
                "bootstrap: already best {} ({}ms, verified)",
                pick.name, pick.url_ms
            ),
        );
        return Ok(SmartSwitchNowResult {
            switched: false,
            from_id: current_id,
            to_id: Some(pick.id),
            to_name: Some(pick.name),
            latency_ms: Some(pick.url_ms),
            probed,
            message: "already best".into(),
        });
    }

    apply_verified_switch(state, &pick, false)?;
    app_log::info(
        "smart_switch",
        format!(
            "bootstrap: {} → {} ({}ms, probed={})",
            current_id.as_deref().unwrap_or("—"),
            pick.name,
            pick.url_ms,
            probed
        ),
    );

    Ok(SmartSwitchNowResult {
        switched: true,
        from_id: current_id,
        to_id: Some(pick.id),
        to_name: Some(pick.name),
        latency_ms: Some(pick.url_ms),
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

    // Routine optimization preserves working sessions. Only failure recovery
    // may clear them, and still only when the user enabled that setting.
    match state.select_current_node_serialized(best_id, false, hard_fail) {
        Ok((_, _, true)) => {}
        Ok((_, _, false)) => return Err("core not running".into()),
        Err(e) => {
            app_log::error(
                "smart_switch",
                format!("select_node_live failed for {name} ({tag}): {e}"),
            );
            return Err(e.to_string());
        }
    }

    app_log::debug(
        "smart_switch",
        format!("applied switch → {name} (hard_fail={hard_fail})"),
    );
    Ok(())
}

async fn tick(state: &AppState) -> Result<(), String> {
    let round_start = Instant::now();
    // Hot path uses try-locks throughout: a contended lock skips this round
    // (with a throttled log) instead of parking the round on a worker — a
    // stuck lock holder must never stop the patrol. Guards live in blocks so
    // they provably never cross an await.
    let (enabled, custom, current_id, nodes, probe_url) = {
        let Some(store) = state.try_lock_store() else {
            note_round_skipped("store");
            return Ok(());
        };
        (
            store.settings.auto_select.is_smart(),
            store.settings.runtime_source().is_custom(),
            store.settings.current_node_id.clone(),
            store.enabled_nodes(),
            store.settings.probe_url.clone(),
        )
    };
    // Custom sing-box configs manage their own outbounds — nothing to switch.
    if !enabled || custom || !state.is_core_running() {
        return Ok(());
    }

    let (clash, core_kind) = {
        let Some(rt) = state.try_lock_runtime() else {
            note_round_skipped("runtime");
            return Ok(());
        };
        (rt.clash_api_clone(), rt.core.kind())
    };
    // See select_best_now: only core-servable nodes are candidates.
    let nodes: Vec<_> = nodes
        .into_iter()
        .filter(|n| core_kind.supports_node(n))
        .collect();

    let Some(current_id) = current_id else {
        return Ok(());
    };
    let Some(current) = nodes.iter().find(|n| n.id == current_id).cloned() else {
        return Ok(());
    };
    let current_tag = outbound_tag(&current);

    // —— Level 0: passive journal ——
    let passive = {
        let Some(rt) = state.try_lock_runtime() else {
            note_round_skipped("runtime");
            return Ok(());
        };
        rt.passive_node_stats(&current_tag, PASSIVE_LOOKBACK_MS)
    };
    let passive_soft = passive.soft_degraded(PASSIVE_MIN_SAMPLES, PASSIVE_FAIL_RATE);
    let passive_hard = passive.hard_degraded();
    let passive_bad = passive_soft || passive_hard;

    let Some(api) = clash else {
        return Ok(());
    };

    // —— Level 0.5: exit patrol ——
    // URL-probe the current exit through the kernel every tick — the engine
    // must notice a dead exit on its own, not only after the user's traffic
    // starts failing. Fast (cache-backed) bar first; a generous-bar confirm
    // on failure so a transient blip or a merely-slow node never triggers a
    // scan.
    let patrol_started = Instant::now();
    let outcome = patrol_exit(&current, &api, &probe_url).await?;
    // Stage timing: with the 2026-09-13 stuck-round incident these lines
    // pinpoint which segment of a round ate the hard limit.
    app_log::debug(
        "smart_switch",
        format!(
            "tick timing: gates={:?} patrol={:?}",
            patrol_started - round_start,
            patrol_started.elapsed()
        ),
    );
    match outcome {
        PatrolOutcome::Healthy(ms) => {
            app_log::trace(
                "smart_switch",
                format!("patrol: {} {ms}ms ok", current.name),
            );
        }
        PatrolOutcome::SlowButAlive(ms) => {
            app_log::info(
                "smart_switch",
                format!(
                    "patrol: {} failed the fast bar, generous confirm {ms}ms — alive, keeping",
                    current.name
                ),
            );
        }
        PatrolOutcome::Dead => {}
    }
    let cur_url_ms = outcome.ms();
    // Stage markers: the 2026-09-13 stuck-round hunt needs each suspect step
    // isolated — the last marker before silence names the blocking call.
    app_log::debug("smart_switch", "stage: post-trace");
    if let Some(ms) = cur_url_ms {
        if state
            .try_with_store_mut(|store| {
                store.update_node_latency(&current_id, Some(ms), now_secs());
                Ok(())
            })
            .is_none()
        {
            note_round_skipped("store (latency write)");
        }
    }
    app_log::debug("smart_switch", "stage: latency written");
    let exit_dead = outcome.is_dead();
    let phase_str = ctrl().phase.as_str();
    app_log::debug(
        "smart_switch",
        format!("stage: ctrl phase read ({phase_str})"),
    );
    let health_due = ctrl().health_probe_due();
    app_log::debug("smart_switch", "stage: ctrl health read");

    app_log::debug(
        "smart_switch",
        format!(
            "signal phase={phase_str} exit_dead={exit_dead} exit_ms={cur_url_ms:?} passive_soft={passive_soft} passive_hard={passive_hard} sus={}/{} dests={}/{} health_due={health_due}",
            passive.suspicious,
            passive.total,
            passive.sus_dests,
            passive.dests,
        ),
    );
    app_log::debug(
        "smart_switch",
        format!(
            "tick timing: through signal line, total={:?}",
            round_start.elapsed()
        ),
    );

    // —— Dwell / cooldown gates — recovery bypasses ——
    let mut dead_eject: Option<(u32, u64)> = None;
    {
        let mut c = ctrl();
        c.clear_eject_if_expired();
        if exit_dead {
            // Probe-confirmed dead exit: recovery must not sit out MIN_DWELL —
            // that blindness window was the "switched to a corpse and stayed"
            // failure mode.
            c.set_phase(Phase::Probing);
            dead_eject = Some(c.eject(&current_id));
        } else if c.in_dwell() {
            c.set_phase(Phase::Cooldown);
            return Ok(());
        } else if c.phase == Phase::Cooldown && !c.in_soft_cooldown() {
            c.set_phase(Phase::Ok);
        }
    }

    if exit_dead {
        let eject_note = dead_eject
            .map(|(n, s)| format!(" (eject {n}x {s}s)"))
            .unwrap_or_default();
        app_log::warn(
            "smart_switch",
            format!(
                "exit probe failed on {}{eject_note} — recovery scan",
                current.name
            ),
        );
        let (picked, _) = run_scan(
            state,
            &nodes,
            &current_id,
            ScanGoal::FirstVerified,
            &api,
            &probe_url,
            "recovery",
        )
        .await?;
        if picked.is_none() {
            // Ejected corpses are skipped next round; the pool is worked
            // through with strict forward progress.
            app_log::warn(
                "smart_switch",
                "recovery scan: no verified candidate this round",
            );
            ctrl().set_phase(Phase::Suspect);
        }
        return Ok(());
    }

    // —— Soft paths (exit healthy): optimization, dwell-guarded ——
    let health_due = ctrl().health_probe_due();
    if !passive_bad && !health_due {
        ctrl().set_phase(Phase::Ok);
        return Ok(());
    }
    // Soft cooldown blocks optimization; a hard passive signal still gets a
    // tolerance-checked reselect.
    if ctrl().in_soft_cooldown() && !passive_hard {
        ctrl().set_phase(Phase::Cooldown);
        return Ok(());
    }
    if passive_bad {
        ctrl().set_phase(Phase::Suspect);
    }

    // Passive-soft band guard: probe-healthy current inside the peer median
    // band → don't expand into a scan.
    if passive_soft && !passive_hard {
        if let Some(ms) = cur_url_ms {
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
                    app_log::info(
                        "smart_switch",
                        format!(
                            "passive soft but {} {}ms within peer median band — skip scan",
                            current.name, ms
                        ),
                    );
                    ctrl().set_phase(Phase::Suspect);
                    return Ok(());
                }
            }
        }
    }

    ctrl().set_phase(Phase::Probing);
    let goal = match cur_url_ms {
        Some(ms) => ScanGoal::BetterThan(ms),
        None => ScanGoal::FirstVerified,
    };
    let reason = if passive_hard {
        "passive hard degrade"
    } else if passive_soft {
        "passive soft degrade"
    } else {
        "drift re-probe"
    };
    let scanned = run_scan(state, &nodes, &current_id, goal, &api, &probe_url, reason).await;
    if health_due {
        ctrl().last_health_probe = Some(Instant::now());
    }
    let (picked, _) = scanned?;
    if picked.is_none() {
        ctrl().set_phase(if passive_bad {
            Phase::Suspect
        } else {
            Phase::Ok
        });
    }
    Ok(())
}

/// Result of one exit-patrol round (the tick-level "is the current exit
/// actually carrying traffic" check).
#[derive(Debug, Clone, Copy)]
enum PatrolOutcome {
    /// Fast (cache-backed) bar passed.
    Healthy(u32),
    /// Fast bar failed; the generous confirm probe still carried traffic —
    /// slow but alive, keep it.
    SlowButAlive(u32),
    /// Both bars failed — probe-confirmed dead exit.
    Dead,
}

impl PatrolOutcome {
    fn ms(self) -> Option<u32> {
        match self {
            Self::Healthy(ms) | Self::SlowButAlive(ms) => Some(ms),
            Self::Dead => None,
        }
    }

    fn is_dead(self) -> bool {
        matches!(self, Self::Dead)
    }
}

/// URL-probe the current exit through the kernel (delay API). Fast bar
/// (cache-backed) first; on failure a generous-bar probe — healthy unless
/// both fail. The engine-side equivalent of the manual "open a page and
/// see if it loads" check, run every tick.
async fn patrol_exit(
    current: &ProxyNode,
    api: &crate::api::ClashApi,
    probe_url: &str,
) -> Result<PatrolOutcome, String> {
    let fast_started = Instant::now();
    let fast = probe_nodes(
        std::slice::from_ref(current),
        Some(PROBE_TIMEOUT_MS),
        Some(1),
        Some(api.clone()),
        probe_url.to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;
    if fast_started.elapsed() > Duration::from_millis(PROBE_TIMEOUT_MS) {
        app_log::debug(
            "smart_switch",
            format!(
                "patrol: fast bar took {:?} (budget {}ms) — probe layer queuing",
                fast_started.elapsed(),
                PROBE_TIMEOUT_MS
            ),
        );
    }
    if let Some(ms) = fast.first().and_then(|r| r.latency_ms) {
        return Ok(PatrolOutcome::Healthy(ms));
    }
    let fair_started = Instant::now();
    let fair = probe_nodes(
        std::slice::from_ref(current),
        Some(VERIFY_TIMEOUT_MS),
        Some(1),
        Some(api.clone()),
        probe_url.to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;
    if fair_started.elapsed() > Duration::from_millis(VERIFY_TIMEOUT_MS) {
        app_log::debug(
            "smart_switch",
            format!(
                "patrol: confirm bar took {:?} (budget {}ms) — probe layer queuing",
                fair_started.elapsed(),
                VERIFY_TIMEOUT_MS
            ),
        );
    }
    let ms = fair.first().and_then(|r| r.latency_ms);
    Ok(match ms {
        Some(ms) => PatrolOutcome::SlowButAlive(ms),
        None => PatrolOutcome::Dead,
    })
}

/// What a candidate scan is trying to achieve.
#[derive(Debug, Clone, Copy)]
enum ScanGoal {
    /// Any node that carries real traffic — as fast as possible (dead exit).
    FirstVerified,
    /// Switch only if clearly better than this URL latency (tolerance).
    BetterThan(u32),
    /// Scan the whole budget and return the best verified node (the
    /// initial pick; the caller decides whether to apply it).
    BestOverall,
}

/// One candidate proven to carry real traffic through the kernel (URL
/// probe verified — no ping-only winners).
struct VerifiedPick {
    id: String,
    name: String,
    url_ms: u32,
}

/// Scan score-ordered candidates in ping batches until the goal is met.
/// `reason` is the trigger ("recovery" / "passive soft degrade" / …) — it
/// only feeds the activity log. Returns the verified pick (already switched
/// to, except for [`ScanGoal::BestOverall`] where the caller applies it)
/// plus the number of nodes pinged.
async fn run_scan(
    state: &AppState,
    nodes: &[ProxyNode],
    current_id: &str,
    goal: ScanGoal,
    api: &crate::api::ClashApi,
    probe_url: &str,
    reason: &str,
) -> Result<(Option<VerifiedPick>, u32), String> {
    let include_current = matches!(goal, ScanGoal::BestOverall);
    let ejected = ctrl().ejected_ids();
    let mut candidates: Vec<ProxyNode> = nodes
        .iter()
        .filter(|n| include_current || n.id != current_id)
        .filter(|n| !ejected.iter().any(|e| e == &n.id))
        .cloned()
        .collect();
    // Weight ranking by each candidate's own recent passive fail rate so a
    // chronically-flaky node doesn't out-rank a merely-slower one just
    // because its last active probe happened to land low. One single-pass
    // scan for every candidate (per-tag scans would be O(nodes × history)
    // under the runtime lock).
    let fail_rates: HashMap<String, f64> = if candidates.is_empty() {
        HashMap::new()
    } else {
        // Contended runtime lock just means no passive weighting this round —
        // the scan itself doesn't need it.
        match state.try_lock_runtime() {
            Some(rt) => {
                let tags: Vec<String> = candidates.iter().map(outbound_tag).collect();
                let stats = rt.passive_stats_for_tags(&tags, PASSIVE_LOOKBACK_MS);
                tags.iter()
                    .zip(candidates.iter())
                    .map(|(tag, n)| {
                        (
                            n.id.clone(),
                            stats.get(tag).map(PassiveNodeStats::fail_rate).unwrap_or(0.0),
                        )
                    })
                    .collect()
            }
            None => HashMap::new(),
        }
    };
    sort_candidates_by_score_with_fail_rate(&mut candidates, &ejected, |n| {
        fail_rates.get(&n.id).copied().unwrap_or(0.0)
    });
    candidates.truncate(SCAN_MAX);

    let goal_desc = match goal {
        ScanGoal::FirstVerified => "first-verified".to_string(),
        ScanGoal::BetterThan(ms) => format!("better-than {ms}ms"),
        ScanGoal::BestOverall => "best-overall".to_string(),
    };
    app_log::info(
        "smart_switch",
        format!(
            "scan start ({reason}): goal={goal_desc}, candidates={} (of {} nodes, {} ejected)",
            candidates.len(),
            nodes.len(),
            ejected.len()
        ),
    );

    let mut probed: u32 = 0;
    let mut best: Option<VerifiedPick> = None;
    for batch in candidates.chunks(SCAN_BATCH) {
        // The user may flip auto_select off mid-scan.
        let still_on = state
            .with_store(|s| Ok(s.settings.auto_select.is_smart()))
            .unwrap_or(false);
        if !still_on {
            ctrl().set_phase(Phase::Ok);
            return Ok((None, probed));
        }
        probed = probed.saturating_add(batch.len() as u32);
        let pick = scan_slice_for_verified(state, batch, api, probe_url).await?;
        let Some(pick) = pick else {
            continue;
        };
        match goal {
            ScanGoal::FirstVerified => {
                apply_verified_switch(state, &pick, true)?;
                app_log::info(
                    "smart_switch",
                    format!("recovery → {} ({}ms, verified)", pick.name, pick.url_ms),
                );
                return Ok((Some(pick), probed));
            }
            ScanGoal::BetterThan(cur_ms) => {
                if should_prefer(pick.url_ms, cur_ms) {
                    apply_verified_switch(state, &pick, false)?;
                    app_log::info(
                        "smart_switch",
                        format!(
                            "improve → {} ({}ms vs cur {cur_ms}ms, verified)",
                            pick.name, pick.url_ms
                        ),
                    );
                    return Ok((Some(pick), probed));
                }
                // Candidates are score-ordered: the first verified pick that
                // can't beat the tolerance bar ends the round.
                let cur_name = nodes
                    .iter()
                    .find(|n| n.id == current_id)
                    .map(|n| n.name.as_str())
                    .unwrap_or(current_id);
                app_log::info(
                    "smart_switch",
                    format!(
                        "scan: best candidate {} ({}ms) doesn't beat {} ({cur_ms}ms) by tolerance — keep",
                        pick.name, pick.url_ms, cur_name
                    ),
                );
                return Ok((None, probed));
            }
            ScanGoal::BestOverall => {
                let better = best
                    .as_ref()
                    .map(|b| pick.url_ms < b.url_ms)
                    .unwrap_or(true);
                if better {
                    best = Some(pick);
                }
            }
        }
    }
    if best.is_none() && matches!(goal, ScanGoal::BetterThan(_)) {
        app_log::info(
            "smart_switch",
            format!("scan ({reason}) complete: no verified candidate, probed={probed}"),
        );
    }
    Ok((best, probed))
}

/// Ping-rank one batch, URL-verify the top few through the kernel, return
/// the fastest verified node. Ping and verify failures are ejected with
/// escalating penalties — a "TCP-alive but proxy-dead" node proves itself
/// dead here and stops wasting scan slots in later rounds.
async fn scan_slice_for_verified(
    state: &AppState,
    batch: &[ProxyNode],
    api: &crate::api::ClashApi,
    probe_url: &str,
) -> Result<Option<VerifiedPick>, String> {
    let pings = probe_nodes_ranked(
        batch,
        PROBE_TIMEOUT_MS,
        SCAN_BATCH,
        Some(api.clone()),
        probe_url,
    )
    .await
    .map_err(|e| e.to_string())?;
    record_latency_results(state, &pings);
    eject_failures(&pings);

    let shortlist = verify_shortlist(&pings, batch, SCAN_VERIFY_TOP);
    let passers = pings.iter().filter(|r| r.latency_ms.is_some()).count();
    if shortlist.is_empty() {
        app_log::info(
            "smart_switch",
            format!(
                "scan batch: ping {passers}/{} pass, none to verify",
                batch.len()
            ),
        );
        return Ok(None);
    }
    let verifies = probe_nodes(
        &shortlist,
        Some(VERIFY_TIMEOUT_MS),
        Some(shortlist.len()),
        Some(api.clone()),
        probe_url.to_string(),
    )
    .await
    .map_err(|e| e.to_string())?;
    record_latency_results(state, &verifies);
    eject_failures(&verifies);

    let mut verified: Vec<VerifiedPick> = verifies
        .iter()
        .filter_map(|r| {
            r.latency_ms.map(|ms| VerifiedPick {
                id: r.id.clone(),
                name: r.name.clone(),
                url_ms: ms,
            })
        })
        .collect();
    verified.sort_by_key(|v| v.url_ms);
    let pick = verified.into_iter().next();
    app_log::info(
        "smart_switch",
        format!(
            "scan batch: ping {passers}/{} pass, verify [{}] → {}",
            batch.len(),
            shortlist
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            pick.as_ref()
                .map(|p| format!("{}:{}ms", p.name, p.url_ms))
                .unwrap_or_else(|| "none verified".into())
        ),
    );
    Ok(pick)
}

/// Ping-passers sorted by ping (lowest first), capped at `top` — the nodes
/// worth a real-path verification.
fn verify_shortlist(pings: &[LatencyResult], batch: &[ProxyNode], top: usize) -> Vec<ProxyNode> {
    let mut passers: Vec<(u32, &ProxyNode)> = pings
        .iter()
        .filter_map(|r| {
            let ms = r.latency_ms?;
            let node = batch.iter().find(|n| n.id == r.id)?;
            Some((ms, node))
        })
        .collect();
    passers.sort_by_key(|(ms, _)| *ms);
    passers
        .into_iter()
        .take(top)
        .map(|(_, n)| n.clone())
        .collect()
}

fn record_latency_results(state: &AppState, results: &[LatencyResult]) {
    let applied = state.try_with_store_mut(|store| {
        for r in results {
            if !r.id.is_empty() {
                store.update_node_latency(&r.id, r.latency_ms, r.tested_at);
            }
        }
        Ok(())
    });
    if applied.is_none() {
        note_round_skipped("store (latency write)");
    }
}

fn eject_failures(results: &[LatencyResult]) {
    let mut c = ctrl();
    for r in results {
        if r.latency_ms.is_none() && !r.id.is_empty() {
            let (count, secs) = c.eject(&r.id);
            app_log::info(
                "smart_switch",
                format!(
                    "eject {} ({count}x, {secs}s): {}",
                    r.name,
                    r.error.as_deref().unwrap_or("probe failed")
                ),
            );
        }
    }
}

/// Hot-switch to a verified pick and start the post-switch dwell (soft
/// paths only — patrol re-verifies the pick next tick, so a dead pick is
/// caught immediately rather than after MIN_DWELL).
fn apply_verified_switch(
    state: &AppState,
    pick: &VerifiedPick,
    recovery: bool,
) -> Result<(), String> {
    apply_switch(state, &pick.id, recovery)?;
    let mut c = ctrl();
    c.mark_switched();
    c.last_health_probe = Some(Instant::now());
    Ok(())
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One smart pool to maintain. `id` doubles as the selector-group tag source
/// (`smart-<id prefix>`) — for per-rule pools it's the rule id, for whole-set
/// pools the set id — so probing/switching lands on the exact group the
/// config builder emits.
struct SmartPool {
    id: String,
    /// Log label (rule payload / set name).
    label: String,
    source: SmartPoolSource,
}

enum SmartPoolSource {
    Keywords {
        include: Vec<String>,
        exclude: Vec<String>,
    },
    /// Whole-set explicit node pool (`RuleSet::node_ids`).
    Explicit { node_ids: Vec<String> },
}

impl SmartPool {
    fn group(&self) -> String {
        format!("smart-{}", &self.id[..self.id.len().min(16)])
    }

    fn member_nodes(&self, nodes: &[ProxyNode]) -> Vec<ProxyNode> {
        match &self.source {
            SmartPoolSource::Keywords { include, exclude } => nodes
                .iter()
                .filter(|n| crate::domain::name_matches_keywords(&n.name, include, exclude))
                .cloned()
                .collect(),
            SmartPoolSource::Explicit { node_ids } => nodes
                .iter()
                .filter(|n| node_ids.iter().any(|id| id == &n.id))
                .cloned()
                .collect(),
        }
    }
}

/// Enabled smart pools to maintain: per-rule keyword pools inside Mixed sets,
/// one per Filter-strategy set (local or remote), and one per explicit
/// node-pool set (strategy Node with 2+ hand-picked members).
fn collect_enabled_smart_pools(state: &AppState) -> Vec<SmartPool> {
    state
        .with_store(|store| {
            let mut out = Vec::new();
            for set in store.rule_sets.iter().filter(|s| s.enabled) {
                match set.strategy {
                    RuleSetStrategy::Filter => out.push(SmartPool {
                        id: set.id.clone(),
                        label: set.name.clone(),
                        source: SmartPoolSource::Keywords {
                            include: set.smart_include.clone(),
                            exclude: set.smart_exclude.clone(),
                        },
                    }),
                    RuleSetStrategy::Node if set.is_node_pool() => out.push(SmartPool {
                        id: set.id.clone(),
                        label: set.name.clone(),
                        source: SmartPoolSource::Explicit {
                            node_ids: set.node_ids.clone(),
                        },
                    }),
                    RuleSetStrategy::Smart => {
                        for r in set
                            .rules
                            .iter()
                            .filter(|r| r.enabled && matches!(r.target, RuleTarget::Smart))
                        {
                            out.push(SmartPool {
                                id: r.id.clone(),
                                label: r.payload.clone(),
                                source: SmartPoolSource::Keywords {
                                    include: r.smart_include.clone(),
                                    exclude: r.smart_exclude.clone(),
                                },
                            });
                        }
                    }
                    _ => {}
                }
            }
            Ok(out)
        })
        .unwrap_or_default()
}

/// Maintain smart-pool selectors (independent of global smart_switch toggle).
async fn tick_smart_rules(state: &AppState) -> Result<(), String> {
    if !state.is_core_running() {
        return Ok(());
    }
    let pools = collect_enabled_smart_pools(state);
    let active_ids: HashSet<_> = pools.iter().map(|pool| pool.id.as_str()).collect();
    RULE_STATE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|rule_id, _| active_ids.contains(rule_id.as_str()));
    if pools.is_empty() {
        return Ok(());
    }

    let (all_nodes, probe_url) = {
        let store = state.lock_store();
        (store.enabled_nodes(), store.settings.probe_url.clone())
    };
    let (clash, core_kind) = {
        let rt = state.lock_runtime();
        (rt.clash_api_clone(), rt.core.kind())
    };
    let Some(api) = clash else {
        return Ok(());
    };
    // Only nodes the running core can actually serve are pool members in
    // the generated config (e.g. unsupported node shapes — which can pass TCP
    // probes beautifully and would otherwise win the score race, then the
    // switch PUT of their tag 400s as a non-member).
    let nodes: Vec<ProxyNode> = all_nodes
        .into_iter()
        .filter(|n| core_kind.supports_node(n))
        .collect();

    for pool in pools {
        if let Err(e) = maintain_smart_pool(state, &pool, &nodes, &probe_url, api.clone()).await {
            app_log::info("smart_switch", format!("smart pool {}: {e}", pool.label));
        }
    }
    Ok(())
}

async fn maintain_smart_pool(
    state: &AppState,
    pool: &SmartPool,
    nodes: &[ProxyNode],
    probe_url: &str,
    api: crate::api::ClashApi,
) -> Result<(), String> {
    let group = pool.group();
    {
        let map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(st) = map.get(&pool.id) {
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
    let mut members = pool.member_nodes(nodes);
    if members.is_empty() {
        return Ok(());
    }
    members.retain(|n| !ejected.iter().any(|e| e == &n.id));
    sort_candidates_by_score(&mut members, &ejected);
    members.truncate(BOOTSTRAP_MAX.min(SCAN_BATCH));

    let results = match probe_nodes_ranked(
        &members,
        PROBE_TIMEOUT_MS,
        BOOTSTRAP_CONCURRENCY,
        Some(api.clone()),
        probe_url,
    )
    .await
    {
        Ok(results) => results,
        Err(e) => {
            record_rule_probe_failure(&pool.id);
            return Err(e.to_string());
        }
    };

    let applied = state.try_with_store_mut(|store| {
        for r in &results {
            if !r.id.is_empty() {
                store.update_node_latency(&r.id, r.latency_ms, r.tested_at);
            }
        }
        Ok(())
    });
    if applied.is_none() {
        note_round_skipped("store (latency write)");
    }

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
        record_rule_probe_failure(&pool.id);
        app_log::info(
            "smart_switch",
            format!(
                "smart pool {}: all {} probed members failed",
                pool.label,
                members.len()
            ),
        );
        return Ok(());
    };

    // Same hysteresis as global path when we know previous pick latency.
    let prev = {
        let map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        map.get(&pool.id).cloned()
    };
    if let Some(st) = &prev {
        if st.last_node_id.as_ref() == Some(&best_id) {
            // Refresh latency bookkeeping only.
            let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
            map.insert(
                pool.id.clone(),
                RuleState {
                    last_switch: st.last_switch,
                    last_probe: Instant::now(),
                    consecutive_probe_fails: 0,
                    last_node_id: Some(best_id),
                    last_latency_ms: Some(best_ms),
                },
            );
            app_log::info(
                "smart_switch",
                format!(
                    "smart pool {}: {} still best ({}ms)",
                    pool.label, best_name, best_ms
                ),
            );
            return Ok(());
        }
        if let Some(cur_ms) = st.last_latency_ms {
            if !should_prefer(best_ms, cur_ms) {
                let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
                if let Some(current) = map.get_mut(&pool.id) {
                    current.last_probe = Instant::now();
                    current.consecutive_probe_fails = 0;
                    current.last_latency_ms = Some(cur_ms);
                }
                let cur_name = st
                    .last_node_id
                    .as_ref()
                    .and_then(|id| nodes.iter().find(|n| n.id == *id))
                    .map(|n| n.name.as_str())
                    .unwrap_or("?");
                app_log::info(
                    "smart_switch",
                    format!(
                        "smart pool {} keep {} (cur={cur_ms}ms best={best_ms}ms tol={TOLERANCE_MS})",
                        pool.label, cur_name
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

    let selected = state
        .select_group_live_serialized(&group, &tag)
        .and_then(|selected| {
            selected
                .then_some(())
                .ok_or_else(|| crate::error::AppError::Core("core not running".into()))
        })
        .map_err(|e| e.to_string());
    if let Err(e) = selected {
        record_rule_probe_failure(&pool.id);
        return Err(e);
    }

    {
        let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        map.insert(
            pool.id.clone(),
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
            pool.label, best_name, best_ms, group
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

    #[test]
    fn expired_ejection_resets_failure_escalation() {
        let mut controller = Controller::default();
        controller.ejected.insert(
            "recovered-node".into(),
            Instant::now() - Duration::from_secs(1),
        );
        controller.eject_counts.insert("recovered-node".into(), 4);

        controller.clear_eject_if_expired();

        assert!(!controller.ejected.contains_key("recovered-node"));
        assert!(!controller.eject_counts.contains_key("recovered-node"));
    }
}

#[cfg(test)]
mod running_edge_tests {
    use super::RunningEdge;

    #[test]
    fn fresh_session_start_fires_once() {
        let mut edge = RunningEdge::default();
        assert!(edge.observe(Some(true)));
        assert!(!edge.observe(Some(true)));
    }

    #[test]
    fn restart_hidden_in_transition_does_not_fire() {
        let mut edge = RunningEdge::default();
        assert!(edge.observe(Some(true)));
        // Core transition window between stop and start: no observation.
        assert!(!edge.observe(None));
        assert!(!edge.observe(Some(true)));
    }

    #[test]
    fn stop_then_start_fires_again() {
        let mut edge = RunningEdge::default();
        assert!(edge.observe(Some(true)));
        assert!(!edge.observe(Some(false)));
        assert!(edge.observe(Some(true)));
    }
}

#[cfg(test)]
mod scan_decision_tests {
    use super::*;

    #[test]
    fn prefer_requires_absolute_or_relative_margin() {
        // Marginal absolute gain (< TOLERANCE_MS) never qualifies.
        assert!(!should_prefer(160, 200));
        // Absolute margin ≥ 50ms qualifies.
        assert!(should_prefer(149, 200));
        assert!(should_prefer(700, 900));
        // Exactly 50ms qualifies only with the 25% relative margin.
        assert!(should_prefer(150, 200));
        assert!(!should_prefer(450, 500));
        assert!(!should_prefer(200, 200));
        assert!(!should_prefer(300, 250));
    }

    fn result(id: &str, ms: Option<u32>) -> LatencyResult {
        LatencyResult {
            id: id.into(),
            name: id.into(),
            latency_ms: ms,
            error: None,
            tested_at: 0,
            method: "tcp".into(),
        }
    }

    fn node(id: &str) -> ProxyNode {
        ProxyNode {
            id: id.into(),
            name: id.into(),
            server: "example.com".into(),
            port: 443,
            protocol: crate::domain::Protocol::Shadowsocks,
            tls: None,
            transport: None,
            udp: None,
            config: crate::domain::ProtocolConfig::Shadowsocks {
                method: "aes-256-gcm".into(),
                password: "pw".into(),
                plugin: None,
                plugin_opts: None,
                shadow_tls: None,
            },
            source: None,
            latency_ms: None,
            latency_at: None,
        }
    }

    #[test]
    fn verify_shortlist_orders_ping_passers_and_caps() {
        let batch = vec![node("a"), node("b"), node("c"), node("d")];
        let pings = vec![
            result("a", Some(180)),
            result("b", None),
            result("c", Some(90)),
            result("d", Some(120)),
        ];

        let shortlist = verify_shortlist(&pings, &batch, 2);

        let ids: Vec<&str> = shortlist.iter().map(|n| n.id.as_str()).collect();
        // Failures dropped, passers ascending by ping, capped at `top`.
        assert_eq!(ids, vec!["c", "d"]);
    }

    #[test]
    fn verify_shortlist_empty_when_all_fail() {
        let batch = vec![node("a"), node("b")];
        let pings = vec![result("a", None), result("b", None)];
        assert!(verify_shortlist(&pings, &batch, 3).is_empty());
    }
}

/// Immediate probe for one smart rule (e.g. after save). Best-effort.
pub async fn refresh_smart_rule_now(state: &AppState, rule: &Rule) -> Result<(), String> {
    if !matches!(rule.target, RuleTarget::Smart) || !rule.enabled {
        return Ok(());
    }
    let pool = SmartPool {
        id: rule.id.clone(),
        label: rule.payload.clone(),
        source: SmartPoolSource::Keywords {
            include: rule.smart_include.clone(),
            exclude: rule.smart_exclude.clone(),
        },
    };
    if !state.is_core_running() {
        return Ok(());
    }
    let (all_nodes, probe_url) = {
        let store = state.lock_store();
        (store.enabled_nodes(), store.settings.probe_url.clone())
    };
    let (clash, core_kind) = {
        let rt = state.lock_runtime();
        (rt.clash_api_clone(), rt.core.kind())
    };
    let api = clash;
    let nodes: Vec<_> = all_nodes
        .into_iter()
        .filter(|n| core_kind.supports_node(n))
        .collect();
    let Some(api) = api else {
        return Ok(());
    };
    // Bypass dwell so new rules get a pick quickly.
    {
        let mut map = RULE_STATE.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&pool.id);
    }
    maintain_smart_pool(state, &pool, &nodes, &probe_url, api).await
}

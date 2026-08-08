//! Orchestrates core + system proxy.

use crate::api::{ClashApi, ConnectionInfo, RequestRecord, TrafficTotals};
use crate::config::{
    build_singbox_config, generate_api_secret, outbound_tag, write_active_config, BuildOptions,
};
use crate::core::manager::{CoreManager, CoreState};
use crate::core::resolve_core_bin;
use crate::error::{AppError, AppResult};
use crate::proxy::{create_system_proxy, SystemProxy, SystemProxySnapshot};
use crate::storage::AppStore;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct ProxyStatus {
    pub running: bool,
    pub core_state: CoreState,
    pub system_proxy: bool,
    /// Whether TUN is enabled in settings (applied on next start / restart).
    pub tun_enabled: bool,
    /// rule | global | direct
    pub outbound_mode: String,
    pub mixed_port: u16,
    pub api_port: u16,
    pub current_node_id: Option<String>,
    pub error: Option<String>,
    pub core_path: Option<String>,
    pub config_path: Option<String>,
    /// bytes/s uplink (approx)
    pub upload_speed: u64,
    /// bytes/s downlink (approx)
    pub download_speed: u64,
    pub upload_total: u64,
    pub download_total: u64,
    pub connections: u32,
    /// Smart auto node switch enabled (derived from auto_select == smart).
    #[serde(default)]
    pub smart_switch: bool,
    /// off | smart | kernel
    #[serde(default)]
    pub auto_select: String,
    /// Unix seconds when the core last entered running state (for uptime UI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_started_at: Option<i64>,
}

/// Cap history to limit RAM (UI only needs recent activity).
const MAX_REQUEST_HISTORY: usize = 3_000;

/// Passive connection-journal stats for one outbound tag (smart switch Level 0).
#[derive(Debug, Clone, Default)]
pub struct PassiveNodeStats {
    /// Closed connections in the lookback window on this node.
    pub total: u32,
    /// Short-lived low-byte closes (proxy path often died early).
    pub suspicious: u32,
    /// Distinct destinations among all samples.
    pub dests: u32,
    /// Distinct destinations among suspicious samples.
    pub sus_dests: u32,
    /// Trailing consecutive suspicious closes (most recent first).
    pub consecutive_recent_sus: u32,
}

impl PassiveNodeStats {
    pub fn fail_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.suspicious as f64 / self.total as f64
        }
    }

    /// Soft degrade: enough samples, high fail rate, ≥2 bad destinations.
    pub fn soft_degraded(&self, min_samples: u32, fail_rate: f64) -> bool {
        self.total >= min_samples && self.fail_rate() >= fail_rate && self.sus_dests >= 2
    }

    /// Stronger passive signal: consecutive bad closes (multi-dest or long streak).
    pub fn hard_degraded(&self) -> bool {
        (self.consecutive_recent_sus >= 3 && self.sus_dests >= 2)
            || self.consecutive_recent_sus >= 5
    }
}
/// Skip redundant HTTP refresh when journal pushed a snapshot this recently.
const FRESH_SAMPLE: Duration = Duration::from_millis(250);

pub struct Runtime {
    pub core: CoreManager,
    pub system_proxy_on: bool,
    pub proxy_snapshot: Option<SystemProxySnapshot>,
    pub api: Option<ClashApi>,
    pub last_config_path: Option<PathBuf>,
    pub last_binary_path: Option<PathBuf>,
    system_proxy: Box<dyn SystemProxy>,
    traffic_prev: Option<(Instant, TrafficTotals)>,
    traffic_speed: (u64, u64),
    /// Live connections (last poll)
    live_connections: Vec<ConnectionInfo>,
    /// History of requests keyed by connection id (or synthetic key).
    request_by_id: HashMap<String, RequestRecord>,
    /// Newest ids at the front.
    request_order: VecDeque<String>,
    /// When journal / sample last applied a snapshot.
    last_sample_at: Option<Instant>,
    /// Monotonic journal sequence (opens).
    journal_seq: u64,
    /// Wall-clock start of current core session (unix secs).
    core_started_at: Option<i64>,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            core: CoreManager::default(),
            system_proxy_on: false,
            proxy_snapshot: None,
            api: None,
            last_config_path: None,
            last_binary_path: None,
            system_proxy: create_system_proxy(),
            traffic_prev: None,
            traffic_speed: (0, 0),
            live_connections: Vec::new(),
            request_by_id: HashMap::new(),
            request_order: VecDeque::new(),
            last_sample_at: None,
            journal_seq: 0,
            core_started_at: None,
        }
    }

    /// Clone of current Clash API client (for journal I/O outside the lock).
    pub fn api_clone(&self) -> Option<ClashApi> {
        self.api.clone()
    }

    pub fn sample_is_fresh(&self) -> bool {
        self.last_sample_at
            .map(|t| t.elapsed() < FRESH_SAMPLE)
            .unwrap_or(false)
    }

    pub fn status(&mut self, store: &AppStore) -> ProxyStatus {
        self.core.poll();
        // Core may have exited outside stop_proxy — keep uptime field consistent.
        if !self.core.is_running() {
            self.core_started_at = None;
        } else if self.core_started_at.is_none() {
            // Recover if we missed setting it (e.g. process still up after soft restart path).
            self.core_started_at = Some(now_unix_secs());
        }
        self.refresh_traffic_if_stale();
        ProxyStatus {
            running: self.core.is_running(),
            core_state: self.core.state(),
            system_proxy: self.system_proxy_on,
            tun_enabled: store.settings.tun_enabled,
            outbound_mode: store.settings.outbound_mode.as_str().to_string(),
            mixed_port: store.settings.mixed_port,
            api_port: store.settings.api_port,
            current_node_id: store.settings.current_node_id.clone(),
            error: self.core.last_error().map(|s| s.to_string()),
            core_path: self
                .last_binary_path
                .as_ref()
                .map(|p| p.display().to_string()),
            config_path: self
                .last_config_path
                .as_ref()
                .map(|p| p.display().to_string()),
            upload_speed: self.traffic_speed.0,
            download_speed: self.traffic_speed.1,
            upload_total: self
                .traffic_prev
                .as_ref()
                .map(|(_, t)| t.upload_total)
                .unwrap_or(0),
            download_total: self
                .traffic_prev
                .as_ref()
                .map(|(_, t)| t.download_total)
                .unwrap_or(0),
            connections: self
                .traffic_prev
                .as_ref()
                .map(|(_, t)| t.connections)
                .unwrap_or(0),
            smart_switch: store.settings.auto_select.is_smart(),
            auto_select: store.settings.auto_select.as_str().to_string(),
            core_started_at: self.core_started_at,
        }
    }

    /// Passive health for smart switch from connection journal (no MITM / no HTTP codes).
    ///
    /// Heuristic "suspicious": closed within 3s with almost no bytes — proxy path often
    /// dies before useful transfer. Multi-destination and consecutive tail reduce
    /// single-site false positives (docs/auto.md).
    pub fn passive_node_stats(&self, node_tag: &str, lookback_ms: i64) -> PassiveNodeStats {
        let now = now_unix_ms();
        let mut samples: Vec<(i64, bool, String)> = Vec::new(); // closed_at, sus, dest_key

        for rec in self.request_by_id.values() {
            if !rec.closed {
                continue;
            }
            let closed_at = rec.closed_at.unwrap_or(rec.last_seen);
            if now.saturating_sub(closed_at) > lookback_ms {
                continue;
            }
            let matches = rec.node == node_tag
                || rec.chains.iter().any(|c| c == node_tag)
                || (!node_tag.is_empty() && rec.node.contains(node_tag));
            if !matches {
                continue;
            }
            let dur = closed_at.saturating_sub(rec.first_seen);
            let sus = dur <= 3000 && rec.download < 1024 && rec.upload < 1024;
            let dest = if !rec.host.is_empty() {
                rec.host.clone()
            } else if !rec.destination.is_empty() && rec.destination != "—" {
                rec.destination.clone()
            } else {
                "unknown".into()
            };
            samples.push((closed_at, sus, dest));
        }

        samples.sort_by_key(|(t, _, _)| *t);

        let mut all_dests = HashSet::new();
        let mut sus_dests = HashSet::new();
        let mut suspicious = 0u32;
        for (_, sus, dest) in &samples {
            all_dests.insert(dest.clone());
            if *sus {
                suspicious = suspicious.saturating_add(1);
                sus_dests.insert(dest.clone());
            }
        }

        // Consecutive suspicious at the most recent end of the window.
        let mut consecutive_recent_sus = 0u32;
        for (_, sus, _) in samples.iter().rev() {
            if *sus {
                consecutive_recent_sus = consecutive_recent_sus.saturating_add(1);
            } else {
                break;
            }
        }

        PassiveNodeStats {
            total: samples.len() as u32,
            suspicious,
            dests: all_dests.len() as u32,
            sus_dests: sus_dests.len() as u32,
            consecutive_recent_sus,
        }
    }

    /// Apply a pre-fetched snapshot (journal / HTTP fallback). Prefer calling I/O outside the lock.
    pub fn apply_snapshot(&mut self, snap: crate::api::ConnectionsSnapshot) {
        let now = Instant::now();
        let totals = TrafficTotals {
            upload_total: snap.upload_total,
            download_total: snap.download_total,
            connections: snap.connections.len() as u32,
        };
        if let Some((prev_t, prev)) = &self.traffic_prev {
            let dt = now.duration_since(*prev_t).as_secs_f64();
            if dt > 0.05 {
                let up = totals.upload_total.saturating_sub(prev.upload_total);
                let down = totals.download_total.saturating_sub(prev.download_total);
                self.traffic_speed = ((up as f64 / dt) as u64, (down as f64 / dt) as u64);
            }
        }
        self.traffic_prev = Some((now, totals));
        self.ingest_connections(snap.connections);
        self.last_sample_at = Some(now);
    }

    /// HTTP sample when WebSocket journal is down (I/O happens inside — prefer journal).
    pub fn sample_connections_http(&mut self) {
        self.core.poll();
        let Some(api) = self.api.clone() else {
            self.traffic_prev = None;
            self.traffic_speed = (0, 0);
            self.live_connections.clear();
            return;
        };
        match api.list_connections() {
            Ok(s) => self.apply_snapshot(s),
            Err(e) => {
                eprintln!("[satelite] connections sample failed: {e}");
            }
        }
    }

    fn refresh_traffic_if_stale(&mut self) {
        if self.api.is_none() {
            self.traffic_prev = None;
            self.traffic_speed = (0, 0);
            self.live_connections.clear();
            return;
        }
        if self.sample_is_fresh() {
            return;
        }
        self.sample_connections_http();
    }

    /// Diff-based journal: upsert live, mark disappeared as closed.
    fn ingest_connections(&mut self, connections: Vec<ConnectionInfo>) {
        let now_ms = now_unix_ms();
        let mut seen: HashSet<String> = HashSet::with_capacity(connections.len());

        for c in &connections {
            let id = connection_history_key(c);
            seen.insert(id.clone());
            if let Some(rec) = self.request_by_id.get_mut(&id) {
                rec.last_seen = now_ms;
                rec.closed = false;
                rec.closed_at = None;
                rec.upload = c.upload.max(rec.upload);
                rec.download = c.download.max(rec.download);
                if !c.node.is_empty() && c.node != "—" {
                    rec.node = c.node.clone();
                }
                if !c.chains.is_empty() {
                    rec.chains = c.chains.clone();
                }
                if rec.destination == "—" && c.destination != "—" {
                    rec.destination = c.destination.clone();
                }
                if rec.host.is_empty() && !c.host.is_empty() {
                    rec.host = c.host.clone();
                }
                if rec.rule.is_empty() && !c.rule.is_empty() {
                    rec.rule = c.rule.clone();
                    rec.rule_payload = c.rule_payload.clone();
                }
                if rec.process.is_empty() && !c.process.is_empty() {
                    rec.process = c.process.clone();
                }
            } else {
                self.journal_seq = self.journal_seq.wrapping_add(1);
                let mut rec = RequestRecord::from_connection(c, now_ms);
                rec.id = id.clone();
                self.request_by_id.insert(id.clone(), rec);
                self.request_order.push_front(id);
                while self.request_order.len() > MAX_REQUEST_HISTORY {
                    if let Some(old) = self.request_order.pop_back() {
                        self.request_by_id.remove(&old);
                    }
                }
            }
        }

        // Connections that left the live snapshot → Closed event in journal.
        for prev in &self.live_connections {
            let id = connection_history_key(prev);
            if !seen.contains(&id) {
                if let Some(rec) = self.request_by_id.get_mut(&id) {
                    if !rec.closed {
                        rec.closed = true;
                        rec.closed_at = Some(now_ms);
                        rec.last_seen = now_ms;
                    }
                }
            }
        }

        self.live_connections = connections;
    }

    pub fn live_connections(&mut self, store: &AppStore) -> Vec<ConnectionView> {
        self.core.poll();
        self.refresh_traffic_if_stale();
        let tag_names = node_tag_name_map(store);
        self.live_connections
            .iter()
            .map(|c| ConnectionView::from_info(c, &tag_names))
            .collect()
    }

    pub fn request_history(
        &mut self,
        store: &AppStore,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Vec<ConnectionView> {
        self.core.poll();
        // Journal fills history continuously; only HTTP-refresh if stale.
        self.refresh_traffic_if_stale();
        let tag_names = node_tag_name_map(store);
        let q = query.unwrap_or("").trim();
        let limit = limit.unwrap_or(800).min(MAX_REQUEST_HISTORY);
        self.request_order
            .iter()
            .filter_map(|id| self.request_by_id.get(id))
            .filter(|r| r.closed)
            .filter(|r| r.matches_query(q))
            .take(limit)
            .map(|r| ConnectionView::from_record(r, &tag_names))
            .collect()
    }

    pub fn clear_request_history(&mut self) {
        // Keep active records so they can still transition into the closed
        // list when they disappear from a later connection snapshot.
        self.request_by_id.retain(|_, record| !record.closed);
        let active_ids: HashSet<String> = self.request_by_id.keys().cloned().collect();
        self.request_order.retain(|id| active_ids.contains(id));
    }

    pub fn clash_api_clone(&self) -> Option<ClashApi> {
        self.api_clone()
    }

    /// Generate config, start sing-box, optionally enable system proxy.
    pub fn start_proxy(
        &mut self,
        app_data_dir: &Path,
        resource_dir: Option<&Path>,
        store: &mut AppStore,
        enable_system_proxy: bool,
    ) -> AppResult<ProxyStatus> {
        self.core.poll();
        if self.core.is_running() {
            return Ok(self.status(store));
        }

        let nodes = store.enabled_nodes();
        if nodes.is_empty() {
            return Err(AppError::Core(
                "no nodes; import a subscription first".into(),
            ));
        }

        let (bin, _src) = resolve_core_bin(app_data_dir, resource_dir);
        let bin = bin.ok_or_else(|| AppError::Core("sing-box binary not found".into()))?;

        let secret = generate_api_secret();
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: store.settings.mixed_port,
                api_port: store.settings.api_port,
                api_secret: secret.clone(),
                current_node_id: store.settings.current_node_id.clone(),
                log_level: "info".into(),
                rules: store.enabled_rules_sorted(),
                tun_enabled: store.settings.tun_enabled,
                tun_stack: store.settings.tun_stack.clone(),
                dns: store.dns.clone(),
                outbound_mode: store.settings.outbound_mode,
                route_final: store.settings.route_final.clone(),
                auto_select: store.settings.auto_select,
                probe_url: store.settings.probe_url.clone(),
                find_process: store.settings.find_process,
            },
        )?;
        let config_path = write_active_config(app_data_dir, &built)?;
        store.settings.clash_api_secret = Some(secret.clone());
        if store.settings.current_node_id.is_none() {
            if let Some(first) = nodes.first() {
                store.settings.current_node_id = Some(first.id.clone());
            }
        }

        let log_dir = app_data_dir.join("logs");
        // TUN creates utun + routes → macOS setuid sing-box / Windows UAC.
        let elevated = store.settings.tun_enabled;
        self.core.start_with_ports(
            &bin,
            &config_path,
            &log_dir,
            store.settings.mixed_port,
            Some(store.settings.api_port),
            elevated,
            resource_dir,
        )?;
        self.last_config_path = Some(config_path.clone());
        self.last_binary_path = Some(bin.clone());

        let api = ClashApi::new("127.0.0.1", store.settings.api_port, &secret);
        // TUN start can take a few seconds (utun + routes). Health uses a short
        // per-try timeout so we do not block the runtime lock for minutes.
        let attempts = if elevated { 50 } else { 30 }; // ~10s / ~6s
        let mut ok = false;
        for _ in 0..attempts {
            if api.health_ok() {
                ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            self.core.poll();
            if !self.core.is_running() {
                break;
            }
        }
        if !ok {
            let log_hint = self
                .core
                .last_error()
                .map(|s| s.to_string())
                .or_else(|| {
                    let log = log_dir.join("sing-box.log");
                    std::fs::read(&log).ok().and_then(|b| {
                        let s = String::from_utf8_lossy(&b);
                        let tail: String = s
                            .chars()
                            .rev()
                            .take(1200)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect();
                        let cleaned = tail.replace('\0', "");
                        if cleaned.trim().is_empty() {
                            None
                        } else {
                            Some(cleaned)
                        }
                    })
                })
                .unwrap_or_default();
            let _ = self.core.stop();
            let detail = if log_hint.is_empty() {
                format!(
                    "sing-box started but clash_api not responding at 127.0.0.1:{}",
                    store.settings.api_port
                )
            } else {
                format!(
                    "sing-box started but clash_api not responding at 127.0.0.1:{}\n--- log ---\n{log_hint}",
                    store.settings.api_port
                )
            };
            return Err(AppError::Core(detail));
        }
        self.api = Some(api);
        self.core_started_at = Some(now_unix_secs());

        // System proxy is independent — optional on start; prefer UI switch after running.
        if enable_system_proxy {
            if let Err(e) = self.set_system_proxy(store, true) {
                // Core stays up; surface error to caller as soft fail via Err still?
                // Keep core running: return Ok and leave system_proxy off.
                let _ = e;
            }
        }

        Ok(self.status(store))
    }

    /// Toggle system HTTP(S)/SOCKS proxy independently of core running state.
    pub fn set_system_proxy(&mut self, store: &AppStore, enabled: bool) -> AppResult<ProxyStatus> {
        self.core.poll();
        if enabled == self.system_proxy_on {
            return Ok(self.status(store));
        }
        if enabled {
            let snap = self
                .system_proxy
                .enable("127.0.0.1", store.settings.mixed_port)?;
            self.proxy_snapshot = Some(snap);
            self.system_proxy_on = true;
        } else {
            let _ = self.system_proxy.disable(self.proxy_snapshot.as_ref());
            self.system_proxy_on = false;
            self.proxy_snapshot = None;
        }
        Ok(self.status(store))
    }

    pub fn stop_proxy(&mut self, store: &AppStore) -> AppResult<ProxyStatus> {
        // System proxy is independent — do not turn it off when stopping core.
        self.core.stop()?;
        self.api = None;
        self.core_started_at = None;
        self.live_connections.clear();
        // keep request_history across stop so user can review
        Ok(self.status(store))
    }

    pub fn restart_core(
        &mut self,
        app_data_dir: &Path,
        resource_dir: Option<&Path>,
        store: &mut AppStore,
    ) -> AppResult<ProxyStatus> {
        let sys = self.system_proxy_on;
        let _ = self.stop_proxy(store);
        self.start_proxy(app_data_dir, resource_dir, store, sys)
    }

    pub fn select_node_live(&self, node_tag: &str) -> AppResult<()> {
        self.select_group_live("proxy", node_tag)
    }

    /// Hot-select outbound `node_tag` inside selector group (main `proxy` or smart-*).
    pub fn select_group_live(&self, group: &str, node_tag: &str) -> AppResult<()> {
        let api = self
            .api
            .as_ref()
            .ok_or_else(|| AppError::Core("core not running".into()))?;
        api.select_proxy(group, node_tag)
    }

    /// Full cleanup on app exit: system proxy off, kill core, free listen ports.
    pub fn shutdown_with_ports(&mut self, ports: &[u16]) {
        if self.system_proxy_on {
            let _ = self.system_proxy.disable(self.proxy_snapshot.as_ref());
            self.system_proxy_on = false;
            self.proxy_snapshot = None;
        }
        self.core.force_shutdown(ports);
        self.api = None;
        self.live_connections.clear();
        self.traffic_prev = None;
        self.traffic_speed = (0, 0);
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Stable history key: prefer clash id; fall back so empty ids still accumulate.
fn connection_history_key(c: &ConnectionInfo) -> String {
    if !c.id.trim().is_empty() {
        return c.id.clone();
    }
    format!(
        "{}|{}|{}|{}|{}",
        c.network, c.destination, c.source, c.process, c.start
    )
}

fn node_tag_name_map(store: &AppStore) -> HashMap<String, String> {
    store
        .enabled_nodes()
        .into_iter()
        .map(|n| (outbound_tag(&n), n.name))
        .collect()
}

/// UI-facing connection / request row.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionView {
    pub id: String,
    pub destination: String,
    pub host: String,
    pub network: String,
    pub conn_type: String,
    /// Raw tag or chain leaf
    pub node_tag: String,
    /// Human node name when known
    pub node_name: String,
    pub chains: Vec<String>,
    pub chains_display: String,
    pub rule: String,
    pub rule_payload: String,
    pub process: String,
    pub source: String,
    pub upload: u64,
    pub download: u64,
    pub start: String,
    pub first_seen: Option<i64>,
    pub last_seen: Option<i64>,
    pub closed: bool,
    pub closed_at: Option<i64>,
}

impl ConnectionView {
    fn from_info(c: &ConnectionInfo, tag_names: &HashMap<String, String>) -> Self {
        let node_name = tag_names
            .get(&c.node)
            .cloned()
            .unwrap_or_else(|| c.node.clone());
        let chains_display = c.chains.join(" → ");
        Self {
            id: c.id.clone(),
            destination: c.destination.clone(),
            host: c.host.clone(),
            network: c.network.clone(),
            conn_type: c.conn_type.clone(),
            node_tag: c.node.clone(),
            node_name,
            chains: c.chains.clone(),
            chains_display,
            rule: format_rule(&c.rule, &c.rule_payload),
            rule_payload: c.rule_payload.clone(),
            process: c.process.clone(),
            source: c.source.clone(),
            upload: c.upload,
            download: c.download,
            start: c.start.clone(),
            first_seen: None,
            last_seen: None,
            closed: false,
            closed_at: None,
        }
    }

    fn from_record(r: &RequestRecord, tag_names: &HashMap<String, String>) -> Self {
        let node_name = tag_names
            .get(&r.node)
            .cloned()
            .unwrap_or_else(|| r.node.clone());
        Self {
            id: r.id.clone(),
            destination: r.destination.clone(),
            host: r.host.clone(),
            network: r.network.clone(),
            conn_type: r.conn_type.clone(),
            node_tag: r.node.clone(),
            node_name,
            chains: r.chains.clone(),
            chains_display: r.chains.join(" → "),
            rule: format_rule(&r.rule, &r.rule_payload),
            rule_payload: r.rule_payload.clone(),
            process: r.process.clone(),
            source: r.source.clone(),
            upload: r.upload,
            download: r.download,
            start: String::new(),
            first_seen: Some(r.first_seen),
            last_seen: Some(r.last_seen),
            closed: r.closed,
            closed_at: r.closed_at,
        }
    }
}

fn format_rule(rule: &str, payload: &str) -> String {
    if rule.is_empty() && payload.is_empty() {
        return "—".into();
    }
    if payload.is_empty() {
        return rule.to_string();
    }
    if rule.is_empty() {
        return payload.to_string();
    }
    format!("{rule}({payload})")
}

//! Orchestrates core + system proxy.

use crate::api::{ClashApi, ConnectionInfo, RequestRecord, TrafficTotals};
use crate::config::{
    build_singbox_config, generate_api_secret, inspect_singbox_config, outbound_tag,
    write_active_config, write_custom_config, BuildOptions,
};
use crate::domain::{RuntimeSource, SubscriptionSource};
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
    /// Persisted desired capture mode: off | system | tun.
    pub capture_mode: String,
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
    /// `generated` or `singbox`.
    #[serde(default)]
    pub runtime_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_profile_name: Option<String>,
    #[serde(default)]
    pub custom_has_clash_api: bool,
    #[serde(default)]
    pub custom_has_tun: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_inbound_port: Option<u16>,
}

/// Cap history to limit RAM (UI only needs recent activity).
const MAX_REQUEST_HISTORY: usize = 3_000;
const MAX_LIVE_REMOVAL_HISTORY: usize = 10_000;

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
    live_revision: u64,
    live_item_revisions: HashMap<String, u64>,
    live_removals: VecDeque<(u64, String)>,
    live_diff_floor: u64,
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
    /// Listen port taken from a user sing-box file (never from settings).
    custom_inbound_port: Option<u16>,
    custom_has_clash_api: bool,
    custom_has_tun: bool,
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
            live_revision: 0,
            live_item_revisions: HashMap::new(),
            live_removals: VecDeque::new(),
            live_diff_floor: 0,
            request_by_id: HashMap::new(),
            request_order: VecDeque::new(),
            last_sample_at: None,
            journal_seq: 0,
            core_started_at: None,
            custom_inbound_port: None,
            custom_has_clash_api: false,
            custom_has_tun: false,
        }
    }

    /// Clone of current Clash API client (for journal I/O outside the lock).
    pub fn api_clone(&self) -> Option<ClashApi> {
        self.api.clone()
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
        ProxyStatus {
            running: self.core.is_running(),
            core_state: self.core.state(),
            system_proxy: self.system_proxy_on,
            tun_enabled: store.settings.tun_enabled,
            capture_mode: store.settings.capture_mode.as_str().to_string(),
            outbound_mode: store.settings.outbound_mode.as_str().to_string(),
            mixed_port: self
                .custom_inbound_port
                .unwrap_or(store.settings.mixed_port),
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
            runtime_source: match store.settings.runtime_source() {
                crate::domain::RuntimeSource::Generated => "generated".into(),
                crate::domain::RuntimeSource::Singbox { .. } => "singbox".into(),
            },
            runtime_profile_id: store
                .settings
                .runtime_source()
                .singbox_id()
                .map(ToString::to_string),
            runtime_profile_name: store.settings.runtime_source().singbox_id().and_then(|id| {
                store
                    .subscriptions
                    .iter()
                    .find(|s| s.id == id)
                    .map(|s| s.name.clone())
            }),
            custom_has_clash_api: self.custom_has_clash_api,
            custom_has_tun: self.custom_has_tun,
            custom_inbound_port: self.custom_inbound_port,
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
                        self.journal_seq = self.journal_seq.saturating_add(1);
                        rec.history_seq = self.journal_seq;
                        rec.closed = true;
                        rec.closed_at = Some(now_ms);
                        rec.last_seen = now_ms;
                    }
                }
            }
        }

        if self.live_connections != connections {
            self.live_revision = self.live_revision.saturating_add(1);
            let revision = self.live_revision;
            let previous: HashMap<String, &ConnectionInfo> = self
                .live_connections
                .iter()
                .map(|connection| (connection_history_key(connection), connection))
                .collect();
            for connection in &connections {
                let id = connection_history_key(connection);
                if previous.get(&id).is_none_or(|old| *old != connection) {
                    self.live_item_revisions.insert(id, revision);
                }
            }
            for id in previous.keys() {
                if !seen.contains(id) {
                    self.live_item_revisions.remove(id);
                    self.live_removals.push_back((revision, id.clone()));
                }
            }
            while self.live_removals.len() > MAX_LIVE_REMOVAL_HISTORY {
                if let Some((removed_revision, _)) = self.live_removals.pop_front() {
                    self.live_diff_floor = removed_revision;
                }
            }
            self.live_connections = connections;
        }
    }

    pub fn live_connections(&mut self, store: &AppStore) -> Vec<ConnectionView> {
        self.core.poll();
        let tag_info = node_tag_info_map(store);
        self.live_connections
            .iter()
            .map(|c| ConnectionView::from_info(c, &tag_info))
            .collect()
    }

    pub fn live_connection_batch(
        &mut self,
        store: &AppStore,
        since_revision: Option<u64>,
    ) -> LiveConnectionBatch {
        self.core.poll();
        if since_revision == Some(self.live_revision) {
            return LiveConnectionBatch {
                rows: Vec::new(),
                removed_ids: Vec::new(),
                order_ids: Vec::new(),
                revision: self.live_revision,
                unchanged: true,
                full: false,
            };
        }
        let full = since_revision.is_none_or(|since| since < self.live_diff_floor);
        let since = since_revision.unwrap_or(0);
        let tag_info = node_tag_info_map(store);
        LiveConnectionBatch {
            rows: self
                .live_connections
                .iter()
                .filter(|connection| {
                    full || self
                        .live_item_revisions
                        .get(&connection_history_key(connection))
                        .is_some_and(|revision| *revision > since)
                })
                .map(|connection| ConnectionView::from_info(connection, &tag_info))
                .collect(),
            removed_ids: if full {
                Vec::new()
            } else {
                self.live_removals
                    .iter()
                    .filter(|(revision, _)| *revision > since)
                    .map(|(_, id)| id.clone())
                    .collect()
            },
            order_ids: self
                .live_connections
                .iter()
                .map(connection_history_key)
                .collect(),
            revision: self.live_revision,
            unchanged: false,
            full,
        }
    }

    pub fn request_history(
        &mut self,
        store: &AppStore,
        query: Option<&str>,
        limit: Option<usize>,
        after_seq: Option<u64>,
    ) -> RequestBatch {
        self.request_batch(store, query, limit, after_seq, false)
    }

    /// Closed requests that look like failures / timeouts: short-lived (≤ 3s)
    /// with almost no bytes transferred — same heuristic used by the passive
    /// smart-switch health check (see `passive_node_stats`).
    pub fn request_failures(
        &mut self,
        store: &AppStore,
        query: Option<&str>,
        limit: Option<usize>,
        after_seq: Option<u64>,
    ) -> RequestBatch {
        self.request_batch(store, query, limit, after_seq, true)
    }

    fn request_batch(
        &mut self,
        store: &AppStore,
        query: Option<&str>,
        limit: Option<usize>,
        after_seq: Option<u64>,
        failures_only: bool,
    ) -> RequestBatch {
        self.core.poll();
        let tag_info = node_tag_info_map(store);
        let q = query.unwrap_or("").trim();
        let limit = limit.unwrap_or(800).min(MAX_REQUEST_HISTORY);
        let is_failure = |record: &RequestRecord| {
            if !failures_only {
                return true;
            }
            let closed_at = record.closed_at.unwrap_or(record.last_seen);
            let duration = closed_at.saturating_sub(record.first_seen);
            duration <= 3000 && record.download < 1024 && record.upload < 1024
        };

        if let Some(after_seq) = after_seq {
            let mut records: Vec<&RequestRecord> = self
                .request_by_id
                .values()
                .filter(|record| record.closed && record.history_seq > after_seq)
                .collect();
            records.sort_unstable_by_key(|record| record.history_seq);
            let mut entries = Vec::new();
            let mut cursor = after_seq;
            let mut hit_limit = false;
            for record in records {
                cursor = record.history_seq;
                if is_failure(record) && record.matches_query(q) {
                    entries.push(ConnectionView::from_record(record, &tag_info));
                    if entries.len() >= limit {
                        hit_limit = true;
                        break;
                    }
                }
            }
            if !hit_limit {
                cursor = self.journal_seq;
            }
            return RequestBatch { entries, cursor };
        }

        let entries = self
            .request_order
            .iter()
            .filter_map(|id| self.request_by_id.get(id))
            .filter(|record| record.closed)
            .filter(|record| is_failure(record))
            .filter(|record| record.matches_query(q))
            .take(limit)
            .map(|record| ConnectionView::from_record(record, &tag_info))
            .collect();
        RequestBatch {
            entries,
            cursor: self.journal_seq,
        }
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

        match store.settings.runtime_source() {
            RuntimeSource::Singbox { id } => {
                return self.start_custom_proxy(
                    app_data_dir,
                    resource_dir,
                    store,
                    &id,
                    enable_system_proxy,
                );
            }
            RuntimeSource::Generated => {}
        }

        self.custom_inbound_port = None;
        self.custom_has_clash_api = false;
        self.custom_has_tun = false;

        let nodes = store.enabled_nodes();
        if nodes.is_empty() {
            return Err(AppError::Core(
                "no nodes; import a subscription first".into(),
            ));
        }

        ensure_listen_port_available(store.settings.mixed_port, "Mixed")?;
        ensure_listen_port_available(store.settings.api_port, "Clash API")?;

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
                rule_sets: store.enabled_rule_sets(),
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
        let max_wait = if elevated {
            Duration::from_secs(10)
        } else {
            Duration::from_secs(6)
        };
        let wait_started = Instant::now();
        let mut ok = false;
        while wait_started.elapsed() < max_wait {
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
                    self.core
                        .log_path()
                        .and_then(|log| std::fs::read(log).ok())
                        .and_then(|b| {
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

    fn start_custom_proxy(
        &mut self,
        app_data_dir: &Path,
        resource_dir: Option<&Path>,
        store: &mut AppStore,
        profile_id: &str,
        enable_system_proxy: bool,
    ) -> AppResult<ProxyStatus> {
        let (name, content) = store
            .subscriptions
            .iter()
            .find(|s| s.id == profile_id)
            .and_then(|s| match &s.source {
                SubscriptionSource::Singbox { content } => Some((s.name.clone(), content.clone())),
                _ => None,
            })
            .ok_or_else(|| AppError::Core("selected sing-box profile was not found".into()))?;
        let _ = name;

        crate::subscription::validate_complete_singbox_config(&content)?;
        let insight = inspect_singbox_config(&content);
        let config_path = write_custom_config(app_data_dir, profile_id, &content)?;

        if let Some(port) = insight.inbound_port {
            ensure_listen_port_available(port, "Inbound")?;
        }
        if let Some(port) = insight.clash_api_port {
            ensure_listen_port_available(port, "Clash API")?;
        }

        let (bin, _src) = resolve_core_bin(app_data_dir, resource_dir);
        let bin = bin.ok_or_else(|| AppError::Core("sing-box binary not found".into()))?;

        let log_dir = app_data_dir.join("logs");
        let elevated = insight.has_tun;
        self.core.start_with_ports(
            &bin,
            &config_path,
            &log_dir,
            insight.inbound_port.unwrap_or(0),
            insight.clash_api_port,
            elevated,
            resource_dir,
        )?;
        self.last_config_path = Some(config_path.clone());
        self.last_binary_path = Some(bin.clone());
        self.custom_inbound_port = insight.inbound_port;
        self.custom_has_clash_api = insight.has_clash_api();
        self.custom_has_tun = insight.has_tun;

        if insight.has_clash_api() {
            let host = insight
                .clash_api_host
                .as_deref()
                .unwrap_or("127.0.0.1");
            let port = insight.clash_api_port.unwrap_or(9090);
            let secret = insight.clash_api_secret.clone().unwrap_or_default();
            let api = ClashApi::new(host, port, &secret);
            let max_wait = if elevated {
                Duration::from_secs(10)
            } else {
                Duration::from_secs(6)
            };
            let wait_started = Instant::now();
            let mut ok = false;
            while wait_started.elapsed() < max_wait {
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
                let _ = self.core.stop();
                return Err(AppError::Core(format!(
                    "sing-box started but clash_api not responding at {host}:{port}"
                )));
            }
            self.api = Some(api);
            store.settings.clash_api_secret = if secret.is_empty() {
                None
            } else {
                Some(secret)
            };
        } else {
            self.api = None;
            let wait_started = Instant::now();
            let mut ok = false;
            while wait_started.elapsed() < Duration::from_secs(4) {
                self.core.poll();
                if self.core.is_running() {
                    ok = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if !ok {
                let log_hint = self.core.last_error().unwrap_or_default();
                return Err(AppError::Core(format!(
                    "sing-box failed to stay running{hint}",
                    hint = if log_hint.is_empty() {
                        String::new()
                    } else {
                        format!(": {log_hint}")
                    }
                )));
            }
        }
        self.core_started_at = Some(now_unix_secs());

        if enable_system_proxy {
            if insight.inbound_port.is_some() {
                let _ = self.set_system_proxy(store, true);
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
            let port = if store.settings.runtime_source().is_custom() {
                self.custom_inbound_port.ok_or_else(|| {
                    AppError::Core(
                        "当前自写配置没有 mixed/http/socks inbound，无法开启系统代理".into(),
                    )
                })?
            } else {
                store.settings.mixed_port
            };
            let snap = self.system_proxy.enable("127.0.0.1", port)?;
            self.proxy_snapshot = Some(snap);
            self.system_proxy_on = true;
        } else {
            // Only clear the in-memory state after the operating-system proxy
            // was actually restored. Otherwise the UI would report success
            // while the machine can still be pointing at our local port.
            self.system_proxy.disable(self.proxy_snapshot.as_ref())?;
            self.system_proxy_on = false;
            self.proxy_snapshot = None;
        }
        Ok(self.status(store))
    }

    /// Stop only the managed sing-box process.
    ///
    fn clear_live_connections(&mut self) {
        if self.live_connections.is_empty() {
            return;
        }
        self.live_revision = self.live_revision.saturating_add(1);
        let revision = self.live_revision;
        for connection in &self.live_connections {
            let id = connection_history_key(connection);
            self.live_item_revisions.remove(&id);
            self.live_removals.push_back((revision, id));
        }
        self.live_connections.clear();
        while self.live_removals.len() > MAX_LIVE_REMOVAL_HISTORY {
            if let Some((removed_revision, _)) = self.live_removals.pop_front() {
                self.live_diff_floor = removed_revision;
            }
        }
    }

    /// Internal restarts deliberately use this path so the saved/effective
    /// system-proxy state survives the short process replacement. A user
    /// initiated stop must use `stop_proxy`, which restores the OS first.
    fn stop_core(&mut self, _store: &AppStore) -> AppResult<()> {
        if let Some(api) = self.api.take() {
            api.deactivate();
        }
        self.core.stop()?;
        // `CoreManager::stop` waits for the process we actually own. Never
        // force-kill arbitrary listeners here: an empty/test runtime has no
        // ownership proof and could otherwise terminate another running app
        // instance (or an unrelated process using the configured ports).
        self.core_started_at = None;
        self.clear_live_connections();
        Ok(())
    }

    pub fn stop_proxy(&mut self, store: &AppStore) -> AppResult<ProxyStatus> {
        // Restore the OS proxy before releasing the local listener. If this
        // fails, keep the core alive: stopping it would strand the machine on
        // a dead 127.0.0.1 proxy and appear as a system-wide network outage.
        if self.system_proxy_on {
            self.system_proxy.disable(self.proxy_snapshot.as_ref())?;
            self.system_proxy_on = false;
            self.proxy_snapshot = None;
        }
        self.stop_core(store)?;
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
        self.stop_core(store)?;
        self.start_proxy(app_data_dir, resource_dir, store, sys)
    }

    /// Full cleanup on app exit: system proxy off and stop the managed core.
    /// Returns true when no owned OS proxy remains and the ownership marker
    /// can be cleared safely.
    pub fn shutdown(&mut self) -> bool {
        let proxy_cleared = if self.system_proxy_on {
            match self.system_proxy.disable(self.proxy_snapshot.as_ref()) {
                Ok(()) => {
                    self.system_proxy_on = false;
                    self.proxy_snapshot = None;
                    true
                }
                Err(error) => {
                    crate::app_log::error(
                        "system_proxy",
                        format!("shutdown restore failed: {error}"),
                    );
                    false
                }
            }
        } else {
            true
        };
        if let Some(api) = self.api.take() {
            api.deactivate();
        }
        self.core.force_shutdown();
        self.clear_live_connections();
        self.traffic_prev = None;
        self.traffic_speed = (0, 0);
        proxy_cleared
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

fn ensure_listen_port_available(port: u16, label: &str) -> AppResult<()> {
    if CoreManager::has_port_listener(port) {
        return Err(AppError::Core(format!(
            "{label} 端口 127.0.0.1:{port} 已被其他程序占用，请关闭冲突程序或修改端口"
        )));
    }
    Ok(())
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

/// Resolved node display info for a connection: human node name + owning
/// subscription name (e.g. "新加坡01" / "机场A").
struct NodeInfo {
    name: String,
    subscription: String,
}

/// Map outbound tag → resolved display info, using only enabled subscriptions.
fn node_tag_info_map(store: &AppStore) -> HashMap<String, NodeInfo> {
    let enabled: std::collections::HashSet<_> = store
        .subscriptions
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.id.as_str())
        .collect();
    // subscription id → name
    let sub_name: HashMap<&str, &str> = store
        .subscriptions
        .iter()
        .map(|s| (s.id.as_str(), s.name.as_str()))
        .collect();
    store
        .nodes
        .iter()
        .filter(|n| enabled.contains(n.subscription_id.as_str()))
        .map(|n| {
            let info = NodeInfo {
                name: n.node.name.clone(),
                subscription: sub_name
                    .get(n.subscription_id.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            };
            (outbound_tag(&n.node), info)
        })
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
    /// Owning subscription name (for tooltip: 订阅配置名 + 节点名称)
    #[serde(default)]
    pub subscription_name: String,
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveConnectionBatch {
    pub rows: Vec<ConnectionView>,
    pub removed_ids: Vec<String>,
    pub order_ids: Vec<String>,
    pub revision: u64,
    pub unchanged: bool,
    pub full: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RequestBatch {
    pub entries: Vec<ConnectionView>,
    pub cursor: u64,
}

impl ConnectionView {
    fn from_info(c: &ConnectionInfo, tag_info: &HashMap<String, NodeInfo>) -> Self {
        let info = tag_info.get(&c.node);
        let node_name = info
            .map(|i| i.name.clone())
            .unwrap_or_else(|| c.node.clone());
        let subscription_name = info.map(|i| i.subscription.clone()).unwrap_or_default();
        let chains_display = c.chains.join(" → ");
        Self {
            id: connection_history_key(c),
            destination: c.destination.clone(),
            host: c.host.clone(),
            network: c.network.clone(),
            conn_type: c.conn_type.clone(),
            node_tag: c.node.clone(),
            node_name,
            subscription_name,
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

    fn from_record(r: &RequestRecord, tag_info: &HashMap<String, NodeInfo>) -> Self {
        let info = tag_info.get(&r.node);
        let node_name = info
            .map(|i| i.name.clone())
            .unwrap_or_else(|| r.node.clone());
        let subscription_name = info.map(|i| i.subscription.clone()).unwrap_or_default();
        Self {
            id: r.id.clone(),
            destination: r.destination.clone(),
            host: r.host.clone(),
            network: r.network.clone(),
            conn_type: r.conn_type.clone(),
            node_tag: r.node.clone(),
            node_name,
            subscription_name,
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

#[cfg(test)]
mod stop_behavior_tests {
    use super::*;
    use crate::proxy::{SystemProxy, SystemProxySnapshot};
    use std::sync::{Arc, Mutex};

    struct RecordingSystemProxy {
        disabled: Arc<Mutex<usize>>,
        fail_disable: bool,
    }

    impl SystemProxy for RecordingSystemProxy {
        fn enable(&self, _host: &str, _port: u16) -> AppResult<SystemProxySnapshot> {
            Ok(SystemProxySnapshot {
                detail: "test".into(),
            })
        }

        fn disable(&self, _snapshot: Option<&SystemProxySnapshot>) -> AppResult<()> {
            *self.disabled.lock().expect("disabled counter") += 1;
            if self.fail_disable {
                Err(AppError::Core("restore failed".into()))
            } else {
                Ok(())
            }
        }

        fn detect_owned(&self, _host: &str, _port: u16) -> AppResult<Option<SystemProxySnapshot>> {
            Ok(None)
        }
    }

    fn runtime_with_system_proxy(fail_disable: bool) -> (Runtime, Arc<Mutex<usize>>) {
        let disabled = Arc::new(Mutex::new(0));
        let mut runtime = Runtime::new();
        runtime.system_proxy = Box::new(RecordingSystemProxy {
            disabled: Arc::clone(&disabled),
            fail_disable,
        });
        runtime.system_proxy_on = true;
        runtime.proxy_snapshot = Some(SystemProxySnapshot {
            detail: "previous system proxy".into(),
        });
        (runtime, disabled)
    }

    fn closed_record(id: &str, history_seq: u64, host: &str) -> RequestRecord {
        RequestRecord {
            id: id.into(),
            history_seq,
            destination: format!("{host}:443"),
            host: host.into(),
            network: "tcp".into(),
            conn_type: "http".into(),
            node: "proxy".into(),
            chains: Vec::new(),
            rule: String::new(),
            rule_payload: String::new(),
            process: String::new(),
            source: String::new(),
            upload: 10,
            download: 10,
            first_seen: 1_000,
            last_seen: 1_100,
            closed: true,
            closed_at: Some(1_100),
        }
    }

    #[test]
    fn request_incremental_cursor_pages_without_skipping() {
        let store = AppStore::default();
        let mut runtime = Runtime::new();
        runtime.journal_seq = 3;
        for (id, seq, host) in [
            ("one", 1, "match.example"),
            ("two", 2, "other.example"),
            ("three", 3, "match.example"),
        ] {
            runtime
                .request_by_id
                .insert(id.into(), closed_record(id, seq, host));
        }

        let first = runtime.request_history(&store, None, Some(1), Some(0));
        assert_eq!(first.entries.len(), 1);
        assert_eq!(first.cursor, 1);
        let second = runtime.request_history(&store, None, Some(1), Some(first.cursor));
        assert_eq!(second.entries.len(), 1);
        assert_eq!(second.cursor, 2);

        let filtered = runtime.request_history(&store, Some("match"), Some(10), Some(1));
        assert_eq!(filtered.entries.len(), 1);
        assert_eq!(filtered.entries[0].id, "three");
        assert_eq!(filtered.cursor, 3);
    }

    #[test]
    fn user_stop_restores_system_proxy_before_reporting_stopped() {
        let store = AppStore::default();
        let (mut runtime, disabled) = runtime_with_system_proxy(false);

        let status = runtime.stop_proxy(&store).expect("stop proxy");

        assert_eq!(*disabled.lock().expect("disabled counter"), 1);
        assert!(!runtime.system_proxy_on);
        assert!(runtime.proxy_snapshot.is_none());
        assert!(!status.system_proxy);
        assert!(!status.running);
    }

    #[test]
    fn user_stop_does_not_clear_state_when_system_proxy_restore_fails() {
        let store = AppStore::default();
        let (mut runtime, disabled) = runtime_with_system_proxy(true);

        let error = runtime.stop_proxy(&store).expect_err("restore must fail");

        assert!(error.to_string().contains("restore failed"));
        assert_eq!(*disabled.lock().expect("disabled counter"), 1);
        assert!(runtime.system_proxy_on);
        assert!(runtime.proxy_snapshot.is_some());
    }

    #[test]
    fn shutdown_reports_whether_system_proxy_was_cleared() {
        let (mut successful, _) = runtime_with_system_proxy(false);
        assert!(successful.shutdown());
        assert!(!successful.system_proxy_on);

        let (mut failed, _) = runtime_with_system_proxy(true);
        assert!(!failed.shutdown());
        assert!(failed.system_proxy_on);
        assert!(failed.proxy_snapshot.is_some());
    }

    #[test]
    fn status_never_waits_for_clash_api_network_io() {
        let store = AppStore::default();
        let mut runtime = Runtime::new();
        runtime.api = Some(ClashApi::new("192.0.2.1", 9, "unreachable"));

        let started = Instant::now();
        let _ = runtime.status(&store);

        assert!(
            started.elapsed() < Duration::from_millis(100),
            "status must remain a memory-only operation"
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn stop_then_start_accepts_the_same_ports() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut holder = std::process::Command::new("/usr/bin/nc")
            .args(["-l", "127.0.0.1", &port.to_string()])
            .spawn()
            .unwrap();
        for _ in 0..20 {
            if CoreManager::has_port_listener(port) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(ensure_listen_port_available(port, "test").is_err());

        let mut store = AppStore::default();
        store.settings.mixed_port = port;
        store.settings.api_port = 0;
        let mut runtime = Runtime::new();
        let api = ClashApi::new("127.0.0.1", port, "test");
        runtime.api = Some(api.clone());
        runtime.stop_proxy(&store).unwrap();
        let restart_allowed = ensure_listen_port_available(port, "test").is_ok();
        let _ = holder.kill();
        let _ = holder.wait();

        assert!(restart_allowed, "stop must allow an immediate restart");
        assert!(!api.is_active(), "stop must cancel Clash API clients");
    }
}

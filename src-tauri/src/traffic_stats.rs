//! Session traffic accounting. Never infer proxy bytes as total minus direct:
//! short connections missing from Clash snapshots remain explicitly unclassified.

use crate::api::{ConnectionInfo, TrafficTotals};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;

const MAX_TRACKED_CONNECTIONS: usize = 16_384;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrafficBytes {
    pub upload: u64,
    pub download: u64,
}

impl TrafficBytes {
    fn from_totals(totals: &TrafficTotals) -> Self {
        Self {
            upload: totals.upload_total,
            download: totals.download_total,
        }
    }

    fn delta(self, previous: Self) -> Self {
        Self {
            upload: self.upload.saturating_sub(previous.upload),
            download: self.download.saturating_sub(previous.download),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassifiedTotals {
    pub proxy: TrafficBytes,
    pub direct: TrafficBytes,
    pub unclassified: TrafficBytes,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct TrafficBucket {
    pub upload_speed: u64,
    pub download_speed: u64,
    pub upload_total: u64,
    pub download_total: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TrafficBreakdown {
    pub source: &'static str,
    pub proxy: TrafficBucket,
    pub direct: TrafficBucket,
    pub unclassified: TrafficBucket,
}

impl Default for TrafficBreakdown {
    fn default() -> Self {
        Self {
            source: "unavailable",
            proxy: TrafficBucket::default(),
            direct: TrafficBucket::default(),
            unclassified: TrafficBucket::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Proxy,
    Direct,
    Unknown,
}

fn connection_scope(chains: &[String]) -> Scope {
    // Rejected/ambiguous paths do not become proxy traffic. Intermediate
    // selector names alone are not enough to classify an actual outbound.
    if chains.iter().any(|tag| {
        ["block", "reject", "reject-drop"]
            .iter()
            .any(|v| tag.eq_ignore_ascii_case(v))
    }) {
        return Scope::Unknown;
    }
    let direct = chains.iter().any(|tag| tag.eq_ignore_ascii_case("direct"));
    let proxy = chains.iter().any(|tag| tag.starts_with("node-"));
    match (proxy, direct) {
        (true, false) => Scope::Proxy,
        (false, true) => Scope::Direct,
        _ => Scope::Unknown,
    }
}

#[derive(Default)]
pub struct TrafficAccounting {
    snapshot: TrafficBreakdown,
    previous: Option<(Instant, TrafficBytes)>,
    observed: HashMap<(String, String), TrafficBytes>,
}

impl TrafficAccounting {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn snapshot(&self, running: bool) -> TrafficBreakdown {
        let mut result = self.snapshot.clone();
        if !running {
            for bucket in [
                &mut result.proxy,
                &mut result.direct,
                &mut result.unclassified,
            ] {
                bucket.upload_speed = 0;
                bucket.download_speed = 0;
            }
        }
        result
    }

    fn begin(&mut self, totals: TrafficBytes, source: &'static str) {
        if self.snapshot.source != source
            || self.previous.as_ref().is_some_and(|(_, prev)| {
                totals.upload < prev.upload || totals.download < prev.download
            })
        {
            self.reset();
        }
        self.snapshot.source = source;
    }

    /// Exact per-outbound counters (generated Xray config). All unknown tags
    /// remain separate; companion cores are not added a second time.
    pub fn apply_outbound(
        &mut self,
        totals: &TrafficTotals,
        counters: &ClassifiedTotals,
        now: Instant,
    ) {
        let total = TrafficBytes::from_totals(totals);
        self.begin(total, "outbound");
        // Invalid/overlapping input is kept unknown, not clipped to a guessed category.
        let up_valid = counters
            .proxy
            .upload
            .checked_add(counters.direct.upload)
            .is_some_and(|v| v <= total.upload);
        let down_valid = counters
            .proxy
            .download
            .checked_add(counters.direct.download)
            .is_some_and(|v| v <= total.download);
        self.publish(
            total,
            TrafficBytes {
                upload: if up_valid { counters.proxy.upload } else { 0 },
                download: if down_valid {
                    counters.proxy.download
                } else {
                    0
                },
            },
            TrafficBytes {
                upload: if up_valid { counters.direct.upload } else { 0 },
                download: if down_valid {
                    counters.direct.download
                } else {
                    0
                },
            },
            now,
        );
    }

    /// Only classify observed connection increments within this frame's global
    /// byte budget. A newly seen old connection cannot retroactively turn old
    /// unknown traffic into a new speed spike.
    pub fn apply_connections(
        &mut self,
        totals: &TrafficTotals,
        connections: &[ConnectionInfo],
        generated: bool,
        now: Instant,
    ) {
        let total = TrafficBytes::from_totals(totals);
        self.begin(
            total,
            if generated {
                "connections"
            } else {
                "unavailable"
            },
        );
        let budget = total.delta(self.previous.map(|(_, t)| t).unwrap_or_default());
        let mut proxy = TrafficBytes::default();
        let mut direct = TrafficBytes::default();
        let mut next = HashMap::new();
        if generated {
            for conn in connections {
                if conn.id.trim().is_empty() || next.len() >= MAX_TRACKED_CONNECTIONS {
                    continue;
                }
                let key = (conn.id.clone(), conn.start.clone());
                if next.contains_key(&key) {
                    continue;
                }
                let baseline = self.observed.get(&key).copied();
                let previous = baseline.unwrap_or_default();
                let current = TrafficBytes {
                    upload: conn.upload.max(previous.upload),
                    download: conn.download.max(previous.download),
                };
                // A non-first-frame new/reappearing row can contain old bytes.
                // Establish its baseline without charging those bytes again.
                let delta = if self.previous.is_some() && baseline.is_none() {
                    TrafficBytes::default()
                } else {
                    current.delta(previous)
                };
                next.insert(key, current);
                let bucket = match connection_scope(&conn.chains) {
                    Scope::Proxy => &mut proxy,
                    Scope::Direct => &mut direct,
                    Scope::Unknown => continue,
                };
                bucket.upload = bucket.upload.saturating_add(delta.upload);
                bucket.download = bucket.download.saturating_add(delta.download);
            }
        }
        self.observed = next;
        // If snapshots disagree, this direction's whole increment stays unknown.
        let up_valid = proxy
            .upload
            .checked_add(direct.upload)
            .is_some_and(|v| v <= budget.upload);
        let down_valid = proxy
            .download
            .checked_add(direct.download)
            .is_some_and(|v| v <= budget.download);
        let proxy = TrafficBytes {
            upload: self
                .snapshot
                .proxy
                .upload_total
                .saturating_add(if up_valid { proxy.upload } else { 0 }),
            download: self
                .snapshot
                .proxy
                .download_total
                .saturating_add(if down_valid { proxy.download } else { 0 }),
        };
        let direct = TrafficBytes {
            upload: self
                .snapshot
                .direct
                .upload_total
                .saturating_add(if up_valid { direct.upload } else { 0 }),
            download: self
                .snapshot
                .direct
                .download_total
                .saturating_add(if down_valid { direct.download } else { 0 }),
        };
        self.publish(total, proxy, direct, now);
    }

    pub fn apply_unavailable(&mut self, totals: &TrafficTotals, now: Instant) {
        self.apply_connections(totals, &[], false, now);
    }

    fn publish(
        &mut self,
        total: TrafficBytes,
        proxy: TrafficBytes,
        direct: TrafficBytes,
        now: Instant,
    ) {
        let unknown = TrafficBytes {
            upload: total
                .upload
                .saturating_sub(proxy.upload)
                .saturating_sub(direct.upload),
            download: total
                .download
                .saturating_sub(proxy.download)
                .saturating_sub(direct.download),
        };
        let old = &self.snapshot;
        let elapsed = self
            .previous
            .map(|(at, _)| now.saturating_duration_since(at).as_secs_f64())
            .unwrap_or(0.0);
        let rate = |value: u64| {
            if elapsed > 0.05 {
                (value as f64 / elapsed) as u64
            } else {
                0
            }
        };
        let bucket = |value: TrafficBytes, before: &TrafficBucket| TrafficBucket {
            upload_total: value.upload,
            download_total: value.download,
            upload_speed: rate(value.upload.saturating_sub(before.upload_total)),
            download_speed: rate(value.download.saturating_sub(before.download_total)),
        };
        let mut proxy = bucket(proxy, &old.proxy);
        let mut direct = bucket(direct, &old.direct);
        let mut unclassified = bucket(unknown, &old.unclassified);
        let delta = total.delta(self.previous.map(|(_, t)| t).unwrap_or_default());
        let total_up_rate = rate(delta.upload);
        let total_down_rate = rate(delta.download);
        if proxy
            .upload_speed
            .checked_add(direct.upload_speed)
            .is_none_or(|v| v > total_up_rate)
        {
            proxy.upload_speed = 0;
            direct.upload_speed = 0;
        }
        if proxy
            .download_speed
            .checked_add(direct.download_speed)
            .is_none_or(|v| v > total_down_rate)
        {
            proxy.download_speed = 0;
            direct.download_speed = 0;
        }
        // Keep rounding consistent with the displayed total rate.
        unclassified.upload_speed = total_up_rate
            .saturating_sub(proxy.upload_speed)
            .saturating_sub(direct.upload_speed);
        unclassified.download_speed = total_down_rate
            .saturating_sub(proxy.download_speed)
            .saturating_sub(direct.download_speed);
        self.snapshot.proxy = proxy;
        self.snapshot.direct = direct;
        self.snapshot.unclassified = unclassified;
        self.previous = Some((now, total));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn totals(up: u64, down: u64) -> TrafficTotals {
        TrafficTotals {
            upload_total: up,
            download_total: down,
            connections: 0,
        }
    }
    fn conn(id: &str, chains: &[&str], up: u64, down: u64) -> ConnectionInfo {
        ConnectionInfo {
            id: id.into(),
            destination: String::new(),
            host: String::new(),
            destination_ip: String::new(),
            destination_port: String::new(),
            network: "tcp".into(),
            conn_type: String::new(),
            source: String::new(),
            process: String::new(),
            chains: chains.iter().map(|s| (*s).into()).collect(),
            node: String::new(),
            rule: String::new(),
            rule_payload: String::new(),
            upload: up,
            download: down,
            start: "session".into(),
        }
    }
    fn reconciles(s: &TrafficBreakdown, up: u64, down: u64) {
        assert_eq!(
            s.proxy.upload_total + s.direct.upload_total + s.unclassified.upload_total,
            up
        );
        assert_eq!(
            s.proxy.download_total + s.direct.download_total + s.unclassified.download_total,
            down
        );
    }

    #[test]
    fn observed_proxy_direct_and_missing_short_connections_reconcile() {
        let mut ledger = TrafficAccounting::default();
        let at = Instant::now();
        let conns = [
            conn("p", &["node-a", "proxy"], 10, 100),
            conn("d", &["DIRECT"], 20, 200),
        ];
        ledger.apply_connections(&totals(40, 400), &conns, true, at);
        let s = ledger.snapshot(true);
        reconciles(&s, 40, 400);
        assert_eq!(s.proxy.download_total, 100);
        assert_eq!(s.direct.download_total, 200);
        assert_eq!(s.unclassified.download_total, 100);
        ledger.apply_connections(
            &totals(100, 1000),
            &[
                conn("p", &["node-a", "proxy"], 30, 300),
                conn("d", &["direct"], 50, 500),
            ],
            true,
            at + Duration::from_secs(1),
        );
        let s = ledger.snapshot(true);
        reconciles(&s, 100, 1000);
        assert_eq!(
            (
                s.proxy.download_speed,
                s.direct.download_speed,
                s.unclassified.download_speed
            ),
            (200, 300, 100)
        );
        ledger.apply_connections(&totals(110, 1100), &[], true, at + Duration::from_secs(2));
        let s = ledger.snapshot(true);
        reconciles(&s, 110, 1100);
        assert_eq!(s.proxy.download_total, 300);
    }

    #[test]
    fn unknown_chains_and_rejected_paths_are_not_proxy() {
        let mut ledger = TrafficAccounting::default();
        let conns = [
            conn("1", &[], 1, 1),
            conn("2", &["custom-node"], 1, 1),
            conn("3", &["REJECT"], 1, 1),
            conn("4", &["node-a", "direct"], 1, 1),
        ];
        ledger.apply_connections(&totals(4, 4), &conns, true, Instant::now());
        assert_eq!(ledger.snapshot(true).unclassified.download_total, 4);
    }

    #[test]
    fn late_old_connection_does_not_reclassify_history_or_create_rate_spike() {
        let mut ledger = TrafficAccounting::default();
        let at = Instant::now();
        ledger.apply_connections(&totals(100, 100), &[], true, at);
        ledger.apply_connections(
            &totals(110, 110),
            &[conn("p", &["node-a"], 100, 100)],
            true,
            at + Duration::from_secs(1),
        );
        assert_eq!(ledger.snapshot(true).proxy.download_total, 0);
        ledger.apply_connections(
            &totals(120, 120),
            &[conn("p", &["node-a"], 110, 110)],
            true,
            at + Duration::from_secs(2),
        );
        let s = ledger.snapshot(true);
        assert_eq!(s.proxy.download_speed, 10);
        reconciles(&s, 120, 120);
    }

    #[test]
    fn duplicate_or_regressing_connection_counters_never_double_count() {
        let mut ledger = TrafficAccounting::default();
        let at = Instant::now();
        let c = conn("p", &["node-a"], 20, 20);
        ledger.apply_connections(&totals(20, 20), &[c.clone(), c], true, at);
        ledger.apply_connections(
            &totals(21, 21),
            &[conn("p", &["node-a"], 10, 10)],
            true,
            at + Duration::from_secs(1),
        );
        ledger.apply_connections(
            &totals(22, 22),
            &[conn("p", &["node-a"], 21, 21)],
            true,
            at + Duration::from_secs(2),
        );
        assert_eq!(ledger.snapshot(true).proxy.download_total, 21);
        reconciles(&ledger.snapshot(true), 22, 22);
    }

    #[test]
    fn global_counter_reset_starts_new_session_and_stop_zeroes_rates() {
        let mut ledger = TrafficAccounting::default();
        let at = Instant::now();
        ledger.apply_connections(
            &totals(100, 100),
            &[conn("p", &["node-a"], 100, 100)],
            true,
            at,
        );
        ledger.apply_connections(
            &totals(5, 5),
            &[conn("d", &["direct"], 5, 5)],
            true,
            at + Duration::from_secs(1),
        );
        let s = ledger.snapshot(false);
        assert_eq!(s.proxy.download_total, 0);
        assert_eq!(s.direct.download_total, 5);
        assert_eq!(s.direct.download_speed, 0);
        ledger.reset();
        assert_eq!(ledger.snapshot(true), TrafficBreakdown::default());
    }

    #[test]
    fn custom_configuration_does_not_guess_outbound_types_from_names() {
        let mut ledger = TrafficAccounting::default();
        ledger.apply_connections(
            &totals(5, 5),
            &[conn("p", &["node-a"], 5, 5)],
            false,
            Instant::now(),
        );
        let s = ledger.snapshot(true);
        assert_eq!(s.source, "unavailable");
        assert_eq!(s.unclassified.download_total, 5);
    }

    #[test]
    fn exact_outbound_counters_include_closed_traffic_and_reconcile_rates() {
        let mut ledger = TrafficAccounting::default();
        let at = Instant::now();
        let mut split = ClassifiedTotals {
            proxy: TrafficBytes {
                upload: 30,
                download: 300,
            },
            direct: TrafficBytes {
                upload: 20,
                download: 200,
            },
            unclassified: TrafficBytes {
                upload: 10,
                download: 100,
            },
        };
        ledger.apply_outbound(&totals(60, 600), &split, at);
        split.proxy.upload += 3;
        split.proxy.download += 30;
        split.direct.upload += 2;
        split.direct.download += 20;
        ledger.apply_outbound(&totals(66, 660), &split, at + Duration::from_secs(1));
        let s = ledger.snapshot(true);
        reconciles(&s, 66, 660);
        assert_eq!(
            s.proxy.download_speed + s.direct.download_speed + s.unclassified.download_speed,
            60
        );
    }

    #[test]
    fn over_budget_inputs_and_max_counters_remain_bounded() {
        let mut ledger = TrafficAccounting::default();
        let bad = ClassifiedTotals {
            proxy: TrafficBytes {
                upload: u64::MAX,
                download: u64::MAX,
            },
            direct: TrafficBytes {
                upload: 1,
                download: 1,
            },
            ..Default::default()
        };
        ledger.apply_outbound(&totals(u64::MAX, u64::MAX), &bad, Instant::now());
        let s = ledger.snapshot(true);
        assert_eq!(s.unclassified.download_total, u64::MAX);
        assert_eq!(s.proxy.download_total, 0);
    }

    #[test]
    fn partial_outbound_counter_replacement_cannot_exceed_total_speed() {
        let mut ledger = TrafficAccounting::default();
        let at = Instant::now();
        ledger.apply_outbound(
            &totals(100, 100),
            &ClassifiedTotals {
                proxy: TrafficBytes {
                    upload: 70,
                    download: 70,
                },
                direct: TrafficBytes {
                    upload: 30,
                    download: 30,
                },
                ..Default::default()
            },
            at,
        );
        ledger.apply_outbound(
            &totals(120, 120),
            &ClassifiedTotals {
                proxy: TrafficBytes {
                    upload: 60,
                    download: 60,
                },
                direct: TrafficBytes {
                    upload: 60,
                    download: 60,
                },
                ..Default::default()
            },
            at + Duration::from_secs(1),
        );
        let s = ledger.snapshot(true);
        reconciles(&s, 120, 120);
        assert_eq!(
            s.proxy.download_speed + s.direct.download_speed + s.unclassified.download_speed,
            20
        );
    }

    #[test]
    fn reappearing_or_late_rows_cannot_reclassify_old_bytes_even_with_room_in_budget() {
        let mut ledger = TrafficAccounting::default();
        let at = Instant::now();
        ledger.apply_connections(
            &totals(100, 100),
            &[conn("same", &["node-a"], 100, 100)],
            true,
            at,
        );
        ledger.apply_connections(&totals(110, 110), &[], true, at + Duration::from_secs(1));
        ledger.apply_connections(
            &totals(210, 210),
            &[conn("same", &["node-a"], 100, 100)],
            true,
            at + Duration::from_secs(2),
        );
        assert_eq!(ledger.snapshot(true).proxy.download_total, 100);
        ledger.apply_connections(
            &totals(310, 310),
            &[conn("late", &["node-a"], 50, 50)],
            true,
            at + Duration::from_secs(3),
        );
        assert_eq!(ledger.snapshot(true).proxy.download_total, 100);
        reconciles(&ledger.snapshot(true), 310, 310);
    }

    #[test]
    fn connection_tracking_is_bounded() {
        let mut ledger = TrafficAccounting::default();
        let connections: Vec<_> = (0..MAX_TRACKED_CONNECTIONS + 3)
            .map(|id| conn(&id.to_string(), &["node-a"], 1, 1))
            .collect();
        let count = connections.len() as u64;
        ledger.apply_connections(&totals(count, count), &connections, true, Instant::now());
        assert_eq!(ledger.observed.len(), MAX_TRACKED_CONNECTIONS);
        assert_eq!(ledger.snapshot(true).unclassified.download_total, 3);
    }
}

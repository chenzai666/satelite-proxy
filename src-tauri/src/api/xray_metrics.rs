//! Xray metrics client — polls the core's `metrics` module at
//! `GET /debug/vars` (Go expvar JSON). Xray has no Clash-compatible API:
//! per-connection snapshots, group selection and delay testing do not exist;
//! traffic totals per outbound tag are the only runtime signal.
//!
//! HTTP via **ureq** for the same nested-runtime reasons as `clash_api.rs`.

use crate::api::TrafficTotals;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct XrayMetrics {
    pub base: String,
    active: Arc<AtomicBool>,
    /// Previous per-outbound counter snapshot for the dominant-tag delta.
    /// Shared across clones of one session (same pattern as `active`).
    last_counters: Arc<Mutex<HashMap<String, TagCounters>>>,
}

fn shared_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .max_idle_connections(0)
            .timeout_connect(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
    })
}

impl XrayMetrics {
    pub fn new(host: &str, port: u16) -> Self {
        Self {
            base: format!("http://{host}:{port}"),
            active: Arc::new(AtomicBool::new(true)),
            last_counters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Clones from one core session share the same activity token.
    pub fn same_session(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.active, &other.active)
    }

    /// Fast readiness probe (short timeout). Used while waiting for core start.
    pub fn health_ok(&self) -> bool {
        shared_agent()
            .get(&format!("{}/debug/vars", self.base))
            .timeout(Duration::from_millis(350))
            .call()
            .map(|r| (200..300).contains(&r.status()))
            .unwrap_or(false)
    }

    /// Sum uplink/downlink counters across every outbound tag.
    ///
    /// Response shape (v2rayN `V2rayMetricsVars`):
    /// `{"stats": {"outbound": {"<tag>": {"uplink": n, "downlink": n}, …}}}`.
    /// Per-connection counts don't exist under Xray — `connections` stays 0
    /// and the connection pages render their "unsupported" state.
    pub fn traffic_totals(&self) -> Option<TrafficTotals> {
        self.traffic_snapshot().map(|(totals, _)| totals)
    }

    /// Total and classified outbound counters from the same single poll.
    /// Only generated node/direct tags have a known class. Auxiliary tags
    /// (including fragment-out, which can overlap node traffic) stay unclassified.
    pub fn traffic_snapshot(
        &self,
    ) -> Option<(TrafficTotals, crate::traffic_stats::ClassifiedTotals)> {
        let per_tag = self.outbound_totals()?;
        Some(snapshot_from_outbounds(&per_tag))
    }

    /// Per-outbound-tag counters, one poll.
    fn outbound_totals(&self) -> Option<HashMap<String, TagCounters>> {
        let resp = shared_agent()
            .get(&format!("{}/debug/vars", self.base))
            .timeout(Duration::from_secs(3))
            .call()
            .ok()?;
        let body: Value = resp.into_json().ok()?;
        parse_outbound_totals(&body)
    }

    /// The node outbound tag that carried the most traffic since the previous
    /// call — Xray's kernel auto-select (balancer + leastPing observatory) has
    /// no query API, but the picked outbound is the one whose counters grow.
    /// Only `node-` tags are considered (direct/block/api/balancer excluded);
    /// idle polls (no positive delta) return `None` so the caller keeps the
    /// last known selection.
    pub fn dominant_outbound_tag(&self) -> Option<String> {
        let now = self.outbound_totals()?;
        let mut last = self.last_counters.lock().unwrap_or_else(|p| p.into_inner());
        let dominant = pick_dominant_node_tag(&last, &now);
        *last = now;
        dominant
    }
}

#[derive(Debug, Clone, Copy)]
struct TagCounters {
    up: u64,
    down: u64,
}

impl TagCounters {
    fn total(self) -> u64 {
        self.up.saturating_add(self.down)
    }
}

fn parse_outbound_totals(body: &Value) -> Option<HashMap<String, TagCounters>> {
    let outbounds = body.get("stats")?.get("outbound")?.as_object()?;
    let mut map = HashMap::with_capacity(outbounds.len());
    for (tag, counters) in outbounds {
        map.insert(
            tag.clone(),
            TagCounters {
                up: counters.get("uplink").and_then(Value::as_u64).unwrap_or(0),
                down: counters
                    .get("downlink")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            },
        );
    }
    Some(map)
}

fn snapshot_from_outbounds(
    per_tag: &HashMap<String, TagCounters>,
) -> (TrafficTotals, crate::traffic_stats::ClassifiedTotals) {
    let mut totals = TrafficTotals::default();
    let mut classified = crate::traffic_stats::ClassifiedTotals::default();
    for (tag, counters) in per_tag {
        totals.upload_total = totals.upload_total.saturating_add(counters.up);
        totals.download_total = totals.download_total.saturating_add(counters.down);
        let bucket = if tag
            .get(..5)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("node-"))
        {
            &mut classified.proxy
        } else if tag.eq_ignore_ascii_case("direct") {
            &mut classified.direct
        } else {
            &mut classified.unclassified
        };
        bucket.upload = bucket.upload.saturating_add(counters.up);
        bucket.download = bucket.download.saturating_add(counters.down);
    }
    (totals, classified)
}

/// Pure decision core (unit-tested): among `node-` tags, the one with the
/// largest positive counter delta wins.
fn pick_dominant_node_tag(
    last: &HashMap<String, TagCounters>,
    now: &HashMap<String, TagCounters>,
) -> Option<String> {
    let mut best: Option<(String, u64)> = None;
    for (tag, counters) in now {
        if !tag.starts_with("node-") {
            continue;
        }
        let delta = counters
            .total()
            .saturating_sub(last.get(tag).map(|c| c.total()).unwrap_or(0));
        if delta == 0 {
            continue;
        }
        if best.as_ref().is_none_or(|(_, d)| delta > *d) {
            best = Some((tag.clone(), delta));
        }
    }
    best.map(|(tag, _)| tag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(up: u64, down: u64) -> TagCounters {
        TagCounters { up, down }
    }

    fn snapshot_from_body(
        body: &str,
    ) -> Option<(TrafficTotals, crate::traffic_stats::ClassifiedTotals)> {
        let body: Value = serde_json::from_str(body).ok()?;
        let per_tag = parse_outbound_totals(&body)?;
        Some(snapshot_from_outbounds(&per_tag))
    }

    fn bytes(value: &crate::traffic_stats::TrafficBytes) -> (u64, u64) {
        (value.upload, value.download)
    }

    #[test]
    fn snapshot_classifies_mixed_outbounds_from_one_body() {
        let (totals, classified) = snapshot_from_body(
            r#"{"stats":{"outbound":{
                "node-a":{"uplink":10,"downlink":20},
                "node-b":{"uplink":30,"downlink":40},
                "direct":{"uplink":50,"downlink":60},
                "custom":{"uplink":70,"downlink":80}
            }}}"#,
        )
        .unwrap();

        assert_eq!((totals.upload_total, totals.download_total), (160, 200));
        assert_eq!(totals.connections, 0);
        assert_eq!(bytes(&classified.proxy), (40, 60));
        assert_eq!(bytes(&classified.direct), (50, 60));
        assert_eq!(bytes(&classified.unclassified), (70, 80));
    }

    #[test]
    fn snapshot_known_tags_are_ascii_case_insensitive() {
        let (totals, classified) = snapshot_from_body(
            r#"{"stats":{"outbound":{
                "NoDe-a":{"uplink":1,"downlink":2},
                "DIRECT":{"uplink":3,"downlink":4},
                "DiReCt":{"uplink":5,"downlink":6}
            }}}"#,
        )
        .unwrap();

        assert_eq!((totals.upload_total, totals.download_total), (9, 12));
        assert_eq!(bytes(&classified.proxy), (1, 2));
        assert_eq!(bytes(&classified.direct), (8, 10));
        assert_eq!(bytes(&classified.unclassified), (0, 0));
    }

    #[test]
    fn snapshot_auxiliary_and_lookalike_tags_remain_unclassified() {
        let tags = [
            "custom",
            "proxy",
            "dns-out",
            "dns-module",
            "direct-dns",
            "DNS",
            "api",
            "block",
            "fragment-out",
            "proxy-balancer",
            "smart-a",
            "filter-a",
            "node",
            "node_a",
            "direct-custom",
            "",
            "节点-a",
        ];
        let per_tag = tags
            .into_iter()
            .map(|tag| (tag.to_string(), tc(2, 3)))
            .collect();
        let (totals, classified) = snapshot_from_outbounds(&per_tag);
        let expected = (tags.len() as u64 * 2, tags.len() as u64 * 3);

        assert_eq!((totals.upload_total, totals.download_total), expected);
        assert_eq!(bytes(&classified.proxy), (0, 0));
        assert_eq!(bytes(&classified.direct), (0, 0));
        assert_eq!(bytes(&classified.unclassified), expected);
    }

    #[test]
    fn snapshot_empty_or_invalid_body_and_missing_stats_return_none() {
        for body in [
            "",
            " ",
            "not-json",
            "null",
            "[]",
            "{}",
            r#"{"stats":{}}"#,
            r#"{"stats":{"outbound":null}}"#,
            r#"{"stats":{"outbound":[]}}"#,
        ] {
            assert!(snapshot_from_body(body).is_none(), "body: {body}");
        }
    }

    #[test]
    fn snapshot_empty_outbounds_and_zero_traffic_are_valid() {
        for body in [
            r#"{"stats":{"outbound":{}}}"#,
            r#"{"stats":{"outbound":{
                "node-a":{"uplink":0,"downlink":0},
                "direct":{"uplink":0,"downlink":0},
                "custom":{"uplink":0,"downlink":0}
            }}}"#,
        ] {
            let (totals, classified) = snapshot_from_body(body).unwrap();
            assert_eq!((totals.upload_total, totals.download_total), (0, 0));
            assert_eq!(totals.connections, 0);
            assert_eq!(bytes(&classified.proxy), (0, 0));
            assert_eq!(bytes(&classified.direct), (0, 0));
            assert_eq!(bytes(&classified.unclassified), (0, 0));
        }
    }

    #[test]
    fn snapshot_missing_or_invalid_counters_keep_zero_fallback() {
        let (totals, classified) = snapshot_from_body(
            r#"{"stats":{"outbound":{
                "node-a":{"uplink":8},
                "direct":{"downlink":9},
                "custom":{"uplink":-1,"downlink":"10"},
                "api":null,
                "block":{},
                "dns-out":{"uplink":1.5,"downlink":false}
            }}}"#,
        )
        .unwrap();

        assert_eq!((totals.upload_total, totals.download_total), (8, 9));
        assert_eq!(bytes(&classified.proxy), (8, 0));
        assert_eq!(bytes(&classified.direct), (0, 9));
        assert_eq!(bytes(&classified.unclassified), (0, 0));
    }

    #[test]
    fn snapshot_parses_u64_max_and_saturates_across_classes() {
        let (totals, classified) = snapshot_from_body(
            r#"{"stats":{"outbound":{
                "node-a":{"uplink":18446744073709551615,"downlink":1},
                "direct":{"uplink":1,"downlink":18446744073709551615},
                "custom":{"uplink":2,"downlink":3}
            }}}"#,
        )
        .unwrap();

        assert_eq!(totals.upload_total, u64::MAX);
        assert_eq!(totals.download_total, u64::MAX);
        assert_eq!(bytes(&classified.proxy), (u64::MAX, 1));
        assert_eq!(bytes(&classified.direct), (1, u64::MAX));
        assert_eq!(bytes(&classified.unclassified), (2, 3));
    }

    #[test]
    fn snapshot_saturates_totals_and_every_class_in_both_directions() {
        let mut per_tag = HashMap::new();
        for tag in ["node-a", "direct", "custom"] {
            per_tag.insert(tag.to_string(), tc(u64::MAX - 1, u64::MAX));
        }
        for tag in ["node-b", "DIRECT", "fragment-out"] {
            per_tag.insert(tag.to_string(), tc(2, 1));
        }
        let (totals, classified) = snapshot_from_outbounds(&per_tag);

        assert_eq!(totals.upload_total, u64::MAX);
        assert_eq!(totals.download_total, u64::MAX);
        assert_eq!(totals.connections, 0);
        assert_eq!(bytes(&classified.proxy), (u64::MAX, u64::MAX));
        assert_eq!(bytes(&classified.direct), (u64::MAX, u64::MAX));
        assert_eq!(bytes(&classified.unclassified), (u64::MAX, u64::MAX));
    }

    #[test]
    fn dominant_tag_follows_largest_node_delta() {
        let mut last = HashMap::new();
        last.insert("node-a".into(), tc(100, 200));
        last.insert("node-b".into(), tc(50, 50));
        last.insert("direct".into(), tc(999, 999));
        let mut now = HashMap::new();
        now.insert("node-a".into(), tc(100, 200)); // idle
        now.insert("node-b".into(), tc(60, 70)); // +25 → winner
        now.insert("direct".into(), tc(9999, 9999)); // excluded tag, huge delta
        assert_eq!(
            pick_dominant_node_tag(&last, &now).as_deref(),
            Some("node-b")
        );
    }

    #[test]
    fn dominant_tag_none_when_idle_or_unknown_tags_only() {
        let last = HashMap::new();
        let mut now = HashMap::new();
        now.insert("node-a".into(), tc(5, 5));
        assert_eq!(
            pick_dominant_node_tag(&last, &now).as_deref(),
            Some("node-a")
        );
        // No movement between polls → None (keep last known).
        assert_eq!(pick_dominant_node_tag(&now, &now), None);
        let mut only_fixed = HashMap::new();
        only_fixed.insert("direct".into(), tc(1, 1));
        only_fixed.insert("block".into(), tc(0, 0));
        assert_eq!(pick_dominant_node_tag(&only_fixed, &only_fixed), None);
    }

    #[test]
    fn session_token_shared_across_clones() {
        let a = XrayMetrics::new("127.0.0.1", 19090);
        let b = a.clone();
        assert!(a.same_session(&b));
        assert!(a.is_active());
        b.deactivate();
        assert!(!a.is_active());
    }
}

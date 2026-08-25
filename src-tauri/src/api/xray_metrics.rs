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
        let per_tag = self.outbound_totals()?;
        let mut upload_total = 0u64;
        let mut download_total = 0u64;
        for counters in per_tag.values() {
            upload_total = upload_total.saturating_add(counters.up);
            download_total = download_total.saturating_add(counters.down);
        }
        Some(TrafficTotals {
            upload_total,
            download_total,
            connections: 0,
        })
    }

    /// Per-outbound-tag counters, one poll.
    fn outbound_totals(&self) -> Option<HashMap<String, TagCounters>> {
        let resp = shared_agent()
            .get(&format!("{}/debug/vars", self.base))
            .timeout(Duration::from_secs(3))
            .call()
            .ok()?;
        let body: Value = resp.into_json().ok()?;
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

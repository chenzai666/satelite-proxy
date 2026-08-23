//! Xray metrics client — polls the core's `metrics` module at
//! `GET /debug/vars` (Go expvar JSON). Xray has no Clash-compatible API:
//! per-connection snapshots, group selection and delay testing do not exist;
//! traffic totals per outbound tag are the only runtime signal.
//!
//! HTTP via **ureq** for the same nested-runtime reasons as `clash_api.rs`.

use crate::api::TrafficTotals;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct XrayMetrics {
    pub base: String,
    active: Arc<AtomicBool>,
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
        let resp = shared_agent()
            .get(&format!("{}/debug/vars", self.base))
            .timeout(Duration::from_secs(3))
            .call()
            .ok()?;
        let body: Value = resp.into_json().ok()?;
        let outbounds = body.get("stats")?.get("outbound")?.as_object()?;
        let mut upload_total = 0u64;
        let mut download_total = 0u64;
        for (_tag, counters) in outbounds {
            let up = counters.get("uplink").and_then(Value::as_u64).unwrap_or(0);
            let down = counters
                .get("downlink")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            upload_total = upload_total.saturating_add(up);
            download_total = download_total.saturating_add(down);
        }
        Some(TrafficTotals {
            upload_total,
            download_total,
            connections: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

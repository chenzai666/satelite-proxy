//! Chain diagnostics: probe every hop of a [`ProxyChain`] through the live
//! Clash delay API, two probes per hop —
//!
//! - **solo**: the hop's standalone outbound (plain node tag / shared pool
//!   selector) — is this node/pool alive on its own?
//! - **chained**: the hop's chain-prefix outbound (the same tags the config
//!   generator mints: chain-local clone/selector at hop i, which dials
//!   through hops 0..i-1 and exits at hop i) — does chaining up to here work?
//!
//! The last hop's `chained` result IS the whole chain (the exit rules route
//! to). A hop with solo-ok / chained-failed localizes the break to the relay
//! into it (typically an entry-side node refusing foreign sources).
//!
//! Beyond the delay URL, [`ExitProbe`] adds real-world verification: a real
//! HTTPS round-trip to <https://api.ip.sb/ip> through the whole chain, and
//! the actual exit IP — fetched through the app's loopback diagnostics
//! inbound (`config::builder::DIAG_INBOUND_PORT`) after switching its
//! `chain-diag` selector to this chain via the Clash API. No user rule or
//! core restart involved (the Clash delay API discards response bodies, so
//! the selector+inbound pair is the only rule-free way to carry a body out
//! of a chosen chain).
//!
//! All probes fire in parallel (`spawn_blocking` per ureq call — the Clash
//! API client must stay off the async runtime, see `api/clash_api.rs`).

use crate::api::ClashApi;
use crate::domain::{ChainHop, NodePool, ProxyChain, ProxyNode};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HopDiag {
    /// Node/pool display name (raw id once stale).
    pub label: String,
    /// "node" | "pool".
    pub kind: String,
    /// Referenced node/pool no longer exists — nothing to probe.
    pub stale: bool,
    pub solo_ms: Option<u32>,
    pub solo_error: Option<String>,
    pub chained_ms: Option<u32>,
    pub chained_error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitProbe {
    pub geo: Option<ExitGeo>,
    pub ip_error: Option<String>,
    pub ip_sb_ms: Option<u32>,
    pub ip_sb_error: Option<String>,
}

/// Geo/quality facts from <https://api.ip.sb/geoip> as seen by the chain's
/// exit (localized server-side via `lang`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitGeo {
    pub ip: String,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub asn: Option<String>,
    pub asn_organization: Option<String>,
    pub organization: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainDiagnosis {
    /// In chain order; the last hop's `chained_*` is the whole-chain result.
    pub hops: Vec<HopDiag>,
    /// Real-world exit verification.
    pub exit: ExitProbe,
}

type Probe = (Option<u32>, Option<String>);

const IP_SB_URL: &str = "https://api.ip.sb/ip";
const IP_SB_GEO_URL: &str = "https://api.ip.sb/geoip";

fn probe(api: &ClashApi, tag: &str, url: &str, timeout_ms: u64) -> Probe {
    match api.delay(tag, url, timeout_ms) {
        Ok(ms) => (Some(ms), None),
        Err(e) => (None, Some(e.to_string())),
    }
}

/// Fetch the exit's geo/quality facts (ip, country, city, ASN, org,
/// timezone) from api.ip.sb's geoip endpoint through the loopback
/// diagnostics inbound (already routed to the `chain-diag` selector).
/// `lang` localizes the place names server-side ("zh-CN" / "en").
fn fetch_exit_geo(lang: &str) -> std::result::Result<ExitGeo, String> {
    let proxy = ureq::Proxy::new(format!(
        "http://127.0.0.1:{}",
        crate::config::DIAG_INBOUND_PORT
    ))
    .map_err(|e| format!("proxy: {e}"))?;
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(12))
        .proxy(proxy)
        .build();
    let resp = agent
        .get(&format!("{IP_SB_GEO_URL}?lang={lang}"))
        .set("Accept", "application/json")
        .call()
        .map_err(|e| match &e {
            ureq::Error::Status(403, _) => {
                "ip.sb 拒绝了该出口（403）——可更换出口节点后重试".to_string()
            }
            other => format!("{other}"),
        })?;
    let body = resp.into_string().map_err(|e| format!("body: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("geoip json: {e}"))?;
    let field = |k: &str| {
        value
            .get(k)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    };
    let ip = field("ip").ok_or("geoip 响应缺少 ip")?;
    Ok(ExitGeo {
        ip,
        country: field("country"),
        country_code: field("country_code"),
        region: field("region"),
        city: field("city"),
        asn: field("asn"),
        asn_organization: field("asn_organization"),
        organization: field("organization"),
        timezone: field("timezone"),
    })
}

pub async fn diagnose(
    api: ClashApi,
    chain: ProxyChain,
    pools: Vec<NodePool>,
    nodes: Vec<ProxyNode>,
    probe_url: String,
    timeout_ms: u64,
    locale: String,
) -> ChainDiagnosis {
    let started = std::time::Instant::now();
    crate::app_log::info(
        "chain_diag",
        format!("开始诊断链路「{}」（{} 跳）", chain.name, chain.hops.len()),
    );
    // Plan phase: resolve labels + probe tags (mirroring the generator's tag
    // scheme — see `config::builder::build_chain_outbounds_for`).
    struct Planned {
        label: String,
        kind: &'static str,
        stale: bool,
        solo_tag: Option<String>,
        chained_tag: Option<String>,
    }
    let mut plan: Vec<Planned> = Vec::with_capacity(chain.hops.len());
    for (i, hop) in chain.hops.iter().enumerate() {
        match hop {
            ChainHop::Node { node_id } => {
                let n = nodes.iter().find(|n| &n.id == node_id);
                plan.push(Planned {
                    label: n.map(|n| n.name.clone()).unwrap_or_else(|| node_id.clone()),
                    kind: "node",
                    stale: n.is_none(),
                    solo_tag: n.map(crate::config::outbound_tag),
                    chained_tag: if n.is_none() {
                        None
                    } else {
                        Some(crate::config::chain_hop_outbound_tag(&chain, i))
                    },
                });
            }
            ChainHop::Pool { pool_id } => {
                let p = pools.iter().find(|p| &p.id == pool_id);
                let shared = crate::domain::pool_outbound_tag_for_id(pool_id);
                plan.push(Planned {
                    label: p.map(|p| p.name.clone()).unwrap_or_else(|| pool_id.clone()),
                    kind: "pool",
                    stale: p.is_none(),
                    // hop[0] pools are client-side entries — their chain
                    // prefix IS the shared selector; deeper pools have a
                    // chain-local selector over detour clones.
                    solo_tag: p.map(|_| shared.clone()),
                    chained_tag: if p.is_none() {
                        None
                    } else if i == 0 {
                        Some(shared)
                    } else {
                        Some(crate::config::chain_hop_outbound_tag(&chain, i))
                    },
                });
            }
        }
    }

    // The whole-chain tag rules point at (the last hop's chain-local tag).
    let exit_tag = plan.last().and_then(|p| p.chained_tag.clone());

    // Fire phase: one blocking task per DISTINCT tag (deduped), all in
    // parallel; results are joined in tag order afterwards.
    let mut handles: Vec<tokio::task::JoinHandle<Probe>> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut spawn = |tag: &Option<String>| -> Option<usize> {
        let tag = tag.as_ref()?;
        if let Some(idx) = seen.get(tag) {
            return Some(*idx);
        }
        let api = api.clone();
        let owned = tag.clone();
        let url = probe_url.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            probe(&api, &owned, &url, timeout_ms)
        }));
        seen.insert(tag.clone(), handles.len() - 1);
        Some(handles.len() - 1)
    };
    let solo_idx: Vec<Option<usize>> = plan.iter().map(|p| spawn(&p.solo_tag)).collect();
    let chained_idx: Vec<Option<usize>> = plan.iter().map(|p| spawn(&p.chained_tag)).collect();

    // Real-world probes alongside: ip.sb round-trip through the whole chain,
    // and the exit IP via the diagnostics inbound (selector switched to this
    // chain's exit tag first).
    let ip_sb_handle = exit_tag.clone().map(|tag| {
        let api = api.clone();
        tokio::task::spawn_blocking(move || probe(&api, &tag, IP_SB_URL, timeout_ms))
    });
    let ip_handle = exit_tag.clone().map(|tag| {
        let api = api.clone();
        let lang = if locale.to_ascii_lowercase().starts_with("zh") {
            "zh-CN"
        } else {
            "en"
        };
        tokio::task::spawn_blocking(move || -> std::result::Result<ExitGeo, String> {
            api.select_proxy(crate::config::DIAG_SELECTOR_TAG, &tag)
                .map_err(|e| format!("切换诊断出口: {e}"))?;
            fetch_exit_geo(lang)
        })
    });
    drop(spawn);

    // Every join is hard-capped: even if some internal wait stalls past its
    // ureq timeouts, the diagnosis always resolves instead of spinning the
    // UI forever.
    let probe_cap = std::time::Duration::from_millis(timeout_ms.saturating_add(8000));
    let mut results: Vec<Probe> = Vec::with_capacity(handles.len());
    for handle in handles {
        let probe = match tokio::time::timeout(probe_cap, handle).await {
            Ok(Ok(probe)) => probe,
            Ok(Err(e)) => (None, Some(format!("probe task failed: {e}"))),
            Err(_) => (None, Some("探测超时".into())),
        };
        results.push(probe);
    }
    let (ip_sb_ms, ip_sb_error) = match ip_sb_handle {
        Some(h) => match tokio::time::timeout(probe_cap, h).await {
            Ok(Ok(probe)) => probe,
            Ok(Err(e)) => (None, Some(format!("ip.sb task failed: {e}"))),
            Err(_) => (None, Some("ip.sb 探测超时".into())),
        },
        None => (None, Some("链路未生成（存在失效跳）".into())),
    };
    let (geo, ip_error) = match ip_handle {
        Some(h) => match tokio::time::timeout(std::time::Duration::from_secs(20), h).await {
            Ok(Ok(Ok(geo))) => (Some(geo), None),
            Ok(Ok(Err(e))) => (None, Some(e)),
            Ok(Err(e)) => (None, Some(format!("exit-ip task failed: {e}"))),
            Err(_) => (None, Some("出口探测超时".into())),
        },
        None => (None, Some("链路未生成（存在失效跳）".into())),
    };

    let pick = |idx: Option<usize>| -> Probe {
        match idx {
            Some(j) => results[j].clone(),
            None => (None, None),
        }
    };
    let hops = plan
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let (solo_ms, solo_error) = pick(solo_idx[i]);
            let (chained_ms, chained_error) = pick(chained_idx[i]);
            HopDiag {
                label: p.label,
                kind: p.kind.to_string(),
                stale: p.stale,
                solo_ms,
                solo_error,
                chained_ms,
                chained_error,
            }
        })
        .collect();
    crate::app_log::info(
        "chain_diag",
        format!(
            "链路「{}」诊断完成，耗时 {} ms",
            chain.name,
            started.elapsed().as_millis()
        ),
    );
    ChainDiagnosis {
        hops,
        exit: ExitProbe {
            geo,
            ip_error,
            ip_sb_ms,
            ip_sb_error,
        },
    }
}

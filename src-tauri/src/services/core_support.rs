//! Per-core node-support report for a stored raw subscription body.
//!
//! The subscription card's "内核支持详情" panel re-parses the saved raw body
//! with the CURRENT parser and evaluates every recognized node against each
//! core's `supports_node` predicate. Pure function of the body — numbers stay
//! truthful for subscriptions imported by older builds, and they update
//! automatically as parsers / support sets evolve.

use crate::core::CoreKind;
use crate::domain::{SkippedProxy, SubscriptionFormat};
use crate::error::AppResult;
use crate::subscription::parse_subscription;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct CoreSupportCounts {
    pub singbox: usize,
    pub mihomo: usize,
    pub xray: usize,
}

/// One node a core cannot serve, with the reason.
#[derive(Debug, Clone, Serialize)]
pub struct CoreUnsupportedItem {
    pub name: String,
    /// Original clash `type` string when known, else the protocol key.
    pub type_label: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct CoreUnsupported {
    pub singbox: Vec<CoreUnsupportedItem>,
    pub mihomo: Vec<CoreUnsupportedItem>,
    pub xray: Vec<CoreUnsupportedItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CoreSupportReport {
    /// Detected body format label (clash_yaml / singbox_json / uri_list / …).
    pub format: String,
    /// Entries in the raw body: recognized + parse-skipped.
    pub total_entries: usize,
    /// Entries the parser turned into nodes.
    pub recognized: usize,
    pub cores: CoreSupportCounts,
    /// Per-core node-level exclusions with reasons (the filter's single
    /// source of truth is `CoreKind::node_unsupported_reason`).
    pub unsupported: CoreUnsupported,
    /// Parse-level drops with reasons (unsupported type / missing field).
    pub skipped: Vec<SkippedProxy>,
}

fn unsupported_item(node: &crate::domain::ProxyNode, reason: &str) -> CoreUnsupportedItem {
    CoreUnsupportedItem {
        name: node.name.clone(),
        type_label: node
            .source
            .clone()
            .unwrap_or_else(|| node.protocol.as_str().to_string()),
        reason: reason.to_string(),
    }
}

pub fn core_support_report(content: &str) -> AppResult<CoreSupportReport> {
    let parsed = parse_subscription(content)?;
    let format = format_label(parsed.format);
    let recognized = parsed.nodes.len();
    let mut counts = CoreSupportCounts::default();
    let mut unsupported = CoreUnsupported::default();
    for node in &parsed.nodes {
        let check =
            |kind: CoreKind, list: &mut Vec<CoreUnsupportedItem>, count: &mut usize| match kind
                .node_unsupported_reason(node)
            {
                Some(reason) => list.push(unsupported_item(node, reason)),
                None => *count += 1,
            };
        check(
            CoreKind::SingBox,
            &mut unsupported.singbox,
            &mut counts.singbox,
        );
        check(
            CoreKind::Mihomo,
            &mut unsupported.mihomo,
            &mut counts.mihomo,
        );
        check(CoreKind::Xray, &mut unsupported.xray, &mut counts.xray);
    }
    Ok(CoreSupportReport {
        format,
        total_entries: recognized + parsed.skipped.len(),
        recognized,
        cores: counts,
        unsupported,
        skipped: parsed.skipped,
    })
}

fn format_label(f: SubscriptionFormat) -> String {
    crate::services::import::format_label(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASH: &str = r#"
proxies:
  - name: ss-ok
    type: ss
    server: a.example.com
    port: 8388
    cipher: aes-256-gcm
    password: x
  - name: vless-h2
    type: vless
    server: b.example.com
    port: 443
    uuid: 11111111-1111-1111-1111-111111111111
    network: h2
    tls: true
  - name: broken
    type: ss
    server: c.example.com
    port: 8388
    cipher: aes-256-gcm
"#;

    #[test]
    fn counts_per_core_and_parse_skips() {
        let report = core_support_report(CLASH).unwrap();
        assert_eq!(report.format, "clash_yaml");
        assert_eq!(report.total_entries, 3);
        assert_eq!(report.recognized, 2);
        // ss-ok: all cores; vless over h2: sing-box + mihomo (Xray v26 dropped
        // the h2 transport at config level).
        assert_eq!(report.cores.singbox, 2);
        assert_eq!(report.cores.mihomo, 2);
        assert_eq!(report.cores.xray, 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].name.as_deref(), Some("broken"));
        assert!(report.skipped[0].reason.contains("password"));
        // Node-level exclusions carry per-core reasons: vless-h2 is the one
        // Xray cannot serve.
        assert!(report.unsupported.singbox.is_empty());
        assert!(report.unsupported.mihomo.is_empty());
        assert_eq!(report.unsupported.xray.len(), 1);
        assert_eq!(report.unsupported.xray[0].name, "vless-h2");
        assert!(report.unsupported.xray[0].reason.contains("h2"));
    }

    #[test]
    fn uri_lists_are_fully_recognized() {
        let body = "trojan://pwd@host.example:443?sni=host.example#T1\n";
        let report = core_support_report(body).unwrap();
        assert_eq!(report.format, "uri_list");
        assert_eq!(report.recognized, 1);
        assert_eq!(report.total_entries, 1);
        assert!(report.skipped.is_empty());
    }
}

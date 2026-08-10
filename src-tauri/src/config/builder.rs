//! Build sing-box JSON from normalized [`ProxyNode`]s.

use crate::config::dns_build::{build_dns_section, build_hosts_route_rules};
use crate::domain::{
    AutoSelectMode, DnsSettings, OutboundMode, Protocol, ProtocolConfig, ProxyNode, Rule, RuleType,
    TlsConfig, Transport,
};
use crate::error::{AppError, AppResult};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub struct BuildOptions {
    pub mixed_port: u16,
    pub api_port: u16,
    pub api_secret: String,
    /// Preferred node id; falls back to first node.
    pub current_node_id: Option<String>,
    pub log_level: String,
    pub rules: Vec<Rule>,
    /// Enable TUN inbound (global capture).
    pub tun_enabled: bool,
    /// system | gvisor | mixed
    pub tun_stack: String,
    /// DNS module settings (always applied).
    pub dns: DnsSettings,
    /// Rule / Global / Direct.
    pub outbound_mode: OutboundMode,
    /// `route.final` in Rule mode: proxy | direct | block.
    pub route_final: String,
    /// off/smart → selector; kernel → urltest.
    pub auto_select: AutoSelectMode,
    /// URL for kernel urltest (and shared probe default).
    pub probe_url: String,
    /// Resolve the originating process per connection (sing-box
    /// `find_process_mode`: on → always, off → off).
    pub find_process: bool,
}

impl BuildOptions {
    pub fn normalized_tun_stack(&self) -> &str {
        match self.tun_stack.to_ascii_lowercase().as_str() {
            "system" => "system",
            "gvisor" => "gvisor",
            _ => "mixed",
        }
    }

    pub fn normalized_route_final(&self) -> &str {
        match self.route_final.to_ascii_lowercase().as_str() {
            "direct" => "direct",
            "block" => "block",
            _ => "proxy",
        }
    }
}

#[derive(Debug)]
pub struct BuiltConfig {
    pub value: Value,
    pub outbound_tags: Vec<String>,
    pub selected_tag: String,
}

/// Convert nodes into a complete sing-box config document.
pub fn build_singbox_config(nodes: &[ProxyNode], opts: &BuildOptions) -> AppResult<BuiltConfig> {
    if nodes.is_empty() {
        return Err(AppError::Config(
            "no nodes available; import a subscription first".into(),
        ));
    }

    let mut node_outbounds = Vec::new();
    let mut tags = Vec::new();
    let mut errors = Vec::new();

    for node in nodes {
        match node_to_outbound(node) {
            Ok((tag, outbound)) => {
                tags.push(tag);
                node_outbounds.push(outbound);
            }
            Err(e) => errors.push(format!("{}: {e}", node.name)),
        }
    }

    if node_outbounds.is_empty() {
        return Err(AppError::Config(format!(
            "failed to map any node to outbound: {}",
            errors.join("; ")
        )));
    }

    let selected_tag = resolve_selected_tag(nodes, &tags, opts.current_node_id.as_deref());

    let mut outbounds = Vec::new();
    // Main group: selector (manual / app smart) vs urltest (kernel auto).
    if opts.auto_select.is_kernel() {
        let url = if opts.probe_url.trim().is_empty() {
            "https://www.gstatic.com/generate_204".to_string()
        } else {
            opts.probe_url.trim().to_string()
        };
        // urltest only lists real nodes (never "direct" — would win on latency).
        outbounds.push(json!({
            "type": "urltest",
            "tag": "proxy",
            "outbounds": tags.clone(),
            "url": url,
            "interval": "5m",
            "tolerance": 50,
            "idle_timeout": "30m",
            "interrupt_exist_connections": false,
        }));
    } else {
        let mut selector_outbounds = tags.clone();
        selector_outbounds.push("direct".into());
        outbounds.push(json!({
            "type": "selector",
            "tag": "proxy",
            "outbounds": selector_outbounds,
            "default": selected_tag,
        }));
    }
    // Per-rule smart selectors (keyword-filtered node pools).
    outbounds.extend(build_smart_rule_selectors(&opts.rules, nodes, &tags));
    outbounds.extend(node_outbounds);
    outbounds.push(json!({ "type": "direct", "tag": "direct" }));
    outbounds.push(json!({ "type": "block", "tag": "block" }));

    // Clash-style modes:
    // - Rule: user rules + configurable final (proxy|direct|block)
    // - Global: no user rules, final proxy
    // - Direct: no user rules, final direct
    let (apply_user_rules, route_final) = match opts.outbound_mode {
        OutboundMode::Rule => (true, opts.normalized_route_final()),
        OutboundMode::Global => (false, "proxy"),
        OutboundMode::Direct => (false, "direct"),
    };

    // DNS `final` is configured independently on the DNS page (local/domestic/
    // remote) and no longer follows the routing `final`.
    let built_dns = build_dns_section(&opts.dns, opts.tun_enabled, &opts.rules);

    let mut route_rules = Vec::new();
    // Sniff helps domain-based route / DNS on mixed + TUN
    route_rules.push(json!({ "action": "sniff" }));
    if built_dns.want_hijack || opts.tun_enabled {
        route_rules.push(json!({ "protocol": "dns", "action": "hijack-dns" }));
    }
    // Hosts must also apply to mixed/system-proxy connections, which can pass a
    // domain directly to the outbound without performing a DNS query.
    route_rules.extend(build_hosts_route_rules(&opts.dns.effective_hosts()));
    if apply_user_rules {
        route_rules.extend(build_route_rules(&opts.rules, nodes, &tags));
    }

    let mut inbounds = vec![json!({
        "type": "mixed",
        "tag": "mixed-in",
        "listen": "127.0.0.1",
        "listen_port": opts.mixed_port
    })];

    if opts.tun_enabled {
        // strict_route is mainly a Windows multi-homed DNS workaround; on macOS it
        // can break host → 127.0.0.1 (clash_api / mixed) while TUN is up.
        // Always exclude loopback so the app can reach clash_api for health checks.
        inbounds.push(json!({
            "type": "tun",
            "tag": "tun-in",
            "address": ["172.19.0.1/30", "fdfe:dcba:9876::1/126"],
            "mtu": 9000,
            "auto_route": true,
            "strict_route": cfg!(target_os = "windows"),
            "route_exclude_address": ["127.0.0.0/8", "::1/128"],
            "stack": opts.normalized_tun_stack()
        }));
    }

    let value = json!({
        "log": {
            "level": opts.log_level,
            "timestamp": true
        },
        "dns": built_dns.dns,
        "inbounds": inbounds,
        "outbounds": outbounds,
        "route": {
            "rules": route_rules,
            "final": route_final,
            "auto_detect_interface": true,
            "default_domain_resolver": built_dns.default_resolver,
            // Resolve the originating process for each connection so the
            // Clash API connections list (and our traffic page) shows a real
            // process name. 1.13 uses route.find_process (bool).
            "find_process": opts.find_process
        },
        "experimental": {
            "clash_api": {
                "external_controller": format!("127.0.0.1:{}", opts.api_port),
                "secret": opts.api_secret,
                "default_mode": opts.outbound_mode.as_str()
            }
        }
    });

    Ok(BuiltConfig {
        value,
        outbound_tags: tags,
        selected_tag,
    })
}

fn resolve_selected_tag(nodes: &[ProxyNode], tags: &[String], current_id: Option<&str>) -> String {
    if let Some(id) = current_id {
        if let Some(node) = nodes.iter().find(|n| n.id == id) {
            let tag = outbound_tag(node);
            if tags.iter().any(|t| t == &tag) {
                return tag;
            }
        }
    }
    tags.first().cloned().unwrap_or_else(|| "direct".into())
}

pub fn outbound_tag(node: &ProxyNode) -> String {
    format!("node-{}", &node.id[..node.id.len().min(16)])
}

fn build_route_rules(rules: &[Rule], nodes: &[ProxyNode], tags: &[String]) -> Vec<Value> {
    let mut sorted: Vec<&Rule> = rules.iter().filter(|r| r.enabled).collect();
    sorted.sort_by_key(|r| r.ord);

    sorted
        .into_iter()
        .filter_map(|r| {
            let payload = r.payload.trim();
            if payload.is_empty() {
                return None;
            }
            // sing-box 1.8+ deprecated / 1.12+ removed inline `geoip` — skip
            if matches!(r.rule_type, RuleType::Geoip) {
                return None;
            }
            let outbound = resolve_rule_outbound(r, nodes, tags);
            let mut rule = match r.rule_type {
                RuleType::Domain => json!({ "domain": [payload] }),
                RuleType::DomainSuffix => json!({ "domain_suffix": [payload] }),
                RuleType::DomainKeyword => json!({ "domain_keyword": [payload] }),
                RuleType::IpCidr => json!({ "ip_cidr": [payload] }),
                RuleType::Process => json!({ "process_name": [payload] }),
                RuleType::Geoip => return None,
            };
            rule.as_object_mut()?
                .insert("outbound".into(), json!(outbound));
            Some(rule)
        })
        .collect()
}

/// Map a rule to an outbound tag. Pinned node missing → fall back to main `proxy` selector.
fn resolve_rule_outbound(r: &Rule, nodes: &[ProxyNode], tags: &[String]) -> String {
    use crate::domain::RuleTarget;
    match r.target {
        RuleTarget::Direct | RuleTarget::Proxy | RuleTarget::Block => {
            r.target.outbound_tag().into()
        }
        RuleTarget::Node => {
            if let Some(id) = r.node_id.as_deref().filter(|s| !s.is_empty()) {
                if let Some(node) = nodes.iter().find(|n| n.id == id) {
                    let tag = outbound_tag(node);
                    if tags.iter().any(|t| t == &tag) {
                        return tag;
                    }
                }
            }
            // Stale pin (subscription updated / node removed / sub disabled).
            RuleTarget::Proxy.outbound_tag().into()
        }
        RuleTarget::Smart => {
            let pool = smart_pool_tags(r, nodes, tags);
            if pool.is_empty() {
                RuleTarget::Proxy.outbound_tag().into()
            } else {
                r.smart_outbound_tag()
            }
        }
    }
}

/// Node outbound tags matching a smart rule's include/exclude name filters.
pub fn smart_pool_tags(r: &Rule, nodes: &[ProxyNode], tags: &[String]) -> Vec<String> {
    let mut pool: Vec<(u32, String)> = nodes
        .iter()
        .filter(|n| r.smart_name_matches(&n.name))
        .filter_map(|n| {
            let tag = outbound_tag(n);
            if tags.iter().any(|t| t == &tag) {
                Some((n.latency_ms.unwrap_or(u32::MAX / 4), tag))
            } else {
                None
            }
        })
        .collect();
    // Prefer historically better latency as selector default.
    pool.sort_by_key(|(lat, _)| *lat);
    pool.into_iter().map(|(_, tag)| tag).collect()
}

/// Nodes matching smart filters (for probe / UI).
pub fn smart_pool_nodes(r: &Rule, nodes: &[ProxyNode]) -> Vec<ProxyNode> {
    nodes
        .iter()
        .filter(|n| r.smart_name_matches(&n.name))
        .cloned()
        .collect()
}

fn build_smart_rule_selectors(rules: &[Rule], nodes: &[ProxyNode], tags: &[String]) -> Vec<Value> {
    use crate::domain::RuleTarget;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in rules
        .iter()
        .filter(|r| r.enabled && matches!(r.target, RuleTarget::Smart))
    {
        let group = r.smart_outbound_tag();
        if !seen.insert(group.clone()) {
            continue;
        }
        let pool = smart_pool_tags(r, nodes, tags);
        if pool.is_empty() {
            continue;
        }
        let default = pool.first().cloned().unwrap_or_else(|| "direct".into());
        out.push(json!({
            "type": "selector",
            "tag": group,
            "outbounds": pool,
            "default": default,
        }));
    }
    out
}

fn node_to_outbound(node: &ProxyNode) -> AppResult<(String, Value)> {
    let tag = outbound_tag(node);
    let mut ob = match (&node.protocol, &node.config) {
        (
            Protocol::Shadowsocks,
            ProtocolConfig::Shadowsocks {
                method,
                password,
                plugin,
                plugin_opts,
            },
        ) => {
            let mut o = json!({
                "type": "shadowsocks",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "method": method,
                "password": password,
            });
            if let Some(p) = plugin {
                o["plugin"] = json!(p);
            }
            if let Some(opts) = plugin_opts {
                o["plugin_opts"] = json!(opts);
            }
            o
        }
        (
            Protocol::Vmess,
            ProtocolConfig::Vmess {
                uuid,
                alter_id,
                security,
            },
        ) => {
            json!({
                "type": "vmess",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "uuid": uuid,
                "security": security,
                "alter_id": alter_id,
            })
        }
        (
            Protocol::Vless,
            ProtocolConfig::Vless {
                uuid,
                flow,
                packet_encoding,
            },
        ) => {
            let mut o = json!({
                "type": "vless",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "uuid": uuid,
                "packet_encoding": packet_encoding,
            });
            if let Some(f) = flow {
                if !f.is_empty() {
                    o["flow"] = json!(f);
                }
            }
            o
        }
        (Protocol::Trojan, ProtocolConfig::Trojan { password }) => {
            json!({
                "type": "trojan",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "password": password,
            })
        }
        (
            Protocol::Hysteria2,
            ProtocolConfig::Hysteria2 {
                password,
                up_mbps,
                down_mbps,
                obfs,
                obfs_password,
            },
        ) => {
            let mut o = json!({
                "type": "hysteria2",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "password": password,
            });
            if let Some(u) = up_mbps {
                o["up_mbps"] = json!(u);
            }
            if let Some(d) = down_mbps {
                o["down_mbps"] = json!(d);
            }
            if let Some(t) = obfs {
                let mut obfs_obj = json!({ "type": t });
                if let Some(p) = obfs_password {
                    obfs_obj["password"] = json!(p);
                }
                o["obfs"] = obfs_obj;
            }
            o
        }
        (
            Protocol::Tuic,
            ProtocolConfig::Tuic {
                uuid,
                password,
                congestion_control,
                udp_relay_mode,
                zero_rtt_handshake,
            },
        ) => {
            let mut o = json!({
                "type": "tuic",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "uuid": uuid,
                "password": password,
                "zero_rtt_handshake": zero_rtt_handshake,
            });
            if let Some(c) = congestion_control {
                o["congestion_control"] = json!(c);
            }
            if let Some(m) = udp_relay_mode {
                o["udp_relay_mode"] = json!(m);
            }
            o
        }
        (Protocol::Socks5, ProtocolConfig::Socks5 { username, password }) => {
            let mut o = json!({
                "type": "socks",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "version": "5",
            });
            if let Some(u) = username {
                o["username"] = json!(u);
            }
            if let Some(p) = password {
                o["password"] = json!(p);
            }
            o
        }
        (Protocol::AnyTls, ProtocolConfig::AnyTls { password }) => {
            // sing-box ≥ 1.12; TLS is required on the outbound.
            json!({
                "type": "anytls",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "password": password,
            })
        }
        (
            Protocol::Snell,
            ProtocolConfig::Snell {
                psk,
                version,
                userkey,
                reuse,
                obfs_mode,
                obfs_host,
                mode,
            },
        ) => {
            // sing-box ≥ 1.14; accepts version 4 or 6 (v1–3/v5 may fail at core runtime).
            let ver = match *version {
                6 => 6,
                // v5 wire ≈ v4 per sing-box docs
                1 | 2 | 3 | 4 | 5 => 4,
                other => other,
            };
            let mut o = json!({
                "type": "snell",
                "tag": tag.clone(),
                "server": node.server,
                "server_port": node.port,
                "version": ver,
                "psk": psk,
            });
            if let Some(uk) = userkey {
                if !uk.is_empty() {
                    o["userkey"] = json!(uk);
                }
            }
            if let Some(true) = reuse {
                o["reuse"] = json!(true);
            }
            if ver == 6 {
                if let Some(m) = mode {
                    let m = m.replace('_', "-").to_ascii_lowercase();
                    if matches!(m.as_str(), "default" | "unshaped" | "unsafe-raw") {
                        o["mode"] = json!(m);
                    }
                }
            } else {
                // v4: HTTP obfs only (`none` | `http`). Clash also uses `tls` → map to none.
                if let Some(m) = obfs_mode {
                    let m = m.to_ascii_lowercase();
                    if m == "http" {
                        o["obfs_mode"] = json!("http");
                        if let Some(h) = obfs_host {
                            if !h.is_empty() {
                                o["obfs_host"] = json!(h);
                            }
                        }
                    }
                }
            }
            o
        }
        _ => {
            return Err(AppError::Config(format!(
                "protocol/config mismatch for {}",
                node.name
            )));
        }
    };

    if let Some(tls) = &node.tls {
        if let Some(tls_val) = tls_to_json(tls) {
            ob.as_object_mut()
                .ok_or_else(|| AppError::Config("outbound not object".into()))?
                .insert("tls".into(), tls_val);
        }
    }

    // AnyTLS requires a TLS block in sing-box.
    if matches!(node.protocol, Protocol::AnyTls) {
        let obj = ob
            .as_object_mut()
            .ok_or_else(|| AppError::Config("outbound not object".into()))?;
        if !obj.contains_key("tls") {
            obj.insert("tls".into(), json!({ "enabled": true }));
        }
    }

    if let Some(transport) = &node.transport {
        if let Some(t) = transport_to_json(transport) {
            ob.as_object_mut()
                .ok_or_else(|| AppError::Config("outbound not object".into()))?
                .insert("transport".into(), t);
        }
    }

    Ok((tag, ob))
}

/// Only emit known uTLS profile names (ignore hex pins / garbage from subscriptions).
fn normalize_utls_fingerprint(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    const VALID: &[&str] = &[
        "chrome",
        "firefox",
        "safari",
        "ios",
        "android",
        "edge",
        "360",
        "qq",
        "random",
        "chrome_psk",
        "chrome_psk_shuffle",
        "chrome_padding_psk_shuffle",
        "chrome_pq",
        "chrome_pq_psk",
    ];
    if VALID.contains(&s.as_str()) {
        Some(s)
    } else {
        None
    }
}

fn tls_to_json(tls: &TlsConfig) -> Option<Value> {
    if !tls.enabled && tls.reality_public_key.is_none() {
        return None;
    }
    let mut o = json!({ "enabled": true });
    if let Some(sni) = &tls.server_name {
        o["server_name"] = json!(sni);
    }
    if let Some(true) = tls.insecure {
        o["insecure"] = json!(true);
    }
    if let Some(alpn) = &tls.alpn {
        if !alpn.is_empty() {
            o["alpn"] = json!(alpn);
        }
    }
    if let Some(fp) = normalize_utls_fingerprint(tls.utls_fingerprint.as_deref()) {
        o["utls"] = json!({
            "enabled": true,
            "fingerprint": fp
        });
    }
    if let Some(pk) = &tls.reality_public_key {
        let mut reality = json!({
            "enabled": true,
            "public_key": pk
        });
        if let Some(sid) = &tls.reality_short_id {
            reality["short_id"] = json!(sid);
        }
        o["reality"] = reality;
    }
    Some(o)
}

fn transport_to_json(t: &Transport) -> Option<Value> {
    match t {
        Transport::Tcp => None,
        Transport::Ws {
            path,
            headers,
            max_early_data,
        } => {
            let mut o = json!({ "type": "ws" });
            if let Some(p) = path {
                o["path"] = json!(p);
            }
            if let Some(h) = headers {
                if !h.is_empty() {
                    o["headers"] = json!(h);
                }
            }
            if let Some(m) = max_early_data {
                o["max_early_data"] = json!(m);
            }
            Some(o)
        }
        Transport::Grpc { service_name } => {
            let mut o = json!({ "type": "grpc" });
            if let Some(s) = service_name {
                o["service_name"] = json!(s);
            }
            Some(o)
        }
        Transport::Http { path, host } => {
            let mut o = json!({ "type": "http" });
            if let Some(p) = path {
                o["path"] = json!(p);
            }
            if let Some(h) = host {
                o["host"] = json!(h);
            }
            Some(o)
        }
        Transport::HttpUpgrade { path, host } => {
            let mut o = json!({ "type": "httpupgrade" });
            if let Some(p) = path {
                o["path"] = json!(p);
            }
            if let Some(h) = host {
                o["host"] = json!(h);
            }
            Some(o)
        }
    }
}

pub fn generate_api_secret() -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}", std::time::SystemTime::now()).as_bytes());
    hasher.update(std::process::id().to_string().as_bytes());
    hasher.update(b"satelite-proxy-clash-api");
    hex::encode(hasher.finalize())[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Protocol, ProtocolConfig, ProxyNode, TlsConfig, Transport};
    use std::collections::BTreeMap;

    fn sample_ss() -> ProxyNode {
        ProxyNode {
            id: "aabbccddeeff0011".into(),
            name: "SS-HK".into(),
            protocol: Protocol::Shadowsocks,
            server: "ss.example.com".into(),
            port: 8388,
            tls: None,
            transport: None,
            udp: Some(true),
            config: ProtocolConfig::Shadowsocks {
                method: "aes-256-gcm".into(),
                password: "secret".into(),
                plugin: None,
                plugin_opts: None,
            },
            source: Some("ss".into()),
            latency_ms: None,
            latency_at: None,
        }
    }

    #[test]
    fn builds_selector() {
        let nodes = vec![sample_ss()];
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        assert_eq!(built.outbound_tags.len(), 1);
        assert_eq!(built.selected_tag, "node-aabbccddeeff0011");
        let inbounds = built.value["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 1);
        assert_eq!(inbounds[0]["type"], "mixed");
        assert!(built.value.get("dns").is_some());
        assert!(built.value["route"]
            .get("default_domain_resolver")
            .is_some());
        assert_eq!(built.value["route"]["final"], "proxy");
        let proxy = built.value["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == "proxy")
            .unwrap();
        assert_eq!(proxy["type"], "selector");
    }

    #[test]
    fn hosts_override_is_injected_before_user_route_rules() {
        use crate::domain::{HostsConfig, HostsEntry};

        let nodes = vec![sample_ss()];
        let dns = DnsSettings {
            hosts: HostsConfig {
                enabled: true,
                include_system: false,
                entries: vec![HostsEntry {
                    id: "baidu".into(),
                    enabled: true,
                    domain: "baidu.com".into(),
                    addr: "192.168.1.1".into(),
                }],
            },
            ..DnsSettings::default()
        };
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns,
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();

        let rules = built.value["route"]["rules"].as_array().unwrap();
        let host_rule = rules
            .iter()
            .find(|rule| rule["override_address"] == "192.168.1.1")
            .expect("hosts route override");
        assert_eq!(host_rule["domain"], json!(["baidu.com"]));
        assert_eq!(host_rule["action"], "route-options");
    }

    #[test]
    fn builds_urltest_when_kernel_auto_select() {
        let nodes = vec![sample_ss()];
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Kernel,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        let proxy = built.value["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["tag"] == "proxy")
            .unwrap();
        assert_eq!(proxy["type"], "urltest");
        assert_eq!(proxy["url"], "https://www.gstatic.com/generate_204");
        assert!(proxy["outbounds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t != "direct"));
    }

    #[test]
    fn builds_with_tun_inbound() {
        let nodes = vec![sample_ss()];
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![],
                tun_enabled: true,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        let inbounds = built.value["inbounds"].as_array().unwrap();
        assert_eq!(inbounds.len(), 2);
        assert_eq!(inbounds[1]["type"], "tun");
        assert_eq!(inbounds[1]["auto_route"], true);
        assert_eq!(inbounds[1]["stack"], "mixed");
        assert!(inbounds[1]["route_exclude_address"]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == "127.0.0.0/8")));
        assert!(built.value.get("dns").is_some());
        assert!(built.value["route"]
            .get("default_domain_resolver")
            .is_some());
        let rules = built.value["route"]["rules"].as_array().unwrap();
        assert!(rules
            .iter()
            .any(|r| r.get("action") == Some(&json!("sniff"))));
        assert!(rules
            .iter()
            .any(|r| r.get("action") == Some(&json!("hijack-dns"))));
    }

    #[test]
    fn maps_vmess_ws() {
        let mut headers = BTreeMap::new();
        headers.insert("Host".into(), "cdn.example.com".into());
        let node = ProxyNode {
            id: "vmessid000000001".into(),
            name: "VM".into(),
            protocol: Protocol::Vmess,
            server: "vm.example.com".into(),
            port: 443,
            tls: Some(TlsConfig {
                enabled: true,
                server_name: Some("cdn.example.com".into()),
                insecure: Some(true),
                alpn: None,
                utls_fingerprint: None,
                reality_public_key: None,
                reality_short_id: None,
            }),
            transport: Some(Transport::Ws {
                path: Some("/ray".into()),
                headers: Some(headers),
                max_early_data: None,
            }),
            udp: None,
            config: ProtocolConfig::Vmess {
                uuid: "11111111-1111-1111-1111-111111111111".into(),
                alter_id: 0,
                security: "auto".into(),
            },
            source: None,
            latency_ms: None,
            latency_at: None,
        };
        let (_, ob) = node_to_outbound(&node).unwrap();
        assert_eq!(ob["type"], "vmess");
        assert_eq!(ob["transport"]["type"], "ws");
    }

    #[test]
    fn empty_nodes_err() {
        let err = build_singbox_config(
            &[],
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "x".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("no nodes"));
    }

    #[test]
    fn outbound_mode_direct_final() {
        let nodes = vec![sample_ss()];
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Direct,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        assert_eq!(built.value["route"]["final"], "direct");
    }

    #[test]
    fn rule_mode_honors_route_final() {
        let nodes = vec![sample_ss()];
        for (rf, expect) in [("direct", "direct"), ("block", "block"), ("proxy", "proxy")] {
            let built = build_singbox_config(
                &nodes,
                &BuildOptions {
                    mixed_port: 2080,
                    api_port: 19090,
                    api_secret: "test".into(),
                    current_node_id: None,
                    log_level: "info".into(),
                    rules: vec![],
                    tun_enabled: false,
                    tun_stack: "mixed".into(),
                    dns: DnsSettings::default(),
                    outbound_mode: OutboundMode::Rule,
                    route_final: rf.into(),
                    auto_select: crate::domain::AutoSelectMode::Off,
                    probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
                },
            )
            .unwrap();
            assert_eq!(built.value["route"]["final"], expect, "rf={rf}");
        }
    }

    #[test]
    fn outbound_mode_global_skips_user_rules() {
        use crate::domain::{Rule, RuleTarget, RuleType};
        let nodes = vec![sample_ss()];
        let built = build_singbox_config(
            &nodes,
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![Rule::new(
                    RuleType::DomainSuffix,
                    "example.com".into(),
                    RuleTarget::Direct,
                    10,
                )],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Global,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        assert_eq!(built.value["route"]["final"], "proxy");
        let rules = built.value["route"]["rules"].as_array().unwrap();
        // only sniff (+ maybe dns hijack from dns settings)
        assert!(!rules.iter().any(|r| r.get("domain_suffix").is_some()));
    }

    #[test]
    fn rule_pin_node_routes_to_node_tag() {
        use crate::domain::{Rule, RuleTarget, RuleType};
        let node = sample_ss();
        let tag = outbound_tag(&node);
        let mut rule = Rule::new(
            RuleType::DomainSuffix,
            "chatgpt.com".into(),
            RuleTarget::Node,
            10,
        );
        rule.node_id = Some(node.id.clone());
        rule.node_name = Some(node.name.clone());
        let built = build_singbox_config(
            &[node],
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![rule],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        let rules = built.value["route"]["rules"].as_array().unwrap();
        let pinned = rules
            .iter()
            .find(|r| r.get("domain_suffix").is_some())
            .expect("pin rule");
        assert_eq!(pinned["outbound"], tag);
    }

    #[test]
    fn rule_pin_stale_node_falls_back_to_proxy() {
        use crate::domain::{Rule, RuleTarget, RuleType};
        let node = sample_ss();
        let mut rule = Rule::new(
            RuleType::DomainSuffix,
            "openai.com".into(),
            RuleTarget::Node,
            10,
        );
        rule.node_id = Some("deadbeefdeadbeef".into());
        rule.node_name = Some("gone".into());
        let built = build_singbox_config(
            &[node],
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![rule],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        let rules = built.value["route"]["rules"].as_array().unwrap();
        let pinned = rules
            .iter()
            .find(|r| r.get("domain_suffix").is_some())
            .expect("stale pin rule");
        assert_eq!(pinned["outbound"], "proxy");
    }

    #[test]
    fn smart_rule_builds_filtered_selector() {
        use crate::domain::{Rule, RuleTarget, RuleType};
        let mut hk = sample_ss();
        hk.id = "aaaaaaaaaaaaaaaa".into();
        hk.name = "香港 01".into();
        let mut sg = sample_ss();
        sg.id = "bbbbbbbbbbbbbbbb".into();
        sg.name = "新加坡 01".into();
        let mut rule = Rule::new(
            RuleType::DomainSuffix,
            "chatgpt.com".into(),
            RuleTarget::Smart,
            10,
        );
        rule.smart_exclude = vec!["香港".into()];
        let built = build_singbox_config(
            &[hk, sg.clone()],
            &BuildOptions {
                mixed_port: 2080,
                api_port: 19090,
                api_secret: "test".into(),
                current_node_id: None,
                log_level: "info".into(),
                rules: vec![rule.clone()],
                tun_enabled: false,
                tun_stack: "mixed".into(),
                dns: DnsSettings::default(),
                outbound_mode: OutboundMode::Rule,
                route_final: "proxy".into(),
                auto_select: crate::domain::AutoSelectMode::Off,
                probe_url: "https://www.gstatic.com/generate_204".into(),
                find_process: true,
            },
        )
        .unwrap();
        let group = rule.smart_outbound_tag();
        let outs = built.value["outbounds"].as_array().unwrap();
        let sel = outs
            .iter()
            .find(|o| o.get("tag") == Some(&json!(group)))
            .expect("smart selector");
        let pool = sel["outbounds"].as_array().unwrap();
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0], json!(outbound_tag(&sg)));
        let rules = built.value["route"]["rules"].as_array().unwrap();
        let routed = rules
            .iter()
            .find(|r| r.get("domain_suffix").is_some())
            .expect("smart route");
        assert_eq!(routed["outbound"], group);
    }
}

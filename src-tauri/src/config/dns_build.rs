//! Build sing-box 1.12+ `dns` object from [`DnsSettings`].

use crate::domain::{
    parse_dns_address, DnsAction, DnsMode, DnsServer, DnsServerRole, DnsSettings, DomainMatcher,
    ParsedDnsAddress,
};
use serde_json::{json, Value};

const TAG_LOCAL: &str = "dns-local";
const TAG_FAKEIP: &str = "dns-fakeip";
const TAG_BLOCK: &str = "dns-block";
const TAG_REMOTE_FALLBACK: &str = "dns-remote";
const TAG_CN_FALLBACK: &str = "dns-cn";

/// Result of DNS section build for injection into full config.
pub struct BuiltDns {
    pub dns: Value,
    /// Tag for `route.default_domain_resolver`.
    pub default_resolver: String,
    /// Whether route should include `hijack-dns` (TUN or settings.hijack).
    pub want_hijack: bool,
}

/// Build DNS config. Always produces a valid 1.12+ DNS block.
pub fn build_dns_section(settings: &DnsSettings, tun_enabled: bool) -> BuiltDns {
    let effective_mode = if !settings.enabled {
        DnsMode::System
    } else {
        settings.mode
    };

    match effective_mode {
        DnsMode::System => build_system(settings, tun_enabled),
        DnsMode::Smart => build_smart(settings, tun_enabled),
        DnsMode::Custom => build_custom(settings, tun_enabled),
    }
}

fn build_system(settings: &DnsSettings, tun_enabled: bool) -> BuiltDns {
    let dns = json!({
        "servers": [
            { "type": "local", "tag": TAG_LOCAL }
        ],
        "final": TAG_LOCAL,
        "independent_cache": settings.cache,
        "strategy": "prefer_ipv4"
    });
    BuiltDns {
        dns,
        default_resolver: TAG_LOCAL.into(),
        want_hijack: tun_enabled || settings.hijack,
    }
}

fn build_smart(settings: &DnsSettings, tun_enabled: bool) -> BuiltDns {
    let mut servers: Vec<Value> = Vec::new();
    let mut tag_local = TAG_LOCAL.to_string();
    let mut tag_cn = TAG_CN_FALLBACK.to_string();
    let mut tag_remote = TAG_REMOTE_FALLBACK.to_string();
    let mut have_cn = false;
    let mut have_remote = false;

    // Ensure local server
    if let Some(s) = settings
        .servers
        .iter()
        .find(|s| s.enabled && s.role == DnsServerRole::Local)
    {
        if let Some(v) = server_to_json(s) {
            tag_local = v["tag"].as_str().unwrap_or(TAG_LOCAL).to_string();
            servers.push(v);
        } else {
            servers.push(json!({ "type": "local", "tag": TAG_LOCAL }));
        }
    } else {
        servers.push(json!({ "type": "local", "tag": TAG_LOCAL }));
    }

    for s in settings.servers.iter().filter(|s| s.enabled) {
        match s.role {
            DnsServerRole::Domestic => {
                if let Some(v) = server_to_json(s) {
                    if !have_cn {
                        tag_cn = v["tag"].as_str().unwrap_or(TAG_CN_FALLBACK).to_string();
                        have_cn = true;
                    }
                    servers.push(v);
                }
            }
            DnsServerRole::Remote | DnsServerRole::Custom => {
                if let Some(v) = server_to_json(s) {
                    if s.role == DnsServerRole::Remote && !have_remote {
                        tag_remote = v["tag"].as_str().unwrap_or(TAG_REMOTE_FALLBACK).to_string();
                        have_remote = true;
                    }
                    servers.push(v);
                }
            }
            DnsServerRole::Local => {}
        }
    }

    if !have_cn {
        servers.push(json!({
            "type": "udp",
            "tag": TAG_CN_FALLBACK,
            "server": "223.5.5.5"
        }));
        tag_cn = TAG_CN_FALLBACK.into();
    }
    if !have_remote {
        servers.push(json!({
            "type": "https",
            "tag": TAG_REMOTE_FALLBACK,
            "server": "1.1.1.1"
        }));
        tag_remote = TAG_REMOTE_FALLBACK.into();
    }

    let fake_ip_on = settings.fake_ip.enabled;
    if fake_ip_on {
        let mut fi = json!({
            "type": "fakeip",
            "tag": TAG_FAKEIP,
            "inet4_range": settings.fake_ip.inet4_range,
        });
        if settings.fake_ip.inet6_enabled {
            fi["inet6_range"] = json!(settings.fake_ip.inet6_range);
        }
        servers.push(fi);
    }

    // rcode block server not needed if we skip block rules; add lightweight reject via empty?
    // sing-box has no simple "block" dns server — use local for Block as safe fallback, or omit.

    let mut rules: Vec<Value> = Vec::new();

    // 1) User whitelist / custom rules
    for r in settings.rules.iter().filter(|r| r.enabled) {
        if let Some(rule) = user_rule_to_json(r, &tag_local, &tag_cn, &tag_remote, fake_ip_on) {
            rules.push(rule);
        }
    }

    // 2) FakeIP bypass suffixes → local
    if fake_ip_on {
        let suffixes: Vec<String> = settings
            .fake_ip
            .bypass
            .iter()
            .map(|s| normalize_suffix(s))
            .filter(|s| !s.is_empty())
            .collect();
        if !suffixes.is_empty() {
            rules.push(json!({
                "domain_suffix": suffixes,
                "server": tag_local
            }));
        }
    }

    // 3) FakeIP for A/AAAA (after bypass)
    if fake_ip_on {
        rules.push(json!({
            "query_type": ["A", "AAAA"],
            "server": TAG_FAKEIP
        }));
    }

    let dns = json!({
        "servers": servers,
        "rules": rules,
        "final": tag_remote,
        "independent_cache": settings.cache,
        "strategy": "prefer_ipv4"
    });

    // tag_cn retained for Domestic DNS rules above; keep leak_protect flag noted.
    let _ = (settings.leak_protect, tag_cn);

    BuiltDns {
        dns,
        default_resolver: tag_local,
        want_hijack: tun_enabled || settings.hijack,
    }
}

fn build_custom(settings: &DnsSettings, tun_enabled: bool) -> BuiltDns {
    let mut servers: Vec<Value> = Vec::new();
    let mut first_tag = TAG_LOCAL.to_string();
    let mut have_local = false;

    for s in settings.servers.iter().filter(|s| s.enabled) {
        if let Some(v) = server_to_json(s) {
            if !have_local {
                first_tag = v["tag"].as_str().unwrap_or(TAG_LOCAL).to_string();
                have_local = true;
            }
            servers.push(v);
        }
    }
    if servers.is_empty() {
        servers.push(json!({ "type": "local", "tag": TAG_LOCAL }));
        first_tag = TAG_LOCAL.into();
    }

    let tag_local = settings
        .servers
        .iter()
        .find(|s| s.enabled && (s.role == DnsServerRole::Local || s.address == "local"))
        .map(|s| server_tag(s))
        .unwrap_or_else(|| TAG_LOCAL.into());

    let tag_cn = settings
        .servers
        .iter()
        .find(|s| s.enabled && s.role == DnsServerRole::Domestic)
        .map(|s| server_tag(s))
        .unwrap_or_else(|| first_tag.clone());

    let tag_remote = settings
        .servers
        .iter()
        .find(|s| s.enabled && s.role == DnsServerRole::Remote)
        .map(|s| server_tag(s))
        .unwrap_or_else(|| first_tag.clone());

    let fake_ip_on = settings.fake_ip.enabled;
    if fake_ip_on {
        let mut fi = json!({
            "type": "fakeip",
            "tag": TAG_FAKEIP,
            "inet4_range": settings.fake_ip.inet4_range,
        });
        if settings.fake_ip.inet6_enabled {
            fi["inet6_range"] = json!(settings.fake_ip.inet6_range);
        }
        servers.push(fi);
    }

    let mut rules: Vec<Value> = Vec::new();
    for r in settings.rules.iter().filter(|r| r.enabled) {
        if let Some(rule) = user_rule_to_json(r, &tag_local, &tag_cn, &tag_remote, fake_ip_on) {
            rules.push(rule);
        }
    }
    if fake_ip_on {
        let suffixes: Vec<String> = settings
            .fake_ip
            .bypass
            .iter()
            .map(|s| normalize_suffix(s))
            .filter(|s| !s.is_empty())
            .collect();
        if !suffixes.is_empty() {
            rules.push(json!({
                "domain_suffix": suffixes,
                "server": tag_local
            }));
        }
        rules.push(json!({
            "query_type": ["A", "AAAA"],
            "server": TAG_FAKEIP
        }));
    }

    let dns = json!({
        "servers": servers,
        "rules": rules,
        "final": tag_remote,
        "independent_cache": settings.cache,
        "strategy": "prefer_ipv4"
    });

    BuiltDns {
        dns,
        default_resolver: tag_local,
        want_hijack: tun_enabled || settings.hijack,
    }
}

fn server_tag(s: &DnsServer) -> String {
    format!("dns-{}", s.id)
}

fn server_to_json(s: &DnsServer) -> Option<Value> {
    let tag = server_tag(s);
    let parsed = parse_dns_address(&s.address)?;
    let v = match parsed {
        ParsedDnsAddress::Local => json!({
            "type": "local",
            "tag": tag,
        }),
        ParsedDnsAddress::Udp { server, port } => {
            let mut o = json!({
                "type": "udp",
                "tag": tag,
                "server": server,
            });
            if let Some(p) = port {
                o["server_port"] = json!(p);
            }
            o
        }
        ParsedDnsAddress::Tcp { server, port } => {
            let mut o = json!({
                "type": "tcp",
                "tag": tag,
                "server": server,
            });
            if let Some(p) = port {
                o["server_port"] = json!(p);
            }
            o
        }
        ParsedDnsAddress::Https { server, path } => {
            let mut o = json!({
                "type": "https",
                "tag": tag,
                "server": server,
            });
            if let Some(p) = path {
                o["path"] = json!(p);
            }
            o
        }
        ParsedDnsAddress::Tls { server, port } => {
            let mut o = json!({
                "type": "tls",
                "tag": tag,
                "server": server,
            });
            if let Some(p) = port {
                o["server_port"] = json!(p);
            }
            o
        }
    };
    let _ = TAG_BLOCK;
    Some(v)
}

fn user_rule_to_json(
    r: &crate::domain::DnsRule,
    tag_local: &str,
    tag_cn: &str,
    tag_remote: &str,
    fake_ip_on: bool,
) -> Option<Value> {
    let payload = r.payload.trim();
    if payload.is_empty() {
        return None;
    }
    let payload = match r.matcher {
        DomainMatcher::DomainSuffix => normalize_suffix(payload),
        _ => payload.trim_start_matches('.').to_string(),
    };
    if payload.is_empty() {
        return None;
    }

    let server = match &r.action {
        DnsAction::System => tag_local.to_string(),
        DnsAction::Domestic => tag_cn.to_string(),
        DnsAction::Remote => tag_remote.to_string(),
        DnsAction::Server { server_id } => format!("dns-{server_id}"),
        DnsAction::Block => tag_local.to_string(), // soft block: resolve via system
        DnsAction::FakeIp => {
            if fake_ip_on {
                TAG_FAKEIP.to_string()
            } else {
                tag_remote.to_string()
            }
        }
    };

    let mut rule = match r.matcher {
        DomainMatcher::Domain => json!({ "domain": [payload] }),
        DomainMatcher::DomainSuffix => json!({ "domain_suffix": [payload] }),
        DomainMatcher::DomainKeyword => json!({ "domain_keyword": [payload] }),
    };
    rule.as_object_mut()?
        .insert("server".into(), json!(server));
    Some(rule)
}

fn normalize_suffix(s: &str) -> String {
    s.trim()
        .trim_start_matches('*')
        .trim_start_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DnsSettings;

    #[test]
    fn smart_default_has_fakeip_and_local() {
        let s = DnsSettings::default();
        let b = build_dns_section(&s, false);
        let servers = b.dns["servers"].as_array().unwrap();
        assert!(servers.iter().any(|x| x["type"] == "local"));
        assert!(servers.iter().any(|x| x["type"] == "fakeip"));
        assert_eq!(b.dns["final"].as_str().unwrap().contains("dns-"), true);
        assert!(!b.want_hijack || s.hijack);
    }

    #[test]
    fn system_mode_only_local() {
        let mut s = DnsSettings::default();
        s.mode = DnsMode::System;
        let b = build_dns_section(&s, true);
        let servers = b.dns["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["type"], "local");
        assert!(b.want_hijack);
    }
}

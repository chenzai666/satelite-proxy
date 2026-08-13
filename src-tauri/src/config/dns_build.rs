//! Build sing-box 1.12+ `dns` object from [`DnsSettings`].
//!
//! Each unified rule set chooses its own resolver. This module defines the
//! resolver pool, unmatched-query default, Hosts, and FakeIP behavior.

use crate::config::punycode::to_ascii_domain;
use crate::domain::{
    read_system_hosts_pairs, DnsAction, DnsRule, DnsSettings, DomainMatcher, FakeIpConfig,
    HostsConfig, Rule,
};
use serde_json::{json, Value};

/// Fixed server tags (servers are no longer user-editable).
const TAG_LOCAL: &str = "dns-local";
const TAG_CN: &str = "dns-cn";
const TAG_REMOTE: &str = "dns-remote";
const TAG_FAKEIP: &str = "dns-fakeip";
/// Tag for the static hosts `predefined` server (highest-priority DNS answers).
const TAG_HOSTS: &str = "dns-hosts";

/// Result of DNS section build for injection into full config.
pub struct BuiltDns {
    pub dns: Value,
    /// Tag for `route.default_domain_resolver`.
    pub default_resolver: String,
    /// Whether route should include `hijack-dns` (TUN or settings.hijack).
    pub want_hijack: bool,
}

/// Build DNS config. Always produces a valid 1.12+ DNS block.
///
pub fn build_dns_section(
    settings: &DnsSettings,
    tun_enabled: bool,
    _route_rules: &[Rule],
) -> BuiltDns {
    let mut effective = settings.clone();
    effective.rules = settings.enabled_dns_rules();
    effective.rules_enabled = settings.has_enabled_dns_sets();
    effective.hosts = settings.effective_hosts();
    let settings = &effective;
    // Reserved for future strategy tuning; referenced to avoid dead-code warnings.
    let _ = settings.leak_protect;
    let hijack = tun_enabled || settings.hijack;
    // DNS final is configured independently on the DNS page (local/domestic/remote);
    // it no longer follows the routing `final`.
    let final_tag = dns_final_tag(settings.normalize_dns_final());

    build_default(settings, hijack, final_tag)
}

/// Map the DNS `final` strategy to a server tag.
/// `local` → dns-local · `domestic` → dns-cn · otherwise → dns-remote.
fn dns_final_tag(dns_final: &str) -> &'static str {
    match dns_final {
        "local" => TAG_LOCAL,
        "domestic" => TAG_CN,
        _ => TAG_REMOTE,
    }
}

/// Hard-coded sing-box server definitions (local + Ali + Tencent + Cloudflare).
///
/// Note: only IP-literal server addresses are used here. Domain-name addresses
/// (e.g. `dns.google`) would require a `domain_resolver`, creating a bootstrap
/// dependency — IPs avoid that entirely.
fn builtin_servers(fake_ip: &FakeIpConfig) -> Vec<Value> {
    let mut servers = vec![
        json!({ "type": "local", "tag": TAG_LOCAL }),
        json!({ "type": "udp", "tag": TAG_CN, "server": "223.5.5.5" }),
        json!({ "type": "udp", "tag": "dns-cn-tencent", "server": "119.29.29.29" }),
        json!({ "type": "https", "tag": TAG_REMOTE, "server": "1.1.1.1" }),
    ];
    if fake_ip.enabled {
        let mut fi = json!({
            "type": "fakeip",
            "tag": TAG_FAKEIP,
            "inet4_range": fake_ip.inet4_range,
        });
        if fake_ip.inet6_enabled {
            fi["inet6_range"] = json!(fake_ip.inet6_range);
        }
        servers.push(fi);
    }
    servers
}

/// FakeIP rules: bypass suffixes → local, then A/AAAA → fakeip. Empty if FakeIP off.
fn fakeip_rules(fake_ip: &FakeIpConfig, tag_local: &str) -> Vec<Value> {
    if !fake_ip.enabled {
        return Vec::new();
    }
    let mut out = Vec::new();
    let suffixes: Vec<String> = fake_ip
        .bypass
        .iter()
        .map(|s| normalize_suffix(s))
        .filter(|s| !s.is_empty())
        .collect();
    if !suffixes.is_empty() {
        out.push(json!({ "domain_suffix": suffixes, "server": tag_local }));
    }
    out.push(json!({ "query_type": ["A", "AAAA"], "server": TAG_FAKEIP }));
    out
}

/// Collect enabled hosts entries (user + optionally system) into `(domain, ip)` pairs.
///
/// User entries take precedence (first wins on duplicate domain); system entries are
/// only appended when `include_system` is on and the domain isn't already mapped.
fn collect_hosts(hosts: &HostsConfig) -> Vec<(String, String)> {
    let mut map: Vec<(String, String)> = Vec::new();
    for entry in hosts.entries.iter().filter(|e| e.enabled) {
        // Hosts entries are always exact matches (no keyword semantics), so
        // Punycode-encoding is safe and necessary — sing-box's `predefined`
        // map and route-options overrides match against wire-format ASCII.
        let domain = to_ascii_domain(
            entry
                .domain
                .trim()
                .trim_end_matches('.')
                .to_ascii_lowercase()
                .as_str(),
        );
        let addr = entry.addr.trim();
        if domain.is_empty()
            || addr.parse::<std::net::IpAddr>().is_err()
            || map.iter().any(|(d, _)| d == &domain)
        {
            continue;
        }
        map.push((domain, addr.to_string()));
    }
    if hosts.include_system {
        for (domain, ip) in read_system_hosts_pairs() {
            if !map.iter().any(|(d, _)| d.eq_ignore_ascii_case(&domain)) {
                map.push((domain, ip));
            }
        }
    }
    map
}

/// Return the configured static addresses for an exact host name.
///
/// This is also used by the UI diagnostic so it follows the same precedence and
/// validation rules as the generated sing-box configuration.
pub fn lookup_hosts(hosts: &HostsConfig, host: &str) -> Vec<String> {
    if !hosts.enabled {
        return Vec::new();
    }
    let host = host.trim().trim_end_matches('.');
    collect_hosts(hosts)
        .into_iter()
        .filter_map(|(domain, addr)| domain.eq_ignore_ascii_case(host).then_some(addr))
        .collect()
}

/// Build route-stage destination overrides for Hosts entries.
///
/// Mixed/system-proxy traffic can carry a domain straight to a proxy outbound,
/// without issuing a DNS query. DNS rules alone therefore cannot implement Hosts
/// semantics for that traffic. `route-options.override_address` makes the static
/// mapping apply to both proxied domain connections and ordinary DNS lookups.
pub fn build_hosts_route_rules(hosts: &HostsConfig) -> Vec<Value> {
    if !hosts.enabled {
        return Vec::new();
    }
    collect_hosts(hosts)
        .into_iter()
        .map(|(domain, addr)| {
            json!({
                "domain": [domain],
                "action": "route-options",
                "override_address": addr
            })
        })
        .collect()
}

/// Build the hosts layer: a `predefined` server + a single domain rule pointing at it.
///
/// Returns `None` when hosts are disabled or produce no mappings. When `Some`, the
/// caller must push the server into `servers` and **prepend** the rule to `rules`
/// (index 0) so hosts answers beat every other DNS rule.
fn hosts_layer(hosts: &HostsConfig) -> Option<(Value, Value)> {
    if !hosts.enabled {
        return None;
    }
    let pairs = collect_hosts(hosts);
    if pairs.is_empty() {
        return None;
    }
    // sing-box `hosts` server: `predefined` maps domain → [ip].
    let predefined: serde_json::Map<String, Value> = pairs
        .iter()
        .map(|(d, ip)| (d.clone(), json!([ip])))
        .collect();
    let server = json!({
        "type": "hosts",
        "tag": TAG_HOSTS,
        "predefined": serde_json::Value::Object(predefined),
    });
    let domains: Vec<String> = pairs.into_iter().map(|(d, _)| d).collect();
    let rule = json!({ "domain": domains, "server": TAG_HOSTS });
    Some((server, rule))
}

/// Global DNS baseline. Unified rule-set DNS rules are prepended later by the
/// top-level builder and therefore override FakeIP and the unmatched default.
fn build_default(settings: &DnsSettings, hijack: bool, final_tag: &str) -> BuiltDns {
    let mut servers = builtin_servers(&settings.fake_ip);
    let mut rules: Vec<Value> = Vec::new();

    if let Some((host_srv, host_rule)) = hosts_layer(&settings.hosts) {
        servers.push(host_srv);
        rules.push(host_rule);
    }
    rules.extend(
        settings
            .rules
            .iter()
            .filter(|r| r.enabled)
            .filter_map(user_rule_to_json),
    );
    rules.extend(fakeip_rules(&settings.fake_ip, TAG_LOCAL));

    let dns = json!({
        "servers": servers,
        "rules": rules,
        "final": final_tag,
        "independent_cache": settings.cache,
        "strategy": "prefer_ipv4"
    });
    BuiltDns {
        dns,
        default_resolver: TAG_LOCAL.into(),
        want_hijack: hijack,
    }
}

/// One DNS-page rule → sing-box rule. Servers are the builtin tags.
fn user_rule_to_json(r: &DnsRule) -> Option<Value> {
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

    // sing-box matches wire-format QNAME, which is always ASCII. Keyword is a
    // substring match — Punycode-encoding it would break that semantic.
    let mut rule = match r.matcher {
        DomainMatcher::Domain => json!({ "domain": [to_ascii_domain(&payload)] }),
        DomainMatcher::DomainSuffix => json!({ "domain_suffix": [to_ascii_domain(&payload)] }),
        DomainMatcher::DomainKeyword => json!({ "domain_keyword": [payload] }),
    };
    if r.action == DnsAction::Block {
        rule.as_object_mut()?
            .insert("action".into(), json!("reject"));
    } else {
        let server = match r.action {
            DnsAction::Local => TAG_LOCAL,
            DnsAction::Domestic => TAG_CN,
            DnsAction::Remote => TAG_REMOTE,
            DnsAction::Block => unreachable!(),
        };
        rule.as_object_mut()?.insert("server".into(), json!(server));
    }
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
    fn dns_final_follows_dns_final_setting() {
        let mut s = DnsSettings::default();
        s.dns_final = "local".into();
        let b = build_dns_section(&s, false, &[]);
        assert_eq!(b.dns["final"].as_str().unwrap(), "dns-local");

        let mut s = DnsSettings::default();
        s.dns_final = "domestic".into();
        let b = build_dns_section(&s, false, &[]);
        assert_eq!(b.dns["final"].as_str().unwrap(), "dns-cn");
    }

    #[test]
    fn legacy_resolution_mode_no_longer_changes_dns_output() {
        let mut local = DnsSettings::default();
        local.mode = crate::domain::DnsMode::Local;
        let mut old_smart = local.clone();
        old_smart.mode = crate::domain::DnsMode::SmartCn;

        let local_dns = build_dns_section(&local, false, &[]).dns;
        let old_smart_dns = build_dns_section(&old_smart, false, &[]).dns;
        assert_eq!(local_dns, old_smart_dns);
    }

    #[test]
    fn enabled_legacy_dns_rules_are_preserved() {
        use crate::domain::{DnsAction, DnsRule, DomainMatcher, Rule, RuleTarget, RuleType};
        let s = DnsSettings {
            rules_enabled: true,
            rules: vec![DnsRule {
                id: "force-remote".into(),
                enabled: true,
                matcher: DomainMatcher::DomainSuffix,
                payload: "x.com".into(),
                action: DnsAction::Remote,
            }],
            fake_ip: FakeIpConfig {
                enabled: false,
                ..FakeIpConfig::default()
            },
            ..DnsSettings::default()
        };
        // Route rule says x.com → direct (which would project to local).
        let route = vec![Rule::new(
            RuleType::DomainSuffix,
            "x.com".into(),
            RuleTarget::Direct,
            1,
        )];
        let b = build_dns_section(&s, false, &route);
        let dns_rules = b.dns["rules"].as_array().unwrap();
        // First matching rule for x.com must be the user DNS rule → remote.
        let first_for_x = dns_rules
            .iter()
            .find(|x| {
                x.get("domain_suffix").is_some_and(|a| {
                    a.as_array()
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some("x.com")))
                })
            })
            .expect("a rule for x.com");
        assert_eq!(first_for_x["server"].as_str().unwrap(), "dns-remote");
    }

    #[test]
    fn enabled_legacy_dns_rules_layer_onto_default() {
        use crate::domain::{DnsAction, DnsRule, DomainMatcher};
        let s = DnsSettings {
            rules_enabled: true,
            dns_final: "local".into(),
            rules: vec![DnsRule {
                id: "force-remote".into(),
                enabled: true,
                matcher: DomainMatcher::Domain,
                payload: "remote.example".into(),
                action: DnsAction::Remote,
            }],
            ..DnsSettings::default()
        };
        let b = build_dns_section(&s, false, &[]);
        assert_eq!(b.dns["final"], TAG_LOCAL);
        let rules = b.dns["rules"].as_array().unwrap();
        assert_eq!(rules[0]["domain"], json!(["remote.example"]));
        assert_eq!(rules[0]["server"], TAG_REMOTE);
    }

    #[test]
    fn disabled_legacy_dns_rules_do_not_affect_default() {
        use crate::domain::{DnsAction, DnsRule, DomainMatcher};
        let s = DnsSettings {
            rules_enabled: false,
            rules: vec![DnsRule {
                id: "disabled-layer".into(),
                enabled: true,
                matcher: DomainMatcher::Domain,
                payload: "not-projected.example".into(),
                action: DnsAction::Remote,
            }],
            fake_ip: FakeIpConfig {
                enabled: false,
                ..FakeIpConfig::default()
            },
            ..DnsSettings::default()
        };
        let b = build_dns_section(&s, false, &[]);
        let rules = b.dns["rules"].as_array().unwrap();
        assert!(rules
            .iter()
            .all(|r| r["domain"] != json!(["not-projected.example"])));
    }

    #[test]
    fn hosts_layer_emits_predefined_server_and_rule() {
        use crate::domain::{HostsConfig, HostsEntry};
        let s = DnsSettings {
            hosts: HostsConfig {
                enabled: true,
                include_system: false,
                entries: vec![
                    HostsEntry {
                        id: "h1".into(),
                        enabled: true,
                        domain: "my.host".into(),
                        addr: "10.0.0.5".into(),
                    },
                    HostsEntry {
                        id: "h2".into(),
                        enabled: false, // disabled — must be skipped
                        domain: "skip.me".into(),
                        addr: "1.2.3.4".into(),
                    },
                ],
            },
            fake_ip: FakeIpConfig {
                enabled: false,
                ..FakeIpConfig::default()
            },
            ..DnsSettings::default()
        };
        let b = build_dns_section(&s, false, &[]);
        let servers = b.dns["servers"].as_array().unwrap();
        // hosts server present
        let host_srv = servers
            .iter()
            .find(|x| x["type"] == "hosts")
            .expect("hosts server emitted");
        assert_eq!(host_srv["tag"].as_str().unwrap(), "dns-hosts");
        assert_eq!(
            host_srv["predefined"]["my.host"][0].as_str().unwrap(),
            "10.0.0.5"
        );
        assert!(host_srv["predefined"]
            .as_object()
            .unwrap()
            .get("skip.me")
            .is_none());

        // hosts rule is first, points at dns-hosts, only contains enabled domain
        let rules = b.dns["rules"].as_array().unwrap();
        assert_eq!(rules[0]["server"].as_str().unwrap(), "dns-hosts");
        let domains = rules[0]["domain"].as_array().unwrap();
        assert_eq!(domains.len(), 1);
        assert_eq!(domains[0].as_str().unwrap(), "my.host");
    }

    #[test]
    fn hosts_route_override_applies_to_system_proxy_domains() {
        use crate::domain::{HostsConfig, HostsEntry};
        let hosts = HostsConfig {
            enabled: true,
            include_system: false,
            entries: vec![HostsEntry {
                id: "baidu".into(),
                enabled: true,
                domain: "Baidu.com.".into(),
                addr: "192.168.1.1".into(),
            }],
        };

        assert_eq!(lookup_hosts(&hosts, "baidu.com"), vec!["192.168.1.1"]);
        let rules = build_hosts_route_rules(&hosts);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0]["domain"], json!(["baidu.com"]));
        assert_eq!(rules[0]["action"], "route-options");
        assert_eq!(rules[0]["override_address"], "192.168.1.1");
    }

    #[test]
    fn hosts_disabled_emits_nothing() {
        use crate::domain::{HostsConfig, HostsEntry};
        let s = DnsSettings {
            hosts: HostsConfig {
                enabled: false,
                include_system: false,
                entries: vec![HostsEntry {
                    id: "h1".into(),
                    enabled: true,
                    domain: "my.host".into(),
                    addr: "10.0.0.5".into(),
                }],
            },
            ..DnsSettings::default()
        };
        let b = build_dns_section(&s, false, &[]);
        let servers = b.dns["servers"].as_array().unwrap();
        assert!(servers.iter().all(|x| x["type"] != "hosts"));
    }

    #[test]
    fn hosts_work_with_default_resolver() {
        use crate::domain::{HostsConfig, HostsEntry};
        let s = DnsSettings {
            hosts: HostsConfig {
                enabled: true,
                include_system: false,
                entries: vec![HostsEntry {
                    id: "h1".into(),
                    enabled: true,
                    domain: "local.host".into(),
                    addr: "127.0.0.1".into(),
                }],
            },
            ..DnsSettings::default()
        };
        let b = build_dns_section(&s, false, &[]);
        let servers = b.dns["servers"].as_array().unwrap();
        assert!(servers.iter().any(|x| x["type"] == "hosts"));
        let rules = b.dns["rules"].as_array().unwrap();
        assert!(!rules.is_empty());
        assert_eq!(rules[0]["server"].as_str().unwrap(), "dns-hosts");
    }

    #[test]
    fn user_dns_rule_converts_domain_and_suffix_to_punycode_but_not_keyword() {
        use crate::domain::{DnsAction, DnsRule, DomainMatcher};
        let domain_rule = DnsRule {
            id: "d1".into(),
            enabled: true,
            matcher: DomainMatcher::Domain,
            payload: "中文.com".into(),
            action: DnsAction::Remote,
        };
        let suffix_rule = DnsRule {
            id: "d2".into(),
            enabled: true,
            matcher: DomainMatcher::DomainSuffix,
            payload: "中国.com".into(),
            action: DnsAction::Remote,
        };
        let keyword_rule = DnsRule {
            id: "d3".into(),
            enabled: true,
            matcher: DomainMatcher::DomainKeyword,
            payload: "中文".into(),
            action: DnsAction::Remote,
        };
        assert_eq!(
            user_rule_to_json(&domain_rule).unwrap()["domain"],
            json!(["xn--fiq228c.com"])
        );
        assert_eq!(
            user_rule_to_json(&suffix_rule).unwrap()["domain_suffix"],
            json!(["xn--fiqs8s.com"])
        );
        assert_eq!(
            user_rule_to_json(&keyword_rule).unwrap()["domain_keyword"],
            json!(["中文"])
        );
    }

    #[test]
    fn hosts_domain_is_converted_to_punycode_in_predefined_and_dns_rule() {
        use crate::domain::{HostsConfig, HostsEntry};
        let s = DnsSettings {
            hosts: HostsConfig {
                enabled: true,
                include_system: false,
                entries: vec![HostsEntry {
                    id: "h1".into(),
                    enabled: true,
                    domain: "中文.com".into(),
                    addr: "10.0.0.5".into(),
                }],
            },
            fake_ip: FakeIpConfig {
                enabled: false,
                ..FakeIpConfig::default()
            },
            ..DnsSettings::default()
        };
        let b = build_dns_section(&s, false, &[]);
        let servers = b.dns["servers"].as_array().unwrap();
        let host_srv = servers
            .iter()
            .find(|x| x["type"] == "hosts")
            .expect("hosts server emitted");
        assert_eq!(
            host_srv["predefined"]["xn--fiq228c.com"],
            json!(["10.0.0.5"])
        );
        let rules = b.dns["rules"].as_array().unwrap();
        assert_eq!(rules[0]["domain"], json!(["xn--fiq228c.com"]));

        let route_rules = build_hosts_route_rules(&s.hosts);
        assert_eq!(route_rules[0]["domain"], json!(["xn--fiq228c.com"]));
        assert_eq!(route_rules[0]["override_address"], json!("10.0.0.5"));
    }
}

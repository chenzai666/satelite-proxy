//! Build sing-box 1.12+ `dns` object from [`DnsSettings`].
//!
//! Resolution modes (see `DnsMode`):
//! - `local`:      local resolver by default.
//! - `smart_local`: route-derived — `direct` domain rules → local DNS, else → remote DNS.
//! - `smart_cn`:   route-derived — `direct` domain rules → domestic DNS, else → remote DNS.
//! User DNS rules are an independent layer. When enabled, they are projected **first**
//! in every base mode so they override the mode's normal behavior.

use crate::domain::{
    read_system_hosts_pairs, DnsAction, DnsMode, DnsRule, DnsSettings, DomainMatcher, FakeIpConfig,
    HostsConfig, Rule, RuleTarget, RuleType,
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
/// `route_rules`: enabled routing rules (rule page). In route-derived modes, each
/// domain-like rule contributes a DNS rule: `direct` target → direct-tag, otherwise →
/// remote-tag. Enabled DNS-page user rules are projected first (they win).
///
/// `route_final`: the normalized routing `final` (`"direct"` | `"proxy"` | `"block"`).
/// The DNS `final` follows it: `direct` → local resolver, otherwise → remote, so that
/// domains not covered by any rule resolve via the same path they'll be routed through.
pub fn build_dns_section(
    settings: &DnsSettings,
    tun_enabled: bool,
    route_rules: &[Rule],
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

    let rules_enabled = settings.rules_enabled;
    match settings.mode {
        DnsMode::Local if rules_enabled => build_local_with_rules(settings, hijack, final_tag),
        DnsMode::Local => build_local(settings, hijack, final_tag),
        DnsMode::SmartLocal => build_smart_variant(
            settings,
            hijack,
            route_rules,
            TAG_LOCAL,
            final_tag,
            rules_enabled,
        ),
        DnsMode::SmartCn => build_smart_variant(
            settings,
            hijack,
            route_rules,
            TAG_CN,
            final_tag,
            rules_enabled,
        ),
        // Legacy value is equivalent to SmartLocal with the rules layer enabled.
        DnsMode::Rules => {
            build_smart_variant(settings, hijack, route_rules, TAG_LOCAL, final_tag, true)
        }
    }
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
        let domain = entry
            .domain
            .trim()
            .trim_end_matches('.')
            .to_ascii_lowercase();
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

/// Pure local resolver. Hosts (if enabled) are honored as the highest priority.
fn build_local(settings: &DnsSettings, hijack: bool, final_tag: &str) -> BuiltDns {
    // In pure-local mode the server list normally only contains dns-local. When
    // the configured final points elsewhere (domestic/remote), include the full
    // builtin server set so the final resolver is actually defined.
    let need_all = final_tag != TAG_LOCAL;

    // Hosts override — works even in pure-local mode.
    if let Some((host_srv, host_rule)) = hosts_layer(&settings.hosts) {
        let mut servers: Vec<Value> = if need_all {
            builtin_servers(&settings.fake_ip)
        } else {
            vec![json!({ "type": "local", "tag": TAG_LOCAL })]
        };
        servers.push(host_srv);
        let dns = json!({
            "servers": servers,
            "rules": [host_rule],
            "final": final_tag,
            "independent_cache": settings.cache,
            "strategy": "prefer_ipv4"
        });
        return BuiltDns {
            dns,
            default_resolver: TAG_LOCAL.into(),
            want_hijack: hijack,
        };
    }

    let servers: Vec<Value> = if need_all {
        builtin_servers(&settings.fake_ip)
    } else {
        vec![json!({ "type": "local", "tag": TAG_LOCAL })]
    };
    let dns = json!({
        "servers": servers,
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

/// Local baseline with user DNS rules layered on top. The final resolver follows
/// the configured DNS `final` strategy, while individual rules may explicitly
/// select domestic or remote DNS.
fn build_local_with_rules(settings: &DnsSettings, hijack: bool, final_tag: &str) -> BuiltDns {
    let mut fake_ip_off = settings.fake_ip.clone();
    fake_ip_off.enabled = false;
    let mut servers = builtin_servers(&fake_ip_off);
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

/// Route-derived smart mode. `direct_tag` is `TAG_LOCAL` (smart_local) or `TAG_CN` (smart_cn):
/// domain-like rules with `direct` target → `direct_tag`, everything else → remote.
/// `final_tag` is the DNS `final` (derived from the routing final).
fn build_smart_variant(
    settings: &DnsSettings,
    hijack: bool,
    route_rules: &[Rule],
    direct_tag: &str,
    final_tag: &str,
    rules_enabled: bool,
) -> BuiltDns {
    let mut servers = builtin_servers(&settings.fake_ip);

    let mut rules: Vec<Value> = Vec::new();
    // 0) Hosts override — highest priority (prepended, sing-box first-match).
    if let Some((host_srv, host_rule)) = hosts_layer(&settings.hosts) {
        servers.push(host_srv);
        rules.push(host_rule);
    }
    // 1) Optional user DNS rules override the base mode.
    if rules_enabled {
        rules.extend(
            settings
                .rules
                .iter()
                .filter(|r| r.enabled)
                .filter_map(user_rule_to_json),
        );
    }
    // 2) Route-derived projection.
    rules.extend(project_route_dns(route_rules, direct_tag, TAG_REMOTE));
    // 3) FakeIP.
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

/// Project domain-like routing rules into DNS rules, **collapsing same-direction
/// entries into one rule per matcher type** (sing-box `domain_suffix`/`domain`/
/// `domain_keyword` accept arrays).
///
/// `direct` target → `direct_tag`; `proxy`/`node`/`smart`/`block` → `remote_tag`.
/// Non-domain types (IP-CIDR, PROCESS, GEOIP) are skipped (irrelevant to DNS).
fn project_route_dns(route_rules: &[Rule], direct_tag: &str, remote_tag: &str) -> Vec<Value> {
    // 6 slots: matcher(0..3) × direction(direct=0, remote=3).
    // Collapsing same-direction entries into one rule per matcher type keeps the
    // generated config small (sing-box domain/domain_suffix/domain_keyword accept arrays).
    let mut slots: [Vec<String>; 6] = Default::default();
    for r in route_rules
        .iter()
        .filter(|r| r.enabled && r.is_domain_like())
    {
        let payload = r.payload.trim();
        if payload.is_empty() {
            continue;
        }
        let m_idx = match r.rule_type {
            RuleType::Domain => 0,
            RuleType::DomainSuffix => 1,
            RuleType::DomainKeyword => 2,
            _ => continue,
        };
        let dir = if r.target == RuleTarget::Direct { 0 } else { 3 };
        slots[m_idx + dir].push(payload.to_string());
    }

    let keys = ["domain", "domain_suffix", "domain_keyword"];
    let mut out = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        // direct bucket
        if !slots[i].is_empty() {
            out.push(json!({ (*key): slots[i].clone(), "server": direct_tag }));
        }
        // remote bucket
        if !slots[i + 3].is_empty() {
            out.push(json!({ (*key): slots[i + 3].clone(), "server": remote_tag }));
        }
    }
    out
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

    let server = match r.action {
        DnsAction::Local => TAG_LOCAL,
        DnsAction::Domestic => TAG_CN,
        DnsAction::Remote => TAG_REMOTE,
    };

    let mut rule = match r.matcher {
        DomainMatcher::Domain => json!({ "domain": [payload] }),
        DomainMatcher::DomainSuffix => json!({ "domain_suffix": [payload] }),
        DomainMatcher::DomainKeyword => json!({ "domain_keyword": [payload] }),
    };
    rule.as_object_mut()?.insert("server".into(), json!(server));
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
    fn smart_local_default_has_fakeip_and_local() {
        let s = DnsSettings::default();
        assert!(matches!(s.mode, DnsMode::SmartLocal));
        let b = build_dns_section(&s, false, &[]);
        let servers = b.dns["servers"].as_array().unwrap();
        assert!(servers.iter().any(|x| x["type"] == "local"));
        assert!(servers.iter().any(|x| x["type"] == "fakeip"));
        // default dns_final=remote → DNS final → remote
        assert_eq!(b.dns["final"].as_str().unwrap(), "dns-remote");
        assert!(!b.want_hijack || s.hijack);
    }

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
    fn local_mode_only_local_server() {
        let mut s = DnsSettings::default();
        s.mode = DnsMode::Local;
        s.dns_final = "local".into();
        let b = build_dns_section(&s, true, &[]);
        let servers = b.dns["servers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["type"], "local");
        assert!(b.want_hijack);
    }

    #[test]
    fn smart_local_projects_direct_to_local_and_others_to_remote() {
        use crate::domain::{Rule, RuleTarget, RuleType};
        let s = DnsSettings {
            fake_ip: FakeIpConfig {
                enabled: false,
                ..FakeIpConfig::default()
            },
            ..DnsSettings::default()
        };
        let rules = vec![
            Rule::new(
                RuleType::DomainSuffix,
                "cn.example.com".into(),
                RuleTarget::Direct,
                1,
            ),
            Rule::new(
                RuleType::DomainSuffix,
                "fw.example.com".into(),
                RuleTarget::Proxy,
                2,
            ),
        ];
        let b = build_dns_section(&s, false, &rules);
        let dns_rules = b.dns["rules"].as_array().unwrap();
        let direct = dns_rules
            .iter()
            .find(|x| {
                x.get("domain_suffix").is_some_and(|a| {
                    a.as_array()
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some("cn.example.com")))
                })
            })
            .expect("direct rule projected");
        assert_eq!(direct["server"].as_str().unwrap(), "dns-local");
        let proxied = dns_rules
            .iter()
            .find(|x| {
                x.get("domain_suffix").is_some_and(|a| {
                    a.as_array()
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some("fw.example.com")))
                })
            })
            .expect("proxy rule projected");
        assert_eq!(proxied["server"].as_str().unwrap(), "dns-remote");
    }

    #[test]
    fn smart_local_collapses_same_direction_into_one_rule() {
        use crate::domain::{Rule, RuleTarget, RuleType};
        let s = DnsSettings {
            fake_ip: FakeIpConfig {
                enabled: false,
                ..FakeIpConfig::default()
            },
            ..DnsSettings::default()
        };
        let rules = vec![
            Rule::new(
                RuleType::DomainSuffix,
                "a.com".into(),
                RuleTarget::Direct,
                1,
            ),
            Rule::new(
                RuleType::DomainSuffix,
                "b.com".into(),
                RuleTarget::Direct,
                2,
            ),
            Rule::new(
                RuleType::DomainSuffix,
                "c.com".into(),
                RuleTarget::Direct,
                3,
            ),
        ];
        let b = build_dns_section(&s, false, &rules);
        let dns_rules = b.dns["rules"].as_array().unwrap();
        // Exactly one domain_suffix rule (all three direct suffixes collapsed).
        let suffix_rules: Vec<&Value> = dns_rules
            .iter()
            .filter(|x| x.get("domain_suffix").is_some())
            .collect();
        assert_eq!(
            suffix_rules.len(),
            1,
            "direct suffixes should collapse into one rule"
        );
        let arr = suffix_rules[0]["domain_suffix"].as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(suffix_rules[0]["server"].as_str().unwrap(), "dns-local");
    }

    #[test]
    fn smart_cn_projects_direct_to_domestic() {
        use crate::domain::{Rule, RuleTarget, RuleType};
        let s = DnsSettings {
            mode: DnsMode::SmartCn,
            fake_ip: FakeIpConfig {
                enabled: false,
                ..FakeIpConfig::default()
            },
            ..DnsSettings::default()
        };
        let rules = vec![Rule::new(
            RuleType::DomainSuffix,
            "cn.example.com".into(),
            RuleTarget::Direct,
            1,
        )];
        let b = build_dns_section(&s, false, &rules);
        let dns_rules = b.dns["rules"].as_array().unwrap();
        let direct = dns_rules
            .iter()
            .find(|x| {
                x.get("domain_suffix").is_some_and(|a| {
                    a.as_array()
                        .is_some_and(|v| v.iter().any(|v| v.as_str() == Some("cn.example.com")))
                })
            })
            .expect("direct rule projected");
        assert_eq!(direct["server"].as_str().unwrap(), "dns-cn");
    }

    #[test]
    fn enabled_dns_rules_override_smart_cn_route_projection() {
        use crate::domain::{DnsAction, DnsRule, DomainMatcher, Rule, RuleTarget, RuleType};
        let s = DnsSettings {
            mode: DnsMode::SmartCn,
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
    fn enabled_dns_rules_layer_onto_local_mode() {
        use crate::domain::{DnsAction, DnsRule, DomainMatcher};
        let s = DnsSettings {
            mode: DnsMode::Local,
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
    fn disabled_dns_rules_do_not_affect_smart_mode() {
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
    fn hosts_works_in_local_mode() {
        use crate::domain::{HostsConfig, HostsEntry};
        let s = DnsSettings {
            mode: DnsMode::Local,
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
}

//! Clash YAML config generation for the meow core (mihomo/Clash Meta in Rust).
//!
//! meow speaks the Clash dialect natively: proxies / proxy-groups / rules are
//! plain Clash YAML, and its REST API is Clash-compatible — so the generated
//! main group keeps the sing-box contract (`proxy`, hot-switched via
//! `PUT /proxies/proxy`; a url-test group in kernel auto-select mode reports
//! its current best through `now`, which the kernel-selection sync reads).
//!
//! Semantics mirrored from the sing-box/Xray generators (see
//! `builder.rs`/`xray.rs`): whole-set Node pins clamp onto each rule,
//! Filter sets route through a keyword-filtered pool group, per-rule Smart
//! pools get their own url-test group, and the three builtin remote sets map
//! onto GEOSITE/GEOIP matchers (MetaCubeX `.mrs`/mmdb geodata lives in the
//! meow home dir — see `core::assets`).

use crate::config::builder::{
    clamp_rule_pin_to_set, effective_route_rules, filter_pool_tags, outbound_tag,
    resolve_selected_tag, rule_set_is_empty_for_config, smart_pool_tags, BuildOptions,
};
use crate::config::punycode::to_ascii_domain;
use crate::core::kind::CoreKind;
use crate::domain::{
    DnsAction, DomainMatcher, OutboundMode, Protocol, ProtocolConfig, ProxyNode, Rule, RuleSet,
    RuleSetDnsStrategy, RuleSetStrategy, RuleTarget, RuleType, Transport,
};
use crate::error::{AppError, AppResult};
use serde_yaml::{Mapping, Value as Yaml};

/// Main group tag — must match the sing-box contract (`state.rs` selects
/// this group over the Clash API and the kernel-selection sync reads `now`).
const MAIN_GROUP: &str = "proxy";
/// Built-in remote DoH pool (Clash queries the entries concurrently,
/// fastest answer wins). Every entry egresses through the main proxy group
/// — direct DoH is unreachable on censored networks (see build_dns).
const REMOTE_DNS_POOL: [&str; 2] = ["https://1.1.1.1/dns-query", "https://8.8.8.8/dns-query"];
/// Built-in domestic plain-UDP pool (bootstrap, node hostnames, cn
/// classification). 114DNS backs up AliDNS.
const DOMESTIC_DNS_POOL: [&str; 2] = ["223.5.5.5", "114.114.114.114"];
/// meow url-test probe defaults.
const PROBE_INTERVAL_SECS: u64 = 300;
const PROBE_TOLERANCE_MS: u32 = 50;

#[derive(Debug)]
pub struct BuiltMeowConfig {
    /// Serialized Clash YAML document.
    pub yaml: String,
    /// Node proxy names included in the config (`node-<id>` tags).
    pub outbound_tags: Vec<String>,
    /// Tag of the node the main group selects by default.
    pub selected_tag: String,
}

/// Convert nodes into a complete Clash YAML config document for meow.
pub fn build_meow_config(nodes: &[ProxyNode], opts: &BuildOptions) -> AppResult<BuiltMeowConfig> {
    let mut skipped: Vec<String> = Vec::new();
    let mut supported: Vec<ProxyNode> = Vec::new();
    for node in nodes {
        // All meow node-level exclusions live in CoreKind::supports_node:
        // unsupported protocols, REALITY (strict servers black-hole meow's
        // fingerprint-less ClientHello), vmess on non-tcp/ws transports,
        // ss + shadow-tls. Keep the human-readable reason per class here.
        if !CoreKind::Meow.supports_node(node) {
            let reason = if !CoreKind::Meow.supports(node.protocol) {
                format!("meow 不支持 {} 协议", node.protocol.as_str())
            } else if node
                .tls
                .as_ref()
                .is_some_and(|t| t.reality_public_key.is_some() || t.reality_short_id.is_some())
            {
                "meow 的 REALITY 握手不兼容（忽略 client-fingerprint，严格服务端会黑洞）"
                    .to_string()
            } else if node.protocol == Protocol::Vmess
                && !matches!(
                    node.transport,
                    None | Some(Transport::Tcp) | Some(Transport::Ws { .. })
                )
            {
                "meow 的 vmess 仅支持 tcp/ws 传输".to_string()
            } else {
                "meow 不支持 shadow-tls 插件".to_string()
            };
            skipped.push(format!("{}: {reason}", node.name));
            continue;
        }
        supported.push(node.clone());
    }
    for reason in &skipped {
        crate::app_log::warn("meow_config", format!("跳过节点 — {reason}"));
    }
    if supported.is_empty() {
        return Err(AppError::Config(
            "no meow-compatible nodes (supports ss/vmess/vless(non-reality)/trojan/hysteria2/anytls/snell/socks5/http)"
                .into(),
        ));
    }

    let tags: Vec<String> = supported.iter().map(outbound_tag).collect();
    let selected_tag = resolve_selected_tag(&supported, &tags, opts.current_node_id.as_deref());

    // —— proxy-groups ——
    // Kernel auto-select: the main group IS a url-test group over all nodes
    // (sing-box's exact shape — `proxy_group_now_with_timeout("proxy")`
    // reads its `now` to sync the dashboard's current node; PUT /proxies on
    // a url-test 400s, so manual picks take the restart path like sing-box
    // kernel mode). Otherwise a select group ordered with the current node
    // first so meow's initial `now` matches our persisted selection.
    let mut groups: Vec<Mapping> = Vec::new();
    let probe_url = probe_url_or_default(opts);
    if opts.auto_select.is_kernel() {
        groups.push(url_test_group(MAIN_GROUP, tags.clone(), &probe_url));
    } else {
        groups.push(select_group(MAIN_GROUP, tags.clone(), Some(&selected_tag)));
    }

    // Filter-strategy sets: whole set routes through a keyword-filtered
    // url-test pool (same semantics as the Xray filter balancer).
    let effective_rules = effective_route_rules(&opts.rule_sets, &opts.rules);
    let mut filter_group_tags: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for set in opts
        .rule_sets
        .iter()
        .filter(|s| s.enabled && s.remote.is_none() && s.strategy == RuleSetStrategy::Filter)
    {
        if rule_set_is_empty_for_config(set) {
            continue;
        }
        let pool = filter_pool_tags(&set.smart_include, &set.smart_exclude, &supported, &tags);
        if pool.is_empty() {
            continue;
        }
        // Same shape as sing-box's filter-set selectors (`smart-<set id>`
        // via RuleSet::smart_set_outbound_tag): the app-side smart switch
        // maintains these pools by PUT (a stand-in rule per Filter set),
        // so they must be select groups under that exact tag.
        let group_tag = set.smart_set_outbound_tag();
        groups.push(select_group(
            &group_tag,
            pool.clone(),
            pool.first().map(String::as_str),
        ));
        filter_group_tags.insert(set.id.clone(), group_tag);
    }

    // Per-RULE smart pools (target=Smart with keywords): select group per
    // rule, like sing-box's smart selectors — the app-side smart switch
    // PUTs its evaluated pick into them (`select_group_live_serialized`),
    // and a url-test group would 400 every PUT (kernel racing and app-side
    // evaluation would also fight). Default = the pool's best-latency
    // member (smart_pool_tags orders by historical latency).
    let mut smart_group_tags: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for rule in effective_rules
        .iter()
        .filter(|r| r.enabled && r.target == RuleTarget::Smart && !r.payload.trim().is_empty())
    {
        let pool = smart_pool_tags(rule, &supported, &tags);
        if pool.is_empty() {
            continue;
        }
        let group_tag = rule.smart_outbound_tag();
        groups.push(select_group(
            &group_tag,
            pool.clone(),
            pool.first().map(String::as_str),
        ));
        smart_group_tags.insert(rule.id.clone(), group_tag);
    }

    // —— rules ——
    let rules = build_rules(
        opts,
        &supported,
        &tags,
        &effective_rules,
        &filter_group_tags,
        &smart_group_tags,
    );

    // —— document ——
    let mut root = Mapping::new();
    root.insert(str_yaml("mixed-port"), num_yaml(opts.mixed_port.into()));
    root.insert(str_yaml("allow-lan"), Yaml::Bool(opts.allow_lan));
    if opts.allow_lan {
        root.insert(str_yaml("bind-address"), str_yaml("*"));
    }
    // Mode stays `rule` even for Global/Direct outbound modes: mihomo's
    // global mode routes through the GLOBAL group (whose selection we do not
    // manage); our Global semantic is "everything via the main group", which
    // the MATCH final expresses directly.
    root.insert(str_yaml("mode"), str_yaml("rule"));
    root.insert(str_yaml("log-level"), str_yaml("info"));
    root.insert(
        str_yaml("external-controller"),
        str_yaml(&format!("127.0.0.1:{}", opts.api_port)),
    );
    root.insert(str_yaml("secret"), str_yaml(&opts.api_secret));
    if opts.tun_enabled {
        root.insert(str_yaml("tun"), Yaml::Mapping(tun_block()));
    }
    root.insert(
        str_yaml("dns"),
        Yaml::Mapping(build_dns(opts, &opts.rule_sets, &effective_rules)),
    );
    if !opts.extra_inbounds.is_empty() {
        root.insert(str_yaml("listeners"), listeners_block(opts));
    }
    root.insert(
        str_yaml("proxies"),
        Yaml::Sequence(
            supported
                .iter()
                .map(|n| Yaml::Mapping(node_to_meow_proxy(n)))
                .collect(),
        ),
    );
    root.insert(
        str_yaml("proxy-groups"),
        Yaml::Sequence(groups.into_iter().map(Yaml::Mapping).collect()),
    );
    root.insert(
        str_yaml("rules"),
        Yaml::Sequence(rules.into_iter().map(Yaml::String).collect()),
    );

    let yaml = serde_yaml::to_string(&root)
        .map_err(|e| AppError::Config(format!("serialize meow config: {e}")))?;
    Ok(BuiltMeowConfig {
        yaml,
        outbound_tags: tags,
        selected_tag,
    })
}

fn str_yaml(s: &str) -> Yaml {
    Yaml::String(s.to_string())
}

/// DNS list value (mihomo queries the entries concurrently, fastest wins).
fn dns_seq(entries: &[String]) -> Yaml {
    Yaml::Sequence(entries.iter().map(|s| Yaml::String(s.clone())).collect())
}

fn num_yaml(n: u64) -> Yaml {
    Yaml::Number(n.into())
}

fn probe_url_or_default(opts: &BuildOptions) -> String {
    let url = opts.probe_url.trim();
    if url.is_empty() {
        "https://www.gstatic.com/generate_204".to_string()
    } else {
        url.to_string()
    }
}

fn select_group(name: &str, mut members: Vec<String>, default_first: Option<&str>) -> Mapping {
    if let Some(first) = default_first {
        members.retain(|m| m != first);
        members.insert(0, first.to_string());
    }
    let mut g = Mapping::new();
    g.insert(str_yaml("name"), str_yaml(name));
    g.insert(str_yaml("type"), str_yaml("select"));
    g.insert(
        str_yaml("proxies"),
        Yaml::Sequence(members.into_iter().map(Yaml::String).collect()),
    );
    g
}

fn url_test_group(name: &str, members: Vec<String>, probe_url: &str) -> Mapping {
    let mut g = Mapping::new();
    g.insert(str_yaml("name"), str_yaml(name));
    g.insert(str_yaml("type"), str_yaml("url-test"));
    g.insert(str_yaml("url"), str_yaml(probe_url));
    g.insert(str_yaml("interval"), num_yaml(PROBE_INTERVAL_SECS));
    g.insert(str_yaml("tolerance"), num_yaml(PROBE_TOLERANCE_MS.into()));
    g.insert(
        str_yaml("proxies"),
        Yaml::Sequence(members.into_iter().map(Yaml::String).collect()),
    );
    g
}

/// meow tun block. meow requires fake-ip DNS for tun (auto-route has nothing
/// safe to route otherwise) — `build_dns` forces fake-ip whenever tun is on.
/// mihomo fields like `stack`/`strict-route` are accepted-but-ignored by meow
/// and intentionally not emitted.
fn tun_block() -> Mapping {
    let mut t = Mapping::new();
    t.insert(str_yaml("enable"), Yaml::Bool(true));
    t.insert(str_yaml("auto-route"), Yaml::Bool(true));
    t.insert(
        str_yaml("dns-hijack"),
        Yaml::Sequence(vec![str_yaml("any:53")]),
    );
    t
}

/// Additional mixed listeners (mihomo `listeners`, spike-validated).
fn listeners_block(opts: &BuildOptions) -> Yaml {
    Yaml::Sequence(
        opts.extra_inbounds
            .iter()
            .map(|inb| {
                let mut l = Mapping::new();
                l.insert(
                    str_yaml("name"),
                    str_yaml(&format!("in-mixed-{}", inb.port)),
                );
                l.insert(str_yaml("type"), str_yaml("mixed"));
                l.insert(
                    str_yaml("listen"),
                    str_yaml(if inb.allow_lan {
                        "0.0.0.0"
                    } else {
                        "127.0.0.1"
                    }),
                );
                l.insert(str_yaml("port"), num_yaml(inb.port.into()));
                Yaml::Mapping(l)
            })
            .collect(),
    )
}

// —— rules ——

/// Build the ordered Clash rule list (strings "TYPE,payload,target").
#[allow(clippy::too_many_arguments)]
fn build_rules(
    opts: &BuildOptions,
    nodes: &[ProxyNode],
    tags: &[String],
    effective_rules: &[Rule],
    filter_group_tags: &std::collections::HashMap<String, String>,
    smart_group_tags: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut rules = Vec::new();
    // Rule mode compiles user rule sets; Global/Direct ignore them (the
    // MATCH final decides — mirroring the other two generators).
    if opts.outbound_mode == OutboundMode::Rule {
        if opts.rule_sets.is_empty() {
            for rule in effective_rules {
                if let Some(s) = rule_to_meow(rule, nodes, tags, MAIN_GROUP, smart_group_tags) {
                    rules.push(s);
                }
            }
        } else {
            for set in opts.rule_sets.iter().filter(|s| s.enabled) {
                if set.remote.is_some() {
                    match builtin_remote_meow_rule(set, MAIN_GROUP) {
                        Some(s) => rules.push(s),
                        None => crate::app_log::warn(
                            "meow_config",
                            format!(
                                "remote rule set '{}' uses the sing-box .srs format and is skipped under meow",
                                set.name
                            ),
                        ),
                    }
                    continue;
                }
                // Local set: per rule, with the same whole-set clamping the
                // sing-box path applies (Node pin / Filter pool / plain).
                let mut sorted: Vec<Rule> = set
                    .rules
                    .iter()
                    .filter(|r| r.enabled && !r.payload.trim().is_empty())
                    .cloned()
                    .collect();
                sorted.sort_by_key(|r| r.ord);
                let filter_group = if set.strategy == RuleSetStrategy::Filter {
                    filter_group_tags.get(&set.id).map(String::as_str)
                } else {
                    None
                };
                for mut rule in sorted {
                    if let Some(target) = set.strategy.route_target() {
                        rule.target = target;
                        rule.node_id = None;
                        rule.node_name = None;
                        rule.smart_include.clear();
                        rule.smart_exclude.clear();
                    } else {
                        clamp_rule_pin_to_set(set, &mut rule);
                    }
                    if let Some(s) = rule_to_meow(&rule, nodes, tags, MAIN_GROUP, smart_group_tags)
                    {
                        rules.push(match filter_group {
                            Some(group) => retarget_rule(&s, group),
                            None => s,
                        });
                    }
                }
            }
        }
        if opts.bypass_lan {
            rules.extend(bypass_lan_rules());
        }
    }
    if opts.block_quic {
        // Logic rule (spike-validated): reject UDP/443 so browsers fall back
        // to TCP — the same QUIC-blocking effect as the other two cores.
        rules.push("AND,((NETWORK,udp),(DST-PORT,443)),REJECT".to_string());
    }
    let final_target = match opts.outbound_mode {
        OutboundMode::Rule => match opts.normalized_route_final() {
            "direct" => "DIRECT".to_string(),
            "block" => "REJECT".to_string(),
            _ => MAIN_GROUP.to_string(),
        },
        OutboundMode::Global => MAIN_GROUP.to_string(),
        OutboundMode::Direct => "DIRECT".to_string(),
    };
    rules.push(format!("MATCH,{final_target}"));
    rules
}

/// Swap the target of a rendered "TYPE,payload,target" rule string.
fn retarget_rule(rule: &str, target: &str) -> String {
    let mut parts: Vec<&str> = rule.split(',').collect();
    if parts.len() >= 3 {
        let last = parts.len() - 1;
        parts[last] = target;
        parts.join(",")
    } else {
        rule.to_string()
    }
}

/// Map one rule to a Clash rule string. `main_target` substitutes for the
/// sing-box `proxy` selector group.
fn rule_to_meow(
    rule: &Rule,
    nodes: &[ProxyNode],
    tags: &[String],
    main_target: &str,
    smart_group_tags: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let payload = rule.payload.trim();
    if payload.is_empty() || matches!(rule.rule_type, RuleType::Geoip) {
        return None;
    }
    let matcher = match rule.rule_type {
        RuleType::Domain => format!("DOMAIN,{}", to_ascii_domain(payload)),
        RuleType::DomainSuffix => format!("DOMAIN-SUFFIX,{}", to_ascii_domain(payload)),
        RuleType::DomainKeyword => format!("DOMAIN-KEYWORD,{payload}"),
        RuleType::IpCidr => format!("IP-CIDR,{payload}"),
        RuleType::Process => format!("PROCESS-NAME,{payload}"),
        RuleType::Geoip => return None,
    };
    let Rule {
        target, node_id, ..
    } = rule;
    // Node pins point straight at the node proxy; Smart rules with a
    // non-empty keyword pool route through their per-rule url-test group;
    // empty pools (and plain Proxy) fall back to the main group.
    let target = match target {
        RuleTarget::Direct => "DIRECT".to_string(),
        RuleTarget::Block => "REJECT".to_string(),
        RuleTarget::Proxy => main_target.to_string(),
        RuleTarget::Smart => smart_group_tags
            .get(&rule.id)
            .cloned()
            .unwrap_or_else(|| main_target.to_string()),
        RuleTarget::Node => node_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|id| nodes.iter().find(|n| n.id == id))
            .filter(|n| tags.iter().any(|t| t == &outbound_tag(n)))
            .map(outbound_tag)
            .unwrap_or_else(|| main_target.to_string()),
    };
    Some(format!("{matcher},{target}"))
}

/// Whole-set rule for a builtin remote set expressed via geodata matchers.
fn builtin_remote_meow_rule(set: &RuleSet, main_target: &str) -> Option<String> {
    let matcher = match set.id.as_str() {
        "system-geosite-cn" => "GEOSITE,cn",
        "system-geoip-cn" => "GEOIP,cn",
        "system-geolocation-not-cn" => "GEOSITE,geolocation-!cn",
        _ => return None,
    };
    let target = match set.strategy {
        RuleSetStrategy::Direct => "DIRECT",
        RuleSetStrategy::Block => "REJECT",
        _ => main_target,
    };
    Some(format!("{matcher},{target}"))
}

/// Explicit private-range bypass (GEOIP,private is a v2ray/sing-box geodata
/// category that MaxMind mmdb — meow's geoip source — does not carry).
/// 198.18.0.0/15 (fake-ip) is intentionally absent: meow terminates fake-ip
/// internally before rule matching.
fn bypass_lan_rules() -> Vec<String> {
    const V4: [&str; 9] = [
        "0.0.0.0/8",
        "10.0.0.0/8",
        "100.64.0.0/10",
        "127.0.0.0/8",
        "169.254.0.0/16",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "224.0.0.0/4",
        "255.255.255.255/32",
    ];
    const V6: [&str; 4] = ["::1/128", "fc00::/7", "fe80::/10", "ff00::/8"];
    let mut rules = vec![
        "DOMAIN,localhost,DIRECT".to_string(),
        "DOMAIN-SUFFIX,local,DIRECT".to_string(),
    ];
    for cidr in V4 {
        rules.push(format!("IP-CIDR,{cidr},DIRECT,no-resolve"));
    }
    for cidr in V6 {
        rules.push(format!("IP-CIDR6,{cidr},DIRECT,no-resolve"));
    }
    rules
}

// —— DNS ——

/// Map the shared DnsSettings onto mihomo's dns block:
/// `nameserver` (default resolver by dns_final), `fallback` (the other
/// side), `nameserver-policy` (per-domain classification from DNS rules and
/// rule-set dns strategies), `hosts`, and fake-ip when enabled — forced when
/// tun is on (meow requires it; the user's stored DNS settings are not
/// written back, so turning tun off restores them).
fn build_dns(opts: &BuildOptions, sets: &[RuleSet], effective_rules: &[Rule]) -> Mapping {
    let mut policy_remote: Vec<String> = Vec::new();
    let mut policy_domestic: Vec<String> = Vec::new();
    let mut policy_local: Vec<String> = Vec::new();

    for rule in opts
        .dns
        .enabled_dns_rules()
        .into_iter()
        .filter(|r| r.enabled && !matches!(r.action, DnsAction::Block))
    {
        let Some(pat) = dns_pattern(rule.matcher, &rule.payload) else {
            continue;
        };
        match rule.action {
            DnsAction::Remote => policy_remote.push(pat),
            DnsAction::Domestic => policy_domestic.push(pat),
            _ => policy_local.push(pat),
        }
    }

    // Rule-set level DNS strategy (local sets classify their domain rules;
    // builtin remote sets map onto the equivalent geosite category).
    let effective_ids: std::collections::HashSet<&str> =
        effective_rules.iter().map(|r| r.id.as_str()).collect();
    for set in sets.iter().filter(|s| s.enabled) {
        let mut classify = |pat: String, strategy: RuleSetDnsStrategy| match strategy {
            RuleSetDnsStrategy::Domestic => policy_domestic.push(pat),
            RuleSetDnsStrategy::Local => policy_local.push(pat),
            RuleSetDnsStrategy::Remote => policy_remote.push(pat),
        };
        if set.remote.is_some() {
            let geosite = match set.id.as_str() {
                "system-geosite-cn" => Some("geosite:cn"),
                "system-geolocation-not-cn" => Some("geosite:geolocation-!cn"),
                _ => None, // ip-only set: no DNS classification
            };
            if let Some(geosite) = geosite {
                classify(geosite.to_string(), set.dns_strategy);
            }
            continue;
        }
        for rule in set.rules.iter().filter(|r| r.enabled) {
            if set.strategy != RuleSetStrategy::Filter && !effective_ids.contains(rule.id.as_str())
            {
                continue;
            }
            let payload = rule.payload.trim();
            if payload.is_empty() {
                continue;
            }
            let pat = match rule.rule_type {
                RuleType::Domain => to_ascii_domain(payload),
                RuleType::DomainSuffix => format!("+.{}", to_ascii_domain(payload)),
                // nameserver-policy has no keyword form — the domain still
                // routes correctly; only its resolver choice falls back.
                _ => continue,
            };
            classify(pat, set.dns_strategy);
        }
    }

    // Static hosts (highest-priority answers).
    let mut hosts = Mapping::new();
    let effective_hosts = opts.dns.effective_hosts();
    if effective_hosts.enabled {
        for entry in &effective_hosts.entries {
            if entry.enabled && !entry.domain.trim().is_empty() && !entry.addr.trim().is_empty() {
                hosts.insert(
                    str_yaml(&to_ascii_domain(entry.domain.trim())),
                    str_yaml(entry.addr.trim()),
                );
            }
        }
    }

    let use_fakeip = opts.tun_enabled || opts.dns.fake_ip.enabled;
    let dns_final = opts.dns.normalize_dns_final();
    // Remote DoH egress goes through the main proxy group (meow's `#adapter`
    // fragment on DNS entries) — mirroring sing-box's remote-resolver detour
    // and Xray's dns-module routing. Direct egress hits the classic
    // chicken-and-egg: the DoH endpoints are unreachable without a proxy,
    // and the proxy's node hostnames resolve via proxy-server-nameserver
    // (plain UDP, always direct), which breaks the loop. Direct outbound
    // mode is the one exception — everything egresses direct there, DNS
    // included.
    let remote_pool: Vec<String> = if opts.outbound_mode == OutboundMode::Direct {
        REMOTE_DNS_POOL.iter().map(|s| (*s).to_string()).collect()
    } else {
        REMOTE_DNS_POOL
            .iter()
            .map(|s| format!("{s}#{MAIN_GROUP}"))
            .collect()
    };
    let domestic_pool: Vec<String> = DOMESTIC_DNS_POOL.iter().map(|s| (*s).to_string()).collect();
    // NOTE: meow has NO `system` resolver (unlike mihomo — unknown schemes
    // like `system` hard-error at load: "nameserver-policy entry has no
    // valid nameservers"). The "use the system resolver" intent is
    // approximated with the plain-UDP domestic pool.
    let (default_ns, fallback_ns): (&[String], &[String]) = match dns_final {
        "domestic" | "local" => (&domestic_pool, &remote_pool),
        _ => (&remote_pool, &domestic_pool),
    };

    let mut dns = Mapping::new();
    dns.insert(str_yaml("enable"), Yaml::Bool(true));
    if !hosts.is_empty() {
        dns.insert(str_yaml("hosts"), Yaml::Mapping(hosts));
    }
    // Bootstrap resolver for DoH hostnames (plain UDP, always direct).
    dns.insert(str_yaml("default-nameserver"), dns_seq(&domestic_pool));
    // Node server hostnames resolve through a dedicated plain-UDP list —
    // never the DoH default or the policy classification. Without this,
    // `dns_final=remote` makes url-test health checks dial blocked DoH
    // endpoints first, which need a working proxy to reach, which needs
    // its hostname resolved first (chicken-and-egg → "meow-dns resolver:
    // no address for <node-host>" WARNs).
    dns.insert(str_yaml("proxy-server-nameserver"), dns_seq(&domestic_pool));
    dns.insert(str_yaml("nameserver"), dns_seq(default_ns));
    dns.insert(str_yaml("fallback"), dns_seq(fallback_ns));
    if !policy_remote.is_empty() || !policy_domestic.is_empty() || !policy_local.is_empty() {
        let mut policy = Mapping::new();
        for pat in policy_domestic {
            policy.insert(str_yaml(&pat), dns_seq(&domestic_pool));
        }
        // Remote-classified domains resolve via the proxy-egress DoH pool.
        for pat in policy_remote {
            policy.insert(str_yaml(&pat), dns_seq(&remote_pool));
        }
        // "local" classification also lands on the plain-UDP domestic
        // pool — meow rejects `system` (see the match above).
        for pat in policy_local {
            policy.insert(str_yaml(&pat), dns_seq(&domestic_pool));
        }
        dns.insert(str_yaml("nameserver-policy"), Yaml::Mapping(policy));
    }
    if use_fakeip {
        dns.insert(str_yaml("enhanced-mode"), str_yaml("fake-ip"));
        dns.insert(
            str_yaml("fake-ip-range"),
            str_yaml(&opts.dns.fake_ip.inet4_range),
        );
        let filter: Vec<Yaml> = opts
            .dns
            .fake_ip
            .bypass
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str_yaml)
            .collect();
        if !filter.is_empty() {
            dns.insert(str_yaml("fake-ip-filter"), Yaml::Sequence(filter));
        }
    }
    dns
}

/// nameserver-policy key for one of our DNS matchers (exact / +.suffix;
/// keywords have no policy form).
fn dns_pattern(matcher: DomainMatcher, payload: &str) -> Option<String> {
    let payload = payload.trim();
    if payload.is_empty() {
        return None;
    }
    match matcher {
        DomainMatcher::Domain => Some(to_ascii_domain(payload)),
        DomainMatcher::DomainSuffix => Some(format!("+.{}", to_ascii_domain(payload))),
        DomainMatcher::DomainKeyword => None,
    }
}

// —— proxies ——

/// Map one node to a Clash proxy mapping (field names mirror what our own
/// `subscription::clash` parser reads — the authoritative inverse).
fn node_to_meow_proxy(node: &ProxyNode) -> Mapping {
    let mut m = Mapping::new();
    m.insert(str_yaml("name"), str_yaml(&outbound_tag(node)));
    m.insert(
        str_yaml("type"),
        str_yaml(match node.protocol {
            Protocol::Shadowsocks => "ss",
            Protocol::Vmess => "vmess",
            Protocol::Vless => "vless",
            Protocol::Trojan => "trojan",
            Protocol::Hysteria2 => "hysteria2",
            Protocol::Socks5 => "socks5",
            Protocol::Http => "http",
            Protocol::AnyTls => "anytls",
            Protocol::Snell => "snell",
            _ => unreachable!("filtered by CoreKind::Meow::supports upstream"),
        }),
    );
    m.insert(str_yaml("server"), str_yaml(&node.server));
    m.insert(str_yaml("port"), num_yaml(node.port.into()));
    if node.udp == Some(true) {
        m.insert(str_yaml("udp"), Yaml::Bool(true));
    }

    match &node.config {
        ProtocolConfig::Shadowsocks {
            method,
            password,
            plugin,
            plugin_opts,
            ..
        } => {
            m.insert(str_yaml("cipher"), str_yaml(method));
            m.insert(str_yaml("password"), str_yaml(password));
            apply_ss_plugin(&mut m, plugin.as_deref(), plugin_opts.as_deref());
        }
        ProtocolConfig::Vmess {
            uuid,
            alter_id,
            security,
        } => {
            m.insert(str_yaml("uuid"), str_yaml(uuid));
            m.insert(str_yaml("alterId"), num_yaml((*alter_id).into()));
            m.insert(str_yaml("cipher"), str_yaml(security));
        }
        ProtocolConfig::Vless { uuid, flow, .. } => {
            m.insert(str_yaml("uuid"), str_yaml(uuid));
            if let Some(flow) = flow.as_deref().filter(|f| !f.trim().is_empty()) {
                m.insert(str_yaml("flow"), str_yaml(flow));
            }
        }
        ProtocolConfig::Trojan { password } => {
            m.insert(str_yaml("password"), str_yaml(password));
        }
        ProtocolConfig::Hysteria2 {
            password,
            obfs,
            obfs_password,
            ..
        } => {
            m.insert(str_yaml("password"), str_yaml(password));
            if let Some(obfs) = obfs.as_deref().filter(|s| !s.trim().is_empty()) {
                m.insert(str_yaml("obfs"), str_yaml(obfs));
                if let Some(pw) = obfs_password.as_deref().filter(|s| !s.trim().is_empty()) {
                    m.insert(str_yaml("obfs-password"), str_yaml(pw));
                }
            }
        }
        ProtocolConfig::Socks5 { username, password } => {
            if let Some(u) = username.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("username"), str_yaml(u));
            }
            if let Some(p) = password.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("password"), str_yaml(p));
            }
        }
        ProtocolConfig::Http {
            username, password, ..
        } => {
            if let Some(u) = username.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("username"), str_yaml(u));
            }
            if let Some(p) = password.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("password"), str_yaml(p));
            }
        }
        ProtocolConfig::AnyTls { password } => {
            m.insert(str_yaml("password"), str_yaml(password));
        }
        ProtocolConfig::Snell {
            psk,
            version,
            userkey,
            obfs_mode,
            obfs_host,
            ..
        } => {
            m.insert(str_yaml("psk"), str_yaml(psk));
            m.insert(str_yaml("version"), str_yaml(&version.to_string()));
            if let Some(key) = userkey.as_deref().filter(|s| !s.trim().is_empty()) {
                m.insert(str_yaml("userkey"), str_yaml(key));
            }
            if let Some(mode) = obfs_mode.as_deref().filter(|s| !s.trim().is_empty()) {
                let mut opts = Mapping::new();
                opts.insert(str_yaml("mode"), str_yaml(mode));
                if let Some(host) = obfs_host.as_deref().filter(|s| !s.trim().is_empty()) {
                    opts.insert(str_yaml("host"), str_yaml(host));
                }
                m.insert(str_yaml("obfs-opts"), Yaml::Mapping(opts));
            }
        }
        // Unsupported protocols were filtered before mapping; nothing to add.
        _ => {}
    }

    apply_tls(&mut m, node);
    apply_transport(&mut m, node);
    m
}

/// SIP003 plugin string → mihomo plugin/plugin-opts (simple-obfs /
/// v2ray-plugin; shadow-tls nodes were skipped upstream).
fn apply_ss_plugin(m: &mut Mapping, plugin: Option<&str>, opts: Option<&str>) {
    let Some(plugin) = plugin.filter(|p| !p.trim().is_empty()) else {
        return;
    };
    let plugin = plugin.trim();
    let is_v2ray = plugin.contains("v2ray-plugin");
    let name = if is_v2ray { "v2ray-plugin" } else { "obfs" };
    m.insert(str_yaml("plugin"), str_yaml(name));
    // Parse the SIP003 k=v;k=v string back into mihomo's typed opts.
    let mut mode = None;
    let mut host = None;
    let mut path = None;
    let mut tls = false;
    if let Some(opts) = opts {
        for pair in opts.split(';') {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or("").trim();
            let value = kv.next().unwrap_or("").trim();
            match key {
                "obfs" | "mode" => mode = Some(value.to_string()),
                "obfs-host" | "host" => host = Some(value.to_string()),
                "path" => path = Some(value.to_string()),
                "tls" => tls = value.eq_ignore_ascii_case("true") || value == "1",
                _ => {}
            }
        }
    }
    if mode.is_none() && is_v2ray {
        mode = Some("websocket".into());
    }
    if mode.is_some() || host.is_some() || path.is_some() || tls {
        let mut o = Mapping::new();
        if let Some(mode) = mode.filter(|s| !s.is_empty()) {
            o.insert(str_yaml("mode"), str_yaml(&mode));
        }
        if let Some(host) = host.filter(|s| !s.is_empty()) {
            o.insert(str_yaml("host"), str_yaml(&host));
        }
        if let Some(path) = path.filter(|s| !s.is_empty()) {
            o.insert(str_yaml("path"), str_yaml(&path));
        }
        if tls {
            o.insert(str_yaml("tls"), Yaml::Bool(true));
        }
        m.insert(str_yaml("plugin-opts"), Yaml::Mapping(o));
    }
}

/// TLS block: `tls`/`servername` for vmess/vless, `sni` for the rest, plus
/// alpn / skip-cert-verify / client-fingerprint / reality-opts.
fn apply_tls(m: &mut Mapping, node: &ProxyNode) {
    let Some(tls) = &node.tls else { return };
    let sni_field = match node.protocol {
        Protocol::Vmess | Protocol::Vless => "servername",
        _ => "sni",
    };
    let has_reality = tls.reality_public_key.is_some() || tls.reality_short_id.is_some();
    if has_reality {
        let mut reality = Mapping::new();
        if let Some(pk) = tls.reality_public_key.as_deref().filter(|s| !s.is_empty()) {
            reality.insert(str_yaml("public-key"), str_yaml(pk));
        }
        if let Some(sid) = tls.reality_short_id.as_deref().filter(|s| !s.is_empty()) {
            reality.insert(str_yaml("short-id"), str_yaml(sid));
        }
        m.insert(str_yaml("reality-opts"), Yaml::Mapping(reality));
    }
    if tls.enabled || has_reality {
        m.insert(str_yaml("tls"), Yaml::Bool(true));
        if let Some(sn) = tls.server_name.as_deref().filter(|s| !s.is_empty()) {
            m.insert(str_yaml(sni_field), str_yaml(sn));
        }
    }
    if tls.insecure == Some(true) {
        m.insert(str_yaml("skip-cert-verify"), Yaml::Bool(true));
    }
    if let Some(alpn) = tls.alpn.as_ref().filter(|a| !a.is_empty()) {
        m.insert(
            str_yaml("alpn"),
            Yaml::Sequence(alpn.iter().map(|a| str_yaml(a)).collect()),
        );
    }
    if let Some(fp) = tls.utls_fingerprint.as_deref().filter(|s| !s.is_empty()) {
        m.insert(str_yaml("client-fingerprint"), str_yaml(fp));
    }
}

/// Transport: network + typed opts (ws / grpc / h2 / httpupgrade).
fn apply_transport(m: &mut Mapping, node: &ProxyNode) {
    let Some(transport) = &node.transport else {
        return;
    };
    match transport {
        Transport::Tcp => {}
        Transport::Ws {
            path,
            headers,
            max_early_data,
        } => {
            m.insert(str_yaml("network"), str_yaml("ws"));
            let mut o = Mapping::new();
            if let Some(path) = path.as_deref().filter(|s| !s.is_empty()) {
                o.insert(str_yaml("path"), str_yaml(path));
            }
            if let Some(headers) = headers {
                let mut h = Mapping::new();
                for (key, value) in headers {
                    h.insert(str_yaml(key), str_yaml(value));
                }
                if !h.is_empty() {
                    o.insert(str_yaml("headers"), Yaml::Mapping(h));
                }
            }
            if let Some(n) = max_early_data {
                o.insert(str_yaml("max-early-data"), num_yaml((*n).into()));
            }
            if !o.is_empty() {
                m.insert(str_yaml("ws-opts"), Yaml::Mapping(o));
            }
        }
        Transport::Grpc { service_name } => {
            m.insert(str_yaml("network"), str_yaml("grpc"));
            if let Some(name) = service_name.as_deref().filter(|s| !s.is_empty()) {
                let mut o = Mapping::new();
                o.insert(str_yaml("grpc-service-name"), str_yaml(name));
                m.insert(str_yaml("grpc-opts"), Yaml::Mapping(o));
            }
        }
        Transport::Http { path, host } => {
            m.insert(str_yaml("network"), str_yaml("h2"));
            let mut o = Mapping::new();
            if let Some(path) = path.as_deref().filter(|s| !s.is_empty()) {
                o.insert(str_yaml("path"), Yaml::Sequence(vec![str_yaml(path)]));
            }
            if let Some(host) = host.as_ref().filter(|h| !h.is_empty()) {
                o.insert(
                    str_yaml("host"),
                    Yaml::Sequence(host.iter().map(|s| str_yaml(s)).collect()),
                );
            }
            if !o.is_empty() {
                m.insert(str_yaml("h2-opts"), Yaml::Mapping(o));
            }
        }
        Transport::HttpUpgrade { path, host } => {
            m.insert(str_yaml("network"), str_yaml("httpupgrade"));
            let mut o = Mapping::new();
            if let Some(path) = path.as_deref().filter(|s| !s.is_empty()) {
                o.insert(str_yaml("path"), str_yaml(path));
            }
            if let Some(host) = host.as_deref().filter(|s| !s.is_empty()) {
                o.insert(str_yaml("host"), str_yaml(host));
            }
            if !o.is_empty() {
                m.insert(str_yaml("httpupgrade-opts"), Yaml::Mapping(o));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AutoSelectMode, DnsSettings, TlsConfig};

    fn vless_node(name: &str, flow: Option<&str>) -> ProxyNode {
        ProxyNode {
            id: String::new(),
            name: name.into(),
            protocol: Protocol::Vless,
            server: "example.com".into(),
            port: 443,
            tls: Some(TlsConfig {
                enabled: true,
                server_name: Some("sni.example.com".into()),
                insecure: None,
                alpn: None,
                utls_fingerprint: Some("chrome".into()),
                reality_public_key: Some("pbk".into()),
                reality_short_id: Some("abcd0123".into()),
            }),
            transport: Some(Transport::Tcp),
            udp: None,
            config: ProtocolConfig::Vless {
                uuid: "uuid-1".into(),
                flow: flow.map(str::to_string),
                packet_encoding: "xudp".into(),
            },
            source: None,
            latency_ms: None,
            latency_at: None,
        }
        .with_computed_id()
    }

    /** Non-REALITY vless node — the shape meow accepts (vless_node carries
     * REALITY, which meow filters out since the handshake incompatibility). */
    fn plain_node(name: &str) -> ProxyNode {
        let mut n = vless_node(name, None);
        n.tls = Some(TlsConfig {
            enabled: true,
            server_name: Some("sni.example.com".into()),
            ..Default::default()
        });
        n
    }

    fn ss_node(name: &str) -> ProxyNode {
        ProxyNode {
            id: String::new(),
            name: name.into(),
            protocol: Protocol::Shadowsocks,
            server: "example.com".into(),
            port: 8388,
            tls: None,
            transport: None,
            udp: Some(true),
            config: ProtocolConfig::Shadowsocks {
                method: "aes-256-gcm".into(),
                password: "pw".into(),
                plugin: None,
                plugin_opts: None,
                shadow_tls: None,
            },
            source: None,
            latency_ms: None,
            latency_at: None,
        }
        .with_computed_id()
    }

    fn default_opts() -> BuildOptions {
        BuildOptions {
            mixed_port: 2080,
            allow_lan: false,
            api_port: 19090,
            extra_inbounds: Vec::new(),
            api_secret: String::new(),
            current_node_id: None,
            log_level: "info".into(),
            rules: Vec::new(),
            rule_sets: Vec::new(),
            tun_enabled: false,
            tun_stack: "mixed".into(),
            dns: DnsSettings::default(),
            outbound_mode: OutboundMode::Rule,
            route_final: "proxy".into(),
            auto_select: AutoSelectMode::Off,
            probe_url: String::new(),
            find_process: true,
            tun_ipv6: false,
            block_quic: false,
            bypass_lan: true,
        }
    }

    fn parse(built: &BuiltMeowConfig) -> serde_yaml::Value {
        serde_yaml::from_str(&built.yaml).expect("generated yaml parses")
    }

    fn rules_of(doc: &serde_yaml::Value) -> Vec<String> {
        doc["rules"]
            .as_sequence()
            .expect("rules sequence")
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    fn groups_of(doc: &serde_yaml::Value) -> Vec<serde_yaml::Value> {
        doc["proxy-groups"].as_sequence().expect("groups").clone()
    }

    fn node_tag_of(node: &ProxyNode) -> String {
        outbound_tag(node)
    }

    #[test]
    fn builds_minimal_config() {
        let nodes = vec![ss_node("n1")];
        let built = build_meow_config(&nodes, &default_opts()).expect("build");
        let doc = parse(&built);
        assert_eq!(doc["mixed-port"].as_u64(), Some(2080));
        assert_eq!(doc["mode"].as_str(), Some("rule"));
        assert_eq!(doc["external-controller"].as_str(), Some("127.0.0.1:19090"));
        // Main group keeps the sing-box contract: select named "proxy", the
        // selected node listed first.
        let groups = groups_of(&doc);
        assert_eq!(groups[0]["name"].as_str(), Some("proxy"));
        assert_eq!(groups[0]["type"].as_str(), Some("select"));
        assert_eq!(
            groups[0]["proxies"][0].as_str(),
            Some(built.selected_tag.as_str())
        );
        // Proxy shape.
        assert_eq!(doc["proxies"][0]["type"].as_str(), Some("ss"));
        assert_eq!(doc["proxies"][0]["cipher"].as_str(), Some("aes-256-gcm"));
        assert_eq!(doc["proxies"][0]["udp"].as_bool(), Some(true));
        // Final rule hits the main group.
        let rules = rules_of(&doc);
        assert_eq!(rules.last().unwrap(), "MATCH,proxy");
    }

    #[test]
    fn vless_reality_flow_shape() {
        // REALITY is filtered out of meow configs (strict servers black-hole
        // meow's fingerprint-less ClientHello) — the node is skipped, and a
        // plain vless node still maps with its tls/sni fields.
        let reality = vless_node("vision", Some("xtls-rprx-vision"));
        let fallback = plain_node("plain");
        let built = build_meow_config(&[reality, fallback], &default_opts()).expect("build");
        assert_eq!(built.outbound_tags.len(), 1, "reality node must be skipped");
        let doc = parse(&built);
        assert_eq!(doc["proxies"].as_sequence().map(|p| p.len()), Some(1));
        let proxy = &doc["proxies"][0];
        assert_eq!(proxy["type"].as_str(), Some("vless"));
        assert_eq!(proxy["uuid"].as_str(), Some("uuid-1"));
        assert_eq!(proxy["tls"].as_bool(), Some(true));
        assert_eq!(proxy["servername"].as_str(), Some("sni.example.com"));
        // Only-reality nodes hard-error (same class as unsupported protocols).
        assert!(build_meow_config(&[vless_node("only-reality", None)], &default_opts()).is_err());
    }

    #[test]
    fn vmess_ws_shape() {
        let mut node = vless_node("ws", None);
        node.protocol = Protocol::Vmess;
        node.config = ProtocolConfig::Vmess {
            uuid: "uuid-2".into(),
            alter_id: 0,
            security: "auto".into(),
        };
        node.tls = Some(TlsConfig {
            enabled: true,
            server_name: Some("sni.example.com".into()),
            insecure: Some(true),
            alpn: Some(vec!["h2".into(), "http/1.1".into()]),
            utls_fingerprint: None,
            reality_public_key: None,
            reality_short_id: None,
        });
        node.transport = Some(Transport::Ws {
            path: Some("/ws".into()),
            headers: Some(
                [("Host".to_string(), "cdn.example.com".to_string())]
                    .into_iter()
                    .collect(),
            ),
            max_early_data: Some(2048),
        });
        let built = build_meow_config(&[node], &default_opts()).expect("build");
        let doc = parse(&built);
        let proxy = &doc["proxies"][0];
        assert_eq!(proxy["type"].as_str(), Some("vmess"));
        assert_eq!(proxy["alterId"].as_u64(), Some(0));
        assert_eq!(proxy["cipher"].as_str(), Some("auto"));
        assert_eq!(proxy["skip-cert-verify"].as_bool(), Some(true));
        assert_eq!(proxy["alpn"].as_sequence().map(|s| s.len()), Some(2));
        assert_eq!(proxy["network"].as_str(), Some("ws"));
        assert_eq!(proxy["ws-opts"]["path"].as_str(), Some("/ws"));
        assert_eq!(
            proxy["ws-opts"]["headers"]["Host"].as_str(),
            Some("cdn.example.com")
        );
        assert_eq!(proxy["ws-opts"]["max-early-data"].as_u64(), Some(2048));
    }

    #[test]
    fn skips_unsupported_protocols() {
        let mut tuic = vless_node("tuic-node", None);
        tuic.protocol = Protocol::Tuic;
        tuic.config = ProtocolConfig::Tuic {
            uuid: "u".into(),
            password: "p".into(),
            congestion_control: None,
            udp_relay_mode: None,
            zero_rtt_handshake: false,
        };
        // Only-unsupported → hard error.
        assert!(build_meow_config(&[tuic.clone()], &default_opts()).is_err());
        // Mixed subscription → unsupported dropped, supported kept.
        let ss = ss_node("ss-ok");
        let built = build_meow_config(&[tuic, ss.clone()], &default_opts()).expect("build");
        assert_eq!(built.outbound_tags.len(), 1);
        assert_eq!(built.outbound_tags[0], node_tag_of(&ss));
    }

    #[test]
    fn vmess_non_ws_transport_is_skipped() {
        let mut node = vless_node("vmess-grpc", None);
        node.protocol = Protocol::Vmess;
        node.config = ProtocolConfig::Vmess {
            uuid: "uuid-2".into(),
            alter_id: 0,
            security: "auto".into(),
        };
        node.transport = Some(Transport::Grpc {
            service_name: Some("svc".into()),
        });
        let fallback = ss_node("fallback");
        let built = build_meow_config(&[node, fallback.clone()], &default_opts()).expect("build");
        assert_eq!(built.outbound_tags, vec![node_tag_of(&fallback)]);
    }

    #[test]
    fn ss_shadowtls_node_is_skipped() {
        let mut node = ss_node("ss-stls");
        node.config = ProtocolConfig::Shadowsocks {
            method: "aes-128-gcm".into(),
            password: "pw".into(),
            plugin: None,
            plugin_opts: None,
            shadow_tls: Some(crate::domain::ShadowTlsOpts {
                host: "stls.example.com".into(),
                password: "pw".into(),
                version: 3,
                fingerprint: None,
            }),
        };
        assert!(build_meow_config(&[node], &default_opts()).is_err());
    }

    #[test]
    fn ss_obfs_plugin_maps_to_plugin_opts() {
        let mut node = ss_node("ss-obfs");
        node.config = ProtocolConfig::Shadowsocks {
            method: "aes-128-gcm".into(),
            password: "pw".into(),
            plugin: Some("obfs-local".into()),
            plugin_opts: Some("obfs=http;obfs-host=bing.com".into()),
            shadow_tls: None,
        };
        let built = build_meow_config(&[node], &default_opts()).expect("build");
        let doc = parse(&built);
        let proxy = &doc["proxies"][0];
        assert_eq!(proxy["plugin"].as_str(), Some("obfs"));
        assert_eq!(proxy["plugin-opts"]["mode"].as_str(), Some("http"));
        assert_eq!(proxy["plugin-opts"]["host"].as_str(), Some("bing.com"));
    }

    #[test]
    fn hysteria2_and_anytls_shapes() {
        let mut hy2 = ss_node("hy2");
        hy2.protocol = Protocol::Hysteria2;
        hy2.tls = Some(TlsConfig {
            enabled: true,
            server_name: Some("hy.example.com".into()),
            insecure: Some(true),
            alpn: None,
            utls_fingerprint: None,
            reality_public_key: None,
            reality_short_id: None,
        });
        hy2.config = ProtocolConfig::Hysteria2 {
            password: "pw".into(),
            up_mbps: None,
            down_mbps: None,
            obfs: Some("salamander".into()),
            obfs_password: Some("obfspw".into()),
        };
        let mut anytls = ss_node("at");
        anytls.protocol = Protocol::AnyTls;
        anytls.tls = Some(TlsConfig {
            enabled: true,
            server_name: Some("at.example.com".into()),
            ..Default::default()
        });
        anytls.config = ProtocolConfig::AnyTls {
            password: "pw".into(),
        };
        let built = build_meow_config(&[hy2, anytls], &default_opts()).expect("build");
        let doc = parse(&built);
        let hy2 = &doc["proxies"][0];
        assert_eq!(hy2["type"].as_str(), Some("hysteria2"));
        assert_eq!(hy2["obfs"].as_str(), Some("salamander"));
        assert_eq!(hy2["obfs-password"].as_str(), Some("obfspw"));
        assert_eq!(hy2["sni"].as_str(), Some("hy.example.com"));
        assert_eq!(hy2["skip-cert-verify"].as_bool(), Some(true));
        let at = &doc["proxies"][1];
        assert_eq!(at["type"].as_str(), Some("anytls"));
        assert_eq!(at["sni"].as_str(), Some("at.example.com"));
    }

    #[test]
    fn snell_shape() {
        let mut node = ss_node("snell");
        node.protocol = Protocol::Snell;
        node.config = ProtocolConfig::Snell {
            psk: "pskpw".into(),
            version: 4,
            userkey: None,
            reuse: None,
            obfs_mode: Some("http".into()),
            obfs_host: Some("bing.com".into()),
            mode: None,
        };
        let built = build_meow_config(&[node], &default_opts()).expect("build");
        let doc = parse(&built);
        let proxy = &doc["proxies"][0];
        assert_eq!(proxy["type"].as_str(), Some("snell"));
        assert_eq!(proxy["psk"].as_str(), Some("pskpw"));
        assert_eq!(proxy["version"].as_str(), Some("4"));
        assert_eq!(proxy["obfs-opts"]["mode"].as_str(), Some("http"));
    }

    #[test]
    fn kernel_autoselect_uses_urltest_group() {
        let a = plain_node("a");
        let b = plain_node("b");
        let mut opts = default_opts();
        opts.auto_select = AutoSelectMode::Kernel;
        let built = build_meow_config(&[a, b], &opts).expect("build");
        let doc = parse(&built);
        let groups = groups_of(&doc);
        // Main group IS the url-test over all nodes (sing-box's exact
        // shape) — its `now` feeds the kernel-selection sync.
        assert_eq!(groups[0]["name"].as_str(), Some("proxy"));
        assert_eq!(groups[0]["type"].as_str(), Some("url-test"));
        assert_eq!(groups[0]["proxies"].as_sequence().map(|p| p.len()), Some(2));
        assert!(groups[0]["url"].as_str().is_some_and(|u| !u.is_empty()));
        assert!(
            !groups.iter().any(|g| g["name"].as_str() == Some("auto")),
            "no select/auto wrapper in kernel mode"
        );
        // Final still points at the main group.
        let rules = rules_of(&doc);
        assert_eq!(rules.last().unwrap(), "MATCH,proxy");
    }

    #[test]
    fn per_rule_node_pin_routes_to_that_node() {
        let a = plain_node("nodeA");
        let b = plain_node("nodeB");
        let mut rule = Rule::new(
            RuleType::DomainSuffix,
            "aa.com".into(),
            RuleTarget::Node,
            10,
        );
        rule.node_id = Some(a.id.clone());
        let mut set = RuleSet::new_user("custom", vec![rule]);
        set.strategy = RuleSetStrategy::Smart; // per-rule decisions preserved
        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        let built = build_meow_config(&[a.clone(), b], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert!(
            rules.contains(&format!("DOMAIN-SUFFIX,aa.com,{}", node_tag_of(&a))),
            "expected aa.com pinned to nodeA; rules: {rules:?}"
        );
    }

    #[test]
    fn whole_set_node_strategy_pins_all_rules() {
        let a = plain_node("nodeA");
        let b = plain_node("nodeB");
        let mut rule = Rule::new(
            RuleType::DomainSuffix,
            "aa.com".into(),
            RuleTarget::Node,
            10,
        );
        rule.node_id = Some(b.id.clone());
        let mut set = RuleSet::new_user("pinned", vec![rule]);
        set.strategy = RuleSetStrategy::Node;
        set.node_id = Some(b.id.clone());
        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        let built = build_meow_config(&[a, b.clone()], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert!(
            rules.contains(&format!("DOMAIN-SUFFIX,aa.com,{}", node_tag_of(&b))),
            "expected aa.com pinned to the set's node (B); rules: {rules:?}"
        );
    }

    #[test]
    fn filter_set_routes_through_keyword_pool_group() {
        let a = plain_node("香港-01");
        let b = plain_node("美国-01");
        let rule = Rule::new(
            RuleType::DomainSuffix,
            "stream.tv".into(),
            RuleTarget::Smart,
            10,
        );
        let mut set = RuleSet::new_user("filter", vec![rule]);
        set.strategy = RuleSetStrategy::Filter;
        set.smart_include = vec!["香港".into()];
        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        let built = build_meow_config(&[a, b], &opts).expect("build");
        let doc = parse(&built);

        // The rule targets the filter pool group — `smart-<set id>` select
        // (sing-box parity: the app-side smart switch maintains these pools
        // by PUT through a stand-in rule per Filter set).
        let rules = rules_of(&doc);
        let stream_rule = rules
            .iter()
            .find(|r| r.contains("stream.tv"))
            .expect("stream.tv rule");
        let group_name = stream_rule.rsplit(',').next().unwrap().to_string();
        assert!(group_name.starts_with("smart-"), "rule was {stream_rule}");
        let group = groups_of(&doc)
            .into_iter()
            .find(|g| g["name"].as_str() == Some(group_name.as_str()))
            .expect("filter pool group");
        assert_eq!(group["type"].as_str(), Some("select"));
        let members = group["proxies"].as_sequence().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].as_str(), Some(built.outbound_tags[0].as_str()));
    }

    #[test]
    fn per_rule_smart_pool_routes_through_own_group() {
        let a = plain_node("香港-01");
        let b = plain_node("美国-01");
        let mut rule = Rule::new(
            RuleType::DomainKeyword,
            "github".into(),
            RuleTarget::Smart,
            10,
        );
        rule.smart_include = vec!["香港".into()];
        let mut set = RuleSet::new_user("smart", vec![rule]);
        set.strategy = RuleSetStrategy::Smart;
        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        let built = build_meow_config(&[a, b], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        let rule_line = rules
            .iter()
            .find(|r| r.starts_with("DOMAIN-KEYWORD,github,"))
            .expect("github rule");
        let group_name = rule_line.rsplit(',').next().unwrap();
        assert!(group_name.starts_with("smart-"), "rule was {rule_line}");
        let doc = parse(&built);
        let group = groups_of(&doc)
            .into_iter()
            .find(|g| g["name"].as_str() == Some(group_name))
            .expect("smart pool group");
        // Select group (app smart switch PUTs into it — url-test 400s),
        // defaulting to the pool's first (best-latency) member.
        assert_eq!(group["type"].as_str(), Some("select"));
        let members = group["proxies"].as_sequence().unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(
            members[0].as_str(),
            Some(built.outbound_tags[0].as_str()),
            "default member = best-latency pool member"
        );
    }

    #[test]
    fn rule_type_matchers_and_targets() {
        let node = plain_node("n1");
        let mut set = RuleSet::new_user(
            "multi",
            vec![
                Rule::new(RuleType::Domain, "exact.com".into(), RuleTarget::Direct, 10),
                Rule::new(
                    RuleType::DomainSuffix,
                    "suffix.com".into(),
                    RuleTarget::Block,
                    20,
                ),
                Rule::new(RuleType::IpCidr, "10.0.0.0/8".into(), RuleTarget::Proxy, 30),
                Rule::new(
                    RuleType::Process,
                    "chrome.exe".into(),
                    RuleTarget::Proxy,
                    40,
                ),
            ],
        );
        set.strategy = RuleSetStrategy::Smart;
        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        let built = build_meow_config(&[node], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert!(rules.contains(&"DOMAIN,exact.com,DIRECT".to_string()));
        assert!(rules.contains(&"DOMAIN-SUFFIX,suffix.com,REJECT".to_string()));
        assert!(rules.contains(&"IP-CIDR,10.0.0.0/8,proxy".to_string()));
        assert!(rules.contains(&"PROCESS-NAME,chrome.exe,proxy".to_string()));
    }

    #[test]
    fn builtin_remote_sets_map_to_geodata() {
        let node = plain_node("n1");
        let mut sets = Vec::new();
        for id in [
            "system-geosite-cn",
            "system-geoip-cn",
            "system-geolocation-not-cn",
        ] {
            let spec = crate::domain::builtin_remote_spec(id).expect("builtin spec");
            let mut set = crate::domain::build_builtin_remote_set(spec);
            set.strategy = if id == "system-geosite-cn" {
                RuleSetStrategy::Direct
            } else {
                RuleSetStrategy::Proxy
            };
            sets.push(set);
        }
        let mut opts = default_opts();
        opts.rule_sets = sets;
        let built = build_meow_config(&[node], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert!(rules.contains(&"GEOSITE,cn,DIRECT".to_string()));
        assert!(rules.contains(&"GEOIP,cn,proxy".to_string()));
        assert!(rules.contains(&"GEOSITE,geolocation-!cn,proxy".to_string()));
    }

    #[test]
    fn bypass_lan_and_block_quic_rules() {
        let node = plain_node("n1");
        let mut opts = default_opts();
        opts.bypass_lan = true;
        opts.block_quic = true;
        let built = build_meow_config(&[node], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert!(rules.contains(&"DOMAIN,localhost,DIRECT".to_string()));
        assert!(rules.contains(&"IP-CIDR,192.168.0.0/16,DIRECT,no-resolve".to_string()));
        assert!(
            rules.contains(&"IP-CIDR6,fe80::/10,DIRECT,no-resolve".to_string()),
            "v6 private bypass missing: {rules:?}"
        );
        // fake-ip range must NOT be bypassed (meow terminates it internally).
        assert!(!rules.iter().any(|r| r.contains("198.18.0.0/15")));
        assert!(rules.contains(&"AND,((NETWORK,udp),(DST-PORT,443)),REJECT".to_string()));
    }

    #[test]
    fn outbound_modes_decide_final() {
        let node = plain_node("n1");
        let mut set = RuleSet::new_user(
            "s",
            vec![Rule::new(
                RuleType::DomainSuffix,
                "a.com".into(),
                RuleTarget::Proxy,
                10,
            )],
        );
        set.strategy = RuleSetStrategy::Smart;

        let mut opts = default_opts();
        opts.rule_sets = vec![set.clone()];
        opts.outbound_mode = OutboundMode::Global;
        let built = build_meow_config(&[node.clone()], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert_eq!(rules.last().unwrap(), "MATCH,proxy");
        assert!(
            !rules.iter().any(|r| r.contains("a.com")),
            "Global ignores user rules"
        );

        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        opts.outbound_mode = OutboundMode::Direct;
        let built = build_meow_config(&[node], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert_eq!(rules.last().unwrap(), "MATCH,DIRECT");

        let mut opts = default_opts();
        opts.route_final = "block".into();
        let built = build_meow_config(&[ss_node("x")], &opts).expect("build");
        assert_eq!(rules_of(&parse(&built)).last().unwrap(), "MATCH,REJECT");
    }

    #[test]
    fn tun_block_forces_fakeip() {
        let node = plain_node("n1");
        let mut opts = default_opts();
        opts.tun_enabled = true;
        opts.dns.fake_ip.enabled = false; // tun must force it anyway
        let built = build_meow_config(&[node], &opts).expect("build");
        let doc = parse(&built);
        assert_eq!(doc["tun"]["enable"].as_bool(), Some(true));
        assert_eq!(doc["tun"]["auto-route"].as_bool(), Some(true));
        assert_eq!(doc["tun"]["dns-hijack"][0].as_str(), Some("any:53"));
        assert_eq!(doc["dns"]["enhanced-mode"].as_str(), Some("fake-ip"));
        assert!(doc["dns"]["fake-ip-range"].as_str().is_some());
    }

    #[test]
    fn dns_split_policy_and_hosts() {
        let node = plain_node("n1");
        let mut opts = default_opts();
        opts.dns.dns_final = "remote".into();
        opts.dns.hosts.enabled = true;
        opts.dns.hosts.entries.push(crate::domain::HostsEntry {
            id: "h1".into(),
            enabled: true,
            domain: "example.test".into(),
            addr: "1.2.3.4".into(),
        });
        let mut set = RuleSet::new_user(
            "cnset",
            vec![Rule::new(
                RuleType::DomainSuffix,
                "cn-site.com".into(),
                RuleTarget::Direct,
                10,
            )],
        );
        set.strategy = RuleSetStrategy::Smart;
        set.dns_strategy = RuleSetDnsStrategy::Domestic;
        opts.rule_sets = vec![set];

        let built = build_meow_config(&[node], &opts).expect("build");
        let doc = parse(&built);
        let dns = &doc["dns"];
        // Built-in pools: remote DoH (each entry egressing through the main
        // proxy group via the `#proxy` fragment) and domestic plain-UDP.
        let nameserver = dns["nameserver"].as_sequence().unwrap();
        assert_eq!(nameserver.len(), 2);
        assert_eq!(
            nameserver[0].as_str(),
            Some("https://1.1.1.1/dns-query#proxy")
        );
        assert_eq!(
            nameserver[1].as_str(),
            Some("https://8.8.8.8/dns-query#proxy")
        );
        let fallback = dns["fallback"].as_sequence().unwrap();
        assert_eq!(fallback[0].as_str(), Some("223.5.5.5"));
        assert_eq!(fallback[1].as_str(), Some("114.114.114.114"));
        assert_eq!(dns["default-nameserver"][0].as_str(), Some("223.5.5.5"));
        // Node hostnames must resolve via the dedicated plain-UDP pool —
        // not the DoH default (chicken-and-egg with unreachable proxies).
        assert_eq!(
            dns["proxy-server-nameserver"][0].as_str(),
            Some("223.5.5.5")
        );
        let policy = dns["nameserver-policy"]["+.cn-site.com"]
            .as_sequence()
            .unwrap();
        assert_eq!(policy[0].as_str(), Some("223.5.5.5"));
        assert_eq!(policy.len(), 2);
        assert_eq!(dns["hosts"]["example.test"].as_str(), Some("1.2.3.4"));

        // Direct outbound mode egresses everything direct — remote DoH
        // drops the proxy-egress tag there.
        let mut opts = opts;
        opts.outbound_mode = OutboundMode::Direct;
        let built = build_meow_config(&[plain_node("n2")], &opts).expect("build");
        let direct = parse(&built);
        let nameserver = direct["dns"]["nameserver"].as_sequence().unwrap();
        assert_eq!(nameserver[0].as_str(), Some("https://1.1.1.1/dns-query"));
        assert_eq!(nameserver[1].as_str(), Some("https://8.8.8.8/dns-query"));
    }

    /// meow rejects `system` as a nameserver value (no system-resolver
    /// scheme — "nameserver-policy entry has no valid nameservers" made the
    /// whole config load fail). Local classification and the `local`
    /// dns_final must land on the plain-UDP domestic resolver instead.
    #[test]
    fn dns_never_emits_system_resolver() {
        let node = plain_node("n1");
        let mut opts = default_opts();
        // Local-classified DNS rule + a set with dns_strategy Local.
        opts.dns.rules_enabled = true;
        opts.dns.rules.push(crate::domain::DnsRule {
            id: "dl1".into(),
            enabled: true,
            matcher: crate::domain::DomainMatcher::DomainSuffix,
            payload: "corp.internal".into(),
            action: crate::domain::DnsAction::Local,
        });
        let mut set = RuleSet::new_user(
            "cnlocal",
            vec![Rule::new(
                RuleType::DomainSuffix,
                "cn-local.com".into(),
                RuleTarget::Direct,
                10,
            )],
        );
        set.strategy = RuleSetStrategy::Smart;
        set.dns_strategy = RuleSetDnsStrategy::Local;
        opts.rule_sets = vec![set];
        // Also cover dns_final=local for the default nameserver.
        opts.dns.dns_final = "local".into();

        let built = build_meow_config(&[node], &opts).expect("build");
        let dns = &parse(&built)["dns"];
        let text = serde_yaml::to_string(dns).unwrap();
        assert!(
            !text.contains("system"),
            "meow rejects the system resolver; dns block was: {text}"
        );
        assert_eq!(dns["nameserver"][0].as_str(), Some("223.5.5.5"));
        assert_eq!(
            dns["nameserver-policy"]["+.corp.internal"][0].as_str(),
            Some("223.5.5.5")
        );
        assert_eq!(
            dns["nameserver-policy"]["+.cn-local.com"][0].as_str(),
            Some("223.5.5.5")
        );
    }

    #[test]
    fn extra_inbounds_become_listeners() {
        let node = plain_node("n1");
        let mut opts = default_opts();
        opts.extra_inbounds = vec![crate::domain::ExtraInbound {
            id: "x1".into(),
            kind: "mixed".into(),
            port: 2081,
            allow_lan: false,
        }];
        let built = build_meow_config(&[node], &opts).expect("build");
        let doc = parse(&built);
        let listeners = doc["listeners"].as_sequence().expect("listeners");
        assert_eq!(listeners[0]["type"].as_str(), Some("mixed"));
        assert_eq!(listeners[0]["port"].as_u64(), Some(2081));
        assert_eq!(listeners[0]["listen"].as_str(), Some("127.0.0.1"));
    }

    /// `cargo test --lib config::meow::tests::live_config_validates -- --ignored`
    /// Validates a generated config with the real meow binary (`-t`). The
    /// config deliberately avoids GEOIP/GEOSITE rules so the empty temp home
    /// dir needs no geodata.
    #[test]
    #[ignore = "needs the bundled dev meow binary"]
    fn live_config_validates() {
        let bin = crate::core::find_bundled_core(None, CoreKind::Meow)
            .expect("bundled meow binary — run the fetch-bundled-meow script");
        let mut node = plain_node("live");
        node.server = "127.0.0.1".into();
        node.tls = Some(TlsConfig {
            enabled: true,
            server_name: Some("sni.example.com".into()),
            insecure: Some(true),
            alpn: Some(vec!["h2".into(), "http/1.1".into()]),
            utls_fingerprint: Some("chrome".into()),
            // Valid-format x25519 key (43 chars base64url = 32 bytes) so the
            // REALITY parser accepts it.
            reality_public_key: Some("a".repeat(43)),
            reality_short_id: Some("abcd0123".into()),
        });
        let mut ws = ss_node("live-ws");
        ws.protocol = Protocol::Vmess;
        ws.config = ProtocolConfig::Vmess {
            uuid: "b831381d-6324-4d53-ad4f-8cda48b30811".into(),
            alter_id: 0,
            security: "auto".into(),
        };
        ws.tls = Some(TlsConfig {
            enabled: true,
            server_name: Some("cdn.example.com".into()),
            insecure: Some(true),
            alpn: None,
            utls_fingerprint: Some("chrome".into()),
            reality_public_key: None,
            reality_short_id: None,
        });
        ws.transport = Some(Transport::Ws {
            path: Some("/ws".into()),
            headers: Some(
                [("Host".to_string(), "cdn.example.com".to_string())]
                    .into_iter()
                    .collect(),
            ),
            max_early_data: Some(2048),
        });

        let mut opts = default_opts();
        opts.bypass_lan = true;
        opts.block_quic = true;
        opts.api_secret = "test-secret".into();
        opts.extra_inbounds = vec![crate::domain::ExtraInbound {
            id: "x1".into(),
            kind: "mixed".into(),
            port: 2081,
            allow_lan: false,
        }];
        let mut set = RuleSet::new_user(
            "live",
            vec![
                Rule::new(
                    RuleType::DomainSuffix,
                    "direct.example".into(),
                    RuleTarget::Direct,
                    10,
                ),
                Rule::new(
                    RuleType::DomainKeyword,
                    "blocked".into(),
                    RuleTarget::Block,
                    20,
                ),
                Rule::new(RuleType::IpCidr, "10.0.0.0/8".into(), RuleTarget::Proxy, 30),
                Rule::new(
                    RuleType::Process,
                    "chrome.exe".into(),
                    RuleTarget::Proxy,
                    40,
                ),
            ],
        );
        set.strategy = RuleSetStrategy::Smart;
        // Reproduce the field-reported failure shape: a builtin remote set
        // with Local DNS strategy puts a `geosite:cn` key into
        // nameserver-policy — the emitted value must be a resolver meow
        // actually accepts (it has no `system` scheme).
        let mut geo_set = crate::domain::build_builtin_remote_set(
            crate::domain::builtin_remote_spec("system-geosite-cn").expect("spec"),
        );
        geo_set.dns_strategy = RuleSetDnsStrategy::Local;
        opts.dns.rules_enabled = true;
        opts.dns.rules.push(crate::domain::DnsRule {
            id: "dl-live".into(),
            enabled: true,
            matcher: crate::domain::DomainMatcher::DomainSuffix,
            payload: "corp.internal".into(),
            action: crate::domain::DnsAction::Local,
        });
        opts.rule_sets = vec![set, geo_set];

        let built = build_meow_config(&[node, ws], &opts).expect("build");

        let tmp = std::env::temp_dir().join(format!(
            "satelite-meow-live-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config_dir = tmp.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join("active.yaml");
        std::fs::write(&config, &built.yaml).unwrap();

        // The geo_set's GEOSITE rule needs the geodata pair in the meow
        // home; stage the bundled dev copy (meow's own auto-download dials
        // through the not-yet-started test proxies and would fail).
        let home = tmp.join("meow");
        let _ = std::fs::create_dir_all(&home);
        if let Some(parent) = bin.parent() {
            for file in ["Country.mmdb", "geosite.dat"] {
                let src = parent.join("meow-geodata").join(file);
                if src.is_file() {
                    let _ = std::fs::copy(&src, home.join(file));
                }
            }
        }

        crate::core::manager::CoreManager::check_config(CoreKind::Meow, &bin, &config)
            .expect("meow -t accepts the generated config");
        let _ = std::fs::remove_dir_all(tmp);
    }
}

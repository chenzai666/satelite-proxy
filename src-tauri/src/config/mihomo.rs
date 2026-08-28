//! Clash YAML config generation for the mihomo core (canonical Clash Meta).
//!
//! mihomo speaks the Clash dialect natively: proxies / proxy-groups / rules are
//! plain Clash YAML, and its REST API is Clash-compatible — so the generated
//! main group keeps the sing-box contract (`proxy`, hot-switched via
//! `PUT /proxies/proxy`; a url-test group in kernel auto-select mode reports
//! its current best through `now`, which the kernel-selection sync reads).
//!
//! Mihomo intentionally uses a Clash-native policy model instead of compiling
//! Satelite's local sing-box rule editor. Every enabled remote category gets a
//! live `select` policy group, and its rule-provider / geodata matcher targets
//! that group. The three system sets map onto GEOSITE/GEOIP matchers
//! (MetaCubeX `.mrs`/mmdb geodata lives in the mihomo home dir).

use crate::config::builder::{outbound_tag, resolve_selected_tag, BuildOptions};
use crate::config::punycode::to_ascii_domain;
use crate::core::kind::CoreKind;
use crate::domain::{
    DnsAction, DomainMatcher, OutboundMode, Protocol, ProtocolConfig, ProxyNode, RuleSet,
    RuleSetDnsStrategy, RuleSetStrategy, Transport,
};
use crate::error::{AppError, AppResult};
use serde_yaml::{Mapping, Value as Yaml};

/// Main group tag — must match the sing-box contract (`state.rs` selects
/// this group over the Clash API and the kernel-selection sync reads `now`).
const MAIN_GROUP: &str = "proxy";
/// Dedicated kernel latency group. The main selector can point to this group
/// while still remaining manually switchable through the Clash API.
const AUTO_GROUP: &str = "auto";
/// Explicit Direct proxy so its IP family can follow the shared setting.
/// Mihomo's magic `DIRECT` target cannot carry `ip-version`.
const DIRECT_PROXY: &str = "Satelite-DIRECT";
/// Built-in remote DoH pool (Clash queries the entries concurrently,
/// fastest answer wins). Every entry egresses through the main proxy group
/// — direct DoH is unreachable on censored networks (see build_dns).
const REMOTE_DNS_POOL: [&str; 2] = ["https://1.1.1.1/dns-query", "https://8.8.8.8/dns-query"];
/// Built-in domestic plain-UDP pool (bootstrap, node hostnames, cn
/// classification). 114DNS backs up AliDNS.
const DOMESTIC_DNS_POOL: [&str; 2] = ["223.5.5.5", "114.114.114.114"];
/// mihomo url-test probe defaults.
const PROBE_INTERVAL_SECS: u64 = 60;
const PROBE_TOLERANCE_MS: u32 = 50;

#[derive(Debug)]
pub struct BuiltMihomoConfig {
    /// Serialized Clash YAML document.
    pub yaml: String,
    /// Node proxy names included in the config (`node-<id>` tags).
    pub outbound_tags: Vec<String>,
    /// Tag of the node the main group selects by default.
    pub selected_tag: String,
}

/// Convert nodes into a complete Clash YAML config document for mihomo.
pub fn build_mihomo_config(
    nodes: &[ProxyNode],
    opts: &BuildOptions,
) -> AppResult<BuiltMihomoConfig> {
    let mut skipped: Vec<String> = Vec::new();
    let mut supported: Vec<ProxyNode> = Vec::new();
    for node in nodes {
        // mihomo (canonical Clash Meta, uTLS) serves every protocol shape we
        // model except Naive/Tor/standalone-ShadowTls (protocol level) and
        // our ss+shadow-tls detour field shape — see supports_node.
        if !CoreKind::Mihomo.supports_node(node) {
            let reason = if !CoreKind::Mihomo.supports(node.protocol) {
                format!("mihomo 不支持 {} 协议", node.protocol.as_str())
            } else {
                "mihomo 暂不支持 ss+shadow-tls 组合的自动映射".to_string()
            };
            skipped.push(format!("{}: {reason}", node.name));
            continue;
        }
        supported.push(node.clone());
    }
    for reason in &skipped {
        crate::app_log::warn("mihomo_config", format!("跳过节点 — {reason}"));
    }
    if supported.is_empty() {
        return Err(AppError::Config(
            "no mihomo-compatible nodes (supports ss/vmess/vless(non-reality)/trojan/hysteria2/anytls/snell/socks5/http)"
                .into(),
        ));
    }

    let tags: Vec<String> = supported.iter().map(outbound_tag).collect();
    let selected_tag = resolve_selected_tag(&supported, &tags, opts.current_node_id.as_deref());
    // Keep providers/groups present in Global and Direct modes too, so users
    // can configure policy selections before returning to Rule mode.
    let (remote_providers, remote_provider_tags) = build_remote_rule_providers(&opts.rule_sets);

    // —— proxy-groups ——
    // Mihomo is exposed as a Clash-native policy engine: the main selector is
    // always manually switchable, with a separate url-test member for kernel
    // auto-selection. Category selectors below can choose the main group,
    // automatic selection, DIRECT/REJECT, or any concrete node.
    let mut groups: Vec<Mapping> = Vec::new();
    let probe_url = probe_url_or_default(opts);
    let mut main_members = vec![AUTO_GROUP.to_string()];
    main_members.extend(tags.clone());
    main_members.push(DIRECT_PROXY.into());
    let main_default = if opts.auto_select.is_kernel() {
        AUTO_GROUP
    } else {
        selected_tag.as_str()
    };
    groups.push(select_group(MAIN_GROUP, main_members, Some(main_default)));
    groups.push(url_test_group(AUTO_GROUP, tags.clone(), &probe_url));

    let (policy_groups, remote_policy_group_tags) =
        build_remote_policy_groups(&opts.rule_sets, &remote_provider_tags, &supported, &tags);
    groups.extend(policy_groups);

    // —— rules ——
    let rules = build_rules(opts, &remote_provider_tags, &remote_policy_group_tags);

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
    // Keep Clash selector choices across Mihomo restarts. The file contains
    // group names and selected members only; credentials remain in active.yaml.
    let mut profile = Mapping::new();
    profile.insert(str_yaml("store-selected"), Yaml::Bool(true));
    profile.insert(str_yaml("store-fake-ip"), Yaml::Bool(true));
    root.insert(str_yaml("profile"), Yaml::Mapping(profile));
    // mihomo honors this switch (unlike some derivatives): `strict` resolves
    // the process only when a rule needs it, `off` disables PROCESS rules.
    root.insert(
        str_yaml("find-process-mode"),
        str_yaml(if opts.find_process { "strict" } else { "off" }),
    );
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
        Yaml::Mapping(build_dns(opts, &opts.rule_sets)),
    );
    if !opts.extra_inbounds.is_empty() {
        root.insert(str_yaml("listeners"), listeners_block(opts));
    }
    let mut proxies: Vec<Yaml> = supported
        .iter()
        .map(|node| Yaml::Mapping(node_to_mihomo_proxy(node)))
        .collect();
    proxies.push(Yaml::Mapping(direct_proxy(opts)));
    root.insert(str_yaml("proxies"), Yaml::Sequence(proxies));
    root.insert(
        str_yaml("proxy-groups"),
        Yaml::Sequence(groups.into_iter().map(Yaml::Mapping).collect()),
    );
    if !remote_providers.is_empty() {
        root.insert(str_yaml("rule-providers"), Yaml::Mapping(remote_providers));
    }
    root.insert(
        str_yaml("rules"),
        Yaml::Sequence(rules.into_iter().map(Yaml::String).collect()),
    );

    let yaml = serde_yaml::to_string(&root)
        .map_err(|e| AppError::Config(format!("serialize mihomo config: {e}")))?;
    Ok(BuiltMihomoConfig {
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

/// mihomo tun block. mihomo works best with fake-ip DNS for tun —
/// `build_dns` forces fake-ip whenever tun is on. `stack` defaults to
/// system in mihomo; `strict-route` etc. stay at defaults intentionally.
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

/// Build one Clash selector for every enabled remote rule category that can
/// contribute a matcher. Local RuleSet rows belong to the sing-box editor and
/// are intentionally absent in mihomo mode.
fn build_remote_policy_groups(
    sets: &[RuleSet],
    provider_tags: &std::collections::HashMap<String, String>,
    nodes: &[ProxyNode],
    node_tags: &[String],
) -> (Vec<Mapping>, std::collections::HashMap<String, String>) {
    let mut groups = Vec::new();
    let mut tags = std::collections::HashMap::new();
    let mut used = std::collections::HashSet::from([
        MAIN_GROUP.to_string(),
        AUTO_GROUP.to_string(),
        "DIRECT".to_string(),
        DIRECT_PROXY.to_string(),
        "REJECT".to_string(),
        "GLOBAL".to_string(),
    ]);
    // Clash requires proxy and group names to share one namespace.
    used.extend(node_tags.iter().cloned());

    for (index, set) in sets
        .iter()
        .filter(|set| set.enabled && set.remote.is_some())
        .enumerate()
    {
        let builtin = matches!(
            set.id.as_str(),
            "system-geosite-cn" | "system-geoip-cn" | "system-geolocation-not-cn"
        );
        if !builtin && !provider_tags.contains_key(&set.id) {
            continue;
        }
        let name = unique_policy_group_name(&set.name, index, &mut used);
        let mut members = vec![
            MAIN_GROUP.to_string(),
            AUTO_GROUP.to_string(),
            DIRECT_PROXY.into(),
            "REJECT".into(),
        ];
        members.extend(node_tags.iter().cloned());
        let default = remote_policy_default(set, nodes, node_tags);
        groups.push(select_group(&name, members, Some(&default)));
        tags.insert(set.id.clone(), name);
    }
    (groups, tags)
}

fn unique_policy_group_name(
    raw: &str,
    index: usize,
    used: &mut std::collections::HashSet<String>,
) -> String {
    let cleaned: String = raw
        .chars()
        .map(|character| {
            if character == ',' || character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let base = cleaned.trim();
    let base = if base.is_empty() {
        format!("规则组 {}", index + 1)
    } else {
        base.to_string()
    };
    let mut candidate = base.clone();
    let mut suffix = 2usize;
    while used.contains(&candidate) {
        candidate = format!("{base} ({suffix})");
        suffix += 1;
    }
    used.insert(candidate.clone());
    candidate
}

fn remote_policy_default(set: &RuleSet, nodes: &[ProxyNode], tags: &[String]) -> String {
    match set.strategy {
        RuleSetStrategy::Direct => DIRECT_PROXY.into(),
        RuleSetStrategy::Block => "REJECT".into(),
        RuleSetStrategy::Node => set
            .node_id
            .as_deref()
            .and_then(|id| nodes.iter().find(|node| node.id == id))
            .map(outbound_tag)
            .filter(|tag| tags.iter().any(|candidate| candidate == tag))
            .unwrap_or_else(|| MAIN_GROUP.into()),
        _ => MAIN_GROUP.into(),
    }
}

/// Build the ordered Clash rule list (strings "TYPE,payload,target").
fn build_rules(
    opts: &BuildOptions,
    remote_provider_tags: &std::collections::HashMap<String, String>,
    remote_policy_group_tags: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let mut rules = Vec::new();
    // Mihomo rule mode is Clash-native: only remote providers/geodata enter
    // routing. Satelite's local sing-box RuleSet rows are not compiled here.
    if opts.outbound_mode == OutboundMode::Rule {
        for set in opts
            .rule_sets
            .iter()
            .filter(|set| set.enabled && set.remote.is_some())
        {
            let Some(target) = remote_policy_group_tags.get(&set.id) else {
                continue;
            };
            if let Some(rule) = builtin_remote_mihomo_rule(set, target) {
                rules.push(rule);
            } else if let Some(provider) = remote_provider_tags.get(&set.id) {
                rules.push(format!("RULE-SET,{provider},{target}"));
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
            "direct" => DIRECT_PROXY.to_string(),
            "block" => "REJECT".to_string(),
            _ => MAIN_GROUP.to_string(),
        },
        OutboundMode::Global => MAIN_GROUP.to_string(),
        OutboundMode::Direct => DIRECT_PROXY.to_string(),
    };
    rules.push(format!("MATCH,{final_target}"));
    rules
}

/// Build inline classical rule-providers from Satelite's validated local
/// remote-rule cache. This keeps route semantics available under mihomo
/// without asking mihomo to understand sing-box source JSON / binary SRS.
/// Provider tags are generated locally instead of embedding RuleSet ids in a
/// comma-delimited rule string, avoiding malformed YAML/rule injection from a
/// damaged or newer store.
fn build_remote_rule_providers(
    sets: &[RuleSet],
) -> (Mapping, std::collections::HashMap<String, String>) {
    let mut providers = Mapping::new();
    let mut tags = std::collections::HashMap::new();
    let mut total_rules = 0usize;

    for (index, set) in sets
        .iter()
        .filter(|set| set.enabled && set.remote.is_some())
        .enumerate()
    {
        if matches!(
            set.id.as_str(),
            "system-geosite-cn" | "system-geoip-cn" | "system-geolocation-not-cn"
        ) {
            continue;
        }
        match remote_set_classical_payload(set) {
            Ok((payload, unsupported)) if !payload.is_empty() => {
                let tag = format!("satelite-remote-{index}");
                let mut provider = Mapping::new();
                provider.insert(str_yaml("type"), str_yaml("inline"));
                provider.insert(str_yaml("behavior"), str_yaml("classical"));
                provider.insert(str_yaml("format"), str_yaml("yaml"));
                provider.insert(
                    str_yaml("payload"),
                    Yaml::Sequence(payload.iter().map(|rule| str_yaml(rule)).collect()),
                );
                total_rules += payload.len();
                providers.insert(str_yaml(&tag), Yaml::Mapping(provider));
                tags.insert(set.id.clone(), tag);
                if unsupported > 0 {
                    crate::app_log::warn(
                        "mihomo_config",
                        format!(
                            "remote rule set '{}' loaded with {unsupported} unsupported entries omitted",
                            set.name
                        ),
                    );
                }
            }
            Ok(_) => crate::app_log::warn(
                "mihomo_config",
                format!(
                    "remote rule set '{}' has no mihomo-compatible entries",
                    set.name
                ),
            ),
            Err(error) => crate::app_log::warn(
                "mihomo_config",
                format!(
                    "remote rule set '{}' unavailable under mihomo: {error}",
                    set.name
                ),
            ),
        }
    }

    if !tags.is_empty() {
        crate::app_log::info(
            "mihomo_config",
            format!(
                "loaded {} remote rule sets into mihomo ({total_rules} classical rules)",
                tags.len()
            ),
        );
    }
    (providers, tags)
}

fn remote_set_classical_payload(set: &RuleSet) -> Result<(Vec<String>, usize), String> {
    let remote = set
        .remote
        .as_ref()
        .ok_or_else(|| "missing remote definition".to_string())?;
    let path = remote
        .local_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "cache is not downloaded".to_string())?;
    let bytes = std::fs::read(path).map_err(|error| format!("read cache: {error}"))?;
    let rules = if remote.format.eq_ignore_ascii_case("binary") {
        crate::srs::parse_with_rules(&bytes)?
            .rules
            .ok_or_else(|| "binary cache contains no readable rules".to_string())?
    } else {
        let source: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| format!("parse cache: {error}"))?;
        source
            .get("rules")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .ok_or_else(|| "source cache has no rules array".to_string())?
    };
    Ok(classical_payload_from_source_rules(&rules))
}

fn classical_payload_from_source_rules(rules: &[serde_json::Value]) -> (Vec<String>, usize) {
    let mut payload = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut unsupported = 0usize;

    for rule in rules {
        let Some(object) = rule.as_object() else {
            unsupported += 1;
            continue;
        };
        if object.contains_key("type")
            || object
                .get("invert")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        {
            unsupported += 1;
            continue;
        }
        let mut groups: std::collections::BTreeMap<&'static str, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut valid = true;
        for (field, value) in object {
            if field == "invert" {
                continue;
            }
            let Some(group) = remote_rule_group(field) else {
                valid = false;
                break;
            };
            let values = value
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| std::slice::from_ref(value));
            if values.is_empty() {
                valid = false;
                break;
            }
            for value in values {
                let Some(rendered) = render_remote_rule_atom(field, value) else {
                    valid = false;
                    break;
                };
                groups.entry(group).or_default().push(rendered);
            }
            if !valid {
                break;
            }
        }
        if !valid || groups.is_empty() {
            unsupported += 1;
            continue;
        }

        // sing-box ORs matchers within one category (domain/IP, port, source
        // address, ...), then ANDs the non-empty categories. A classical
        // provider ORs its payload rows, so preserve that exact structure:
        // flatten a single category, or emit one explicit logical AND row.
        if groups.len() == 1 {
            for atom in groups.into_values().flatten() {
                if seen.insert(atom.clone()) {
                    payload.push(atom);
                }
            }
            continue;
        }
        let conditions = groups
            .into_values()
            .map(|mut atoms| {
                atoms.sort();
                atoms.dedup();
                if atoms.len() == 1 {
                    atoms.pop().unwrap_or_default()
                } else {
                    format!(
                        "OR,({})",
                        atoms
                            .into_iter()
                            .map(|atom| format!("({atom})"))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
            })
            .map(|condition| format!("({condition})"))
            .collect::<Vec<_>>()
            .join(",");
        let logical = format!("AND,({conditions})");
        if seen.insert(logical.clone()) {
            payload.push(logical);
        }
    }
    (payload, unsupported)
}

fn remote_rule_group(field: &str) -> Option<&'static str> {
    match field {
        "domain" | "domain_suffix" | "domain_keyword" | "domain_regex" | "ip_cidr" => {
            Some("destination")
        }
        "port" | "port_range" => Some("destination_port"),
        "source_ip_cidr" => Some("source_address"),
        "source_port" | "source_port_range" => Some("source_port"),
        "network" => Some("network"),
        "process_name" => Some("process_name"),
        "process_path" => Some("process_path"),
        _ => None,
    }
}

fn render_remote_rule_atom(field: &str, value: &serde_json::Value) -> Option<String> {
    match field {
        "domain" => domain_text(value, false).map(|value| format!("DOMAIN,{value}")),
        "domain_suffix" => domain_text(value, true).map(|value| format!("DOMAIN-SUFFIX,{value}")),
        "domain_keyword" => safe_rule_text(value).map(|value| format!("DOMAIN-KEYWORD,{value}")),
        "domain_regex" => safe_rule_text(value).map(|value| format!("DOMAIN-REGEX,{value}")),
        "ip_cidr" => cidr_text(value).map(|value| {
            let kind = if value.contains(':') {
                "IP-CIDR6"
            } else {
                "IP-CIDR"
            };
            format!("{kind},{value},no-resolve")
        }),
        "source_ip_cidr" => cidr_text(value).map(|value| format!("SRC-IP-CIDR,{value},no-resolve")),
        "process_name" => safe_rule_text(value).map(|value| format!("PROCESS-NAME,{value}")),
        "process_path" => safe_rule_text(value).map(|value| format!("PROCESS-PATH,{value}")),
        "network" => safe_rule_text(value)
            .map(str::to_ascii_lowercase)
            .filter(|value| matches!(value.as_str(), "tcp" | "udp"))
            .map(|value| format!("NETWORK,{value}")),
        "port" | "port_range" => port_text(value).map(|value| format!("DST-PORT,{value}")),
        "source_port" | "source_port_range" => {
            port_text(value).map(|value| format!("SRC-PORT,{value}"))
        }
        _ => None,
    }
}

/// Remote rule caches are untrusted network input. Classical provider entries
/// are comma-delimited strings, so reject separators/control characters that
/// could append an unexpected policy or inject another rule.
fn safe_rule_text(value: &serde_json::Value) -> Option<&str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| {
            !value
                .chars()
                .any(|character| character == ',' || character.is_control())
        })
}

fn domain_text(value: &serde_json::Value, suffix: bool) -> Option<String> {
    let value = safe_rule_text(value)?;
    let value = if suffix {
        value.trim_start_matches(['*', '.'])
    } else {
        value
    };
    if value.is_empty() {
        return None;
    }
    let ascii = to_ascii_domain(value);
    (!ascii.is_empty()).then_some(ascii)
}

fn cidr_text(value: &serde_json::Value) -> Option<&str> {
    let value = safe_rule_text(value)?;
    let (address, prefix) = value.split_once('/')?;
    let address = address.parse::<std::net::IpAddr>().ok()?;
    let prefix = prefix.parse::<u8>().ok()?;
    let valid = match address {
        std::net::IpAddr::V4(_) => prefix <= 32,
        std::net::IpAddr::V6(_) => prefix <= 128,
    };
    valid.then_some(value)
}

fn port_text(value: &serde_json::Value) -> Option<String> {
    let normalized = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        // sing-box uses `start:end`; Mihomo classical rules use `start-end`.
        .map(|value| value.replace(':', "-"))
        .or_else(|| value.as_u64().map(|value| value.to_string()))?;
    let ports = normalized
        .split('-')
        .map(str::parse::<u16>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if ports.is_empty()
        || ports.len() > 2
        || ports.iter().any(|port| *port == 0)
        || (ports.len() == 2 && ports[0] > ports[1])
    {
        return None;
    }
    Some(normalized)
}

/// Whole-set rule for a builtin remote set expressed via geodata matchers.
fn builtin_remote_mihomo_rule(set: &RuleSet, target: &str) -> Option<String> {
    let matcher = match set.id.as_str() {
        "system-geosite-cn" => "GEOSITE,cn",
        "system-geoip-cn" => "GEOIP,cn",
        "system-geolocation-not-cn" => "GEOSITE,geolocation-!cn",
        _ => return None,
    };
    Some(format!("{matcher},{target}"))
}

/// Explicit private-range bypass (GEOIP,private is a v2ray/sing-box geodata
/// category that MaxMind mmdb — mihomo's geoip source — does not carry).
/// 198.18.0.0/15 (fake-ip) is intentionally absent: mihomo terminates fake-ip
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
        format!("DOMAIN,localhost,{DIRECT_PROXY}"),
        format!("DOMAIN-SUFFIX,local,{DIRECT_PROXY}"),
    ];
    for cidr in V4 {
        rules.push(format!("IP-CIDR,{cidr},{DIRECT_PROXY},no-resolve"));
    }
    for cidr in V6 {
        rules.push(format!("IP-CIDR6,{cidr},{DIRECT_PROXY},no-resolve"));
    }
    rules
}

// —— DNS ——

/// Map the shared DnsSettings onto mihomo's dns block:
/// `nameserver` (default resolver by dns_final), `fallback` (the other
/// side), `nameserver-policy` (per-domain classification from DNS rules and
/// rule-set dns strategies), `hosts`, and fake-ip when enabled — forced when
/// tun is on (mihomo requires it; the user's stored DNS settings are not
/// written back, so turning tun off restores them).
fn build_dns(opts: &BuildOptions, sets: &[RuleSet]) -> Mapping {
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

    // Mihomo does not consume the local sing-box rule editor. Only system
    // geodata categories contribute resolver classification here.
    for set in sets
        .iter()
        .filter(|set| set.enabled && set.remote.is_some())
    {
        let mut classify = |pat: String, strategy: RuleSetDnsStrategy| match strategy {
            RuleSetDnsStrategy::Domestic => policy_domestic.push(pat),
            RuleSetDnsStrategy::Local => policy_local.push(pat),
            RuleSetDnsStrategy::Remote => policy_remote.push(pat),
        };
        let geosite = match set.id.as_str() {
            "system-geosite-cn" => Some("geosite:cn"),
            "system-geolocation-not-cn" => Some("geosite:geolocation-!cn"),
            _ => None, // ip-only / ordinary provider: no DNS classification
        };
        if let Some(geosite) = geosite {
            classify(geosite.to_string(), set.dns_strategy);
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
    // Remote DoH egress goes through the main proxy group (mihomo's `#adapter`
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
    // mihomo supports the `system` resolver natively — local-classified
    // domains and dns_final=local use it; domestic stays plain-UDP.
    let system_pool: Vec<String> = vec!["system".into()];
    let (default_ns, fallback_ns): (&[String], &[String]) = match dns_final {
        "domestic" => (&domestic_pool, &remote_pool),
        "local" => (&system_pool, &remote_pool),
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
    // its hostname resolved first (chicken-and-egg → "mihomo-dns resolver:
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
        for pat in policy_local {
            policy.insert(str_yaml(&pat), dns_seq(&system_pool));
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

/// An explicit direct proxy has the same routing behavior as Mihomo's magic
/// `DIRECT`, but accepts `ip-version` for actual domain dialing.
fn direct_proxy(opts: &BuildOptions) -> Mapping {
    let mut proxy = Mapping::new();
    proxy.insert(str_yaml("name"), str_yaml(DIRECT_PROXY));
    proxy.insert(str_yaml("type"), str_yaml("direct"));
    let ip_version = match opts.direct_ip_strategy {
        crate::domain::DirectIpStrategy::PreferIpv4 => Some("ipv4-prefer"),
        crate::domain::DirectIpStrategy::Ipv4Only => Some("ipv4"),
        crate::domain::DirectIpStrategy::System => None,
    };
    if let Some(ip_version) = ip_version {
        proxy.insert(str_yaml("ip-version"), str_yaml(ip_version));
    }
    proxy
}

/// Map one node to a Clash proxy mapping (field names mirror what our own
/// `subscription::clash` parser reads — the authoritative inverse).
fn node_to_mihomo_proxy(node: &ProxyNode) -> Mapping {
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
            Protocol::Tuic => "tuic",
            Protocol::WireGuard => "wireguard",
            Protocol::Hysteria => "hysteria",
            Protocol::Ssh => "ssh",
            _ => unreachable!("filtered by CoreKind::Mihomo::supports upstream"),
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
        ProtocolConfig::Tuic {
            uuid,
            password,
            congestion_control,
            udp_relay_mode,
            zero_rtt_handshake,
        } => {
            m.insert(str_yaml("uuid"), str_yaml(uuid));
            m.insert(str_yaml("password"), str_yaml(password));
            if let Some(cc) = congestion_control.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("congestion-controller"), str_yaml(cc));
            }
            if let Some(mode) = udp_relay_mode.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("udp-relay-mode"), str_yaml(mode));
            }
            if *zero_rtt_handshake {
                m.insert(str_yaml("reduce-rtt"), Yaml::Bool(true));
            }
        }
        ProtocolConfig::WireGuard {
            local_address,
            private_key,
            peer_public_key,
            pre_shared_key,
            reserved,
            mtu,
        } => {
            m.insert(str_yaml("private-key"), str_yaml(private_key));
            m.insert(str_yaml("public-key"), str_yaml(peer_public_key));
            if let Some(psk) = pre_shared_key.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("pre-shared-key"), str_yaml(psk));
            }
            if let Some(ip) = local_address.first() {
                m.insert(str_yaml("ip"), str_yaml(ip));
            }
            if let Some(ip6) = local_address.get(1) {
                m.insert(str_yaml("ipv6"), str_yaml(ip6));
            }
            if !reserved.is_empty() {
                m.insert(
                    str_yaml("reserved"),
                    Yaml::Sequence(reserved.iter().map(|b| num_yaml((*b).into())).collect()),
                );
            }
            if let Some(mtu) = mtu {
                m.insert(str_yaml("mtu"), num_yaml((*mtu).into()));
            }
        }
        ProtocolConfig::Hysteria {
            auth,
            auth_base64: _,
            up_mbps,
            down_mbps,
            obfs,
        } => {
            m.insert(str_yaml("auth-str"), str_yaml(auth));
            if let Some(up) = up_mbps {
                m.insert(str_yaml("up"), str_yaml(&up.to_string()));
            }
            if let Some(down) = down_mbps {
                m.insert(str_yaml("down"), str_yaml(&down.to_string()));
            }
            if let Some(obfs) = obfs.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("obfs"), str_yaml(obfs));
            }
        }
        ProtocolConfig::Ssh {
            user,
            password,
            private_key,
            private_key_passphrase,
            ..
        } => {
            m.insert(str_yaml("username"), str_yaml(user));
            if let Some(p) = password.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("password"), str_yaml(p));
            }
            if let Some(pk) = private_key.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("private-key"), str_yaml(pk));
            }
            if let Some(pp) = private_key_passphrase.as_deref().filter(|s| !s.is_empty()) {
                m.insert(str_yaml("private-key-passphrase"), str_yaml(pp));
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
    use crate::domain::{AutoSelectMode, DnsSettings, Rule, RuleTarget, RuleType, TlsConfig};

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

    /** Non-REALITY vless node — the shape mihomo accepts (vless_node carries
     * REALITY, which mihomo filters out since the handshake incompatibility). */
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
            direct_ip_strategy: crate::domain::DirectIpStrategy::PreferIpv4,
            tun_interface_name: None,
        }
    }

    fn parse(built: &BuiltMihomoConfig) -> serde_yaml::Value {
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
        let built = build_mihomo_config(&nodes, &default_opts()).expect("build");
        let doc = parse(&built);
        assert_eq!(doc["mixed-port"].as_u64(), Some(2080));
        assert_eq!(doc["mode"].as_str(), Some("rule"));
        assert_eq!(doc["profile"]["store-selected"].as_bool(), Some(true));
        assert_eq!(doc["external-controller"].as_str(), Some("127.0.0.1:19090"));
        // Clash-native main selector keeps the selected node first and a
        // dedicated automatic url-test group as another member.
        let groups = groups_of(&doc);
        assert_eq!(groups[0]["name"].as_str(), Some("proxy"));
        assert_eq!(groups[0]["type"].as_str(), Some("select"));
        assert_eq!(
            groups[0]["proxies"][0].as_str(),
            Some(built.selected_tag.as_str())
        );
        assert_eq!(groups[1]["name"].as_str(), Some("auto"));
        assert_eq!(groups[1]["type"].as_str(), Some("url-test"));
        // Proxy shape.
        assert_eq!(doc["proxies"][0]["type"].as_str(), Some("ss"));
        assert_eq!(doc["proxies"][0]["cipher"].as_str(), Some("aes-256-gcm"));
        assert_eq!(doc["proxies"][0]["udp"].as_bool(), Some(true));
        let direct = doc["proxies"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|proxy| proxy["name"].as_str() == Some(DIRECT_PROXY))
            .expect("configured direct proxy");
        assert_eq!(direct["type"].as_str(), Some("direct"));
        assert_eq!(direct["ip-version"].as_str(), Some("ipv4-prefer"));
        // Final rule hits the main group.
        let rules = rules_of(&doc);
        assert_eq!(rules.last().unwrap(), "MATCH,proxy");
    }

    #[test]
    fn direct_proxy_honors_ip_strategy() {
        let node = ss_node("n1");

        let mut ipv4_only = default_opts();
        ipv4_only.direct_ip_strategy = crate::domain::DirectIpStrategy::Ipv4Only;
        let built = build_mihomo_config(&[node.clone()], &ipv4_only).expect("build IPv4-only");
        let doc = parse(&built);
        let direct = doc["proxies"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|proxy| proxy["name"].as_str() == Some(DIRECT_PROXY))
            .expect("configured direct proxy");
        assert_eq!(direct["ip-version"].as_str(), Some("ipv4"));

        let mut system = default_opts();
        system.direct_ip_strategy = crate::domain::DirectIpStrategy::System;
        let built = build_mihomo_config(&[node], &system).expect("build system-default");
        let doc = parse(&built);
        let direct = doc["proxies"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|proxy| proxy["name"].as_str() == Some(DIRECT_PROXY))
            .expect("configured direct proxy");
        assert!(direct["ip-version"].is_null());
    }

    #[test]
    fn vless_reality_flow_shape() {
        // Vision+REALITY is KEPT under mihomo (field-audited working; dead
        // REALITY servers surface as through-kernel probe timeouts) — the
        // node maps with its flow, reality-opts and fingerprint.
        let built = build_mihomo_config(
            &[vless_node("vision", Some("xtls-rprx-vision"))],
            &default_opts(),
        )
        .expect("build");
        let doc = parse(&built);
        let proxy = &doc["proxies"][0];
        assert_eq!(proxy["type"].as_str(), Some("vless"));
        assert_eq!(proxy["uuid"].as_str(), Some("uuid-1"));
        assert_eq!(proxy["flow"].as_str(), Some("xtls-rprx-vision"));
        assert_eq!(proxy["tls"].as_bool(), Some(true));
        assert_eq!(proxy["servername"].as_str(), Some("sni.example.com"));
        assert_eq!(proxy["client-fingerprint"].as_str(), Some("chrome"));
        assert_eq!(proxy["reality-opts"]["public-key"].as_str(), Some("pbk"));
        assert_eq!(proxy["reality-opts"]["short-id"].as_str(), Some("abcd0123"));
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
        let built = build_mihomo_config(&[node], &default_opts()).expect("build");
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
        // mihomo (Clash Meta) serves Tuic; Naive has no mihomo outbound.
        let mut naive = vless_node("naive-node", None);
        naive.protocol = Protocol::Naive;
        naive.config = ProtocolConfig::Naive {
            username: "u".into(),
            password: "p".into(),
            quic: false,
        };
        // Only-unsupported → hard error.
        assert!(build_mihomo_config(&[naive.clone()], &default_opts()).is_err());
        // Mixed subscription → unsupported dropped, tuic + ss kept.
        let mut tuic = vless_node("tuic-node", None);
        tuic.protocol = Protocol::Tuic;
        tuic.config = ProtocolConfig::Tuic {
            uuid: "u".into(),
            password: "p".into(),
            congestion_control: None,
            udp_relay_mode: None,
            zero_rtt_handshake: false,
        };
        let ss = ss_node("ss-ok");
        let built =
            build_mihomo_config(&[naive, tuic, ss.clone()], &default_opts()).expect("build");
        assert_eq!(built.outbound_tags.len(), 2);
        assert_eq!(built.outbound_tags[1], node_tag_of(&ss));
    }

    #[test]
    fn vmess_non_ws_transport_is_kept() {
        // mihomo serves vmess over every transport (early Clash Meta was tcp/ws only).
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
        let built = build_mihomo_config(&[node, fallback], &default_opts()).expect("build");
        assert_eq!(built.outbound_tags.len(), 2);
        let doc = parse(&built);
        let proxy = doc["proxies"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|p| p["network"].as_str() == Some("grpc"))
            .expect("vmess grpc proxy present");
        assert_eq!(proxy["type"].as_str(), Some("vmess"));
        assert_eq!(
            proxy["grpc-opts"]["grpc-service-name"].as_str(),
            Some("svc")
        );
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
        assert!(build_mihomo_config(&[node], &default_opts()).is_err());
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
        let built = build_mihomo_config(&[node], &default_opts()).expect("build");
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
        let built = build_mihomo_config(&[hy2, anytls], &default_opts()).expect("build");
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
        let built = build_mihomo_config(&[node], &default_opts()).expect("build");
        let doc = parse(&built);
        let proxy = &doc["proxies"][0];
        assert_eq!(proxy["type"].as_str(), Some("snell"));
        assert_eq!(proxy["psk"].as_str(), Some("pskpw"));
        assert_eq!(proxy["version"].as_str(), Some("4"));
        assert_eq!(proxy["obfs-opts"]["mode"].as_str(), Some("http"));
    }

    #[test]
    fn kernel_autoselect_uses_dedicated_urltest_member() {
        let a = plain_node("a");
        let b = plain_node("b");
        let mut opts = default_opts();
        opts.auto_select = AutoSelectMode::Kernel;
        let built = build_mihomo_config(&[a, b], &opts).expect("build");
        let doc = parse(&built);
        let groups = groups_of(&doc);
        // Main stays a switchable selector and defaults to the dedicated
        // automatic group.
        assert_eq!(groups[0]["name"].as_str(), Some("proxy"));
        assert_eq!(groups[0]["type"].as_str(), Some("select"));
        assert_eq!(groups[0]["proxies"][0].as_str(), Some("auto"));
        assert_eq!(groups[1]["name"].as_str(), Some("auto"));
        assert_eq!(groups[1]["type"].as_str(), Some("url-test"));
        assert_eq!(groups[1]["proxies"].as_sequence().map(|p| p.len()), Some(2));
        assert!(groups[1]["url"].as_str().is_some_and(|u| !u.is_empty()));
        // Final still points at the main group.
        let rules = rules_of(&doc);
        assert_eq!(rules.last().unwrap(), "MATCH,proxy");
    }

    #[test]
    fn local_singbox_rules_are_disabled_in_mihomo() {
        let node = plain_node("n1");
        let set = RuleSet::new_user(
            "sing-box local rules",
            vec![Rule::new(
                RuleType::DomainSuffix,
                "must-not-compile.example".into(),
                RuleTarget::Block,
                10,
            )],
        );
        let mut opts = default_opts();
        opts.rules = set.rules.clone();
        opts.rule_sets = vec![set];
        let built = build_mihomo_config(&[node], &opts).expect("build");
        assert!(!built.yaml.contains("must-not-compile.example"));
        assert_eq!(groups_of(&parse(&built)).len(), 2);
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
        let built = build_mihomo_config(&[node], &opts).expect("build");
        let doc = parse(&built);
        let rules = rules_of(&doc);
        assert!(rules.iter().any(|rule| rule.starts_with("GEOSITE,cn,")));
        assert!(rules.iter().any(|rule| rule.starts_with("GEOIP,cn,")));
        assert!(rules
            .iter()
            .any(|rule| rule.starts_with("GEOSITE,geolocation-!cn,")));
        let groups = groups_of(&doc);
        assert_eq!(groups.len(), 5, "main + auto + three category selectors");
        for group in groups.iter().skip(2) {
            assert_eq!(group["type"].as_str(), Some("select"));
            assert!(group["proxies"].as_sequence().is_some_and(|members| members
                .iter()
                .any(|member| member.as_str() == Some(DIRECT_PROXY))));
        }
    }

    #[test]
    fn source_remote_set_becomes_inline_classical_provider() {
        let cache = std::env::temp_dir().join(format!(
            "satelite-mihomo-provider-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(
            &cache,
            br#"{"rules":[{"domain":["exact.example"],"domain_suffix":[".suffix.example"],"domain_keyword":["needle"],"ip_cidr":["10.0.0.0/8","2001:db8::/32"],"process_name":["client.exe"]}]}"#,
        )
        .unwrap();
        let mut set = RuleSet::new_remote(
            "remote",
            "https://example.com/rules.json",
            RuleTarget::Block,
        );
        // A store id is data, not trusted rule syntax. The generator must use
        // its own safe provider tag rather than interpolating this value.
        set.id = "bad,id\nMATCH,DIRECT".into();
        set.strategy = RuleSetStrategy::Block;
        let remote = set.remote.as_mut().unwrap();
        remote.local_path = Some(cache.to_string_lossy().to_string());
        remote.format = "source".into();

        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        let built = build_mihomo_config(&[plain_node("n1")], &opts).expect("build");
        let _ = std::fs::remove_file(&cache);
        let doc = parse(&built);
        let provider = &doc["rule-providers"]["satelite-remote-0"];
        assert_eq!(provider["type"].as_str(), Some("inline"));
        assert_eq!(provider["behavior"].as_str(), Some("classical"));
        let payload: Vec<&str> = provider["payload"]
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(serde_yaml::Value::as_str)
            .collect();
        assert_eq!(payload.len(), 1);
        let logical = payload[0];
        assert!(logical.starts_with("AND,("));
        assert!(logical.contains("OR,((DOMAIN,exact.example)"));
        assert!(logical.contains("DOMAIN-SUFFIX,suffix.example"));
        assert!(logical.contains("DOMAIN-KEYWORD,needle"));
        assert!(logical.contains("IP-CIDR,10.0.0.0/8,no-resolve"));
        assert!(logical.contains("IP-CIDR6,2001:db8::/32,no-resolve"));
        assert!(logical.contains("(PROCESS-NAME,client.exe)"));
        let rules = rules_of(&doc);
        assert!(rules.contains(&"RULE-SET,satelite-remote-0,remote".to_string()));
        let policy = groups_of(&doc)
            .into_iter()
            .find(|group| group["name"].as_str() == Some("remote"))
            .expect("remote policy group");
        assert_eq!(policy["type"].as_str(), Some("select"));
        assert_eq!(policy["proxies"][0].as_str(), Some("REJECT"));
        assert!(!built.yaml.contains("bad,id"));
    }

    #[test]
    fn unsupported_remote_logical_rules_are_not_broadened() {
        let rules = vec![serde_json::json!({
            "type": "logical",
            "mode": "and",
            "rules": [{"domain": ["example.com"]}, {"port": [443]}]
        })];
        let (payload, unsupported) = classical_payload_from_source_rules(&rules);
        assert!(payload.is_empty());
        assert_eq!(unsupported, 1);
    }

    #[test]
    fn bypass_lan_and_block_quic_rules() {
        let node = plain_node("n1");
        let mut opts = default_opts();
        opts.bypass_lan = true;
        opts.block_quic = true;
        let built = build_mihomo_config(&[node], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert!(rules.contains(&format!("DOMAIN,localhost,{DIRECT_PROXY}")));
        assert!(rules.contains(&format!("IP-CIDR,192.168.0.0/16,{DIRECT_PROXY},no-resolve")));
        assert!(
            rules.contains(&format!("IP-CIDR6,fe80::/10,{DIRECT_PROXY},no-resolve")),
            "v6 private bypass missing: {rules:?}"
        );
        // fake-ip range must NOT be bypassed (mihomo terminates it internally).
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
        let built = build_mihomo_config(&[node.clone()], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert_eq!(rules.last().unwrap(), "MATCH,proxy");
        assert!(
            !rules.iter().any(|r| r.contains("a.com")),
            "Global ignores user rules"
        );

        let mut opts = default_opts();
        opts.rule_sets = vec![set];
        opts.outbound_mode = OutboundMode::Direct;
        let built = build_mihomo_config(&[node], &opts).expect("build");
        let rules = rules_of(&parse(&built));
        assert_eq!(rules.last().unwrap(), &format!("MATCH,{DIRECT_PROXY}"));

        let mut opts = default_opts();
        opts.route_final = "block".into();
        let built = build_mihomo_config(&[ss_node("x")], &opts).expect("build");
        assert_eq!(rules_of(&parse(&built)).last().unwrap(), "MATCH,REJECT");
    }

    #[test]
    fn tun_block_forces_fakeip() {
        let node = plain_node("n1");
        let mut opts = default_opts();
        opts.tun_enabled = true;
        opts.dns.fake_ip.enabled = false; // tun must force it anyway
        let built = build_mihomo_config(&[node], &opts).expect("build");
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

        let built = build_mihomo_config(&[node], &opts).expect("build");
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
        assert!(
            dns["nameserver-policy"]["+.cn-site.com"]
                .as_sequence()
                .is_none(),
            "local sing-box rule sets must not leak into mihomo DNS policy"
        );
        assert_eq!(dns["hosts"]["example.test"].as_str(), Some("1.2.3.4"));

        // Direct outbound mode egresses everything direct — remote DoH
        // drops the proxy-egress tag there.
        let mut opts = opts;
        opts.outbound_mode = OutboundMode::Direct;
        let built = build_mihomo_config(&[plain_node("n2")], &opts).expect("build");
        let direct = parse(&built);
        let nameserver = direct["dns"]["nameserver"].as_sequence().unwrap();
        assert_eq!(nameserver[0].as_str(), Some("https://1.1.1.1/dns-query"));
        assert_eq!(nameserver[1].as_str(), Some("https://8.8.8.8/dns-query"));
    }

    /// mihomo supports the `system` resolver natively —
    /// local classification and dns_final=local emit it directly.
    #[test]
    fn dns_local_classification_uses_system_resolver() {
        let node = vless_node("n1", None);
        let mut opts = default_opts();
        opts.dns.rules_enabled = true;
        opts.dns.rules.push(crate::domain::DnsRule {
            id: "dl1".into(),
            enabled: true,
            matcher: crate::domain::DomainMatcher::DomainSuffix,
            payload: "corp.internal".into(),
            action: crate::domain::DnsAction::Local,
        });
        opts.dns.dns_final = "local".into();

        let built = build_mihomo_config(&[node], &opts).expect("build");
        let dns = &parse(&built)["dns"];
        assert_eq!(dns["nameserver"][0].as_str(), Some("system"));
        assert_eq!(
            dns["nameserver-policy"]["+.corp.internal"][0].as_str(),
            Some("system")
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
        let built = build_mihomo_config(&[node], &opts).expect("build");
        let doc = parse(&built);
        let listeners = doc["listeners"].as_sequence().expect("listeners");
        assert_eq!(listeners[0]["type"].as_str(), Some("mixed"));
        assert_eq!(listeners[0]["port"].as_u64(), Some(2081));
        assert_eq!(listeners[0]["listen"].as_str(), Some("127.0.0.1"));
    }

    /// `cargo test --lib config::mihomo::tests::live_config_validates -- --ignored`
    /// Validates a generated config with the real mihomo binary (`-t`). The
    /// config deliberately avoids GEOIP/GEOSITE rules so the empty temp home
    /// dir needs no geodata.
    #[test]
    #[ignore = "needs the bundled dev mihomo binary"]
    fn live_config_validates() {
        let bin = crate::core::find_bundled_core(None, CoreKind::Mihomo)
            .expect("bundled mihomo binary — run the fetch-bundled-mihomo script");
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
        // nameserver-policy — the emitted value must be a resolver mihomo
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

        let built = build_mihomo_config(&[node, ws], &opts).expect("build");

        let tmp = std::env::temp_dir().join(format!(
            "satelite-mihomo-live-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config_dir = tmp.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let config = config_dir.join("active.yaml");
        std::fs::write(&config, &built.yaml).unwrap();

        // The geo_set's GEOSITE rule needs the geodata pair in the mihomo
        // home; stage the bundled dev copy (mihomo's own auto-download dials
        // through the not-yet-started test proxies and would fail).
        let home = tmp.join("mihomo");
        let _ = std::fs::create_dir_all(&home);
        if let Some(parent) = bin.parent() {
            for file in ["Country.mmdb", "GeoSite.dat"] {
                let src = parent.join("mihomo-geodata").join(file);
                if src.is_file() {
                    let _ = std::fs::copy(&src, home.join(file));
                }
            }
        }

        crate::core::manager::CoreManager::check_config(CoreKind::Mihomo, &bin, &config)
            .expect("mihomo -t accepts the generated config");
        let _ = std::fs::remove_dir_all(tmp);
    }
}

//! DNS settings (PRD: docs/dns.md) — stored in AppStore, emitted into sing-box `dns`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DnsMode {
    /// Only system resolver (`local`).
    System,
    /// Domestic / remote / system split + optional FakeIP (default).
    #[default]
    Smart,
    /// Only user-defined servers and rules.
    Custom,
}

/// Role of a DNS server in Smart mode routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DnsServerRole {
    /// System / local resolver.
    Local,
    /// Domestic (direct path).
    Domestic,
    /// Remote / foreign (via proxy when possible).
    #[default]
    Remote,
    /// User custom (only used when referenced by a rule in custom mode, or as extra remote).
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DomainMatcher {
    Domain,
    #[default]
    DomainSuffix,
    DomainKeyword,
}

/// Where a DNS rule sends the query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DnsAction {
    /// Use system / local DNS.
    System,
    /// Use server by id (tag will be derived).
    Server { server_id: String },
    /// Prefer domestic servers.
    Domestic,
    /// Prefer remote servers.
    Remote,
    /// Block resolution.
    Block,
    /// Force FakeIP for matching domains (A/AAAA).
    FakeIp,
}

impl Default for DnsAction {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsServer {
    pub id: String,
    pub name: String,
    /// `local` | `223.5.5.5` | `tcp://8.8.8.8` | `https://1.1.1.1/dns-query` | `tls://1.1.1.1`
    pub address: String,
    #[serde(default)]
    pub role: DnsServerRole,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsRule {
    pub id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub matcher: DomainMatcher,
    pub payload: String,
    #[serde(default)]
    pub action: DnsAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FakeIpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_fakeip_v4")]
    pub inet4_range: String,
    #[serde(default)]
    pub inet6_enabled: bool,
    #[serde(default = "default_fakeip_v6")]
    pub inet6_range: String,
    /// Domain suffixes that must not use FakeIP (go system / real DNS).
    #[serde(default)]
    pub bypass: Vec<String>,
}

impl Default for FakeIpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            inet4_range: default_fakeip_v4(),
            inet6_enabled: false,
            inet6_range: default_fakeip_v6(),
            bypass: default_fakeip_bypass(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsSettings {
    /// Master switch: when false, behave as System mode.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: DnsMode,
    #[serde(default)]
    pub servers: Vec<DnsServer>,
    /// User DNS rules (whitelist / force resolver). Evaluated first.
    #[serde(default)]
    pub rules: Vec<DnsRule>,
    #[serde(default)]
    pub fake_ip: FakeIpConfig,
    /// Inject route `hijack-dns` (always on with TUN; optional otherwise).
    #[serde(default = "default_true")]
    pub hijack: bool,
    /// independent_cache in sing-box DNS.
    #[serde(default = "default_true")]
    pub cache: bool,
    /// Prefer remote/final over silent system leak (disables strategy fallbacks).
    #[serde(default = "default_true")]
    pub leak_protect: bool,
}

impl Default for DnsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: DnsMode::Smart,
            servers: default_servers(),
            rules: default_rules(),
            fake_ip: FakeIpConfig::default(),
            hijack: true,
            cache: true,
            leak_protect: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_fakeip_v4() -> String {
    "198.18.0.0/15".into()
}

fn default_fakeip_v6() -> String {
    "fc00::/18".into()
}

fn default_fakeip_bypass() -> Vec<String> {
    vec![
        "local".into(),
        "lan".into(),
        "internal".into(),
        "corp".into(),
        "localhost".into(),
    ]
}

fn default_servers() -> Vec<DnsServer> {
    vec![
        DnsServer {
            id: "sys-local".into(),
            name: "System DNS".into(),
            address: "local".into(),
            role: DnsServerRole::Local,
            enabled: true,
        },
        DnsServer {
            id: "cn-ali".into(),
            name: "AliDNS".into(),
            address: "223.5.5.5".into(),
            role: DnsServerRole::Domestic,
            enabled: true,
        },
        DnsServer {
            id: "cn-tencent".into(),
            name: "Tencent".into(),
            address: "119.29.29.29".into(),
            role: DnsServerRole::Domestic,
            enabled: true,
        },
        DnsServer {
            id: "remote-cf".into(),
            name: "Cloudflare".into(),
            address: "https://1.1.1.1/dns-query".into(),
            role: DnsServerRole::Remote,
            enabled: true,
        },
        DnsServer {
            id: "remote-google".into(),
            name: "Google".into(),
            address: "https://dns.google/dns-query".into(),
            role: DnsServerRole::Remote,
            enabled: false,
        },
    ]
}

fn default_rules() -> Vec<DnsRule> {
    // PRD default whitelist → System DNS.
    // Bundled extras: `resources/dns/*.list` (see load_bundled_dns_whitelist).
    ["local", "lan", "internal", "corp"]
        .into_iter()
        .map(|s| DnsRule {
            id: format!("bypass-{s}"),
            enabled: true,
            matcher: DomainMatcher::DomainSuffix,
            payload: s.into(),
            action: DnsAction::System,
        })
        .collect()
}

/// Candidate dirs for bundled DNS lists (dev + packaged).
pub fn dns_dir_candidates(resource_dir: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    out.push(manifest.join("resources/dns"));
    if let Some(res) = resource_dir {
        out.push(res.join("resources/dns"));
        out.push(res.join("dns"));
    }
    out
}

pub fn find_dns_dir(resource_dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    dns_dir_candidates(resource_dir)
        .into_iter()
        .find(|p| p.is_dir())
}

fn parse_dns_action(raw: &str) -> Option<DnsAction> {
    match raw.trim().to_ascii_uppercase().as_str() {
        "SYSTEM" | "LOCAL" => Some(DnsAction::System),
        "DOMESTIC" | "CN" => Some(DnsAction::Domestic),
        "REMOTE" | "PROXY" => Some(DnsAction::Remote),
        "BLOCK" | "REJECT" => Some(DnsAction::Block),
        "FAKEIP" | "FAKE-IP" => Some(DnsAction::FakeIp),
        _ => None,
    }
}

fn parse_dns_matcher(kind: &str) -> Option<DomainMatcher> {
    match kind.trim().to_ascii_uppercase().as_str() {
        "DOMAIN" => Some(DomainMatcher::Domain),
        "DOMAIN-SUFFIX" | "SUFFIX" => Some(DomainMatcher::DomainSuffix),
        "DOMAIN-KEYWORD" | "KEYWORD" => Some(DomainMatcher::DomainKeyword),
        _ => None,
    }
}

/// Parse one DNS whitelist list file.
///
/// Lines:
/// - `example.com` → domain_suffix + system
/// - `DOMAIN-SUFFIX,example.com,SYSTEM`
/// - `DOMAIN,api.example.com,SYSTEM`
/// - `DOMAIN-KEYWORD,corp,DOMESTIC`
pub fn parse_dns_whitelist_text(text: &str, file_stem: &str) -> Vec<DnsRule> {
    let mut out = Vec::new();
    let mut idx = 0u32;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (matcher, payload, action) = if line.contains(',') {
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() < 2 {
                continue;
            }
            let Some(matcher) = parse_dns_matcher(parts[0]) else {
                continue;
            };
            let payload = parts[1].trim();
            if payload.is_empty() {
                continue;
            }
            let action = if parts.len() >= 3 {
                parse_dns_action(parts[2]).unwrap_or(DnsAction::System)
            } else {
                DnsAction::System
            };
            (matcher, payload.to_string(), action)
        } else {
            // bare domain → domain_suffix + system
            if line.contains(char::is_whitespace) {
                continue;
            }
            (
                DomainMatcher::DomainSuffix,
                line.to_string(),
                DnsAction::System,
            )
        };
        idx += 1;
        out.push(DnsRule {
            id: format!("bundled-{file_stem}-{idx}"),
            enabled: true,
            matcher,
            payload,
            action,
        });
    }
    out
}

/// Scan `resources/dns/*.list` (sorted) and load all bundled DNS rules.
pub fn load_bundled_dns_whitelist(resource_dir: Option<&std::path::Path>) -> Vec<DnsRule> {
    let Some(dir) = find_dns_dir(resource_dir) else {
        return Vec::new();
    };
    let mut paths: Vec<std::path::PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("list"))
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();

    let mut out = Vec::new();
    for path in paths {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("dns")
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        out.extend(parse_dns_whitelist_text(&text, &stem));
    }
    out
}

/// Merge bundled DNS whitelist into settings (idempotent by matcher+payload+action).
pub fn ensure_bundled_dns_whitelist(settings: &mut DnsSettings, resource_dir: Option<&std::path::Path>) {
    for rule in load_bundled_dns_whitelist(resource_dir) {
        let exists = settings.rules.iter().any(|r| {
            r.payload.eq_ignore_ascii_case(&rule.payload)
                && r.matcher == rule.matcher
                && r.action == rule.action
        });
        if exists {
            continue;
        }
        settings.rules.insert(0, rule);
    }
}

#[cfg(test)]
mod whitelist_tests {
    use super::*;

    #[test]
    fn parse_bare_and_explicit() {
        let text = r#"
# comment
xiaojukeji.com
DOMAIN-SUFFIX,didichuxing.com,SYSTEM
DOMAIN,api.example.com,DOMESTIC
DOMAIN-KEYWORD,corp,REMOTE
"#;
        let rules = parse_dns_whitelist_text(text, "test");
        assert_eq!(rules.len(), 4);
        assert!(matches!(rules[0].matcher, DomainMatcher::DomainSuffix));
        assert_eq!(rules[0].payload, "xiaojukeji.com");
        assert!(matches!(rules[0].action, DnsAction::System));
        assert!(matches!(rules[2].matcher, DomainMatcher::Domain));
        assert!(matches!(rules[2].action, DnsAction::Domestic));
        assert!(matches!(rules[3].action, DnsAction::Remote));
    }

    #[test]
    fn scan_bundled_dns_dir() {
        let rules = load_bundled_dns_whitelist(None);
        assert!(
            rules.iter().any(|r| r.payload == "xiaojukeji.com"),
            "expected resources/dns/system-whitelist.list"
        );
        assert!(rules.iter().all(|r| matches!(r.action, DnsAction::System)));
    }
}



/// Parsed form of a user-facing DNS address string.
#[derive(Debug, Clone)]
pub enum ParsedDnsAddress {
    Local,
    Udp { server: String, port: Option<u16> },
    Tcp { server: String, port: Option<u16> },
    Https { server: String, path: Option<String> },
    Tls { server: String, port: Option<u16> },
}

/// Parse PRD-style address into sing-box 1.12+ fields.
pub fn parse_dns_address(raw: &str) -> Option<ParsedDnsAddress> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower == "local" || lower == "system" {
        return Some(ParsedDnsAddress::Local);
    }
    if let Some(rest) = lower.strip_prefix("udp://") {
        return parse_host_port(rest).map(|(h, p)| ParsedDnsAddress::Udp {
            server: h,
            port: p,
        });
    }
    if let Some(rest) = lower.strip_prefix("tcp://") {
        return parse_host_port(rest).map(|(h, p)| ParsedDnsAddress::Tcp {
            server: h,
            port: p,
        });
    }
    if let Some(rest) = lower.strip_prefix("tls://") {
        return parse_host_port(rest).map(|(h, p)| ParsedDnsAddress::Tls {
            server: h,
            port: p,
        });
    }
    if lower.starts_with("https://") {
        // https://1.1.1.1/dns-query  or  https://dns.google/dns-query
        let without = s.trim_start_matches("https://").trim_start_matches("HTTPS://");
        let (host_port, path) = match without.split_once('/') {
            Some((h, p)) => (h, Some(format!("/{p}"))),
            None => (without, None),
        };
        let (host, _) = parse_host_port(host_port)?;
        return Some(ParsedDnsAddress::Https {
            server: host,
            path,
        });
    }
    // bare IP / host → UDP
    parse_host_port(s).map(|(h, p)| ParsedDnsAddress::Udp {
        server: h,
        port: p,
    })
}

fn parse_host_port(s: &str) -> Option<(String, Option<u16>)> {
    let s = s.trim().trim_end_matches('/');
    if s.is_empty() {
        return None;
    }
    // [ipv6]:port
    if let Some(rest) = s.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        if tail.is_empty() {
            return Some((host.to_string(), None));
        }
        let port = tail.strip_prefix(':')?.parse().ok();
        return Some((host.to_string(), port));
    }
    // host:port (only if single colon and port is numeric — avoid mangling IPv6)
    if let Some((h, p)) = s.rsplit_once(':') {
        if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(port) = p.parse::<u16>() {
                // IPv4 or hostname
                if h.parse::<std::net::Ipv4Addr>().is_ok() || h.contains('.') || h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    return Some((h.to_string(), Some(port)));
                }
            }
        }
    }
    Some((s.to_string(), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_local() {
        assert!(matches!(
            parse_dns_address("local"),
            Some(ParsedDnsAddress::Local)
        ));
    }

    #[test]
    fn parse_udp_bare() {
        let p = parse_dns_address("223.5.5.5").unwrap();
        match p {
            ParsedDnsAddress::Udp { server, port } => {
                assert_eq!(server, "223.5.5.5");
                assert!(port.is_none());
            }
            _ => panic!("expected udp"),
        }
    }

    #[test]
    fn parse_doh() {
        let p = parse_dns_address("https://1.1.1.1/dns-query").unwrap();
        match p {
            ParsedDnsAddress::Https { server, path } => {
                assert_eq!(server, "1.1.1.1");
                assert_eq!(path.as_deref(), Some("/dns-query"));
            }
            _ => panic!("expected https"),
        }
    }

    #[test]
    fn parse_tls() {
        let p = parse_dns_address("tls://1.1.1.1").unwrap();
        match p {
            ParsedDnsAddress::Tls { server, port } => {
                assert_eq!(server, "1.1.1.1");
                assert!(port.is_none());
            }
            _ => panic!("expected tls"),
        }
    }
}

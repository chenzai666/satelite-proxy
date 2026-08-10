use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    Domain,
    DomainSuffix,
    DomainKeyword,
    IpCidr,
    Process,
    /// Deprecated in sing-box 1.12+; kept for deserialize only.
    Geoip,
}

impl RuleType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Domain => "domain",
            Self::DomainSuffix => "domain_suffix",
            Self::DomainKeyword => "domain_keyword",
            Self::IpCidr => "ip_cidr",
            Self::Process => "process",
            Self::Geoip => "geoip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleTarget {
    Direct,
    Proxy,
    Block,
    /// Pin to a specific subscription node (`node_id` on [`Rule`]).
    Node,
    /// Smart pool: filter nodes by name keywords, then pick best via smart-switch probe.
    Smart,
}

impl RuleTarget {
    pub fn outbound_tag(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Proxy | Self::Node | Self::Smart => "proxy",
            Self::Block => "block",
        }
    }

    /// Clash-compatible third column (NODE/SMART export as PROXY).
    pub fn clash_token(self) -> &'static str {
        match self {
            Self::Direct => "DIRECT",
            Self::Proxy | Self::Node | Self::Smart => "PROXY",
            Self::Block => "REJECT",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "DIRECT" => Some(Self::Direct),
            "PROXY" => Some(Self::Proxy),
            "BLOCK" | "REJECT" | "REJECT-NO-DROP" => Some(Self::Block),
            "NODE" => Some(Self::Node),
            "SMART" => Some(Self::Smart),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    /// Lower = higher priority (applied first).
    pub ord: i32,
    #[serde(rename = "type")]
    pub rule_type: RuleType,
    pub payload: String,
    pub target: RuleTarget,
    pub enabled: bool,
    /// When `target == Node`: stable node id to pin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Snapshot of node display name at save time (for stale-node UI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    /// When `target == Smart`: whitelist — name must contain any keyword (OR). Empty = all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_include: Vec<String>,
    /// When `target == Smart`: blacklist — name containing any keyword is skipped (OR).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_exclude: Vec<String>,
}

impl Rule {
    pub fn new(rule_type: RuleType, payload: String, target: RuleTarget, ord: i32) -> Self {
        let payload = payload.trim().to_string();
        let id = Self::compute_id(rule_type, &payload, target, None, &[], &[]);
        Self {
            id,
            ord,
            rule_type,
            payload,
            target,
            enabled: true,
            node_id: None,
            node_name: None,
            smart_include: Vec::new(),
            smart_exclude: Vec::new(),
        }
    }

    pub fn is_domain_like(&self) -> bool {
        matches!(
            self.rule_type,
            RuleType::Domain | RuleType::DomainSuffix | RuleType::DomainKeyword
        )
    }

    pub fn clash_type_token(&self) -> &'static str {
        match self.rule_type {
            RuleType::Domain => "DOMAIN",
            RuleType::DomainSuffix => "DOMAIN-SUFFIX",
            RuleType::DomainKeyword => "DOMAIN-KEYWORD",
            RuleType::IpCidr => "IP-CIDR",
            RuleType::Process => "PROCESS-NAME",
            RuleType::Geoip => "GEOIP",
        }
    }

    pub fn compute_id(
        rule_type: RuleType,
        payload: &str,
        target: RuleTarget,
        node_id: Option<&str>,
        smart_include: &[String],
        smart_exclude: &[String],
    ) -> String {
        let mut h = Sha256::new();
        h.update(rule_type.as_str().as_bytes());
        h.update(b"|");
        h.update(payload.trim().as_bytes());
        h.update(b"|");
        h.update(format!("{target:?}").as_bytes());
        if let Some(nid) = node_id.filter(|s| !s.is_empty()) {
            h.update(b"|");
            h.update(nid.as_bytes());
        }
        if matches!(target, RuleTarget::Smart) {
            for k in smart_include {
                h.update(b"|+");
                h.update(k.as_bytes());
            }
            for k in smart_exclude {
                h.update(b"|-");
                h.update(k.as_bytes());
            }
        }
        hex::encode(&h.finalize()[..12])
    }

    /// Normalize keyword lists (trim, drop empty, de-dup case-insensitively, preserve order).
    pub fn normalize_keywords(raw: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for s in raw {
            let t = s.trim();
            if t.is_empty() {
                continue;
            }
            let lower = t.to_lowercase();
            if out.iter().any(|x: &String| x.to_lowercase() == lower) {
                continue;
            }
            out.push(t.to_string());
        }
        out
    }

    /// Whether a node display name matches this rule's smart include/exclude filters.
    pub fn smart_name_matches(&self, node_name: &str) -> bool {
        name_matches_keywords(node_name, &self.smart_include, &self.smart_exclude)
    }

    /// Selector outbound tag for a smart rule (stable, short).
    pub fn smart_outbound_tag(&self) -> String {
        format!("smart-{}", &self.id[..self.id.len().min(16)])
    }
}

/// Whitelist (`include`): empty = allow all; otherwise name must contain **any** keyword (OR).
/// Blacklist (`exclude`): name must contain **none** of the keywords (any hit skips).
/// Matching is case-insensitive substring on the display name.
pub fn name_matches_keywords(node_name: &str, include: &[String], exclude: &[String]) -> bool {
    let name = node_name.to_lowercase();

    // Blacklist first: any hit → skip
    for k in exclude {
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        if name.contains(&k.to_lowercase()) {
            return false;
        }
    }

    let include_keys: Vec<&str> = include
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if include_keys.is_empty() {
        return true;
    }
    // Whitelist: any keyword match → allow
    include_keys
        .into_iter()
        .any(|k| name.contains(&k.to_lowercase()))
}

/// Keywords that appear in both include and exclude (case-insensitive). Empty if no conflict.
pub fn keyword_list_overlap(include: &[String], exclude: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for a in include {
        let al = a.trim().to_lowercase();
        if al.is_empty() {
            continue;
        }
        if exclude.iter().any(|b| b.trim().to_lowercase() == al)
            && !out.iter().any(|x: &String| x.to_lowercase() == al)
        {
            out.push(a.trim().to_string());
        }
    }
    out
}

/// Named rule set (built-in or user). Multiple sets can be enabled at once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSet {
    pub id: String,
    pub name: String,
    /// Built-in sets cannot be deleted.
    pub builtin: bool,
    /// When true, rules in this set are merged into the active routing config.
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub rules: Vec<Rule>,
}

fn default_true() -> bool {
    true
}

impl RuleSet {
    pub fn new_user(name: &str, rules: Vec<Rule>) -> Self {
        let id = {
            let mut h = Sha256::new();
            h.update(name.as_bytes());
            h.update(b"|");
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            h.update(nanos.to_le_bytes());
            // Extra entropy so rapid creates don't collide
            h.update(std::process::id().to_le_bytes());
            format!("rs-{}", hex::encode(&h.finalize()[..10]))
        };
        Self {
            id,
            name: name.trim().to_string(),
            builtin: false,
            enabled: true,
            rules,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSetSummary {
    pub id: String,
    pub name: String,
    pub builtin: bool,
    pub rule_count: u32,
    /// Enabled for routing (multiple sets can be true).
    pub enabled: bool,
}

pub const GENERAL_SET_ID: &str = "general-rules";
pub const GENERAL_SET_NAME: &str = "通用规则";

/// Legacy / known id for the large default list file `builtin-ruleset.list`.
pub const BUILTIN_SET_ID: &str = "builtin-ruleset";
pub const BUILTIN_SET_NAME: &str = "内置规则集";

/// Sets that ship as factory templates under `resources/rules/`.
/// - **Restart**: store edits are kept (templates only fill missing sets).
/// - **Reset**: reloads that set from the resource file.
pub fn is_factory_set_id(id: &str) -> bool {
    id == GENERAL_SET_ID || id.starts_with("builtin-")
}

/// Whether the set may be deleted by the user (pure custom sets only).
pub fn is_deletable_rule_set(id: &str, builtin: bool) -> bool {
    !builtin && !is_factory_set_id(id)
}

/// Minimal fallback when `general-rules.list` is missing from resources.
pub fn default_rules() -> Vec<Rule> {
    vec![
        Rule::new(
            RuleType::DomainSuffix,
            "local".into(),
            RuleTarget::Direct,
            10,
        ),
        Rule::new(
            RuleType::DomainSuffix,
            "localhost".into(),
            RuleTarget::Direct,
            20,
        ),
        Rule::new(
            RuleType::IpCidr,
            "10.0.0.0/8".into(),
            RuleTarget::Direct,
            30,
        ),
        Rule::new(
            RuleType::IpCidr,
            "172.16.0.0/12".into(),
            RuleTarget::Direct,
            31,
        ),
        Rule::new(
            RuleType::IpCidr,
            "192.168.0.0/16".into(),
            RuleTarget::Direct,
            32,
        ),
        Rule::new(
            RuleType::IpCidr,
            "127.0.0.0/8".into(),
            RuleTarget::Direct,
            33,
        ),
        Rule::new(RuleType::DomainSuffix, "cn".into(), RuleTarget::Direct, 50),
    ]
}

pub fn sanitize_rules(rules: &[Rule]) -> Vec<Rule> {
    rules
        .iter()
        .filter(|r| !matches!(r.rule_type, RuleType::Geoip))
        .cloned()
        .collect()
}

/// Metadata from leading `# key: value` comments in a `.list` file.
#[derive(Debug, Clone, Default)]
pub struct RuleListMeta {
    pub name: Option<String>,
}

/// One factory rule file discovered under `resources/rules/`.
#[derive(Debug, Clone)]
pub struct BuiltinRuleFile {
    pub id: String,
    pub name: String,
    pub rules: Vec<Rule>,
    /// `true` for `builtin-*.list`; `false` for `general-rules.list` (still factory / resettable).
    pub factory_builtin: bool,
}

impl BuiltinRuleFile {
    pub fn into_rule_set(self) -> RuleSet {
        RuleSet {
            id: self.id,
            name: self.name,
            builtin: self.factory_builtin,
            enabled: true,
            rules: self.rules,
        }
    }
}

/// Parse Shadowrocket / Surge-like rule lines into Rules.
/// Built-in lists: no DNS field → all `dns_policy = inherit`.
pub fn parse_shadowrocket_rules(text: &str) -> Vec<Rule> {
    let mut out = Vec::new();
    let mut ord = 10i32;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        // DOMAIN-SUFFIX,example.com,PROXY
        // DOMAIN-KEYWORD,google,PROXY
        // IP-CIDR,1.2.3.0/24,DIRECT,no-resolve
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let kind = parts[0].to_ascii_uppercase();
        // FINAL,PROXY — skip, app uses route.final
        if kind == "FINAL" || kind == "GEOIP" || kind == "IP-ASN" || kind == "USER-AGENT" {
            continue;
        }
        let (rtype, payload) = match kind.as_str() {
            "DOMAIN" => (RuleType::Domain, parts[1]),
            "DOMAIN-SUFFIX" => (RuleType::DomainSuffix, parts[1]),
            "DOMAIN-KEYWORD" => (RuleType::DomainKeyword, parts[1]),
            "IP-CIDR" | "IP-CIDR6" => (RuleType::IpCidr, parts[1]),
            "PROCESS-NAME" | "PROCESS" => (RuleType::Process, parts[1]),
            _ => continue,
        };
        let Some(target) = RuleTarget::parse(parts[2]) else {
            continue;
        };
        out.push(Rule::new(rtype, payload.to_string(), target, ord));
        ord += 10;
    }
    out
}

/// Serialize a rule set to Clash-style `.list` (routing only; no DNS columns).
pub fn format_clash_rules_list(set_name: &str, rules: &[Rule]) -> String {
    let mut lines = Vec::new();
    lines.push(format!("# name: {set_name}"));
    lines.push("# format: clash/shadowrocket DOMAIN-SUFFIX,host,DIRECT".into());
    lines.push(String::new());
    let mut sorted: Vec<&Rule> = rules
        .iter()
        .filter(|r| !matches!(r.rule_type, RuleType::Geoip))
        .collect();
    sorted.sort_by_key(|r| r.ord);
    for r in sorted {
        if !r.enabled {
            lines.push(format!(
                "# disabled: {},{},{}",
                r.clash_type_token(),
                r.payload.trim(),
                r.target.clash_token()
            ));
            continue;
        }
        let mut line = format!(
            "{},{},{}",
            r.clash_type_token(),
            r.payload.trim(),
            r.target.clash_token()
        );
        if matches!(r.rule_type, RuleType::IpCidr) {
            line.push_str(",no-resolve");
        }
        lines.push(line);
    }
    lines.push(String::new());
    lines.join("\n")
}

/// Parse leading comment metadata (`# name: …`).
pub fn parse_list_meta(text: &str) -> RuleListMeta {
    let mut meta = RuleListMeta::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('#') {
            break;
        }
        let rest = line.trim_start_matches('#').trim();
        if let Some(v) = rest.strip_prefix("name:") {
            let n = v.trim();
            if !n.is_empty() {
                meta.name = Some(n.to_string());
            }
        }
    }
    meta
}

/// Candidate directories for bundled rule lists (dev source tree + packaged resources).
pub fn rules_dir_candidates(resource_dir: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    out.push(manifest.join("resources/rules"));
    if let Some(res) = resource_dir {
        out.push(res.join("resources/rules"));
        out.push(res.join("rules"));
    }
    out
}

/// First existing rules directory.
pub fn find_rules_dir(resource_dir: Option<&std::path::Path>) -> Option<std::path::PathBuf> {
    rules_dir_candidates(resource_dir)
        .into_iter()
        .find(|p| p.is_dir())
}

/// True for routing lists (`foo.list`).
fn is_routing_list_path(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    name.to_ascii_lowercase().ends_with(".list")
}

/// Scan `resources/rules/*.list` (sorted by filename) as **factory templates**.
///
/// - `builtin-*.list` → `factory_builtin = true` (label 内置, cannot delete)
/// - `general-rules.list` → factory seed, not builtin flag (label 通用, cannot delete, can reset)
pub fn load_builtin_rule_files(resource_dir: Option<&std::path::Path>) -> Vec<BuiltinRuleFile> {
    let Some(dir) = find_rules_dir(resource_dir) else {
        return Vec::new();
    };
    let mut paths: Vec<std::path::PathBuf> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_routing_list_path(p))
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();

    let mut out = Vec::new();
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if stem.is_empty() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let meta = parse_list_meta(&text);
        let rules = parse_shadowrocket_rules(&text);
        if rules.is_empty() {
            continue;
        }
        let factory_builtin = stem.starts_with("builtin-");
        let name = meta.name.unwrap_or_else(|| humanize_list_stem(stem));
        out.push(BuiltinRuleFile {
            id: stem.to_string(),
            name,
            rules,
            factory_builtin,
        });
    }
    out
}

fn humanize_list_stem(stem: &str) -> String {
    if stem == GENERAL_SET_ID {
        return GENERAL_SET_NAME.into();
    }
    // builtin-didi → DIDI; builtin-ruleset → 内置规则集
    if let Some(rest) = stem.strip_prefix("builtin-") {
        if rest.eq_ignore_ascii_case("ruleset") {
            return BUILTIN_SET_NAME.into();
        }
        return rest.to_ascii_uppercase();
    }
    stem.to_string()
}

/// Load one factory template by set id from resources (for Reset).
pub fn load_factory_rule_set(
    resource_dir: Option<&std::path::Path>,
    set_id: &str,
) -> Option<RuleSet> {
    load_builtin_rule_files(resource_dir)
        .into_iter()
        .find(|f| f.id == set_id)
        .map(BuiltinRuleFile::into_rule_set)
        .or_else(|| {
            // Fallback only for general-rules if file missing.
            if set_id == GENERAL_SET_ID {
                Some(RuleSet {
                    id: GENERAL_SET_ID.into(),
                    name: GENERAL_SET_NAME.into(),
                    builtin: false,
                    enabled: true,
                    rules: default_rules(),
                })
            } else {
                None
            }
        })
}

/// Factory rule sets from disk (filename order). Empty if directory missing.
pub fn load_builtin_rule_sets(resource_dir: Option<&std::path::Path>) -> Vec<RuleSet> {
    load_builtin_rule_files(resource_dir)
        .into_iter()
        .map(BuiltinRuleFile::into_rule_set)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sample_lines() {
        let text = r#"
DOMAIN-SUFFIX,google.com,PROXY
DOMAIN,api.openai.com,PROXY
DOMAIN-KEYWORD,facebook,PROXY
IP-CIDR,10.0.0.0/8,DIRECT,no-resolve
GEOIP,CN,DIRECT
FINAL,PROXY
"#;
        let rules = parse_shadowrocket_rules(text);
        assert_eq!(rules.len(), 4);
        assert!(matches!(rules[0].rule_type, RuleType::DomainSuffix));
        assert!(matches!(rules[2].rule_type, RuleType::DomainKeyword));
    }

    #[test]
    fn smart_keywords_whitelist_or_blacklist_or() {
        let inc = vec!["新加坡".into(), "日本".into()];
        let exc = vec!["香港".into(), "台湾".into()];
        // Whitelist OR: either keyword ok
        assert!(name_matches_keywords("新加坡 01", &inc, &exc));
        assert!(name_matches_keywords("日本 东京", &inc, &exc));
        assert!(!name_matches_keywords("美国 01", &inc, &exc));
        // Blacklist OR: any hit skips (even if whitelist would pass)
        assert!(!name_matches_keywords("新加坡香港", &inc, &exc));
        assert!(!name_matches_keywords("香港 01", &inc, &exc));
        // Empty whitelist = all except blacklist
        assert!(name_matches_keywords("任意节点", &[], &exc));
        assert!(!name_matches_keywords("HK 香港专线", &[], &exc));
        assert!(!name_matches_keywords("台湾专线", &[], &exc));
    }

    #[test]
    fn smart_keywords_list_overlap() {
        let a = vec!["新加坡".into(), "香港".into()];
        let b = vec!["香港".into(), "日本".into()];
        let o = keyword_list_overlap(&a, &b);
        assert_eq!(o, vec!["香港".to_string()]);
        assert!(keyword_list_overlap(&a, &[]).is_empty());
    }

    #[test]
    fn format_clash_rules_list_basic() {
        let direct = Rule::new(
            RuleType::DomainSuffix,
            "corp.internal".into(),
            RuleTarget::Direct,
            10,
        );
        let proxy = Rule::new(
            RuleType::DomainSuffix,
            "openai.com".into(),
            RuleTarget::Proxy,
            20,
        );
        let clash = format_clash_rules_list("通用", &[direct, proxy]);
        assert!(clash.contains("DOMAIN-SUFFIX,corp.internal,DIRECT"));
        assert!(clash.contains("DOMAIN-SUFFIX,openai.com,PROXY"));
    }

    #[test]
    fn parse_meta_headers() {
        let text = "# name: DIDI\n\nDOMAIN-SUFFIX,a.com,DIRECT\n";
        let meta = parse_list_meta(text);
        assert_eq!(meta.name.as_deref(), Some("DIDI"));
    }

    #[test]
    fn scan_rules_dir_loads_factory_templates() {
        let files = load_builtin_rule_files(None);
        assert!(
            !files.is_empty(),
            "expected resources/rules under CARGO_MANIFEST_DIR"
        );

        let general = files.iter().find(|f| f.id == GENERAL_SET_ID);
        assert!(general.is_some(), "missing general-rules.list");
        let general = general.unwrap();
        assert!(!general.factory_builtin);
        assert!(general.rules.iter().any(|r| r.payload == "local"));

        let large = files.iter().find(|f| f.id == BUILTIN_SET_ID);
        assert!(large.is_some());
        assert!(large.unwrap().factory_builtin);
        assert!(large.unwrap().rules.len() > 100);
        assert!(!large
            .unwrap()
            .rules
            .iter()
            .any(|r| matches!(r.rule_type, RuleType::Geoip)));
    }
}

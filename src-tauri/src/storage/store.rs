use crate::domain::{
    default_rules, is_factory_set_id, load_builtin_rule_sets, load_factory_rule_set,
    sanitize_rules, AppSettings, DnsAction, DnsRuleSetKind, DnsSettings, DomainMatcher, ProxyNode,
    Rule, RuleSet, RuleSetDnsStrategy, RuleSetOwnership, RuleSetStrategy, RuleSetSummary,
    RuleTarget, RuleType, Subscription, BUILTIN_SET_ID, BUILTIN_SET_NAME, GENERAL_SET_ID,
    GENERAL_SET_NAME,
};
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppStore {
    #[serde(default)]
    pub schema_version: u32,
    pub subscriptions: Vec<Subscription>,
    pub nodes: Vec<StoredNode>,
    #[serde(default)]
    pub settings: AppSettings,
    /// DNS module (docs/dns.md).
    #[serde(default)]
    pub dns: DnsSettings,
    /// Legacy flat rules (migrated into a user rule set once).
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub rule_sets: Vec<RuleSet>,
    /// Legacy single-active field; migrated into `RuleSet.enabled`.
    #[serde(default)]
    pub active_rule_set_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredNode {
    pub subscription_id: String,
    #[serde(flatten)]
    pub node: ProxyNode,
}

impl AppStore {
    pub fn load(path: &Path, resource_dir: Option<&Path>) -> AppResult<Self> {
        if !path.exists() {
            return Ok(Self::with_builtin_sets(resource_dir));
        }
        let raw = fs::read_to_string(path)?;
        if raw.trim().is_empty() {
            return Ok(Self::with_builtin_sets(resource_dir));
        }
        let mut store: Self = serde_json::from_str(&raw)
            .map_err(|e| AppError::Storage(format!("invalid store json: {e}")))?;
        let schema_before = store.schema_version;
        store.settings.migrate_auto_select();
        store.settings.migrate_capture_mode();
        store.dns.ensure_rule_sets();
        store.migrate_unified_rule_sets();
        store.ensure_rule_sets(resource_dir);
        store.migrate_redundant_general_rule_set();
        store.migrate_remote_update_policy();
        store.ensure_subscription_enable_policy();
        if schema_before < 5 {
            let backup = path.with_file_name("store.pre-v5.backup.json");
            if !backup.exists() {
                let _ = fs::write(backup, &raw);
            }
        }
        // Persist migrations (new rule files) so they survive read-only sessions.
        let _ = store.save(path);
        Ok(store)
    }

    fn with_builtin_sets(resource_dir: Option<&Path>) -> Self {
        let mut s = Self::default();
        s.dns.ensure_rule_sets();
        s.migrate_unified_rule_sets();
        s.ensure_rule_sets(resource_dir);
        s.migrate_redundant_general_rule_set();
        s.migrate_remote_update_policy();
        s
    }

    /// Ensure factory rule sets from `resources/rules/*.list`.
    ///
    /// **Restart policy**: only *insert missing* factory sets. Existing sets keep
    /// user edits (rules, enabled). Never overwrite rules from disk on startup.
    ///
    /// **Reset policy**: use [`Self::reset_rule_set`] to reload one factory set
    /// from resources (explicit user action).
    pub fn ensure_rule_sets(&mut self, resource_dir: Option<&Path>) {
        // Migrate old id `builtin-shadowrocket` → `builtin-ruleset`
        const OLD_BUILTIN_ID: &str = "builtin-shadowrocket";
        if let Some(set) = self.rule_sets.iter_mut().find(|s| s.id == OLD_BUILTIN_ID) {
            set.id = BUILTIN_SET_ID.into();
            set.name = BUILTIN_SET_NAME.into();
            set.builtin = true;
        }
        if self.active_rule_set_id.as_deref() == Some(OLD_BUILTIN_ID) {
            self.active_rule_set_id = Some(BUILTIN_SET_ID.into());
        }

        // Rename migrated-legacy / 自定义 → 通用规则 (before factory insert)
        for set in self.rule_sets.iter_mut() {
            if set.id == "migrated-legacy" || set.name == "我的规则（迁移）" || set.name == "自定义"
            {
                set.id = GENERAL_SET_ID.into();
                set.name = GENERAL_SET_NAME.into();
                set.builtin = false;
            }
        }
        let mut seen_general = false;
        self.rule_sets.retain(|s| {
            if s.id == GENERAL_SET_ID {
                if seen_general {
                    return false;
                }
                seen_general = true;
                // general is factory but not "builtin" label
            }
            true
        });
        if let Some(g) = self.rule_sets.iter_mut().find(|s| s.id == GENERAL_SET_ID) {
            g.builtin = false;
            g.name = GENERAL_SET_NAME.into();
        }

        // Factory templates: insert missing only; never clobber store rules on restart.
        let discovered = load_builtin_rule_sets(resource_dir);
        let factory_ids: Vec<String> = discovered.iter().map(|s| s.id.clone()).collect();
        for set in discovered {
            if let Some(existing) = self.rule_sets.iter_mut().find(|s| s.id == set.id) {
                // Keep edits; only refresh metadata flags/name from template.
                existing.builtin = set.builtin;
                existing.ownership = RuleSetOwnership::Builtin;
                if !set.name.is_empty() {
                    existing.name = set.name;
                }
                continue;
            }
            // Insert factory sets near the front (after other factory already present).
            let insert_at = self
                .rule_sets
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    is_factory_set_id(&s.id) && factory_ids.iter().any(|id| id == &s.id)
                })
                .map(|(i, _)| i + 1)
                .last()
                .unwrap_or(0);
            self.rule_sets.insert(insert_at, set);
        }

        // Migrate legacy flat rules → 通用规则
        let legacy = sanitize_rules(&self.rules);
        if !legacy.is_empty() {
            if let Some(set) = self.rule_sets.iter_mut().find(|s| s.id == GENERAL_SET_ID) {
                if set.rules.is_empty() {
                    set.rules = legacy;
                } else {
                    set.rules.extend(legacy);
                }
            } else {
                self.rule_sets.push(RuleSet {
                    id: GENERAL_SET_ID.into(),
                    name: GENERAL_SET_NAME.into(),
                    builtin: false,
                    enabled: true,
                    ownership: RuleSetOwnership::User,
                    strategy: RuleSetStrategy::Smart,
                    dns_strategy: RuleSetDnsStrategy::Remote,
                    remote: None,
                    dns_rules: Vec::new(),
                    rules: legacy,
                });
            }
            self.rules.clear();
        }

        // Migrate single active_rule_set_id → RuleSet.enabled (multi)
        if let Some(id) = self.active_rule_set_id.take() {
            let any_enabled = self.rule_sets.iter().any(|s| s.enabled);
            if !any_enabled {
                for s in self.rule_sets.iter_mut() {
                    s.enabled = s.id == id || is_factory_set_id(&s.id);
                }
            } else if let Some(s) = self.rule_sets.iter_mut().find(|s| s.id == id) {
                s.enabled = true;
            }
        }

        // If nothing enabled, enable all factory sets
        if !self.rule_sets.iter().any(|s| s.enabled) {
            for s in self.rule_sets.iter_mut() {
                if is_factory_set_id(&s.id) {
                    s.enabled = true;
                }
            }
        }
    }

    /// Upgrade rule sets to the unified route + DNS policy model.
    pub fn migrate_unified_rule_sets(&mut self) {
        const VERSION: u32 = 3;
        if self.schema_version >= VERSION {
            return;
        }

        if self.schema_version < 2 {
            for set in &mut self.rule_sets {
                if set.id == "builtin-shadowrocket" {
                    set.id = BUILTIN_SET_ID.into();
                    set.name = BUILTIN_SET_NAME.into();
                    set.builtin = true;
                }
                if set.id == "migrated-legacy"
                    || set.name == "我的规则（迁移）"
                    || set.name == "自定义"
                {
                    set.id = GENERAL_SET_ID.into();
                    set.name = GENERAL_SET_NAME.into();
                }
            }

            let legacy = sanitize_rules(&self.rules);
            if !legacy.is_empty() {
                if let Some(general) = self
                    .rule_sets
                    .iter_mut()
                    .find(|set| set.id == GENERAL_SET_ID)
                {
                    general.rules.extend(legacy);
                } else {
                    let mut general = RuleSet::new_user(GENERAL_SET_NAME, legacy);
                    general.id = GENERAL_SET_ID.into();
                    self.rule_sets.push(general);
                }
                self.rules.clear();
            }

            let mut migrated = Vec::new();
            for mut set in std::mem::take(&mut self.rule_sets) {
                set.ownership = if set.builtin || is_factory_set_id(&set.id) {
                    RuleSetOwnership::Builtin
                } else {
                    RuleSetOwnership::User
                };
                if set.remote.is_some() {
                    if let Some(remote) = &set.remote {
                        set.strategy = RuleSetStrategy::from_target(remote.target);
                    }
                    migrated.push(set);
                    continue;
                }

                let mut buckets: Vec<(&'static str, Vec<Rule>)> = Vec::new();
                for rule in std::mem::take(&mut set.rules) {
                    let key = match rule.target {
                        RuleTarget::Proxy => "proxy",
                        RuleTarget::Direct => "direct",
                        RuleTarget::Block => "block",
                        RuleTarget::Node | RuleTarget::Smart => "smart",
                    };
                    if let Some((_, rules)) = buckets.iter_mut().find(|(bucket, _)| *bucket == key)
                    {
                        rules.push(rule);
                    } else {
                        buckets.push((key, vec![rule]));
                    }
                }
                if buckets.is_empty() {
                    migrated.push(set);
                    continue;
                }
                let mixed = buckets.len() > 1;
                for (key, rules) in buckets {
                    let mut sibling = set.clone();
                    sibling.rules = rules;
                    sibling.strategy = match key {
                        "proxy" => RuleSetStrategy::Proxy,
                        "direct" => RuleSetStrategy::Direct,
                        "block" => RuleSetStrategy::Block,
                        _ => RuleSetStrategy::Smart,
                    };
                    if mixed && !(set.id == GENERAL_SET_ID && key == "direct") {
                        let suffix = match sibling.strategy {
                            RuleSetStrategy::Proxy => "代理",
                            RuleSetStrategy::Direct => "直连",
                            RuleSetStrategy::Block => "拦截",
                            RuleSetStrategy::Smart => "智能",
                        };
                        sibling.id = format!("{}-{key}", set.id);
                        sibling.name = format!("{} · {suffix}", set.name);
                    }
                    migrated.push(sibling);
                }
            }

            for dns_set in self
                .dns
                .rule_sets
                .iter()
                .filter(|set| set.kind == DnsRuleSetKind::Dns)
            {
                let mut buckets: Vec<(&'static str, Vec<_>)> = Vec::new();
                for rule in &dns_set.dns_rules {
                    let key = match rule.action {
                        DnsAction::Local => "direct",
                        DnsAction::Remote => "proxy",
                        DnsAction::Domestic => "smart",
                        DnsAction::Block => "block",
                    };
                    if let Some((_, rules)) = buckets.iter_mut().find(|(bucket, _)| *bucket == key)
                    {
                        rules.push(rule.clone());
                    } else {
                        buckets.push((key, vec![rule.clone()]));
                    }
                }
                for (key, dns_rules) in buckets {
                    let strategy = match key {
                        "direct" => RuleSetStrategy::Direct,
                        "proxy" => RuleSetStrategy::Proxy,
                        "block" => RuleSetStrategy::Block,
                        _ => RuleSetStrategy::Smart,
                    };
                    let suffix = match strategy {
                        RuleSetStrategy::Direct => "直连",
                        RuleSetStrategy::Proxy => "代理",
                        _ => "智能",
                    };
                    migrated.push(RuleSet {
                        id: format!("dns-{}-{key}", dns_set.id),
                        name: format!("{} · {suffix}", dns_set.name),
                        builtin: dns_set.builtin,
                        enabled: dns_set.enabled,
                        ownership: if dns_set.builtin {
                            RuleSetOwnership::Builtin
                        } else {
                            RuleSetOwnership::User
                        },
                        strategy,
                        dns_strategy: match key {
                            "direct" => RuleSetDnsStrategy::Local,
                            "smart" => RuleSetDnsStrategy::Domestic,
                            _ => RuleSetDnsStrategy::Remote,
                        },
                        remote: None,
                        dns_rules,
                        rules: Vec::new(),
                    });
                }
            }

            self.rule_sets = migrated;
            self.dns.unified_rules = true;
            self.dns
                .rule_sets
                .retain(|set| set.kind == DnsRuleSetKind::Hosts);
            self.dns.ensure_rule_sets();
            self.schema_version = 2;
        }

        // v3: one matcher list is shared by route and DNS. v2 briefly stored
        // DNS matchers separately; fold those entries back without losing data.
        if self.schema_version < 3 {
            for set in &mut self.rule_sets {
                set.dns_strategy = set
                    .dns_rules
                    .first()
                    .map(|rule| match rule.action {
                        DnsAction::Local => RuleSetDnsStrategy::Local,
                        DnsAction::Domestic => RuleSetDnsStrategy::Domestic,
                        DnsAction::Remote | DnsAction::Block => RuleSetDnsStrategy::Remote,
                    })
                    .unwrap_or_else(|| match set.strategy {
                        RuleSetStrategy::Direct => RuleSetDnsStrategy::Local,
                        RuleSetStrategy::Proxy
                        | RuleSetStrategy::Block
                        | RuleSetStrategy::Smart => RuleSetDnsStrategy::Remote,
                    });

                let dns_rules = std::mem::take(&mut set.dns_rules);
                let mut next_ord = set.rules.iter().map(|rule| rule.ord).max().unwrap_or(0) + 10;
                for dns_rule in dns_rules {
                    let rule_type = match dns_rule.matcher {
                        DomainMatcher::Domain => RuleType::Domain,
                        DomainMatcher::DomainSuffix => RuleType::DomainSuffix,
                        DomainMatcher::DomainKeyword => RuleType::DomainKeyword,
                    };
                    if set.rules.iter().any(|rule| {
                        rule.rule_type == rule_type
                            && rule.payload.eq_ignore_ascii_case(&dns_rule.payload)
                    }) {
                        continue;
                    }
                    set.rules.push(Rule {
                        id: dns_rule.id,
                        ord: next_ord,
                        rule_type,
                        payload: dns_rule.payload,
                        target: set.strategy.route_target().unwrap_or(RuleTarget::Direct),
                        enabled: dns_rule.enabled,
                        node_id: None,
                        node_name: None,
                        smart_include: Vec::new(),
                        smart_exclude: Vec::new(),
                    });
                    next_ord += 10;
                }
            }
            self.schema_version = VERSION;
        }
    }

    /// v4 removes the old factory "通用规则" because all seven entries are
    /// already present in the built-in direct set. Preserve edited copies as a
    /// normal user set; only the untouched factory payload is redundant.
    pub fn migrate_redundant_general_rule_set(&mut self) {
        const VERSION: u32 = 4;
        if self.schema_version >= VERSION {
            return;
        }

        if let Some(index) = self
            .rule_sets
            .iter()
            .position(|set| set.id == GENERAL_SET_ID)
        {
            if same_rules_ignoring_storage_fields(&self.rule_sets[index].rules, &default_rules()) {
                self.rule_sets.remove(index);
            } else {
                let set = &mut self.rule_sets[index];
                set.builtin = false;
                set.ownership = RuleSetOwnership::User;
            }
        }
        self.schema_version = VERSION;
    }

    /// v5: remote updates used to run hourly without an explicit user choice.
    /// Upgrade existing remote sets to opt-in scheduling; newly created sets
    /// persist the user's selected interval and are already on schema v5.
    pub fn migrate_remote_update_policy(&mut self) {
        const VERSION: u32 = 5;
        if self.schema_version >= VERSION {
            return;
        }
        for set in &mut self.rule_sets {
            if let Some(remote) = set.remote.as_mut() {
                remote.update_interval = "disabled".into();
            }
        }
        self.schema_version = VERSION;
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Storage(format!("serialize store: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn upsert_subscription(
        &mut self,
        sub: Subscription,
        nodes: Vec<ProxyNode>,
    ) -> AppResult<()> {
        let id = sub.id.clone();
        self.nodes.retain(|n| n.subscription_id != id);
        if let Some(existing) = self.subscriptions.iter_mut().find(|s| s.id == id) {
            *existing = sub;
        } else {
            self.subscriptions.push(sub);
        }
        for node in nodes {
            self.nodes.push(StoredNode {
                subscription_id: id.clone(),
                node,
            });
        }
        Ok(())
    }

    pub fn remove_subscription(&mut self, id: &str) -> AppResult<()> {
        let before = self.subscriptions.len();
        self.subscriptions.retain(|s| s.id != id);
        if self.subscriptions.len() == before {
            return Err(AppError::NotFound(id.to_string()));
        }
        self.nodes.retain(|n| n.subscription_id != id);
        // If removed was the only enabled, enable first remaining.
        if !self.subscriptions.iter().any(|s| s.enabled) {
            if let Some(first) = self.subscriptions.first_mut() {
                first.enabled = true;
            }
        }
        self.ensure_current_node_valid();
        Ok(())
    }

    pub fn get_subscription(&self, id: &str) -> Option<&Subscription> {
        self.subscriptions.iter().find(|s| s.id == id)
    }

    pub fn enabled_nodes(&self) -> Vec<ProxyNode> {
        let enabled: std::collections::HashSet<_> = self
            .subscriptions
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.id.as_str())
            .collect();
        self.nodes
            .iter()
            .filter(|n| enabled.contains(n.subscription_id.as_str()))
            .map(|n| n.node.clone())
            .collect()
    }

    /// Exclusive (default): only one subscription enabled.
    /// Mix: multiple can be enabled.
    pub fn ensure_subscription_enable_policy(&mut self) {
        if self.subscriptions.is_empty() {
            return;
        }
        if !self.settings.mix_mode {
            let enabled: Vec<String> = self
                .subscriptions
                .iter()
                .filter(|s| s.enabled)
                .map(|s| s.id.clone())
                .collect();
            if enabled.len() > 1 {
                let keep = enabled[0].clone();
                for s in &mut self.subscriptions {
                    s.enabled = s.id == keep;
                }
            } else if enabled.is_empty() {
                if let Some(first) = self.subscriptions.first_mut() {
                    first.enabled = true;
                }
            }
        } else if !self.subscriptions.iter().any(|s| s.enabled) {
            if let Some(first) = self.subscriptions.first_mut() {
                first.enabled = true;
            }
        }
        self.ensure_current_node_valid();
    }

    /// Click card: exclusive → enable only this; Mix → toggle this.
    pub fn activate_subscription(&mut self, id: &str) -> AppResult<()> {
        if !self.subscriptions.iter().any(|s| s.id == id) {
            return Err(AppError::NotFound(id.to_string()));
        }
        if self.settings.mix_mode {
            let currently = self
                .subscriptions
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.enabled)
                .unwrap_or(false);
            // Don't allow disabling the last enabled subscription.
            if currently {
                let enabled_count = self.subscriptions.iter().filter(|s| s.enabled).count();
                if enabled_count <= 1 {
                    return Ok(());
                }
                if let Some(s) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                    s.enabled = false;
                }
            } else if let Some(s) = self.subscriptions.iter_mut().find(|s| s.id == id) {
                s.enabled = true;
            }
        } else {
            for s in &mut self.subscriptions {
                s.enabled = s.id == id;
            }
        }
        self.ensure_current_node_valid();
        Ok(())
    }

    pub fn set_mix_mode(&mut self, mix: bool) -> AppResult<()> {
        self.settings.mix_mode = mix;
        self.ensure_subscription_enable_policy();
        Ok(())
    }

    /// Drop current_node if it is not in any enabled subscription.
    pub fn ensure_current_node_valid(&mut self) {
        if let Some(ref cur) = self.settings.current_node_id {
            let still = self.nodes.iter().any(|n| {
                &n.node.id == cur && {
                    self.subscriptions
                        .iter()
                        .any(|s| s.enabled && s.id == n.subscription_id)
                }
            });
            if !still {
                self.settings.current_node_id = self.enabled_nodes().first().map(|n| n.id.clone());
            }
        }
    }

    /// New subscription: enable only when no other is enabled (or none exist).
    pub fn prepare_new_subscription_enabled(&self, sub: &mut Subscription) {
        let any_enabled = self
            .subscriptions
            .iter()
            .any(|s| s.enabled && s.id != sub.id);
        if any_enabled {
            sub.enabled = false;
        } else {
            sub.enabled = true;
        }
    }

    pub fn find_node(&self, id: &str) -> Option<&ProxyNode> {
        self.nodes.iter().find(|n| n.node.id == id).map(|n| &n.node)
    }

    pub fn update_node_latency(
        &mut self,
        id: &str,
        latency_ms: Option<u32>,
        latency_at: i64,
    ) -> bool {
        if let Some(n) = self.nodes.iter_mut().find(|n| n.node.id == id) {
            n.node.latency_ms = latency_ms;
            n.node.latency_at = Some(latency_at);
            true
        } else {
            false
        }
    }

    /// Merge rules from all **enabled** rule sets (set order, then rule.ord).
    pub fn enabled_rules_sorted(&self) -> Vec<Rule> {
        let mut out = Vec::new();
        for set in &self.rule_sets {
            if !set.enabled {
                continue;
            }
            let mut rules: Vec<_> = set
                .rules
                .iter()
                .filter(|r| r.enabled)
                .filter(|r| !matches!(r.rule_type, crate::domain::RuleType::Geoip))
                .cloned()
                .collect();
            rules.sort_by_key(|r| r.ord);
            out.extend(rules);
        }
        if out.is_empty()
            && !self
                .rule_sets
                .iter()
                .any(|set| set.enabled && set.remote.is_some())
        {
            return sanitize_rules(&default_rules());
        }
        out
    }

    pub fn list_rule_set_summaries(&self) -> Vec<RuleSetSummary> {
        self.rule_sets
            .iter()
            .map(|s| RuleSetSummary {
                id: s.id.clone(),
                name: s.name.clone(),
                builtin: s.builtin,
                rule_count: s
                    .remote
                    .as_ref()
                    .and_then(|remote| remote.rule_count)
                    .unwrap_or(s.rules.len() as u32),
                enabled: s.enabled,
                ownership: s.ownership,
                strategy: s.strategy,
                dns_strategy: s.dns_strategy,
                remote: s.remote.clone(),
            })
            .collect()
    }

    /// Enable/disable a rule set for routing (multiple can be enabled).
    pub fn set_rule_set_enabled(&mut self, id: &str, enabled: bool) -> AppResult<()> {
        let set = self
            .rule_sets
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        set.enabled = enabled;
        Ok(())
    }

    pub fn get_rule_set(&self, id: &str) -> Option<&RuleSet> {
        self.rule_sets.iter().find(|s| s.id == id)
    }

    pub fn upsert_rule_in_set(&mut self, set_id: &str, rule: Rule) -> AppResult<Rule> {
        let set = self
            .rule_sets
            .iter_mut()
            .find(|s| s.id == set_id)
            .ok_or_else(|| AppError::NotFound(set_id.to_string()))?;
        if let Some(existing) = set.rules.iter_mut().find(|r| r.id == rule.id) {
            *existing = rule.clone();
        } else {
            set.rules.push(rule.clone());
        }
        Ok(rule)
    }

    pub fn remove_rule_from_set(&mut self, set_id: &str, rule_id: &str) -> AppResult<()> {
        let set = self
            .rule_sets
            .iter_mut()
            .find(|s| s.id == set_id)
            .ok_or_else(|| AppError::NotFound(set_id.to_string()))?;
        let before = set.rules.len();
        set.rules.retain(|r| r.id != rule_id);
        if set.rules.len() == before {
            return Err(AppError::NotFound(rule_id.to_string()));
        }
        Ok(())
    }

    pub fn create_rule_set(&mut self, name: &str) -> RuleSet {
        let set = RuleSet::new_user(name, vec![]);
        self.rule_sets.insert(0, set.clone());
        set
    }

    pub fn create_remote_rule_set(
        &mut self,
        name: &str,
        url: &str,
        target: crate::domain::RuleTarget,
        update_interval: &str,
    ) -> RuleSet {
        let mut set = RuleSet::new_remote(name, url, target);
        if let Some(remote) = set.remote.as_mut() {
            remote.update_interval = update_interval.to_string();
        }
        self.rule_sets.insert(0, set.clone());
        set
    }

    pub fn enabled_rule_sets(&self) -> Vec<RuleSet> {
        self.rule_sets
            .iter()
            .filter(|set| set.enabled)
            .cloned()
            .collect()
    }

    /// Reorder rule sets by id list. Unknown ids ignored; missing ids appended at end.
    /// List order = match priority (first set matched first).
    pub fn reorder_rule_sets(&mut self, ordered_ids: &[String]) -> AppResult<()> {
        if ordered_ids.is_empty() {
            return Err(AppError::Config("ordered ids empty".into()));
        }
        let mut by_id: std::collections::HashMap<String, RuleSet> = self
            .rule_sets
            .drain(..)
            .map(|s| (s.id.clone(), s))
            .collect();
        let mut next = Vec::with_capacity(by_id.len());
        for id in ordered_ids {
            if let Some(s) = by_id.remove(id) {
                next.push(s);
            }
        }
        // Keep any sets not mentioned (shouldn't happen) at the end
        for (_, s) in by_id {
            next.push(s);
        }
        if next.is_empty() {
            return Err(AppError::Config("no rule sets after reorder".into()));
        }
        self.rule_sets = next;
        Ok(())
    }

    pub fn delete_rule_set(&mut self, id: &str) -> AppResult<()> {
        let before = self.rule_sets.len();
        self.rule_sets.retain(|set| set.id != id);
        if self.rule_sets.len() == before {
            return Err(AppError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Reload **one** factory set from `resources/rules/{id}.list` (+ optional `.dns.list`).
    /// Preserves `enabled`. Fails if id is not a factory template.
    pub fn reset_rule_set(
        &mut self,
        resource_dir: Option<&Path>,
        set_id: &str,
    ) -> AppResult<RuleSet> {
        if !is_factory_set_id(set_id) {
            return Err(AppError::Config("只能重置内置规则集".into()));
        }
        let template = load_factory_rule_set(resource_dir, set_id)
            .ok_or_else(|| AppError::NotFound(format!("factory template missing: {set_id}")))?;
        if let Some(s) = self.rule_sets.iter_mut().find(|x| x.id == set_id) {
            let was_enabled = s.enabled;
            *s = template;
            s.enabled = was_enabled;
            Ok(s.clone())
        } else {
            let mut inserted = template;
            inserted.enabled = true;
            self.rule_sets.push(inserted.clone());
            Ok(inserted)
        }
    }

    /// Reload all `builtin-*` factory sets from disk (legacy bulk reset).
    pub fn reset_all_builtin_rule_sets(&mut self, resource_dir: Option<&Path>) -> Vec<String> {
        let removed: Vec<String> = self
            .rule_sets
            .iter()
            .filter(|set| set.ownership == RuleSetOwnership::Builtin)
            .map(|set| set.id.clone())
            .collect();
        self.rule_sets
            .retain(|set| set.ownership != RuleSetOwnership::Builtin);
        self.rule_sets.extend(load_builtin_rule_sets(resource_dir));
        removed
    }
}

fn same_rules_ignoring_storage_fields(left: &[Rule], right: &[Rule]) -> bool {
    fn canonical(rules: &[Rule]) -> Vec<String> {
        let mut out: Vec<String> = rules
            .iter()
            .cloned()
            .map(|mut rule| {
                rule.id.clear();
                rule.ord = 0;
                serde_json::to_string(&rule).unwrap_or_default()
            })
            .collect();
        out.sort();
        out
    }
    canonical(left) == canonical(right)
}

pub fn default_store_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("data").join("store.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::RuleType;

    #[test]
    fn unified_migration_splits_mixed_sets_once() {
        let mut store = AppStore::default();
        store.rule_sets.push(RuleSet::new_user(
            "混合",
            vec![
                Rule::new(
                    RuleType::DomainSuffix,
                    "a.test".into(),
                    RuleTarget::Proxy,
                    10,
                ),
                Rule::new(
                    RuleType::DomainSuffix,
                    "b.test".into(),
                    RuleTarget::Direct,
                    20,
                ),
                Rule::new(RuleType::Domain, "c.test".into(), RuleTarget::Smart, 30),
            ],
        ));
        store.migrate_unified_rule_sets();

        assert_eq!(store.schema_version, 3);
        assert!(store
            .rule_sets
            .iter()
            .any(|set| set.strategy == RuleSetStrategy::Proxy));
        assert!(store
            .rule_sets
            .iter()
            .any(|set| set.strategy == RuleSetStrategy::Direct));
        assert!(store
            .rule_sets
            .iter()
            .any(|set| set.strategy == RuleSetStrategy::Smart));
        assert_eq!(
            store
                .rule_sets
                .iter()
                .map(|set| set.rules.len())
                .sum::<usize>(),
            3
        );
        let once = serde_json::to_string(&store.rule_sets).unwrap();
        store.migrate_unified_rule_sets();
        assert_eq!(once, serde_json::to_string(&store.rule_sets).unwrap());
    }

    #[test]
    fn v3_migration_folds_dns_matchers_into_shared_rules() {
        let mut store = AppStore {
            schema_version: 2,
            ..AppStore::default()
        };
        let mut set = RuleSet::new_user("国内解析", Vec::new());
        set.dns_rules.push(crate::domain::DnsRule {
            id: "dns-cn".into(),
            enabled: true,
            matcher: DomainMatcher::DomainSuffix,
            payload: "example.cn".into(),
            action: DnsAction::Domestic,
        });
        store.rule_sets.push(set);

        store.migrate_unified_rule_sets();

        assert_eq!(store.schema_version, 3);
        assert_eq!(
            store.rule_sets[0].dns_strategy,
            RuleSetDnsStrategy::Domestic
        );
        assert!(store.rule_sets[0].dns_rules.is_empty());
        assert_eq!(store.rule_sets[0].rules.len(), 1);
        assert_eq!(store.rule_sets[0].rules[0].payload, "example.cn");
    }

    #[test]
    fn v4_removes_untouched_redundant_general_set() {
        let mut store = AppStore {
            schema_version: 3,
            ..AppStore::default()
        };
        let mut general = RuleSet::new_user(GENERAL_SET_NAME, default_rules());
        general.id = GENERAL_SET_ID.into();
        general.ownership = RuleSetOwnership::Builtin;
        general.strategy = RuleSetStrategy::Direct;
        store.rule_sets.push(general);

        store.migrate_redundant_general_rule_set();

        assert_eq!(store.schema_version, 4);
        assert!(!store.rule_sets.iter().any(|set| set.id == GENERAL_SET_ID));
    }

    #[test]
    fn v4_preserves_edited_general_set_as_user_owned() {
        let mut store = AppStore {
            schema_version: 3,
            ..AppStore::default()
        };
        let mut rules = default_rules();
        rules.push(Rule::new(
            RuleType::DomainSuffix,
            "user.example".into(),
            RuleTarget::Direct,
            100,
        ));
        let mut general = RuleSet::new_user(GENERAL_SET_NAME, rules);
        general.id = GENERAL_SET_ID.into();
        general.ownership = RuleSetOwnership::Builtin;
        general.strategy = RuleSetStrategy::Direct;
        store.rule_sets.push(general);

        store.migrate_redundant_general_rule_set();

        let preserved = store
            .get_rule_set(GENERAL_SET_ID)
            .expect("preserved general");
        assert_eq!(preserved.ownership, RuleSetOwnership::User);
        assert!(!preserved.builtin);
        store.delete_rule_set(GENERAL_SET_ID).unwrap();
        assert!(store.get_rule_set(GENERAL_SET_ID).is_none());
    }

    #[test]
    fn v5_disables_implicit_remote_auto_updates_once() {
        let mut store = AppStore {
            schema_version: 4,
            ..AppStore::default()
        };
        let mut remote = RuleSet::new_remote(
            "旧远程规则",
            "https://example.com/rules.json",
            RuleTarget::Proxy,
        );
        remote.remote.as_mut().unwrap().update_interval = "1h".into();
        store.rule_sets.push(remote);

        store.migrate_remote_update_policy();
        assert_eq!(store.schema_version, 5);
        assert_eq!(
            store.rule_sets[0].remote.as_ref().unwrap().update_interval,
            "disabled"
        );

        store.rule_sets[0].remote.as_mut().unwrap().update_interval = "12h".into();
        store.migrate_remote_update_policy();
        assert_eq!(
            store.rule_sets[0].remote.as_ref().unwrap().update_interval,
            "12h"
        );
    }

    #[test]
    fn deleted_builtin_is_restored_only_when_still_bundled() {
        let mut store = AppStore::default();
        let bundled = load_builtin_rule_sets(None)
            .into_iter()
            .next()
            .expect("bundled rule set");
        let bundled_id = bundled.id.clone();
        store.rule_sets.push(bundled);
        let mut obsolete = RuleSet::new_user("过时内置", Vec::new());
        obsolete.id = "builtin-obsolete".into();
        obsolete.builtin = true;
        obsolete.ownership = RuleSetOwnership::Builtin;
        store.rule_sets.push(obsolete);

        store.delete_rule_set(&bundled_id).unwrap();
        store.delete_rule_set("builtin-obsolete").unwrap();
        store.ensure_rule_sets(None);

        assert!(store.get_rule_set(&bundled_id).is_some());
        assert!(store.get_rule_set("builtin-obsolete").is_none());
    }

    #[test]
    fn reset_all_builtin_preserves_user_sets() {
        let mut store = AppStore::default();
        let user = RuleSet::new_remote(
            "用户远程",
            "https://example.com/rules.json",
            RuleTarget::Proxy,
        );
        let user_id = user.id.clone();
        store.rule_sets.push(user);
        store.rule_sets.push(RuleSet {
            id: "builtin-old".into(),
            name: "旧内置".into(),
            builtin: true,
            enabled: true,
            ownership: RuleSetOwnership::Builtin,
            strategy: RuleSetStrategy::Proxy,
            dns_strategy: RuleSetDnsStrategy::Remote,
            remote: None,
            dns_rules: Vec::new(),
            rules: Vec::new(),
        });

        let removed = store.reset_all_builtin_rule_sets(None);
        assert!(removed.iter().any(|id| id == "builtin-old"));
        assert!(store.rule_sets.iter().any(|set| set.id == user_id));
    }

    #[test]
    fn new_local_and_remote_sets_are_inserted_at_highest_priority() {
        let mut store = AppStore::default();
        store
            .rule_sets
            .push(RuleSet::new_user("已有规则", Vec::new()));

        let local = store.create_rule_set("新本地");
        assert_eq!(store.rule_sets[0].id, local.id);

        let remote = store.create_remote_rule_set(
            "新远程",
            "https://example.com/rules.json",
            RuleTarget::Proxy,
            "1h",
        );
        assert_eq!(store.rule_sets[0].id, remote.id);
        assert_eq!(store.rule_sets[1].id, local.id);
    }
}

use crate::domain::{
    default_rules, ensure_bundled_dns_whitelist, is_deletable_rule_set, is_factory_set_id,
    load_builtin_rule_sets,
    load_factory_rule_set, sanitize_rules, AppSettings, DnsSettings, ProxyNode, Rule, RuleSet,
    RuleSetSummary, Subscription, BUILTIN_SET_ID, BUILTIN_SET_NAME, GENERAL_SET_ID,
    GENERAL_SET_NAME,
};
use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppStore {
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
        store.ensure_rule_sets(resource_dir);
        store.ensure_dns_defaults(resource_dir);
        store.ensure_subscription_enable_policy();
        // Persist migrations (new rule files / DNS whitelist) so they survive read-only sessions.
        let _ = store.save(path);
        Ok(store)
    }

    fn with_builtin_sets(resource_dir: Option<&Path>) -> Self {
        let mut s = Self::default();
        s.ensure_rule_sets(resource_dir);
        s.ensure_dns_defaults(resource_dir);
        s
    }

    /// Inject bundled DNS whitelist from `resources/dns/*.list`.
    pub fn ensure_dns_defaults(&mut self, resource_dir: Option<&Path>) {
        ensure_bundled_dns_whitelist(&mut self.dns, resource_dir);
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
            if set.id == "migrated-legacy"
                || set.name == "我的规则（迁移）"
                || set.name == "自定义"
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
                .filter(|(_, s)| is_factory_set_id(&s.id) && factory_ids.iter().any(|id| id == &s.id))
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
                    rules: legacy,
                });
            }
            self.rules.clear();
        }

        // Ensure 通用规则 exists (file seed, or hardcoded fallback).
        if !self.rule_sets.iter().any(|s| s.id == GENERAL_SET_ID) {
            let mut general = load_factory_rule_set(resource_dir, GENERAL_SET_ID)
                .unwrap_or_else(|| {
                    let mut g = RuleSet::new_user(GENERAL_SET_NAME, default_rules());
                    g.id = GENERAL_SET_ID.into();
                    g
                });
            general.id = GENERAL_SET_ID.into();
            general.name = GENERAL_SET_NAME.into();
            general.builtin = false;
            general.enabled = true;
            self.rule_sets.push(general);
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
            let still = self
                .nodes
                .iter()
                .any(|n| &n.node.id == cur && {
                    self.subscriptions
                        .iter()
                        .any(|s| s.enabled && s.id == n.subscription_id)
                });
            if !still {
                self.settings.current_node_id =
                    self.enabled_nodes().first().map(|n| n.id.clone());
            }
        }
    }

    /// New subscription: enable only when no other is enabled (or none exist).
    pub fn prepare_new_subscription_enabled(&self, sub: &mut Subscription) {
        let any_enabled = self.subscriptions.iter().any(|s| s.enabled && s.id != sub.id);
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
        if out.is_empty() {
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
                rule_count: s.rules.len() as u32,
                enabled: s.enabled,
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
        self.rule_sets.push(set.clone());
        set
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
        let set = self
            .rule_sets
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::NotFound(id.to_string()))?;
        if !is_deletable_rule_set(id, set.builtin) {
            return Err(AppError::Config(
                "不能删除出厂规则集（内置/通用）；可重置为资源文件默认".into(),
            ));
        }
        self.rule_sets.retain(|s| s.id != id);
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
            return Err(AppError::Config(
                "只能重置出厂规则集（内置/通用）".into(),
            ));
        }
        let template = load_factory_rule_set(resource_dir, set_id).ok_or_else(|| {
            AppError::NotFound(format!("factory template missing: {set_id}"))
        })?;
        if let Some(s) = self.rule_sets.iter_mut().find(|x| x.id == set_id) {
            let was_enabled = s.enabled;
            *s = template;
            s.enabled = was_enabled;
            if set_id == GENERAL_SET_ID {
                s.builtin = false;
                s.name = GENERAL_SET_NAME.into();
            }
            Ok(s.clone())
        } else {
            let mut inserted = template;
            if set_id == GENERAL_SET_ID {
                inserted.builtin = false;
                inserted.name = GENERAL_SET_NAME.into();
            }
            inserted.enabled = true;
            self.rule_sets.push(inserted.clone());
            Ok(inserted)
        }
    }

    /// Reload all `builtin-*` factory sets from disk (legacy bulk reset).
    pub fn reset_all_builtin_rule_sets(&mut self, resource_dir: Option<&Path>) {
        let ids: Vec<String> = load_builtin_rule_sets(resource_dir)
            .into_iter()
            .filter(|s| s.builtin)
            .map(|s| s.id)
            .collect();
        for id in ids {
            let _ = self.reset_rule_set(resource_dir, &id);
        }
        self.ensure_dns_defaults(resource_dir);
    }
}

pub fn default_store_path(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("data").join("store.json")
}

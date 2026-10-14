//! User-editable subconverter INI policy; subscription credentials stay separate.
use crate::domain::{MihomoTemplate, ProxyNode};
use regex::RegexBuilder;
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub const TEMPLATE_KEY: &str = "satelite-user-template";

fn text(s: &str) -> Value {
    Value::String(s.into())
}

pub fn encode_template(template: &MihomoTemplate) -> String {
    let mut root = Mapping::new();
    root.insert(
        text(TEMPLATE_KEY),
        serde_yaml::to_value(template).unwrap_or_default(),
    );
    serde_yaml::to_string(&root).unwrap_or_default()
}

pub fn template_from_configs(configs: &[String]) -> Result<Option<MihomoTemplate>, String> {
    for config in configs {
        if let Ok(root) = serde_yaml::from_str::<Value>(config) {
            if let Some(value) = root.get(TEMPLATE_KEY) {
                let template = serde_yaml::from_value(value.clone())
                    .map_err(|_| "自定义 Mihomo 模板数据无效")?;
                return Ok(Some(template));
            }
        }
    }
    Ok(None)
}

pub fn validate_overrides(raw: &str) -> Result<Mapping, String> {
    if raw.len() > 256 * 1024 {
        return Err("Mihomo 覆写超过大小限制".into());
    }
    let root: Value = serde_yaml::from_str(raw).map_err(|_| "Mihomo 覆写不是有效 YAML")?;
    let map = root
        .as_mapping()
        .cloned()
        .ok_or("Mihomo 覆写必须是 YAML 对象")?;
    for key in map.keys() {
        if !matches!(
            key.as_str(),
            Some("dns" | "allow-lan" | "log-level" | "ipv6")
        ) {
            return Err("基础覆写仅允许 dns、allow-lan、log-level、ipv6；2080 入口、模式、TUN 和控制接口由应用管理".into());
        }
    }
    for key in ["allow-lan", "ipv6"] {
        if map.get(text(key)).is_some_and(|v| v.as_bool().is_none()) {
            return Err(format!("{key} 必须是布尔值"));
        }
    }
    if let Some(dns) = map.get(text("dns")) {
        if !dns.is_mapping() {
            return Err("dns 覆写必须是对象".into());
        }
        if let Some(listen) = dns.get("listen").and_then(Value::as_str) {
            let address: std::net::SocketAddr =
                listen.parse().map_err(|_| "DNS listen 必须是 IP:端口")?;
            if address.port() == 0 {
                return Err("DNS listen 端口不能为 0".into());
            }
        }
    }
    Ok(map)
}

pub fn apply_overrides(root: &mut Mapping, raw: &str, mixed: u16, api: u16) -> Result<(), String> {
    let overrides = validate_overrides(raw)?;
    if let Some(port) = overrides
        .get(text("dns"))
        .and_then(|dns| dns.get("listen"))
        .and_then(Value::as_str)
        .and_then(|listen| listen.parse::<std::net::SocketAddr>().ok())
        .map(|a| a.port())
    {
        if port == mixed || port == api {
            return Err("Mihomo DNS 监听端口与代理或控制端口冲突".into());
        }
    }
    for (key, value) in overrides {
        // DNS module overwrite is deliberate: no source-subscription DNS or
        // hidden nameserver-policy may override the user's explicit template.
        root.insert(key, value);
    }
    if root.get(text("allow-lan")).and_then(Value::as_bool) == Some(true) {
        root.insert(text("bind-address"), text("*"));
    } else {
        root.remove(text("bind-address"));
    }
    Ok(())
}

pub fn render_template(template: &MihomoTemplate, nodes: &[ProxyNode]) -> Result<String, String> {
    validate_overrides(&template.override_yaml)?;
    if template.ini.len() > 512 * 1024 {
        return Err("INI 模板超过大小限制".into());
    }
    let mut groups: Vec<Vec<&str>> = Vec::new();
    let mut rules = Vec::new();
    let mut in_custom = false;
    for line in template.ini.lines() {
        let line = line.trim();
        if line.starts_with(';') || line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            in_custom = line == "[custom]";
            continue;
        }
        if !in_custom {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err("INI 模板存在无效行".into());
        };
        match key.trim() {
            "custom_proxy_group" => groups.push(value.trim().split('`').collect()),
            "ruleset" => rules.push(value.trim()),
            "enable_rule_generator" | "overwrite_original_rules" if value.trim() != "true" => {
                return Err("自定义模板必须启用规则生成及规则覆写".into())
            }
            _ => {}
        }
    }
    if groups.is_empty() || groups.len() > 128 || rules.len() > 1024 {
        return Err("INI 策略组或规则数量无效".into());
    }
    let mut names = HashSet::new();
    for fields in &groups {
        if fields.len() < 3
            || fields[0].trim().is_empty()
            || fields[0].contains(',')
            || !names.insert(fields[0])
            || ["DIRECT", "REJECT", "GLOBAL", "Satelite-DIRECT"].contains(&fields[0])
        {
            return Err("INI 策略组名称重复、保留名称或定义无效".into());
        }
    }
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut emitted = Vec::new();
    for fields in &groups {
        let kind = fields[1];
        if !["select", "url-test", "fallback", "load-balance"].contains(&kind) {
            return Err("INI 策略组类型不支持".into());
        }
        let automatic = kind != "select";
        let end = if automatic {
            if fields.len() < 5 {
                return Err("自动策略组缺少探测地址或间隔".into());
            }
            url::Url::parse(fields[fields.len() - 2])
                .ok()
                .filter(|u| ["http", "https"].contains(&u.scheme()))
                .ok_or("策略组探测 URL 无效")?;
            fields.len() - 2
        } else {
            fields.len()
        };
        let mut members = Vec::new();
        for expr in &fields[2..end] {
            if let Some(reference) = expr.strip_prefix("[]") {
                if !names.contains(reference) && !["DIRECT", "REJECT"].contains(&reference) {
                    return Err("INI 策略组引用了不存在的组".into());
                }
                if names.contains(reference) {
                    dependencies.entry(fields[0]).or_default().push(reference);
                }
                members.push(reference.to_string());
            } else {
                if expr.len() > 2048 {
                    return Err("节点过滤表达式过长".into());
                }
                let regex = RegexBuilder::new(expr)
                    .size_limit(2 * 1024 * 1024)
                    .build()
                    .map_err(|_| "INI 节点过滤正则无效或不支持")?;
                members.extend(
                    nodes
                        .iter()
                        .filter(|n| regex.is_match(&n.name))
                        .map(crate::config::outbound_tag),
                );
            }
        }
        let mut unique = HashSet::new();
        members.retain(|name| unique.insert(name.clone()));
        if members.is_empty() {
            members.push("REJECT".into());
        }
        let mut group = Mapping::new();
        group.insert(text("name"), text(fields[0]));
        group.insert(text("type"), text(kind));
        group.insert(
            text("proxies"),
            Value::Sequence(members.iter().map(|s| text(s)).collect()),
        );
        if automatic {
            let options: Vec<_> = fields[fields.len() - 1].split(',').collect();
            let interval = options[0]
                .parse::<u64>()
                .ok()
                .filter(|v| *v > 0 && *v <= 86400)
                .ok_or("策略组探测间隔无效")?;
            group.insert(text("url"), text(fields[fields.len() - 2]));
            group.insert(text("interval"), Value::Number(interval.into()));
            if let Some(timeout) = options.get(1) {
                let seconds = timeout
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0 && *n <= 60)
                    .ok_or("策略组探测超时无效")?;
                group.insert(text("timeout"), Value::Number((seconds * 1000).into()));
            }
            if let Some(tolerance) = options.get(2) {
                group.insert(
                    text("tolerance"),
                    Value::Number(
                        tolerance
                            .parse::<u64>()
                            .map_err(|_| "策略组容差无效")?
                            .into(),
                    ),
                );
            }
        }
        emitted.push(Value::Mapping(group));
    }
    fn visit<'a>(
        name: &'a str,
        graph: &HashMap<&'a str, Vec<&'a str>>,
        active: &mut HashSet<&'a str>,
        done: &mut HashSet<&'a str>,
    ) -> Result<(), String> {
        if done.contains(name) {
            return Ok(());
        }
        if !active.insert(name) {
            return Err("INI 策略组存在循环引用".into());
        }
        for child in graph.get(name).into_iter().flatten() {
            visit(child, graph, active, done)?;
        }
        active.remove(name);
        done.insert(name);
        Ok(())
    }
    let mut done = HashSet::new();
    for name in &names {
        visit(name, &dependencies, &mut HashSet::new(), &mut done)?;
    }
    let mut providers = Mapping::new();
    let mut route_rules = Vec::new();
    for rule in rules {
        let (target, source) = rule.split_once(',').ok_or("ruleset 缺少出口或来源")?;
        if !names.contains(target) && !["DIRECT", "REJECT"].contains(&target) {
            return Err("ruleset 引用了不存在的策略组".into());
        }
        if let Some(inline) = source.strip_prefix("[]") {
            let parts: Vec<_> = inline.split(',').collect();
            match parts.as_slice() {
                ["FINAL"] | ["MATCH"] => route_rules.push(format!("MATCH,{target}")),
                ["GEOIP" | "GEOSITE" | "DOMAIN" | "DOMAIN-SUFFIX" | "DOMAIN-KEYWORD" | "IP-CIDR"
                | "IP-CIDR6" | "PROCESS-NAME" | "PROCESS-PATH" | "DST-PORT" | "SRC-IP-CIDR", value]
                    if !value.is_empty() =>
                {
                    route_rules.push(format!("{},{value},{target}", parts[0]))
                }
                _ => return Err("不支持该 INI 内联规则类型".into()),
            }
        } else {
            let url = url::Url::parse(source).map_err(|_| "规则集 URL 无效")?;
            if !["http", "https"].contains(&url.scheme())
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("规则集必须是 HTTP/HTTPS URL".into());
            }
            let hash = hex::encode(Sha256::digest(source.as_bytes()));
            let tag = format!("satelite-template-{}", &hash[..16]);
            let provider = serde_yaml::to_value(serde_json::json!({"type":"http", "behavior":"classical", "format":"text", "url":source, "path":format!("./rule-providers/{tag}.list"), "interval":86400})).map_err(|_| "规则集生成失败")?;
            providers.insert(text(&tag), provider);
            route_rules.push(format!("RULE-SET,{tag},{target}"));
        }
    }
    if !route_rules.iter().any(|r| r.starts_with("MATCH,")) {
        return Err("INI 模板缺少 FINAL/MATCH 兜底规则".into());
    }
    if !route_rules.last().is_some_and(|r| r.starts_with("MATCH,"))
        || route_rules
            .iter()
            .filter(|r| r.starts_with("MATCH,"))
            .count()
            != 1
    {
        return Err("FINAL/MATCH 必须唯一且位于规则末尾".into());
    }
    let mut root = Mapping::new();
    root.insert(text("proxy-groups"), Value::Sequence(emitted));
    root.insert(
        text("rules"),
        Value::Sequence(route_rules.iter().map(|s| text(s)).collect()),
    );
    root.insert(text("rule-providers"), Value::Mapping(providers));
    root.insert(text("satelite-template-policy"), Value::Bool(true));
    serde_yaml::to_string(&root).map_err(|_| "模板生成失败".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_keeps_real_groups_and_rule_order() {
        let template = MihomoTemplate::default();
        let doc: Value = serde_yaml::from_str(&render_template(&template, &[]).unwrap()).unwrap();
        assert_eq!(doc["proxy-groups"].as_sequence().unwrap().len(), 18);
        assert_eq!(doc["proxy-groups"][0]["name"].as_str(), Some("🚀 节点选择"));
        assert_eq!(doc["proxy-groups"][2]["interval"].as_u64(), Some(300));
        assert_eq!(doc["proxy-groups"][2]["timeout"].as_u64(), Some(5000));
        assert_eq!(doc["proxy-groups"][2]["tolerance"].as_u64(), Some(100));
        let rules = doc["rules"].as_sequence().unwrap();
        assert!(rules[0].as_str().unwrap().ends_with(",🔒 局域网"));
        assert_eq!(rules.last().unwrap().as_str(), Some("MATCH,🐟 其他域名"));
        assert!(doc["rule-providers"]
            .as_mapping()
            .unwrap()
            .values()
            .all(|p| p["format"].as_str() == Some("text")));
    }

    #[test]
    fn invalid_templates_and_unsafe_controller_overrides_are_rejected() {
        for ini in [
            "[custom]\ncustom_proxy_group=A`select`[]A\nruleset=A,[]FINAL",
            "[custom]\ncustom_proxy_group=A`select`[]missing\nruleset=A,[]FINAL",
            "[custom]\ncustom_proxy_group=A`select`(bad\nruleset=A,[]FINAL",
            "[custom]\ncustom_proxy_group=A`select`.*\nruleset=A,file:///private",
            "[custom]\ncustom_proxy_group=A`select`.*\nruleset=A,[]FINAL\nruleset=A,[]DOMAIN,test",
        ] {
            let mut template = MihomoTemplate::default();
            template.ini = ini.into();
            assert!(render_template(&template, &[]).is_err());
        }
        assert!(validate_overrides("external-controller: 0.0.0.0:9090").is_err());
        assert!(validate_overrides("mixed-port: 7890").is_err());
        assert!(apply_overrides(
            &mut Mapping::new(),
            "dns: {listen: '127.0.0.1:2080'}",
            2080,
            19090
        )
        .is_err());
    }
}

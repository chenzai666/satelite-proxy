//! Subscription body → normalized [`ProxyNode`] list.

mod clash;
mod json_util;
mod manual;
mod singbox;
mod uri;
mod yaml_util;

pub use clash::parse_clash_yaml;
pub use manual::{draft_to_node, node_to_draft, parse_manual_draft, parse_single_uri};
pub use singbox::{looks_like_singbox_json, parse_singbox_json, validate_complete_singbox_config};
pub use uri::{parse_uri_list, serialize_share_uri};

use crate::domain::{CustomConfigKind, ParseResult, SubscriptionFormat};
use crate::error::{AppError, AppResult};
use base64::{engine::general_purpose, Engine as _};

/// Protect parsing, persistence, config generation, and UI rendering from
/// pathological subscription payloads while remaining well above normal use.
pub(super) const MAX_SUBSCRIPTION_ENTRIES: usize = 10_000;

pub(super) fn ensure_entry_limit(count: usize) -> AppResult<()> {
    if count > MAX_SUBSCRIPTION_ENTRIES {
        return Err(AppError::SubscriptionTooLarge {
            max: MAX_SUBSCRIPTION_ENTRIES,
        });
    }
    Ok(())
}

/// Detect which kernel a complete custom config ("自定义配置") targets.
///
/// sing-box JSON and Xray JSON both carry `inbounds`/`outbounds` arrays — the
/// discriminator is the entry shape: sing-box entries have `type`, Xray
/// entries have `protocol`. mihomo is Clash YAML with any well-known top-level
/// key (`proxies` / `mixed-port` / …).
pub fn detect_custom_config_kind(content: &str) -> AppResult<CustomConfigKind> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::EmptySubscription);
    }

    if looks_like_json(trimmed) {
        let value: serde_json::Value = serde_json::from_str(trimmed)
            .map_err(|e| AppError::SubscriptionParse(format!("invalid json: {e}")))?;
        let obj = value.as_object().ok_or_else(|| {
            AppError::SubscriptionParse(
                "自定义配置必须是完整配置对象（sing-box / Xray JSON 或 mihomo YAML）".into(),
            )
        })?;
        let entries = obj
            .get("outbounds")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .chain(
                obj.get("inbounds")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten(),
            );
        let mut has_type = false;
        let mut has_protocol = false;
        for entry in entries {
            let Some(map) = entry.as_object() else {
                continue;
            };
            if map.get("type").and_then(|v| v.as_str()).is_some() {
                has_type = true;
            }
            if map.get("protocol").and_then(|v| v.as_str()).is_some() {
                has_protocol = true;
            }
        }
        if has_protocol && !has_type {
            return Ok(CustomConfigKind::Xray);
        }
        if has_type && !has_protocol {
            return Ok(CustomConfigKind::Singbox);
        }
        return Err(AppError::SubscriptionParse(
            "无法识别 JSON 配置类型：sing-box 条目需含 type 字段，Xray 条目需含 protocol 字段（mihomo 配置请使用 YAML）"
                .into(),
        ));
    }

    if looks_like_clash_yaml(trimmed) {
        return Ok(CustomConfigKind::Mihomo);
    }

    Err(AppError::SubscriptionParse(
        "无法识别配置类型：支持 sing-box JSON（outbounds 含 type）、Xray JSON（outbounds 含 protocol）、mihomo Clash YAML（含 proxies / mixed-port 等）"
            .into(),
    ))
}

/// mihomo custom config: must parse as a YAML mapping. Detection already
/// required a known Clash key; deeper validation is mihomo's own `-t` check
/// at start time.
pub fn validate_custom_mihomo_config(content: &str) -> AppResult<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::EmptySubscription);
    }
    let value: serde_yaml::Value = serde_yaml::from_str(trimmed)
        .map_err(|e| AppError::SubscriptionParse(format!("mihomo 配置必须是合法 YAML：{e}")))?;
    if value.as_mapping().is_none() {
        return Err(AppError::SubscriptionParse(
            "mihomo 配置必须是 YAML 键值映射，不能是列表或片段".into(),
        ));
    }
    Ok(trimmed.to_string())
}

/// Xray custom config: JSON object with a non-empty `outbounds` array
/// (`inbounds` optional — API-only configs exist).
pub fn validate_custom_xray_config(content: &str) -> AppResult<String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::EmptySubscription);
    }
    let value: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|e| AppError::SubscriptionParse(format!("Xray 配置必须是合法 JSON：{e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| AppError::SubscriptionParse("Xray 配置必须是 JSON 对象".into()))?;
    let outbounds = obj
        .get("outbounds")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AppError::SubscriptionParse("Xray 配置必须包含 outbounds 数组".into()))?;
    if outbounds.is_empty() {
        return Err(AppError::SubscriptionParse(
            "Xray 配置的 outbounds 不能为空".into(),
        ));
    }
    serde_json::to_string_pretty(&value)
        .map_err(|e| AppError::SubscriptionParse(format!("serialize xray config: {e}")))
}

/// Detect format and parse subscription / config body
/// (sing-box JSON, Clash YAML/JSON, URI list, base64 URI list).
pub fn parse_subscription(content: &str) -> AppResult<ParseResult> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::EmptySubscription);
    }

    // 1) JSON: sing-box config / outbounds, or Clash-as-JSON
    if looks_like_json(trimmed) {
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(value) => {
                if looks_like_singbox_json(&value) {
                    match parse_singbox_json(trimmed) {
                        Ok(r) => return Ok(r),
                        Err(e) => {
                            if value.get("outbounds").is_some() {
                                return Err(e);
                            }
                        }
                    }
                }
                if value.get("proxies").is_some() {
                    match parse_clash_from_json(&value) {
                        Ok(r) => return Ok(r),
                        Err(e) => return Err(e),
                    }
                }
                if looks_like_singbox_json(&value) {
                    return parse_singbox_json(trimmed);
                }
            }
            Err(e) => {
                return Err(AppError::SubscriptionParse(format!("invalid json: {e}")));
            }
        }
    }

    // 2) Looks like Clash YAML
    if looks_like_clash_yaml(trimmed) {
        match parse_clash_yaml(trimmed) {
            Ok(r) => return Ok(r),
            Err(e) => {
                // If it strongly looks like yaml with proxies, don't silently fall through.
                if trimmed.contains("proxies:") || trimmed.contains("proxies :") {
                    return Err(e);
                }
            }
        }
    }

    // 3) Plain URI lines
    if looks_like_uri_list(trimmed) {
        return parse_uri_list(trimmed, SubscriptionFormat::UriList);
    }

    // 4) Whole-body base64 → decode → recurse-ish
    if let Some(decoded) = try_decode_base64_body(trimmed) {
        let inner = decoded.trim();
        if looks_like_json(inner) {
            if let Ok(r) = parse_subscription(inner) {
                return Ok(r);
            }
        }
        if looks_like_clash_yaml(inner) {
            if let Ok(r) = parse_clash_yaml(inner) {
                return Ok(r);
            }
        }
        if looks_like_uri_list(inner) || inner.lines().any(|l| is_proxy_uri(l.trim())) {
            return parse_uri_list(inner, SubscriptionFormat::Base64UriList);
        }
        // Some providers base64 a single long line of URIs joined by newline after decode.
        if inner.contains("://") {
            return parse_uri_list(inner, SubscriptionFormat::Base64UriList);
        }
    }

    // 5) Last attempt: yaml without strong heuristic
    if let Ok(r) = parse_clash_yaml(trimmed) {
        return Ok(r);
    }

    // 6) Last attempt: treat as URI list
    if let Ok(r) = parse_uri_list(trimmed, SubscriptionFormat::UriList) {
        return Ok(r);
    }

    Err(AppError::SubscriptionParse(
        "unable to detect format (expected sing-box JSON, Clash YAML/JSON, or proxy URI list)"
            .into(),
    ))
}

fn looks_like_json(s: &str) -> bool {
    let t = s.trim_start();
    t.starts_with('{') || t.starts_with('[')
}

fn parse_clash_from_json(value: &serde_json::Value) -> AppResult<ParseResult> {
    let yaml = serde_yaml::to_string(value)
        .map_err(|e| AppError::SubscriptionParse(format!("clash json: {e}")))?;
    parse_clash_yaml(&yaml)
}

fn looks_like_clash_yaml(s: &str) -> bool {
    let head: String = s.chars().take(400).collect();
    head.contains("proxies:")
        || head.contains("proxies :")
        || head.contains("proxy-groups:")
        || head.contains("mixed-port:")
        || head.contains("port:") && (head.contains("socks-port:") || head.contains("allow-lan:"))
}

fn looks_like_uri_list(s: &str) -> bool {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .take(5)
        .any(is_proxy_uri)
}

fn is_proxy_uri(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("ss://")
        || lower.starts_with("vmess://")
        || lower.starts_with("vless://")
        || lower.starts_with("trojan://")
        || lower.starts_with("hysteria2://")
        || lower.starts_with("hy2://")
        || lower.starts_with("tuic://")
        || lower.starts_with("socks://")
        || lower.starts_with("socks5://")
        || lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("hysteria://")
        || lower.starts_with("hy://")
        || lower.starts_with("shadowtls://")
        || lower.starts_with("ssh://")
        || lower.starts_with("naive://")
        || lower.starts_with("naive+https://")
        || lower.starts_with("naive+quic://")
        || lower.starts_with("tor://")
        || lower.starts_with("anytls://")
        || lower.starts_with("snell://")
}

fn try_decode_base64_body(s: &str) -> Option<String> {
    // Avoid treating yaml as base64.
    if s.contains(':') && s.contains('\n') && s.lines().count() > 3 {
        // multi-line with colons → likely yaml/text
        if s.contains("proxies") || s.contains("://") {
            return None;
        }
    }

    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() < 16 {
        return None;
    }
    // Heuristic: base64 alphabet
    if !cleaned.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
    }) {
        return None;
    }

    let bytes = general_purpose::STANDARD
        .decode(&cleaned)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(&cleaned))
        .or_else(|_| general_purpose::URL_SAFE.decode(&cleaned))
        .or_else(|_| general_purpose::URL_SAFE_NO_PAD.decode(&cleaned))
        .ok()?;

    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_subscription_entry_count_above_limit() {
        assert!(ensure_entry_limit(MAX_SUBSCRIPTION_ENTRIES).is_ok());
        assert!(matches!(
            ensure_entry_limit(MAX_SUBSCRIPTION_ENTRIES + 1),
            Err(AppError::SubscriptionTooLarge { .. })
        ));
    }
    use base64::Engine;

    #[test]
    fn detect_clash() {
        let yaml = r#"
proxies:
  - name: a
    type: ss
    server: a.com
    port: 1
    cipher: aes-256-gcm
    password: x
"#;
        let r = parse_subscription(yaml).unwrap();
        assert_eq!(r.format, SubscriptionFormat::ClashYaml);
        assert_eq!(r.nodes.len(), 1);
    }

    #[test]
    fn detect_base64_uri_list() {
        let plain = "trojan://pwd@host.example:443?sni=host.example#T1\nss://aes-256-gcm:pwd@1.1.1.1:8388#S1\n";
        let b64 = general_purpose::STANDARD.encode(plain);
        let r = parse_subscription(&b64).unwrap();
        assert_eq!(r.format, SubscriptionFormat::Base64UriList);
        assert_eq!(r.nodes.len(), 2);
    }

    #[test]
    fn detect_plain_uri() {
        let plain = "vless://11111111-1111-1111-1111-111111111111@v.example.com:443?security=tls&type=tcp#V1\n";
        let r = parse_subscription(plain).unwrap();
        assert_eq!(r.format, SubscriptionFormat::UriList);
        assert_eq!(r.nodes[0].name, "V1");
    }

    #[test]
    fn detect_singbox_outbounds() {
        let json = r#"{"outbounds":[{"type":"trojan","tag":"T1","server":"t.example.com","server_port":443,"password":"x"}]}"#;
        let r = parse_subscription(json).unwrap();
        assert_eq!(r.format, SubscriptionFormat::SingboxJson);
        assert_eq!(r.nodes[0].name, "T1");
    }

    #[test]
    fn detect_custom_kind_discriminates_json_shapes() {
        let singbox = r#"{"inbounds":[{"type":"mixed","listen_port":7890}],"outbounds":[{"type":"direct","tag":"direct"}]}"#;
        assert_eq!(
            detect_custom_config_kind(singbox).unwrap(),
            CustomConfigKind::Singbox
        );
        let xray = r#"{"inbounds":[{"protocol":"socks","port":10808}],"outbounds":[{"protocol":"freedom","tag":"direct"}]}"#;
        assert_eq!(
            detect_custom_config_kind(xray).unwrap(),
            CustomConfigKind::Xray
        );
        let clash = "mixed-port: 7890\nproxies:\n  - name: a\n    type: ss\n    server: a.com\n    port: 1\n    cipher: aes-256-gcm\n    password: x\n";
        assert_eq!(
            detect_custom_config_kind(clash).unwrap(),
            CustomConfigKind::Mihomo
        );
        assert!(detect_custom_config_kind("hello world").is_err());
        assert!(detect_custom_config_kind("{\"dns\":{\"servers\":[]}}").is_err());
    }

    #[test]
    fn validate_custom_configs_reject_incomplete_bodies() {
        assert!(validate_custom_mihomo_config("- a\n- b").is_err());
        assert!(validate_custom_mihomo_config("mixed-port: 7890").is_ok());
        assert!(validate_custom_xray_config(r#"{"outbounds":[]}"#).is_err());
        assert!(validate_custom_xray_config(r#"{"outbounds":[{"protocol":"freedom"}]}"#).is_ok());
    }
}

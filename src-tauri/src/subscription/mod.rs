//! Subscription body → normalized [`ProxyNode`] list.

mod clash;
mod uri;
mod yaml_util;

pub use clash::parse_clash_yaml;
pub use uri::parse_uri_list;

use crate::domain::{ParseResult, SubscriptionFormat};
use crate::error::{AppError, AppResult};
use base64::{engine::general_purpose, Engine as _};

/// Detect format and parse subscription body (YAML / URI list / base64 URI list).
pub fn parse_subscription(content: &str) -> AppResult<ParseResult> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AppError::EmptySubscription);
    }

    // 1) Looks like Clash YAML
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

    // 2) Plain URI lines
    if looks_like_uri_list(trimmed) {
        return parse_uri_list(trimmed, SubscriptionFormat::UriList);
    }

    // 3) Whole-body base64 → decode → recurse-ish
    if let Some(decoded) = try_decode_base64_body(trimmed) {
        let inner = decoded.trim();
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

    // 4) Last attempt: yaml without strong heuristic
    if let Ok(r) = parse_clash_yaml(trimmed) {
        return Ok(r);
    }

    // 5) Last attempt: treat as URI list
    if let Ok(r) = parse_uri_list(trimmed, SubscriptionFormat::UriList) {
        return Ok(r);
    }

    Err(AppError::SubscriptionParse(
        "unable to detect subscription format (expected Clash YAML or proxy URI list)".into(),
    ))
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
}

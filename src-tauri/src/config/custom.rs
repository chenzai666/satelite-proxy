//! Read-only inspection of a user sing-box JSON. Never mutates the document.

use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomConfigInsight {
    pub clash_api_host: Option<String>,
    pub clash_api_port: Option<u16>,
    pub clash_api_secret: Option<String>,
    pub inbound_port: Option<u16>,
    pub has_tun: bool,
}

impl CustomConfigInsight {
    pub fn has_clash_api(&self) -> bool {
        self.clash_api_port.is_some()
    }
}

/// Probe listen / API / TUN from a complete sing-box document. The input is
/// never rewritten.
pub fn inspect_singbox_config(content: &str) -> CustomConfigInsight {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return CustomConfigInsight::default();
    };
    let Some(obj) = value.as_object() else {
        return CustomConfigInsight::default();
    };

    let mut insight = CustomConfigInsight::default();
    if let Some(api) = obj
        .get("experimental")
        .and_then(Value::as_object)
        .and_then(|exp| exp.get("clash_api"))
        .and_then(Value::as_object)
    {
        if let Some(controller) = api
            .get("external_controller")
            .and_then(Value::as_str)
            .or_else(|| api.get("listen").and_then(Value::as_str))
        {
            if let Some((host, port)) = split_host_port(controller) {
                insight.clash_api_host = Some(host);
                insight.clash_api_port = Some(port);
            }
        }
        insight.clash_api_secret = api
            .get("secret")
            .and_then(Value::as_str)
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());
    }

    if let Some(inbounds) = obj.get("inbounds").and_then(Value::as_array) {
        for inbound in inbounds {
            let Some(map) = inbound.as_object() else {
                continue;
            };
            let ty = map
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if ty == "tun" {
                insight.has_tun = true;
                continue;
            }
            if insight.inbound_port.is_some() {
                continue;
            }
            if matches!(ty.as_str(), "mixed" | "http" | "socks" | "socks5") {
                if let Some(port) = json_u16(map.get("listen_port")).or_else(|| {
                    map.get("listen")
                        .and_then(Value::as_str)
                        .and_then(|listen| split_host_port(listen).map(|(_, p)| p))
                }) {
                    insight.inbound_port = Some(port);
                }
            }
        }
    }

    insight
}

fn json_u16(value: Option<&Value>) -> Option<u16> {
    match value? {
        Value::Number(n) => n.as_u64().and_then(|v| u16::try_from(v).ok()),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn split_host_port(raw: &str) -> Option<(String, u16)> {
    let raw = raw.trim().trim_start_matches("http://").trim_start_matches("https://");
    if let Some(rest) = raw.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = tail.trim_start_matches(':').parse().ok()?;
        return Some((host.to_string(), port));
    }
    let (host, port) = raw.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let host = if host.is_empty() {
        "127.0.0.1".into()
    } else {
        host.to_string()
    };
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_mixed_and_clash_api() {
        let json = r#"{
          "inbounds": [{"type":"mixed","listen":"127.0.0.1","listen_port":2080}],
          "outbounds": [{"type":"direct","tag":"direct"}],
          "experimental": {"clash_api":{"external_controller":"127.0.0.1:19090","secret":"s"}}
        }"#;
        let insight = inspect_singbox_config(json);
        assert_eq!(insight.inbound_port, Some(2080));
        assert_eq!(insight.clash_api_port, Some(19090));
        assert_eq!(insight.clash_api_secret.as_deref(), Some("s"));
        assert!(!insight.has_tun);
    }

    #[test]
    fn inspects_tun_without_rewriting() {
        let json = r#"{
          "inbounds": [{"type":"tun","tag":"tun-in","address":["172.19.0.1/30"]}],
          "outbounds": [{"type":"direct","tag":"direct"}]
        }"#;
        let insight = inspect_singbox_config(json);
        assert!(insight.has_tun);
        assert!(insight.inbound_port.is_none());
        assert!(!insight.has_clash_api());
    }
}

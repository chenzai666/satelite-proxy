//! Read-only inspection of a user's complete custom config (sing-box JSON /
//! mihomo Clash YAML / Xray JSON). Never mutates the document.

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
            if matches!(ty.as_str(), "mixed" | "http") {
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

/// Probe listen / API / TUN from a complete mihomo (Clash) YAML document.
pub fn inspect_mihomo_config(content: &str) -> CustomConfigInsight {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return CustomConfigInsight::default();
    };
    let Some(map) = value.as_mapping() else {
        return CustomConfigInsight::default();
    };

    let get = |key: &str| map.get(serde_yaml::Value::String(key.into()));
    let yaml_u16 = |v: Option<&serde_yaml::Value>| -> Option<u16> {
        v.and_then(|value| match value {
            serde_yaml::Value::Number(n) => n.as_u64().and_then(|x| u16::try_from(x).ok()),
            serde_yaml::Value::String(s) => s.parse().ok(),
            _ => None,
        })
    };

    let mut insight = CustomConfigInsight::default();
    // Windows system proxy requires HTTP CONNECT; never point it at SOCKS-only.
    insight.inbound_port = yaml_u16(get("mixed-port"))
        .or_else(|| yaml_u16(get("port")))
        .filter(|port| *port != 0);
    if insight.inbound_port.is_none() {
        // Extra `listeners:` (clash listeners API) may hold the only inbound.
        if let Some(listeners) = get("listeners").and_then(|v| v.as_sequence()) {
            for listener in listeners {
                if !matches!(listener.get("type").and_then(|v| v.as_str()), Some("mixed" | "http")) {
                    continue;
                }
                if let Some(port) = yaml_u16(
                    listener
                        .as_mapping()
                        .and_then(|m| m.get(serde_yaml::Value::String("port".into()))),
                ) {
                    insight.inbound_port = Some(port);
                    break;
                }
            }
        }
    }

    if let Some(controller) = get("external-controller").and_then(|v| v.as_str()) {
        if let Some((host, port)) = split_host_port(controller) {
            insight.clash_api_host = Some(host);
            insight.clash_api_port = Some(port);
        }
    }
    insight.clash_api_secret = get("secret")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty());

    if let Some(tun) = get("tun").and_then(|v| v.as_mapping()) {
        let enabled = tun
            .get(serde_yaml::Value::String("enable".into()))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        insight.has_tun = enabled;
    }
    insight
}

/// Probe listen / TUN from a complete Xray JSON document. Xray has no
/// Clash-compatible REST API in a stock config, so `has_clash_api` stays
/// false (readiness falls back to process-alive).
pub fn inspect_xray_config(content: &str) -> CustomConfigInsight {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return CustomConfigInsight::default();
    };
    let Some(obj) = value.as_object() else {
        return CustomConfigInsight::default();
    };

    let mut insight = CustomConfigInsight::default();
    if let Some(inbounds) = obj.get("inbounds").and_then(Value::as_array) {
        for inbound in inbounds {
            let Some(map) = inbound.as_object() else {
                continue;
            };
            let protocol = map
                .get("protocol")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            if protocol == "tun" {
                insight.has_tun = true;
                continue;
            }
            if insight.inbound_port.is_some() {
                continue;
            }
            if protocol == "http" {
                if let Some(port) = json_u16(map.get("port")) {
                    insight.inbound_port = Some(port);
                }
            }
        }
    }
    insight
}

fn split_host_port(raw: &str) -> Option<(String, u16)> {
    let raw = raw
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
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

    #[test]
    fn inspects_mihomo_yaml() {
        let yaml = r#"
mixed-port: 7897
external-controller: 127.0.0.1:9090
secret: "abc"
tun:
  enable: true
proxies: []
"#;
        let insight = inspect_mihomo_config(yaml);
        assert_eq!(insight.inbound_port, Some(7897));
        assert_eq!(insight.clash_api_port, Some(9090));
        assert_eq!(insight.clash_api_secret.as_deref(), Some("abc"));
        assert!(insight.has_tun);
    }

    #[test]
    fn inspects_mihomo_port_fallbacks() {
        let yaml = "socks-port: 1080\nproxies: []\n";
        assert_eq!(inspect_mihomo_config(yaml).inbound_port, None);
        let listeners = "listeners:\n  - name: mixed-in\n    type: mixed\n    port: 17070\n";
        assert_eq!(inspect_mihomo_config(listeners).inbound_port, Some(17070));
    }

    #[test]
    fn inspects_xray_json() {
        let json = r#"{
          "inbounds": [
            {"protocol":"socks","listen":"127.0.0.1","port":10808},
            {"protocol":"tun","tag":"tun-in"}
          ],
          "outbounds": [{"protocol":"freedom","tag":"direct"}]
        }"#;
        let insight = inspect_xray_config(json);
        assert_eq!(insight.inbound_port, None);
        assert!(insight.has_tun);
        assert!(!insight.has_clash_api());
    }
}

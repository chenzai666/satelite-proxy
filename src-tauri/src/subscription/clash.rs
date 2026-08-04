//! Parse Clash YAML `proxies:` list into normalized [`ProxyNode`]s.

use crate::domain::{
    ParseResult, Protocol, ProtocolConfig, ProxyNode, SkippedProxy, SubscriptionFormat, TlsConfig,
    Transport,
};
use crate::error::{AppError, AppResult};
use crate::subscription::yaml_util::{
    as_mapping, get_bool, get_map, get_str, get_u16, get_u32, map_to_string_map, value_to_string,
};
use serde_yaml::Value;

/// Parse a full Clash config document or a bare proxies list.
pub fn parse_clash_yaml(content: &str) -> AppResult<ParseResult> {
    let content = content.trim();
    if content.is_empty() {
        return Err(AppError::EmptySubscription);
    }

    let root: Value = serde_yaml::from_str(content)
        .map_err(|e| AppError::SubscriptionParse(format!("invalid yaml: {e}")))?;

    let proxies = extract_proxies_seq(&root).ok_or_else(|| {
        AppError::SubscriptionParse("no `proxies` list found in clash yaml".into())
    })?;

    let mut nodes = Vec::new();
    let mut skipped = Vec::new();

    for (idx, item) in proxies.iter().enumerate() {
        match parse_proxy_entry(item) {
            Ok(node) => nodes.push(node.with_computed_id()),
            Err(reason) => {
                let name = as_mapping(item)
                    .and_then(|m| get_str(m, &["name"]))
                    .or_else(|| Some(format!("index-{idx}")));
                skipped.push(SkippedProxy { name, reason });
            }
        }
    }

    if nodes.is_empty() {
        return Err(AppError::NoProxies);
    }

    Ok(ParseResult {
        nodes,
        skipped,
        format: SubscriptionFormat::ClashYaml,
    })
}

fn extract_proxies_seq(root: &Value) -> Option<&Vec<Value>> {
    match root {
        Value::Sequence(seq) => Some(seq),
        Value::Mapping(map) => map
            .get(Value::String("proxies".into()))
            .and_then(|v| v.as_sequence()),
        _ => None,
    }
}

fn parse_proxy_entry(value: &Value) -> Result<ProxyNode, String> {
    let map = as_mapping(value).ok_or_else(|| "proxy entry is not a map".to_string())?;

    let name = get_str(map, &["name"]).unwrap_or_else(|| "unnamed".into());
    let type_str = get_str(map, &["type"]).ok_or_else(|| "missing type".to_string())?;
    let server = get_str(map, &["server"]).ok_or_else(|| "missing server".to_string())?;
    let port = get_u16(map, &["port"]).ok_or_else(|| "missing or invalid port".to_string())?;

    let protocol = Protocol::from_clash_type(&type_str)
        .ok_or_else(|| format!("unsupported type: {type_str}"))?;

    let udp = get_bool(map, &["udp"]);
    let (tls, transport, config) = match protocol {
        Protocol::Shadowsocks => parse_ss(map)?,
        Protocol::Vmess => parse_vmess(map)?,
        Protocol::Vless => parse_vless(map)?,
        Protocol::Trojan => parse_trojan(map)?,
        Protocol::Hysteria2 => parse_hysteria2(map)?,
        Protocol::Tuic => parse_tuic(map)?,
        Protocol::Socks5 => parse_socks5(map)?,
    };

    Ok(ProxyNode {
        id: String::new(),
        name,
        protocol,
        server,
        port,
        tls,
        transport,
        udp,
        config,
        source: Some(type_str),
        latency_ms: None,
        latency_at: None,
    })
}

fn parse_ss(
    map: &serde_yaml::Mapping,
) -> Result<(Option<TlsConfig>, Option<Transport>, ProtocolConfig), String> {
    let method =
        get_str(map, &["cipher", "method"]).ok_or_else(|| "ss: missing cipher".to_string())?;
    let password =
        get_str(map, &["password"]).ok_or_else(|| "ss: missing password".to_string())?;
    let plugin = get_str(map, &["plugin"]);
    let plugin_opts = get_map(map, &["plugin-opts", "plugin_opts"]).map(|m| {
        // Clash uses nested map; sing-box often wants semicolon opts or structured later.
        m.iter()
            .filter_map(|(k, v)| {
                Some(format!(
                    "{}={}",
                    value_to_string(k)?,
                    value_to_string(v)?
                ))
            })
            .collect::<Vec<_>>()
            .join(";")
    });

    Ok((
        None,
        None,
        ProtocolConfig::Shadowsocks {
            method,
            password,
            plugin,
            plugin_opts,
        },
    ))
}

fn parse_vmess(
    map: &serde_yaml::Mapping,
) -> Result<(Option<TlsConfig>, Option<Transport>, ProtocolConfig), String> {
    let uuid = get_str(map, &["uuid", "id"]).ok_or_else(|| "vmess: missing uuid".to_string())?;
    let alter_id = get_u16(map, &["alterId", "alter_id", "aid"]).unwrap_or(0);
    let security = get_str(map, &["cipher", "security"]).unwrap_or_else(|| "auto".into());

    let tls = parse_tls_common(map, false);
    let transport = parse_transport(map);

    Ok((
        tls,
        transport,
        ProtocolConfig::Vmess {
            uuid,
            alter_id,
            security,
        },
    ))
}

fn parse_vless(
    map: &serde_yaml::Mapping,
) -> Result<(Option<TlsConfig>, Option<Transport>, ProtocolConfig), String> {
    let uuid = get_str(map, &["uuid", "id"]).ok_or_else(|| "vless: missing uuid".to_string())?;
    let flow = get_str(map, &["flow"]);
    let packet_encoding =
        get_str(map, &["packet-encoding", "packet_encoding"]).unwrap_or_else(|| "xudp".into());

    let mut tls = parse_tls_common(map, true);
    if let Some(ref mut t) = tls {
        if let Some(opts) = get_map(map, &["reality-opts", "reality_opts"]) {
            t.reality_public_key = get_str(opts, &["public-key", "public_key"]);
            t.reality_short_id = get_str(opts, &["short-id", "short_id"]);
        }
    }

    let transport = parse_transport(map);

    Ok((
        tls,
        transport,
        ProtocolConfig::Vless {
            uuid,
            flow,
            packet_encoding,
        },
    ))
}

fn parse_trojan(
    map: &serde_yaml::Mapping,
) -> Result<(Option<TlsConfig>, Option<Transport>, ProtocolConfig), String> {
    let password =
        get_str(map, &["password"]).ok_or_else(|| "trojan: missing password".to_string())?;

    // Trojan is TLS by default in clash.
    let mut tls = parse_tls_common(map, true).unwrap_or(TlsConfig {
        enabled: true,
        ..Default::default()
    });
    tls.enabled = true;
    if tls.server_name.is_none() {
        tls.server_name = get_str(map, &["sni", "servername", "server-name"]);
    }

    let transport = parse_transport(map);

    Ok((
        Some(tls),
        transport,
        ProtocolConfig::Trojan { password },
    ))
}

fn parse_hysteria2(
    map: &serde_yaml::Mapping,
) -> Result<(Option<TlsConfig>, Option<Transport>, ProtocolConfig), String> {
    let password = get_str(map, &["password", "auth"])
        .ok_or_else(|| "hysteria2: missing password".to_string())?;
    let up_mbps = get_u32(map, &["up", "up-mbps", "up_mbps"]);
    let down_mbps = get_u32(map, &["down", "down-mbps", "down_mbps"]);

    let mut obfs = get_str(map, &["obfs"]);
    let mut obfs_password = get_str(map, &["obfs-password", "obfs_password"]);
    if let Some(opts) = get_map(map, &["obfs-opts", "obfs_opts"]) {
        if obfs.is_none() {
            obfs = get_str(opts, &["type"]);
        }
        if obfs_password.is_none() {
            obfs_password = get_str(opts, &["password"]);
        }
    }

    let mut tls = parse_tls_common(map, true).unwrap_or(TlsConfig {
        enabled: true,
        ..Default::default()
    });
    tls.enabled = true;
    if tls.server_name.is_none() {
        tls.server_name = get_str(map, &["sni", "servername"]);
    }

    Ok((
        Some(tls),
        None,
        ProtocolConfig::Hysteria2 {
            password,
            up_mbps,
            down_mbps,
            obfs,
            obfs_password,
        },
    ))
}

fn parse_tuic(
    map: &serde_yaml::Mapping,
) -> Result<(Option<TlsConfig>, Option<Transport>, ProtocolConfig), String> {
    let uuid = get_str(map, &["uuid"]).ok_or_else(|| "tuic: missing uuid".to_string())?;
    let password = get_str(map, &["password"]).unwrap_or_default();
    let congestion_control =
        get_str(map, &["congestion-controller", "congestion_controller", "congestion-control"]);
    let udp_relay_mode = get_str(map, &["udp-relay-mode", "udp_relay_mode"]);
    let zero_rtt_handshake =
        get_bool(map, &["reduce-rtt", "zero-rtt-handshake", "zero_rtt_handshake"]).unwrap_or(false);

    let mut tls = parse_tls_common(map, true).unwrap_or(TlsConfig {
        enabled: true,
        ..Default::default()
    });
    tls.enabled = true;
    if tls.server_name.is_none() {
        tls.server_name = get_str(map, &["sni", "servername"]);
    }
    if tls.alpn.is_none() {
        if let Some(alpn) = get_str(map, &["alpn"]) {
            tls.alpn = Some(alpn.split(',').map(|s| s.trim().to_string()).collect());
        }
    }

    Ok((
        Some(tls),
        None,
        ProtocolConfig::Tuic {
            uuid,
            password,
            congestion_control,
            udp_relay_mode,
            zero_rtt_handshake,
        },
    ))
}

fn parse_socks5(
    map: &serde_yaml::Mapping,
) -> Result<(Option<TlsConfig>, Option<Transport>, ProtocolConfig), String> {
    let username = get_str(map, &["username", "user"]);
    let password = get_str(map, &["password"]);
    let tls = parse_tls_common(map, false);

    Ok((
        tls,
        None,
        ProtocolConfig::Socks5 {
            username,
            password,
        },
    ))
}

fn parse_tls_common(map: &serde_yaml::Mapping, default_enabled: bool) -> Option<TlsConfig> {
    let explicit = get_bool(map, &["tls"]);
    let has_sni = get_str(map, &["sni", "servername", "server-name"]).is_some();
    let has_reality = get_map(map, &["reality-opts", "reality_opts"]).is_some();
    let enabled = explicit.unwrap_or(default_enabled || has_sni || has_reality);

    if !enabled && !has_reality {
        // Still allow skip-cert-only entries to be ignored.
        if explicit == Some(false) {
            return Some(TlsConfig {
                enabled: false,
                ..Default::default()
            });
        }
        if !default_enabled {
            return None;
        }
    }

    let server_name = get_str(map, &["sni", "servername", "server-name"]);
    let insecure = get_bool(map, &["skip-cert-verify", "skip_cert_verify", "insecure"]);
    // Prefer explicit client-fingerprint. Generic `fingerprint` is often a pin/hash
    // (e.g. 64-char hex on hysteria2) and is NOT a valid sing-box uTLS name.
    let utls_fingerprint = get_str(map, &["client-fingerprint", "client_fingerprint"])
        .or_else(|| get_str(map, &["fingerprint"]))
        .and_then(|s| normalize_utls_fingerprint(&s));

    let alpn = get_str(map, &["alpn"]).map(|s| {
        s.split(',')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect()
    });

    let mut tls = TlsConfig {
        enabled: enabled || has_reality,
        server_name,
        insecure,
        alpn,
        utls_fingerprint,
        reality_public_key: None,
        reality_short_id: None,
    };

    if let Some(opts) = get_map(map, &["reality-opts", "reality_opts"]) {
        tls.reality_public_key = get_str(opts, &["public-key", "public_key"]);
        tls.reality_short_id = get_str(opts, &["short-id", "short_id"]);
        tls.enabled = true;
    }

    Some(tls)
}

/// sing-box uTLS only accepts named profiles (not pin-sha256 / hex hashes).
fn normalize_utls_fingerprint(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    const VALID: &[&str] = &[
        "chrome",
        "firefox",
        "safari",
        "ios",
        "android",
        "edge",
        "360",
        "qq",
        "random",
        "chrome_psk",
        "chrome_psk_shuffle",
        "chrome_padding_psk_shuffle",
        "chrome_pq",
        "chrome_pq_psk",
    ];
    if VALID.contains(&s.as_str()) {
        Some(s)
    } else {
        None
    }
}

fn parse_transport(map: &serde_yaml::Mapping) -> Option<Transport> {
    let network = get_str(map, &["network", "net"]).unwrap_or_else(|| "tcp".into());
    match network.to_ascii_lowercase().as_str() {
        "ws" | "websocket" => {
            let opts = get_map(map, &["ws-opts", "ws_opts"]);
            let path = opts
                .and_then(|m| get_str(m, &["path"]))
                .or_else(|| get_str(map, &["ws-path", "ws_path", "path"]));
            let headers = opts
                .and_then(|m| get_map(m, &["headers"]))
                .map(map_to_string_map)
                .or_else(|| {
                    get_str(map, &["ws-headers", "host", "Host"]).map(|h| {
                        let mut m = std::collections::BTreeMap::new();
                        m.insert("Host".into(), h);
                        m
                    })
                });
            let max_early_data = opts.and_then(|m| get_u32(m, &["max-early-data", "max_early_data"]));
            Some(Transport::Ws {
                path,
                headers,
                max_early_data,
            })
        }
        "grpc" => {
            let opts = get_map(map, &["grpc-opts", "grpc_opts"]);
            let service_name = opts
                .and_then(|m| get_str(m, &["grpc-service-name", "grpc_service_name", "serviceName"]))
                .or_else(|| get_str(map, &["grpc-service-name", "service_name"]));
            Some(Transport::Grpc { service_name })
        }
        "http" | "h2" => {
            let opts = get_map(map, &["http-opts", "h2-opts", "http_opts"]);
            let path = opts.and_then(|m| {
                m.get(Value::String("path".into()))
                    .and_then(|v| match v {
                        Value::Sequence(seq) => seq
                            .first()
                            .and_then(value_to_string),
                        other => value_to_string(other),
                    })
            });
            let host = opts.and_then(|m| {
                m.get(Value::String("host".into())).and_then(|v| match v {
                    Value::Sequence(seq) => Some(
                        seq.iter()
                            .filter_map(value_to_string)
                            .collect::<Vec<_>>(),
                    ),
                    Value::String(s) => Some(vec![s.clone()]),
                    _ => None,
                })
            });
            Some(Transport::Http { path, host })
        }
        "httpupgrade" | "http-upgrade" => {
            let opts = get_map(map, &["http-opts", "httpupgrade-opts"]);
            let path = opts.and_then(|m| get_str(m, &["path"]));
            let host = opts.and_then(|m| get_str(m, &["host"]));
            Some(Transport::HttpUpgrade { path, host })
        }
        "tcp" | "" => Some(Transport::Tcp),
        _ => Some(Transport::Tcp),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ProtocolConfig;

    const SAMPLE: &str = r#"
proxies:
  - name: "SS-HK"
    type: ss
    server: ss.example.com
    port: 8388
    cipher: aes-256-gcm
    password: "secret"
    udp: true
  - name: "VMess-WS"
    type: vmess
    server: vm.example.com
    port: 443
    uuid: 11111111-1111-1111-1111-111111111111
    alterId: 0
    cipher: auto
    tls: true
    skip-cert-verify: true
    servername: cdn.example.com
    network: ws
    ws-opts:
      path: /ray
      headers:
        Host: cdn.example.com
  - name: "VLESS-Reality"
    type: vless
    server: vl.example.com
    port: 443
    uuid: 22222222-2222-2222-2222-222222222222
    tls: true
    servername: www.microsoft.com
    client-fingerprint: chrome
    network: tcp
    flow: xtls-rprx-vision
    reality-opts:
      public-key: pubkey123
      short-id: abcd
  - name: "Trojan-1"
    type: trojan
    server: tj.example.com
    port: 443
    password: "tjpass"
    sni: tj.example.com
    skip-cert-verify: false
  - name: "Hy2"
    type: hysteria2
    server: hy2.example.com
    port: 443
    password: "hy2pass"
    sni: hy2.example.com
    skip-cert-verify: true
    up: "100"
    down: "100"
  - name: "TUIC-1"
    type: tuic
    server: tuic.example.com
    port: 443
    uuid: 33333333-3333-3333-3333-333333333333
    password: "tuicpass"
    sni: tuic.example.com
    congestion-controller: bbr
    udp-relay-mode: native
  - name: "Socks"
    type: socks5
    server: 127.0.0.1
    port: 1080
    username: user
    password: pass
  - name: "SSR-skip"
    type: ssr
    server: x.com
    port: 1
"#;

    #[test]
    fn parses_mixed_clash_proxies() {
        let result = parse_clash_yaml(SAMPLE).expect("parse ok");
        assert_eq!(result.format, SubscriptionFormat::ClashYaml);
        assert_eq!(result.nodes.len(), 7);
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0]
            .reason
            .contains("unsupported type: ssr"));

        let ss = result
            .nodes
            .iter()
            .find(|n| n.name == "SS-HK")
            .expect("ss");
        assert_eq!(ss.protocol, Protocol::Shadowsocks);
        assert_eq!(ss.server, "ss.example.com");
        assert_eq!(ss.port, 8388);
        assert_eq!(ss.udp, Some(true));
        match &ss.config {
            ProtocolConfig::Shadowsocks { method, password, .. } => {
                assert_eq!(method, "aes-256-gcm");
                assert_eq!(password, "secret");
            }
            _ => panic!("expected ss config"),
        }

        let vm = result
            .nodes
            .iter()
            .find(|n| n.name == "VMess-WS")
            .expect("vmess");
        assert!(vm.tls.as_ref().is_some_and(|t| t.enabled));
        assert!(matches!(
            vm.transport,
            Some(Transport::Ws {
                path: Some(ref p),
                ..
            }) if p == "/ray"
        ));

        let vl = result
            .nodes
            .iter()
            .find(|n| n.name == "VLESS-Reality")
            .expect("vless");
        let tls = vl.tls.as_ref().expect("tls");
        assert_eq!(tls.reality_public_key.as_deref(), Some("pubkey123"));
        assert_eq!(tls.utls_fingerprint.as_deref(), Some("chrome"));
        match &vl.config {
            ProtocolConfig::Vless { flow, .. } => {
                assert_eq!(flow.as_deref(), Some("xtls-rprx-vision"));
            }
            _ => panic!("expected vless"),
        }

        assert!(result.nodes.iter().all(|n| !n.id.is_empty()));
    }

    #[test]
    fn ignores_hex_fingerprint_as_utls() {
        // Many hy2 panels put a pin/hash in `fingerprint` — not a uTLS profile.
        let yaml = r#"
- name: "Hy2-BadFp"
  type: hysteria2
  server: hy2.example.com
  port: 443
  password: "x"
  sni: www.example.com
  fingerprint: 59777b9f4c7e20e49d88b179b5e3f75031f1e08be731670b2ee09acb6c1f3811
"#;
        let result = parse_clash_yaml(yaml).unwrap();
        let n = &result.nodes[0];
        let tls = n.tls.as_ref().expect("tls");
        assert!(
            tls.utls_fingerprint.is_none(),
            "hex fingerprint must not become utls"
        );
    }

    #[test]
    fn parses_bare_sequence() {
        let yaml = r#"
- name: only
  type: ss
  server: a.com
  port: 1
  cipher: aes-128-gcm
  password: p
"#;
        let result = parse_clash_yaml(yaml).unwrap();
        assert_eq!(result.nodes.len(), 1);
    }
}

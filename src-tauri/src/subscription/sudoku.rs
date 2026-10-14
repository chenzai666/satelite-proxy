//! Independently implemented official Base64URL/JSON Sudoku share format.
use crate::domain::{Protocol, ProtocolConfig, ProxyNode, SudokuHttpMask, SudokuSettings};
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Link {
    h: String,
    p: u16,
    k: String,
    #[serde(default)]
    a: String,
    #[serde(default)]
    e: String,
    #[serde(default)]
    x: bool,
    #[serde(default)]
    t: String,
    #[serde(default)]
    ts: Vec<String>,
    #[serde(default)]
    hd: bool,
    #[serde(default)]
    hm: String,
    #[serde(default)]
    ht: bool,
    #[serde(default)]
    hh: String,
    #[serde(default)]
    hx: String,
    #[serde(default)]
    hy: String,
    // Optional local extensions: official clients safely ignore unknown keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    n: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    padding_min: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    padding_max: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    u: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    multiplex: Option<String>,
}

pub(super) fn parse_uri(uri: &str) -> Result<ProxyNode, String> {
    let (_, body) = uri.split_once("://").ok_or("sudoku: invalid URI")?;
    let (encoded, fragment) = body
        .split_once('#')
        .map_or((body, None), |(a, b)| (a, Some(b)));
    let decoded = general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .or_else(|_| general_purpose::URL_SAFE.decode(encoded))
        .map_err(|_| "sudoku: invalid Base64URL payload")?;
    let link: Link =
        serde_json::from_slice(&decoded).map_err(|_| "sudoku: invalid JSON payload")?;
    if link.h.trim().is_empty() || link.p == 0 {
        return Err("sudoku: missing server or invalid port".into());
    }
    let settings = SudokuSettings {
        key: link.k,
        // Official short-link decoder defaults, distinct from Mihomo YAML defaults.
        aead_method: Some(if link.e.is_empty() {
            "none".into()
        } else {
            link.e
        }),
        table_type: Some(match link.a.as_str() {
            "" | "entropy" => "prefer_entropy".into(),
            "ascii" => "prefer_ascii".into(),
            _ => link.a,
        }),
        padding_min: Some(link.padding_min.unwrap_or(5)),
        padding_max: Some(link.padding_max.unwrap_or(15)),
        custom_table: (!link.t.is_empty()).then_some(link.t),
        custom_tables: link.ts,
        enable_pure_downlink: Some(!link.x),
        multiplex: link.multiplex,
        httpmask: Some(SudokuHttpMask {
            disable: link.hd,
            mode: link.hm,
            tls: link.ht,
            host: link.hh,
            path_root: link.hy,
            multiplex: link.hx,
        }),
    };
    settings.validate()?;
    let fragment_name = fragment.map(|s| {
        urlencoding::decode(s)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| s.into())
    });
    Ok(ProxyNode {
        id: String::new(),
        name: fragment_name
            .or(link.n)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("sudoku-{}-{}", link.h, link.p)),
        protocol: Protocol::Sudoku,
        server: link.h,
        port: link.p,
        config: ProtocolConfig::Sudoku { settings },
        tls: None,
        transport: None,
        udp: link.u,
        source: Some("sudoku".into()),
        raw: None,
        latency_ms: None,
        latency_at: None,
    })
}

pub(super) fn share(node: &ProxyNode, settings: &SudokuSettings) -> Result<String, String> {
    settings.validate()?;
    let mask = settings.httpmask.clone().unwrap_or_default();
    let table = settings.table_type.as_deref().unwrap_or("prefer_entropy");
    let mut min = settings.padding_min.unwrap_or(10);
    let mut max = settings.padding_max.unwrap_or(30);
    if settings.padding_min.is_none() && max < min {
        min = max;
    }
    if settings.padding_max.is_none() && max < min {
        max = min;
    }
    let link = Link {
        h: node.server.clone(),
        p: node.port,
        k: settings.key.clone(),
        a: match table {
            "prefer_ascii" => "ascii",
            "prefer_entropy" => "entropy",
            _ => table,
        }
        .into(),
        e: settings
            .aead_method
            .clone()
            .unwrap_or_else(|| "chacha20-poly1305".into()),
        x: !settings.enable_pure_downlink.unwrap_or(true),
        t: settings.custom_table.clone().unwrap_or_default(),
        ts: settings.custom_tables.clone(),
        hd: mask.disable,
        hm: mask.mode,
        ht: mask.tls,
        hh: mask.host,
        hx: if mask.multiplex.trim().is_empty() {
            settings.multiplex.clone().unwrap_or_default()
        } else {
            mask.multiplex
        },
        hy: mask.path_root,
        n: Some(node.name.clone()),
        padding_min: Some(min),
        padding_max: Some(max),
        u: node.udp,
        multiplex: settings.multiplex.clone(),
    };
    let bytes = serde_json::to_vec(&link).map_err(|_| "sudoku: cannot serialize share link")?;
    Ok(format!(
        "sudoku://{}",
        general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    ))
}

pub(super) fn clash_settings(map: &serde_yaml::Mapping) -> Result<SudokuSettings, String> {
    let mut settings: SudokuSettings =
        serde_yaml::from_value(serde_yaml::Value::Mapping(map.clone()))
            .map_err(|_| "sudoku: invalid parameter types")?;
    use super::yaml_util::{get_bool, get_str};
    let flat = SudokuHttpMask {
        disable: !get_bool(map, &["http-mask"]).unwrap_or(true),
        mode: get_str(map, &["http-mask-mode"]).unwrap_or_default(),
        tls: get_bool(map, &["http-mask-tls"]).unwrap_or(false),
        host: get_str(map, &["http-mask-host"]).unwrap_or_default(),
        path_root: get_str(map, &["path-root"]).unwrap_or_default(),
        multiplex: get_str(map, &["http-mask-multiplex"]).unwrap_or_default(),
    };
    if let Some(mask) = settings.httpmask.as_mut() {
        // Same nested-over-flat precedence as Mihomo: booleans and host
        // always override; empty mode/path/mux inherit their flat aliases.
        if mask.mode.is_empty() {
            mask.mode = flat.mode;
        }
        if mask.path_root.trim().is_empty() {
            mask.path_root = flat.path_root;
        }
        if mask.multiplex.trim().is_empty() {
            mask.multiplex = flat.multiplex;
        }
    } else {
        settings.httpmask = Some(flat);
    }
    settings.normalize();
    settings.validate()?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::{
        draft_to_node, node_to_draft, parse_subscription, serialize_share_uri,
    };

    #[test]
    fn sudoku_official_link_base64_subscription_and_name_roundtrip() {
        let payload = r#"{"h":"2001:db8::1","p":443,"k":"test-key","a":"ascii","e":"chacha20-poly1305","x":true,"ts":["xpxvvpvv"],"hm":"ws","ht":true,"hh":"cdn.example.com","hy":"api","hx":"on"}"#;
        let uri = format!(
            "sudoku://{}",
            general_purpose::URL_SAFE_NO_PAD.encode(payload)
        );
        let subscription = general_purpose::STANDARD.encode(&uri);
        let original = parse_subscription(&subscription).unwrap().nodes.remove(0);
        let ProtocolConfig::Sudoku { settings } = &original.config else {
            panic!("not Sudoku")
        };
        assert_eq!(settings.enable_pure_downlink, Some(false));
        assert_eq!(settings.padding_min, Some(5));
        let mut draft = node_to_draft(&original);
        draft.name = Some("新名 + clone 🚀".into());
        let edited = draft_to_node(&draft, None).unwrap();
        assert_eq!(edited.config, original.config);
        let shared = serialize_share_uri(&edited).unwrap();
        assert!(
            !shared.contains('#'),
            "official decoder expects the entire body to be base64"
        );
        let restored = parse_subscription(&shared).unwrap().nodes.remove(0);
        assert_eq!(restored.name, edited.name);
        assert_eq!(restored.config, edited.config);
        assert_eq!(restored.server, "2001:db8::1");
    }

    #[test]
    fn sudoku_yaml_edit_and_share_keep_effective_defaults_and_flat_httpmask() {
        let yaml = "proxies:\n- {name: test, type: sudoku, server: example.com, port: 443, key: test-key, http-mask: false, http-mask-mode: ws, http-mask-host: cdn.example.com, padding-min: 40}";
        let original = parse_subscription(yaml).unwrap().nodes.remove(0);
        let edited = draft_to_node(&node_to_draft(&original), None).unwrap();
        assert_eq!(edited.config, original.config);
        let shared = serialize_share_uri(&edited).unwrap();
        let restored = parse_subscription(&shared).unwrap().nodes.remove(0);
        let ProtocolConfig::Sudoku { settings } = restored.config else {
            panic!()
        };
        assert_eq!(settings.aead_method.as_deref(), Some("chacha20-poly1305"));
        assert_eq!(settings.enable_pure_downlink, Some(true));
        assert_eq!(
            (settings.padding_min, settings.padding_max),
            (Some(40), Some(40))
        );
        assert!(settings.httpmask.unwrap().disable);
    }

    #[test]
    fn sudoku_rejects_invalid_options_without_exposing_credentials() {
        for text in [
            "{bad",
            r#"{"h":"host","p":0,"k":"private-secret"}"#,
            r#"{"h":"host","p":443,"k":"private-secret","e":"bad"}"#,
        ] {
            let error = parse_uri(&format!(
                "sudoku://{}",
                general_purpose::URL_SAFE_NO_PAD.encode(text)
            ))
            .unwrap_err();
            assert!(!error.contains("private-secret"));
        }
        let mixed = "proxies:\n- {name: ok, type: sudoku, server: host, port: 443, key: test-key}\n- {name: bad, type: sudoku, server: host, port: 443, key: test-key, padding-min: 70, padding-max: 10}";
        let parsed = parse_subscription(mixed).unwrap();
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.skipped.len(), 1);
        assert!(!crate::core::CoreKind::Xray.supports_node(&parsed.nodes[0]));
        assert!(crate::core::CoreKind::Mihomo.supports_node(&parsed.nodes[0]));
        assert_eq!(
            crate::core::CoreKind::Xray.manual_node_fallback(&parsed.nodes[0]),
            Some(crate::core::CoreKind::Mihomo)
        );
        assert_eq!(
            crate::core::CoreKind::SingBox.manual_node_fallback(&parsed.nodes[0]),
            Some(crate::core::CoreKind::Mihomo)
        );
        assert_eq!(
            crate::core::CoreKind::Mihomo.manual_node_fallback(&parsed.nodes[0]),
            None
        );
        let report = crate::services::core_support::core_support_report(mixed).unwrap();
        assert_eq!(report.cores.singbox, 0);
        assert_eq!(report.cores.mihomo, 1);
        assert_eq!(report.cores.xray, 0);
    }

    #[test]
    fn sudoku_share_puts_top_level_mux_in_the_official_field() {
        let node = parse_subscription("proxies:\n- {name: test, type: sudoku, server: host, port: 443, key: test-key, multiplex: on, http-mask-mode: ws, path-root: api, httpmask: {tls: true, host: cdn.example.com}}").unwrap().nodes.remove(0);
        let ProtocolConfig::Sudoku { settings } = &node.config else {
            panic!()
        };
        let shared = share(&node, settings).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(
            &general_purpose::URL_SAFE_NO_PAD
                .decode(shared.strip_prefix("sudoku://").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["hx"], "on");
        assert_eq!(payload["hm"], "ws");
        assert_eq!(payload["hy"], "api");
        assert_eq!(payload["hh"], "cdn.example.com");
        assert_eq!(payload["ht"], true);
    }

    #[test]
    fn sudoku_blank_optional_exporter_fields_use_kernel_defaults() {
        let parsed = parse_subscription("proxies:\n- {name: test, type: sudoku, server: host, port: 443, key: test-key, aead-method: '', table-type: '', custom-table: '', multiplex: ''}").unwrap();
        assert!(parsed.skipped.is_empty());
        let shared = serialize_share_uri(&parsed.nodes[0]).unwrap();
        let restored = parse_subscription(&shared).unwrap();
        let ProtocolConfig::Sudoku { settings } = &restored.nodes[0].config else {
            panic!()
        };
        assert_eq!(settings.aead_method.as_deref(), Some("chacha20-poly1305"));
        assert_eq!(settings.table_type.as_deref(), Some("prefer_entropy"));
    }
}

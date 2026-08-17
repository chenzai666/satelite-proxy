use satelite_proxy_lib::{parse_subscription, Protocol, SubscriptionFormat};

#[test]
fn fixture_clash_yaml() {
    let yaml = include_str!("fixtures/clash_sample.yaml");
    let result = parse_subscription(yaml).expect("parse fixture");
    assert_eq!(result.format, SubscriptionFormat::ClashYaml);
    assert_eq!(result.nodes.len(), 5);
    assert_eq!(result.skipped.len(), 1);

    let names: Vec<_> = result.nodes.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"SS-HK"));
    assert!(names.contains(&"VLESS-Reality"));

    let vless = result
        .nodes
        .iter()
        .find(|n| n.protocol == Protocol::Vless)
        .unwrap();
    assert_eq!(vless.server, "vl.example.com");
    assert!(vless
        .tls
        .as_ref()
        .unwrap()
        .reality_public_key
        .as_ref()
        .is_some());
}

#[test]
fn fixture_singbox_json() {
    let json = include_str!("fixtures/singbox_sample.json");
    let result = parse_subscription(json).expect("parse singbox fixture");
    assert_eq!(result.format, SubscriptionFormat::SingboxJson);
    assert_eq!(result.nodes.len(), 2);
    assert!(result.skipped.len() >= 2);
    let names: Vec<_> = result.nodes.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"VLESS-Reality"));
    assert!(names.contains(&"SS-HK"));
}

#[test]
fn parse_singbox_outbounds_only() {
    let json = r#"{
      "outbounds": [
        {
          "type": "trojan",
          "tag": "TJ",
          "server": "tj.example.com",
          "server_port": 443,
          "password": "x",
          "tls": { "enabled": true, "server_name": "tj.example.com" }
        }
      ]
    }"#;
    let result = parse_subscription(json).expect("parse outbounds");
    assert_eq!(result.format, SubscriptionFormat::SingboxJson);
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].name, "TJ");
}

#[test]
fn fixture_clash_full_config_keeps_every_proxy() {
    let yaml = include_str!("fixtures/clash_free_sample.yaml");
    let result = parse_subscription(yaml).expect("parse full clash config");
    assert_eq!(result.format, SubscriptionFormat::ClashYaml);
    assert_eq!(
        result.skipped.len(),
        0,
        "skipped: {:?}",
        result
            .skipped
            .iter()
            .map(|s| format!("{}: {}", s.name.as_deref().unwrap_or("?"), s.reason))
            .collect::<Vec<_>>()
    );
    assert_eq!(result.nodes.len(), 8);
    assert_eq!(
        result
            .nodes
            .iter()
            .filter(|n| n.protocol == Protocol::Http)
            .count(),
        3
    );
    assert_eq!(
        result
            .nodes
            .iter()
            .filter(|n| n.protocol == Protocol::AnyTls)
            .count(),
        2
    );
    assert!(result.nodes.iter().any(|n| n.name == "US-HTTP-NULL-USER"));
    assert!(result.nodes.iter().any(|n| n.name == "JP-HTTP-NO-AUTH-A"));
    assert!(result.nodes.iter().any(|n| n.name == "JP-HTTP-NO-AUTH-B"));
    assert!(result.nodes.iter().any(|n| n.name == "SG-ANYTLS-A"));
}

#[test]
fn parse_clash_json_uses_same_path() {
    let json = r#"{
      "proxies": [
        {
          "name": "SS-HK",
          "type": "ss",
          "server": "ss.example.com",
          "port": 8388,
          "cipher": "aes-256-gcm",
          "password": "secret"
        }
      ]
    }"#;
    let result = parse_subscription(json).expect("parse clash json");
    assert_eq!(result.format, SubscriptionFormat::ClashYaml);
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(result.nodes[0].name, "SS-HK");
}

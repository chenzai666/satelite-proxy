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

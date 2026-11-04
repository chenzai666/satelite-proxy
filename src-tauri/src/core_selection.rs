//! Local core preferences, distinct from sing-box's sidecar protocol pins.
use crate::core::{inspect_core_bin, CoreKind};
use crate::domain::{AppSettings, AutoSelectMode, Protocol, ProxyNode};
use crate::error::{AppError, AppResult};
use crate::storage::AppStore;
use std::path::Path;

pub(crate) fn strict_core(raw: &str) -> Option<CoreKind> {
    match raw {
        "singbox" => Some(CoreKind::SingBox),
        "xray" => Some(CoreKind::Xray),
        "mihomo" => Some(CoreKind::Mihomo),
        _ => None,
    }
}

pub(crate) fn validate_node_override(node: &ProxyNode, raw: &str) -> AppResult<Option<String>> {
    if raw == "auto" {
        return Ok(None);
    }
    let kind = strict_core(raw).ok_or_else(|| AppError::Config("无效的节点内核选择".into()))?;
    if let Some(reason) = kind.native_node_unsupported_reason(node) {
        return Err(AppError::Config(format!(
            "{} 无法使用该节点：{reason}",
            kind.display_name()
        )));
    }
    Ok(Some(kind.as_str().into()))
}

pub(crate) fn preferred_core(settings: &AppSettings) -> CoreKind {
    settings
        .preferred_core_type
        .as_deref()
        .and_then(strict_core)
        .unwrap_or_else(|| CoreKind::parse(&settings.core_type))
}

pub(crate) fn choose_core(settings: &AppSettings, node: &ProxyNode) -> AppResult<CoreKind> {
    // Resolve the effective compatibility option without mutating subscription
    // data. Explicit Xray requests cannot silently discard TLS/obfs fields.
    let mut candidates = vec![node.clone()];
    crate::config::apply_udp_node_compatibility(&mut candidates, settings.udp_tls_compat);
    let node = &candidates[0];
    if let Some(raw) = settings.node_core_overrides.get(&node.id) {
        if let Some(core) = validate_node_override(node, raw)? {
            return Ok(strict_core(&core).expect("validated core"));
        }
    }
    if let Some(kind) = settings
        .protocol_core_defaults
        .iter()
        .find(|row| row.protocol == node.protocol.as_str())
        .and_then(|row| strict_core(&row.core))
    {
        if kind.native_node_unsupported_reason(node).is_none() {
            return Ok(kind);
        }
    }
    let preferred = preferred_core(settings);
    if preferred == CoreKind::Xray
        && matches!(
            node.protocol,
            Protocol::Hysteria2 | Protocol::Tuic | Protocol::AnyTls
        )
        && CoreKind::SingBox
            .native_node_unsupported_reason(node)
            .is_none()
    {
        return Ok(CoreKind::SingBox);
    }
    [
        preferred,
        CoreKind::SingBox,
        CoreKind::Xray,
        CoreKind::Mihomo,
    ]
    .into_iter()
    .find(|kind| kind.native_node_unsupported_reason(node).is_none())
    .ok_or_else(|| AppError::Config("当前支持的内核均无法表达该节点的协议和参数".into()))
}

/// Resolve the actual main core; a valid sidecar can satisfy a node choice
/// without replacing the sing-box main process. Explicit node overrides must
/// match the real plan, including port/chain/limit exclusions.
pub(crate) fn main_core_for_node(store: &AppStore, node: &ProxyNode) -> AppResult<CoreKind> {
    let chosen = choose_core(&store.settings, node)?;
    let current = CoreKind::parse(&store.settings.core_type);
    if current == CoreKind::SingBox && store.settings.multi_core_enabled {
        let delegated = crate::runtime::compute_sidecar_plan(
            &store.settings,
            &store.chains,
            &store.enabled_nodes(),
        )
        .and_then(|plan| {
            plan.ports
                .into_iter()
                .find(|entry| entry.node_id == node.id)
        });
        if let Some(entry) = delegated {
            let explicit = store
                .settings
                .node_core_overrides
                .get(&node.id)
                .filter(|raw| raw.as_str() != "auto" && !raw.is_empty());
            if explicit.is_none() || entry.kind == chosen {
                return Ok(current);
            }
        }
    }
    Ok(chosen)
}

pub(crate) fn ensure_core_present(
    app_data: &Path,
    resources: Option<&Path>,
    kind: CoreKind,
) -> AppResult<()> {
    if inspect_core_bin(app_data, resources, kind).0.is_none() {
        return Err(AppError::Core(format!(
            "需要 {} 内核，请先在设置中安装该内核",
            kind.display_name()
        )));
    }
    Ok(())
}

pub(crate) fn ensure_sidecars_present(
    app_data: &Path,
    resources: Option<&Path>,
    store: &AppStore,
) -> AppResult<()> {
    if let Some(plan) =
        crate::runtime::compute_sidecar_plan(&store.settings, &store.chains, &store.enabled_nodes())
    {
        for kind in plan.used_kinds() {
            ensure_core_present(app_data, resources, kind)?;
        }
    }
    Ok(())
}

/// Smart/kernel-auto selection stays within the running core. Unconfigured
/// nodes keep existing behavior; explicit preferences must be satisfiable by
/// that core or its actual sidecar plan. Build the plan once per candidate set.
pub(crate) fn smart_nodes(store: &AppStore) -> Vec<ProxyNode> {
    let nodes = store.enabled_nodes();
    let settings = &store.settings;
    if settings.core_type == "mihomo"
        || settings.runtime_source().is_custom()
        || (settings.node_core_overrides.is_empty() && settings.protocol_core_defaults.is_empty())
    {
        return nodes;
    }
    let current = CoreKind::parse(&settings.core_type);
    let delegated: std::collections::HashMap<_, _> =
        crate::runtime::compute_sidecar_plan(settings, &store.chains, &nodes)
            .map(|plan| {
                plan.ports
                    .into_iter()
                    .map(|entry| (entry.node_id, entry.kind))
                    .collect()
            })
            .unwrap_or_default();
    nodes
        .into_iter()
        .filter(|node| {
            let explicit = settings
                .node_core_overrides
                .get(&node.id)
                .is_some_and(|raw| raw != "auto" && !raw.is_empty());
            let configured = explicit
                || settings
                    .protocol_core_defaults
                    .iter()
                    .any(|row| row.protocol == node.protocol.as_str());
            if !configured || (!explicit && delegated.contains_key(&node.id)) {
                return true;
            }
            choose_core(settings, node)
                .is_ok_and(|chosen| delegated.get(&node.id).copied().unwrap_or(current) == chosen)
        })
        .collect()
}

/// Constrain only the main automatic group. Keep all generated outbounds and
/// rule pools intact, and never recompute sidecar ports on a filtered list.
pub(crate) fn needs_kernel_restriction(settings: &AppSettings) -> bool {
    settings.auto_select.is_kernel()
        && settings.core_type != "mihomo"
        && !settings.runtime_source().is_custom()
        && (!settings.node_core_overrides.is_empty() || !settings.protocol_core_defaults.is_empty())
}

pub(crate) fn restrict_kernel_candidates(
    settings: &AppSettings,
    eligible: &[ProxyNode],
    value: &mut serde_json::Value,
    generated_tags: &[String],
) -> AppResult<()> {
    if !needs_kernel_restriction(settings) {
        return Ok(());
    }
    let allowed: std::collections::HashSet<_> =
        eligible.iter().map(crate::config::outbound_tag).collect();
    let tags: Vec<_> = generated_tags
        .iter()
        .filter(|tag| allowed.contains(*tag))
        .cloned()
        .collect();
    if tags.is_empty() {
        return Err(AppError::Config(
            "当前内核没有符合自动选择设置的节点".into(),
        ));
    }
    if settings.core_type == "xray" {
        if let Some(groups) = value["routing"]["balancers"].as_array_mut() {
            for group in groups {
                if group["tag"] == "proxy-balancer" {
                    group["selector"] = serde_json::json!(tags);
                }
            }
        }
    } else if let Some(outbounds) = value["outbounds"].as_array_mut() {
        for group in outbounds {
            if group["tag"] == "proxy" && group["type"] == "urltest" {
                group["outbounds"] = serde_json::json!(tags);
            }
        }
    }
    Ok(())
}

/// Called before stopping an old process and again on cold start. Native
/// Mihomo policy groups and custom configurations retain their own lifecycle.
pub(crate) fn prepare_selected_core(
    app_data: &Path,
    resources: Option<&Path>,
    store: &mut AppStore,
) -> AppResult<()> {
    if store.settings.runtime_source().is_custom() {
        return Ok(());
    }
    let current = CoreKind::parse(&store.settings.core_type);
    if current == CoreKind::Mihomo {
        return ensure_core_present(app_data, resources, current);
    }
    if store.settings.auto_select != AutoSelectMode::Off {
        ensure_core_present(app_data, resources, current)?;
        ensure_sidecars_present(app_data, resources, store)?;
        let delegated = crate::runtime::delegated_node_ids(
            &store.settings,
            &store.chains,
            &store.enabled_nodes(),
        );
        let mut candidates = smart_nodes(store);
        crate::config::apply_udp_node_compatibility(&mut candidates, store.settings.udp_tls_compat);
        if !candidates.iter().any(|node| {
            current.native_node_unsupported_reason(node).is_none() || delegated.contains(&node.id)
        }) && !store.enabled_nodes().is_empty()
        {
            return Err(AppError::Config(
                "当前自动选择模式中没有符合内核设置的节点，请先手动选择节点".into(),
            ));
        }
        return Ok(());
    }
    store.ensure_current_node_valid();
    let Some(node) = store
        .settings
        .current_node_id
        .as_deref()
        .and_then(|id| store.find_node(id))
    else {
        return Ok(());
    };
    let chosen = main_core_for_node(store, node)?;
    ensure_core_present(app_data, resources, chosen)?;
    if chosen == CoreKind::SingBox {
        ensure_sidecars_present(app_data, resources, store)?;
    }
    if store.settings.preferred_core_type.is_none() {
        store.settings.preferred_core_type = Some(store.settings.core_type.clone());
    }
    if store.settings.core_type != chosen.as_str() {
        store.settings.core_type = chosen.as_str().into();
        store.settings.enforce_multi_core_scope();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn node(uri: &str) -> ProxyNode {
        crate::subscription::parse_single_uri(uri, None)
            .unwrap()
            .nodes
            .remove(0)
    }
    fn vless() -> ProxyNode {
        node("vless://11111111-1111-4111-8111-111111111111@example.invalid:443?security=tls&type=tcp#test")
    }
    fn hy2() -> ProxyNode {
        node("hysteria2://test-password@example.invalid:443?insecure=1#test")
    }

    fn store_with_node(node: ProxyNode) -> AppStore {
        let mut store = AppStore::default();
        store.subscriptions.push(serde_json::from_value(serde_json::json!({
            "id":"fixture", "name":"Fixture", "source":{"kind":"url","url":"https://example.invalid/sub"},
            "last_update":0, "node_count":1, "enabled":true
        })).unwrap());
        store.settings.current_node_id = Some(node.id.clone());
        let mut entry = serde_json::to_value(node).unwrap();
        entry["subscription_id"] = serde_json::json!("fixture");
        store.nodes.push(serde_json::from_value(entry).unwrap());
        store
    }

    #[test]
    fn node_override_wins_over_a_protocol_sidecar_pin() {
        let node = vless();
        let mut store = store_with_node(node.clone());
        store.settings.multi_core_enabled = true;
        store
            .settings
            .protocol_cores
            .push(crate::domain::ProtocolCoreItem {
                protocol: "vless".into(),
                core: "xray".into(),
            });
        let plan = crate::runtime::compute_sidecar_plan(
            &store.settings,
            &store.chains,
            &store.enabled_nodes(),
        )
        .unwrap();
        assert_eq!(plan.ports[0].kind, CoreKind::Xray);
        assert_eq!(
            main_core_for_node(&store, &node).unwrap(),
            CoreKind::SingBox
        );
        store
            .settings
            .node_core_overrides
            .insert(node.id.clone(), "singbox".into());
        assert!(crate::runtime::compute_sidecar_plan(
            &store.settings,
            &store.chains,
            &store.enabled_nodes()
        )
        .is_none());
        assert_eq!(
            main_core_for_node(&store, &node).unwrap(),
            CoreKind::SingBox
        );
        store
            .settings
            .node_core_overrides
            .insert(node.id.clone(), "mihomo".into());
        let plan = crate::runtime::compute_sidecar_plan(
            &store.settings,
            &store.chains,
            &store.enabled_nodes(),
        )
        .unwrap();
        assert_eq!(plan.ports[0].kind, CoreKind::Mihomo);
        assert_eq!(
            main_core_for_node(&store, &node).unwrap(),
            CoreKind::SingBox
        );
    }

    #[test]
    fn smart_candidates_do_not_ignore_explicit_node_core_choices() {
        let node = vless();
        let mut store = store_with_node(node.clone());
        store
            .settings
            .node_core_overrides
            .insert(node.id.clone(), "xray".into());
        assert!(smart_nodes(&store).is_empty());
        store.settings.core_type = "xray".into();
        assert_eq!(smart_nodes(&store).len(), 1);
        store.settings.core_type = "singbox".into();
        store.settings.multi_core_enabled = true;
        assert_eq!(smart_nodes(&store).len(), 1);
    }

    #[test]
    fn kernel_filter_does_not_remove_outbounds_or_rule_pool_members() {
        let first = vless();
        let mut second = first.clone();
        second.id = "second-node".into();
        let mut settings = AppSettings::default();
        settings.auto_select = AutoSelectMode::Kernel;
        settings
            .node_core_overrides
            .insert(second.id.clone(), "xray".into());
        let tags = vec![
            crate::config::outbound_tag(&first),
            crate::config::outbound_tag(&second),
        ];
        let mut document = serde_json::json!({"outbounds":[
            {"type":"urltest","tag":"proxy","outbounds":tags},
            {"type":"selector","tag":"smart-rule","outbounds":tags},
            {"type":"trojan","tag":tags[0]}, {"type":"trojan","tag":tags[1]}
        ]});
        restrict_kernel_candidates(&settings, &[first.clone()], &mut document, &tags).unwrap();
        assert_eq!(
            document["outbounds"][0]["outbounds"],
            serde_json::json!([tags[0]])
        );
        assert_eq!(
            document["outbounds"][1]["outbounds"],
            serde_json::json!(tags)
        );
        assert_eq!(document["outbounds"].as_array().unwrap().len(), 4);
        settings.core_type = "xray".into();
        let mut document = serde_json::json!({"routing":{"balancers":[
            {"tag":"proxy-balancer","selector":["node-"]},
            {"tag":"filter-rule","selector":tags}
        ]}});
        restrict_kernel_candidates(&settings, &[first], &mut document, &tags).unwrap();
        assert_eq!(
            document["routing"]["balancers"][0]["selector"],
            serde_json::json!([tags[0]])
        );
        assert_eq!(
            document["routing"]["balancers"][1]["selector"],
            serde_json::json!(tags)
        );
    }

    #[test]
    fn automatic_preflight_rejects_nonrenderable_candidates_before_retargeting() {
        let first = vless();
        let mut store = store_with_node(first.clone());
        let mut entry =
            serde_json::to_value(node("anytls://fixture-password@example.invalid:443")).unwrap();
        entry["subscription_id"] = serde_json::json!("fixture");
        store.nodes.push(serde_json::from_value(entry).unwrap());
        store.settings.core_type = "xray".into();
        store.settings.auto_select = AutoSelectMode::Kernel;
        store
            .settings
            .node_core_overrides
            .insert(first.id, "singbox".into());
        let root = std::env::temp_dir().join(format!(
            "satelite-core-preflight-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let binary = bin_dir.join(CoreKind::Xray.binary_name());
        std::fs::write(&binary, b"fixture, never executed").unwrap();
        let checked = prepare_selected_core(&root, None, &mut store);
        std::fs::remove_file(&binary).unwrap();
        std::fs::remove_dir(&bin_dir).unwrap();
        std::fs::remove_dir(&root).unwrap();
        assert!(checked.is_err());
        assert_eq!(store.settings.core_type, "xray");
        assert!(store.settings.preferred_core_type.is_none());
        assert_eq!(
            store.nodes.len(),
            2,
            "preflight must retain all rule-pool nodes"
        );
    }

    #[test]
    fn udp_compatibility_cannot_be_lost_in_an_xray_sidecar() {
        let mut node = hy2();
        node.tls.as_mut().unwrap().insecure = Some(false);
        let mut store = store_with_node(node.clone());
        store.settings.multi_core_enabled = true;
        store
            .settings
            .protocol_cores
            .push(crate::domain::ProtocolCoreItem {
                protocol: "hysteria2".into(),
                core: "xray".into(),
            });
        assert!(crate::runtime::compute_sidecar_plan(
            &store.settings,
            &store.chains,
            &store.enabled_nodes()
        )
        .is_some());
        store.settings.udp_tls_compat = true;
        assert!(crate::runtime::compute_sidecar_plan(
            &store.settings,
            &store.chains,
            &store.enabled_nodes()
        )
        .is_none());
        assert_eq!(
            main_core_for_node(&store, &node).unwrap(),
            CoreKind::SingBox
        );
        assert_eq!(
            store
                .find_node(&node.id)
                .unwrap()
                .tls
                .as_ref()
                .unwrap()
                .insecure,
            Some(false)
        );
    }

    #[test]
    fn custom_and_mihomo_policy_modes_do_not_retarget_the_main_core() {
        let mut store = store_with_node(hy2());
        store.settings.core_type = "xray".into();
        store.settings.runtime_source = "singbox:custom-profile".into();
        prepare_selected_core(Path::new("unused-test-path"), None, &mut store).unwrap();
        assert_eq!(store.settings.core_type, "xray");
        assert!(store.settings.preferred_core_type.is_none());
        store.settings.runtime_source = "generated".into();
        store.settings.core_type = "mihomo".into();
        let root = std::env::temp_dir().join(format!(
            "satelite-mihomo-policy-core-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let binary = bin_dir.join(CoreKind::Mihomo.binary_name());
        std::fs::write(&binary, b"fixture, never executed").unwrap();
        let checked = prepare_selected_core(&root, None, &mut store);
        std::fs::remove_file(&binary).unwrap();
        std::fs::remove_dir(&bin_dir).unwrap();
        std::fs::remove_dir(&root).unwrap();
        checked.unwrap();
        assert_eq!(store.settings.core_type, "mihomo");
    }

    #[test]
    fn xray_vless_hy2_vless_restores_the_user_preference() {
        let mut settings = AppSettings {
            core_type: "xray".into(),
            ..AppSettings::default()
        };
        assert_eq!(choose_core(&settings, &vless()).unwrap(), CoreKind::Xray);
        let selected = choose_core(&settings, &hy2()).unwrap();
        assert_eq!(selected, CoreKind::SingBox);
        settings.preferred_core_type = Some(settings.core_type.clone());
        settings.core_type = selected.as_str().into();
        assert_eq!(choose_core(&settings, &vless()).unwrap(), CoreKind::Xray);
    }

    #[test]
    fn plain_hy2_uses_singbox_automatically_and_explicit_xray_remains_possible() {
        let mut settings = AppSettings {
            core_type: "xray".into(),
            udp_tls_compat: false,
            ..AppSettings::default()
        };
        let mut node = hy2();
        node.tls.as_mut().unwrap().insecure = Some(false);
        assert_eq!(choose_core(&settings, &node).unwrap(), CoreKind::SingBox);
        settings
            .node_core_overrides
            .insert(node.id.clone(), "xray".into());
        assert_eq!(choose_core(&settings, &node).unwrap(), CoreKind::Xray);
        settings.udp_tls_compat = true;
        assert!(choose_core(&settings, &node).is_err());
    }

    #[test]
    fn node_overrides_protocol_default_without_changing_siblings() {
        let mut settings = AppSettings::default();
        let first = vless();
        let mut second = first.clone();
        second.id = "independent-node".into();
        settings
            .protocol_core_defaults
            .push(crate::domain::ProtocolCoreItem {
                protocol: "vless".into(),
                core: "xray".into(),
            });
        settings
            .node_core_overrides
            .insert(first.id.clone(), "singbox".into());
        assert_eq!(choose_core(&settings, &first).unwrap(), CoreKind::SingBox);
        assert_eq!(choose_core(&settings, &second).unwrap(), CoreKind::Xray);
        settings.node_core_overrides.remove(&first.id);
        assert_eq!(choose_core(&settings, &first).unwrap(), CoreKind::Xray);
    }

    #[test]
    fn explicit_invalid_or_incompatible_core_is_rejected_and_tls_is_preserved() {
        let original = hy2();
        assert!(validate_node_override(&original, "xray").is_err());
        assert!(validate_node_override(&original, "untrusted-core").is_err());
        assert!(validate_node_override(&original, "").is_err());
        assert_eq!(validate_node_override(&original, "auto").unwrap(), None);
        assert_eq!(original.tls.as_ref().unwrap().insecure, Some(true));
    }
}

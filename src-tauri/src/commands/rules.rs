use crate::config::{dump_rule_set_files, remove_rule_set_files};
use crate::domain::{Rule, RuleSet, RuleSetSummary, RuleTarget, RuleType};
use crate::state::AppState;
use serde::Deserialize;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Deserialize)]
pub struct SaveRuleInput {
    pub set_id: Option<String>,
    pub id: Option<String>,
    pub rule_type: RuleType,
    pub payload: String,
    pub target: RuleTarget,
    pub ord: Option<i32>,
    pub enabled: Option<bool>,
    /// Required when `target == node`.
    pub node_id: Option<String>,
    /// When `target == smart`: name must contain each keyword.
    #[serde(default)]
    pub smart_include: Option<Vec<String>>,
    /// When `target == smart`: name must not contain any keyword.
    #[serde(default)]
    pub smart_exclude: Option<Vec<String>>,
}

fn resource_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().resource_dir().ok()
}

/// Persist done; if core running, restart so route rules apply.
fn apply_running(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let res = resource_dir(app);
    match state.restart_if_running(res.as_deref()) {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Ok(()),
        Err(e) => Err(format!("已保存，但重启内核失败: {e}")),
    }
}

/// Write Clash `.list` for a set under app data.
fn dump_set(state: &AppState, set_id: &str) {
    let set = state
        .with_store(|s| Ok(s.get_rule_set(set_id).cloned()))
        .ok()
        .flatten();
    if let Some(set) = set {
        if let Err(e) = dump_rule_set_files(&state.app_data_dir, &set) {
            eprintln!("[satelite] dump rule files {set_id}: {e}");
        }
    }
}

#[tauri::command]
pub fn list_rule_sets(state: State<'_, AppState>) -> Result<Vec<RuleSetSummary>, String> {
    state
        .with_store(|store| Ok(store.list_rule_set_summaries()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_rule_set(state: State<'_, AppState>, id: String) -> Result<RuleSet, String> {
    state
        .with_store(|store| {
            store
                .get_rule_set(&id)
                .cloned()
                .ok_or_else(|| crate::error::AppError::NotFound(id))
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_active_rule_set(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    // Back-compat: enable this set (does not disable others).
    state
        .with_store_mut(|store| store.set_rule_set_enabled(&id, true))
        .map_err(|e| e.to_string())?;
    apply_running(&app, &state)
}

#[tauri::command]
pub fn set_rule_set_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .with_store_mut(|store| store.set_rule_set_enabled(&id, enabled))
        .map_err(|e| e.to_string())?;
    apply_running(&app, &state)
}

/// Reorder rule sets. `ids` is full preferred order (first = highest priority).
#[tauri::command]
pub fn reorder_rule_sets(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<RuleSetSummary>, String> {
    if ids.is_empty() {
        return Err("ids is empty".into());
    }
    state
        .with_store_mut(|store| store.reorder_rule_sets(&ids))
        .map_err(|e| e.to_string())?;
    // Order is already saved; restart failure must not revert UI order.
    let _ = apply_running(&app, &state);
    state
        .with_store(|store| Ok(store.list_rule_set_summaries()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_rule_set(state: State<'_, AppState>, name: String) -> Result<RuleSet, String> {
    let set = state
        .with_store_mut(|store| {
            let n = name.trim();
            if n.is_empty() {
                return Err(crate::error::AppError::Config("规则集名称不能为空".into()));
            }
            if n.chars().count() > 64 {
                return Err(crate::error::AppError::Config(
                    "规则集名称过长（最多 64 字）".into(),
                ));
            }
            // Avoid duplicate names (case-insensitive)
            if store
                .rule_sets
                .iter()
                .any(|s| s.name.eq_ignore_ascii_case(n))
            {
                return Err(crate::error::AppError::Config(format!(
                    "已存在同名规则集「{n}」"
                )));
            }
            Ok(store.create_rule_set(n))
        })
        .map_err(|e| e.to_string())?;
    dump_set(&state, &set.id);
    Ok(set)
}

#[tauri::command]
pub fn delete_rule_set(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .with_store_mut(|store| store.delete_rule_set(&id))
        .map_err(|e| e.to_string())?;
    remove_rule_set_files(&state.app_data_dir, &id);
    apply_running(&app, &state)
}

/// Reset one factory set (builtin-* or general-rules) from `resources/rules/{id}.list`.
#[tauri::command]
pub fn reset_rule_set(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<RuleSet, String> {
    let set = state
        .with_store_mut(|store| store.reset_rule_set(state.resource_dir.as_deref(), &id))
        .map_err(|e| e.to_string())?;
    dump_set(&state, &set.id);
    apply_running(&app, &state)?;
    Ok(set)
}

/// Legacy: reset all `builtin-*` sets (not general-rules).
#[tauri::command]
pub fn reset_builtin_rule_set(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuleSet, String> {
    let set = state
        .with_store_mut(|store| {
            store.reset_all_builtin_rule_sets(state.resource_dir.as_deref());
            store
                .get_rule_set(crate::domain::BUILTIN_SET_ID)
                .cloned()
                .or_else(|| store.rule_sets.iter().find(|s| s.builtin).cloned())
                .ok_or_else(|| crate::error::AppError::NotFound("builtin".into()))
        })
        .map_err(|e| e.to_string())?;
    // Dump every builtin factory set
    if let Ok(sets) = state.with_store(|s| {
        Ok(s.rule_sets
            .iter()
            .filter(|x| x.builtin)
            .cloned()
            .collect::<Vec<_>>())
    }) {
        for s in sets {
            let _ = crate::config::dump_rule_set_files(&state.app_data_dir, &s);
        }
    }
    apply_running(&app, &state)?;
    Ok(set)
}

/// List rules of a set (default: active set).
#[tauri::command]
pub fn list_rules(state: State<'_, AppState>, set_id: Option<String>) -> Result<Vec<Rule>, String> {
    state
        .with_store(|store| {
            let id = set_id.unwrap_or_else(|| {
                store
                    .rule_sets
                    .iter()
                    .find(|s| s.enabled)
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|| crate::domain::BUILTIN_SET_ID.into())
            });
            let set = store
                .get_rule_set(&id)
                .ok_or_else(|| crate::error::AppError::NotFound(id))?;
            let mut rules = set.rules.clone();
            rules.sort_by_key(|r| r.ord);
            Ok(rules)
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_rule(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SaveRuleInput,
) -> Result<Rule, String> {
    let rule = state
        .with_store_mut(|store| {
            if matches!(input.rule_type, RuleType::Geoip) {
                return Err(crate::error::AppError::Config(
                    "GEOIP 规则已不被 sing-box 1.12+ 支持，请改用 DOMAIN-SUFFIX / IP-CIDR".into(),
                ));
            }
            let payload = input.payload.trim().to_string();
            if payload.is_empty() {
                return Err(crate::error::AppError::Config("payload empty".into()));
            }
            let set_id = input.set_id.clone().unwrap_or_else(|| {
                store
                    .rule_sets
                    .iter()
                    .find(|s| s.id == crate::domain::GENERAL_SET_ID || s.enabled)
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|| crate::domain::GENERAL_SET_ID.into())
            });

            let set = store
                .get_rule_set(&set_id)
                .ok_or_else(|| crate::error::AppError::NotFound(set_id.clone()))?;

            let ord = input
                .ord
                .unwrap_or_else(|| set.rules.iter().map(|r| r.ord).max().unwrap_or(0) + 10);

            // Resolve pin fields for target=node (snapshot name for stale UI).
            let (node_id, node_name) = if matches!(input.target, RuleTarget::Node) {
                let nid = input
                    .node_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        crate::error::AppError::Config("指定节点出口需要选择一个节点".into())
                    })?;
                let stored = store
                    .nodes
                    .iter()
                    .find(|n| n.node.id == nid)
                    .ok_or_else(|| {
                        crate::error::AppError::Config(
                            "指定的节点不存在或已从订阅中移除，请重新选择".into(),
                        )
                    })?;
                (Some(stored.node.id.clone()), Some(stored.node.name.clone()))
            } else {
                (None, None)
            };

            let (smart_include, smart_exclude) = if matches!(input.target, RuleTarget::Smart) {
                let include =
                    Rule::normalize_keywords(input.smart_include.as_deref().unwrap_or(&[]));
                let exclude =
                    Rule::normalize_keywords(input.smart_exclude.as_deref().unwrap_or(&[]));
                let overlap = crate::domain::keyword_list_overlap(&include, &exclude);
                if !overlap.is_empty() {
                    return Err(crate::error::AppError::Config(format!(
                        "智能模式：关键字不能同时出现在白名单与黑名单中：{}",
                        overlap.join("、")
                    )));
                }
                let match_count = store
                    .enabled_nodes()
                    .iter()
                    .filter(|n| crate::domain::name_matches_keywords(&n.name, &include, &exclude))
                    .count();
                if match_count == 0 {
                    return Err(crate::error::AppError::Config(
                        "智能模式：当前没有符合关键字条件的节点，请调整白名单/黑名单或先导入订阅"
                            .into(),
                    ));
                }
                (include, exclude)
            } else {
                (Vec::new(), Vec::new())
            };

            let rule = if let Some(id) = input.id.clone() {
                if let Some(existing) = set.rules.iter().find(|r| r.id == id) {
                    let mut r = existing.clone();
                    r.rule_type = input.rule_type;
                    r.payload = payload;
                    r.target = input.target;
                    r.ord = ord;
                    r.node_id = node_id;
                    r.node_name = node_name;
                    r.smart_include = smart_include;
                    r.smart_exclude = smart_exclude;
                    if let Some(en) = input.enabled {
                        r.enabled = en;
                    }
                    r
                } else {
                    let mut r = Rule::new(input.rule_type, payload, input.target, ord);
                    r.id = id;
                    r.node_id = node_id;
                    r.node_name = node_name;
                    r.smart_include = smart_include;
                    r.smart_exclude = smart_exclude;
                    if let Some(en) = input.enabled {
                        r.enabled = en;
                    }
                    r
                }
            } else {
                let mut r = Rule::new(input.rule_type, payload, input.target, ord);
                r.node_id = node_id;
                r.node_name = node_name;
                r.smart_include = smart_include;
                r.smart_exclude = smart_exclude;
                if matches!(input.target, RuleTarget::Smart) {
                    r.id = Rule::compute_id(
                        r.rule_type,
                        &r.payload,
                        r.target,
                        None,
                        &r.smart_include,
                        &r.smart_exclude,
                    );
                }
                if let Some(en) = input.enabled {
                    r.enabled = en;
                }
                r
            };

            store.upsert_rule_in_set(&set_id, rule)
        })
        .map_err(|e| e.to_string())?;
    // Dual files: Clash route list + optional SYSTEM DNS sidecar.
    if let Some(sid) = rule_set_id_of(&state, &rule) {
        dump_set(&state, &sid);
    } else if let Some(sid) = input.set_id.as_deref() {
        dump_set(&state, sid);
    }
    apply_running(&app, &state)?;
    // Best-effort: pick best node for new/updated smart rule after core restarts.
    if matches!(rule.target, RuleTarget::Smart) && rule.enabled {
        let r = rule.clone();
        let app2 = app.clone();
        tauri::async_runtime::spawn(async move {
            // Wait for restart/clash_api to come up.
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            if let Some(state) = app2.try_state::<AppState>() {
                let _ = crate::smart_switch::refresh_smart_rule_now(&state, &r).await;
            }
        });
    }
    Ok(rule)
}

fn rule_set_id_of(state: &AppState, rule: &Rule) -> Option<String> {
    state
        .with_store(|store| {
            Ok(store
                .rule_sets
                .iter()
                .find(|s| s.rules.iter().any(|r| r.id == rule.id))
                .map(|s| s.id.clone()))
        })
        .ok()
        .flatten()
}

#[tauri::command]
pub fn remove_rule(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    set_id: Option<String>,
) -> Result<(), String> {
    let sid = set_id.unwrap_or_else(|| crate::domain::GENERAL_SET_ID.into());
    state
        .with_store_mut(|store| store.remove_rule_from_set(&sid, &id))
        .map_err(|e| e.to_string())?;
    dump_set(&state, &sid);
    apply_running(&app, &state)
}

#[tauri::command]
pub fn set_rule_enabled(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
    set_id: Option<String>,
) -> Result<Rule, String> {
    let sid = set_id.unwrap_or_else(|| crate::domain::GENERAL_SET_ID.into());
    let rule = state
        .with_store_mut(|store| {
            let set = store
                .rule_sets
                .iter_mut()
                .find(|s| s.id == sid)
                .ok_or_else(|| crate::error::AppError::NotFound(sid.clone()))?;
            let rule = set
                .rules
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or_else(|| crate::error::AppError::NotFound(id))?;
            rule.enabled = enabled;
            Ok(rule.clone())
        })
        .map_err(|e| e.to_string())?;
    dump_set(&state, &sid);
    apply_running(&app, &state)?;
    Ok(rule)
}

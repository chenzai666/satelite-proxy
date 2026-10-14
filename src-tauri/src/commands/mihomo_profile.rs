use crate::domain::MihomoTemplate;
use crate::state::AppState;
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Serialize)]
pub struct MihomoTemplateView {
    template: MihomoTemplate,
    use_for_active: bool,
    group_order: Vec<String>,
}

fn view(store: &crate::storage::AppStore) -> MihomoTemplateView {
    MihomoTemplateView {
        template: store.mihomo_template.clone(),
        use_for_active: store
            .subscriptions
            .iter()
            .any(|s| s.enabled && store.mihomo_template_subscriptions.contains(&s.id)),
        group_order: store.mihomo_group_order.clone(),
    }
}

#[tauri::command(async)]
pub fn get_mihomo_template(state: State<'_, AppState>) -> Result<MihomoTemplateView, String> {
    state
        .with_store(|store| Ok(view(store)))
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn save_mihomo_template(
    app: AppHandle,
    state: State<'_, AppState>,
    template: MihomoTemplate,
    use_for_active: bool,
) -> Result<MihomoTemplateView, String> {
    crate::config::mihomo_template::render_template(&template, &[])?;
    let result = state
        .with_store_mut(|store| {
            store.mihomo_template = template;
            if use_for_active && store.settings.core_type == "mihomo" {
                store.settings.auto_select = crate::domain::AutoSelectMode::Off;
            }
            for sub in &store.subscriptions {
                if sub.enabled && sub.source.contributes_nodes() {
                    if use_for_active {
                        store.mihomo_template_subscriptions.insert(sub.id.clone());
                    } else {
                        store.mihomo_template_subscriptions.remove(&sub.id);
                    }
                }
            }
            Ok(view(store))
        })
        .map_err(|e| e.to_string())?;
    crate::rule_apply::request_restart(app, Vec::new());
    Ok(result)
}

#[tauri::command(async)]
pub fn reorder_mihomo_groups(
    state: State<'_, AppState>,
    ids: Vec<String>,
) -> Result<Vec<String>, String> {
    if ids.len() > 256
        || ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 512 || id.contains(['\r', '\n']))
    {
        return Err("策略组顺序无效".into());
    }
    let mut unique = std::collections::HashSet::new();
    if ids.iter().any(|id| !unique.insert(id.clone())) {
        return Err("策略组顺序包含重复名称".into());
    }
    state
        .with_store_mut(|store| {
            let mut next = ids;
            next.extend(
                store
                    .mihomo_group_order
                    .iter()
                    .filter(|name| !unique.contains(*name))
                    .cloned(),
            );
            next.truncate(256);
            store.mihomo_group_order = next;
            Ok(store.mihomo_group_order.clone())
        })
        .map_err(|e| e.to_string())
}

/// Fetch a candidate only. The user can inspect it before saving; a failed
/// fetch/parse never replaces the last valid policy or the running core.
#[tauri::command]
pub async fn fetch_mihomo_template(app: AppHandle, url: String) -> Result<String, String> {
    let parsed = url::Url::parse(url.trim()).map_err(|_| "INI 模板 URL 无效")?;
    if !["http", "https"].contains(&parsed.scheme())
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("INI 模板必须是无内嵌凭据的 HTTP/HTTPS URL".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;
        let state = app.try_state::<AppState>().ok_or("app state unavailable")?;
        let mut builder = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(20))
            .redirects(5);
        if state.is_core_running() {
            let port = state
                .with_store(|store| Ok(store.settings.mixed_port))
                .map_err(|e| e.to_string())?;
            builder = builder.proxy(
                ureq::Proxy::new(format!("http://127.0.0.1:{port}"))
                    .map_err(|_| "模板下载代理配置无效")?,
            );
        }
        let response = builder
            .build()
            .get(parsed.as_str())
            .call()
            .map_err(|_| "INI 模板下载失败，旧配置仍保留")?;
        let mut body = String::new();
        response
            .into_reader()
            .take(512 * 1024 + 1)
            .read_to_string(&mut body)
            .map_err(|_| "INI 模板读取失败")?;
        let mut candidate = state
            .with_store(|store| Ok(store.mihomo_template.clone()))
            .map_err(|e| e.to_string())?;
        candidate.ini = body.clone();
        crate::config::mihomo_template::render_template(&candidate, &[])?;
        Ok(body)
    })
    .await
    .map_err(|e| format!("模板下载任务失败：{e}"))?
}

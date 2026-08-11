//! Download remote rule sets in the app, so sing-box only loads local files.

use crate::domain::RuleSet;
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

const EVENT: &str = "remote-rule-set-status";
const MAX_BYTES: usize = 32 * 1024 * 1024;
const REFRESH_SECS: i64 = 60 * 60;
const TICK_SECS: u64 = 60;

static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Clone, Serialize)]
struct StatusEvent {
    id: String,
    status: String,
    error: Option<String>,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn emit(app: &AppHandle, id: &str, status: &str, error: Option<String>) {
    let _ = app.emit(
        EVENT,
        StatusEvent {
            id: id.to_string(),
            status: status.to_string(),
            error,
        },
    );
}

pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        loop {
            let due = due_ids(&app);
            for id in due {
                if let Err(error) = refresh(app.clone(), id.clone()).await {
                    crate::app_log::warn("remote_rules", format!("refresh {id} failed: {error}"));
                }
            }
            tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
        }
    });
}

fn due_ids(app: &AppHandle) -> Vec<String> {
    let Some(state) = app.try_state::<AppState>() else {
        return Vec::new();
    };
    let now = now_secs();
    state
        .with_store(|store| {
            Ok(store
                .rule_sets
                .iter()
                .filter_map(|set| {
                    let remote = set.remote.as_ref()?;
                    let due = remote.download_status == "downloading"
                        || remote.local_path.is_none()
                        || now.saturating_sub(remote.last_attempt.unwrap_or(0)) >= REFRESH_SECS;
                    due.then(|| set.id.clone())
                })
                .collect())
        })
        .unwrap_or_default()
}

pub async fn refresh(app: AppHandle, id: String) -> Result<RuleSet, String> {
    {
        let mut active = ACTIVE
            .get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .map_err(|_| "remote rule download lock poisoned".to_string())?;
        if !active.insert(id.clone()) {
            return Err("该远程规则集正在下载".into());
        }
    }

    let result = refresh_inner(&app, &id).await;
    if let Ok(mut active) = ACTIVE.get_or_init(|| Mutex::new(HashSet::new())).lock() {
        active.remove(&id);
    }
    result
}

async fn refresh_inner(app: &AppHandle, id: &str) -> Result<RuleSet, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "app state unavailable".to_string())?;
    let attempt = now_secs();
    let use_proxy = state.is_core_running();
    let (url, mixed_port) = state
        .with_store_mut(|store| {
            let mixed_port = store.settings.mixed_port;
            let set = store
                .rule_sets
                .iter_mut()
                .find(|set| set.id == id)
                .ok_or_else(|| crate::error::AppError::NotFound(id.to_string()))?;
            let remote = set
                .remote
                .as_mut()
                .ok_or_else(|| crate::error::AppError::Config("该规则集不是远程规则集".into()))?;
            remote.download_status = "downloading".into();
            remote.download_error = None;
            remote.last_attempt = Some(attempt);
            Ok((remote.url.clone(), mixed_port))
        })
        .map_err(|error| error.to_string())?;
    emit(app, id, "downloading", None);

    let bytes = match download(&url, use_proxy.then_some(mixed_port)).await {
        Ok(bytes) => Ok(bytes),
        Err(first) if use_proxy => download(&url, None)
            .await
            .map_err(|second| format!("代理下载失败: {first}; 直连下载失败: {second}")),
        Err(error) => Err(error),
    };

    let bytes = match bytes {
        Ok(bytes) => bytes,
        Err(error) => return fail(app, id, error),
    };
    let rule_count = match validate_source(&bytes) {
        Ok(count) => count,
        Err(error) => return fail(app, id, error),
    };

    let cache_dir = match app.path().app_data_dir() {
        Ok(path) => path.join("remote-rule-sets"),
        Err(error) => return fail(app, id, error.to_string()),
    };
    let safe_id: String = id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let path = cache_dir.join(format!("{safe_id}-{attempt}.json"));
    let write_path = path.clone();
    let write_dir = cache_dir.clone();
    let write_result = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&write_dir).map_err(|error| error.to_string())?;
        std::fs::write(&write_path, bytes).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    if let Err(error) = write_result {
        return fail(app, id, error);
    }

    let path_text = path.to_string_lossy().to_string();
    let updated = state
        .with_store_mut(|store| {
            let set = store
                .rule_sets
                .iter_mut()
                .find(|set| set.id == id)
                .ok_or_else(|| crate::error::AppError::NotFound(id.to_string()))?;
            let remote = set
                .remote
                .as_mut()
                .ok_or_else(|| crate::error::AppError::Config("该规则集不是远程规则集".into()))?;
            let old_path = remote.local_path.replace(path_text);
            remote.download_status = "ready".into();
            remote.download_error = None;
            remote.last_update = Some(attempt);
            remote.rule_count = Some(rule_count);
            Ok((set.clone(), old_path))
        })
        .map_err(|error| error.to_string());
    let (set, old_path) = match updated {
        Ok(updated) => updated,
        Err(error) => return fail(app, id, error),
    };

    // The cache is ready even if applying it to a currently running core later
    // fails. Tell the UI to stop spinning and surface restart failure separately.
    emit(app, id, "ready", None);

    let restart_app = app.clone();
    let restart_result = tauri::async_runtime::spawn_blocking(move || {
        let state = restart_app
            .try_state::<AppState>()
            .ok_or_else(|| "app state unavailable".to_string())?;
        let resource_dir = restart_app.path().resource_dir().ok();
        state
            .restart_if_running(resource_dir.as_deref())
            .map_err(|error| format!("规则已下载，但重启内核失败: {error}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result);
    if let Err(error) = restart_result {
        return Err(error);
    }

    if let Some(old_path) = old_path.filter(|old| old != &path.to_string_lossy()) {
        let old = std::path::PathBuf::from(old_path);
        if old.parent() == Some(cache_dir.as_path()) {
            let _ = std::fs::remove_file(old);
        }
    }
    Ok(set)
}

async fn download(url: &str, proxy_port: Option<u16>) -> Result<Vec<u8>, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .user_agent("Satelite/1 remote-rule-set");
    if let Some(port) = proxy_port {
        builder = builder.proxy(
            reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).map_err(|e| e.to_string())?,
        );
    }
    let response = builder
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    if response.content_length().unwrap_or(0) > MAX_BYTES as u64 {
        return Err("远程规则集超过 32 MB".into());
    }
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("远程规则集超过 32 MB".into());
    }
    Ok(bytes.to_vec())
}

fn validate_source(bytes: &[u8]) -> Result<u32, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("远程规则集不是有效的 sing-box source JSON: {error}"))?;
    let rules = value
        .get("rules")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "远程规则集缺少 rules 数组".to_string())?;
    if rules.is_empty() {
        return Err("远程规则集 rules 为空".into());
    }
    u32::try_from(rules.len()).map_err(|_| "远程规则集条目数量过多".to_string())
}

fn fail(app: &AppHandle, id: &str, error: String) -> Result<RuleSet, String> {
    if let Some(state) = app.try_state::<AppState>() {
        let _ = state.with_store_mut(|store| {
            if let Some(remote) = store
                .rule_sets
                .iter_mut()
                .find(|set| set.id == id)
                .and_then(|set| set.remote.as_mut())
            {
                remote.download_status = "error".into();
                remote.download_error = Some(error.clone());
            }
            Ok(())
        });
    }
    emit(app, id, "error", Some(error.clone()));
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_sing_box_source_json() {
        assert_eq!(
            validate_source(br#"{"version":3,"rules":[{"domain_suffix":["example.com"]}]}"#),
            Ok(1)
        );
    }

    #[test]
    fn rejects_html_and_empty_rules() {
        assert!(validate_source(b"<html>not a rule set</html>").is_err());
        assert!(validate_source(br#"{"version":3,"rules":[]}"#).is_err());
    }
}

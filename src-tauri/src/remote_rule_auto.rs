//! Download remote rule sets in the app, so sing-box only loads local files.

use crate::domain::RuleSet;
use crate::state::AppState;
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

const EVENT: &str = "remote-rule-set-status";
const MAX_BYTES: usize = 32 * 1024 * 1024;
const TICK_SECS: u64 = 60;

static ACTIVE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Clone, Copy)]
enum RuleSetFileFormat {
    Source,
    Binary,
}

impl RuleSetFileFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Binary => "binary",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Source => "json",
            Self::Binary => "srs",
        }
    }
}

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
            let mut changed = false;
            let mut cleanup_after_apply = Vec::new();
            for id in due {
                match refresh_download(app.clone(), id.clone()).await {
                    Ok(downloaded) => {
                        changed = true;
                        cleanup_after_apply.extend(downloaded.cleanup_after_apply);
                    }
                    Err(error) => {
                        crate::app_log::warn(
                            "remote_rules",
                            format!("refresh {id} failed: {error}"),
                        );
                    }
                }
            }
            // Apply the entire due set with one restart instead of restarting
            // once after every sequential download.
            if changed {
                crate::rule_apply::request_restart(app.clone(), cleanup_after_apply);
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
                    let interval =
                        crate::domain::remote_update_interval_secs(&remote.update_interval);
                    let due = interval.is_some_and(|seconds| {
                        remote.download_status == "downloading"
                            || remote.local_path.is_none()
                            || now.saturating_sub(remote.last_attempt.unwrap_or(0)) >= seconds
                    });
                    due.then(|| set.id.clone())
                })
                .collect())
        })
        .unwrap_or_default()
}

pub async fn refresh(app: AppHandle, id: String) -> Result<RuleSet, String> {
    let downloaded = refresh_download(app.clone(), id).await?;
    crate::rule_apply::request_restart(app, downloaded.cleanup_after_apply);
    Ok(downloaded.set)
}

struct DownloadedRule {
    set: RuleSet,
    cleanup_after_apply: Vec<std::path::PathBuf>,
}

async fn refresh_download(app: AppHandle, id: String) -> Result<DownloadedRule, String> {
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

async fn refresh_inner(app: &AppHandle, id: &str) -> Result<DownloadedRule, String> {
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
    let (format, source_rule_count) = match validate_source(&bytes) {
        Ok(count) => (RuleSetFileFormat::Source, Some(count)),
        Err(_) if bytes.starts_with(b"SRS") => (RuleSetFileFormat::Binary, None),
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
    let path = cache_dir.join(format!("{safe_id}-{attempt}.{}", format.extension()));
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

    let rule_count = match source_rule_count {
        Some(count) => count,
        None => {
            let resource_dir = app.path().resource_dir().ok();
            let (core, _) =
                crate::core::resolve_core_bin(&state.app_data_dir, resource_dir.as_deref());
            let Some(core) = core else {
                let _ = std::fs::remove_file(&path);
                return fail(app, id, "无法校验 SRS：sing-box 内核不可用".into());
            };
            let input = path.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                let source = decompile_srs(&core, &input)?;
                validate_source(&source)
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            match result {
                Ok(count) => count,
                Err(error) => {
                    let _ = std::fs::remove_file(&path);
                    return fail(app, id, error);
                }
            }
        }
    };

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
            remote.format = format.as_str().to_string();
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

    let mut cleanup_after_apply = Vec::new();
    if let Some(old_path) = old_path.filter(|old| old != &path.to_string_lossy()) {
        let old = std::path::PathBuf::from(old_path);
        if old.parent() == Some(cache_dir.as_path()) {
            cleanup_after_apply.push(old);
        }
    }
    Ok(DownloadedRule {
        set,
        cleanup_after_apply,
    })
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
    let count = rules
        .iter()
        .try_fold(0usize, |total, rule| {
            total.checked_add(crate::domain::remote_rule_display_count(rule))
        })
        .ok_or_else(|| "远程规则集条目数量过多".to_string())?;
    u32::try_from(count).map_err(|_| "远程规则集条目数量过多".to_string())
}

/// Decompile and validate a binary `.srs` with the active sing-box core.
/// The temporary JSON is created beside the input and always removed.
pub(crate) fn decompile_srs(core: &Path, input: &Path) -> Result<Vec<u8>, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let output = input.with_extension(format!("decompiled-{}-{stamp}.json", std::process::id()));
    let mut command = Command::new(core);
    command
        .arg("rule-set")
        .arg("decompile")
        .arg(input)
        .arg("-o")
        .arg(&output);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let result = command
        .output()
        .map_err(|error| format!("无法运行 sing-box 校验 SRS: {error}"));
    let bytes = match result {
        Ok(result) if result.status.success() => {
            std::fs::read(&output).map_err(|error| format!("无法读取 SRS 反编译结果: {error}"))
        }
        Ok(result) => Err(format!(
            "SRS 校验失败: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        )),
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_file(output);
    bytes
}

fn fail<T>(app: &AppHandle, id: &str, error: String) -> Result<T, String> {
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
    fn counts_expanded_matcher_values() {
        assert_eq!(
            validate_source(
                br#"{"version":3,"rules":[{"domain_suffix":["a.com","b.com"],"ip_cidr":["10.0.0.0/8"]}]}"#
            ),
            Ok(3)
        );
    }

    #[test]
    fn rejects_html_and_empty_rules() {
        assert!(validate_source(b"<html>not a rule set</html>").is_err());
        assert!(validate_source(br#"{"version":3,"rules":[]}"#).is_err());
    }

    #[test]
    fn decompiles_binary_srs_with_bundled_core_when_available() {
        let Some(core) = crate::core::find_bundled_core(None) else {
            return;
        };
        const SRS: &[u8] = &[
            0x53, 0x52, 0x53, 0x02, 0x78, 0xda, 0x62, 0x64, 0x60, 0x62, 0x60, 0x64, 0x00, 0x03,
            0x01, 0x08, 0x83, 0x71, 0xd5, 0xaa, 0x55, 0x3c, 0xb9, 0xf9, 0xc9, 0x7a, 0xa9, 0x39,
            0x05, 0xb9, 0x89, 0x15, 0xa9, 0x5c, 0xff, 0x19, 0x00, 0x01, 0x00, 0x00, 0xff, 0xff,
            0x4d, 0xcc, 0x07, 0x83,
        ];
        let path = std::env::temp_dir().join(format!(
            "satelite-test-{}-{}.srs",
            std::process::id(),
            now_secs()
        ));
        std::fs::write(&path, SRS).unwrap();
        let result = decompile_srs(&core, &path).and_then(|bytes| validate_source(&bytes));
        let _ = std::fs::remove_file(path);
        assert_eq!(result, Ok(1));
    }
}

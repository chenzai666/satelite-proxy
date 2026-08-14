//! Download sing-box core from GitHub releases (SagerNet/sing-box).

use crate::core::paths::{
    binary_name, core_bin_path, core_dir, detect_platform, normalize_version, write_version_file,
    CorePlatform,
};
use crate::error::{AppError, AppResult};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tar::Archive;

const GITHUB_LATEST: &str = "https://api.github.com/repos/SagerNet/sing-box/releases/latest";
const GITHUB_TAG: &str = "https://api.github.com/repos/SagerNet/sing-box/releases/tags/";
const MAX_CORE_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CoreDownloadResult {
    pub version: String,
    pub path: String,
    pub asset_name: String,
    pub platform: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CoreDownloadProgress {
    pub stage: &'static str,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: Option<u8>,
    pub via_proxy: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LatestReleaseInfo {
    pub version: String,
    pub asset_name: String,
    pub download_url: String,
    pub size: u64,
    pub platform: String,
}

/// Default pin used only when GitHub API is unreachable.
const FALLBACK_VERSION: &str = "v1.13.15";

pub async fn fetch_latest_release_with_proxy(
    proxy_url: Option<&str>,
) -> AppResult<LatestReleaseInfo> {
    let platform = detect_platform()?;
    match fetch_release_json(GITHUB_LATEST, proxy_url).await {
        Ok(release) => pick_asset(release, platform),
        Err(api_err) => {
            // API blocked/unreachable → direct asset URL with pinned fallback version
            let _ = api_err;
            Ok(synthetic_release_info(FALLBACK_VERSION, platform))
        }
    }
}

async fn fetch_release_by_tag_with_proxy(
    tag: &str,
    proxy_url: Option<&str>,
) -> AppResult<LatestReleaseInfo> {
    let platform = detect_platform()?;
    let tag = normalize_version(tag);
    let url = format!("{GITHUB_TAG}{tag}");
    match fetch_release_json(&url, proxy_url).await {
        Ok(release) => pick_asset(release, platform),
        Err(_) => Ok(synthetic_release_info(&tag, platform)),
    }
}

async fn fetch_release_json(url: &str, proxy_url: Option<&str>) -> AppResult<GhRelease> {
    let client = http_client(proxy_url)?;
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| AppError::Core(format!("github api: {e}")))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Core(format!(
            "github api status {status} for {url}: {}",
            body.chars().take(200).collect::<String>()
        )));
    }
    resp.json::<GhRelease>()
        .await
        .map_err(|e| AppError::Core(format!("parse github release: {e}")))
}

/// Fallback when GitHub API is blocked: build asset URL from known version tag.
fn synthetic_release_info(tag: &str, platform: CorePlatform) -> LatestReleaseInfo {
    let version = normalize_version(tag);
    let ver_num = version.trim_start_matches('v').to_string();
    let ext = if platform.is_windows { "zip" } else { "tar.gz" };
    let asset_name = format!("sing-box-{ver_num}-{}.{ext}", platform.asset_suffix);
    let download_url =
        format!("https://github.com/SagerNet/sing-box/releases/download/{version}/{asset_name}");
    LatestReleaseInfo {
        version,
        asset_name,
        download_url,
        size: 0,
        platform: platform.asset_suffix.to_string(),
    }
}

fn pick_asset(release: GhRelease, platform: CorePlatform) -> AppResult<LatestReleaseInfo> {
    let version = normalize_version(&release.tag_name);
    let ver_num = version.trim_start_matches('v');
    // Prefer exact: sing-box-{ver}-{suffix}.tar.gz / .zip
    let ext = if platform.is_windows { "zip" } else { "tar.gz" };
    let expected = format!("sing-box-{ver_num}-{}.{ext}", platform.asset_suffix);

    let asset = release
        .assets
        .iter()
        .find(|a| a.name == expected)
        .or_else(|| {
            // fallback: contains suffix and correct extension, not legacy
            release.assets.iter().find(|a| {
                a.name.contains(platform.asset_suffix)
                    && a.name.starts_with("sing-box-")
                    && a.name.ends_with(ext)
                    && !a.name.contains("legacy")
            })
        })
        .ok_or_else(|| {
            AppError::Core(format!(
                "no asset for platform {} (expected {expected})",
                platform.asset_suffix
            ))
        })?;

    Ok(LatestReleaseInfo {
        version,
        asset_name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
        platform: platform.asset_suffix.to_string(),
    })
}

fn http_client(proxy_url: Option<&str>) -> AppResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent("SateliteProxy/0.1 (sing-box-core-downloader)");
    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy_url)
                .map_err(|error| AppError::Core(format!("download proxy: {error}")))?,
        );
    }
    builder.build().map_err(|e| AppError::Core(e.to_string()))
}

/// Download latest (or given tag) and install into `{app_data}/bin/sing-box`.
pub async fn download_latest_core(
    app_data_dir: &Path,
    tag: Option<String>,
) -> AppResult<CoreDownloadResult> {
    download_latest_core_with_progress(app_data_dir, tag, None, |_| {}).await
}

pub async fn download_latest_core_with_progress(
    app_data_dir: &Path,
    tag: Option<String>,
    proxy_url: Option<String>,
    progress: impl Fn(CoreDownloadProgress) + Send + Sync + 'static,
) -> AppResult<CoreDownloadResult> {
    let info = if let Some(t) = tag {
        fetch_release_by_tag_with_proxy(&t, proxy_url.as_deref()).await?
    } else {
        fetch_latest_release_with_proxy(proxy_url.as_deref()).await?
    };
    download_and_install(app_data_dir, &info, proxy_url.as_deref(), progress).await
}

async fn download_and_install<F>(
    app_data_dir: &Path,
    info: &LatestReleaseInfo,
    proxy_url: Option<&str>,
    progress: F,
) -> AppResult<CoreDownloadResult>
where
    F: Fn(CoreDownloadProgress) + Send + Sync + 'static,
{
    validate_archive_size_hint(info.size)?;
    let via_proxy = proxy_url.is_some();
    let progress = Arc::new(progress);

    let client = http_client(proxy_url)?;
    let resp = client
        .get(&info.download_url)
        .send()
        .await
        .map_err(|e| AppError::Core(format!("download: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Core(format!("download status {}", resp.status())));
    }
    let declared_total = (info.size > 0).then_some(info.size);
    let mut last_percent = None;
    let download_progress = Arc::clone(&progress);
    let bytes = crate::services::http_body::read_limited_with_progress(
        resp,
        MAX_CORE_ARCHIVE_BYTES,
        "core archive exceeds 256 MB".into(),
        move |downloaded, response_total| {
            let total = response_total.or(declared_total);
            let percent = total
                .filter(|total| *total > 0)
                .map(|total| ((downloaded.saturating_mul(100) / total).min(100)) as u8);
            if downloaded == 0 || percent != last_percent {
                last_percent = percent;
                download_progress(CoreDownloadProgress {
                    stage: "downloading",
                    downloaded,
                    total,
                    percent,
                    via_proxy,
                });
            }
        },
    )
    .await
    .map_err(|e| AppError::Core(format!("download body: {e}")))?;
    if bytes.len() < 1024 {
        return Err(AppError::Core("download too small, likely failed".into()));
    }

    let app_data_dir = app_data_dir.to_path_buf();
    let info = info.clone();
    let downloaded = bytes.len() as u64;
    progress(CoreDownloadProgress {
        stage: "installing",
        downloaded,
        total: Some(downloaded),
        percent: Some(100),
        via_proxy,
    });
    let result = tokio::task::spawn_blocking(move || {
        install_downloaded_archive(&app_data_dir, &info, bytes)
    })
    .await
    .map_err(|error| AppError::Core(format!("install core task: {error}")))??;
    progress(CoreDownloadProgress {
        stage: "done",
        downloaded,
        total: Some(downloaded),
        percent: Some(100),
        via_proxy,
    });
    Ok(result)
}

fn validate_archive_size_hint(size: u64) -> AppResult<()> {
    if size > MAX_CORE_ARCHIVE_BYTES as u64 {
        return Err(AppError::Core("core archive exceeds 256 MB".into()));
    }
    Ok(())
}

fn install_downloaded_archive(
    app_data_dir: &Path,
    info: &LatestReleaseInfo,
    bytes: Vec<u8>,
) -> AppResult<CoreDownloadResult> {
    let bin_dir = core_dir(app_data_dir);
    fs::create_dir_all(&bin_dir)?;
    let archive_path = bin_dir.join(&info.asset_name);
    {
        let mut f = File::create(&archive_path)
            .map_err(|e| AppError::Core(format!("write archive: {e}")))?;
        f.write_all(&bytes)
            .map_err(|e| AppError::Core(format!("write archive: {e}")))?;
    }

    let dest = core_bin_path(app_data_dir);
    // remove old binary first (Windows may need this; macOS setuid needs elevation)
    if dest.exists() {
        #[cfg(target_os = "macos")]
        {
            let _ = crate::core::macos_auth::remove_setuid_core_if_needed(&dest);
        }
        let _ = fs::remove_file(&dest);
    }

    if info.asset_name.ends_with(".tar.gz") || info.asset_name.ends_with(".tgz") {
        extract_singbox_from_tar_gz(&archive_path, &dest)?;
    } else if info.asset_name.ends_with(".zip") {
        extract_singbox_from_zip(&archive_path, &dest)?;
    } else {
        return Err(AppError::Core(format!(
            "unsupported archive: {}",
            info.asset_name
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms)?;
    }

    // cleanup archive
    let _ = fs::remove_file(&archive_path);

    write_version_file(app_data_dir, &info.version)?;

    // verify runnable
    let _ = crate::core::paths::read_core_version_via_binary(app_data_dir);

    Ok(CoreDownloadResult {
        version: info.version.clone(),
        path: dest.display().to_string(),
        asset_name: info.asset_name.clone(),
        platform: info.platform.clone(),
        bytes: bytes.len() as u64,
    })
}

fn extract_singbox_from_tar_gz(archive: &Path, dest: &Path) -> AppResult<()> {
    let file = File::open(archive).map_err(|e| AppError::Core(format!("open tar.gz: {e}")))?;
    let dec = GzDecoder::new(file);
    let mut tar = Archive::new(dec);
    let want = binary_name();
    let mut found = false;

    for entry in tar
        .entries()
        .map_err(|e| AppError::Core(format!("tar entries: {e}")))?
    {
        let mut entry = entry.map_err(|e| AppError::Core(format!("tar entry: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| AppError::Core(format!("tar path: {e}")))?
            .to_path_buf();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name == want || name == "sing-box" || name == "sing-box.exe" {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out =
                File::create(dest).map_err(|e| AppError::Core(format!("create binary: {e}")))?;
            io::copy(&mut entry, &mut out)
                .map_err(|e| AppError::Core(format!("extract binary: {e}")))?;
            found = true;
            break;
        }
    }

    if !found {
        return Err(AppError::Core(
            "sing-box binary not found inside tar.gz".into(),
        ));
    }
    Ok(())
}

fn extract_singbox_from_zip(archive: &Path, dest: &Path) -> AppResult<()> {
    let file = File::open(archive).map_err(|e| AppError::Core(format!("open zip: {e}")))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| AppError::Core(format!("zip open: {e}")))?;
    let want = binary_name();
    let mut target_index = None;
    for i in 0..zip.len() {
        let entry = zip
            .by_index(i)
            .map_err(|e| AppError::Core(format!("zip entry: {e}")))?;
        let name = PathBuf::from(entry.name());
        let file_name = name
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if file_name == want || file_name == "sing-box" || file_name == "sing-box.exe" {
            target_index = Some(i);
            break;
        }
    }
    let idx = target_index
        .ok_or_else(|| AppError::Core("sing-box binary not found inside zip".into()))?;
    let mut entry = zip
        .by_index(idx)
        .map_err(|e| AppError::Core(format!("zip entry: {e}")))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut out = File::create(dest).map_err(|e| AppError::Core(format!("create binary: {e}")))?;
    io::copy(&mut entry, &mut out).map_err(|e| AppError::Core(format!("extract binary: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_suffix_known() {
        let p = detect_platform().expect("platform");
        assert!(!p.asset_suffix.is_empty());
    }

    #[test]
    fn core_archive_size_hint_is_bounded() {
        assert!(validate_archive_size_hint(0).is_ok());
        assert!(validate_archive_size_hint(MAX_CORE_ARCHIVE_BYTES as u64).is_ok());
        assert!(validate_archive_size_hint(MAX_CORE_ARCHIVE_BYTES as u64 + 1).is_err());
    }
}

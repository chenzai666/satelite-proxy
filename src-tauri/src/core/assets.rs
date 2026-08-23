//! geosite.dat / geoip.dat asset management for the Xray core.
//!
//! Xray resolves `geosite:` / `geoip:` matchers (and `geoip:private`) through
//! plain `.dat` files located via `XRAY_LOCATION_ASSET` (set to the app-data
//! `bin/` dir by `CoreKind::spawn_env`). Sources, in order:
//! 1. Already staged in app data (`bin/geosite.dat`, `bin/geoip.dat`) —
//!    staged automatically when the bundled Xray is copied in
//!    (`paths::stage_bundled_core`) or when the core zip is downloaded
//!    (`download::extract_from_zip`).
//! 2. Bundled with the app (`resources/bin/<plat>/*.dat`).
//! 3. Network download from Loyalsoldier/v2ray-rules-dat (v2rayN's source).

use crate::error::{AppError, AppResult};
use std::io::Read;
use std::path::Path;

const GEODATA_FILES: [&str; 2] = ["geosite.dat", "geoip.dat"];
const GEODATA_BASE_URL: &str =
    "https://github.com/Loyalsoldier/v2ray-rules-dat/releases/latest/download";
/// dat files are ~10-60 MB each; anything larger is a corrupt/hijacked body.
const MAX_DAT_BYTES: u64 = 128 * 1024 * 1024;

pub fn geodata_present(app_data_dir: &Path) -> bool {
    let bin = crate::core::paths::core_dir(app_data_dir);
    GEODATA_FILES.iter().all(|f| bin.join(f).is_file())
}

/// Stage missing dat files from the bundled resources. Returns true when all
/// files are present afterwards.
pub fn stage_bundled_geodata(app_data_dir: &Path, resource_dir: Option<&Path>) -> bool {
    let bin = crate::core::paths::core_dir(app_data_dir);
    let Some(resource_dir) = resource_dir else {
        return geodata_present(app_data_dir);
    };
    // Local layout uses the shared platform directory (sing-box naming).
    let platform = crate::core::paths::detect_platform()
        .map(|p| p.asset_suffix)
        .unwrap_or("windows-amd64");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bundled_dirs = [
        manifest.join("resources/bin").join(platform),
        resource_dir.join("resources/bin").join(platform),
        resource_dir.join("bin").join(platform),
        resource_dir.join(platform),
    ];
    for file in GEODATA_FILES {
        let dest = bin.join(file);
        if dest.is_file() {
            continue;
        }
        for dir in &bundled_dirs {
            let src = dir.join(file);
            if src.is_file() {
                let _ = std::fs::create_dir_all(&bin);
                if std::fs::copy(&src, &dest).is_ok() {
                    break;
                }
            }
        }
    }
    geodata_present(app_data_dir)
}

/// Download missing dat files (sync; ureq — safe inside blocking workers).
/// `proxy_url` mirrors the core-download routing (local mixed port when up).
pub fn download_missing_geodata(app_data_dir: &Path, proxy_url: Option<&str>) -> AppResult<()> {
    let bin = crate::core::paths::core_dir(app_data_dir);
    std::fs::create_dir_all(&bin)?;
    for file in GEODATA_FILES {
        let dest = bin.join(file);
        if dest.is_file() {
            continue;
        }
        let url = format!("{GEODATA_BASE_URL}/{file}");
        let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(120));
        if let Some(proxy) = proxy_url {
            let proxy = ureq::Proxy::new(proxy)
                .map_err(|e| AppError::Core(format!("geodata proxy: {e}")))?;
            builder = builder.proxy(proxy);
        }
        let agent = builder.build();
        let resp = agent
            .get(&url)
            .call()
            .map_err(|e| AppError::Core(format!("geodata download {file}: {e}")))?;
        let len = resp
            .header("content-length")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        if len > MAX_DAT_BYTES {
            return Err(AppError::Core(format!(
                "geodata {file} exceeds {MAX_DAT_BYTES} bytes"
            )));
        }
        let mut bytes = Vec::new();
        resp.into_reader()
            .take(MAX_DAT_BYTES)
            .read_to_end(&mut bytes)
            .map_err(|e| AppError::Core(format!("geodata read {file}: {e}")))?;
        // Sanity: dat files are protobuf containers, never tiny.
        if bytes.len() < 1024 {
            return Err(AppError::Core(format!(
                "geodata {file} too small ({} bytes), likely failed",
                bytes.len()
            )));
        }
        let staged = dest.with_extension("dat.part");
        std::fs::write(&staged, &bytes)?;
        std::fs::rename(&staged, &dest)?;
        crate::app_log::info(
            "geodata",
            format!("downloaded {file} ({} bytes)", bytes.len()),
        );
    }
    Ok(())
}

/// Full ensure chain used before starting Xray: staged → bundled → network.
pub fn ensure_geodata(
    app_data_dir: &Path,
    resource_dir: Option<&Path>,
    proxy_url: Option<&str>,
) -> AppResult<()> {
    if geodata_present(app_data_dir) {
        return Ok(());
    }
    if stage_bundled_geodata(app_data_dir, resource_dir) {
        return Ok(());
    }
    download_missing_geodata(app_data_dir, proxy_url)?;
    if !geodata_present(app_data_dir) {
        return Err(AppError::Core(
            "geosite.dat / geoip.dat missing — geosite:/geoip: rules cannot load".into(),
        ));
    }
    Ok(())
}

/// Xray's native tun inbound loads wintun.dll on Windows (not shipped in the
/// Xray release zip). Ensure it sits next to the core binary in `bin/`.
#[cfg(target_os = "windows")]
pub fn ensure_wintun(
    app_data_dir: &Path,
    resource_dir: Option<&Path>,
    proxy_url: Option<&str>,
) -> AppResult<()> {
    const WINTUN_URL: &str = "https://www.wintun.net/builds/wintun-0.14.1.zip";
    let bin = crate::core::paths::core_dir(app_data_dir);
    let dest = bin.join("wintun.dll");
    if dest.is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(&bin)?;

    // 1. staged alongside a staged xray.exe (paths::stage_bundled_core) —
    //    nothing to do; 2. bundled resources dir; 3. network download.
    if let Some(resource_dir) = resource_dir {
        // Local layout uses the shared platform directory (sing-box naming).
        let suffix = crate::core::paths::detect_platform()
            .map(|p| p.asset_suffix)
            .unwrap_or("windows-amd64");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        for dir in [
            manifest.join("resources/bin").join(suffix),
            resource_dir.join("resources/bin").join(suffix),
            resource_dir.join("bin").join(suffix),
            resource_dir.join(suffix),
        ] {
            let src = dir.join("wintun.dll");
            if src.is_file() && std::fs::copy(&src, &dest).is_ok() {
                return Ok(());
            }
        }
    }

    let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(120));
    if let Some(proxy) = proxy_url {
        let proxy =
            ureq::Proxy::new(proxy).map_err(|e| AppError::Core(format!("wintun proxy: {e}")))?;
        builder = builder.proxy(proxy);
    }
    let resp = builder
        .build()
        .get(WINTUN_URL)
        .call()
        .map_err(|e| AppError::Core(format!("wintun download: {e}")))?;
    let mut zip_bytes = Vec::new();
    resp.into_reader()
        .take(MAX_DAT_BYTES)
        .read_to_end(&mut zip_bytes)
        .map_err(|e| AppError::Core(format!("wintun read: {e}")))?;
    let staged = dest.with_extension("part");
    extract_wintun_amd64(&zip_bytes, &staged)?;
    std::fs::rename(&staged, &dest)?;
    Ok(())
}

/// Pull `wintun/bin/amd64/wintun.dll` out of the official wintun.zip.
#[cfg(target_os = "windows")]
fn extract_wintun_amd64(zip_bytes: &[u8], dest: &Path) -> AppResult<()> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| AppError::Core(format!("wintun zip open: {e}")))?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::Core(format!("wintun zip entry: {e}")))?;
        if entry.name().replace('\\', "/") != "wintun/bin/amd64/wintun.dll" {
            continue;
        }
        let mut out = std::fs::File::create(dest)
            .map_err(|e| AppError::Core(format!("create wintun.dll: {e}")))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| AppError::Core(format!("extract wintun.dll: {e}")))?;
        return Ok(());
    }
    Err(AppError::Core(
        "wintun/bin/amd64/wintun.dll not found in archive".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dir_is_not_geodata_present() {
        let dir = std::env::temp_dir().join(format!(
            "satelite-geodata-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!geodata_present(&dir));
        // staging from nothing changes nothing
        assert!(!stage_bundled_geodata(&dir, None));
        assert!(!geodata_present(&dir));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stage_from_fake_bundled_dir() {
        let root = std::env::temp_dir().join(format!(
            "satelite-geodata-stage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let app_data = root.join("app-data");
        std::fs::create_dir_all(&app_data).unwrap();
        // Fake a bundled platform dir layout: resources/bin/<suffix> (shared
        // sing-box-style platform naming for both cores).
        let resource_root = root.join("res");
        let suffix = crate::core::paths::detect_platform().unwrap().asset_suffix;
        let bundled = resource_root.join("bin").join(suffix);
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::write(bundled.join("geosite.dat"), b"fake-geosite").unwrap();
        std::fs::write(bundled.join("geoip.dat"), b"fake-geoip").unwrap();

        assert!(stage_bundled_geodata(&app_data, Some(&resource_root)));
        assert!(geodata_present(&app_data));
        let _ = std::fs::remove_dir_all(root);
    }
}

use crate::error::{AppError, AppResult};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorePlatform {
    /// e.g. darwin-arm64, windows-amd64
    pub asset_suffix: &'static str,
    pub is_windows: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreSource {
    /// User-downloaded under app data
    Downloaded,
    /// Bundled with the app package / repo resources
    Bundled,
    Missing,
}

pub fn detect_platform() -> AppResult<CorePlatform> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let (suffix, is_windows) = match (os, arch) {
        ("macos", "aarch64") => ("darwin-arm64", false),
        ("macos", "x86_64") => ("darwin-amd64", false),
        ("linux", "aarch64") => ("linux-arm64", false),
        ("linux", "x86_64") => ("linux-amd64", false),
        ("windows", "x86_64") => ("windows-amd64", true),
        ("windows", "aarch64") => ("windows-arm64", true),
        _ => {
            return Err(AppError::Core(format!("unsupported platform: {os}/{arch}")));
        }
    };
    Ok(CorePlatform {
        asset_suffix: suffix,
        is_windows,
    })
}

pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "sing-box.exe"
    } else {
        "sing-box"
    }
}

pub fn core_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("bin")
}

/// User-managed binary path (download / update target).
pub fn core_bin_path(app_data_dir: &Path) -> PathBuf {
    core_dir(app_data_dir).join(binary_name())
}

pub fn version_file_path(app_data_dir: &Path) -> PathBuf {
    core_dir(app_data_dir).join("version.txt")
}

/// Absolute path candidates for the built-in binary (dev + packaging).
pub fn bundled_core_candidates(resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let bin = binary_name();
    let plat = detect_platform()
        .map(|p| p.asset_suffix)
        .unwrap_or("darwin-arm64");

    // Dev source tree first: running from target/debug/resources can be SIGKILL'd on macOS.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    out.push(manifest.join("resources/bin").join(plat).join(bin));

    if let Some(res) = resource_dir {
        // Tauri resource root layouts (varies by OS / config)
        out.push(res.join("resources/bin").join(plat).join(bin));
        out.push(res.join("bin").join(plat).join(bin));
        out.push(res.join(plat).join(bin));
        out.push(res.join(bin));
    }

    out
}

/// Paths under `target/{debug,release}/…` — same bytes can get SIGKILL when executed from there.
fn is_cargo_target_path(p: &Path) -> bool {
    let mut comps = p
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned());
    while let Some(c) = comps.next() {
        if c == "target" {
            if let Some(profile) = comps.next() {
                if profile == "debug" || profile == "release" {
                    return true;
                }
            }
        }
    }
    false
}

pub fn find_bundled_core(resource_dir: Option<&Path>) -> Option<PathBuf> {
    let cands: Vec<PathBuf> = bundled_core_candidates(resource_dir)
        .into_iter()
        .filter(|p| p.is_file())
        .collect();
    // Prefer non-target paths (src-tauri/resources, app bundle, …)
    cands
        .iter()
        .find(|p| !is_cargo_target_path(p))
        .cloned()
        .or_else(|| cands.into_iter().next())
}

/// Copy bundled core into app data `bin/` so we always execute from a stable path.
fn stage_bundled_core(app_data_dir: &Path, bundled: &Path) -> AppResult<PathBuf> {
    let dest = core_bin_path(app_data_dir);
    if dest.is_file() {
        // Same size → assume OK; re-copy if source is newer/different size.
        // Preserve setuid binaries (TUN auth) when content length matches.
        let same = std::fs::metadata(&dest)
            .ok()
            .zip(std::fs::metadata(bundled).ok())
            .map(|(a, b)| a.len() == b.len())
            .unwrap_or(false);
        if same {
            return Ok(dest);
        }
        // Root-owned setuid binary cannot be overwritten by the user without elevation.
        #[cfg(target_os = "macos")]
        {
            if let Err(e) = super::macos_auth::remove_setuid_core_if_needed(&dest) {
                crate::app_log::warn("core", format!("could not replace setuid sing-box: {e}"));
            }
        }
    }
    let dir = core_dir(app_data_dir);
    std::fs::create_dir_all(&dir)?;
    std::fs::copy(bundled, &dest)
        .map_err(|e| AppError::Core(format!("copy sing-box to {}: {e}", dest.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&dest, perms)?;
    }
    // Best-effort clear quarantine on macOS
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("xattr").args(["-cr"]).arg(&dest).output();
    }
    // Keep version.txt next to staged binary when available
    if let Some(parent) = bundled.parent() {
        let vf = parent.join("version.txt");
        if vf.is_file() {
            if let Ok(v) = std::fs::read_to_string(&vf) {
                let _ = write_version_file(app_data_dir, v.trim());
            }
        }
    }
    Ok(dest)
}

/// Prefer staged/downloaded core under app data; stage bundled on first use.
pub fn resolve_core_bin(
    app_data_dir: &Path,
    resource_dir: Option<&Path>,
) -> (Option<PathBuf>, CoreSource) {
    let downloaded = core_bin_path(app_data_dir);
    if downloaded.is_file() {
        return (Some(downloaded), CoreSource::Downloaded);
    }
    if let Some(bundled) = find_bundled_core(resource_dir) {
        match stage_bundled_core(app_data_dir, &bundled) {
            Ok(staged) => return (Some(staged), CoreSource::Bundled),
            Err(_) => {
                // Fall back to direct path if not under cargo target
                if !is_cargo_target_path(&bundled) {
                    return (Some(bundled), CoreSource::Bundled);
                }
            }
        }
    }
    (None, CoreSource::Missing)
}

pub fn installed_core_version(app_data_dir: &Path) -> Option<String> {
    let vf = version_file_path(app_data_dir);
    if let Ok(s) = std::fs::read_to_string(vf) {
        let t = s.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}

/// Read bundled version from `version.txt` only (no process spawn — keeps UI instant).
pub fn bundled_core_version(resource_dir: Option<&Path>) -> Option<String> {
    if let Some(bin) = find_bundled_core(resource_dir) {
        if let Some(parent) = bin.parent() {
            let vf = parent.join("version.txt");
            if let Ok(s) = std::fs::read_to_string(vf) {
                let t = s.trim().to_string();
                if !t.is_empty() {
                    return Some(normalize_version(&t));
                }
            }
        }
    }
    // Also try fixed relative layout next to candidates (dev)
    for cand in bundled_core_candidates(resource_dir) {
        if let Some(parent) = cand.parent() {
            let vf = parent.join("version.txt");
            if let Ok(s) = std::fs::read_to_string(vf) {
                let t = s.trim().to_string();
                if !t.is_empty() {
                    return Some(normalize_version(&t));
                }
            }
        }
    }
    None
}

/// Resolve version for whatever core is active (file metadata only; no `sing-box version`).
pub fn active_core_version(app_data_dir: &Path, resource_dir: Option<&Path>) -> Option<String> {
    let (_path, source) = resolve_core_bin(app_data_dir, resource_dir);
    match source {
        CoreSource::Downloaded => installed_core_version(app_data_dir),
        CoreSource::Bundled => bundled_core_version(resource_dir),
        CoreSource::Missing => None,
    }
}

pub fn read_core_version_via_binary(app_data_dir: &Path) -> AppResult<String> {
    let bin = core_bin_path(app_data_dir);
    read_version_of_binary(&bin)
}

pub fn read_version_of_binary(bin: &Path) -> AppResult<String> {
    if !bin.exists() {
        return Err(AppError::Core("sing-box binary not found".into()));
    }
    let mut cmd = Command::new(bin);
    cmd.arg("version");
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd
        .output()
        .map_err(|e| AppError::Core(format!("run version failed: {e}")))?;
    if !out.status.success() {
        return Err(AppError::Core(format!(
            "version exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("sing-box version ") {
            return Ok(normalize_version(rest.trim()));
        }
        if let Some(v) = line
            .split_whitespace()
            .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
        {
            return Ok(normalize_version(v));
        }
        return Ok(normalize_version(line));
    }
    Err(AppError::Core("empty version output".into()))
}

pub fn normalize_version(v: &str) -> String {
    let v = v.trim();
    if v.starts_with('v') {
        v.to_string()
    } else {
        format!("v{v}")
    }
}

pub fn write_version_file(app_data_dir: &Path, version: &str) -> AppResult<()> {
    let dir = core_dir(app_data_dir);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(version_file_path(app_data_dir), normalize_version(version))?;
    Ok(())
}

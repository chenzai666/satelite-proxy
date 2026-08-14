mod download;
#[cfg(target_os = "windows")]
mod elevate;
#[cfg(target_os = "windows")]
mod job;
#[cfg(target_os = "macos")]
mod macos_auth;
pub mod manager;
mod paths;

pub use download::{
    download_latest_core, fetch_latest_release, CoreDownloadResult, LatestReleaseInfo,
};
#[cfg(test)]
pub use paths::find_bundled_core;
pub use paths::{
    active_core_version, bundled_core_version, detect_platform, resolve_core_bin, CoreSource,
};

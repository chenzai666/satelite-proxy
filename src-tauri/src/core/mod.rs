mod download;
pub mod manager;
#[cfg(target_os = "windows")]
mod job;
mod paths;

pub use download::{
    download_latest_core, fetch_latest_release, CoreDownloadResult, LatestReleaseInfo,
};
pub use paths::{
    active_core_version, bundled_core_version, detect_platform, resolve_core_bin, CoreSource,
};

mod assets;
mod download;
#[cfg(target_os = "windows")]
mod elevate;
#[cfg(target_os = "windows")]
mod job;
pub mod kind;
#[cfg(target_os = "macos")]
mod macos_auth;
#[cfg(target_os = "macos")]
pub mod macos_net;
pub mod manager;
mod memory;
mod paths;

pub use assets::ensure_geodata;
#[cfg(target_os = "windows")]
pub use assets::ensure_wintun;
pub use assets::{download_missing_geodata, geodata_state};
pub use assets::{download_missing_meow_geodata, ensure_meow_geodata, meow_geodata_state};
pub use kind::CoreKind;
pub use memory::read_process_rss_bytes;

pub use download::{
    download_latest_core, download_latest_core_with_progress, fetch_latest_app_tag,
    fetch_latest_app_tag_via_redirect, fetch_latest_release_with_proxy, CoreDownloadResult,
    LatestReleaseInfo,
};
#[cfg(test)]
pub use paths::find_bundled_core;
pub use paths::{
    active_core_version, bundled_core_version, detect_platform, inspect_core_bin, resolve_core_bin,
    CoreSource,
};

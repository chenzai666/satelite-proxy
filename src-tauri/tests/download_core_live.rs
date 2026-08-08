//! Live download of sing-box core (network required).
//! Run: `cargo test -p satelite-proxy --test download_core_live -- --ignored --nocapture`

use satelite_proxy_lib::download_core_to;
use std::path::PathBuf;

#[tokio::test]
#[ignore = "network: downloads real sing-box release"]
async fn download_latest_into_temp() {
    let dir = std::env::temp_dir().join(format!("satelite-core-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let result = download_core_to(&dir, None).await.expect("download core");

    println!(
        "installed {} → {} ({} bytes)",
        result.version, result.path, result.bytes
    );
    assert!(PathBuf::from(&result.path).exists());
    assert!(result.version.contains('1') || result.version.starts_with('v'));
}

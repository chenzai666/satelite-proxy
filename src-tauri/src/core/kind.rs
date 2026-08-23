//! Core kind descriptor — everything that differs between sing-box and Xray
//! at the process-management level (binary name, release asset naming, CLI
//! arguments, version output, spawn environment). Config generation lives
//! separately: `config/builder.rs` (sing-box) and `config/xray.rs` (Xray).

use crate::domain::Protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreKind {
    SingBox,
    Xray,
}

impl CoreKind {
    pub fn binary_name(self) -> &'static str {
        match self {
            Self::SingBox => {
                if cfg!(windows) {
                    "sing-box.exe"
                } else {
                    "sing-box"
                }
            }
            Self::Xray => {
                if cfg!(windows) {
                    "xray.exe"
                } else {
                    "xray"
                }
            }
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "Xray",
        }
    }

    /// Stable token used by settings storage and the frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SingBox => "singbox",
            Self::Xray => "xray",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "xray" => Self::Xray,
            _ => Self::SingBox,
        }
    }

    /// `owner/repo` for GitHub release queries.
    pub fn repo(self) -> &'static str {
        match self {
            Self::SingBox => "SagerNet/sing-box",
            Self::Xray => "XTLS/Xray-core",
        }
    }

    /// Pinned version used only when the GitHub API is unreachable.
    pub fn fallback_version(self) -> &'static str {
        match self {
            Self::SingBox => "v1.13.15",
            Self::Xray => "v26.3.27",
        }
    }

    /// Release asset name for a platform. Note the naming schemes differ:
    /// sing-box `sing-box-{ver}-darwin-arm64.tar.gz`, Xray
    /// `Xray-macos-arm64-v8a.zip` (no version in the name, `64` not `amd64`).
    pub fn asset_name(self, version: &str, platform_suffix: &str, is_windows: bool) -> String {
        let ver_num = version.trim_start_matches('v');
        match self {
            Self::SingBox => {
                let ext = if is_windows { "zip" } else { "tar.gz" };
                format!("sing-box-{ver_num}-{platform_suffix}.{ext}")
            }
            Self::Xray => format!("Xray-{platform_suffix}.zip"),
        }
    }

    /// CLI arguments that print the version.
    pub fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::SingBox => &["version"],
            Self::Xray => &["-version"],
        }
    }

    /// Extract the version from `version_args` output. sing-box prints
    /// `sing-box version 1.13.15 (...)`; Xray prints
    /// `Xray 26.3.27 (Custom) ... (go1.24 ...)`.
    pub fn parse_version_output(self, out: &str) -> Option<String> {
        for line in out.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match self {
                Self::SingBox => {
                    if let Some(rest) = line.strip_prefix("sing-box version ") {
                        return Some(rest.split_whitespace().next()?.to_string());
                    }
                }
                Self::Xray => {
                    if let Some(rest) = line.strip_prefix("Xray ") {
                        return Some(rest.split_whitespace().next()?.to_string());
                    }
                }
            }
            // Fallback: first token that starts with a digit.
            if let Some(token) = line
                .split_whitespace()
                .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
            {
                return Some(token.to_string());
            }
            return None;
        }
        None
    }

    /// CLI arguments that validate a config file without starting the server.
    /// sing-box: `check -c <file>`; Xray: `run -test -c <file>`.
    pub fn check_args(self) -> &'static [&'static str] {
        match self {
            Self::SingBox => &["check", "-c"],
            Self::Xray => &["run", "-test", "-c"],
        }
    }

    /// CLI arguments that run the core with a config file.
    pub fn run_args(self) -> &'static [&'static str] {
        // Both cores happen to use `run -c <file>`.
        &["run", "-c"]
    }

    /// Infer the kind from a binary path's file stem (e.g. the Windows
    /// elevated-helper entry point receives only the binary path).
    pub fn from_binary_path(path: &std::path::Path) -> Self {
        match path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("xray") => Self::Xray,
            _ => Self::SingBox,
        }
    }

    /// Environment variables the child process needs. Xray resolves
    /// geosite.dat / geoip.dat (and TLS certs) relative to `bin_dir`.
    pub fn spawn_env(self, bin_dir: &std::path::Path) -> Vec<(String, String)> {
        match self {
            Self::SingBox => Vec::new(),
            Self::Xray => {
                let dir = bin_dir.display().to_string();
                vec![
                    ("XRAY_LOCATION_ASSET".into(), dir.clone()),
                    ("XRAY_LOCATION_CERT".into(), dir),
                ]
            }
        }
    }

    /// Log file / display prefix.
    pub fn log_prefix(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "xray",
        }
    }

    /// Version file name inside the app-data `bin/` directory. sing-box keeps
    /// the historical `version.txt`; Xray uses a prefixed name so the two
    /// cores can coexist without clobbering each other's metadata.
    pub fn version_file_name(self) -> &'static str {
        match self {
            Self::SingBox => "version.txt",
            Self::Xray => "xray-version.txt",
        }
    }

    /// Whether this core can serve the given outbound protocol. Xray lacks
    /// hysteria(2)/tuic/anytls/snell/shadowtls/ssh/naive/tor.
    pub fn supports(self, protocol: Protocol) -> bool {
        match self {
            Self::SingBox => true,
            Self::Xray => matches!(
                protocol,
                Protocol::Shadowsocks
                    | Protocol::Vmess
                    | Protocol::Vless
                    | Protocol::Trojan
                    | Protocol::Socks5
                    | Protocol::Http
                    | Protocol::WireGuard
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_output() {
        assert_eq!(
            CoreKind::SingBox
                .parse_version_output("sing-box version 1.13.15 (go1.24)\n")
                .as_deref(),
            Some("1.13.15")
        );
        assert_eq!(
            CoreKind::Xray
                .parse_version_output("Xray 26.3.27 (Custom) 1234560 (go1.24)\n")
                .as_deref(),
            Some("26.3.27")
        );
        // Fallback: first digit-leading token.
        assert_eq!(
            CoreKind::Xray
                .parse_version_output("Xray 1.2.3 weird output\n")
                .as_deref(),
            Some("1.2.3")
        );
        assert_eq!(CoreKind::Xray.parse_version_output(""), None);
    }

    #[test]
    fn asset_names_match_release_schemes() {
        assert_eq!(
            CoreKind::SingBox.asset_name("1.13.15", "windows-amd64", true),
            "sing-box-1.13.15-windows-amd64.zip"
        );
        assert_eq!(
            CoreKind::SingBox.asset_name("1.13.15", "darwin-arm64", false),
            "sing-box-1.13.15-darwin-arm64.tar.gz"
        );
        assert_eq!(
            CoreKind::Xray.asset_name("26.3.27", "windows-64", true),
            "Xray-windows-64.zip"
        );
        assert_eq!(
            CoreKind::Xray.asset_name("26.3.27", "macos-arm64-v8a", false),
            "Xray-macos-arm64-v8a.zip"
        );
    }

    #[test]
    fn protocol_support_matrix() {
        assert!(CoreKind::Xray.supports(Protocol::Vless));
        assert!(CoreKind::Xray.supports(Protocol::Vmess));
        assert!(CoreKind::Xray.supports(Protocol::WireGuard));
        assert!(!CoreKind::Xray.supports(Protocol::Hysteria2));
        assert!(!CoreKind::Xray.supports(Protocol::Tuic));
        assert!(CoreKind::SingBox.supports(Protocol::Hysteria2));
    }

    #[test]
    fn settings_roundtrip() {
        assert_eq!(CoreKind::parse("xray"), CoreKind::Xray);
        assert_eq!(CoreKind::parse("singbox"), CoreKind::SingBox);
        assert_eq!(CoreKind::parse("garbage"), CoreKind::SingBox);
        assert_eq!(CoreKind::Xray.as_str(), "xray");
    }
}

//! Core kind descriptor — everything that differs between sing-box, Xray and
//! meow at the process-management level (binary name, release asset naming,
//! CLI arguments, version output, spawn environment). Config generation lives
//! separately: `config/builder.rs` (sing-box), `config/xray.rs` (Xray) and
//! `config/meow.rs` (meow, Clash YAML).

use crate::domain::Protocol;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreKind {
    SingBox,
    Xray,
    Meow,
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
            Self::Meow => {
                if cfg!(windows) {
                    "meow.exe"
                } else {
                    "meow"
                }
            }
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "Xray",
            Self::Meow => "meow",
        }
    }

    /// Stable token used by settings storage and the frontend.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SingBox => "singbox",
            Self::Xray => "xray",
            Self::Meow => "meow",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "xray" => Self::Xray,
            "meow" => Self::Meow,
            _ => Self::SingBox,
        }
    }

    /// `owner/repo` for GitHub release queries.
    pub fn repo(self) -> &'static str {
        match self {
            Self::SingBox => "SagerNet/sing-box",
            Self::Xray => "XTLS/Xray-core",
            Self::Meow => "madeye/meow-rs",
        }
    }

    /// Pinned version used only when the GitHub API is unreachable.
    pub fn fallback_version(self) -> &'static str {
        match self {
            Self::SingBox => "v1.13.15",
            Self::Xray => "v26.3.27",
            Self::Meow => "v0.21.0",
        }
    }

    /// Release asset name for a platform. The naming schemes all differ:
    /// sing-box `sing-box-{ver}-darwin-arm64.tar.gz`, Xray
    /// `Xray-macos-arm64-v8a.zip` (no version, `64` not `amd64`), meow
    /// `meow-v{ver}-aarch64-apple-darwin.tar.gz` (Rust target triples).
    pub fn asset_name(self, version: &str, platform_suffix: &str, is_windows: bool) -> String {
        let ver_num = version.trim_start_matches('v');
        match self {
            Self::SingBox => {
                let ext = if is_windows { "zip" } else { "tar.gz" };
                format!("sing-box-{ver_num}-{platform_suffix}.{ext}")
            }
            Self::Xray => format!("Xray-{platform_suffix}.zip"),
            Self::Meow => {
                let ext = if is_windows { "zip" } else { "tar.gz" };
                format!("meow-v{ver_num}-{platform_suffix}.{ext}")
            }
        }
    }

    /// CLI arguments that print the version.
    pub fn version_args(self) -> &'static [&'static str] {
        match self {
            Self::SingBox => &["version"],
            Self::Xray => &["-version"],
            Self::Meow => &["-v"],
        }
    }

    /// Extract the version from `version_args` output. sing-box prints
    /// `sing-box version 1.13.15 (...)`; Xray prints
    /// `Xray 26.3.27 (Custom) ... (go1.24 ...)`; meow prints
    /// `Meow Meta 0.21.0` (mihomo `-v` compatibility).
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
                Self::Meow => {
                    if let Some(rest) = line.strip_prefix("Meow Meta ") {
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

    /// Full CLI argument vector that validates `config` without starting the
    /// server. sing-box: `check -c <file>`; Xray: `run -test -c <file>`;
    /// meow: `-t -f <file> -d <home>` (the home dir hosts its geodata, and
    /// relative config paths resolve against it, so both are absolute here).
    pub fn check_command_args(self, config: &Path) -> Vec<String> {
        let config = config.display().to_string();
        match self {
            Self::SingBox => vec!["check".into(), "-c".into(), config],
            Self::Xray => vec!["run".into(), "-test".into(), "-c".into(), config],
            Self::Meow => {
                let mut args = vec!["-t".into(), "-f".into(), config.clone()];
                args.extend(meow_home_args(&config));
                args
            }
        }
    }

    /// Full CLI argument vector that runs the core with `config`. sing-box
    /// and Xray: `run -c <file>`; meow: `-f <file> -d <home>` — meow has no
    /// `run` subcommand, the config alone starts the server.
    pub fn run_command_args(self, config: &Path) -> Vec<String> {
        let config = config.display().to_string();
        match self {
            Self::SingBox | Self::Xray => vec!["run".into(), "-c".into(), config],
            Self::Meow => {
                let mut args = vec!["-f".into(), config.clone()];
                args.extend(meow_home_args(&config));
                args
            }
        }
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
            Some("meow") => Self::Meow,
            _ => Self::SingBox,
        }
    }

    /// Environment variables the child process needs. Xray resolves
    /// geosite.dat / geoip.dat (and TLS certs) relative to `bin_dir`; meow
    /// loads its bundled wintun.dll (Windows tun) from `bin/meow-wintun.dll`
    /// — kept beside the binary under a meow-specific name so it never
    /// collides with Xray's own `bin/wintun.dll`.
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
            Self::Meow => {
                let dll = bin_dir.join("meow-wintun.dll");
                if cfg!(windows) && dll.is_file() {
                    vec![("MEOW_WINTUN_DLL".into(), dll.display().to_string())]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Log file / display prefix.
    pub fn log_prefix(self) -> &'static str {
        match self {
            Self::SingBox => "sing-box",
            Self::Xray => "xray",
            Self::Meow => "meow",
        }
    }

    /// Version file name inside the app-data `bin/` directory. sing-box keeps
    /// the historical `version.txt`; the other cores use prefixed names so
    /// all three can coexist without clobbering each other's metadata.
    pub fn version_file_name(self) -> &'static str {
        match self {
            Self::SingBox => "version.txt",
            Self::Xray => "xray-version.txt",
            Self::Meow => "meow-version.txt",
        }
    }

    /// Whether this core can serve the given outbound protocol. Xray and meow
    /// each delegate to their `Protocol::*_supported` counterpart — the
    /// single source of truth lives on the protocol enum.
    pub fn supports(self, protocol: Protocol) -> bool {
        match self {
            Self::SingBox => true,
            Self::Xray => protocol.xray_supported(),
            Self::Meow => protocol.meow_supported(),
        }
    }

    /// Whether this core can serve the NODE as a whole — protocol plus the
    /// per-node shapes a core cannot represent. sing-box serves everything;
    /// meow additionally rejects:
    /// - REALITY (any protocol): meow's REALITY client hand-rolls a minimal
    ///   ClientHello and ignores `client-fingerprint` (boring-tls/uTLS only
    ///   covers its plain-TLS path), so strict REALITY servers black-hole
    ///   the handshake ("Reality TLS: handshake did not complete within
    ///   10s" — verified against a node that returns 204 through sing-box
    ///   with the same parameters);
    /// - vless with a flow (XTLS Vision): meow's raw passthrough only
    ///   works over its REALITY stream (transport::enable_raw_* only
    ///   downcasts RealityTlsStream), so plain-TLS Vision nodes die at
    ///   "vision: DIRECT requested but transport cannot switch to raw
    ///   passthrough" — and Vision+REALITY is already excluded above.
    ///   These nodes pass TCP latency probes (they look alive!), so
    ///   selecting one silently kills the proxy-egress remote DNS and
    ///   takes the whole network down;
    /// - vmess on non-tcp/ws transports (meow parser rejects them);
    /// - ss + shadow-tls (a sing-box-only outbound detour shape).
    pub fn supports_node(self, node: &crate::domain::ProxyNode) -> bool {
        if !self.supports(node.protocol) {
            return false;
        }
        if self == Self::Meow {
            let reality = node
                .tls
                .as_ref()
                .is_some_and(|t| t.reality_public_key.is_some() || t.reality_short_id.is_some());
            if reality {
                return false;
            }
            if node.protocol == Protocol::Vless
                && matches!(
                    &node.config,
                    crate::domain::ProtocolConfig::Vless {
                        flow: Some(f), ..
                    } if !f.trim().is_empty()
                )
            {
                return false;
            }
            if node.protocol == Protocol::Vmess
                && !matches!(
                    node.transport,
                    None | Some(crate::domain::Transport::Tcp)
                        | Some(crate::domain::Transport::Ws { .. })
                )
            {
                return false;
            }
            if matches!(
                &node.config,
                crate::domain::ProtocolConfig::Shadowsocks {
                    shadow_tls: Some(_),
                    ..
                }
            ) {
                return false;
            }
        }
        true
    }
}

/// `-d <home>` argument pair for meow. The home dir is derived from the
/// config location: active.yaml lives in `<app_data>/config/`, so the meow
/// home (holding `Country.mmdb` + `geosite.dat`, see `core::assets`) is the
/// sibling directory `<app_data>/meow/`. Keeping it separate from `bin/` is
/// deliberate — meow's `geosite.dat` is MetaCubeX .mrs format and would
/// collide with Xray's v2ray-format `bin/geosite.dat` of the same name.
fn meow_home_args(config: &str) -> Vec<String> {
    let home = Path::new(config)
        .parent()
        .and_then(|p| p.parent())
        .map(|root| root.join("meow"))
        .unwrap_or_else(|| Path::new("meow").to_path_buf());
    vec!["-d".into(), home.display().to_string()]
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
        assert_eq!(
            CoreKind::Meow
                .parse_version_output("Meow Meta 0.21.0\n")
                .as_deref(),
            Some("0.21.0")
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
        assert_eq!(
            CoreKind::Meow.asset_name("0.21.0", "x86_64-pc-windows-msvc", true),
            "meow-v0.21.0-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            CoreKind::Meow.asset_name("0.21.0", "aarch64-apple-darwin", false),
            "meow-v0.21.0-aarch64-apple-darwin.tar.gz"
        );
    }

    #[test]
    fn meow_command_args_include_home_dir() {
        let config = Path::new("/data/config/active.yaml");
        let expect_args = |mut head: Vec<String>, config: &str| {
            head.push("-f".into());
            head.push(config.to_string());
            head.push("-d".into());
            head.push(
                Path::new(config)
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|root| root.join("meow"))
                    .unwrap_or_else(|| Path::new("meow").to_path_buf())
                    .display()
                    .to_string(),
            );
            head
        };
        let cfg = "/data/config/active.yaml";
        assert_eq!(
            CoreKind::Meow.check_command_args(config),
            expect_args(vec!["-t".into()], cfg)
        );
        assert_eq!(
            CoreKind::Meow.run_command_args(config),
            expect_args(vec![], cfg)
        );
        // JSON cores keep the historical shape.
        assert_eq!(
            CoreKind::SingBox.check_command_args(config),
            ["check", "-c", "/data/config/active.yaml"].map(String::from)
        );
        assert_eq!(
            CoreKind::Xray.check_command_args(config),
            ["run", "-test", "-c", "/data/config/active.yaml"].map(String::from)
        );
        assert_eq!(
            CoreKind::SingBox.run_command_args(config),
            ["run", "-c", "/data/config/active.yaml"].map(String::from)
        );
    }

    #[test]
    fn meow_home_falls_back_for_relative_config() {
        let args = meow_home_args("active.yaml");
        assert_eq!(args, vec!["-d".to_string(), "meow".to_string()]);
    }

    #[test]
    fn protocol_support_matrix() {
        assert!(CoreKind::Xray.supports(Protocol::Vless));
        assert!(CoreKind::Xray.supports(Protocol::Vmess));
        assert!(CoreKind::Xray.supports(Protocol::WireGuard));
        assert!(!CoreKind::Xray.supports(Protocol::Hysteria2));
        assert!(!CoreKind::Xray.supports(Protocol::Tuic));
        assert!(CoreKind::SingBox.supports(Protocol::Hysteria2));
        // meow: Clash-family protocols only.
        assert!(CoreKind::Meow.supports(Protocol::Hysteria2));
        assert!(CoreKind::Meow.supports(Protocol::AnyTls));
        assert!(CoreKind::Meow.supports(Protocol::Snell));
        assert!(!CoreKind::Meow.supports(Protocol::Tuic));
        assert!(!CoreKind::Meow.supports(Protocol::WireGuard));
        assert!(!CoreKind::Meow.supports(Protocol::Hysteria));
    }

    #[test]
    fn meow_node_level_support_matrix() {
        use crate::domain::{ProtocolConfig, ProxyNode, TlsConfig, Transport};

        fn node(
            protocol: Protocol,
            tls: Option<TlsConfig>,
            transport: Option<Transport>,
        ) -> ProxyNode {
            ProxyNode {
                id: String::new(),
                name: "n".into(),
                protocol,
                server: "example.com".into(),
                port: 443,
                tls,
                transport,
                udp: None,
                config: match protocol {
                    Protocol::Vmess => ProtocolConfig::Vmess {
                        uuid: "u".into(),
                        alter_id: 0,
                        security: "auto".into(),
                    },
                    _ => ProtocolConfig::Shadowsocks {
                        method: "aes-256-gcm".into(),
                        password: "pw".into(),
                        plugin: None,
                        plugin_opts: None,
                        shadow_tls: None,
                    },
                },
                source: None,
                latency_ms: None,
                latency_at: None,
            }
            .with_computed_id()
        }

        let reality = Some(TlsConfig {
            enabled: true,
            server_name: Some("www.microsoft.com".into()),
            insecure: None,
            alpn: None,
            utls_fingerprint: None,
            reality_public_key: Some("pk".into()),
            reality_short_id: Some("abcd".into()),
        });
        let plain_tls = Some(TlsConfig {
            enabled: true,
            ..Default::default()
        });

        // meow rejects REALITY regardless of protocol.
        assert!(!CoreKind::Meow.supports_node(&node(Protocol::Vless, reality.clone(), None)));
        // meow rejects vless Vision flows: raw passthrough only exists for
        // its REALITY stream, plain-TLS Vision nodes die mid-handshake
        // while passing TCP probes.
        let mut vision = node(Protocol::Vless, plain_tls.clone(), None);
        vision.config = crate::domain::ProtocolConfig::Vless {
            uuid: "u".into(),
            flow: Some("xtls-rprx-vision".into()),
            packet_encoding: "xudp".into(),
        };
        assert!(!CoreKind::Meow.supports_node(&vision));
        // sing-box serves vision nodes happily.
        assert!(CoreKind::SingBox.supports_node(&vision));
        // Plain-TLS vless is fine.
        assert!(CoreKind::Meow.supports_node(&node(Protocol::Vless, plain_tls.clone(), None)));
        // vmess: tcp/ws ok, grpc rejected.
        assert!(CoreKind::Meow.supports_node(&node(
            Protocol::Vmess,
            plain_tls.clone(),
            Some(Transport::Ws {
                path: None,
                headers: None,
                max_early_data: None
            })
        )));
        assert!(!CoreKind::Meow.supports_node(&node(
            Protocol::Vmess,
            plain_tls.clone(),
            Some(Transport::Grpc { service_name: None })
        )));
        // ss + shadow-tls detour rejected; plain ss fine.
        let mut ss_stls = node(Protocol::Shadowsocks, None, None);
        ss_stls.config = ProtocolConfig::Shadowsocks {
            method: "aes-256-gcm".into(),
            password: "pw".into(),
            plugin: None,
            plugin_opts: None,
            shadow_tls: Some(crate::domain::ShadowTlsOpts {
                host: "h".into(),
                password: "p".into(),
                version: 3,
                fingerprint: None,
            }),
        };
        assert!(!CoreKind::Meow.supports_node(&ss_stls));
        assert!(CoreKind::Meow.supports_node(&node(Protocol::Shadowsocks, None, None)));
        // Protocol-level exclusions still apply; sing-box accepts everything.
        assert!(!CoreKind::Meow.supports_node(&node(Protocol::Tuic, None, None)));
        assert!(CoreKind::SingBox.supports_node(&node(Protocol::Vless, reality, None)));
        assert!(CoreKind::SingBox.supports_node(&ss_stls));
    }

    #[test]
    fn settings_roundtrip() {
        assert_eq!(CoreKind::parse("xray"), CoreKind::Xray);
        assert_eq!(CoreKind::parse("meow"), CoreKind::Meow);
        assert_eq!(CoreKind::parse("singbox"), CoreKind::SingBox);
        assert_eq!(CoreKind::parse("garbage"), CoreKind::SingBox);
        assert_eq!(CoreKind::Xray.as_str(), "xray");
        assert_eq!(CoreKind::Meow.as_str(), "meow");
    }
}

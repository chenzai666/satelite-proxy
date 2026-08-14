use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Supported outbound protocols for MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Shadowsocks,
    Vmess,
    Vless,
    Trojan,
    Hysteria2,
    Tuic,
    Socks5,
    Http,
    Hysteria,
    ShadowTls,
    Ssh,
    Naive,
    Tor,
    WireGuard,
    /// AnyTLS (sing-box ≥ 1.12).
    AnyTls,
    /// Snell (sing-box ≥ 1.14, versions 4 / 6).
    Snell,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadowsocks => "shadowsocks",
            Self::Vmess => "vmess",
            Self::Vless => "vless",
            Self::Trojan => "trojan",
            Self::Hysteria2 => "hysteria2",
            Self::Tuic => "tuic",
            Self::Socks5 => "socks5",
            Self::Http => "http",
            Self::Hysteria => "hysteria",
            Self::ShadowTls => "shadowtls",
            Self::Ssh => "ssh",
            Self::Naive => "naive",
            Self::Tor => "tor",
            Self::WireGuard => "wireguard",
            Self::AnyTls => "anytls",
            Self::Snell => "snell",
        }
    }

    pub fn from_clash_type(t: &str) -> Option<Self> {
        match t.to_ascii_lowercase().as_str() {
            "ss" | "shadowsocks" => Some(Self::Shadowsocks),
            "vmess" => Some(Self::Vmess),
            "vless" => Some(Self::Vless),
            "trojan" => Some(Self::Trojan),
            "hysteria2" | "hy2" => Some(Self::Hysteria2),
            "tuic" => Some(Self::Tuic),
            "socks5" | "socks" => Some(Self::Socks5),
            "http" | "https" => Some(Self::Http),
            "hysteria" | "hy" => Some(Self::Hysteria),
            "shadowtls" => Some(Self::ShadowTls),
            "ssh" => Some(Self::Ssh),
            "naive" => Some(Self::Naive),
            "tor" => Some(Self::Tor),
            "wireguard" | "wg" => Some(Self::WireGuard),
            "anytls" => Some(Self::AnyTls),
            "snell" => Some(Self::Snell),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insecure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpn: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utls_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reality_public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reality_short_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Transport {
    Tcp,
    Ws {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        headers: Option<std::collections::BTreeMap<String, String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_early_data: Option<u32>,
    },
    Grpc {
        #[serde(skip_serializing_if = "Option::is_none")]
        service_name: Option<String>,
    },
    Http {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        host: Option<Vec<String>>,
    },
    HttpUpgrade {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        host: Option<String>,
    },
}

/// Protocol-specific fields needed to build a sing-box outbound later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum ProtocolConfig {
    Shadowsocks {
        method: String,
        password: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        plugin: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        plugin_opts: Option<String>,
    },
    Vmess {
        uuid: String,
        #[serde(default)]
        alter_id: u16,
        #[serde(default = "default_vmess_security")]
        security: String,
    },
    Vless {
        uuid: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        flow: Option<String>,
        #[serde(default = "default_vless_packet_encoding")]
        packet_encoding: String,
    },
    Trojan {
        password: String,
    },
    Hysteria2 {
        password: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        up_mbps: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        down_mbps: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfs: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfs_password: Option<String>,
    },
    Tuic {
        uuid: String,
        password: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        congestion_control: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        udp_relay_mode: Option<String>,
        #[serde(default)]
        zero_rtt_handshake: bool,
    },
    Socks5 {
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        password: Option<String>,
    },
    Http {
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    Hysteria {
        auth: String,
        #[serde(default)]
        auth_base64: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        up_mbps: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        down_mbps: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfs: Option<String>,
    },
    ShadowTls {
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        password: Option<String>,
    },
    Ssh {
        user: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        private_key: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        private_key_passphrase: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        host_key: Vec<String>,
    },
    Naive {
        username: String,
        password: String,
        #[serde(default)]
        quic: bool,
    },
    Tor {
        executable_path: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        extra_args: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        data_directory: Option<String>,
    },
    /// sing-box 1.13+ WireGuard endpoint represented as a selectable node.
    WireGuard {
        local_address: Vec<String>,
        private_key: String,
        peer_public_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pre_shared_key: Option<String>,
        #[serde(default)]
        reserved: Vec<u8>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mtu: Option<u32>,
    },
    /// AnyTLS password (often a UUID in share links).
    AnyTls {
        password: String,
    },
    /// Snell PSK + version / obfs (Clash) or mode (v6).
    Snell {
        psk: String,
        /// Protocol version; sing-box accepts 4 or 6 (v5 ≈ v4 wire).
        version: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        userkey: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reuse: Option<bool>,
        /// v4 HTTP obfs: `http` / `none` / `tls` (mapped carefully for sing-box).
        #[serde(skip_serializing_if = "Option::is_none")]
        obfs_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfs_host: Option<String>,
        /// v6 traffic shaping: default / unshaped / unsafe-raw.
        #[serde(skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
}

fn default_vmess_security() -> String {
    "auto".into()
}

fn default_vless_packet_encoding() -> String {
    "xudp".into()
}

/// Normalized proxy node — intermediate model between subscription and sing-box config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyNode {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub server: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<Transport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp: Option<bool>,
    pub config: ProtocolConfig,
    /// Original clash `type` string or uri scheme for debugging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Last measured latency in milliseconds (TCP connect or delay API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    /// Unix timestamp of last latency test.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_at: Option<i64>,
}

impl ProxyNode {
    /// Stable id without subscription context (subscription layer may re-hash later).
    pub fn compute_id(name: &str, server: &str, port: u16, protocol: Protocol) -> String {
        let mut hasher = Sha256::new();
        hasher.update(name.as_bytes());
        hasher.update(b"|");
        hasher.update(server.as_bytes());
        hasher.update(b"|");
        hasher.update(port.to_string().as_bytes());
        hasher.update(b"|");
        hasher.update(protocol.as_str().as_bytes());
        let digest = hasher.finalize();
        hex::encode(&digest[..16])
    }

    pub fn with_computed_id(mut self) -> Self {
        self.id = Self::compute_id(&self.name, &self.server, self.port, self.protocol);
        self
    }
}

/// Result of parsing a subscription body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    pub nodes: Vec<ProxyNode>,
    /// Skipped entries (unsupported type / invalid fields).
    pub skipped: Vec<SkippedProxy>,
    pub format: SubscriptionFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedProxy {
    pub name: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionFormat {
    ClashYaml,
    UriList,
    Base64UriList,
}

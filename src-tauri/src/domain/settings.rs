use serde::{Deserialize, Serialize};

/// Clash-style outbound routing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutboundMode {
    /// Follow user / builtin route rules; unmatched → proxy.
    #[default]
    Rule,
    /// Ignore user rules; all traffic → proxy.
    Global,
    /// Ignore user rules; all traffic → direct.
    Direct,
}

impl OutboundMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Global => "global",
            Self::Direct => "direct",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rule" | "rules" => Some(Self::Rule),
            "global" => Some(Self::Global),
            "direct" => Some(Self::Direct),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// mixed inbound listen port
    pub mixed_port: u16,
    /// clash_api controller port
    pub api_port: u16,
    /// Last selected node id (ProxyNode.id)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_id: Option<String>,
    /// Secret written into last generated config (for future clash_api client)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clash_api_secret: Option<String>,
    /// Probe URL for latency tests (future)
    #[serde(default = "default_probe_url")]
    pub probe_url: String,
    /// When true, multiple subscriptions can be enabled (Mix); otherwise exclusive.
    #[serde(default)]
    pub mix_mode: bool,
    /// Enable sing-box TUN inbound (system-wide capture). Requires privileges on macOS.
    #[serde(default)]
    pub tun_enabled: bool,
    /// TUN TCP/IP stack: `system` | `gvisor` | `mixed` (default mixed).
    #[serde(default = "default_tun_stack")]
    pub tun_stack: String,
    /// Rule / Global / Direct (Clash-style).
    #[serde(default)]
    pub outbound_mode: OutboundMode,

    // —— Application preferences ——
    /// Close window → hide to tray (keep process + core). If false, quit app.
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
    /// Launch at OS login.
    #[serde(default)]
    pub launch_at_login: bool,
    /// Start without showing main window (use tray).
    #[serde(default)]
    pub silent_start: bool,
    /// Start proxy core automatically after app launch.
    #[serde(default)]
    pub auto_start_proxy: bool,
    /// Close all connections after switching node.
    #[serde(default = "default_true")]
    pub close_connections_on_switch: bool,
    /// UI language: `zh` | `en` (sidebar labels stay English).
    #[serde(default = "default_locale")]
    pub locale: String,
    /// UI theme: `day` (light default) | `aerospace` (dark).
    #[serde(default = "default_theme")]
    pub theme: String,
    /// When closing to tray, destroy WebView to free GPU/JS memory (tray + core stay).
    #[serde(default = "default_true")]
    pub unload_ui_on_tray: bool,
    /// Smart node auto-switch: passive observation + on-demand probe (docs/auto.md).
    #[serde(default)]
    pub smart_switch: bool,
}

fn default_probe_url() -> String {
    "https://www.gstatic.com/generate_204".into()
}

fn default_tun_stack() -> String {
    "mixed".into()
}

fn default_true() -> bool {
    true
}

fn default_locale() -> String {
    "zh".into()
}

fn default_theme() -> String {
    "day".into()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            mixed_port: 2080,
            api_port: 19090,
            current_node_id: None,
            clash_api_secret: None,
            probe_url: default_probe_url(),
            mix_mode: false,
            tun_enabled: false,
            tun_stack: default_tun_stack(),
            outbound_mode: OutboundMode::Rule,
            close_to_tray: true,
            launch_at_login: false,
            silent_start: false,
            auto_start_proxy: false,
            close_connections_on_switch: true,
            locale: default_locale(),
            theme: default_theme(),
            unload_ui_on_tray: true,
            smart_switch: false,
        }
    }
}

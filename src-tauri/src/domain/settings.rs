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

/// How the main `proxy` outbound picks a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoSelectMode {
    /// Manual only (selector; user / app picks node).
    #[default]
    Off,
    /// App-level smart switch (passive + on-demand probe; selector).
    Smart,
    /// sing-box `urltest` group; kernel picks by delay.
    Kernel,
}

impl AutoSelectMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Smart => "smart",
            Self::Kernel => "kernel",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "manual" | "false" | "0" => Some(Self::Off),
            "smart" | "app" | "true" | "1" => Some(Self::Smart),
            "kernel" | "urltest" | "core" => Some(Self::Kernel),
            _ => None,
        }
    }

    pub fn is_kernel(self) -> bool {
        matches!(self, Self::Kernel)
    }

    pub fn is_smart(self) -> bool {
        matches!(self, Self::Smart)
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
    /// `route.final` when in Rule mode: `proxy` | `direct` | `block`.
    /// Global/Direct modes ignore this and force proxy/direct respectively.
    #[serde(default = "default_route_final")]
    pub route_final: String,

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
    /// UI accent (brand/primary color) preset id, e.g. `green` | `blue` | ...
    #[serde(default = "default_accent")]
    pub accent: String,
    /// Low-memory mode: when closing to tray, destroy WebView to free GPU/JS
    /// memory. Default false — hide only so reopen is instant. When true, next
    /// wake recreates the WebView (brief black screen).
    #[serde(default)]
    pub unload_ui_on_tray: bool,
    /// Node auto-select: off | smart (app) | kernel (sing-box urltest).
    #[serde(default)]
    pub auto_select: AutoSelectMode,
    /// Resolve the originating process for each connection (sing-box
    /// `find_process_mode`): on = always, off = off. Lets the traffic page
    /// show a real process name. Off saves some CPU.
    #[serde(default = "default_true")]
    pub find_process: bool,
    /// Legacy bool (pre auto_select). Migrated on store load; not re-written.
    #[serde(default, skip_serializing)]
    pub smart_switch: bool,
}

fn default_probe_url() -> String {
    "https://www.gstatic.com/generate_204".into()
}

fn default_tun_stack() -> String {
    "mixed".into()
}

fn default_route_final() -> String {
    "proxy".into()
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

fn default_accent() -> String {
    "green".into()
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
            route_final: default_route_final(),
            close_to_tray: true,
            launch_at_login: false,
            silent_start: false,
            auto_start_proxy: false,
            close_connections_on_switch: true,
            locale: default_locale(),
            theme: default_theme(),
            accent: default_accent(),
            unload_ui_on_tray: false,
            auto_select: AutoSelectMode::Off,
            find_process: true,
            smart_switch: false,
        }
    }
}

impl AppSettings {
    /// Apply legacy `smart_switch: true` → `auto_select: smart` once.
    pub fn migrate_auto_select(&mut self) {
        if self.auto_select == AutoSelectMode::Off && self.smart_switch {
            self.auto_select = AutoSelectMode::Smart;
        }
        // Keep in-memory legacy flag aligned for any transitional readers.
        self.smart_switch = self.auto_select.is_smart();
    }

    /// Normalize `route.final` tag: proxy | direct | block.
    pub fn normalized_route_final(&self) -> &str {
        match self.route_final.to_ascii_lowercase().as_str() {
            "direct" => "direct",
            "block" => "block",
            _ => "proxy",
        }
    }
}

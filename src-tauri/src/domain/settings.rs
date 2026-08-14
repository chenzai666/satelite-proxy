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

/// Persisted traffic-capture preference. Runtime system proxy state is still
/// cleaned up on exit, then restored when the proxy starts again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    Off,
    System,
    Tun,
}

impl CaptureMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::System => "system",
            Self::Tun => "tun",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "system" => Some(Self::System),
            "tun" => Some(Self::Tun),
            _ => None,
        }
    }
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

/// Which tray / menu-bar mark to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrayIconStyle {
    /// Black rounded tile + white / mint satellite.
    #[default]
    Badge,
    /// Flat satellite on transparent; stopped is a macOS template.
    Mark,
    /// Pac-Man sheet ghost; white eyes stopped, mint eyes running.
    Ghost,
    /// head.jpg buddy; black shades off, green shades on.
    Buddy,
}

impl TrayIconStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Badge => "badge",
            Self::Mark => "mark",
            Self::Ghost => "ghost",
            Self::Buddy => "buddy",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "badge" | "tile" | "black" => Some(Self::Badge),
            "mark" | "white" | "flat" | "legacy" | "transparent" => Some(Self::Mark),
            "ghost" => Some(Self::Ghost),
            "buddy" | "cool" | "laoyou" | "head" => Some(Self::Buddy),
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
    /// Last selected traffic-capture mode: off | system | tun.
    #[serde(default)]
    pub capture_mode: CaptureMode,
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
    /// Menu-bar / tray mark: badge | mark | ghost | buddy.
    #[serde(default)]
    pub tray_icon: TrayIconStyle,
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
            capture_mode: CaptureMode::Off,
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
            tray_icon: TrayIconStyle::default(),
            unload_ui_on_tray: false,
            auto_select: AutoSelectMode::Off,
            find_process: true,
            smart_switch: false,
        }
    }
}

impl AppSettings {
    /// Infer the new capture preference from the legacy persisted TUN flag.
    pub fn migrate_capture_mode(&mut self) {
        if self.tun_enabled && self.capture_mode == CaptureMode::Off {
            self.capture_mode = CaptureMode::Tun;
        }
        self.tun_enabled = self.capture_mode == CaptureMode::Tun;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_tun_flag_migrates_to_capture_mode() {
        let mut settings = AppSettings {
            tun_enabled: true,
            capture_mode: CaptureMode::Off,
            ..AppSettings::default()
        };
        settings.migrate_capture_mode();
        assert_eq!(settings.capture_mode, CaptureMode::Tun);
        assert!(settings.tun_enabled);
    }

    #[test]
    fn system_capture_clears_stale_tun_flag() {
        let mut settings = AppSettings {
            tun_enabled: true,
            capture_mode: CaptureMode::System,
            ..AppSettings::default()
        };
        settings.migrate_capture_mode();
        assert_eq!(settings.capture_mode, CaptureMode::System);
        assert!(!settings.tun_enabled);
    }

    #[test]
    fn tray_icon_style_parses_and_defaults_to_badge() {
        assert_eq!(AppSettings::default().tray_icon, TrayIconStyle::Badge);
        assert_eq!(TrayIconStyle::parse("ghost"), Some(TrayIconStyle::Ghost));
        assert_eq!(TrayIconStyle::parse("legacy"), Some(TrayIconStyle::Mark));
        assert_eq!(TrayIconStyle::parse("laoyou"), Some(TrayIconStyle::Buddy));
        assert_eq!(TrayIconStyle::parse("nope"), None);
    }
}

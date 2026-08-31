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

/// How the direct outbound resolves a domain before dialing it.
///
/// This is intentionally independent from the DNS page. A DNS resolver may
/// return both A and AAAA records, while the direct outbound still needs a
/// deterministic dial policy on hosts whose IPv6 stack is unavailable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DirectIpStrategy {
    /// Keep dual-stack support, but attempt IPv4 before IPv6.
    #[default]
    #[serde(rename = "prefer_ipv4", alias = "prefer-ipv4", alias = "ipv4_prefer")]
    PreferIpv4,
    /// Never dial an IPv6 address through the direct outbound.
    #[serde(rename = "ipv4_only", alias = "ipv4-only", alias = "ipv4")]
    Ipv4Only,
    /// Leave the choice to the operating system and sing-box defaults.
    #[serde(rename = "system", alias = "default", alias = "auto")]
    System,
}

impl DirectIpStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PreferIpv4 => "prefer_ipv4",
            Self::Ipv4Only => "ipv4_only",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "prefer_ipv4" | "prefer-ipv4" | "ipv4_prefer" => Some(Self::PreferIpv4),
            "ipv4_only" | "ipv4-only" | "ipv4" => Some(Self::Ipv4Only),
            "system" | "default" | "auto" => Some(Self::System),
            _ => None,
        }
    }

    /// The resolver strategy is omitted for System so sing-box uses its
    /// native behavior instead of a second app-level policy. This maps to
    /// the supported `domain_resolver.strategy`, not legacy dial fields.
    pub fn singbox_domain_resolver_strategy(self) -> Option<&'static str> {
        match self {
            Self::PreferIpv4 => Some("prefer_ipv4"),
            Self::Ipv4Only => Some("ipv4_only"),
            Self::System => None,
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
    /// Danger mark; dim off, red-alert on.
    Danger,
    /// Danger mark on transparent bg (luminance-as-alpha).
    Danger2,
    /// Pac-Man sheet ghost, recolored: white/green body, black eyes.
    Ghost2,
    /// Face ID smiley; mint smile running, black frown (template) stopped.
    Faceid,
}

impl TrayIconStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Badge => "badge",
            Self::Mark => "mark",
            Self::Ghost => "ghost",
            Self::Buddy => "buddy",
            Self::Danger => "danger",
            Self::Danger2 => "danger2",
            Self::Ghost2 => "ghost2",
            Self::Faceid => "faceid",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "badge" | "tile" | "black" => Some(Self::Badge),
            "mark" | "white" | "flat" | "legacy" | "transparent" => Some(Self::Mark),
            "ghost" => Some(Self::Ghost),
            "buddy" | "cool" | "laoyou" | "head" => Some(Self::Buddy),
            "danger" | "warning" | "alert" => Some(Self::Danger),
            "danger2" => Some(Self::Danger2),
            "ghost2" => Some(Self::Ghost2),
            "faceid" | "face" | "smile" => Some(Self::Faceid),
            _ => None,
        }
    }
}

/// Extra inbound listener for the generated sing-box config.
/// `kind`: `mixed` | `http`; `allow_lan` decides the listen host
/// (0.0.0.0 vs 127.0.0.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraInbound {
    /// Stable row id (UI key / list identity).
    pub id: String,
    #[serde(default = "default_extra_inbound_kind")]
    pub kind: String,
    pub port: u16,
    #[serde(default)]
    pub allow_lan: bool,
}

fn default_extra_inbound_kind() -> String {
    "mixed".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// mixed inbound listen port
    pub mixed_port: u16,
    /// Mixed inbound listens on 0.0.0.0 (LAN) instead of 127.0.0.1.
    #[serde(default)]
    pub allow_lan: bool,
    /// clash_api controller port
    pub api_port: u16,
    /// Additional inbound listeners emitted into the generated config.
    #[serde(default)]
    pub extra_inbounds: Vec<ExtraInbound>,
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
    /// Include an IPv6 address on the TUN interface. Off by default: most
    /// budget VPS nodes have no IPv6 egress, and an IPv6-addressed tun makes
    /// the OS (Chrome especially) prefer AAAA/v6 and black-hole every
    /// connection when the node can't actually route v6 out. Turn on only if
    /// your node has real IPv6 egress.
    #[serde(default)]
    pub tun_ipv6_enabled: bool,
    /// Reject sniffed QUIC (UDP/443) traffic so browsers fall back to TCP.
    /// QUIC relayed through XUDP-in-TCP gets double congestion control
    /// (inner QUIC CC + outer TCP CC), which stutters video on mediocre
    /// links. Off by default — most users are fine with QUIC passthrough.
    #[serde(default)]
    pub block_quic: bool,
    /// Compatibility mode for Hysteria2/TUIC subscriptions whose servers use
    /// an incomplete or private certificate chain. When enabled, generated
    /// outbounds skip certificate verification for these two protocols only.
    /// TUIC ALPN normalization is safe and is applied independently.
    #[serde(default)]
    pub udp_tls_compat: bool,
    /// Bypass localhost and LAN segments (loopback, RFC1918 private ranges,
    /// link-local) as built-in direct rules appended after the rule sets.
    /// Not exposed as a rule set — it is a routing setting only.
    #[serde(default = "default_true")]
    pub bypass_lan: bool,
    /// Direct-outbound IP family policy. DNS can still resolve both A and
    /// AAAA records; this governs which resolved address the direct dialer
    /// uses. Prefer IPv4 by default for hosts without usable IPv6.
    #[serde(default)]
    pub direct_ip_strategy: DirectIpStrategy,
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
    /// Rebuild existing proxy connections after switching nodes. sing-box can
    /// interrupt the changed selector; Clash-compatible cores use their
    /// connection API.
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
    /// Background halo (glow) color: `accent` (follow the UI accent) |
    /// accent preset id | custom `#rrggbb`.
    #[serde(default = "default_glow_color")]
    pub glow_color: String,
    /// Overview hero visual: `particle` | `classic` | `smiley`.
    #[serde(default = "default_hero_style")]
    pub hero_style: String,
    /// Frosted-glass look for the repeated glass controls (seg / buttons /
    /// switches). Default true — measured memory cost is ~0 (backdrop blur
    /// costs frame-compositing time, not resident memory; see
    /// docs/webview2-memory-optimization-plan.md). "Lite" solid fills remain
    /// available for low-end GPUs.
    #[serde(default = "default_glass_frost")]
    pub glass_frost: bool,
    /// Menu-bar / tray mark: badge | mark | ghost | buddy.
    #[serde(default)]
    pub tray_icon: TrayIconStyle,
    /// Low-memory mode: when closing to tray, destroy WebView to free GPU/JS
    /// memory. Default true — the WebView tree costs 300-400MB resident and
    /// tray-only sessions don't need it. When true, next wake recreates the
    /// WebView (brief skeleton before React repaints).
    #[serde(default = "default_unload_ui_on_tray")]
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
    /// Which config the kernel should run: `generated` or `singbox:<profile_id>`.
    #[serde(default = "default_runtime_source")]
    pub runtime_source: String,
    /// Which core binary generates config and runs: `singbox` (default) | `xray`.
    /// Switching goes through the dedicated `set_core_type` command (restarts
    /// a running core); plain `update_settings` never touches it.
    #[serde(default = "default_core_type")]
    pub core_type: String,

    /// Multi-core mode master switch (sing-box main mode only): protocols
    /// pinned to a non-main core in `protocol_cores` are forwarded through a
    /// companion core process via loopback socks outbounds. Off (default) =
    /// every protocol follows the main core, unchanged behavior.
    #[serde(default)]
    pub multi_core_enabled: bool,
    /// Per-protocol core routing entries of the multi-core table. Only
    /// actual delegations are stored (v1: `core == "xray"`); protocols
    /// without an entry follow the main core. Per-node core support is
    /// re-checked at build time (REALITY+ws etc. fall back native).
    #[serde(default)]
    pub protocol_cores: Vec<ProtocolCoreItem>,
    /// Base port for the sidecar's per-node loopback inbounds; delegated
    /// node i listens on `base + i` (127.0.0.1 only).
    #[serde(default = "default_sidecar_port")]
    pub sidecar_port: u16,
}

/// One protocol→core row of the multi-core settings table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolCoreItem {
    /// `Protocol::as_str` value (e.g. `vless`).
    pub protocol: String,
    /// `CoreKind::as_str` value the protocol is pinned to (v1: `xray`).
    pub core: String,
}

fn default_sidecar_port() -> u16 {
    20890
}

fn default_runtime_source() -> String {
    "generated".into()
}

fn default_core_type() -> String {
    "singbox".into()
}

/// Kernel launch source. Custom sing-box profiles never overwrite `active.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSource {
    Generated,
    Singbox { id: String },
}

impl RuntimeSource {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if let Some(id) = raw.strip_prefix("singbox:") {
            let id = id.trim();
            if !id.is_empty() {
                return Self::Singbox { id: id.to_string() };
            }
        }
        Self::Generated
    }

    pub fn as_store_value(&self) -> String {
        match self {
            Self::Generated => "generated".into(),
            Self::Singbox { id } => format!("singbox:{id}"),
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Singbox { .. })
    }

    pub fn singbox_id(&self) -> Option<&str> {
        match self {
            Self::Singbox { id } => Some(id.as_str()),
            Self::Generated => None,
        }
    }
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

/// Low-memory mode defaults ON — the WebView2 tree holds 300-400MB resident
/// and tray-only sessions don't need it (docs/webview2-memory-optimization-plan.md).
fn default_unload_ui_on_tray() -> bool {
    true
}

/// Frosted controls by default: measured memory delta is ~0 (blur costs
/// compositing time, not resident memory) — see the plan doc's P0-1 correction.
fn default_glass_frost() -> bool {
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

fn default_glow_color() -> String {
    "accent".into()
}

fn default_hero_style() -> String {
    "particle".into()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            mixed_port: 2080,
            allow_lan: false,
            api_port: 19090,
            extra_inbounds: Vec::new(),
            current_node_id: None,
            clash_api_secret: None,
            probe_url: default_probe_url(),
            mix_mode: false,
            tun_enabled: false,
            capture_mode: CaptureMode::Off,
            tun_stack: default_tun_stack(),
            tun_ipv6_enabled: false,
            block_quic: false,
            udp_tls_compat: false,
            bypass_lan: true,
            direct_ip_strategy: DirectIpStrategy::PreferIpv4,
            outbound_mode: OutboundMode::Rule,
            route_final: default_route_final(),
            close_to_tray: true,
            launch_at_login: false,
            silent_start: false,
            auto_start_proxy: false,
            close_connections_on_switch: false,
            locale: default_locale(),
            theme: default_theme(),
            accent: default_accent(),
            glow_color: default_glow_color(),
            hero_style: default_hero_style(),
            glass_frost: default_glass_frost(),
            tray_icon: TrayIconStyle::default(),
            unload_ui_on_tray: default_unload_ui_on_tray(),
            auto_select: AutoSelectMode::Off,
            find_process: true,
            smart_switch: false,
            runtime_source: default_runtime_source(),
            core_type: default_core_type(),
            multi_core_enabled: false,
            protocol_cores: Vec::new(),
            sidecar_port: default_sidecar_port(),
        }
    }
}

impl AppSettings {
    pub fn runtime_source(&self) -> RuntimeSource {
        RuntimeSource::parse(&self.runtime_source)
    }

    pub fn set_runtime_source(&mut self, source: RuntimeSource) {
        self.runtime_source = source.as_store_value();
    }

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

    /// Multi-core delegation only exists under the sing-box main core — the
    /// sidecar hangs off the sing-box generated config. `set_core_type` uses
    /// this to auto-disable the mode when switching to another core (protocol
    /// pins are kept so switching back only needs the switch re-flipped), and
    /// `update_settings` rejects enabling it under any other core.
    pub fn multi_core_available(&self) -> bool {
        self.core_type == "singbox"
    }

    /// Keep `multi_core_enabled` consistent with `core_type` after a core
    /// switch. Cheap no-op when the mode is off or sing-box stays active.
    pub fn enforce_multi_core_scope(&mut self) {
        if self.multi_core_enabled && !self.multi_core_available() {
            self.multi_core_enabled = false;
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
    fn direct_ip_strategy_defaults_to_prefer_ipv4_and_parses_all_choices() {
        assert_eq!(
            AppSettings::default().direct_ip_strategy,
            DirectIpStrategy::PreferIpv4
        );
        assert_eq!(
            DirectIpStrategy::parse("ipv4_only"),
            Some(DirectIpStrategy::Ipv4Only)
        );
        assert_eq!(
            DirectIpStrategy::parse("system"),
            Some(DirectIpStrategy::System)
        );
        assert_eq!(
            DirectIpStrategy::System.singbox_domain_resolver_strategy(),
            None
        );
        assert_eq!(
            serde_json::to_string(&DirectIpStrategy::Ipv4Only).unwrap(),
            r#""ipv4_only""#
        );
        assert_eq!(
            serde_json::from_str::<DirectIpStrategy>(r#""ipv4_only""#).unwrap(),
            DirectIpStrategy::Ipv4Only
        );
    }

    #[test]
    fn core_switch_auto_disables_multi_core_but_keeps_pins() {
        let mut settings = AppSettings {
            core_type: "singbox".into(),
            multi_core_enabled: true,
            protocol_cores: vec![ProtocolCoreItem {
                protocol: "vless".into(),
                core: "xray".into(),
            }],
            ..AppSettings::default()
        };
        assert!(settings.multi_core_available());

        // Switching to Xray flips the mode off but keeps the table pins so
        // switching back to sing-box restores the previous selection.
        settings.core_type = "xray".into();
        settings.enforce_multi_core_scope();
        assert!(!settings.multi_core_enabled);
        assert_eq!(settings.protocol_cores.len(), 1);

        // Back on sing-box the user only needs to re-enable the switch.
        settings.core_type = "singbox".into();
        settings.multi_core_enabled = true;
        settings.enforce_multi_core_scope();
        assert!(settings.multi_core_enabled);
    }

    #[test]
    fn new_store_keeps_api_secret_disabled_by_default() {
        // A clean/new AppSettings never had a secret, so the migration is a
        // no-op: users who never turned this on don't get surprised by it.
        let mut settings = AppSettings::default();
        settings.migrate_api_secret_enabled();
        assert!(!settings.api_secret_enabled);
    }

    #[test]
    fn store_with_a_preexisting_secret_migrates_to_enabled() {
        // Pre-toggle stores always carried a secret unconditionally — treat
        // that as "was enabled" so upgrading doesn't drop anyone's auth.
        let mut settings = AppSettings {
            clash_api_secret: Some("abc123".into()),
            ..AppSettings::default()
        };
        settings.migrate_api_secret_enabled();
        assert!(settings.api_secret_enabled);
    }

    #[test]
    fn store_with_an_empty_secret_field_stays_disabled() {
        let mut settings = AppSettings {
            clash_api_secret: Some("   ".into()),
            ..AppSettings::default()
        };
        settings.migrate_api_secret_enabled();
        assert!(!settings.api_secret_enabled);
    }

    #[test]
    fn tray_icon_style_parses_and_defaults_to_badge() {
        assert_eq!(AppSettings::default().tray_icon, TrayIconStyle::Badge);
        assert_eq!(TrayIconStyle::parse("ghost"), Some(TrayIconStyle::Ghost));
        assert_eq!(TrayIconStyle::parse("legacy"), Some(TrayIconStyle::Mark));
        assert_eq!(TrayIconStyle::parse("laoyou"), Some(TrayIconStyle::Buddy));
        assert_eq!(TrayIconStyle::parse("warning"), Some(TrayIconStyle::Danger));
        assert_eq!(
            TrayIconStyle::parse("danger2"),
            Some(TrayIconStyle::Danger2)
        );
        assert_eq!(TrayIconStyle::parse("ghost2"), Some(TrayIconStyle::Ghost2));
        assert_eq!(TrayIconStyle::parse("faceid"), Some(TrayIconStyle::Faceid));
        assert_eq!(TrayIconStyle::parse("nope"), None);
    }

    #[test]
    fn runtime_source_roundtrip() {
        assert_eq!(RuntimeSource::parse(""), RuntimeSource::Generated);
        assert_eq!(RuntimeSource::parse("generated"), RuntimeSource::Generated);
        assert_eq!(
            RuntimeSource::parse("singbox:abc"),
            RuntimeSource::Singbox { id: "abc".into() }
        );
        let mut settings = AppSettings::default();
        settings.set_runtime_source(RuntimeSource::Singbox { id: "p1".into() });
        assert!(settings.runtime_source().is_custom());
        settings.set_runtime_source(RuntimeSource::Generated);
        assert!(!settings.runtime_source().is_custom());
    }

    #[test]
    fn extra_inbounds_default_when_missing_and_roundtrip() {
        // Old store JSON without the field loads with an empty list.
        let legacy = r#"{"mixed_port":2080,"api_port":19090}"#;
        let settings: AppSettings = serde_json::from_str(legacy).unwrap();
        assert!(settings.extra_inbounds.is_empty());

        // New entries survive a serde round-trip; kind defaults to mixed.
        let raw = r#"{"id":"i1","port":2081,"allow_lan":true}"#;
        let inbound: ExtraInbound = serde_json::from_str(raw).unwrap();
        assert_eq!(inbound.kind, "mixed");
        let settings = AppSettings {
            extra_inbounds: vec![inbound],
            ..AppSettings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.extra_inbounds, settings.extra_inbounds);
        assert!(json.contains("\"allow_lan\":true"));
    }
}

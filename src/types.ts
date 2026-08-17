export type NavKey =
  | "dashboard"
  | "config"
  | "nodes"
  | "traffic"
  | "logs"
  | "settings";

export type DnsFinalStrategy = "local" | "domestic" | "remote";
export type DomainMatcher = "domain" | "domain_suffix" | "domain_keyword";

export type DnsAction =
  | { kind: "local" }
  | { kind: "domestic" }
  | { kind: "remote" }
  | { kind: "block" };

export interface DnsRule {
  id: string;
  enabled: boolean;
  matcher: DomainMatcher;
  payload: string;
  action: DnsAction;
}

export interface FakeIpConfig {
  enabled: boolean;
  inet4_range: string;
  inet6_enabled: boolean;
  inet6_range: string;
  bypass: string[];
}

export interface HostsEntry {
  id: string;
  enabled: boolean;
  domain: string;
  addr: string;
}

export interface HostsConfig {
  enabled: boolean;
  include_system: boolean;
  entries: HostsEntry[];
}

export type DnsRuleSetKind = "dns" | "hosts";

export interface DnsRuleSet {
  id: string;
  name: string;
  kind: DnsRuleSetKind;
  builtin: boolean;
  read_only: boolean;
  enabled: boolean;
  dns_rules: DnsRule[];
  hosts: HostsEntry[];
}

export interface DnsSettings {
  enabled: boolean;
  rules_enabled: boolean;
  rules: DnsRule[];
  fake_ip: FakeIpConfig;
  hosts: HostsConfig;
  rule_sets: DnsRuleSet[];
  unified_rules: boolean;
  hijack: boolean;
  cache: boolean;
  leak_protect: boolean;
  /** Default resolver for domains unmatched by a rule set. */
  dns_final: DnsFinalStrategy;
}

export interface DnsTestResult {
  domain: string;
  ok: boolean;
  addrs: string[];
  elapsed_ms: number;
  error?: string | null;
  note: string;
}

/** From subscription-userinfo header and/or remark node names. */
export interface SubscriptionTraffic {
  upload?: number | null;
  download?: number | null;
  total?: number | null;
  /** Explicit remaining bytes (e.g. from `剩余流量：2.41 TB`). */
  quota_remaining?: number | null;
  expire?: number | null;
  /** Human-readable expire when not a unix timestamp (e.g. `长期有效`). */
  expire_text?: string | null;
}

export interface SubscriptionView {
  id: string;
  name: string;
  source_kind: "url" | "file" | "text" | "node" | "singbox" | string;
  source_display: string;
  last_update: number;
  node_count: number;
  enabled: boolean;
  format?: string | null;
  skipped_count: number;
  /** Periodically refresh this profile. */
  auto_update?: boolean;
  /** Minutes between auto updates (default 1440). */
  auto_update_interval_min?: number;
  traffic?: SubscriptionTraffic | null;
}

/** Full subscription for edit form (raw url/path). */
export interface SubscriptionDetail {
  id: string;
  name: string;
  source_kind: "url" | "file" | "text" | "node" | "singbox" | string;
  url?: string | null;
  path?: string | null;
  content?: string | null;
  uri?: string | null;
  node?: ManualNodeDraft | null;
  last_update: number;
  node_count: number;
  enabled: boolean;
  format?: string | null;
  skipped_count: number;
  via_proxy: boolean;
  auto_update?: boolean;
  auto_update_interval_min?: number;
  traffic?: SubscriptionTraffic | null;
}

export interface SubscriptionUrlEntry {
  id: string;
  url: string;
}

export interface ImportResult {
  subscription: SubscriptionView;
  node_count: number;
  skipped_count: number;
}

export interface ProxyNode {
  id: string;
  name: string;
  protocol: string;
  server: string;
  port: number;
  source?: string;
  latency_ms?: number | null;
  latency_at?: number | null;
  /** Present from list_all_nodes — owning subscription. */
  subscription_id?: string;
  subscription_name?: string;
}

export type ViewMode = "list" | "grid";
export type SortMode = "default" | "name" | "latency";

export interface NodePage {
  nodes: ProxyNode[];
  total: number;
  offset: number;
}

export interface LatencyResult {
  id: string;
  name: string;
  latency_ms?: number | null;
  error?: string | null;
  tested_at: number;
}

export interface LatencyBatchResult {
  results: LatencyResult[];
  tested: number;
  ok: number;
  failed: number;
  method?: string;
}

export type AddSourceKind = "url" | "file" | "text" | "node" | "singbox";

export type ProfileKind = "subscription" | "local" | "singbox";
export type LocalKind = "node" | "multi";
export type ConfigInputMode = "paste" | "file";

/** Flattened single-node form. Field names match the Rust `ManualNodeDraft`. */
export interface ManualNodeDraft {
  protocol: string;
  server: string;
  port: number;
  name?: string | null;
  password?: string | null;
  uuid?: string | null;
  method?: string | null;
  plugin?: string | null;
  pluginOpts?: string | null;
  alterId?: number | null;
  security?: string | null;
  flow?: string | null;
  packetEncoding?: string | null;
  username?: string | null;
  path?: string | null;
  upMbps?: number | null;
  downMbps?: number | null;
  obfs?: string | null;
  obfsPassword?: string | null;
  congestionControl?: string | null;
  udpRelayMode?: string | null;
  zeroRttHandshake?: boolean | null;
  psk?: string | null;
  version?: number | null;
  user?: string | null;
  privateKey?: string | null;
  privateKeyPassphrase?: string | null;
  peerPublicKey?: string | null;
  localAddress?: string | null;
  preSharedKey?: string | null;
  mtu?: number | null;
  quic?: boolean | null;
  executablePath?: string | null;
  tls?: boolean | null;
  sni?: string | null;
  insecure?: boolean | null;
  alpn?: string | null;
  fingerprint?: string | null;
  realityPublicKey?: string | null;
  realityShortId?: string | null;
  network?: string | null;
  host?: string | null;
  serviceName?: string | null;
  udp?: boolean | null;
}

/** Clash-style routing mode. */
export type OutboundMode = "rule" | "global" | "direct";

/** Extra sing-box inbound listener (settings-managed). */
export interface ExtraInbound {
  id: string;
  /** mixed | http */
  kind: "mixed" | "http";
  port: number;
  /** true → listen 0.0.0.0, false → 127.0.0.1 */
  allow_lan?: boolean;
}

export interface AppSettings {
  mixed_port: number;
  api_port: number;
  /** Additional mixed/http listeners emitted into the generated config. */
  extra_inbounds?: ExtraInbound[];
  current_node_id?: string | null;
  clash_api_secret?: string | null;
  probe_url: string;
  /** Multi-subscription enable (Mix). */
  mix_mode?: boolean;
  /** sing-box TUN inbound (global capture). */
  tun_enabled?: boolean;
  /** Persisted traffic capture preference. */
  capture_mode?: "off" | "system" | "tun";
  /** system | gvisor | mixed */
  tun_stack?: string;
  /** rule | global | direct */
  outbound_mode?: OutboundMode;
  /** route.final in Rule mode: proxy | direct | block */
  route_final?: "proxy" | "direct" | "block" | string;
  /** Close window → tray (keep process + core). */
  close_to_tray?: boolean;
  /** Launch at OS login. */
  launch_at_login?: boolean;
  /** Start without showing main window. */
  silent_start?: boolean;
  /** Auto-start proxy after app launch. */
  auto_start_proxy?: boolean;
  /** Close all connections when switching node. */
  close_connections_on_switch?: boolean;
  /** UI language: zh | en (sidebar stays English). */
  locale?: string;
  /** UI theme: aerospace | day */
  theme?: string;
  /** UI accent (brand/primary color) preset id, e.g. green | blue | purple ... */
  accent?: string;
  /** Overview hero visual: particle | classic */
  hero_style?: HeroStyle;
  /** Tray mark: badge | mark | ghost | buddy */
  tray_icon?: TrayIconStyle;
  /** Destroy WebView when closing to tray (free GPU/JS; tray+core stay). */
  unload_ui_on_tray?: boolean;
  /** off | smart | kernel — node auto-select mode. */
  auto_select?: AutoSelectMode;
  /** Resolve originating process per connection (sing-box find_process_mode). */
  find_process?: boolean;
  /** @deprecated derived from auto_select === "smart" */
  smart_switch?: boolean;
  /** `generated` or `singbox:<profile_id>`. */
  runtime_source?: string;
}

/** Manual / app smart switch / sing-box urltest. */
export type AutoSelectMode = "off" | "smart" | "kernel";

export type ThemeId = "aerospace" | "day";

export type HeroStyle = "particle" | "classic";

export type TrayIconStyle = "badge" | "mark" | "ghost" | "buddy";

export interface GenerateConfigResult {
  path: string;
  selected_tag: string;
  outbound_count: number;
  mixed_port: number;
  api_port: number;
  preview: string;
}

export interface CoreInfo {
  installed: boolean;
  version?: string | null;
  path?: string | null;
  platform: string;
  latest_version?: string | null;
  update_available: boolean;
  /** bundled | downloaded | missing */
  source: string;
  bundled_version?: string | null;
}

export interface CoreDownloadResult {
  version: string;
  path: string;
  asset_name: string;
  platform: string;
  bytes: number;
}

export interface CoreDownloadProgress {
  stage: "preparing" | "downloading" | "installing" | "done";
  downloaded: number;
  total?: number | null;
  percent?: number | null;
  via_proxy: boolean;
}

export type CoreState =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "error";

export interface ProxyStatus {
  running: boolean;
  core_state: CoreState;
  system_proxy: boolean;
  tun_enabled: boolean;
  /** Persisted desired traffic capture mode. */
  capture_mode?: "off" | "system" | "tun";
  /** rule | global | direct */
  outbound_mode: string;
  mixed_port: number;
  api_port: number;
  current_node_id?: string | null;
  error?: string | null;
  core_path?: string | null;
  config_path?: string | null;
  upload_speed: number;
  download_speed: number;
  upload_total: number;
  download_total: number;
  connections: number;
  /** @deprecated use auto_select === "smart" */
  smart_switch?: boolean;
  /** off | smart | kernel */
  auto_select?: AutoSelectMode | string;
  /** Unix seconds when core last started (uptime = now - this). */
  core_started_at?: number | null;
  /** `generated` or `singbox`. */
  runtime_source?: string;
  runtime_profile_id?: string | null;
  runtime_profile_name?: string | null;
  custom_has_clash_api?: boolean;
  custom_has_tun?: boolean;
  custom_inbound_port?: number | null;
}

export type RuleType =
  | "domain"
  | "domain_suffix"
  | "domain_keyword"
  | "ip_cidr"
  | "process"
  | "geoip";

export interface RuleSetSummary {
  id: string;
  name: string;
  builtin: boolean;
  rule_count: number;
  /** Multiple sets can be enabled and merged for routing. */
  enabled: boolean;
  ownership: "builtin" | "user" | "system";
  strategy: RuleSetStrategy;
  dns_strategy: RuleSetDnsStrategy;
  remote?: RemoteRuleSetConfig | null;
}

export type RuleSetStrategy = "proxy" | "direct" | "block" | "smart";
export type RuleSetDnsStrategy = "local" | "domestic" | "remote";

export interface RemoteRuleSetConfig {
  url: string;
  format: "source" | "binary" | string;
  update_interval: "disabled" | "1h" | "12h" | "24h" | string;
  target: "proxy" | "direct" | "block";
  local_path?: string | null;
  download_status?: "idle" | "downloading" | "ready" | "error" | string;
  download_error?: string | null;
  last_update?: number | null;
  last_attempt?: number | null;
  rule_count?: number | null;
}

export interface RemoteRuleItem {
  index: number;
  kind: string;
  summary: string;
  raw: string;
  raw_truncated: boolean;
  complex: boolean;
}

export interface RemoteRulePage {
  total: number;
  offset: number;
  limit: number;
  items: RemoteRuleItem[];
}

export interface RuleSet {
  id: string;
  name: string;
  builtin: boolean;
  enabled: boolean;
  ownership: "builtin" | "user" | "system";
  strategy: RuleSetStrategy;
  dns_strategy: RuleSetDnsStrategy;
  remote?: RemoteRuleSetConfig | null;
  dns_rules: DnsRule[];
  rules: Rule[];
}

export type RuleTarget = "direct" | "proxy" | "block" | "node" | "smart";

export interface Rule {
  id: string;
  ord: number;
  type: RuleType;
  payload: string;
  target: RuleTarget;
  enabled: boolean;
  /** When target is `node`: pinned subscription node id. */
  node_id?: string | null;
  /** Snapshot name at save time (stale UI when id missing). */
  node_name?: string | null;
  /** Smart mode whitelist: name must contain any keyword (OR). Empty = no whitelist. */
  smart_include?: string[];
  /** Smart mode blacklist: name containing any keyword is skipped (OR). */
  smart_exclude?: string[];
}

/** Live connection or historical request row */
export interface ConnectionView {
  id: string;
  destination: string;
  host: string;
  network: string;
  conn_type: string;
  node_tag: string;
  node_name: string;
  /** Owning subscription name (for tooltip). */
  subscription_name?: string;
  chains: string[];
  chains_display: string;
  rule: string;
  rule_payload: string;
  process: string;
  source: string;
  upload: number;
  download: number;
  start: string;
  first_seen?: number | null;
  last_seen?: number | null;
  closed?: boolean;
  closed_at?: number | null;
}

export interface LiveConnectionBatch {
  rows: ConnectionView[];
  removed_ids: string[];
  order_ids: string[];
  revision: number;
  unchanged: boolean;
  full: boolean;
}

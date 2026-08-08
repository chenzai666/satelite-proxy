export type NavKey =
  | "dashboard"
  | "config"
  | "nodes"
  | "traffic"
  | "logs"
  | "settings";

export type DnsMode = "local" | "smart_local" | "smart_cn";
export type DomainMatcher = "domain" | "domain_suffix" | "domain_keyword";

export type DnsAction =
  | { kind: "local" }
  | { kind: "domestic" }
  | { kind: "remote" };

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
  mode: DnsMode;
  rules_enabled: boolean;
  rules: DnsRule[];
  fake_ip: FakeIpConfig;
  hosts: HostsConfig;
  rule_sets: DnsRuleSet[];
  hijack: boolean;
  cache: boolean;
  leak_protect: boolean;
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
  source_kind: "url" | "file" | string;
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
  source_kind: "url" | "file" | string;
  url?: string | null;
  path?: string | null;
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

export type AddSourceKind = "url" | "file";

/** Clash-style routing mode. */
export type OutboundMode = "rule" | "global" | "direct";

export interface AppSettings {
  mixed_port: number;
  api_port: number;
  current_node_id?: string | null;
  clash_api_secret?: string | null;
  probe_url: string;
  /** Multi-subscription enable (Mix). */
  mix_mode?: boolean;
  /** sing-box TUN inbound (global capture). */
  tun_enabled?: boolean;
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
  /** Destroy WebView when closing to tray (free GPU/JS; tray+core stay). */
  unload_ui_on_tray?: boolean;
  /** off | smart | kernel — node auto-select mode. */
  auto_select?: AutoSelectMode;
  /** @deprecated derived from auto_select === "smart" */
  smart_switch?: boolean;
}

/** Manual / app smart switch / sing-box urltest. */
export type AutoSelectMode = "off" | "smart" | "kernel";

export type ThemeId = "aerospace" | "day";

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
}

export interface RuleSet {
  id: string;
  name: string;
  builtin: boolean;
  enabled: boolean;
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

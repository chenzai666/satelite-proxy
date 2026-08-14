import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  CoreDownloadResult,
  CoreInfo,
  GenerateConfigResult,
  ImportResult,
  LatencyBatchResult,
  ConnectionView,
  ProxyNode,
  ProxyStatus,
  Rule,
  RuleSet,
  RuleSetStrategy,
  RuleSetDnsStrategy,
  RuleSetSummary,
  RuleTarget,
  RuleType,
  SubscriptionDetail,
  SubscriptionView,
  DnsSettings,
  DnsTestResult,
  HostsEntry,
} from "./types";
import { trackCoreBusy } from "./coreBusy";

export function listSubscriptions() {
  return invoke<SubscriptionView[]>("list_subscriptions");
}

export function getSubscription(id: string) {
  return invoke<SubscriptionDetail>("get_subscription", { id });
}

export function addSubscriptionUrl(
  name: string | null,
  url: string,
  viaProxy = false,
  autoUpdate = false,
  autoUpdateIntervalMin = 1440,
) {
  return invoke<ImportResult>("add_subscription_url", {
    name,
    url,
    viaProxy,
    autoUpdate,
    autoUpdateIntervalMin,
  });
}

export function addSubscriptionFile(
  name: string | null,
  path: string,
  autoUpdate = false,
  autoUpdateIntervalMin = 1440,
) {
  return invoke<ImportResult>("add_subscription_file", {
    name,
    path,
    autoUpdate,
    autoUpdateIntervalMin,
  });
}

export function updateSubscription(input: {
  id: string;
  name: string | null;
  kind: "url" | "file";
  url?: string | null;
  path?: string | null;
  viaProxy?: boolean | null;
  autoUpdate?: boolean | null;
  autoUpdateIntervalMin?: number | null;
}) {
  return invoke<ImportResult>("update_subscription", {
    id: input.id,
    name: input.name,
    kind: input.kind,
    url: input.url ?? null,
    path: input.path ?? null,
    viaProxy: input.viaProxy ?? null,
    autoUpdate: input.autoUpdate ?? null,
    autoUpdateIntervalMin: input.autoUpdateIntervalMin ?? null,
  });
}

export function refreshSubscription(id: string, viaProxy?: boolean | null) {
  return invoke<ImportResult>("refresh_subscription", {
    id,
    viaProxy: viaProxy ?? null,
  });
}

export function removeSubscription(id: string) {
  return invoke<void>("remove_subscription", { id });
}

/** Exclusive select / Mix toggle. Returns updated subscription list. */
export function activateSubscription(id: string) {
  return invoke<SubscriptionView[]>("activate_subscription", { id });
}

export function setMixMode(mix: boolean) {
  return invoke<AppSettings>("set_mix_mode", { mix });
}

export function listSubscriptionNodes(id: string) {
  return invoke<ProxyNode[]>("list_subscription_nodes", { id });
}

export function listAllNodes() {
  return invoke<ProxyNode[]>("list_all_nodes");
}

export function getSettings() {
  return invoke<AppSettings>("get_settings");
}

export function updateSettings(payload: {
  mixedPort?: number | null;
  apiPort?: number | null;
  probeUrl?: string | null;
  tunEnabled?: boolean | null;
  tunStack?: string | null;
  closeToTray?: boolean | null;
  launchAtLogin?: boolean | null;
  silentStart?: boolean | null;
  autoStartProxy?: boolean | null;
  closeConnectionsOnSwitch?: boolean | null;
  locale?: string | null;
  theme?: string | null;
  accent?: string | null;
  unloadUiOnTray?: boolean | null;
  /** @deprecated prefer autoSelect */
  smartSwitch?: boolean | null;
  /** off | smart | kernel */
  autoSelect?: string | null;
  /** proxy | direct | block — route.final in Rule mode */
  routeFinal?: string | null;
  /** Resolve originating process per connection (find_process_mode). */
  findProcess?: boolean | null;
}) {
  return invoke<AppSettings>("update_settings", {
    mixedPort: payload.mixedPort ?? null,
    apiPort: payload.apiPort ?? null,
    probeUrl: payload.probeUrl ?? null,
    tunEnabled: payload.tunEnabled ?? null,
    tunStack: payload.tunStack ?? null,
    closeToTray: payload.closeToTray ?? null,
    launchAtLogin: payload.launchAtLogin ?? null,
    silentStart: payload.silentStart ?? null,
    autoStartProxy: payload.autoStartProxy ?? null,
    closeConnectionsOnSwitch: payload.closeConnectionsOnSwitch ?? null,
    locale: payload.locale ?? null,
    theme: payload.theme ?? null,
    accent: payload.accent ?? null,
    unloadUiOnTray: payload.unloadUiOnTray ?? null,
    smartSwitch: payload.smartSwitch ?? null,
    autoSelect: payload.autoSelect ?? null,
    routeFinal: payload.routeFinal ?? null,
    findProcess: payload.findProcess ?? null,
  });
}

export function setCurrentNode(nodeId: string) {
  return invoke<AppSettings>("set_current_node", { nodeId });
}

export interface SmartSwitchNowResult {
  switched: boolean;
  from_id?: string | null;
  to_id?: string | null;
  to_name?: string | null;
  latency_ms?: number | null;
  probed: number;
  message: string;
}

/** Probe candidates and switch to best node (used when enabling smart switch). */
export function smartSwitchNow() {
  return invoke<SmartSwitchNowResult>("smart_switch_now");
}

export type AppLogLevel = "trace" | "debug" | "info" | "warn" | "error";

export interface AppLogEntry {
  id: number;
  ts_ms: number;
  level: AppLogLevel;
  target: string;
  message: string;
}

export function listAppLogs(opts?: {
  minLevel?: AppLogLevel | null;
  limit?: number | null;
  query?: string | null;
}) {
  return invoke<AppLogEntry[]>("list_app_logs", {
    minLevel: opts?.minLevel ?? "info",
    limit: opts?.limit ?? 500,
    query: opts?.query ?? null,
  });
}

export function clearAppLogs() {
  return invoke<void>("clear_app_logs");
}

export function generateSingboxConfig() {
  return invoke<GenerateConfigResult>("generate_singbox_config");
}

export function previewSingboxConfig() {
  return invoke<GenerateConfigResult>("preview_singbox_config");
}

export function getActiveConfigPath() {
  return invoke<string | null>("get_active_config_path");
}

/** Local only — no network. Use for first paint. */
export function getCoreInfo() {
  return invoke<CoreInfo>("get_core_info");
}

export function checkCoreUpdate(localVersion?: string | null) {
  return invoke<{
    latest_version: string;
    update_available: boolean;
    asset_name: string;
    size: number;
  }>("check_core_update", { localVersion: localVersion ?? null });
}

export function downloadCore(tag?: string | null) {
  return invoke<CoreDownloadResult>("download_core", { tag: tag ?? null });
}

export function fetchCoreLatest() {
  return invoke<{
    version: string;
    asset_name: string;
    download_url: string;
    size: number;
    platform: string;
  }>("fetch_core_latest");
}

export function testNodesLatency(ids?: string[] | null, timeoutMs?: number | null) {
  // Tauri 2 accepts camelCase; include snake_case for compatibility.
  const args: Record<string, unknown> = {
    ids: ids ?? null,
    timeoutMs: timeoutMs ?? null,
    timeout_ms: timeoutMs ?? null,
  };
  return invoke<LatencyBatchResult>("test_nodes_latency", args);
}

export function getProxyStatus() {
  return invoke<ProxyStatus>("get_proxy_status");
}

export function startProxy(enableSystemProxy = false) {
  return trackCoreBusy(
    invoke<ProxyStatus>("start_proxy", {
      enableSystemProxy,
    }),
  );
}

export function stopProxy() {
  return trackCoreBusy(invoke<ProxyStatus>("stop_proxy"));
}

export function restartProxy() {
  // Slightly longer min hold so ⋯ / Overview restart never flash-clears.
  return trackCoreBusy(invoke<ProxyStatus>("restart_proxy"), 700);
}

export function setSystemProxy(enabled: boolean) {
  return invoke<ProxyStatus>("set_system_proxy", { enabled });
}

/** Toggle TUN; restarts core when running so config applies. */
export function setTunEnabled(enabled: boolean) {
  return trackCoreBusy(invoke<ProxyStatus>("set_tun_enabled", { enabled }));
}

/** Traffic capture mode: off | system | tun (mutually exclusive). */
export function setCaptureMode(mode: "off" | "system" | "tun") {
  return trackCoreBusy(invoke<ProxyStatus>("set_capture_mode", { mode }));
}

/** rule | global | direct — restarts core when running. */
export function setOutboundMode(mode: "rule" | "global" | "direct") {
  return trackCoreBusy(invoke<ProxyStatus>("set_outbound_mode", { mode }));
}

export function getDnsSettings() {
  return invoke<DnsSettings>("get_dns_settings");
}

export function updateDnsSettings(settings: DnsSettings, apply = true) {
  return invoke<DnsSettings>("update_dns_settings", { settings, apply });
}

/** Reset DNS servers or rules to factory defaults (`"servers"` | `"rules"`). */
export function resetDnsDefaults(section: "servers" | "rules", apply = true) {
  return invoke<DnsSettings>("reset_dns_defaults", { section, apply });
}

export function testDnsLookup(domain: string) {
  return invoke<DnsTestResult>("test_dns_lookup", { domain });
}

/** Read the OS hosts file as a read-only entry list (for the Hosts UI). */
export function readSystemHosts() {
  return invoke<HostsEntry[]>("read_system_hosts");
}

export function listRuleSets() {
  return invoke<RuleSetSummary[]>("list_rule_sets");
}

export function getRuleSet(id: string) {
  return invoke<RuleSet>("get_rule_set", { id });
}

export function setActiveRuleSet(id: string) {
  return invoke<void>("set_active_rule_set", { id });
}

export function setRuleSetEnabled(id: string, enabled: boolean) {
  return invoke<void>("set_rule_set_enabled", { id, enabled });
}

export function setRuleSetStrategy(id: string, strategy: RuleSetStrategy) {
  return invoke<RuleSet>("set_rule_set_strategy", { id, strategy });
}

export function setRuleSetDnsStrategy(id: string, strategy: RuleSetDnsStrategy) {
  return invoke<RuleSet>("set_rule_set_dns_strategy", { id, strategy });
}

export function createRuleSet(
  name: string,
  remoteUrl?: string | null,
  target?: "proxy" | "direct" | "block" | null,
  updateInterval?: "disabled" | "1h" | "12h" | "24h" | null,
) {
  return invoke<RuleSet>("create_rule_set", {
    name,
    remoteUrl: remoteUrl ?? null,
    target: target ?? null,
    updateInterval: updateInterval ?? null,
  });
}

export function refreshRemoteRuleSet(id: string) {
  return invoke<RuleSet>("refresh_remote_rule_set", { id });
}

export function updateRuleSet(
  id: string,
  name: string,
  remoteUrl?: string | null,
  updateInterval?: "disabled" | "1h" | "12h" | "24h" | null,
) {
  return invoke<RuleSet>("update_rule_set", {
    id,
    name,
    remoteUrl: remoteUrl ?? null,
    updateInterval: updateInterval ?? null,
  });
}

export function listRemoteRuleItems(
  id: string,
  offset: number,
  limit: number,
  query?: string,
) {
  return invoke<import("./types").RemoteRulePage>("list_remote_rule_items", {
    id,
    offset,
    limit,
    query: query?.trim() || null,
  });
}

/** First id = highest match priority. Restarts core when running. */
export function reorderRuleSets(ids: string[]) {
  return invoke<RuleSetSummary[]>("reorder_rule_sets", { ids });
}

export function deleteRuleSet(id: string) {
  return invoke<void>("delete_rule_set", { id });
}

/** Reset one builtin factory set from resources. */
export function resetRuleSet(id: string) {
  return invoke<RuleSet>("reset_rule_set", { id });
}

/** Legacy: reset all builtin-* sets only. Prefer `resetRuleSet(id)`. */
export function resetBuiltinRuleSet() {
  return invoke<RuleSet>("reset_builtin_rule_set");
}

export function listRules(setId?: string | null) {
  return invoke<Rule[]>("list_rules", { setId: setId ?? null });
}

export function saveRule(input: {
  setId?: string | null;
  id?: string | null;
  ruleType: RuleType;
  payload: string;
  target: RuleTarget;
  ord?: number | null;
  enabled?: boolean | null;
  nodeId?: string | null;
  smartInclude?: string[] | null;
  smartExclude?: string[] | null;
}) {
  return invoke<Rule>("save_rule", {
    input: {
      set_id: input.setId ?? null,
      id: input.id ?? null,
      rule_type: input.ruleType,
      payload: input.payload,
      target: input.target,
      ord: input.ord ?? null,
      enabled: input.enabled ?? null,
      node_id: input.nodeId ?? null,
      smart_include: input.smartInclude ?? null,
      smart_exclude: input.smartExclude ?? null,
    },
  });
}

export function removeRule(id: string, setId?: string | null) {
  return invoke<void>("remove_rule", { id, setId: setId ?? null });
}

export function setRuleEnabled(id: string, enabled: boolean, setId?: string | null) {
  return invoke<Rule>("set_rule_enabled", {
    id,
    enabled,
    setId: setId ?? null,
  });
}

export function listConnections() {
  return invoke<ConnectionView[]>("list_connections");
}

export function listRequests(query?: string | null, limit?: number | null) {
  return invoke<ConnectionView[]>("list_requests", {
    query: query ?? null,
    limit: limit ?? null,
  });
}

/** Suspicious closed requests: short-lived & near-zero bytes (failure/timeout). */
export function listRequestFailures(query?: string | null, limit?: number | null) {
  return invoke<ConnectionView[]>("list_request_failures", {
    query: query ?? null,
    limit: limit ?? null,
  });
}

export function clearRequestHistory() {
  return invoke<void>("clear_request_history");
}

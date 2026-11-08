import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppSettings, ConnectionView, ProxyNode, ProxyStatus, SubscriptionView } from "../../src/types";
import "../../src/App.css";

// 所有数据仅存内存；先安装 IPC mock，再加载真实页面及其 API/Provider。
const params = new URLSearchParams(location.search);
const locale = params.get("locale") === "en" ? "en" : "zh";
const theme = params.get("theme") === "day" ? "day" : "aerospace";
const scenario = params.get("state") ?? "running";
const custom = scenario === "custom";
const nodeName = "香港 Hong Kong · IPv6 Premium Edge · LongNodeName-0123456789";
const profileName = "演示订阅 Subscription-Long-Name-0123456789-ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const ipv6 = "2001:db8:abcd:1234:5678:90ab:cdef:1234";
document.documentElement.lang = locale === "zh" ? "zh-CN" : "en";
localStorage.setItem("satelite.theme", theme);
localStorage.setItem("satelite.heroStyle", "classic");

const settings: AppSettings = {
  mixed_port: 2080, api_port: 19090, current_node_id: "fixture-node",
  probe_url: "https://example.test/204", locale, theme, hero_style: "classic",
  accent: "green", glow_color: "accent", glass_frost: false, core_type: "singbox",
  outbound_mode: "rule", auto_select: "off", capture_mode: "off",
  runtime_source: custom ? "singbox:fixture-sub" : "generated",
};
const nodes: ProxyNode[] = [{
  id: "fixture-node", name: nodeName, protocol: "vless", server: "2001:db8::10", port: 443,
  latency_ms: 42, latency_method: "singbox_http", subscription_id: "fixture-sub", subscription_name: profileName,
}];
const subscriptions: SubscriptionView[] = [{
  id: "fixture-sub", name: profileName, enabled: true, source_kind: custom ? "custom" : "url",
  custom_kind: custom ? "singbox" : null, source_display: "https://example.test/sub",
  node_count: 1, skipped_count: 0, last_update: 1_700_000_000,
  traffic: { upload: 2 ** 30, download: 5 * 2 ** 30, total: 100 * 2 ** 30 },
}];
const bucket = (up: number, down: number, upload: number, download: number) => ({
  upload_speed: up, download_speed: down, upload_total: upload, download_total: download,
});
const running = scenario === "running" || custom;
const status: ProxyStatus = {
  running, core_state: running ? "running" : scenario === "starting" ? "starting" : scenario === "error" ? "error" : "stopped",
  system_proxy: false, tun_enabled: false, capture_mode: "off", outbound_mode: "rule",
  mixed_port: 2080, api_port: 19090, core_type: "singbox", current_node_id: nodes[0].id,
  runtime_source: custom ? "singbox" : "generated", runtime_profile_id: custom ? "fixture-sub" : null,
  runtime_profile_name: custom ? profileName : null, custom_inbound_port: custom ? 2080 : null,
  error: scenario === "error" ? "模拟内核启动失败" : null,
  core_started_at: Math.floor(Date.now() / 1000) - 3661, core_memory_bytes: 48 * 2 ** 20,
  auto_select: "off", connections: running ? 2 : 0,
  ...bucket(running ? 3072 : 0, running ? 12288 : 0, 49152, 98304),
  traffic_breakdown: {
    source: custom ? "unavailable" : "connections",
    proxy: bucket(running ? 2048 : 0, running ? 8192 : 0, 32768, 65536),
    direct: bucket(running ? 1024 : 0, running ? 4096 : 0, 16384, 32768),
    unclassified: bucket(0, 0, 0, 0),
  },
};
const connections: ConnectionView[] = ["node-fixture", "direct"].map((tag, i) => ({
  id: `connection-${i}`, destination: "[2001:db8::80]:443", host: "example.test", network: "tcp",
  conn_type: "Mixed", node_tag: tag, node_name: i ? "direct" : nodeName, chains: [tag],
  chains_display: tag, rule: "fixture", rule_payload: "", process: i ? "browser.exe" : "LongApplicationName-0123456789.exe",
  source: "192.0.2.20:50000", upload: 1024, download: 4096, start: "2026-01-01T00:00:00Z", closed: false,
}));
const state = {
  settings, status, nodes, subscriptions, ipv6, commands: [] as { command: string; payload: unknown }[],
  unexpected: [] as string[], statusReads: 0, connectionReads: 0, trafficOpens: 0,
  holdStart: false, releaseStart: null as null | (() => void),
};
(window as unknown as { dashboardFixture: typeof state }).dashboardFixture = state;
const copy = <T,>(value: T): T => structuredClone(value);
mockIPC((command, payload) => {
  state.commands.push({ command, payload: copy(payload) });
  const args = (payload ?? {}) as Record<string, unknown>;
  switch (command) {
    case "get_settings": return copy(settings);
    case "get_proxy_status": state.statusReads++; return copy(status);
    case "list_all_nodes": return copy(nodes);
    case "list_subscriptions": return copy(subscriptions);
    case "get_subscription": return { ...copy(subscriptions[0]), content: "{}" };
    case "get_core_info": return {
      kind: args.kind, name: args.kind === "singbox" ? "sing-box" : args.kind === "xray" ? "Xray" : "mihomo",
      installed: true, version: "1.13.18", source: "bundled", platform: "windows-amd64", update_available: false,
    };
    case "get_lan_ip": return ipv6;
    case "check_exit_ip": return { ip: ipv6, countryCode: "HK", viaProxy: true, source: "fixture" };
    case "list_connection_changes":
      state.connectionReads++;
      return { rows: copy(connections), removed_ids: [], order_ids: connections.map(row => row.id),
        revision: state.connectionReads, order_revision: 1, unchanged: false, full: true };
    case "test_nodes_latency": return {
      results: [{ id: nodes[0].id, name: nodeName, latency_ms: 42, tested_at: 1_700_000_000, method: "singbox_http" }],
      tested: 1, ok: 1, failed: 0,
    };
    case "set_outbound_mode":
      if (!["rule", "global", "direct"].includes(String(args.mode))) throw new Error("非法模式参数");
      settings.outbound_mode = status.outbound_mode = args.mode as "rule" | "global" | "direct";
      return copy(status);
    case "set_capture_mode":
      if (!["off", "system", "tun"].includes(String(args.mode))) throw new Error("非法接管参数");
      settings.capture_mode = status.capture_mode = args.mode as "off" | "system" | "tun";
      status.system_proxy = args.mode === "system";
      settings.tun_enabled = status.tun_enabled = args.mode === "tun";
      return copy(status);
    case "update_settings":
      if (args.autoSelect !== "off" && args.autoSelect !== "kernel") throw new Error("夹具只允许手动/自动切换");
      settings.auto_select = status.auto_select = args.autoSelect;
      return copy(settings);
    case "start_proxy": {
      const finish = () => {
        status.running = true; status.core_state = "running"; status.error = null;
        state.releaseStart = null;
        return copy(status);
      };
      if (!state.holdStart) return finish();
      status.core_state = "starting";
      return new Promise(resolve => { state.releaseStart = () => resolve(finish()); });
    }
    case "stop_proxy":
      status.running = false; status.core_state = "stopped"; status.connections = 0;
      status.upload_speed = status.download_speed = 0;
      for (const row of Object.values(status.traffic_breakdown!).filter(value => typeof value === "object")) {
        row.upload_speed = row.download_speed = 0;
      }
      return copy(status);
    case "plugin:app|version": return "1.0.46";
    default:
      state.unexpected.push(command);
      throw new Error(`概览夹具禁止未声明 IPC：${command}`);
  }
}, { shouldMockEvents: true });

const { DashboardPage } = await import("../../src/pages/DashboardPage");
const { LocaleProvider } = await import("../../src/i18n");
const { ThemeProvider } = await import("../../src/theme");
createRoot(document.getElementById("root")!).render(
  <ThemeProvider><LocaleProvider>
    <div className="app-shell" style={{ flexDirection: "row", height: "100vh" }}>
      <aside aria-hidden="true" style={{ width: 220, flexShrink: 0, borderRight: "1px solid var(--border)", padding: "28px 20px", boxSizing: "border-box" }}>
        <strong>SATELITE</strong>
      </aside>
      <main className="main"><DashboardPage onGoTraffic={() => { state.trafficOpens++; }} /></main>
    </div>
  </LocaleProvider></ThemeProvider>,
);

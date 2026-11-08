import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AppSettings, ConnectionView, ProxyStatus } from "../../src/types";
// 与 App.tsx 一致：组件基础样式先于 App.css / 专业外观覆盖加载。
import "../../src/components/SideNav.css";
import "../../src/App.css";
import "../../src/ProAppearance.css";

// 不引入夹具布局 CSS；真实页面、侧栏及 Provider 使用生产样式。
const params = new URLSearchParams(location.search);
const locale = params.get("locale") === "en" ? "en" : "zh";
const theme = locale === "zh" ? "day" : "aerospace";
const zoom = params.get("zoom") === "1.5" ? 1.5 : 1;
document.documentElement.lang = locale === "zh" ? "zh-CN" : "en";
document.documentElement.style.zoom = String(zoom);
localStorage.setItem("satelite.theme", theme);
localStorage.setItem("satelite.uiMode", "pro");
const settings: AppSettings = {
  mixed_port: 2080, api_port: 19090, probe_url: "https://example.test/204",
  locale, theme, accent: "green", glow_color: "accent", glass_frost: false,
  hero_style: "classic", core_type: "singbox", capture_mode: "off",
  outbound_mode: "rule", auto_select: "off", runtime_source: "generated",
};
const rows: ConnectionView[] = [
  "api.long-destination-name.example.test:443",
  "[2001:db8:abcd:1234:5678:90ab:cdef:1234]:443",
  "192.0.2.80:8443",
  "cdn.example.test:443",
].map((destination, index) => ({
  id: `traffic-table-${index}`, destination, host: "example.test", network: "tcp",
  conn_type: "Mixed", node_tag: index === 2 ? "direct" : "node-fixture",
  node_name: index === 2 ? "direct" : "香港 Hong Kong · Long Premium Node Name 0123456789",
  subscription_name: "合成订阅 Synthetic Subscription 0123456789",
  chains: [index === 2 ? "direct" : "node-fixture"], chains_display: "fixture",
  rule: "DOMAIN-SUFFIX, long-rule-payload.example.test", rule_payload: "example.test",
  process: "LongApplicationProcessName-0123456789.exe", source: "192.0.2.20:50000",
  upload: 12345678, download: 987654321, start: "2026-01-01T00:00:00Z",
  first_seen: 1767225600000, last_seen: 1767225601000,
}));
const status: ProxyStatus = {
  running: true, core_state: "running", system_proxy: false, tun_enabled: false,
  outbound_mode: "rule", mixed_port: 2080, api_port: 19090, core_type: "singbox",
  upload_speed: 1024, download_speed: 4096, upload_total: 12345678,
  download_total: 987654321, connections: rows.length, runtime_source: "generated",
};
const fixture = {
  locale, zoom, rows, unexpected: [] as string[],
  commands: [] as string[], reads: { live: 0, history: 0, failures: 0 },
};
(window as unknown as { trafficTableFixture: typeof fixture }).trafficTableFixture = fixture;
const copy = <T,>(value: T): T => structuredClone(value);
// 先 mock，再动态加载任何包含真实 IPC 的页面/Provider。
mockIPC((command, payload) => {
  fixture.commands.push(command);
  const args = (payload ?? {}) as Record<string, unknown>;
  switch (command) {
    case "get_settings": return copy(settings);
    case "get_proxy_status": return copy(status);
    case "list_connection_changes":
      fixture.reads.live++;
      return { rows: copy(rows), removed_ids: [], order_ids: rows.map(row => row.id),
        revision: fixture.reads.live, order_revision: 1, unchanged: false, full: true };
    case "list_requests":
    case "list_request_failures": {
      const mode = command === "list_requests" ? "history" : "failures";
      fixture.reads[mode]++;
      return { cursor: rows.length, entries: args.afterSeq == null ? rows.map(row => ({
        ...copy(row), closed: true, closed_at: 1767225601000,
        ...(mode === "failures" ? { upload: 0, download: 0 } : {}),
      })) : [] };
    }
    // 真实 UiModeProvider 的窗口同步仅在内存响应。
    case "set_ui_mode_pref":
    case "plugin:window|set_min_size":
    case "plugin:window|set_max_size":
    case "plugin:window|set_resizable":
    case "plugin:window|set_size": return;
    case "plugin:window|current_monitor": return null;
    default:
      fixture.unexpected.push(command);
      throw new Error(`流量表格夹具禁止未声明 IPC：${command}`);
  }
}, { shouldMockEvents: true });
mockWindows("main");

const { TrafficPage } = await import("../../src/pages/TrafficPage");
const { SideNav } = await import("../../src/components/SideNav");
const { LocaleProvider } = await import("../../src/i18n");
const { ThemeProvider } = await import("../../src/theme");
const { UiModeProvider } = await import("../../src/ui/UiModeContext");
createRoot(document.getElementById("root")!).render(
  <ThemeProvider><LocaleProvider><UiModeProvider>
    <div className="app-shell pro-shell">
      <SideNav active="traffic" onChange={() => {}} />
      <main className="main">
        <div className="page-enter"><TrafficPage /></div>
      </main>
    </div>
  </UiModeProvider></LocaleProvider></ThemeProvider>,
);

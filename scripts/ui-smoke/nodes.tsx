import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { AppSettings, ProxyNode } from "../../src/types";
// 与 App.tsx 一致的样式顺序，避免夹具漏掉专业模式的实际覆盖。
import "../../src/components/SideNav.css";
import "../../src/App.css";
import "../../src/ProAppearance.css";
import "../../src/ProMotion.css";

// 隔离的真实页面夹具：不读用户配置、不启动内核、不调用真实删除。
const params = new URLSearchParams(location.search);
const grid = params.get("view") === "grid";
const count = grid ? (params.get("large") === "true" ? 603 : 9) : 3;
const locale = params.get("locale") === "en" ? "en" : "zh";
const theme = params.get("theme") === "aerospace" ? "aerospace" : "day";
const zoom = params.get("zoom") === "1.5" ? 1.5 : 1;
document.documentElement.style.transition = "none";
document.documentElement.style.zoom = String(zoom);
document.documentElement.dataset.motionReduced = "true";
localStorage.setItem("nodes.viewMode", grid ? "grid" : "list");
localStorage.setItem("nodes.groupBy.v2", "default");
localStorage.setItem("nodes.sortMode", "default");
localStorage.setItem("satelite.theme", theme);
localStorage.setItem("satelite.accent", "pink");
localStorage.setItem("satelite.uiMode", "pro");
const nodes: ProxyNode[] = Array.from({ length: count }, (_, i) => ({
  id: ["a", "b", "c"][i] ?? `node-${i}`,
  name: grid ? `香港 Hong Kong · 超长节点名称 Premium 0123456789 · ${i}` : `Node ${["a", "b", "c"][i]}`,
  protocol: grid ? ["shadowsocks", "hysteria2", "wireguard"][i % 3] : "hysteria2",
  server: "example.test", port: 443,
  subscription_id: "fixture", subscription_name: "测试配置 Synthetic Subscription 0123456789",
}));
const settings: AppSettings = {
  mixed_port: 2080, api_port: 19090, probe_url: "https://example.test/204",
  locale, theme, accent: "pink", glass_frost: false, core_type: "singbox",
  runtime_source: "generated", auto_select: "off", current_node_id: null,
  multi_core_enabled: grid, protocol_cores: [{ protocol: "shadowsocks", core: "mihomo" }],
};
const state = {
  nodes, settings, confirms: 0, deleted: [] as string[][], answer: "Cancel", hold: false,
  release: null as null | ((answer: string) => void),
  selected: [] as string[], reordered: [] as string[][], tested: [] as string[][],
  unexpected: [] as string[],
};
(window as unknown as { nodeFixture: typeof state }).nodeFixture = state;
mockIPC((command, payload) => {
  const args = (payload ?? {}) as Record<string, unknown>;
  if (command === "get_settings") return structuredClone(settings);
  if (command === "get_proxy_status") return { running: true, core_type: "singbox", runtime_source: "generated", outbound_mode: "rule" };
  if (command === "list_all_nodes") return structuredClone(state.nodes);
  if (command === "list_node_ids") return state.nodes.map(node => node.id);
  if (command === "plugin:dialog|message") {
    state.confirms++;
    return state.hold ? new Promise<string>(resolve => { state.release = resolve; }) : state.answer;
  }
  if (command === "delete_nodes") {
    const ids = args.ids as string[];
    state.deleted.push(ids);
    state.nodes = state.nodes.filter(node => !ids.includes(node.id));
    return ids.length;
  }
  if (command === "set_current_node") {
    const id = args.nodeId as string;
    state.selected.push(id);
    settings.current_node_id = id;
    return structuredClone(settings);
  }
  if (command === "reorder_nodes") {
    const ids = args.ids as string[];
    state.reordered.push(ids);
    const byId = new Map(state.nodes.map(node => [node.id, node]));
    state.nodes = ids.map(id => byId.get(id)!);
    return;
  }
  if (command === "test_nodes_latency") {
    const ids = args.ids as string[];
    state.tested.push(ids);
    const tested_at = Date.now();
    state.nodes = state.nodes.map(node => ids.includes(node.id)
      ? { ...node, latency_ms: 42, latency_at: tested_at, latency_method: "clash_api" } : node);
    return { results: ids.map(id => ({ id, name: id, latency_ms: 42, tested_at, method: "clash_api" })),
      tested: ids.length, ok: ids.length, failed: 0, method: "clash_api" };
  }
  // 真实 UiModeProvider 的窗口同步仅在内存响应。
  if (["set_ui_mode_pref", "plugin:window|set_min_size", "plugin:window|set_max_size",
    "plugin:window|set_resizable", "plugin:window|set_size"].includes(command)) return;
  if (command === "plugin:window|current_monitor") return null;
  state.unexpected.push(command);
  throw new Error(`夹具未实现命令：${command}`);
}, { shouldMockEvents: true });
mockWindows("main");
const { NodesPage } = await import("../../src/pages/NodesPage");
const { SideNav } = await import("../../src/components/SideNav");
const { LocaleProvider } = await import("../../src/i18n");
const { ThemeProvider } = await import("../../src/theme");
const { UiModeProvider } = await import("../../src/ui/UiModeContext");
createRoot(document.getElementById("root")!).render(
  <ThemeProvider><LocaleProvider><UiModeProvider>
    <div className="app-shell pro-shell">
      <SideNav active="nodes" onChange={() => {}} />
      <main className="main"><div className="page-enter"><NodesPage /></div></main>
    </div>
  </UiModeProvider></LocaleProvider></ThemeProvider>,
);

import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { NodesPage } from "../../src/pages/NodesPage";
import { SimpleServersPage } from "../../src/ui/simple/SimpleServersPage";
import { ImportIntentProvider } from "../../src/ImportIntentContext";
import { refreshProxyStatus } from "../../src/api";
import { LocaleProvider } from "../../src/i18n";
import "../../src/App.css";

const nodes = ["a", "b", "c"].map(id => ({ id, name: `Node ${id}`, protocol: "vless", server: "example.test", port: 443,
  config: { protocol: "vless", uuid: "fixture" }, subscription_id: "fixture", subscription_name: "配置" }));
const labels = { "node-a": "Node a", "node-b": "Node b", "node-c": "Node c", proxy: "🚀 节点选择", auto: "📈 自动选择" };
const groups = [
  { name: "proxy", group_type: "Selector", now: "auto", all: ["auto", "node-a", "node-b", "DIRECT"], labels },
  { name: "auto", group_type: "URLTest", now: "node-a", all: ["node-a", "node-b"], labels },
  { name: "🤖 AI 域名", group_type: "Selector", now: "node-a", all: ["node-a", "node-b"], labels },
  { name: "🇨🇳 大陆域名及IP", group_type: "Selector", now: "DIRECT", all: ["node-a", "node-b", "DIRECT"], labels },
];
const state = { order: [] as string[], switches: [] as string[][], nodeSelections: 0, saves: 0, fetched: 0,
  mode: "rule", running: true, groups, failGroups: false, holdGroups: false, heldRequests: 0,
  releaseHeld: () => {}, pushStatus: refreshProxyStatus,
  template: { url: "https://example.test/rules.ini", ini: "[custom]\ncustom_proxy_group=A`select`.*\nruleset=A,[]FINAL", override_yaml: "dns:\n  ipv6: false" } };
(window as unknown as { mihomoFixture: typeof state }).mihomoFixture = state;
const status = () => ({ running: state.running, core_type: "mihomo", outbound_mode: state.mode, auto_select: "off" });
const view = () => ({ template: state.template, use_for_active: true, group_order: state.order });
mockIPC((command, payload) => {
  if (command === "get_settings") return { core_type: "mihomo", runtime_source: "generated", current_node_id: "c", locale: "zh", auto_select: "off" };
  if (command === "get_proxy_status") return status();
  if (command === "list_all_nodes") return nodes;
  if (command === "list_nodes_page") return { nodes, total: nodes.length };
  if (command === "peek_pending_import_urls") return [];
  if (command === "list_node_ids") return nodes.map(n => n.id);
  if (command === "get_mihomo_template") return view();
  if (command === "reorder_mihomo_groups") { state.order = (payload as { ids: string[] }).ids; return state.order; }
  if (command === "list_mihomo_proxy_groups") {
    if (state.failGroups) throw new Error("fixture group read failed");
    const result = state.mode === "global" ? [{ ...groups[0], name: "GLOBAL", now: "node-b" }] : state.mode === "direct" ? [] : groups;
    if (state.holdGroups) {
      state.heldRequests++;
      return new Promise(resolve => { state.releaseHeld = () => { state.holdGroups = false; resolve(result); }; });
    }
    return result;
  }
  if (command === "select_mihomo_proxy_group") {
    const { group, member } = payload as { group: string; member: string };
    state.switches.push([group, member]); groups.find(g => g.name === group)!.now = member; return groups;
  }
  if (command === "set_outbound_mode") { state.mode = (payload as { mode: string }).mode; return status(); }
  if (command === "set_current_node") { state.nodeSelections++; return {}; }
  if (command === "fetch_mihomo_template") { state.fetched++; return "[custom]\ncustom_proxy_group=B`select`.*\nruleset=B,[]FINAL"; }
  if (command === "save_mihomo_template") { state.saves++; state.template = (payload as { template: typeof state.template }).template; return view(); }
  throw new Error(`策略组夹具未实现：${command}`);
}, { shouldMockEvents: true });
createRoot(document.getElementById("root")!).render(<LocaleProvider><ImportIntentProvider><main className="main">{new URLSearchParams(location.search).has("simple") ? <SimpleServersPage /> : <NodesPage />}</main></ImportIntentProvider></LocaleProvider>);

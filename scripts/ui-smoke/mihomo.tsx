import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { NodesPage } from "../../src/pages/NodesPage";
import { LocaleProvider } from "../../src/i18n";
import "../../src/App.css";

const nodes = ["a", "b"].map(id => ({ id, name: `Node ${id}`, protocol: "vless", server: "example.test", port: 443,
  config: { protocol: "vless", uuid: "fixture" }, subscription_id: "fixture", subscription_name: "配置" }));
const labels = { a: "Node a", b: "Node b", proxy: "🚀 节点选择" };
const groups = ["proxy", "🤖 AI 域名", "🇨🇳 大陆域名及IP"].map(name => ({ name, group_type: "Selector", now: "a", all: ["a", "b"], labels }));
const state = { order: [] as string[], switches: [] as string[][], nodeSelections: 0, saves: 0, fetched: 0,
  mode: "rule", template: { url: "https://example.test/rules.ini", ini: "[custom]\ncustom_proxy_group=A`select`.*\nruleset=A,[]FINAL", override_yaml: "dns:\n  ipv6: false" } };
(window as unknown as { mihomoFixture: typeof state }).mihomoFixture = state;
const status = () => ({ running: true, core_type: "mihomo", outbound_mode: state.mode, auto_select: "off" });
const view = () => ({ template: state.template, use_for_active: true, group_order: state.order });
mockIPC((command, payload) => {
  if (command === "get_settings") return { core_type: "mihomo", runtime_source: "generated", locale: "zh", auto_select: "off" };
  if (command === "get_proxy_status") return status();
  if (command === "list_all_nodes") return nodes;
  if (command === "list_node_ids") return nodes.map(n => n.id);
  if (command === "get_mihomo_template") return view();
  if (command === "reorder_mihomo_groups") { state.order = (payload as { ids: string[] }).ids; return state.order; }
  if (command === "list_mihomo_proxy_groups") return state.mode === "global" ? [{ ...groups[0], name: "GLOBAL" }] : state.mode === "direct" ? [] : groups;
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
createRoot(document.getElementById("root")!).render(<LocaleProvider><main className="main"><NodesPage /></main></LocaleProvider>);

import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { NodesPage } from "../../src/pages/NodesPage";
import { LocaleProvider } from "../../src/i18n";
import "../../src/App.css";

// 隔离的真实页面夹具：不读用户配置、不启动内核、不调用真实删除。
const nodes = ["a", "b", "c"].map(id => ({ id, name: `Node ${id}`, protocol: "hysteria2",
  server: "example.test", port: 443, config: { protocol: "hysteria2", password: "fixture" },
  subscription_id: "fixture", subscription_name: "测试配置" }));
const state = { nodes, confirms: 0, deleted: [] as string[][], answer: "Cancel", hold: false,
  release: null as null | ((answer: string) => void) };
(window as unknown as { nodeFixture: typeof state }).nodeFixture = state;
localStorage.setItem("nodes.viewMode", "list");
mockIPC((command, payload) => {
  if (command === "get_settings") return { locale: "zh", runtime_source: "generated", core_type: "singbox" };
  if (command === "list_all_nodes") return state.nodes;
  if (command === "list_node_ids") return state.nodes.map(node => node.id);
  if (command === "plugin:dialog|message") {
    state.confirms++;
    return state.hold ? new Promise<string>(resolve => { state.release = resolve; }) : state.answer;
  }
  if (command === "delete_nodes") {
    const ids = (payload as { ids: string[] }).ids;
    state.deleted.push(ids);
    state.nodes = state.nodes.filter(node => !ids.includes(node.id));
    return ids.length;
  }
  throw new Error(`夹具未实现命令：${command}`);
}, { shouldMockEvents: true });
createRoot(document.getElementById("root")!).render(
  <LocaleProvider><main className="main"><NodesPage /></main></LocaleProvider>,
);

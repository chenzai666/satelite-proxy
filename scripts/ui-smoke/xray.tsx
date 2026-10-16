import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { NodesPage } from "../../src/pages/NodesPage";
import { SimpleServersPage } from "../../src/ui/simple/SimpleServersPage";
import { ImportIntentProvider } from "../../src/ImportIntentContext";
import { LocaleProvider } from "../../src/i18n";
import "../../src/App.css";

const nodes = ["a", "b"].map(id => ({ id, name: `Node ${id}`, protocol: "vless", server: "example.test", port: 443,
  config: { protocol: "vless", uuid: "fixture" }, subscription_id: "fixture", subscription_name: "配置" }));
const state = { runs: [] as string[][], switches: 0, running: new URLSearchParams(location.search).has("running") };
(window as unknown as { xrayFixture: typeof state }).xrayFixture = state;
localStorage.setItem("nodes.viewMode", "list");
mockIPC((command, payload) => {
  if (command === "get_settings") return { core_type: "xray", runtime_source: "generated", current_node_id: "a", locale: "zh", auto_select: "off" };
  if (command === "get_proxy_status") return { running: state.running, core_type: "xray", outbound_mode: "rule" };
  if (command === "list_all_nodes") return structuredClone(nodes);
  if (command === "list_nodes_page") return { nodes: structuredClone(nodes), total: nodes.length };
  if (command === "list_node_ids") return nodes.map(n => n.id);
  if (command === "peek_pending_import_urls") return [];
  if (command === "test_nodes_latency") {
    const ids = (payload as { ids?: string[] }).ids ?? nodes.map(n => n.id);
    state.runs.push(ids);
    const results = ids.map(id => ({ id, name: `Node ${id}`, latency_ms: 123, tested_at: Date.now() / 1000, method: "xray_http" }));
    for (const result of results) {
      Object.assign(nodes.find(node => node.id === result.id)!, { latency_ms: result.latency_ms, latency_at: result.tested_at, latency_method: result.method });
      (payload as { onResult: { onmessage: (value: typeof result) => void } }).onResult.onmessage(result);
    }
    return { results, tested: results.length, ok: results.length, failed: 0, method: "xray_http" };
  }
  if (["set_current_node", "start_proxy", "stop_proxy", "restart_proxy", "set_system_proxy"].includes(command)) { state.switches++; throw new Error("测速不能切换主代理"); }
  throw new Error(`Xray 夹具未实现：${command}`);
}, { shouldMockEvents: true });
createRoot(document.getElementById("root")!).render(<LocaleProvider><ImportIntentProvider><main className="main">{new URLSearchParams(location.search).has("simple") ? <SimpleServersPage /> : <NodesPage />}</main></ImportIntentProvider></LocaleProvider>);

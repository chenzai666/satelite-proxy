import React from "react";
import { createRoot } from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { NodesPage } from "../../src/pages/NodesPage";
import { SimpleServersPage } from "../../src/ui/simple/SimpleServersPage";
import { ImportIntentProvider } from "../../src/ImportIntentContext";
import { LocaleProvider } from "../../src/i18n";
import type { LatencyResult, NodeLatencyChange, ProxyNode } from "../../src/types";
import { createLatencyFixtureClock } from "./latencyFixtureClock";
import "../../src/App.css";

const params = new URLSearchParams(location.search);
const storageKey = `xray-latency-fixture:${location.search}`;
const nodes: ProxyNode[] = JSON.parse(sessionStorage.getItem(storageKey) ?? "null") ?? ["a", "b"].map(id => ({ id, name: `Node ${id}`, protocol: "vless", server: "example.test", port: 443,
  config: { protocol: "vless", uuid: "fixture" }, subscription_id: "fixture", subscription_name: "配置" }));
const save = () => sessionStorage.setItem(storageKey, JSON.stringify(nodes));
const clock = createLatencyFixtureClock(nodes);
// 模拟后端持久化；对事件可显式跳过写入，以单独检查前端缓存保护。
function store(change: NodeLatencyChange) {
  const node = nodes.find(node => node.id === change.id);
  if (!node) return;
  if (change.method === "tcp" && (node.latency_method === "clash_api" || node.latency_method?.endsWith("_http"))) return;
  if (node.latency_at != null && change.latency_at != null && change.latency_at < node.latency_at) return;
  Object.assign(node, { latency_ms: change.latency_ms, latency_at: change.latency_at,
    latency_method: change.method, latency_error: change.latency_error ?? null });
  save();
}
const state = {
  runs: [] as string[][], switches: 0, running: params.has("running"),
  method: "xray_http", nextError: null as string | null,
  holdCompletion: false, finish: null as null | (() => void),
  async emitChange(change: NodeLatencyChange, persist = true) {
    clock.observe(change.latency_at);
    if (persist) store(change);
    await emit("node-latency-changed", [change]);
  },
};
(window as unknown as { xrayFixture: typeof state }).xrayFixture = state;
localStorage.setItem("nodes.viewMode", params.has("grid") ? "grid" : "list");
localStorage.setItem("nodes.groupBy.v2", params.has("grouped") ? "sub" : "default");
localStorage.setItem("nodes.favoritesOnly", "0");
mockIPC((command, payload) => {
  if (command === "get_settings") return { core_type: "xray", runtime_source: "generated", current_node_id: "a", locale: "zh", auto_select: "off" };
  if (command === "get_proxy_status") return { running: state.running, core_type: "xray", outbound_mode: "rule" };
  if (command === "list_all_nodes") return structuredClone(nodes);
  if (command === "list_nodes_page") return { nodes: structuredClone(nodes), total: nodes.length };
  if (command === "list_node_ids") return nodes.map(n => n.id);
  if (command === "peek_pending_import_urls") return [];
  if (command === "test_nodes_latency" || command === "ping_nodes_latency") {
    const ids = (payload as { ids?: string[] }).ids ?? nodes.map(n => n.id);
    state.runs.push(ids);
    const method = command === "ping_nodes_latency" ? "tcp" : state.method;
    const results: LatencyResult[] = ids.map(id => ({ id, name: `Node ${id}`,
      latency_ms: state.nextError ? null : 123, error: state.nextError,
      tested_at: clock.next(), method }));
    for (const result of results) {
      store({ id: result.id, name: result.name, latency_ms: result.latency_ms ?? null,
        latency_at: result.tested_at, method, latency_error: result.error });
      (payload as { onResult: { onmessage: (value: typeof result) => void } }).onResult.onmessage(result);
    }
    const ok = results.filter(result => result.latency_ms != null).length;
    const batch = { results, tested: results.length, ok, failed: results.length - ok, method };
    if (state.holdCompletion) return new Promise(resolve => {
      state.finish = () => { state.finish = null; resolve(batch); };
    });
    return batch;
  }
  if (["set_current_node", "start_proxy", "stop_proxy", "restart_proxy", "set_system_proxy"].includes(command)) { state.switches++; throw new Error("测速不能切换主代理"); }
  throw new Error(`Xray 夹具未实现：${command}`);
}, { shouldMockEvents: true });
createRoot(document.getElementById("root")!).render(<LocaleProvider><ImportIntentProvider><main className="main">{new URLSearchParams(location.search).has("simple") ? <SimpleServersPage /> : <NodesPage />}</main></ImportIntentProvider></LocaleProvider>);

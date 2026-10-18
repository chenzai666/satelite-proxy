import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import {
  latencyState, isRealLatencyMethod, mergeNodeLatency,
  applyLatencyResult, applyNodeLatencyChange,
} from "../src/nodeLatencyState.ts";
import { applyCustomLatency } from "../src/customNodes.ts";
import { createLatencyResultBuffer } from "../src/latencyStream.ts";
import { createLatencyFixtureClock } from "./ui-smoke/latencyFixtureClock.ts";

const methods = ["xray_http", "singbox_http", "mihomo_http", "clash_api", "future_http"];
const failures = ["timeout", "unsupported", "config", "tls", "connection", "http", "core", "missing_core"];
const legacyNoCore = "core not running: start the proxy to test this protocol";
const legacyQuicPing = "QUIC-only protocol: no TCP port to ping — use the real-latency test with the core running";
const result = (method, ms, error = null, at = 20) => ({
  id: "a", name: "节点", method, latency_ms: ms, error, tested_at: at,
});
const change = (method, ms, error = null, at = 20) => ({
  id: "a", name: "节点", method, latency_ms: ms, latency_error: error, latency_at: at,
});

test("浏览器夹具分组重测越过未来成功、被拒 TCP 事件和重载后的缓存时间", () => {
  const nodes = [{ latency_at: 1600 }, { latency_at: 1600 }];
  const clock = createLatencyFixtureClock(nodes, () => 1000);
  clock.observe(1500);
  clock.observe(1600);
  clock.observe(1700); // +700 TCP 虽被 UI 拒绝，下一轮仍越过它。
  const group = nodes.map(() => clock.next());
  assert.deepEqual(group, [1701, 1702]);
  nodes[0].latency_at = 1900;
  assert.equal(clock.next(), 1901);
  const restoredClock = createLatencyFixtureClock(nodes, () => 1000);
  assert.equal(restoredClock.next(), 1901);
});

test("八类安全前缀分别显示状态及完整安全详情，未知错误不泄漏", () => {
  for (const method of methods) for (const kind of failures) {
    const error = `${kind}: 后端安全详情`;
    const node = applyLatencyResult({}, result(method, null, error));
    assert.deepEqual(latencyState(node), { kind, detail: error });
    assert.deepEqual(latencyState(JSON.parse(JSON.stringify(node))), { kind, detail: error });
  }
  assert.deepEqual(latencyState({ latency_error: "raw unsafe credentials" }), { kind: "failed" });
});

test("成功、未测和旧后端无错误兼容，失败不会统一误报超时", () => {
  assert.deepEqual(latencyState({}), { kind: "untested" });
  assert.deepEqual(latencyState({ latency_ms: null, latency_at: 0 }), { kind: "failed" });
  assert.deepEqual(latencyState({ latency_method: "unsupported" }), { kind: "unsupported" });
  assert.deepEqual(latencyState({ latency_ms: null, latency_at: 20, latency_method: "tcp", latency_error: "timeout" }), { kind: "timeout" });
  for (const error of [
    "QUIC-only protocol: no TCP port to ping — use the real-latency test with the core running",
    "core not running: start the proxy to test this protocol",
  ]) assert.deepEqual(latencyState({ latency_method: "unsupported", latency_error: error }), { kind: "unsupported" });
  for (const ms of [0, 123]) {
    assert.deepEqual(latencyState({ latency_ms: ms, latency_error: "tls: 旧错误" }), { kind: "success", ms });
  }
  for (const ms of [-1, NaN, Infinity]) {
    assert.deepEqual(latencyState({ latency_ms: ms, latency_at: 1 }), { kind: "failed" });
  }
});

test("TCP 成功/失败均不能覆盖所有 *_http 和 clash_api 的成功/失败", () => {
  for (const method of methods) for (const ms of [null, 123]) {
    const node = applyLatencyResult({}, result(method, ms, ms == null ? "tls: 证书失败" : null));
    assert.equal(isRealLatencyMethod(method), true);
    for (const tcpMs of [null, 1]) {
      assert.strictEqual(applyLatencyResult(node, result("tcp", tcpMs, "timeout: TCP", 30)), node);
      assert.strictEqual(applyNodeLatencyChange(node, change("tcp", tcpMs, "timeout: TCP", 30)), node);
    }
  }
  assert.equal(isRealLatencyMethod("tcp"), false);
  assert.equal(isRealLatencyMethod(undefined), false);
  assert.equal(applyLatencyResult({}, result("tcp", 12)).latency_ms, 12);
});

test("legacy QUIC Ping 和停核错误保护所有真延迟成功/失败，节点参数不支持仍接受", () => {
  for (const method of methods) for (const ms of [null, 123]) {
    const node = applyLatencyResult({}, result(method, ms, ms == null ? "tls: 原安全错误" : null));
    for (const error of [legacyNoCore, legacyQuicPing]) {
      assert.strictEqual(applyLatencyResult(node, result("unsupported", null, error, 30)), node);
      assert.strictEqual(applyNodeLatencyChange(node, change("unsupported", null, error, 30)), node);
      // 未有真延迟时仍正常记录可达性测试结果。
      assert.equal(applyLatencyResult({}, result("unsupported", null, error)).latency_method, "unsupported");
    }
    for (const error of ["unsupported: 此节点参数不受支持", "unsupported: QUIC-only 协议没有可供 TCP Ping 的端口",
      "unsupported: 请先启动代理以测试此协议", `${legacyNoCore} extra`, null]) {
      const updated = applyLatencyResult(node, result("unsupported", null, error, 30));
      assert.notStrictEqual(updated, node);
      assert.equal(updated.latency_method, "unsupported");
      assert.equal(updated.latency_error, error);
      assert.deepEqual(applyNodeLatencyChange(node, change("unsupported", null, error, 30)), updated);
    }
    // 旧错误文本仅在 method=unsupported 时表示 reachability_only。
    assert.equal(applyLatencyResult(node, result("singbox_http", null, legacyNoCore, 30)).latency_at, 30);
  }
});

test("两种节点页重测开始只切换 spinner，完整旧结果留待新结果接受后替换", () => {
  for (const path of ["../src/pages/NodesPage.tsx", "../src/ui/simple/SimpleServersPage.tsx"]) {
    const page = readFileSync(new URL(path, import.meta.url), "utf8");
    const start = page.slice(page.indexOf("  async function onTestNodes("), page.indexOf("    const buffer = createLatencyResultBuffer"));
    assert.match(start, /setTestingIds\(idSet\)/);
    assert.doesNotMatch(start, /setNodes\(|clearLatencyForRetest|latency_(?:ms|at|error)\s*:/);
    assert.match(page, /applyLatencyResult\(n, r\)/);
    assert.match(page, /for \(const result of batch.results\) buffer.push\(result\)/);
  }
});

test("新成功清空错误，新失败替换旧分类；拒绝降级时旧结果完整保留", () => {
  const node = applyLatencyResult({}, result("xray_http", null, "config: 旧配置"));
  assert.strictEqual(applyLatencyResult(node, result("tcp", 1, null, 21)), node);
  assert.equal(applyLatencyResult(node, result("xray_http", 10, null, 21)).latency_error, null);
  assert.deepEqual(latencyState(applyLatencyResult(node, result("singbox_http", null, "core: 新错误", 21))),
    { kind: "core", detail: "core: 新错误" });
  assert.equal(applyNodeLatencyChange(node, change("xray_http", 1, null, 21)).latency_error, null);
});

test("stream 停核提示、batch QUIC Ping 及 TCP 回退均保留重测前完整成功/失败", () => {
  const oldWindow = globalThis.window;
  let frame;
  globalThis.window = {
    requestAnimationFrame(callback) { frame = callback; return 1; },
    cancelAnimationFrame() { frame = undefined; },
  };
  try {
    for (const method of methods) for (const ms of [123, null]) {
      const old = applyLatencyResult({}, result(method, ms, ms == null ? "tls: 旧失败" : null));
      let node = old; // 重测仅显示 spinner，不修改节点。
      const buffer = createLatencyResultBuffer(batch => {
        for (const value of batch.values()) node = applyLatencyResult(node, value);
      });
      buffer.push(result("unsupported", null, legacyNoCore, 30));
      frame();
      assert.strictEqual(node, old);
      // ping_nodes_streaming 在最终 batch 才将停核文本改写为 QUIC Ping 文本。
      buffer.push(result("unsupported", null, legacyQuicPing, 30));
      buffer.flushNow();
      assert.strictEqual(node, old);
      for (const tcpMs of [1, null]) {
        buffer.push(result("tcp", tcpMs, tcpMs == null ? "timeout: TCP" : null, 31));
        frame();
        assert.strictEqual(node, old);
        buffer.push(result("tcp", tcpMs, tcpMs == null ? "timeout: TCP" : null, 31));
        buffer.flushNow();
        assert.strictEqual(node, old);
        assert.deepEqual(latencyState(node), latencyState(old));
      }
      // 真正的不支持节点参数结果必须替换旧值，并在下一次成功时清错。
      buffer.push(result("unsupported", null, "unsupported: 此节点参数不受支持", 32));
      buffer.flushNow();
      assert.deepEqual(latencyState(node), { kind: "unsupported", detail: "unsupported: 此节点参数不受支持" });
      buffer.push(result("singbox_http", 42, null, 33));
      buffer.flushNow();
      assert.equal(node.latency_error, null);
      assert.deepEqual(latencyState(node), { kind: "success", ms: 42 });
    }
  } finally {
    if (oldWindow === undefined) delete globalThis.window;
    else globalThis.window = oldWindow;
  }
});

test("事件及 stream 拒绝迟到回包，同秒事件可以携带新错误", () => {
  const node = applyLatencyResult({}, result("mihomo_http", null, "http: 503"));
  assert.strictEqual(applyLatencyResult(node, result("mihomo_http", 1, null, 19)), node);
  assert.strictEqual(applyNodeLatencyChange(node, change("mihomo_http", 1, null, 19)), node);
  const next = applyNodeLatencyChange(node, change("mihomo_http", null, "tls: 握手失败"));
  assert.equal(next.latency_error, "tls: 握手失败");
  assert.equal(mergeNodeLatency(next, { latency_ms: 10 }).latency_error, null);
});

test("自写配置会话刷新保留错误和方法", () => {
  const [node] = applyCustomLatency([{ id: "a" }, { id: "b" }], new Map([
    ["a", { ms: null, at: 20, error: "connection: 拒绝连接", method: "tcp" }],
  ]));
  assert.equal(node.latency_error, "connection: 拒绝连接");
  assert.equal(node.latency_method, "tcp");
  assert.equal(latencyState(node).kind, "connection");
});

test("rAF 缓冲及最终 flush 携带错误，每个节点独立更新", () => {
  const oldWindow = globalThis.window;
  let frame;
  globalThis.window = {
    requestAnimationFrame(callback) { frame = callback; return 1; },
    cancelAnimationFrame() { frame = undefined; },
  };
  try {
    const nodes = new Map([["a", {}], ["b", {}]]);
    const buffer = createLatencyResultBuffer(batch => {
      for (const [id, value] of batch) nodes.set(id, applyLatencyResult(nodes.get(id), value));
    });
    buffer.push(result("singbox_http", null, "unsupported: 传输不支持"));
    buffer.push({ ...result("mihomo_http", null, "missing_core: 缺少内核"), id: "b" });
    frame();
    assert.equal(nodes.get("a").latency_error, "unsupported: 传输不支持");
    assert.equal(nodes.get("b").latency_error, "missing_core: 缺少内核");
    buffer.push(result("singbox_http", 42, null, 21));
    buffer.flushNow();
    assert.equal(nodes.get("a").latency_error, null);
    assert.equal(nodes.get("b").latency_error, "missing_core: 缺少内核");
  } finally {
    if (oldWindow === undefined) delete globalThis.window;
    else globalThis.window = oldWindow;
  }
});

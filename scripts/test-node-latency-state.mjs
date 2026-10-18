import assert from "node:assert/strict";
import test from "node:test";
import {
  latencyState, isRealLatencyMethod, mergeNodeLatency,
  applyLatencyResult, applyNodeLatencyChange, clearLatencyForRetest,
} from "../src/nodeLatencyState.ts";
import { applyCustomLatency } from "../src/customNodes.ts";
import { createLatencyResultBuffer } from "../src/latencyStream.ts";
import { createLatencyFixtureClock } from "./ui-smoke/latencyFixtureClock.ts";

const methods = ["xray_http", "singbox_http", "mihomo_http", "clash_api", "future_http"];
const failures = ["timeout", "unsupported", "config", "tls", "connection", "http", "core", "missing_core"];
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
    assert.strictEqual(clearLatencyForRetest(node, "ping"), node);
  }
  assert.equal(isRealLatencyMethod("tcp"), false);
  assert.equal(isRealLatencyMethod(undefined), false);
  assert.equal(applyLatencyResult({}, result("tcp", 12)).latency_ms, 12);
});

test("重测清理旧错误并保留保护元数据；成功清空错误，失败可替换旧分类", () => {
  const node = applyLatencyResult({}, result("xray_http", null, "config: 旧配置"));
  const cleared = clearLatencyForRetest(node);
  assert.equal(cleared.latency_error, null);
  assert.equal(cleared.latency_ms, undefined);
  assert.equal(cleared.latency_at, 20);
  assert.equal(cleared.latency_method, "xray_http");
  assert.equal(node.latency_error, "config: 旧配置");
  assert.strictEqual(applyLatencyResult(cleared, result("tcp", 1, null, 21)), cleared);
  assert.equal(applyLatencyResult(cleared, result("xray_http", 10, null, 21)).latency_error, null);
  assert.deepEqual(latencyState(applyLatencyResult(cleared, result("singbox_http", null, "core: 新错误", 21))),
    { kind: "core", detail: "core: 新错误" });
  assert.equal(applyNodeLatencyChange(node, change("xray_http", 1, null, 21)).latency_error, null);
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

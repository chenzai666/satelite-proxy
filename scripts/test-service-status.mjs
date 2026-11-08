import assert from "node:assert/strict";
import test from "node:test";
import {
  createServiceStatusMonitor, DEFAULT_SERVICE_IDS, MAX_VISIBLE_SERVICES,
  readVisibleServiceIds, SERVICE_STATUS_CATALOG,
} from "../src/serviceStatus.ts";
import { catalogs, translate } from "../src/i18n/messages.ts";

const allIds = SERVICE_STATUS_CATALOG.map(service => service.id);
const tick = () => new Promise(resolve => queueMicrotask(resolve));
const result = (id, overrides = {}) => ({
  id, status: "reachable", latencyMs: 32, httpStatus: 204, checkedAt: 1000, viaProxy: true, ...overrides,
});
const batch = (ids, overrides = {}) => ({ results: ids.map(id => result(id)), checkedAt: 1000, viaProxy: true, ...overrides });

// 仅模拟 IPC promise，不发网络请求、不启动浏览器或内核。
function mockIpc() {
  const calls = [];
  const monitor = createServiceStatusMonitor(ids => new Promise((resolve, reject) => {
    calls.push({ ids: [...ids], resolve, reject });
  }));
  return { monitor, calls };
}

test("固定目录恰好包含约定的 13 个唯一服务", () => {
  assert.deepEqual(allIds, ["google", "github", "youtube", "chatgpt", "claude", "gemini", "netflix", "disney", "prime", "spotify", "tiktok", "discord", "bilibili"]);
  assert.equal(new Set(allIds).size, 13);
  assert.ok(SERVICE_STATUS_CATALOG.every(service => service.title.length));
  assert.deepEqual(DEFAULT_SERVICE_IDS, ["google", "github"]);
});

test("展示选择仅接收固定 ID、去重、最多四个；损坏或空存储回到默认两项", () => {
  for (const raw of [null, "", "{", "null", "{}", "[]", '["unknown"]', "x".repeat(5000)]) {
    assert.deepEqual(readVisibleServiceIds(raw), DEFAULT_SERVICE_IDS);
  }
  assert.equal(MAX_VISIBLE_SERVICES, 4);
  assert.deepEqual(readVisibleServiceIds('["claude","claude",null,"unknown","google","github","youtube","netflix"]'), ["claude", "google", "github", "youtube"]);
  assert.deepEqual(readVisibleServiceIds('["github"]'), ["github"]);
});

test("StrictMode setup/cleanup/setup 只自动检测前两个展示项一次", async () => {
  const { monitor, calls } = mockIpc();
  const selected = ["claude", "google", "netflix", "discord"];
  monitor.start(selected);
  monitor.stop();
  monitor.start(selected);
  await tick();
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].ids, selected.slice(0, 2));
  calls[0].resolve(batch(calls[0].ids));
  await tick();
  assert.deepEqual(Object.keys(monitor.getSnapshot().results), selected.slice(0, 2));
  monitor.stop();
  monitor.start(selected);
  await tick();
  assert.equal(calls.length, 1);
  monitor.stop();
});

test("同步互斥拒绝单项与全批重复 in-flight，完成后可按需测全部", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS);
  await tick();
  await Promise.all([monitor.request(["google"]), monitor.request(allIds)]);
  assert.equal(calls.length, 1);
  calls[0].resolve(batch(calls[0].ids));
  await tick();
  const request = monitor.request(allIds);
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[1].ids, allIds);
  calls[1].resolve(batch(allIds));
  await request;
  assert.equal(Object.keys(monitor.getSnapshot().results).length, 13);
  assert.deepEqual(monitor.getSnapshot().pending, []);
  monitor.stop();
});

test("新会话快照为空，旧会话回包与失败均被取消，epoch 阻止旧结果覆盖新结果", async () => {
  for (const rejectOld of [false, true]) {
    const old = mockIpc();
    old.monitor.start(DEFAULT_SERVICE_IDS);
    await tick();
    old.monitor.stop();
    const current = mockIpc();
    assert.deepEqual(current.monitor.getSnapshot(), { results: {}, errors: {}, pending: [] });
    current.monitor.start(DEFAULT_SERVICE_IDS);
    await tick();
    current.calls[0].resolve(batch(DEFAULT_SERVICE_IDS));
    await tick();
    if (rejectOld) old.calls[0].reject(new Error("mock IPC failure"));
    else old.calls[0].resolve(batch(DEFAULT_SERVICE_IDS, { results: [result("google", { status: "restricted" })] }));
    await tick();
    assert.deepEqual(old.monitor.getSnapshot().results, {});
    assert.equal(current.monitor.getSnapshot().results.google.status, "reachable");
    assert.deepEqual(current.monitor.getSnapshot().errors, {});
    current.monitor.stop();
  }
  const { monitor, calls } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS);
  await tick();
  monitor.stop();
  monitor.start(DEFAULT_SERVICE_IDS);
  const newer = monitor.request(["google"]);
  calls[1].resolve(batch(["google"], { results: [result("google", { latencyMs: 9 })] }));
  await newer;
  calls[0].resolve(batch(DEFAULT_SERVICE_IDS));
  await tick();
  assert.equal(monitor.getSnapshot().results.google.latencyMs, 9);
  assert.equal(monitor.getSnapshot().results.github, undefined);
  monitor.stop();
});

test("单项和全批 IPC 失败保留所有旧结果，并标记对应错误；重试可恢复", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS);
  await tick();
  calls[0].resolve(batch(DEFAULT_SERVICE_IDS));
  await tick();
  const before = { ...monitor.getSnapshot().results };
  for (const ids of [["google"], allIds]) {
    const request = monitor.request(ids);
    calls.at(-1).reject(new Error("mock IPC failure"));
    await request;
    assert.deepEqual(monitor.getSnapshot().results, before);
    assert.ok(ids.every(id => monitor.getSnapshot().errors[id]));
    assert.deepEqual(monitor.getSnapshot().pending, []);
  }
  const retry = monitor.request(allIds);
  calls.at(-1).resolve(batch(allIds));
  await retry;
  assert.deepEqual(monitor.getSnapshot().errors, {});
  monitor.stop();
});

test("缺失、重复或非法单项保旧值，合法单项独立更新，不保留额外诊断字段", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS);
  await tick();
  calls[0].resolve(batch(DEFAULT_SERVICE_IDS));
  await tick();
  for (const bad of [[], [result("google"), result("google")], [result("google", { status: "unknown" })], [result("google", { latencyMs: NaN })], [result("google", { httpStatus: 700 })], [result("google", { viaProxy: "yes" })]]) {
    const request = monitor.request(DEFAULT_SERVICE_IDS);
    calls.at(-1).resolve(batch([], { results: [...bad, result("github", { latencyMs: 11, extra: "discard" })] }));
    await request;
    const snapshot = monitor.getSnapshot();
    assert.equal(snapshot.results.google.latencyMs, 32);
    assert.equal(snapshot.errors.google, true);
    assert.equal(snapshot.results.github.latencyMs, 11);
    assert.equal(snapshot.results.github.extra, undefined);
    assert.equal(snapshot.errors.github, undefined);
  }
  monitor.stop();
});

test("六类后端结果都按实展示，缺失的可选指标保持未提供", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS);
  await tick();
  calls[0].resolve(batch(DEFAULT_SERVICE_IDS));
  await tick();
  for (const status of ["reachable", "redirect", "restricted", "http_error", "timeout", "network_error"]) {
    const request = monitor.request(["google"]);
    calls.at(-1).resolve(batch([], { results: [result("google", { status, latencyMs: undefined, httpStatus: undefined, viaProxy: false })] }));
    await request;
    const row = monitor.getSnapshot().results.google;
    assert.equal(row.status, status);
    assert.equal(row.latencyMs, undefined);
    assert.equal(row.httpStatus, undefined);
    assert.equal(row.viaProxy, false);
  }
  monitor.stop();
});

test("非法批次保旧值；已停止、空 ID 和非目录 ID 不发 IPC", async () => {
  const { monitor, calls } = mockIpc();
  await monitor.request(allIds);
  assert.equal(calls.length, 0);
  monitor.start([]);
  await tick();
  for (const malformed of [null, {}, { results: [], checkedAt: NaN, viaProxy: true }]) {
    const request = monitor.request(["google"]);
    calls.at(-1).resolve(malformed);
    await request;
    assert.equal(monitor.getSnapshot().errors.google, true);
    assert.deepEqual(monitor.getSnapshot().pending, []);
  }
  const count = calls.length;
  await monitor.request([]);
  await monitor.request(["unknown"]);
  assert.equal(calls.length, count);
  monitor.stop();
  await monitor.request(allIds);
  assert.equal(calls.length, count);
});

test("中英文服务文案齐全，受限与跳转语义明确", () => {
  for (const locale of ["zh", "en"]) {
    for (const key of Object.keys(catalogs.en).filter(key => key.startsWith("serviceStatus."))) {
      assert.ok(catalogs[locale][key], `${locale}: ${key}`);
    }
  }
  assert.equal(translate("zh", "serviceStatus.restricted"), "HTTP 访问受限");
  assert.equal(translate("zh", "serviceStatus.redirect"), "可达需跳转");
  assert.match(translate("zh", "serviceStatus.scope"), /HTTP.*当前内核规则.*直连/);
});

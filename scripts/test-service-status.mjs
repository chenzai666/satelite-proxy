import assert from "node:assert/strict";
import test from "node:test";
import {
  createServiceStatusMonitor, DEFAULT_SERVICE_IDS, MAX_VISIBLE_SERVICES,
  readVisibleServiceIds, SERVICE_STATUS_CATALOG,
} from "../src/serviceStatus.ts";
import { catalogs, translate } from "../src/i18n/messages.ts";

const allIds = SERVICE_STATUS_CATALOG.map(service => service.id);
const availabilityStatuses = ["available", "unavailable", "blocked", "disallowed_isp", "unsupported_region", "originals_only", "coming_soon", "failed"];
const tick = () => new Promise(resolve => queueMicrotask(resolve));
const result = (id, overrides = {}) => ({
  id, status: "reachable", latencyMs: 32, httpStatus: 204, checkedAt: 1000, viaProxy: true, ...overrides,
});
const batch = (ids, overrides = {}) => ({ results: ids.map(id => result(id)), checkedAt: 1000, viaProxy: true, ...overrides });

// 仅模拟 IPC promise，不发网络请求、不启动浏览器或内核。
function mockIpc() {
  const calls = [];
  const timers = new Map();
  let time = 0, nextTimer = 0;
  const clock = {
    now: () => time,
    setTimeout(callback, ms) { const id = ++nextTimer; timers.set(id, { callback, at: time + ms }); return id; },
    clearTimeout(id) { timers.delete(id); },
  };
  const monitor = createServiceStatusMonitor(ids => new Promise((resolve, reject) => {
    calls.push({ ids: [...ids], resolve, reject });
  }), clock);
  async function advance(ms) {
    const target = time + ms;
    while (true) {
      const entry = [...timers].filter(([, timer]) => timer.at <= target).sort((a, b) => a[1].at - b[1].at)[0];
      if (!entry) break;
      const [id, timer] = entry;
      time = timer.at; timers.delete(id); timer.callback(); await tick();
    }
    time = target; await tick();
  }
  return { monitor, calls, advance, timers };
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

test("StrictMode setup/cleanup/setup 只发一批，包含全部四个展示项", async () => {
  const { monitor, calls } = mockIpc();
  const selected = ["claude", "google", "netflix", "discord"];
  monitor.start(selected);
  monitor.stop();
  monitor.start(selected);
  await tick();
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].ids, selected);
  calls[0].resolve(batch(calls[0].ids));
  await tick();
  assert.deepEqual(Object.keys(monitor.getSnapshot().results), selected);
  monitor.stop();
  monitor.start(selected);
  await tick();
  assert.equal(calls.length, 2);
  calls[1].resolve(batch(selected));
  await tick();
  monitor.stop();
});

test("新增展示项在当前批次后补测，移除排队项不再检测", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS); await tick();
  monitor.setVisible(["google", "github", "youtube"]);
  monitor.setVisible(["google", "github", "youtube", "chatgpt"]);
  monitor.setVisible(["google", "github", "chatgpt"]);
  assert.equal(calls.length, 1);
  calls[0].resolve(batch(DEFAULT_SERVICE_IDS)); await tick();
  assert.deepEqual(calls[1].ids, ["chatgpt"]);
  calls[1].resolve(batch(["chatgpt"])); await tick();
  monitor.setVisible(["google", "github", "chatgpt", "youtube"]);
  assert.deepEqual(calls[2].ids, ["youtube"]);
  calls[2].resolve(batch(["youtube"])); await tick();
  monitor.stop();
});

test("网络失败按30/60/120/300秒退避，可达后恢复五分钟周期", async () => {
  const { monitor, calls, advance, timers } = mockIpc();
  monitor.start(["github"]); await tick();
  for (const delay of [30000, 60000, 120000, 300000]) {
    calls.at(-1).resolve(batch(["github"], { results: [result("github", { status: "timeout" })] })); await tick();
    const count = calls.length;
    await advance(delay - 1); assert.equal(calls.length, count);
    await advance(1); assert.equal(calls.length, count + 1);
  }
  calls.at(-1).resolve(batch(["github"])); await tick();
  const count = calls.length;
  await advance(299999); assert.equal(calls.length, count);
  await advance(1); assert.equal(calls.length, count + 1);
  monitor.stop(); assert.equal(timers.size, 0);
});

test("IPC切换忙碌会自动恢复，HTTP受限不进入快速重试", async () => {
  const { monitor, calls, advance } = mockIpc();
  monitor.start(["github"]); await tick();
  calls[0].reject(new Error("kernel transitioning")); await tick();
  await advance(2000); assert.equal(calls.length, 2);
  calls[1].resolve(batch(["github"], { results: [result("github", { status: "restricted", httpStatus: 403 })] })); await tick();
  await advance(299999); assert.equal(calls.length, 2);
  await advance(1); assert.equal(calls.length, 3);
  monitor.stop();
});

test("八种服务语义状态保持五分钟周期，焦点和可见性恢复复用结果，不触发失败退避", async () => {
  for (const status of availabilityStatuses) {
    const { monitor, calls, advance, timers } = mockIpc();
    monitor.start(["chatgpt"]); await tick();
    calls[0].resolve(batch([], { results: [result("chatgpt", { status, region: "US", checkKind: "availability" })] }));
    await tick();
    const snapshot = monitor.getSnapshot();
    assert.equal(snapshot.results.chatgpt.status, status);
    assert.deepEqual(snapshot.errors, {});
    await advance(60000); monitor.refreshVisible();
    monitor.setEnabled(false); await advance(60000); monitor.setEnabled(true); monitor.refreshVisible();
    assert.equal(calls.length, 1, status);
    assert.strictEqual(monitor.getSnapshot(), snapshot);
    await advance(179999); assert.equal(calls.length, 1, status);
    await advance(1); assert.equal(calls.length, 2, status);
    assert.deepEqual(calls[1].ids, ["chatgpt"]);
    calls[1].resolve(batch([], { results: [result("chatgpt", { status, checkKind: "availability" })] })); await tick();
    await advance(299999); assert.equal(calls.length, 2, status);
    await advance(1); assert.equal(calls.length, 3, status);
    monitor.stop(); assert.equal(timers.size, 0);
  }
});

test("隐藏或离线暂停计时，切回窗口复用近期结果，过期才补测", async () => {
  const { monitor, calls, advance, timers } = mockIpc();
  monitor.setEnabled(false); monitor.start(DEFAULT_SERVICE_IDS); await tick();
  await advance(120000); assert.equal(calls.length, 0);
  monitor.setVisible(["github", "youtube", "chatgpt"]);
  monitor.setEnabled(true); assert.deepEqual(calls[0].ids, ["github", "youtube", "chatgpt"]);
  calls[0].resolve(batch(calls[0].ids)); await tick();
  monitor.refreshVisible(); monitor.refreshVisible(); assert.equal(calls.length, 1);
  await advance(5000); monitor.refreshVisible(); assert.equal(calls.length, 1);
  monitor.setEnabled(false); assert.equal(timers.size, 0);
  await advance(120000); assert.equal(calls.length, 1);
  monitor.setEnabled(true); assert.equal(calls.length, 1);
  monitor.setEnabled(false); await advance(180000);
  monitor.setEnabled(true); assert.equal(calls.length, 2);
  monitor.refreshVisible(); monitor.refreshVisible(); assert.equal(calls.length, 2);
  monitor.stop(); assert.equal(timers.size, 0);
});

test("频繁焦点事件不会刷新结果或推迟原来的五分钟截止时间", async () => {
  const { monitor, calls, advance } = mockIpc();
  monitor.start(["github"]); await tick();
  calls[0].resolve(batch(["github"])); await tick();
  const snapshot = monitor.getSnapshot();
  for (let minute = 0; minute < 4; minute++) {
    await advance(60000); monitor.refreshVisible(); monitor.refreshVisible();
    assert.equal(calls.length, 1);
    assert.strictEqual(monitor.getSnapshot(), snapshot);
  }
  await advance(60000); assert.equal(calls.length, 2);
  monitor.stop();
});

test("隐藏期间完成的检测仍保留有效期，恢复联网可显式单次重测", async () => {
  const { monitor, calls, advance } = mockIpc();
  monitor.start(["github"]); await tick();
  monitor.setEnabled(false);
  await advance(1000); calls[0].resolve(batch(["github"])); await tick();
  await advance(10000); monitor.setEnabled(true);
  assert.equal(calls.length, 1);
  monitor.refreshVisible(true); assert.equal(calls.length, 2);
  calls[1].resolve(batch(["github"])); await tick();
  monitor.refreshVisible(true); assert.equal(calls.length, 2);
  monitor.stop();
});

test("IPC异常仅快速确认一次，切回窗口不绕过后续退避", async () => {
  const { monitor, calls, advance } = mockIpc();
  monitor.start(["github"]); await tick();
  calls[0].reject(new Error("core transitioning")); await tick();
  await advance(2000); assert.equal(calls.length, 2);
  calls[1].reject(new Error("still transitioning")); await tick();
  await advance(5000); monitor.setEnabled(false); monitor.setEnabled(true); monitor.refreshVisible();
  assert.equal(calls.length, 2);
  await advance(25000); assert.equal(calls.length, 3);
  monitor.stop();
});

test("新增展示项已经在当前批次内时复用在途请求", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS); await tick();
  calls[0].resolve(batch(DEFAULT_SERVICE_IDS)); await tick();
  const all = monitor.request(allIds);
  monitor.setVisible(["google", "github", "youtube", "chatgpt"]);
  calls[1].resolve(batch(allIds)); await all;
  assert.equal(calls.length, 2);
  assert.equal(monitor.getSnapshot().results.youtube.status, "reachable");
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

test("旧后端六类结果缺少新字段仍接受，缺失的可选指标保持未提供", async () => {
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
    assert.equal(row.region, undefined);
    assert.equal(row.checkKind, undefined);
    assert.equal(row.viaProxy, false);
  }
  monitor.stop();
});

test("检测类型和地区独立可选，合法字段按契约复制，后续缺字段清除旧值", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start(["netflix"]); await tick();
  calls[0].resolve(batch(["netflix"])); await tick();
  for (const fields of [
    { region: "US", checkKind: "availability" }, { region: "HK", checkKind: "reachability" },
    { region: "JP" }, { checkKind: "availability" }, {},
  ]) {
    const request = monitor.request(["netflix"]);
    const source = result("netflix", { status: "available", ...fields, diagnostic: "discard" });
    calls.at(-1).resolve(batch([], { results: [source] })); await request;
    const row = monitor.getSnapshot().results.netflix;
    assert.equal(row.status, "available");
    assert.equal(row.region, fields.region);
    assert.equal(row.checkKind, fields.checkKind);
    assert.equal(row.diagnostic, undefined);
    assert.notStrictEqual(row, source);
    source.region = "DE"; source.checkKind = "reachability";
    assert.equal(row.region, fields.region);
    assert.equal(row.checkKind, fields.checkKind);
    assert.deepEqual(monitor.getSnapshot().errors, {});
  }
  monitor.stop();
});

test("非法地区或检测类型保留旧结果，合法同批服务仍更新，合法重试清除错误", async () => {
  const { monitor, calls } = mockIpc();
  const selected = ["chatgpt", "google"];
  const original = result("chatgpt", { status: "available", region: "US", checkKind: "availability" });
  monitor.start(selected); await tick();
  calls[0].resolve(batch([], { results: [original, result("google")] })); await tick();
  const retained = monitor.getSnapshot().results.chatgpt;
  const invalidFields = [
    ...["", "U", "USA", "us", "Us", " US", "US ", "US\n", "US\r\n", "美国", "ＵＳ", null, 12, true, ["US"], {}].map(region => ({ region })),
    ...["", "Availability", "http", "availability\n", null, 1, true, ["availability"], {}].map(checkKind => ({ checkKind })),
  ];
  for (const fields of invalidFields) {
    const request = monitor.request(selected);
    calls.at(-1).resolve(batch([], { results: [
      result("chatgpt", { status: "blocked", region: "GB", checkKind: "availability", ...fields }),
      result("google", { checkKind: "reachability", latencyMs: 11 }),
    ] })); await request;
    const snapshot = monitor.getSnapshot();
    assert.strictEqual(snapshot.results.chatgpt, retained, JSON.stringify(fields));
    assert.equal(snapshot.errors.chatgpt, true);
    assert.equal(snapshot.results.google.checkKind, "reachability");
    assert.equal(snapshot.results.google.latencyMs, 11);
    assert.equal(snapshot.errors.google, undefined);
  }
  const retry = monitor.request(["chatgpt"]);
  calls.at(-1).resolve(batch(["chatgpt"])); await retry;
  assert.deepEqual(monitor.getSnapshot().errors, {});
  assert.equal(monitor.getSnapshot().results.chatgpt.region, undefined);
  assert.equal(monitor.getSnapshot().results.chatgpt.checkKind, undefined);
  monitor.stop();
});

test("旧ChatGPT通用403保持restricted，HTTP码不覆盖后端服务语义状态", async () => {
  const { monitor, calls, advance } = mockIpc();
  monitor.start(["chatgpt"]); await tick();
  calls[0].resolve(batch([], { results: [result("chatgpt", { status: "restricted", httpStatus: 403 })] })); await tick();
  assert.equal(monitor.getSnapshot().results.chatgpt.status, "restricted");
  assert.equal(monitor.getSnapshot().results.chatgpt.checkKind, undefined);
  await advance(299999); assert.equal(calls.length, 1);
  await advance(1); assert.equal(calls.length, 2);
  calls[1].resolve(batch([], { results: [result("chatgpt", { status: "unsupported_region", httpStatus: 403, checkKind: "availability" })] })); await tick();
  assert.equal(monitor.getSnapshot().results.chatgpt.status, "unsupported_region");
  assert.deepEqual(monitor.getSnapshot().errors, {});
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
  const serviceKeys = locale => Object.keys(catalogs[locale]).filter(key => key.startsWith("serviceStatus.")).sort();
  assert.deepEqual(serviceKeys("zh"), serviceKeys("en"));
  for (const locale of ["zh", "en"]) {
    for (const key of Object.keys(catalogs.en).filter(key => key.startsWith("serviceStatus."))) {
      assert.ok(catalogs[locale][key], `${locale}: ${key}`);
    }
    for (const key of ["available", "unavailable", "blocked", "disallowedIsp", "unsupportedRegion", "originalsOnly", "comingSoon", "resultFailed", "checkKind", "region", "reachability", "availability", "notProvided"]) {
      assert.ok(catalogs[locale][`serviceStatus.${key}`], `${locale}: ${key}`);
    }
  }
  assert.equal(translate("zh", "serviceStatus.restricted"), "HTTP 访问受限");
  assert.equal(translate("zh", "serviceStatus.redirect"), "可达需跳转");
  assert.match(translate("zh", "serviceStatus.scope"), /Google、GitHub.*连通性.*其他服务.*只读响应.*当前内核规则.*直连.*403.*请求受限/);
  assert.match(translate("en", "serviceStatus.scope"), /Google and GitHub.*connectivity.*other services.*read-only.*kernel rules.*direct.*403.*restricted request/);
});

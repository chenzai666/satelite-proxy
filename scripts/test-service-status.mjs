import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  createServiceStatusMonitor, DEFAULT_SERVICE_IDS, MAX_VISIBLE_SERVICES,
  readVisibleServiceIds, SERVICE_STATUS_CATALOG,
  SERVICE_REASON_MESSAGES, SERVICE_REGION_REASON_MESSAGES,
  isServiceExitIpLiteral, primaryServiceRoute,
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
    assert.equal(row.reason, undefined);
    assert.equal(row.regionReason, undefined);
    assert.equal(row.routes, undefined);
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

test("原因白名单完整且独立可选，合法字段复制后不受原对象影响，旧响应清除旧原因", async () => {
  assert.deepEqual(Object.keys(SERVICE_REASON_MESSAGES), [
    "response_too_large", "unsupported_encoding", "invalid_response", "unrecognized_response", "expected_denial", "challenge",
    "availability_requires_session", "unsafe_redirect", "redirect_loop", "redirect_limit",
  ]);
  assert.deepEqual(Object.keys(SERVICE_REGION_REASON_MESSAGES), ["not_provided", "lookup_timeout", "invalid_response", "conflicting_region"]);
  const { monitor, calls, timers } = mockIpc();
  monitor.start([]); await tick();
  try {
    for (const reason of [...Object.keys(SERVICE_REASON_MESSAGES), undefined]) {
      for (const regionReason of [...Object.keys(SERVICE_REGION_REASON_MESSAGES), undefined]) {
        const request = monitor.request(["youtube"]);
        const source = result("youtube", {
          status: "failed", httpStatus: 200, reason, regionReason,
          body: "private raw body", diagnostic: "untrusted diagnostic",
        });
        calls.at(-1).resolve(batch([], { results: [source] })); await request;
        const row = monitor.getSnapshot().results.youtube;
        assert.equal(row.reason, reason);
        assert.equal(row.regionReason, regionReason);
        assert.equal(row.region, undefined);
        assert.equal(row.status, "failed");
        assert.equal(row.httpStatus, 200);
        assert.equal(row.body, undefined);
        assert.equal(row.diagnostic, undefined);
        assert.notStrictEqual(row, source);
        source.reason = "challenge"; source.regionReason = "lookup_timeout";
        assert.equal(row.reason, reason);
        assert.equal(row.regionReason, regionReason);
        assert.deepEqual(monitor.getSnapshot().errors, {});
      }
    }
    const request = monitor.request(["youtube"]);
    calls.at(-1).resolve(batch(["youtube"])); await request;
    assert.equal(monitor.getSnapshot().results.youtube.reason, undefined);
    assert.equal(monitor.getSnapshot().results.youtube.regionReason, undefined);
  } finally { monitor.stop(); }
  assert.equal(timers.size, 0);
});

test("原因字段拒绝任意正文、非字符串、大小写变体与其他白名单值，单项隔离并可恢复", async () => {
  const { monitor, calls } = mockIpc();
  const selected = ["chatgpt", "google"];
  monitor.start(selected); await tick();
  calls[0].resolve(batch([], { results: [
    result("chatgpt", { status: "available", httpStatus: 403, reason: "expected_denial", regionReason: "not_provided" }),
    result("google"),
  ] })); await tick();
  const retained = monitor.getSnapshot().results.chatgpt;
  const invalidValues = ["", "raw response body", "<script>alert(1)</script>", "toString", "__proto__", null, 1, true, [], {}];
  try {
    for (const [field, whitelist, otherValues] of [
      ["reason", Object.keys(SERVICE_REASON_MESSAGES), ["not_provided", "lookup_timeout"]],
      ["regionReason", Object.keys(SERVICE_REGION_REASON_MESSAGES), ["expected_denial", "challenge", "response_too_large", "unsupported_encoding", "unrecognized_response"]],
    ]) {
      const values = [...invalidValues, ...otherValues,
        ...whitelist.flatMap(value => [value.toUpperCase(), ` ${value}`, `${value} `, `${value}\n`, `${value}\0`, [value]]),
      ];
      for (const value of values) {
        const request = monitor.request(selected);
        calls.at(-1).resolve(batch([], { results: [
          result("chatgpt", { status: "failed", [field]: value }), result("google", { latencyMs: 11 }),
        ] })); await request;
        const snapshot = monitor.getSnapshot();
        assert.strictEqual(snapshot.results.chatgpt, retained, `${field}: ${JSON.stringify(value)}`);
        assert.equal(snapshot.errors.chatgpt, true);
        assert.equal(snapshot.results.google.latencyMs, 11);
        assert.equal(snapshot.errors.google, undefined);
      }
    }
    const retry = monitor.request(["chatgpt"]);
    calls.at(-1).resolve(batch(["chatgpt"])); await retry;
    assert.deepEqual(monitor.getSnapshot().errors, {});
    assert.equal(monitor.getSnapshot().results.chatgpt.reason, undefined);
    assert.equal(monitor.getSnapshot().results.chatgpt.regionReason, undefined);
  } finally { monitor.stop(); }
});

test("预期拒绝原因独立于服务状态，真实403保留，地区缺失不推测，刷新周期不变", async () => {
  const { monitor, calls, advance, timers } = mockIpc();
  monitor.start(["chatgpt"]); await tick();
  try {
    for (const status of ["available", "restricted", "failed"]) {
      calls.at(-1).resolve(batch([], { results: [result("chatgpt", {
        status, httpStatus: 403, reason: "expected_denial", regionReason: "not_provided", checkKind: "availability",
      })] })); await tick();
      const row = monitor.getSnapshot().results.chatgpt;
      assert.equal(row.status, status);
      assert.equal(row.httpStatus, 403);
      assert.equal(row.reason, "expected_denial");
      assert.equal(row.region, undefined);
      const count = calls.length;
      await advance(299999); assert.equal(calls.length, count);
      await advance(1); assert.equal(calls.length, count + 1);
    }
  } finally { monitor.stop(); }
  assert.equal(timers.size, 0);
});

test("判定与地区原因映射固定双语文案，Failed仅表示无法判定，详情保留HTTP与地区占位", () => {
  assert.equal(translate("zh", "serviceStatus.resultFailed"), "无法判定");
  assert.equal(translate("en", "serviceStatus.resultFailed"), "Inconclusive");
  const expected = [
    [SERVICE_REASON_MESSAGES.response_too_large, "响应超过大小限制", "Response exceeds the size limit"],
    [SERVICE_REASON_MESSAGES.unsupported_encoding, "不支持的响应编码", "Unsupported response encoding"],
    [SERVICE_REASON_MESSAGES.invalid_response, "服务响应无效", "Invalid service response"],
    [SERVICE_REASON_MESSAGES.unrecognized_response, "未识别的服务响应", "Unrecognized service response"],
    [SERVICE_REASON_MESSAGES.expected_denial, "探测入口的预期拒绝响应", "Expected denial response from the probe endpoint"],
    [SERVICE_REASON_MESSAGES.challenge, "需要完成验证挑战", "Verification challenge required"],
    [SERVICE_REASON_MESSAGES.availability_requires_session, "已获取地区，可用性需通过会话确认", "Region reported; a session is required to confirm availability"],
    [SERVICE_REASON_MESSAGES.unsafe_redirect, "重定向目标超出允许范围", "Redirect target is outside the allowed scope"],
    [SERVICE_REASON_MESSAGES.redirect_loop, "检测到重定向循环", "Redirect loop detected"],
    [SERVICE_REASON_MESSAGES.redirect_limit, "已达重定向次数上限", "Redirect limit reached"],
    [SERVICE_REGION_REASON_MESSAGES.not_provided, "服务未提供地区", "Service did not provide a region"],
    [SERVICE_REGION_REASON_MESSAGES.lookup_timeout, "地区查询超时", "Region lookup timed out"],
    [SERVICE_REGION_REASON_MESSAGES.invalid_response, "地区响应无效", "Invalid region response"],
    [SERVICE_REGION_REASON_MESSAGES.conflicting_region, "服务响应提供的地区存在冲突", "Service responses reported conflicting regions"],
  ];
  for (const [key, zh, en] of expected) {
    assert.equal(translate("zh", key), zh);
    assert.equal(translate("en", key), en);
  }
  const card = readFileSync(new URL("../src/components/ServiceStatusCard.tsx", import.meta.url), "utf8");
  assert.match(card, /failed: "serviceStatus\.resultFailed"/);
  assert.ok(card.includes("t(SERVICE_REASON_MESSAGES[result.reason])"));
  assert.ok(card.includes("t(SERVICE_REGION_REASON_MESSAGES[result.regionReason])"));
  assert.ok(card.includes('result?.httpStatus ?? "—"'));
  assert.ok(card.includes('result?.region ?? t("serviceStatus.notProvided")'));
  assert.ok(card.includes('result.region ?? "—"'));
  assert.doesNotMatch(card, /dangerouslySetInnerHTML|result\??\.body/);
  const css = readFileSync(new URL("../src/components/ServiceStatusCard.css", import.meta.url), "utf8");
  assert.match(css, /\.service-status-state\[data-status="failed"\]\s*\{\s*color: var\(--muted\);\s*\}/);
});

const route = (overrides = {}) => ({ role: "primary", host: "www.google.com", matchStatus: "matched", ...overrides });

test("出口只接受严格 IPv4/IPv6 literal，拒绝URL、主机名、目的地址包装及畸形IP", () => {
  for (const value of ["0.0.0.0", "1.2.3.4", "255.255.255.255", "::", "::1", "2001:db8::1234", "2001:DB8:0:1:2:3:4:5", "::ffff:192.0.2.128", "ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255"]) {
    assert.equal(isServiceExitIpLiteral(value), true, value);
  }
  for (const value of [undefined, null, 0, true, {}, [], "", "US", "example.com", "https://1.2.3.4", "http://[::1]", "[::1]", "1.2.3.4:443", "1.2.3.4/32", "2001:db8::/64", "fe80::1%eth0", "127.1", "1234", "0x7f000001", "01.2.3.4", "256.2.3.4", "1.2.3.-1", "1.2.3.4\n", " 1.2.3.4", "1.2.3.4 ", "::1\n", "::g", "1:2:3:4:5:6:7", "1:2:3:4:5:6:7:8:9", "1::2::3", ":::1", "::ffff:999.0.0.1", "::ffff:192.00.2.1", "::1/", "::1?x", "::1#x", "::1]@evil.test", "f".repeat(46)]) {
    assert.equal(isServiceExitIpLiteral(value), false, JSON.stringify(value));
  }
});

test("主辅请求、两类出口来源和缓存标志白名单深复制，Google/GitHub无地区仍接受", async () => {
  const { monitor, calls, timers } = mockIpc();
  monitor.start([]); await tick();
  try {
    for (const id of DEFAULT_SERVICE_IDS) {
      const sourceRoutes = [
        route({ role: "auxiliary", host: "region.example.com", exitIp: "192.0.2.8", exitSource: "service_response", exitCached: false }),
        route({ host: "www.example.com", chains: ["Auto", "Proxy", "Node"], rule: "DOMAIN-SUFFIX", rulePayload: "example.com", outboundTag: "Node",
          exitIp: "2001:db8::1", exitSource: "pinned_outbound_probe", exitCached: true,
          endpoint: "https://198.51.100.9/", destinationIP: "198.51.100.9", extra: "discard" }),
      ];
      const expected = sourceRoutes.map(({ endpoint, destinationIP, extra, ...fields }) => structuredClone(fields));
      const request = monitor.request([id]);
      calls.at(-1).resolve(batch([], { results: [result(id, { routes: sourceRoutes, reason: "expected_denial", httpStatus: 403, endpoint: "https://198.51.100.9/", destinationIP: "198.51.100.9" })] })); await request;
      const row = monitor.getSnapshot().results[id];
      assert.deepEqual(row.routes, expected);
      assert.notStrictEqual(row.routes, sourceRoutes);
      assert.notStrictEqual(row.routes[1].chains, sourceRoutes[1].chains);
      assert.equal(primaryServiceRoute(row).exitIp, "2001:db8::1");
      assert.equal(row.region, undefined);
      assert.equal(row.reason, "expected_denial");
      assert.equal(row.httpStatus, 403);
      assert.equal(row.endpoint, undefined); assert.equal(row.destinationIP, undefined);
      sourceRoutes[1].chains.push("changed"); sourceRoutes[1].exitIp = "192.0.2.99"; sourceRoutes.push(route());
      assert.deepEqual(row.routes, expected);
    }
  } finally { monitor.stop(); }
  assert.equal(timers.size, 0);
});

test("routes各字段边界、角色、来源与类型严格校验，非法服务保旧且同批合法项更新", async () => {
  const { monitor, calls, timers } = mockIpc();
  monitor.start(DEFAULT_SERVICE_IDS); await tick();
  calls[0].resolve(batch(DEFAULT_SERVICE_IDS)); await tick();
  const retained = monitor.getSnapshot().results.google;
  const invalidRoutes = [null, {}, "routes", [null], [[]], Array.from({ length: 9 }, () => route()),
    ...[
      { role: undefined }, { role: "Primary" }, { role: "other" }, { host: undefined }, { host: "" }, { host: "a".repeat(254) }, { host: 123 },
      { matchStatus: undefined }, { matchStatus: "MATCHED" }, { matchStatus: "unknown" }, { matchStatus: null },
      { chains: null }, { chains: "Proxy" }, { chains: [123] }, { chains: ["a".repeat(257)] }, { chains: Array(17).fill("Node") },
      ...["rule", "rulePayload"].flatMap(field => [{ [field]: "a".repeat(513) }, { [field]: null }, { [field]: 12 }]),
      { outboundTag: "a".repeat(257) }, { outboundTag: {} }, { exitIp: "example.com" }, { exitIp: "https://1.2.3.4" },
      { exitIp: "256.2.3.4" }, { exitIp: "[::1]" }, { exitIp: null }, { exitIp: "" },
      { exitSource: "destinationIP" }, { exitSource: "Service_Response" }, { exitSource: null }, { exitSource: 1 },
      { exitCached: "true" }, { exitCached: 0 }, { exitCached: null },
    ].map(fields => [route(fields)]),
  ];
  try {
    for (const routes of invalidRoutes) {
      const request = monitor.request(DEFAULT_SERVICE_IDS);
      calls.at(-1).resolve(batch([], { results: [result("google", { routes, status: "blocked" }), result("github", { latencyMs: 11 })] })); await request;
      const snapshot = monitor.getSnapshot();
      assert.strictEqual(snapshot.results.google, retained, JSON.stringify(routes));
      assert.equal(snapshot.errors.google, true);
      assert.equal(snapshot.results.github.latencyMs, 11);
      assert.equal(snapshot.errors.github, undefined);
    }
    const boundary = route({ host: "a".repeat(253), chains: Array(16).fill("a".repeat(256)), rule: "r".repeat(512), rulePayload: "p".repeat(512), outboundTag: "o".repeat(256) });
    for (const routes of [[], Array.from({ length: 8 }, () => boundary), [route({ chains: [], rule: "MATCH", rulePayload: "" })]]) {
      const request = monitor.request(["google"]);
      calls.at(-1).resolve(batch([], { results: [result("google", { routes })] })); await request;
      assert.deepEqual(monitor.getSnapshot().results.google.routes, routes);
      assert.deepEqual(monitor.getSnapshot().errors, {});
    }
    const legacy = monitor.request(["google"]); calls.at(-1).resolve(batch(["google"])); await legacy;
    assert.equal(monitor.getSnapshot().results.google.routes, undefined);
  } finally { monitor.stop(); }
  assert.equal(timers.size, 0);
});

test("出口不会从auxiliary、地区、endpoint、destinationIP或全局IP推测；unsupported保服务状态", async () => {
  const { monitor, calls } = mockIpc();
  monitor.start([]); await tick();
  try {
    for (const routes of [undefined, [], [route({ role: "auxiliary", exitIp: "192.0.2.8" })],
      [route({ endpoint: "https://192.0.2.9", destinationIP: "192.0.2.9", exit_ip: "192.0.2.9" })],
      [route({ matchStatus: "unsupported" })], [route({ matchStatus: "not_observed" })]]) {
      const request = monitor.request(["netflix"]);
      calls.at(-1).resolve(batch([], { results: [result("netflix", { status: "available", region: "US", routes,
        endpoint: "https://192.0.2.10", destinationIP: "192.0.2.10", globalIp: "192.0.2.11" })] })); await request;
      const row = monitor.getSnapshot().results.netflix;
      assert.equal(row.status, "available");
      assert.equal(primaryServiceRoute(row)?.exitIp, undefined);
      assert.equal(row.globalIp, undefined); assert.equal(row.destinationIP, undefined); assert.equal(row.endpoint, undefined);
      for (const evidence of row.routes ?? []) {
        assert.equal(evidence.destinationIP, undefined); assert.equal(evidence.endpoint, undefined); assert.equal(evidence.exit_ip, undefined);
      }
      assert.deepEqual(monitor.getSnapshot().errors, {});
    }
    assert.equal(primaryServiceRoute(), undefined);
  } finally { monitor.stop(); }
});

test("会话确认说明保持Disney地区与后端状态，路由证据与原因字段共存且刷新周期不变", async () => {
  const { monitor, calls, advance, timers } = mockIpc();
  monitor.start(["disney"]); await tick();
  calls[0].resolve(batch([], { results: [result("disney", { status: "reachable", region: "US", reason: "availability_requires_session", regionReason: "conflicting_region", routes: [route({ host: "www.disneyplus.com", matchStatus: "unsupported" })] })] })); await tick();
  try {
    const row = monitor.getSnapshot().results.disney;
    assert.equal(row.status, "reachable"); assert.equal(row.region, "US");
    assert.equal(row.reason, "availability_requires_session"); assert.equal(row.regionReason, "conflicting_region");
    assert.equal(primaryServiceRoute(row).matchStatus, "unsupported");
    await advance(299999); assert.equal(calls.length, 1);
    await advance(1); assert.equal(calls.length, 2);
  } finally { monitor.stop(); }
  assert.equal(timers.size, 0);
});

test("路由详情具备中英文来源、未确认与内核提示；主卡只读取primary出口且长文本可换行", () => {
  const labels = [
    ["exitServiceResponse", "服务回显", "Service response"],
    ["exitPinnedProbe", "按已匹配出站复测", "Re-probed through the matched outbound"],
    ["routeNotConfirmed", "尚未确认", "Not yet confirmed"],
    ["routeUnsupported", "内核暂不支持路线证据", "Route evidence is not supported by this kernel"],
  ];
  for (const [key, zh, en] of labels) {
    assert.equal(translate("zh", `serviceStatus.${key}`), zh); assert.equal(translate("en", `serviceStatus.${key}`), en);
  }
  const card = readFileSync(new URL("../src/components/ServiceStatusCard.tsx", import.meta.url), "utf8");
  assert.match(card, /const exitIp = primaryServiceRoute\(results\[id\]\)\?\.exitIp/);
  assert.match(card, /exitIp && <span className="service-status-card-exit"/);
  assert.match(card, /routeDetails\(primaryServiceRoute\(result\)\)/);
  assert.match(card, /route\.role === "auxiliary"/);
  assert.match(card, /<details className="service-status-auxiliary">/);
  assert.match(card, /route\.chains\.join\(" → "\)/);
  assert.doesNotMatch(card, /destinationIP|globalIp|dangerouslySetInnerHTML/);
  const css = readFileSync(new URL("../src/components/ServiceStatusCard.css", import.meta.url), "utf8");
  assert.match(css, /\.service-status-card-exit\s*\{[^}]*min-width: 0;[^}]*overflow-wrap: anywhere;/);
  assert.match(css, /\.service-status-route-fields dd\s*\{[^}]*min-width: 0;[^}]*overflow-wrap: anywhere;/);
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

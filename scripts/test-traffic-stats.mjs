import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { formatTrafficValue, trafficStatsRows } from "../src/trafficStats.ts";
import { translate } from "../src/i18n/messages.ts";

const fields = ["upload_speed", "download_speed", "upload_total", "download_total"];
const bucket = (up, down, upload, download) => ({
  upload_speed: up, download_speed: down, upload_total: upload, download_total: download,
});
const zero = () => bucket(0, 0, 0, 0);
const unknown = () => Object.fromEntries(fields.map(key => [key, null]));
function status(source = "connections", unclassified = bucket(512, 1024, 4096, 8192)) {
  const proxy = bucket(2048, 8192, 32768, 65536);
  const direct = bucket(1024, 4096, 16384, 32768);
  const total = Object.fromEntries(fields.map(key => [key, proxy[key] + direct[key] + unclassified[key]]));
  return { running: true, ...total, traffic_breakdown: { source, proxy, direct, unclassified } };
}

function assertReconciled(rows) {
  const total = rows.find(row => row.id === "total").values;
  for (const key of fields) {
    assert.equal(rows.filter(row => row.id !== "total").reduce((sum, row) => sum + row.values[key], 0), total[key], key);
  }
}

test("缺快照与旧后端：代理直连显示缺失，保留根合计", () => {
  for (const missing of [null, undefined]) {
    assert.deepEqual(trafficStatsRows(missing), ["proxy", "direct", "total"].map(id => ({ id, values: unknown() })));
  }
  for (const running of [true, false]) {
    const root = { running, ...bucket(123, 456, 789, 1011) };
    const rows = trafficStatsRows(root);
    assert.deepEqual(rows.slice(0, 2).map(row => row.values), [unknown(), unknown()]);
    assert.deepEqual(rows[2].values, bucket(running ? 123 : 0, running ? 456 : 0, 789, 1011));
  }
});

test("首轮 unavailable 与无接口：忽略占位桶的零或旧值，仍显示横线", () => {
  for (const running of [true, false]) {
    const snapshot = { ...status("unavailable"), running };
    const rows = trafficStatsRows(snapshot);
    assert.deepEqual(rows.map(row => row.id), ["proxy", "direct", "total"]);
    assert.deepEqual(rows[0].values, unknown());
    assert.deepEqual(rows[1].values, unknown());
    assert.equal(rows[2].values.upload_total, snapshot.upload_total);
  }
});

for (const source of ["connections", "outbound"]) {
  test(`${source}：四类行的速度、本次累计均与根字段逐方向对账`, () => {
    const snapshot = status(source);
    const before = structuredClone(snapshot);
    const rows = trafficStatsRows(snapshot);
    assert.deepEqual(rows.map(row => row.id), ["proxy", "direct", "unclassified", "total"]);
    for (const row of rows) {
      const expected = row.id === "total" ? snapshot : snapshot.traffic_breakdown[row.id];
      assert.deepEqual(row.values, Object.fromEntries(fields.map(key => [key, expected[key]])));
    }
    assertReconciled(rows);
    assert.deepEqual(snapshot, before, "展示不能修改快照");
  });

  test(`${source}：未分类全零时隐藏该行，保留真实的零值`, () => {
    const rows = trafficStatsRows(status(source, zero()));
    assert.deepEqual(rows.map(row => row.id), ["proxy", "direct", "total"]);
    assertReconciled(rows);
    const idle = { running: true, ...zero(), traffic_breakdown: { source, proxy: zero(), direct: zero(), unclassified: zero() } };
    for (const row of trafficStatsRows(idle)) assert.deepEqual(row.values, zero());
  });

  test(`${source}：停止归零所有速度，累计仍与根字段对账并跟随后端重置`, () => {
    const snapshot = { ...status(source), running: false };
    const rows = trafficStatsRows(snapshot);
    for (const row of rows) {
      const expected = row.id === "total" ? snapshot : snapshot.traffic_breakdown[row.id];
      assert.equal(row.values.upload_speed, 0);
      assert.equal(row.values.download_speed, 0);
      assert.equal(row.values.upload_total, expected.upload_total);
      assert.equal(row.values.download_total, expected.download_total);
    }
    assertReconciled(rows);
    const reset = { running: false, ...zero(), traffic_breakdown: { source, proxy: zero(), direct: zero(), unclassified: zero() } };
    for (const row of trafficStatsRows(reset)) assert.deepEqual(row.values, zero());
  });

  test(`${source}：未分类任一字段非零均展示，不限于下载累计`, () => {
    for (const key of fields) {
      const unclassified = { ...zero(), [key]: 64 };
      const snapshot = status(source, unclassified);
      assertReconciled(trafficStatsRows(snapshot));
      for (const running of [true, false]) {
        assert.ok(trafficStatsRows({ ...snapshot, running }).some(row => row.id === "unclassified"), key);
      }
    }
  });
}

test("异常字段降级为缺失，UI 不用其他桶或根字段补齐分类", () => {
  for (const invalid of [NaN, Infinity, -1, null, undefined, "1024"]) {
    const snapshot = status();
    snapshot.traffic_breakdown.proxy.upload_total = invalid;
    const rows = trafficStatsRows(snapshot);
    assert.equal(rows[0].values.upload_total, null);
    assert.equal(rows.at(-1).values.upload_total, snapshot.upload_total);
  }
  const snapshot = status();
  delete snapshot.traffic_breakdown.proxy;
  assert.deepEqual(trafficStatsRows(snapshot)[0].values, unknown());
});

test("B/s 与字节单位独立，支持零、小数和大累计", () => {
  for (const invalid of [null, NaN, Infinity, -1, undefined]) assert.equal(formatTrafficValue(invalid), "—");
  assert.equal(formatTrafficValue(0, true), "0 B/s");
  assert.equal(formatTrafficValue(0), "0 B");
  assert.equal(formatTrafficValue(12.4, true), "12 B/s");
  assert.equal(formatTrafficValue(1024, true), "1.0 KB/s");
  assert.equal(formatTrafficValue(1.5 * 1024 ** 3), "1.5 GB");
  assert.equal(formatTrafficValue(1024 ** 4, true), "1.0 TB/s");
  assert.equal(formatTrafficValue(2 ** 64), "16.0 EB");
});

test("中英文来源说明明确三桶对账、连接漏采与出站未知 tag", () => {
  for (const locale of ["zh", "en"]) {
    for (const source of ["Connections", "Outbound"]) {
      const hint = translate(locale, `dashboard.traffic${source}Hint`);
      assert.match(hint, locale === "zh" ? /代理 \+ 直连 \+ 未分类 = 合计/ : /proxy \+ direct \+ unclassified/);
      assert.match(hint, locale === "zh" ? /UI 只展示后端值，不推导分类/ : /without inferring categories/);
    }
    assert.match(translate(locale, "dashboard.trafficConnectionsHint"), locale === "zh" ? /漏采的短连接与未知出站/ : /missed short-lived connections and unknown outbounds/);
    assert.match(translate(locale, "dashboard.trafficOutboundHint"), /tag/);
  }
});

test("Dashboard 静态接线：实际引用组件并传入快照及流量导航", () => {
  const dashboard = readFileSync(new URL("../src/pages/DashboardPage.tsx", import.meta.url), "utf8");
  assert.match(dashboard, /import\s*\{\s*TrafficStatsCard\s*\}\s*from\s*["']\.\.\/components\/TrafficStatsCard["']/);
  assert.match(dashboard, /<TrafficStatsCard\s+status=\{proxy\}\s+onOpen=\{onGoTraffic\}\s*\/>/);
  assert.doesNotMatch(dashboard, /instrument-traffic-cols|fmtSpeed\(/, "删除旧合计卡，避免重复展示");
  const css = readFileSync(new URL("../src/App.css", import.meta.url), "utf8");
  assert.ok((css.match(/--border-subtle\s*:/g) ?? []).length >= 2, "深浅主题均定义分隔线变量");
  assert.match(css, /\.instrument-grid\.instrument-grid-6\s*\{[^}]*grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\)[^}]*grid-auto-rows:\s*minmax\(9\.5rem,\s*auto\)/);
});

test("浏览器脚本在非 CI 环境先行拒绝，不导入或启动浏览器", () => {
  const result = spawnSync(process.execPath, [fileURLToPath(new URL("./test-traffic-stats-browser.mjs", import.meta.url))], {
    env: { ...process.env, CI: "false", PLAYWRIGHT_MODULE: "" },
    encoding: "utf8",
    timeout: 10_000,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /浏览器回归仅允许在 CI 运行/);
  assert.doesNotMatch(result.stderr, /ERR_MODULE_NOT_FOUND|chromium\.launch/);
});

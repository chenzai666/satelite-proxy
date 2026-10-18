import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const browser = await chromium.launch({ headless: true });
const errorsByPrefix = {
  timeout: "超时", unsupported: "不支持", config: "配置错误", tls: "TLS 错误",
  connection: "连接失败", http: "HTTP 错误", core: "内核错误", missing_core: "缺少内核",
};
async function expectCell(page, id, text, title = null) {
  await page.waitForFunction(({ id, text, title }) => {
    const cell = document.querySelector(`[data-node-id="${id}"] .lat`);
    return cell?.textContent === text && cell.getAttribute("title") === title;
  }, { id, text, title });
}
async function finishRun(page) {
  await page.evaluate(() => window.xrayFixture.finish?.());
  await page.waitForFunction(() => !document.querySelector('[data-node-id="b"] .node-latency-action').disabled);
}
try {
  for (const query of ["", "?running", "?grid&grouped", "?grid&grouped&running", "?simple", "?simple&running"]) {
    const page = await browser.newPage({ viewport: { width: query.includes("simple") ? 420 : 1280, height: 900 } });
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:1440/scripts/ui-smoke/xray.html${query}`);
    const row = page.locator('[data-node-id="b"]');
    await row.waitFor();
    // Use the existing per-node latency action, independent of translated labels.
    await row.locator(".node-latency-action").click();
    await page.waitForFunction(() => window.xrayFixture.runs.length === 1);
    assert.deepEqual(await page.evaluate(() => window.xrayFixture.runs[0]), ["b"]);
    await page.waitForFunction(() => [...document.querySelectorAll('[data-node-id="b"] .lat')].some(e => e.textContent.includes("123ms")));
    const all = page.getByRole("button", { name: /测真实延迟/ }).first();
    await page.waitForFunction(() => !document.querySelector('[data-node-id="b"] .node-latency-action').disabled);
    assert.equal(await all.isDisabled(), false);
    await all.click();
    await page.waitForFunction(() => window.xrayFixture.runs.length === 2);
    assert.deepEqual(new Set(await page.evaluate(() => window.xrayFixture.runs[1])), new Set(["a", "b"]));
    await expectCell(page, "a", "123ms");
    await page.waitForFunction(() => !document.querySelector('[data-node-id="b"] .node-latency-action').disabled);

    for (const method of ["xray_http", "singbox_http", "mihomo_http"]) {
      for (const [prefix, label] of Object.entries(errorsByPrefix)) {
        const error = `${prefix}: ${method} 安全诊断`;
        await page.evaluate(({ method, error }) => {
          Object.assign(window.xrayFixture, { method, nextError: error, holdCompletion: true });
        }, { method, error });
        await row.locator(".node-latency-action").click();
        // invoke 保持未完成：此处只观察逐节点 Channel 更新，刷新不会遮盖流式缺陷。
        await expectCell(page, "b", label, error);
        await finishRun(page);
        await expectCell(page, "b", label, error);
      }
      // 通过真实页面重载从 list_all_nodes/list_nodes_page 恢复持久错误。
      await page.reload();
      const persisted = `missing_core: ${method} 安全诊断`;
      await expectCell(page, "b", "缺少内核", persisted);

      // 后台 TCP 事件不能覆盖持久的真延迟失败，旧 HTTP 事件也不能回滚结果。
      await page.evaluate(async () => {
        const fixture = window.xrayFixture;
        await fixture.emitChange({ id: "b", name: "Node b", latency_ms: 1, latency_at: Date.now() / 1000 + 100,
          method: "tcp", latency_error: null }, false);
        await fixture.emitChange({ id: "b", name: "Node b", latency_ms: null, latency_at: 1,
          method: "xray_http", latency_error: "tls: 迟到旧错误" }, false);
      });
      await expectCell(page, "b", "缺少内核", persisted);

      // 专业 Ping 的 stream 也必须保留旧真延迟失败；批量刷新后继续保留。
      if (!query.includes("simple")) {
        await page.evaluate(() => { window.xrayFixture.holdCompletion = true; });
        await page.getByRole("button", { name: "Ping 测试", exact: true }).click();
        await page.waitForFunction(() => window.xrayFixture.finish !== null);
        await expectCell(page, "b", "缺少内核", persisted);
        await finishRun(page);
        await expectCell(page, "b", "缺少内核", persisted);
      }

      const error = `config: ${method} 批量安全诊断`;
      await page.evaluate(({ method, error }) => {
        Object.assign(window.xrayFixture, { method, nextError: error, holdCompletion: true });
      }, { method, error });
      await all.click();
      for (const id of ["a", "b"]) await expectCell(page, id, "配置错误", error);
      await finishRun(page);

      // 单点重测成功只清理该节点，其他节点的批量错误保留。
      await page.evaluate(() => { window.xrayFixture.nextError = null; });
      await row.locator(".node-latency-action").click();
      await expectCell(page, "b", "123ms");
      await expectCell(page, "a", "配置错误", error);
      await finishRun(page);
      await page.reload();
      await expectCell(page, "b", "123ms");
      await expectCell(page, "a", "配置错误", error);
    }

    // 接收持久后台真延迟错误并在刷新后恢复，再由成功事件清理错误。
    await page.evaluate(() => window.xrayFixture.emitChange({ id: "b", name: "Node b", latency_ms: null,
      latency_at: Date.now() / 1000 + 500, method: "singbox_http", latency_error: "connection: 后台连接失败" }));
    await expectCell(page, "b", "连接失败", "connection: 后台连接失败");
    await page.reload();
    await expectCell(page, "b", "连接失败", "connection: 后台连接失败");
    await page.evaluate(() => window.xrayFixture.emitChange({ id: "b", name: "Node b", latency_ms: 55,
      latency_at: Date.now() / 1000 + 600, method: "mihomo_http", latency_error: null }));
    await expectCell(page, "b", "55ms");
    await page.evaluate(() => window.xrayFixture.emitChange({ id: "b", name: "Node b", latency_ms: null,
      latency_at: Date.now() / 1000 + 700, method: "tcp", latency_error: "timeout: TCP 超时" }, false));
    await expectCell(page, "b", "55ms");

    if (query.includes("grouped")) {
      await page.getByRole("button", { name: "测真实延迟——仅该分组", exact: true }).click();
      await expectCell(page, "a", "123ms");
      await expectCell(page, "b", "123ms");
      assert.deepEqual(new Set(await page.evaluate(() => window.xrayFixture.runs.at(-1))), new Set(["a", "b"]));
    }
    assert.equal(await page.evaluate(() => window.xrayFixture.switches), 0);
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log("真实延迟回归通过：专业列表/卡片/简洁、主代理开启/停止、三种 HTTP 方法、八类安全错误、stream/事件/重载、单点/分组/批量及 TCP 缓存保护");
} finally { await browser.close(); }

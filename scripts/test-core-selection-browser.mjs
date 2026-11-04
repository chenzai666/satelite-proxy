import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

if (process.env.CI !== "true") throw new Error("此浏览器测试仅允许在 CI 运行");
if (!process.env.PLAYWRIGHT_MODULE) throw new Error("缺少 PLAYWRIGHT_MODULE");
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const browser = await chromium.launch({ headless: true });
let page;
try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: "block" });
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (["http:", "https:"].includes(url.protocol) && !["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)) return route.abort();
    return route.continue();
  });
  await context.routeWebSocket("**/*", ws => {
    const url = new URL(ws.url());
    if (["127.0.0.1", "localhost", "[::1]"].includes(url.hostname)) ws.connectToServer();
    else ws.close();
  });
  page = await context.newPage();
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  const base = "http://127.0.0.1:1440/scripts/ui-smoke/core-selection.html";
  await page.goto(base);
  await page.getByRole("button", { name: "内核设置", exact: true }).click();
  const vless = page.getByRole("button", { name: "协议默认内核 VLESS" });
  const tuic = page.getByRole("button", { name: "协议默认内核 TUIC" });
  await vless.waitFor();
  await page.evaluate(() => { window.coreFixture.hold = true; });
  await vless.click();
  await page.getByRole("option", { name: "Xray", exact: true }).click();
  await page.waitForFunction(() => window.coreFixture.release !== null);
  assert.equal(await tuic.isDisabled(), true, "写入期间必须阻止另一协议覆盖待保存数组");
  await page.evaluate(() => window.coreFixture.release());
  await tuic.waitFor({ state: "visible" });
  await page.waitForFunction(() => !document.querySelector('button[aria-label="协议默认内核 TUIC"]').disabled);
  await tuic.click();
  await page.getByRole("option", { name: "sing-box", exact: true }).click();
  await page.waitForFunction(() => window.coreFixture.settings.protocol_core_defaults.length === 2);
  assert.deepEqual(await page.evaluate(() => window.coreFixture.settings.protocol_core_defaults), [{ protocol: "vless", core: "xray" }, { protocol: "tuic", core: "singbox" }]);
  assert.deepEqual(await page.evaluate(() => window.coreFixture.writes.map(w => w.protocolCoreDefaults).filter(Boolean)), [[{ protocol: "vless", core: "xray" }], [{ protocol: "vless", core: "xray" }, { protocol: "tuic", core: "singbox" }]]);
  assert.equal(await page.evaluate(() => window.coreFixture.settings.multi_core_enabled), false);
  await page.screenshot({ path: "test-results/core-selection-defaults.png" });
  await page.reload();
  await page.getByRole("button", { name: "内核设置", exact: true }).click();
  await vless.waitFor();
  assert.match(await vless.innerText(), /Xray/);
  assert.match(await tuic.innerText(), /sing-box/);
  await page.goto(`${base}?mode=draft`);
  await page.getByRole("button", { name: "此节点的内核" }).click();
  await page.getByRole("option", { name: "Mihomo", exact: true }).click();
  assert.match(await page.getByRole("button", { name: "此节点的内核" }).innerText(), /Mihomo/);
  await page.goto(`${base}?mode=edit`);
  const core = page.getByRole("button", { name: "此节点的内核" });
  await core.waitFor();
  await core.click();
  await page.getByRole("option", { name: "Xray", exact: true }).click();
  const readsBeforeSave = await page.evaluate(() => window.coreFixture.settingsReads);
  const before = await page.evaluate(() => structuredClone(window.coreFixture.draft));
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.coreFixture.settings.node_core_overrides.fixture === "xray");
  await page.getByText("节点已保存；内核正在运行时会自动重载。").waitFor();
  assert.ok(await page.evaluate(() => window.coreFixture.settingsReads) > readsBeforeSave, "保存后刷新设置缓存");
  const submitted = await page.evaluate(() => window.coreFixture.writes.find(w => w.draft)?.draft);
  assert.deepEqual(submitted, { ...before, preferredCore: "xray" }, "仅修改偏好时保留全部草稿参数及 camelCase 字段");
  await page.screenshot({ path: "test-results/core-selection-node.png" });
  await page.reload();
  await core.waitFor();
  assert.match(await core.innerText(), /Xray/);
  assert.equal(await page.evaluate(() => window.coreFixture.settings.node_core_overrides.fixture), "xray");
  await core.click();
  await page.getByRole("option", { name: "自动 / 跟随协议默认", exact: true }).click();
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await page.waitForFunction(() => window.coreFixture.draft.preferredCore === "auto" && window.coreFixture.writes.length > 0);
  assert.equal(await page.evaluate(() => window.coreFixture.settings.node_core_overrides.fixture), undefined);
  await page.reload();
  await core.waitFor();
  assert.match(await core.innerText(), /自动/);

  for (const mode of ["pro", "simple"]) {
    await page.evaluate(() => {
      sessionStorage.setItem("core-selection-settings", JSON.stringify({ locale: "zh", core_type: "singbox", runtime_source: "generated", current_node_id: null, auto_select: "off" }));
      sessionStorage.setItem("core-selection-status", JSON.stringify({ running: true, core_type: "singbox", runtime_source: "generated", outbound_mode: "rule" }));
    });
    await page.goto(`${base}?mode=${mode}`);
    const select = page.getByRole("button", { name: "切换到 Fixture" });
    await select.waitFor();
    await select.click();
    await page.getByText("已切换为手动模式，正在重启内核…").waitFor();
    await page.waitForFunction(() => window.coreFixture.settings.core_type === "mihomo");
    assert.equal(await select.isDisabled(), true);
    await page.evaluate(() => window.coreFixture.finish("mihomo"));
    await page.getByRole("button", { name: "节点管理", exact: true }).click();
    const managed = page.getByRole("button", { name: "策略组选中状态：Fixture" });
    await managed.waitFor();
    assert.equal(await managed.isDisabled(), true, `${mode} 的 Mihomo 节点管理禁止直选`);
    assert.deepEqual(await page.evaluate(() => window.coreFixture.unexpected), []);

    await page.evaluate(() => {
      sessionStorage.setItem("core-selection-settings", JSON.stringify({ locale: "zh", core_type: "singbox", runtime_source: "generated", auto_select: "off" }));
      sessionStorage.setItem("core-selection-status", JSON.stringify({ running: true, core_type: "mihomo", runtime_source: "generated", outbound_mode: "rule" }));
    });
    await page.goto(`${base}?mode=${mode}`);
    await page.getByRole("button", { name: "节点管理", exact: true }).waitFor();
    await page.getByRole("button", { name: "节点管理", exact: true }).click();
    assert.equal(await page.getByRole("button", { name: "策略组选中状态：Fixture" }).isDisabled(), true, `${mode} 初始化使用实际运行核心`);
    assert.deepEqual(await page.evaluate(() => window.coreFixture.unexpected), []);

    await page.evaluate(() => {
      sessionStorage.setItem("core-selection-settings", JSON.stringify({ locale: "zh", core_type: "singbox", runtime_source: "singbox:fixture", auto_select: "off" }));
      sessionStorage.setItem("core-selection-status", JSON.stringify({ running: true, core_type: "singbox", runtime_source: "singbox", outbound_mode: "rule" }));
    });
    await page.goto(`${base}?mode=${mode}`);
    const custom = page.getByRole("button", { name: "切换到 Fixture" });
    await custom.waitFor();
    assert.equal(await custom.isDisabled(), true, `${mode} 的自写配置禁止直选`);
    assert.deepEqual(await page.evaluate(() => window.coreFixture.unexpected), []);
  }
  assert.deepEqual(errors, []);
  console.log("协议默认和节点偏好真实组件回归通过");
} catch (error) {
  if (page) await page.screenshot({ path: "test-results/core-selection-failure.png" }).catch(() => {});
  throw error;
} finally {
  await browser.close();
}

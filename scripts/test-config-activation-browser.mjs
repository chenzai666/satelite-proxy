import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";

if (process.env.CI !== "true") throw new Error("此浏览器测试仅允许在 CI 运行");
if (!process.env.PLAYWRIGHT_MODULE) throw new Error("缺少 PLAYWRIGHT_MODULE");
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
await mkdir("test-results", { recursive: true });
const browser = await chromium.launch({ headless: true });
const errors = [];
let page;
try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 1000 }, serviceWorkers: "block" });
  const local = url => ["127.0.0.1", "localhost", "[::1]"].includes(new URL(url).hostname);
  await context.route("**/*", route => {
    const url = route.request().url();
    if (/^https?:/.test(url) && !local(url)) return route.abort();
    return route.continue();
  });
  await context.routeWebSocket("**/*", ws => {
    if (local(ws.url())) ws.connectToServer();
    else ws.close();
  });
  page = await context.newPage();
  page.on("pageerror", error => errors.push(error.stack ?? error.message));
  const base = "http://127.0.0.1:1440/scripts/ui-smoke/config-activation.html";
  const card = id => page.getByRole("button", { name: `Fixture ${id}`, exact: true });
  const mixSwitch = () => page.getByRole("switch", { name: "Mix", exact: true });
  const snapshot = () => page.evaluate(() => {
    const s = window.configActivationFixture;
    return { settings: s.settings, profiles: s.profiles, activations: s.activations,
      settingsReads: s.settingsReads, listReads: s.listReads, mixWrites: s.mixWrites,
      unexpected: s.unexpected, commands: s.commands.map(entry => entry.command) };
  });
  const flags = s => s.profiles.filter(item => item.source_kind !== "custom").map(item => [item.id, item.enabled]);
  async function idle() {
    await page.waitForFunction(() => {
      const cards = [...document.querySelectorAll(".sub-card")];
      return cards.length === 5 && cards.every(c => c.getAttribute("aria-disabled") === "false");
    });
  }
  async function active(ids) {
    const names = ids.map(id => `Fixture ${id}`).sort();
    await page.waitForFunction(expected => {
      const actual = [...document.querySelectorAll('.sub-card[aria-pressed="true"]')]
        .map(c => c.getAttribute("aria-label")).sort();
      return JSON.stringify(actual) === JSON.stringify(expected);
    }, names);
    assert.deepEqual(await page.locator(".sub-card.enabled").evaluateAll(cards => cards.map(c => c.getAttribute("aria-label")).sort()), names,
      "高亮类和无障碍按下状态必须一致");
  }
  async function cleanFixture() {
    assert.deepEqual((await snapshot()).unexpected, [], "未声明的 IPC 必须导致失败");
    assert.deepEqual(errors, [], "真实页面不应出现未捕获异常");
  }
  async function fresh(query = {}) {
    await cleanFixture();
    await page.evaluate(() => sessionStorage.removeItem("config-activation-fixture"));
    await page.goto(`${base}?${new URLSearchParams(query)}`);
    await idle();
  }
  async function closeError() {
    await page.locator(".error-modal").getByRole("button", { name: "关闭", exact: true }).last().click();
  }
  await page.goto(base);
  await idle();
  await active(["generated-a"]);

  // 三种自定义内核都走统一激活命令，保留普通配置勾选，重载后来源仍有效。
  for (const core of ["singbox", "mihomo", "xray"]) {
    const id = `custom-${core}`;
    const before = await snapshot();
    await card(id).click();
    await idle();
    await active([id]);
    const saved = await snapshot();
    assert.equal(saved.settings.runtime_source, `singbox:${id}`);
    assert.deepEqual(flags(saved), flags(before));
    assert.equal(saved.settings.mix_mode, false);
    assert.ok(saved.settingsReads > before.settingsReads, "激活成功后刷新 settings");
    assert.equal(saved.activations.at(-1), id);
    assert.equal(await mixSwitch().isDisabled(), true);
    assert.equal(await card(id).locator(".sub-type-tag").innerText(), { singbox: "sing-box", mihomo: "mihomo", xray: "Xray" }[core]);
    if (core === "mihomo") await page.screenshot({ path: "test-results/config-activation-custom.png" });
    await cleanFixture();
    await page.reload();
    await idle();
    await active([id]);
    await card("generated-a").click();
    await idle();
    await active(["generated-a"]);
    assert.equal((await snapshot()).settings.runtime_source, "generated");
    assert.equal(await mixSwitch().isDisabled(), false);
  }
  await card("generated-b").click();
  await idle();
  await active(["generated-b"]);
  assert.deepEqual(flags(await snapshot()), [["generated-a", false], ["generated-b", true]], "单选模式可从订阅切换到本地配置");

  // Mix 保留开关及全部 generated 勾选；回到已启用普通卡不取消勾选。
  await fresh({ mix: "1" });
  await active(["generated-a", "generated-b"]);
  for (const [custom, generated] of [["custom-mihomo", "generated-a"], ["custom-xray", "generated-b"]]) {
    await card(custom).click();
    await idle();
    await active([custom]);
    assert.equal(await mixSwitch().getAttribute("aria-checked"), "true");
    assert.equal(await mixSwitch().isDisabled(), true);
    assert.deepEqual(flags(await snapshot()), [["generated-a", true], ["generated-b", true]]);
    await card(generated).click();
    await idle();
    await active(["generated-a", "generated-b"]);
    assert.equal((await snapshot()).settings.mix_mode, true);
  }
  assert.deepEqual((await snapshot()).mixWrites, [], "激活卡片不能额外改写 Mix");
  await card("generated-b").click();
  await idle();
  await active(["generated-a"]);
  await card("generated-b").click();
  await idle();
  await active(["generated-a", "generated-b"]);
  await mixSwitch().click();
  await page.waitForFunction(() => window.configActivationFixture.settings.mix_mode === false);
  await active(["generated-a"]);
  assert.deepEqual((await snapshot()).mixWrites, [false], "保留用户主动修改 Mix 的原有路径");
  await fresh({ source: "custom-singbox", mix: "1" });
  await active(["custom-singbox"]);
  await card("generated-b").click();
  await idle();
  await active(["generated-a", "generated-b"]);

  // 同一事件循环的双击和跨卡点击只能产生一次 IPC，锁持续到 getSettings 完成。
  await fresh({ mix: "1" });
  await page.evaluate(() => {
    window.configActivationFixture.holdActivation = true;
    const cards = document.querySelectorAll(".sub-card");
    cards[2].click(); cards[2].click(); cards[4].click();
  });
  await page.waitForFunction(() => window.configActivationFixture.releaseActivation !== null);
  assert.deepEqual((await snapshot()).activations, ["custom-singbox"]);
  await active(["generated-a", "generated-b"]);
  assert.equal(await card("custom-singbox").getAttribute("aria-busy"), "true");
  assert.equal(await page.getByRole("button", { name: "添加配置", exact: true }).isDisabled(), true);
  assert.equal(await page.getByRole("button", { name: "更新全部", exact: true }).isDisabled(), true);
  assert.equal(await mixSwitch().isDisabled(), true);
  assert.equal(await card("custom-xray").getAttribute("aria-disabled"), "true");
  await page.evaluate(() => {
    const s = window.configActivationFixture;
    s.holdSettings = true;
    s.releaseActivation();
  });
  await page.waitForFunction(() => window.configActivationFixture.releaseSettings !== null);
  await page.evaluate(() => {
    const c = document.querySelector('.sub-card[aria-label="Fixture custom-xray"]');
    c.click(); c.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  });
  assert.deepEqual((await snapshot()).activations, ["custom-singbox"], "设置刷新完成前继续阻止激活");
  await active(["generated-a", "generated-b"]);
  await page.evaluate(() => window.configActivationFixture.releaseSettings());
  await idle();
  await active(["custom-singbox"]);
  await card("custom-xray").click();
  await idle();
  await active(["custom-xray"]);
  assert.deepEqual((await snapshot()).activations, ["custom-singbox", "custom-xray"]);

  // 激活前失败保留原来源；保存后重启失败显示已保存来源，并保留原始错误。
  await fresh();
  const beforeFailure = await snapshot();
  await page.evaluate(() => { window.configActivationFixture.failActivation = "模拟激活失败"; });
  await card("custom-mihomo").click();
  await page.getByText("模拟激活失败", { exact: true }).waitFor();
  await idle();
  await active(["generated-a"]);
  assert.equal((await snapshot()).settings.runtime_source, "generated");
  assert.ok((await snapshot()).listReads > beforeFailure.listReads, "失败后重新读取保存状态");
  await closeError();
  await page.evaluate(() => { window.configActivationFixture.failActivation = null; });
  await card("custom-mihomo").click();
  await idle();
  await active(["custom-mihomo"]);
  await fresh();
  await page.evaluate(() => { window.configActivationFixture.failAfterSave = "模拟内核重启失败"; });
  await card("custom-xray").click();
  await page.getByText("模拟内核重启失败", { exact: true }).waitFor();
  await idle();
  await active(["custom-xray"]);
  assert.equal((await snapshot()).settings.runtime_source, "singbox:custom-xray");
  await closeError();
  await page.evaluate(() => { window.configActivationFixture.failAfterSave = null; });
  await card("generated-a").click();
  await idle();
  await active(["generated-a"]);
  await fresh();
  await page.evaluate(() => {
    const s = window.configActivationFixture;
    s.failAfterSave = "原始激活错误";
    s.failList = true;
  });
  await card("custom-singbox").click();
  await page.getByText("原始激活错误", { exact: true }).waitFor();
  await idle();
  assert.equal(await page.locator(".error-modal-text").innerText(), "原始激活错误", "刷新失败不得覆盖激活错误");
  await fresh();
  await page.evaluate(() => { window.configActivationFixture.failSettings = true; });
  await card("custom-singbox").click();
  await page.getByText("模拟设置刷新失败", { exact: true }).waitFor();
  await idle();
  await closeError();
  await page.evaluate(() => { window.configActivationFixture.failSettings = false; });
  await card("custom-singbox").click();
  await idle();
  await active(["custom-singbox"]);

  // Enter/Space 仅在卡片本身处理，子元素、菜单鼠标/键盘操作不能激活卡片。
  await fresh();
  await card("custom-mihomo").focus();
  await page.keyboard.press("Enter");
  await idle();
  await active(["custom-mihomo"]);
  await card("custom-xray").focus();
  await page.keyboard.press("Space");
  await idle();
  await active(["custom-xray"]);
  const beforeMenu = (await snapshot()).activations;
  await card("custom-singbox").locator("h3").dispatchEvent("keydown", { key: "Enter", bubbles: true });
  await card("custom-singbox").dispatchEvent("keydown", { key: "Enter", repeat: true, bubbles: true });
  const menu = card("custom-singbox").getByRole("button", { name: "操作", exact: true });
  await menu.click();
  await page.getByRole("menu").waitFor();
  await menu.press("Enter");
  await page.getByRole("menu").waitFor({ state: "hidden" });
  await menu.press("Space");
  await page.getByRole("menuitem", { name: "查看配置", exact: true }).press("Enter");
  await page.locator(".raw-config-modal pre").waitFor();
  assert.equal(await page.locator(".raw-config-modal pre").innerText(), '{"outbounds":[{"type":"direct","tag":"direct"}]}');
  await page.locator(".raw-config-modal").getByRole("button", { name: "关闭", exact: true }).last().click();
  await menu.click();
  await page.getByRole("menuitem", { name: "编辑", exact: true }).press("Enter");
  await page.locator("textarea.config-paste").waitFor();
  assert.equal(await page.locator("textarea.config-paste").inputValue(), '{"outbounds":[{"type":"direct","tag":"direct"}]}', "自定义编辑仍使用完整原文");
  await page.evaluate(() => document.querySelector('.sub-card[aria-label="Fixture generated-a"]').click());
  await page.getByRole("button", { name: "取消", exact: true }).click();
  await idle();
  assert.deepEqual((await snapshot()).activations, beforeMenu);
  await page.screenshot({ path: "test-results/config-activation.png" });
  await cleanFixture();
  console.log("配置激活真实页面回归通过：三核自定义、回普通、Mix、失败恢复、同步防重入、键盘及菜单隔离");
} catch (error) {
  if (page) {
    // 仅记录本夹具的合成状态，不采集真实用户配置。
    console.error("配置激活夹具失败状态", await page.evaluate(() => ({
      unexpected: window.configActivationFixture?.unexpected,
      activations: window.configActivationFixture?.activations,
      source: window.configActivationFixture?.settings.runtime_source,
      error: document.querySelector(".error-modal-text")?.textContent,
    })).catch(() => null));
    await page.screenshot({ path: "test-results/config-activation-failure.png" }).catch(() => {});
  }
  throw error;
} finally {
  await browser.close();
}

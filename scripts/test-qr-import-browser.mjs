import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const browser = await chromium.launch({ headless: true });
let page;
const errors = [];
const consoleErrors = [];
try {
  page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.on("pageerror", e => errors.push(e.message));
  page.on("console", message => { if (message.type() === "error") consoleErrors.push(message.text()); });
  await page.goto("http://127.0.0.1:1440/scripts/ui-smoke/qr.html");
  const scan = page.getByRole("button", { name: "扫描二维码", exact: true });
  await scan.waitFor();
  await page.evaluate(() => {
    window.qrEvents = [];
    for (const type of ["pointerdown", "pointerup", "click", "scroll"]) {
      window.addEventListener(type, event => {
        if (window.qrEvents.length >= 25) return;
        const target = event.target;
        window.qrEvents.push({ type, target: target instanceof Element ? target.tagName : "window",
          text: target instanceof Element ? target.textContent?.slice(0, 50) : "",
          menu: target instanceof Element && !!target.closest("[data-ruleset-menu]") });
      }, true);
    }
  });
  const uri = "vless://11111111-1111-4111-8111-111111111111@example.test:443?security=tls#test";
  await page.evaluate(text => window.qrFixture.setQr(text), uri);
  await scan.click();
  await page.getByRole("menuitem", { name: /扫描屏幕上的二维码/ }).click();
  await page.getByRole("dialog").waitFor();
  assert.equal(await page.locator("textarea.config-paste").inputValue(), uri);
  assert.equal(await page.evaluate(() => window.qrFixture.writes.length), 0, "recognition must not import automatically");
  await page.keyboard.press("Control+s");
  assert.equal(await page.evaluate(() => window.qrFixture.capture), 1, "dialog must block shortcut");
  await page.getByRole("button", { name: "取消", exact: true }).click();
  await page.evaluate(() => window.qrFixture.setQr("https://example.test/sub?token=fixture"));
  await scan.click();
  await page.getByRole("menuitem", { name: "扫描图片中的二维码", exact: true }).click();
  await page.getByRole("dialog").waitFor();
  assert.equal(await page.locator('input[placeholder="https://…"]').inputValue(), "https://example.test/sub?token=fixture");
  assert.equal(await page.evaluate(() => window.qrFixture.read), 1);
  assert.equal(await page.evaluate(() => window.qrFixture.writes.length), 0);
  await page.getByRole("button", { name: "取消", exact: true }).click();
  await page.evaluate(() => { window.qrFixture.cancel = true; });
  await scan.click();
  await page.getByRole("menuitem", { name: "扫描图片中的二维码", exact: true }).click();
  await page.waitForFunction(() => window.qrFixture.dialog === 2);
  await scan.waitFor({ state: "visible" });
  assert.equal(await page.evaluate(() => window.qrFixture.read), 1);
  await page.evaluate(() => { window.qrFixture.cancel = false; });
  await page.evaluate(() => window.qrFixture.setQr("javascript:alert(1)"));
  await page.keyboard.press("Control+s");
  await page.getByText("二维码内容不是支持的节点分享链接或 HTTP/HTTPS 订阅链接。", { exact: true }).waitFor();
  await page.getByRole("button", { name: "关闭", exact: true }).last().click();
  await page.evaluate(() => { window.qrFixture.fail = true; });
  await page.keyboard.press("Control+s");
  await page.getByText("模拟屏幕捕获失败", { exact: true }).waitFor();
  await page.getByRole("button", { name: "关闭", exact: true }).last().click();
  await page.evaluate(text => { window.qrFixture.fail = false; return window.qrFixture.setQr(text); }, uri);
  await page.keyboard.press("Control+s");
  await page.getByRole("dialog").waitFor();
  await page.getByRole("button", { name: "取消", exact: true }).click();
  assert.equal(await page.evaluate(() => window.qrFixture.writes.length), 0);
  // A late capture must not replace a form the user has already opened.
  await page.evaluate(() => { window.qrFixture.hold = true; });
  await page.keyboard.press("Control+s");
  await page.waitForFunction(() => window.qrFixture.release !== null);
  await page.getByRole("button", { name: "添加配置", exact: true }).first().click();
  await page.evaluate(() => window.qrFixture.release?.());
  await page.getByRole("button", { name: "扫描二维码", exact: true }).waitFor();
  assert.equal(await page.locator("textarea.config-paste").count(), 0);
  assert.equal(await page.locator('input[placeholder="https://…"]').inputValue(), "");
  await page.getByRole("button", { name: "取消", exact: true }).click();
  await page.evaluate(() => { window.qrFixture.hold = false; });
  await page.screenshot({ path: "test-results/qr-import.png" });
  await page.goto("http://127.0.0.1:1440/scripts/ui-smoke/qr.html?draft");
  for (const protocol of ["VLESS", "VMess", "TUIC"]) {
    await page.getByRole("button", { name: "协议", exact: true }).click();
    await page.getByRole("option", { name: protocol, exact: true }).click();
    const uuid = page.getByRole("textbox", { name: "UUID", exact: true });
    await uuid.fill("existing-server-uuid");
    assert.equal(await uuid.inputValue(), "existing-server-uuid");
    await page.getByRole("button", { name: "生成", exact: true }).click();
    assert.match(await uuid.inputValue(), /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    assert.equal(await page.evaluate(() => window.qrFixture.submits), 0, "Generate must not submit form");
  }
  await page.getByRole("button", { name: "toggle-disabled" }).click();
  assert.equal(await page.getByRole("button", { name: "生成", exact: true }).isDisabled(), true);
  assert.deepEqual(errors, []);
  await page.close();
  console.log("二维码屏幕/图片/快捷键/取消/恢复/安全预填及三协议 UUID 生成浏览器回归通过");
} catch (error) {
  // This page contains synthetic fixtures only, never user subscriptions or screenshots.
  if (page) {
    console.error("扫码夹具失败诊断", { errors, consoleErrors,
      state: await page.evaluate(() => ({ capture: window.qrFixture.capture, read: window.qrFixture.read,
        dialog: window.qrFixture.dialog, writes: window.qrFixture.writes.length })),
      events: await page.evaluate(() => window.qrEvents),
      body: await page.locator("body").innerText() });
    await page.screenshot({ path: "test-results/qr-import-failure.png" });
  }
  throw error;
} finally { await browser.close(); }

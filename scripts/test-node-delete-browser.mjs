import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";

const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const browser = await chromium.launch({ headless: true });
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("http://127.0.0.1:1440/scripts/ui-smoke/nodes.html");
  const row = page.locator('[data-node-id="a"]');
  await row.waitFor();
  await page.keyboard.press("Delete");
  assert.equal(await page.evaluate(() => window.nodeFixture.confirms), 0);
  await row.locator(".node-list-name").click();
  await page.locator("input.search").focus();
  await page.keyboard.press("Delete");
  assert.equal(await page.evaluate(() => window.nodeFixture.confirms), 0);
  await row.locator(".node-list-name").click();
  // 取消必须保留节点及选择，确认前不调用后端。
  await page.keyboard.press("Delete");
  await page.waitForFunction(() => window.nodeFixture.confirms === 1);
  assert.deepEqual(await page.evaluate(() => window.nodeFixture.deleted), []);
  assert.equal(await row.count(), 1);
  await page.evaluate(() => { window.nodeFixture.hold = true; });
  await page.keyboard.press("Delete");
  await page.waitForFunction(() => window.nodeFixture.confirms === 2);
  await page.keyboard.press("Delete");
  await page.evaluate(() => window.dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", repeat: true })));
  assert.equal(await page.evaluate(() => window.nodeFixture.confirms), 2);
  assert.deepEqual(await page.evaluate(() => window.nodeFixture.deleted), []);
  await page.evaluate(() => { window.nodeFixture.release("Ok"); window.nodeFixture.hold = false; window.nodeFixture.answer = "Ok"; });
  await row.waitFor({ state: "detached" });
  assert.deepEqual(await page.evaluate(() => window.nodeFixture.deleted), [["a"]]);
  // Ctrl+A + Delete 复用相同批量确认与删除路径。
  await page.locator('[data-node-id="b"] .node-list-name').click();
  await page.keyboard.press("Control+a");
  await page.waitForFunction(() => document.querySelectorAll(".row-selected").length === 2);
  await page.keyboard.press("Delete");
  await page.locator('[data-node-id="b"]').waitFor({ state: "detached" });
  assert.deepEqual(await page.evaluate(() => window.nodeFixture.deleted), [["a"], ["b", "c"]]);
  assert.deepEqual(errors, []);
  console.log("节点页浏览器回归通过：无选择/输入保护、取消、确认前防重入、单删及 Ctrl+A 批删");
} finally { await browser.close(); }

import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const browser = await chromium.launch({ headless: true });
try {
  for (const query of ["", "?running", "?simple", "?simple&running"]) {
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
    assert.equal(await all.isDisabled(), false);
    await all.click();
    await page.waitForFunction(() => window.xrayFixture.runs.length === 2);
    assert.deepEqual(new Set(await page.evaluate(() => window.xrayFixture.runs[1])), new Set(["a", "b"]));
    assert.equal(await page.evaluate(() => window.xrayFixture.switches), 0);
    assert.deepEqual(errors, []);
    await page.close();
  }
  console.log("Xray 真实延迟入口通过：专业/简洁、代理开启/停止、单点/批量与不切换主代理");
} finally { await browser.close(); }

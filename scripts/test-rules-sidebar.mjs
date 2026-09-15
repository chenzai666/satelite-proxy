import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";

// 只在隔离的 CI 浏览器加载测试夹具，不接入用户配置或代理内核。
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const browser = await chromium.launch({ headless: true, ignoreDefaultArgs: ["--hide-scrollbars"] });
await mkdir("test-results", { recursive: true });
try {
  for (const zoom of [1, 1.5, 2]) {
    const page = await browser.newPage({ viewport: { width: 960 * zoom, height: 720 * zoom } });
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto(`http://127.0.0.1:1440/scripts/ui-smoke/index.html?zoom=${zoom}`);
    const scroll = page.locator("[data-ruleset-scroll]");
    await scroll.waitFor();
    await page.waitForTimeout(250);
    const rightTop = (await page.locator(".rules-main").boundingBox()).y;
    assert.ok(await scroll.evaluate(el => el.scrollHeight > el.clientHeight + 100));
    assert.ok(await scroll.evaluate(el => el.offsetWidth > el.clientWidth), "独立滚动条必须有可见槽位");
    await scroll.evaluate(el => { el.scrollTop = el.scrollHeight; });
    assert.equal((await page.locator(".rules-main").boundingBox()).y, rightTop);
    const sidebar = await page.locator(".rules-route-list").boundingBox();
    assert.ok(sidebar.y + sidebar.height <= 720 * zoom + 2, JSON.stringify(sidebar));
    await page.locator('[data-menu="28"]').click();
    const pop = page.locator(".ruleset-menu-portal");
    await pop.waitFor();
    const menu = await pop.boundingBox();
    assert.ok(menu.y >= 0 && menu.y + menu.height <= 720 * zoom + 2, JSON.stringify(menu));
    await pop.locator(".rule-menu-subhost").hover();
    await pop.locator("[data-sub-action]").click();
    await pop.waitFor({state: "detached"});
    const card = await page.locator('[data-ruleset-id="28"]').boundingBox();
    const area = await scroll.boundingBox();
    const beforeScroll = await scroll.evaluate(el => el.scrollTop);
    await page.mouse.move(card.x + 20 * zoom, card.y + 20 * zoom);
    await page.mouse.down();
    await page.mouse.move(card.x + 25 * zoom, card.y + 30 * zoom, {steps: 5});
    await page.mouse.move(card.x + 25 * zoom, area.y + 5, {steps: 8});
    await page.waitForTimeout(400);
    assert.ok(await scroll.evaluate(el => el.scrollTop) < beforeScroll, "拖拽必须滚动内层列表");
    await page.mouse.up();
    await page.waitForTimeout(250);
    const order = (await page.locator("#order").textContent()).split(",");
    assert.equal(new Set(order).size, 28);
    assert.notEqual(order[27], "28", "拖拽应改变排序");
    assert.equal((await page.locator(".rules-main").boundingBox()).y, rightTop);
    assert.deepEqual(errors, []);
    await page.screenshot({path: `test-results/rules-sidebar-${zoom}.png`});
    console.log(`PASS 规则侧栏 zoom=${zoom}: 独立滚动、菜单子菜单、拖拽排序`);
    await page.close();
  }
} finally { await browser.close(); }

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
    const main = page.locator(".main");
    await scroll.waitFor();
    await page.waitForTimeout(250);
    const rightTop = (await page.locator(".rules-main").boundingBox()).y;
    assert.ok(await scroll.evaluate(el => el.scrollHeight > el.clientHeight + 100));
    assert.ok(await scroll.evaluate(el => el.offsetWidth > el.clientWidth), "独立滚动条必须有可见槽位");
    // The side list must consume wheel input even at its boundaries. The
    // outer page may scroll only after the pointer leaves the side list.
    await main.evaluate(el => { el.scrollTop = 80; });
    await scroll.evaluate(el => { el.scrollTop = 0; });
    const sideBox = await scroll.boundingBox();
    await page.mouse.move(sideBox.x + sideBox.width / 2, sideBox.y + sideBox.height / 2);
    await page.mouse.wheel(0, 260);
    assert.ok(await scroll.evaluate(el => el.scrollTop > 0), "左栏应响应自身滚轮");
    assert.equal(await main.evaluate(el => el.scrollTop), 80, "左栏滚动不能带动页面");
    await scroll.evaluate(el => { el.scrollTop = el.scrollHeight; });
    await page.mouse.wheel(0, 260);
    assert.equal(await main.evaluate(el => el.scrollTop), 80, "左栏到底后也不能把滚动交给页面");
    const innerAtBoundary = await scroll.evaluate(el => el.scrollTop);
    // Hover through Playwright rather than raw coordinates: root zoom changes
    // the CSS-pixel/device-pixel conversion on Windows WebView Chromium.
    const rightContent = page.locator(".rules-main > .card");
    await rightContent.hover({ position: { x: 30, y: 120 } });
    await page.mouse.wheel(0, 180);
    assert.ok(await main.evaluate(el => el.scrollTop) > 80, "鼠标离开左栏后页面应可单独滚动");
    assert.equal(await scroll.evaluate(el => el.scrollTop), innerAtBoundary, "页面滚动不能带动左栏");
    await main.evaluate(el => { el.scrollTop = 0; });
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
    console.log(`PASS 规则侧栏 zoom=${zoom}: 独立滚动输入、菜单子菜单、拖拽排序`);
    await page.close();
  }
} finally { await browser.close(); }

import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";

// 本机仅 node --check；浏览器与 Vite 服务由 CI 工作流负责。
if (process.env.CI !== "true") throw new Error("此浏览器测试仅允许在 CI 运行");
if (!process.env.PLAYWRIGHT_MODULE) throw new Error("缺少 PLAYWRIGHT_MODULE");
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const base = "http://127.0.0.1:1440";
await mkdir("test-results", { recursive: true });
const browser = await chromium.launch({ headless: true });
let context, page;
let currentCase = "setup";
let errors = [], blocked = [];
const words = {
  zh: { stop: "停止代理", start: "启动代理", routing: "路由模式", global: "全局", rule: "规则", state: "运行中" },
  en: { stop: "Stop", start: "Start", routing: "Routing mode", global: "Global", rule: "Rule", state: "Running" },
};
async function clean() {
  assert.deepEqual(await page.evaluate(() => window.dashboardFixture.unexpected), [], "禁止未声明 IPC");
  assert.deepEqual(blocked, [], "夹具不得发起外部网络请求");
  assert.deepEqual(errors, [], `${currentCase} 页面异常`);
}
async function open(width, height, locale, theme, state = "running", hero = "classic") {
  if (page) await clean();
  if (context) await context.close();
  errors = []; blocked = [];
  currentCase = `${width}x${height}-${locale}-${theme}-${state}`;
  context = await browser.newContext({ viewport: { width, height }, serviceWorkers: "block" });
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin === base) return route.continue();
    blocked.push(url.origin);
    return route.abort();
  });
  await context.routeWebSocket("**/*", socket => {
    if (new URL(socket.url()).origin === base.replace("http:", "ws:")) socket.connectToServer();
    else { blocked.push(new URL(socket.url()).origin); socket.close(); }
  });
  page = await context.newPage();
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(`${base}/scripts/ui-smoke/dashboard.html?${new URLSearchParams({ locale, theme, state, hero })}`);
  await page.locator(".dashboard-details-grid > :nth-child(4)").waitFor();
  await page.waitForFunction(({ theme, label }) =>
    document.documentElement.dataset.theme === theme &&
    document.querySelector(".dash-kicker")?.textContent.includes(label) &&
    document.querySelector(".dash-hero-title")?.getAttribute("title")?.length > 0 &&
    !document.querySelector(".dashboard-page .skel"),
  { theme, label: state === "running" || state === "custom" ? words[locale].state : "" });
  await page.evaluate(() => document.fonts.ready);
  // 与 App 相同的 page-enter 会暂时创建 transform 容器；等其退出再测量和截图。
  await page.waitForFunction(() => [...document.querySelectorAll("main > .page-enter")].every(el =>
    el.getAnimations().every(animation => animation.playState === "finished") && getComputedStyle(el).transform === "none"));
}

async function layout() {
  const issues = await page.evaluate(() => {
    const issues = [];
    const root = document.documentElement;
    const dashboard = document.querySelector(".dashboard-page");
    const cards = [...dashboard.querySelectorAll(".dash-hero, .dashboard-rate-card, .dashboard-top-grid > .service-status-card, .dash-side-rail, .dashboard-details-grid > *, .dashboard-bottom-grid > .instrument, .dashboard-bottom-grid > .simple-spark")];
    const describe = el => `${el.tagName}.${el.className}`;
    const outside = (r, box) => r.left < box.left - 1.5 || r.right > box.right + 1.5 || r.top < box.top - 1.5 || r.bottom > box.bottom + 1.5;
    for (const el of [root, document.body, document.querySelector("main"), dashboard, ...cards]) {
      if (el.scrollWidth > el.clientWidth + 1) issues.push(`横向溢出 ${describe(el)}`);
    }
    for (const card of cards) {
      const box = card.getBoundingClientRect();
      if (box.width < 1 || box.height < 1) issues.push(`空卡片 ${describe(card)}`);
      if (card.scrollHeight > card.clientHeight + 1) issues.push(`卡片纵向裁切 ${describe(card)}`);
      for (const other of cards) {
        if (card === other || card.contains(other) || other.contains(card)) continue;
        const b = other.getBoundingClientRect();
        if (box.left < b.right - 1.5 && box.right > b.left + 1.5 && box.top < b.bottom - 1.5 && box.bottom > b.top + 1.5) issues.push(`卡片重叠 ${describe(card)} / ${describe(other)}`);
      }
    }
    // 长名称允许既有缩小/省略及完整 title；统计数值必须完整可见。
    const critical = ".dashboard-rate-card__number, .dashboard-rate-card__unit, .dashboard-rate-card__stats dd, .traffic-stats-number, .traffic-stats-table th";
    for (const el of dashboard.querySelectorAll(critical)) {
      if (!el.textContent.trim()) { issues.push(`统计为空 ${describe(el)}`); continue; }
      if (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1) issues.push(`统计裁切 ${describe(el)}`);
      const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        if (!walker.currentNode.textContent.trim()) continue;
        const range = document.createRange(); range.selectNodeContents(walker.currentNode);
        for (const rect of range.getClientRects()) {
          if (outside(rect, el.getBoundingClientRect())) issues.push(`文字超出统计单元 ${describe(el)}`);
          for (let ancestor = el.parentElement; ancestor && ancestor !== document.querySelector("main"); ancestor = ancestor.parentElement) {
            const style = getComputedStyle(ancestor);
            const box = ancestor.getBoundingClientRect();
            if (style.overflowX !== "visible" && (rect.left < box.left - 1.5 || rect.right > box.right + 1.5)) issues.push(`祖先横向裁字 ${describe(el)}`);
            if (style.overflowY !== "visible" && (rect.top < box.top - 1.5 || rect.bottom > box.bottom + 1.5)) issues.push(`祖先纵向裁字 ${describe(el)}`);
          }
        }
      }
    }
    return [...new Set(issues)];
  });
  assert.deepEqual(issues, [], `${currentCase} 布局/统计文字`);
}
async function structure(locale) {
  const navigation = page.locator(".pro-shell > .side-nav").getByRole("navigation");
  assert.equal(await navigation.count(), 1, "必须使用真实 SideNav 导航");
  assert.equal(await navigation.getByRole("button").count(), 6, "六个页面入口");
  assert.equal(await navigation.locator('[aria-current="page"]').count(), 1);
  assert.equal(await navigation.locator('[aria-current="page"]').getAttribute("aria-label"), locale === "zh" ? "概览" : "Overview");
  assert.equal(await page.locator('.pro-shell > aside[aria-hidden="true"], .pro-shell > .topnav-wrap').count(), 0, "禁止空白占位侧栏和专业模式 TopNav");
  assert.equal(await page.locator(".side-nav-tools .topnav-theme-switch").count(), 1);
  assert.equal(await page.locator(".side-nav-tools [data-ui-mode-menu]").count(), 1);
  assert.equal(await page.locator(".dashboard-page.dashboard-bento").count(), 1);
  assert.equal(await page.locator(".dashboard-heading h1").count(), 1);
  assert.equal(await page.locator(".dashboard-top-grid .dash-hero").count(), 1);
  assert.equal(await page.locator(".dashboard-top-grid .dashboard-rate-card").count(), 2);
  assert.equal(await page.locator(".dash-hero .dash-side-rail").count(), 0, "控制栏独立于状态卡");
  assert.equal(await page.locator(".dash-side-rail").count(), 1);
  assert.equal(await page.locator(".dashboard-details-grid > *").count(), 4);
  assert.equal(await page.locator(".dashboard-power-visual .orbit").count(), 1, "首页动效保留");
  await page.waitForFunction(() => document.querySelector('.ip-organization')?.textContent.includes('Without Truncation'));
  assert.equal(await page.locator('.ip-organization-row .ip-organization').evaluate(el => getComputedStyle(el).textOverflow), "clip");
  assert.equal(await page.locator('.service-status-card').count(), 1);
  assert.equal(await page.locator(".dashboard-bottom-grid .traffic-stats-card").count(), 1);
  assert.equal(await page.locator(".dashboard-bottom-grid .simple-spark").count(), 1);
  for (const id of ["proxy", "direct", "total"]) {
    assert.equal(await page.locator(`[data-traffic-row="${id}"]`).isVisible(), true, `分类 ${id} 可见`);
  }
  assert.equal(await page.locator("main").getByRole("button", { name: words[locale].stop, exact: true }).isEnabled(), true);
  assert.equal(await page.locator("h2.dash-hero-title").getAttribute("title"), await page.evaluate(() => window.dashboardFixture.nodes[0].name));
  await page.waitForFunction(() => [...document.querySelectorAll(".dashboard-details-grid .kv-v")].some(el => el.textContent.includes(window.dashboardFixture.ipv6)));
  const coreKinds = await page.evaluate(() => window.dashboardFixture.commands.filter(c => c.command === "get_core_info").map(c => c.payload.kind));
  assert.deepEqual([...new Set(coreKinds)].sort(), ["mihomo", "singbox", "xray"]);
}

try {
  // 三种尺寸各覆盖中文浅色、英文深色，共六组；只保存两张成功截图。
  for (const [width, height] of [[960, 720], [1280, 900], [1600, 1000]]) {
    for (const [locale, theme] of [["zh", "day"], ["en", "aerospace"]]) {
      await open(width, height, locale, theme);
      await structure(locale);
      await layout();
      if ((width === 960 && locale === "zh") || (width === 1280 && locale === "en")) {
        await page.screenshot({ path: `test-results/dashboard-${width}-${theme}.png` });
      }
    }
  }

  await open(1280, 900, "zh", "aerospace");
  // 等待真实组件的第二轮状态/连接轮询，所有请求仍由内存 mock 响应。
  const reads = await page.evaluate(() => ({ status: window.dashboardFixture.statusReads, connections: window.dashboardFixture.connectionReads }));
  await page.waitForFunction(before => window.dashboardFixture.statusReads > before.status && window.dashboardFixture.connectionReads > before.connections, reads);
  const routing = page.getByRole("group", { name: words.zh.routing, exact: true });
  for (const [mode, label] of [["global", words.zh.global], ["rule", words.zh.rule]]) {
    await routing.getByRole("button", { name: label, exact: true }).click();
    await page.waitForFunction(mode => window.dashboardFixture.status.outbound_mode === mode, mode);
    await page.waitForFunction(label => document.querySelector('.dash-rail-block .glass-seg-btn.active')?.textContent === label, label);
  }
  assert.deepEqual(await page.evaluate(() => window.dashboardFixture.commands.filter(c => c.command === "set_outbound_mode").map(c => c.payload.mode)), ["global", "rule"]);
  await page.getByRole("button", { name: words.zh.stop, exact: true }).click();
  const start = page.getByRole("button", { name: words.zh.start, exact: true });
  await start.waitFor();
  await page.waitForFunction(() => document.querySelector(".dashboard-power")?.disabled === false);
  await page.evaluate(() => { window.dashboardFixture.holdStart = true; });
  await start.click();
  await page.waitForFunction(() => window.dashboardFixture.releaseStart !== null);
  assert.equal(await page.locator(".dashboard-power").isDisabled(), true, "启动过渡禁用按钮");
  await page.evaluate(() => window.dashboardFixture.releaseStart());
  await page.getByRole("button", { name: words.zh.stop, exact: true }).waitFor();
  assert.deepEqual(await page.evaluate(() => window.dashboardFixture.commands.filter(c => ["start_proxy", "stop_proxy"].includes(c.command)).map(c => c.command)), ["stop_proxy", "start_proxy"]);
  assert.deepEqual(await page.evaluate(() => window.dashboardFixture.commands.find(c => c.command === "start_proxy").payload), { enableSystemProxy: false });

  // 先滚动到趋势卡；展开后必须相对视口定位、越过卡片边界且未被裁切。
  const spark = page.locator(".simple-spark");
  const original = await spark.boundingBox();
  await spark.locator(".simple-spark-fs").click();
  await page.locator(".simple-spark.expanded").waitFor();
  assert.equal(await page.evaluate(() => {
    const spark = document.querySelector(".simple-spark.expanded");
    const box = spark.getBoundingClientRect();
    const backdrop = document.querySelector(".simple-spark-backdrop").getBoundingClientRect();
    return getComputedStyle(spark).position === "fixed" && box.width >= innerWidth * 0.6 && box.height >= innerHeight * 0.5 &&
      box.left >= 0 && box.top >= 0 && box.right <= innerWidth + 1 && box.bottom <= innerHeight + 1 &&
      backdrop.left <= 1 && backdrop.top <= 1 && backdrop.right >= innerWidth - 1 && backdrop.bottom >= innerHeight - 1 &&
      spark.contains(document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2));
  }), true, "趋势全屏及遮罩覆盖视口，不被祖先约束");
  assert.ok((await spark.boundingBox()).width > original.width, "展开宽度应超出原卡片");
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => !document.querySelector(".simple-spark.expanded, .simple-spark-backdrop") && !document.documentElement.classList.contains("spark-fullscreen-open"));
  assert.equal(await page.evaluate(() => window.dashboardFixture.trafficOpens), 0, "展开/退出不触发流量页导航");
  await layout();

  // 自定义配置的路由/自动选点保持禁用，不扩大测试到配置管理。
  await open(960, 720, "zh", "day", "custom");
  assert.equal(await page.locator('.dash-side-rail .glass-seg').nth(0).locator("button:enabled").count(), 0);
  assert.equal(await page.locator('.dash-side-rail .glass-seg').nth(1).locator("button:enabled").count(), 0);
  await clean();
  await open(1280, 900, "zh", "aerospace", "running", "smiley");
  assert.equal(await page.locator('.dashboard-power-visual .face-orbit').count(), 1);
  await page.getByRole('button', { name: 'IP 信息 ↗', exact: true }).click();
  await page.getByRole('dialog', { name: 'IP 信息', exact: true }).waitFor();
  assert.match(await page.locator('.ip-details-dialog .ip-organization').innerText(), /Without Truncation/);
  await page.keyboard.press('Escape');
  await page.locator('.ip-details-dialog').waitFor({ state: 'hidden' });
  await page.locator('.service-status-card').click();
  await page.getByRole('dialog').waitFor();
  assert.equal(await page.locator('.service-status-list .service-status-row').count(), 13);
  await page.keyboard.press('Escape');
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await open(1280, 900, "zh", "aerospace", "running", "particle");
  assert.equal(await page.locator('.dashboard-power-visual .particle-orbit').count(), 1);
  await clean();
  console.log("概览回归通过：真实侧栏、紧凑布局、IP组织完整、服务详情、三种首页动效、模式/启停与全屏。");
} catch (error) {
  console.error("概览夹具失败场景", currentCase);
  // 失败最多追加一张，整次运行截图上限三张。
  if (page && !page.isClosed()) await page.screenshot({ path: "test-results/dashboard-failure.png" }).catch(() => {});
  throw error;
} finally {
  await context?.close();
  await browser.close();
}

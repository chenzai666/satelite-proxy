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
async function cardMotion(running) {
  const selector = ".dashboard-bento .dash-hero .dashboard-card-ambient";
  const name = await page.locator(selector).evaluate(el => getComputedStyle(el, "::before").animationName);
  assert.equal(name, running ? "dashboard-card-flow" : "none", "状态卡动效跟随运行状态");
  if (running) {
    const before = await page.locator(selector).evaluate(el => getComputedStyle(el, "::before").transform);
    await page.waitForFunction(({ selector, before }) => getComputedStyle(document.querySelector(selector), "::before").transform !== before, { selector, before });
  }
}
async function open(width, height, locale, theme, state = "running", hero = "classic", network = "business", sample = "long") {
  if (page) await clean();
  if (context) await context.close();
  errors = []; blocked = [];
  currentCase = `${width}x${height}-${locale}-${theme}-${state}-${sample}`;
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
  await page.goto(`${base}/scripts/ui-smoke/dashboard.html?${new URLSearchParams({ locale, theme, state, hero, network, sample })}`);
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
    // 节点标题和组织名必须完整换行；统计数值必须完整可见。
    const critical = ".dashboard-exit-ip, .dashboard-node-context .dash-hero-title, .dash-hero .ip-organization, .dashboard-rate-card__number, .dashboard-rate-card__unit, .dashboard-rate-card__stats dd, .traffic-stats-number, .traffic-stats-table th";
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
async function heroGeometry() {
  await page.waitForFunction(() => document.querySelector('.dash-hero .ip-organization')?.textContent === window.dashboardFixture.organization);
  const metrics = await page.evaluate(() => {
    const hero = document.querySelector('.dash-hero');
    const copy = hero.querySelector('.dash-hero-copy');
    const node = hero.querySelector('.dashboard-node-context');
    const title = node.querySelector('.dash-hero-title');
    const actions = hero.querySelector('.dash-hero-actions');
    const box = hero.getBoundingClientRect(), content = copy.getBoundingClientRect();
    const n = node.getBoundingClientRect(), a = actions.getBoundingClientRect(), t = title.getBoundingClientRect();
    const style = getComputedStyle(hero), textStyle = getComputedStyle(title);
    const range = document.createRange(); range.selectNodeContents(title);
    const lines = [...range.getClientRects()].filter(rect => rect.width > 0 && rect.height > 0);
    const buttons = [...actions.querySelectorAll('.secondary, .dash-more-btn')].map(el => el.getBoundingClientRect());
    return {
      height: box.height, rightHeight: document.querySelector('.service-status-card').getBoundingClientRect().height,
      align: style.alignSelf, below: a.top >= n.bottom - 1,
      buttonsBelow: buttons.length === 2 && buttons.every(b => b.top >= n.bottom - 1 && b.bottom <= a.bottom + 1),
      fullWidth: Math.abs(n.left - content.left) <= 1 && Math.abs(n.right - content.right) <= 1 && Math.abs(t.width - n.width) <= 1,
      complete: title.textContent === window.dashboardFixture.nodes[0].name,
      wraps: textStyle.whiteSpace === 'normal' && textStyle.textOverflow === 'clip' && textStyle.webkitLineClamp === 'none',
      lineCount: new Set(lines.map(rect => Math.round(rect.top))).size,
      textInside: lines.every(rect => rect.left >= t.left - 1 && rect.right <= t.right + 1 && rect.top >= t.top - 1 && rect.bottom <= t.bottom + 1),
      // 既检查卡片外部余量，也检查 copy 自身是否被撑高。
      bottomSlack: box.bottom - content.bottom - parseFloat(style.paddingBottom) - parseFloat(style.borderBottomWidth),
      copySlack: content.bottom - a.bottom,
      short: window.dashboardFixture.short,
    };
  });
  assert.equal(metrics.below && metrics.buttonsBelow, true, `${currentCase} 切换节点与更多按钮独立位于节点下方`);
  assert.equal(metrics.fullWidth, true, `${currentCase} 节点标题占满 copy 可用宽度`);
  assert.equal(metrics.complete && metrics.wraps && metrics.textInside, true, `${currentCase} 节点标题完整且无裁切`);
  if (!metrics.short) assert.ok(metrics.lineCount >= 2, `${currentCase} 长节点名称实际换行`);
  assert.equal(metrics.align, 'start', `${currentCase} hero 不随右侧网格拉伸`);
  assert.ok(Math.abs(metrics.bottomSlack) <= 2 && Math.abs(metrics.copySlack) <= 2, `${currentCase} copy 底部加 padding 后无多余空白 ${JSON.stringify(metrics)}`);
  return metrics;
}

async function serviceRoundTrip(saveScreenshot = false) {
  const settled = async count => page.waitForFunction(count => {
    const rows = [...document.querySelectorAll('.service-status-card-row')];
    return document.querySelector('.dash-hero .ip-organization')?.textContent === window.dashboardFixture.organization &&
      rows.length === count && rows.every(row => row.querySelector('.service-status-state')?.getAttribute('data-status') ===
        (row.getAttribute('data-service-id') === 'chatgpt' ? 'restricted' : 'reachable')) &&
      !document.querySelector('.service-status-card .service-status-checking');
  }, count);
  await settled(2);
  const before = await heroGeometry();
  for (const count of [4, 2]) {
    currentCase = currentCase.replace(/-services-\d+$/, '') + `-services-${count}`;
    await page.locator('.service-status-card').click();
    await page.getByRole('dialog').waitFor();
    for (const name of ['YouTube', 'ChatGPT']) {
      await page.getByRole('switch', { name: `在卡片展示 ${name}`, exact: true }).setChecked(count === 4);
    }
    await page.keyboard.press('Escape');
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    await settled(count);
    const after = await heroGeometry();
    assert.ok(Math.abs(after.height - before.height) <= 2, `${currentCase} 2→4→2 服务不改变 hero 高度 ${before.height} → ${after.height}`);
    if (count === 4) assert.ok(after.rightHeight > before.rightHeight + 2, `${currentCase} 右侧服务卡确实增高`);
    await layout();
    if (count === 4 && saveScreenshot) await page.screenshot({ path: 'test-results/dashboard-1280-four-services.png' });
  }
}
async function countryRefresh() {
  const button = page.locator('.dashboard-country-button');
  await page.waitForFunction(() => document.querySelector('.dashboard-country-button text')?.textContent === 'HK');
  assert.equal(await button.getAttribute('aria-label'), '刷新 出口IP');
  for (const [country, text] of [['US', 'US'], ['', '—'], ['invalid', '—']]) {
    const before = await page.evaluate(country => {
      window.dashboardFixture.countryCode = country;
      window.dashboardFixture.holdExitIp = true;
      return window.dashboardFixture.exitIpReads;
    }, country);
    if (country === 'US') await button.click();
    else { await button.focus(); await page.keyboard.press('Enter'); }
    await page.waitForFunction(() => window.dashboardFixture.releaseExitIp !== null);
    assert.equal(await button.isDisabled(), true, '探测中禁止重复点击');
    assert.equal(await button.getAttribute('aria-busy'), 'true');
    await page.keyboard.press('Enter');
    assert.equal(await page.evaluate(() => window.dashboardFixture.exitIpReads), before + 1, '每次点击仅发一个 check_exit_ip，忙碌时不重复');
    await page.evaluate(() => {
      window.dashboardFixture.holdExitIp = false;
      window.dashboardFixture.releaseExitIp();
    });
    await page.waitForFunction(() => document.querySelector('.dashboard-country-button')?.getAttribute('aria-busy') === 'false');
    assert.equal(await button.locator('text').textContent(), text, '国家代码来自当前出口结果，未知使用占位');
    assert.equal(await page.locator('.dashboard-exit-ip').textContent(), await page.evaluate(() => window.dashboardFixture.ipv6));
  }
  await layout();
}
async function freshServicesOnResume() {
  await page.waitForFunction(() => [...document.querySelectorAll('.service-status-card-row')].length === 2 &&
    [...document.querySelectorAll('.service-status-card-row')].every(row => row.querySelector('[data-status="reachable"]')) &&
    !document.querySelector('.service-status-card .service-status-checking'));
  const counts = await page.evaluate(async () => {
    const count = () => window.dashboardFixture.commands.filter(c => c.command === 'check_service_status').length;
    const before = count();
    const original = Object.getOwnPropertyDescriptor(document, 'visibilityState');
    try {
      for (let i = 0; i < 3; i++) {
        window.dispatchEvent(new Event('focus'));
        Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'hidden' });
        document.dispatchEvent(new Event('visibilitychange'));
        Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => 'visible' });
        document.dispatchEvent(new Event('visibilitychange'));
        window.dispatchEvent(new Event('focus'));
      }
      await new Promise(resolve => setTimeout(resolve, 150));
      return { before, after: count() };
    } finally {
      if (original) Object.defineProperty(document, 'visibilityState', original);
      else delete document.visibilityState;
    }
  });
  assert.equal(counts.after, counts.before, '近期服务结果在重复 focus / hidden→visible 后不新增 IPC');
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
  assert.equal(await page.evaluate(() => {
    const ip = document.querySelector('.dash-hero .ip-info-panel').getBoundingClientRect();
    const node = document.querySelector('.dashboard-node-context').getBoundingClientRect();
    return ip.bottom <= node.top + 1;
  }), true, "出口信息位于节点信息上方");
  await heroGeometry();
  const exit = await page.locator('.dashboard-exit-ip').evaluate(el => {
    const box = el.getBoundingClientRect();
    const power = document.querySelector('.dashboard-power').getBoundingClientRect();
    const panel = document.querySelector('.dash-hero .ip-info-panel').getBoundingClientRect();
    const icon = document.querySelector('.dashboard-country-button').getBoundingClientRect();
    const rootSize = parseFloat(getComputedStyle(document.documentElement).fontSize);
    return { text: el.textContent, rem: parseFloat(getComputedStyle(el).fontSize) / rootSize,
      wrap: getComputedStyle(el).overflowWrap, iconWidth: icon.width, iconHeight: icon.height,
      beforePower: box.right <= power.left + 1, abovePanel: box.bottom <= panel.top + 1 };
  });
  const expectedIp = await page.evaluate(() => window.dashboardFixture.ipv6);
  assert.equal(exit.text, expectedIp, '主视觉保留完整 IPv4/IPv6');
  assert.ok(exit.rem >= (expectedIp.includes(':') ? 1.25 : 1.5) && exit.rem <= 1.7, '大 IP 字体比例');
  assert.equal(exit.wrap, 'anywhere', '长 IPv6 可完整换行');
  assert.ok(exit.iconWidth >= 44 && exit.iconWidth <= 48 && exit.iconHeight >= 44 && exit.iconHeight <= 48, '国家按钮触控尺寸');
  assert.ok(exit.beforePower && exit.abovePanel, 'IP 为启停按钮留空间，类型和质量位于下方');
  assert.equal(await page.locator(".dash-side-rail").count(), 1);
  assert.equal(await page.locator(".dashboard-details-grid > *").count(), 4);
  assert.equal(await page.locator(".dashboard-power-visual .orbit").count(), 1, "首页动效保留");
  await page.waitForFunction(() => document.querySelector('.ip-organization')?.textContent === window.dashboardFixture.organization);
  assert.equal(await page.locator('.ip-organization-row .ip-organization').evaluate(el => getComputedStyle(el).textOverflow), "clip");
  assert.ok((await page.locator('.ip-info-badges').innerText()).includes(locale === "zh" ? "商业" : "Commercial"));
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
  // 保留原两张截图，短场景四服务仅追加一张代表截图。
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

  for (const sample of ['short', 'long']) {
    for (const [width, height] of [[960, 720], [1280, 900], [1600, 1000]]) {
      await open(width, height, 'zh', 'aerospace', 'running', 'classic', 'business', sample);
      await structure('zh');
      await serviceRoundTrip(sample === 'short' && width === 1280);
    }
  }

  await open(1280, 900, "zh", "aerospace");
  await countryRefresh();
  await freshServicesOnResume();
  const sideButtons = page.locator('.side-nav-items .side-nav-item');
  const selectedIcon = await sideButtons.nth(0).locator('svg path').getAttribute('d');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await sideButtons.nth(1).click();
  await page.waitForFunction(() => document.querySelector('.side-nav-indicator')?.getAnimations().some(animation => animation.playState === 'running'));
  assert.equal(await sideButtons.nth(1).locator('svg path').getAttribute('fill'), 'currentColor', '选中图标切为实心');
  assert.equal(await sideButtons.nth(0).locator('svg path').getAttribute('fill'), 'none', '未选中图标回到线框');
  assert.notEqual(await sideButtons.nth(0).locator('svg path').getAttribute('d'), selectedIcon, '图标形态切换而非只换色');
  for (let index = 2; index < 6; index++) {
    await sideButtons.nth(index).click();
    assert.equal(await sideButtons.nth(index).locator('svg path').getAttribute('fill'), 'currentColor');
    assert.equal(await sideButtons.nth(index - 1).locator('svg path').getAttribute('fill'), 'none');
  }
  await page.evaluate(() => window.dashboardFixture.setMotion('reduced'));
  await sideButtons.nth(0).click();
  assert.equal(await page.locator('.side-nav-indicator').evaluate(el => getComputedStyle(el).transitionDuration), '0s');
  assert.equal(await page.locator('main > .page-enter').evaluate(el => getComputedStyle(el).animationName), 'none');
  await page.evaluate(() => window.dashboardFixture.setMotion('standard'));
  assert.equal(await page.evaluate(() => document.documentElement.dataset.motionReduced), 'false', '应用标准模式独立于系统减少动态');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.waitForFunction(() => [...document.querySelectorAll('main > .page-enter')].every(el => el.getAnimations().every(animation => animation.playState === 'finished')));
  await cardMotion(true);
  await page.evaluate(() => window.dashboardFixture.setMotion('system'));
  await page.emulateMedia({ reducedMotion: "reduce" });
  await cardMotion(false);
  assert.equal(await page.locator('.dashboard-power-visual .orbit').evaluate(el =>
    [el, ...el.querySelectorAll('*')].every(node => ['::before', '::after', null].every(pseudo =>
      getComputedStyle(node, pseudo).animationName === 'none'))), true, "减少动态效果时关闭经典轨道CSS动画");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.evaluate(() => window.dashboardFixture.setMotion('standard'));
  await cardMotion(true);
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
  await cardMotion(false);
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
  await page.getByRole('switch', { name: '在卡片展示 YouTube', exact: true }).check();
  await page.getByRole('switch', { name: '在卡片展示 ChatGPT', exact: true }).check();
  await page.waitForFunction(() => {
    const rows = [...document.querySelectorAll('.service-status-card-row')];
    return rows.length === 4 && rows.every(row => row.querySelector('[data-status]'));
  });
  assert.ok(await page.locator('.service-status-card-row[data-service-id="chatgpt"]').innerText().then(text => text.includes('HTTP 访问受限')));
  await page.keyboard.press('Escape');
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: words.zh.stop, exact: true }).click();
  await page.getByRole('button', { name: words.zh.start, exact: true }).waitFor();
  await page.locator('.service-status-card').click();
  await page.waitForFunction(() => document.querySelector('.service-status-row[data-service-id="github"] .service-status-row-meta')?.textContent.includes('直连'));
  assert.equal(await page.locator('.service-status-card-row[data-service-id="github"] .service-status-state').getAttribute('data-status'), 'reachable', 'GitHub直连可达按真实结果展示');
  await page.keyboard.press('Escape');
  const beforeStart = await page.evaluate(() => {
    window.dashboardFixture.serviceFailures = 1;
    return window.dashboardFixture.commands.filter(c => c.command === 'check_service_status').length;
  });
  await page.getByRole('button', { name: words.zh.start, exact: true }).click();
  await page.waitForFunction(count => {
    const calls = window.dashboardFixture.commands.filter(c => c.command === 'check_service_status');
    return calls.length >= count + 2 && document.querySelector('.service-status-card-row[data-service-id="github"] .service-status-state')?.getAttribute('data-status') === 'reachable';
  }, beforeStart);
  const latestServices = await page.evaluate(() => window.dashboardFixture.commands.filter(c => c.command === 'check_service_status').at(-1).payload.ids);
  assert.deepEqual([...latestServices].sort(), ['google', 'github', 'youtube', 'chatgpt'].sort(), '开启代理后全部展示项自动检测并从暂态失败恢复');
  await open(1280, 900, "zh", "aerospace", "running", "particle");
  assert.equal(await page.locator('.dashboard-power-visual .particle-orbit').count(), 1);
  for (const [network, text, quality] of [["residential", "家宽", "优"], ["business", "商业", "优"], ["mobile", "移动网络", "优"], ["datacenter", "机房", "普通"], ["isp", "ISP 网络", "未知"], ["unknown", "未知", "未知"], ["tor", "商业", "风险"], ["abuse", "商业", "风险"]]) {
    await open(1280, 900, "zh", "aerospace", "running", "classic", network);
    await page.waitForFunction(() => document.querySelector('.ip-organization')?.textContent.includes('Without Truncation'));
    if (["isp", "unknown"].includes(network)) {
      assert.equal(await page.locator('.ip-network-tag').count(), 0, "首页只展示四种明确类型");
      assert.equal(await page.locator('[class*="ip-quality-"]').count(), 0, "未知类型不显示质量标签");
    } else {
      await page.waitForFunction(text => document.querySelector('.ip-info-badges')?.textContent.includes(text), text);
      assert.ok((await page.locator('.ip-info-badges').innerText()).includes(`质量 ${quality}`), "质量与网络类型/风险标记一致");
    }
    assert.doesNotMatch(await page.locator('.ip-info-badges').innerText(), /推测/);
  }
  await clean();
  console.log("概览回归通过：真实侧栏、紧凑布局、IP组织完整、服务详情、三种首页动效、模式/启停与全屏。");
} catch (error) {
  console.error("概览夹具失败场景", currentCase);
  // 失败最多追加一张；成功最多三张（保留原两张及一张四服务代表图）。
  if (page && !page.isClosed()) await page.screenshot({ path: "test-results/dashboard-failure.png" }).catch(() => {});
  throw error;
} finally {
  await context?.close();
  await browser.close();
}

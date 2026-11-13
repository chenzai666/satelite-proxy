import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";

// 本机只允许 node --check / tsc；浏览器与 Vite 均由 CI 启动。
if (process.env.CI !== "true") throw new Error("节点浏览器回归仅允许在 CI 运行");
assert.ok(process.env.PLAYWRIGHT_MODULE, "缺少 CI PLAYWRIGHT_MODULE");
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const base = "http://127.0.0.1:1440";
await mkdir("test-results", { recursive: true });
const browser = await chromium.launch({ headless: true });
let page;
let currentCase = "delete";
try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, serviceWorkers: "block" });
  const errors = [];
  const blocked = [];
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
  async function clean() {
    assert.deepEqual(await page.evaluate(() => window.nodeFixture.unexpected), [], `${currentCase} 未声明 IPC`);
    assert.deepEqual(errors, [], `${currentCase} 页面异常`);
    assert.deepEqual(blocked, [], `${currentCase} 外部网络请求`);
  }

  async function settledGrid() {
    await page.locator(".node-card").first().waitFor();
    await page.evaluate(() => document.fonts.ready);
    await page.waitForFunction(() => {
      const grid = document.querySelector(".node-grid");
      if (!grid) return false;
      const width = Number.parseFloat(getComputedStyle(grid).width);
      const expected = Math.max(1, Math.min(4, Math.floor((width + 10) / 250)));
      return getComputedStyle(grid).gridTemplateColumns.split(" ").length === expected;
    });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  }

  async function layout(virtualized) {
    await settledGrid();
    const result = await page.evaluate(virtualized => {
      const issues = [];
      const zoom = Number.parseFloat(getComputedStyle(document.documentElement).zoom) || 1;
      const grid = document.querySelector(".node-grid");
      const gridBox = grid.getBoundingClientRect();
      const cards = [...grid.querySelectorAll(".node-card")];
      const cols = getComputedStyle(grid).gridTemplateColumns.split(" ").length;
      const overlap = (a, b) => a.left < b.right - .5 && a.right > b.left + .5 && a.top < b.bottom - .5 && a.bottom > b.top + .5;
      if (!document.querySelector(".pro-shell > .side-nav")) issues.push("未加载真实侧栏");
      for (const card of cards) {
        const box = card.getBoundingClientRect();
        if (box.width / zoom < Math.min(240, gridBox.width / zoom) - 1) issues.push("卡片小于容器允许的最小宽度");
        if (box.left < gridBox.left - 1 || box.right > gridBox.right + 1) issues.push("卡片超出网格");
        const menu = card.querySelector(".node-card-menu");
        const menuBox = menu.getBoundingClientRect();
        const style = getComputedStyle(menu);
        if (Math.abs(menuBox.width / zoom - 22) > .5 || Math.abs(menuBox.height / zoom - 22) > .5) issues.push("更多按钮不是 22px 图标按钮");
        if (menu.parentElement !== card.querySelector(".node-card-top")) issues.push("更多按钮未在顶行占位");
        if (style.position === "absolute" || style.padding !== "0px") issues.push("更多按钮受到绝对定位或大内边距覆盖");
        if (style.backgroundColor !== "rgba(0, 0, 0, 0)" || style.boxShadow !== "none") issues.push("更多按钮误用主色背景或光晕");
        if (style.color !== getComputedStyle(card.querySelector(".node-sub-label")).color) issues.push("更多按钮未使用中性色");
        for (const selector of [".node-select-dot", ".node-card-meta", ".node-card-name", ".node-card-footer"]) {
          const area = card.querySelector(selector).getBoundingClientRect();
          if (overlap(area, menuBox)) issues.push(`更多按钮重叠 ${selector}`);
          if (area.top < box.top - 1 || area.bottom > box.bottom + 1) issues.push(`卡片内容被裁切 ${selector}`);
        }
        const top = card.querySelector(".node-card-top").getBoundingClientRect();
        const name = card.querySelector(".node-card-name");
        if (name.getBoundingClientRect().top < top.bottom - .5) issues.push("名称与顶行重叠");
        for (const pill of card.querySelectorAll(".node-card-meta > *")) {
          const pillBox = pill.getBoundingClientRect();
          const range = document.createRange();
          range.selectNodeContents(pill);
          const text = range.getBoundingClientRect();
          if (text.left < pillBox.left - 1 || text.right > pillBox.right + 1) issues.push("协议或副核标签文字被裁切");
          if (pill.scrollWidth > pill.clientWidth + 1) issues.push("协议或副核标签溢出");
        }
        if (name.scrollWidth > name.clientWidth + 1 && getComputedStyle(name).textOverflow !== "ellipsis") issues.push("长名称缺少省略保护");
        if (virtualized && Math.abs(box.height / zoom - 84) > .5) issues.push("虚拟卡片行高改变");
      }
      if (virtualized) {
        if (!grid.classList.contains("node-grid-virtual") || cards.length >= window.nodeFixture.nodes.length) issues.push("大列表未虚拟化");
        const container = grid.parentElement;
        const box = container.getBoundingClientRect();
        const count = window.nodeFixture.nodes.length;
        const total = Math.ceil(count / cols) * 94 - 10;
        if (Math.abs(box.height / zoom - total) > 2) issues.push("虚拟总高度与 CSS 列数不一致");
        const first = window.nodeFixture.nodes.findIndex(node => node.id === cards[0].dataset.nodeId);
        if (first % cols !== 0) issues.push("虚拟窗口从半行开始");
        if (Math.abs((cards[0].getBoundingClientRect().top - box.top) / zoom - first / cols * 94) > 2) issues.push("虚拟占位与首行位置不一致");
        const scroller = document.querySelector(".main").getBoundingClientRect();
        if (!cards.some(card => { const r = card.getBoundingClientRect(); return r.bottom > scroller.top && r.top < scroller.bottom; })) issues.push("虚拟窗口可视区域空白");
      }
      return { issues: [...new Set(issues)], cols, count: cards.length, width: gridBox.width / zoom };
    }, virtualized);
    assert.deepEqual(result.issues, [], `${currentCase} 卡片布局`);
    return result;
  }

  await page.goto(`${base}/scripts/ui-smoke/nodes.html`);
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
  await clean();

  let cases = 0;
  for (const width of [960, 1280, 1600]) for (const theme of ["day", "aerospace"]) for (const zoom of [1, 1.5]) for (const large of [false, true]) {
    const locale = theme === "day" ? "zh" : "en";
    currentCase = `${width}-${theme}-zoom${zoom}-${large ? "virtual" : "grid"}`;
    await page.setViewportSize({ width, height: 900 });
    await page.goto(`${base}/scripts/ui-smoke/nodes.html?${new URLSearchParams({ view: "grid", theme, locale, zoom: String(zoom), large: String(large) })}`);
    await page.waitForFunction(theme => document.documentElement.dataset.theme === theme, theme);
    await page.locator(".node-card").first().scrollIntoViewIfNeeded();
    const start = await layout(large);
    if (!large) assert.equal(start.count, 9, "普通网格节点齐全");
    if (large) {
      await page.locator(".main").evaluate(el => { el.scrollTop = (el.scrollHeight - el.clientHeight) / 2; });
      await page.waitForFunction(() => !document.querySelector('.node-card[data-node-id="a"]'));
      await layout(true);
      await page.locator(".main").evaluate(el => { el.scrollTop = el.scrollHeight; });
      await page.locator('.node-card[data-node-id="node-602"]').waitFor();
      await layout(true);
      // 在列表底部改变侧栏宽度，核对重新分行及占位高度。
      await page.locator(".side-nav-toggle").click();
      await layout(true);
      await page.locator(".main").evaluate(el => { el.scrollTop = el.scrollHeight; });
      await page.locator('.node-card[data-node-id="node-602"]').waitFor();
      await layout(true);
    }
    if (width === 960 && theme === "day" && zoom === 1 && !large) {
      const a = page.locator('.node-card[data-node-id="a"]');
      const b = page.locator('.node-card[data-node-id="b"]');
      await a.locator(".node-card-name").click();
      await b.locator(".node-card-name").click({ modifiers: ["Control"] });
      assert.equal(await page.locator(".node-card.selected").count(), 2, "卡片多选保留");
      const menu = a.locator(".node-card-menu");
      await menu.focus();
      await page.keyboard.press("Enter");
      await page.getByRole("menu").waitFor();
      assert.equal(await menu.getAttribute("aria-expanded"), "true");
      assert.equal(await page.locator(".node-card.selected").count(), 2, "菜单不改变选择");
      await page.keyboard.press("Escape");
      await page.getByRole("menu").waitFor({ state: "detached" });
      await menu.click();
      await page.getByRole("menu").waitFor();
      await page.locator("h1").click();
      await page.getByRole("menu").waitFor({ state: "detached" });
      await menu.hover();
      assert.equal(await menu.evaluate(el => getComputedStyle(el).boxShadow), "none", "悬停不产生主色光晕");
      await page.mouse.move(0, 0);
      await page.locator('.node-card[data-node-id="c"] .node-select-dot').click();
      await page.waitForFunction(() => window.nodeFixture.selected.length === 1);
      assert.deepEqual(await page.evaluate(() => window.nodeFixture.selected), ["c"]);
      await a.locator(".node-card-latency").click();
      await page.waitForFunction(() => document.querySelector('.node-card[data-node-id="a"] .node-card-latency').textContent.includes("42"));
      assert.deepEqual(await page.evaluate(() => window.nodeFixture.tested), [["a"]]);
      const from = await a.locator(".node-card-name").boundingBox();
      const target = await b.locator(".node-card-name").boundingBox();
      await page.mouse.move(from.x + 16, from.y + from.height / 2);
      await page.mouse.down();
      await page.mouse.move(target.x + target.width * .8, target.y + target.height / 2, { steps: 8 });
      await page.mouse.up();
      await page.waitForFunction(() => window.nodeFixture.reordered.length === 1);
      assert.deepEqual((await page.evaluate(() => window.nodeFixture.reordered[0])).slice(0, 3), ["b", "a", "c"], "拖拽排序保留");
      const before = await layout(false);
      await page.locator(".side-nav-toggle").click();
      const after = await layout(false);
      assert.ok(Math.abs(after.width - before.width) > 50, "实测侧栏折叠改变容器宽度");
      assert.notEqual(after.cols, before.cols, "侧栏变化同步更新列数");
      await page.evaluate(() => { document.documentElement.style.zoom = "1.5"; });
      await layout(false);
      await page.evaluate(() => { document.documentElement.style.zoom = "1"; });
      await page.locator(".side-nav-toggle").click();
      await layout(false);
    }
    if (!large && zoom === 1 && ((width === 960 && theme === "day") || (width === 1600 && theme === "aerospace"))) {
      await page.screenshot({ path: `test-results/nodes-${currentCase}.png` });
    }
    await clean();
    cases++;
  }
  assert.equal(cases, 24);
  console.log("节点页浏览器回归通过：Delete 保护/确认/单删/批删，24 组卡片布局（960/1280/1600、明暗主题、中英、zoom 1/1.5、普通/603 节点虚拟网格），选择/菜单/测速/拖拽、侧栏变化与滚动；两张代表截图。");
} catch (error) {
  console.error("节点页失败场景", currentCase);
  if (page && !page.isClosed()) await page.screenshot({ path: "test-results/nodes-failure.png" }).catch(() => {});
  throw error;
} finally { await browser.close(); }

import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";

// 本机只允许 node --check / tsc；CI 负责 Playwright 安装和隔离 Vite 服务。
if (process.env.CI !== "true") throw new Error("流量表格浏览器回归仅允许在 CI 运行");
assert.ok(process.env.PLAYWRIGHT_MODULE, "缺少 CI PLAYWRIGHT_MODULE");
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
const base = "http://127.0.0.1:1440";
const labels = {
  zh: { tabs: ["连接中", "已关闭", "失败"], headers: ["时间", "目标", "节点", "规则", "进程", "流量"], actions: "操作" },
  en: { tabs: ["Active", "Closed", "Failed"], headers: ["Time", "Destination", "Node", "Rule", "Process", "Bytes"], actions: "Actions" },
};
const modes = ["live", "history", "failures"];
await mkdir("test-results", { recursive: true });
const browser = await chromium.launch({ headless: true });
let context, page;
let currentCase = "setup";
let errors = [], blocked = [];
let cases = 0;

async function clean() {
  assert.deepEqual(await page.evaluate(() => window.trafficTableFixture.unexpected), [], `${currentCase} 未声明 IPC`);
  assert.deepEqual(blocked, [], `${currentCase} 外部网络请求`);
  assert.deepEqual(errors, [], `${currentCase} 页面异常`);
}

async function layout(zoom) {
  const result = await page.evaluate(zoom => {
    const issues = [];
    const table = document.querySelector(".traffic-panel .conn-table");
    const wrap = table.closest(".table-wrap");
    const headers = [...table.querySelectorAll("thead th")];
    const rows = [...table.querySelectorAll("tbody tr:not(.virt-pad)")];
    const boxes = headers.map(el => el.getBoundingClientRect());
    const tolerance = 1.5;
    if (headers.length !== 6 || rows.length !== window.trafficTableFixture.rows.length) issues.push("表头或正文数量错误");
    const texts = [];
    headers.forEach((header, index) => {
      const box = boxes[index];
      if (!(box.width > 0 && box.height > 0)) issues.push(`表头 ${index} 无正尺寸`);
      if (index && box.left < boxes[index - 1].right - tolerance) issues.push(`表头 ${index} 单元重叠`);
      if (!header.textContent.trim()) issues.push(`表头 ${index} 为空`);
      const walker = document.createTreeWalker(header, NodeFilter.SHOW_TEXT);
      const rects = [];
      while (walker.nextNode()) {
        if (!walker.currentNode.textContent.trim()) continue;
        const range = document.createRange();
        range.selectNodeContents(walker.currentNode);
        rects.push(...[...range.getClientRects()].filter(r => r.width > 0 && r.height > 0));
      }
      if (!rects.length) issues.push(`表头 ${index} 无文字尺寸`);
      for (const rect of rects) {
        if (rect.left < box.left - tolerance || rect.right > box.right + tolerance ||
            rect.top < box.top - tolerance || rect.bottom > box.bottom + tolerance) issues.push(`表头 ${index} 文字超出单元`);
        if (Math.abs(rect.top - rects[0].top) > tolerance) issues.push(`表头 ${index} 文字换行`);
        texts.push({ index, left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom });
      }
    });
    for (const a of texts) for (const b of texts) {
      if (a.index >= b.index) continue;
      if (a.left < b.right - tolerance && a.right > b.left + tolerance &&
          a.top < b.bottom - tolerance && a.bottom > b.top + tolerance) issues.push(`表头文字 ${a.index}/${b.index} 重叠`);
    }
    if (boxes[1].width / zoom < 160 - 0.5) issues.push(`目标列小于 160px：${boxes[1].width / zoom}`);
    if (!headers[1].classList.contains("conn-th-dest")) issues.push("缺少真实目标列表头类名");
    for (const [rowIndex, row] of rows.entries()) {
      if (row.cells.length !== 6) { issues.push(`正文 ${rowIndex} 列数错误`); continue; }
      [...row.cells].forEach((cell, index) => {
        const box = cell.getBoundingClientRect();
        if (box.width <= 0 || Math.abs(box.left - boxes[index].left) > tolerance ||
            Math.abs(box.right - boxes[index].right) > tolerance) issues.push(`正文 ${rowIndex}/${index} 与表头错位`);
      });
    }
    // 列宽按布局 CSS 像素验证，getBoundingClientRect 则包含根节点 zoom。
    if (table.getBoundingClientRect().width / zoom < 960 - 1) issues.push("窄屏压缩表格最小宽度");
    const scrolling = wrap.scrollWidth > wrap.clientWidth + 1;
    if (scrolling && !["auto", "scroll"].includes(getComputedStyle(wrap).overflowX)) issues.push("表格溢出却没有横向滚动");
    for (const el of [document.querySelector("main"), document.querySelector(".traffic-page"), document.querySelector(".traffic-panel")]) {
      if (el.scrollWidth > el.clientWidth + 1) issues.push(`外层横向溢出 ${el.className}`);
    }
    const wrapBox = wrap.getBoundingClientRect();
    const last = boxes.at(-1);
    return { issues: [...new Set(issues)], scrolling, widths: boxes.map(box => box.width / zoom),
      scrollLeft: wrap.scrollLeft, maxScroll: wrap.scrollWidth - wrap.clientWidth,
      firstLeft: boxes[0].left, lastRight: last.right, wrapLeft: wrapBox.left, wrapRight: wrapBox.right };
  }, zoom);
  assert.deepEqual(result.issues, [], `${currentCase} 布局检查`);
  return result;
}

try {
  for (const width of [720, 960, 1280]) for (const locale of ["zh", "en"]) for (const zoom of [1, 1.5]) {
    if (context) await context.close();
    errors = []; blocked = [];
    currentCase = `${width}-${locale}-zoom${zoom}-setup`;
    context = await browser.newContext({ viewport: { width, height: 900 }, locale: locale === "zh" ? "zh-CN" : "en-US", serviceWorkers: "block" });
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
    await page.goto(`${base}/scripts/ui-smoke/traffic-table.html?${new URLSearchParams({ locale, zoom: String(zoom) })}`);
    await page.locator(".traffic-header .glass-seg").getByRole("button", { name: labels[locale].tabs[0], exact: true }).waitFor();
    await page.waitForFunction(theme => document.documentElement.dataset.theme === theme &&
      getComputedStyle(document.querySelector(".pro-shell")).getPropertyValue("--pro-panel").trim().length > 0,
    locale === "zh" ? "day" : "aerospace");
    await page.evaluate(() => document.fonts.ready);
    assert.equal(await page.locator(".pro-shell > .side-nav").count(), 1, "使用真实侧栏");
    assert.equal(await page.evaluate(() => Number(getComputedStyle(document.documentElement).zoom)), zoom);
    for (const [index, mode] of modes.entries()) {
      currentCase = `${width}-${locale}-zoom${zoom}-${mode}`;
      await page.locator(".traffic-header .glass-seg").getByRole("button", { name: labels[locale].tabs[index], exact: true }).click();
      await page.waitForFunction(mode => window.trafficTableFixture.reads[mode] > 0 &&
        document.querySelectorAll(".traffic-panel .conn-table tbody tr:not(.virt-pad)").length === window.trafficTableFixture.rows.length, mode);
      await page.waitForFunction(() => [...document.querySelectorAll(".page-enter")].every(el =>
        el.getAnimations().every(animation => animation.playState === "finished") && getComputedStyle(el).transform === "none"));
      const headers = [...labels[locale].headers];
      if (mode === "failures") headers[5] = labels[locale].actions;
      assert.deepEqual(await page.locator(".traffic-panel thead th").allTextContents(), headers, `${currentCase} 六列翻译`);
      assert.equal(await page.locator(".traffic-panel .fails-table").count(), mode === "failures" ? 1 : 0);
      const start = await layout(zoom);
      if (width === 720) assert.equal(start.scrolling, true, `${currentCase} 窄屏应在表格容器横滚`);
      if (start.scrolling) {
        await page.locator(".traffic-panel .table-wrap").evaluate(el => { el.scrollLeft = el.scrollWidth; });
        const end = await layout(zoom);
        assert.ok(end.scrollLeft > 0 && Math.abs(end.scrollLeft - end.maxScroll) <= 1, `${currentCase} 横滚到末列`);
        assert.ok(end.lastRight <= end.wrapRight + 2 && end.lastRight >= end.wrapLeft, `${currentCase} 末列表头可滚动到可视区`);
        end.widths.forEach((value, i) => assert.ok(Math.abs(value - start.widths[i]) <= 1, `${currentCase} 滚动保持第 ${i} 列宽`));
        await page.locator(".traffic-panel .table-wrap").evaluate(el => { el.scrollLeft = 0; });
        const reset = await layout(zoom);
        assert.equal(reset.scrollLeft, 0);
        assert.ok(Math.abs(reset.firstLeft - reset.wrapLeft) <= 2, `${currentCase} 首列回到可视区`);
      }
      await clean();
      if ((width === 720 && locale === "zh" && zoom === 1 && mode === "live") ||
          (width === 1280 && locale === "en" && zoom === 1.5 && mode === "failures")) {
        await page.screenshot({ path: `test-results/traffic-table-${width}-${locale}-zoom${zoom}-${mode}.png` });
      }
      cases++;
    }
    assert.ok(Object.values(await page.evaluate(() => window.trafficTableFixture.reads)).every(count => count > 0), "三个真实页面均已通过 mock IPC 取数");
    await clean();
  }
  assert.equal(cases, 36);
  console.log("流量表格回归通过：36 组，三页面、720/960/1280、中英文、zoom 1/1.5、六列表头/目标列/正文对齐与横滚；两张代表截图。");
} catch (error) {
  console.error("流量表格失败场景", currentCase);
  // 成功仅两张代表截图；整次运行最多额外保存一张失败截图。
  if (page && !page.isClosed()) await page.screenshot({ path: "test-results/traffic-table-failure.png" }).catch(() => {});
  throw error;
} finally {
  try { await context?.close(); } finally { await browser.close(); }
}

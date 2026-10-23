import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { pathToFileURL } from "node:url";

// 仅供 CI：由调用方启动隔离 Vite（默认 127.0.0.1:1440），本脚本不启动客户端或内核。
if (!["true", "1"].includes(process.env.CI ?? "")) {
  throw new Error("流量卡浏览器回归仅允许在 CI 运行；本机请运行 Node 单测和 tsc。");
}
assert.ok(process.env.PLAYWRIGHT_MODULE, "请设置 CI 安装的 Playwright index.mjs 路径");
const base = new URL(process.env.TRAFFIC_STATS_BASE_URL ?? "http://127.0.0.1:1440");
assert.ok(base.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(base.hostname), "夹具服务仅允许 HTTP loopback");
assert.ok(!base.username && !base.password && base.pathname === "/" && !base.search && !base.hash);
const { chromium } = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
await mkdir("test-results", { recursive: true });
const browser = await chromium.launch({ headless: true });

const fields = ["upload_speed", "download_speed", "upload_total", "download_total"];
const expected = {
  proxy: ["8.0 KB/s", "2.0 KB/s", "64.0 KB", "32.0 KB"],
  direct: ["4.0 KB/s", "1.0 KB/s", "32.0 KB", "16.0 KB"],
  unclassified: ["1.0 KB/s", "512 B/s", "8.0 KB", "4.0 KB"],
  total: ["13.0 KB/s", "3.5 KB/s", "104 KB", "52.0 KB"],
};
const missing = ["—", "—", "—", "—"];
const zero = ["0 B/s", "0 B/s", "0 B", "0 B"];
const rowIds = ["proxy", "direct", "unclassified", "total"];
const labels = {
  zh: { rows: ["代理", "直连", "未分类", "合计"], columns: ["出口", "实时", "本次累计"], connections: "连接采样", outbound: "出站计数", unavailable: "分类不可用", link: "流量统计 ↗", down: "下载", up: "上传" },
  en: { rows: ["Proxy", "Direct", "Unclassified", "Total"], columns: ["Outbound", "Live", "Session total"], connections: "Connection sample", outbound: "Outbound counters", unavailable: "Classification unavailable", link: "Traffic stats ↗", down: "Download", up: "Upload" },
};

async function scenario(page, name) {
  await page.evaluate(name => window.trafficFixture.setScenario(name), name);
  await page.waitForFunction(name => window.trafficFixture.status === window.trafficFixture.scenarios[name], name);
}

async function rowValues(page, id) {
  return page.locator(`[data-traffic-row="${id}"] .traffic-stats-number`).allTextContents();
}

async function expectRows(page, expectedRows, locale) {
  assert.deepEqual(await page.locator("[data-traffic-row]").evaluateAll(rows => rows.map(row => row.dataset.trafficRow)), Object.keys(expectedRows));
  assert.deepEqual(await page.locator("thead th").allTextContents(), labels[locale].columns);
  for (const [id, values] of Object.entries(expectedRows)) {
    assert.deepEqual(await rowValues(page, id), values, id);
    assert.equal(await page.locator(`[data-traffic-row="${id}"] > th`).textContent(), labels[locale].rows[rowIds.indexOf(id)]);
    for (const [index, period] of ["live", "session"].entries()) {
      for (const [offset, direction] of ["down", "up"].entries()) {
        const metric = page.locator(`[data-traffic-row="${id}"] [data-period="${period}"] [data-direction="${direction}"]`);
        assert.equal(await metric.getAttribute("aria-label"), `${labels[locale][direction]}: ${values[index * 2 + offset]}`);
      }
    }
  }
}

async function assertReconciled(page) {
  const result = await page.evaluate(fields => {
    const snapshot = window.trafficFixture.status;
    const breakdown = snapshot?.traffic_breakdown;
    if (!breakdown || breakdown.source === "unavailable") return null;
    return fields.map(key => ({
      key,
      sum: breakdown.proxy[key] + breakdown.direct[key] + breakdown.unclassified[key],
      root: snapshot[key],
    }));
  }, fields);
  assert.ok(result);
  for (const { key, sum, root } of result) assert.equal(sum, root, `夹具必须对账：${key}`);
}

async function assertTooltip(page, source, locale) {
  const badge = page.locator(".traffic-stats-source");
  assert.equal(await badge.textContent(), labels[locale][source]);
  const hint = await badge.getAttribute("title");
  assert.equal(await badge.getAttribute("aria-label"), hint);
  if (source === "unavailable") {
    assert.match(hint, locale === "zh" ? /横线而非零/ : /dash, not zero/);
  } else {
    assert.match(hint, locale === "zh" ? /代理 \+ 直连 \+ 未分类 = 合计/ : /proxy \+ direct \+ unclassified/);
    assert.match(hint, locale === "zh" ? /不推导分类/ : /without inferring categories/);
    if (source === "connections") {
      assert.match(hint, locale === "zh" ? /漏采的短连接与未知出站/ : /missed short-lived connections and unknown outbounds/);
    } else assert.match(hint, locale === "zh" ? /未知出站 tag/ : /unknown outbound tags/);
  }
  await badge.hover();
  await badge.focus();
  assert.equal(await badge.evaluate(element => element === document.activeElement), true);
  await badge.click();
  assert.equal(await page.evaluate(() => window.trafficFixture.opens), 0, "说明焦点不能触发导航");
}

async function assertLayout(page, zoom, grid) {
  const issues = await page.evaluate(({ zoom, grid }) => {
    const issues = [];
    const card = document.querySelector(".traffic-stats-card");
    const cardBox = card.getBoundingClientRect();
    const tolerance = 1.5;
    const describe = element => `${element.tagName}.${element.className}: ${element.textContent?.trim().slice(0, 60)}`;
    // scrollWidth alone misses clipped ancestors and overlapping cells: check
    // element bounds and each rendered text fragment as well.
    for (const element of [card, ...card.querySelectorAll("*")]) {
      const box = element.getBoundingClientRect();
      if (element.clientWidth > 0 && element.scrollWidth > element.clientWidth + 1) issues.push(`横向溢出 ${describe(element)}`);
      if (element.clientHeight > 0 && element.scrollHeight > element.clientHeight + 1) issues.push(`纵向裁切 ${describe(element)}`);
      if (box.left < cardBox.left - tolerance || box.right > cardBox.right + tolerance || box.top < cardBox.top - tolerance || box.bottom > cardBox.bottom + tolerance) {
        issues.push(`超出卡片 ${describe(element)}`);
      }
      const style = getComputedStyle(element);
      if (style.textOverflow === "ellipsis") issues.push(`禁止省略号 ${describe(element)}`);
    }
    for (const cell of card.querySelectorAll("th, td")) {
      const cellBox = cell.getBoundingClientRect();
      const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT);
      while (walker.nextNode()) {
        if (!walker.currentNode.textContent.trim()) continue;
        const range = document.createRange();
        range.selectNodeContents(walker.currentNode);
        for (const box of range.getClientRects()) {
          if (box.left < cellBox.left - tolerance || box.right > cellBox.right + tolerance || box.top < cellBox.top - tolerance || box.bottom > cellBox.bottom + tolerance) {
            issues.push(`文本越过单元格 ${describe(cell)}`);
          }
        }
      }
    }
    if (grid) {
      const gridElement = document.querySelector("[data-fixture-grid]");
      const columns = getComputedStyle(gridElement).gridTemplateColumns.split(" ").map(Number.parseFloat);
      const gridWidth = gridElement.getBoundingClientRect().width / zoom;
      const expectedWidth = (gridWidth - 2 * 12) / 3;
      if (columns.length !== 3) issues.push(`专业概览应保留三列：${columns}`);
      for (const element of gridElement.children) {
        const box = element.getBoundingClientRect();
        if (Math.abs(box.width / zoom - expectedWidth) > 1) issues.push(`相邻卡片列宽被挤压 ${describe(element)}`);
        if (box.height / zoom < 151) issues.push(`相邻卡片高度小于原9.5rem ${describe(element)}`);
        for (const other of gridElement.children) {
          if (element === other) continue;
          const otherBox = other.getBoundingClientRect();
          if (box.left < otherBox.right - tolerance && box.right > otherBox.left + tolerance && box.top < otherBox.bottom - tolerance && box.bottom > otherBox.top + tolerance) issues.push("仪表卡片重叠");
        }
      }
    }
    const root = document.documentElement;
    if (root.scrollWidth > root.clientWidth + 1) issues.push("页面横向溢出");
    return issues;
  }, { zoom, grid });
  assert.deepEqual(issues, [], "卡片文字、单元格、网格应无溢出/裁切/挤压");
}

let currentPage;
let currentCase = "setup";
let screenshots = 0;
try {
  for (const theme of ["day", "aerospace"]) {
    for (const locale of ["zh", "en"]) {
      for (const zoom of [1, 1.5, 2]) {
        for (const { width, grid } of [{ width: 240, grid: false }, { width: 280, grid: false }, { width: 320, grid: false }, { width: 420, grid: false }, { width: 960, grid: true }, { width: 1280, grid: true }]) {
          currentCase = `traffic-stats-${theme}-${locale}-${grid ? "grid" : "card"}-${width}-zoom-${zoom}`;
          const saveRepresentative = async name => {
            // 成功截图上限12张：8张主题/语言×280卡/960网格，4张zoom2缺数据/大数值。
            const regular = zoom === 1 && width === (grid ? 960 : 280) && name === "classified";
            const zoomed = zoom === 2 && !grid && width === 280 && name === (locale === "zh" ? "large-values" : "unavailable");
            if (regular || zoomed) {
              await currentPage.screenshot({ path: `test-results/${currentCase}-${name}.png`, fullPage: true });
              screenshots++;
            }
          };
          const page = await browser.newPage({ viewport: { width: Math.ceil((width + 32) * zoom), height: Math.ceil(950 * zoom) } });
          currentPage = page;
          const errors = [];
          page.on("pageerror", error => errors.push(error.message));
          page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
          await page.route("**/*", route => {
            const url = new URL(route.request().url());
            return url.origin === base.origin ? route.continue() : route.abort();
          });
          const params = new URLSearchParams({ theme, locale, zoom: String(zoom), width: String(width), layout: grid ? "grid" : "card" });
          await page.goto(`${base.origin}/scripts/ui-smoke/traffic-stats.html?${params}`);
          await page.waitForFunction(() => !!window.trafficFixture);
          await page.waitForFunction(({ label }) => document.querySelector("thead th")?.textContent === label, { label: labels[locale].columns[0] });
          await page.evaluate(() => document.fonts.ready);
          assert.equal(await page.locator("html").getAttribute("data-theme"), theme);
          assert.equal(await page.evaluate(() => Number(getComputedStyle(document.documentElement).zoom)), zoom);
          const originalWidths = await page.locator("[data-neighbor]").evaluateAll(elements => elements.map(element => element.getBoundingClientRect().width));

          for (const name of ["missing", "legacy", "unavailable"]) {
            await scenario(page, name);
            const total = name === "missing" ? missing : name === "legacy" ? ["1.0 KB/s", "512 B/s", "8.0 KB", "4.0 KB"] : expected.total;
            await expectRows(page, { proxy: missing, direct: missing, total }, locale);
            await assertTooltip(page, "unavailable", locale);
            await assertLayout(page, zoom, grid);
          }
          await saveRepresentative("unavailable");

          for (const source of ["connections", "outbound"]) {
            await scenario(page, source);
            await expectRows(page, expected, locale);
            await assertReconciled(page);
            await assertTooltip(page, source, locale);
            await assertLayout(page, zoom, grid);
          }
          await scenario(page, "connections");
          await saveRepresentative("classified");

          await scenario(page, "three");
          await expectRows(page, { proxy: expected.proxy, direct: expected.direct, total: ["12.0 KB/s", "3.0 KB/s", "96.0 KB", "48.0 KB"] }, locale);
          await assertReconciled(page);
          await assertLayout(page, zoom, grid);

          await scenario(page, "stopped");
          const stoppedRows = Object.fromEntries(Object.entries(expected).map(([id, values]) => [id, ["0 B/s", "0 B/s", ...values.slice(2)]]));
          await expectRows(page, stoppedRows, locale);
          await assertReconciled(page);
          await assertLayout(page, zoom, grid);

          await scenario(page, "reset");
          await expectRows(page, { proxy: zero, direct: zero, total: zero }, locale);
          await assertReconciled(page);
          await assertLayout(page, zoom, grid);

          await scenario(page, "huge");
          await assertReconciled(page);
          await assertLayout(page, zoom, grid);
          assert.equal(await page.locator('[data-traffic-row="unclassified"]').count(), 1);
          await saveRepresentative("large-values");
          assert.deepEqual(await page.locator("[data-neighbor]").evaluateAll(elements => elements.map(element => element.getBoundingClientRect().width)), originalWidths, "分类与缺数据切换不改变相邻卡片列宽");

          // 保留原流量页入口，使用键盘和鼠标都只打开一次，不改变 hash。
          const link = page.getByRole("link", { name: labels[locale].link, exact: true });
          await link.focus();
          await page.keyboard.press("Enter");
          await page.waitForFunction(() => window.trafficFixture.opens === 1);
          await link.click();
          await page.waitForFunction(() => window.trafficFixture.opens === 2);
          await page.locator('[data-traffic-row="proxy"] [data-period="live"]').click();
          await page.waitForFunction(() => window.trafficFixture.opens === 3);
          assert.equal(new URL(page.url()).hash, "");
          assert.deepEqual(errors, [], currentCase);
          await page.close();
          currentPage = null;
        }
      }
    }
  }
  assert.equal(screenshots, 12, "成功截图仅保留12张关键代表");
  console.log("流量卡回归通过：72组主题/语言/缩放/宽度组合，缺数据、三/四类行、双来源tooltip、双向对账、停止/重置、280/320窄卡、三列相邻卡片；关键截图12张。");
} catch (error) {
  if (currentPage && !currentPage.isClosed()) {
    await currentPage.screenshot({ path: `test-results/${currentCase}-failure.png`, fullPage: true }).catch(() => {});
  }
  throw error;
} finally {
  await browser.close();
}

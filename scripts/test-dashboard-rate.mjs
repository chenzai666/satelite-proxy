import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import ts from "typescript";
import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import { formatTrafficValue } from "../src/trafficStats.ts";
import { translate } from "../src/i18n/messages.ts";

// Execute the real TSX in Node; only the locale hook and CSS import need adapters.
const filename = new URL("../src/components/DashboardRateCard.tsx", import.meta.url);
const output = ts.transpileModule(readFileSync(filename, "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2020 },
}).outputText;
const require = createRequire(filename);

function component(locale = "zh") {
  const module = { exports: {} };
  const load = name => {
    if (name === "./DashboardRateCard.css") return {};
    if (name === "../i18n") return { useI18n: () => ({ t: (key, vars) => translate(locale, key, vars) }) };
    if (name === "../trafficStats") return { formatTrafficValue };
    return require(name);
  };
  new Function("require", "module", "exports", output)(load, module, module.exports);
  return module.exports.DashboardRateCard;
}

const props = { direction: "down", speed: 2048, total: 8192, samples: [1024, 4096, 0], running: true };
const render = (overrides = {}, locale = "zh") => renderToStaticMarkup(createElement(component(locale), { ...props, ...overrides }));
const rateText = html => html.match(/class="dashboard-rate-card__rate" title="([^"]+)"/)[1];
const stats = html => [...html.matchAll(/<dd>([^<]+)<\/dd>/g)].map(match => match[1]);
const line = html => html.match(/class="dashboard-rate-card__line" d="([^"]+)"/)[1];

test("运行、空闲和停止只改变当前速率，保留累计与近期峰值", () => {
  assert.equal(rateText(render()), "2.0 KB/s");
  for (const overrides of [{ speed: 0 }, { running: false }]) {
    const html = render(overrides);
    assert.equal(rateText(html), "0 B/s");
    assert.deepEqual(stats(html), ["8.0 KB", "4.0 KB/s"]);
    assert.equal(line(html), line(render()));
  }
});

test("仅最后60点参与峰值和折线，不用当前速度代替采样峰值且不修改输入", () => {
  const samples = [Number.MAX_VALUE, ...Array.from({ length: 60 }, (_, index) => index)];
  const before = [...samples];
  const html = render({ speed: 1024 ** 4, samples });
  assert.deepEqual(stats(html), ["8.0 KB", "59 B/s"]);
  assert.equal((line(html).match(/[ML]/g) || []).length, 60);
  assert.deepEqual(samples, before);
});

test("非法值归零并保持采样位置，极大有限值仍生成有效坐标", () => {
  for (const invalid of [NaN, Infinity, -Infinity, -1]) {
    const html = render({ speed: invalid, total: invalid, samples: [1, invalid, 2] });
    assert.equal(rateText(html), "0 B/s");
    assert.deepEqual(stats(html), ["0 B", "2 B/s"]);
    assert.equal(line(html), "M6.00 32.00 L150.00 58.00 L294.00 6.00");
  }
  const huge = render({ samples: [0, Number.MAX_VALUE, Number.MAX_VALUE / 2] });
  assert.equal(line(huge), "M6.00 58.00 L150.00 6.00 L294.00 32.00");
});

test("无采样、全零与单点都有可见且有限的SVG路径", () => {
  assert.equal(line(render({ samples: [] })), "M6 58 L294 58");
  assert.equal(line(render({ samples: [0, 0] })), "M6.00 58.00 L294.00 58.00");
  const single = render({ samples: [128] });
  assert.equal(line(single), "M294.00 6.00");
  assert.match(single, /<circle cx="294" cy="6"/);
  assert.deepEqual(stats(render({ samples: [] })), ["8.0 KB", "0 B/s"]);
});

test("可操作标题使用原生button，无回调时使用标题且不占用键盘焦点", () => {
  const card = component();
  let opened = 0;
  const onOpen = () => opened++;
  const interactive = render({ onOpen });
  assert.match(interactive, /<button[^>]*type="button"[^>]*aria-label="查看下载流量详情"/);
  // The TSX tree exposes the actual native handler without browser simulation.
  const tree = card({ ...props, onOpen });
  const button = tree.props.children[0].props.children;
  assert.equal(button.type, "button");
  button.props.onClick();
  assert.equal(opened, 1);
  const passive = render();
  assert.match(passive, /<h3 class="dashboard-rate-card__title"/);
  assert.doesNotMatch(passive, /<button|tabindex|role="button"/);
});

test("上传下载与辅助文字随中英文翻译，峰值明确属于近期采样", () => {
  for (const [locale, up, down, peak] of [["zh", "上传", "下载", "近期峰值"], ["en", "Upload", "Download", "Recent peak"]]) {
    for (const [direction, label] of [["up", up], ["down", down]]) {
      const html = render({ direction, onOpen: () => {} }, locale);
      assert.ok(html.includes(label));
      assert.ok(html.includes(peak));
      for (const key of ["dashboard.ratePeakHint", "dashboard.rateHistory", "dashboard.rateOpen"]) {
        assert.ok(html.includes(translate(locale, key, { direction: label })));
      }
      assert.doesNotMatch(html, /dashboard\.rate/);
    }
  }
});

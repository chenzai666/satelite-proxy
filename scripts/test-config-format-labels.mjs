import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";
import { translate } from "../src/i18n/messages.ts";

// 从真实 TSX 提取纯展示函数；不加载页面依赖，不启动浏览器或 Tauri。
const source = ts.createSourceFile(
  "ConfigPage.tsx",
  readFileSync(new URL("../src/pages/ConfigPage.tsx", import.meta.url), "utf8"),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const labelFunction = source.statements.find(
  node => ts.isFunctionDeclaration(node) && node.name?.text === "typeTagLabel",
);
assert.ok(labelFunction, "配置类型标签函数应存在");
const { outputText } = ts.transpileModule(labelFunction.getText(source), {
  compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
});
const typeTagLabel = runInNewContext(`${outputText}\ntypeTagLabel;`);

// 同时执行支持报告原有的内核命名，避免格式标签污染内核名称。
let coreNameExpression;
function visit(node) {
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === "coreName") {
    coreNameExpression = node.initializer;
  }
  ts.forEachChild(node, visit);
}
visit(source);
assert.ok(coreNameExpression, "内核支持报告命名函数应存在");
const coreNameJs = ts.transpileModule(`(${coreNameExpression.getText(source)})`, {
  compilerOptions: { target: ts.ScriptTarget.ES2020, module: ts.ModuleKind.CommonJS },
}).outputText;

for (const locale of ["zh", "en"]) {
  const t = key => translate(locale, key);
  test(`${locale}：普通订阅按源格式显示，忽略无关 custom_kind`, () => {
    for (const source_kind of ["url", "file", "text"]) {
      for (const [format, expected] of [
        ["clash_yaml", "Clash YAML"],
        ["singbox_json", "sing-box JSON"],
        ["uri_list", "URI"],
        ["base64_uri_list", "Base64 URI"],
      ]) {
        const item = Object.freeze({ source_kind, format, custom_kind: "xray" });
        assert.equal(typeTagLabel(t, item), expected);
      }
    }
  });

  test(`${locale}：自定义配置优先显示真实内核，保留旧数据兜底`, () => {
    for (const [custom_kind, expected] of [
      ["mihomo", "mihomo"], ["singbox", "sing-box"], ["xray", "Xray"],
      [undefined, "sing-box"], [null, "sing-box"],
    ]) {
      for (const format of ["clash_yaml", "singbox_json", undefined]) {
        assert.equal(typeTagLabel(t, { source_kind: "custom", custom_kind, format }), expected);
      }
    }
  });

  test(`${locale}：手动配置及缺失或未知格式保留无标签行为`, () => {
    for (const source_kind of ["node", "url", "file", "text"]) {
      for (const format of [undefined, null, "", "future_format"]) {
        assert.equal(typeTagLabel(t, { source_kind, format }), null);
      }
    }
  });

  test(`${locale}：内核支持报告仍使用内核名称`, () => {
    const coreName = runInNewContext(coreNameJs, { t });
    for (const [core, expected] of [["singbox", "sing-box"], ["mihomo", "mihomo"], ["xray", "Xray"]]) {
      assert.equal(coreName(core), expected);
    }
  });
}

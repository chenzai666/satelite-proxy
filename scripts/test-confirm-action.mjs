import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import test from "node:test";
import ts from "typescript";

const source = await readFile("src/confirmAction.ts", "utf8");
const plugin = pathToFileURL(path.resolve("node_modules/@tauri-apps/plugin-dialog/dist-js/index.js")).href;
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText.replace('"@tauri-apps/plugin-dialog"', JSON.stringify(plugin));
const { confirmAction } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`);

test("确认弹窗必须等待用户结果、走 message 命令且失败时取消", async () => {
  const calls = [];
  let resolveDialog;
  globalThis.window = { __TAURI_INTERNALS__: { invoke: (command, args) => {
    calls.push({ command, args });
    return new Promise(resolve => { resolveDialog = resolve; });
  } } };
  try {
    let finished = false;
    const first = confirmAction("删除测试节点？").then(value => { finished = true; return value; });
    await Promise.resolve();
    assert.equal(finished, false);
    assert.equal(await confirmAction("重复点击"), false);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].command, "plugin:dialog|message");
    resolveDialog("Cancel");
    assert.equal(await first, false);

    const accepted = confirmAction("再次确认");
    resolveDialog("Ok");
    assert.equal(await accepted, true);

    const originalError = console.error;
    console.error = () => {};
    try {
      window.__TAURI_INTERNALS__.invoke = async () => { throw new Error("ACL denied"); };
      assert.equal(await confirmAction("弹窗失败"), false);
    } finally { console.error = originalError; }
    window.__TAURI_INTERNALS__.invoke = async () => "Ok";
    assert.equal(await confirmAction("失败后重试"), true);
  } finally { delete globalThis.window; }
});

test("所有页面确认操作都显式 await，禁止全局 confirm 回归", async () => {
  async function scan(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory()) { await scan(filename); continue; }
      if (!/\.tsx?$/.test(filename) || entry.name === "confirmAction.ts") continue;
      const file = ts.createSourceFile(filename, await readFile(filename, "utf8"), ts.ScriptTarget.Latest, true);
      function visit(node) {
        if (ts.isCallExpression(node)) {
          const expression = node.expression.getText(file);
          assert.ok(!["confirm", "window.confirm", "globalThis.confirm"].includes(expression), filename);
          if (expression === "confirmAction") {
            assert.ok(ts.isAwaitExpression(node.parent), `确认调用未等待：${filename}`);
          }
        }
        ts.forEachChild(node, visit);
      }
      visit(file);
    }
  }
  await scan("src");
});

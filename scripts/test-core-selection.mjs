import { readFile } from "node:fs/promises";
import assert from "node:assert/strict";
import { test } from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/coreSelection.ts", import.meta.url), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
const { protocolDefaultValue, updateProtocolDefault } = await import(`data:text/javascript;base64,${Buffer.from(js).toString("base64")}`);

test("protocol defaults retain unrelated rows and auto removes only the selected protocol", () => {
  const rows = [{ protocol: "vless", core: "xray" }, { protocol: "tuic", core: "singbox" }];
  assert.equal(protocolDefaultValue(rows, "vless"), "xray");
  assert.equal(protocolDefaultValue(rows, "vmess"), "auto");
  assert.deepEqual(updateProtocolDefault(rows, "vless", "auto"), [rows[1]]);
  assert.deepEqual(updateProtocolDefault(rows, "vless", "mihomo"), [rows[1], { protocol: "vless", core: "mihomo" }]);
  assert.deepEqual(rows[0], { protocol: "vless", core: "xray" });
});

test("unknown persisted core is displayed as auto without changing other rows", () => {
  const rows = [{ protocol: "vless", core: "future" }, { protocol: "tuic", core: "singbox" }];
  assert.equal(protocolDefaultValue(rows, "vless"), "auto");
  assert.deepEqual(updateProtocolDefault(rows, "tuic", "mihomo")[0], rows[0]);
});

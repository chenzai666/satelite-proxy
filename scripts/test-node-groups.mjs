import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { mergeUnsupportedIds } from "../src/nodeLatencyState.ts";

test("分组重测仅替换本组标记，并保持输入集合不变", () => {
  const previous = new Set(["group-a-1", "group-a-2", "group-b-1"]);
  const next = mergeUnsupportedIds(previous, ["group-a-1", "group-a-2"], [
    { id: "group-a-1", method: "clash_api" },
    { id: "group-a-2", method: "unsupported" },
  ]);
  assert.deepEqual([...next].sort(), ["group-a-2", "group-b-1"]);
  assert.equal(previous.size, 3);
});

test("流式回包可清除旧标记，空结果不影响其他节点", () => {
  assert.deepEqual([...mergeUnsupportedIds(new Set(["old"]), ["old"], [])], []);
  assert.deepEqual([...mergeUnsupportedIds(new Set(["other"]), [], [])], ["other"]);
  assert.deepEqual([...mergeUnsupportedIds(new Set(["old"]), ["old"], [
    { id: "old", method: "tcp" },
  ])], []);
});

test("分组按钮复用单点批量测速并隔离折叠事件，独立订阅列保留最小宽度", () => {
  const page = readFileSync(new URL("../src/pages/NodesPage.tsx", import.meta.url), "utf8");
  assert.match(page, /event\.stopPropagation\(\);\s*void onTestNodes\(item\.nodes\.map/);
  assert.match(page, /customRuntime \|\| item\.nodes\.length === 0/);
  assert.match(page, /className="node-list-sub"/);
  assert.match(page, /minmax\(260px,3fr\) minmax\(160px,1\.4fr\)/);
  assert.match(page, /setTestingGroupKey\(null\)/);
});

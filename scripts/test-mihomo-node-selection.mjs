import assert from "node:assert/strict";
import test from "node:test";
import { mihomoNodeSelections } from "../src/mihomoNodeSelection.ts";

const nodes = [{ id: "a" }, { id: "b" }, { id: "stale" }];
const group = (name, now, all = [now], labels) => ({ name, now, all, labels, group_type: "Selector" });

test("multiple groups mark different nodes without a legacy current-node fallback", () => {
  const selected = mihomoNodeSelections([
    group("proxy", "node-a", undefined, { proxy: "🚀 节点选择" }),
    group("AI", "node-b"), group("China", "DIRECT"), group("Ads", "REJECT"),
  ], nodes);
  assert.deepEqual([...selected], [["a", ["🚀 节点选择"]], ["b", ["AI"]]]);
  assert.equal(selected.has("stale"), false);
});

test("nested and automatic groups follow the live final node and retain group names", () => {
  const groups = [group("AI", "proxy"), group("proxy", "auto"), group("auto", "node-a", ["node-a", "node-b"])];
  assert.deepEqual(mihomoNodeSelections(groups, nodes).get("a"), ["AI", "proxy", "auto"]);
  groups[2].now = "node-b";
  const next = mihomoNodeSelections(groups, nodes);
  assert.equal(next.has("a"), false);
  assert.deepEqual(next.get("b"), ["AI", "proxy", "auto"]);
});

test("empty, direct, missing, cyclic and invalid selections never invent a node", () => {
  for (const groups of [[], [group("proxy", "Satelite-DIRECT")], [group("proxy", "missing")],
    [group("A", "B"), group("B", "A")], [group("proxy", "node-a", ["node-b"])],
    [group("proxy", "", ["node-a"])], [group("proxy", "same name")]]) {
    assert.equal(mihomoNodeSelections(groups, nodes).size, 0);
  }
});

test("same display names cannot match a different node and ambiguous tags are rejected", () => {
  const sameNameNodes = [{ id: "a", name: "same name" }, { id: "b", name: "same name" }];
  assert.deepEqual([...mihomoNodeSelections([group("AI", "node-b")], sameNameNodes)], [["b", ["AI"]]]);
  const prefix = "1234567890abcdef";
  assert.equal(mihomoNodeSelections([group("AI", `node-${prefix}`)], [{ id: `${prefix}1` }, { id: `${prefix}2` }]).size, 0);
});

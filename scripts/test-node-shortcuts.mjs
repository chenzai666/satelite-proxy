import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { shouldDeleteSelectedNodes } from "../src/nodeShortcuts.ts";

const key = { key: "Delete", repeat: false, defaultPrevented: false,
  ctrlKey: false, metaKey: false, altKey: false, shiftKey: false };
const context = { editing: false, dialogOpen: false, readOnly: false, busy: false, selectedCount: 2 };

test("Delete deletes selected nodes, never Backspace or modified/repeated keys", () => {
  assert.equal(shouldDeleteSelectedNodes(key, context), true);
  for (const field of ["repeat", "defaultPrevented", "ctrlKey", "metaKey", "altKey", "shiftKey"]) {
    assert.equal(shouldDeleteSelectedNodes({ ...key, [field]: true }, context), false, field);
  }
  for (const value of ["Backspace", "a", "Enter"]) {
    assert.equal(shouldDeleteSelectedNodes({ ...key, key: value }, context), false);
  }
});

test("Delete ignores editing, dialogs, custom configs, busy and empty selections", () => {
  for (const field of ["editing", "dialogOpen", "readOnly", "busy"]) {
    assert.equal(shouldDeleteSelectedNodes(key, { ...context, [field]: true }), false, field);
  }
  assert.equal(shouldDeleteSelectedNodes(key, { ...context, selectedCount: 0 }), false);
});

test("node page uses the same confirmed deletion for Delete and other actions", () => {
  const page = readFileSync(new URL("../src/pages/NodesPage.tsx", import.meta.url), "utf8");
  assert.match(page, /shouldDeleteSelectedNodes\(event,/);
  assert.match(page, /void deleteSelected\(ids\)/);
  assert.match(page, /deletePendingRef\.current = true;[\s\S]*?await confirmAction/);
  assert.match(page, /if \(!confirmed\) return;[\s\S]*?await deleteNodes\(ids\)/);
  assert.match(page, /finally \{\s*deletePendingRef\.current = false/);
});

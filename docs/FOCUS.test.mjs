// Run: node --test docs/FOCUS.test.mjs
// Speed fix: Now holds only ready rows in focus order (width-30 waves).
// Fails on revert: restoring the 128-id Now (done LJ1/EG19 plus 43 waiting
// rows) fails the count and the all-ready assertions below.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  archivedDoneRows,
  focusNowIds,
  loadBoardText,
  parseBoard,
  rowReady,
} from "../tools/board-slice.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FOCUS_PATH = join(ROOT, "docs", "FOCUS.md");

function focusText() {
  return readFileSync(FOCUS_PATH, "utf8").replace(/\r\n/g, "\n");
}

function nowIds() {
  return focusNowIds(focusText());
}

function board() {
  return parseBoard(loadBoardText(), archivedDoneRows());
}

test("Now holds the ready-only count, not the 128-id overload", () => {
  const ids = nowIds();
  assert.equal(ids.length, 83, `Now holds ${ids.length} ids, want 83 ready-only (was 128)`);
  assert.match(focusText(), /Now holds 83 ids/, "count line names 83 (revert to 128 and this goes red)");
  assert.doesNotMatch(focusText(), /Now holds 128 ids/, "stale 128-id count is gone");
});

test("every Now id is open and ready (no done, no waiting)", () => {
  const { tasks } = board();
  const ids = nowIds();
  for (const id of ids) {
    const t = tasks.get(id);
    assert.ok(t, `${id} is on the board`);
    assert.ok(!t.done, `${id} is ticked/done and must leave Now (revert LJ1/EG19 and this goes red)`);
    assert.ok(rowReady(tasks, t), `${id} waits and must leave Now (revert waiting rows and this goes red)`);
  }
});

test("ticked rows stay out of Now", () => {
  const ids = nowIds();
  assert.ok(!ids.includes("LJ1"), "LJ1 is done and stays out (restore it and this goes red)");
  assert.ok(!ids.includes("EG19"), "EG19 is done and stays out (restore it and this goes red)");
});

test("waiting chains stay out of Now (ready focus order)", () => {
  const ids = nowIds();
  for (const id of ["SP63b", "SP63c", "EG11b", "EG11c", "Q1", "GP38", "HS32", "EG46b"]) {
    assert.ok(!ids.includes(id), `${id} waits and stays out (restore it and this goes red)`);
  }
});

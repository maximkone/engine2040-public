// Run: node --test docs/FOCUS.test.mjs
// FOCUS "## Now" is the work order: `board-slice --focus` reads only that
// section, so every numbered focus item must sit inside it, and every id it
// names must exist on the board.
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
} from "../tools/board-slice.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FOCUS_PATH = join(ROOT, "docs", "FOCUS.md");

function focusText() {
  return readFileSync(FOCUS_PATH, "utf8").replace(/\r\n/g, "\n");
}

// Ids on every numbered "N. **Title** ... Rows: ..." line, wherever it sits.
function numberedItemIds(text) {
  const ids = [];
  for (const line of text.split("\n")) {
    if (!/^\d+\. \*\*/.test(line)) continue;
    const m = /\bRows:\s*(.+)$/.exec(line);
    if (!m) continue;
    for (const tok of m[1].split(/[\s,;]+/)) {
      const id = tok.replace(/[.)]+$/, "");
      if (/^[A-Z][A-Z0-9]*\d[a-z]?$/.test(id)) ids.push(id);
    }
  }
  return ids;
}

test("Now names at least one row", () => {
  assert.ok(focusNowIds(focusText()).length > 0, "FOCUS.md ## Now has no Rows: ids");
});

test("every numbered focus item sits inside ## Now, so --focus sees its rows", () => {
  const text = focusText();
  const now = new Set(focusNowIds(text));
  const outside = numberedItemIds(text).filter((id) => !now.has(id));
  assert.deepEqual(outside, [], `ids on numbered items outside ## Now: ${outside.join(" ")}`);
});

// Ticked ids are harmless (--focus prints ready rows only); unknown ids are typos.
test("every Now id is a board row", () => {
  const { tasks } = parseBoard(loadBoardText(), archivedDoneRows());
  const unknown = focusNowIds(focusText()).filter((id) => !tasks.get(id));
  assert.deepEqual(unknown, [], `ids not on the board: ${unknown.join(" ")}`);
});

test("no stale pointers to removed loop tools", () => {
  const text = focusText();
  for (const gone of ["drift-check.mjs", "steal-next.mjs", "PARKED_SECTIONS", "C:\\empire"]) {
    assert.ok(!text.includes(gone), `FOCUS.md still names ${gone}`);
  }
});

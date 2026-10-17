// tools/ladder-split.test.mjs: LADDER.md single-copy gate.
// One copy only: docs/research/LADDER.md (seats read one rung by id: Grep `^\| <ID> `).
// The ladder.d split copy was removed 2026-10-10: it was 1,177 bytes stale.
// Run: node --test tools/ladder-split.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
const REPO = process.cwd();
const SRC = join(REPO, "docs", "research", "LADDER.md");
const PART_DIR = join(REPO, "docs", "research", "ladder.d");

test("no split copy next to LADDER.md", () => {
  assert.equal(existsSync(PART_DIR), false, "ladder.d must not duplicate LADDER.md");
});

test("LADDER.md is non-empty with expected headings", () => {
  const st = statSync(SRC);
  assert.ok(st.size > 0, `LADDER.md non-empty (size=${st.size})`);
  const text = readFileSync(SRC, "utf8");
  assert.match(text, /^# Research ladder/m, "title heading present");
  assert.match(text, /^## Pillar cursor/m, "pillar heading present");
  assert.match(text, /^## Lines/m, "lines heading present");
});

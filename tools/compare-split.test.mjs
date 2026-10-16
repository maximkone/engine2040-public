// tools/compare-split.test.mjs: COMPARE.md single-copy gate.
// One copy only: docs/research/COMPARE.md (docs name it). The compare.d split
// copy was removed 2026-10-10 because it was byte-equal to COMPARE.md.
// Run: node --test tools/compare-split.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const REPO = process.cwd();
const SRC = join(REPO, "docs", "research", "COMPARE.md");
const PART_DIR = join(REPO, "docs", "research", "compare.d");

test("no split copy next to COMPARE.md", () => {
  assert.equal(existsSync(PART_DIR), false, "compare.d must not duplicate COMPARE.md");
});

test("COMPARE.md is non-empty with expected headings", () => {
  const st = statSync(SRC);
  assert.ok(st.size > 0, "COMPARE.md non-empty");
  const text = readFileSync(SRC, "utf8");
  assert.match(text, /^## Lookup index/m, "lookup heading present");
  assert.match(text, /^# Engine comparison/m, "title heading present");
});

// Run: node --test tools/plan-atlas-cache.test.mjs
// plan-atlas parsers and the single-flight plan read (E51, 2026-10-20: the
// plan file cache is module wide, so a caller cannot make the tool reread a
// plan). The CLI tests live in tools/plan-atlas.test.mjs.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { cratesIn, headingAbove, parsePrefixTable, parseVisionPlansMap, planText, planTextCache, prefixOf, section7Ids } from "./plan-atlas.mjs";

test("the VISION plans table reads into file, serves and notes", () => {
  const rows = parseVisionPlansMap([
    "| Plan | Serves | Notes |",
    "|---|---|---|",
    "| `docs/plan/render-look.md` | the look | EB-2026-10-08 |",
    "| not a plan row | x | y |",
  ].join("\n"));
  assert.equal(rows.length, 1);
  assert.equal(rows[0].file, "docs/plan/render-look.md");
  assert.equal(rows[0].serves, "the look");
  assert.equal(rows[0].notes, "EB-2026-10-08");
});

test("the plan index table maps prefixes to plan files and back", () => {
  const { fileToPrefixes, prefixToFiles } = parsePrefixTable([
    "| Row | File | Prefixes |",
    "|---|---|---|",
    "| a | `docs/plan/render-look.md` | `RL` |",
    "| b | `docs/plan/runtime.md` | `MV`, `GA` |",
    "| c | `docs/plan/no-prefix.md` | - |",
  ].join("\n"));
  assert.deepEqual([...fileToPrefixes.keys()], ["docs/plan/render-look.md", "docs/plan/runtime.md"]);
  assert.deepEqual(fileToPrefixes.get("docs/plan/runtime.md"), ["MV", "GA"]);
  assert.deepEqual(prefixToFiles.get("GA"), ["docs/plan/runtime.md"]);
});

test("prefixOf reads the row prefix", () => {
  assert.equal(prefixOf("RL25"), "RL");
  assert.equal(prefixOf("GA12b"), "GA");
  assert.equal(prefixOf("not a row"), "");
});

test("cratesIn finds the crates a plan names, sorted and once each", () => {
  const found = cratesIn("crates/render holds it; crates/assets too; crates/render again");
  assert.deepEqual(found, ["assets", "render"]);
});

test("section7Ids reads the board rows of section 7 only", () => {
  const ids = section7Ids(["## 6. Other", "- [ ] XX1 not mine", "## 7. Board tasks", "- [ ] RL25 first", "  more text", "- [ ] GA3 second", "## 8. Later"].join("\n"));
  assert.deepEqual(ids, ["RL25", "GA3"]);
});

test("headingAbove finds the heading that owns a line", () => {
  const lines = ["# Top", "## Middle", "text", "### Deep", "more"];
  assert.equal(headingAbove(lines, 2), "Middle");
  assert.equal(headingAbove(lines, 4), "Deep");
  assert.equal(headingAbove(["plain"], 0), "(top)");
});

test("planText reads a plan file once: the cache is module wide (E51, no reread)", () => {
  const dir = mkdtempSync(join(tmpdir(), "plan-atlas-"));
  const before = planTextCache.size;
  try {
    writeFileSync(join(dir, "one.md"), "first text\n");
    assert.equal(planText(dir, "one.md"), "first text\n");
    // The file changes on disk: a reread would see the new bytes.
    writeFileSync(join(dir, "one.md"), "second text\n");
    assert.equal(planText(dir, "one.md"), "first text\n", "the second call must come from the cache");
    assert.equal(planTextCache.size, before + 1, "one entry for one plan file");
    assert.equal(planText(dir, "one.md", new Map()), "second text\n", "an explicit cache is the caller's own");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// Run: node --test docs/SPRINT.test.mjs
// Speed fix: board stays slice-readable, wrapper-first, snapshot-free, under the size cap.
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BOARD_PATH = join(ROOT, "docs", "SPRINT.md");
const DOCS_DIR = join(ROOT, "docs");

function loadUnion() {
  let text = readFileSync(BOARD_PATH, "utf8");
  for (const n of readdirSync(DOCS_DIR)
    .filter((f) => f.startsWith("SPRINT-") && f.endsWith(".md") && f !== "SPRINT-ARCHIVE.md")
    .sort()) {
    text += `\n${readFileSync(join(DOCS_DIR, n), "utf8")}`;
  }
  return text;
}

test("board enforces slice reads in its first lines", () => {
  const head = readFileSync(BOARD_PATH, "utf8").split("\n").slice(0, 6).join("\n");
  assert.match(head, /board-slice\.mjs --ready/, "header names the slice command");
  assert.match(head, /Never read this file whole/, "header forbids full-board reads");
});

test("board header carries the one-line ready+row wrapper", () => {
  const head = readFileSync(BOARD_PATH, "utf8").split("\n").slice(0, 6).join("\n");
  assert.match(
    head,
    /board-slice\.mjs --ready; node tools\/board-slice\.mjs <ID>/,
    "header has the ready+row one-liner (revert this and the test goes red)",
  );
});

test("board carries no stale ready snapshot", () => {
  const text = readFileSync(BOARD_PATH, "utf8");
  assert.doesNotMatch(text, /Top 12 ready now/, "timestamped Top-12 snapshot is gone");
  assert.doesNotMatch(text, /### Ready \(\d+ rows,/, "timestamped Ready-count snapshot is gone");
});

test("board is smaller than the pre-fix 608000 bytes", () => {
  const size = statSync(BOARD_PATH).size;
  assert.ok(size < 605000, `board ${size} bytes should be under 605000 (was 608000)`);
});

test("board documents the sprint-loop width cap 1..30", () => {
  const text = readFileSync(BOARD_PATH, "utf8");
  assert.match(text, /sprint-loop\.mjs --width 1\.\.30/, "doc names the 1..30 cap (revert this and the test goes red)");
  assert.doesNotMatch(text, /sprint-loop\.mjs --width 1\.\.60/, "stale 1..60 cap is gone");
  assert.match(text, /knob width \(30 now/, "doc knob width is 30 (revert this and the test goes red)");
  assert.doesNotMatch(text, /knob width \(60 now/, "stale 60-wide knob is gone");
});

test("old verbose 2026-10-03 receipt bodies are archived to git history", () => {
  const union = loadUnion();
  assert.doesNotMatch(union, /cargo-mutants 27\.1\.0 `--in-diff`/, "verbose CQ11 body is gone");
  assert.match(union, /Archived to git history/, "pruned rows point at git history");
  assert.match(union, /## Receipts/, "receipts section still exists");
  const index = readFileSync(BOARD_PATH, "utf8");
  assert.match(index, /SPRINT-19-receipts\.md/, "index links the receipts shard");
});

test("board index stays slice-thin: no shard-header duplicates or snapshot stamp", () => {
  const index = readFileSync(BOARD_PATH, "utf8");
  assert.doesNotMatch(index, /snapshot from 2026/, "stale shard-list snapshot stamp is gone (revert this and the test goes red)");
  assert.doesNotMatch(
    index,
    /2026-10-02 restructure/,
    "shard header duplicate is gone from the index; the canonical title lives in the shard (revert this and the test goes red)",
  );
  const size = statSync(BOARD_PATH).size;
  assert.ok(size < 8000, `index ${size} bytes should stay under 8000 (was 8716 before the dedup)`);
});

test("viewer lane stays pruned (Bevy viewer dropped, zero ready rows)", () => {
  const index = readFileSync(BOARD_PATH, "utf8");
  assert.doesNotMatch(index, /crates\/viewer/, "viewer lane is gone from the active table (revert this and the test goes red)");
  assert.doesNotMatch(index, /Bevy viewer/, "Bevy viewer label is gone (revert this and the test goes red)");
});

test("board carries no stale width history", () => {
  const index = readFileSync(BOARD_PATH, "utf8");
  assert.doesNotMatch(index, /20-wide/, "stale 20-wide packing note is gone (revert this and the test goes red)");
  assert.doesNotMatch(index, /old 20-cap/, "old-cap history lives in git, not the board (revert this and the test goes red)");
});

test("board index stays under 7600 after the viewer-lane prune", () => {
  const size = statSync(BOARD_PATH).size;
  assert.ok(size < 7600, `index ${size} bytes should stay under 7600 (was 7628 before the prune)`);
});

test("board runs in one tree: lanes are retired (owner 2026-10-10)", () => {
  const index = readFileSync(BOARD_PATH, "utf8");
  assert.doesNotMatch(index, /\(owner 2026-10-03\)/, "the old lane-cap owner stamp is gone");
  assert.doesNotMatch(index, /Cap 10 live lanes/, "the live-lane cap is gone (revert this and the test goes red)");
  assert.doesNotMatch(index, /lanes\.mjs (create|prune|diff)/, "no lane tool step in the wave flow (revert this and the test goes red)");
  assert.doesNotMatch(index, /git apply --3way/, "no collected-patch landing step (revert this and the test goes red)");
  assert.match(index, /no lanes, no\s+worktrees/, "the one-tree rule is stated (revert this and the test goes red)");
  assert.match(index, /git restore --source=HEAD -- <owned paths>/, "abandon command is stated (revert this and the test goes red)");
});

test("board index stays under 7520 after the lane-cap dedup", () => {
  const size = statSync(BOARD_PATH).size;
  assert.ok(size < 7520, `index ${size} bytes should stay under 7520 (was 7524 before the dedup)`);
});

test("board enforces two-strike BLOCKED plus BLOCKERS row", () => {
  const text = readFileSync(BOARD_PATH, "utf8");
  assert.match(text, /Second identical failure ends it/, "two-strike rule names the second failure (revert this and the test goes red)");
  assert.match(text, /BLOCKED:/, "two-strike rule names BLOCKED text (revert this and the test goes red)");
  assert.match(text, /BLOCKERS\.md/, "two-strike rule names the BLOCKERS row (revert this and the test goes red)");
  assert.match(text, /never resend whole/, "two-strike rule bans resend-whole (revert this and the test goes red)");
});

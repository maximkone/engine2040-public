// Run: node --test docs/research/LADDER.test.mjs
// Ladder-rebase2: ladder stays rung-readable and under the size cap.
import assert from "node:assert/strict";
import { readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const LADDER_PATH = join(ROOT, "docs", "research", "LADDER.md");

test("ladder enforces rung reads in its first lines", () => {
  const head = readFileSync(LADDER_PATH, "utf8").split("\n").slice(0, 6).join("\n");
  assert.match(head, /Read one rung only/, "header names rung-only reads");
  assert.match(head, /Never read this file whole/, "header forbids full-file reads");
});

test("ladder is smaller than the pre-fix 429783 bytes", () => {
  const size = statSync(LADDER_PATH).size;
  assert.ok(size < 429783, `ladder ${size} bytes should be under 429783 (was 429783)`);
});

test("ladder has fewer lines than the pre-fix 399", () => {
  const lines = readFileSync(LADDER_PATH, "utf8").split("\n").length;
  assert.ok(lines < 399, `ladder ${lines} lines should be under 399 (was 399)`);
});

test("merge history is collapsed with a git-history pointer", () => {
  const text = readFileSync(LADDER_PATH, "utf8");
  assert.match(text, /34 merges collapsed/, "merge collapse note exists");
  assert.match(text, /git log -p -- docs\/research\/LADDER\.md/, "collapsed rows point at git history");
  assert.doesNotMatch(text, /Merge: 2026-10-01T16:41:00Z/, "verbose merge lines are gone");
});

test("open rows still live in the file", () => {
  const text = readFileSync(LADDER_PATH, "utf8");
  assert.match(text, /## Lines/, "lines section still exists");
  assert.match(text, /- CARD ACCEPTED \| P5 \| phased-fighter benchmark/, "open CARD rows stay");
  assert.match(text, /Pillar cursor/, "pillar cursor stays");
});

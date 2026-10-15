// Run: node --test docs/SPRINT-12-tier-2040.test.mjs
// Speed fix speed-202610090814-f02: 52 frozen LATER rows shrink dispatch width.
// D8 (FOCUS 2026-10-06) kills the D7 Bevy gate; RT19 already owns crates/render,
// needs RL6+GX4 ticked, so its D7 freeze lifts. Test fails on the 52-line revert.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const TIER_PATH = join(ROOT, "docs", "SPRINT-12-tier-2040.md");

function tierText() {
  return readFileSync(TIER_PATH, "utf8");
}

function laterCount(text) {
  const m = text.match(/LATER:/g);
  return m ? m.length : 0;
}

function rt19Block(text) {
  const start = text.indexOf("- [ ] RT19 render:");
  assert.ok(start >= 0, "RT19 row exists");
  const next = text.indexOf("- [ ] RT20 render:", start);
  return text.slice(start, next >= 0 ? next : start + 2000);
}

test("tier has fewer than the pre-fix 52 LATER lines", () => {
  const n = laterCount(tierText());
  assert.ok(n < 52, `LATER ${n} should be under pre-fix 52 (revert to 52 and this goes red)`);
});

test("RT19 freeze lifted per D8 (needs met, owns crates/render)", () => {
  const block = rt19Block(tierText());
  assert.doesNotMatch(block, /LATER:/, "RT19 carries no LATER (revert adds it and this goes red)");
  assert.doesNotMatch(
    block,
    /until the first real Bevy frame lands/,
    "dead D7 Bevy gate is gone from RT19",
  );
  assert.match(block, /D8 re-seated at crates\/render/, "RT19 names the D8 home");
});

// W1 stale doc refs pin (read-only sweep 2026-10-09).
// Source: sprint/notes/w1-stale-doc-refs.md items 1-6 (live docs only).
// Items 7-8 (EMPIRE-INBOX-ARCHIVE history) excluded as frozen history.
// Wave-2 fix denied: this file pins the gap, lead applies the one-liners.
// Run: node --test tools/stale-refs.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { existsSync } from "node:fs";
import { join } from "node:path";

const REPO = process.cwd();
const EXPECT_STALE = 6;

// 6 stale lines, 2 distinct missing targets.
const REFS = [
  { from: "docs/VISION.md", line: 320, target: "docs/research/HARD-SYSTEMS-CARDS.md" },
  { from: "docs/plan/hard-systems.md", line: 18, target: "docs/research/HARD-SYSTEMS-CARDS.md" },
  { from: "docs/plan/README.md", line: 169, target: "docs/research/HARD-SYSTEMS-CARDS.md" },
  { from: "docs/plan/steals.md", line: 90, target: "docs/research/HARD-SYSTEMS-CARDS.md" },
  { from: "docs/SPRINT-10-empire-inbox-drain.md", line: 21, target: "docs/research/DISCOVERED.md" },
  { from: "docs/archive/2026-10-10/plans/futures.md", line: 75, target: "docs/research/DISCOVERED.md" },
];

test("stale doc refs pin: gap count unchanged (lead applies one-liners)", () => {
  const stale = REFS.filter((r) => !existsSync(join(REPO, r.target)));
  console.log(`EXPECT-STALE ${EXPECT_STALE} FOUND ${stale.length}`);
  for (const s of stale) console.log(`STALE ${s.from}:${s.line} -> ${s.target}`);
  assert.strictEqual(
    stale.length,
    EXPECT_STALE,
    `gap moved: FOUND ${stale.length}, EXPECT-STALE ${EXPECT_STALE} (fix landed or new stale appeared)`,
  );
});


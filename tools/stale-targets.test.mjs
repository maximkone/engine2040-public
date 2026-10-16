// Pin the two missing stale-ref targets so fixing them breaks loudly.
// Lead: delete this pin when the refs are fixed.
import assert from "node:assert/strict";
import test from "node:test";
import { existsSync } from "node:fs";
import { join } from "node:path";

const REPO = process.cwd();
const TARGETS = [
  "docs/research/HARD-SYSTEMS-CARDS.md",
  "docs/research/DISCOVERED.md",
];

test("stale targets pin: both targets still missing", () => {
  for (const target of TARGETS) {
    assert.deepStrictEqual(existsSync(join(REPO, target)), false);
  }
});

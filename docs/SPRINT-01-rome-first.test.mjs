// Run: node --test docs/SPRINT-01-rome-first.test.mjs
// Speed fix f01: 8 dead viewer BLOCKED lines rot ready scans; rows stay parked via LATER.
// Fails on revert: restoring one "BLOCKED: viewer dropped" line fails the first test.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const SHARD = join(dirname(fileURLToPath(import.meta.url)), "SPRINT-01-rome-first.md");

function shard() {
  return readFileSync(SHARD, "utf8").replace(/\r\n/g, "\n");
}

test("viewer BLOCKED rot is gone (restore one and this goes red)", () => {
  const text = shard();
  assert.doesNotMatch(text, /BLOCKED: viewer dropped/, "8 viewer BLOCKED lines are parked via LATER");
  const n = (text.match(/BLOCKED:/g) || []).length;
  assert.equal(n, 0, `expected 0 BLOCKED lines in this shard, found ${n}`);
});

test("parked viewer rows stay via LATER (no over-prune)", () => {
  const text = shard();
  for (const id of ["BV3", "BV5", "BV6", "BV7", "BV8", "BV9", "BV10", "BV17"]) {
    assert.ok(
      text.includes(`- [ ] ${id} `),
      `${id} row is still present (parked, not deleted)`,
    );
  }
  const later = (text.match(/LATER: parked/g) || []).length;
  assert.ok(later >= 14, `expected at least 14 parked LATER lines (8 fixed + 6 BV11-16), found ${later}`);
  assert.ok(text.includes("crates/viewer gone, rot02 PARK"), "viewer park reason stays");
});

test("BV11-16 parked precedent untouched", () => {
  const text = shard();
  for (const id of ["BV11", "BV12", "BV13", "BV14", "BV15", "BV16"]) {
    assert.ok(text.includes(id), `${id} stays`);
  }
});

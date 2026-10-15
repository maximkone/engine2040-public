// ship-dupe-gap.test.mjs: gap test pinning ship.mjs pack/run duplication.
// Wave-2 dedupe was denied, so this test documents the duplication honestly.
// EXPECT-DUPE 34 is the pack-block top-level def count (1 def site each).
// Shared pack/run names each keep 1 site in pack plus 1 in run until dedupe lands.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
const SHIP = join(dirname(fileURLToPath(import.meta.url)), "ship.mjs");
const EXPECT_DUPE = 34;
function topDefs(section) {
  return section.split("\n").filter((l) => /^(function|const|let|var)\s/.test(l)).map((l) => l.trim().split(/[\s(]/)[1].replace(/[^A-Za-z0-9_$].*$/, ""));
}
test("ship pack block pins EXPECT-DUPE 34 (wave-2 dedupe denied gap)", () => {
  const src = readFileSync(SHIP, "utf8");
  const packStart = src.indexOf("function packEntry");
  const runStart = src.indexOf("function runEntry");
  const tailStart = src.indexOf("function shipUsage");
  assert.ok(packStart !== -1 && runStart > packStart && tailStart > runStart);
  const packDefs = topDefs(src.slice(packStart, runStart));
  const runDefs = topDefs(src.slice(runStart, tailStart));
  assert.equal(packDefs.length, EXPECT_DUPE);
  assert.equal(new Set(packDefs).size, packDefs.length);
  const shared = [...new Set(packDefs)].filter((n) => runDefs.includes(n)).sort();
  assert.ok(shared.length > 0);
  for (const name of shared) {
    assert.equal(packDefs.filter((n) => n === name).length, 1);
    assert.equal(runDefs.filter((n) => n === name).length, 1);
  }
});

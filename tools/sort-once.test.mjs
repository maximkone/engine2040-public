// Run: node --test tools/sort-once.test.mjs
// Pins identical output for naiveSortUnique vs sortOnce and reports
// a paired 10k-fixture benchmark (ms before -> after).
import assert from "node:assert/strict";
import test from "node:test";
import { naiveSortUnique, sortOnce, makeFixture } from "./sort-once.mjs";
import { benchFns } from "./bench-compare.mjs";

test("empty and single inputs match", () => {
  assert.deepEqual(sortOnce([]), naiveSortUnique([]));
  assert.deepEqual(sortOnce([7]), naiveSortUnique([7]));
  assert.deepEqual(sortOnce([3, 3, 3]), [3]);
});

test("sorted, reverse and mixed duplicates match", () => {
  const cases = [
    [1, 2, 3, 4],
    [4, 3, 2, 1],
    [5, 1, 5, 3, 1, 4, 3],
    [-2, -2, 0, 5, -2, 5, 10, 0],
  ];
  for (const c of cases) {
    assert.deepEqual(sortOnce(c), naiveSortUnique(c));
  }
});

test("10k deterministic fixture pins identical output", () => {
  const fixture = makeFixture(10000, 2040);
  const a = naiveSortUnique(fixture);
  const b = sortOnce(fixture);
  assert.deepEqual(b, a);
  assert.ok(b.length > 9000);
  for (let i = 1; i < b.length; i += 1) {
    assert.ok(b[i - 1] < b[i]);
  }
});

test("does not mutate the input", () => {
  const input = [3, 1, 2, 1];
  const snap = [...input];
  sortOnce(input);
  naiveSortUnique(input);
  assert.deepEqual(input, snap);
});

test("benchmark 10k fixture via shared loop: single-pass is faster (ms before -> after)", () => {
  const fixture = makeFixture(10000, 2040);
  const rows = benchFns(
    [
      { name: "naive", fn: () => naiveSortUnique(fixture) },
      { name: "once", fn: () => sortOnce(fixture) },
    ],
    { warmup: 2, runs: 20 },
  );
  const naiveMs = rows[0].mean_ms;
  const onceMs = rows[1].mean_ms;
  console.log(`benchmark 10k x20: naive ${naiveMs.toFixed(2)}ms -> once ${onceMs.toFixed(2)}ms`);
  assert.deepEqual(sortOnce(fixture), naiveSortUnique(fixture));
  assert.ok(onceMs < naiveMs, `expected once ${onceMs}ms < naive ${naiveMs}ms`);
});

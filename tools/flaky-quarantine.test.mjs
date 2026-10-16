import assert from "node:assert/strict";
import test from "node:test";
import {
  QUARANTINE_THRESHOLD,
  hasFatalMarker,
  normalizeCounts,
  parseQuarantineArgs,
  quarantineList,
} from "./flaky-quarantine.mjs";

test("parseQuarantineArgs defaults and --fail/--threshold/--counts", () => {
  assert.deepEqual(parseQuarantineArgs([]), { counts: null, fails: [], threshold: 2, selfTest: false, help: false });
  const args = parseQuarantineArgs(["--fail", "foo=2", "--threshold", "3", "--counts", "c.json"]);
  assert.deepEqual(args.fails, [{ name: "foo", count: 2 }]);
  assert.equal(args.threshold, 3);
  assert.equal(args.counts, "c.json");
});

test("parseQuarantineArgs rejects unknown flags and bad values (failure cases)", () => {
  assert.throws(() => parseQuarantineArgs(["--nope"]), /Usage:/);
  assert.throws(() => parseQuarantineArgs(["--counts"]), /Usage:/);
  assert.throws(() => parseQuarantineArgs(["--fail", "foo"]), /Usage:/);
  assert.throws(() => parseQuarantineArgs(["--threshold", "0"]), /Usage:/);
  assert.equal(parseQuarantineArgs(["--help"]).help, true);
});

test("hasFatalMarker fails closed on panic and race, passes clean", () => {
  assert.equal(hasFatalMarker("panic: runtime error"), true);
  assert.equal(hasFatalMarker("WARNING: DATA RACE detected"), true);
  assert.equal(hasFatalMarker("ok all green"), false);
  assert.equal(hasFatalMarker(null), false);
});

test("normalizeCounts reads flat and failures shapes, drops junk", () => {
  assert.deepEqual(normalizeCounts({ a: 2, b: 1 }).rows, [{ name: "a", count: 2 }, { name: "b", count: 1 }]);
  assert.deepEqual(normalizeCounts({ failures: { flaky: 3 } }).rows, [{ name: "flaky", count: 3 }]);
  assert.deepEqual(normalizeCounts(null), []);
  const junk = normalizeCounts({ "": 5, ok: NaN, good: 2.7 });
  assert.deepEqual(junk.rows, [{ name: "good", count: 2 }]);
});

test("quarantineList keeps only >= threshold, sorted (failure case)", () => {
  const rows = [{ name: "b", count: 3 }, { name: "a", count: 2 }, { name: "c", count: 1 }];
  assert.deepEqual(quarantineList(rows, 2), ["a", "b"]);
  assert.deepEqual(quarantineList(rows, 4), []);
  assert.deepEqual(quarantineList(rows), ["a", "b"], "default threshold");
  assert.equal(QUARANTINE_THRESHOLD, 2);
});

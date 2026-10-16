// Run: node --test tools/duration-fmt.test.mjs
// duration-fmt is the shared pure duration pretty-printer: raw-ms numbers
// and unit-suffixed tokens parse to integer ms and format as one largest
// unit. Pure functions only, so no repo file is touched.
import assert from "node:assert/strict";
import test from "node:test";
import { formatDuration, formatDurationInput, parseDurationMs, selfTest, unitToMs } from "./duration-fmt.mjs";

test("unitToMs maps every suffix to its ms multiplier", () => {
  assert.equal(unitToMs(""), 1);
  assert.equal(unitToMs("ms"), 1);
  assert.equal(unitToMs("MS"), 1);
  assert.equal(unitToMs("s"), 1000);
  assert.equal(unitToMs("m"), 60000);
  assert.equal(unitToMs("h"), 3600000);
  assert.equal(unitToMs("d"), 86400000);
  assert.throws(() => unitToMs("x"), RangeError);
});

test("parseDurationMs accepts raw ms and suffixed tokens", () => {
  assert.equal(parseDurationMs(0), 0);
  assert.equal(parseDurationMs(1500), 1500);
  assert.equal(parseDurationMs("1500"), 1500);
  assert.equal(parseDurationMs("0ms"), 0);
  assert.equal(parseDurationMs("1.5s"), 1500);
  assert.equal(parseDurationMs("2m"), 120000);
  assert.equal(parseDurationMs("1h"), 3600000);
  assert.equal(parseDurationMs("1d"), 86400000);
});

test("parseDurationMs rounds and rejects bad input", () => {
  assert.equal(parseDurationMs(1.6), 2);
  for (const bad of ["-1", "-1ms", "abc", "1x", "", NaN, -5, Infinity, null, undefined]) {
    assert.throws(() => parseDurationMs(bad), RangeError, JSON.stringify(bad));
  }
});

test("formatDuration prints one largest unit", () => {
  assert.equal(formatDuration(0), "0ms");
  assert.equal(formatDuration(1), "1ms");
  assert.equal(formatDuration(999), "999ms");
  assert.equal(formatDuration(1000), "1s");
  assert.equal(formatDuration(1500), "1.5s");
  assert.equal(formatDuration(60000), "1m");
  assert.equal(formatDuration(90000), "1.5m");
  assert.equal(formatDuration(3600000), "1h");
  assert.equal(formatDuration(86400000), "1d");
  assert.throws(() => formatDuration(-1), RangeError);
  assert.throws(() => formatDuration(NaN), RangeError);
});

test("formatDurationInput round-trips tokens and raw ms", () => {
  assert.equal(formatDurationInput(1500), "1.5s");
  assert.equal(formatDurationInput("1500"), "1.5s");
  assert.equal(formatDurationInput("1.5s"), "1.5s");
  assert.equal(formatDurationInput("2h"), "2h");
  assert.equal(formatDurationInput(0), "0ms");
});

test("selfTest passes on the pinned samples", () => {
  const r = selfTest();
  assert.equal(r.fail, 0);
  assert.ok(r.pass > 0);
});

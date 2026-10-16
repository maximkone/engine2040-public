// Run: node --test tools/desync-diff.test.mjs
// desync-diff over canonical byte dumps: pure first-byte diff plus a
// Factorio-style report dir (report.json, both dumps, diff.txt). CLI runs in
// a temp dir, so no repo file is touched.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { diffRows, firstDivergingByte, parseDesyncArgs, writeDesyncReport } from "./desync-diff.mjs";
import { fnv1a64Hex } from "./fnv1a64.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = join(HERE, "desync-diff.mjs");

test("firstDivergingByte: identical is -1, a flip names its offset", () => {
  assert.equal(firstDivergingByte(new Uint8Array([1, 2, 3]), new Uint8Array([1, 2, 3])), -1);
  assert.equal(firstDivergingByte(new Uint8Array([]), new Uint8Array([])), -1);
  assert.equal(firstDivergingByte(new Uint8Array([0, 1, 2]), new Uint8Array([0, 9, 2])), 1);
  assert.equal(firstDivergingByte(new Uint8Array([7]), new Uint8Array([8])), 0);
});

test("firstDivergingByte: a length mismatch diverges at the shared end", () => {
  assert.equal(firstDivergingByte(new Uint8Array([1, 2]), new Uint8Array([1, 2, 3])), 2);
  assert.equal(firstDivergingByte(new Uint8Array([1, 2, 3]), new Uint8Array([1, 2])), 2);
});

test("diffRows: lists diverging offsets with both bytes, capped", () => {
  const rows = diffRows(new Uint8Array([1, 2, 3, 4]), new Uint8Array([1, 9, 3, 5]));
  assert.deepEqual(rows, [
    { offset: 1, expected: 2, actual: 9 },
    { offset: 3, expected: 4, actual: 5 },
  ]);
  const many = diffRows(new Uint8Array(64).fill(1), new Uint8Array(64).fill(2), 4);
  assert.equal(many.length, 4);
  assert.equal(many[0].offset, 0);
  assert.deepEqual(diffRows(new Uint8Array([1, 2]), new Uint8Array([1, 2, 3])), [{ offset: 2, expected: null, actual: 3, truncated: true }]);
});

test("parseDesyncArgs: accepts the full flag set, rejects bad system/tick", () => {
  const args = parseDesyncArgs(["--expected", "a.bin", "--actual", "b.bin", "--system", "world", "--tick", "60", "--out", "report"]);
  assert.equal(args.system, "world");
  assert.equal(args.tick, "60");
  for (const system of ["world", "space", "combat", "rng", "ai", "clock"]) {
    assert.equal(parseDesyncArgs(["--expected", "a", "--actual", "b", "--system", system, "--tick", "0", "--out", "o"]).system, system);
  }
  assert.throws(() => parseDesyncArgs(["--expected", "a", "--actual", "b", "--system", "nope", "--tick", "0", "--out", "o"]), /unknown system/);
  assert.throws(() => parseDesyncArgs(["--expected", "a", "--actual", "b", "--system", "world", "--tick", "x", "--out", "o"]), /--tick must be a u32/);
  assert.throws(() => parseDesyncArgs(["--expected", "a", "--actual", "b", "--system", "world", "--tick", "0"]), /missing required/);
  assert.throws(() => parseDesyncArgs(["--expected", "a", "--actual", "b", "--system", "world", "--tick", "0", "--out", "o", "--bogus", "x"]), /unknown argument/);
});

test("writeDesyncReport: dumps both sides plus first byte and hashes", () => {
  const root = mkdtempSync(join(tmpdir(), "desync-diff-"));
  const out = join(root, "report");
  const expected = new Uint8Array([10, 20, 30, 40]);
  const actual = new Uint8Array([10, 20, 31, 40]);
  const { offset, report } = writeDesyncReport({ outDir: out, tick: 60, system: "world", expectedBytes: expected, actualBytes: actual });
  assert.equal(offset, 2);
  assert.equal(report.tick, 60);
  assert.equal(report.system, "world");
  assert.equal(report.first_byte, 2);
  assert.equal(report.expected_hash, fnv1a64Hex(expected));
  assert.equal(report.actual_hash, fnv1a64Hex(actual));
  assert.deepEqual(new Uint8Array(readFileSync(join(out, "expected.bin"))), expected);
  assert.deepEqual(new Uint8Array(readFileSync(join(out, "actual.bin"))), actual);
  const saved = JSON.parse(readFileSync(join(out, "report.json"), "utf8"));
  assert.equal(saved.first_byte, 2);
  assert.match(readFileSync(join(out, "diff.txt"), "utf8"), /first divergence at byte 2/);
});

test("CLI: a one-byte flip exits 0 with a tick+system report dir", () => {
  const root = mkdtempSync(join(tmpdir(), "desync-diff-"));
  writeFileSync(join(root, "a.bin"), Buffer.from([1, 2, 3, 4]));
  writeFileSync(join(root, "b.bin"), Buffer.from([1, 2, 9, 4]));
  const run = spawnSync(process.execPath, [TOOL, "--expected", "a.bin", "--actual", "b.bin", "--system", "combat", "--tick", "60", "--out", "report"], { encoding: "utf8", cwd: root });
  assert.equal(run.status, 0, run.stderr);
  assert.match(run.stdout, /desync at tick 60 \(system combat\): first byte 2/);
  assert.match(run.stdout, /REPORT:/);
  const report = JSON.parse(readFileSync(join(root, "report", "report.json"), "utf8"));
  assert.equal(report.first_byte, 2);
  assert.equal(report.system, "combat");
});

test("CLI: identical dumps exit 1 with no divergence, bad flags exit 2", () => {
  const root = mkdtempSync(join(tmpdir(), "desync-diff-"));
  writeFileSync(join(root, "a.bin"), Buffer.from([5, 6, 7]));
  writeFileSync(join(root, "b.bin"), Buffer.from([5, 6, 7]));
  const same = spawnSync(process.execPath, [TOOL, "--expected", "a.bin", "--actual", "b.bin", "--system", "world", "--tick", "3", "--out", "report"], { encoding: "utf8", cwd: root });
  assert.equal(same.status, 1);
  assert.match(same.stdout, /no divergence at tick 3 \(system world\)/);
  const missing = spawnSync(process.execPath, [TOOL, "--expected", "gone.bin", "--actual", "b.bin", "--system", "world", "--tick", "3", "--out", "report"], { encoding: "utf8", cwd: root });
  assert.equal(missing.status, 2);
  const badSystem = spawnSync(process.execPath, [TOOL, "--expected", "a.bin", "--actual", "b.bin", "--system", "nope", "--tick", "3", "--out", "report"], { encoding: "utf8", cwd: root });
  assert.equal(badSystem.status, 2);
});

test("CLI: a wrong --expected-hash fails loudly instead of reporting", () => {
  const root = mkdtempSync(join(tmpdir(), "desync-diff-"));
  writeFileSync(join(root, "a.bin"), Buffer.from([1, 2, 3]));
  writeFileSync(join(root, "b.bin"), Buffer.from([1, 2, 4]));
  const run = spawnSync(process.execPath, [TOOL, "--expected", "a.bin", "--actual", "b.bin", "--system", "rng", "--tick", "9", "--out", "report", "--expected-hash", "0000000000000000"], { encoding: "utf8", cwd: root });
  assert.equal(run.status, 2);
  assert.match(run.stderr, /expected dump hash mismatch/);
});

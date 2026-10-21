// Run: node --test tools/ctrf-report.test.mjs
// HX12: the CTRF report a stranger can read. The tool turns a nextest JUnit
// receipt into the CTRF 0.2.0 JSON shape. The report counts the tests the run
// printed, and the same tree gives byte-equal bytes.
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { buildCtrf, main, readReceipt, readSummaryLine, renderCtrf } from "./ctrf-report.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const WORK = join(ROOT, "target", "ctrf-test");

// A nextest JUnit receipt in the shape nextest writes: one <testcase> per test
// the run printed, skipped tests included (report-skipped = "all").
const RECEIPT = `<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="4" failures="1" errors="0" uuid="11111111-2222-3333-4444-555555555555" timestamp="2026-10-21T09:00:00.000000000+00:00" time="0.030">
  <testsuite name="engine-endpoint" tests="3" disabled="0" errors="0" failures="1">
    <testcase name="door_opens" classname="engine-endpoint" timestamp="2026-10-21T09:00:00.000000000+00:00" time="0.007"/>
    <testcase name="door_refuses" classname="engine-endpoint" timestamp="2026-10-21T09:00:00.010000000+00:00" time="0.011">
      <failure message="assertion failed: door refused" type="assertion failed">thread 'door_refuses' panicked at crates/endpoint/src/lib.rs:12:5</failure>
    </testcase>
    <testcase name="door_flaky" classname="engine-endpoint" timestamp="2026-10-21T09:00:00.021000000+00:00" time="0.005" type="flaky failure">
      <flakyFailure message="attempt 1 failed" type="assertion failed"/>
    </testcase>
  </testsuite>
  <testsuite name="engine-playtest" tests="1" disabled="0" errors="0" failures="0">
    <testcase name="playtest_skipped" classname="engine-playtest" timestamp="2026-10-21T09:00:00.026000000+00:00" time="0.000">
      <skipped message="ignored"/>
    </testcase>
  </testsuite>
</testsuites>
`;

function writeFixture(name, text) {
  mkdirSync(WORK, { recursive: true });
  const path = join(WORK, name);
  writeFileSync(path, text, "utf8");
  return path;
}

function run(argv) {
  const errors = [];
  const lines = [];
  const code = main(argv, (line) => lines.push(line), (line) => errors.push(line));
  return { code, lines, errors };
}

test("the report counts the tests the run printed", () => {
  const receipt = readReceipt(RECEIPT);
  assert.equal(receipt.cases.length, 4, "four testcases in the receipt");
  const report = buildCtrf(receipt);
  const summary = report.results.summary;
  assert.equal(summary.tests, receipt.cases.length, "summary.tests equals the testcase count");
  assert.equal(summary.tests, 4);
  assert.equal(summary.passed, 2);
  assert.equal(summary.failed, 1);
  assert.equal(summary.skipped, 1);
  assert.equal(summary.pending, 0);
  assert.equal(summary.other, 0);
  assert.equal(summary.flaky, 1);
  assert.equal(summary.suites, 2);
  assert.equal(report.results.tests.length, 4);
});

test("the report carries the CTRF 0.2.0 shape", () => {
  const report = buildCtrf(readReceipt(RECEIPT));
  assert.equal(report.reportFormat, "CTRF");
  assert.equal(report.specVersion, "0.2.0");
  assert.equal(report.results.tool.name, "cargo-nextest");
  for (const key of ["tests", "passed", "failed", "skipped", "pending", "other", "start", "stop"]) {
    assert.equal(typeof report.results.summary[key], "number", `summary.${key} is a number`);
  }
  for (const entry of report.results.tests) {
    assert.equal(typeof entry.name, "string");
    assert.ok(["passed", "failed", "skipped", "pending", "other"].includes(entry.status), "status is in the enum");
    assert.equal(typeof entry.duration, "number");
    assert.ok(Array.isArray(entry.suite), "suite is a list");
  }
  const failed = report.results.tests.find((t) => t.status === "failed");
  assert.match(failed.message, /door refused/);
  assert.match(failed.trace, /crates\/endpoint\/src\/lib\.rs/);
  const flaky = report.results.tests.find((t) => t.name === "door_flaky");
  assert.equal(flaky.flaky, true);
  assert.equal(flaky.status, "passed");
});

test("the file is byte-equal across two runs of the same tree", () => {
  const input = writeFixture("run.junit.xml", RECEIPT);
  const first = join(WORK, "ctrf-a.json");
  const second = join(WORK, "ctrf-b.json");
  assert.equal(run(["--in", input, "--out", first]).code, 0);
  assert.equal(run(["--in", input, "--out", second]).code, 0);
  const a = readFileSync(first, "utf8");
  const b = readFileSync(second, "utf8");
  assert.equal(a, b, "two runs over the same receipt give the same bytes");
  assert.ok(a.endsWith("\n"), "the file ends with a newline");
});

test("the summary line of the run must agree with the receipt", () => {
  const input = writeFixture("run2.junit.xml", RECEIPT);
  const log = writeFixture(
    "run2.log",
    "     Summary [   0.030s] 4 tests run: 2 passed, 1 failed, 1 skipped\n",
  );
  assert.equal(run(["--in", input, "--out", join(WORK, "ctrf-c.json"), "--log", log]).code, 0);
  const wrong = writeFixture("run3.log", "     Summary [   0.030s] 9 tests run: 9 passed, 0 failed, 0 skipped\n");
  const bad = run(["--in", input, "--out", join(WORK, "ctrf-d.json"), "--log", wrong]);
  assert.equal(bad.code, 1, "a receipt that disagrees with the run fails");
  assert.match(bad.errors.join("\n"), /printed 9 tests/);
});

test("the summary line is read as nextest prints it", () => {
  const summary = readSummaryLine("  Summary [   1.234s] 42 tests run: 40 passed, 1 failed, 1 skipped\n");
  assert.deepEqual(summary, { tests: 42, passed: 40, failed: 1, skipped: 1 });
  assert.equal(readSummaryLine("nothing here"), null);
});

test("a missing receipt is refused, a broken one is named", () => {
  const missing = run(["--in", join(WORK, "nope.xml"), "--out", join(WORK, "ctrf-e.json")]);
  assert.equal(missing.code, 2);
  assert.match(missing.errors.join("\n"), /cannot read the receipt/);
  const broken = writeFixture("broken.junit.xml", "<not-a-receipt/>");
  const bad = run(["--in", broken, "--out", join(WORK, "ctrf-f.json")]);
  assert.equal(bad.code, 1);
  assert.match(bad.errors.join("\n"), /not a JUnit receipt/);
});

test("a receipt whose own count disagrees with its cases is named", () => {
  const lying = RECEIPT.replace('tests="4"', 'tests="9"');
  const input = writeFixture("lying.junit.xml", lying);
  const bad = run(["--in", input, "--out", join(WORK, "ctrf-g.json")]);
  assert.equal(bad.code, 0, "the report is still written");
  assert.match(bad.errors.join("\n"), /says 9 tests but holds 4 testcases/);
  assert.equal(JSON.parse(readFileSync(join(WORK, "ctrf-g.json"), "utf8")).results.summary.tests, 4);
});

test("the report is a pure function of the receipt bytes", () => {
  const a = renderCtrf(buildCtrf(readReceipt(RECEIPT)));
  const b = renderCtrf(buildCtrf(readReceipt(RECEIPT)));
  assert.equal(a, b);
  assert.doesNotMatch(a, /reportId|runId|generatedAt/, "no invented ids and no clock");
});

test.after(() => rmSync(WORK, { recursive: true, force: true }));

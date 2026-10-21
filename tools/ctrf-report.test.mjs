// Run: node --test tools/ctrf-report.test.mjs
// HX12: the CTRF report a stranger can read. The tool turns a nextest JUnit
// receipt into the CTRF 0.2.0 JSON shape. The tool finds the receipt by
// itself, matches it to the run by the run id, and checks the receipt against
// what the run printed. The report is a pure function of the receipt bytes.
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  buildCtrf,
  compareSummary,
  listReceipts,
  main,
  pickReceipt,
  readReceipt,
  readSummaryLine,
  renderCtrf,
} from "./ctrf-report.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const WORK = join(ROOT, "target", "ctrf-test");
const RUN_ID = "11111111-2222-3333-4444-555555555555";

// A nextest JUnit receipt in the shape nextest writes: one <testcase> per test
// the run printed, skipped tests included (report-skipped = "all").
const RECEIPT = `<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="4" failures="1" errors="0" uuid="${RUN_ID}" timestamp="2026-10-21T09:00:00.000000000+00:00" time="0.030">
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

function run(argv, root = ROOT) {
  const errors = [];
  const lines = [];
  const code = main(argv, (line) => lines.push(line), (line) => errors.push(line), root);
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
  // The run counts the tests that finished: the skipped one is not among them.
  const log = writeFixture(
    "run2.log",
    `     Summary [   0.030s] 3 tests run: 2 passed (1 flaky), 1 failed, 1 skipped\n`,
  );
  assert.equal(run(["--in", input, "--out", join(WORK, "ctrf-c.json"), "--log", log]).code, 0);
  const wrong = writeFixture("run3.log", "     Summary [   0.030s] 9 tests run: 9 passed, 0 skipped\n");
  const bad = run(["--in", input, "--out", join(WORK, "ctrf-d.json"), "--log", wrong]);
  assert.equal(bad.code, 1, "a receipt that disagrees with the run fails");
  assert.match(bad.errors.join("\n"), /printed 9 tests/);
});

test("the summary line is read as nextest prints it", () => {
  const summary = readSummaryLine("  Summary [   1.234s] 42 tests run: 40 passed, 1 failed, 1 skipped\n");
  assert.deepEqual(summary, {
    finished: 42,
    initial: 42,
    passed: 40,
    failed: 1,
    execFailed: 0,
    timedOut: 0,
    skipped: 1,
    flaky: 0,
    slow: 0,
    leaky: 0,
    leakyFailed: 0,
  });
  assert.equal(readSummaryLine("nothing here"), null);
});

test("the notes of the summary line are read", () => {
  const summary = readSummaryLine(
    "  Summary [   1.234s] 6 tests run: 3 passed (1 slow, 1 flaky, 1 leaky), 2 failed (1 due to being leaky), 1 skipped\n",
  );
  assert.equal(summary.passed, 3);
  assert.equal(summary.failed, 2);
  assert.equal(summary.skipped, 1);
  assert.equal(summary.slow, 1);
  assert.equal(summary.flaky, 1);
  assert.equal(summary.leaky, 1);
  assert.equal(summary.leakyFailed, 1);
});

test("a run that timed out or failed to execute is read apart from a failure", () => {
  const summary = readSummaryLine(
    "  Summary [   1.234s] 4 tests run: 1 passed, 1 failed, 1 timed out, 1 exec failed, 0 skipped\n",
  );
  assert.equal(summary.passed, 1);
  assert.equal(summary.failed, 1);
  assert.equal(summary.timedOut, 1);
  assert.equal(summary.execFailed, 1);
  assert.equal(summary.skipped, 0);
});

test("a cancelled run is read as the tests that ran over the tests it started", () => {
  const summary = readSummaryLine("  Summary [   1.234s] 8/10 tests run: 7 passed, 1 failed, 0 skipped\n");
  assert.equal(summary.finished, 8, "eight tests finished");
  assert.equal(summary.initial, 10, "ten tests were started");
  assert.equal(summary.passed, 7);
  assert.equal(summary.failed, 1);
});

test("one test is written in the singular", () => {
  const summary = readSummaryLine("  Summary [   0.001s] 1 test run: 1 passed, 0 skipped\n");
  assert.equal(summary.finished, 1);
  assert.equal(summary.initial, 1);
  assert.equal(summary.passed, 1);
});

test("a setup script is not a test", () => {
  const receipt = readReceipt(
    `<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="3" failures="0" errors="0" uuid="${RUN_ID}" timestamp="2026-10-21T09:00:00.000000000+00:00" time="0.020">
  <testsuite name="@setup-script:seed" tests="1" disabled="0" errors="0" failures="0">
    <testcase name="seed" classname="@setup-script:seed" timestamp="2026-10-21T09:00:00.000000000+00:00" time="0.002"/>
  </testsuite>
  <testsuite name="engine-endpoint" tests="2" disabled="0" errors="0" failures="0">
    <testcase name="door_opens" classname="engine-endpoint" timestamp="2026-10-21T09:00:00.010000000+00:00" time="0.007"/>
    <testcase name="door_closes" classname="engine-endpoint" timestamp="2026-10-21T09:00:00.020000000+00:00" time="0.005"/>
  </testsuite>
</testsuites>
`,
  );
  assert.equal(receipt.setupScripts, 1, "the setup script is counted apart");
  assert.equal(receipt.cases.length, 2, "the setup script is not a testcase");
  assert.equal(receipt.suites.length, 1);
  assert.equal(buildCtrf(receipt).results.summary.tests, 2);
});

test("a test that passed but leaked is a pass", () => {
  const receipt = readReceipt(
    `<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="1" failures="0" errors="0" uuid="${RUN_ID}" timestamp="2026-10-21T09:00:00.000000000+00:00" time="0.010">
  <testsuite name="engine-endpoint" tests="1" disabled="0" errors="0" failures="0">
    <testcase name="door_leaks" classname="engine-endpoint" timestamp="2026-10-21T09:00:00.000000000+00:00" time="0.010">
      <error message="test passed but leaked handles" type="test passed but leaked handles"/>
    </testcase>
  </testsuite>
</testsuites>
`,
  );
  assert.equal(receipt.cases[0].status, "passed", "the run counts it as a pass");
  assert.equal(buildCtrf(receipt).results.summary.passed, 1);
});

test("the receipt of a run is found by the run id", () => {
  const root = join(WORK, "discover");
  const dir = join(root, "target", "nextest", "ci", "target", "nextest-junit");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "ci.xml"), RECEIPT, "utf8");
  const receipts = listReceipts(root);
  assert.equal(receipts.length, 1, "the store dir is scanned");
  const log = `     Starting nextest with the nextest profile: ci\n     Nextest run ID ${RUN_ID} with nextest profile: ci\n`;
  assert.equal(pickReceipt(log, root), join(dir, "ci.xml"), "the run id names the receipt");
  // A run id that names no receipt is no receipt.
  const other = `     Nextest run ID 99999999-9999-9999-9999-999999999999 with nextest profile: ci\n`;
  assert.equal(pickReceipt(other, root), null);
  // Without a run id the profile names the receipt.
  const profileOnly = "     Starting nextest with the nextest profile: ci\n";
  assert.equal(pickReceipt(profileOnly, root), join(dir, "ci.xml"));
});

test("a run with no receipt is refused", () => {
  const root = join(WORK, "empty");
  mkdirSync(root, { recursive: true });
  const log = writeFixture(
    "empty.log",
    `     Nextest run ID ${RUN_ID} with nextest profile: ci\n     Summary [   0.030s] 4 tests run: 2 passed (1 flaky), 1 failed, 1 skipped\n`,
  );
  const bad = run(["--log", log, "--out", join(WORK, "ctrf-h.json")], root);
  assert.equal(bad.code, 2, "no receipt is exit 2");
  assert.match(bad.errors.join("\n"), /no test receipt/);
});

test("a log with no summary line is refused", () => {
  const input = writeFixture("run4.junit.xml", RECEIPT);
  const log = writeFixture("run4.log", "     something else entirely\n");
  const bad = run(["--in", input, "--out", join(WORK, "ctrf-i.json"), "--log", log]);
  assert.equal(bad.code, 1);
  assert.match(bad.errors.join("\n"), /no nextest summary line/);
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

test("the receipt and the run are compared per status", () => {
  const receipt = readReceipt(RECEIPT);
  const printed = readSummaryLine(
    "  Summary [   0.030s] 3 tests run: 2 passed (1 flaky), 1 failed, 1 skipped\n",
  );
  assert.deepEqual(compareSummary(receipt, printed), [], "the run and the receipt agree");
  const wrongCount = readSummaryLine(
    "  Summary [   0.030s] 3 tests run: 3 passed (1 flaky), 1 failed, 1 skipped\n",
  );
  assert.match(compareSummary(receipt, wrongCount).join("\n"), /printed 3 passed/);
  const wrongFlaky = readSummaryLine("  Summary [   0.030s] 3 tests run: 2 passed, 1 failed, 1 skipped\n");
  assert.match(compareSummary(receipt, wrongFlaky).join("\n"), /printed 0 flaky/);
});

test("the report is a pure function of the receipt bytes", () => {
  const a = renderCtrf(buildCtrf(readReceipt(RECEIPT)));
  const b = renderCtrf(buildCtrf(readReceipt(RECEIPT)));
  assert.equal(a, b);
  assert.doesNotMatch(a, /reportId|runId|generatedAt/, "no invented ids and no clock");
});

test.after(() => rmSync(WORK, { recursive: true, force: true }));

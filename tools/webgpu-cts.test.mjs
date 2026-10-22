// Run: node --test tools/webgpu-cts.test.mjs
// The WebGPU conformance gate (HX4) over fixtures, no GPU: a fake runner
// stands in for the native one, so the states, the counts, the expectations
// and the strict switch are all checked without a device.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { parseYaml } from "./pages-workflow.test.mjs";
import {
  COMPILED,
  CRASH,
  EXPECTED,
  FAIL,
  PASS,
  REJECTED,
  caseState,
  judgeCase,
  listLines,
  loadCases,
  loadExpectations,
  parseCaseLine,
  runSlice,
  strictRequested,
  summaryLine,
} from "./webgpu-cts.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOL = resolve(HERE, "webgpu-cts.mjs");
const ROOT = resolve(HERE, "..");

/** A fake runner: a node script that prints the receipt line the tool reads. */
function fakeRunner(lines) {
  const dir = mkdtempSync(join(tmpdir(), "webgpu-cts-"));
  const script = join(dir, "runner.mjs");
  // The table the child reads: the JSON text, not the object.
  const table = JSON.stringify(lines);
  writeFileSync(
    script,
    [
      "// The fake runner: one receipt line per stdin WGSL, in the order given.",
      "import { readFileSync } from 'node:fs';",
      "const table = JSON.parse(process.env.FAKE_TABLE);",
      "const wgsl = readFileSync(0, 'utf8');",
      "const line = table[wgsl] ?? table['*'];",
      // No process.exit here: on Windows it can cut the line off before it
      // reaches the pipe.
      "process.stdout.write(line ?? '');",
    ].join("\n"),
    "utf8",
  );
  // The fake runner is node itself: the tool spawns a binary, so the test
  // hands it a node command line instead (the runner path plus its script).
  return { path: process.execPath, script, table };
}

/** The tool run over a fake runner, with the case file and the expectations. */
function runTool({ cases, expectations, runner, runnerArg, env = {} }) {
  const dir = mkdtempSync(join(tmpdir(), "webgpu-cts-cases-"));
  const casePath = join(dir, "cases.json");
  const expectPath = join(dir, "expectations.txt");
  writeFileSync(casePath, JSON.stringify(cases), "utf8");
  writeFileSync(expectPath, expectations, "utf8");
  const args = [TOOL, "--cases", casePath, "--expectations", expectPath, "--runner", runner];
  if (runnerArg) args.push("--runner-arg", runnerArg);
  const r = spawnSync(process.execPath, args, { encoding: "utf8", env: { ...process.env, ...env } });
  return { code: r.status, out: r.stdout, err: r.stderr };
}

const ONE_CASE = [{ id: "webgpu:shader,validation,decl,const:case", expect: "pass", wgsl: "const x = 0;" }];

test("the case file of the repo holds the chosen slice and its donor", () => {
  const { donor, cases } = loadCases();
  assert.equal(donor.repo, "gpuweb/cts");
  assert.equal(donor.license, "BSD-3-Clause");
  assert.ok(cases.length >= 20, `the slice holds ${cases.length} cases`);
  for (const one of cases) {
    assert.match(one.id, /^webgpu:/, one.id);
    assert.ok(one.source, `${one.id} names the suite file it came from`);
    assert.ok(one.wgsl.length > 0, `${one.id} holds its WGSL`);
  }
  // Every case id is unique: a duplicate would run twice and count twice.
  assert.equal(new Set(cases.map((c) => c.id)).size, cases.length);
  // Every case names a file of the official suite.
  for (const one of cases) {
    assert.match(one.source, /^src\/webgpu\/shader\/validation\/[\w/]+\.spec\.ts$/, one.id);
  }
  // The slice covers the ported areas, so a port that lost a file is caught.
  const areas = new Set(cases.map((c) => c.source.replace(/^src\/webgpu\/shader\/validation\//, "")));
  assert.deepEqual(
    [...areas].sort(),
    ["const_assert/const_assert.spec.ts", "decl/const.spec.ts", "decl/override.spec.ts", "types/alias.spec.ts"],
  );
});

test("the expectations file of the repo is readable and its lines name real cases", () => {
  const { cases } = loadCases();
  const expectations = loadExpectations();
  for (const id of expectations.keys()) {
    assert.ok(cases.some((c) => c.id === id), `the expectations name the unknown case ${id}`);
  }
});

test("the strict switch is the render stack's own env name", () => {
  assert.equal(strictRequested({ E2040_STRICT_WEBGPU: "1" }), true);
  assert.equal(strictRequested({ E2040_STRICT_WEBGPU: " 1 " }), true);
  assert.equal(strictRequested({ E2040_STRICT_WEBGPU: "0" }), false);
  assert.equal(strictRequested({}), false);
});

test("the receipt line of the runner is parsed into its fields", () => {
  const fields = parseCaseLine("webgpu_cts_case: verdict=compiled adapter=NVIDIA strict=1");
  assert.equal(fields.verdict, "compiled");
  assert.equal(fields.adapter, "NVIDIA");
  assert.equal(fields.strict, "1");
  assert.equal(parseCaseLine("some other line"), null);
  assert.equal(parseCaseLine("webgpu_cts_case: adapter=NVIDIA"), null);
});

test("the reason of a case is the rest of the line after `why=`", () => {
  const fields = parseCaseLine(
    "webgpu_cts_case: verdict=error adapter=none strict=1 why=no wgpu adapter on this machine",
  );
  assert.equal(fields.verdict, "error");
  assert.equal(fields.why, "no wgpu adapter on this machine");
});

test("a case that compiled passes, one that was rejected fails, one that could not run crashes", () => {
  assert.equal(caseState(0, "webgpu_cts_case: verdict=compiled adapter=X strict=1").state, COMPILED);
  assert.equal(caseState(0, "webgpu_cts_case: verdict=rejected adapter=X strict=1").state, REJECTED);
  assert.equal(caseState(1, "webgpu_cts_case: verdict=error adapter=none strict=1 why=no adapter").state, CRASH);
});

test("the device's verdict is judged against the suite's expectation", () => {
  // The suite expects the program to compile, and it compiled: a pass.
  assert.equal(judgeCase(PASS, { state: COMPILED, why: "" }).state, PASS);
  // The suite expects the program to be refused, and it was: a pass.
  assert.equal(judgeCase(FAIL, { state: REJECTED, why: "" }).state, PASS);
  // The suite expects it to compile and the device refused it: a failure.
  const missed = judgeCase(PASS, { state: REJECTED, why: "" });
  assert.equal(missed.state, FAIL);
  assert.match(missed.why, /expects the program to compile/);
  // The suite expects it to be refused and the device compiled it: a failure.
  const wrong = judgeCase(FAIL, { state: COMPILED, why: "" });
  assert.equal(wrong.state, FAIL);
  assert.match(wrong.why, /expects the program to be refused/);
  // A crash is a crash, whatever the suite expects.
  assert.equal(judgeCase(PASS, { state: CRASH, why: "no adapter" }).state, CRASH);
});

test("a runner that died without a receipt line is a crash, never a pass", () => {
  const got = caseState(1, "");
  assert.equal(got.state, CRASH);
  assert.match(got.why, /no receipt line/);
});

test("a run whose instance was not strict is a crash, never a pass", () => {
  const got = caseState(0, "webgpu_cts_case: verdict=compiled adapter=X strict=0");
  assert.equal(got.state, CRASH);
  assert.match(got.why, /not strict/);
});

test("a case named in the expectations file is reported as expected, never as pass", () => {
  // The suite expects the program to compile and the device refused it: a
  // known defect, so the expectations file names it.
  const { path, script, table } = fakeRunner({ "const x = 0;": "webgpu_cts_case: verdict=rejected adapter=X strict=1" });
  const { rows, counts } = runSlice(
    [{ id: "case-a", expect: "pass", wgsl: "const x = 0;" }],
    new Map([["case-a", "known defect"]]),
    path,
    { ...process.env, FAKE_TABLE: table },
    [script],
  );
  assert.equal(rows[0].state, EXPECTED);
  assert.equal(counts.expected, 1);
  assert.equal(counts.pass, 0);
  assert.equal(counts.fail, 0);
});

test("a case that passes while the expectations file names it is a stale line, and the gate fails", () => {
  const { path, script, table } = fakeRunner({ "const x = 0;": "webgpu_cts_case: verdict=compiled adapter=X strict=1" });
  const { rows, counts } = runSlice(
    [{ id: "case-a", expect: "pass", wgsl: "const x = 0;" }],
    new Map([["case-a", "known defect"]]),
    path,
    { ...process.env, FAKE_TABLE: table },
    [script],
  );
  assert.equal(rows[0].state, PASS);
  assert.equal(counts.pass, 1);
  assert.match(rows[0].why, /stale/);
});

test("the count line carries pass, fail, crash and expected", () => {
  assert.equal(summaryLine({ pass: 3, fail: 0, crash: 0, expected: 1 }), "webgpu_cts: pass=3 fail=0 crash=0 expected=1");
});

test("--list prints the chosen cases and their expectations", () => {
  const out = listLines(
    [
      { id: "case-a", expect: "pass", wgsl: "a" },
      { id: "case-b", expect: "fail", wgsl: "b" },
    ],
    new Map([["case-b", "known defect"]]),
  );
  const lines = out.split("\n");
  assert.equal(lines[0], "pass case-a");
  assert.equal(lines[1], "expected case-b");
  assert.equal(lines[2], "2 case(s), 1 expected");
});

test("the tool refuses a run whose instance was not strict", () => {
  const { path, script, table } = fakeRunner({ "*": "webgpu_cts_case: verdict=compiled adapter=X strict=1" });
  const { code, err } = runTool({
    cases: ONE_CASE,
    expectations: "",
    runner: path,
    runnerArg: script,
    env: { FAKE_TABLE: table, E2040_STRICT_WEBGPU: "0" },
  });
  assert.equal(code, 1);
  assert.match(err, /not strict/);
  assert.match(err, /E2040_STRICT_WEBGPU/);
});

test("the tool runs the slice over a fake runner and prints the count line", () => {
  const { path, script, table } = fakeRunner({
    "const x = 0;": "webgpu_cts_case: verdict=compiled adapter=X strict=1",
  });
  const { code, out } = runTool({
    cases: ONE_CASE,
    expectations: "",
    runner: path,
    runnerArg: script,
    env: { FAKE_TABLE: table, E2040_STRICT_WEBGPU: "1" },
  });
  assert.equal(code, 0);
  assert.match(out, /^pass webgpu:shader,validation,decl,const:case$/m);
  assert.match(out, /^webgpu_cts: pass=1 fail=0 crash=0 expected=0$/m);
});

test("the tool fails when a case does not match the suite's expectation", () => {
  // The suite expects the program to compile, the device refused it.
  const { path, script, table } = fakeRunner({
    "const x = 0;": "webgpu_cts_case: verdict=rejected adapter=X strict=1",
  });
  const { code, out } = runTool({
    cases: ONE_CASE,
    expectations: "",
    runner: path,
    runnerArg: script,
    env: { FAKE_TABLE: table, E2040_STRICT_WEBGPU: "1" },
  });
  assert.equal(code, 1);
  assert.match(out, /^webgpu_cts: pass=0 fail=1 crash=0 expected=0$/m);
});

test("the tool fails when the device compiles a program the suite expects to be refused", () => {
  // The suite expects the program to be refused, the device compiled it.
  const { path, script, table } = fakeRunner({
    "const x = 0;": "webgpu_cts_case: verdict=compiled adapter=X strict=1",
  });
  const { code, out } = runTool({
    cases: [{ id: "case-a", expect: "fail", wgsl: "const x = 0;" }],
    expectations: "",
    runner: path,
    runnerArg: script,
    env: { FAKE_TABLE: table, E2040_STRICT_WEBGPU: "1" },
  });
  assert.equal(code, 1);
  assert.match(out, /^webgpu_cts: pass=0 fail=1 crash=0 expected=0$/m);
});

test("the tool fails when the runner is missing", () => {
  const { code, err } = runTool({
    cases: ONE_CASE,
    expectations: "",
    runner: join(tmpdir(), "webgpu-cts-not-there.exe"),
    env: { E2040_STRICT_WEBGPU: "1" },
  });
  assert.equal(code, 2);
  assert.match(err, /runner .* is not there/);
});

test("the expectations file is read with its comments and its trailing notes", () => {
  const dir = mkdtempSync(join(tmpdir(), "webgpu-cts-exp-"));
  const path = join(dir, "wgpu.txt");
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    path,
    ["# a comment", "", "case-a  the backend rejects it", "case-b", "# another comment"].join("\n"),
    "utf8",
  );
  const expectations = loadExpectations(path);
  assert.equal(expectations.get("case-a"), "the backend rejects it");
  assert.equal(expectations.get("case-b"), "");
  assert.equal(expectations.size, 2);
});

test("the CI leg builds the runner and runs the slice with the strict switch on", () => {
  const text = readFileSync(join(ROOT, ".github", "workflows", "webgpu-cts.yml"), "utf8");
  const wf = parseYaml(text);
  // The strict switch is the render stack's own env name, and the gate
  // refuses to run without it, so the leg must set it.
  assert.equal(wf.env.E2040_STRICT_WEBGPU, "1");
  // The leg builds the runner the tool spawns.
  const runs = wf.jobs.slice.steps.filter((s) => s.run).map((s) => s.run).join("\n");
  assert.match(runs, /cargo build --locked -p engine-render --features window --bin webgpu_cts/);
  // The leg runs the tool itself.
  assert.match(runs, /node tools\/webgpu-cts\.mjs/);
  // The leg never touches the golden matrix workflow.
  assert.ok(!/golden-matrix/.test(text), "the golden matrix workflow is untouched");
});

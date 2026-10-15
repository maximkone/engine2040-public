import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findOutcomes, formatReport, judge, parseArgs, parseOutcomes, parseProgress, patchFiles, summarize } from "./mutants.mjs";

// Recorded from `cargo mutants --in-diff weak.patch -p engine-core` (cargo-mutants
// 27.1.0): a function with an `is_ok()`-only test. Two outcomes are verbatim
// (phase_results trimmed); the rest of the file follows the same schema.
const recorded = {
  outcomes: [
    { scenario: "Baseline", summary: "Success", log_path: "log/baseline.log", diff_path: null, phase_results: [] },
    {
      scenario: {
        Mutant: {
          name: "crates/core/src/time.rs:158:5: replace steps_for_ns -> Result<u32, TimeError> with Ok(0)",
          package: "engine-core",
          file: "crates/core/src/time.rs",
          function: {
            function_name: "steps_for_ns",
            return_type: "-> Result<u32, TimeError>",
            span: { start: { line: 155, column: 1 }, end: { line: 163, column: 2 } },
          },
          span: { start: { line: 158, column: 5 }, end: { line: 162, column: 38 } },
          replacement: "Ok(0)",
          genre: "FnValue",
        },
      },
      summary: "MissedMutant",
      log_path: "log/crates__core__src__time.rs_line_158_col_5.log",
      diff_path: "diff/crates__core__src__time.rs_line_158_col_5.diff",
      phase_results: [],
    },
    {
      scenario: {
        Mutant: {
          name: "crates/core/src/time.rs:158:11: replace > with < in steps_for_ns",
          package: "engine-core",
          file: "crates/core/src/time.rs",
          function: {
            function_name: "steps_for_ns",
            return_type: "-> Result<u32, TimeError>",
            span: { start: { line: 155, column: 1 }, end: { line: 163, column: 2 } },
          },
          span: { start: { line: 158, column: 11 }, end: { line: 158, column: 12 } },
          replacement: "<",
          genre: "BinaryOperator",
        },
      },
      summary: "CaughtMutant",
      log_path: "log/crates__core__src__time.rs_line_158_col_11_002.log",
      diff_path: "diff/crates__core__src__time.rs_line_158_col_11_002.diff",
      phase_results: [],
    },
  ],
  total_mutants: 2,
  missed: 1,
  caught: 1,
  timeout: 0,
  unviable: 0,
  success: 0,
  cargo_mutants_version: "27.1.0",
};

// The same run's stdout, verbatim. UNVIABLE and TIMEOUT lines are the
// documented shapes (not recorded); a baseline failure line likewise.
const progress = [
  "Found 9 mutants to test",
  "ok       Unmutated baseline in 1s build + 0s test",
  " INFO Auto-set test timeout to 20s",
  "MISSED   crates/core/src/time.rs:158:5: replace steps_for_ns -> Result<u32, TimeError> with Ok(0) in 1s build + 0s test",
  "MISSED   crates/core/src/time.rs:158:11: replace > with == in steps_for_ns in 1s build + 0s test",
  "CAUGHT   crates/core/src/time.rs:158:11: replace > with < in steps_for_ns in 1s build + 0s test",
  "UNVIABLE crates/core/src/time.rs:160:9: replace f -> Foo with Default::default() in 0s build",
  "TIMEOUT  crates/core/src/tick.rs:36:20: replace >= with < in FixedAcc::push in 1s build + 20s test",
  "9 mutants tested in 20s: 2 missed, 1 caught, 1 unviable, 1 timeout",
].join("\n");

const lane = (mutants, baseline = "ok") => ({ baseline, mutants });
const m = (status, path, line, mutation) => ({ status, path, line, column: 1, mutation });

test("outcomes.json: names, lines and statuses come from the recorded schema", () => {
  const { baseline, mutants } = parseOutcomes(JSON.stringify(recorded));
  assert.equal(baseline, "ok");
  assert.deepEqual(mutants.map((x) => [x.status, x.path, x.line, x.column, x.mutation]), [
    ["missed", "crates/core/src/time.rs", 158, 5, "replace steps_for_ns -> Result<u32, TimeError> with Ok(0)"],
    ["caught", "crates/core/src/time.rs", 158, 11, "replace > with < in steps_for_ns"],
  ]);
});

test("outcomes.json: a nameless mutant falls back to its span; unreadable results fail closed", () => {
  const nameless = { outcomes: [{ scenario: { Mutant: { file: "a.rs", span: { start: { line: 4, column: 2 } }, replacement: "true", genre: "FnValue" } }, summary: "MissedMutant" }] };
  assert.deepEqual(parseOutcomes(nameless).mutants[0], { status: "missed", path: "a.rs", line: 4, column: 2, mutation: "FnValue: true" });
  const unknown = structuredClone(recorded);
  unknown.outcomes[1].summary = "Mystery";
  assert.throws(() => parseOutcomes(unknown), /unknown outcome "Mystery"/);
  assert.throws(() => parseOutcomes({ ...recorded, total_mutants: 5 }), /total_mutants 5/);
  assert.throws(() => parseOutcomes({ nope: 1 }), /no `outcomes` array/);
  assert.throws(() => parseOutcomes({ outcomes: [{ scenario: "Strange", summary: "Success" }] }), /unknown scenario/);
  assert.equal(parseOutcomes({ outcomes: [{ scenario: "Baseline", summary: "Failure" }] }).baseline, "failed");
});

test("progress lines: the timing tail goes, a mutation that says `in` stays", () => {
  const { baseline, mutants } = parseProgress(progress);
  assert.equal(baseline, "ok");
  assert.deepEqual(mutants.map((x) => [x.status, x.line, x.mutation]), [
    ["missed", 158, "replace steps_for_ns -> Result<u32, TimeError> with Ok(0)"],
    ["missed", 158, "replace > with == in steps_for_ns"],
    ["caught", 158, "replace > with < in steps_for_ns"],
    ["unviable", 160, "replace f -> Foo with Default::default()"],
    ["timeout", 36, "replace >= with < in FixedAcc::push"],
  ]);
  assert.equal(parseProgress("FAILED   Unmutated baseline in 2s build + 3s test").baseline, "failed");
  assert.equal(parseProgress("TIMEOUT  Unmutated baseline in 20s").baseline, "failed", "timed-out baseline is not green");
  assert.equal(parseProgress("MISSED   a.rs:3:4: replace x with y").mutants[0].mutation, "replace x with y", "--no-times output");
});

test("score counts timeouts as detected and ignores unviable mutants", () => {
  const s = summarize(lane([m("caught", "a.rs", 1, "x"), m("timeout", "a.rs", 2, "y"), m("missed", "a.rs", 3, "z"), m("unviable", "a.rs", 4, "w")]));
  assert.equal(s.score, (100 * 2) / 3);
  assert.deepEqual([s.caught, s.timeout, s.missed, s.unviable, s.total, s.ok], [1, 1, 1, 1, 4, false]);
  assert.equal(summarize(lane([])).score, null);
  assert.equal(summarize(lane([])).ok, true, "a diff with nothing to mutate has nothing missed");
  assert.equal(summarize(lane([m("unviable", "a.rs", 1, "x")])).score, null);
});

test("PASS needs missed <= max-missed, a green baseline and a complete run", () => {
  const missed = lane([m("missed", "a.rs", 1, "x"), m("caught", "a.rs", 2, "y")]);
  assert.equal(summarize(missed).ok, false);
  assert.equal(summarize(missed, { maxMissed: 1 }).ok, true);
  assert.equal(summarize(lane([m("caught", "a.rs", 1, "x")], "failed")).ok, false);
  assert.equal(summarize(lane([m("caught", "a.rs", 1, "x")]), { incomplete: true }).ok, false);
});

test("the report names every missed mutant as path:line: mutation and ends with RESULT", () => {
  const parsed = lane([
    m("missed", "crates/b/src/z.rs", 9, "replace f with 1"),
    m("caught", "crates/a/src/y.rs", 1, "replace g with 0"),
    m("missed", "crates/a/src/y.rs", 40, "replace + with -"),
    m("timeout", "crates/a/src/y.rs", 7, "replace < with >"),
  ]);
  const fail = formatReport(parsed, { label: "engine-a" });
  assert.deepEqual(fail.lines.slice(0, 3), [
    "crates/a/src/y.rs:40: replace + with -",
    "crates/b/src/z.rs:9: replace f with 1",
    "WARN timeout crates/a/src/y.rs:7: replace < with >",
  ]);
  assert.match(fail.lines[3], /^mutants: caught=1 missed=2 unviable=0 timeout=1 total=4 score=50\.0%$/);
  assert.match(fail.lines.at(-1), /^RESULT FAIL: 2 missed \(max 0\), score 50\.0% \(engine-a\)$/);
  assert.match(formatReport(parsed, { maxMissed: 2 }).lines.at(-1), /^RESULT PASS: 2 missed \(max 2\)/);
  assert.match(formatReport(lane([]), {}).lines.at(-1), /^RESULT PASS: 0 missed \(max 0\), score n\/a, no mutants in the diff/);
  assert.match(formatReport(lane([], "failed")).lines.at(-1), /^RESULT FAIL: unmutated baseline failed/);
  const cut = formatReport(lane([m("caught", "a.rs", 1, "x")]), { incomplete: true, note: "budget of 10 min exceeded" });
  assert.match(cut.lines.at(-1), /^RESULT FAIL: run incomplete.*budget of 10 min exceeded$/);
});

test("patchFiles lists post-image files with their added line counts", () => {
  const patch = [
    "diff --git a/crates/x/src/new.rs b/crates/x/src/new.rs",
    "new file mode 100644",
    "--- /dev/null",
    "+++ b/crates/x/src/new.rs",
    "@@ -0,0 +2 @@",
    "+fn a() {}",
    "+fn b() {}",
    "diff --git a/crates/x/src/old.rs b/crates/x/src/old.rs",
    "--- a/crates/x/src/old.rs",
    "+++ /dev/null",
    "@@ -1 +0,0 @@",
    "-fn gone() {}",
    'diff --git "a/crates/x/src/with space.rs" "b/crates/x/src/with space.rs"',
    '--- "a/crates/x/src/with space.rs"',
    '+++ "b/crates/x/src/with space.rs"',
    "@@ -1 +1 @@",
    "-fn c() {}",
    "+fn d() {}",
    "",
  ].join("\n");
  assert.deepEqual(patchFiles(patch), [
    { path: "crates/x/src/new.rs", deleted: false, added: 2 },
    { path: "crates/x/src/old.rs", deleted: true, added: 0 },
    { path: "crates/x/src/with space.rs", deleted: false, added: 1 },
  ]);
  assert.deepEqual(patchFiles("not a patch"), []);
});

test("arguments: required flags, bounded numbers, passthrough after --", () => {
  const ok = parseArgs(["--patch", "p.patch", "-p", "engine-core", "--package", "engine-ecs", "--timeout-min", "2.5", "--max-missed", "3", "--jobs", "2", "--", "--features", "x"]);
  assert.deepEqual([ok.patch, ok.packages, ok.timeoutMin, ok.maxMissed, ok.jobs, ok.extra], ["p.patch", ["engine-core", "engine-ecs"], 2.5, 3, 2, ["--features", "x"]]);
  assert.equal(parseArgs(["--patch", "-", "-p", "c"]).timeoutMin, 10);
  assert.equal(parseArgs(["--from-out", "dir"]).fromOut, "dir");
  assert.throws(() => parseArgs(["-p", "engine-core"]), /--patch is required/);
  assert.throws(() => parseArgs(["--patch", "p"]), /-p <crate> is required/);
  assert.throws(() => parseArgs(["--patch", "p", "-p", "c", "--max-missed", "-1"]), /--max-missed needs an integer >= 0/);
  assert.throws(() => parseArgs(["--patch", "p", "-p", "c", "--max-missed", "1.5"]), /integer/);
  assert.throws(() => parseArgs(["--patch", "p", "-p", "c", "--timeout-min", "0"]), /--timeout-min needs a number/);
  assert.throws(() => parseArgs(["--patch", "p", "-p", "c", "--jobs"]), /--jobs needs a value/);
  assert.throws(() => parseArgs(["--patch", "p", "-p", "c", "--bogus"]), /unknown argument: --bogus/);
});

// A tree with one Rust file and a patch touching it; cargo is replaced by `fake`.
function fixture(patchText) {
  const dir = mkdtempSync(join(tmpdir(), "mutants-"));
  mkdirSync(join(dir, "crates", "x", "src"), { recursive: true });
  writeFileSync(join(dir, "crates", "x", "src", "lib.rs"), "pub fn a() {}\n");
  const patch = join(dir, "lane.patch");
  writeFileSync(patch, patchText ?? ["diff --git a/crates/x/src/lib.rs b/crates/x/src/lib.rs", "--- a/crates/x/src/lib.rs", "+++ b/crates/x/src/lib.rs", "@@ -0,0 +1 @@", "+pub fn a() {}", ""].join("\n"));
  const opts = { patch, packages: ["engine-x"], root: dir, timeoutMin: 10, maxMissed: 0, jobs: 0, output: join(dir, "out"), report: "", fromOut: "", extra: [] };
  return { dir, opts, done: () => rmSync(dir, { recursive: true, force: true }) };
}
const installed = () => ({ status: 0 });
const outcomesFile = (dir, doc) => {
  mkdirSync(join(dir, "out", "mutants.out"), { recursive: true });
  writeFileSync(join(dir, "out", "mutants.out", "outcomes.json"), JSON.stringify(doc));
};

test("judge runs cargo mutants on the patch and fails on a missed mutant", async () => {
  const { dir, opts, done } = fixture();
  let call;
  try {
    const report = await judge({ ...opts, jobs: 2, extra: ["--features", "x"] }, {
      version: installed,
      runCargo: async (args, run) => { call = { args, run }; outcomesFile(dir, recorded); return { status: 2, stdout: "", stderr: "" }; },
    });
    assert.deepEqual(call.args, ["mutants", "--in-diff", opts.patch, "--package", "engine-x", "--output", opts.output, "--colors", "never", "--jobs", "2", "--features", "x"]);
    assert.equal(call.run.cwd, dir);
    assert.equal(call.run.timeoutMs, 600_000);
    assert.equal(report.ok, false);
    assert.equal(report.lines[0], "crates/core/src/time.rs:158: replace steps_for_ns -> Result<u32, TimeError> with Ok(0)");
  } finally { done(); }
});

test("judge never reads a stale mutants.out from an earlier run", async () => {
  const { dir, opts, done } = fixture();
  try {
    outcomesFile(dir, recorded);
    const report = await judge(opts, { version: installed, runCargo: async () => ({ status: 0, stdout: " INFO No mutants to filter\n", stderr: "" }) });
    assert.equal(report.ok, true);
    assert.equal(report.total, 0);
    assert.equal(findOutcomes(opts.output), null);
  } finally { done(); }
});

test("judge fails closed on budget, baseline, tool errors and a missing cargo-mutants", async () => {
  const { dir, opts, done } = fixture();
  try {
    const cut = await judge({ ...opts, timeoutMin: 3 }, { version: installed, runCargo: async () => ({ status: null, timedOut: true, stdout: progress, stderr: "" }) });
    assert.equal(cut.ok, false);
    assert.equal(cut.missed, 2, "the partial progress is still reported");
    assert.match(cut.lines.at(-1), /budget of 3 min exceeded after 5 mutants tested/);
    const base = await judge(opts, { version: installed, runCargo: async () => ({ status: 4, stdout: "", stderr: "" }) });
    assert.match(base.lines.at(-1), /^RESULT FAIL: unmutated baseline failed/);
    await assert.rejects(judge(opts, { version: installed, runCargo: async () => ({ status: 5, stdout: "", stderr: "ERROR Diff content doesn't match source file" }) }), /exited 5.*Diff content/);
    await assert.rejects(judge(opts, { version: installed, runCargo: async () => ({ status: 0, stdout: "", stderr: "" }) }), /no outcomes\.json/);
    await assert.rejects(judge(opts, { version: () => ({ status: 101 }), runCargo: async () => assert.fail("cargo must not run") }), /cargo install --locked cargo-mutants/);
    await assert.rejects(judge(opts, { version: installed, runCargo: async () => ({ error: new Error("spawn cargo ENOENT") }) }), /could not run cargo/);
  } finally { done(); }
});

test("judge refuses a patch that cannot map onto the root and skips patches without Rust", async () => {
  const wrong = fixture(["diff --git a/crates/y/src/lib.rs b/crates/y/src/lib.rs", "--- a/crates/y/src/lib.rs", "+++ b/crates/y/src/lib.rs", "@@ -0,0 +1 @@", "+x", ""].join("\n"));
  const docs = fixture(["diff --git a/README.md b/README.md", "--- a/README.md", "+++ b/README.md", "@@ -0,0 +1 @@", "+words", ""].join("\n"));
  const none = fixture("");
  writeFileSync(join(docs.dir, "README.md"), "words\n");
  try {
    await assert.rejects(judge(wrong.opts, { version: installed }), /missing under root.*--root/);
    const report = await judge(docs.opts, { version: installed, runCargo: async () => assert.fail("no Rust lines, no cargo") });
    assert.equal(report.ok, true);
    assert.match(report.lines.at(-1), /adds no Rust lines/);
    await assert.rejects(judge(none.opts, { version: installed }), /no usable git patch/);
    await assert.rejects(judge({ ...none.opts, root: join(none.dir, "absent") }, { version: installed }), /does not exist/);
  } finally { wrong.done(); docs.done(); none.done(); }
});

test("--from-out re-judges a finished run without cargo", async () => {
  const { dir, done } = fixture();
  try {
    outcomesFile(dir, recorded);
    const base = { fromOut: join(dir, "out"), maxMissed: 0 };
    const report = await judge(base);
    assert.equal(report.ok, false);
    assert.equal(report.missed, 1);
    assert.equal((await judge({ ...base, maxMissed: 1 })).ok, true);
    assert.equal((await judge({ ...base, fromOut: join(dir, "out", "mutants.out", "outcomes.json") })).missed, 1);
    await assert.rejects(judge({ ...base, fromOut: join(dir, "nowhere") }), /no outcomes\.json/);
    assert.equal(JSON.parse(readFileSync(join(dir, "out", "mutants.out", "outcomes.json"), "utf8")).total_mutants, 2);
  } finally { done(); }
});

import assert from "node:assert/strict";
import test from "node:test";
import { FRAME_P95_BAR_MS, frameCsvVerdict, GOLDEN_VERIFY_RUNS, changedCrateDirs, goldenVerdict, hasNextestProfile, irGateVerdict, parityVerdict, parseParityLine, NEXTEST_CONFIG, NEXTEST_PROFILE, nextestFlakeSummary, nextestTestCommand, packageNameOf, parseProofArgs, runsToolTests, toolTestSummary } from "./proof.mjs";

test("parseProofArgs: empty and single-flag runs pass through", () => {
  assert.deepEqual(parseProofArgs([]), []);
  assert.deepEqual(parseProofArgs(["--skip-build"]), ["--skip-build"]);
  assert.deepEqual(parseProofArgs(["--perf"]), ["--perf"]);
  assert.deepEqual(parseProofArgs(["--mutants"]), ["--mutants"]);
});

test("parseProofArgs: two flags or an unknown flag throw the usage line", () => {
  assert.throws(() => parseProofArgs(["--skip-build", "--perf"]), /Usage: node tools\/proof\.mjs/);
  assert.throws(() => parseProofArgs(["--mutants", "--perf"]), /Usage: node tools\/proof\.mjs/);
  assert.throws(() => parseProofArgs(["--bogus"]), /Usage: node tools\/proof\.mjs/);
  assert.throws(() => parseProofArgs(["--verify"]), /Usage: node tools\/proof\.mjs/);
});

test("toolTestSummary: picks the node --test counters out of mixed output", () => {
  const out = ["TAP version 13", "ok 1 - something", "# pass 12", "# fail 0", "# duration_ms 345"].join("\n");
  assert.equal(toolTestSummary(out), "# pass 12, # fail 0");
});

test("toolTestSummary: empty or counter-less output summarizes to nothing", () => {
  assert.equal(toolTestSummary(""), "");
  assert.equal(toolTestSummary(undefined), "");
  assert.equal(toolTestSummary("RESULT PASS (proof fixtures only)\n"), "");
});

// Gungraun summaries as cargo-bench prints them (recorded 2026-10-03, 0.20.0).
const IR_GREEN = [
  "instructions::simulation::arena_step fighters_16:(ArenaTick :: canonical())",
  "  Instructions:                                   14430630|14430630             (No change)",
  "instructions::simulation::physics_step bodies_256:(PhysicsStep :: canonical())",
  "  Instructions:                                   15466911|15466911             (No change)",
  "instructions::simulation::ecs_spine entities_1000:(spine_fixture(1_000))",
  "  Instructions:                                   11729310|11729310             (No change)",
  "Gungraun result: Ok. 3 without regressions; 0 regressed; 0 filtered; 3 benchmarks finished in 1.51071s",
].join("\n");

test("irGateVerdict: three nonzero counts against a baseline pass", () => {
  const verdict = irGateVerdict({ status: 0, stdout: IR_GREEN, stderr: "" });
  assert.equal(verdict.ok, true, verdict.line);
  assert.deepEqual(verdict.rows.map((r) => r.now), [14430630, 15466911, 11729310]);
});

test("irGateVerdict: a regression past the budget fails and names the counts", () => {
  const regressed = IR_GREEN.replace("11729310|11729310             (No change)", "12315771|11729310             (+4.99996%) [+1.05000x]")
    + "\n    Callgrind: Instructions (11729310 -> 12315771): +4.99996% exceeds limit of +2.00000%\n";
  const verdict = irGateVerdict({ status: 3, stdout: regressed, stderr: "error: bench failed" });
  assert.equal(verdict.ok, false);
  assert.match(verdict.line, /Ir over the budget: 11729310 -> 12315771, \+4\.99996%/);
});

test("irGateVerdict: no baseline, a zero count or a missing bench is never green", () => {
  const noBaseline = IR_GREEN.replace("14430630|14430630", "14430630|N/A");
  assert.match(irGateVerdict({ status: 0, stdout: noBaseline }).line, /no perf_gate baseline/);
  const zero = IR_GREEN.replace("15466911|15466911", "0|15466911");
  assert.match(irGateVerdict({ status: 0, stdout: zero }).line, /counted 0 instructions/);
  const two = IR_GREEN.split("\n").slice(2).join("\n");
  assert.match(irGateVerdict({ status: 0, stdout: two }).line, /2 of 3 benches/);
  assert.match(irGateVerdict({ status: 101, stdout: "", stderr: "valgrind: not found" }).line, /exited 101/);
  assert.match(irGateVerdict({ error: new Error("spawnSync cargo ENOENT") }).line, /ENOENT/);
});

test("changedCrateDirs: crate src/ edits only, sorted and unique", () => {
  const diff = [
    "diff --git a/crates/nav/src/flow.rs b/crates/nav/src/flow.rs",
    "+++ b/crates/nav/src/flow.rs",
    "+++ b/crates/core/src/hash.rs",
    "+++ b/crates/nav/src/tiles.rs",
    "+++ b/crates/perf/benches/instructions.rs",
    "+++ b/tools/proof.mjs",
  ].join("\n");
  assert.deepEqual(changedCrateDirs(diff), ["core", "nav"]);
  assert.deepEqual(changedCrateDirs(""), []);
});

test("packageNameOf: the [package] name, not a bin, bench or lib name", () => {
  const manifest = '[lib]\nname = "not_this"\n\n[package]\nname = "engine-perf"\nversion.workspace = true\n\n[[bench]]\nname = "instructions"\n';
  assert.equal(packageNameOf(manifest), "engine-perf");
  assert.equal(packageNameOf('[package]\r\nname = "e2040-editor"\r\n'), "e2040-editor");
  assert.equal(packageNameOf('[[bin]]\nname = "tool"\n'), "");
});

test("nextest profile: constants point at the proof profile and its file", () => {
  assert.equal(NEXTEST_PROFILE, "proof");
  assert.equal(NEXTEST_CONFIG, ".config/nextest.toml");
  assert.equal(hasNextestProfile("[profile.proof]\nfail-fast = false\n"), true);
  assert.equal(hasNextestProfile("[profile.default]\nfail-fast = false\n"), false);
  assert.equal(hasNextestProfile(""), false);
});

test("nextestTestCommand: cargo test legs map under the proof profile", () => {
  assert.deepEqual(
    nextestTestCommand(["test", "-j4", "--locked", "--workspace", "--all-targets"]),
    ["nextest", "run", "--profile", "proof", "--cargo-profile", "fast-test", "--locked", "--workspace", "--all-targets"],
  );
  assert.deepEqual(
    nextestTestCommand(["test", "--locked", "-p", "engine-render", "--all-targets", "--all-features"]),
    ["nextest", "run", "--profile", "proof", "--cargo-profile", "fast-test", "--locked", "-p", "engine-render", "--all-targets", "--all-features"],
  );
  assert.equal(nextestTestCommand(["build", "--locked", "--workspace"]), null);
  assert.equal(nextestTestCommand(["clippy", "--locked"]), null);
});

test("nextestFlakeSummary: retry lines count, silence counts zero", () => {
  assert.equal(nextestFlakeSummary("ok\n"), "0 retried");
  assert.equal(nextestFlakeSummary(""), "0 retried");
  assert.equal(nextestFlakeSummary("test foo ... RETRIED (1 retry)\ntest bar ... ok\n"), "1 retried");
  assert.equal(nextestFlakeSummary("FLAKY test baz passed after 2 retries\n"), "2 retried");
});

test("fixture-only: golden verifies once, tool tests run in CI nightly only", () => {
  assert.equal(GOLDEN_VERIFY_RUNS, 1);
  assert.equal(runsToolTests(["--skip-build"], {}), false);
  assert.equal(runsToolTests([], {}), false);
  assert.equal(runsToolTests([], { E2040_NIGHTLY: "1" }), true);
  assert.equal(runsToolTests(["--skip-build"], { E2040_NIGHTLY: "1" }), false);
});

test("goldenVerdict: hash law plus version pin in one receipt", () => {
  const good = { final_hash: "d57b5c8ae3096614", golden_checked: true, verified: true, engine_version: "0.1.0" };
  assert.equal(goldenVerdict(good, "0.1.0"), true);
  assert.equal(goldenVerdict({ ...good, final_hash: "0000000000000000" }, "0.1.0"), false);
  assert.equal(goldenVerdict({ ...good, engine_version: "0.0.9" }, "0.1.0"), false);
  assert.equal(goldenVerdict({ ...good, verified: false }, "0.1.0"), false);
  assert.equal(goldenVerdict({ ...good, golden_checked: false }, "0.1.0"), false);
  assert.equal(goldenVerdict(good, ""), false);
});

test("frameCsvVerdict: 100 steady rows pass", () => {
  const rows = ["frame,wall_ms,work_ms"];
  for (let i = 0; i < 100; i += 1) {
    rows.push(`${i},16.7,10`);
  }
  const verdict = frameCsvVerdict(rows.join("\n"));
  assert.equal(verdict.ok, true);
  assert.equal(verdict.frames, 100);
  assert.equal(verdict.p50, 16.7);
  assert.equal(verdict.p95, 16.7);
  assert.match(verdict.line, /^\[PASS\]/);
});

test("frameCsvVerdict: slow tail fails on p95", () => {
  const rows = ["frame,wall_ms,work_ms"];
  for (let i = 0; i < 80; i += 1) {
    rows.push(`${i},16,10`);
  }
  for (let i = 80; i < 100; i += 1) {
    rows.push(`${i},60,10`);
  }
  const verdict = frameCsvVerdict(rows.join("\n"));
  assert.equal(verdict.ok, false);
  assert.equal(verdict.p95, 60);
  assert.match(verdict.line, /^\[FAIL\]/);
  assert.match(verdict.line, /p95=60\.000/);
});

test("frameCsvVerdict: same text twice gives equal results", () => {
  const rows = ["frame,wall_ms,work_ms", "0,16.7,10", "1,16.7,10", "2,20,10"];
  const csv = rows.join("\n");
  assert.deepEqual(frameCsvVerdict(csv), frameCsvVerdict(csv));
});

test("frameCsvVerdict: bad shapes fail", () => {
  const noWall = ["frame,work_ms", "0,10"].join("\n");
  const badNoWall = frameCsvVerdict(noWall);
  assert.equal(badNoWall.ok, false);
  assert.match(badNoWall.line, /^\[FAIL\] frame times:/);
  const badEmpty = frameCsvVerdict("");
  assert.equal(badEmpty.ok, false);
  assert.match(badEmpty.line, /^\[FAIL\] frame times:/);
  const badAbc = ["frame,wall_ms,work_ms", "0,abc,10"].join("\n");
  const badAbcVerdict = frameCsvVerdict(badAbc);
  assert.equal(badAbcVerdict.ok, false);
  assert.match(badAbcVerdict.line, /^\[FAIL\] frame times:/);
});

test("frameCsvVerdict: column order and CRLF do not matter", () => {
  const lfRows = ["frame,work_ms,wall_ms,note", "0,10,16.7,hi", "1,11,20,ho"];
  const lf = frameCsvVerdict(lfRows.join("\n"));
  assert.equal(lf.frames, 2);
  assert.equal(lf.p50, 16.7);
  assert.equal(lf.p95, 20);
  const crlf = frameCsvVerdict(lfRows.join("\r\n"));
  assert.deepEqual(crlf, lf);
});

test("frameCsvVerdict: bar is inclusive at 50 ms", () => {
  assert.equal(FRAME_P95_BAR_MS, 50);
  const rows = ["frame,wall_ms,work_ms"];
  for (let i = 0; i < 100; i += 1) {
    rows.push(`${i},50,10`);
  }
  const verdict = frameCsvVerdict(rows.join("\n"));
  assert.equal(verdict.p95, 50);
  assert.equal(verdict.ok, true);
  assert.match(verdict.line, /^\[PASS\]/);
});

test("frameCsvVerdict: real timed-smoke header parses", () => {
  const csv = ["frame,wall_ms,input_to_present_ms", "0,16.667,12.000", "1,20.000,12.100"].join("\n");
  const verdict = frameCsvVerdict(csv);
  assert.equal(verdict.ok, true);
  assert.equal(verdict.frames, 2);
  assert.equal(verdict.p50, 16.667);
  assert.equal(verdict.p95, 20);
  assert.match(verdict.line, /^\[PASS\]/);
});

// Parity columns as the bin prints them: the first is the clean build, the
// second the seeded-wall canary (the same line the Rust test pins).
const PARITY_CLEAN = "spine parity: seeds=20 ticks=600 match=20 mismatch=0 first=none";
const PARITY_SEEDED = "spine parity: seeds=20 ticks=600 match=8 mismatch=12 first=seed 1001 tick 105 Column(Transform)";

test("parseParityLine: reads the scorecard column and ignores other lines", () => {
  const col = parseParityLine(`warning: noise
${PARITY_SEEDED}
`);
  assert.deepEqual({ ...col, line: undefined }, { seeds: 20, ticks: 600, matched: 8, mismatch: 12, first: "seed 1001 tick 105 Column(Transform)", line: undefined });
  assert.equal(parseParityLine("nothing here"), null);
});

test("parityVerdict: all 20 tapes matching passes, one divergence fails the gate", () => {
  assert.deepEqual(parityVerdict({ status: 0, stdout: `${PARITY_CLEAN}\n` }), { ok: true, line: PARITY_CLEAN });
  const red = parityVerdict({ status: 1, stdout: `${PARITY_SEEDED}\n` });
  assert.equal(red.ok, false);
  assert.match(red.line, /first=seed 1001 tick 105/);
  // A clean-looking line with a nonzero exit is still red.
  assert.equal(parityVerdict({ status: 1, stdout: PARITY_CLEAN }).ok, false);
  assert.match(parityVerdict({ status: 101, stdout: "", stderr: "error: no bin" }).line, /no parity line: error: no bin/);
});

test("parityVerdict: the canary is green only when the seeded divergence is caught", () => {
  const caught = parityVerdict({ status: 1, stdout: PARITY_SEEDED }, { canary: true });
  assert.equal(caught.ok, true);
  assert.match(caught.line, /^canary caught: /);
  const missed = parityVerdict({ status: 0, stdout: PARITY_CLEAN }, { canary: true });
  assert.equal(missed.ok, false);
  assert.match(missed.line, /^canary NOT caught: /);
});

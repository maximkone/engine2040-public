import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { FRAME_P95_BAR_MS, GREEN_MAX_AGE_HOURS, bookBuildVerdict, driftVerdict, frameCsvVerdict, gpuLiveLine, gpuLiveReceipt, ranAtLeastOneTest, recordGreen, GOLDEN_VERIFY_RUNS, changedCrateDirs, goldenVerdict, hasNextestProfile, irGateVerdict, parityVerdict, parseParityLine, NEXTEST_CONFIG, NEXTEST_PROFILE, nextestFlakeSummary, nextestTestCommand, packageNameOf, parseProofArgs, runsToolTests, toolTestSummary } from "./proof.mjs";

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

test("parseProofArgs: --drift is a flag of its own", () => {
  assert.deepEqual(parseProofArgs(["--drift"]), ["--drift"]);
  assert.throws(() => parseProofArgs(["--drift", "--perf"]), /Usage: node tools\/proof\.mjs/);
});

const RAN_NOTHING = [
  "running 0 tests",
  "",
  "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 214 filtered out; finished in 0.00s",
].join("\n");

test("ranAtLeastOneTest: a filter that matches nothing is RED, a real run is not", () => {
  assert.equal(ranAtLeastOneTest(RAN_NOTHING), false);
  assert.equal(ranAtLeastOneTest(""), false);
  assert.equal(ranAtLeastOneTest("running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"), true);
  assert.equal(ranAtLeastOneTest("test result: FAILED. 0 passed; 1 failed; 0 ignored"), true, "a failing test still ran");
  assert.equal(ranAtLeastOneTest(`${RAN_NOTHING}\n${RAN_NOTHING.replace("0 passed", "2 passed")}`), true, "one empty binary beside a real one is fine");
  assert.equal(ranAtLeastOneTest("Summary [   0.010s] 0 tests run: 0 passed, 214 skipped"), false);
  assert.equal(ranAtLeastOneTest("Summary [   0.510s] 4 tests run: 4 passed, 210 skipped"), true);
});

const HEAD = "a".repeat(40);
const GREEN_AT = new Date("2026-10-18T00:00:00.000Z");
const hoursLater = (h) => new Date(GREEN_AT.getTime() + h * 3_600_000);
const green = { gate: "full", sha: HEAD, time: GREEN_AT.toISOString() };

test("driftVerdict: a fresh record on HEAD passes, a 25 h old one FAILs", () => {
  const fresh = driftVerdict("full", green, { head: HEAD, isAncestor: true, now: hoursLater(24.9) });
  assert.equal(fresh.ok, true, fresh.line);
  assert.match(fresh.line, /^\[PASS\] drift full: green aaaaaaa 24\.9 h ago/);
  assert.equal(GREEN_MAX_AGE_HOURS, 25);
  const old = driftVerdict("full", green, { head: HEAD, isAncestor: true, now: hoursLater(25) });
  assert.equal(old.ok, false);
  assert.match(old.line, /^\[FAIL\] drift full: green record is 25\.0 h old \(limit 25 h\)/);
});

test("driftVerdict: a sha outside HEAD's history, a missing or a broken record FAIL", () => {
  const behind = { head: "b".repeat(40), now: hoursLater(1) };
  assert.equal(driftVerdict("full", green, { ...behind, isAncestor: true }).ok, true, "an ancestor of HEAD is fine");
  assert.match(driftVerdict("full", green, { ...behind, isAncestor: false }).line, /green sha aaaaaaa is neither HEAD nor an ancestor/);
  assert.match(driftVerdict("full", null, { ...behind, isAncestor: false }).line, /no green record in target\/proof\/full-latest\.json/);
  assert.match(driftVerdict("full", { ...green, time: "never" }, { ...behind, isAncestor: true }).line, /not a past time/);
  assert.match(driftVerdict("full", green, { head: HEAD, isAncestor: true, now: hoursLater(-1) }).line, /not a past time/);
});

test("recordGreen: a green run writes the record, a red run leaves the old one alone", () => {
  const dir = mkdtempSync(join(tmpdir(), "proof-green-"));
  try {
    assert.equal(recordGreen("full", { failed: true, sha: HEAD, now: GREEN_AT, dir }), null);
    assert.deepEqual(readdirSync(dir), [], "a red first run writes nothing");
    const path = recordGreen("full", { failed: false, sha: HEAD, now: GREEN_AT, dir });
    assert.equal(path, join(dir, "full-latest.json"));
    assert.deepEqual(JSON.parse(readFileSync(path, "utf8")), green);
    const before = readFileSync(path, "utf8");
    assert.equal(recordGreen("full", { failed: true, sha: "c".repeat(40), now: hoursLater(5), dir }), null);
    assert.equal(readFileSync(path, "utf8"), before, "the red run did not overwrite the green record");
    recordGreen("fixtures", { failed: false, sha: HEAD, now: GREEN_AT, dir });
    assert.deepEqual(readdirSync(dir).sort(), ["fixtures-latest.json", "full-latest.json"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

const DEVICE_REPORT = [
  "# pass truth 2026-10-17",
  "",
  "| row | verdict | diff | source | region | counted | pass |",
  "| graph | LIVE | 10057 | live | frame | 196608 | submit_gpu |",
  "| shadow_cluster | PROBE | 0 | live | geometry | 10059 | submit_gpu |",
  "| sky | LIVE | 186549 | live | sky | 186549 | submit_gpu_sky |",
  "| post | PROBE | 0 | live | frame | 196608 | submit_gpu |",
].join("\n");

test("gpuLiveReceipt: only rows read from the live frame count, anything else is not_wired", () => {
  assert.deepEqual(gpuLiveReceipt(DEVICE_REPORT), { live: 2, probe: 2 });
  const stubs = "| row | verdict | diff | pass |\n| graph | LIVE | 1 | submit_gpu |\n| sky | PROBE | 0 | submit_gpu_sky |\n";
  assert.equal(gpuLiveReceipt(stubs), null, "synthetic stub rows have no live source");
  assert.equal(gpuLiveReceipt(DEVICE_REPORT.replaceAll("| live |", "| synth |")), null);
  assert.equal(gpuLiveReceipt(""), null);
  assert.equal(gpuLiveReceipt(undefined), null);
});

test("gpuLiveLine: counts come from the device report, else the line says not_wired", () => {
  const wired = gpuLiveLine({ live: 2, probe: 2 }, "pass-truth-2026-10-17.md", "ok");
  assert.match(wired, /LIVE 2\/PROBE 2 from the device report pass-truth-2026-10-17\.md/);
  assert.doesNotMatch(wired, /not_wired/);
  const unwired = gpuLiveLine(null, undefined, "ok");
  assert.match(unwired, /not_wired/);
  assert.doesNotMatch(unwired, /\d LIVE|LIVE \d/, "no LIVE count without a device report");
});

test("bookBuildVerdict: exit 0 with INFO lines only is green, a WARN or ERROR line or a bad exit is not", () => {
  const info = " INFO Book building has started\n INFO Running the html backend\n INFO HTML book written to `target/book-check`\n";
  assert.deepEqual(bookBuildVerdict({ status: 0, stdout: "", stderr: info }), { ok: true, line: "docs/book builds clean" });
  const warned = bookBuildVerdict({ status: 0, stdout: "", stderr: `${info} WARN The "linkcheck" renderer failed\n` });
  assert.equal(warned.ok, false);
  assert.match(warned.line, /mdbook build warned: WARN The "linkcheck" renderer failed/);
  const broken = bookBuildVerdict({ status: 101, stdout: "", stderr: `${info}ERROR Chapter file not found, ./gone.md\r\n` });
  assert.equal(broken.ok, false);
  assert.match(broken.line, /mdbook build exited 101: ERROR Chapter file not found/);
  assert.match(bookBuildVerdict({ status: 1, stdout: "", stderr: "" }).line, /^mdbook build exited 1$/);
  assert.match(bookBuildVerdict({ error: new Error("spawn mdbook ENOENT") }).line, /mdbook did not run: spawn mdbook ENOENT/);
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

// OPT6 spike: one proof lane under cargo-nextest against the same lane under
// cargo test.
//
// The row is parked (2026-10-05: cargo-nextest is absent on this PC, exit 101;
// "retry only after the owner installs it and posts the version; never
// re-probe whole"). So this tool never runs a whole-workspace nextest probe:
// it asks cargo for the nextest version, and when cargo has no nextest it
// stops there with the exact error and exit 2 (could not run).
//
// With nextest installed it runs ONE lane twice - the same crate, the same
// filter, the same job count - once under `cargo test` (the before) and once
// under `cargo nextest run` (the after), and compares:
//   - wall time (the bar: nextest must be faster)
//   - the failures (the bar: the same failures must be visible, retries
//     reported, never hidden)
//
// Run: node tools/nextest-spike.mjs --crate <name> [--filter <name>] [--jobs <n>] [--json]
// Exit 0 when both legs ran and the verdict is in the table, 2 when the lane
// could not run (nextest absent, or a leg failed).
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { slotGate } from "./jobserver.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

export const DEFAULT_JOBS = 4;

// The proof lane: one crate, one filter, four jobs (the job count that shows
// the flake fingerprint).
export function laneArgs({ crate, filter, jobs }) {
  const args = ["-j", String(jobs), "--locked", "-p", crate, "--lib"];
  if (filter) args.push(filter);
  return args;
}

export function testArgs({ crate, filter, jobs }) {
  return ["test", ...laneArgs({ crate, filter, jobs })];
}

export function nextestArgs({ crate, filter, jobs }) {
  return ["nextest", "run", ...laneArgs({ crate, filter, jobs })];
}

// The nextest version, or the exact error cargo gave. Never a guess.
export function readNextestVersion(runner = spawnSync) {
  const r = runner("cargo", ["nextest", "--version"], { encoding: "utf8" });
  if (r.status !== 0) {
    const err = `${r.stderr ?? ""}${r.stdout ?? ""}`.trim().split("\n").filter(Boolean);
    const line = err.find((l) => /no such command|not found|nextest/i.test(l)) ?? err[0] ?? "cargo nextest is not installed";
    return { ok: false, version: null, error: line.trim() };
  }
  return { ok: true, version: `${r.stdout ?? ""}`.trim() || "unknown", error: null };
}

// A failed test line: "test foo ... FAILED" (cargo test) or
// "FAIL [   0.005s] engine-core foo" (nextest).
export function failedTests(output) {
  const out = new Set();
  for (const line of String(output ?? "").split(/\r?\n/)) {
    let m = /^\s*test\s+(.+?)\s+\.\.\.\s+FAILED/.exec(line);
    if (m) { out.add(m[1]); continue; }
    m = /^\s*FAIL\s+\[\s*[\d.]+s\]\s+(\S+)\s+(\S+)/.exec(line);
    if (m) { out.add(`${m[1]} ${m[2]}`); continue; }
    m = /^\s*FAIL\s+(\S+)\s+(\S+)/.exec(line);
    if (m) out.add(`${m[1]} ${m[2]}`);
  }
  return [...out].sort();
}

// The tests a run reported, by name, from either runner.
export function passedTests(output) {
  const out = new Set();
  for (const line of String(output ?? "").split(/\r?\n/)) {
    let m = /^test\s+(.+?)\s+\.\.\.\s+ok/.exec(line);
    if (m) { out.add(m[1]); continue; }
    m = /^\s*PASS\s+\[\s*[\d.]+s\]\s+(\S+)\s+(\S+)/.exec(line);
    if (m) out.add(`${m[1]} ${m[2]}`);
  }
  return [...out].sort();
}

// The retries nextest prints: "TRY 2 FAIL" / "TRY 2 PASS". They are reported,
// never hidden.
export function retriedTests(output) {
  const out = [];
  for (const line of String(output ?? "").split(/\r?\n/)) {
    const m = /^\s*TRY\s+(\d+)\s+(FAIL|PASS)\s+\[\s*[\d.]+s\]\s+(\S+)\s+(\S+)/.exec(line);
    if (m) out.push({ try: Number(m[1]), result: m[2], test: `${m[3]} ${m[4]}` });
  }
  return out;
}

export function verdict({ testSeconds, nextestSeconds, testFailures, nextestFailures, retries }) {
  const reasons = [];
  if (!Number.isFinite(testSeconds) || !Number.isFinite(nextestSeconds) || testSeconds <= 0) reasons.push("no wall-time pair");
  const faster = Number.isFinite(testSeconds) && Number.isFinite(nextestSeconds) && nextestSeconds < testSeconds;
  if (!faster) reasons.push("nextest was not faster");
  const sameFailures = JSON.stringify(testFailures) === JSON.stringify(nextestFailures);
  if (!sameFailures) reasons.push(`the failures differ: cargo test [${testFailures.join(", ")}] vs nextest [${nextestFailures.join(", ")}]`);
  return { keep: reasons.length === 0, faster, sameFailures, retries: retries.length, reasons };
}

export function parseArgs(argv) {
  const out = { crate: "", filter: "", jobs: DEFAULT_JOBS, json: false, force: false, error: null };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--crate") { out.crate = argv[++i] ?? ""; if (!out.crate) { out.error = "--crate needs a crate name"; return out; } }
    else if (a === "--filter") { out.filter = argv[++i] ?? ""; if (!out.filter) { out.error = "--filter needs a test name"; return out; } }
    else if (a === "--jobs") { const n = Number(argv[++i]); if (!Number.isInteger(n) || n < 1) { out.error = "--jobs needs a whole number of 1 or more"; return out; } out.jobs = n; }
    else if (a === "--json") out.json = true;
    else if (a === "--force") out.force = true;
    else if (a === "-h" || a === "--help") out.help = true;
    else { out.error = `unknown flag ${a}`; return out; }
  }
  if (!out.crate) { out.error = "--crate needs a crate name"; return out; }
  return out;
}

export function helpText() {
  return [
    "Usage: node tools/nextest-spike.mjs --crate <name> [--filter <name>] [--jobs <n>] [--json] [--force]",
    "  Runs one proof lane under cargo test and under cargo nextest and compares",
    "  the wall time and the failures (retries reported).",
    "  With no free compile slot in the pool it refuses to start (exit 2, one line);",
    "  --force runs it anyway.",
    "  Without cargo-nextest the tool stops with cargo's own error and exit 2:",
    "  the row is parked until the owner installs nextest.",
  ].join("\n") + "\n";
}

const runLeg = (args) => {
  const t0 = performance.now();
  const r = spawnSync("cargo", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28 });
  const seconds = Math.round((performance.now() - t0) / 100) / 10;
  return { status: r.status, seconds, output: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

const main = (argv) => {
  const a = parseArgs(argv);
  if (a.help) { process.stdout.write(helpText()); return 0; }
  if (a.error) { console.log(`refused: ${a.error}`); return 2; }
  // A spike waits for a free compile slot: with the pool up and no slot free it
  // must not start, so it never competes with the live work (--force runs it anyway).
  // The gate comes first, so a refusal is one line and nothing runs.
  const gate = slotGate({ force: a.force });
  if (!gate.ok) {
    console.log(`refused: ${gate.reason}: a spike must not fight the live work (retry when a slot frees, or pass --force)`);
    return 2;
  }
  console.log(`[pool] ${gate.reason}`);
  // Then the cheap nextest version probe: it compiles nothing, and the row stays
  // parked with its own message when nextest is not installed.
  const nextest = readNextestVersion();
  if (!nextest.ok) {
    console.log(`RESULT could not run: cargo nextest is not installed (${nextest.error})`);
    console.log("The row stays parked: retry only after the owner installs nextest and posts the version.");
    return 2;
  }
  console.log(`nextest: ${nextest.version}`);
  const test = runLeg(testArgs(a));
  if (test.status !== 0) {
    console.log(test.output.trimEnd().split("\n").slice(-20).join("\n"));
    console.log("RESULT could not run: the cargo test leg failed");
    return 2;
  }
  const next = runLeg(nextestArgs(a));
  if (next.status !== 0) {
    console.log(next.output.trimEnd().split("\n").slice(-20).join("\n"));
    console.log("RESULT could not run: the nextest leg failed");
    return 2;
  }
  const testFailures = failedTests(test.output);
  const nextestFailures = failedTests(next.output);
  const retries = retriedTests(next.output);
  const v = verdict({ testSeconds: test.seconds, nextestSeconds: next.seconds, testFailures, nextestFailures, retries });
  const report = {
    row: "OPT6",
    ran_utc: new Date().toISOString(),
    crate: a.crate,
    filter: a.filter || null,
    jobs: a.jobs,
    nextest_version: nextest.version,
    test: { seconds: test.seconds, passed: passedTests(test.output).length, failures: testFailures },
    nextest: { seconds: next.seconds, passed: passedTests(next.output).length, failures: nextestFailures, retries },
    verdict: v,
  };
  console.log("");
  console.log("| leg | wall s | tests passed | failures |");
  console.log("|---|---|---|---|");
  console.log(`| cargo test | ${test.seconds} | ${report.test.passed} | ${testFailures.length} |`);
  console.log(`| cargo nextest | ${next.seconds} | ${report.nextest.passed} | ${nextestFailures.length} |`);
  console.log(`retries reported: ${retries.length}${retries.length ? ` (${retries.map((r) => `${r.test} try ${r.try} ${r.result}`).join(", ")})` : ""}`);
  console.log(`RESULT ${v.keep ? "KEEP" : "DROP"}: nextest ${next.seconds} s vs cargo test ${test.seconds} s${v.reasons.length ? ` (${v.reasons.join("; ")})` : ""}`);
  if (a.json) console.log(JSON.stringify(report, null, 2));
  return 0;
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));

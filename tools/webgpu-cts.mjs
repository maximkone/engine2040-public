// webgpu-cts.mjs - the WebGPU conformance gate (HX4).
//
// The strict WebGPU flag already exists in the render stack
// (`crates/render/src/backend.rs`, `E2040_STRICT_WEBGPU=1`), but nothing
// measured against the official suite: the flag was a promise, not a number.
// This tool runs a chosen slice of the WebGPU conformance test suite
// (gpuweb/cts, BSD-3-Clause) against the strict instance and prints one
// count line.
//
// What is ported: the `shader,validation` area of the suite, where every
// case is one WGSL program and one expected outcome. That is the part a
// native wgpu instance can answer: the device either compiles the program
// or refuses it with a validation error. The other areas (api,validation,
// api,operation, shader,execution) need the whole harness and a browser
// oracle, so they are not in this slice.
//
// Donors:
//   - gpuweb/cts@e97492c38728b11ee336bf3ddf513cb27f1331ed (BSD-3-Clause):
//     the case names, the WGSL and the expected outcome.
//   - infosia/webgpu-native-cts@main (expectations/wgpu-native-vulkan.txt):
//     the shape of the gate: an expectations file names the cases a backend
//     gets wrong, so a known defect is reported as expected instead of
//     blocking the run.
//
// Usage:
//   node tools/webgpu-cts.mjs --list         print the chosen cases and their expectations
//   node tools/webgpu-cts.mjs                run the slice
//   node tools/webgpu-cts.mjs --json         run the slice, print the receipt as JSON
//   node tools/webgpu-cts.mjs --help
//
// Rules:
//   - one case per process. A driver abort in one case must not take the
//     run with it, so every case is a fresh child process.
//   - the instance must be strict. A run without `E2040_STRICT_WEBGPU=1`
//     proves nothing and fails.
//   - every case is judged against the suite's expectation. The suite says
//     what the device must do: compile the program, or refuse it. A device
//     that does the opposite is a conformance failure, not a pass.
//   - a case named in the expectations file is reported as `expected`, never
//     as `pass`. It is a known backend defect, and the count line keeps it
//     in its own column.
//   - the run never touches the golden hash and never writes a tracked file.
//
// Exit: 0 the slice ran and every case passed or was expected, 1 a case
// failed, crashed or the instance was not strict, 2 the tool could not run
// (no runner binary, unreadable case file).
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
export const ROOT = resolve(HERE, "..");

/** The chosen cases: the ported slice of the official suite. */
export const DEFAULT_CASES = join(HERE, "webgpu-cts", "cases.json");
/** The expectations file: the cases this backend is known to get wrong. */
export const DEFAULT_EXPECTATIONS = join(HERE, "expectations", "wgpu.txt");
/** The env name of the strict switch, the render stack's own name. */
export const STRICT_ENV = "E2040_STRICT_WEBGPU";
/** The runner binary: one case per process. */
export const DEFAULT_RUNNER = join(ROOT, "target", "debug", "webgpu_cts.exe");
/** The runner's receipt line. */
export const CASE_LINE = "webgpu_cts_case:";

export const HELP = `Usage: node tools/webgpu-cts.mjs [--list] [--json] [--cases FILE] [--expectations FILE] [--runner PATH] [--help]

The WebGPU conformance gate (HX4): a chosen slice of the official WebGPU
conformance test suite (gpuweb/cts, BSD-3-Clause) run against the strict
WebGPU instance of the render stack.

  --list              print the chosen cases and their expectations
  --json              print the receipt as JSON
  --cases FILE        the case file (default tools/webgpu-cts/cases.json)
  --expectations FILE the expectations file (default tools/expectations/wgpu.txt)
  --runner PATH       the runner binary (default target/debug/webgpu_cts.exe)
  --runner-arg ARG    the argument the runner gets (default --wgsl-stdin)

The run needs a GPU and the strict switch:
  E2040_STRICT_WEBGPU=1 node tools/webgpu-cts.mjs

Exit: 0 the slice ran, 1 a case failed or crashed or the instance was not
strict, 2 the tool could not run.`;

/** The three states a case can end in. `expected` is a known defect. */
export const PASS = "pass";
export const FAIL = "fail";
export const CRASH = "crash";
export const EXPECTED = "expected";

/** What the device said about one program: it compiled it, or refused it. */
export const COMPILED = "compiled";
export const REJECTED = "rejected";

/** Read the case file. Throws with the path when it is not there. */
export function loadCases(path = DEFAULT_CASES) {
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (err) {
    throw new Error(`cannot read the case file ${path}: ${err.message}`);
  }
  let data;
  try {
    data = JSON.parse(text);
  } catch (err) {
    throw new Error(`the case file ${path} is not JSON: ${err.message}`);
  }
  const cases = Array.isArray(data) ? data : data?.cases;
  if (!Array.isArray(cases) || !cases.length) {
    throw new Error(`the case file ${path} holds no cases`);
  }
  for (const one of cases) {
    if (typeof one?.id !== "string" || !one.id) throw new Error(`a case in ${path} has no id`);
    if (typeof one?.wgsl !== "string") throw new Error(`the case ${one.id} has no wgsl`);
    if (one.expect !== PASS && one.expect !== FAIL) {
      throw new Error(`the case ${one.id} has the unknown expectation ${one.expect}`);
    }
  }
  return { donor: data?.donor ?? null, cases };
}

/** Read the expectations file: one case id per line, `#` comments allowed. */
export function loadExpectations(path = DEFAULT_EXPECTATIONS) {
  let text;
  try {
    text = readFileSync(path, "utf8");
  } catch (err) {
    throw new Error(`cannot read the expectations file ${path}: ${err.message}`);
  }
  const out = new Map();
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    // A comment may follow the id: `case-id  # why`.
    const [id, ...rest] = line.split(/\s+/);
    if (!id) continue;
    out.set(id, rest.join(" ").trim());
  }
  return out;
}

/** The strict switch, read from the env the render stack names. */
export function strictRequested(env = process.env) {
  return String(env[STRICT_ENV] ?? "").trim() === "1";
}

/** The runner's receipt line, parsed into its fields. */
export function parseCaseLine(line) {
  if (!line.includes(CASE_LINE)) return null;
  const fields = {};
  const body = line.slice(line.indexOf(CASE_LINE) + CASE_LINE.length).trim();
  // `why` is the last field and may hold blanks, so it takes the rest of
  // the line. Every other field is one token.
  const whyAt = body.indexOf(" why=");
  const head = whyAt < 0 ? body : body.slice(0, whyAt);
  if (whyAt >= 0) fields.why = body.slice(whyAt + " why=".length).trim();
  for (const part of head.split(/\s+/)) {
    const at = part.indexOf("=");
    if (at < 0) continue;
    fields[part.slice(0, at)] = part.slice(at + 1);
  }
  if (!fields.verdict) return null;
  return fields;
}

/**
 * The state of one case, from the child's exit code and its receipt line.
 * A child that died without a line is a crash: the driver aborted, or the
 * process could not start.
 */
export function caseState(status, stdout) {
  const fields = parseCaseLine(stdout);
  if (!fields) return { state: CRASH, why: "the runner printed no receipt line" };
  if (fields.verdict === "error") {
    return { state: CRASH, why: fields.why ?? "the runner could not run the case" };
  }
  if (fields.strict !== "1") {
    return { state: CRASH, why: "the instance was not strict" };
  }
  if (status !== 0) {
    return { state: CRASH, why: `the runner exited ${status}` };
  }
  // The device's answer: it compiled the program, or it refused it.
  if (fields.verdict === COMPILED) return { state: COMPILED, why: "" };
  if (fields.verdict === REJECTED) return { state: REJECTED, why: fields.why ?? "" };
  return { state: CRASH, why: `the runner printed the unknown verdict ${fields.verdict}` };
}

/**
 * The verdict of one case against the suite's expectation.
 *
 * The suite says what the device must do: a case that must compile, and a
 * case that must be refused. A device that does the opposite is a
 * conformance failure, not a pass: the run failed to reject a bad program,
 * or it rejected a good one.
 *
 * `expect` is `pass` when the suite expects the program to compile, and
 * `fail` when it expects the device to refuse it.
 */
export function judgeCase(expect, verdict) {
  if (verdict.state === CRASH) return { state: CRASH, why: verdict.why };
  // The suite's `pass` means the program compiles; `fail` means it is refused.
  const wanted = expect === PASS ? COMPILED : REJECTED;
  if (verdict.state === wanted) return { state: PASS, why: "" };
  const what = wanted === COMPILED ? "compile" : "be refused";
  const got = verdict.state === COMPILED ? "compiled it" : "refused it";
  return {
    state: FAIL,
    why: `the suite expects the program to ${what}, the device ${got}`,
  };
}

/** Run one case in its own process. The WGSL goes in on stdin. */
export function runCase(runner, wgsl, env, args = ["--wgsl-stdin"]) {
  const child = spawnSync(runner, args, {
    input: wgsl,
    encoding: "utf8",
    env,
    timeout: 120000,
    windowsHide: true,
    // Windows cannot start a .cmd or .bat without a shell, so a wrapper
    // script needs one; the real runner is an .exe and runs without it.
    shell: process.platform === "win32" && /\.(cmd|bat)$/i.test(runner),
  });
  if (child.error) {
    return { state: CRASH, why: `the runner could not start: ${child.error.message}` };
  }
  return caseState(child.status ?? -1, child.stdout ?? "");
}

/** The count line the gate prints. */
export function summaryLine(counts) {
  return `webgpu_cts: pass=${counts.pass} fail=${counts.fail} crash=${counts.crash} expected=${counts.expected}`;
}

/**
 * The whole run: one child process per case, in case-file order.
 * `runner` is the binary; `env` carries the strict switch.
 */
export function runSlice(cases, expectations, runner, env, args = ["--wgsl-stdin"]) {
  const rows = [];
  const counts = { pass: 0, fail: 0, crash: 0, expected: 0 };
  for (const one of cases) {
    const got = runCase(runner, one.wgsl, env, args);
    const judged = judgeCase(one.expect, got);
    const isExpected = expectations.has(one.id);
    let state = judged.state;
    let why = judged.why;
    if (isExpected && state === FAIL) {
      // A known defect: reported as expected, never as a pass.
      state = EXPECTED;
    }
    if (isExpected && state === PASS) {
      // The backend now does the right thing: the expectation is stale and
      // the case is a pass. The stale line is named, so the file gets fixed.
      why = "the expectations file names this case but it passed: the line is stale";
    }
    counts[state] += 1;
    rows.push({ id: one.id, expect: one.expect, state, why, source: one.source ?? null });
  }
  return { rows, counts };
}

/** The `--list` output: the chosen cases and their expectations. */
export function listLines(cases, expectations) {
  const lines = [];
  for (const one of cases) {
    const note = expectations.get(one.id);
    const state = note ? EXPECTED : one.expect;
    lines.push(`${state} ${one.id}`);
  }
  lines.push(`${cases.length} case(s), ${expectations.size} expected`);
  return lines.join("\n");
}

function parseArgs(argv) {
  const out = { list: false, json: false, cases: DEFAULT_CASES, expectations: DEFAULT_EXPECTATIONS, runner: DEFAULT_RUNNER, runnerArgs: ["--wgsl-stdin"], help: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--list") out.list = true;
    else if (a === "--json") out.json = true;
    else if (a === "--help" || a === "-h") out.help = true;
    else if (a === "--cases") out.cases = argv[++i] ?? "";
    else if (a === "--expectations") out.expectations = argv[++i] ?? "";
    else if (a === "--runner") out.runner = argv[++i] ?? "";
    else if (a === "--runner-arg") out.runnerArgs = [argv[++i] ?? ""];
    else {
      console.error(`webgpu-cts: unknown argument ${a}\n\n${HELP}`);
      process.exit(2);
    }
  }
  return out;
}

function main(argv) {
  const args = parseArgs(argv);
  if (args.help) {
    console.log(HELP);
    return 0;
  }
  let cases;
  let expectations;
  try {
    const loaded = loadCases(args.cases);
    cases = loaded.cases;
    expectations = loadExpectations(args.expectations);
  } catch (err) {
    console.error(`webgpu-cts: ${err.message}`);
    return 2;
  }
  if (args.list) {
    console.log(listLines(cases, expectations));
    return 0;
  }
  if (!existsSync(args.runner)) {
    console.error(
      `webgpu-cts: the runner ${args.runner} is not there. Build it first:\n  cargo build -p engine-render --features window --bin webgpu_cts`,
    );
    return 2;
  }
  if (!strictRequested()) {
    // A run without the strict switch measures the wrong thing.
    console.error(
      `webgpu-cts: ${STRICT_ENV} is not 1, so the instance was not strict and the run proves nothing.\n  ${STRICT_ENV}=1 node tools/webgpu-cts.mjs`,
    );
    return 1;
  }
  const env = { ...process.env, [STRICT_ENV]: "1" };
  const { rows, counts } = runSlice(cases, expectations, args.runner, env, args.runnerArgs);
  if (args.json) {
    console.log(JSON.stringify({ schema: 1, counts, rows }, null, 1));
  } else {
    for (const row of rows) {
      const why = row.why ? ` (${row.why})` : "";
      console.log(`${row.state} ${row.id}${why}`);
    }
    console.log(summaryLine(counts));
  }
  // A failed case, a crash or a stale expectation line all fail the gate.
  const stale = rows.filter((row) => row.why.startsWith("the expectations file"));
  if (counts.fail || counts.crash || stale.length) return 1;
  return 0;
}

if (process.argv[1] && process.argv[1].endsWith("webgpu-cts.mjs")) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (err) {
    console.error(`webgpu-cts: crashed: ${String(err?.stack ?? err).slice(0, 600)}`);
    process.exitCode = 2;
  }
}

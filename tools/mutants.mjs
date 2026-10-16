#!/usr/bin/env node
// Mutation-test a patch: run `cargo mutants --in-diff <patch> -p <crate>` and
// fail closed when a mutant in the changed lines survives the tests.
// Usage: node tools/mutants.mjs --patch target/lanes/<id>.patch -p engine-core
//   [--root .lanes/<id>] [--timeout-min 10] [--max-missed N] [--jobs N]
//   [--output <dir>] [--report <file>] [-- <extra cargo-mutants args>]
//   (`--patch -` reads the diff from stdin)
//   node tools/mutants.mjs --from-out <mutants.out dir | outcomes.json> [--max-missed N]
//   (re-judges a finished run without cargo)
// --root is the tree holding the post-patch files (default: the lane
// .lanes/<id> named by the patch path, else "."): cargo-mutants maps the diff
// onto the files it finds there. Install once: cargo install --locked cargo-mutants.
// It builds a scratch copy of the tree, so leave CARGO_TARGET_DIR unset or point
// it at a throwaway directory.
// Output: one `path:line: <mutation>` line per MISSED mutant, then the counts,
// the mutation score and `RESULT PASS|FAIL` last. The same text is written
// beside the patch (`<name>.mutants.txt`, or --report) for the judge.
// Score = (caught + timeout) / (caught + timeout + missed); unviable mutants
// never compiled, so they cannot be tested. PASS needs missed <= --max-missed
// (default 0), a green unmutated baseline and a run inside --timeout-min.
// Exit: 0 PASS, 1 FAIL, 2 usage or tool error.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const DEFAULT_TIMEOUT_MIN = 10;
const KILL_GRACE_MS = 10_000;
const OUTCOME_STATUS = { CaughtMutant: "caught", MissedMutant: "missed", Unviable: "unviable", Timeout: "timeout" };
const TEXT_STATUS = { MISSED: "missed", CAUGHT: "caught", UNVIABLE: "unviable", TIMEOUT: "timeout" };
// cargo-mutants exit codes whose results are complete: all caught, some
// missed, some timed out. 4 is a failing unmutated baseline; the rest are errors.
const COMPLETE_EXIT = new Set([0, 2, 3]);

// `path:line:col: <mutation>` as cargo-mutants names a mutant.
const NAME = /^(.+?):(\d+):(\d+): (.*)$/;
// The `in 1s build + 0s test` timing tail of a progress line.
const TIMING = String.raw`(?: in \d[^ ]*(?: build)?(?: \+ \d[^ ]* test)?)?`;
const PROGRESS = new RegExp(`^(MISSED|CAUGHT|UNVIABLE|TIMEOUT)\\s+(.+?):(\\d+):(\\d+): (.*?)${TIMING}$`);
const BASELINE = new RegExp(`^(ok|FAILED|TIMEOUT)\\s+Unmutated baseline${TIMING}$`);

function mutantFrom(status, path, line, column, mutation) {
  return { status, path, line: Number(line), column: Number(column), mutation };
}

// Mutants of an `outcomes.json` (exported for tests). Unknown outcome names
// throw: a gate that cannot read a result must not call it green.
export function parseOutcomes(text) {
  const doc = typeof text === "string" ? JSON.parse(text) : text;
  if (!doc || !Array.isArray(doc.outcomes)) throw new Error("outcomes.json has no `outcomes` array");
  let baseline = null;
  const mutants = [];
  for (const outcome of doc.outcomes) {
    if (outcome.scenario === "Baseline") {
      baseline = outcome.summary === "Success" ? "ok" : "failed";
      continue;
    }
    const mutant = outcome.scenario?.Mutant;
    if (!mutant) throw new Error(`unknown scenario in outcomes.json: ${JSON.stringify(outcome.scenario)}`);
    const status = OUTCOME_STATUS[outcome.summary];
    if (!status) throw new Error(`unknown outcome ${JSON.stringify(outcome.summary)} for ${mutant.name ?? mutant.file}`);
    const named = NAME.exec(mutant.name ?? "");
    if (named) mutants.push(mutantFrom(status, named[1], named[2], named[3], named[4]));
    else {
      const at = mutant.span?.start ?? {};
      mutants.push(mutantFrom(status, mutant.file ?? "?", at.line ?? 0, at.column ?? 0, `${mutant.genre ?? "mutation"}: ${mutant.replacement ?? "?"}`));
    }
  }
  if (Number.isInteger(doc.total_mutants) && doc.total_mutants !== mutants.length) {
    throw new Error(`outcomes.json lists ${mutants.length} mutants but reports total_mutants ${doc.total_mutants}`);
  }
  return { baseline, mutants };
}

// Mutants of cargo-mutants' progress lines (exported for tests): the fallback
// when no outcomes.json exists, e.g. a run cut off by the budget.
export function parseProgress(text) {
  let baseline = null;
  const mutants = [];
  for (const raw of String(text).split(/\r?\n/)) {
    const line = raw.trim();
    const base = BASELINE.exec(line);
    if (base) { baseline = base[1] === "ok" ? "ok" : "failed"; continue; }
    const m = PROGRESS.exec(line);
    if (m) mutants.push(mutantFrom(TEXT_STATUS[m[1]], m[2], m[3], m[4], m[5]));
  }
  return { baseline, mutants };
}

// Counts, score and verdict (exported for tests): one pass over mutants.
export function summarize({ baseline, mutants }, { maxMissed = 0, incomplete = false } = {}) {
  let caught = 0, missed = 0, unviable = 0, timeout = 0;
  for (const m of mutants) {
    if (m.status === "caught") caught++;
    else if (m.status === "missed") missed++;
    else if (m.status === "unviable") unviable++;
    else if (m.status === "timeout") timeout++;
  }
  const detected = caught + timeout;
  const score = detected + missed === 0 ? null : (100 * detected) / (detected + missed);
  const ok = baseline !== "failed" && !incomplete && missed <= maxMissed;
  return { caught, missed, unviable, timeout, total: mutants.length, score, baseline, ok };
}

const order = (a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : a.line - b.line || a.column - b.column || (a.mutation < b.mutation ? -1 : a.mutation > b.mutation ? 1 : 0));

// The report: one line per missed mutant (timeouts as WARN), counts, score, RESULT.
export function formatReport(parsed, { maxMissed = 0, label = "", note = "", incomplete = false } = {}) {
  const s = summarize(parsed, { maxMissed, incomplete });
  const lines = [];
  const missed = [], timed = [];
  for (const m of parsed.mutants) {
    if (m.status === "missed") missed.push(m);
    else if (m.status === "timeout") timed.push(m);
  }
  for (const m of missed.sort(order)) lines.push(`${m.path}:${m.line}: ${m.mutation}`);
  for (const m of timed.sort(order)) lines.push(`WARN timeout ${m.path}:${m.line}: ${m.mutation}`);
  const score = s.score === null ? "n/a" : `${s.score.toFixed(1)}%`;
  lines.push(`mutants: caught=${s.caught} missed=${s.missed} unviable=${s.unviable} timeout=${s.timeout} total=${s.total} score=${score}`);
  const why = s.baseline === "failed" ? "unmutated baseline failed" : incomplete ? "run incomplete" : `${s.missed} missed (max ${maxMissed})`;
  const nothing = s.total === 0 && !incomplete ? ", no mutants in the diff" : "";
  lines.push(`RESULT ${s.ok ? "PASS" : "FAIL"}: ${why}, score ${score}${nothing}${label ? ` (${label})` : ""}${note ? `; ${note}` : ""}`);
  return { ...s, lines };
}

// Post-image files of a git patch with their added line counts (exported for tests).
export function patchFiles(patch) {
  const files = [];
  for (const section of String(patch).split(/(?=^diff --git )/m).filter((s) => s.startsWith("diff --git "))) {
    const next = /^\+\+\+ (.+)$/m.exec(section)?.[1];
    if (!next) continue;
    if (next === "/dev/null") { files.push({ path: /^--- (?:a\/)?(.+)$/m.exec(section)?.[1] ?? "", deleted: true, added: 0 }); continue; }
    const path = next.replace(/^"?b\//, "").replace(/"$/, "");
    const added = section.split(/\r?\n/).filter((l) => l.startsWith("+") && !l.startsWith("+++")).length;
    files.push({ path, deleted: false, added });
  }
  return files;
}

// Files holding `outcomes.json` for a `mutants.out` directory, its parent or the file itself.
export function findOutcomes(path) {
  for (const candidate of [path, join(path, "outcomes.json"), join(path, "mutants.out", "outcomes.json")]) {
    if (candidate.endsWith(".json") && existsSync(candidate)) return candidate;
  }
  return null;
}

export function parseArgs(argv) {
  const opts = { patch: "", packages: [], root: "", timeoutMin: DEFAULT_TIMEOUT_MIN, maxMissed: 0, jobs: 0, output: "", report: "", fromOut: "", extra: [] };
  const number = (name, value, min, integer) => {
    const n = Number(value);
    if (value === undefined || value === "" || !Number.isFinite(n) || n < min || (integer && !Number.isInteger(n))) throw new Error(`${name} needs ${integer ? "an integer" : "a number"} >= ${min}, got ${JSON.stringify(value)}`);
    return n;
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = () => {
      if (argv[i + 1] === undefined) throw new Error(`${arg} needs a value`);
      return argv[++i];
    };
    if (arg === "--") { opts.extra = argv.slice(i + 1); break; }
    else if (arg === "--patch") opts.patch = value();
    else if (arg === "-p" || arg === "--package") opts.packages.push(value());
    else if (arg === "--root") opts.root = value();
    else if (arg === "--timeout-min") opts.timeoutMin = number(arg, value(), 0.01, false);
    else if (arg === "--max-missed") opts.maxMissed = number(arg, value(), 0, true);
    else if (arg === "--jobs") opts.jobs = number(arg, value(), 1, true);
    else if (arg === "--output") opts.output = value();
    else if (arg === "--report") opts.report = value();
    else if (arg === "--from-out") opts.fromOut = value();
    else throw new Error(`unknown argument: ${arg}`);
  }
  if (opts.fromOut) return opts;
  if (!opts.patch) throw new Error("--patch is required (a file, or - for stdin)");
  if (!opts.packages.length) throw new Error("-p <crate> is required (e.g. -p engine-core)");
  return opts;
}

// A collected lane patch target/lanes/<id>.patch reads its files from .lanes/<id>.
function defaultRoot(patch) {
  const lane = /(?:^|[\\/])target[\\/]lanes[\\/]([^\\/]+)\.patch$/.exec(patch)?.[1];
  return lane && existsSync(join(".lanes", lane)) ? join(".lanes", lane) : ".";
}

// Stop a process tree: cargo-mutants spawns cargo, which spawns test binaries.
function killTree(child) {
  if (child.pid === undefined) return;
  if (process.platform === "win32") { spawnSync("taskkill", ["/pid", String(child.pid), "/T", "/F"]); return; }
  try { process.kill(-child.pid, "SIGTERM"); } catch { /* already gone */ }
  setTimeout(() => { try { process.kill(-child.pid, "SIGKILL"); } catch { /* already gone */ } }, KILL_GRACE_MS).unref();
}

// Run cargo with a wall-clock budget; progress streams to stderr.
export function runCargo(args, { cwd, timeoutMs, env = process.env, echo = true }) {
  return new Promise((resolveRun) => {
    const child = spawn("cargo", args, { cwd, env, detached: process.platform !== "win32", stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    let settled = false;
    // The child leads its own process group, so a terminal interrupt would miss it.
    const stop = () => killTree(child);
    const timer = setTimeout(() => { timedOut = true; killTree(child); }, timeoutMs);
    process.once("SIGINT", stop);
    process.once("SIGTERM", stop);
    const finish = (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      process.off("SIGINT", stop);
      process.off("SIGTERM", stop);
      resolveRun({ stdout, stderr, timedOut, ...result });
    };
    child.stdout.on("data", (d) => { stdout += d; if (echo) process.stderr.write(d); });
    child.stderr.on("data", (d) => { stderr += d; if (echo) process.stderr.write(d); });
    child.on("error", (error) => finish({ status: null, error }));
    child.on("close", (status, signal) => finish({ status, signal }));
  });
}

function tail(text, lines = 8) {
  return String(text).split("\n").filter(Boolean).slice(-lines).join(" | ").slice(0, 1500);
}

// Run (or re-read) the mutation test and return the finished report. `deps`
// lets tests replace cargo.
export async function judge(opts, deps = {}) {
  const run = deps.runCargo ?? runCargo;
  const label = opts.fromOut ? `from ${opts.fromOut}` : `${opts.packages.join(",")} ${opts.patch === "-" ? "stdin patch" : opts.patch}`;
  const format = (parsed, note = "", incomplete = false) => formatReport(parsed, { maxMissed: opts.maxMissed, label, note, incomplete });
  if (opts.fromOut) {
    const file = findOutcomes(opts.fromOut);
    if (!file) throw new Error(`no outcomes.json under ${opts.fromOut}`);
    return format(parseOutcomes(readFileSync(file, "utf8")));
  }
  const root = resolve(opts.root || defaultRoot(opts.patch));
  if (!existsSync(root)) throw new Error(`--root ${root} does not exist`);
  const patch = readFileSync(opts.patch === "-" ? 0 : opts.patch, "utf8");
  const files = patchFiles(patch);
  if (!files.length) throw new Error("no usable git patch (expected `git diff` output)");
  // A patch that names files the root lacks maps to no mutants and would pass
  // vacuously: refuse it instead.
  for (const file of files.filter((f) => !f.deleted)) {
    if (!existsSync(join(root, file.path))) throw new Error(`patch names ${file.path}, missing under root ${root}: pass --root <tree holding the post-patch files>`);
  }
  if (!files.some((f) => !f.deleted && /\.rs$/.test(f.path) && f.added > 0)) {
    return format({ baseline: "ok", mutants: [] }, "patch adds no Rust lines, nothing to mutate");
  }
  const version = deps.version ? deps.version() : spawnSync("cargo", ["mutants", "--version"], { encoding: "utf8" });
  if (version.error || version.status !== 0) throw new Error("cargo-mutants is not installed: cargo install --locked cargo-mutants");

  const output = resolve(opts.output || join(root, "target", "mutants"));
  // A stale mutants.out from an earlier run must never be read as this run's.
  for (const stale of ["mutants.out", "mutants.out.old"]) rmSync(join(output, stale), { recursive: true, force: true });
  mkdirSync(output, { recursive: true });
  let patchFile = opts.patch === "-" ? "" : resolve(opts.patch);
  if (opts.patch === "-") {
    patchFile = join(output, "stdin.patch");
    writeFileSync(patchFile, patch);
  }
  const args = ["mutants", "--in-diff", patchFile, ...opts.packages.flatMap((p) => ["--package", p]), "--output", output, "--colors", "never"];
  if (opts.jobs) args.push("--jobs", String(opts.jobs));
  args.push(...opts.extra);
  const result = await run(args, { cwd: root, timeoutMs: Math.round(opts.timeoutMin * 60_000) });
  if (result.error) throw new Error(`could not run cargo: ${result.error.message}`);

  const outcomes = findOutcomes(output);
  const partial = () => parseProgress(`${result.stdout}\n${result.stderr}`);
  if (result.timedOut) {
    const parsed = partial();
    return format(parsed, `budget of ${opts.timeoutMin} min exceeded after ${parsed.mutants.length} mutants tested`, true);
  }
  if (result.status === 4) {
    return format({ baseline: "failed", mutants: [] }, "run the crate's tests on the unmutated tree first");
  }
  if (!COMPLETE_EXIT.has(result.status)) {
    throw new Error(`cargo mutants exited ${result.status}: ${tail(result.stderr || result.stdout)}`);
  }
  if (outcomes) return format(parseOutcomes(readFileSync(outcomes, "utf8")));
  // Exit 0 without outcomes.json: cargo-mutants found nothing to mutate.
  const parsed = partial();
  if (!parsed.mutants.length && /No mutants (?:found|to filter)/i.test(`${result.stdout}${result.stderr}`)) return format({ baseline: "ok", mutants: [] });
  if (!parsed.mutants.length) throw new Error(`cargo mutants exited ${result.status} with no outcomes.json and no progress lines: ${tail(result.stderr || result.stdout)}`);
  return format(parsed);
}

async function cli(argv) {
  const opts = parseArgs(argv);
  const report = await judge(opts);
  for (const line of report.lines) console.log(line);
  const target = opts.report || (opts.patch && opts.patch !== "-" && !opts.fromOut ? opts.patch.replace(/(?:\.patch)?$/, ".mutants.txt") : "");
  if (target) {
    mkdirSync(dirname(resolve(target)), { recursive: true });
    writeFileSync(target, `${report.lines.join("\n")}\n`);
  }
  if (!report.ok) process.exitCode = 1;
}

if (import.meta.main) {
  cli(process.argv.slice(2)).catch((error) => { console.error(`RESULT FAIL: ${error.message}`); process.exitCode = 2; });
}

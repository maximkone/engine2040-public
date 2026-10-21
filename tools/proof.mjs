// Local gate runner, from the repository root: workspace audit, fmt, build,
// default and all-features tests and lint, cargo-deny when installed, then the
// real seed/input-tape proofs. --skip-build runs only the proof fixtures (CI
// runs the other steps itself). --perf runs only the timing tests, which are
// #[ignore]d in the default suite because parallel load makes them flaky, and
// on Linux the instruction-count budget. --mutants mutation-tests this tree's
// crate changes against HEAD (tools/mutants.mjs). --help prints usage plus
// nextest, gungraun and fuzz examples and exits 0. A GREEN run writes
// target/proof/<gate>-latest.json; --drift FAILs a record that is 25 h old or
// whose sha is not HEAD or an ancestor.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { lowerPriority, poolEnv } from "./jobserver.mjs";

// Every cargo this file starts: joins the shared slot pool when it is up, and runs below the desktop's priority (tools/jobserver.mjs).
const cargoEnv = () => { lowerPriority(); return { ...process.env, ...poolEnv() }; };
// Timed runs keep normal priority (below-normal would skew their clocks); they still join the pool.
const timedEnv = () => ({ ...process.env, ...poolEnv() });

// Pure CLI arg validation (exported for tests): throws the usage line on
// anything but [] or one of the flags, mirroring the old exit(2).
export function parseProofArgs(argv) {
  if (argv.length > 1 || argv.some((arg) => !["--skip-build", "--perf", "--mutants", "--drift"].includes(arg))) {
    throw new Error("Usage: node tools/proof.mjs [--skip-build | --perf | --mutants | --drift]\nTry 'node tools/proof.mjs --help' for the gate map and copy-paste examples.");
  }
  return argv;
}

// Help text for `--help` (docs only; no gate logic lives here). Examples are
// the exact commands the gates run, so copy-paste stays true.
export const PROOF_HELP = [
  "Usage: node tools/proof.mjs [--skip-build | --perf | --mutants | --drift]",
  "       node tools/proof.mjs --help | -h",
  "",
  "Full gate (default): workspace audit, fmt/build/test/clippy/deny, proof fixtures.",
  "  node tools/proof.mjs",
  "Fixture-only (CI runs the other steps itself):",
  "  node tools/proof.mjs --skip-build",
  "Timing + instruction gate (release, one thread; Linux adds Callgrind):",
  "  node tools/proof.mjs --perf",
  "Mutation gate for this tree's crate changes against HEAD:",
  "  node tools/proof.mjs --mutants",
  "Drift gate over the green records a GREEN run writes (target/proof/<gate>-latest.json):",
  "  node tools/proof.mjs --drift",
  "",
  "Examples:",
  "  nextest (proof profile; opt-in when `cargo nextest` and",
  "  .config/nextest.toml [profile.proof] both exist, else plain cargo test):",
  "    cargo nextest run --profile proof --cargo-profile fast-test --locked --workspace --all-targets",
  "  receipt: target/nextest-junit/proof.xml; retries print as `<n> retried` per leg.",
  "  partitioned (same suite split, no test dropped; merge proof-1-2.xml + proof-2-2.xml to proof.xml):",
  "    cargo nextest run --profile proof --partition count:1/2 --cargo-profile fast-test --locked --workspace --all-targets",
  "    cargo nextest run --profile proof --partition count:2/2 --cargo-profile fast-test --locked --workspace --all-targets",
  "  gungraun (Linux only, 2% Ir budget over the perf_gate baseline):",
  "    cargo install --locked --version 0.20.0 gungraun-runner",
  "    cargo bench --locked -p engine-perf --bench instructions -- --save-baseline=perf_gate",
  "    cargo bench --locked -p engine-perf --bench instructions -- --baseline=perf_gate",
  "  fuzz (deterministic arena corpus, engine-proof `fuzz` feature):",
  "    cargo test --locked -p engine-proof --all-features --lib a9",
  "    cargo test --locked -p engine-proof --all-features --lib fuzz_corpus_runs_clean_and_reproduces",
].join("\n");

// Benches in crates/perf/benches/instructions.rs: arena, physics, ECS spine.
export const IR_BENCHES = 3;
const IR_SETUP = "the instruction gate needs valgrind, `cargo install --locked --version 0.20.0 gungraun-runner` and a baseline saved on master: `cargo bench --locked -p engine-perf --bench instructions -- --save-baseline=perf_gate`";

// E12 quarantine list (docs/research/speed-2026-10-06-E12-gates.md).
// Default gate never runs these. They run in --perf, nightly, soak, or by
// hand. A fail here never blocks a logic packet. Each entry names where it
// runs and why it stays out of the default gate.
export const E12_QUARANTINE = [
  { name: '#[ignore = "timing: ..."]', runner: "--perf", reason: "wall clock flakes under load; needs release, one thread, quiet box" },
  { name: "game netgame UDP loopback", runner: "--perf", reason: "passes alone, flakes to empty under full load" },
  { name: "render GPU submit and window execution", runner: "--perf", reason: "compile is must-pass, run on GPU is not; needs reference GPU at boost clocks" },
  { name: "PQ1 input latency (engine-perf latency)", runner: "--perf", reason: "needs release clocks and headless report lines" },
  { name: "instruction counts (gungraun, Callgrind)", runner: "--perf on Linux", reason: "Linux only; needs valgrind, saved perf_gate baseline, 2% budget" },
  { name: "gpu_live truth gate", runner: "report-only", reason: "report-only until GX2-GX5 and GX7-GX10 land; logs, never fails" },
  { name: "--mutants gate", runner: "--mutants", reason: "slow; fails closed per crate; changed-crate or nightly, not per packet" },
  { name: "live provider tests", runner: "by hand", reason: "needs tokens or servers; secrets never gate a local run" },
  { name: "provider transport TLS stub", runner: "by hand", reason: "needs a test CA; no minter in the graph" },
  { name: "rules tests/data writer", runner: "by hand", reason: "writes gold once by hand on purpose; not a per-run check" },
  { name: "rules budget probe and strict timing", runner: "by hand", reason: "tuning charts for a quiet PC; not a gate" },
  { name: "intake_verb real props (E2040_MAKE_RUN)", runner: "manual", reason: "needs external Kaggle output" },
  { name: "hour-long online soak and 10 min siege p99", runner: "soak lane", reason: "too long for a packet gate" },
  { name: "game practice 1s shader-edit visibility", runner: "by hand", reason: "wall clock demo; run once by hand" },
];

// Pure verdict over `cargo bench --bench instructions -- --baseline=perf_gate`
// (exported for tests). Green needs exit 0 and every bench reporting a nonzero
// count against a stored baseline: a zero count (Callgrind measured nothing)
// or `N/A` (no baseline) is red, never a silent pass.
export function irGateVerdict(run) {
  const out = `${run.stdout ?? ""}${run.stderr ?? ""}`;
  const rows = [...out.matchAll(/^\s*Instructions:\s+(\d+)\|(\d+|N\/A)/gm)].map(([, now, was]) => ({ now: Number(now), was }));
  let why = "";
  if (run.error) why = run.error.message;
  else if (/exceeds limit/.test(out)) why = `Ir over the budget: ${/Instructions \((\d+ -> \d+)\): (\S+)/.exec(out)?.slice(1).join(", ") ?? "see the bench output"}`;
  else if (run.status !== 0) why = `cargo bench exited ${run.status}`;
  else if (rows.length < IR_BENCHES) why = `${rows.length} of ${IR_BENCHES} benches reported a count`;
  else if (rows.some((r) => r.now === 0)) why = "a bench counted 0 instructions";
  else if (rows.some((r) => r.was === "N/A")) why = "no perf_gate baseline to compare against";
  const ok = why === "";
  return { ok, rows, line: ok ? `[PASS] instruction counts inside the 2% Ir budget (${rows.map((r) => r.now).join(", ")})` : `[FAIL] instruction counts: ${why}` };
}

// Pure verdicts over the `spine_parity_line` bin (ARC10, exported for tests).
// The scorecard column is one line: `spine parity: seeds=20 ticks=600 match=20
// mismatch=0 first=none`. The real run is green only on exit 0 with every seed
// matched; the canary run (`--seeded-wall`, a wall only the spine side knows)
// is green only when the gate FAILS and names the first divergence, so a gate
// that cannot fail is red too.
export function parseParityLine(text) {
  const m = /^spine parity: seeds=(\d+) ticks=(\d+) match=(\d+) mismatch=(\d+) first=(.+)$/m.exec(`${text ?? ""}`);
  return m ? { seeds: Number(m[1]), ticks: Number(m[2]), matched: Number(m[3]), mismatch: Number(m[4]), first: m[5], line: m[0] } : null;
}

export function parityVerdict(run, { canary = false } = {}) {
  const col = parseParityLine(run.stdout);
  const lastStderr = `${run.stderr ?? ""}`.trim().split("\n").pop();
  if (run.error || !col) return { ok: false, line: `no parity line: ${run.error?.message ?? (lastStderr || "empty output")}` };
  const clean = run.status === 0 && col.matched === col.seeds && col.mismatch === 0 && col.first === "none";
  if (!canary) return { ok: clean, line: col.line };
  const caught = run.status !== 0 && col.mismatch >= 1 && col.first !== "none";
  return { ok: caught, line: caught ? `canary caught: ${col.line}` : `canary NOT caught: ${col.line}` };
}

// Pure verdict over the frame-time CSV text (exported for tests). Only the
// wall_ms column is required, looked up by header name so order and extra
// columns do not matter. Blank lines are skipped, CRLF is handled.
export const FRAME_P95_BAR_MS = 50;
export const FRAME_CSV_PATH = "target/perf/frame-times.csv";

export function frameCsvVerdict(csvText, barMs = FRAME_P95_BAR_MS) {
  const text = `${csvText ?? ""}`;
  const lines = text.split(/\r?\n/);
  let headerIdx = -1;
  for (let i = 0; i < lines.length; i += 1) {
    if (lines[i].trim() !== "") {
      headerIdx = i;
      break;
    }
  }
  if (headerIdx < 0) {
    return { ok: false, frames: 0, p50: null, p95: null, line: "[FAIL] frame times: empty csv" };
  }
  const header = lines[headerIdx].split(",").map((c) => c.trim());
  const wallIdx = header.indexOf("wall_ms");
  if (wallIdx < 0) {
    return { ok: false, frames: 0, p50: null, p95: null, line: "[FAIL] frame times: missing wall_ms column" };
  }
  const values = [];
  for (let i = headerIdx + 1; i < lines.length; i += 1) {
    const raw = lines[i];
    if (raw.trim() === "") {
      continue;
    }
    const cols = raw.split(",");
    const cell = (cols[wallIdx] ?? "").trim();
    const n = Number(cell);
    if (cell === "" || !Number.isFinite(n) || n < 0) {
      return { ok: false, frames: values.length, p50: null, p95: null, line: `[FAIL] frame times: bad wall_ms value on row ${values.length + 1}` };
    }
    values.push(n);
  }
  if (values.length === 0) {
    return { ok: false, frames: 0, p50: null, p95: null, line: "[FAIL] frame times: zero data rows" };
  }
  values.sort((a, b) => a - b);
  const n = values.length;
  const p50 = values[Math.ceil(0.5 * n) - 1];
  const p95 = values[Math.ceil(0.95 * n) - 1];
  const ok = p95 <= barMs;
  const tag = ok ? "PASS" : "FAIL";
  return { ok, frames: n, p50, p95, line: `[${tag}] frame times: frames=${n} p50=${p50.toFixed(3)} ms p95=${p95.toFixed(3)} ms (bar ${barMs} ms)` };
}

// Crate directories whose `src/` a git diff touches (exported for tests).
export function changedCrateDirs(diff) {
  return [...new Set([...`${diff}`.matchAll(/^\+\+\+ b\/crates\/([^/\s]+)\/src\//gm)].map((m) => m[1]))].sort();
}

// The `[package]` name of a Cargo.toml (exported for tests); bin, bench and
// lib sections carry names of their own.
export function packageNameOf(manifest) {
  let inPackage = false;
  for (const line of `${manifest}`.split(/\r?\n/)) {
    const header = /^\s*\[\[?([^\]]+)\]\]?\s*$/.exec(line);
    if (header) {
      inPackage = header[1].trim() === "package";
      continue;
    }
    const name = inPackage ? /^\s*name\s*=\s*"([^"]+)"/.exec(line) : null;
    if (name) return name[1];
  }
  return "";
}

// Pure summary of a `node --test` run's stdout (exported for tests): the
// "# pass / # fail" lines the proof gate reports.
export function toolTestSummary(output) {
  return `${output ?? ""}`.split("\n").filter((l) => /^# (pass|fail)/.test(l)).join(", ");
}

// Snapshot pending gate (CQ16, insta discipline without the dependency): the
// std-only Rust helper in tests/common/mod.rs compares text with `<name>.snap`,
// writes `<name>.snap.new` on a mismatch and fails with that path. A tree that
// still holds a pending file holds an unreviewed change, so this check fails
// with every pending path named. Accepting is a review: rerun the failing test
// with E2040_ACCEPT_SNAPSHOTS=1, which rewrites the `.snap` file in the same
// commit.
export const PENDING_SNAPSHOT_SUFFIX = ".snap.new";
const SNAPSHOT_SCAN_SKIP = new Set([".git", ".venv", "node_modules", "target"]);

export function pendingSnapshotVerdict(paths) {
  const pending = [...new Set(paths ?? [])].filter((p) => `${p}`.endsWith(PENDING_SNAPSHOT_SUFFIX)).sort();
  const ok = pending.length === 0;
  return {
    ok,
    pending,
    line: ok
      ? `no pending ${PENDING_SNAPSHOT_SUFFIX} in the tree`
      : `${pending.length} pending ${PENDING_SNAPSHOT_SUFFIX} (review the diff, rerun the test with E2040_ACCEPT_SNAPSHOTS=1): ${pending.join(", ")}`,
  };
}

// Every `<name>.snap.new` under root, as repo-relative slash paths (exported
// for tests). Build and cache directories are skipped; nothing is read.
export function findPendingSnapshots(root = ".", skip = SNAPSHOT_SCAN_SKIP) {
  const found = [];
  const walk = (dir, prefix) => {
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.name.endsWith(PENDING_SNAPSHOT_SUFFIX)) {
        found.push(rel);
        continue;
      }
      if (!entry.isDirectory() || skip.has(entry.name)) continue;
      walk(join(dir, entry.name), rel);
    }
  };
  walk(root, "");
  return found.sort();
}

// Nextest profile discipline (STEAL-speed OPT6, donor nextest-rs/nextest
// @d643f89 .config/nextest.toml, MIT, reimplemented, no donor bytes).
// `.config/nextest.toml` holds `[profile.proof]` (fail-fast=false,
// 60s slow-timeout with terminate-after, bounded fixed-backoff retries
// count 2 delay 2s, junit receipt); the proof test legs run under it per the
// `.cargo/config.toml` fast-test comment, with retries reported never hidden.
// Partitioned run splits the suite without dropping a test:
// `cargo nextest run --profile proof --partition count:1/2` then `count:2/2`,
// junit `target/nextest-junit/proof-1-2.xml` plus `proof-2-2.xml` merged to
// `target/nextest-junit/proof.xml`; persistent flakes (>=2) quarantine via
// tools/flaky-quarantine.mjs threshold 2 to `--perf`, never hidden.
export const NEXTEST_PROFILE = "proof";
export const NEXTEST_CONFIG = ".config/nextest.toml";
export const NEXTEST_JUNIT = "target/nextest-junit/proof.xml";

// Pure junit path for a leg (exported for tests): unpartitioned legs post
// `target/nextest-junit/proof.xml`; partitioned legs post
// `target/nextest-junit/proof-M-N.xml` merged to proof.xml, so every leg
// posts one junit artifact beside the merged receipt.
export function nextestJunitPath(partition = null) {
  if (partition === null || partition === undefined || partition === "") return NEXTEST_JUNIT;
  const raw = String(partition).startsWith("count:") ? String(partition).slice(6) : String(partition);
  const [m, n] = raw.split("/");
  return `target/nextest-junit/proof-${m}-${n}.xml`;
}

// Pure check over the profile file text (exported for tests): true when the
// proof profile the legs run under is present.
export function hasNextestProfile(tomlText) {
  return /^\s*\[profile\.proof\]/m.test(`${tomlText ?? ""}`);
}

// Pure map from a `cargo test ...` leg to its `cargo nextest run ...`
// equivalent (exported for tests): non-test legs map to null, `-j4`/`-j N`
// are dropped (nextest owns threads via the profile), and the cargo
// fast-test profile rides `--cargo-profile` per `.cargo/config.toml`.
// Default legs run unpartitioned; pass partition `M/N` (or `count:M/N`) for
// the partitioned run `cargo nextest run --profile proof --partition
// count:M/N`, which splits the suite without dropping a test.
export function nextestTestCommand(cargoArgs, partition = null) {
  const args = [...(cargoArgs ?? [])];
  if (args[0] !== "test") return null;
  const rest = [];
  for (let i = 1; i < args.length; i += 1) {
    if (args[i] === "-j4") continue;
    if (args[i] === "-j" || args[i] === "--jobs") {
      i += 1;
      continue;
    }
    rest.push(args[i]);
  }
  const base = ["nextest", "run", "--profile", NEXTEST_PROFILE];
  if (partition !== null && partition !== undefined && partition !== "") {
    const raw = String(partition);
    const spec = raw.startsWith("count:") ? raw : `count:${raw}`;
    if (!/^count:\d+\/\d+$/.test(spec)) throw new Error(`bad partition ${partition}: want M/N like 1/2`);
    const [m, n] = spec.slice(6).split("/").map(Number);
    if (!(m >= 1 && n >= 1 && m <= n)) throw new Error(`bad partition ${partition}: want 1 <= M <= N`);
    base.push("--partition", spec);
  }
  return [...base, "--cargo-profile", "fast-test", ...rest];
}

// Pure flake count over a finished leg's combined output (exported for
// tests): nextest prints `retry`/`retried`/`flaky` lines per retried test,
// so the proof log prints `<n> retried` beside every leg verdict and the
// OPT6 before/after counts stay visible. Zero retries prints `0 retried`,
// never an empty string that could hide a flake-free run.
export function nextestFlakeSummary(output) {
  const lines = `${output ?? ""}`.split("\n");
  let n = 0;
  for (const line of lines) {
    if (/\bretries?\b|\bretried\b|\bflaky\b/i.test(line)) {
      const m = /(\d+)\s+retr(?:y|ied|ies)/i.exec(line);
      n += m ? Number(m[1]) : 1;
    }
  }
  return `${n} retried`;
}

// Pure check over a finished `cargo test` leg's combined output (exported
// for tests): true only when at least one test ran. A zero-match filter
// prints `test result: ok. 0 passed; ...` and exits 0, so grading on exit
// code alone passes vacuously; the proof gate fails closed on zero tests-run.
export function ranAtLeastOneTest(output) {
  const text = `${output ?? ""}`;
  const results = [...text.matchAll(/test result:\s*(?:ok|FAILED)\.\s*(\d+)\s+passed;\s*(\d+)\s+failed/gi)];
  if (results.length > 0) {
    let ran = 0;
    for (const [, passed, failed] of results) {
      ran += Number(passed) + Number(failed);
    }
    return ran > 0;
  }
  // Nextest summary fallback: an explicit nonzero `N passed` means tests ran.
  // Absence of any passed count means zero ran.
  const nextest = [...text.matchAll(/(\d+)\s+passed/gi)];
  return nextest.some(([, n]) => Number(n) > 0);
}

// One finished leg's verdict (exported for tests): a compile or exit failure is
// red, and so is a test leg that ran zero tests, because `cargo test` exits 0
// when its filter (or an --exclude set) matches nothing and a hollow green
// proves nothing. Non-test legs (fmt, build, clippy, deny) grade on exit code.
export function legVerdict(isTest, ok, output) {
  if (!ok) return { pass: false, why: null };
  if (isTest && !ranAtLeastOneTest(output)) {
    return { pass: false, why: "zero tests ran (filter matched nothing)" };
  }
  return { pass: true, why: null };
}

// Per-packet fixture-only discipline: the golden receipt verifies once; the
// seed double-runs below still prove determinism, so a second 600-tick verify
// is duplicate work. Tool self-tests run in CI nightly only
// (`node --test tools/*.test.mjs` in .github/workflows/ci.yml), never in the
// proof gate: 155 files reran in every full gate with no signal the cargo
// legs do not already give, so per-packet proof stays fixtures plus ledgers.
export const GOLDEN_VERIFY_RUNS = 1;
export function runsToolTests(argv, env = process.env) {
  return env.E2040_NIGHTLY === "1" && !argv.includes("--skip-build");
}

// Single-receipt golden verdict (exported for tests): the hash law plus the
// version pin, so a stale-version receipt with the right hash still FAILs.
export function goldenVerdict(receipt, workspaceVersion) {
  return (
    receipt?.final_hash === "d57b5c8ae3096614" &&
    !!receipt?.golden_checked &&
    !!receipt?.verified &&
    workspaceVersion !== "" &&
    receipt?.engine_version === workspaceVersion
  );
}

// API2 handbook build: `mdbook build docs/book` is green only on exit 0 with no
// WARN or ERROR line (a missing chapter file or a broken include prints one).
export function bookBuildVerdict(run) {
  if (run.error) return { ok: false, line: `mdbook did not run: ${run.error.message}` };
  const bad = `${run.stdout ?? ""}${run.stderr ?? ""}`.split(/\r?\n/).find((l) => /\b(?:WARN|ERROR)\b/.test(l));
  if (run.status !== 0) return { ok: false, line: `mdbook build exited ${run.status}${bad ? `: ${bad.trim()}` : ""}` };
  return bad ? { ok: false, line: `mdbook build warned: ${bad.trim()}` } : { ok: true, line: "docs/book builds clean" };
}

// AD4 green record: a gate that ends GREEN writes target/proof/<gate>-latest.json
// ({ gate, sha, time }); a red run never touches it. `--drift` reads the
// records back and FAILs one that is GREEN_MAX_AGE_HOURS old or whose sha is
// neither HEAD nor an ancestor, so an old green cannot pass for a fresh one.
export const GREEN_MAX_AGE_HOURS = 25;
export const GREEN_DIR = "target/proof";
const GREEN_SUFFIX = "-latest.json";

// Writes the record when the gate was green and returns its path; a red gate
// returns null and leaves the previous record as it was.
export function recordGreen(gate, { failed, sha, now = new Date(), dir = GREEN_DIR }) {
  if (failed) return null;
  mkdirSync(dir, { recursive: true });
  const path = join(dir, `${gate}${GREEN_SUFFIX}`);
  writeFileSync(path, `${JSON.stringify({ gate, sha, time: now.toISOString() })}\n`);
  return path;
}

// `record` is the parsed record or null (missing, unreadable). `isAncestor`
// says whether record.sha is an ancestor of `head`.
export function driftVerdict(gate, record, { head, isAncestor, now = new Date() }) {
  const fail = (why) => ({ ok: false, line: `[FAIL] drift ${gate}: ${why}` });
  if (!record) return fail(`no green record in ${GREEN_DIR}/${gate}${GREEN_SUFFIX}`);
  const hours = (now.getTime() - Date.parse(record.time)) / 3_600_000;
  if (!Number.isFinite(hours) || hours < 0) return fail(`record time ${record.time} is not a past time`);
  if (hours >= GREEN_MAX_AGE_HOURS) return fail(`green record is ${hours.toFixed(1)} h old (limit ${GREEN_MAX_AGE_HOURS} h), run the gate again`);
  if (record.sha !== head && !isAncestor) return fail(`green sha ${String(record.sha).slice(0, 7)} is neither HEAD nor an ancestor of it`);
  return { ok: true, line: `[PASS] drift ${gate}: green ${String(record.sha).slice(0, 7)} ${hours.toFixed(1)} h ago` };
}

// Device evidence for the gpu-live gate: the render window's live-frame leg
// writes target/reports/pass-truth-<date>.md with a `source` column; only rows
// read from the `live` frame count. The fixture flags of engine-proof prove
// nothing about a device, so without such rows the gate prints `not_wired`.
export function gpuLiveReceipt(reportText) {
  let live = 0;
  let probe = 0;
  for (const line of `${reportText ?? ""}`.split(/\r?\n/)) {
    const cols = line.split("|").map((c) => c.trim());
    if (cols[4] !== "live") continue;
    if (cols[2] === "LIVE") live += 1;
    else if (cols[2] === "PROBE") probe += 1;
  }
  return live + probe > 0 ? { live, probe } : null;
}

export function gpuLiveLine(receipt, file, unitGate) {
  return receipt
    ? `[INFO] gpu-live truth gate (report-only): LIVE ${receipt.live}/PROBE ${receipt.probe} from the device report ${file}; unit gate: ${unitGate}`
    : `[INFO] gpu-live truth gate (report-only): not_wired, no device report with live rows (target/reports/pass-truth-<date>.md), so no pass counts as LIVE; unit gate: ${unitGate}`;
}

// OPT38 made the local golden build on a thin-LTO profile of its own, because
// `release` used to be fat LTO with one codegen unit. OPT45 turned `release`
// itself into thin LTO with 16 units, so the two shapes are now the same and
// a second name would only mean a second target dir: a whole extra compile of
// the tree for every session that runs the golden next to a release build.
// The local golden therefore builds on `release` and shares that tree. CI
// keeps the shipped shape (`ship`, fat LTO and one codegen unit), so the
// golden is still proven under both shapes.
export function goldenProfile(ci) {
  return ci ? "--profile=ship" : "--release";
}
const GOLDEN_PROFILE = goldenProfile(process.env.CI);
// The receipt line of the golden build: the profile it used and the seconds
// the build plus the run took. A local run prints the `release` time, a CI
// run the `ship` time, so the two build times are readable from the logs.
export function goldenBuildLine(profile, seconds) {
  return `[INFO] golden build (${profile}): ${seconds.toFixed(1)} s`;
}

function main() {
const raw = process.argv.slice(2);
if (raw.includes("--help") || raw.includes("-h")) {
  console.log(PROOF_HELP);
  process.exit(0);
}
let args;
try {
  args = parseProofArgs(raw);
} catch (error) {
  console.error(error.message);
  process.exit(2);
}

if (args.includes("--drift")) {
  const git = (gitArgs) => spawnSync("git", gitArgs, { encoding: "utf8" });
  const head = (git(["rev-parse", "HEAD"]).stdout ?? "").trim();
  const gates = existsSync(GREEN_DIR) ? readdirSync(GREEN_DIR).filter((f) => f.endsWith(GREEN_SUFFIX)).map((f) => f.slice(0, -GREEN_SUFFIX.length)) : [];
  let green = gates.length > 0;
  if (!green) console.log(`[FAIL] drift: no green record in ${GREEN_DIR}, run the gate green once`);
  for (const gate of gates) {
    let record = null;
    try { record = JSON.parse(readFileSync(join(GREEN_DIR, `${gate}${GREEN_SUFFIX}`), "utf8")); } catch { /* unreadable reads as missing */ }
    const isAncestor = !!record?.sha && git(["merge-base", "--is-ancestor", record.sha, head]).status === 0;
    const verdict = driftVerdict(gate, record, { head, isAncestor });
    console.log(verdict.line);
    green &&= verdict.ok;
  }
  console.log(green ? "RESULT PASS (drift)" : "RESULT FAIL (drift)");
  process.exit(green ? 0 : 1);
}

if (args.includes("--mutants")) {
  // Tracked edits come from `git diff HEAD`, new source files as /dev/null
  // diffs. tools/mutants.mjs fails closed per crate on a missed mutant, a red
  // unmutated baseline, an overrun budget or a tool error.
  const git = (gitArgs) => spawnSync("git", gitArgs, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).stdout ?? "";
  let diff = git(["diff", "HEAD", "--", "crates/"]);
  const fresh = git(["ls-files", "--others", "--exclude-standard", "--", "crates/"]).split("\n").filter((p) => /^crates\/[^/]+\/src\/.+\.rs$/.test(p));
  for (const path of fresh) diff += git(["diff", "--no-index", "--", "/dev/null", path]);
  const dirs = changedCrateDirs(diff);
  if (!dirs.length) {
    console.log("RESULT PASS (mutants: no crate src/ change against HEAD)");
    process.exit(0);
  }
  let green = true;
  for (const dir of dirs) {
    const name = packageNameOf(readFileSync(join("crates", dir, "Cargo.toml"), "utf8"));
    const run = spawnSync(process.execPath, ["tools/mutants.mjs", "--patch", "-", "-p", name], { input: diff, encoding: "utf8", maxBuffer: 16 * 1024 * 1024, stdio: ["pipe", "pipe", "inherit"] });
    const passed = !run.error && run.status === 0;
    console.log(`[${passed ? "PASS" : "FAIL"}] mutants ${name}`);
    process.stdout.write(run.stdout ?? "");
    green &&= passed;
  }
  console.log(green ? "RESULT PASS (mutants)" : "RESULT FAIL (mutants)");
  process.exit(green ? 0 : 1);
}

if (args.includes("--perf")) {
  // Release build, one test thread: numbers are only meaningful without contention.
  const result = spawnSync("cargo", ["test", "--locked", "--workspace", "--release", "--all-features", "--", "--ignored", "--test-threads=1"], { encoding: "utf8", timeout: 1_200_000, maxBuffer: 16 * 1024 * 1024, env: timedEnv() });
  const out = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  for (const line of out.split("\n").filter((l) => /^test .* \.\.\. |panicked/.test(l))) console.log(line);
  // PQ1 input-latency leg (wave-44 judge FAIL-to-tick: headless_report
  // unwired): run the engine-perf latency suite and print its
  // headless_report lines (sim/present p50/p95 + GPU-PENDING marker) into
  // the proof log. Leg passes iff the latency tests pass; a latency
  // failure exits non-zero here so the --perf gate cannot pass without it.
  // Collected wave 45: root-relative cargo (lane prefixes rewritten).
  const latency = spawnSync("cargo", ["test", "--locked", "-p", "engine-perf", "latency", "--", "--nocapture"], { encoding: "utf8", timeout: 1_200_000, maxBuffer: 16 * 1024 * 1024, env: timedEnv() });
  const latencyOut = `${latency.stdout ?? ""}${latency.stderr ?? ""}`;
  for (const line of latencyOut.split("\n").filter((l) => /input_latency|reference_gpu|sim_p50|sim_p95|present_p50|present_p95|GPU-PENDING|PENDING/.test(l))) console.log(line);
  const latencyOk = !latency.error && latency.status === 0;
  console.log(latencyOk ? "[PASS] PQ1 input latency (headless_report in proof log)" : "[FAIL] PQ1 input latency (engine-perf latency suite)");
  if (!latencyOk) {
    console.log("RESULT FAIL (PQ1 input latency)");
    process.exit(1);
  }
  if (existsSync(FRAME_CSV_PATH)) {
    const verdict = frameCsvVerdict(readFileSync(FRAME_CSV_PATH, "utf8"));
    console.log(verdict.line);
    if (!verdict.ok) {
      console.log("RESULT FAIL (frame times)");
      process.exit(1);
    }
  } else {
    console.log("[SKIP] frame times: no target/perf/frame-times.csv yet (arena --frames N --exit --timed-smoke writes it)");
  }
  if (process.platform === "linux") {
    // Callgrind counts repeat exactly, so the 2% Ir budget set in
    // crates/perf/benches/instructions.rs gates where wall clock cannot.
    const ir = spawnSync("cargo", ["bench", "--locked", "-p", "engine-perf", "--bench", "instructions", "--", "--baseline=perf_gate"], { encoding: "utf8", timeout: 1_200_000, maxBuffer: 16 * 1024 * 1024, env: timedEnv() });
    const verdict = irGateVerdict(ir);
    console.log(verdict.line);
    if (!verdict.ok) {
      console.log(`[INFO] ${IR_SETUP}`);
      console.log("RESULT FAIL (instruction counts)");
      process.exit(1);
    }
  } else {
    console.log("[SKIP] instruction counts: Callgrind runs on Linux only");
  }
  const ok = !result.error && result.status === 0;
  console.log(ok ? "RESULT PASS (timing tests, release, one thread)" : "RESULT FAIL (timing tests)");
  process.exit(ok ? 0 : 1);
}

function cargo(args, timeoutMs = 1_800_000) {
  // Suite growth (wave 21): the all-features workspace leg exceeds 10 min on
  // the reference laptop, so the harness budget is 30 min, not 10.
  // Sentinel 2026-09-28T13:30Z: the monolithic all-features workspace leg hit
  // spawnSync ETIMEDOUT (60 min) while the default-features leg passed, so no
  // single default test hangs; the timeout driver is the all-features-only
  // expansion (engine-render window/GPU chart tests with wait_indefinitely
  // submits plus the proof m1_workflow suite) compounded by -j4 contention
  // (BLOCKERS wave-32 proof-lib flake, wave-55 package-cache lock). Each leg
  // keeps its own 30 min budget; the split below gives the same coverage two
  // budgets instead of one. Timeout details keep the output tail so the next
  // hang names its package instead of printing ETIMEDOUT alone.
  const result = spawnSync("cargo", args, { encoding: "utf8", timeout: timeoutMs, maxBuffer: 16 * 1024 * 1024, env: cargoEnv() });
  if (result.error) {
    const tail = `${result.stdout ?? ""}${result.stderr ?? ""}`.split("\n").filter(Boolean).slice(-8).join(" | ");
    const detail = `${result.error.message}${tail ? ` | tail: ${tail.slice(0, 2000)}` : ""}`;
    return { ok: false, out: result.stdout ?? "", error: detail };
  }
  return { ok: result.status === 0, out: result.stdout ?? "", error: result.stderr ?? "" };
}

// Nextest runner for the `cargo test` legs (exported shape tested via the
// pure helpers above): when `cargo nextest` and `.config/nextest.toml`
// `[profile.proof]` both exist the leg runs as
// `cargo nextest run --profile proof --cargo-profile fast-test ...` with its
// junit receipt and visible retry count; otherwise it falls back to the
// plain `cargo test` leg with the same failures visible and `0 retried`.
// Partitioned legs add `--partition count:M/N` (default legs run
// unpartitioned) and post the per-partition junit artifact
// (`target/nextest-junit/proof-M-N.xml`) beside the merged `proof.xml`, so
// every leg line names its junit path and its `<n> retried` count.
let nextestProbe = null;
function useNextest() {
  if (nextestProbe !== null) return nextestProbe;
  try {
    const probe = spawnSync("cargo", ["nextest", "--version"], { encoding: "utf8", timeout: 30_000, env: cargoEnv() });
    const bin = !probe.error && probe.status === 0;
    const text = existsSync(NEXTEST_CONFIG) ? readFileSync(NEXTEST_CONFIG, "utf8") : "";
    nextestProbe = bin && hasNextestProfile(text);
  } catch {
    nextestProbe = false;
  }
  return nextestProbe;
}

function cargoTest(command, partition = null) {
  const mapped = nextestTestCommand(command, partition);
  const junit = nextestJunitPath(partition);
  if (!mapped || !useNextest()) {
    const fallback = cargo(command);
    return { ...fallback, flake: nextestFlakeSummary(`${fallback.out ?? ""}\n${fallback.error ?? ""}`), via: "cargo", junit, partition: partition ?? null };
  }
  const run = cargo(mapped);
  return { ...run, flake: nextestFlakeSummary(`${run.out ?? ""}\n${run.error ?? ""}`), via: "nextest", junit, partition: partition ?? null };
}

// GPU tests with no adapter print `SKIPPED <test>: <why>` (render's
// test_gpu::require_or_skip). Counting those lines lets the RESULT line name
// the GPU tests that did not run.
function countSkipped(text) {
  return text.split("\n").filter((line) => /^SKIPPED \S/.test(line.trim())).length;
}

let renderSkipped = null;
let failed = false;
function check(name, passed, detail = "") {
  console.log(`[${passed ? "PASS" : "FAIL"}] ${name}${detail ? ` — ${detail}` : ""}`);
  if (!passed) failed = true;
}

function proof(args) {
  const started = Date.now();
  const result = cargo(["run", "-p", "engine2040", "--locked", GOLDEN_PROFILE, "--quiet", "--", ...args, "--format", "json"]);
  const seconds = (Date.now() - started) / 1000;
  if (!result.ok) throw new Error(result.error || result.out || "proof process failed");
  if (args.includes("--verify") && args.length === 1) {
    console.log(goldenBuildLine(GOLDEN_PROFILE, seconds));
  }
  return JSON.parse(result.out);
}

if (!args.includes("--skip-build")) {
  const audit = spawnSync(process.execPath, ["tools/workspace-check.mjs"], { encoding: "utf8" });
  check("workspace metadata, dependency edges and compiled modules", !audit.error && audit.status === 0, audit.status === 0 ? "" : audit.stderr || audit.stdout);
  for (const command of [
    ["fmt", "--all", "--", "--check"],
    ["build", "--locked", "--workspace", "--release"],
    // Default features prove the headless std-only build on its own.
    // -j4 caps concurrent test binaries: full parallelism saturates this
    // machine and flakes the loopback-socket/timing tests (provider, game,
    // net) that all pass alone; assertions are unchanged, only pressure.
    ["test", "-j4", "--locked", "--workspace", "--all-targets"],
    ["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"],
    // Feature-gated lanes (window, import, registry, math) must be tested and
    // linted too. Window code compiles here; this is not GPU execution.
    // Split 2026-09-28 (sentinel ETIMEDOUT on the monolithic leg): the two
    // test legs partition the workspace exactly once (exclude + single =
    // workspace), so no test is skipped and no flag is weakened. Render GPU
    // chart tests run alone instead of contending with net/provider socket
    // tests; each leg keeps its own 30 min budget (60 min total).
    ["test", "-j4", "--locked", "--workspace", "--exclude", "engine-render", "--all-targets", "--all-features"],
    ["test", "-j4", "--locked", "-p", "engine-render", "--all-targets", "--all-features"],
    ["clippy", "--locked", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"],
  ]) {
    const isTest = command[0] === "test";
    const result = isTest ? cargoTest(command) : cargo(command);
    if (isTest && command.includes("engine-render")) {
      renderSkipped = countSkipped(`${result.out ?? ""}\n${result.error ?? ""}`);
    }
    const flake = isTest ? `, ${result.flake} via ${result.via} junit ${result.junit}` : "";
    // Every test leg needs one test to have run, not just exit 0 (see legVerdict).
    const leg = legVerdict(isTest, result.ok, `${result.out ?? ""}\n${result.error ?? ""}`);
    check(
      `cargo ${command.join(" ")}`,
      leg.pass,
      leg.pass ? (isTest ? `${result.flake} via ${result.via} junit ${result.junit}` : "") : (leg.why ?? `${result.error || result.out}${flake}`),
    );
  }
  // Named consumer gates (wave 21): RV1 red-team tapes replay every wave;
  // PQ2 balance-tournament gate reproduces its report x2 (GA3 consumes the
  // pinned stuck legs); VK1 keypoint suite runs 30 gameplay specs (ARC8
  // spine specs join it). Wave 24 adds PV5 provider matrix + LW4 living
  // eval. Wave 30 adds SP12 spatial-gold eval. Wave 34 adds PQ10
  // exploit-gate replay (the proof-shape sweep consumed by HR6). All run inside the
  // workspace gates above too; the names here are the acceptance hooks.
  for (const command of [
    ["test", "--locked", "-p", "engine-endpoint", "--test", "it", "redteam::"],
    ["test", "--locked", "-p", "engine-endpoint", "--features", "mcp", "--lib", "packs::"],
    ["test", "--locked", "-p", "engine-playtest", "tournament::tests::proof_gate_reproduces"],
    ["test", "--locked", "-p", "engine-playtest", "keypoint"],
    ["test", "--locked", "-p", "engine-provider", "--test", "matrix"],
    ["test", "--locked", "-p", "engine-playtest", "living"],
    ["test", "--locked", "-p", "engine-playtest", "exploit_gate_replays"],
    ["test", "--locked", "-p", "engine-playtest", "spatial_eval"],
    // BX28 Bodies proof: 10,000 seeded cuts through the cut resolver, one
    // pinned digest (GOLDEN_CUT_DIGEST), run twice in the test itself.
    ["test", "--locked", "-p", "engine-proof", "--lib", "break_corpus"],
  ]) {
    const result = cargoTest(command);
    const combined = `${result.out ?? ""}\n${result.error ?? ""}`;
    const leg = legVerdict(true, result.ok, combined);
    check(
      `cargo ${command.join(" ")}`,
      leg.pass,
      leg.pass
        ? `${result.flake} via ${result.via} junit ${result.junit}`
        : leg.why
          ? `${leg.why}, ${result.flake} via ${result.via} junit ${result.junit}`
          : `${result.error || result.out}, ${result.flake} via ${result.via} junit ${result.junit}`,
    );
  }
  const deny = cargo(["deny", "check"]);
  check("cargo deny check", deny.ok, deny.ok ? "" : deny.error || deny.out);
} else {
  console.log("[INFO] compilation/test/lint gates skipped; running proof fixtures");
}

let temp;
try {
  const a = proof(["--verify"]);
  // Version pin folds into the golden check: a stale-version receipt with the
  // right hash still FAILs, and the seed double-runs below prove determinism.
  const workspaceVersion = (readFileSync("Cargo.toml", "utf8").split("[workspace.package]")[1] ?? "").match(/^\s*version\s*=\s*"([^"]+)"/m)?.[1] ?? "";
  check("legacy golden", goldenVerdict(a, workspaceVersion), `${a.final_hash} engine=${a.engine_version ?? "missing"} workspace=${workspaceVersion || "missing"}`);

  // Seeds 2k and 2k+1 share a legacy trajectory, so these differ above bit 0.
  // Receipts fold in the raw seed; engine-proof's unit tests assert that these
  // seeds also simulate differently. Keep both seed lists in sync.
  const hashes = new Set();
  for (const seed of ["0", "2", "2040", "999983"]) {
    const a = proof(["--seed", seed, "--ticks", "40", "--verify"]);
    const b = proof(["--seed", seed, "--ticks", "40", "--verify"]);
    check(`seed ${seed} repeated`, a.seed === seed && b.seed === seed && a.ticks === 40 && b.ticks === 40 && a.final_hash === b.final_hash && a.state_hash === b.state_hash && a.verified && b.verified, a.final_hash);
    hashes.add(a.final_hash);
  }
  check("different seeds give different receipts", hashes.size === 4);

  // EV13 record-path policy (wave 47): the proof CLI refuses absolute
  // tape paths, so keep the roundtrip tape inside the repo worktree
  // (gitignored target/, relative path) instead of the OS tmpdir.
  temp = join("target", "proof-tape");
  mkdirSync(temp, { recursive: true });
  const path = join(temp, "actions.e2040");
  const recorded = proof(["--seed", "19", "--ticks", "120", "--input", "3:0:-8:4", "--input", "40:1:2:6", "--record", path, "--verify"]);
  const replayed = proof(["--replay", path]);
  check("saved nonempty input tape replay", recorded.inputs === 2 && replayed.inputs === 2 && replayed.mode === "replay" && replayed.verified && recorded.final_hash === replayed.final_hash && recorded.state_hash === replayed.state_hash, replayed.final_hash);

  const bytes = readFileSync(path);
  bytes[bytes.length - 1] ^= 1;
  writeFileSync(path, bytes);
  const corrupted = cargo(["run", "-p", "engine2040", "--locked", GOLDEN_PROFILE, "--quiet", "--", "--replay", path]);
  check("corrupted checkpoint fails with first divergent tick", !corrupted.ok && corrupted.error.includes("diverged at tick 119"));
  const unsupported = cargo(["run", "-p", "engine2040", "--locked", GOLDEN_PROFILE, "--quiet", "--", "--backend", "wgpu"]);
  check("unsupported proof backend cannot fall back silently", !unsupported.ok && unsupported.error.includes("unsupported proof backend"));

  // GX6 truth gate (report-only): only forward is LIVE by on/off pixel proof;
  // the other eight stay PROBE until device captures prove them.
  // It turns enforcing (failing this gate) only when every registry entry
  // is live; until GX2-GX5 and GX7-GX10 land it reports without failing.
  const gpuLive = cargo(["test", "--locked", "--quiet", "-p", "engine-proof", "--lib", "gpu_live"]);
  const gpuLine = gpuLive.out.split("\n").filter((line) => line.includes("test result")).join("; ");
  const reports = existsSync("target/reports") ? readdirSync("target/reports").filter((f) => /^pass-truth-\d{4}-\d{2}-\d{2}\.md$/.test(f)).sort() : [];
  const deviceReport = reports.at(-1);
  const receipt = deviceReport ? gpuLiveReceipt(readFileSync(join("target/reports", deviceReport), "utf8")) : null;
  console.log(gpuLiveLine(receipt, deviceReport, gpuLive.ok ? gpuLine : `gate did not execute: ${gpuLive.error}`));
  // ARC10 spine scorecard: the parity column per build (legacy step against
  // spine step, 20 seeded 600-tick tapes). A mismatch fails the gate; the
  // canary run must fail too, or the column proves nothing.
  // The parity leg runs in the golden profile, so CI (which builds the shipped
  // shape) does not pay a second build of the same crates in another profile.
  const parityRun = (extra) => spawnSync("cargo", ["run", "-p", "engine-proof", "--locked", GOLDEN_PROFILE, "--quiet", "--features", "spine", "--bin", "spine_parity_line", "--", ...extra], { encoding: "utf8", timeout: 1_800_000, maxBuffer: 16 * 1024 * 1024, env: cargoEnv() });
  const parity = parityVerdict(parityRun([]));
  check("spine parity column", parity.ok, parity.line);
  const canary = parityVerdict(parityRun(["--seeded-wall"]), { canary: true });
  check("spine parity canary (seeded divergence fails the gate)", canary.ok, canary.line);
} catch (error) {
  check("proof execution", false, error.message);
} finally {
  if (temp) rmSync(temp, { recursive: true, force: true });
}

// GE9 generality ratchet (check, not refactor): runs every wave, in both
// full and --skip-build modes, so generality never rots while fixtures move.
// (a) GE2 genre-ledger grep gate: re-runs tools/workspace-check.mjs, which
// fails on any Fighter|Gladiator token outside the ledger/arena/game.
// (b) GP5 engine-diff-empty gate: fails on any crates/ working-tree diff,
// so a fixture that needs an engine edit cannot pass proof.
// (c) GE1 fixtures stay listed: the four fixture rule sets must exist; the
// full (non-skip-build) run additionally replays the ge1_genres and
// second_game harnesses as named hooks (they also run inside the
// workspace test legs above; the names here keep them listed until green).
try {
  const audit = spawnSync(process.execPath, ["tools/workspace-check.mjs"], { encoding: "utf8" });
  check("generality ratchet: GE2 genre ledger (workspace-check)", !audit.error && audit.status === 0, audit.status === 0 ? "" : (audit.stderr || audit.stdout || "workspace-check failed").split("\n").filter(Boolean).slice(-3).join(" | "));
} catch (error) {
  check("generality ratchet: GE2 genre ledger (workspace-check)", false, error.message);
}
try {
  const diff = spawnSync("git", ["diff", "--name-only", "--", "crates/"], { encoding: "utf8" });
  const status = spawnSync("git", ["status", "--porcelain", "--", "crates/"], { encoding: "utf8" });
  const diffFiles = `${diff.stdout ?? ""}`.split("\n").map((l) => l.trim()).filter(Boolean);
  const statusFiles = `${status.stdout ?? ""}`.split("\n").map((l) => l.trim()).filter(Boolean);
  const dirty = [...new Set([...diffFiles, ...statusFiles])];
  const ok = !diff.error && !status.error && diff.status === 0 && status.status === 0 && dirty.length === 0;
  check("generality ratchet: engine diff empty (GP5)", ok, ok ? "" : dirty.length ? `crates/ dirty: ${dirty.slice(0, 5).join(", ")}${dirty.length > 5 ? ` (+${dirty.length - 5} more)` : ""}` : "git gate did not execute");
} catch (error) {
  check("generality ratchet: engine diff empty (GP5)", false, error.message);
}
// Snapshot pending gate (CQ16): a `.snap.new` in the tree is a review that did
// not happen, so it fails with the path named. Static and cheap, so it runs in
// both full and --skip-build modes. Golden bytes themselves stay untouched.
{
  const verdict = pendingSnapshotVerdict(findPendingSnapshots("."));
  check("pending snapshots", verdict.ok, verdict.ok ? "" : verdict.line);
}
// Rust quality ratchet (docs/RUST-QUALITY.md, layer 3): static, about 2 s,
// so it runs in both modes. No crate's debt count may rise over the
// baseline, and simulation crates keep the canonical clippy config.
{
  const quality = spawnSync(process.execPath, ["tools/rust-quality.mjs"], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  const lines = `${quality.stdout ?? ""}${quality.stderr ?? ""}`.split("\n");
  const score = lines.find((l) => l.startsWith("WORKSPACE")) ?? "";
  const fails = lines.filter((l) => l.startsWith("[FAIL]") || l.startsWith("RESULT FAIL"));
  check("rust quality ratchet (tools/rust-quality.mjs)", !quality.error && quality.status === 0, quality.status === 0 ? score.replace(/\s+/g, " ") : fails.slice(0, 4).join(" | "));
}
// Docs coverage ratchet (API2): static, about 1 s. A crate's undocumented
// public items may not rise over tools/docs-coverage.baseline.json, so a new
// public item without a doc comment fails the proof.
{
  const docs = spawnSync(process.execPath, ["tools/docs-coverage.mjs"], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  const lines = `${docs.stdout ?? ""}${docs.stderr ?? ""}`.split("\n");
  const fails = lines.filter((l) => l.startsWith("[FAIL]") || l.startsWith("RESULT FAIL"));
  check("docs coverage ratchet (tools/docs-coverage.mjs)", !docs.error && docs.status === 0, docs.status === 0 ? (lines.find((l) => l.startsWith("RESULT")) ?? "") : fails.slice(0, 4).join(" | "));
}
// API2 handbook: the book builds clean. Skipped when mdbook is not installed
// (pages.yml builds the book on GitHub); the output goes outside the book folder.
{
  const probe = spawnSync("mdbook", ["--version"], { encoding: "utf8" });
  if (probe.error || probe.status !== 0) {
    console.log("[SKIP] mdbook build docs/book: mdbook is not installed (pages.yml builds the book on GitHub)");
  } else {
    const dest = join("target", "book-check");
    const verdict = bookBuildVerdict(spawnSync("mdbook", ["build", "docs/book", "--dest-dir", dest], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 }));
    rmSync(dest, { recursive: true, force: true });
    check("mdbook build docs/book", verdict.ok, verdict.line);
  }
}
// The gate tools gate themselves in CI nightly only (`node --test
// tools/*.test.mjs` in .github/workflows/ci.yml, E2040_NIGHTLY=1); the proof
// gate never reruns them, so per-packet proof stays fixtures only.
if (runsToolTests(args)) {
  const tests = readdirSync("tools").filter((f) => f.endsWith(".test.mjs")).map((f) => `tools/${f}`);
  const run = spawnSync(process.execPath, ["--test", ...tests], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 });
  const summary = toolTestSummary(`${run.stdout ?? ""}`);
  check(`tool tests (${tests.length} files)`, !run.error && run.status === 0, summary);
}
for (const fixture of ["fixtures/games/twin-stick/rules.json", "fixtures/games/vehicle-course/rules.json", "fixtures/games/traversal-course/rules.json", "fixtures/games/second-game/rules.json"]) {
  check(`generality ratchet: fixture listed (${fixture})`, existsSync(fixture), existsSync(fixture) ? "" : "missing fixture rules");
}
// SP12 spatial-gold generator: the checked-in TSV must match its generator
// (16 fix bases plus order, gold kept, capture_hex recomputed).
{
  const gen = spawnSync(process.execPath, ["fixtures/spatial-gold/generate.mjs", "--check"], { encoding: "utf8" });
  const out = `${gen.stdout ?? ""}${gen.stderr ?? ""}`.trim().split("\n").filter(Boolean).slice(-1).join(" ");
  check("spatial-gold generator matches tasks.tsv", !gen.error && gen.status === 0, out);
}
if (!args.includes("--skip-build")) {
  for (const command of [
    ["test", "--locked", "--test", "ge1_genres"],
    ["test", "--locked", "--test", "second_game"],
  ]) {
    const result = cargo(command);
    check(`cargo ${command.join(" ")}`, result.ok, result.ok ? "" : result.error || result.out);
  }
}

const skippedNote = renderSkipped === null ? "" : `; engine-render GPU tests SKIPPED: ${renderSkipped}`;
console.log((failed ? "RESULT FAIL" : args.includes("--skip-build") ? "RESULT PASS (proof fixtures only)" : "RESULT PASS (all local gates)") + skippedNote);
process.exitCode = failed ? 1 : 0;
const gate = args.includes("--skip-build") ? "fixtures" : "full";
const written = recordGreen(gate, { failed, sha: (spawnSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).stdout ?? "").trim() });
if (written) console.log(`[INFO] green record ${written}`);
} // main

if (import.meta.main) {
  main();
}

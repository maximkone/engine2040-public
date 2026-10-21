// Donor: mozilla/sccache (Apache-2.0) - idea only: measure one cold lane build
// before and after a warm compiler cache, on this machine, same command.
// All code below is written fresh for engine2040; no donor code is copied.
// SPDX-License-Identifier: Apache-2.0
// Ledger: mozilla/sccache | Apache-2.0 | cold lane cache spike | tools/compile-cache-spike.mjs | M

// OPT9 spike: measurement only. It never edits Cargo.toml, .cargo/config.toml or
// any lane tool; the cache is opt-in through the environment of each child cargo.
// Four cells, each in its own CARGO_TARGET_DIR so nothing is incremental:
//   cold  - rustc directly, incremental as configured: today's lane build
//   prime - sccache on an empty cache: the same lane with the spike flags
//   warm  - sccache on the cache prime filled: the "after" number
//   warm2 - warm again in a fresh dir: stability of two warm runs
// Run: node tools/compile-cache-spike.mjs [--lane <crate>]... [--jobs <n>] [--json] [--dry-run]
// Exit 0 when every cell built (the verdict is in the table, a DROP is not an
// error); 2 when a cell failed or an argument is wrong.
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { cpus, platform, totalmem } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

export const CELLS = ["cold", "prime", "rebuild", "warm", "warm2"];
export const DEFAULT_CRATES = ["engine-physics"];
export const DEFAULT_JOBS = 4;
export const SPEED_BAR = 0.30; // the row's bar: the warm lane must be 30% faster
export const HIT_BAR = 0.8;
export const STABLE_BAR = 0.1;

// The compile half of `laneCommand` (tools/affected.mjs) plus --no-run: a lane
// build is timed, not the tests it would then run.
export function laneArgs({ crates, jobs }) {
  return ["test", "-j", String(jobs), "--locked", ...crates.flatMap((c) => ["-p", c]), "--all-targets", "--no-run"];
}

// The client view of this run's cache: its own dir, its own server port (an
// already running server keeps the dir it started with, so a shared one would
// hide the spike's own traffic), and the repo as the base dir.
export const serverEnv = (dirs) => ({
  SCCACHE_DIR: dirs.cache,
  SCCACHE_BASEDIRS: dirs.base,
  ...(dirs.port ? { SCCACHE_SERVER_PORT: String(dirs.port) } : {}),
});

// The rebuild cell is the CI shape: the same target dir path (so the cache
// key still matches) with the artifacts removed. Every other cell owns a
// fresh dir of its own.
export function cellDir(cell, dirs) {
  return join(dirs.run, cell === "rebuild" ? "prime" : cell);
}

// Env overrides of one cell on top of the caller's env. The cold cell drops any
// inherited wrapper so it measures this machine as it builds today.
export function cellEnv(cell, dirs, base = {}) {
  const env = { ...base, CARGO_TARGET_DIR: cellDir(cell, dirs) };
  if (cell === "cold") {
    delete env.RUSTC_WRAPPER;
    delete env.CARGO_INCREMENTAL;
    delete env.SCCACHE_DIR;
    delete env.SCCACHE_BASEDIRS;
    delete env.SCCACHE_SERVER_PORT;
    return env;
  }
  return { ...env, ...serverEnv(dirs), RUSTC_WRAPPER: "sccache", CARGO_INCREMENTAL: "0" };
}

const sumCounts = (m) => Object.values(m ?? {}).reduce((a, b) => a + (Number(b) || 0), 0);

// sccache 0.18 keeps hits and misses as {counts, adv_counts} maps over language.
// counts is the total; adv_counts repeats it per flavour, so it is never added.
export function statsTotals(json) {
  const s = json?.stats ?? {};
  return { hits: sumCounts(s.cache_hits?.counts), misses: sumCounts(s.cache_misses?.counts), requests: Number(s.compile_requests) || 0 };
}

export function delta(before, after) {
  return { hits: after.hits - before.hits, misses: after.misses - before.misses, requests: after.requests - before.requests };
}

// Hits over cacheable requests: a not-cacheable build script is neither.
export function hitRate({ hits = 0, misses = 0 } = {}) {
  const n = hits + misses;
  return n > 0 ? hits / n : null;
}

const pct = (x) => `${Math.round(x * 100)}%`;

// Keep or drop from the four wall times and the warm hit rate.
export function verdict({ coldSeconds, primeSeconds, warmSeconds, warm2Seconds, rate, rebuildSeconds = null }) {
  const reasons = [];
  const ratio = coldSeconds > 0 && warmSeconds > 0 ? warmSeconds / coldSeconds : null;
  if (ratio === null) reasons.push("no cold/warm pair measured");
  else if (ratio > 1 - SPEED_BAR) reasons.push(`warm is ${pct(ratio)} of the cold lane, under the 30% faster bar`);
  if (rate === null || rate === undefined) reasons.push("no cache traffic, hit rate unknown");
  else if (rate < HIT_BAR) reasons.push(`hit rate ${pct(rate)} under the 80% bar`);
  if (warmSeconds > 0 && warm2Seconds > 0) {
    const drift = Math.abs(warm2Seconds - warmSeconds) / warmSeconds;
    if (drift > STABLE_BAR) reasons.push(`two warm runs differ by ${(drift * 100).toFixed(1)}%, over 10%`);
  }
  // The CI shape: the same target dir path with the artifacts removed.
  let rebuildRatio = null;
  if (rebuildSeconds !== null) {
    rebuildRatio = coldSeconds > 0 ? rebuildSeconds / coldSeconds : null;
    if (rebuildRatio === null) reasons.push("no cold/rebuild pair measured");
    else if (rebuildRatio > 1 - SPEED_BAR) reasons.push(`the CI-shape rebuild is ${pct(rebuildRatio)} of the cold lane, under the 30% faster bar`);
  }
  const spikeRatio = primeSeconds > 0 && warmSeconds > 0 ? warmSeconds / primeSeconds : null;
  return { keep: reasons.length === 0, ratio, rebuildRatio, spikeRatio, reasons };
}

export function parseArgs(argv) {
  const out = { crates: [...DEFAULT_CRATES], jobs: DEFAULT_JOBS, json: false, dryRun: false, error: null };
  const lanes = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--lane") {
      const v = argv[++i];
      if (!v || v.startsWith("-")) { out.error = "--lane needs a crate name"; return out; }
      lanes.push(v);
    } else if (a === "--jobs") {
      const n = Number(argv[++i]);
      if (!Number.isInteger(n) || n < 1) { out.error = "--jobs needs a whole number of 1 or more"; return out; }
      out.jobs = n;
    } else if (a === "--json") out.json = true;
    else if (a === "--dry-run") out.dryRun = true;
    else if (a === "-h" || a === "--help") out.help = true;
    else { out.error = `unknown flag ${a}`; return out; }
  }
  if (lanes.length) out.crates = lanes;
  return out;
}

export function helpText() {
  return [
    "Usage: node tools/compile-cache-spike.mjs [--lane <crate>]... [--jobs <n>] [--json] [--dry-run]",
    "  cold, prime, warm and warm2 cells of one lane build, each in its own target dir.",
    `  Defaults: --lane ${DEFAULT_CRATES.join(" --lane ")} --jobs ${DEFAULT_JOBS}.`,
    "  A DROP verdict still exits 0: the numbers are the result, not a gate.",
    "  Spike only: it writes target/spike-opt9/<utc>/ and no tracked file.",
  ].join("\n") + "\n";
}

const cellLine = (cell, args, dirs) => {
  const env = cellEnv(cell, dirs);
  const shown = Object.entries(env).map(([k, v]) => `${k}=${v}`).join(" ");
  const note = cell === "cold"
    ? "baseline: no RUSTC_WRAPPER, incremental as configured"
    : cell === "rebuild"
      ? "CI shape: same target dir path as prime, artifacts removed, cache primed"
      : "opt-in wrapper for this run only";
  return `[${cell}] cargo ${args.join(" ")}  (${note}; ${shown})`;
};

const clientEnv = (dirs) => ({ ...process.env, ...serverEnv(dirs) });

const readStats = (dirs) => {
  const r = spawnSync("sccache", ["--show-stats", "--stats-format", "json"], {
    cwd: ROOT,
    env: clientEnv(dirs),
    encoding: "utf8",
    maxBuffer: 1 << 26,
  });
  if (r.status !== 0) return { error: (r.stderr || r.stdout || "sccache --show-stats failed").trim().split("\n")[0] };
  try {
    return statsTotals(JSON.parse(r.stdout));
  } catch (e) {
    return { error: `could not read sccache stats: ${e.message}` };
  }
};

const startServer = (dirs) => {
  const r = spawnSync("sccache", ["--start-server"], { cwd: ROOT, env: clientEnv(dirs), encoding: "utf8", maxBuffer: 1 << 20 });
  if (r.status !== 0) return (r.stderr || r.stdout || "sccache --start-server failed").trim().split("\n")[0];
  const z = spawnSync("sccache", ["--zero-stats"], { cwd: ROOT, env: clientEnv(dirs), encoding: "utf8", maxBuffer: 1 << 20 });
  return z.status === 0 ? null : `could not zero the counters: ${(z.stderr || "").trim().split("\n")[0]}`;
};

const stamp = () => new Date().toISOString().replace(/[-:]/g, "").replace(/\..+/, "");

const main = (argv) => {
  const a = parseArgs(argv);
  if (a.help) { process.stdout.write(helpText()); return 0; }
  if (a.error) { console.log(`refused: ${a.error}`); return 2; }

  const runDir = join(ROOT, "target", "spike-opt9", stamp());
  // Own port: a server already running for another agent keeps its own dir.
  const dirs = { run: runDir, cache: join(runDir, "sccache"), base: ROOT, port: 4300 + (process.pid % 700) };
  const args = laneArgs(a);
  if (a.dryRun) { for (const cell of CELLS) console.log(cellLine(cell, args, dirs)); return 0; }

  mkdirSync(dirs.cache, { recursive: true });
  const started = startServer(dirs);
  if (started) { console.log(`could not start the spike's own cache server: ${started}`); return 2; }
  console.log(`[cache] own sccache server on 127.0.0.1:${dirs.port}, dir ${dirs.cache}, basedir ${dirs.base}`);

  const cells = {};
  for (const cell of CELLS) {
    console.log(cellLine(cell, args, dirs));
    if (cell === "rebuild") {
      // The CI shape: the same target dir path, the artifacts gone. Only this
      // spike's own directory is cleaned (target/spike-opt9/<utc>/prime).
      const clean = spawnSync("cargo", ["clean", "--target-dir", cellDir(cell, dirs)], { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 24 });
      if (clean.status !== 0) {
        console.log(`[${cell}] could not clear the spike's own target dir (exit ${clean.status}): no measurement`);
        return 2;
      }
      console.log(`[${cell}] cleared ${cellDir(cell, dirs)} (the spike's own dir)`);
    }
    const before = cell === "cold" ? null : readStats(dirs);
    if (before?.error) { console.log(`[${cell}] ${before.error}`); return 2; }
    const t0 = performance.now();
    const r = spawnSync("cargo", args, { cwd: ROOT, env: cellEnv(cell, dirs, process.env), encoding: "utf8", maxBuffer: 1 << 28 });
    const seconds = Math.round((performance.now() - t0) / 100) / 10;
    if (r.status !== 0) {
      const tail = `${r.stdout ?? ""}${r.stderr ?? ""}`.trimEnd().split("\n").slice(-20);
      console.log(tail.join("\n"));
      console.log(`[${cell}] the lane build failed (exit ${r.status}): no measurement, fix the build first`);
      return 2;
    }
    const after = cell === "cold" ? null : readStats(dirs);
    if (after?.error) { console.log(`[${cell}] ${after.error}`); return 2; }
    const traffic = before && after ? delta(before, after) : { hits: 0, misses: 0, requests: 0 };
    cells[cell] = { seconds, ...traffic, rate: hitRate(traffic) };
    const rate = cells[cell].rate;
    console.log(`[${cell}] ${seconds} s, ${traffic.hits} hits, ${traffic.misses} misses${rate === null ? "" : `, hit rate ${pct(rate)}`}`);
  }

  const v = verdict({
    coldSeconds: cells.cold.seconds,
    primeSeconds: cells.prime.seconds,
    rebuildSeconds: cells.rebuild.seconds,
    warmSeconds: cells.warm.seconds,
    warm2Seconds: cells.warm2.seconds,
    rate: cells.warm.rate,
  });
  const host = { platform: platform(), cpus: cpus().length, ram_gb: Math.round(totalmem() / 2 ** 30), crates: a.crates, jobs: a.jobs };
  const cache = { dir: dirs.cache, basedirs: dirs.base, port: dirs.port, version: spawnSync("sccache", ["--version"], { encoding: "utf8" }).stdout?.trim() ?? "unknown" };
  const report = { row: "OPT9", ran_utc: new Date().toISOString(), host, cache, lane_args: args, cells, verdict: v };

  console.log("");
  console.log("| cell | wall s | hits | misses | hit rate |");
  console.log("|---|---|---|---|---|");
  for (const cell of CELLS) {
    const c = cells[cell];
    console.log(`| ${cell} | ${c.seconds} | ${c.hits} | ${c.misses} | ${c.rate === null ? "-" : pct(c.rate)} |`);
  }
  const line = `RESULT ${v.keep ? "KEEP" : "DROP"}: warm ${cells.warm.seconds} s vs cold ${cells.cold.seconds} s (${v.ratio === null ? "no ratio" : `${pct(1 - v.ratio)} faster`}), warm hit rate ${cells.warm.rate === null ? "unknown" : pct(cells.warm.rate)}${v.spikeRatio === null ? "" : `, warm vs prime ${pct(1 - v.spikeRatio)} faster`}${v.rebuildRatio === null ? "" : `, CI-shape rebuild ${cells.rebuild.seconds} s vs cold ${cells.cold.seconds} s (${pct(1 - v.rebuildRatio)} faster, ${cells.rebuild.hits} hits)`}${v.reasons.length ? `: ${v.reasons.join("; ")}` : ""}`;

  mkdirSync(dirs.cache, { recursive: true });
  writeFileSync(join(runDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  if (a.json) console.log(JSON.stringify(report, null, 2));
  else console.log(line);
  console.log(`report: ${join("target", "spike-opt9", runDir.split(/[\\/]/).pop(), "report.json")}`);
  return 0;
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));

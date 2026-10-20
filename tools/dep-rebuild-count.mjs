// OPT8 spike: count the dependency units five lane builds rebuild.
//
// The bar (board row OPT8): the dependency rebuild count across five lane
// builds drops by a third when the crates depend on the workspace-hack (the
// cargo-hakari pattern). This tool measures that count the way a lane pool
// sees it: every lane builds in ONE shared CARGO_TARGET_DIR, so a dependency
// that two lanes want with different features is compiled twice (two copies,
// two metadata hashes) and shows up as two "Compiling" lines.
//
// Run: node tools/dep-rebuild-count.mjs [--lane <crate>]... [--jobs <n>]
//                                     [--target-dir <dir>] [--json] [--dry-run]
// Exit 0 when every lane built (the number is the result, not a gate),
// 2 when a lane failed or an argument is wrong.
import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

export const DEFAULT_LANES = ["engine-physics", "engine-input", "engine-nav", "engine-combat", "engine-ai"];
export const DEFAULT_JOBS = 4;
// The row's bar: the count must drop by at least a third.
export const DROP_BAR = 1 / 3;

// The compile half of the affected gate's lane command (tools/affected.mjs)
// plus --no-run: a lane build is timed, not the tests it would then run.
export function laneArgs({ crates, jobs }) {
  return ["test", "-j", String(jobs), "--locked", ...crates.flatMap((c) => ["-p", c]), "--all-targets", "--no-run"];
}

// "   Compiling serde v1.0.229" -> "serde v1.0.229".
export function parseCompiling(output) {
  const out = [];
  for (const line of String(output ?? "").split(/\r?\n/)) {
    const m = /^\s*Compiling\s+(\S+)\s+v(\S+)/.exec(line);
    if (m) out.push(`${m[1]} v${m[2]}`);
  }
  return out;
}

// The lane's own crate is not a dependency: it is the unit under test.
export function dependencyUnits(compiled, ownCrate) {
  return compiled.filter((unit) => {
    const name = unit.split(" ")[0];
    return name !== ownCrate;
  });
}

// Copies of one dependency across the lanes: cargo prints a "Compiling" line
// only when it really builds a unit, so a line per lane means a second copy
// (a different feature set, a different metadata hash).
export function copyCounts(laneCompiled) {
  const counts = new Map();
  for (const units of laneCompiled) {
    for (const unit of units) {
      const name = unit.split(" ")[0];
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
  }
  return counts;
}

export function parseArgs(argv) {
  const out = { crates: [], jobs: DEFAULT_JOBS, targetDir: null, json: false, dryRun: false, error: null };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--lane") {
      const v = argv[++i];
      if (!v || v.startsWith("-")) { out.error = "--lane needs a crate name"; return out; }
      out.crates.push(v);
    } else if (a === "--jobs") {
      const n = Number(argv[++i]);
      if (!Number.isInteger(n) || n < 1) { out.error = "--jobs needs a whole number of 1 or more"; return out; }
      out.jobs = n;
    } else if (a === "--target-dir") {
      const v = argv[++i];
      if (!v) { out.error = "--target-dir needs a path"; return out; }
      out.targetDir = v;
    } else if (a === "--json") out.json = true;
    else if (a === "--dry-run") out.dryRun = true;
    else if (a === "-h" || a === "--help") out.help = true;
    else { out.error = `unknown flag ${a}`; return out; }
  }
  if (!out.crates.length) out.crates = [...DEFAULT_LANES];
  return out;
}

export function helpText() {
  return [
    "Usage: node tools/dep-rebuild-count.mjs [--lane <crate>]... [--jobs <n>] [--target-dir <dir>] [--json] [--dry-run]",
    `  Builds ${DEFAULT_LANES.join(", ")} as lane builds in ONE shared target dir`,
    "  and counts the dependency units they rebuild (the OPT8 bar: a third less).",
    "  --target-dir defaults to target/spike-opt8/<utc>; the dir must be fresh for the count to mean anything.",
  ].join("\n") + "\n";
}

// The verdict: the count after against the count before.
export function verdict({ before, after }) {
  if (!Number.isFinite(before) || !Number.isFinite(after) || before <= 0) return { keep: false, ratio: null, reason: "no before/after pair" };
  const ratio = after / before;
  return {
    keep: ratio <= 1 - DROP_BAR,
    ratio,
    reason: `after ${after} vs before ${before} (${Math.round((1 - ratio) * 100)}% less, the bar is ${Math.round(DROP_BAR * 100)}%)`,
  };
}

const stamp = () => new Date().toISOString().replace(/[-:]/g, "").replace(/\..+/, "");

const main = (argv) => {
  const a = parseArgs(argv);
  if (a.help) { process.stdout.write(helpText()); return 0; }
  if (a.error) { console.log(`refused: ${a.error}`); return 2; }
  const targetDir = a.targetDir ?? join(ROOT, "target", "spike-opt8", stamp());
  const args = laneArgs(a);
  if (a.dryRun) {
    for (const c of a.crates) {
      const one = ["test", "-j", String(a.jobs), "--locked", "-p", c, "--all-targets", "--no-run"];
      console.log(`[${c}] cargo ${one.join(" ")}  (CARGO_TARGET_DIR=${targetDir})`);
    }
    return 0;
  }
  mkdirSync(targetDir, { recursive: true });
  const lanes = [];
  for (const crate of a.crates) {
    const one = ["test", "-j", String(a.jobs), "--locked", "-p", crate, "--all-targets", "--no-run"];
    const t0 = performance.now();
    const r = spawnSync("cargo", one, { cwd: ROOT, env: { ...process.env, CARGO_TARGET_DIR: targetDir }, encoding: "utf8", maxBuffer: 1 << 28 });
    const seconds = Math.round((performance.now() - t0) / 100) / 10;
    if (r.status !== 0) {
      const tail = `${r.stdout ?? ""}${r.stderr ?? ""}`.trimEnd().split("\n").slice(-20);
      console.log(tail.join("\n"));
      console.log(`[${crate}] the lane build failed (exit ${r.status}): no count, fix the build first`);
      return 2;
    }
    const compiled = parseCompiling(`${r.stdout ?? ""}${r.stderr ?? ""}`);
    const units = dependencyUnits(compiled, crate);
    lanes.push({ crate, seconds, compiled: compiled.length, units: units.length, unitList: units, compiledList: compiled });
    console.log(`[${crate}] ${seconds} s, ${units.length} dependency units rebuilt (${compiled.length} units compiled)`);
  }
  const total = lanes.reduce((a2, l) => a2 + l.units, 0);
  const copies = copyCounts(lanes.map((l) => l.compiledList));
  const duplicated = [...copies.entries()].filter(([, n]) => n > 1).sort((x, y) => y[1] - x[1]);
  console.log("");
  console.log("| lane | wall s | dependency units rebuilt |");
  console.log("|---|---|---|");
  for (const l of lanes) console.log(`| ${l.crate} | ${l.seconds} | ${l.units} |`);
  console.log(`dependency rebuild count across ${lanes.length} lane builds: ${total}`);
  console.log(`distinct dependencies: ${copies.size}, copied more than once: ${duplicated.length}`);
  for (const [name, n] of duplicated.slice(0, 12)) console.log(`  ${name}: ${n} copies`);
  const report = { row: "OPT8", ran_utc: new Date().toISOString(), target_dir: targetDir, lanes: lanes.map((l) => ({ crate: l.crate, seconds: l.seconds, units: l.units })), total_units: total, copied_more_than_once: duplicated.map(([name, n]) => ({ name, copies: n })) };
  writeFileSync(join(targetDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  console.log(`report: ${join("target", "spike-opt8", targetDir.split(/[\\/]/).pop(), "report.json")}`);
  if (a.json) console.log(JSON.stringify(report, null, 2));
  return 0;
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));

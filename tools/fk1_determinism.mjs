// FK1 determinism gate: one scripted 6k-tick headless arena match, twice.
//
// Runs the gym headless path (`crates/arena/examples/fk1_hashdump.rs`, a
// scripted `GymLive` match that also asserts gym/run parity) as two
// independent processes, compares the per-tick hash chains byte-identical and
// prints PASS/FAIL plus the first-diverging tick (HASH-LAW tier 1).
//
// Usage (from the repository root):
//   node tools/fk1_determinism.mjs [--seed <u64>] [--ticks <u32>]
//     [--fighters <u8>] [--extent <m>] [--out <dir>] [--release|--debug]
//   node tools/fk1_determinism.mjs --help
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const FK1_DEFAULTS = {
  seed: "2040",
  ticks: "6000",
  fighters: "4",
  extent: "20",
  profile: "release",
};

export const FK1_USAGE = [
  "usage: node tools/fk1_determinism.mjs [--seed <u64>] [--ticks <u32>] [--fighters <u8>] [--extent <m>] [--out <dir>] [--release|--debug]",
  "  defaults: seed=2040 ticks=6000 fighters=4 extent=20 profile=release",
  "  chains land in <root>/target/fk1-determinism/ (gitignored) unless --out sets a dir",
].join("\n");

// Pure CLI parse (exported for tests): throws the usage line on anything
// unexpected. Values stay strings; the Rust helper owns numeric checks.
export function parseFk1Args(argv) {
  const args = { ...FK1_DEFAULTS, out: null };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    if (flag === "--help" || flag === "-h") throw new Error(FK1_USAGE);
    if (flag === "--release" || flag === "--debug") {
      args.profile = flag.slice(2);
      continue;
    }
    const next = argv[i + 1];
    if (flag === "--seed" || flag === "--ticks" || flag === "--fighters" || flag === "--extent" || flag === "--out") {
      if (next === undefined || next.startsWith("--")) throw new Error(`${FK1_USAGE}\nmissing value for ${flag}`);
      i += 1;
      if (flag === "--out") args.out = next;
      else args[flag.slice(2)] = next;
    } else {
      throw new Error(`${FK1_USAGE}\nunknown argument: ${flag}`);
    }
  }
  return args;
}

// Pure chain compare (exported for tests): first index where the two chains
// differ, or -1 when they are byte-identical. A length mismatch diverges at
// the shared-prefix end.
export function firstDivergingTick(aLines, bLines) {
  const shared = Math.min(aLines.length, bLines.length);
  for (let i = 0; i < shared; i += 1) {
    if (aLines[i] !== bLines[i]) return i;
  }
  if (aLines.length !== bLines.length) return shared;
  return -1;
}

// Last `hash` hex cell of a chain file's rows (`000123 abcdef...`).
export function finalHashOf(lines) {
  if (lines.length === 0) return "none";
  const cells = lines[lines.length - 1].trim().split(/\s+/);
  return cells[cells.length - 1] ?? "none";
}

function runHelper(binary, args, outPath) {
  const run = spawnSync(binary, ["--seed", args.seed, "--ticks", args.ticks, "--fighters", args.fighters, "--extent", args.extent], {
    encoding: "utf8",
    timeout: 1_200_000,
    maxBuffer: 256 * 1024 * 1024,
  });
  if (run.error) throw new Error(`helper run failed: ${run.error.message}`);
  if (run.status !== 0) throw new Error(`helper exited ${run.status}: ${(run.stderr ?? "").trim().split("\n").slice(-3).join(" | ")}`);
  writeFileSync(outPath, run.stdout ?? "");
  return `${run.stderr ?? ""}`.trim().split("\n").filter(Boolean).slice(-1).join("");
}

function main(argv, cwd) {
  let args;
  try {
    args = parseFk1Args(argv);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 2;
    return;
  }
  const build = spawnSync("cargo", ["build", "--locked", "-p", "engine-arena", "--example", "fk1_hashdump", args.profile === "release" ? "--release" : null].filter(Boolean), {
    cwd,
    encoding: "utf8",
    timeout: 1_800_000,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (build.error || build.status !== 0) {
    process.stderr.write(`fk1_determinism: helper build failed: ${build.error ? build.error.message : (build.stderr ?? build.stdout ?? "").split("\n").filter(Boolean).slice(-5).join(" | ")}\n`);
    process.exitCode = 1;
    return;
  }
  const exe = process.platform === "win32" ? "fk1_hashdump.exe" : "fk1_hashdump";
  const binary = join(cwd, "target", args.profile === "release" ? "release" : "debug", "examples", exe);
  if (!existsSync(binary)) {
    process.stderr.write(`fk1_determinism: helper binary missing: ${binary}\n`);
    process.exitCode = 1;
    return;
  }
  const outDir = args.out ? resolve(cwd, args.out) : join(cwd, "target", "fk1-determinism");
  mkdirSync(outDir, { recursive: true });
  const pathA = join(outDir, "chain-a.txt");
  const pathB = join(outDir, "chain-b.txt");
  let receiptA = "";
  let receiptB = "";
  try {
    receiptA = runHelper(binary, args, pathA);
    receiptB = runHelper(binary, args, pathB);
  } catch (error) {
    process.stderr.write(`fk1_determinism: ${error.message}\n`);
    process.exitCode = 1;
    return;
  }
  const bytesA = readFileSync(pathA);
  const bytesB = readFileSync(pathB);
  const linesA = bytesA.toString("utf8").split("\n").filter((line) => line.trim() !== "");
  const linesB = bytesB.toString("utf8").split("\n").filter((line) => line.trim() !== "");
  console.log(`fk1_determinism: seed=${args.seed} ticks=${args.ticks} fighters=${args.fighters} extent=${args.extent} profile=${args.profile}`);
  console.log(`fk1_determinism: run A rows=${linesA.length} final_hash=${finalHashOf(linesA)} (${receiptA})`);
  console.log(`fk1_determinism: run B rows=${linesB.length} final_hash=${finalHashOf(linesB)} (${receiptB})`);
  const at = firstDivergingTick(linesA, linesB);
  if (at === -1 && bytesA.equals(bytesB)) {
    console.log(`PASS fk1_determinism: ${linesA.length}/${linesB.length} checkpoint hashes byte-identical`);
    return;
  }
  const rowA = linesA[at] ?? "<eof>";
  const rowB = linesB[at] ?? "<eof>";
  console.log(`FAIL fk1_determinism: first divergence at tick ${at} (A ${rowA} / B ${rowB})`);
  process.exitCode = 1;
}

const here = dirname(fileURLToPath(import.meta.url));
const isMain = import.meta.main;
if (isMain) main(process.argv.slice(2), resolve(here, ".."));

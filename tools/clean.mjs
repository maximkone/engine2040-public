// Auto-clean for target/ bloat. Safe by default, never touches live state.
//   node tools/clean.mjs --dry-run    report only, change nothing
//   node tools/clean.mjs --auto       quarantine trash + unused pools (safe mid-sprint)
//   node tools/clean.mjs --deep --yes quarantine release cache too (needs no lanes)
// Trash (auto): target/tmp, target/debug-workspace-check, hunt/prov/j4/perf
// logs, old per-lane build dirs under target/lanes (keeps *.patch evidence),
// and pool-N dirs no lane in target/lanes-worktrees.json points at.
// Never touched: *.patch, lanes-worktrees.json, sprint.lock/halt/handoff,
// any pool listed in lanes-worktrees.json, target/debug (needs --include-debug).
//
// Nothing is ever hard-deleted (owner rule, AGENTS.md): a removed path is MOVED
// to C:\_quarantine\<date>\ and one MOVES.tsv line records the move. The bytes
// stay on the same volume, so the owner alone decides when the quarantine goes.
import { existsSync, mkdirSync, readdirSync, renameSync, statSync, readFileSync, appendFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join } from "node:path";

const ROOT = process.cwd();
const at = (p) => join(ROOT, p);
const args = process.argv.slice(2);
const dryRun = args.includes("--dry-run") || (!args.includes("--yes") && !args.includes("--auto"));
const auto = args.includes("--auto");
const deep = args.includes("--deep");
const includeDebug = args.includes("--include-debug");
if (args.some((a) => !["--dry-run", "--yes", "--auto", "--deep", "--include-debug"].includes(a))) {
  console.error("Usage: node tools/clean.mjs [--dry-run] [--auto] [--deep --yes] [--include-debug]");
  process.exit(2);
}

// The quarantine root. E2040_QUARANTINE_ROOT moves it for drills and tests;
// the default is the owner's C:\_quarantine, which this tool never deletes.
const QUARANTINE_ROOT = process.env.E2040_QUARANTINE_ROOT ?? "C:\\_quarantine";
const QUARANTINE_DATE = process.env.E2040_QUARANTINE_DATE ?? new Date().toISOString().slice(0, 10);
const quarantineDir = () => join(QUARANTINE_ROOT, QUARANTINE_DATE);
// One MOVES.tsv row per moved path: date, from, to, reason.
const movesFile = () => join(quarantineDir(), "MOVES.tsv");

// A name that is unique inside the quarantine day folder: the relative path
// flattened (target\debug -> target-debug), then a counter on a clash.
const quarantineName = (rel, taken) => {
  const flat = rel.replace(/[\\/]+/g, "-").replace(/[:*?"<>|]/g, "_");
  let name = flat;
  for (let n = 2; taken.has(name.toLowerCase()); n += 1) name = `${flat}-${n}`;
  taken.add(name.toLowerCase());
  return name;
};

// Move a path into the quarantine and record it. Never deletes.
const quarantineMove = (rel, reason) => {
  const from = at(rel);
  const destDir = quarantineDir();
  mkdirSync(destDir, { recursive: true });
  let taken = new Set();
  try { taken = new Set(readdirSync(destDir).map((f) => f.toLowerCase())); } catch (err) { console.warn(`WARN readdir ${destDir}: ${err?.message ?? err}`); }
  const to = join(destDir, quarantineName(rel, taken));
  try {
    renameSync(from, to);
  } catch (err) {
    // A move across volumes cannot be a rename: the owner rule says no hard
    // delete, so say so instead of copying bytes away and dropping the source.
    console.warn(`WARN move ${from}: ${err?.code ?? err?.message ?? err}`);
    throw new Error(`cannot move ${from} to ${to} (${err?.code ?? err?.message ?? err}); a cross-volume move needs the owner`);
  }
  appendFileSync(movesFile(), `${QUARANTINE_DATE}\t${from}\t${to}\t${reason}\n`, "utf8");
  return to;
};

// cargo or rustc alive means a build may be writing the tree: refuse to move.
// E2040_CLEAN_TASKLIST replaces the process list for drills.
export const buildsRunning = () => {
  const canned = process.env.E2040_CLEAN_TASKLIST;
  let out;
  if (canned !== undefined) {
    out = canned;
  } else {
    try {
      out = execFileSync(process.platform === "win32" ? "tasklist" : "ps", process.platform === "win32" ? ["/FO", "CSV", "/NH"] : ["-eo", "comm="], { encoding: "utf8" });
    } catch (err) {
      console.warn(`WARN process list: cannot read the running builds: ${err?.message ?? err}`);
      return -1; // unknown: fail honest, touch nothing
    }
  }
  const names = out.split("\n").map((l) => l.trim().replace(/^"/, "").split(",")[0].replace(/"/g, "").toLowerCase());
  return names.filter((n) => n === "cargo" || n === "cargo.exe" || n === "rustc" || n === "rustc.exe").length;
};

const du = (p) => {
  let bytes = 0;
  try {
    const st = statSync(p);
    if (!st.isDirectory()) return st.size;
    for (const e of readdirSync(p)) bytes += du(join(p, e));
  } catch (err) { console.warn(`WARN du: ${p}: ${err?.message ?? err}`); }
  return bytes;
};
const gb = (b) => `${(b / 1e9).toFixed(2)} GB`;

let usedPools = new Set();
try {
  const admin = JSON.parse(readFileSync(at(join("target", "lanes-worktrees.json")), "utf8"));
  for (const info of Object.values(admin.worktrees ?? {})) usedPools.add(`pool-${info.pool ?? 1}`);
} catch (err) { usedPools = null; console.warn(`WARN pools: lanes-worktrees.json unreadable, touch nothing pooled: ${err?.message ?? err}`); } // unknown: never touch pools

const trashDirs = [
  join("target", "tmp"),
  join("target", "debug-workspace-check"),
];
const trashLogs = ["canon-fail.log", "hang-hunt.log", "perf-out.log",
  ...readdirSafe("target").filter((f) => /^(hunt\d*|j4-\d|prov-hunt\d*)\.log$/.test(f))];
function readdirSafe(d) {
  try { return readdirSync(at(d)); } catch (err) { console.warn(`WARN readdir: ${d}: ${err?.message ?? err}`); return []; }
}

// Old per-lane build dirs under target/lanes (legacy layout). New code only
// writes target/lanes/*.patch, so directories here are orphaned caches.
const laneDirs = readdirSafe(join("target", "lanes"))
  .filter((f) => { try { return statSync(at(join("target", "lanes", f))).isDirectory(); } catch (err) { console.warn(`WARN stat: ${f}: ${err?.message ?? err}`); return false; } })
  .map((f) => join("target", "lanes", f));

// Unused pools: exist on disk but no registered lane points at them.
let unusedPools = [];
if (usedPools !== null) {
  unusedPools = [1, 2, 3, 4].map((k) => join("target", `pool-${k}`))
    .filter((d) => existsSync(at(d)) && !usedPools.has(d.split("\\").pop().split("/").pop()));
}

const deepTargets = [];
if (deep && usedPools !== null && [...usedPools].length === 0) {
  if (existsSync(at(join("target", "release")))) deepTargets.push(join("target", "release"));
}
const debugDir = join("target", "debug");
// target/debug only enters the plan with --include-debug; the report names it
// either way, so a dry run shows the big cache instead of hiding it.
const debugTarget = includeDebug ? [debugDir] : [];

const plan = [
  ...trashDirs.filter((d) => existsSync(at(d))).map((d) => ({ dir: d, kind: "trash" })),
  ...trashLogs.filter((f) => existsSync(at(join("target", f)))).map((f) => ({ dir: join("target", f), kind: "log" })),
  ...laneDirs.map((d) => ({ dir: d, kind: "old-lane" })),
  ...unusedPools.map((d) => ({ dir: d, kind: "unused-pool" })),
  ...deepTargets.map((d) => ({ dir: d, kind: "deep-release" })),
  ...debugTarget.filter((d) => existsSync(at(d))).map((d) => ({ dir: d, kind: "deep-debug" })),
];
// Report-only: the debug cache the owner wants capped. It moves only with
// --include-debug, so the report says so.
const reportOnly = !includeDebug && existsSync(at(debugDir)) ? [debugDir] : [];

if (deep && usedPools !== null && [...usedPools].length !== 0) {
  console.log(`SKIP release: ${[...usedPools].length} lane(s) registered, delete it by hand when idle`);
}
if (usedPools === null) console.log("SKIP pools: lanes-worktrees.json unreadable, fail honest, touch nothing pooled");

// A live build owns the tree: moving a build dir out from under it breaks the
// build. The row says "with no cargo running"; the tool checks and refuses.
if (!dryRun) {
  const busy = buildsRunning();
  if (busy !== 0) {
    console.error(`REFUSE: ${busy > 0 ? `${busy} cargo/rustc process(es) running` : "cannot tell whether a build runs"}; clean later`);
    process.exit(3);
  }
}

let total = 0;
let moved = 0;
for (const t of plan) {
  const size = du(at(t.dir));
  total += size;
  console.log(`${dryRun ? "WOULD" : t.kind} ${t.dir} ${gb(size)}`);
  if (dryRun) continue;
  try {
    const to = quarantineMove(t.dir, `clean.mjs ${t.kind}: ${t.dir} moved, not deleted (owner rule: never a hard delete)`);
    moved += 1;
    console.log(`MOVED ${t.dir} -> ${to}`);
  } catch (err) {
    console.warn(`WARN move ${t.dir}: ${err?.message ?? err}`);
    console.error(`FAIL move ${t.dir}: ${err?.message ?? err}`);
    process.exitCode = 1;
  }
}
for (const d of reportOnly) {
  console.log(`${dryRun ? "WOULD" : "KEEP"} ${d} ${gb(du(at(d)))} (needs --include-debug)`);
}
console.log(`${dryRun ? "WOULD reclaim" : "MOVED"} ${gb(total)} across ${plan.length} path(s), ${moved} moved to ${quarantineDir()}`);
if (!dryRun && moved > 0) console.log(`MOVES.tsv ${movesFile()}`);

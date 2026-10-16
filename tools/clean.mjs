// Auto-clean for target/ bloat. Safe by default, never touches live state.
//   node tools/clean.mjs --dry-run    report only, change nothing
//   node tools/clean.mjs --auto       delete trash + unused pools (safe mid-sprint)
//   node tools/clean.mjs --deep --yes delete release cache too (needs no lanes)
// Trash (auto): target/tmp, target/debug-workspace-check, hunt/prov/j4/perf
// logs, old per-lane build dirs under target/lanes (keeps *.patch evidence),
// and pool-N dirs no lane in target/lanes-worktrees.json points at.
// Never touched: *.patch, lanes-worktrees.json, sprint.lock/halt/handoff,
// any pool listed in lanes-worktrees.json, target/debug (needs --include-debug).
import { existsSync, readdirSync, rmSync, statSync, readFileSync } from "node:fs";
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
// --auto implies --yes for the safe subset only.
const mutate = auto || args.includes("--yes");

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
const debugTarget = includeDebug ? [join("target", "debug")] : [];

const plan = [
  ...trashDirs.filter((d) => existsSync(at(d))).map((d) => ({ dir: d, kind: "trash" })),
  ...trashLogs.filter((f) => existsSync(at(join("target", f)))).map((f) => ({ dir: join("target", f), kind: "log" })),
  ...laneDirs.map((d) => ({ dir: d, kind: "old-lane" })),
  ...unusedPools.map((d) => ({ dir: d, kind: "unused-pool" })),
  ...deepTargets.map((d) => ({ dir: d, kind: "deep-release" })),
  ...debugTarget.filter((d) => existsSync(at(d))).map((d) => ({ dir: d, kind: "deep-debug" })),
];

if (deep && usedPools !== null && [...usedPools].length !== 0) {
  console.log(`SKIP release: ${[...usedPools].length} lane(s) registered, delete it by hand when idle`);
}
if (usedPools === null) console.log("SKIP pools: lanes-worktrees.json unreadable, fail honest, touch nothing pooled");

let total = 0;
for (const t of plan) {
  const size = du(at(t.dir));
  total += size;
  console.log(`${dryRun ? "WOULD" : t.kind} ${t.dir} ${gb(size)}`);
  if (!dryRun) rmSync(at(t.dir), { recursive: true, force: true });
}
console.log(`${dryRun ? "WOULD reclaim" : "RECLAIMED"} ${gb(total)} across ${plan.length} path(s)`);

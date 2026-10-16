// Lead brief: farm state + dirty order + claims + queue rot. Fast, no cargo.
// Usage: node tools/lead-brief.mjs  (prints <3KB, called by sprint-loop.mjs)
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = join(import.meta.dirname ?? ".", "..");
const run = (cmd, args) => {
  try { return execFileSync(cmd, args, { cwd: ROOT, encoding: "utf8", timeout: 8000, stdio: ["ignore", "pipe", "pipe"] }).trim(); }
  catch { return ""; }
};
const minAgo = (ms) => Math.max(0, Math.round((Date.now() - ms) / 60000));
const mtime = (p) => { try { return statSync(p).mtimeMs; } catch { return null; } };

// 1. Farm: job lines + RESULT + first cause per red job + age vs HEAD (a stale farm is not evidence).
let farmHead = "farm: unknown (node tools/farm.mjs never ran)";
let farmJobs = [];
let farmCauses = [];
try {
  const p = join(ROOT, "target", "farm-last.txt");
  const all = readFileSync(p, "utf8").split("\n").filter(Boolean);
  farmHead = all[0] ?? farmHead;
  const age = mtime(p);
  const farmSha = (farmHead.match(/@([0-9a-f]{7,})/i) ?? [])[1];
  const head = run("git", ["rev-parse", "--short", "HEAD"]);
  if (farmSha && head && farmSha !== head) {
    const behind = run("git", ["rev-list", "--count", `${farmSha}..HEAD`]);
    farmHead += ` (${age ? minAgo(age) + " min old; " : ""}STALE: HEAD ${head}${behind ? ` is ${behind} commits ahead` : " moved"} - re-run node tools/farm.mjs before acting on red)`;
  } else if (age) farmHead += ` (${minAgo(age)} min old)`;
  const firstCause = all.findIndex((l) => l.startsWith("--- "));
  farmJobs = (firstCause === -1 ? all : all.slice(0, firstCause)).filter((l) => /^(PASS|FAILURE|RESULT)/.test(l));
  const result = [...all].reverse().find((l) => l.startsWith("RESULT"));
  if (result && !farmJobs.includes(result)) farmJobs.push(result);
  let cur = null;
  for (const l of all.slice(1)) {
    if (l.startsWith("--- ")) { cur = l.slice(4).trim(); continue; }
    if (cur) { farmCauses.push(`  ${cur}: ${l.slice(0, 140)}`); cur = null; }
  }
  farmCauses = farmCauses.slice(0, 6);
} catch { /* no farm yet */ }

// 2. Dirty tree: count + ordered paths (landing order = what to land/clean first).
const raw = run("git", ["status", "--short", "--untracked-files=all"]);
const dirty = raw ? raw.split("\n").filter(Boolean) : [];
const dirtyHead = dirty.slice(0, 15).join("\n");

// 3. Claims: the file size + the active holders (<2h), whole lines, never a mid-word slice.
let claims = "CLAIMS: none";
try {
  const lines = readFileSync(join(ROOT, "target", "queue", "claims.txt"), "utf8").trim().split("\n").filter(Boolean);
  const live = lines.filter((l) => { const m = l.match(/\|\s*(20\d\d-\d\d-\d\dT\d\d:\d\dZ)/); return m && Date.now() - Date.parse(m[1]) < 2 * 3_600_000; });
  claims = `CLAIMS: ${lines.length} lines, ${live.length} active (<2h)${live.length ? ":\n" + live.slice(-8).map((l) => l.slice(0, 200)).join("\n") : ""}`;
} catch { /* no claims */ }

// 4. Queue depth: ready per crate + the rot buckets the lead must see every wave.
let ready = "READY: 0 packet(s) in target/queue/ready/";
let rot = "ROT: ?";
try {
  const q = join(ROOT, "target", "queue");
  const dir = join(q, "ready");
  const files = readdirSync(dir);
  const by = {};
  let oldest = null;
  for (const f of files) {
    const t = mtime(join(dir, f));
    if (t && (oldest === null || t < oldest)) oldest = t;
    let crate = null;
    try { crate = (readFileSync(join(dir, f), "utf8").match(/^paths:\s*(.+)$/m)?.[1] ?? "").match(/crates\/([a-z0-9-]+)/)?.[1] ?? null; } catch { /* keep */ }
    const key = crate ?? "other";
    by[key] = (by[key] ?? 0) + 1;
  }
  const spread = Object.entries(by).map(([k, v]) => `${k} ${v}`).join(", ");
  ready = `READY: ${files.length} total${spread ? ` (${spread})` : ""} in target/queue/ready/`;
  const count = (d) => { try { return readdirSync(join(q, d)).length; } catch { return 0; } };
  const stateAge = mtime(join(ROOT, "target", "state.md"));
  rot = `ROT: held ${count("held")} parked ${count("parked")} failed ${count("failed")} backlog ${count("backlog")} | oldest ready ${oldest ? minAgo(oldest) + " min" : "-"} | state.md ${stateAge ? Math.round((Date.now() - stateAge) / 86_400_000) + " d stale" : "missing"}`;
} catch { /* no queue */ }

// 5. Receipt: the newest green landing age vs the 15 min rule; and 0-file runs today (waste).
let receipt = "RECEIPT: none";
try {
  const last = readFileSync(join(ROOT, "target", "queue", "landed.tsv"), "utf8").trim().split("\n").pop() ?? "";
  const m = last.match(/^(20\d\d-\d\d-\d\dT\d\d:\d\d:\d\dZ)/);
  if (m) receipt = `RECEIPT: newest landing ${minAgo(Date.parse(m[1]))} min ago (rule: 15)`;
} catch { /* no receipts */ }
let nofile = "";
try {
  const today = new Date().toISOString().slice(0, 10);
  const n = readFileSync(join(ROOT, "target", "loop-keeper.log"), "utf8").split("\n").filter((l) => l.includes("0 files changed") && l.startsWith(today)).length;
  nofile = `NOFILE: ${n} run(s) with 0 files changed today (split or park at the source)`;
} catch { /* no log */ }

// 6. Handoff age: a stale header lies; the mtime does not.
let handoff = "";
try { const t = mtime(join(ROOT, "target", "sprint-handoff.md")); if (t) handoff = `HANDOFF: rewritten ${minAgo(t)} min ago`; } catch { /* none */ }

console.log([
  `FARM: ${farmHead}`,
  ...farmJobs,
  ...farmCauses,
  `DIRTY: ${dirty.length} file(s)${dirty.length ? " (land oldest first):\n" + dirtyHead : " (tree clean)"}`,
  claims,
  rot,
  receipt,
  ...(nofile ? [nofile] : []),
  ...(handoff ? [handoff] : []),
  ready,
  `RULE: FARM-FIRST: while the farm is stale or RED, repair the farm before new features. Local cargo only for the edit loop: cargo check -p <crate> / cargo test -p <crate> --lib <one test>.`,
].join("\n"));

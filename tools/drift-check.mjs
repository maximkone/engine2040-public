// drift-check: keeps this repo's loop aligned with itself and with the empire
// (owner 2026-09-25). Read-only. The loop runs it at Start and every 5 passes;
// FAIL is the first packet of the next pass, WARN goes in the handoff.
//   node tools/drift-check.mjs            -> report, exit 1 on FAIL
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import { archivedDoneRows, loadBoardText, parseBoard } from "./board-slice.mjs";
// Center's meter (owner 2026-09-28: center's state folder moved out of AppData to ~/.empire/state;
// the old AppData copy is read only until the move has happened).
const centerMeter = () => {
  const moved = join(process.env.EMPIRE_STATE || join(homedir(), ".empire", "state"), "metrics.db");
  return existsSync(moved) ? moved : join(process.env.LOCALAPPDATA ?? "", "Empire", "metrics.db");
};

const EMPIRE_MJS = process.env.EMPIRE_MJS || ["C:", "empire", "center", "empire.mjs"].join("/");

const ROOT = process.cwd();
const CFG = {
 "command": ".opencode/commands/sprint.md",
 "agents": ".opencode/agents",
 "features": [
  [
   "empire vision read",
   "EMPIRE.md"
  ],
  [
   "empire inbox read",
   "EMPIRE-INBOX"
  ],
  [
   "re-read after compaction",
   "compaction"
  ],
  [
   "keeper stop line",
   "LOOP STOP:"
  ],
  [
   "lock session token",
   "(orchestrator|lead)#"
  ],
  [
   "standing seats",
   "Standing seats"
  ],
  [
   "the chain",
   "The chain"
  ],
  [
   "live foreground batches (owner 2026-09-28)",
   "packet: <name>"
  ],
  [
   "one-offs carry a card",
   "Goal, Scope, Proof and Stop"
  ],
  [
   "depth and rust scores",
   "DEPTH: n/5.*RUST: n/5"
  ],
  [
   "commit and push",
   "push every"
  ],
  [
   "retro every 5 waves",
   "retro"
  ],
  [
   "Muse eyes",
   "Muse sees images"
  ]
 ],
 "empire": "docs/EMPIRE.md",
 "inbox": "docs/EMPIRE-INBOX.md",
 "rotations": [],
 "files": [
  "docs/SPRINT.md",
  "docs/FINISH-RICH.md",
  "docs/DIRECTION-LOG.md"
 ],
 "health": [
  [
   "board valid (workspace-check)",
   [
    "node",
    "tools/workspace-check.mjs"
   ]
  ],
  [
   "rust quality ratchet",
   [
    "node",
    "tools/rust-quality.mjs"
   ]
  ]
 ],
 "metaPaths": [
  "docs",
  "skills",
  "tools"
 ],
 "showcase": "target/showcase",
 "duties": {"repo": "engine2040", "unit": "wave", "list": []}
};
const out = { PASS: 0, WARN: 0, FAIL: 0 };
const say = (level, msg) => {
  out[level]++;
  console.log(`[${level}] ${msg}`);
};
const read = (p) => (existsSync(p) ? readFileSync(p, "utf8").replace(/\r\n/g, "\n") : "");
const abs = (p) => (/^[A-Za-z]:[\\/]/.test(p) ? p : join(ROOT, p));
const run = (cmd, args) => {
  try {
    return { ok: true, out: execFileSync(cmd, args, { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 600_000 }) };
  } catch (e) {
    return { ok: false, out: `${e.stdout ?? ""}${e.stderr ?? ""}` };
  }
};
const daysOld = (iso) => (Date.now() - Date.parse(iso)) / 86_400_000;

// E47 speed-only: rust-quality result cache (verdict identical, child proc skipped
// on fingerprint hit) plus one shared metrics.db open for sections 10/13/14/15
// (was 4 opens of the ~300 MB file). No verdict logic lives here.
const QUALITY_CACHE = "target/drift-check.quality-cache.json";
const qualityWalk = (dir, out) => {
  let entries = [];
  try { entries = readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of entries) {
    if (e.name === "target" || e.name === "node_modules" || e.name === ".git" || e.name.startsWith(".")) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) qualityWalk(p, out);
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};
const qualityFingerprint = () => {
  const fps = [];
  const statOne = (p) => {
    try { const st = statSync(p); fps.push(`${p}:${st.mtimeMs}:${st.size}`); }
    catch { fps.push(`${p}:missing`); }
  };
  for (const f of qualityWalk(join(ROOT, "crates"), [])) statOne(f);
  for (const d of ["src", "tests", "examples", "benches"])
    for (const f of qualityWalk(join(ROOT, d), [])) statOne(f);
  statOne(join(ROOT, "tools", "rust-quality.mjs"));
  statOne(join(ROOT, "tools", "rust-quality.baseline.json"));
  statOne(join(ROOT, "tools", "sim-clippy.toml"));
  statOne(join(ROOT, "docs", "SPRINT.md"));
  try {
    for (const n of readdirSync(join(ROOT, "docs")).filter((n) => n.startsWith("SPRINT-") && n.endsWith(".md"))) statOne(join(ROOT, "docs", n));
  } catch (e) { say("WARN", `quality fingerprint: cannot list docs/SPRINT-*.md (${e instanceof Error ? e.message : String(e)})`); }
  try {
    for (const d of readdirSync(join(ROOT, "crates"))) statOne(join(ROOT, "crates", d, "clippy.toml"));
  } catch (e) { say("WARN", `quality fingerprint: cannot list crates/*/clippy.toml (${e instanceof Error ? e.message : String(e)})`); }
  fps.sort();
  return createHash("sha256").update(fps.join("\n")).digest("hex");
};
// One walk per run: a cache miss reads then writes, so share one fingerprint.
let __qualityFp = null;
const qualityFpOnce = () => (__qualityFp ??= qualityFingerprint());
const readQualityCache = () => {
  try {
    const raw = readFileSync(join(ROOT, QUALITY_CACHE), "utf8");
    const cached = JSON.parse(raw);
    if (cached && cached.fp === qualityFpOnce() && typeof cached.ok === "boolean" && typeof (cached.tail ?? "") === "string")
      return { ok: cached.ok, tail: cached.tail ?? "" };
  } catch (e) { say("WARN", `quality cache unreadable (${QUALITY_CACHE}: ${e instanceof Error ? e.message : String(e)}); recomputing`); }
  return null;
};
const writeQualityCache = (ok, tail) => {
  try {
    writeFileSync(join(ROOT, QUALITY_CACHE), JSON.stringify({ fp: qualityFpOnce(), ok, tail }));
  } catch (e) { say("WARN", `quality cache unwritable (${QUALITY_CACHE}: ${e instanceof Error ? e.message : String(e)})`); }
};
const WORKSPACE_CACHE = "target/drift-check.workspace-cache.json";
const workspaceFingerprint = () => {
  const fps = [];
  const s = (p) => {
    try { const st = statSync(p); fps.push(`${p}:${st.mtimeMs}:${st.size}`); }
    catch { fps.push(`${p}:missing`); }
  };
  // All .rs inputs workspace-check audits (module reachability, determinism,
  // genre ledger, baselines gate, test layout): same walk as qualityFingerprint.
  for (const f of qualityWalk(join(ROOT, "crates"), [])) s(f);
  s(join(ROOT, "tools", "workspace-check.mjs"));
  s(join(ROOT, "tools", "rust-quality.mjs"));
  s(join(ROOT, "tools", "board-slice.mjs"));
  s(join(ROOT, "tools", "board-memo.mjs"));
  s(join(ROOT, "tools", "research-cap.mjs"));
  s(join(ROOT, "tools", "presence.mjs"));
  s(join(ROOT, "tools", "sim-source-scan.mjs"));
  s(join(ROOT, "tools", "sim-clippy.toml"));
  s(join(ROOT, "Cargo.toml"));
  s(join(ROOT, "Cargo.lock"));
  s(join(ROOT, "rust-toolchain.toml"));
  s(join(ROOT, ".cargo", "config.toml"));
  s(join(ROOT, ".gitattributes"));
  s(join(ROOT, "docs", "SPRINT.md"));
  try { for (const n of readdirSync(join(ROOT, "docs")).filter((n) => n.startsWith("SPRINT-") && n.endsWith(".md"))) s(join(ROOT, "docs", n)); } catch (e) { say("WARN", `workspace fingerprint: cannot list docs/SPRINT-*.md (${e instanceof Error ? e.message : String(e)})`); }
  try { for (const n of readdirSync(join(ROOT, "docs", "archive")).filter((n) => n.startsWith("SPRINT-done-") && n.endsWith(".md"))) s(join(ROOT, "docs", "archive", n)); } catch (e) { say("WARN", `workspace fingerprint: cannot list docs/archive/SPRINT-done-*.md (${e instanceof Error ? e.message : String(e)})`); }
  try { for (const n of readdirSync(join(ROOT, "docs", "research", "cards")).filter((n) => n.endsWith(".md"))) s(join(ROOT, "docs", "research", "cards", n)); } catch (e) { say("WARN", `workspace fingerprint: cannot list docs/research/cards/*.md (${e instanceof Error ? e.message : String(e)})`); }
  try { for (const d of readdirSync(join(ROOT, "crates"))) s(join(ROOT, "crates", d, "Cargo.toml")); } catch (e) { say("WARN", `workspace fingerprint: cannot list crates/*/Cargo.toml (${e instanceof Error ? e.message : String(e)})`); }
  // LFS guard key: staged set is a function of the index plus HEAD.
  s(join(ROOT, ".git", "index"));
  s(join(ROOT, ".git", "HEAD"));
  s(join(ROOT, ".git", "packed-refs"));
  try {
    const head = readFileSync(join(ROOT, ".git", "HEAD"), "utf8").trim();
    const ref = /^ref:\s*(.+)$/.exec(head)?.[1];
    if (ref) s(join(ROOT, ".git", ...ref.split("/")));
  } catch (e) { say("WARN", `workspace fingerprint: cannot read git HEAD ref (${e instanceof Error ? e.message : String(e)})`); }
  fps.sort();
  return createHash("sha256").update(fps.join("\n")).digest("hex");
};
// One walk per run: a cache miss reads then writes, so share one fingerprint.
let __workspaceFp = null;
const workspaceFpOnce = () => (__workspaceFp ??= workspaceFingerprint());
const readWorkspaceCache = () => {
  try {
    const c = JSON.parse(readFileSync(join(ROOT, WORKSPACE_CACHE), "utf8"));
    if (c && c.fp === workspaceFpOnce() && typeof c.ok === "boolean" && typeof (c.tail ?? "") === "string") return { ok: c.ok, tail: c.tail ?? "" };
  } catch { return null; }
  return null;
};
const writeWorkspaceCache = (ok, tail) => {
  try { writeFileSync(join(ROOT, WORKSPACE_CACHE), JSON.stringify({ fp: workspaceFpOnce(), ok, tail })); } catch (e) { say("WARN", `workspace cache unwritable (${WORKSPACE_CACHE}: ${e instanceof Error ? e.message : String(e)})`); }
};
// Single shared read-only handle for the whole run; sections keep their own
// messages, only the open/close is shared.
let __meterDb = null;
let __meterPath = "";
let __meterOpened = false;
const meterOpenOnce = async () => {
  if (__meterOpened) return { db: __meterDb, meter: __meterPath };
  __meterOpened = true;
  __meterPath = centerMeter();
  try {
    process.removeAllListeners("warning");
    const { DatabaseSync } = await import("node:sqlite");
    if (existsSync(__meterPath)) __meterDb = new DatabaseSync(__meterPath, { readOnly: true });
  } catch { __meterDb = null; }
  return { db: __meterDb, meter: __meterPath };
};
const meterCloseOnce = () => {
  try { __meterDb?.close?.(); } catch (e) { say("WARN", `meter close failed (${e instanceof Error ? e.message : String(e)})`); }
  __meterDb = null;
};

// Pure helpers (exported for node --test; sections 6/11/12 below call them,
// behavior unchanged). AW3: shell-junk names; §12: GPU-row excuse lines;
// §6: open inbox dates.
// §13's families come from docs/FOCUS.md "## Now" itself (owner 2026-10-11: the hard-coded list named rows of an older
// FOCUS (DT5, VS, OPT1-OPT13), matched none of the EG rows the Now list holds, and demanded a multiplayer row every 6 h
// though the owner put multiplayer last; center turned its FAILs into packets that sent coders off the board).
// One family per numbered item `N. **Name:** ... Rows: ID ID ...`; an item whose name says "last" only warns.
export const focusFamilies = (focus) => {
  const now = String(focus ?? "").replace(/\r\n/g, "\n").split(/\n(?=## )/).find((s) => /^## Now\b/.test(s)) ?? "";
  const out = [];
  for (const line of now.split("\n")) {
    const m = line.match(/^\s*\d+\.\s+\*\*([^*]+?)\*\*.*?\bRows:\s*(.+)$/);
    if (!m) continue;
    const ids = [...new Set(m[2].match(/\b[A-Z]{1,4}\d+[a-z]?\b/g) ?? [])];
    if (!ids.length) continue;
    const name = m[1].replace(/\s*\(.*$/, "").replace(/:\s*$/, "").trim().toLowerCase();
    out.push({ name, ids, last: /\blast\b/i.test(m[1]), re: new RegExp(`\\b(?:${ids.join("|")})\\b`) });
  }
  return out;
};

export const isShellJunk = (n) => n.startsWith("=") || /^chk-.*\.out$/.test(n);
export const shellJunk = (names) => [...names].filter(isShellJunk);
export const DRIFT_ROW_RE = /\b(?:GX\d+|RL\d+|RT\d+|VG\d+|VX\d+|Q1|PQ\d+|GM3)\b/;
export const DRIFT_EXCUSE_SOURCE = "GPU[- ]gated|operator[- ]side|operator[- ]only|needs? (?:a|the) GPU|no GPU";
export const DRIFT_NEG_RE = /\b(?:no\s+row\s+is|not|never|no\s+longer|is\s+not|are\s+not|n't)\b/i;
export const isExcuseLine = (line) => {
  if (!DRIFT_ROW_RE.test(line)) return false;
  for (const m of line.matchAll(new RegExp(DRIFT_EXCUSE_SOURCE, "gi"))) {
    const before = line.slice(Math.max(0, (m.index ?? 0) - 40), m.index ?? 0);
    if (!DRIFT_NEG_RE.test(before)) return true;
  }
  return false;
};
export const excuseLines = (text) => String(text ?? "").split("\n").filter(isExcuseLine);
export const parseInboxOpen = (inbox) =>
  [...String(inbox ?? "").matchAll(/^- \[ \] EB-(\d{4}-\d\d-\d\d)/gm)].map((m) => m[1]);

export const parseKeeperState = (text) => {
  if (!text || !String(text).trim()) return { state: null, error: null };
  try {
    return { state: JSON.parse(String(text)), error: null };
  } catch (e) {
    return { state: null, error: e instanceof Error ? e.message : String(e) };
  }
};

// Idea from arXiv 2402.05223, reimplemented fresh (no bytes copied): adapt a
// static stuck-row window from observed done-record durations with a small
// median-times-k estimator, clamped so a thin sample keeps the static value.
export const STUCK_STATIC_WINDOW_MS = 72 * 3_600_000;
export const STUCK_STATIC_THINKER_MS = 48 * 3_600_000;
export const STUCK_TUNED_MIN_MS = 4 * 3_600_000;
const finiteDurations = (xs) => [...xs ?? []].filter((n) => Number.isFinite(n) && n > 0 && n <= 30 * 24 * 3_600_000);
export const medianMs = (xs) => {
  const vs = finiteDurations(xs).sort((a, b) => a - b);
  if (!vs.length) return 0;
  const mid = Math.floor(vs.length / 2);
  return vs.length % 2 ? vs[mid] : (vs[mid - 1] + vs[mid]) / 2;
};
export const tuneStuckTimeout = (durationsMs, baseMs = STUCK_STATIC_WINDOW_MS) => {
  const base = Number.isFinite(baseMs) && baseMs > 0 ? baseMs : STUCK_STATIC_WINDOW_MS;
  const vs = finiteDurations(durationsMs);
  if (vs.length < 3) return base;
  const tuned = medianMs(vs) * 3;
  return Math.min(base * 2, Math.max(STUCK_TUNED_MIN_MS, tuned));
};
export const rowTimeoutGuidance = (rowId, durationsMs, baseMs = STUCK_STATIC_WINDOW_MS) => {
  const vs = finiteDurations(durationsMs);
  const base = Number.isFinite(baseMs) && baseMs > 0 ? baseMs : STUCK_STATIC_WINDOW_MS;
  if (vs.length < 3) return rowId + ": keep " + Math.round(base / 3_600_000) + "h (n=" + vs.length + ", thin sample)";
  const tuned = tuneStuckTimeout(vs, base);
  return rowId + ": tune " + Math.round(base / 3_600_000) + "h -> " + Math.round(tuned / 3_600_000) + "h (n=" + vs.length + ", median " + Math.round(medianMs(vs) / 3_600_000) + "h)";
};
// 1. The loop command and its required features.
async function main() {
const cmd = read(abs(CFG.command));
if (!cmd) say("FAIL", `loop command missing: ${CFG.command}`);
for (const [feature, rx] of CFG.features) {
  if (new RegExp(rx, "i").test(cmd.replace(/\s+/g, " "))) say("PASS", `command has: ${feature}`);
  else say("FAIL", `command lost: ${feature} (/${rx}/)`);
}

// 2. loop-keeper still recognizes this command.
const keeper = read(abs(".opencode/plugin/loop-keeper.js"));
const marker = keeper.match(/marker:\s*("(?:[^"\\]|\\.)*")/);
if (!keeper) say("FAIL", "loop-keeper plugin missing: .opencode/plugin/loop-keeper.js");
else if (!marker) say("FAIL", "loop-keeper has no marker");
else if (!cmd.includes(JSON.parse(marker[1]))) say("FAIL", `loop-keeper marker no longer in the command: ${marker[1]}`);
else say("PASS", "loop-keeper marker matches the command");
// OpenCode starts every exported function as its own plugin: two exports = two keepers, double continues.
const keeperExports = (keeper.match(/^export /gm) ?? []).length;
if (keeper && keeperExports !== 1) say("FAIL", `loop-keeper exports ${keeperExports} things; export only LoopKeeper`);
else if (keeper) say("PASS", "loop-keeper has one export");
// The three repos run one keeper program; only its CFG block differs.
const KEEPERS = ["C:/engine2040/.opencode/plugin/loop-keeper.js", "C:/forge/.opencode/plugin/loop-keeper.js", join(EMPIRE_MJS, "..", "..", "autonomous-factory", ".opencode", "plugin", "loop-keeper.js")];
const keeperBody = (t) => t.split("// ---- identical below this line in every repo ----")[1] ?? "";
const keeperDrift = KEEPERS.filter((p) => existsSync(p) && keeperBody(read(p)) !== keeperBody(keeper));
if (keeper && !keeperBody(keeper)) say("WARN", "loop-keeper lost its shared-body marker line");
else if (keeperDrift.length) say("WARN", `loop-keeper body differs from ${keeperDrift.join(", ")}: keep all three identical (center: node --test loop-keeper.test.mjs)`);
else if (keeper) say("PASS", "loop-keeper body matches the other two repos");
// The keeper's last decision. A stop the loop caused itself (no tools, no handoff
// update, errors) is retro input; owner stops, LOOP STOP and halts are not.
let keeperRaw = read("C:/engine2040/target/loop-keeper.json");
let keeperParsed = parseKeeperState(keeperRaw);
let keeperState = keeperParsed.state;
if (keeperParsed.error) say("WARN", `loop-keeper state corrupt: ${keeperParsed.error} (delete or fix target/loop-keeper.json, then run /sprint)`);
else if (!keeperState) say("WARN", "loop-keeper has no decision yet (restart OpenCode to load it, then run /sprint)");
else if (keeperState.decision === "stopped" && !/owner|LOOP STOP|one-round|halted|stale lock/i.test(keeperState.reason ?? "")) {
  say("WARN", `loop-keeper stopped the loop itself at ${keeperState.at}: ${keeperState.reason} (retro input: find and fix why)`);
} else say("PASS", `loop-keeper last decision: ${keeperState.decision} at ${keeperState.at}`);

// 2b. Files the loop depends on.
for (const f of CFG.files ?? []) {
  if (existsSync(abs(f))) say("PASS", `exists: ${f}`);
  else say("FAIL", `missing: ${f}`);
}

// 3. Agents: frontmatter, no flat bash deny on Zen models, twins in sync.
const adir = abs(CFG.agents);
const agents = existsSync(adir) ? readdirSync(adir).filter((f) => f.endsWith(".md") && !f.startsWith("_")) : [];
for (const f of agents) {
  const t = read(join(adir, f));
  const fm = t.startsWith("---") ? t.split("---")[1] : "";
  if (!/^description:/m.test(fm)) say("FAIL", `agent ${f}: no description`);
  if (/^model:\s*zen-proxy/m.test(fm) && /^\s{2}bash:\s*deny\s*$/m.test(fm)) say("FAIL", `agent ${f}: flat "bash: deny" on a Zen model (free tier rejects it)`);
}
say("PASS", `${agents.length} agent files read`);
if (CFG.twins) {
  const r = run("node", CFG.twins);
  if (!r.ok || !/RESULT 0 twin change/.test(r.out)) say("FAIL", `paid twins drifted: run node ${CFG.twins[0]}`);
  else say("PASS", "paid twins in sync");
}

// 4. EMPIRE copy equals center (center wins).
const hash = (p) => createHash("sha256").update(read(p)).digest("hex");
const center = join(EMPIRE_MJS, "..", "EMPIRE.md");
if (!existsSync(abs(CFG.empire))) say("FAIL", `EMPIRE copy missing: ${CFG.empire}`);
else if (existsSync(center) && hash(center) !== hash(abs(CFG.empire))) say("FAIL", `${CFG.empire} differs from center/EMPIRE.md: copy it over`);
else say("PASS", "EMPIRE copy matches center");

// 5. Research rotations are moving.
for (const f of CFG.rotations) {
  const t = read(abs(f));
  if (!t) { say("FAIL", `rotation file missing: ${f}`); continue; }
  const dates = [...t.matchAll(/\|\s*(20\d\d-\d\d-\d\d)\s*\|/g)].map((m) => m[1]).sort();
  const rows = (t.match(/^\|\s*\d+\s*\|/gm) ?? []).length;
  if (!dates.length) say("WARN", `${f}: 0 of ${rows} served yet`);
  else if (daysOld(dates[dates.length - 1]) > 3) say("WARN", `${f}: nothing served since ${dates[dates.length - 1]} (rotation stalled)`);
  else say("PASS", `${f}: ${dates.length} served entries, latest ${dates[dates.length - 1]}`);
}

// 6. Empire inbox: nothing open for long.
const inbox = read(abs(CFG.inbox));
if (!inbox) say("FAIL", `empire inbox missing: ${CFG.inbox}`);
else {
  const open = parseInboxOpen(inbox);
  const old = open.filter((d) => daysOld(d) > 2);
  if (old.length) say("WARN", `empire inbox: ${old.length} item(s) open over 2 days`);
  else say("PASS", `empire inbox: ${open.length} open, none stale`);
}

// 7. Repo-specific health commands (E47: rust-quality child proc cached by
// fingerprint; cache hit replays the same PASS/FAIL verdict with no proc).
// M slow-path: --skip-health gates the child procs (workspace-check,
// rust-quality); --help prints the flag and also skips the procs.
const __driftArgs = new Set(process.argv.slice(2));
if (__driftArgs.has("--help") || __driftArgs.has("-h")) {
  console.log("Usage: node tools/drift-check.mjs [--skip-health] [--help]");
  console.log("  --skip-health  skip section 7 health child procs (workspace-check, rust-quality; saves ~5s)");
  console.log("  --help, -h     print this help, exit 0");
}
if (__driftArgs.has("--skip-health") || __driftArgs.has("--help") || __driftArgs.has("-h")) {
  say("PASS", "health skipped via --skip-health (workspace-check, rust-quality)");
} else for (const [name, args] of CFG.health) {
  if (args.slice(1).join(" ").includes("rust-quality")) {
    const cached = readQualityCache();
    if (cached) {
      if (cached.ok) say("PASS", `${name}`);
      else say("FAIL", `${name} failed: ${cached.tail}`);
      continue;
    }
    const r = run(args[0], args.slice(1));
    const tail = r.ok ? "" : r.out.split("\n").filter(Boolean).slice(-2).join(" | ").slice(0, 200);
    writeQualityCache(r.ok, tail);
    if (r.ok) say("PASS", `${name}`);
    else say("FAIL", `${name} failed: ${tail}`);
    continue;
  }
  if (args.slice(1).join(" ").includes("workspace-check")) {
    const cached = readWorkspaceCache();
    if (cached) {
      if (cached.ok) say("PASS", `${name}`);
      else say("FAIL", `${name} failed: ${cached.tail}`);
      continue;
    }
    const r = run(args[0], args.slice(1));
    const tail = r.ok ? "" : r.out.split("\n").filter(Boolean).slice(-2).join(" | ").slice(0, 200);
    writeWorkspaceCache(r.ok, tail);
    if (r.ok) say("PASS", `${name}`);
    else say("FAIL", `${name} failed: ${tail}`);
    continue;
  }
  const r = run(args[0], args.slice(1));
  if (r.ok) say("PASS", `${name}`);
  else say("FAIL", `${name} failed: ${r.out.split("\n").filter(Boolean).slice(-2).join(" | ").slice(0, 200)}`);
}

// 8. Git: pushed, and nothing meta left uncommitted for a long time.
const ahead = run("git", ["rev-list", "--count", "@{u}..HEAD"]);
if (!ahead.ok) say("WARN", "git: no upstream (push not set up)");
else if (Number(ahead.out.trim()) > 0) say("WARN", `git: ${ahead.out.trim()} commit(s) not pushed`);
else say("PASS", "git: everything committed is pushed");
const dirtyRun = run("git", ["status", "--porcelain", "--", ".opencode", ...CFG.metaPaths]);
if (!dirtyRun.ok) say("FAIL", `git status failed: ${dirtyRun.out.split("\n").filter(Boolean).slice(-2).join(" | ").slice(0, 200)}`);
else {
  const dirty = dirtyRun.out.split("\n").filter(Boolean);
  if (dirty.length) say("WARN", `git: ${dirty.length} uncommitted meta file(s) (${dirty.slice(0, 3).map((l) => l.slice(3)).join(", ")}${dirty.length > 3 ? ", ..." : ""})`);
  else say("PASS", "git: no uncommitted meta files");
}
// A board commit carries board files only (center 2026-09-28: 4542c31 "Board: ..." carried three
// crates/net files a helper had staged; they skipped the merger's tests and ratchet and broke it).
{
  const log = run("git", ["log", "--since=6 hours ago", "--no-merges", "--name-only", "--format=@@%h %s"]);
  if (!log.ok) say("FAIL", `board log unreadable: ${log.out.split("\n").filter(Boolean).slice(-2).join(" | ").slice(0, 200)}`);
  else {
  const carried = log.out.split("@@").filter(Boolean).map((c) => { const [head, ...files] = c.trim().split("\n"); return { head, code: files.filter((f) => /^(?:crates|tools|content)\//.test(f.trim())) }; })
    .filter((c) => /^\S+ Board:/i.test(c.head) && c.code.length);
  if (carried.length) say("FAIL", `board commit carried code: ${carried.slice(0, 2).map((c) => `${c.head.slice(0, 40)} (${c.code.slice(0, 3).join(", ")})`).join("; ")}: stage board files by path (git commit -- docs/SPRINT.md ...); code lands through the merger`);
  else say("PASS", "board commits carry board files only (6 h)");
  }
}

// 9. Showcase freshness (for the factory's gold programs).
if (CFG.showcase) {
  const d = abs(CFG.showcase);
  // Newest set by folder time, any name (sets were named wave22, not by date, so
  // this check skipped them). The duty is every 5 waves: a day without one is a FAIL
  // (center 2026-09-26: last set wave 22 at wave 41, the factory's G2 had nothing).
  const sets = existsSync(d) ? readdirSync(d).flatMap((n) => {
    try { const s = statSync(join(d, n)); return s.isDirectory() ? [{ n, t: s.mtimeMs }] : []; } catch { return []; }
  }).sort((a, b) => a.t - b.t) : [];
  const newest = sets[sets.length - 1];
  if (!newest) say("FAIL", `showcase: no capture set yet in ${CFG.showcase}`);
  else if ((Date.now() - newest.t) / 3_600_000 > 24) say("FAIL", `showcase: newest set ${newest.n} is ${Math.round((Date.now() - newest.t) / 3_600_000)} h old (duty: every 5 waves)`);
  else say("PASS", `showcase: newest set ${newest.n}`);
}

// 10. Duties with a cadence: the role that owns each one ran recently. Center found on
// 2026-09-26 that the engine playtester had not run in 48 h and the forge planner and
// benchmark-engineer once. Last runs come from center's meter (~/.empire/state/metrics.db,
// refreshed every 20 min); a duty is not checked while the loop itself is not running.
if (CFG.duties?.list?.length) {
  const { db, meter } = await meterOpenOnce();
  if (!db) say("WARN", `duties: center's meter not found (${meter}); run node ${EMPIRE_MJS} import`);
  else {
    try {
      const runs = db.prepare("select agent, max(created) t from sessions where repo=? and parent is not null group by agent").all(CFG.duties.repo);
      // A duty is done by its role or by any helper whose packet title names the work (center
      // 2026-09-27: "Wild reader 51" ran on the researcher while the scout-only duty FAILed, and
      // the role merges of knowledge/TEAMS.md move duties to merged roles).
      const titled = db.prepare("select title, created from sessions where repo=? and parent is not null and created>?").all(CFG.duties.repo, Date.now() - 48 * 3_600_000);
      for (const [roles, hours, what, titles] of CFG.duties.list) {
        const since = Date.now() - hours * 3_600_000;
        const helpers = db.prepare("select count(*) n from sessions where repo=? and parent is not null and created>?").get(CFG.duties.repo, since).n;
        if (helpers < 10) { say("PASS", `duty ${what}: the loop did not run in the last ${hours} h, not checked`); continue; }
        const re = new RegExp(`^(?:${roles})$`);
        const tre = titles ? new RegExp(titles, "i") : null;
        const t = Math.max(0, ...runs.filter((r) => re.test(String(r.agent).replace(/-paid$/, ""))).map((r) => r.t),
          ...(tre ? titled.filter((r) => tre.test(String(r.title ?? ""))).map((r) => r.created) : []));
        const h = Math.round((Date.now() - t) / 3_600_000);
        if (t > since) say("PASS", `duty ${what}: done ${h} h ago`);
        else say("FAIL", `duty ${what}: not done for ${t ? `${h} h` : "the meter's whole record"} (limit ${hours} h; ${roles}${titles ? ` or a packet titled /${titles}/` : ""}): dispatch it this ${CFG.duties.unit}`);
      }
    } catch (e) { say("WARN", `duties: meter unreadable (${e.message})`); }
  }
}

// 11. AW3 shell-junk guard: pwsh parses `>=` as a redirect, so hand-typed
// search queries once sprayed `=` and chk-*.out droppings across the root.
// Any such file FAILs with its name until removed; queries go through
// tools/gh-search.mjs and tools/web-search.mjs (argument arrays, no shell).
{
  let rootNames = null;
  try { rootNames = readdirSync(ROOT); } catch (e) { say("FAIL", `shell junk: cannot list root (${e instanceof Error ? e.message : String(e)})`); }
  if (rootNames) {
  const junk = shellJunk(rootNames);
  if (junk.length) say("FAIL", `shell junk at root: ${junk.slice(0, 5).join(", ")}${junk.length > 5 ? ", ..." : ""} (AW3: remove the files; search via tools/gh-search.mjs and tools/web-search.mjs)`);
  else say("PASS", "no shell junk (=*, chk-*.out) at root");
  }
}

// 12. No excuse words for work this machine can do (center 2026-09-27). Owner decision O5:
// the reference GPU is this laptop's RTX 3050, and GM3 is agent-proven. The handoff still
// called GX3/GX5/Q1/RL5 "GPU-gated, operator-side" for 20+ waves while A4 and C1 stayed open.
{
  const handoff = read(abs("target/sprint-handoff.md"));
  // Explicit denials ("No row is operator-side", "not/never operator-side") are not excuses: an
  // excuse phrase counts only when the ~40 chars before it carry no negation word. The "no" inside
  // a "no GPU" match sits inside the match, so that excuse still FAILs.
  const bad = excuseLines(handoff);
  if (bad.length) say("FAIL", `excuse: the handoff calls GPU rows operator work (${bad.length} line(s), first: ${bad[0].trim().slice(0, 100)}); O5: this machine is the reference GPU: the look seats and the pilot take them`);
  else say("PASS", "excuse: no GPU row is called operator work");
}

// 13. FOCUS Now gets the writers (center 2026-09-27). `docs/FOCUS.md` says the look, AI spatial
// understanding and speed take the first writers of every wave; in the 12 h to 11:30Z the loop
// sent 137 helpers and 2 touched a look row, 0 a spatial row, 3 a speed row (the role duty above
// passed on any pilot run). Counted by the rows each helper's title names, from center's meter; since the foreground batches (2026-09-28)
{
  const { db, meter } = await meterOpenOnce();
  // The families and their rows are docs/FOCUS.md "## Now" itself (focusFamilies above); a helper counts for a family when
  // its title names one of that family's row IDs (coder packets and their reviews are titled `<ID> ...`).
  const FOCUS = focusFamilies(read(abs("docs/FOCUS.md")));
  if (!FOCUS.length) say("WARN", "focus: docs/FOCUS.md \"## Now\" has no `N. **Name:** ... Rows: ...` items, not checked");
  else if (db) {
    try {
      const since = Date.now() - 6 * 3_600_000;
      const titles = db.prepare("select title from sessions where repo='engine2040' and parent is not null and created>?").all(since).map((r) => String(r.title ?? ""));
      if (titles.length < 10) say("WARN", `focus: the loop sent ${titles.length} helpers in 6 h, not checked`);
      else for (const { name, ids, last, re } of FOCUS) {
        const n = titles.filter((t) => re.test(t)).length;
        if (n) say("PASS", `focus ${name}: ${n} of ${titles.length} helpers in 6 h`);
        else say(last ? "WARN" : "FAIL", `focus ${name}: 0 of ${titles.length} helpers in 6 h named a ${name} row (FOCUS Now${last ? ", last by the owner" : ""}): the next coder packets take its open rows in order: ${ids.slice(0, 8).join(" ")}`);
      }
    } catch (e) { say("FAIL", `focus: meter unreadable (${e.message})`); }
  } else say("WARN", `focus: center's meter not found (${meter}); run node ${EMPIRE_MJS} import`);
}

// 14. Every board section moves (center 2026-09-27, owner: "sprints cover all repo evolution").
// The board is read ONCE here and shared with §15 below (was two identical
// 460+ KB reads); §15's open-row set comes from the shared parser.
const boardText = loadBoardText(ROOT).replace(/\r\n/g, "\n");
// In 24 h to 17:00Z the loop's 328 helpers touched 5 of 12 sections; Hard stuff, 2040 tier,
// Engine edge, Agent workspace, Platform and live AI, Urban war and Code quality (238 open rows)
// got none while 14 of 20 helper places stood empty. The order of the sections still decides the
// first writers; the spare width goes to a section with open rows and no helper in 12 h, one
// packet each. Counted by the row IDs in helper titles, from center's meter.
{
  const PARKED_SECTIONS = /^(?:Platform and live AI)$/i; // Urban war is NOT parked (Maxim 2026-09-30: "no")
  const { db, meter } = await meterOpenOnce();
  const board = boardText;
  const from = board.indexOf("### Finish line"), to = board.indexOf("## Parked");
  const sections = [];
  if (from >= 0 && to > from) {
    for (const part of board.slice(from, to).split(/^### /m).slice(1)) {
      const name = part.split("\n", 1)[0].replace(/\s*\(.*$/, "").trim();
      if (/^(?:Operator|Done)$/i.test(name)) continue;
      // Parked by the owner's focus (2026-09-30: no audio, netcode, UI, modding or platform work unless a bar needs it): a section that
      // is parked is not owed a packet from the spare width. Its rows stay on the board, never unticked.
      if (PARKED_SECTIONS.test(name)) continue;
      const rows = [...part.matchAll(/^- \[( |x)\] ([A-Z][A-Z0-9]*\d+[A-Za-z0-9]*)/gm)].map((m) => ({ open: m[1] === " ", id: m[2] }));
      const open = rows.filter((r) => r.open);
      if (open.length) sections.push({ name, open, ids: rows.map((r) => r.id) });
    }
  }
  if (db && sections.length) {
    try {
      const titles = db.prepare("select title from sessions where repo='engine2040' and parent is not null and created>?").all(Date.now() - 12 * 3_600_000).map((r) => String(r.title ?? ""));
      if (titles.length < 20) say("PASS", `coverage: the loop sent ${titles.length} helpers in 12 h, not checked`);
      else {
        const text = titles.join("\n");
        const starved = sections.filter((s) => !s.ids.some((id) => new RegExp(`\\b${id}\\b`).test(text)));
        if (!starved.length) say("PASS", `coverage: all ${sections.length} board sections with open rows got a helper in 12 h`);
        for (const s of starved) say("WARN", `coverage ${s.name}: ${s.open.length} open rows, 0 of ${titles.length} helpers in 12 h: one packet from the spare width this wave (first open: ${s.open.slice(0, 3).map((r) => r.id).join(", ")})`);
      }
    } catch (e) { say("WARN", `coverage: meter unreadable (${e.message})`); }
  }
}

// 15. A stuck row gets a thinker before its next writer (center 2026-09-27, owner: "others need
// thinkers?"; gate per .opencode/commands/sprint.md §2: 6+ tries in 72 h, no planner in 48 h).
// The Tier A blockers GX3, GX5, Q1, EV7 and GM3 first showed the pattern (repeated writers
// still open (the look's last judge FAIL: "clear-color only, zero scene bits"): more of the same writer
// does not move them. An open row with 6+ helpers in 72 h and no planner in 48 h FAILs (the 3
// most-worked): the planner (paid twin first) writes the root cause, a different approach and the
// proof command into its plan, and the next writer works from that.
// Circus pass 2026-10-03: the check could not see its own remedy. The stuck-closer seat titles carry
// old row IDs, so a deep-dive it wrote for OPT3, CT12 or WIN9 never counted, the FAIL stayed, and the
// seat wrote the same row again (PQ1 five times, OPT3 three). A dated "deep-dive YYYY-MM-DD" in the
// row text, or a fresh docs/plan/<ID>-deepdive.md, now counts as the thinker for 48 h; a parked
// (LATER:) or blocked row waits on purpose and is not stuck.
{
  const { db } = await meterOpenOnce();
  const { tasks: stuckTasks, order: stuckOrder } = parseBoard(boardText, archivedDoneRows(ROOT));
  const open = new Set(stuckOrder.filter((id) => !stuckTasks.get(id).done && !/\b(?:LATER|BLOCKED):/.test(stuckTasks.get(id).lines.join(" "))));
  const deepDiveFresh = (id) => {
    const day = 24 * 3_600_000;
    const dates = [...stuckTasks.get(id).lines.join(" ").matchAll(/deep-dive (\d{4}-\d{2}-\d{2})/g)].map((m) => Date.parse(`${m[1]}T00:00:00Z`) + day);
    if (dates.some((end) => Date.now() - end < 48 * 3_600_000)) return true;
    try { return Date.now() - statSync(join(ROOT, "docs", "plan", `${id}-deepdive.md`)).mtimeMs < 48 * 3_600_000; } catch { return false; }
  };
  if (db && open.size) {
    try {
      // A run under 2 minutes is no work (the lead's own rest rule): 11 builders killed by a restart
      // inside 15 s at 2026-10-03 00:37 had counted as helpers on OPT4 and PF5.
      const rows = db.prepare("select title, agent, created, updated from sessions where repo='engine2040' and parent is not null and updated-created>=120000 and created>?").all(Date.now() - 72 * 3_600_000);
      const worked = new Map();
      for (const r of rows) for (const id of new Set(String(r.title ?? "").match(/\b[A-Z][A-Z0-9]*\d+[A-Za-z0-9]*\b/g) ?? [])) {
        if (!open.has(id)) continue;
        const w = worked.get(id) ?? { n: 0, thought: false };
        w.n += 1;
        if (/^planner/.test(String(r.agent ?? "")) && r.created > Date.now() - 48 * 3_600_000) w.thought = true;
        worked.set(id, w);
      }
      for (const [id, w] of worked) if (!w.thought && deepDiveFresh(id)) w.thought = true;
      const stuck = [...worked].filter(([, w]) => w.n >= 6 && !w.thought).sort((a, b) => b[1].n - a[1].n).slice(0, 3);
      const doneMs = rows.map((r) => (r.updated ?? 0) - (r.created ?? 0)).filter((n) => Number.isFinite(n) && n > 0);
      if (doneMs.length >= 3) say("PASS", rowTimeoutGuidance("stuck-window", doneMs, STUCK_STATIC_WINDOW_MS));
      if (!stuck.length) say("PASS", "stuck: no open row took 6+ helpers in 72 h without a planner deep-dive");
      for (const [id, w] of stuck) say("FAIL", `stuck ${id}: ${w.n} helpers in 72 h, still open, no planner in 48 h: a planner deep-dive first (root cause, a different approach, the proof command), then the writer`);
    } catch (e) { say("WARN", `stuck: meter unreadable (${e.message})`); }
  }
}

meterCloseOnce();

console.log(`RESULT ${out.FAIL ? "FAIL" : out.WARN ? "WARN" : "PASS"}: ${out.PASS} pass, ${out.WARN} warn, ${out.FAIL} fail`);
process.exit(out.FAIL ? 1 : 0);
} // main

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());
if (isMain) await main();

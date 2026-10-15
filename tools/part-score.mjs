// part-score: one live line per front of the repo's focus, so an owner can
// aim at a number and a judge can see if it moved.
//
// Usage from the repo/lane root:
//   node tools/part-score.mjs [look|bodies|aidoor|net|speed] [--json] [--round]
//   node tools/part-score.mjs --help
//   node --test tools/part-score.test.mjs
//
// Read-only (the only write is --round refreshing its own UTC stamp in
// target/part-score.round), under 60 s, no cargo: numbers come from files
// and git only, UNKNOWN when a source is missing, never a guess.
//
// Sources (each line names files, never estimates):
//   board rows via the board parser of tools/board-slice.mjs (VS/BX/EV/MP/OPT
//     ticked over total; Q1 n/14 and the A4 gate from tools/finish.mjs --json,
//     whose wall seconds also feed the lane cold-check line).
//   target/evidence/*, target/q1/manifest.json and team/look.md for frame
//     MOVED (none while no MOVED evidence names a percent).
//   tools/perf-baseline.json (baselines.rs pins) for CPU/GPU p95;
//     PENDING rows print UNKNOWN.
//   crates/anim/src + crates/physics/src #[test] count, crates/endpoint
//     e2_tool!( verbs and the tools.rs registry, crates/net loopback and
//     DET1 evidence files, git log for the --round window.
//
// Exit codes: 0 ok - 2 usage (unknown part or flag).
import { readFileSync, existsSync, readdirSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { spawnSync } from "node:child_process";
import { archivedDoneRows, loadBoardText, parseBoard } from "./board-slice.mjs";

const PARTS = ["look", "bodies", "aidoor", "net", "speed"];
const UNKNOWN = "UNKNOWN";
const HYGIENE_RE = /^(auto-backup|CQ|SL|DOCS|fmt)/;
const ROUND_WINDOW_MS = 24 * 3600 * 1000;
const MAX_BUFFER_BYTES = 64 * 1024 * 1024;

const stripAnsi = (s) => `${s ?? ""}`.replace(/\[[0-9;]*m/g, "");

// File-read memo: one live run hits ~120 .rs files plus a dozen small
// inputs; rescore loops reread the same paths, so cache by path for
// the life of the process (and skip the extra existsSync stat).
const readCache = new Map();
const walkCache = new Map();

export function clearPartScoreCache() {
  readCache.clear();
  walkCache.clear();
}

function readIf(path) {
  if (readCache.has(path)) return readCache.get(path);
  let out = null;
  try {
    out = readFileSync(path, "utf8");
  } catch {
    out = null;
  }
  readCache.set(path, out);
  return out;
}

function walkRs(dir, out = []) {
  const isTop = out.length === 0;
  if (isTop && walkCache.has(dir)) return [...walkCache.get(dir)];
  let entries = [];
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    if (isTop) walkCache.set(dir, [...out]);
    return out;
  }
  for (const e of entries) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walkRs(p, out);
    else if (e.isFile() && e.name.endsWith(".rs")) out.push(p);
  }
  if (isTop) walkCache.set(dir, [...out]);
  return out;
}

// Board: ticked over total for one id prefix. Exported for tests.
export function countByPrefix(tasks, order, re) {
  const ids = order.filter((id) => re.test(id));
  const done = ids.filter((id) => tasks.get(id)?.done).length;
  return { done, total: ids.length };
}

// `archived` (archivedDoneRows) keeps moved done rows in the counts: they stay
// ticked and in the total, as in sprint-loop's count (9d7e9102).
export function parseBoardCounts(boardText, archived = null) {
  const { tasks } = parseBoard(boardText, archived);
  const order = [...tasks.keys()];
  return {
    vs: countByPrefix(tasks, order, /^VS/),
    bx: countByPrefix(tasks, order, /^BX/),
    ev: countByPrefix(tasks, order, /^EV/),
    mp: countByPrefix(tasks, order, /^MP/),
    opt: countByPrefix(tasks, order, /^OPT/),
  };
}

// finish.mjs --json runner (the "(seconds)" reuse: wall time plus the gate
// table). Returns { ok, seconds, data }.
export function runFinish(root) {
  const start = Date.now();
  const r = spawnSync("node", [join(root, "tools", "finish.mjs"), "--json"], {
    encoding: "utf8",
    timeout: 55000,
    cwd: root,
    maxBuffer: MAX_BUFFER_BYTES,
  });
  const seconds = (Date.now() - start) / 1000;
  if (r.error || r.status !== 0) return { ok: false, seconds, data: null };
  try {
    return { ok: true, seconds, data: JSON.parse(r.stdout) };
  } catch {
    return { ok: false, seconds, data: null };
  }
}

export function finishGate(data, gate) {
  const row = (data ?? []).find((r) => r?.gate === gate);
  if (!row) return null;
  return {
    open: row.openRows?.length ?? 0,
    total: row.rows?.length ?? 0,
    status: row.status ?? UNKNOWN,
  };
}

// Frame MOVED: a percent only when some evidence names one, else none.
export function parseMovedPercent(text) {
  const m = /MOVED:\s*(?:pixel\s+)?(\d+(?:\.\d+)?)\s*%/i.exec(`${text ?? ""}`);
  return m ? Number(m[1]) : null;
}

export function decideFrameMoved({ frameMovedJson = null, lookMd = null, extra = null } = {}) {
  if (frameMovedJson) {
    try {
      const j = JSON.parse(frameMovedJson);
      if (Number.isFinite(j?.moved_pct)) return `pixel ${j.moved_pct}%`;
      if (j?.moved === "none") return "none";
    } catch {
      // Fall through to text scan.
    }
    const p = parseMovedPercent(frameMovedJson);
    if (p !== null) return `pixel ${p}%`;
  }
  let best = null;
  for (const blob of [lookMd, extra]) {
    const p = parseMovedPercent(blob);
    if (p !== null && (best === null || p > best)) best = p;
  }
  if (best !== null) return `pixel ${best}%`;
  return "none";
}

export function movedToRoundPct(moved) {
  const p = parseMovedPercent(moved ?? "");
  return p === null ? "0%" : `${p}%`;
}

// Break corpus digest: a hex digest only when a corpus file names one.
export function decideBreakDigest(texts = []) {
  for (const t of texts) {
    if (!t) continue;
    const m = /(?:digest|golden|suite_digest|corpus_digest)[:\s=]+([0-9a-f]{8,64})/i.exec(t);
    if (m) return m[1];
  }
  return "none";
}

// Test-attribute counter over [path, text] entries (comment lines skipped).
export function countTestAttrs(entries = []) {
  let n = 0;
  for (const [, text] of entries) {
    for (const line of `${text ?? ""}`.split("\n")) {
      const s = line.trim();
      if (s.startsWith("//") || s.startsWith("*")) continue;
      if (line.includes("#[test]")) n += 1;
    }
  }
  return n;
}

export function countAnimRagdollTests(root) {
  const files = [...walkRs(join(root, "crates", "anim", "src")), ...walkRs(join(root, "crates", "physics", "src"))];
  if (!files.length) return UNKNOWN;
  const entries = files.map((p) => [p, readIf(p) ?? ""]);
  return countTestAttrs(entries);
}

// Spatial eval score: only a file whose name says spatial-eval counts.
export function decideSpatialScore(texts = []) {
  for (const t of texts) {
    if (!t) continue;
    const m =
      /spatial[^\n:]*score[^\n:]*[:=]\s*(\d+(?:\.\d+)?)/i.exec(t) ??
      /score[^\n:]*[:=]\s*(\d+(?:\.\d+)?)/i.exec(t) ??
      /passed\s*=\s*(\d+\s*\/\s*\d+)/i.exec(t);
    if (m) return m[1].replace(/\s+/g, "");
  }
  return UNKNOWN;
}

// Verbs: non-comment e2_tool!( declarations in crates/endpoint/src.
export function countVerbsInSources(entries = []) {
  let n = 0;
  for (const [, text] of entries) {
    for (const line of `${text ?? ""}`.split("\n")) {
      if (line.trim().startsWith("//")) continue;
      if (/e2_tool!\(/.test(line)) n += 1;
    }
  }
  return n;
}

export function countEndpointVerbs(root) {
  const files = walkRs(join(root, "crates", "endpoint", "src"));
  if (!files.length) return UNKNOWN;
  return countVerbsInSources(files.map((p) => [p, readIf(p) ?? ""]));
}

// Door tools: the frozen TOOLS registry in crates/endpoint/src/tools.rs.
export function countDoorTools(toolsRsText) {
  if (!toolsRsText) return UNKNOWN;
  const m = toolsRsText.match(/name:\s*"[^"]+"/g);
  return m ? m.length : UNKNOWN;
}

// DET1 sync test: green only when an evidence file says PASS/green.
export function decideDet1(texts = []) {
  for (const t of texts) {
    if (t && /PASS|green/i.test(t)) return "green";
  }
  return "none";
}

// Loopback hash: 1/0 only when an evidence file records the comparison.
// Bout receipts (target/mp6/played-bout.json, target/on30/online-bout.json)
// record hash_x2_identical per bout leg; all true reads 1, any false reads 0.
function collectHashX2Bits(node, out = []) {
  if (!node || typeof node !== "object") return out;
  for (const [k, v] of Object.entries(node)) {
    if ((k === "hash_x2_identical" || k === "hash_x2") && typeof v === "boolean") out.push(v);
    else if (v && typeof v === "object") collectHashX2Bits(v, out);
  }
  return out;
}

// Loopback hash: 1/0 only when an evidence file records the comparison.
export function decideLoopback(texts = []) {
  let seen = false;
  for (const t of texts) {
    if (!t) continue;
    try {
      const j = JSON.parse(t);
      if (typeof j?.hash_equal === "boolean") return j.hash_equal ? "1" : "0";
      if (j?.server_hash && j?.client_hash) return j.server_hash === j.client_hash ? "1" : "0";
      const bits = collectHashX2Bits(j);
      if (bits.length) return bits.every((b) => b) ? "1" : "0";
    } catch {
      // Fall through to text scan.
    }
    const m = /hash[_ ]?equal[:\s=]+(1|0|true|false|equal|match)/i.exec(t);
    if (m) return /^(1|true|equal|match)$/i.test(m[1]) ? "1" : "0";
    const mx2 = /hash[_ ]?x2(?:[_ ]?identical)?[:\s=]+(1|0|true|false)/i.exec(t);
    if (mx2) return /^(1|true)$/i.test(mx2[1]) ? "1" : "0";
    if (/server_hash|client_hash|hash[_ ]?x2/i.test(t)) seen = true;
  }
  return seen ? "0" : UNKNOWN;
}

// perf-baseline.json p95: max p95 across rows carrying that quantile, else
// UNKNOWN (PENDING reference-GPU rows carry nulls, never zeros).
export function perfP95(baselineText, which) {
  try {
    const doc = typeof baselineText === "string" ? JSON.parse(baselineText) : baselineText;
    const key = which === "cpu" ? "cpu_ms" : "gpu_ms";
    const vals = [];
    for (const r of doc?.rows ?? []) {
      const q = r?.[key];
      if (q && Number.isFinite(q.p95)) vals.push(q.p95);
    }
    if (!vals.length) return UNKNOWN;
    return `${Math.max(...vals)}ms`;
  } catch {
    return UNKNOWN;
  }
}

// One line per front. Missing inputs print UNKNOWN, never a guess.
export function lookLine({ a4 = null, vs = null, moved = "none" } = {}) {
  return `look: Q1 ${a4 ? `${a4.open}/${a4.total}` : UNKNOWN} | VS ticked ${vs ? `${vs.done}/${vs.total}` : UNKNOWN} | frame MOVED ${moved} | A4 gate ${a4 ? a4.status : UNKNOWN}`;
}

export function bodiesLine({ bx = null, digest = "none", tests = UNKNOWN } = {}) {
  return `bodies: BX ticked ${bx ? `${bx.done}/${bx.total}` : UNKNOWN} | break corpus BX28 ${digest} | anim and ragdoll tests ${tests}`;
}

export function aidoorLine({ ev = null, spatial = UNKNOWN, verbs = UNKNOWN, tools = UNKNOWN } = {}) {
  return `aidoor: EV tasks passing ${ev ? `${ev.done}/${ev.total}` : UNKNOWN} | spatial eval score ${spatial} | verbs per system ${verbs} | door tools ${tools}`;
}

export function netLine({ mp = null, det1 = "none", loopback = UNKNOWN } = {}) {
  return `net: MP ticked ${mp ? `${mp.done}/${mp.total}` : UNKNOWN} | DET1 sync test ${det1} | loopback hash equal ${loopback}`;
}

export function speedLine({ cpu = UNKNOWN, gpu = UNKNOWN, cold = UNKNOWN, opt = null } = {}) {
  return `speed: CPU p95 ${cpu} | GPU p95 ${gpu} | lane cold-check ${cold} | OPT ticked ${opt ? `${opt.done}/${opt.total}` : UNKNOWN}`;
}

export function buildLines(ctx = {}) {
  return {
    look: lookLine(ctx),
    bodies: bodiesLine(ctx),
    aidoor: aidoorLine(ctx),
    net: netLine(ctx),
    speed: speedLine(ctx),
  };
}

// ---- --round helpers (pure, fixture-driven for tests) ----

export function parseGitRecords(raw) {
  const out = [];
  for (const rec of `${raw ?? ""}`.split("")) {
    if (!rec.trim()) continue;
    const [hash = "", subject = "", body = ""] = rec.split("");
    if (!hash.trim()) continue;
    out.push({ hash: hash.trim(), subject: subject.trim(), body: (body ?? "").trim() });
  }
  return out;
}

export function landedCommits(commits = []) {
  return commits.filter((c) => !HYGIENE_RE.test(c.subject));
}

export function hasResultLine(text) {
  return /^RESULT:/m.test(stripAnsi(text ?? ""));
}

export function hasVerdictPass(text) {
  return /VERDICT:\s*PASS/i.test(stripAnsi(text ?? ""));
}

export function countCapKills(logText, sinceMs) {
  let n = 0;
  for (const line of `${logText ?? ""}`.split("\n")) {
    const m = /^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\S+)/.exec(line);
    if (!m) continue;
    const at = Date.parse(m[1]);
    if (!Number.isFinite(at) || at < sinceMs) continue;
    const clean = stripAnsi(line);
    if (clean.includes("over") && clean.includes("helper_max_min")) n += 1;
  }
  return n;
}

export function countStealsLanded(stealsText, sinceMs) {
  let n = 0;
  for (const line of `${stealsText ?? ""}`.split("\n")) {
    const m = /^(\d{4}-\d{2}-\d{2})\s*\|/.exec(line.trim());
    if (!m) continue;
    const at = Date.parse(`${m[1]}T00:00:00Z`);
    if (!Number.isFinite(at) || at < sinceMs) continue;
    if (/\blanded\b/i.test(line)) n += 1;
  }
  return n;
}

export function clipDelivered(ordersText, sinceMs) {
  for (const line of `${ordersText ?? ""}`.split("\n")) {
    if (!/clip/i.test(line) || !/delivered/i.test(line)) continue;
    const m = /(\d{4}-\d{2}-\d{2})/.exec(line);
    if (m) {
      const at = Date.parse(`${m[1]}T00:00:00Z`);
      if (Number.isFinite(at) && at < sinceMs) continue;
    }
    return "1";
  }
  return "0";
}

// Distinct tools with a test: git-touched tools/*.mjs (non-test) whose
// sibling .test.mjs exists in the tree.
export function countToolsLanded(toolFiles = [], existsTest = () => false) {
  const bases = new Set();
  for (const f of toolFiles) {
    const base = `${f ?? ""}`.split(/[\\/]/).pop() ?? "";
    if (!base.endsWith(".mjs") || base.endsWith(".test.mjs")) continue;
    const sib = base.replace(/\.mjs$/, ".test.mjs");
    if (existsTest(sib)) bases.add(base);
  }
  return bases.size;
}

export function roundVerdict({ movedCommit = false, verdictMoved = false, steals = 0, clip = "0", tools = 0 } = {}) {
  const pass = movedCommit || verdictMoved || steals > 0 || clip === "1" || tools > 0;
  return pass ? { pass: true, line: "RESULT: PASS - something real was made" } : { pass: false, line: "RESULT: FAIL - nothing real was made" };
}

export function roundLine({ landed = UNKNOWN, owners = UNKNOWN, movedPct = "0%", kills = 0, steals = 0, clip = "0", tools = 0 } = {}) {
  return `ROUND: landed ${landed} | owners reported ${owners}/5 | frame MOVED ${movedPct} | cap kills ${kills} | steals landed ${steals} | clip delivered ${clip} | tools landed ${tools}`;
}

// ---- live collection (files + git, no cargo) ----

function collectNormal(root) {
  // The index holds few rows: the board is index plus docs/SPRINT-*.md shards (loadBoardText).
  const boardText = readIf(join(root, "docs", "SPRINT.md")) ? loadBoardText(root) : null;
  const counts = boardText ? parseBoardCounts(boardText, archivedDoneRows(root)) : null;
  const finish = runFinish(root);
  const a4 = finish.ok ? finishGate(finish.data, "A4") : null;
  const moved = decideFrameMoved({
    frameMovedJson: readIf(join(root, "target", "frame-moved.json")),
    lookMd: readIf(join(root, "team", "look.md")),
    extra: readIf(join(root, "target", "q1", "manifest.json")),
  });
  const digest = decideBreakDigest([
    readIf(join(root, "target", "evidence", "bx28", "report.txt")),
    readIf(join(root, "target", "break-corpus.json")),
    readIf(join(root, "crates", "proof", "src", "break_corpus.rs")),
  ]);
  const tests = countAnimRagdollTests(root);
  const spatial = decideSpatialScore(
    [
      join(root, "target", "evidence", "spatial-eval", "report.txt"),
      join(root, "target", "spatial-eval.json"),
      join(root, "target", "evidence", "ev-spatial", "report.txt"),
      join(root, "target", "evidence", "spatial", "report.txt"),
    ].map(readIf),
  );
  const verbs = countEndpointVerbs(root);
  const tools = countDoorTools(readIf(join(root, "crates", "endpoint", "src", "tools.rs")));
  const det1 = decideDet1([
    readIf(join(root, "target", "evidence", "det1", "report.txt")),
    readIf(join(root, "target", "evidence", "net-det1", "report.txt")),
  ]);
  const loopback = decideLoopback([
    readIf(join(root, "target", "evidence", "net-loopback.json")),
    readIf(join(root, "target", "net-loopback.json")),
    readIf(join(root, "target", "mp6", "played-bout.json")),
    readIf(join(root, "target", "on30", "online-bout.json")),
  ]);
  const baselineText = readIf(join(root, "tools", "perf-baseline.json"));
  const cold = finish.ok ? `${finish.seconds.toFixed(1)}s` : UNKNOWN;
  return {
    a4,
    vs: counts?.vs ?? null,
    moved,
    bx: counts?.bx ?? null,
    digest,
    tests,
    ev: counts?.ev ?? null,
    spatial,
    verbs,
    tools,
    mp: counts?.mp ?? null,
    det1,
    loopback,
    cpu: baselineText ? perfP95(baselineText, "cpu") : UNKNOWN,
    gpu: baselineText ? perfP95(baselineText, "gpu") : UNKNOWN,
    cold,
    opt: counts?.opt ?? null,
  };
}

function gitRecordsSince(root, sinceIso) {
  const r = spawnSync("git", ["log", `--since=${sinceIso}`, "--pretty=format:%H%x01%s%x01%b%x1e", "--no-merges"], {
    encoding: "utf8",
    timeout: 30000,
    cwd: root,
    maxBuffer: MAX_BUFFER_BYTES,
  });
  if (r.error || r.status !== 0) return null;
  return parseGitRecords(r.stdout);
}

function gitToolFilesSince(root, sinceIso) {
  const r = spawnSync(
    "git",
    ["log", `--since=${sinceIso}`, "--pretty=format:%H", "--name-only", "--", "tools/"],
    { encoding: "utf8", timeout: 30000, cwd: root, maxBuffer: MAX_BUFFER_BYTES },
  );
  if (r.error || r.status !== 0) return null;
  return r.stdout.split("\n").map((l) => l.trim()).filter((l) => l.startsWith("tools/"));
}

function doneResultsSince(root, sinceMs) {
  const dir = join(root, "target", "queue", "done");
  let files = [];
  try {
    files = readdirSync(dir).filter((f) => f.endsWith(".md"));
  } catch {
    return { count: 0, verdictMoved: false };
  }
  let count = 0;
  let verdictMoved = false;
  for (const f of files) {
    const p = join(dir, f);
    let mt = 0;
    try {
      mt = statSync(p).mtimeMs;
    } catch {
      continue;
    }
    if (mt < sinceMs) continue;
    const text = readIf(p);
    if (!text || !hasResultLine(text)) continue;
    count += 1;
    if (/MOVED/i.test(stripAnsi(text)) && hasVerdictPass(text)) verdictMoved = true;
  }
  return { count, verdictMoved };
}

function collectRound(root) {
  const stampPath = join(root, "target", "part-score.round");
  const now = Date.now();
  let sinceMs = now - ROUND_WINDOW_MS;
  const stampText = readIf(stampPath);
  if (stampText) {
    const at = Date.parse(stampText.trim());
    if (Number.isFinite(at) && at <= now) sinceMs = at;
  }
  const sinceIso = new Date(sinceMs).toISOString();
  const records = gitRecordsSince(root, sinceIso);
  const landed = records ? landedCommits(records) : null;
  const movedCommit = landed ? landed.some((c) => /MOVED/i.test(`${c.subject}\n${c.body}`)) : false;
  const done = doneResultsSince(root, sinceMs);
  const kills = countCapKills(readIf(join(root, "target", "loop-keeper.log")), sinceMs);
  const steals = countStealsLanded(readIf(join(root, "docs", "plan", "steals.md")), sinceMs);
  const clip = clipDelivered(readIf(join(root, "orders.csv")), sinceMs);
  const toolFiles = gitToolFilesSince(root, sinceIso);
  const tools =
    toolFiles === null
      ? UNKNOWN
      : countToolsLanded(toolFiles, (sib) => existsSync(join(root, "tools", sib)));
  const lookMd = readIf(join(root, "team", "look.md"));
  const movedPct = movedToRoundPct(
    [lookMd, ...(landed ?? []).map((c) => `${c.subject}\n${c.body}`)].find((t) => parseMovedPercent(t) !== null) ?? "none",
  );
  const verdict = roundVerdict({ movedCommit, verdictMoved: done.verdictMoved, steals, clip, tools: tools === UNKNOWN ? 0 : tools });
  return {
    line: roundLine({
      landed: landed === null ? UNKNOWN : landed.length,
      owners: done.count,
      movedPct,
      kills,
      steals,
      clip,
      tools,
    }),
    resultLine: verdict.line,
    pass: verdict.pass,
    stampPath,
  };
}

function printHelp() {
  console.log("Usage: node tools/part-score.mjs [look|bodies|aidoor|net|speed] [--json] [--round]");
  console.log("Prints one live line per front from files and git (UNKNOWN when a source is missing).");
  console.log("--round prints the ROUND line plus RESULT: PASS|FAIL and rewrites target/part-score.round.");
}

function main(argv) {
  const root = process.cwd();
  let part = null;
  let json = false;
  let round = false;
  for (const a of argv) {
    if (a === "--json") json = true;
    else if (a === "--round") round = true;
    else if (a === "--help" || a === "-h") {
      printHelp();
      process.exit(0);
    } else if (a.startsWith("-")) {
      console.error(`part-score: error: unknown flag ${a}`);
      process.exit(2);
    } else if (!part && PARTS.includes(a)) part = a;
    else {
      console.error(`part-score: error: unknown part ${a} (want ${PARTS.join("|")})`);
      process.exit(2);
    }
  }

  if (round) {
    const r = collectRound(root);
    try {
      mkdirSync(dirname(r.stampPath), { recursive: true });
      writeFileSync(r.stampPath, `${new Date().toISOString()}\n`);
    } catch {
      // The stamp is machine state; a failed rewrite never fails the report.
    }
    if (json) {
      console.log(JSON.stringify({ round: r.line, result: r.resultLine, pass: r.pass }, null, 2));
    } else {
      console.log(r.line);
      console.log(r.resultLine);
    }
    return;
  }

  const ctx = collectNormal(root);
  const lines = buildLines(ctx);
  const picked = part ? [part] : PARTS;
  if (json) {
    console.log(JSON.stringify(picked.map((p) => ({ part: p, line: lines[p] })), null, 2));
  } else {
    for (const p of picked) console.log(lines[p]);
  }
}

if (process.argv[1] && process.argv[1].replace(/\\/g, "/").endsWith("tools/part-score.mjs")) {
  main(process.argv.slice(2));
}

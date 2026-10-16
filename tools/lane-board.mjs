// lane-board.mjs — keeper-readable dispatch table for `ready: board` seats.
//
// docs/SPRINT.md is checkbox lines, not a table, so no seat could ever use
// `ready: board` before: the keeper's createSeatReadiness.rows needs a header
// with ID, Status and Role, and wakes a seat only for READY or TOP rows
// matched by ready-role and ready-match. This tool writes that table to
// target/queue/lane-board.md (gitignored, refreshed every round).
//
//   node tools/lane-board.mjs [--root DIR]   write target/queue/lane-board.md
//   node tools/lane-board.mjs --check        keeper-style count per standing seat
//
// Table format, exact: header `| ID | Status | Role | Lane | Row |`, separator,
// then one line per item, cells separated by ` | ` with single spaces, a `|`
// inside text replaced by `/`, Row text at most 140 chars.
//
// Lane comes from the id prefix, copied from the FOCUS table in
// tools/drift-check.mjs. Order: the owner's ids of docs/FOCUS.md first (the
// "## Now" `Rows:` lists, then a legacy TOP line; owner order beats everything,
// clearing them takes the order away with no code edit), then SALVAGE items, then pieces, then split rows, then PARK items,
// then the Rome-first section in board order, then the rest in dry-run order,
// then STEAL / CLIP / TWIN / COMPACT extras. The keeper hands a woken seat the
// FIRST matching line, so this order is the dispatch order.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { archivedDoneRows, parseBoard, rowReady, needsOf, ownedOf, blockedBy, loadBoardText, focusNowIds } from "./board-slice.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_ROOT = dirname(HERE);

const HELP = `lane-board.mjs — keeper-readable dispatch table (tool sprint 1/6)

Usage:
  node tools/lane-board.mjs [--root DIR]   write target/queue/lane-board.md
  node tools/lane-board.mjs --check [--root DIR]
      parse the file the way the keeper does and print, per seat file in
      target/queue/standing/, how many items its ready-role and ready-match
      select (exit 1 unless look, body, ai, net and speed each get >= 1)
  node tools/lane-board.mjs --help

Exit codes: 0 ok - 1 check FAIL or unreadable input - 2 usage.
`;

// Lane by id prefix, copied from the FOCUS table in tools/drift-check.mjs
// (look: VS GX RT VG VX RL RP; body: BX HS PX DX AM LM TV; ai: MCP AX EA EV SP;
// speed: OPT PF; net: MP ON). Plus the packet's own extensions: BV and Q read
// as look, PH AN CB as body, SN as ai, PQ BENCH as speed, NET DET as net,
// DT as body (FOCUS TOP line: DT5 is HS15's prerequisite, a body seat item),
// CT and rows owning content/ as content, CQ SL as hygiene, GM WIN ROME and
// every row with no lane as ship.
const LOOK = new Set(["VS", "GX", "RT", "VG", "VX", "RL", "RP", "BV", "Q"]);
const BODY = new Set(["BX", "HS", "PX", "DX", "AM", "LM", "TV", "PH", "AN", "CB", "DT"]);
const AI = new Set(["MCP", "AX", "EA", "EV", "SP", "SN"]);
const SPEED = new Set(["OPT", "PF", "PQ", "BENCH"]);
const NET = new Set(["MP", "ON", "NET", "DET"]);
const HYGIENE = new Set(["CQ", "SL"]);

// Sections whose rows stay but never wake anyone (FOCUS.md Parked).
const PARKED_SECTION = /platform and live ai|audio|\bui\b|modding|platform and shipping|living world/i;

// One-off packets the lead parked 2026-10-04 until VS1 lands; only the pieces
// come back, written by the splitter.
const PARKED_PIECES = [
  { packet: "audit-gym-endpoint", pieces: "GYM", note: "gym verbs reset step observe for the ai-door owner" },
  { packet: "audit-sync-test", pieces: "DET1", note: "SyncTest plus 2-client loopback for the net owner" },
];

export function idPrefix(id) {
  const m = /^([A-Z]+)/.exec(String(id ?? ""));
  return m ? m[1] : "";
}

export function laneOf(id, owned = []) {
  const prefix = idPrefix(id);
  // VS is look work even when its text names content/ (VS3 textures,
  // VS5b clips): the content/ mention is a data source, not the owner.
  if (prefix === "VS") return "look";
  if (prefix === "CT" || owned.some((p) => p === "content/" || p.startsWith("content/"))) return "content";
  if (LOOK.has(prefix)) return "look";
  if (BODY.has(prefix)) return "body";
  if (AI.has(prefix)) return "ai";
  if (SPEED.has(prefix)) return "speed";
  if (NET.has(prefix)) return "net";
  if (HYGIENE.has(prefix)) return "hygiene";
  return "ship";
}

export function roleOfLane(lane) {
  if (lane === "split") return "planner";
  if (lane === "steal" || lane === "twin") return "researcher";
  if (lane === "clip") return "pilot";
  return "builder";
}

// Owner order: the ids after `Rows:` in the first "## Now" section of
// docs/FOCUS.md (owner 2026-10-10), then the ids of a line that starts TOP, go
// before every other line of their own lane, in that order. Clearing them
// takes the order away; no code edit needed.
export function parseTopLine(focusText) {
  const ids = focusNowIds(focusText);
  const m = /^TOP\s*\(.*?\)\s*:\s*(.+)$/m.exec(String(focusText ?? ""));
  if (!m) return ids;
  for (const s of m[1].split(/[\s,;]+/).map((x) => x.trim())) {
    if (/^[A-Z][A-Z0-9]*\d[a-z]?$/.test(s) && !ids.includes(s)) ids.push(s);
  }
  return ids;
}

export function sizeOf(task) {
  const m = /\bSize ([SLM])\b/.exec(task.lines.join(" "));
  return m ? m[1] : "-";
}

export function shortSection(task) {
  const s = task.section ?? task.h3 ?? "-";
  return String(s).split(" (")[0].trim().slice(0, 40) || "-";
}

export function titleOf(task) {
  const first = task.lines[0].replace(/^- \[[ x]\] [A-Z][A-Z0-9]*\d[a-z]?\s*/, "");
  return first.replace(/\s+/g, " ").trim();
}

// Row text: `<Size> <section> <title start>`, at most 140 chars, no `|`.
export function rowCell(task) {
  return `${sizeOf(task)} ${shortSection(task)} ${titleOf(task)}`
    .replace(/\|/g, "/").replace(/\s+/g, " ").trim().slice(0, 140);
}

export function waitsOf(tasks, task) {
  const open = needsOf(task).filter((d) => !tasks.get(d)?.done);
  if (open.length) return `WAITS ${open.slice(0, 4).join(" ")}`;
  return `WAITS ${blockedBy(tasks, task)}`;
}

// The keeper's own table parser (copied from createSeatReadiness.rows in
// .opencode/plugin/loop-keeper.js): the test proves our file parses with it,
// and --check counts with it.
export function keeperRows(raw) {
  const lines = String(raw).replace(/\r\n/g, "\n").split("\n");
  const cells = (line) => line.trim().replace(/^\||\|$/g, "").split(/(?<!\\)\|/).map((cell) => cell.trim());
  const start = lines.findIndex((line) => /^\|/.test(line) && cells(line).includes("ID") && cells(line).includes("Status") && (cells(line).includes("Role") || cells(line).includes("Owner role")));
  if (start < 0) return null;
  const header = cells(lines[start]).map((cell) => cell === "Owner role" ? "Role" : cell);
  const out = [];
  for (let i = start + 2; i < lines.length && /^\s*\|/.test(lines[i]); i++) {
    const values = cells(lines[i]);
    if (values.length !== header.length) continue;
    out.push({ ...Object.fromEntries(header.map((key, j) => [key, values[j]])), text: lines[i] });
  }
  return out;
}

// Splitter piece lines: `a | crate | owned paths | test | what moves | size`.
export function parsePieces(text) {
  const out = [];
  for (const line of String(text ?? "").split(/\r?\n/)) {
    const m = /^\s*([A-Za-z0-9]{1,4})\s*\|(.*)$/.exec(line);
    if (!m || /^\s*piece\s*\|/i.test(line)) continue;
    out.push({ piece: m[1].trim(), rest: m[2].trim().replace(/\s+/g, " ") });
  }
  return out;
}

const cell = (s) => String(s).replace(/\|/g, "/").replace(/\s+/g, " ").trim();

// Pure dispatch: every input arrives in `data`, so the test can drive it with
// fixtures and the CLI only gathers.
export function buildBoard(data) {
  const { tasks, order } = data;
  const lines = ["| ID | Status | Role | Lane | Row |", "|---|---|---|---|---|"];
  const emitted = new Set();
  const push = (id, status, role, lane, row) => {
    if (emitted.has(id)) return;
    emitted.add(id);
    lines.push(`| ${cell(id)} | ${cell(status)} | ${cell(role)} | ${cell(lane)} | ${cell(row).slice(0, 140)} |`);
  };

  // 1. Owner order: the FOCUS ids before every other line of their lane. A row
  // with pieces or held back by salvage waits for steps 2-3; a row to split
  // goes to the planner here, never whole to a builder.
  for (const id of data.topIds) {
    const task = tasks.get(id);
    if (!task || task.done) continue;
    if (data.held.has(id) || data.pieces.has(id)) continue;
    if (data.splitIds.has(id)) {
      push(id, rowReady(tasks, task) ? "READY" : waitsOf(tasks, task), "planner", "split", rowCell(task));
      continue;
    }
    const lane = laneOf(id, ownedOf(task));
    push(id, rowReady(tasks, task) ? "TOP" : waitsOf(tasks, task), roleOfLane(lane), lane, rowCell(task));
  }

  // 2. Salvage: a cut lane's patch goes to a judge; its row waits for owners.
  for (const s of data.salvage) {
    push(`SALVAGE-${s.lane}`, "READY", "planner", "split",
      `- split review ${s.lane} patch ${s.bytes} bytes to a judge before rebuild`);
  }

  // 3. Pieces replace their row; only the first undone piece is READY.
  for (const id of order) {
    const pieces = data.pieces.get(id);
    if (!pieces || !pieces.length) continue;
    const task = tasks.get(id);
    if (!task || task.done) continue;
    const lane = laneOf(id, ownedOf(task));
    const role = roleOfLane(lane);
    let first = true;
    // A row that is BLOCKED, LATER or waiting on open needs keeps its pieces waiting too (2026-10-07: pieces of a
    // blocked row were handed to builders as READY).
    const parentReady = rowReady(tasks, task);
    for (const p of pieces) {
      const done = data.commitsText.includes(`${id}.${p.piece}`);
      if (done) push(`${id}.${p.piece}`, "DONE", role, lane, `- split piece ${id}.${p.piece} ${p.rest}`.slice(0, 140));
      else if (first) { push(`${id}.${p.piece}`, parentReady ? "READY" : waitsOf(tasks, task), role, lane, `- split piece ${id}.${p.piece} ${p.rest}`.slice(0, 140)); first = false; }
      else push(`${id}.${p.piece}`, "QUEUED", role, lane, `- split piece ${id}.${p.piece} ${p.rest}`.slice(0, 140));
    }
  }

  // 4. Split: Size L rows, and Size M rows owning more than 3 paths, with no
  // pieces file yet. VS1 is labelled M and owns 4 paths, so it qualifies.
  for (const id of order) {
    const task = tasks.get(id);
    if (!task || task.done || emitted.has(id)) continue;
    if (data.pieces.has(id) || data.held.has(id)) continue;
    const text = task.lines.join(" ");
    const bigM = /\bSize M\b/.test(text) && ownedOf(task).length > 3;
    if (!/\bSize L\b/.test(text) && !bigM) continue;
    // Only a row that can start is cut: a BLOCKED, LATER or waiting row is listed with its wait and never wakes the
    // splitter (2026-10-07 measured: 18 of 50 split rows were not ready, among them the dropped Bevy spike BV17).
    push(id, rowReady(tasks, task) ? "READY" : waitsOf(tasks, task), "planner", "split", rowCell(task));
  }

  // 5. Park: repeat one-offs and cap-cut rows come back as pieces, never whole.
  for (const p of data.park) push(p.id, "READY", "planner", "split", p.text);

  // 6. Rome-first section in board order, then the rest in dry-run order
  // (the dry-run ready list is board order filtered to ready).
  const step6Start = lines.length;
  const romeFirst = order.filter((id) => (tasks.get(id).section ?? "").startsWith("Rome first"));
  const rest = order.filter((id) => !(tasks.get(id).section ?? "").startsWith("Rome first"));
  // work-steal per-lane deque plus idle-first steal plus wait boost. Inspired by taskflow/taskflow MIT https://github.com/taskflow/taskflow (per-thread queues); fresh code, no copy.
  const dispatchOrder = [...romeFirst, ...rest];
  const above = keeperRows(lines.join("\n")) ?? [];
  const busyAbove = new Set(above.filter((r) => r.Status === "READY" || r.Status === "TOP").map((r) => r.Lane));
  const deques = new Map();
  const waitsPile = [];
  const parkedPile = [];
  for (const id of dispatchOrder) {
    const task = tasks.get(id);
    if (!task || task.done || emitted.has(id)) continue;
    if (data.pieces.has(id) || data.held.has(id)) continue;
    if (data.splitIds.has(id)) continue;
    const lane = laneOf(id, ownedOf(task));
    const role = roleOfLane(lane);
    if (PARKED_SECTION.test(shortSection(task))) { parkedPile.push({ id, role, lane, row: rowCell(task) }); continue; }
    if (data.capCut.has(id)) continue;
    if (rowReady(tasks, task) && (data.capCuts.get(id) ?? 0) < 2) {
      if (!deques.has(lane)) deques.set(lane, []);
      deques.get(lane).push({ id, role, lane, row: rowCell(task) });
    } else if (!rowReady(tasks, task)) {
      waitsPile.push({ id, status: waitsOf(tasks, task), role, lane, row: rowCell(task) });
    }
  }
  const laneOrder = [...deques.keys()];
  laneOrder.sort((a, b) => ((busyAbove.has(a) ? 1 : 0) - (busyAbove.has(b) ? 1 : 0)) || (deques.get(b).length - deques.get(a).length));
  const readyOut = [];
  let more = true;
  while (more) {
    more = false;
    for (const lane of laneOrder) {
      const own = deques.get(lane);
      if (own && own.length) { readyOut.push(own.shift()); more = true; continue; }
      let victim = null;
      for (const q of deques.values()) {
        if (q.length && (!victim || q.length > victim.length)) victim = q;
      }
      if (victim && victim.length) { readyOut.push(victim.shift()); more = true; }
    }
  }
  let boosted = false;
  for (let i = 1; i < waitsPile.length && !boosted; i++) {
    if ((data.capCuts.get(waitsPile[i].id) ?? 0) < 2) {
      const t = waitsPile[i - 1]; waitsPile[i - 1] = waitsPile[i]; waitsPile[i] = t; boosted = true;
    }
  }
  for (const r of readyOut) push(r.id, "READY", r.role, r.lane, r.row);
  for (const w of waitsPile) push(w.id, w.status, w.role, w.lane, w.row);
  for (const p of parkedPile) push(p.id, "PARKED", p.role, p.lane, p.row);

  // Join-the-Idle-Queue route to idle/fast seats. Ideas only from arXiv:1709.10119 + arXiv:2106.04707 (idle-queue plus fast-seat choice); fresh code, no copy.
  // Lanes with no READY/TOP above step6Start are idle; their step-6 READY
  // moves first (fast net/speed among idle lead), busy rounds keep order.
  // Order only, status never flips.
  {
    const fastLanes = new Set(["net", "speed"]);
    const busyLanes = new Set();
    for (let i = 2; i < step6Start && i < lines.length; i++) {
      const parts = String(lines[i]).split("|").map((c) => c.trim());
      if (parts.length < 6) continue;
      const st = parts[2];
      const ln = parts[4];
      if ((st === "READY" || st === "TOP") && ln) busyLanes.add(ln);
    }
    const tail = lines.slice(step6Start);
    const fastIdle = [];
    const idleRest = [];
    const keepOrder = [];
    for (const line of tail) {
      const parts = String(line).split("|").map((c) => c.trim());
      if (parts.length < 6) { keepOrder.push(line); continue; }
      const st = parts[2];
      const ln = parts[4];
      if ((st === "READY" || st === "TOP") && ln && !busyLanes.has(ln)) {
        if (fastLanes.has(ln)) fastIdle.push(line);
        else idleRest.push(line);
      } else {
        keepOrder.push(line);
      }
    }
    if (fastIdle.length || idleRest.length) {
      lines.splice(step6Start, tail.length, ...fastIdle, ...idleRest, ...keepOrder);
    }
  }

  // 7. Extras: steals, clips, twins, compacts.
  for (const e of [...data.steals, ...data.clips, ...data.twins, ...data.compacts]) {
    push(e.id, "READY", e.role, e.lane, e.text);
  }
  return lines.join("\n") + "\n";
}

function safeRead(p) {
  try { return readFileSync(p, "utf8"); } catch { return null; }
}

function mtimeMs(p) {
  try { return statSync(p).mtimeMs; } catch { return 0; }
}

function maxMtime(dir, depth) {
  let top = mtimeMs(dir);
  if (depth <= 0 || !existsSync(dir)) return top;
  let kids = [];
  try { kids = readdirSync(dir, { withFileTypes: true }); } catch { return top; }
  for (const k of kids) {
    if (k.name === ".git" || k.name === "target") continue;
    const p = join(dir, k.name);
    top = Math.max(top, k.isDirectory() ? maxMtime(p, depth - 1) : mtimeMs(p));
  }
  return top;
}

// Speed: the done|failed verdict files (1000+) and the keeper log (500KB)
// were read twice per run (salvage + cap-cuts, cap-cuts + park). Read each
// once in gather() and share the result; the helpers below accept the
// pre-read data but still read it themselves when called directly.
function readVerdictTexts(root) {
  const verdicts = new Map();
  for (const dir of [join(root, "target", "queue", "done"), join(root, "target", "queue", "failed")]) {
    let files = [];
    try { files = readdirSync(dir).filter((f) => f.endsWith(".md")); } catch { continue; }
    for (const f of files) {
      const text = safeRead(join(dir, f)) ?? "";
      verdicts.set(f, text);
    }
  }
  return verdicts;
}

// Every .lanes/<id> whose target/lanes/<id>.patch is non-empty, newer than the
// lane's last cut and has no verdict in target/queue/done|failed. While the
// salvage item stands, the row is held back from the owners.
function gatherSalvage(root, sharedVerdicts = null) {
  const out = [];
  const held = new Set();
  let lanesDir = join(root, ".lanes");
  let patchDir = join(root, "target", "lanes");
  let doneDir = join(root, "target", "queue", "done");
  let failedDir = join(root, "target", "queue", "failed");
  let lanes = [];
  try { lanes = readdirSync(lanesDir, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => d.name); } catch { return { salvage: out, held }; }
  const verdicts = sharedVerdicts ?? readVerdictTexts(root);
  for (const lane of lanes) {
    const patch = join(patchDir, `${lane}.patch`);
    let size = 0;
    try { size = statSync(patch).size; } catch { continue; }
    if (!size) continue;
    const patchMtime = mtimeMs(patch);
    if (Date.now() - patchMtime > 7 * 86_400_000) continue; // stale salvage, not this week's (cheap stat first)
    if (patchMtime + 5 * 60_000 < maxMtime(join(lanesDir, lane), 2)) continue; // older than the lane's last cut
    const rowId = lane.toUpperCase();
    const hasVerdict = [...verdicts].some(([f, text]) =>
      f.toLowerCase().includes(lane.toLowerCase()) ||
      (new RegExp(`ROW:\\s*${rowId}\\b`).test(text) && /VERDICT:|RESULT:\s*(?:DONE|FAIL|PASS)/i.test(text)));
    if (hasVerdict) continue;
    out.push({ lane, bytes: size, at: patchMtime });
    held.add(lane.toUpperCase());
  }
  return { salvage: out, held };
}

// Rows cut at the cap twice in 24 h: count the `over helper_max_min` lines of
// target/loop-keeper.log for run ids `<seat>-rN`, joined to the row through the
// run's record in target/queue/done|failed that names `ROW: <ID>`.
function gatherCapCuts(root, sharedVerdicts = null, sharedLog = null) {
  const counts = new Map();
  const log = sharedLog ?? safeRead(join(root, "target", "loop-keeper.log")) ?? "";
  const runRows = new Map();
  if (sharedVerdicts) {
    for (const text of sharedVerdicts.values()) {
      const row = /ROW:\s*([A-Z][A-Z0-9]*\d[a-z]?)/.exec(text);
      const run = /([A-Za-z][\w-]*-r\d+)/.exec(text);
      if (row && run) runRows.set(run[1], row[1]);
    }
  } else {
    for (const dir of [join(root, "target", "queue", "done"), join(root, "target", "queue", "failed")]) {
      let files = [];
      try { files = readdirSync(dir).filter((f) => f.endsWith(".md")); } catch { continue; }
      for (const f of files) {
        const text = safeRead(join(dir, f)) ?? "";
        const row = /ROW:\s*([A-Z][A-Z0-9]*\d[a-z]?)/.exec(text);
        const run = /([A-Za-z][\w-]*-r\d+)/.exec(text);
        if (row && run) runRows.set(run[1], row[1]);
      }
    }
  }
  for (const line of log.split("\n")) {
    if (!/over helper_max_min/.test(line)) continue;
    const at = Date.parse(line.slice(0, 24));
    if (Number.isFinite(at) && Date.now() - at > 24 * 3_600_000) continue;
    const run = /([A-Za-z][\w-]*-r\d+)/.exec(line);
    const id = (run && runRows.get(run[1])) || null;
    if (!id) continue;
    counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  return { counts, joined: runRows.size };
}

// One-off packets in target/queue/ready/ that started 3+ times in 24 h with 0
// completions (count `batch: <name> ran|started` and `completed` lines of the
// keeper log), plus the two parked audits while their pieces files are missing.
function gatherPark(root, sharedLog = null) {
  const park = [];
  const log = sharedLog ?? safeRead(join(root, "target", "loop-keeper.log")) ?? "";
  const logLines = log.split("\n");
  let ready = [];
  try { ready = readdirSync(join(root, "target", "queue", "ready")).filter((f) => f.endsWith(".md")); } catch { ready = []; }
  for (const f of ready) {
    const core = basename(f, ".md").replace(/^\d+-/, "");
    if (!core || /^\d+$/.test(core)) continue;
    let started = 0, done = 0;
    for (const line of logLines) {
      if (!line.includes(core)) continue;
      const at = Date.parse(line.slice(0, 24));
      if (!Number.isFinite(at) || Date.now() - at > 24 * 3_600_000) continue;
      if (/\b(batch:.*(ran|started)|started)\b/.test(line)) started += 1;
      if (/\bcompleted\b/.test(line)) done += 1;
    }
    if (started >= 3 && done === 0) {
      park.push({ id: `PARK-${core}`, text: `- split one-off ${core} started ${started}x in 24h with 0 completions comes back as pieces never whole` });
    }
  }
  for (const { packet, pieces, note } of PARKED_PIECES) {
    if (existsSync(join(root, "target", "queue", "parked", `${packet}.md`)) && !existsSync(join(root, "target", "queue", "pieces", `${pieces}.md`))) {
      park.push({ id: `PARK-${packet}`, text: `- split parked ${packet} comes back as pieces ${pieces} ${note}` });
    }
  }
  return park;
}

// Steal rows: one STEAL-<part> per part with fewer than 3 open lines in
// docs/plan/steals.md, lowest open count first. Row text carries the seat's
// match tag: [hunt] for look, net, speed; [frontier] for bodies and futures;
// [ai-scout] for aidoor.
const STEAL_PARTS = [
  { part: "look", tag: "[hunt]", lane: "steal", re: /look|render|gpu|meshlet|temporal|shadow|taa|pcss|cull|\bVG\d|\bQ1\b|bevy.*taa|voidin|meshoptimizer/i },
  { part: "net", tag: "[hunt]", lane: "steal", re: /multiplayer|\bnet\b|rollback|replicat|ggrs|lightyear|\bMP\d|\bON\d|visibility|naia|renet/i },
  { part: "speed", tag: "[hunt]", lane: "steal", re: /speed|frame|profile|sccache|nextest|sweep|bench|opt-level|opt\b|\bPF\d|\bOPT\d/i },
  { part: "bodies", tag: "[frontier]", lane: "steal", re: /bod|ragdoll|anim|destruct|gore|melee|cloth|fem|soft bod|physics|avian|jolt|active-ragdoll|kickback|\bBX\d|\bPX\d/i },
  { part: "aidoor", tag: "[ai-scout]", lane: "steal", re: /ai.?door|mcp|asset|generat|spatial|provider|blender|triposr|trellis|objaverse|\bSP\d|\bAX\d|\bEV\d|\bEA\d/i },
  { part: "future", tag: "[frontier]", lane: "steal", re: /world model|gaussian|splat|retopo|neural|diffusion|tts|voice| Harris |urban/i },
];

function openStealLines(root) {
  const text = safeRead(join(root, "docs", "plan", "steals.md")) ?? "";
  const lines = text.split("\n");
  const open = [];
  for (let i = 0; i < lines.length; i++) {
    if (/^\d{4}-\d\d-\d\d \| .* \| open\s*$/.test(lines[i])) {
      open.push(`${lines[i]} ${(lines[i + 1] ?? "").trim()}`);
    }
  }
  return open;
}

function gatherSteals(root) {
  const open = openStealLines(root);
  const per = new Map(STEAL_PARTS.map((p) => [p.part, []]));
  const unassigned = [];
  for (const line of open) {
    const hit = STEAL_PARTS.find((p) => p.re.test(line));
    if (hit) per.get(hit.part).push(line);
    else unassigned.push(line);
  }
  for (const line of unassigned) per.get("future").push(line);
  const steals = [...per]
    .filter(([, lines]) => lines.length < 3)
    .sort((a, b) => a[1].length - b[1].length || STEAL_PARTS.findIndex((p) => p.part === a[0]) - STEAL_PARTS.findIndex((p) => p.part === b[0]))
    .map(([part, lines]) => {
      const def = STEAL_PARTS.find((p) => p.part === part);
      const oldest = lines.length ? `${lines.length} open oldest ${lines[0].slice(0, 60)}` : "0 open lines land the oldest card first";
      return { id: `STEAL-${part}`, role: "researcher", lane: def.lane, text: `- steal ${def.tag} ${part} ${oldest}`.slice(0, 140) };
    });
  // One STEAL-future row per STUCK topic with no open line yet.
  const frontier = safeRead("C:/empire/center/research/FRONTIER-COVERAGE.md")
    ?? safeRead(join(root, "docs", "research", "FRONTIER-COVERAGE.md")) ?? "";
  const stuckAt = frontier.search(/Spike next \(STUCK/);
  if (stuckAt >= 0) {
    const section = frontier.slice(stuckAt, stuckAt + 6000);
    for (const m of section.matchAll(/^\|\s*([^|]+?)\s*\|/gm)) {
      const topic = m[1].trim();
      if (!topic || /topic|---/i.test(topic) || topic.length > 60) continue;
      if (open.some((l) => l.toLowerCase().includes(topic.toLowerCase().slice(0, 12)))) continue;
      const slug = topic.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 24);
      if (!slug) continue;
      steals.push({ id: `STEAL-future-${slug}`, role: "researcher", lane: "steal", text: `- steal [frontier] future STUCK with no open line ${topic}`.slice(0, 140) });
    }
  }
  return steals;
}

function isoWeek(date) {
  const d = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
  const day = (d.getUTCDay() + 6) % 7;
  d.setUTCDate(d.getUTCDate() - day + 3);
  const first = new Date(Date.UTC(d.getUTCFullYear(), 0, 4));
  const week = 1 + Math.round(((d - first) / 86_400_000 - 3 + ((first.getUTCDay() + 6) % 7)) / 7);
  return { year: d.getUTCFullYear(), week };
}

function gatherClips() {
  const { year, week } = isoWeek(new Date());
  const id = `CLIP-${year}W${week}`;
  const dir = `C:/empire/autonomous-factory/marketing/campaign-studio/from-engine2040/${year}W${week}/`;
  if (existsSync(dir)) return [];
  return [{ id, role: "pilot", lane: "clip", text: `- clip 20 second arena clip for ${year}W${week} films the newest landed look` }];
}

function gatherTwins(root) {
  const twins = [];
  const parity = safeRead(join(root, "docs", "plan", "rome-spec.md")) ?? "";
  const open = parity.split("\n").filter((l) => /^- \[ \]/.test(l));
  open.forEach((line, i) => {
    twins.push({ id: `TWIN-${i + 1}`, role: "researcher", lane: "twin", text: `- twin parity item ${line.replace(/^- \[ \] /, "").replace(/\|/g, "/").trim()}`.slice(0, 140) });
  });
  if (!open.length && !existsSync(join(root, "docs", "plan", "rome-spec.json"))) {
    twins.push({ id: "TWIN-0", role: "researcher", lane: "twin", text: "- twin write docs/plan/rome-spec.json the same Rome game as forge" });
  }
  return twins;
}

function gatherCompacts(root) {
  const compacts = [];
  const inbox = safeRead(join(root, "docs", "EMPIRE-INBOX.md")) ?? "";
  const cap = /at most (\d+) items open/i.exec(inbox);
  const openEb = (inbox.match(/^- \[ \] EB-/gm) ?? []).length;
  if (cap && openEb > Number(cap[1])) {
    compacts.push({ id: "COMPACT-inbox", role: "planner", lane: "split", text: `- split inbox ${openEb} open over cap ${cap[1]} merge or tick down to the cap` });
  }
  const ladder = safeRead(join(root, "docs", "research", "LADDER.md")) ?? "";
  const cards = (ladder.match(/^- CARD \|/gm) ?? []).length || (ladder.match(/^- CARD /gm) ?? []).length;
  if (cards > 60) {
    compacts.push({ id: "COMPACT-ladder", role: "planner", lane: "split", text: `- split ladder holds ${cards} plain CARD lines fold stale ones to the archive` });
  }
  const atlas = safeRead(join(root, "docs", "plan", "ATLAS.md")) ?? "";
  const s6 = atlas.split(/^## 6\./m)[1]?.split(/^## \d+\./m)[0] ?? "";
  if (/docs\/plan\/|backtick|`[a-z-]+`/.test(s6) && /^\d+\. `/m.test(s6)) {
    compacts.push({ id: "COMPACT-atlas", role: "planner", lane: "split", text: `- split atlas section 6 refresh queue lists a plan deepen the top line` });
  }
  return compacts;
}

function gatherPieces(root) {
  const pieces = new Map();
  let files = [];
  try { files = readdirSync(join(root, "target", "queue", "pieces")).filter((f) => f.endsWith(".md")); } catch { return pieces; }
  for (const f of files) {
    const id = basename(f, ".md");
    const parsed = parsePieces(safeRead(join(root, "target", "queue", "pieces", f)));
    if (parsed.length) pieces.set(id, parsed);
  }
  return pieces;
}

function gatherCommits(root) {
  try {
    return execFileSync("git", ["log", "--format=%s", "-2000"], { cwd: root, encoding: "utf8", timeout: 60_000 });
  } catch { return ""; }
}

export function gather(root) {
  let boardRaw = null;
  try { boardRaw = loadBoardText(root); } catch { boardRaw = null; }
  boardRaw ??= safeRead(join(root, "docs", "SPRINT.md"));
  if (!boardRaw) throw new Error(`board missing: ${join(root, "docs", "SPRINT.md")}`);
  // Done rows moved to docs/archive/SPRINT-done-*.md count as ticked for needs: (9d7e9102).
  const { tasks, order } = parseBoard(boardRaw.split("## Receipts")[0], archivedDoneRows(root));
  const focusText = safeRead(join(root, "docs", "FOCUS.md")) ?? "";
  // Speed: verdict files and the keeper log are read once and shared by
  // salvage/cap-cuts/park (was: 1000+ files and the 500KB log read twice).
  // The git log (1.5 s) runs only when a pieces file needs its DONE check.
  const verdicts = readVerdictTexts(root);
  const keeperLog = safeRead(join(root, "target", "loop-keeper.log")) ?? "";
  const pieces = gatherPieces(root);
  const { salvage, held } = gatherSalvage(root, verdicts);
  // A salvage that holds a board row dispatches before tool-lane salvage,
  // then newest patch first (today's VS1 cut leads).
  for (const s of salvage) s.holdsRow = tasks.has(s.lane.toUpperCase());
  salvage.sort((a, b) => Number(b.holdsRow) - Number(a.holdsRow) || b.at - a.at);
  const { counts } = gatherCapCuts(root, verdicts, keeperLog);
  const capCut = new Set([...counts].filter(([, n]) => n >= 2).map(([id]) => id));
  const park = gatherPark(root, keeperLog);
  for (const id of capCut) {
    if (tasks.has(id) && !tasks.get(id).done && !park.some((p) => p.id === `PARK-${id}`)) {
      park.push({ id: `PARK-${id}`, text: `- split ${id} cut at the cap twice in 24h comes back as pieces never whole` });
    }
  }
  const splitIds = new Set();
  for (const id of order) {
    const task = tasks.get(id);
    if (!task || task.done || held.has(id)) continue;
    const text = task.lines.join(" ");
    if (/\bSize L\b/.test(text) || (/\bSize M\b/.test(text) && ownedOf(task).length > 3)) splitIds.add(id);
  }
  return {
    tasks, order,
    topIds: parseTopLine(focusText),
    pieces,
    commitsText: pieces.size ? gatherCommits(root) : "",
    salvage, held, splitIds,
    park,
    capCut, capCuts: counts,
    steals: gatherSteals(root),
    clips: gatherClips(),
    twins: gatherTwins(root),
    compacts: gatherCompacts(root),
  };
}

function seatFrontmatter(text) {
  const m = /^---\r?\n([\s\S]*?)\r?\n---/.exec(String(text ?? ""));
  const fm = {};
  if (!m) return fm;
  for (const line of m[1].split(/\r?\n/)) {
    const kv = /^([A-Za-z-]+):\s*(.*)$/.exec(line);
    if (kv) fm[kv[1]] = kv[2].trim();
  }
  return fm;
}

function activeClaims(root) {
  const ids = new Set();
  const text = safeRead(join(root, "target", "queue", "claims.txt")) ?? "";
  for (const line of text.split(/\r?\n/)) {
    const cells = line.split("|").map((c) => c.trim());
    const at = Date.parse(cells[2]);
    if (cells.length >= 3 && cells[0] && (!Number.isFinite(at) || Date.now() - at < 2 * 3_600_000)) ids.add(cells[0]);
  }
  return ids;
}

const baseRole = (role) => String(role ?? "").trim().split(/\s+/)[0].replace(/-paid$/, "");

// A VS row is look work even when its Lane column says content or split
// (VS1 pieces own content/ so they dispatch as content).
export function seatSelects(row, readyMatch) {
  if (!readyMatch) return true;
  if (row.text.includes(readyMatch)) return true;
  if (String(readyMatch).includes("| look |") && idPrefix(row.ID) === "VS") return true;
  return false;
}

// --check: parse the file the way the keeper does and print, per seat file,
// how many items its ready-role and ready-match select.
export function check(root) {
  const out = join(root, "target", "queue", "lane-board.md");
  const raw = safeRead(out);
  if (raw == null) { process.stderr.write(`lane-board: error: board file missing: ${out}\n`); return 1; }
  const board = keeperRows(raw);
  if (!board) { process.stderr.write("lane-board: error: no ID/Status/Role table\n"); return 1; }
  const claims = activeClaims(root);
  let seats = [];
  try { seats = readdirSync(join(root, "target", "queue", "standing")).filter((f) => f.endsWith(".md")).sort(); } catch { /* none */ }
  let fail = 0;
  const need = { look: 0, body: 0, ai: 0, net: 0, speed: 0 };
  for (const f of seats) {
    const fm = seatFrontmatter(safeRead(join(root, "target", "queue", "standing", f)));
    if (fm.ready !== "board") { console.log(`${f}: not a board seat (ready: ${fm.ready ?? "none"})`); continue; }
    const items = board.filter((row) => row.ID && ["READY", "TOP"].includes(row.Status) && !claims.has(row.ID)
      && baseRole(row.Role) === baseRole(fm["ready-role"])
      && seatSelects(row, fm["ready-match"]));
    console.log(`${f}: ${items.length} item(s)${items.length ? ` first: ${items[0].ID}` : ""}`);
    for (const lane of Object.keys(need)) {
      if ((fm["ready-match"] ?? "").includes(`| ${lane} |`)) need[lane] = Math.max(need[lane], items.length);
    }
  }
  for (const [lane, n] of Object.entries(need)) {
    if (n < 1) { process.stderr.write(`lane-board: error: no item for ${lane} today\n`); fail = 1; }
  }
  console.log(`RESULT ${fail ? "FAIL" : "PASS"}: look ${need.look}, body ${need.body}, ai ${need.ai}, net ${need.net}, speed ${need.speed}`);
  return fail;
}


function main(argv) {
  const args = [...argv];
  let root = DEFAULT_ROOT;
  let mode = "write";
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === "--root") { root = args[i + 1] ?? ""; i += 1; if (!root) { process.stderr.write("lane-board: error: --root needs a dir\n"); process.exit(2); } }
    else if (a === "--check") mode = "check";
    else if (a === "--help" || a === "-h") { process.stdout.write(HELP); process.exit(0); }
    else { process.stderr.write(`lane-board: error: unknown flag ${a}\n\n${HELP}`); process.exit(2); }
  }
  if (mode === "check") process.exit(check(root));
  let data;
  try {
    data = gather(root);
  } catch (e) {
    process.stderr.write(`lane-board: error: ${e.message}\n`);
    process.exit(1);
  }
  const text = buildBoard(data);
  const out = join(root, "target", "queue", "lane-board.md");
  mkdirSync(join(root, "target", "queue"), { recursive: true });
  writeFileSync(out, text);
  const rows = keeperRows(text) ?? [];
  const ready = rows.filter((r) => ["READY", "TOP"].includes(r.Status));
  const byLane = new Map();
  for (const r of ready) byLane.set(r.Lane, (byLane.get(r.Lane) ?? 0) + 1);
  console.log(`lane-board: ${rows.length} rows, ${ready.length} READY/TOP -> ${out}`);
  console.log(`lanes: ${[...byLane].map(([k, v]) => `${k} ${v}`).join(", ")}`);
  for (const [seat, match] of [["builder-look", "| look |"], ["builder-body", "| body |"], ["builder-ai-door", "| ai |"], ["builder-net", "| net |"], ["builder-speed", "| speed |"], ["splitter", "| split |"]]) {
    const first = rows.find((r) => ["READY", "TOP"].includes(r.Status) && r.text.includes(match));
    console.log(`first ${seat}: ${first ? `${first.ID} [${first.Status}]` : "(none)"}`);
  }
}

if (import.meta.main) {
  main(process.argv.slice(2));
}

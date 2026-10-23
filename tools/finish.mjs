// FS1 finish scoreboard: reads the gate tables in docs/FINISH-RICH.md
// (A, B, C and P, with their "Proven by" rows) plus the board ticks in
// docs/SPRINT.md, then prints every gate A1-A10, B, C1-C5, P1-P10 as
// met, partial or open with the rows still open and its evidence line.
// A gate whose rows are all ticked but carries no evidence line prints
// partial, never met (P10 evidence bar).
//
// The board prune moved finished rows into docs/archive/, so a row's tick
// is read from the live board and from the archive: the prune must not
// drop a met gate. The live board wins when it still carries the id.
//
// Usage from the repo/lane root:
//   node tools/finish.mjs [--finish <path>] [--sprint <path>] [--archive <dir>] [--json]
//   node --test tools/finish.test.mjs
import { spawnSync } from "node:child_process";
import { readFileSync, existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { loadBoardText, parseBoard, ticksOf } from "./board-slice.mjs";
import { auditRows, buildContext } from "./ticked-rows-audit.mjs";

export const GATES = [
  "A1", "A2", "A3", "A4", "A5", "A6", "A7", "A8", "A9", "A10",
  "B",
  "C1", "C2", "C3", "C4", "C5",
  "P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8", "P9", "P10",
];

// Gate row sets that FINISH-RICH does not spell out: the A gates (their status
// text names rows only sometimes) and the C gates (their table has no Proven by
// column). The live status text is parsed first and only fills gaps. Every row
// here must resolve to the live board or the archive; finish.test.mjs fails
// otherwise. The P entries mirror the Proven by column and are checked against
// it, so a tree without that table still prints every gate.
export const FALLBACK_ROWS = {
  A1: [],
  A2: ["A1", "A2", "A3", "AS1", "AS2", "CT7", "CT8"],
  A3: ["E1", "E2", "E3", "ED1", "ED2", "ED3", "SU1", "ED4", "ED5"],
  A4: ["GX1", "GX2", "GX3", "GX4", "GX5", "GX6", "GX7", "GX8", "GX9", "GX10", "CT7", "CT8", "Q1"],
  A5: ["WIN1", "MP1", "MP2", "MP3", "GP1", "GP3", "PX1", "DX1", "GM3"],
  A6: ["MCP5", "EA1a", "EA1b", "EA1c", "EA1d", "EA1e", "EV1", "FK1"],
  A7: ["BR2", "PV3", "QA1", "CR1"],
  A8: ["GM3"],
  A9: ["OB3", "AH1", "AH2", "AH3", "AH4", "AH5", "HL2", "FZ1"],
  A10: ["HR1", "HR2", "HR3", "HR4", "HR5", "TU1", "DOC1", "FX3"],
  B: [],
  C1: ["GX1", "GX2", "GX3", "GX4", "GX5", "GX6", "GX7", "GX8", "GX9", "GX10"],
  C2: ["RT0", "RT1", "RT2", "RT3", "RT4", "RT5", "RT6", "RT7", "RT8"],
  C3: ["VG1", "VG2", "VG3", "VG4"],
  C4: ["ON1", "ON2", "ON3", "ON4", "ON5", "ON6", "ON7", "ON8"],
  C5: ["WD1", "WD2", "WD3", "WD4", "WD5", "WD6", "CT9"],
  // P fallbacks mirror the Finish P "Proven by" column in docs/FINISH-RICH.md
  // (root tree 2026-09-24) so stale trees without that table still print rows.
  P1: ["GX1", "GX2", "GX3", "GX4", "GX5", "GX6", "GX7", "GX8", "GX9", "GX10", "RL4", "RL5", "RL6", "RL7", "RP2", "RP3", "RP4", "RT0", "RT1", "RT2", "RT3", "RT4", "RT5", "RT6", "RT7", "RT8", "VG1", "VG2", "VG3", "VG4", "VX1", "VX2", "VX3", "VX4", "VX5", "Q1", "PQ4"],
  P2: ["LM2", "AM2", "AM3", "AM4", "AM5", "AM6", "AM9", "AM10", "PX4", "PQ1"],
  P3: ["GP1", "GP2", "GP3", "GP4", "GP5", "GA1", "GA2", "PQ2"],
  P4: ["MP2", "MP3", "MP4", "MP5", "ON1", "ON2", "ON3", "ON4", "ON5", "ON6", "ON7", "ON8"],
  P5: ["AH1", "AH2", "AH3", "AH4", "AH5", "FZ1", "PQ3"],
  P6: ["PF1", "PF2", "PF3", "PF4", "PF5", "PF6", "PF7", "PF8", "PF9"],
  P7: ["ED2", "ED3", "ED4", "ED5", "ED6", "EV1", "AB1", "AB2", "AB3", "AB4", "LW1", "LW2", "LW3", "LW4", "DM1"],
  P8: ["SL1", "SL2", "SL3"],
  P9: ["WIN1", "WIN2", "WIN3", "WIN4", "UIX1", "UIX2", "UIX3", "UIX4", "GM3"],
  P10: ["FS1"],
};

// Verdict-dropped rows: left out of the gates while the live board does not
// reopen them. GX5 (wave-56 verdict in docs/SPRINT-03-finish-line.md: gtao.rs,
// not skinning; DROP 078-gx5-r2) has no live row, but the GX1-GX10 range and
// the fallback sets still name it. If the live board reopens it, it is counted
// again and finish.mjs warns.
export const DROPPED_ROWS = new Set(["GX5"]);

// The board prune moved finished rows out of docs/SPRINT.md and into
// docs/archive/ (SPRINT-done-*.md, the older SPRINT-2026-10-05.md snapshot
// and docs/SPRINT-ARCHIVE.md). A row the archive ticks is done, so the
// gates score it from the archive tick instead of printing it open: the
// prune must not drop a met gate.
//
// The live board wins when it still carries the row. An id the board
// reopened (or reuses for new work) is scored from the board, never from
// the older archive tick: the board is the newer record.
//
// The landed-code check that a hand-kept allowlist used to carry is the
// row audit (ticked-rows-audit): a ticked row whose code is gone is a
// suspect, and a gate with a suspect row never prints met.

const ARCHIVE_DONE = /^SPRINT-done-.*\.md$/;

// Done files first, then the older snapshots, then the board-diet file beside
// the archive dir: the same first-wins order ticked-rows-audit.tickedRows uses.
function archiveFiles(dir) {
  let names;
  try {
    names = readdirSync(dir).filter((f) => f.endsWith(".md"));
  } catch {
    return [];
  }
  names.sort((a, b) => (ARCHIVE_DONE.test(b) - ARCHIVE_DONE.test(a)) || a.localeCompare(b));
  const files = names.map((n) => join(dir, n));
  const sibling = join(dir, "..", "SPRINT-ARCHIVE.md");
  if (existsSync(sibling)) files.push(sibling);
  return files;
}

// Ticks and row texts from the same read. PF8's text only lives in
// docs/SPRINT-ARCHIVE.md, so a tick read that drops that file hides the row.
export function readArchive(archiveDir) {
  const ticks = new Map();
  const texts = new Map();
  for (const file of archiveFiles(archiveDir)) {
    let md;
    try {
      md = readFileSync(file, "utf8").replace(/\r\n/g, "\n");
    } catch {
      continue;
    }
    for (const [id, row] of parseBoard(md).tasks) {
      if (row.done) ticks.set(id, true);
      if (!texts.has(id)) texts.set(id, row.lines.join(" "));
    }
  }
  return { ticks, texts };
}

// Kept for finish.test.mjs and any other existing caller.
export function ticksFromArchive(archiveDir) {
  return readArchive(archiveDir).ticks;
}

// The 2026-10-22 board cut moved 437 open rows off the board into
// docs/plan/parked-2026-10-22.md. They are known rows that are not done:
// the owner parked them, so the gates still count them open, but the
// scoreboard must not call them unresolved (a row on neither the board
// nor the archive is a stale-alarm). The file lists a row as
// "- ID title:" with no checkbox, so it needs its own reader.
export const PARKED_FILE = "parked-2026-10-22.md";

// The parked ids: the row id and its text, from the parked file.
export function readParked(parkedPath) {
  let md;
  try {
    md = readFileSync(parkedPath, "utf8").replace(/\r\n/g, "\n");
  } catch {
    return new Map();
  }
  const out = new Map();
  for (const line of md.split("\n")) {
    const m = /^- ([A-Z][A-Z0-9]*\d[a-z]?)\b(.*)$/.exec(line);
    if (m && !out.has(m[1])) out.set(m[1], line.slice(2).trim());
  }
  return out;
}

// Suspects among the gate rows this tree can read: id -> { reasons, evidence }.
// The full ticked-rows-audit CLI reads every ticked row and walks git history
// (about 12 s); the scoreboard needs only the ~125 gate rows (about 0.3 s), so
// it imports the same auditRows instead of spawning that CLI. That audit also
// reads each suspect row's own closure note: a note that names a real commit, a
// run id, a path that exists or a ticked replacement row drops the finding; a
// note with no such evidence keeps it. An unticked row's owned path may not
// exist yet: that is the row's work, so a missing-path-only finding on an open
// row is dropped here. `tickedIds` is the whole ticked board,
// because a gate set alone is too narrow to tell whether a replacement row is
// ticked. ok=false means the repo context could not be built (no git); the
// caller decides whether that is a warning (default) or a failure (--check).
export function loadSuspects(root, texts, ids, tickedIds = null) {
  const ctx = buildContext(root);
  if (!ctx) return { ok: false, suspects: new Map(), audited: 0, unchecked: 0 };
  const rows = [...new Set(ids)]
    .filter((id) => texts.has(id))
    .map((id) => ({ id, source: "gate", text: texts.get(id) }));
  const report = auditRows(rows, ctx, tickedIds ? { tickedIds } : {});
  // An unticked row's owned path may not exist yet: that is the row's work, not
  // rot. Only a TICKED row can lose the code it was ticked for, so a
  // path-missing-only finding on an open row is dropped here. Anything else
  // (a duplicate, a test that no longer exists) still counts on either side.
  const keepsFinding = (s) =>
    tickedIds === null ||
    tickedIds.has(s.id) ||
    s.findings.some((f) => f.reason !== "path-missing");
  const suspects = new Map(
    report.suspects.filter(keepsFinding).map((s) => [
      s.id,
      {
        reasons: [...new Set(s.findings.map((f) => f.reason))],
        evidence: s.findings.map((f) => f.evidence).join(" | "),
      },
    ]),
  );
  return { ok: true, suspects, audited: rows.length, unchecked: report.unchecked };
}

function argPath(flag, fallback) {
  const i = process.argv.indexOf(flag);
  if (i !== -1 && process.argv[i + 1]) return process.argv[i + 1];
  return fallback;
}

function section(md, heading) {
  const start = md.indexOf(heading);
  if (start === -1) return "";
  const rest = md.slice(start + heading.length);
  const next = rest.search(/\n## /);
  return next === -1 ? rest : rest.slice(0, next);
}

function tableRows(sectionText) {
  return sectionText
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.startsWith("|"))
    .filter((l) => !/^\|[\s:-]+\|/.test(l));
}

// Gate cell "B, C, P" expands to B plus the whole C and P families.
function expandGateCell(cell) {
  const out = [];
  const has = (re) => re.test(cell);
  const singles = cell.match(/\b[ABCP][0-9]+\b/g) ?? [];
  for (const s of singles) if (!out.includes(s)) out.push(s);
  if (/\bB\b/.test(cell) && !out.includes("B")) out.push("B");
  if (has(/(^|[^A-Z])C([^0-9]|$)/)) {
    for (let i = 1; i <= 5; i++) if (!out.includes(`C${i}`)) out.push(`C${i}`);
  }
  if (has(/(^|[^A-Z])P([^0-9]|$)/)) {
    for (let i = 1; i <= 10; i++) if (!out.includes(`P${i}`)) out.push(`P${i}`);
  }
  return out;
}

function parseStatusTable(md) {
  const map = new Map();
  const sec = section(md, "## Finish-line status");
  for (const row of tableRows(sec)) {
    const cols = row.split("|").map((c) => c.trim());
    if (cols.length < 3) continue;
    // cols[0] is "" (leading pipe), cols[1] gate, cols[2] status.
    if (/gate/i.test(cols[1])) continue;
    const gates = expandGateCell(cols[1]);
    const status = cols[2];
    for (const g of gates) if (!map.has(g)) map.set(g, status);
  }
  return map;
}

export function parseProvenBy(md) {
  const map = new Map();
  const sec = section(md, "## Finish P");
  for (const row of tableRows(sec)) {
    const cols = row.split("|").map((c) => c.trim());
    if (cols.length < 4) continue;
    if (/gate/i.test(cols[1])) continue;
    const m = cols[1].match(/\bP[0-9]+\b/);
    if (!m) continue;
    map.set(m[0], cols[3]);
  }
  return map;
}

function ticksFromBoard(md) {
  // One parser for every SPRINT reader (board-slice.mjs): the old hand-rolled
  // parseBoardTicks agreed with it tick-for-tick on the live board (2026-10-02).
  return ticksOf(parseBoard(md).tasks);
}

const dash = (s) => s.replace(/[–—−]/g, "-");

function splitId(id) {
  const m = id.match(/^([A-Z]{1,4})(\d+)([a-z]?)$/);
  if (!m) return null;
  return { prefix: m[1], num: Number(m[2]), suffix: m[3] };
}

// Expand "GX1-GX10, EA1a-e, Q1" into ordered unique row ids.
// Handles full ranges (GX1-GX10), suffix-letter ranges (EA1a-e) and singles.
export function expandIds(raw) {
  if (!raw) return [];
  let text = dash(raw);
  const out = [];
  const push = (id) => {
    if (!out.includes(id)) out.push(id);
  };
  // Suffix-letter ranges first: EA1a-e.
  text = text.replace(
    /\b([A-Z]{1,4}\d+)([a-z])\s*-\s*([a-z])\b/g,
    (_, head, a, b) => {
      const p = splitId(`${head}${a}`);
      if (!p) return _;
      const lo = Math.min(a.charCodeAt(0), b.charCodeAt(0));
      const hi = Math.max(a.charCodeAt(0), b.charCodeAt(0));
      for (let c = lo; c <= hi; c++) push(`${head}${String.fromCharCode(c)}`);
      return " ";
    },
  );
  // Full ranges: GX1-GX10 (same prefix and suffix required).
  text = text.replace(
    /\b([A-Z]{1,4}\d+[a-z]?)\s*-\s*([A-Z]{1,4}\d+[a-z]?)\b/g,
    (_, a, b) => {
      const pa = splitId(a);
      const pb = splitId(b);
      if (!pa || !pb || pa.prefix !== pb.prefix || pa.suffix !== pb.suffix) {
        push(a);
        push(b);
        return " ";
      }
      const lo = Math.min(pa.num, pb.num);
      const hi = Math.max(pa.num, pb.num);
      for (let n = lo; n <= hi; n++) push(`${pa.prefix}${n}${pa.suffix}`);
      return " ";
    },
  );
  for (const m of text.matchAll(/\b([A-Z]{1,4}\d+[a-z]?)\b/g)) push(m[1]);
  return out;
}

function hasEvidence(statusText) {
  if (!statusText) return false;
  if (!/^\s*Met\s*:/i.test(statusText)) return false;
  if (/needs?\b/i.test(statusText)) return false;
  if (/open until\b/i.test(statusText)) return false;
  return true;
}

function oneLine(s, max = 220) {
  const t = (s ?? "").replace(/\s+/g, " ").trim();
  return t.length > max ? `${t.slice(0, max - 1)}…` : t;
}

export function scoreGate(gate, rows, ticks, statusText, suspects = new Map()) {
  const openRows = rows.filter((r) => !ticks.get(r));
  const suspectRows = rows
    .filter((r) => suspects.has(r))
    .map((id) => {
      const found = suspects.get(id);
      return typeof found === "string"
        ? { id, reason: found, evidence: "" }
        : { id, reason: found.reasons.join(", "), evidence: oneLine(found.evidence, 160) };
    });
  const evidence = hasEvidence(statusText);
  let status;
  if (rows.length === 0) status = evidence ? "met" : "open";
  else if (openRows.length === 0) status = evidence && suspectRows.length === 0 ? "met" : "partial";
  else if (openRows.length === rows.length) status = "open";
  else status = "partial";
  return {
    gate,
    status,
    rows,
    openRows,
    suspectRows,
    evidence: statusText ? oneLine(statusText) : "none",
  };
}

// One command per met gate. A gate with no headless command is `missing`, not
// green: the probe mode can never upgrade a gate and never fakes a pass.
export const PROBES = {
  A1: { cmd: "node", args: ["tools/proof.mjs", "--skip-build"], marker: "RESULT PASS \\(proof fixtures only\\)", timeoutMs: 1_800_000 },
  A2: { missing: "needs the editor window and a GPU: recorded --record-a2 (FINISH-RICH A2)" },
  A3: { missing: "needs the editor window, a GPU and the child crash drill: recorded --record-a3 (FINISH-RICH A3)" },
  A4: { missing: "needs reference-GPU stills and the motion review (FINISH-RICH A4)" },
  A5: { missing: "needs a packaged 5-10 minute LAN session (FINISH-RICH A5)" },
  A6: { cmd: "cargo", args: ["test", "-p", "engine-playtest", "--lib", "eval"], marker: "test result: ok\\.", minPassed: 1, timeoutMs: 900_000 },
  A7: { cmd: "cargo", args: ["test", "-p", "engine-playtest", "seeded_bug"], marker: "test result: ok\\.", minPassed: 1, timeoutMs: 900_000 },
  A8: { missing: "needs a clean Windows machine without the source checkout (FINISH-RICH A8)" },
  A9: { cmd: "cargo", args: ["test", "-p", "engine-proof", "--all-features", "--lib", "a9"], marker: "test result: ok\\.", minPassed: 1, timeoutMs: 1_800_000 },
  A10: { cmd: "cargo", args: ["test", "-p", "engine-editor", "--all-features", "tuning"], marker: "test result: ok\\.", minPassed: 1, timeoutMs: 1_800_000 },
  B: { missing: "needs a second game, dedicated-server loss/jitter runs and a one-hour soak (FINISH-RICH B)" },
  C1: { missing: "needs a real-device submit per ticked render row (FINISH-RICH C1)" },
  C2: { missing: "needs the RT tier on the reference GPU (FINISH-RICH C2)" },
  C3: { missing: "needs virtual geometry and the 20k crowd on the GPU (FINISH-RICH C3)" },
  C4: { missing: "needs internet clients through the relay (FINISH-RICH C4)" },
  C5: { missing: "needs streamed maps and a 10-minute 64-slot siege (FINISH-RICH C5)" },
  P1: { missing: "needs GPU stills, ghost/shimmer tests and the review panel (FINISH-RICH P1)" },
  P2: { missing: "needs the release-clock latency leg and the feel fixtures in one run (FINISH-RICH P2)" },
  P3: { cmd: "cargo", args: ["test", "-p", "engine-playtest", "--all-features", "tournament", "--", "--test-threads=1"], marker: "test result: ok\\.", minPassed: 1, timeoutMs: 1_800_000 },
  P4: { missing: "needs 64 clients under RTT and loss (FINISH-RICH P4)" },
  P5: { missing: "needs a one-hour soak and the 10k fuzz corpus (FINISH-RICH P5)" },
  P6: { missing: "needs CPU/GPU frame p95 on the reference GPU (FINISH-RICH P6)" },
  P7: { missing: "needs six suite commands plus the 15-minute newcomer run (FINISH-RICH P7)" },
  P8: { missing: "needs janitor sweeps, clippy -D warnings and a stub scan; no one command prints that verdict (FINISH-RICH P8)" },
  P9: { missing: "needs a packaged exe on a clean Windows machine (FINISH-RICH P9)" },
  P10: { cmd: "node", args: ["--test", "--test-reporter=tap", "tools/finish.test.mjs"], marker: "# fail 0\\b", minPassed: 1, timeoutMs: 180_000 },
};

export function parseProbes(text) {
  const raw = JSON.parse(text);
  const out = {};
  for (const [gate, entry] of Object.entries(raw)) {
    if (!GATES.includes(gate)) throw new Error(`unknown gate ${gate}`);
    if (entry.missing) {
      out[gate] = { missing: String(entry.missing) };
      continue;
    }
    if (typeof entry.cmd !== "string" || !Array.isArray(entry.args) || typeof entry.marker !== "string") {
      throw new Error(`${gate}: needs cmd, args and marker`);
    }
    out[gate] = {
      cmd: entry.cmd,
      args: entry.args.map(String),
      marker: entry.marker,
      timeoutMs: entry.timeoutMs ?? 120_000,
      minPassed: entry.minPassed,
    };
  }
  return out;
}

export function runProbe(entry, { cwd = process.cwd(), env = process.env } = {}) {
  if (!entry || entry.missing) {
    return { state: "missing", why: entry?.missing ?? "no probe for this gate" };
  }
  const bin = entry.cmd === "node" ? process.execPath : entry.cmd;
  const cmd = [entry.cmd, ...(entry.args ?? [])].join(" ");
  const started = Date.now();
  const result = spawnSync(bin, entry.args ?? [], {
    cwd,
    env,
    encoding: "utf8",
    timeout: entry.timeoutMs ?? 120_000,
    maxBuffer: 32 * 1024 * 1024,
    shell: false,
  });
  const ms = Date.now() - started;
  const out = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
  const base = { cmd, ms, exit: result.status };
  if (result.error) {
    return {
      ...base,
      state: result.error.code === "ETIMEDOUT" ? "timeout" : "fail",
      why: result.error.message,
    };
  }
  if (!new RegExp(entry.marker).test(out)) {
    return { ...base, state: "fail", why: `marker /${entry.marker}/ not found` };
  }
  const passed =
    [...out.matchAll(/(\d+) passed/g)].reduce((n, m) => n + Number(m[1]), 0) +
    [...out.matchAll(/^# pass (\d+)$/gm)].reduce((n, m) => n + Number(m[1]), 0);
  // A run that reports tests but ran none is never a pass: `test result: ok. 0
  // passed` (a filter that matched nothing) and TAP `# pass 0` both exit 0, so
  // the marker alone would grade a probe green having checked nothing. This
  // holds whatever minPassed says, including when a --probes fixture omits it.
  if (passed === 0 && /test result:|^# pass \d+$/m.test(out)) {
    return { ...base, state: "fail", why: "0 tests passed (the run checked nothing)" };
  }
  if (entry.minPassed !== undefined && passed < entry.minPassed) {
    return { ...base, state: "fail", why: `${passed} test(s) passed (min ${entry.minPassed})` };
  }
  if (result.status !== 0) {
    return { ...base, state: "fail", why: `exit ${result.status}`, tail: oneLine(out.slice(-300)) };
  }
  return { ...base, state: "pass", passed };
}

// `--check` is the scoreboard's own integrity test: every gate row resolves to
// the board or the archive, and no met gate carries a suspect ticked row. It
// does not require every gate met; the gates are the later stages' work.
export function checkVerdict(results, unresolved, audit) {
  const reasons = [];
  if (!audit.ok) reasons.push("row audit unavailable (git ls-files failed)");
  if (unresolved.size) {
    reasons.push(
      `${unresolved.size} gate row(s) on neither the board nor the archive: ${[...unresolved].join(", ")}`,
    );
  }
  for (const r of results) {
    if (r.status === "met" && r.suspectRows.length) {
      reasons.push(`${r.gate} met with suspect row(s): ${r.suspectRows.map((s) => s.id).join(", ")}`);
    }
  }
  return { pass: reasons.length === 0, reasons, audited: audit.audited, unchecked: audit.unchecked };
}

function probesFor(argv) {
  if (argv.includes("--probes") && !argv.includes("--probe")) {
    console.error("finish: --probes needs --probe");
    process.exit(2);
  }
  if (!argv.includes("--probe")) return {};
  const i = argv.indexOf("--probes");
  if (i === -1) return PROBES;
  const path = argv[i + 1];
  if (!path || !existsSync(path)) {
    console.error(`finish: probe fixture not found: ${path ?? "(missing path)"}`);
    process.exit(2);
  }
  try {
    return parseProbes(readFileSync(path, "utf8"));
  } catch (error) {
    console.error(`finish: bad probe fixture ${path}: ${error.message}`);
    process.exit(2);
  }
}

function main() {
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    console.log("Usage: node tools/finish.mjs [--finish <path>] [--sprint <path>] [--archive <dir>] [--parked <path>] [--json] [--no-suspects] [--probe] [--probes <path>] [--check]");
    process.exit(0);
  }
  const cwd = process.cwd();
  const finishPath = argPath("--finish", join(cwd, "docs", "FINISH-RICH.md"));
  const sprintPath = argPath("--sprint", join(cwd, "docs", "SPRINT.md"));
  const asJson = process.argv.includes("--json");
  for (const [label, p] of [["finish", finishPath], ["sprint", sprintPath]]) {
    if (!existsSync(p)) {
      console.error(`finish: missing ${label} file: ${p}`);
      process.exit(2);
    }
  }
  const finishMd = readFileSync(finishPath, "utf8");
  // Full board: index + shards via board-slice (big01 split). A custom
  // --sprint path keeps single-file behavior (read that file only).
  const defaultSprint = join(cwd, "docs", "SPRINT.md");
  const sprintMd = (sprintPath === defaultSprint)
    ? loadBoardText().replace(/\r\n/g, "\n")
    : readFileSync(sprintPath, "utf8");
  const checkMode = process.argv.includes("--check");
  const probeMode = process.argv.includes("--probe");
  if (checkMode && process.argv.includes("--no-suspects")) {
    console.error("finish: --check needs the row audit; drop --no-suspects");
    process.exit(2);
  }
  const probes = probeMode ? probesFor(process.argv) : {};

  const statusByGate = parseStatusTable(finishMd);
  const provenBy = parseProvenBy(finishMd);
  const board = parseBoard(sprintMd);
  const liveTicks = ticksOf(board.tasks);
  const ticks = new Map(liveTicks);
  const archiveDir = argPath("--archive", join(cwd, "docs", "archive"));
  const archive = readArchive(archiveDir);
  // The prune moved finished rows into the archive, so the archive tick is
  // the row's real state. The live board wins when it still carries the id:
  // a row the board reopened (or reuses for new work) is scored from the
  // board, never from the older archive tick.
  for (const [id, done] of archive.ticks) {
    if (done && !liveTicks.has(id)) ticks.set(id, true);
  }
  // A dropped row leaves the gates only while the live board does not reopen it.
  const liveOpen = (id) => liveTicks.get(id) === false;
  const parked = readParked(argPath("--parked", join(cwd, "docs", "plan", PARKED_FILE)));
  const texts = new Map([...board.tasks].map(([id, row]) => [id, row.lines.join(" ")]));
  for (const [id, text] of archive.texts) if (!texts.has(id)) texts.set(id, text);
  for (const [id, text] of parked) if (!texts.has(id)) texts.set(id, text);

  const unresolved = new Set();
  const droppedOpen = [];
  const resolved = GATES.map((gate) => {
    const statusText = statusByGate.get(gate) ?? "";
    let rows;
    if (gate.startsWith("P")) {
      rows = provenBy.has(gate) ? expandIds(provenBy.get(gate)) : [];
      if (rows.length === 0) rows = [...(FALLBACK_ROWS[gate] ?? [])];
      // "janitor sweeps" and prose in Proven by expand to nothing; keep rows honest.
    } else {
      // Generic "see their tables" status names the tool (FS1), not rows.
      // Specific status texts list board rows; keep only ids the board knows
      // so gate cross-references (e.g. "the look (A4)") stay in the evidence
      // line instead of masquerading as rows.
      rows = /see their tables/i.test(statusText) ? [] : expandIds(statusText).filter((id) => ticks.has(id));
      for (const r of FALLBACK_ROWS[gate] ?? []) if (!rows.includes(r)) rows.push(r);
    }
    for (const r of rows) if (DROPPED_ROWS.has(r) && liveOpen(r)) droppedOpen.push(`${gate}:${r}`);
    rows = rows.filter((r) => !DROPPED_ROWS.has(r) || liveOpen(r));
    // A row on neither the board nor the archive is counted open, and named.
    // A parked row is known: the owner took it off the board, so it counts
    // open without the stale-alarm warning.
    for (const r of rows) {
      if (!liveTicks.has(r) && !archive.ticks.get(r) && !parked.has(r)) unresolved.add(r);
    }
    return { gate, statusText, rows };
  });

  const audit = process.argv.includes("--no-suspects")
    ? { ok: true, suspects: new Map(), audited: 0, unchecked: 0, skipped: true }
    : loadSuspects(
        cwd,
        texts,
        resolved.flatMap((r) => r.rows),
        new Set([...ticks].filter(([, done]) => done).map(([id]) => id)),
      );
  if (!audit.ok) {
    console.error("finish: warning: row audit unavailable (git ls-files failed); gates score ticks only");
  }
  const results = resolved.map((r) => scoreGate(r.gate, r.rows, ticks, r.statusText, audit.suspects));

  if (unresolved.size) {
    console.error(`finish: warning: rows on neither the live board nor the archive, counted open: ${[...unresolved].join(", ")} — restore their tick or update FALLBACK_ROWS in tools/finish.mjs`);
  }
  if (droppedOpen.length) {
    console.error(`finish: warning: dropped rows the live board reopened, counted again: ${droppedOpen.join(", ")} — update DROPPED_ROWS in tools/finish.mjs`);
  }

  if (probeMode) {
    for (const r of results) {
      if (r.status !== "met") {
        r.probe = { state: "skipped", why: "gate is not met on the board" };
        continue;
      }
      r.probe = runProbe(probes[r.gate]);
      r.status = r.probe.state === "pass"
        ? "met"
        : r.probe.state === "missing"
          ? "probe-missing"
          : "probe-failed";
    }
  }

  const check = checkMode ? checkVerdict(results, unresolved, audit) : null;
  if (checkMode) process.exitCode = check.pass ? 0 : audit.ok ? 1 : 2;

  if (asJson) {
    console.log(JSON.stringify(checkMode ? { check, gates: results } : results, null, 2));
    return;
  }
  if (checkMode) {
    const metWithSuspect = results.filter((r) => r.status === "met" && r.suspectRows.length).length;
    console.log(
      `Check: ${check.pass ? "PASS" : "FAIL"} — ${results.length} gates, ${audit.audited} row(s) audited ` +
        `(${audit.unchecked} name no crate path or test), ${unresolved.size} unresolved, ` +
        `${metWithSuspect} met gates with a suspect row`,
    );
    for (const reason of check.reasons) console.log(`  - ${reason}`);
  }
  for (const r of results) {
    const open = r.openRows.length ? r.openRows.join(", ") : "none";
    const sus = r.suspectRows.length
      ? ` — suspect: ${r.suspectRows.map((s) => `${s.id} (${s.reason})`).join(", ")}`
      : "";
    const probe = r.probe && r.probe.state !== "skipped" ? ` — probe: ${r.probe.state}` : "";
    console.log(`${r.gate}: ${r.status} — open rows: ${open} (${r.openRows.length}/${r.rows.length}) — evidence: ${r.evidence}${sus}${probe}`);
  }
  if (!checkMode) {
    const met = results.filter((r) => r.status === "met").length;
    const partial = results.filter((r) => r.status === "partial").length;
    const open = results.filter((r) => r.status === "open").length;
    const missing = results.filter((r) => r.status === "probe-missing").length;
    const failed = results.filter((r) => r.status === "probe-failed").length;
    const extra = probeMode ? `, ${missing} probe-missing, ${failed} probe-failed` : "";
    console.log(`Summary: ${met} met, ${partial} partial, ${open} open${extra} (FS1 scoreboard)`);
  }
}

if (import.meta.main) main();

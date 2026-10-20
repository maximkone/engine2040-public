// FS1 finish scoreboard: reads the gate tables in docs/FINISH-RICH.md
// (A, B, C and P, with their "Proven by" rows) plus the board ticks in
// docs/SPRINT.md, then prints every gate A1-A10, B, C1-C5, P1-P10 as
// met, partial or open with the rows still open and its evidence line.
// A gate whose rows are all ticked but carries no evidence line prints
// partial, never met (P10 evidence bar).
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

// Rows ticked-and-pruned into docs/archive/ with landed code behind them:
// OB3 (crates/endpoint/src/log_tools.rs, endpoint lib green), DOC1 (doctor
// drills green on tools/doctor.mjs), FX3 (retired by design: report-first
// /fix is standing practice, no code row). Extended 2026-10-08 (verified
// archive tick + landed code + green tests each): LW3
// (docs/archive/SPRINT-2026-10-05.md ludus_npcs.rs), DM1 (ai_builds_arena.rs),
// PF4 (arena+workloads), PF6 (rust-lld receipt 121s->79s), PF7 (archive.rs),
// PF8 (server_budget.rs). Extended 2026-10-12 (verified archive tick in
// docs/archive/SPRINT-done-2026-10-10.md each): A1 (glTF import), A2
// (texture cook), A3 (skinned import), AS1 (bundle v1), AS2 (streaming),
// CT7 (hero+fighters), CT8 (arena kit). The live board no longer carries
// these rows, so score them from the archive ticks instead of printing them
// open. Membership here is the landed-code check; the archive tick must still
// be present. GX5 stays dropped (never resurrected).
// Extended 2026-10-12 (verified archive tick in docs/archive/SPRINT-done-2026-10-10.md each):
// E1 (dock shell), E2 (viewport pick+gizmo), E3 (save/reopen+play+journal),
// ED1 (editor exe), ED2 (inspector), ED3 (browser),
// ED4 (play-in-editor), ED5 (onboarding), SU1 (selective undo).
// Extended 2026-10-12 FS1 packet (verified archive tick + landed code + green lib test each):
// AM2 (SPRINT-done-2026-10-10.md:1057 space.rs, engine-anim interrupt_has_no_pop_then_settles+replay_is_identical_bytes_twice),
// AM3 (SPRINT-done-2026-10-10.md:1060 layers.rs, engine-anim am16_layers_replay_x2_and_restore_equal),
// AM5 (SPRINT-2026-10-05.md:897 pack.rs, engine-anim ct3_bake_replays_through_pack+unpack_returns_source_bit_exact),
// AM9 (SPRINT-done-2026-10-10.md:1536 fbik.rs, engine-anim reach_lands_inside_1cm_analytic_first+fixed_cost_and_64_fighter_scale),
// LW1 (SPRINT-done-2026-10-10.md:1791 mind.rs, engine-ai conformance_green_mock_local_remote+providers_off_replay_reproduces_checkpoints_x2),
// LW2 (SPRINT-done-2026-10-10.md:1797 memory.rs, engine-ai same_log_same_bytes_x2+soak_10k_holds_cap),
// GP2 (SPRINT-done-2026-10-10.md:1186 sweep.rs, engine-combat thin_fast_rapier_never_tunnels+replay_exact_twice_over_sweep_tape),
// GP3 (SPRINT-done-2026-10-10.md:1190 camera.rs, engine-game scripted_orbit_keeps_both_fighters_framed_600_ticks+gp19_trauma_replays_exactly_twice),
// PX4 (SPRINT-done-2026-10-10.md:1584 balance.rs, engine-physics balance_stagger_threshold_orders_five_strengths+balance_stagger_hash_stable_x2),
// HR3 (SPRINT-done-2026-10-10.md:208 tables.rs, engine-combat replacement_tables_validate_and_change_outcomes),
// HR4 (SPRINT-done-2026-10-10.md:211 curves/gladiator.rs, engine-ai replacement_curves_validate_and_change_decisions),
// SL2 (SPRINT-2026-10-05.md:1425 world+nav, engine-world 25 passed + engine-nav 93 passed),
// UIX2 (SPRINT-done-2026-10-10.md:153 settings.rs, engine-ui replay_reads_the_same_settings_on_the_same_tick_twice),
// FS1 (SPRINT-2026-10-05.md:449 tools/finish.mjs, node --test tools/finish.test.mjs hand-checked sample).
// Extended 2026-10-17 (owner-verified green; each archive tick checked in the
// file named): AM4, AM6, AH1, AH2, EA1a-e, FK1, BR2, PV3, QA1, CR1, GP1, GP4 in
// docs/archive/SPRINT-done-2026-10-10.md; SL1 in docs/archive/SPRINT-2026-10-05.md.
// PF7 and PF8 ticks sit in docs/SPRINT-ARCHIVE.md (board diet), which
// ticksFromArchive reads. finish.test.mjs fails a member with no archive tick.
export const ARCHIVE_TICKED_ROWS = new Set(["OB3", "DOC1", "FX3", "LW3", "DM1", "PF4", "PF6", "PF7", "PF8", "A1", "A2", "A3", "AS1", "AS2", "CT7", "CT8", "E1", "E2", "E3", "ED1", "ED2", "ED3", "SU1", "ED4", "ED5", "AM2", "AM3", "AM5", "AM9", "LW1", "LW2", "GP2", "GP3", "PX4", "HR3", "HR4", "SL2", "UIX2", "FS1", "AM4", "AM6", "AH1", "AH2", "EA1a", "EA1b", "EA1c", "EA1d", "EA1e", "FK1", "BR2", "PV3", "QA1", "CR1", "GP1", "GP4", "SL1"]);

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

// Suspects among the gate rows this tree can read: id -> { reasons, evidence }.
// The full ticked-rows-audit CLI reads every ticked row and walks git history
// (about 12 s); the scoreboard needs only the ~125 gate rows (about 0.3 s), so
// it imports the same auditRows instead of spawning that CLI. ok=false means
// the repo context could not be built (no git); the caller decides whether
// that is a warning (default) or a failure (--check).
export function loadSuspects(root, texts, ids) {
  const ctx = buildContext(root);
  if (!ctx) return { ok: false, suspects: new Map(), audited: 0, unchecked: 0 };
  const rows = [...new Set(ids)]
    .filter((id) => texts.has(id))
    .map((id) => ({ id, source: "gate", text: texts.get(id) }));
  const report = auditRows(rows, ctx);
  const suspects = new Map(
    report.suspects.map((s) => [
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
    console.log("Usage: node tools/finish.mjs [--finish <path>] [--sprint <path>] [--archive <dir>] [--json] [--no-suspects] [--probe] [--probes <path>] [--check]");
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
  for (const id of ARCHIVE_TICKED_ROWS) if (archive.ticks.get(id)) ticks.set(id, true);
  // A dropped row leaves the gates only while the live board does not reopen it.
  const liveOpen = (id) => liveTicks.get(id) === false;
  const texts = new Map([...board.tasks].map(([id, row]) => [id, row.lines.join(" ")]));
  for (const [id, text] of archive.texts) if (!texts.has(id)) texts.set(id, text);

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
    for (const r of rows) if (!liveTicks.has(r) && !archive.ticks.get(r)) unresolved.add(r);
    return { gate, statusText, rows };
  });

  const audit = process.argv.includes("--no-suspects")
    ? { ok: true, suspects: new Map(), audited: 0, unchecked: 0, skipped: true }
    : loadSuspects(cwd, texts, resolved.flatMap((r) => r.rows));
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

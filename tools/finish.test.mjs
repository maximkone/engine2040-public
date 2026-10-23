// FS1 behavioral sample: 5 hand-checked gates over fixture docs.
// A gate whose rows are all ticked but lacks an evidence line prints
// partial (the P10 evidence bar). Run: node --test tools/finish.test.mjs
import { spawnSync } from "node:child_process";
import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadBoardText, parseBoard, ticksOf } from "./board-slice.mjs";
import {
  DROPPED_ROWS,
  FALLBACK_ROWS,
  GATES,
  PARKED_FILE,
  checkVerdict,
  expandIds,
  loadSuspects,
  parseProbes,
  parseProvenBy,
  readArchive,
  readParked,
  runProbe,
  scoreGate,
  ticksFromArchive,
} from "./finish.mjs";

const REPO = process.cwd();
const FINISH = join(REPO, "tools", "finish.mjs");

const FINISH_MD = `# FINISH-RICH — fixture
## Finish A — usable developer alpha (M0–M3)
| Gate | Required demonstration |
|---|---|
| A1: reproducible foundation | Pinned toolchain and seeded replay |
| A6: AI in editor | Twenty fixed tasks across backends |
## Finish B — dependable desktop engine v1 (M4–M5)
- Second small game through public APIs.
## Finish C — the 2040 tier (M6)
| Gate | Required demonstration |
|---|---|
| C1: live look | Every ticked render row submits on a real device |
## Finish P — the polish bar
| Gate | The bar | Proven by |
|---|---|---|
| P5: stability | One-hour soak clean | AH1, PQ3 |
| P10: evidence | Every met gate has an evidence line | FS1 |
## Finish-line status
| Gate | Status 2026-09-24 (from the board) |
|---|---|
| A1 | Met: pinned toolchain, \`node tools/proof.mjs\` PASS with golden x2 |
| A6 | Rows ticked (MCP5, EA1a, EV1): needs one recorded run of the 20 tasks as its evidence line |
| B, C, P | Open: see their tables; FS1 prints the live status every wave |
| P5 | Open: soak harness landed (AH1); one-hour run open (PQ3) |
| P10 | Open: FS1 tool lands this row |
`;

const SPRINT_MD = `# SPRINT — fixture board
- [x] AH1 observe: supervisor and fault catalog.
- [ ] PQ3 proof: one-hour soak.
- [ ] FS1 tools: finish scoreboard.
- [x] MCP5 mcp: stdio door binary.
- [x] EA1a mcp: assistant loop skeleton.
- [x] EA1b mcp: preview plus spatial checks.
- [x] EA1c mcp: fork play-test before apply.
- [x] EA1d mcp: apply with approval and undo.
- [x] EA1e mcp: same loop through local and remote.
- [x] EV1 playtest: eval harness.
- [x] FK1 arena: counterfactual forks.
`;

function run(finishPath, sprintPath, extra = []) {
  const r = spawnSync(process.execPath, [FINISH, "--finish", finishPath, "--sprint", sprintPath, "--json", ...extra], {
    encoding: "utf8",
  });
  assert.equal(r.status, 0, `finish.mjs failed: ${r.stderr}${r.stdout}`);
  return JSON.parse(r.stdout);
}

test("a suspect row blocks met and names the reason", () => {
  const ticks = new Map([["A1", true]]);
  const suspects = new Map([["A1", "path-missing"]]);
  const gate = scoreGate("A1", ["A1"], ticks, "Met: pinned toolchain", suspects);
  assert.equal(gate.status, "partial", "a suspect row is never met");
  assert.deepEqual(gate.suspectRows, [{ id: "A1", reason: "path-missing", evidence: "" }]);
});

test("loadSuspects reads the in-process audit for the gate rows", () => {
  const texts = new Map([["ZZ1", "owns `crates/zz-fixture/src/gone.rs`"]]);
  const report = loadSuspects(process.cwd(), texts, ["ZZ1"]);
  assert.equal(report.ok, true);
  assert.ok(report.suspects.has("ZZ1"), "a missing path is a suspect");
  assert.match(report.suspects.get("ZZ1").reasons.join(","), /path-missing/);
});

test("loadSuspects drops a gate row only when its closure note names evidence", () => {
  const head = spawnSync("git", ["rev-parse", "--short", "HEAD"], { cwd: REPO, encoding: "utf8" }).stdout.trim();
  assert.match(head, /^[0-9a-f]{7,40}$/, "the fixture needs a real HEAD commit");
  const text = "owns `crates/zz-fixture/src/gone.rs`";
  const texts = new Map([
    ["ZZ1", `${text}. Tick audit 2026-10-19: the seam landed in ${head}.`],
    ["ZZ2", `${text}. Tick audit 2026-10-19: the seam landed in deadbee.`],
    ["ZZ3", `${text}. Tick audit 2026-10-19: closed by crates/render/src/lib.rs.`],
  ]);
  const ids = ["ZZ1", "ZZ2", "ZZ3"];
  const report = loadSuspects(REPO, texts, ids, new Set(ids));
  assert.equal(report.ok, true);
  assert.equal(report.suspects.has("ZZ1"), false, "a real commit clears the gate row");
  assert.equal(report.suspects.has("ZZ2"), true, "a bogus sha keeps it a suspect");
  assert.equal(report.suspects.has("ZZ3"), false, "an existing path clears it");
});

test("hand-checked sample: 5/5 gates correct", () => {
  const dir = join(tmpdir(), `finish-sample-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    writeFileSync(f, FINISH_MD);
    writeFileSync(s, SPRINT_MD);
    const empty = join(dir, "empty-archive");
    mkdirSync(empty, { recursive: true });
    const rows = run(f, s, ["--archive", empty]);
    const byGate = new Map(rows.map((r) => [r.gate, r]));

    // A1: no board rows, Met status with command+golden evidence -> met.
    assert.equal(byGate.get("A1").status, "met", "A1 met with evidence");
    assert.deepEqual(byGate.get("A1").openRows, []);

    // A6: every row ticked (MCP5, EA1a, EV1) but the status says it still
    // needs its recorded run -> partial, never met.
    assert.equal(byGate.get("A6").status, "partial", "A6 all ticked without evidence is partial");
    assert.deepEqual(byGate.get("A6").openRows, []);

    // P5: Proven by AH1 (ticked) + PQ3 (open) -> partial with PQ3 still open.
    assert.equal(byGate.get("P5").status, "partial", "P5 partial while PQ3 open");
    assert.deepEqual(byGate.get("P5").openRows, ["PQ3"]);

    // P10: Proven by FS1 (open) -> open with FS1 still open.
    assert.equal(byGate.get("P10").status, "open", "P10 open while FS1 open");
    assert.deepEqual(byGate.get("P10").openRows, ["FS1"]);

    // B: no Proven by rows and an Open status line -> open.
    assert.equal(byGate.get("B").status, "open", "B open with no rows");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("human output names every gate with open rows and evidence", () => {
  const dir = join(tmpdir(), `finish-human-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    writeFileSync(f, FINISH_MD);
    writeFileSync(s, SPRINT_MD);
    const r = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s], { encoding: "utf8" });
    assert.equal(r.status, 0);
    for (const g of ["A1:", "A6:", "P5:", "P10:", "B:"]) assert.match(r.stdout, new RegExp(g), `output prints ${g}`);
    assert.match(r.stdout, /PQ3/, "still-open row named");
    assert.match(r.stdout, /evidence:/, "evidence line printed");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("archive ticks close pruned rows (E7: OB3/DOC1/FX3), GX5 stays dropped", () => {
  const dir = join(tmpdir(), `finish-archive-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    mkdirSync(a, { recursive: true });
    writeFileSync(f, `# FINISH-RICH — archive fixture
## Finish-line status
| Gate | Status 2026-10-05 (from the board) |
|---|---|
| A9 | Met: recorded chaos + fuzz run seed 9, log digest landed over observe events |
| A10 | Met: edit-to-game timing, doctor drills green |
`);
    // Live board: every A9/A10 row ticked except OB3/DOC1/FX3, which were
    // pruned (no row at all) after landing.
    writeFileSync(s, `# SPRINT — fixture board, OB3/DOC1/FX3 pruned
- [x] AH1 observe: supervisor and fault catalog.
- [x] AH2 observe: more.
- [x] AH3 observe: more.
- [x] AH4 observe: more.
- [x] AH5 observe: more.
- [x] HL2 observe: more.
- [x] FZ1 observe: more.
- [x] HR1 timing: more.
- [x] HR2 timing: more.
- [x] HR3 timing: more.
- [x] HR4 timing: more.
- [x] HR5 timing: more.
- [x] TU1 timing: more.
`);
    writeFileSync(join(a, "SPRINT-2026-10-05.md"), `# SPRINT archive — fixture
- [x] OB3 mcp: \`log.digest\` and cursor-paged \`log.query\` tools.
- [x] DOC1 ops: harden \`tools/doctor.mjs\`.
- [x] FX3 ops: harden \`/fix\` report-first.
`);
    // Archive tick + landed code (the E7 set): A9/A10 score met.
    const met = run(f, s, ["--archive", a]);
    const byMet = new Map(met.map((r) => [r.gate, r]));
    assert.equal(byMet.get("A9").status, "met", "A9 met from archive OB3 tick");
    assert.deepEqual(byMet.get("A9").openRows, []);
    assert.equal(byMet.get("A10").status, "met", "A10 met from archive DOC1/FX3 ticks");
    assert.deepEqual(byMet.get("A10").openRows, []);
    // No archive: pruned rows read open again (partial, never silently met).
    const empty = join(dir, "empty-archive");
    mkdirSync(empty, { recursive: true });
    const open = run(f, s, ["--archive", empty]);
    const byOpen = new Map(open.map((r) => [r.gate, r]));
    assert.equal(byOpen.get("A9").status, "partial", "A9 partial without archive tick");
    assert.deepEqual(byOpen.get("A9").openRows, ["OB3"]);
    assert.equal(byOpen.get("A10").status, "partial", "A10 partial without archive ticks");
    assert.deepEqual(byOpen.get("A10").openRows, ["DOC1", "FX3"]);
    // GX5 stays dropped: never listed as a row, even with archive ticks.
    for (const rows of [met, open]) {
      for (const r of rows) {
        assert.ok(!r.rows.includes("GX5"), `${r.gate} never names GX5`);
        assert.ok(!r.openRows.includes("GX5"), `${r.gate} never leaves GX5 open`);
      }
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the archive tick closes a pruned row the live board dropped (E7)", () => {
  const dir = join(tmpdir(), `finish-archive-read-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    mkdirSync(a, { recursive: true });
    writeFileSync(f, `# FINISH-RICH — archive-read fixture
## Finish-line status
| Gate | Status 2026-10-05 (from the board) |
|---|---|
| A9 | Met: recorded chaos + fuzz run seed 9, log digest landed over observe events |
`);
    // The live board carries AH1 and AH2 ticked; AH3 was pruned into the
    // archive with its tick. No allowlist names AH3.
    writeFileSync(s, `# SPRINT — fixture board, AH3 pruned into the archive
- [x] AH1 observe: supervisor and fault catalog.
- [x] AH2 observe: more.
- [x] AH4 observe: more.
- [x] AH5 observe: more.
- [x] HL2 observe: more.
- [x] FZ1 observe: more.
`);
    writeFileSync(join(a, "SPRINT-done-2026-10-10.md"), `# SPRINT archive — fixture
- [x] AH3 audio: default-device change or loss reopens with backoff.
- [x] OB3 mcp: \`log.digest\` and cursor-paged \`log.query\` tools.
`);
    const rows = run(f, s, ["--archive", a]);
    const a9 = rows.find((r) => r.gate === "A9");
    assert.equal(a9.status, "met", "A9 met from the archive tick of the pruned AH3");
    assert.deepEqual(a9.openRows, [], "the pruned row is not open");
    assert.ok(a9.rows.includes("AH3"), "A9 still names AH3 as one of its rows");
    // No archive: the pruned row reads open again (partial, never silently met).
    const empty = join(dir, "empty-archive");
    mkdirSync(empty, { recursive: true });
    const open = run(f, s, ["--archive", empty]);
    const a9open = open.find((r) => r.gate === "A9");
    assert.equal(a9open.status, "partial", "A9 partial without the archive tick");
    assert.deepEqual(a9open.openRows, ["OB3", "AH3"], "both pruned rows read open");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the live board wins over an older archive tick for the same id", () => {
  const dir = join(tmpdir(), `finish-archive-reopen-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    mkdirSync(a, { recursive: true });
    writeFileSync(f, `# FINISH-RICH — reopen fixture
## Finish-line status
| Gate | Status 2026-10-05 (from the board) |
|---|---|
| A9 | Met: recorded chaos + fuzz run seed 9, log digest landed over observe events |
`);
    // The board reopened AH3 as open work; the older archive still ticks it.
    writeFileSync(s, `# SPRINT — fixture board, AH3 reopened
- [x] AH1 observe: supervisor and fault catalog.
- [x] AH2 observe: more.
- [ ] AH3 observe: reopened for new work.
- [x] AH4 observe: more.
- [x] AH5 observe: more.
- [x] HL2 observe: more.
- [x] FZ1 observe: more.
`);
    writeFileSync(join(a, "SPRINT-done-2026-10-10.md"), `# SPRINT archive — fixture
- [x] AH3 audio: the old, finished row.
`);
    const rows = run(f, s, ["--archive", a]);
    const a9 = rows.find((r) => r.gate === "A9");
    assert.equal(a9.status, "partial", "a reopened row is never met from an old archive tick");
    assert.deepEqual(a9.openRows, ["OB3", "AH3"], "the reopened row reads open");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a parked row counts open and is not called unresolved", () => {
  const dir = join(tmpdir(), `finish-parked-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    const p = join(dir, "parked.md");
    mkdirSync(a, { recursive: true });
    writeFileSync(f, `# FINISH-RICH — parked fixture
## Finish-line status
| Gate | Status 2026-10-22 (from the board) |
|---|---|
| C2 | Open: see their tables; FS1 prints the live status every wave |
`);
    // The board cut took RT2 off the board; it is parked, not done.
    writeFileSync(s, `# SPRINT — fixture board, RT2 parked
- [x] RT0 render: the base tier.
- [x] RT1 render: the ray-traced path.
- [x] RT3 render: many-light direct light.
- [x] RT4 render: the denoiser.
- [x] RT5 render: the temporal filter.
- [x] RT6 render: the bounce pass.
- [x] RT7 render: the reference path tracer.
- [x] RT8 render: the fallback.
`);
    writeFileSync(p, `# Parked rows
- RT2 render: ray-traced shadows - parked by the 2026-10-22 cut.
`);
    const empty = join(dir, "empty-archive");
    mkdirSync(empty, { recursive: true });
    // Parked: RT2 counts open, and no warning names it.
    const r = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s, "--archive", empty, "--parked", p, "--json"], {
      encoding: "utf8",
    });
    assert.equal(r.status, 0, `finish.mjs failed: ${r.stderr}${r.stdout}`);
    const c2 = JSON.parse(r.stdout).find((g) => g.gate === "C2");
    assert.deepEqual(c2.openRows, ["RT2"], "the parked row counts open");
    assert.doesNotMatch(r.stderr, /RT2/, "a parked row is not called unresolved");
    // The stale-alarm still names the rows that are on neither board, archive
    // nor parked file (the fixture leaves the other gates' rows out).
    assert.match(r.stderr, /neither the live board nor the archive/, "the other missing rows are still named");
    // Not parked and nowhere else: the row is named on stderr.
    const missing = join(dir, "missing-parked.md");
    const r2 = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s, "--archive", empty, "--parked", missing, "--json"], {
      encoding: "utf8",
    });
    assert.equal(r2.status, 0);
    assert.match(r2.stderr, /RT2/, "a row on neither the board, the archive nor the parked file is named");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a row on neither the board nor the archive counts open and is named on stderr", () => {
  const dir = join(tmpdir(), `finish-missing-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const empty = join(dir, "empty-archive");
    mkdirSync(empty, { recursive: true });
    // ZZ9 is named by P5's Proven by column and exists nowhere.
    writeFileSync(f, FINISH_MD.replace("| P5: stability | One-hour soak clean | AH1, PQ3 |", "| P5: stability | One-hour soak clean | AH1, PQ3, ZZ9 |"));
    writeFileSync(s, SPRINT_MD);
    const r = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s, "--archive", empty, "--json"], {
      encoding: "utf8",
    });
    assert.equal(r.status, 0, `finish.mjs failed: ${r.stderr}${r.stdout}`);
    assert.doesNotThrow(() => JSON.parse(r.stdout), "stdout stays JSON");
    const p5 = JSON.parse(r.stdout).find((g) => g.gate === "P5");
    assert.deepEqual(p5.openRows, ["PQ3", "ZZ9"], "missing row is counted open, never hidden");
    assert.match(r.stderr, /neither the live board nor the archive/, "warning names the unresolved rows");
    assert.match(r.stderr, /ZZ9/, "warning names ZZ9");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a dropped row the live board reopens is counted, not hidden", () => {
  const dir = join(tmpdir(), `finish-reopen-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const empty = join(dir, "empty-archive");
    mkdirSync(empty, { recursive: true });
    writeFileSync(f, FINISH_MD);
    writeFileSync(s, `${SPRINT_MD}- [ ] GX5 render: gtao reopened by a new verdict.\n`);
    const r = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s, "--archive", empty, "--json"], {
      encoding: "utf8",
    });
    assert.equal(r.status, 0, `finish.mjs failed: ${r.stderr}${r.stdout}`);
    // A4 names GX5 through its fallback set; the open board row keeps it.
    const a4 = JSON.parse(r.stdout).find((g) => g.gate === "A4");
    assert.ok(a4.openRows.includes("GX5"), "reopened GX5 is open, not filtered");
    assert.match(r.stderr, /dropped rows the live board reopened/, "reopened drop warns");
    assert.match(r.stderr, /GX5/, "reopened drop names GX5");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// Live-tree checks: the scoreboard must agree with the board shards, the
// archive (docs/archive plus docs/SPRINT-ARCHIVE.md) and FINISH-RICH as they
// stand in this repo. They assert invariants, not counts, so a normal board
// edit does not break them, but a stale row does.
function liveBoard() {
  return ticksOf(parseBoard(loadBoardText().replace(/\r\n/g, "\n")).tasks);
}

function liveScoreboard() {
  const r = spawnSync(process.execPath, [FINISH, "--json"], { cwd: REPO, encoding: "utf8" });
  assert.equal(r.status, 0, `finish.mjs failed on the live tree: ${r.stderr}`);
  return { rows: JSON.parse(r.stdout), stderr: r.stderr };
}

test("live tree: the scoreboard prints no warning", () => {
  const { stderr } = liveScoreboard();
  assert.equal(stderr, "", `unexpected warning on the live tree: ${stderr}`);
});

test("live tree: every gate row is on the live board, in the archive or parked", () => {
  const { rows } = liveScoreboard();
  const live = liveBoard();
  const archive = ticksFromArchive(join(REPO, "docs", "archive"));
  const parked = readParked(join(REPO, "docs", "plan", PARKED_FILE));
  for (const g of rows) {
    for (const r of g.rows) {
      assert.ok(
        live.has(r) || archive.get(r) || parked.has(r),
        `${g.gate} row ${r} is on neither the board nor the archive, and is not parked`,
      );
    }
  }
});

test("live tree: a parked row counts open and never ticks", () => {
  const { rows } = liveScoreboard();
  const live = liveBoard();
  const parked = readParked(join(REPO, "docs", "plan", PARKED_FILE));
  assert.ok(parked.size > 0, "the parked file holds the rows the 2026-10-22 cut took off the board");
  for (const g of rows) {
    for (const r of g.rows) {
      if (!parked.has(r)) continue;
      assert.notEqual(live.get(r), true, `${g.gate} parks ${r} but the board ticks it`);
      assert.ok(g.openRows.includes(r), `${g.gate} parks ${r} but does not count it open`);
    }
  }
});

test("live tree: a row the live board ticks never prints open", () => {
  const { rows } = liveScoreboard();
  const live = liveBoard();
  for (const g of rows) {
    for (const r of g.openRows) {
      assert.notEqual(live.get(r), true, `${g.gate} prints ${r} open but the board ticks it`);
    }
  }
});

test("live tree: a row the archive ticks and the board dropped never prints open", () => {
  const { rows } = liveScoreboard();
  const live = liveBoard();
  const archive = ticksFromArchive(join(REPO, "docs", "archive"));
  let honoured = 0;
  for (const g of rows) {
    for (const r of g.openRows) {
      if (live.has(r)) continue;
      assert.notEqual(archive.get(r), true, `${g.gate} prints ${r} open but the archive ticks it`);
    }
  }
  // The prune moved the A6/A9/A10/P3 rows out of the board: the archive
  // tick is what closes them, so the scoreboard must honour it.
  for (const id of ["MCP5", "EV1", "AH3", "AH4", "HR1", "HR2", "GP5", "OB3", "FS1"]) {
    assert.equal(archive.get(id), true, `${id} has no archive tick`);
    assert.notEqual(live.has(id), true, `${id} is still on the live board`);
    honoured += 1;
  }
  assert.equal(honoured, 9);
});

test("live tree: the live board wins over an older archive tick for the same id", () => {
  const { rows } = liveScoreboard();
  const live = liveBoard();
  const archive = ticksFromArchive(join(REPO, "docs", "archive"));
  for (const g of rows) {
    for (const r of g.openRows) {
      // A row the board reopened is scored from the board, never from the
      // older archive tick that shares its id.
      if (live.get(r) === false && archive.get(r) === true) {
        assert.fail(`${g.gate} prints ${r} open although the board reopened it`);
      }
    }
  }
});

test("live tree: DROPPED_ROWS never print open while the board leaves them unreopened", () => {
  const { rows } = liveScoreboard();
  const live = liveBoard();
  for (const id of DROPPED_ROWS) {
    assert.notEqual(live.get(id), false, `${id} is open on the live board; update DROPPED_ROWS`);
    for (const g of rows) assert.ok(!g.openRows.includes(id), `${g.gate} prints dropped ${id} open`);
  }
});

test("live tree: each P fallback set mirrors its Proven by column", () => {
  const md = readFileSync(join(REPO, "docs", "FINISH-RICH.md"), "utf8").replace(/\r\n/g, "\n");
  const provenBy = parseProvenBy(md);
  for (const gate of GATES.filter((g) => g.startsWith("P"))) {
    const proven = provenBy.has(gate) ? expandIds(provenBy.get(gate)) : [];
    assert.deepEqual(
      [...(FALLBACK_ROWS[gate] ?? [])].sort(),
      [...proven].sort(),
      `${gate} FALLBACK_ROWS differ from its Proven by column`,
    );
  }
});

test("archive ticks close A2 pruned rows (A1/A2/A3/AS1/AS2/CT7/CT8)", () => {
  const dir = join(tmpdir(), `finish-a2-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    mkdirSync(a, { recursive: true });
    writeFileSync(f, `# FINISH-RICH — A2 fixture
## Finish-line status
| Gate | Status 2026-10-12 (from the board) |
|---|---|
| A2 | Met: recorded editable-scene run ev7-wave56 (EV7, wave 56). |
`);
    // Live board: A2 rows pruned (no rows at all) after landing.
    writeFileSync(s, `# SPRINT — fixture board, A2 rows pruned
- [x] EV7 proof: recorded editable-scene run.
`);
    writeFileSync(join(a, "SPRINT-done-2026-10-10.md"), `# SPRINT archive — fixture
- [x] A1 assets: glTF mesh and material import.
- [x] A2 assets: texture cook with mips and sRGB rules.
- [x] A3 assets: skinned mesh, skeleton and clip import.
- [x] AS1 assets: content bundle v1.
- [x] AS2 assets: streaming.
- [x] CT7 content: hero and fighters.
- [x] CT8 content: arena kit.
`);
    const rows = run(f, s, ["--archive", a]);
    const byGate = new Map(rows.map((r) => [r.gate, r]));
    assert.equal(byGate.get("A2").status, "met", "A2 met from archive ticks");
    assert.deepEqual(byGate.get("A2").openRows, []);
    const r = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s, "--archive", a], {
      encoding: "utf8",
    });
    assert.equal(r.status, 0);
    const line = r.stdout.split("\n").find((l) => l.startsWith("A2:"));
    assert.ok(line.startsWith("A2: met"), "A2 gate line starts with A2: met");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("archive ticks close A3 pruned rows (E1/E2/E3/ED1/ED2/ED3/SU1/ED4/ED5)", () => {
  const dir = join(tmpdir(), `finish-a3-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    mkdirSync(a, { recursive: true });
    writeFileSync(f, `# FINISH-RICH — A3 fixture
## Finish-line status
| Gate | Status 2026-10-12 (from the board) |
|---|---|
| A3 | Met: recorded editor run ev8-wave56 (EV8, wave 56). |
`);
    // Live board: A3 rows pruned (no rows at all) after landing.
    writeFileSync(s, `# SPRINT — fixture board, A3 rows pruned
- [x] EV8 proof: recorded editor run.
`);
    writeFileSync(join(a, "SPRINT-done-2026-10-10.md"), `# SPRINT archive — fixture
- [x] E1 editor: dock shell over command batches.
- [x] E2 editor: viewport picking and gizmo to batch.
- [x] E3 editor: save and reopen, child-process play, durable journal.
- [x] ED1 editor: editor executable.
- [x] ED2 editor: schema inspector.
- [x] ED3 editor: asset browser.
- [x] SU1 editor: selective undo with conflict reports.
- [x] ED4 editor: play-in-editor.
- [x] ED5 editor: onboarding.
`);
    const rows = run(f, s, ["--archive", a]);
    const byGate = new Map(rows.map((r) => [r.gate, r]));
    assert.equal(byGate.get("A3").status, "met", "A3 met from archive ticks");
    assert.deepEqual(byGate.get("A3").openRows, []);
    const r = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s, "--archive", a], {
      encoding: "utf8",
    });
    assert.equal(r.status, 0);
    const line = r.stdout.split("\n").find((l) => l.startsWith("A3:"));
    assert.ok(line.startsWith("A3: met"), "A3 gate line starts with A3: met");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("archive-only ticked row in the set counts as ticked (P10/FS1)", () => {
  const dir = join(tmpdir(), `finish-p10-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    mkdirSync(a, { recursive: true });
    writeFileSync(f, `# FINISH-RICH — P10 fixture
## Finish-line status
| Gate | Status 2026-10-04 (from the board) |
|---|---|
| P10 | Met: evidence-line audit 2026-10-04 (PQ14). |
## Finish P — the polish bar
| Gate | The bar | Proven by |
|---|---|---|
| P10: evidence | Every met gate has an evidence line | FS1 |
`);
    // Live board: FS1 pruned (no row at all) after landing.
    writeFileSync(s, `# SPRINT — fixture board, FS1 pruned
- [x] PQ14 proof: evidence-line audit.
`);
    writeFileSync(join(a, "SPRINT-2026-10-05.md"), `# SPRINT archive — fixture
- [x] FS1 tools: finish scoreboard.
`);
    // Archive tick + landed code (the FS1 set entry): P10 scores met.
    const met = run(f, s, ["--archive", a]);
    const byMet = new Map(met.map((r) => [r.gate, r]));
    assert.equal(byMet.get("P10").status, "met", "P10 met from archive FS1 tick");
    assert.deepEqual(byMet.get("P10").openRows, []);
    // No archive: pruned row reads open again (partial, never silently met).
    const empty = join(dir, "empty-archive");
    mkdirSync(empty, { recursive: true });
    const open = run(f, s, ["--archive", empty]);
    const byOpen = new Map(open.map((r) => [r.gate, r]));
    assert.equal(byOpen.get("P10").status, "open", "P10 open without archive tick");
    assert.deepEqual(byOpen.get("P10").openRows, ["FS1"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("runProbe enforces marker, exit and minPassed", () => {
  const ok = runProbe({
    cmd: process.execPath,
    args: ["-e", "console.log('PROBE OK')"],
    marker: "PROBE OK",
    timeoutMs: 5000,
  });
  assert.equal(ok.state, "pass");
  const badExit = runProbe({
    cmd: process.execPath,
    args: ["-e", "console.log('PROBE OK'); process.exit(3)"],
    marker: "PROBE OK",
    timeoutMs: 5000,
  });
  assert.equal(badExit.state, "fail");
  const noMarker = runProbe({
    cmd: process.execPath,
    args: ["-e", "console.log('nope')"],
    marker: "PROBE OK",
    timeoutMs: 5000,
  });
  assert.equal(noMarker.state, "fail");
  const zeroTests = runProbe({
    cmd: process.execPath,
    args: ["-e", "console.log('test result: ok. 0 passed')"],
    marker: "test result: ok\\.",
    minPassed: 1,
    timeoutMs: 5000,
  });
  assert.equal(zeroTests.state, "fail", "a hollow green is not a pass");
  // The same hollow run through a probe entry with no minPassed (a --probes
  // fixture, or a entry that only names a marker): still red, never a pass.
  const hollow = runProbe({
    cmd: process.execPath,
    args: ["-e", "console.log('running 0 tests'); console.log('test result: ok. 0 passed; 0 failed; 0 filtered out')"],
    marker: "test result: ok\\.",
    timeoutMs: 5000,
  });
  assert.equal(hollow.state, "fail", "a run that checked nothing fails without minPassed");
  assert.match(hollow.why, /0 tests/);
  const tapHollow = runProbe({
    cmd: process.execPath,
    args: ["-e", "console.log('# pass 0'); console.log('# fail 0')"],
    marker: "# fail 0\\b",
    timeoutMs: 5000,
  });
  assert.equal(tapHollow.state, "fail", "TAP `# pass 0` is a hollow green too");
  assert.equal(runProbe({ missing: "needs a GPU" }).state, "missing");
  const timed = runProbe({
    cmd: process.execPath,
    args: ["-e", "setTimeout(() => {}, 5000)"],
    marker: "never",
    timeoutMs: 150,
  });
  assert.equal(timed.state, "timeout");
});

test("parseProbes rejects an unknown gate and keeps a missing reason", () => {
  assert.throws(
    () => parseProbes('{"ZZ9":{"cmd":"node","args":[],"marker":"x"}}'),
    /unknown gate/,
  );
  assert.deepEqual(parseProbes('{"A1":{"missing":"needs a GPU"}}').A1, {
    missing: "needs a GPU",
  });
});

test("checkVerdict fails a met gate that still names a suspect row", () => {
  const check = checkVerdict(
    [{ gate: "P10", status: "met", suspectRows: [{ id: "PF8" }] }],
    new Set(),
    { ok: true, audited: 1, unchecked: 0 },
  );
  assert.equal(check.pass, false);
  assert.match(check.reasons.join(" "), /PF8/);
});

test("--check passes on the live tree", () => {
  const pass = spawnSync(process.execPath, [FINISH, "--check", "--json"], {
    encoding: "utf8",
  });
  assert.equal(pass.status, 0, pass.stderr);
  const parsed = JSON.parse(pass.stdout);
  assert.equal(parsed.check.pass, true);
  assert.ok(Array.isArray(parsed.gates));
  assert.match(pass.stderr, /^$/);
});

test("archive row text finds a stale path even when the tick sits in SPRINT-ARCHIVE.md", () => {
  const dir = join(tmpdir(), `finish-archive-text-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    const a = join(dir, "archive");
    mkdirSync(a, { recursive: true });
    writeFileSync(
      f,
      `# FINISH-RICH — archive text fixture
## Finish-line status
| Gate | Status 2026-10-05 |
|---|---|
| P10 | Met: evidence-line audit |
## Finish P - the polish bar
| Gate | The bar | Proven by |
|---|---|---|
| P10: evidence | Every met gate has an evidence line | FS1 |
`,
    );
    writeFileSync(s, `# SPRINT — FS1 pruned
- [x] PQ14 proof: evidence-line audit.
`);
    writeFileSync(join(a, "SPRINT-2026-10-05.md"), "# old archive\n");
    writeFileSync(
      join(dir, "SPRINT-ARCHIVE.md"),
      "- [x] FS1 tools: owns `crates/zz-fixture/src/gone.rs`.\n",
    );
    const rows = run(f, s, ["--archive", a]);
    const p10 = rows.find((r) => r.gate === "P10");
    assert.equal(p10.status, "partial", "a suspect row cannot be met");
    assert.equal(p10.suspectRows[0].id, "FS1");
    assert.match(p10.suspectRows[0].reason, /path-missing/);
    const archive = readArchive(a);
    assert.equal(archive.ticks.get("FS1"), true, "SPRINT-ARCHIVE.md is read for ticks");
    assert.match(archive.texts.get("FS1"), /crates\/zz-fixture/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

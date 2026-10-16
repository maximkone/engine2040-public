// FS1 behavioral sample: 5 hand-checked gates over fixture docs.
// A gate whose rows are all ticked but lacks an evidence line prints
// partial (the P10 evidence bar). Run: node --test tools/finish.test.mjs
import { spawnSync } from "node:child_process";
import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

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

test("fallback use prints a warning on stderr, stdout stays JSON", () => {
  const dir = join(tmpdir(), `finish-warn-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  try {
    const f = join(dir, "FINISH-RICH.md");
    const s = join(dir, "SPRINT.md");
    writeFileSync(f, FINISH_MD);
    writeFileSync(s, SPRINT_MD);
    const r = spawnSync(process.execPath, [FINISH, "--finish", f, "--sprint", s, "--json"], {
      encoding: "utf8",
    });
    assert.equal(r.status, 0, `finish.mjs failed: ${r.stderr}${r.stdout}`);
    // Stdout stays pure JSON even with the warning.
    assert.doesNotThrow(() => JSON.parse(r.stdout), "stdout stays JSON");
    // Fallback rows back P1 (no Proven by row) so the warning must fire,
    // never silently wrong about the open set.
    assert.match(r.stderr, /warning/i, "warning fires when fallback is used");
    assert.match(r.stderr, /fallback/i, "warning names the fallback");
    assert.match(r.stderr, /FALLBACK_ROWS/, "warning points at FALLBACK_ROWS");
    // P1 fallback names GX5, which DROPPED_ROWS filters: that filtering warns too.
    assert.match(r.stderr, /dropped/i, "dropped-row filtering warns");
    assert.match(r.stderr, /GX5/, "dropped warning names GX5");
  } finally {
    rmSync(dir, { recursive: true, force: true });
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

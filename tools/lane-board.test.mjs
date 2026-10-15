// lane-board.test.mjs — fixture proof for tools/lane-board.mjs.
//
// Covers: a Size L row becomes `split`; a row with a pieces file shows pieces;
// a parked-section row is PARKED; a table with a `|` in a title still parses;
// FOCUS parity (every drift-check FOCUS regex has the same lane here); a TOP
// line in a fixture FOCUS.md puts its ids first with Status TOP; a
// still-blocked id says WAITS.
import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { buildBoard, gather, keeperRows as libRows, laneOf, parseTopLine, seatSelects } from "./lane-board.mjs";
import { parseBoard, rowReady } from "./board-slice.mjs";
import { mkdirSync, mkdtempSync as mkdtempRaw, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// Remove every temp folder this file makes when it exits (test temp folders piled up in %TEMP%).
const madeDirs = [];
const mkdtempSync = (prefix, options) => { const dir = mkdtempRaw(prefix, options); madeDirs.push(dir); return dir; };
process.on("exit", () => { for (const dir of madeDirs) try { rmSync(dir, { recursive: true, force: true }); } catch { /* still in use: leave it */ } });

// rows() copied from createSeatReadiness in .opencode/plugin/loop-keeper.js:
// --check must parse the file exactly the way the keeper does.
function keeperRowsCopy(raw) {
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

const BOARD = `### Rome first (owner 2026-10-02, \`docs/plan/render-look.md\`)

- [ ] BV9 viewer: big bevy | pipe dream — draws the kit. Tier A. Size L. Owns \`crates/viewer/**\`. needs: none.
- [ ] VS9 render: textured | draws — base color binds. Tier A. Size S. Owns \`crates/render/src/x.rs\`. needs: none.
- [ ] DT9 distill: dataset provenance — manifests only. Tier A. Size S. Owns \`crates/distill/src/data.rs\`. needs: DT1.
- [ ] HS9 proof: tape export — shards plus manifest. Tier B. Size M. Owns \`crates/proof/src/dataset.rs\`. needs: OB1, DT9.

### Platform and live AI (owner 2026-09-26, \`docs/archive/2026-10-10/plans/platform-live-ai.md\`)

- [ ] PLF9 platform: store build — packaged demo. Tier C. Size S. Owns \`tools/ship.mjs\`. needs: none.

### Done

- [x] DT1 arena: fork decision recorder. needs: none.
- [x] OB1 observe: typed events. needs: none.
`;

const FOCUS = `# FOCUS

TOP (owner 2026-10-04, Maxim in the center chat, game data first): DT9 HS9
`;

function fixture(extra = {}) {
  const { tasks, order } = parseBoard(BOARD);
  return {
    tasks, order,
    topIds: parseTopLine(FOCUS),
    pieces: new Map([["VS9", [{ piece: "a", rest: "crate paths test moves S" }, { piece: "b", rest: "crate paths test moves S" }]]]),
    commitsText: "",
    salvage: [{ lane: "vs1", bytes: 912 }],
    held: new Set(["VS1"]),
    splitIds: new Set(["BV9"]),
    park: [],
    capCut: new Set(),
    capCuts: new Map(),
    steals: [],
    clips: [],
    twins: [],
    compacts: [],
    ...extra,
  };
}

describe("lane-board", () => {
  it("writes the exact header and keeper-parseable rows", () => {
    const text = buildBoard(fixture());
    assert.ok(text.startsWith("| ID | Status | Role | Lane | Row |\n|---|---|---|---|---|\n"));
    const rows = keeperRowsCopy(text);
    assert.ok(rows && rows.length > 0);
    // The library copy agrees with the keeper copy.
    assert.deepEqual(libRows(text), rows);
  });

  it("Size L row becomes split for the planner", () => {
    const rows = keeperRowsCopy(buildBoard(fixture()));
    const bv9 = rows.filter((r) => r.ID === "BV9");
    assert.equal(bv9.length, 1);
    assert.equal(bv9[0].Status, "READY");
    assert.equal(bv9[0].Role, "planner");
    assert.ok(bv9[0].text.includes("| split |"));
  });

  it("a row with a pieces file shows pieces, only the first READY", () => {
    const rows = keeperRowsCopy(buildBoard(fixture()));
    assert.ok(!rows.some((r) => r.ID === "VS9"), "parent row is replaced by its pieces");
    const a = rows.find((r) => r.ID === "VS9.a");
    const b = rows.find((r) => r.ID === "VS9.b");
    assert.equal(a.Status, "READY");
    assert.equal(b.Status, "QUEUED");
  });

  it("a done piece stays done when a commit names it", () => {
    const rows = keeperRowsCopy(buildBoard(fixture({ commitsText: "land VS9.a green" })));
    assert.equal(rows.find((r) => r.ID === "VS9.a").Status, "DONE");
    assert.equal(rows.find((r) => r.ID === "VS9.b").Status, "READY");
  });

  it("a parked-section row is PARKED", () => {
    const rows = keeperRowsCopy(buildBoard(fixture()));
    const plf = rows.find((r) => r.ID === "PLF9");
    assert.equal(plf.Status, "PARKED");
  });

  it("a `|` in a title still parses with 5 cells", () => {
    const rows = keeperRowsCopy(buildBoard(fixture()));
    const bv9 = rows.find((r) => r.ID === "BV9");
    assert.ok(bv9 && !bv9.text.includes(" | pipe "));
    assert.ok(bv9.Row.includes("/"));
  });

  it("Row text is at most 140 chars and shaped `<Size> <section> <title>`", () => {
    const rows = keeperRowsCopy(buildBoard(fixture()));
    for (const r of rows) {
      assert.ok(r.Row.length <= 140, `${r.ID} row too long`);
      assert.match(r.Row, /^[SLM-] /);
    }
  });

  it("TOP ids come first, ready one TOP, blocked one WAITS", () => {
    const rows = keeperRowsCopy(buildBoard(fixture()));
    const ids = rows.map((r) => r.ID);
    assert.ok(ids.indexOf("DT9") < ids.indexOf("HS9"));
    assert.ok(ids.indexOf("HS9") < ids.indexOf("BV9"));
    assert.equal(rows.find((r) => r.ID === "DT9").Status, "TOP");
    assert.equal(rows.find((r) => r.ID === "HS9").Status, "WAITS DT9");
    assert.equal(rows.find((r) => r.ID === "DT9").Lane, "body");
    assert.equal(rows.find((r) => r.ID === "HS9").Lane, "body");
  });

  it("salvage stands while the row is held back from owners", () => {
    const rows = keeperRowsCopy(buildBoard(fixture()));
    const sal = rows.find((r) => r.ID === "SALVAGE-vs1");
    assert.equal(sal.Status, "READY");
    assert.equal(sal.Role, "planner");
    assert.ok(sal.text.includes("| split |"));
  });

  it("FOCUS parity: every drift-check FOCUS regex has the same lane here", () => {
    // Patterns copied from the FOCUS table in tools/drift-check.mjs §13.
    const focus = [
      ["look", /\b(?:VS|GX|RT|VG|VX|RL|RP)\d+|\bQ1\b/],
      ["body", /\b(?:BX|HS|PX|DX|AM|LM|TV)\d+|\bDT5\b/],
      ["ai", /\b(?:MCP|AX|EA|EV)\d+|\bSP(?:1[6-9]|[2-5]\d)\b/],
      ["speed", /\bOPT\d+|\bPF\d+|\bBENCH/],
      ["net", /\b(?:MP|ON)\d+/],
    ];
    const samples = {
      look: ["VS1", "GX3", "RT7", "VG1", "VX2", "RL5", "RP8", "Q1"],
      body: ["BX1", "HS15", "PX27", "DX6", "AM7", "LM2", "TV2", "DT5"],
      ai: ["MCP15", "AX11", "EA4", "EV15", "SP35", "SP19"],
      speed: ["OPT3", "PF5", "BENCH1"],
      net: ["MP6", "ON6"],
    };
    for (const [lane, re] of focus) {
      for (const id of samples[lane]) {
        assert.ok(re.test(id), `${id} should match the drift FOCUS ${lane} regex`);
        assert.equal(laneOf(id), lane, `${id} lane parity`);
      }
    }
    // Packet extensions beyond the drift regexes.
    for (const [id, lane] of [["BV1", "look"], ["Q2", "look"], ["PH1", "body"], ["AN1", "body"], ["CB3", "body"], ["SN11", "ai"], ["PQ1", "speed"], ["NET1", "net"], ["DET1", "net"], ["CT22", "content"], ["CQ5", "hygiene"], ["SL3", "hygiene"], ["GM3", "ship"], ["WIN9", "ship"], ["R6", "ship"]]) {
      assert.equal(laneOf(id), lane, `${id} extension lane`);
    }
    // Content ownership overrides the prefix, except VS stays look
    // (VS3 textures and VS5b clips name content/ as a data source).
    assert.equal(laneOf("VS9", ["content/arena/kit.json"]), "look");
    assert.equal(laneOf("VS3", ["content/arena/textures"]), "look");
    assert.equal(laneOf("BV1", ["content/arena/placement.json"]), "content");
    assert.equal(laneOf("BV1", ["crates/viewer/src/main.rs"]), "look");
  });

  it("clearing the TOP line takes the order away with no code edit", () => {
    const { tasks, order } = parseBoard(BOARD);
    const data = fixture({ topIds: parseTopLine("# FOCUS\n\nNo top line.\n"), tasks, order });
    const rows = keeperRowsCopy(buildBoard(data));
    assert.ok(!rows.some((r) => r.Status === "TOP"));
    assert.equal(rows.find((r) => r.ID === "DT9").Status, "READY");
  });

  it("## Now Rows: ids lead like a TOP line; the Old Now section is ignored; TOP-line ids follow", () => {
    const text = "# FOCUS\n\n## Now (owner 2026-10-10): four Core proofs\n\n1. Look. Rows: HS9 DT9\nOwner waits: PLF9.\n\n## Old Now (owner 2026-09-27)\nRows: BV9\n\nTOP (owner 2026-10-04, x): DT9 VS9\n";
    assert.deepEqual(parseTopLine(text), ["HS9", "DT9", "VS9"]);
    assert.deepEqual(parseTopLine("# FOCUS\n\n## Now\nno list\n"), []);
  });

  it("a Now row to split goes to the planner, a Now row with pieces waits for its pieces, none goes whole to a builder", () => {
    const rows = keeperRowsCopy(buildBoard(fixture({ topIds: parseTopLine("## Now\nRows: BV9 VS9 DT9\n") })));
    const ids = rows.map((r) => r.ID);
    assert.equal(ids[0], "BV9", "the Now order leads the table");
    assert.equal(rows[0].Role, "planner");
    assert.equal(rows[0].Status, "READY");
    assert.ok(rows[0].text.includes("| split |"));
    assert.equal(ids.filter((id) => id === "BV9").length, 1, "emitted once");
    assert.ok(!ids.includes("VS9"), "the parent of pieces is never handed out whole");
    assert.equal(rows.find((r) => r.ID === "DT9").Status, "TOP");
    assert.ok(ids.indexOf("DT9") < ids.indexOf("VS9.a"), "a Now id still leads the pieces");
  });

  it("a BLOCKED or waiting Size L row never wakes the splitter", () => {
    const board = `### Rome first (owner 2026-10-02, \`docs/plan/render-look.md\`)

- [ ] BV8 viewer: dropped spike - nothing. Tier A. Size M. Owns \`crates/viewer/a.rs\`, \`crates/viewer/b.rs\`, \`crates/viewer/c.rs\`, \`crates/viewer/d.rs\`. needs: none.
  BLOCKED: viewer dropped.
- [ ] Q8 render: waits on one - big. Tier A. Size L. Owns \`crates/render/x.rs\`. needs: RL8.
- [ ] RL8 render: open need - small. Tier A. Size S. Owns \`crates/render/y.rs\`. needs: none.
- [ ] VS8 render: cut me - big. Tier A. Size L. Owns \`crates/render/z.rs\`. needs: none.
`;
    const { tasks, order } = parseBoard(board);
    const rows = keeperRowsCopy(buildBoard(fixture({
      tasks, order, topIds: [], pieces: new Map(), salvage: [], held: new Set(), splitIds: new Set(["BV8", "Q8", "VS8"]),
    })));
    assert.equal(rows.find((r) => r.ID === "BV8").Status, "WAITS BLOCKED");
    assert.equal(rows.find((r) => r.ID === "Q8").Status, "WAITS RL8");
    assert.equal(rows.find((r) => r.ID === "VS8").Status, "READY");
    assert.equal(rows.filter((r) => r.Status === "READY" && r.Role === "planner").length, 1, "only VS8 can wake the splitter");
  });

  it("pieces of a BLOCKED row wait instead of going to builders", () => {
    const board = `### Rome first (owner 2026-10-02, \`docs/plan/render-look.md\`)

- [ ] VS7 render: blocked row with pieces - big. Tier A. Size L. Owns \`crates/render/q.rs\`. needs: none.
  BLOCKED: parked on purpose.
`;
    const { tasks, order } = parseBoard(board);
    const pieces = new Map([["VS7", [{ piece: "a", rest: "crate paths test moves S" }, { piece: "b", rest: "crate paths test moves S" }]]]);
    const rows = keeperRowsCopy(buildBoard(fixture({ tasks, order, topIds: [], pieces, salvage: [], held: new Set(), splitIds: new Set() })));
    assert.equal(rows.find((r) => r.ID === "VS7.a").Status, "WAITS BLOCKED");
    assert.equal(rows.find((r) => r.ID === "VS7.b").Status, "QUEUED");
    assert.ok(!rows.some((r) => r.Status === "READY"), "nothing is READY for any seat");
  });

  it("a VS piece in another lane still selects the look seat", () => {
    const vsContent = { ID: "VS1.a", text: "| VS1.a | READY | builder | content | - split piece VS1.a crate paths |" };
    const otherContent = { ID: "PX6.a", text: "| PX6.a | READY | builder | content | - split piece PX6.a crate paths |" };
    const lookRow = { ID: "RL20", text: "| RL20 | READY | builder | look | S render sky |" };
    assert.equal(seatSelects(vsContent, "| look |"), true);
    assert.equal(seatSelects(otherContent, "| look |"), false);
    assert.equal(seatSelects(lookRow, "| look |"), true);
    assert.equal(seatSelects(otherContent, "| body |"), false);
  });
});

describe("lane-board reads the archive (moved done rows are ticked)", () => {
  it("a needs: on a row moved to docs/archive counts as ticked, so the row is READY", () => {
    const root = mkdtempSync(join(tmpdir(), "lane-board-archive-"));
    mkdirSync(join(root, "docs", "archive"), { recursive: true });
    writeFileSync(join(root, "docs", "SPRINT.md"), "# SPRINT\n\n### Render\n\n- [ ] VS9 render: textured | draws. Tier A. Size S. Owns `crates/render/src/x.rs`. needs: DT7.\n");
    writeFileSync(join(root, "docs", "archive", "SPRINT-done-2026-10-10.md"), "# SPRINT done\n\n- [x] DT7 distill: moved to the archive. needs: none.\n");
    const data = gather(root);
    assert.equal(rowReady(data.tasks, data.tasks.get("VS9")), true, "VS9 needs DT7, which is ticked in the archive");
    assert.ok(!buildBoard(data).includes("WAITS DT7"), "no WAITS DT7 line for a row whose need is archived");
  });
});

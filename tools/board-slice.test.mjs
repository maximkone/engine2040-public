// Run: node --test tools/board-slice.test.mjs
// board-slice's list modes (--ready, --open, --find) on a small board, with the same readiness as sprint-loop.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync as mkdtempRaw, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { archivedDoneRows, blockedBy, boardShardFiles, focusAncestors, focusNowIds, loadBoardText, needsOf, parseBoard, pickNext, rowReady, ticksOf } from "./board-slice.mjs";

// Remove every temp folder this file makes when it exits (test temp folders piled up in %TEMP%).
const madeDirs = [];
const mkdtempSync = (prefix, options) => { const dir = mkdtempRaw(prefix, options); madeDirs.push(dir); return dir; };
process.on("exit", () => { for (const dir of madeDirs) try { rmSync(dir, { recursive: true, force: true }); } catch { /* still in use: leave it */ } });

const TOOL = join(dirname(fileURLToPath(import.meta.url)), "board-slice.mjs");
const BOARD = [
  "# SPRINT",
  "## Board",
  "### Done",
  "- [x] A1 core: the first row.",
  "### Finish line",
  "- [ ] B1 render: ready row, needs: A1.",
  "- [ ] B2 net: waits on B3, needs: B3.",
  "- [ ] B3 audio: itself open, needs: none.",
  "- [ ] B4 game: parked BLOCKED: no GPU.",
  "- [ ] B5 game: later LATER: after the merge.",
  "### Operator tasks",
  "- [ ] O1 owner: a task only the operator can do, needs: none.",
  "  second line of O1 mentions IBL.",
  "",
].join("\n");

const run = (args, focus = null) => {
  const root = mkdtempSync(join(tmpdir(), "board-slice-"));
  mkdirSync(join(root, "tools"));
  mkdirSync(join(root, "docs", "plan"), { recursive: true });
  copyFileSync(TOOL, join(root, "tools", "board-slice.mjs"));
  writeFileSync(join(root, "docs", "SPRINT.md"), BOARD);
  if (focus !== null) writeFileSync(join(root, "docs", "FOCUS.md"), focus);
  const r = spawnSync(process.execPath, [join(root, "tools", "board-slice.mjs"), ...args], { encoding: "utf8", cwd: root });
  return { code: r.status, out: r.stdout, err: r.stderr };
};

test("--ready lists what the loop would start: open, needs ticked, not BLOCKED, LATER or an operator row", () => {
  const { code, out } = run(["--ready"]);
  assert.equal(code, 0);
  const ids = out.split("\n").filter((l) => /^[A-Z]\d/.test(l)).map((l) => l.split(/\s+/)[0]);
  assert.deepEqual(ids, ["B1", "B3"]);
  assert.match(out, /^2 row\(s\)$/m);
  assert.match(out, /B1\s+open Finish line\s+render: ready row/);
});

test("--ready --open adds every open row with what it waits on", () => {
  const { out } = run(["--ready", "--open"]);
  assert.match(out, /B2 .*\[waits: B3\]/);
  assert.match(out, /B4 .*\[waits: BLOCKED\]/);
  assert.match(out, /B5 .*\[waits: LATER\]/);
  assert.match(out, /O1 .*\[waits: operator\]/);
  assert.doesNotMatch(out, /^A1 /m, "a done row is not open");
});


test("--open and --ready outputs stay disjoint slices", () => {
  const readyIds = run(["--ready"]).out.split("\n").filter((l) => /^[A-Z]\d/.test(l)).map((l) => l.split(/\s+/)[0]);
  const openOut = run(["--ready", "--open"]).out;
  const waitingIds = openOut.split("\n").filter((l) => l.includes("[waits:")).map((l) => l.split(/\s+/)[0]);
  assert.ok(waitingIds.length > 0, "open slice has waiting rows");
  assert.deepEqual(waitingIds.filter((id) => readyIds.includes(id)), [], "ready and waiting slices stay disjoint");
});

// A FOCUS.md shaped like the live one: the first "## Now" holds the order, the
// old section is titled "## Old Now" and a later "## Core" repeats ids as prose.
const FOCUS_TEXT = [
  "# FOCUS",
  "",
  "## Now (owner 2026-10-10): four Core proofs",
  "",
  "1. **Look:** proof B5 text. Rows: B3 B1 A1 ZZ9",
  "2. **Net:** mention of O1 and B2 here, no list.",
  "3. **Last:** Rows: B2, B1; B4.",
  "Owner waits: B5 needs the owner.",
  "",
  "## Old Now (owner 2026-09-27): the old list",
  "Rows: O1",
  "",
  "## Core",
  "Rows: B2",
  "",
].join("\n");

test("focusNowIds reads only the ids after Rows: in the first ## Now section, in order, once each", () => {
  assert.deepEqual(focusNowIds(FOCUS_TEXT), ["B3", "B1", "A1", "ZZ9", "B2", "B4"]);
  assert.deepEqual(focusNowIds("# FOCUS\n\n## Core\nRows: B1\n"), [], "no ## Now section, no ids");
  assert.deepEqual(focusNowIds(null), []);
  assert.deepEqual(focusNowIds("## Now\r\nRows: B1 B3\r\n## Next\r\nRows: B2\r\n"), ["B1", "B3"], "CRLF, stops at the next ##");
});

test("--focus lists only ready focus rows, in the focus order, not the whole ready list", () => {
  const { code, out, err } = run(["--focus"], FOCUS_TEXT);
  assert.equal(code, 0);
  const ids = out.split("\n").filter((l) => /^[A-Z]\d/.test(l)).map((l) => l.split(/\s+/)[0]);
  assert.deepEqual(ids, ["B3", "B1"], "B3 then B1 (focus order); A1 done, B2 waits, B4 BLOCKED");
  assert.match(out, /^2 row\(s\)$/m);
  assert.match(err, /focus id ZZ9 is not on the board/);
  assert.deepEqual(run(["--ready"]).out.split("\n").filter((l) => /^[A-Z]\d/.test(l)).map((l) => l.split(/\s+/)[0]), ["B1", "B3"], "--ready keeps board order");
});

test("--focus --open adds the waiting focus rows with what they wait on; no ## Now means 0 rows", () => {
  const open = run(["--focus", "--open"], FOCUS_TEXT).out;
  assert.match(open, /^B2 .*\[waits: B3\]/m);
  assert.match(open, /^B4 .*\[waits: BLOCKED\]/m);
  assert.doesNotMatch(open, /^A1 /m, "a done row is never listed");
  assert.doesNotMatch(open, /^O1 /m, "ids outside the first ## Now are ignored");
  const none = run(["--focus"], "# FOCUS\n\n## Core\nRows: B1\n");
  assert.equal(none.code, 0);
  assert.match(none.out, /^0 row\(s\)$/m);
  const missing = run(["--focus"]);
  assert.equal(missing.code, 0, "no docs/FOCUS.md at all is 0 rows, not a crash");
  assert.match(missing.out, /^0 row\(s\)$/m);
});

test("the live docs/FOCUS.md Now list names only ids that exist on the board", () => {
  const ids = focusNowIds(readFileSync(join(dirname(TOOL), "..", "docs", "FOCUS.md"), "utf8"));
  assert.ok(ids.length >= 4 && ids.length <= 128, `Now list holds ${ids.length} ids`);
  const { tasks } = parseBoard(loadBoardText(), archivedDoneRows());
  assert.deepEqual(ids.filter((id) => !tasks.has(id)), [], "every Now id is a board row");
});

test("--find searches every row, done ones and continuation lines too", () => {
  assert.match(run(["--find", "first row"]).out, /^A1\s+done/m);
  assert.match(run(["--find", "ibl"]).out, /^O1\s+open/m, "case-insensitive, across the row's later lines");
  assert.match(run(["--find", "nothing like this"]).out, /^0 row\(s\)$/m);
});

test("--find without a text is a usage error; the single-row slice still works", () => {
  const bad = run(["--find"]);
  assert.equal(bad.code, 2);
  assert.match(bad.err, /--find needs a text/);
  const slice = run(["B1"]);
  assert.equal(slice.code, 0);
  assert.match(slice.out, /- \[ \] B1 render: ready row, needs: A1\./);
});

test("parsers are importable with no side effects: parseBoard/needsOf/rowReady agree with --ready", () => {
  const { tasks, order } = parseBoard(BOARD);
  assert.deepEqual(order, ["A1", "B1", "B2", "B3", "B4", "B5", "O1"]);
  assert.deepEqual(needsOf(tasks.get("B1")), ["A1"]);
  assert.deepEqual(needsOf(tasks.get("B3")), []);
  const ready = order.map((id) => tasks.get(id)).filter((t) => rowReady(tasks, t)).map((t) => t.id);
  assert.deepEqual(ready, ["B1", "B3"], "imported rowReady matches the --ready CLI");
  assert.equal(blockedBy(tasks, tasks.get("B2")), "B3");
  assert.equal(blockedBy(tasks, tasks.get("B4")), "BLOCKED");
});

test("--help board size stays within 10 KB of the board (index + shards)", () => {
  const r = spawnSync(process.execPath, [TOOL, "--help"], { encoding: "utf8" });
  assert.equal(r.status, 0);
  const m = /The board is\s+(\d+(?:\.\d+)?)\s*KB/.exec(r.stdout);
  assert.ok(m, "help states the board size");
  const statedKb = Number(m[1]);
  // Split-safe: the board is docs/SPRINT.md plus docs/SPRINT-*.md shards
  // (ARCHIVE excluded), exactly what loadBoardText returns.
  const bytes = Buffer.byteLength(loadBoardText(), "utf8");
  const actualKb = bytes / 1024;
  assert.ok(
    Math.abs(actualKb - statedKb) <= 10,
    `help says ${statedKb} KB but the board is ${actualKb.toFixed(1)} KiB`,
  );
});

test("split-safe: index + SPRINT-*.md shards parse to the same rows; ARCHIVE excluded", () => {
  const root = mkdtempSync(join(tmpdir(), "board-split-"));
  mkdirSync(join(root, "docs", "plan"), { recursive: true });
  // Single-file board: 3 rows.
  const single = [
    "# SPRINT",
    "## Board",
    "- [x] A1 core: done.",
    "- [ ] B1 render: ready, needs: A1.",
    "- [ ] B2 net: waits, needs: B1.",
    "",
  ].join("\n");
  // Split form: index keeps A1 + B1, shard holds B2; ARCHIVE holds a stale row.
  const index = [
    "# SPRINT",
    "## Board",
    "- [x] A1 core: done.",
    "- [ ] B1 render: ready, needs: A1.",
    "",
  ].join("\n");
  const shard = ["- [ ] B2 net: waits, needs: B1.", ""].join("\n");
  writeFileSync(join(root, "docs", "SPRINT.md"), index);
  writeFileSync(join(root, "docs", "SPRINT-NET.md"), shard);
  writeFileSync(join(root, "docs", "SPRINT-ARCHIVE.md"), "- [ ] ZZ9 stale: must never count.\n");
  const combined = loadBoardText(root);
  const { tasks, order } = parseBoard(combined);
  assert.deepEqual(order, ["A1", "B1", "B2"]);
  assert.ok(!tasks.has("ZZ9"), "ARCHIVE rows never join the board");
  assert.deepEqual(boardShardFiles(root).map((p) => p.split("\\").join("/").split("/").pop()), ["SPRINT-NET.md"]);
  const ready = order.map((id) => tasks.get(id)).filter((t) => rowReady(tasks, t)).map((t) => t.id);
  assert.deepEqual(ready, ["B1"]);
});

test("single-file tree: loadBoardText is byte-identical to the index file", () => {
  const root = mkdtempSync(join(tmpdir(), "board-nosplit-"));
  mkdirSync(join(root, "docs", "plan"), { recursive: true });
  writeFileSync(join(root, "docs", "SPRINT.md"), BOARD);
  writeFileSync(join(root, "docs", "SPRINT-ARCHIVE.md"), "- [ ] ZZ9 stale: must never count.\n");
  assert.deepEqual(boardShardFiles(root), []);
  assert.equal(loadBoardText(root), BOARD, "single file returns index bytes unchanged");
  const { tasks, order } = parseBoard(loadBoardText(root));
  assert.deepEqual(order, ["A1", "B1", "B2", "B3", "B4", "B5", "O1"]);
  assert.equal(tasks.size, 7);
});

// Adopted from tools/board-parser-parity.test.mjs (W2-01 parity, orphan adopted 2026-10-08):
// finish.mjs, sprint-loop.mjs and drift-check.mjs share the one SPRINT parser in board-slice.mjs.
const HERE = dirname(fileURLToPath(import.meta.url));

// Fixture: done/open mix, needs chains, continuation lines, BLOCKED/LATER,
// an Operator section, and a ## Receipts tail (mirrors the live board shape).
const PARITY_BOARD = [
  "# SPRINT",
  "### Finish line (owner lead, `docs/plan/a.md`)",
  "- [x] A1 core: the first row.",
  "- [ ] B1 render: ready row, needs: A1.",
  "  continued: still fast.",
  "- [ ] B2 net: waits on B3, needs: B3.",
  "- [ ] B3 audio: itself open, needs: none.",
  "- [ ] B4 game: parked BLOCKED: no GPU.",
  "- [ ] B5 game: later LATER: after the merge.",
  "### Operator",
  "- [ ] O1 owner: only the operator can do this, needs: none.",
  "## Receipts",
  "no rows down here, just prose.",
  "",
].join("\n");

// The old finish.mjs parseBoardTicks, verbatim (removed 2026-10-02).
function oldParseBoardTicks(md) {
  const ticks = new Map();
  for (const line of md.split("\n")) {
    const m = line.match(/^-\s*\[(x| )\]\s*([A-Za-z]+\d+[a-z]?)/i);
    if (!m) continue;
    const id = m[2];
    const ticked = m[1].toLowerCase() === "x";
    ticks.set(id, (ticks.get(id) ?? false) || ticked);
  }
  return ticks;
}

// The old sprint-loop.mjs board(), verbatim minus the file read.
function oldSprintBoard(text) {
  const tasks = new Map();
  let section = "";
  let current = null;
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith("### ")) {
      section = line.slice(4).trim();
      current = null;
      continue;
    }
    const head = line.match(/^- \[( |x)\] ([A-Z][A-Z0-9]*\d[a-z]?)\b(.*)$/);
    if (head) {
      current = { id: head[2], done: head[1] === "x", text: head[3], section };
      tasks.set(current.id, current);
    } else if (current && /^\s+\S/.test(line)) {
      current.text += ` ${line.trim()}`;
    } else {
      current = null;
    }
  }
  const ready = [];
  for (const task of tasks.values()) {
    if (task.done || task.section.startsWith("Operator") || /\b(BLOCKED|LATER):/.test(task.text)) continue;
    const needs = task.text.match(/needs:\s*([^.]*)\./);
    const deps = needs ? needs[1].split(",").map((s) => s.trim().split(/\s/)[0]).filter((s) => /^[A-Z]/.test(s)) : [];
    if (deps.every((dep) => tasks.get(dep)?.done)) ready.push(task.id);
  }
  const done = [...tasks.values()].filter((task) => task.done).length;
  return { done, total: tasks.size, ready };
}

// The old drift-check.mjs §15 open-row set, verbatim minus the file read.
function oldStuckOpen(board) {
  return new Set([...board.matchAll(/^- \[ \] ([A-Z][A-Z0-9]*\d+[A-Za-z0-9]*)/gm)].map((m) => m[1]));
}

test("ticksOf agrees with the old finish.mjs parser, tick for tick", () => {
  const { tasks } = parseBoard(PARITY_BOARD);
  assert.deepEqual([...ticksOf(tasks)], [...oldParseBoardTicks(PARITY_BOARD)]);
});

test("parseBoard+rowReady agrees with the old sprint-loop.mjs board()", () => {
  const text = PARITY_BOARD.split("## Receipts")[0];
  const { tasks, order } = parseBoard(text);
  const ready = order.map((id) => tasks.get(id)).filter((t) => rowReady(tasks, t)).map((t) => t.id);
  const done = order.filter((id) => tasks.get(id).done).length;
  assert.deepEqual({ done, total: tasks.size, ready }, oldSprintBoard(text));
});

test("parseBoard open set agrees with the old drift-check.mjs §15 regex", () => {
  const { tasks, order } = parseBoard(PARITY_BOARD);
  assert.deepEqual(
    new Set(order.filter((id) => !tasks.get(id).done)),
    oldStuckOpen(PARITY_BOARD),
  );
});

test("drift-check.mjs reads the board once via the shared parser (no double 460KB read)", () => {
  const src = readFileSync(join(HERE, "drift-check.mjs"), "utf8");
  assert.match(src, /from "\.\/board-slice\.mjs"/, "drift-check imports the shared parser");  const reads = src.match(/loadBoardText\(ROOT\)/g) ?? [];
  assert.equal(reads.length, 1, `expected 1 shared board read, found ${reads.length}`);
  for (const f of ["finish.mjs", "sprint-loop.mjs"]) {
    const s = readFileSync(join(HERE, f), "utf8");
    assert.match(s, /from "\.\/board-slice\.mjs"/, `${f} imports the shared parser`);
  }
});

// Done rows moved to docs/archive/SPRINT-done-*.md: needs: resolution and the ticked count only.
const ARCHIVE_DONE = [
  "## SPRINT-01-rome-first.md",
  "",
  "- [x] A9 core: moved out of the shard, still done.",
  "  continuation line of A9.",
  "- [ ] Z1 stale: an open row in an archive never counts.",
  "",
].join("\n");
const MOVED_BOARD = [
  "# SPRINT",
  "## Board",
  "### Finish line",
  "- [ ] B1 render: ready once the archived A9 counts, needs: A9.",
  "- [ ] B2 net: waits on the open B3, needs: B3.",
  "- [ ] B3 audio: itself open, needs: none.",
  "",
].join("\n");
const boardRoot = (index, archive = null) => {
  const root = mkdtempSync(join(tmpdir(), "board-archive-"));
  mkdirSync(join(root, "docs", "archive"), { recursive: true });
  mkdirSync(join(root, "docs", "plan"), { recursive: true });
  writeFileSync(join(root, "docs", "SPRINT.md"), index);
  if (archive !== null) writeFileSync(join(root, "docs", "archive", "SPRINT-done-2026-10-10.md"), archive);
  return root;
};
const runIn = (root, args) => {
  mkdirSync(join(root, "tools"), { recursive: true });
  copyFileSync(TOOL, join(root, "tools", "board-slice.mjs"));
  const r = spawnSync(process.execPath, [join(root, "tools", "board-slice.mjs"), ...args], { encoding: "utf8", cwd: root });
  return { code: r.status, out: r.stdout, err: r.stderr };
};

test("a needs: on an archived id resolves as ticked: the row that waits on it is ready", () => {
  const root = boardRoot(MOVED_BOARD, ARCHIVE_DONE);
  const archived = archivedDoneRows(root);
  assert.deepEqual([...archived.keys()], ["A9"], "only [x] rows in docs/archive count; Z1 is open");
  const { tasks, order } = parseBoard(loadBoardText(root), archived);
  assert.equal(tasks.get("A9").done, true);
  assert.equal(rowReady(tasks, tasks.get("B1")), true);
  assert.equal(rowReady(tasks, tasks.get("B2")), false);
  assert.ok(!order.includes("A9"), "an archived row never joins the order");
  // Without the archive the same board blocks B1: A9 is unknown.
  const bare = parseBoard(loadBoardText(root));
  assert.equal(rowReady(bare.tasks, bare.tasks.get("B1")), false);
});

test("archived rows never appear in --ready or --focus, even when the focus list names them", () => {
  const root = boardRoot(MOVED_BOARD, ARCHIVE_DONE);
  writeFileSync(join(root, "docs", "FOCUS.md"), "## Now\n\nRows: A9 B1\n");
  const ready = runIn(root, ["--ready"]);
  assert.equal(ready.code, 0);
  assert.doesNotMatch(ready.out, /^A9\s/m);
  assert.match(ready.out, /^B1\s+open/m);
  const focus = runIn(root, ["--focus"]);
  assert.equal(focus.code, 0);
  assert.doesNotMatch(focus.out, /^A9\s/m);
  assert.match(focus.out, /^B1\s+open/m);
  assert.match(focus.out, /^1 row\(s\)$/m);
  assert.equal(focus.err, "", "A9 is on the board through the archive: no 'not on the board' warning");
  const slice = runIn(root, ["A9"]);
  assert.equal(slice.code, 1);
  assert.match(slice.err, /archived/);
});

test("the ticked/total count is the same when a done row moves from a shard to the archive", () => {
  const inShard = [
    "# SPRINT",
    "## Board",
    "### Done",
    "- [x] A9 core: moved out of the shard, still done.",
    "  continuation line of A9.",
    "### Finish line",
    "- [ ] B1 render: ready once the archived A9 counts, needs: A9.",
    "- [ ] B2 net: waits on the open B3, needs: B3.",
    "- [ ] B3 audio: itself open, needs: none.",
    "",
  ].join("\n");
  const count = (p) => ({ done: [...p.tasks.values()].filter((t) => t.done).length, total: p.tasks.size });
  const before = count(parseBoard(inShard));
  const root = boardRoot(MOVED_BOARD, ARCHIVE_DONE);
  const after = count(parseBoard(loadBoardText(root), archivedDoneRows(root)));
  assert.deepEqual(before, { done: 1, total: 4 });
  assert.deepEqual(after, before);
});

// Now rows wait on rows the focus rule never picks (2026-10-12: Q1 on RL26 and GX13). --focus lists the open READY
// rows of every Now row's needs: chain too, marked with the Now row they unblock.
const CHAIN_BOARD = [
  "# SPRINT",
  "## Board",
  "### Finish line",
  "- [x] A1 core: done.",
  "- [ ] Q1 proof: waits on two ready rows, needs: RL26, GX13.",
  "- [ ] RL26 render: ready, needs: A1.",
  "- [ ] GX13 render: ready, needs: none.",
  "- [ ] N1 nested: waits on M1, needs: M1.",
  "- [ ] M1 mid: waits on L1, needs: L1.",
  "- [ ] L1 leaf: ready, needs: A1.",
  "- [ ] P1 parked: waits on a BLOCKED row, needs: X1.",
  "- [ ] X1 stuck: BLOCKED: no GPU, needs: L1.",
  "- [ ] R1 shared: needs RL26 too, needs: RL26.",
  "- [ ] S1 other: ready and not in the chain of any Now row, needs: none.",
  "",
].join("\n");
const chain = (nowRows, args = ["--focus"]) => {
  const root = boardRoot(CHAIN_BOARD);
  writeFileSync(join(root, "docs", "FOCUS.md"), `## Now\n\nRows: ${nowRows}\n`);
  const r = runIn(root, args);
  const lines = r.out.split("\n").filter((l) => /^[A-Z][A-Z0-9]*\d\s/.test(l));
  return { ...r, ids: lines.map((l) => l.split(/\s+/)[0]), lines, root };
};

test("--focus lists the open ready ancestors of a Now row, marked with the Now row they unblock", () => {
  const r = chain("Q1");
  assert.equal(r.code, 0);
  assert.deepEqual(r.ids, ["RL26", "GX13"], "Q1 itself waits; its two ready needs are listed");
  assert.match(r.lines[0], /^RL26 .*\(ancestor of Q1\)$/);
  assert.match(r.lines[1], /^GX13 .*\(ancestor of Q1\)$/);
  assert.match(r.out, /^2 row\(s\)$/m);
  assert.doesNotMatch(r.out, /^S1 /m, "a ready row outside every Now chain stays out");
});

test("--focus follows the needs: chain through waiting rows, not through BLOCKED ones", () => {
  const nested = chain("N1");
  assert.deepEqual(nested.ids, ["L1"], "N1 needs M1 (waiting) needs L1 (ready)");
  assert.match(nested.lines[0], /\(ancestor of N1\)$/);
  const parked = chain("P1");
  assert.deepEqual(parked.ids, [], "X1 is BLOCKED and L1 sits behind it: nothing to pick");
  assert.match(parked.out, /^0 row\(s\)$/m);
});

test("--focus puts Now rows first and lists an ancestor once, under the first Now row it unblocks", () => {
  const r = chain("R1 S1 Q1");
  assert.deepEqual(r.ids, ["S1", "RL26", "GX13"], "S1 is a ready Now row; R1 waits; RL26 once (for R1), then GX13");
  assert.match(r.lines[1], /^RL26 .*\(ancestor of R1\)$/);
  assert.doesNotMatch(r.lines[0], /ancestor/, "a Now row is never marked");
  const both = chain("RL26 Q1");
  assert.deepEqual(both.ids, ["RL26", "GX13"], "a ready ancestor that is a Now row is a Now row, not an ancestor");
  assert.doesNotMatch(both.lines[0], /ancestor/);
  assert.match(both.lines[1], /^GX13 .*\(ancestor of Q1\)$/);
});

test("--focus --open lists waiting Now rows and their ready ancestors; the focus list itself is not changed", () => {
  const r = chain("Q1", ["--focus", "--open"]);
  assert.deepEqual(r.ids, ["Q1", "RL26", "GX13"]);
  assert.match(r.lines[0], /^Q1 .*\[waits: RL26 GX13\]$/);
  assert.match(r.lines[1], /\(ancestor of Q1\)$/);
  assert.equal(readFileSync(join(r.root, "docs", "FOCUS.md"), "utf8"), "## Now\n\nRows: Q1\n", "ancestors are listed, never written into FOCUS.md");
  assert.deepEqual(focusNowIds("## Now\n\nRows: Q1\n"), ["Q1"]);
});

test("focusAncestors is pure: tasks and Now ids in, [{ id, of }] out", () => {
  const { tasks } = parseBoard(CHAIN_BOARD);
  assert.deepEqual(focusAncestors(tasks, ["Q1", "N1"]), [{ id: "RL26", of: "Q1" }, { id: "GX13", of: "Q1" }, { id: "L1", of: "N1" }]);
  assert.deepEqual(focusAncestors(tasks, ["A1", "ZZ9"]), [], "a done row and an unknown id have no ancestors");
  assert.deepEqual(focusAncestors(tasks, []), []);
});

test("shard/archive readers warn loudly on unreadable dirs, stay silent on missing ones", () => {
  const warns = [];
  const orig = console.warn;
  console.warn = (...a) => warns.push(a.join(" "));
  try {
    const missing = mkdtempSync(join(tmpdir(), "board-missing-"));
    assert.deepEqual(boardShardFiles(missing), [], "no docs dir reads as no shards");
    assert.deepEqual(archivedDoneRows(missing), new Map(), "no archive reads as no archived rows");
    assert.deepEqual(warns, [], "a missing dir stays silent");
    const noArchive = mkdtempSync(join(tmpdir(), "board-noarchive-"));
    mkdirSync(join(noArchive, "docs"));
    writeFileSync(join(noArchive, "docs", "archive"), "a file where the archive dir belongs");
    assert.deepEqual(boardShardFiles(noArchive), [], "no shards, still silent");
    assert.equal(warns.length, 0);
    assert.deepEqual(archivedDoneRows(noArchive), new Map());
    assert.equal(warns.length, 1, "an unreadable archive dir warns loudly");
    assert.match(warns[0], /WARN board-slice.*archive/);
    const noDocs = mkdtempSync(join(tmpdir(), "board-nodocs-"));
    writeFileSync(join(noDocs, "docs"), "a file where the docs dir belongs");
    assert.deepEqual(boardShardFiles(noDocs), []);
    assert.equal(warns.length, 2, "an unreadable shard dir warns loudly");
    assert.match(warns[1], /WARN board-slice.*shard/);
  } finally {
    console.warn = orig;
  }
});

test("pickNext merges focus+ready+find: focus order first, then board order", () => {
  assert.equal(typeof pickNext, "function", "pickNext is importable (fails when --next is reverted)");
  const { tasks, order } = parseBoard(BOARD);
  assert.equal(pickNext(tasks, order, { nowIds: ["B3", "B1"] }).id, "B3");
  assert.equal(pickNext(tasks, order, { nowIds: [] }).id, "B1", "empty focus falls back to board order");
  assert.equal(pickNext(tasks, order, { nowIds: ["B3", "B1"], findText: "ready row" }).id, "B1");
  assert.equal(pickNext(tasks, order, { nowIds: ["B2"] }).id, "B3", "B2 waits on B3 so the ready ancestor wins");
  assert.equal(pickNext(tasks, order, { nowIds: ["B2"], open: true }).id, "B2");
  assert.equal(pickNext(tasks, order, { nowIds: [], findText: "zzz-no-such-text" }), null);
});

test("--next prints one read: the picked slice, byte-identical to the <ID> slice", () => {
  const next = run(["--next"], FOCUS_TEXT);
  assert.equal(next.code, 0);
  assert.match(next.out, /^# B3 slice/m, "--next picks the first ready focus row");
  const one = run(["B3"]);
  assert.equal(one.code, 0);
  assert.equal(next.out, one.out, "one --next read replaces --focus + <ID>");
});

test("--next [<text>] filters like --find before the pick; no match is exit 1", () => {
  const filtered = run(["--next", "ready row"], FOCUS_TEXT);
  assert.equal(filtered.code, 0);
  assert.match(filtered.out, /^# B1 slice/m);
  const nomatch = run(["--next", "zzz-no-such-text"], FOCUS_TEXT);
  assert.equal(nomatch.code, 1);
  assert.match(nomatch.err, /no ready row matches/);
});

test("--next --json prints the picked slice as JSON; --next runs alone", () => {
  const r = run(["--next", "--json"], FOCUS_TEXT);
  assert.equal(r.code, 0);
  const j = JSON.parse(r.out);
  assert.equal(j.id, "B3");
  assert.ok(j.row.includes("B3 audio"), "JSON row is the picked row");
  const clash = run(["--next", "--ready"], FOCUS_TEXT);
  assert.equal(clash.code, 2);
  assert.match(clash.err, /--next runs alone/);
});

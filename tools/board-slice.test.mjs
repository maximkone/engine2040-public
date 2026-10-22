// Run: node --test tools/board-slice.test.mjs
// board-slice's list modes (--ready, --open, --find) on a small board, with the same readiness as sprint-loop.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync as mkdtempRaw, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { archivedDoneRows, blockedBy, boardShardFiles, focusAncestors, focusNowIds, loadBoardText, needsOf, ownedOf, parseBoard, parkedUnmarked, pathsOverlap, pickNext, planWave, rowReady, rowsOverlap, ticksOf } from "./board-slice.mjs";

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

const run = (args, focus = null, board = BOARD) => {
  const root = mkdtempSync(join(tmpdir(), "board-slice-"));
  mkdirSync(join(root, "tools"));
  mkdirSync(join(root, "docs", "plan"), { recursive: true });
  copyFileSync(TOOL, join(root, "tools", "board-slice.mjs"));
  writeFileSync(join(root, "docs", "SPRINT.md"), board);
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


test("--parked finds ready rows a dated note parked without the BLOCKED marker the ready filter reads", () => {
  const board = [
    "# SPRINT",
    "## Board",
    "### Finish line",
    "- [ ] P1 render: ready row, needs: none.",
    "  Progress 2026-10-19: operator/owner: the code half landed, the reference-GPU",
    "  receipt is missing.",
    "- [ ] P2 render: ready row, needs: none.",
    "  Progress 2026-10-19: the sweep landed; left: the second rung and a human",
    "  lookcheck.",
    "- [ ] P3 render: parked with the marker, needs: none. BLOCKED: needs a human.",
    "- [ ] P4 render: parked at the operator's door, needs: none.",
    "  Progress 2026-10-19: blocked until the owner answers.",
    "- [ ] P5 render: parked once, answered since, needs: none.",
    "  Progress 2026-10-19: operator: needs the reference GPU.",
    "  Owner chat 2026-10-19 (delegated): the bar reads p50/p99, so the row is work again.",
    "",
  ].join("\n");
  const { tasks, order } = parseBoard(board);
  assert.deepEqual(
    parkedUnmarked(tasks, order).map((t) => t.id),
    ["P1", "P4"],
    "a park note at the start of a dated note; a partial note that mentions a human later is still work",
  );
  const { code, out } = run(["--parked"], null, board);
  assert.equal(code, 1, "the report exits 1 while a parked row lacks the marker");
  assert.match(out, /^P1\s+Progress 2026-10-19: operator\/owner/m);
  assert.match(out, /^P4\s+Progress 2026-10-19: blocked/m);
  assert.doesNotMatch(out, /^P2 /m, "P2 has agent work left");
  assert.doesNotMatch(out, /^P3 /m, "P3 carries the marker the ready filter reads");
  assert.doesNotMatch(out, /^P5 /m, "P5 carries a recorded answer, so it is no longer an open park");
  assert.match(out, /^2 ready row\(s\) parked without a BLOCKED: marker$/m);
  const clean = run(["--parked"], null, ["# SPRINT", "## Board", "### Finish line", "- [ ] P9 render: ready, needs: none.", ""].join("\n"));
  assert.equal(clean.code, 0);
  assert.match(clean.out, /^0 ready row\(s\) parked without a BLOCKED: marker$/m);
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

// W2-01 parity cases adopted 2026-10-08 from the removed board-parser-parity test, kept here:
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

test("finish.mjs reads the board through the shared parser", () => {
  const s = readFileSync(join(HERE, "finish.mjs"), "utf8");
  assert.match(s, /from "\.\/board-slice\.mjs"/, "finish.mjs imports the shared parser");
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

// 2026-10-18: the head regex required a digit right after the letters, so
// E-* (E-4, E-STUCKDG), BET-* (BET-AX13) and CQ-NET-* rows were invisible to
// every list mode. One grammar now reads plain and hyphenated ids alike.
const HYPHEN_BOARD = [
  "# SPRINT",
  "## Board",
  "### Finish line",
  "- [x] A1 core: done.",
  "- [ ] E-4 render: short hyphenated id, needs: A1.",
  "- [ ] BET-AX13 playtest: two hyphens, needs: E-4.",
  "- [ ] CQ-NET-API5 net: three segments, needs: none.",
  "- [ ] E-STUCKDG research: hyphenated and digit-free, needs: A1.",
  "",
].join("\n");
// A list line starts with the padded id, then open/done, then the title.
const ID_LINE = /^[A-Z][A-Z0-9-]*\s+(?:open|done)\s/;
const listIds = (out) => out.split("\n").filter((l) => ID_LINE.test(l)).map((l) => l.split(/\s+/)[0]);

test("parseBoard/needsOf/rowReady read hyphenated ids first-class", () => {
  const { tasks, order } = parseBoard(HYPHEN_BOARD);
  assert.deepEqual(order, ["A1", "E-4", "BET-AX13", "CQ-NET-API5", "E-STUCKDG"], "no hyphenated id is dropped");
  assert.equal(tasks.size, 5, "no id merged or duplicated");
  assert.deepEqual(needsOf(tasks.get("BET-AX13")), ["E-4"], "a needs: on a hyphenated id resolves");
  assert.deepEqual(needsOf(tasks.get("E-STUCKDG")), ["A1"], "a digit-free hyphenated id parses too");
  const ready = order.map((id) => tasks.get(id)).filter((t) => rowReady(tasks, t)).map((t) => t.id);
  assert.deepEqual(ready, ["E-4", "CQ-NET-API5", "E-STUCKDG"], "E-4 is ready; BET-AX13 waits on it");
  assert.equal(blockedBy(tasks, tasks.get("BET-AX13")), "E-4");
});

test("--ready/--ready --open/--find and the <ID> slice see hyphenated rows", () => {
  const root = boardRoot(HYPHEN_BOARD);
  const ready = runIn(root, ["--ready"]);
  assert.equal(ready.code, 0);
  assert.deepEqual(listIds(ready.out), ["E-4", "CQ-NET-API5", "E-STUCKDG"]);
  assert.match(ready.out, /^3 row\(s\)$/m);
  const open = runIn(root, ["--ready", "--open"]);
  assert.deepEqual(listIds(open.out), ["E-4", "BET-AX13", "CQ-NET-API5", "E-STUCKDG"]);
  assert.match(open.out, /^BET-AX13 +open .*\[waits: E-4\]$/m);
  const found = runIn(root, ["--find", "cq-net-api5"]);
  assert.deepEqual(listIds(found.out), ["CQ-NET-API5"], "case-insensitive");
  const slice = runIn(root, ["BET-AX13"]);
  assert.equal(slice.code, 0, `unknown row would exit 1: ${slice.err}`);
  assert.match(slice.out, /^# BET-AX13 slice$/m);
  assert.match(slice.out, /- \[ \] BET-AX13 playtest: two hyphens, needs: E-4\./, "row verbatim");
});

test("--focus and the archive reader take hyphenated ids too", () => {
  const root = boardRoot(HYPHEN_BOARD);
  writeFileSync(join(root, "docs", "FOCUS.md"), "## Now\n\nRows: BET-AX13, E-4; CQ-NET-API5.\n");
  assert.deepEqual(focusNowIds("## Now\n\nRows: BET-AX13, E-4; CQ-NET-API5.\n"), ["BET-AX13", "E-4", "CQ-NET-API5"]);
  const focus = runIn(root, ["--focus"]);
  assert.equal(focus.code, 0);
  assert.deepEqual(listIds(focus.out), ["E-4", "CQ-NET-API5"], "BET-AX13 waits; E-4 also stands as a Now row");
  assert.equal(focus.err, "", "every Now id is on the board");
  const archive = ["## SPRINT-01-rome-first.md", "", "- [x] E-DOGFOOD206 dispatch: done and moved.", "- [ ] E-ZOO18 dispatch: still open, never counts.", ""].join("\n");
  const index = ["# SPRINT", "## Board", "### Finish line", "- [ ] E-DIRT83 dispatch: needs an archived hyphenated row, needs: E-DOGFOOD206.", ""].join("\n");
  const moved = boardRoot(index, archive);
  const archived = archivedDoneRows(moved);
  assert.deepEqual([...archived.keys()], ["E-DOGFOOD206"], "only the [x] hyphenated row counts");
  const { tasks } = parseBoard(loadBoardText(moved), archived);
  assert.equal(tasks.get("E-DOGFOOD206").done, true);
  assert.equal(rowReady(tasks, tasks.get("E-DIRT83")), true);
});

// The 2026-10-05 size-cap prune and the 2026-10-02 board diet (F06) moved done
// rows out of the board whole; each file's header promises its moved ids keep
// every needs: target resolving. Both are read for ticks only.
test("done rows in the older board snapshots still tick a needs:", () => {
  const root = boardRoot([
    "# SPRINT",
    "## Board",
    "### Finish line",
    "- [ ] L1 net: needs a diet-archived row, needs: OLD1.",
    "- [ ] L2 net: needs a prune-archived row, needs: E-DOGFOOD206.",
    "",
  ].join("\n"));
  writeFileSync(join(root, "docs", "SPRINT-ARCHIVE.md"), "- [x] OLD1 net: moved whole by the diet.\n");
  writeFileSync(join(root, "docs", "archive", "SPRINT-2026-10-05.md"), "- [x] E-DOGFOOD206 dispatch: moved by the size-cap prune.\n  second line of E-DOGFOOD206.\n");
  const archived = archivedDoneRows(root);
  assert.deepEqual([...archived.keys()].sort(), ["E-DOGFOOD206", "OLD1"]);
  const { tasks, order } = parseBoard(loadBoardText(root), archived);
  assert.equal(rowReady(tasks, tasks.get("L1")), true, "a diet-archived done row ticks its dependent");
  assert.equal(rowReady(tasks, tasks.get("L2")), true, "a prune-archived hyphenated done row does too");
  assert.deepEqual(order, ["L1", "L2"], "the archived rows never join the order");
  const ready = runIn(root, ["--ready"]);
  assert.deepEqual(listIds(ready.out), ["L1", "L2"], "and are never listed");
});

test("paths overlap when equal or nested, never by a name prefix", () => {
  assert.equal(pathsOverlap("crates/game/src/lib.rs", "crates/game/src/lib.rs"), true);
  assert.equal(pathsOverlap("crates/game", "crates/game/src/lib.rs"), true);
  assert.equal(pathsOverlap("crates/game/src/lib.rs", "crates/game"), true);
  assert.equal(pathsOverlap("crates/game/src/", "crates/game/src/lib.rs"), true, "a trailing slash is the same lease");
  assert.equal(pathsOverlap("crates/game/src/a.rs", "crates/game/src/a.rs.bak"), false, "a name prefix is not a hold");
  assert.equal(pathsOverlap("crates/game/src", "crates/game/src2/lib.rs"), false);
  assert.equal(rowsOverlap(["crates/anim/src/a.rs"], ["crates/anim", "crates/render"]), true);
  assert.equal(rowsOverlap(["crates/anim/src/a.rs"], ["crates/render/src/b.rs"]), false);
});

test("a wave takes a row only when its paths are disjoint from every row taken", () => {
  const rows = [
    { id: "R1", owned: ["crates/render/src/a.rs"], title: "a" },
    { id: "R2", owned: ["crates/render/src/a.rs"], title: "b" },
    { id: "R3", owned: ["crates/render"], title: "c" },
    { id: "R4", owned: ["crates/anim/src/b.rs"], title: "d" },
    { id: "R5", owned: [], title: "e" },
  ];
  const { picked, skipped } = planWave(rows, 9);
  assert.deepEqual(picked.map((r) => r.id), ["R1", "R4"]);
  assert.deepEqual(skipped, [
    { id: "R2", why: "overlap" },
    { id: "R3", why: "overlap" },
    { id: "R5", why: "no paths" },
  ]);
  assert.deepEqual(planWave(rows, 1).picked.map((r) => r.id), ["R1"], "the count is a hard cap");
});

test("--wave prints N rows with disjoint paths and counts what it skipped", () => {
  const index = [
    "# SPRINT",
    "## Board",
    "- [ ] W1 render: owns `crates/render/src/a.rs`. needs: none.",
    "- [ ] W2 render: owns `crates/render/src/a.rs`. needs: none.",
    "- [ ] W3 anim: owns `crates/anim/src/b.rs`. needs: none.",
    "- [ ] W4 broad: owns `crates/render`. needs: none.",
    "- [ ] W5 nothing: no path here. needs: none.",
    "",
  ].join("\n");
  const root = boardRoot(index);
  const wave = runIn(root, ["--wave", "3"]);
  assert.equal(wave.code, 0);
  assert.match(wave.out, /^W1\s/m);
  assert.match(wave.out, /^W3\s/m);
  assert.doesNotMatch(wave.out, /^W2\s/m, "the same file is never in one wave twice");
  assert.doesNotMatch(wave.out, /^W4\s/m, "a row that holds a picked file is skipped");
  assert.doesNotMatch(wave.out, /^W5\s/m, "a row with no path cannot hold a lease");
  assert.match(wave.out, /wave 2\/3: 2 skipped for overlap with a picked row, 1 skipped for naming no path/);

  const json = runIn(root, ["--wave", "3", "--json"]);
  assert.equal(json.code, 0);
  const parsed = JSON.parse(json.out);
  assert.deepEqual(parsed.picked.map((r) => r.id), ["W1", "W3"]);
  assert.deepEqual(parsed.picked[0].owned, ["crates/render/src/a.rs"]);
  assert.equal(parsed.skipped.length, 3);
});

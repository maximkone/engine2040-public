// board-slice: print one board row as a small exact packet slice.
//
// A packet carries the slice instead of the whole 100KB+ board. The row
// itself is reproduced byte-identical from docs/SPRINT.md; everything else
// is derived context (prerequisite first-lines, plan excerpts, owned paths).
//
// Usage from the repo/lane root:
//   node tools/board-slice.mjs <ID> [--json]
//   node tools/board-slice.mjs --next [<text>] [--json]
//   node tools/board-slice.mjs --self-check
//   node tools/board-slice.mjs --ready [--open]
//   node tools/board-slice.mjs --focus [--open]
//   node tools/board-slice.mjs --find <text>
//   node tools/board-slice.mjs --help
//
// Exit codes: 0 ok (self-check passes) - 1 unknown row or self-check FAIL - 2 usage.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const BOARD = join(ROOT, "docs", "SPRINT.md");
const DOCS_DIR = join(ROOT, "docs");
const PLAN_DIR = join(ROOT, "docs", "plan");
const SLICE_BUDGET = 8192;
const ID_WIDTH = 8;
const SECTION_WIDTH = 26;
const TITLE_WIDTH = 110;
const SELF_CHECK_LIMIT = 10;

// Split-safe board reads: docs/SPRINT.md is the index; shard files
// docs/SPRINT-*.md (except SPRINT-ARCHIVE.md, which is history, not board
// rows) hold moved rows after a split. Without shards this returns the
// index bytes unchanged, so single-file behavior is byte-identical.
export function boardShardFiles(root = ROOT) {
  const dir = root === ROOT ? DOCS_DIR : join(root, "docs");
  let names;
  try {
    names = readdirSync(dir).filter(
      (n) => n.startsWith("SPRINT-") && n.endsWith(".md") && n !== "SPRINT-ARCHIVE.md",
    );
  } catch (err) {
    if (err?.code !== "ENOENT") console.warn(`WARN board-slice: shard dir ${dir} unreadable (${err?.message ?? err}), reading index only`);
    return [];
  }
  names.sort();
  return names.map((n) => join(dir, n));
}

// receipts: false drops each file's `## Receipts` part on its own, so a
// shard that sorts after the receipts shard keeps its rows (2026-10-10:
// sprint-loop cut the joined text at the first `## Receipts` and never saw
// a later shard).
export function loadBoardText(root = ROOT, { receipts = true } = {}) {
  const cut = (t) => (receipts ? t : t.split("## Receipts")[0]);
  const index = root === ROOT ? BOARD : join(root, "docs", "SPRINT.md");
  let text = cut(readFileSync(index, "utf8"));
  for (const shard of boardShardFiles(root)) {
    text += `\n${cut(readFileSync(shard, "utf8"))}`;
  }
  return text;
}

// Done rows moved out of the shards keep their `- [x] <ID>` line in
// docs/archive/SPRINT-done-*.md. Those ids are ticked for needs: resolution
// and the ticked count only; parseBoard never lists them as open or ready.
export function archiveDoneFiles(root = ROOT) {
  const dir = join(root, "docs", "archive");
  try {
    return readdirSync(dir)
      .filter((n) => n.startsWith("SPRINT-done-") && n.endsWith(".md"))
      .sort()
      .map((n) => join(dir, n));
  } catch (err) {
    if (err?.code !== "ENOENT") console.warn(`WARN board-slice: archive dir ${dir} unreadable (${err?.message ?? err}), ignoring archived rows`);
    return [];
  }
}

export function archivedDoneRows(root = ROOT) {
  const done = new Map();
  for (const file of archiveDoneFiles(root)) {
    for (const line of readFileSync(file, "utf8").split(/\r?\n/)) {
      const m = /^- \[x\] ([A-Z][A-Z0-9]*\d[a-z]?)\b/.exec(line);
      if (m && !done.has(m[1])) done.set(m[1], line);
    }
  }
  return done;
}

const HELP = `board-slice.mjs — print one board row as a small packet slice (AW1)

Usage:
  node tools/board-slice.mjs <ID> [--json]
  node tools/board-slice.mjs --next [<text>] [--json]
  node tools/board-slice.mjs --self-check
  node tools/board-slice.mjs --ready [--open]     one line per ready (or open) row: id, section, title
  node tools/board-slice.mjs --focus [--open]     the same, only for the ids in docs/FOCUS.md "## Now", in its order, plus the ready rows they wait on
  node tools/board-slice.mjs --find <text>        one line per row (done too) whose text holds <text>

The board is 598 KB: read it through one --next (list plus slice), never whole.

Output sections:
  row verbatim (byte-identical to docs/SPRINT.md), first lines of its
  prerequisite (needs:) rows, its plan §3 excerpt and §7 entry, owned paths.

Options:
  --json         with <ID> or --next only: print the slice as JSON ({ id, row, needs, plan, owned, bytes })
  --next [<text>]  one read: focus-ready rows in FOCUS order, then ready rows
                 in board order, filtered by <text> when given, plus the
                 picked row's slice (replaces --focus + --ready + --find + <ID>)
  --self-check   slice 10 ready rows; FAIL unless each is <= 8 KB with the
                 row byte-identical (AW1 acceptance, minus the context meter)
  --ready        list ready rows (open, every needs: ticked, no BLOCKED:/LATER:, not an operator row)
  --open         with --ready, --focus or --next: every open row, ready or not, with what it waits on
  --focus        ready rows named after "Rows:" in the first "## Now" section of docs/FOCUS.md,
                 in that order (the loop takes these first; 0 rows means no focus row is ready),
                 then the open ready rows a Now row waits on (needs: chain, transitively), each
                 marked "(ancestor of <ID>)": a Now chain never waits on a row nobody picks
  --find <text>  case-insensitive search of every row's text
  -h, --help     print this help and exit 0

Exit codes: 0 ok - 1 unknown row or self-check FAIL - 2 usage.
`;

const HEAD = /^- \[( |x)\] ([A-Z][A-Z0-9]*\d[a-z]?)\b(.*)$/;
const FOCUS = join(ROOT, "docs", "FOCUS.md");

// Row ids of the owner's focus list: every id after "Rows:" on a line of the
// first "## Now" section of docs/FOCUS.md, in that order, once each. Other
// text in the section (notes, proofs) is never read as ids.
export function focusNowIds(focusText) {
  const lines = String(focusText ?? "").split(/\r?\n/);
  const start = lines.findIndex((l) => /^## Now\b/.test(l));
  if (start < 0) return [];
  const ids = [];
  for (let i = start + 1; i < lines.length && !/^## /.test(lines[i]); i += 1) {
    const m = /\bRows:\s*(.+)$/.exec(lines[i]);
    if (!m) continue;
    for (const tok of m[1].split(/[\s,;]+/)) {
      const id = tok.replace(/[.)]+$/, "");
      if (/^[A-Z][A-Z0-9]*\d[a-z]?$/.test(id) && !ids.includes(id)) ids.push(id);
    }
  }
  return ids;
}

function fail(message, code = 1) {
  process.stderr.write(`board-slice: error: ${message}\n`);
  process.exit(code);
}

function usageFail(message) {
  process.stderr.write(`board-slice: error: ${message}\n\n${HELP}`);
  process.exit(2);
}

// Parse the board into tasks keyed by id, keeping each row's exact lines.
// Exported: the other board readers (finish.mjs, sprint-loop.mjs,
// drift-check.mjs) import these instead of re-parsing SPRINT.md by hand.
// `archived` (archivedDoneRows) adds moved done rows as ticked tasks, but only
// to `tasks`: they never join `order`, so no list mode shows them.
export function parseBoard(text, archived = null) {
  const lines = text.includes("\r") ? text.split(/\r?\n/) : text.split("\n");
  const tasks = new Map();
  const order = [];
  let current = null;
  let sectionPlan = null;
  let sectionName = null;
  let h3 = "";
  for (const line of lines) {
    if (!line) {
      current = null;
      continue;
    }
    const c0 = line.charCodeAt(0);
    if (c0 === 35) {
      // `### Name (owner <date>, ...)`: the section's plan is its first
      // backticked docs/plan/*.md, else none (the slice falls back to the
      // plan that lists the row). A header without one, or a `#`/`##`
      // heading, ends the previous section: rows never inherit another
      // shard's plan (2026-10-10: 87 rows read living-world.md).
      const section = /^###\s+(.+?)\s*\(owner\b([^)]*)\)/.exec(line);
      if (section) {
        sectionName = section[1].trim();
        sectionPlan = (/`(docs\/plan\/[^`]+\.md)`/.exec(section[2]) ?? [])[1] ?? null;
        h3 = sectionName;
        current = null;
        continue;
      }
      if (/^#{1,2}\s/.test(line) || /^###\s+Operator\b/.test(line)) {
        sectionName = null;
        sectionPlan = null;
        h3 = "";
      }
      if (/^#{1,4}\s/.test(line)) {
        if (line.startsWith("### ")) h3 = line.slice(4).trim();
        current = null;
        continue;
      }
      current = null;
      continue;
    }
    if (c0 !== 45) {
      if (current && /^\s+\S/.test(line)) {
        current.lines.push(line);
      } else {
        current = null;
      }
      continue;
    }
    const head = HEAD.exec(line);
    if (head) {
      if (tasks.has(head[2])) {
        current = null;
        continue;
      }
      current = {
        id: head[2],
        done: head[1] === "x",
        lines: [line],
        section: sectionName,
        sectionPlan,
        h3,
      };
      tasks.set(current.id, current);
      order.push(current.id);
    } else if (current && /^\s+\S/.test(line)) {
      current.lines.push(line);
    } else {
      current = null;
    }
  }
  for (const [id, line] of archived ?? []) {
    if (!tasks.has(id)) {
      tasks.set(id, { id, done: true, archived: true, lines: [line], section: null, sectionPlan: null, h3: "" });
    }
  }
  return { tasks, order };
}

export function needsOf(task) {
  const text = task.lines.join(" ");
  const m = /needs:\s*([^.]*)\./.exec(text);
  if (!m) return [];
  if (/^\s*none\s*$/i.test(m[1].trim())) return [];
  return m[1]
    .split(",")
    .map((s) => s.trim().split(/\s+/)[0])
    .filter((s) => /^[A-Z]/.test(s));
}

export function ownedOf(task) {
  const text = task.lines.join(" ");
  const spans = [...text.matchAll(/`([^`]+)`/g)].map((m) => m[1].trim());
  // Usage spans like `tools/board-slice.mjs <ID>` are not paths.
  const paths = spans.filter(
    (s) => !/[\s<>]/.test(s) && (s.includes("/") || /\.(mjs|md|json|rs)$/.test(s)),
  );
  return [...new Set(paths)];
}

// Private, not exported: no external importer (verified 2026-10-08, only
// buildSlice below reads the plan file). The plan file is the row's
// section plan; fall back to whichever docs/plan/*.md names the id in
// its §7 board-task list.
function planPathFor(task) {
  if (task.sectionPlan) {
    const p = join(ROOT, task.sectionPlan);
    if (existsSync(p)) return p;
  }
  return null;
}

// Index-once fallback: one PLAN_DIR readdir per run, one read per plan file
// per run. Private, not exported: only buildSlice below needs the lookup.
let planFallbackNames = null;
const planTextCache = new Map();
function readPlanCached(file) {
  let t = planTextCache.get(file);
  if (t === undefined) {
    t = readFileSync(file, "utf8");
    planTextCache.set(file, t);
  }
  return t;
}
function planFallbackFile(id) {
  if (planFallbackNames === null) {
    planFallbackNames = readdirSync(PLAN_DIR).filter((n) => n.endsWith(".md"));
  }
  const want = new RegExp(`^- \\[ \\] ${id}\\b`, "m");
  for (const n of planFallbackNames) {
    const q = join(PLAN_DIR, n);
    if (want.test(readPlanCached(q))) return q;
  }
  return null;
}

// Private, not exported: no external importer (verified 2026-10-08, only
// buildSlice below needs plan §3 text).
function sectionText(text, n) {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((l) => new RegExp(`^##\\s+${n}\\.\\s`).test(l));
  if (start < 0) return "";
  let end = lines.findIndex((l, i) => i > start && /^##\s+\d+\.\s/.test(l));
  if (end < 0) end = lines.length;
  return lines.slice(start, end).join("\n").trim();
}

// Private, not exported: no external importer (verified 2026-10-08, only
// buildSlice below needs plan §7 entries).
function planEntry(planText, id) {
  const lines = planText.split(/\r?\n/);
  let start = -1;
  for (let i = 0; i < lines.length; i += 1) {
    if (/^##\s+7\.\s/.test(lines[i])) start = i;
    if (start >= 0 && new RegExp(`^- \\[ \\] ${id}\\b`).test(lines[i])) {
      const block = [lines[i]];
      for (let j = i + 1; j < lines.length; j += 1) {
        if (/^\s+\S/.test(lines[j])) block.push(lines[j]);
        else break;
      }
      return block.join("\n");
    }
  }
  return "";
}

// Private, not exported: no external importer (verified 2026-10-08, only
// main below slices rows for the CLI).
function buildSlice(boardText, tasks, task) {
  const row = task.lines.join("\n");
  const deps = needsOf(task).filter((d) => tasks.has(d));
  const needs = deps.map((d) => ({ id: d, firstLine: tasks.get(d).lines[0] }));
  // The plan that lists the row (`- [ ] ID`) wins over the section header:
  // the 2026-10-10 merges moved rows between plans but not between shards.
  // Index-once lookup: the directory is read a single time per run and
  // each plan file is read at most once per run (see planFallbackFile).
  let planFile = planFallbackFile(task.id);
  if (!planFile) {
    planFile = planPathFor(task);
  }
  let plan3 = "";
  let plan7 = "";
  if (planFile && existsSync(planFile)) {
    const planText = readPlanCached(planFile).replace(/\r\n/g, "\n");
    plan3 = sectionText(planText, 3);
    plan7 = planEntry(planText, task.id);
  }
  const owned = ownedOf(task);
  const slice = [
    `# ${task.id} slice`,
    "",
    "## Row (verbatim, byte-identical to docs/SPRINT.md)",
    row,
    "",
    "## Needs (first lines)",
    needs.length ? needs.map((n) => `${n.id}: ${n.firstLine}`).join("\n") : "(none)",
    "",
    "## Plan §3 excerpt",
    plan3 || "(no plan §3 found)",
    "",
    "## Plan §7 entry",
    plan7 || "(no plan §7 entry found)",
    "",
    "## Owned paths",
    owned.length ? owned.join("\n") : "(none listed)",
    "",
  ].join("\n");
  return { slice, row, needs, plan3, plan7, owned, planFile };
}

// Ready as docs/SPRINT.md states it (same test as tools/sprint-loop.mjs): open, every needs: row ticked, no BLOCKED:/LATER: in the text, not an operator row.
export function rowReady(tasks, t) {
  const text = t.lines.join(" ");
  if (t.done || (t.h3 ?? "").startsWith("Operator") || /\b(BLOCKED|LATER):/.test(text)) return false;
  return needsOf(t).every((d) => tasks.has(d) && tasks.get(d).done);
}

// The open READY rows that Now rows wait on, through the needs: chain (transitively), that the Now list does not name:
// [{ id, of }], `of` = the first Now row (in Now order) whose chain reaches the row. A Now row that waits on a ready row
// the focus rule never picks waits forever (2026-10-12: Q1 on RL26 and GX13). The chain runs through waiting rows and
// stops at done, unknown and BLOCKED/LATER/operator rows (nothing behind them can run). Listed, never added to FOCUS.md.
export function focusAncestors(tasks, nowIds) {
  const now = new Set(nowIds);
  const seen = new Set();
  const out = [];
  for (const root of nowIds) {
    const start = tasks.get(root);
    if (!start || start.done) continue;
    const queue = [...needsOf(start)];
    while (queue.length) {
      const id = queue.shift();
      const t = tasks.get(id);
      if (!t || t.done || now.has(id) || seen.has(id)) continue;
      seen.add(id);
      if (rowReady(tasks, t)) out.push({ id, of: root });
      else if (!/^(?:BLOCKED|LATER|operator)$/.test(blockedBy(tasks, t))) queue.push(...needsOf(t));
    }
  }
  return out;
}

export function blockedBy(tasks, t) {
  const text = t.lines.join(" ");
  if (/\bBLOCKED:/.test(text)) return "BLOCKED";
  if (/\bLATER:/.test(text)) return "LATER";
  if ((t.h3 ?? "").startsWith("Operator")) return "operator";
  return needsOf(t).filter((d) => !tasks.get(d)?.done).slice(0, 4).join(" ");
}

// Next actionable row in one pass: focus-ready rows in FOCUS order (plus
// their ready ancestors), then ready rows in board order. With findText the
// list is filtered like --find before the pick; with open every open row
// counts, else only rowReady rows. Returns the task or null.
export function pickNext(tasks, order, { nowIds = [], findText = null, open = false } = {}) {
  const seen = new Set();
  const cands = [];
  const push = (t) => {
    if (!t || t.done || t.archived || seen.has(t.id)) return;
    if (!open && !rowReady(tasks, t)) return;
    seen.add(t.id);
    cands.push(t);
  };
  for (const k of nowIds) push(tasks.get(k));
  for (const a of focusAncestors(tasks, nowIds)) push(tasks.get(a.id));
  for (const k of order) push(tasks.get(k));
  const needle = findText == null ? "" : String(findText).toLowerCase();
  if (needle) return cands.find((t) => t.lines.join(" ").toLowerCase().includes(needle)) ?? null;
  return cands[0] ?? null;
}

// Private, not exported: no external importer (verified 2026-10-08, only
// the --self-check path below lists ready slices).
function readyRows(tasks, order) {
  return order
    .map((id) => tasks.get(id))
    .filter((t) => !t.done && needsOf(t).every((d) => tasks.has(d) && tasks.get(d).done));
}

// The board as an id -> ticked map (what finish.mjs and drift-check.mjs need:
// no duplicates exist, so first-wins parseBoard agrees with the old
// accumulate-the-regex readers; verified 1120/1120, 0 diffs 2026-10-02).
export function ticksOf(tasks) {
  return new Map([...tasks].map(([id, t]) => [id, t.done]));
}

function main(argv) {
  const args = [...argv];
  let json = false;
  let selfCheck = false;
  let id = null;
  let listReady = false;
  let listOpen = false;
  let listFocus = false;
  let listNext = false;
  let find = null;
  for (let i = 0; i < args.length; i += 1) {
    const a = args[i];
    if (a === "--json") json = true;
    else if (a === "--ready") listReady = true;
    else if (a === "--focus") listFocus = true;
    else if (a === "--next") listNext = true;
    else if (a === "--open") listOpen = true;
    else if (a === "--find") {
      find = args[i + 1];
      i += 1;
      if (!find) usageFail("--find needs a text");
    }
    else if (a === "--self-check") selfCheck = true;
    else if (a === "--help" || a === "-h") {
      const kb = Math.round(Buffer.byteLength(loadBoardText(), "utf8") / 1024);
      process.stdout.write(HELP.replace(/The board is\s+\d+\s*KB/, `The board is ${kb} KB`));
      process.exit(0);
    } else if (a.startsWith("-")) usageFail(`unknown flag ${a}`);
    else if (!id) id = a;
    else usageFail("at most one <ID>");
  }
  let nextText = null;
  if (listNext) {
    if (listReady || listFocus || selfCheck) usageFail("--next runs alone: not with --ready, --focus or --self-check");
    if (find && id) usageFail("use one filter: --next [<text>] or --next --find <text>, not both");
    nextText = find ?? id ?? null;
    find = null;
    id = null;
  }
  if (!existsSync(BOARD)) fail(`board missing: ${BOARD}`);
  const rawBoard = loadBoardText();
  const boardText = rawBoard.includes("\r") ? rawBoard.replace(/\r\n/g, "\n") : rawBoard;
  const { tasks, order } = parseBoard(boardText, archivedDoneRows());

  if (listNext) {
    const ids = focusNowIds(existsSync(FOCUS) ? readFileSync(FOCUS, "utf8") : "");
    for (const k of ids) if (!tasks.has(k)) process.stderr.write(`board-slice: focus id ${k} is not on the board\n`);
    const task = pickNext(tasks, order, { nowIds: ids, findText: nextText, open: listOpen });
    if (!task) fail(nextText ? `no ready row matches ${nextText}` : "no ready row to take next");
    const built = buildSlice(boardText, tasks, task);
    if (json) {
      process.stdout.write(
        `${JSON.stringify({ id: task.id, row: built.row, needs: built.needs, plan: built.planFile, plan3: built.plan3, plan7: built.plan7, owned: built.owned, bytes: Buffer.byteLength(built.slice, "utf8") }, null, 2)}\n`,
      );
    } else {
      process.stdout.write(built.slice);
    }
    process.exit(0);
  }

  if (listReady || listFocus || find) {
    let rows;
    const ancestorOf = new Map();
    if (find) {
      rows = order.map((k) => tasks.get(k)).filter((t) => t.lines.join(" ").toLowerCase().includes(find.toLowerCase()));
    } else if (listFocus) {
      const ids = focusNowIds(existsSync(FOCUS) ? readFileSync(FOCUS, "utf8") : "");
      for (const k of ids) if (!tasks.has(k)) process.stderr.write(`board-slice: focus id ${k} is not on the board\n`);
      rows = ids.map((k) => tasks.get(k)).filter((t) => t && !t.done && (listOpen || rowReady(tasks, t)));
      for (const a of focusAncestors(tasks, ids)) {
        rows.push(tasks.get(a.id));
        ancestorOf.set(a.id, a.of);
      }
    } else {
      rows = order.map((k) => tasks.get(k)).filter((t) => !t.done && (listOpen || rowReady(tasks, t)));
    }
    const outLines = [];
    for (const t of rows) {
      const title = t.lines[0].replace(HEAD, "$3").trim().replace(/\s+/g, " ").slice(0, TITLE_WIDTH);
      const wait = !t.done && listOpen && !rowReady(tasks, t) ? ` [waits: ${blockedBy(tasks, t)}]` : "";
      const ancestor = ancestorOf.has(t.id) ? ` (ancestor of ${ancestorOf.get(t.id)})` : "";
      outLines.push(`${t.id.padEnd(ID_WIDTH)} ${t.done ? "done" : "open"} ${(t.h3 || t.section || "-").slice(0, SECTION_WIDTH).padEnd(SECTION_WIDTH)} ${title}${wait}${ancestor}`);
    }
    outLines.push(`${rows.length} row(s)`);
    process.stdout.write(`${outLines.join("\n")}\n`);
    process.exit(0);
  }

  if (selfCheck) {
    const ready = readyRows(tasks, order).slice(0, SELF_CHECK_LIMIT);
    if (!ready.length) fail("self-check: no ready rows to slice");
    let bad = 0;
    for (const t of ready) {
      const { slice, row } = buildSlice(boardText, tasks, t);
      const bytes = Buffer.byteLength(slice, "utf8");
      const identical = boardText.includes(row) && slice.includes(row);
      const ok = bytes <= SLICE_BUDGET && identical;
      if (!ok) bad += 1;
      console.log(
        `[${ok ? "PASS" : "FAIL"}] ${t.id}: ${bytes} bytes (budget ${SLICE_BUDGET}), row byte-identical: ${identical ? "yes" : "no"}`,
      );
    }
    console.log(`RESULT ${bad ? "FAIL" : "PASS"}: ${ready.length - bad}/${ready.length} ready slices within budget and byte-identical`);
    process.exit(bad ? 1 : 0);
  }

  if (!id) usageFail("need <ID> or --self-check");
  const task = tasks.get(id);
  if (!task) fail(`unknown row ${id}`);
  if (task.archived) fail(`row ${id} is done and archived in docs/archive/; it has no live slice`);
  const { slice, row, needs, plan3, plan7, owned, planFile } = buildSlice(boardText, tasks, task);
  if (json) {
    process.stdout.write(
      `${JSON.stringify({ id, row, needs, plan: planFile, plan3, plan7, owned, bytes: Buffer.byteLength(slice, "utf8") }, null, 2)}\n`,
    );
  } else {
    process.stdout.write(slice);
  }
}

// Importable (the parsers above) and still a CLI: main runs only when this
// file is the invoked script, so `import { parseBoard } from "./board-slice.mjs"`
// has no side effects.
if (import.meta.main) {
  main(process.argv.slice(2));
}

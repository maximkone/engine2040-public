// plan-atlas: generated map of every plan, its rows, crates and health.
//
// VISION.md Plans map promises: `node tools/plan-atlas.mjs --write` regenerates
// docs/plan/ATLAS.md; never hand-edited. docs/plan/README.md documents the queries.
// Reuses the one board parser in board-slice.mjs (parseBoard/loadBoardText/rowReady).
//
// Usage from the repo/lane root:
//   node tools/plan-atlas.mjs --write
//   node tools/plan-atlas.mjs --row <ID>
//   node tools/plan-atlas.mjs --crate <name>
//   node tools/plan-atlas.mjs --priority <look|bodies|aidoor|speed|proof|other>
//   node tools/plan-atlas.mjs --plan <name>
//   node tools/plan-atlas.mjs --find <text>
//   node tools/plan-atlas.mjs --help
//
// Exit codes: 0 ok - 1 unknown row/plan/crate or empty result - 2 usage.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isMainModule } from './steal-next.mjs';
import { archivedDoneRows, loadBoardText, parseBoard, rowReady } from "./board-slice.mjs";
import { cliArgs } from "./argv.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const VISION = join(ROOT, "docs", "VISION.md");
const README = join(ROOT, "docs", "plan", "README.md");
const PLAN_DIR = join(ROOT, "docs", "plan");
const ATLAS = join(PLAN_DIR, "ATLAS.md");

const HELP = `plan-atlas.mjs - generated map of every plan, its rows, crates and health

Usage:
  node tools/plan-atlas.mjs --write
  node tools/plan-atlas.mjs --row <ID>
  node tools/plan-atlas.mjs --crate <name>
  node tools/plan-atlas.mjs --priority <look|bodies|aidoor|speed|proof|other>
  node tools/plan-atlas.mjs --plan <name>
  node tools/plan-atlas.mjs --find <text>

ATLAS.md is generated (--write); never hand-edited. Reads the VISION Plans map,
docs/plan/*.md and docs/SPRINT.md through the shared board parser.

Exit codes: 0 ok - 1 unknown row/plan/crate or empty result - 2 usage.
`;

// Parse the VISION "Plans: which plan serves which part" table into
// [{ file, serves, notes }]. Exported for tests.
export function parseVisionPlansMap(text) {
  const rows = [];
  for (const line of text.split(/\r?\n/)) {
    const m = /^\|\s*`(docs\/[^`]+)`\s*\|\s*(.*?)\s*\|\s*(.*?)\s*\|$/.exec(line);
    if (m) rows.push({ file: m[1], serves: m[2].trim(), notes: m[3].trim() });
  }
  return rows;
}

// Parse the docs/plan/README.md "| Row | File | Prefixes |" table into
// { fileToPrefixes: Map<file, string[]>, prefixToFiles: Map<prefix, string[]> }.
export function parsePrefixTable(text) {
  const fileToPrefixes = new Map();
  const prefixToFiles = new Map();
  for (const line of text.split(/\r?\n/)) {
    const m = /^\|\s*[^|]*\|\s*`([^`]+)`\s*\|\s*(.*?)\s*\|$/.exec(line);
    if (!m) continue;
    const file = m[1].trim();
    if (!file.endsWith(".md")) continue;
    const prefixes = m[2]
      .split(",")
      .map((s) => s.trim().split(/\s+/)[0].replace(/[`()]/g, ""))
      .filter((s) => /^[A-Z][A-Z0-9]*$/.test(s));
    if (!prefixes.length) continue;
    fileToPrefixes.set(file, prefixes);
    for (const p of prefixes) {
      if (!prefixToFiles.has(p)) prefixToFiles.set(p, []);
      if (!prefixToFiles.get(p).includes(file)) prefixToFiles.get(p).push(file);
    }
  }
  return { fileToPrefixes, prefixToFiles };
}

// Row id prefix: leading letters (RL25 -> RL, EA1a -> EA, RA16 -> RA).
export function prefixOf(id) {
  const m = /^([A-Z]+)\d/i.exec(id);
  return m ? m[1].toUpperCase() : "";
}

// Crate names named by one plan text: `crates/<name>/` path mentions.
// Sorted, deduped. Exported for tests.
export function cratesIn(text) {
  const found = new Set();
  for (const m of text.matchAll(/crates\/([a-z0-9_-]+)/g)) found.add(m[1]);
  return [...found].sort();
}

// Section-7 board-task ids named in one plan ("- [ ] RL5 ..." under "## 7.").
export function section7Ids(planText) {
  const lines = planText.split(/\r?\n/);
  let in7 = false;
  const ids = [];
  for (const line of lines) {
    if (/^##\s+7\.\s/.test(line)) in7 = true;
    else if (in7 && /^##\s+\d+\.\s/.test(line)) break;
    else if (in7) {
      const m = /^-\s*\[[ x]\]\s*([A-Z][A-Z0-9]*\d[a-z]?)\b/.exec(line);
      if (m) ids.push(m[1]);
    }
  }
  return ids;
}

// Heading (## ..) owning the line index, for --find context.
export function headingAbove(lines, i) {
  for (let j = i; j >= 0; j -= 1) {
    const m = /^(#{1,6})\s+(.*\S)\s*$/.exec(lines[j]);
    if (m) return m[2].trim();
  }
  return "(top)";
}

function planNameOf(file) {
  return file.replace(/\.md$/, "").split("/").pop();
}

function fail(message, code = 1) {
  process.stderr.write(`plan-atlas: error: ${message}\n`);
  process.exit(code);
}

function usageFail(message) {
  process.stderr.write(`plan-atlas: error: ${message}\n\n${HELP}`);
  process.exit(2);
}

// Single-flight plan file read: one read per filename, shared by the
// text/words/crates/missing7 getters below. Cache key is the plan
// filename; dir is the docs/plan dir for this index.
export function planText(dir, f, cache = new Map()) {
  if (!cache.has(f)) cache.set(f, readFileSync(join(dir, f), "utf8"));
  return cache.get(f);
}

// Shared index: one entry per docs/plan/*.md file on disk.
export function buildIndex(root = ROOT) {
  const dir = root === ROOT ? PLAN_DIR : join(root, "docs", "plan");
  const visionPath = root === ROOT ? VISION : join(root, "docs", "VISION.md");
  const readmePath = root === ROOT ? README : join(root, "docs", "plan", "README.md");
  const visionText = existsSync(visionPath) ? readFileSync(visionPath, "utf8") : "";
  const servesOf = new Map(parseVisionPlansMap(visionText).map((r) => [r.file.split("/").pop(), r.serves]));
  const readmeText = existsSync(readmePath) ? readFileSync(readmePath, "utf8") : "";
  const { prefixToFiles } = parsePrefixTable(readmeText);
  let board = { tasks: new Map(), order: [] };
  try {
    // Moved done rows (docs/archive/SPRINT-done-*.md) count as ticked for needs: (9d7e9102).
    board = parseBoard(loadBoardText(root).replace(/\r\n/g, "\n"), archivedDoneRows(root));
  } catch {
    board = { tasks: new Map(), order: [] };
  }
  // Rows grouped by the plan file their board section names.
  const rowsByFile = new Map();
  for (const id of board.order) {
    const t = board.tasks.get(id);
    const f = (t.sectionPlan || "").split("/").pop();
    if (!f) continue;
    if (!rowsByFile.has(f)) rowsByFile.set(f, []);
    rowsByFile.get(f).push(t);
  }
  let files = [];
  try {
    files = readdirSync(dir).filter((n) => n.endsWith(".md")).sort();
  } catch {
    files = [];
  }
  const boardIds = new Set(board.order);
  const textCache = new Map();
  const plans = files.map((f) => {
    const rows = rowsByFile.get(f) ?? [];
    const open = rows.filter((t) => !t.done).map((t) => t.id);
    const ready = rows.filter((t) => rowReady(board.tasks, t)).map((t) => t.id);
    const done = rows.filter((t) => t.done).map((t) => t.id);
    const serves = servesOf.get(f) ?? "-";
    const name = planNameOf(f);
    const getText = () => planText(dir, f, textCache);
    return {
      file: f,
      name,
      serves,
      open,
      ready,
      done,
      get text() {
        return getText();
      },
      get words() {
        return getText().split(/\s+/).filter(Boolean).length;
      },
      get crates() {
        return cratesIn(getText());
      },
      get missing7() {
        return section7Ids(getText()).filter((id) => !boardIds.has(id));
      },
    };
  });
  return { plans, prefixToFiles, tasks: board.tasks, order: board.order };
}

// Minimal generated ATLAS.md: every plan with its rows, crates and health.
// Health here = open/ready/done counts plus section-7 rows missing from the
// board (the actionable "no longer true" signal); code file-touches come
// from git history, out of scope for this minimal writer.
export function buildAtlas(index, date) {
  const day = date ?? new Date().toISOString().slice(0, 10);
  const lines = [
    "# Plan atlas - where everything is planned, and how true each plan is today",
    "",
    `Generated ${day} by \`node tools/plan-atlas.mjs --write\` from the plans, \`docs/SPRINT.md\` and the VISION Plans map. Never edit by hand: fix the plan, then regenerate.`,
    "",
    "Find things fast:",
    "",
    "- `node tools/plan-atlas.mjs --row RL25` - which plan owns a row, its status and its plan entry",
    "- `node tools/plan-atlas.mjs --crate render` - every plan that touches a crate",
    "- `node tools/plan-atlas.mjs --priority look` - the reading order for one Core priority (look, bodies, aidoor, speed, proof, other)",
    "- `node tools/plan-atlas.mjs --plan render-look` - one plan health card: rows, crates, section-7 rows off the board",
    "- `node tools/plan-atlas.mjs --find ragdoll` - plans whose text names a word, with the heading it sits under",
    "",
    "## 1. Every plan: rows, crates and health",
    "",
    "| Plan | Open / ready / done rows | Crates | Serves | Health |",
    "|---|---|---|---|---|",
  ];
  for (const p of index.plans) {
    const health = p.missing7.length ? `${p.missing7.length} section-7 row(s) off the board (${p.missing7.slice(0, 4).join(", ")})` : "ok";
    lines.push(
      `| [\`${p.name}\`](${p.file}) | ${p.open.length} / ${p.ready.length} / ${p.done.length} | ${p.crates.join(", ") || "-"} | ${p.serves} | ${health} |`,
    );
  }
  lines.push(
    "",
    "## 2. Row prefix -> plan",
    "",
    "| Prefix | Plan(s) |",
    "|---|---|",
  );
  for (const [prefix, files] of [...index.prefixToFiles.entries()].sort()) {
    lines.push(`| ${prefix} | ${files.map((f) => `\`${planNameOf(f)}\``).join(", ")} |`);
  }
  lines.push(
    "",
    "## 3. Crate -> plans",
    "",
    "| Crate | Plans (open rows) |",
    "|---|---|",
  );
  const byCrate = new Map();
  for (const p of index.plans) {
    for (const c of p.crates) {
      if (!byCrate.has(c)) byCrate.set(c, []);
      byCrate.get(c).push(p);
    }
  }
  for (const [crate, owners] of [...byCrate.entries()].sort()) {
    owners.sort((a, b) => b.open.length - a.open.length);
    lines.push(`| \`${crate}\` | ${owners.map((p) => `\`${p.name}\` (${p.open.length})`).join(", ")} |`);
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
}

const PRIORITY_WORDS = {
  look: ["look", "rtx"],
  bodies: ["dismemberment", "destruction", "wounds", "fracture"],
  aidoor: ["ai builds", "ai in gameplay", "spatial understanding"],
  speed: ["sim speed"],
  proof: ["headless proofs", "determinism", "exact replay"],
  other: [],
};

function priorityMatch(serves, key) {
  if (key === "other") return true;
  const words = PRIORITY_WORDS[key] ?? [key];
  const low = serves.toLowerCase();
  return words.some((w) => low.includes(w));
}

function main(argv) {
  const args = [...argv];
  let write = false;
  let row = null;
  let crate = null;
  let priority = null;
  let plan = null;
  let find = null;
  for (let i = 0; i < args.length; i += 1) {
    const a = args[i];
    if (a === "--write") write = true;
    else if (a === "--row") {
      row = args[i + 1];
      i += 1;
      if (!row) usageFail("--row needs an <ID>");
    } else if (a === "--crate") {
      crate = args[i + 1];
      i += 1;
      if (!crate) usageFail("--crate needs a <name>");
    } else if (a === "--priority") {
      priority = args[i + 1];
      i += 1;
      if (!priority) usageFail("--priority needs a <name>");
    } else if (a === "--plan") {
      plan = args[i + 1];
      i += 1;
      if (!plan) usageFail("--plan needs a <name>");
    } else if (a === "--find") {
      find = args[i + 1];
      i += 1;
      if (!find) usageFail("--find needs a <text>");
    } else if (a === "--help" || a === "-h") {
      process.stdout.write(HELP);
      process.exit(0);
    } else if (a.startsWith("-")) usageFail(`unknown flag ${a}`);
    else usageFail(`unexpected ${a}`);
  }
  if (Number(write) + Number(row !== null) + Number(crate !== null) + Number(priority !== null) + Number(plan !== null) + Number(find !== null) !== 1) {
    usageFail("pass exactly one of --write, --row, --crate, --priority, --plan, --find");
  }

  const index = buildIndex();

  if (write) {
    const text = buildAtlas(index);
    writeFileSync(ATLAS, text);
    console.log(`plan-atlas: wrote ${ATLAS} (${index.plans.length} plans, ${Buffer.byteLength(text, "utf8")} bytes)`);
    process.exit(0);
  }

  if (row) {
    const id = row.toUpperCase();
    const task = index.tasks.get(id);
    let owner = "(no plan owns this row)";
    const owners = index.prefixToFiles.get(prefixOf(id)) ?? [];
    if (owners.length) owner = owners.map((f) => planNameOf(f)).join(", ");
    if (task?.sectionPlan) owner = planNameOf(task.sectionPlan.split("/").pop());
    if (!task && !owners.length) fail(`unknown row ${row}`);
    const status = task ? (task.done ? "done" : rowReady(index.tasks, task) ? "ready" : "open") : "not on the board";
    console.log(`${id}: plan ${owner}, status ${status}`);
    if (task) console.log(task.lines.join("\n"));
    process.exit(0);
  }

  if (crate) {
    const name = crate.toLowerCase();
    const hits = index.plans.filter((p) => p.crates.includes(name));
    if (!hits.length) fail(`no plan touches crate ${crate}`);
    hits.sort((a, b) => b.open.length - a.open.length);
    for (const p of hits) console.log(`${p.name} (${p.open.length} open): ${p.crates.join(", ")}`);
    console.log(`${hits.length} plan(s) touch crate ${name}`);
    process.exit(0);
  }

  if (priority) {
    const key = priority.toLowerCase();
    if (!Object.hasOwn(PRIORITY_WORDS, key)) usageFail(`unknown priority ${priority} (look|bodies|aidoor|speed|proof|other)`);
    const hits = key === "other"
      ? index.plans.filter((p) => !Object.keys(PRIORITY_WORDS).filter((k) => k !== "other").some((k) => priorityMatch(p.serves, k)))
      : index.plans.filter((p) => priorityMatch(p.serves, key));
    if (!hits.length) fail(`no plan serves priority ${priority}`);
    for (const p of hits) console.log(`${p.name}: ${p.serves} (${p.open.length} open)`);
    console.log(`${hits.length} plan(s) serve ${key}`);
    process.exit(0);
  }

  if (plan) {
    const key = plan.toLowerCase().replace(/\.md$/, "");
    const hit = index.plans.find((p) => p.name.toLowerCase() === key);
    if (!hit) fail(`unknown plan ${plan}`);
    console.log(`# ${hit.name} health`);
    console.log(`serves: ${hit.serves}`);
    console.log(`words: ${hit.words}`);
    console.log(`rows: ${hit.open.length} open / ${hit.ready.length} ready / ${hit.done.length} done`);
    console.log(`crates: ${hit.crates.join(", ") || "-"}`);
    console.log(`first ready: ${hit.ready.slice(0, 4).join(", ") || "-"}`);
    console.log(hit.missing7.length ? `off the board: ${hit.missing7.join(", ")}` : "off the board: -");
    process.exit(0);
  }

  if (find) {
    const needle = find.toLowerCase();
    let hits = 0;
    for (const p of index.plans) {
      const text = p.text;
      const lines = text.split(/\r?\n/);
      const at = lines.findIndex((l) => l.toLowerCase().includes(needle));
      if (at < 0) continue;
      hits += 1;
      console.log(`${p.name}: ${headingAbove(lines, at)}: ${lines[at].trim().slice(0, 120)}`);
    }
    console.log(`${hits} plan(s) name ${find}`);
    process.exit(hits ? 0 : 1);
  }
}

if (isMainModule(import.meta.url)) {
  main(cliArgs());
}

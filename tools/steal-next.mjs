// steal-next: coldest-topic-first picker for research readers.
//
// Docs name it in `docs/VISION-TABLES.md`, `docs/plan/frontier.md` and
// `docs/plan/frontier.md`: it names the coldest future topic (fewest
// plan + research mentions in `docs/plan/frontier.md` §1) and the stalest
// donor row (oldest Swept date in the VISION-TABLES Parts table) so the
// next reader takes the topic with room instead of a HOT pile.
//
// Usage from the repo root:
//   node tools/steal-next.mjs
//   node tools/steal-next.mjs --top <n>
//   node tools/steal-next.mjs --json
//   node tools/steal-next.mjs --help
//
// Exit codes: 0 ok - 1 no data (missing files or tables) - 2 usage.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cliArgs } from "./argv.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const FUTURES = join(ROOT, "docs", "plan", "frontier.md");
const TABLES = join(ROOT, "docs", "VISION-TABLES.md");

const HELP = `steal-next.mjs - coldest-topic-first picker for research readers

Usage:
  node tools/steal-next.mjs [--top <n>] [--json]

Prints the coldest future topic (fewest plan + research mentions in
docs/plan/frontier.md section 1) and the stalest Parts row (oldest Swept
date in docs/VISION-TABLES.md) with its steal-next cell.

Options:
  --top <n>    print the n coldest topics (default 1)
  --json       print { coldest, stalest } as JSON
  -h, --help   print this help and exit 0

Exit codes: 0 ok - 1 no data - 2 usage.
`;

// One row of docs/plan/frontier.md section 1:
// `| Capability | Status | Evidence |` -> { capability, status, plans, research, score }.
// Score is plans + research mentions (lower is colder); GAP sorts before
// COLD before anything else on ties, then file order. Exported for tests.
export function parseFuturesTopics(text) {
  const topics = [];
  let index = 0;
  for (const line of String(text ?? "").split(/\r?\n/)) {
    if (!line.startsWith("|")) continue;
    const cells = line.split("|").slice(1, -1).map((s) => s.trim());
    if (cells.length < 3) continue;
    if (/^capability$/i.test(cells[0])) continue;
    if (/^-+$/.test(cells[0])) continue;
    const [capability, status] = cells;
    if (!capability || !status) continue;
    const plans = (/(\d+)\s*plan/i.exec(status) ?? [])[1];
    const research = (/(\d+)\s*research/i.exec(status) ?? [])[1];
    if (plans === undefined && research === undefined) continue;
    const p = plans === undefined ? 0 : Number(plans);
    const r = research === undefined ? 0 : Number(research);
    const tier = /^gap\b/i.test(status) ? 0 : /^cold\b/i.test(status) ? 1 : 2;
    topics.push({ capability, status, plans: p, research: r, score: p + r, tier, index: index++ });
  }
  return topics;
}

// Coldest first: lowest score, GAP before COLD before other, then file order.
// Fast path: default --top 1 skips copy+sort with one linear min pass.
export function coldestTopics(text, top = 1) {
  const n = Number(top);
  const limit = Number.isFinite(n) && n > 0 ? Math.floor(n) : 1;
  const topics = parseFuturesTopics(text);
  if (limit === 1) {
    let best = null;
    for (const t of topics) {
      if (!best || (t.score !== best.score ? t.score < best.score : t.tier !== best.tier ? t.tier < best.tier : t.index < best.index)) best = t;
    }
    return best ? [best] : [];
  }
  return [...topics]
    .sort((a, b) => a.score - b.score || a.tier - b.tier || a.index - b.index)
    .slice(0, limit);
}

// One row of the VISION-TABLES Parts table:
// `| Part | Consumer | Ours | Best | They beat | Steal next | Proof | Swept |`.
// Swept holds a YYYY-MM-DD date. Exported for tests.
export function parseVisionParts(text) {
  const rows = [];
  let index = 0;
  for (const line of String(text ?? "").split(/\r?\n/)) {
    if (!line.startsWith("|")) continue;
    const cells = line.split("|").slice(1, -1).map((s) => s.trim());
    if (cells.length < 8) continue;
    if (/^part$/i.test(cells[0])) continue;
    if (/^-+$/.test(cells[0])) continue;
    const swept = (/\d{4}-\d{2}-\d{2}/.exec(cells[7] ?? "") ?? [])[0];
    if (!swept) continue;
    rows.push({ part: cells[0], stealNext: cells[5] ?? "", swept, index: index++ });
  }
  return rows;
}

// Stalest donor row: oldest Swept date, file order on ties. Null when empty.
export function stalestPart(text) {
  const rows = parseVisionParts(text);
  if (!rows.length) return null;
  return [...rows].sort((a, b) => (a.swept < b.swept ? -1 : a.swept > b.swept ? 1 : a.index - b.index))[0];
}

// Shared main-module guard (one copy): true when this file is the invoked script.
export function isMainModule(metaUrl, argv1){var a=argv1===undefined?process.argv[1]:argv1;return Boolean(a)&&resolve(a)===fileURLToPath(metaUrl);}

function fail(message, code = 1) {
  process.stderr.write(`steal-next: error: ${message}\n`);
  process.exit(code);
}

function usageFail(message) {
  process.stderr.write(`steal-next: error: ${message}\n\n${HELP}`);
  process.exit(2);
}

function applyCliFlag(a, next, state) {
  if (a === "--top") {
    const v = Number(next);
    if (!Number.isFinite(v) || v < 1) usageFail("--top needs a positive <n>");
    state.top = Math.floor(v);
    return 1;
  }
  if (a === "--json") {
    state.json = true;
    return 0;
  }
  if (a === "--help" || a === "-h") {
    process.stdout.write(HELP);
    process.exit(0);
  }
  if (a.startsWith("-")) usageFail(`unknown flag ${a}`);
  usageFail(`unexpected ${a}`);
}

function parseCliArgs(argv) {
  const args = [...argv];
  const state = { top: 1, json: false };
  for (let i = 0; i < args.length; i += 1) {
    i += applyCliFlag(args[i], args[i + 1], state);
  }
  return state;
}

function loadPickerInputs() {
  if (!existsSync(FUTURES)) fail(`missing ${FUTURES}`);
  if (!existsSync(TABLES)) fail(`missing ${TABLES}`);
  return {
    futuresText: readFileSync(FUTURES, "utf8"),
    tablesText: readFileSync(TABLES, "utf8"),
  };
}

function selectPicks(futuresText, tablesText, top) {
  const coldest = coldestTopics(futuresText, top);
  const stalest = stalestPart(tablesText);
  if (!coldest.length) fail("no future topics in docs/plan/frontier.md section 1");
  if (!stalest) fail("no dated Parts rows in docs/VISION-TABLES.md");
  return { coldest, stalest };
}

function printPicks(coldest, stalest, json) {
  if (json) {
    console.log(JSON.stringify({ coldest, stalest }, null, 2));
    process.exit(0);
  }
  for (const t of coldest) console.log(`coldest-topic: ${t.capability} | ${t.status} | score ${t.score}`);
  console.log(`stalest-part: ${stalest.part} | swept ${stalest.swept}`);
  console.log(`steal-next: ${stalest.stealNext.slice(0, 160)}`);
  process.exit(0);
}

function main(argv) {
  const { top, json } = parseCliArgs(argv);
  const { futuresText, tablesText } = loadPickerInputs();
  const { coldest, stalest } = selectPicks(futuresText, tablesText, top);
  printPicks(coldest, stalest, json);
}

if (isMainModule(import.meta.url)) {
  main(cliArgs());
}

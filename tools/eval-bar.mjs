#!/usr/bin/env node
// eval-bar: compare an agent scorecard card with `tools/eval-bar.json`, by the
// scorecard's own rule (crates/playtest/src/eval/scorecard.rs, `Scorecard::reds`):
// a line whose pass rate drops below the baseline is red, and a baseline line
// the card lost is red. A new line is never red, and an unbuilt line (total 0)
// has no rate, so it is skipped. The bar is measured, never invented: it holds
// the numbers one scorecard run printed, plus the command that printed them.
//   node tools/eval-bar.mjs                     the newest card in target/reports
//   node tools/eval-bar.mjs --card <card.md>    one card
//   node tools/eval-bar.mjs --bar <file.json>   another baseline (a test fixture)
// Exit: 0 every line holds, 1 a red line, 2 the card or the bar does not read.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const BAR_PATH = "tools/eval-bar.json";
export const CARD_DIR = "target/reports";

const HELP = `usage: node tools/eval-bar.mjs [--card <card.md>] [--bar <bar.json>]

  no flag   compare the newest scorecard-*.md in ${CARD_DIR}
  --card    compare this card
  --bar     compare against this baseline instead of ${BAR_PATH}
  --help    print this text, exit 0`;

// Pass rate in permille; `null` for an unbuilt line (total 0), as the harness has it.
export const ratePermille = (line) => (line.total > 0 ? Math.floor((line.passed * 1000) / line.total) : null);

const key = (line) => `${line.suite}\t${line.tier}`;

// The card table: `| suite | tier | passed | total | pass rate | tokens | vs |`.
export function parseCard(text) {
  const lines = [];
  for (const raw of String(text).split(/\r?\n/)) {
    const row = raw.trim();
    if (!row.startsWith("|")) continue;
    const cells = row.replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
    if (cells.length < 4) continue;
    const [suite, tier, passed, total] = cells;
    if (!/^[A-Za-z0-9_.-]+$/.test(tier)) continue;
    if (!/^\d+$/.test(passed) || !/^\d+$/.test(total)) continue;
    lines.push({ suite, tier, passed: Number(passed), total: Number(total) });
  }
  if (!lines.length) throw new Error("no scorecard table rows");
  return lines;
}

// The reds the card would print against the bar, in bar order.
export function compare(bar, card) {
  const have = new Map(card.map((line) => [key(line), line]));
  const reds = [];
  const rows = [];
  for (const old of bar.rows) {
    const was = ratePermille(old);
    if (was === null) continue; // unbuilt in the bar: nothing to hold
    const now = have.get(key(old));
    const nowRate = now ? ratePermille(now) : null;
    const red = nowRate === null || nowRate < was;
    rows.push({ suite: old.suite, tier: old.tier, was, now: nowRate, red });
    if (red) reds.push({ suite: old.suite, tier: old.tier, was, now: nowRate });
  }
  return { rows, reds };
}

const percent = (permille) => `${Math.floor(permille / 10)}.${permille % 10}%`;

export function newestCard(dir) {
  if (!existsSync(dir)) return null;
  const names = readdirSync(dir)
    .filter((name) => /^scorecard-.*\.md$/.test(name))
    .sort();
  return names.length ? join(dir, names[names.length - 1]) : null;
}

export function main(argv = process.argv.slice(2), say = console.log) {
  let card = null;
  let bar = join(ROOT, BAR_PATH);
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--help") {
      say(HELP);
      return 0;
    }
    if ((a === "--card" || a === "--bar") && i + 1 < argv.length) {
      const value = argv[++i];
      if (a === "--card") card = resolve(value);
      else bar = resolve(value);
      continue;
    }
    say(`unknown argument: ${a}`);
    say(HELP);
    return 2;
  }
  card ??= newestCard(join(ROOT, CARD_DIR));
  if (!card || !existsSync(card)) {
    say(`eval-bar: no scorecard card found in ${CARD_DIR}; run the scorecard leg first`);
    return 2;
  }
  let baseline;
  let lines;
  try {
    baseline = JSON.parse(readFileSync(bar, "utf8"));
    lines = parseCard(readFileSync(card, "utf8"));
  } catch (error) {
    say(`eval-bar: cannot read the card or the bar: ${error.message}`);
    return 2;
  }
  const { rows, reds } = compare(baseline, lines);
  say(`eval-bar: ${card}`);
  say(`| suite | tier | bar | card | verdict |`);
  say(`|---|---|---|---|---|`);
  for (const row of rows) {
    const now = row.now === null ? "lost" : percent(row.now);
    say(`| ${row.suite} | ${row.tier} | ${percent(row.was)} | ${now} | ${row.red ? "RED" : "same"} |`);
  }
  say(`eval-bar: ${rows.length} rows, ${reds.length} red`);
  for (const red of reds) say(`RED ${red.suite} ${red.tier}: ${percent(red.was)} -> ${red.now === null ? "lost" : percent(red.now)}`);
  return reds.length ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}

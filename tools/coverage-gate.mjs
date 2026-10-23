#!/usr/bin/env node
// coverage-gate: hold a committed per-crate line-coverage floor (TG4).
// It reads one `cargo llvm-cov --json --summary-only` export (the same run the
// workflow does), rolls each source file up to its crate directory, and exits 1
// when a package in the bar falls under its floor. A package the run never
// touched is red too: a floor is only held by a number.
//   node tools/coverage-gate.mjs [--summary target/reports/coverage.json] [--bar tools/coverage-bar.json]
//   node tools/coverage-gate.mjs --help
// The bar is measured, never invented: every floor is the number one real run
// printed, and `_how` names the command that printed it (same rule as eval-bar).
// Exit: 0 every package holds, 1 under the floor or missing, 2 unreadable input.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const BAR_PATH = "tools/coverage-bar.json";
export const SUMMARY_PATH = "target/reports/coverage.json";

const HELP = `usage: node tools/coverage-gate.mjs [--summary <llvm-cov.json>] [--bar <bar.json>]

  no flag    read ${SUMMARY_PATH} against ${BAR_PATH}
  --summary  the llvm-cov --json --summary-only export to read
  --bar      compare against this bar instead of ${BAR_PATH} (a test fixture)
  --help     print this text, exit 0`;

/** The crate directory of one coverage file, as `crates/<dir>`, or null. */
export function crateDirOf(filename) {
  const m = /(?:^|[\\/])crates[\\/]+([A-Za-z0-9_-]+)[\\/]/.exec(String(filename ?? ""));
  return m ? `crates/${m[1]}` : null;
}

/** Line totals per crate directory from one summary export. */
export function rollup(summary) {
  const out = new Map();
  for (const block of summary?.data ?? []) {
    for (const file of block?.files ?? []) {
      const dir = crateDirOf(file?.filename);
      if (!dir) continue;
      const row = out.get(dir) ?? { covered: 0, count: 0 };
      row.covered += Number(file?.summary?.lines?.covered ?? 0) || 0;
      row.count += Number(file?.summary?.lines?.count ?? 0) || 0;
      out.set(dir, row);
    }
  }
  return out;
}

/** Line percentage to one decimal; null when the crate ran no line at all. */
export function percent(covered, count) {
  if (!count) return null;
  return Math.floor((1000 * covered) / count) / 10;
}

/** One row per bar package; a missing or empty crate is red. */
export function compare(bar, totals) {
  const rows = [];
  const reds = [];
  const gated = new Set();
  for (const [name, spec] of Object.entries(bar?.packages ?? {})) {
    gated.add(spec?.dir);
    const total = totals.get(spec?.dir);
    const now = total ? percent(total.covered, total.count) : null;
    const floor = Number(spec?.lines);
    const ok = now !== null && Number.isFinite(floor) && now >= floor;
    const row = { package: name, dir: spec?.dir, floor, now, ok };
    rows.push(row);
    if (!ok) reds.push(row);
  }
  // Crates the run measured that the bar does not gate yet: printed, never red.
  const others = [...totals.keys()]
    .filter((dir) => !gated.has(dir))
    .sort()
    .map((dir) => ({ dir, now: percent(totals.get(dir).covered, totals.get(dir).count) }));
  return { rows, reds, others, ok: reds.length === 0 };
}

const show = (n) => (n === null ? "not run" : `${n.toFixed(1)}%`);

export function main(argv = process.argv.slice(2), say = console.log) {
  let summary = join(ROOT, SUMMARY_PATH);
  let bar = join(ROOT, BAR_PATH);
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--help" || a === "-h") {
      say(HELP);
      return 0;
    }
    if ((a === "--summary" || a === "--bar") && i + 1 < argv.length) {
      const value = resolve(argv[++i]);
      if (a === "--summary") summary = value;
      else bar = value;
      continue;
    }
    say(`unknown argument: ${a}`);
    say(HELP);
    return 2;
  }
  if (!existsSync(summary)) {
    say(`coverage-gate: no summary at ${summary}; run cargo llvm-cov first`);
    return 2;
  }
  let parsed;
  let baseline;
  try {
    parsed = JSON.parse(readFileSync(summary, "utf8"));
    baseline = JSON.parse(readFileSync(bar, "utf8"));
  } catch (error) {
    say(`coverage-gate: cannot read the summary or the bar: ${error.message}`);
    return 2;
  }
  const { rows, reds, others, ok } = compare(baseline, rollup(parsed));
  say(`coverage-gate: ${summary}`);
  say("| package | dir | floor | lines | verdict |");
  say("|---|---|---:|---:|---|");
  for (const row of rows) {
    say(`| ${row.package} | ${row.dir} | ${show(row.floor)} | ${show(row.now)} | ${row.ok ? "holds" : "RED"} |`);
  }
  for (const other of others) {
    say(`report-only ${other.dir}: ${show(other.now)} (no floor in the bar)`);
  }
  for (const red of reds) {
    say(`RED ${red.package}: floor ${show(red.floor)}, run ${show(red.now)}`);
  }
  say(`coverage-gate: ${rows.length} packages, ${reds.length} under the floor`);
  return ok ? 0 : 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main();
}

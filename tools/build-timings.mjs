// build-timings: read a `cargo build --timings` report as the table a receipt
// pastes. Cargo writes the report to target/cargo-timings/cargo-timing*.html;
// the numbers a receipt needs are the top units by duration and the split
// between a unit's frontend and codegen sections.
//
// Two JS blocks hold the data: UNIT_DATA and CONCURRENCY_DATA. Each is read by
// balanced-bracket scanning and then JSON.parse, because a regex over a 100 KB
// HTML file breaks on the first brace inside a feature name.
//
// Times in the report are SECONDS. A unit's section start/end are seconds
// offset inside that unit, not absolute times.
//
// Run: node tools/build-timings.mjs [file] [--top N] [--json] [--min SECONDS]
// Exit: 0 a report was printed, 1 the file is missing or unreadable,
// 2 an argument is wrong.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const TIMINGS_DIR = join(ROOT, "target", "cargo-timings");

// A unit under this duration did not compile (a "todo" or "run-custom-build"
// entry carries zero seconds), so it is not one of the units compiled.
export const COMPILED_MIN_S = 0.01;

// The JS array that follows a marker, found by counting brackets so a string
// holding "]" cannot end the block early. Null when the marker is absent.
export function blockAfter(html, marker) {
  const at = html.indexOf(marker);
  if (at < 0) return null;
  const open = html.indexOf("[", at);
  if (open < 0) return null;
  let depth = 0;
  let inString = false;
  let escaped = false;
  for (let i = open; i < html.length; i += 1) {
    const ch = html[i];
    if (inString) {
      if (escaped) escaped = false;
      else if (ch === "\\") escaped = true;
      else if (ch === '"') inString = false;
      continue;
    }
    if (ch === '"') inString = true;
    else if (ch === "[") depth += 1;
    else if (ch === "]") {
      depth -= 1;
      if (depth === 0) return html.slice(open, i + 1);
    }
  }
  return null;
}

// The two blocks the report writes, or a one-line reason the file is no report.
export function parseReport(html) {
  const unitText = blockAfter(html, "const UNIT_DATA =");
  const concText = blockAfter(html, "const CONCURRENCY_DATA =");
  if (!unitText) throw new Error("no UNIT_DATA block: not a cargo timing report");
  let units;
  try {
    units = JSON.parse(unitText);
  } catch {
    throw new Error("UNIT_DATA is not valid JSON: the report looks truncated");
  }
  if (!Array.isArray(units)) throw new Error("UNIT_DATA is not an array");
  // Concurrency is a nice-to-have: an old report without it still shows units.
  let concurrency = [];
  if (concText) {
    try {
      concurrency = JSON.parse(concText);
    } catch {
      concurrency = [];
    }
  }
  return { units, concurrency: Array.isArray(concurrency) ? concurrency : [] };
}

// frontend runs before codegen inside one unit; a "total" section is a sum, not
// a part, so it is not one of the two columns.
export function splitSections(sections) {
  const out = { frontend_s: 0, codegen_s: 0 };
  for (const entry of Array.isArray(sections) ? sections : []) {
    const [name, span] = Array.isArray(entry) ? entry : [];
    if (name !== "frontend" && name !== "codegen") continue;
    const start = Number(span?.start) || 0;
    const end = Number(span?.end) || 0;
    out[`${name}_s`] += Math.max(0, end - start);
  }
  return out;
}

const round2 = (x) => Math.round(x * 100) / 100;

// wall is the last unit end; peak is the busiest active count; mean is the
// active count weighted by how long it held (a plain sample average would
// under-count the long plateaus).
export function summarize({ units, concurrency }, { min = 0, top = 10 } = {}) {
  const rows = units.map((unit) => {
    const duration_s = Number(unit.duration) || 0;
    return {
      name: String(unit.name ?? ""),
      duration_s: round2(duration_s),
      ...splitSections(unit.sections),
      features: Array.isArray(unit.features) ? unit.features.join(",") : String(unit.features ?? ""),
    };
  });
  const wall_s = round2(units.reduce((m, u) => Math.max(m, (Number(u.start) || 0) + (Number(u.duration) || 0)), 0));
  const compiled = units.filter((u) => (Number(u.duration) || 0) > COMPILED_MIN_S).length;
  const samples = concurrency.filter((s) => Number.isFinite(Number(s?.t)));
  const peak = samples.reduce((m, s) => Math.max(m, Number(s.active) || 0), 0);
  let weighted = 0;
  for (let i = 0; i < samples.length; i += 1) {
    const next = samples[i + 1];
    const dt = next ? Math.max(0, Number(next.t) - Number(samples[i].t)) : 0;
    weighted += (Number(samples[i].active) || 0) * dt;
  }
  const mean = wall_s > 0 ? weighted / wall_s : 0;
  const shown = rows
    .filter((r) => r.duration_s >= min)
    .sort((a, b) => b.duration_s - a.duration_s)
    .slice(0, top);
  return { wall_s, units: units.length, compiled, peak, mean: round2(mean), rows: shown };
}

export function parseArgs(argv) {
  const out = { file: null, top: 10, json: false, min: 0, help: false, error: null };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--top") {
      const n = Number(argv[++i]);
      if (!Number.isInteger(n) || n < 1) { out.error = "--top needs a whole number of 1 or more"; return out; }
      out.top = n;
    } else if (a === "--min") {
      const n = Number(argv[++i]);
      if (!Number.isFinite(n) || n < 0) { out.error = "--min needs a number of seconds, 0 or more"; return out; }
      out.min = n;
    } else if (a === "--json") out.json = true;
    else if (a === "-h" || a === "--help") out.help = true;
    else if (a.startsWith("-")) { out.error = `unknown flag ${a}`; return out; }
    else if (out.file) { out.error = "one report file at a time"; return out; }
    else out.file = a;
  }
  return out;
}

export function helpText() {
  return [
    "Usage: node tools/build-timings.mjs [file] [--top N] [--json] [--min SECONDS]",
    "  Reads a cargo `--timings` report (default: newest target/cargo-timings/cargo-timing*.html)",
    "  and prints wall seconds, units compiled, concurrency peak/mean and the slowest units.",
    "  --top N        rows to show, default 10",
    "  --min SECONDS  hide units under this duration, default 0",
    "  --json         one JSON object instead of the table",
  ].join("\n") + "\n";
}

// Newest by mtime, so the dated copy cargo wrote last is the one read.
export function newestReport(dir) {
  const names = readdirSync(dir).filter((n) => /^cargo-timing.*\.html$/.test(n));
  if (names.length === 0) return null;
  const newest = names
    .map((name) => ({ name, mtime: statSync(join(dir, name)).mtimeMs }))
    .sort((a, b) => b.mtime - a.mtime)[0];
  return join(dir, newest.name);
}

const label = (path) => {
  const rel = relative(ROOT, path);
  return rel.startsWith("..") ? path : rel.split("\\").join("/");
};

const main = (argv) => {
  const a = parseArgs(argv);
  if (a.help) { process.stdout.write(helpText()); return 0; }
  if (a.error) { console.log(`refused: ${a.error}`); return 2; }
  let file = a.file;
  if (!file) {
    try {
      file = newestReport(TIMINGS_DIR);
    } catch {
      file = null;
    }
    if (!file) {
      console.log("no report: run `cargo build --timings` first, or name a cargo-timing*.html file");
      return 1;
    }
  }
  let html;
  try {
    html = readFileSync(resolve(file), "utf8");
  } catch {
    console.log(`cannot read ${file}: no such timing report`);
    return 1;
  }
  let report;
  try {
    report = parseReport(html);
  } catch (error) {
    console.log(`cannot parse ${file}: ${error.message}`);
    return 1;
  }
  const summary = summarize(report, { min: a.min, top: a.top });
  if (a.json) {
    console.log(JSON.stringify({ file: label(resolve(file)), ...summary }, null, 2));
    return 0;
  }
  console.log(`cargo timings: ${label(resolve(file))}`);
  console.log(
    `wall ${summary.wall_s.toFixed(2)} s, ${summary.compiled} units compiled ` +
      `(${summary.units} entries), concurrency peak ${summary.peak}, mean ${summary.mean.toFixed(2)}`,
  );
  console.log("| duration s | unit | frontend s | codegen s | features |");
  console.log("|---|---|---|---|---|");
  for (const r of summary.rows) {
    console.log(`| ${r.duration_s.toFixed(2)} | ${r.name} | ${r.frontend_s.toFixed(2)} | ${r.codegen_s.toFixed(2)} | ${r.features} |`);
  }
  return 0;
};

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv.slice(2));
}

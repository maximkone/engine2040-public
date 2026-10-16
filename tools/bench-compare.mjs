// bench-compare: warmup + compare + markdown/json export for the bench ledger.
//
// Attribution: idea from sharkdp/hyperfine@c5161524980522320c7d3425e714cc0965a869a5
// (MIT/Apache-2.0, idea only, zero donor bytes) -- warmup runs discarded,
// timed runs summarized, relative-vs-baseline table exported as markdown and
// JSON. Written fresh here; no hyperfine code copied. See docs/DEPENDENCIES.md
// donor policy (idea/fresh-code only).
// PORT(sharkdp/hyperfine@c5161524980522320c7d3425e714cc0965a869a5:src/benchmark/mod.rs): warmup-iteration discard shape.
// PORT(sharkdp/hyperfine@c5161524980522320c7d3425e714cc0965a869a5:src/export/markdown.rs): MarkdownExporter table_row/table_divider shape.
//
// Job: time small named workloads (or shell commands via -- bench args),
// discard warmup, summarize runs, print/export a ledger row set.
// Pure: summarize/compare/toMarkdown/toJsonLedger give the same output
// for the same input; only runTimed/benchFns touch the wall clock.
//
// Usage:
//   node tools/bench-compare.mjs [--warmup N] [--runs N] [--export-md f] [--export-json f] [--format md|json|both] [--self-test] [--sort-demo] [--help]
//   node tools/bench-compare.mjs -- "node -v" "node --version"
//   bare run (no bench args): demo bench of two synthetic workloads, markdown to stdout, exit 0.

import { spawnSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { makeFixture, naiveSortUnique, sortOnce } from "./sort-once.mjs";

export const DEFAULT_WARMUP = 2;
export const DEFAULT_RUNS = 10;

/** Usage text (exported so tests pin it without spawning). */
export function helpText() {
  return [
    "Usage: node tools/bench-compare.mjs [options] [-- <command> ...]",
    "  Benchmark each <command> (shell string) or the demo workloads when none given.",
    "  Warmup runs are discarded; timed runs are summarized per benchmark.",
    "Options:",
    "  --warmup N        warmup runs per benchmark, discarded (default 2, >= 0)",
    "  --runs N          timed runs per benchmark (default 10, >= 1)",
    "  --export-md F     write the markdown table to file F",
    "  --export-json F   write the JSON ledger to file F",
    "  --format F        stdout shape: md (default), json, or both",
    "  --self-test       run pinned pure-function checks, print PASS/FAIL, exit 0/1",
    "  --sort-demo       bench the sort-once fixture (naive vs sortOnce) via the shared loop",
    "  --help, -h        print this help, exit 0",
    "Examples:",
    "  node tools/bench-compare.mjs --warmup 3 --runs 20 -- \"node -v\" \"node --version\"",
    "  node tools/bench-compare.mjs --export-md ledger.md --export-json ledger.json",
  ].join("\n");
}

/** Parse a non-negative integer CLI count (throws RangeError on bad input). */
export function parseCount(raw, name) {
  if (raw === null || raw === undefined) throw new RangeError(`${name} must be a non-negative integer, got ${JSON.stringify(raw)}`);
  if (typeof raw === "string" && raw.trim() === "") throw new RangeError(`${name} must be a non-negative integer, got ${JSON.stringify(raw)}`);
  const n = typeof raw === "number" ? raw : Number(`${raw ?? ""}`.trim());
  if (!Number.isInteger(n) || n < 0) throw new RangeError(`${name} must be a non-negative integer, got ${JSON.stringify(raw)}`);
  return n;
}

/** Mean of finite samples (>= 1). Throws on empty/bad input. */
export function mean(samples) {
  if (!Array.isArray(samples) || samples.length === 0) throw new RangeError("samples must be a non-empty array");
  let sum = 0;
  for (const s of samples) {
    if (typeof s !== "number" || !Number.isFinite(s) || s < 0) throw new RangeError(`sample must be finite ms >= 0, got ${JSON.stringify(s)}`);
    sum += s;
  }
  return sum / samples.length;
}

/**
 * Summarize run samples (ms) into one ledger row.
 * Pure: same samples give the same row. Population stddev (divide by n).
 */
export function summarize(name, samples) {
  if (typeof name !== "string" || name.trim() === "") throw new RangeError("benchmark name must be a non-empty string");
  if (!Array.isArray(samples) || samples.length === 0) throw new RangeError("samples must be a non-empty array");
  const sorted = [...samples];
  for (const s of sorted) {
    if (typeof s !== "number" || !Number.isFinite(s) || s < 0) throw new RangeError(`sample must be finite ms >= 0, got ${JSON.stringify(s)}`);
  }
  sorted.sort((a, b) => a - b);
  const n = sorted.length;
  // One sum pass in sorted order (same order mean() used): identical mean
  // without re-validating (checked above); multiply replaces ** 2 below.
  let sum = 0;
  for (let i = 0; i < n; i += 1) sum += sorted[i];
  const meanMs = sum / n;
  let variance = 0;
  for (let i = 0; i < n; i += 1) {
    const d = sorted[i] - meanMs;
    variance += d * d;
  }
  variance /= n;
  const mid = Math.floor(n / 2);
  const medianMs = n % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
  return {
    name,
    n,
    mean_ms: meanMs,
    stddev_ms: Math.sqrt(variance),
    min_ms: sorted[0],
    max_ms: sorted[n - 1],
    median_ms: medianMs,
  };
}

/**
 * Compare one candidate row against the baseline row.
 * Pure. ratio = candidate.mean / baseline.mean (< 1 means faster).
 */
export function compareRows(baseline, candidate) {
  if (!baseline || typeof baseline.mean_ms !== "number" || !(baseline.mean_ms > 0)) {
    throw new RangeError("baseline must have mean_ms > 0");
  }
  if (!candidate || typeof candidate.mean_ms !== "number" || !(candidate.mean_ms >= 0)) {
    throw new RangeError("candidate must have finite mean_ms >= 0");
  }
  const ratio = candidate.mean_ms / baseline.mean_ms;
  const speedup = baseline.mean_ms / Math.max(candidate.mean_ms, Number.EPSILON);
  const deltaPct = (ratio - 1) * 100;
  const winner = ratio < 1 ? candidate.name : ratio > 1 ? baseline.name : "tie";
  return { baseline: baseline.name, candidate: candidate.name, ratio, speedup, delta_pct: deltaPct, winner };
}

/** Format ms with two decimals plus unit (pure). */
export function formatMs(ms) {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) throw new RangeError(`ms must be finite >= 0, got ${ms}`);
  return `${ms.toFixed(2)}ms`;
}

/**
 * Render ledger rows as a markdown table, relative to rows[0] baseline.
 * Pure: same rows give the same text.
 */
export function toMarkdown(rows) {
  if (!Array.isArray(rows) || rows.length === 0) throw new RangeError("rows must be a non-empty array");
  const baseline = rows[0];
  const lines = [
    "| benchmark | n | mean | stddev | min | max | median | vs baseline |",
    "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
  ];
  for (const r of rows) {
    for (const k of ["name", "n", "mean_ms", "stddev_ms", "min_ms", "max_ms", "median_ms"]) {
      if (r[k] === undefined) throw new RangeError(`row ${JSON.stringify(r.name)} misses ${k}`);
    }
    let rel = "1.00x (baseline)";
    if (r.name !== baseline.name) {
      const c = compareRows(baseline, r);
      rel = `${c.ratio.toFixed(2)}x (${c.delta_pct >= 0 ? "+" : ""}${c.delta_pct.toFixed(1)}%)`;
    }
    lines.push(
      `| ${r.name} | ${r.n} | ${formatMs(r.mean_ms)} | ${formatMs(r.stddev_ms)} | ${formatMs(r.min_ms)} | ${formatMs(r.max_ms)} | ${formatMs(r.median_ms)} | ${rel} |`,
    );
  }
  return `${lines.join("\n")}\n`;
}

/**
 * Render ledger rows as a JSON ledger document string.
 * Pure: same rows/opts give the same text (key order stable).
 */
export function toJsonLedger(rows, opts = {}) {
  if (!Array.isArray(rows) || rows.length === 0) throw new RangeError("rows must be a non-empty array");
  const warmup = opts.warmup ?? DEFAULT_WARMUP;
  const runs = opts.runs ?? DEFAULT_RUNS;
  const doc = {
    tool: "bench-compare",
    version: 1,
    warmup,
    runs,
    baseline: rows[0].name,
    entries: rows.map((r) => ({
      name: r.name,
      n: r.n,
      mean_ms: r.mean_ms,
      stddev_ms: r.stddev_ms,
      min_ms: r.min_ms,
      max_ms: r.max_ms,
      median_ms: r.median_ms,
    })),
  };
  return `${JSON.stringify(doc, null, 2)}\n`;
}

/**
 * Time a sync function: run warmup (discarded), then runs timed samples.
 * Returns the timed samples array (length === runs). Durations via
 * performance.now(); fn return values are ignored.
 */
export function runTimed(fn, { warmup = DEFAULT_WARMUP, runs = DEFAULT_RUNS } = {}) {
  if (typeof fn !== "function") throw new TypeError("fn must be a function");
  const w = parseCount(warmup, "warmup");
  const r = parseCount(runs, "runs");
  if (r < 1) throw new RangeError(`runs must be >= 1, got ${r}`);
  for (let i = 0; i < w; i += 1) fn();
  const samples = [];
  for (let i = 0; i < r; i += 1) {
    const start = performance.now();
    fn();
    samples.push(performance.now() - start);
  }
  return samples;
}

/** Time one shell command string via the platform shell; returns ms (>= 0).
 * The command is the local operator's own CLI input (hyperfine-style); this
 * seam never takes network or file content. NUL bytes are refused (they
 * truncate at the OS exec layer), and the spawn locks shell:false so no
 * second shell layer can be introduced by option merging. */
export function timeCommand(cmd, { timeoutMs = 120_000 } = {}) {
  if (typeof cmd !== "string" || cmd.trim() === "") throw new RangeError("command must be a non-empty string");
  if (cmd.includes("\0")) throw new RangeError("command must not contain NUL bytes");
  if (typeof timeoutMs !== "number" || !Number.isFinite(timeoutMs) || timeoutMs <= 0) {
    throw new RangeError(`timeoutMs must be a finite number > 0, got ${JSON.stringify(timeoutMs)}`);
  }
  const start = performance.now();
  const opts = { encoding: "utf8", timeout: timeoutMs, shell: false };
  const prober = process.platform === "win32"
    ? spawnSync("cmd.exe", ["/d", "/s", "/c", cmd], opts)
    : spawnSync("sh", ["-c", cmd], opts);
  const ms = performance.now() - start;
  if (prober.error) throw new Error(`command failed to spawn ${JSON.stringify(cmd)}: ${String(prober.error.message ?? prober.error)}`);
  if (prober.status !== 0) throw new Error(`command exited ${prober.status}: ${JSON.stringify(cmd)}`);
  return ms;
}

/** Benchmark named sync functions with shared warmup/runs; returns rows. */
export function benchFns(defs, { warmup = DEFAULT_WARMUP, runs = DEFAULT_RUNS } = {}) {
  if (!Array.isArray(defs) || defs.length === 0) throw new RangeError("defs must be a non-empty array");
  return defs.map(({ name, fn }) => summarize(name, runTimed(fn, { warmup, runs })));
}

/** Benchmark shell command strings with warmup discarded; returns rows. */
export function benchCommands(commands, { warmup = DEFAULT_WARMUP, runs = DEFAULT_RUNS, timeoutMs = 120_000 } = {}) {
  if (!Array.isArray(commands) || commands.length === 0) throw new RangeError("commands must be a non-empty array");
  const w = parseCount(warmup, "warmup");
  const r = parseCount(runs, "runs");
  if (r < 1) throw new RangeError(`runs must be >= 1, got ${r}`);
  return commands.map((cmd) => {
    for (let i = 0; i < w; i += 1) timeCommand(cmd, { timeoutMs });
    const samples = [];
    for (let i = 0; i < r; i += 1) samples.push(timeCommand(cmd, { timeoutMs }));
    return summarize(cmd, samples);
  });
}

// Demo workloads (deterministic results; wall time is what is measured).
function demoSum() {
  let acc = 0;
  for (let i = 1; i <= 100_000; i += 1) acc += i;
  return acc;
}
function demoSort() {
  const arr = [];
  let state = 2040 >>> 0;
  for (let i = 0; i < 2000; i += 1) {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    arr.push(state % 100000);
  }
  arr.sort((a, b) => a - b);
  return arr.length;
}

/** Pinned pure-function checks; returns { pass, fail, lines }. */
export function selfTest() {
  const lines = [];
  let pass = 0;
  let fail = 0;
  const check = (label, fn) => {
    try {
      fn();
      pass += 1;
      lines.push(`ok ${label}`);
    } catch (e) {
      fail += 1;
      lines.push(`FAIL ${label}: ${e.message}`);
    }
  };
  check("summarize pins [10,20,30]", () => {
    const r = summarize("a", [10, 20, 30]);
    if (r.n !== 3) throw new Error(`n ${r.n}`);
    if (r.mean_ms !== 20) throw new Error(`mean ${r.mean_ms}`);
    if (r.min_ms !== 10 || r.max_ms !== 30 || r.median_ms !== 20) throw new Error("min/max/median");
    if (Math.abs(r.stddev_ms - Math.sqrt(200 / 3)) > 1e-9) throw new Error(`stddev ${r.stddev_ms}`);
  });
  check("compare ratio 2x slower", () => {
    const c = compareRows(summarize("base", [10, 10]), summarize("cand", [20, 20]));
    if (Math.abs(c.ratio - 2) > 1e-9) throw new Error(`ratio ${c.ratio}`);
    if (c.winner !== "base") throw new Error(`winner ${c.winner}`);
  });
  check("markdown has baseline + table", () => {
    const md = toMarkdown([summarize("base", [10, 10]), summarize("cand", [20, 20])]);
    if (!md.includes("| benchmark |")) throw new Error("missing header");
    if (!md.includes("baseline")) throw new Error("missing baseline cell");
    if (!md.includes("2.00x")) throw new Error("missing 2.00x");
  });
  check("json ledger parses with entries", () => {
    const doc = JSON.parse(toJsonLedger([summarize("base", [10, 10])], { warmup: 1, runs: 2 }));
    if (doc.tool !== "bench-compare" || doc.baseline !== "base") throw new Error("bad doc");
    if (!Array.isArray(doc.entries) || doc.entries.length !== 1) throw new Error("bad entries");
  });
  check("runTimed honors warmup+runs", () => {
    let calls = 0;
    const samples = runTimed(() => { calls += 1; }, { warmup: 3, runs: 4 });
    if (calls !== 7) throw new Error(`calls ${calls}`);
    if (samples.length !== 4) throw new Error(`samples ${samples.length}`);
  });
  return { pass, fail, lines };
}

function printHelp() {
  console.log(helpText());
}

function parseCli(argv) {
  const opts = { warmup: DEFAULT_WARMUP, runs: DEFAULT_RUNS, exportMd: null, exportJson: null, format: "md", selfTest: false, sortDemo: false, commands: [] };
  let i = 0;
  let positional = [];
  // Split off commands after a bare `--`.
  const dash = argv.indexOf("--");
  if (dash >= 0) {
    positional = argv.slice(dash + 1);
    argv = argv.slice(0, dash);
  }
  while (i < argv.length) {
    const a = argv[i];
    if (a === "--help" || a === "-h") {
      printHelp();
      process.exit(0);
    } else if (a === "--self-test") {
      opts.selfTest = true;
      i += 1;
    } else if (a === "--sort-demo") {
      opts.sortDemo = true;
      i += 1;
    } else if (a === "--warmup") {
      opts.warmup = parseCount(argv[i + 1], "warmup");
      i += 2;
    } else if (a.startsWith("--warmup=")) {
      opts.warmup = parseCount(a.slice("--warmup=".length), "warmup");
      i += 1;
    } else if (a === "--runs") {
      opts.runs = parseCount(argv[i + 1], "runs");
      if (opts.runs < 1) throw new RangeError(`runs must be >= 1, got ${opts.runs}`);
      i += 2;
    } else if (a.startsWith("--runs=")) {
      opts.runs = parseCount(a.slice("--runs=".length), "runs");
      if (opts.runs < 1) throw new RangeError(`runs must be >= 1, got ${opts.runs}`);
      i += 1;
    } else if (a === "--export-md") {
      opts.exportMd = argv[i + 1];
      if (!opts.exportMd) throw new RangeError("--export-md needs a file path");
      i += 2;
    } else if (a.startsWith("--export-md=")) {
      opts.exportMd = a.slice("--export-md=".length);
      if (!opts.exportMd) throw new RangeError("--export-md needs a file path");
      i += 1;
    } else if (a === "--export-json") {
      opts.exportJson = argv[i + 1];
      if (!opts.exportJson) throw new RangeError("--export-json needs a file path");
      i += 2;
    } else if (a.startsWith("--export-json=")) {
      opts.exportJson = a.slice("--export-json=".length);
      if (!opts.exportJson) throw new RangeError("--export-json needs a file path");
      i += 1;
    } else if (a === "--format") {
      opts.format = argv[i + 1];
      i += 2;
    } else if (a.startsWith("--format=")) {
      opts.format = a.slice("--format=".length);
      i += 1;
    } else if (a.startsWith("-")) {
      console.error(`bench-compare: error: unknown flag ${a}`);
      process.exit(2);
    } else {
      positional.push(a);
      i += 1;
    }
  }
  if (!["md", "json", "both"].includes(opts.format)) throw new RangeError(`--format must be md|json|both, got ${JSON.stringify(opts.format)}`);
  opts.commands = positional;
  return opts;
}

function main(argv) {
  let opts;
  try {
    opts = parseCli(argv);
  } catch (e) {
    console.error(`bench-compare: error: ${e.message}`);
    process.exit(2);
  }
  if (opts.selfTest) {
    const r = selfTest();
    for (const l of r.lines) console.log(l);
    console.log(r.fail === 0 ? `PASS bench-compare self-test ${r.pass}/${r.pass + r.fail}` : `FAIL bench-compare self-test ${r.pass} pass ${r.fail} fail`);
    process.exit(r.fail === 0 ? 0 : 1);
  }
  let rows;
  try {
    if (opts.sortDemo) {
      const f = makeFixture();
      rows = benchFns(
        [
          { name: "naive", fn: () => naiveSortUnique(f) },
          { name: "once", fn: () => sortOnce(f) },
        ],
        { warmup: opts.warmup, runs: opts.runs },
      );
    } else if (opts.commands.length > 0) {
      rows = benchCommands(opts.commands, { warmup: opts.warmup, runs: opts.runs });
    } else {
      rows = benchFns(
        [
          { name: "sum-100k", fn: demoSum },
          { name: "sort-2k", fn: demoSort },
        ],
        { warmup: opts.warmup, runs: opts.runs },
      );
    }
  } catch (e) {
    console.error(`bench-compare: error: ${e.message}`);
    process.exit(1);
  }
  const md = toMarkdown(rows);
  const json = toJsonLedger(rows, { warmup: opts.warmup, runs: opts.runs });
  try {
    if (opts.exportMd) writeFileSync(opts.exportMd, md, "utf8");
    if (opts.exportJson) writeFileSync(opts.exportJson, json, "utf8");
  } catch (e) {
    console.error(`bench-compare: error: export failed: ${e.message}`);
    process.exit(1);
  }
  if (opts.format === "md") console.log(md.trimEnd());
  else if (opts.format === "json") console.log(json.trimEnd());
  else console.log(`${md.trimEnd()}\n\n\`\`\`json\n${json.trimEnd()}\n\`\`\``);
}

if (process.argv[1] && process.argv[1].replace(/\\/g, "/").endsWith("tools/bench-compare.mjs")) {
  main(process.argv.slice(2));
}

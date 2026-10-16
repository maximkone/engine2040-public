// Run: node --test tools/bench-compare.test.mjs
// bench-compare is the hyperfine-idea warmup + markdown/json exporter for the
// bench ledger: warmup runs are discarded, timed runs are summarized, and the
// rows export as a markdown table plus a JSON ledger. Pure functions only
// below except the --help spawn and the export round-trip into $env:TEMP.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  benchFns,
  compareRows,
  formatMs,
  helpText,
  mean,
  parseCount,
  runTimed,
  summarize,
  toJsonLedger,
  toMarkdown,
} from "./bench-compare.mjs";

test("parseCount accepts zero and rejects bad counts", () => {
  assert.equal(parseCount(0, "warmup"), 0);
  assert.equal(parseCount("3", "warmup"), 3);
  for (const bad of ["-1", "1.5", "abc", "", null, undefined, NaN, -2]) {
    assert.throws(() => parseCount(bad, "warmup"), RangeError, JSON.stringify(bad));
  }
});

test("mean pins the average and rejects empty/bad samples", () => {
  assert.equal(mean([10, 20, 30]), 20);
  assert.equal(mean([5]), 5);
  assert.throws(() => mean([]), RangeError);
  assert.throws(() => mean([10, -1]), RangeError);
  assert.throws(() => mean([10, NaN]), RangeError);
});

test("summarize pins n/mean/min/max/median/population-stddev", () => {
  const r = summarize("a", [10, 20, 30]);
  assert.equal(r.n, 3);
  assert.equal(r.mean_ms, 20);
  assert.equal(r.min_ms, 10);
  assert.equal(r.max_ms, 30);
  assert.equal(r.median_ms, 20);
  assert.ok(Math.abs(r.stddev_ms - Math.sqrt(200 / 3)) < 1e-9);
  assert.equal(summarize("even", [10, 20]).median_ms, 15);
  assert.throws(() => summarize("", [10]), RangeError);
  assert.throws(() => summarize("a", []), RangeError);
});

test("compareRows reports ratio/speedup/winner against baseline", () => {
  const base = summarize("base", [10, 10]);
  const slower = summarize("slow", [20, 20]);
  const faster = summarize("fast", [5, 5]);
  const c1 = compareRows(base, slower);
  assert.equal(c1.ratio, 2);
  assert.equal(c1.winner, "base");
  assert.ok(c1.delta_pct > 0);
  const c2 = compareRows(base, faster);
  assert.equal(c2.ratio, 0.5);
  assert.equal(c2.winner, "fast");
  assert.ok(c2.delta_pct < 0);
  assert.throws(() => compareRows({ name: "x", mean_ms: 0 }, slower), RangeError);
});

test("formatMs prints two decimals with unit", () => {
  assert.equal(formatMs(0), "0.00ms");
  assert.equal(formatMs(1.5), "1.50ms");
  assert.throws(() => formatMs(-1), RangeError);
});

test("toMarkdown tables the baseline plus relative ratios", () => {
  const md = toMarkdown([summarize("base", [10, 10]), summarize("slow", [20, 20])]);
  assert.match(md, /^\| benchmark \| n \| mean \|/);
  assert.match(md, /\| base \| 2 \| 10\.00ms/);
  assert.match(md, /1\.00x \(baseline\)/);
  assert.match(md, /2\.00x \(\+100\.0%\)/);
  assert.throws(() => toMarkdown([]), RangeError);
});

test("toJsonLedger writes a parseable ledger with baseline + entries", () => {
  const doc = JSON.parse(toJsonLedger([summarize("base", [10, 10])], { warmup: 1, runs: 2 }));
  assert.equal(doc.tool, "bench-compare");
  assert.equal(doc.baseline, "base");
  assert.equal(doc.warmup, 1);
  assert.equal(doc.runs, 2);
  assert.equal(doc.entries.length, 1);
  assert.equal(doc.entries[0].mean_ms, 10);
});

test("runTimed discards warmup and returns exactly runs samples", () => {
  let calls = 0;
  const samples = runTimed(() => { calls += 1; }, { warmup: 3, runs: 4 });
  assert.equal(calls, 7);
  assert.equal(samples.length, 4);
  for (const s of samples) assert.ok(s >= 0);
  assert.throws(() => runTimed(() => {}, { warmup: 0, runs: 0 }), RangeError);
});

test("benchFns benchmarks named functions with shared warmup/runs", () => {
  const rows = benchFns(
    [
      { name: "noop", fn: () => {} },
      { name: "loop", fn: () => { let a = 0; for (let i = 0; i < 1000; i += 1) a += i; return a; } },
    ],
    { warmup: 1, runs: 3 },
  );
  assert.equal(rows.length, 2);
  assert.equal(rows[0].name, "noop");
  assert.equal(rows[0].n, 3);
  assert.throws(() => benchFns([], { warmup: 0, runs: 1 }), RangeError);
});

test("helpText documents warmup and markdown/json export", () => {
  assert.match(helpText(), /--warmup/);
  assert.match(helpText(), /--export-md/);
  assert.match(helpText(), /--export-json/);
  assert.match(helpText(), /--help/);
});

test("--help exits 0 and prints usage", () => {
  const tool = fileURLToPath(new URL("./bench-compare.mjs", import.meta.url));
  const r = spawnSync(process.execPath, [tool, "--help"], { encoding: "utf8" });
  assert.equal(r.status, 0);
  assert.match(r.stdout, /Usage: node tools\/bench-compare\.mjs/);
});

test("export round-trip: --export-md/--export-json write ledger files", () => {
  const dir = mkdtempSync(join(tmpdir(), "bench-compare-"));
  try {
    const tool = fileURLToPath(new URL("./bench-compare.mjs", import.meta.url));
    const mdFile = join(dir, "ledger.md");
    const jsonFile = join(dir, "ledger.json");
    const r = spawnSync(
      process.execPath,
      [tool, "--warmup", "1", "--runs", "2", "--export-md", mdFile, "--export-json", jsonFile],
      { encoding: "utf8" },
    );
    assert.equal(r.status, 0);
    const md = readFileSync(mdFile, "utf8");
    assert.match(md, /\| benchmark \|/);
    const doc = JSON.parse(readFileSync(jsonFile, "utf8"));
    assert.equal(doc.tool, "bench-compare");
    assert.ok(doc.entries.length >= 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

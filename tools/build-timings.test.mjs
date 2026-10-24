// Run: node --test tools/build-timings.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { blockAfter, newestReport, parseArgs, parseReport, summarize } from "./build-timings.mjs";

const TOOL = join(dirname(fileURLToPath(import.meta.url)), "build-timings.mjs");

// A report as cargo writes it: the two blocks sit inside <script>, and one
// feature name holds a "]" that a regex would take for the end of the array.
const FIXTURE = `<html><body>
<script>
const UNIT_DATA = [
  { "i": 1, "name": "engine-render", "version": "0.1.0", "mode": "todo", "features": ["std", "wgpu]x"],
    "start": 0.5, "duration": 20.0,
    "sections": [["frontend", { "start": 0.0, "end": 2.0 }], ["codegen", { "start": 2.0, "end": 20.0 }]] },
  { "i": 2, "name": "serde", "version": "1.0.229", "mode": "todo", "features": ["derive"],
    "start": 1.0, "duration": 5.0, "sections": null },
  { "i": 3, "name": "engine-core", "version": "0.1.0", "mode": "todo", "features": [],
    "start": 2.0, "duration": 0.5,
    "sections": [["frontend", { "start": 0.0, "end": 0.1 }], ["codegen", { "start": 0.1, "end": 0.5 }]] },
  { "i": 4, "name": "proc-macro2", "version": "1.0.107", "mode": "run-custom-build", "features": [],
    "start": 0.0, "duration": 0.0, "sections": null }
];
const CONCURRENCY_DATA = [
  { "t": 0.0, "active": 0, "waiting": 0, "inactive": 4 },
  { "t": 1.0, "active": 4, "waiting": 0, "inactive": 0 },
  { "t": 3.0, "active": 2, "waiting": 0, "inactive": 2 },
  { "t": 20.5, "active": 0, "waiting": 0, "inactive": 4 }
];
</script></body></html>`;

test("the blocks are found by bracket depth, not by a regex", () => {
  const text = blockAfter(FIXTURE, "const UNIT_DATA =");
  assert.ok(text.startsWith("[") && text.endsWith("]"));
  const units = JSON.parse(text);
  assert.equal(units.length, 4, "the ] inside a feature name does not end the array");
  assert.deepEqual(units[0].features, ["std", "wgpu]x"]);
  assert.equal(blockAfter(FIXTURE, "const NOPE ="), null);
});

test("a unit's sections split into frontend and codegen seconds", () => {
  const { units } = parseReport(FIXTURE);
  const s = summarize({ units, concurrency: [] });
  const render = s.rows.find((r) => r.name === "engine-render");
  assert.equal(render.frontend_s, 2);
  assert.equal(render.codegen_s, 18, "codegen is the rest of the unit");
  const serde = s.rows.find((r) => r.name === "serde");
  assert.equal(serde.frontend_s, 0);
  assert.equal(serde.codegen_s, 0, "a null sections list has no split");
});

test("wall is the last unit end and only real builds count as compiled", () => {
  const s = summarize(parseReport(FIXTURE));
  assert.equal(s.wall_s, 20.5, "0.5 + 20.0 is the last unit end");
  assert.equal(s.units, 4);
  assert.equal(s.compiled, 3, "the zero-second todo unit did not compile");
});

test("concurrency reads peak and the time-weighted mean", () => {
  const s = summarize(parseReport(FIXTURE));
  assert.equal(s.peak, 4);
  assert.equal(s.mean, 2.1, "(4*2 + 2*17.5) / 20.5");
});

test("rows sort by duration and --min and --top cut them", () => {
  const report = parseReport(FIXTURE);
  assert.deepEqual(summarize(report).rows.map((r) => r.name), ["engine-render", "serde", "engine-core", "proc-macro2"]);
  assert.deepEqual(summarize(report, { min: 1 }).rows.map((r) => r.name), ["engine-render", "serde"]);
  assert.deepEqual(summarize(report, { top: 1 }).rows.map((r) => r.name), ["engine-render"]);
});

test("the JSON object carries the receipt fields", () => {
  const s = summarize(parseReport(FIXTURE));
  const out = { file: "x.html", ...s };
  assert.deepEqual(Object.keys(out), ["file", "wall_s", "units", "compiled", "peak", "mean", "rows"]);
  assert.deepEqual(Object.keys(out.rows[0]), ["name", "duration_s", "frontend_s", "codegen_s", "features"]);
  assert.equal(out.rows[0].features, "std,wgpu]x");
});

test("the arguments are read without surprises", () => {
  const a = parseArgs(["a.html", "--top", "3", "--min", "1.5", "--json"]);
  assert.equal(a.error, null);
  assert.equal(a.file, "a.html");
  assert.equal(a.top, 3);
  assert.equal(a.min, 1.5);
  assert.equal(a.json, true);
  assert.equal(parseArgs([]).top, 10, "ten rows by default");
  assert.equal(parseArgs([]).min, 0);
  assert.match(parseArgs(["--top", "0"]).error, /--top needs/);
  assert.match(parseArgs(["--min", "x"]).error, /--min needs/);
  assert.match(parseArgs(["--nope"]).error, /unknown flag/);
  assert.match(parseArgs(["a.html", "b.html"]).error, /one report file/);
});

test("the newest report is the last one cargo wrote", () => {
  const dir = mkdtempSync(join(tmpdir(), "build-timings-"));
  writeFileSync(join(dir, "cargo-timing.html"), "old");
  writeFileSync(join(dir, "cargo-timing-20260101T000000Z-1.html"), "new");
  writeFileSync(join(dir, "notes.txt"), "x");
  assert.equal(newestReport(dir), join(dir, "cargo-timing-20260101T000000Z-1.html"));
});

test("the CLI prints the table and the JSON, and exits 1 on a bad file", () => {
  const dir = mkdtempSync(join(tmpdir(), "build-timings-cli-"));
  const file = join(dir, "cargo-timing.html");
  writeFileSync(file, FIXTURE);
  const table = spawnSync(process.execPath, [TOOL, file, "--top", "2"], { encoding: "utf8" });
  assert.equal(table.status, 0, table.stderr);
  assert.match(table.stdout, /wall 20\.50 s, 3 units compiled/);
  assert.match(table.stdout, /\| 20\.00 \| engine-render \| 2\.00 \| 18\.00 \|/);
  assert.match(table.stdout, /concurrency peak 4, mean 2\.10/);
  assert.equal(table.stdout.trimEnd().split("\n").length, 6, "header, header line, two table lines, two rows");

  const json = spawnSync(process.execPath, [TOOL, file, "--json", "--min", "4"], { encoding: "utf8" });
  assert.equal(json.status, 0, json.stderr);
  const parsed = JSON.parse(json.stdout);
  assert.deepEqual(parsed.rows.map((r) => r.name), ["engine-render", "serde"]);

  const missing = spawnSync(process.execPath, [TOOL, join(dir, "gone.html")], { encoding: "utf8" });
  assert.equal(missing.status, 1);
  assert.equal(missing.stdout.trim().split("\n").length, 1, "one line, no stack");
  assert.match(missing.stdout, /cannot read/);
});

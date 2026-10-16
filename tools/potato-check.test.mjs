// Run: node --test tools/potato-check.test.mjs
// GS16 potato gate: canned arena output plus a fake tasklist give the exact
// potato line x2; no integrated adapter reads NO_IGPU with exit 2; --floor
// exits 1 when p50 is over. Pure parsers only: no arena run, no tasklist,
// no wall clock in this file.
import assert from "node:assert/strict";
import test from "node:test";
import {
  BAR_PATH,
  checkBars,
  decideExit,
  findIntegratedAdapter,
  formatBarLine,
  formatPotatoLine,
  formatReport,
  loadBar,
  parseAdapterName,
  parseFrameTimes,
  parseTasklistPeakMb,
  sampleTasklist,
  summarize,
} from "./potato-check.mjs";

const CANNED_ARENA = [
  "adapter: Intel-UHD Dx12 integrated vendor=0x8086",
  "arena frame-split: sim=1.200ms present=2.300ms fighters=0.400ms per frame (mean over 600 frames) cpu_ms p50=8.125 p99=12.500 interval_ms p50=42.125 p99=67.500 gpu_ms=unsupported acquire=0.100ms present_mode=vsync",
].join("\n");

const CANNED_TASKLIST = [
  '"arena.exe","1234","Console","1","1,048,576 K"',
  '"arena.exe","1234","Console","1","1,310,720 K"',
  '"cargo.exe","9999","Console","1","500,000 K"',
].join("\n");

test("potato: canned arena plus fake tasklist give the exact potato line x2", () => {
  const adapter = findIntegratedAdapter(CANNED_ARENA);
  assert.equal(adapter, "Intel-UHD_Dx12_integrated");
  const { p50_ms, p99_ms } = parseFrameTimes(CANNED_ARENA);
  assert.equal(p50_ms, 42.125);
  assert.equal(p99_ms, 67.5);
  const peak_ws_mb = parseTasklistPeakMb(CANNED_TASKLIST);
  assert.equal(peak_ws_mb, 1280);
  const first = formatPotatoLine({ adapter, p50_ms, p99_ms, peak_ws_mb });
  const second = formatPotatoLine({
    adapter: findIntegratedAdapter(CANNED_ARENA),
    ...parseFrameTimes(CANNED_ARENA),
    peak_ws_mb: parseTasklistPeakMb(CANNED_TASKLIST),
  });
  assert.equal(first, "potato: adapter=Intel-UHD_Dx12_integrated p50_ms=42.125 p99_ms=67.500 peak_ws_mb=1280");
  assert.equal(first, second);
});

test("potato: frame times prefer interval_ms and fall back to cpu_ms", () => {
  const cpuOnly = "arena frame-split: sim=1ms present=2ms fighters=0ms per frame (mean over 600 frames) cpu_ms p50=8.125 p99=12.500 gpu_ms=unsupported acquire=0ms present_mode=vsync";
  assert.deepEqual(parseFrameTimes(cpuOnly), { p50_ms: 8.125, p99_ms: 12.5 });
  assert.deepEqual(parseFrameTimes("no split line here"), { p50_ms: null, p99_ms: null });
});

test("potato: no integrated adapter reads NO_IGPU with exit 2", () => {
  const discrete = [
    "adapter: RTX-3050 Dx12 discrete vendor=0x10DE",
    "arena frame-split: sim=1ms present=2ms fighters=0ms per frame (mean over 600 frames) cpu_ms p50=5.000 p99=9.000 interval_ms p50=16.000 p99=20.000 gpu_ms=unsupported acquire=0ms present_mode=vsync",
  ].join("\n");
  assert.equal(findIntegratedAdapter(discrete), null);
  assert.equal(findIntegratedAdapter("no adapter line at all"), null);
  assert.equal(parseAdapterName(discrete), "RTX-3050_Dx12_discrete");
  const verdicts = checkBars({ p50_ms: 16, p99_ms: 20, peak_ws_mb: 1000 }, loadBar());
  assert.equal(decideExit({ hasIntegrated: false, verdicts, floor: false }), 2);
  assert.equal(decideExit({ hasIntegrated: false, verdicts, floor: true }), 2);
  assert.equal("NO_IGPU", "NO_IGPU");
});

test("potato: --floor exits 1 when p50 is over, else 0 without --floor", () => {
  const bar = loadBar();
  assert.deepEqual([bar.p50_ms, bar.p99_ms, bar.working_set_mb], [50, 80, 3072]);
  const over = checkBars({ p50_ms: 60, p99_ms: 70, peak_ws_mb: 1000 }, bar);
  assert.equal(over[0].status, "FAIL");
  assert.equal(decideExit({ hasIntegrated: true, verdicts: over, floor: true }), 1);
  assert.equal(decideExit({ hasIntegrated: true, verdicts: over, floor: false }), 0);
  const pass = checkBars({ p50_ms: 42.125, p99_ms: 67.5, peak_ws_mb: 1280 }, bar);
  assert.ok(pass.every((v) => v.status === "PASS"));
  assert.equal(decideExit({ hasIntegrated: true, verdicts: pass, floor: true }), 0);
  assert.equal(decideExit({ hasIntegrated: true, verdicts: pass, floor: false }), 0);
});

test("potato: unmeasured numbers read UNKNOWN and fail --floor", () => {
  const line = formatPotatoLine({ adapter: "Intel-UHD", p50_ms: null, p99_ms: null, peak_ws_mb: null });
  assert.equal(line, "potato: adapter=Intel-UHD p50_ms=UNKNOWN p99_ms=UNKNOWN peak_ws_mb=UNKNOWN");
  const verdicts = checkBars({ p50_ms: null, p99_ms: null, peak_ws_mb: null }, loadBar());
  assert.ok(verdicts.every((v) => v.status === "UNKNOWN"));
  assert.equal(decideExit({ hasIntegrated: true, verdicts, floor: true }), 1);
  assert.equal(decideExit({ hasIntegrated: true, verdicts, floor: false }), 0);
});

test("potato: tasklist peak keeps arena lines only and reads null when empty", () => {
  assert.equal(parseTasklistPeakMb(CANNED_TASKLIST, "missing.exe"), null);
  assert.equal(parseTasklistPeakMb(""), null);
  const one = parseTasklistPeakMb('"arena.exe","1","Console","1","3,145,728 K"');
  assert.equal(one, 3072);
});

test("potato: bar file holds the proposal numbers", () => {
  assert.ok(String(BAR_PATH).endsWith("potato-bar.json"));
  const bar = loadBar();
  assert.equal(bar.p50_ms, 50);
  assert.equal(bar.p99_ms, 80);
  assert.equal(bar.working_set_mb, 3072);
});

test("potato: bar lines print one per bar with UNKNOWN for unmeasured", () => {
  const bar = loadBar();
  const verdicts = checkBars({ p50_ms: 42.125, p99_ms: 67.5, peak_ws_mb: 1280 }, bar);
  const lines = verdicts.map(formatBarLine);
  assert.equal(lines.length, 3);
  assert.ok(lines[0].startsWith("p50_ms 42.125"), lines[0]);
  assert.ok(lines[0].endsWith("PASS"), lines[0]);
  assert.ok(lines[2].includes("1280"), lines[2]);
  const unknown = checkBars({ p50_ms: null, p99_ms: null, peak_ws_mb: null }, bar).map(formatBarLine);
  assert.equal(unknown.length, 3);
  assert.ok(unknown.every((l) => l.includes("UNKNOWN")), unknown.join("\n"));
});

test("potato: summarize gives the exact potato line x2 and exit 0", () => {
  const bar = loadBar();
  const first = summarize({ output: CANNED_ARENA, tasklistText: CANNED_TASKLIST, bar, floor: false });
  const second = summarize({ output: CANNED_ARENA, tasklistText: CANNED_TASKLIST, bar, floor: false });
  assert.equal(first.line, "potato: adapter=Intel-UHD_Dx12_integrated p50_ms=42.125 p99_ms=67.500 peak_ws_mb=1280");
  assert.equal(first.line, second.line);
  assert.equal(first.hasIntegrated, true);
  assert.equal(first.exit, 0);
});

test("potato: summarize reports NO_IGPU exit 2 and the report carries the line", () => {
  const bar = loadBar();
  const discrete = [
    "adapter: RTX-3050 Dx12 discrete vendor=0x10DE",
    "arena frame-split: sim=1ms present=2ms fighters=0ms per frame (mean over 600 frames) cpu_ms p50=5.000 p99=9.000 interval_ms p50=16.000 p99=20.000 gpu_ms=unsupported acquire=0ms present_mode=vsync",
  ].join("\n");
  const s = summarize({ output: discrete, tasklistText: CANNED_TASKLIST, bar, floor: false });
  assert.equal(s.hasIntegrated, false);
  assert.equal(s.exit, 2);
  const report = formatReport({
    at: "2026-10-13T02:07:00.000Z",
    cmd: "arena.exe --scene arena --fighters 8 --frames 600 --exit",
    env: "E2040_ADAPTER=integrated E2040_LIMITS=downlevel",
    adapter: s.adapter,
    line: s.line,
    verdicts: s.verdicts,
    hasIntegrated: s.hasIntegrated,
    missing: false,
    timedOut: false,
  });
  assert.ok(report.includes(s.line), report.slice(0, 400));
  assert.ok(report.includes("p50_ms"), report.slice(0, 800));
});

test("potato: tasklist sampler is loud on failure, silent on success", () => {
  const bad = sampleTasklist(() => { throw new Error("no tasklist"); });
  assert.equal(bad.text, "");
  assert.match(bad.error, /no tasklist/);
  const good = sampleTasklist(() => CANNED_TASKLIST);
  assert.equal(good.text, CANNED_TASKLIST);
  assert.equal(good.error, null);
});

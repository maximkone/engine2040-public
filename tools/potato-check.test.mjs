// Run: node --test tools/potato-check.test.mjs
// GS16 potato gate: canned arena output plus a fake tasklist give the exact
// potato line x2; no integrated adapter reads NO_IGPU with exit 2; --floor
// exits 1 when p50 is over. Pure parsers only: no arena run, no tasklist,
// no wall clock in this file.
import assert from "node:assert/strict";
import test from "node:test";
import {
  BAR_PATH,
  arenaAdapterName,
  checkBars,
  classifyAdapterKind,
  decideExit,
  findIntegratedAdapter,
  formatBarLine,
  formatLoadLine,
  formatLoadText,
  formatPotatoLine,
  formatReport,
  loadBar,
  parseAdapterName,
  parseAdapterTypeWord,
  parseBuildLoad,
  parseFrameTimes,
  parsePerPassAdapter,
  parseTasklistPeakMb,
  sampleTasklist,
  summarize,
} from "./potato-check.mjs";
// Namespace import for the tool entry point (`run`): the Accept says the
// *tool* prints NO_IGPU / returns non-zero, so the tests drive the real
// print, exit and report path with injected seams, never a live arena.
import * as potato from "./potato-check.mjs";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join as joinPath } from "node:path";

const CANNED_ARENA = [
  "adapter: Intel-UHD Dx12 integrated vendor=0x8086",
  "arena frame-split: sim=1.200ms present=2.300ms fighters=0.400ms per frame (mean over 600 frames) cpu_ms p50=8.125 p99=12.500 interval_ms p50=42.125 p99=67.500 gpu_ms=unsupported acquire=0.100ms present_mode=vsync",
].join("\n");

// Real bytes from this PC's Intel UHD, 2026-10-17: arena prints no
// `adapter:` line, only `arena per-pass: adapter="..."`. The gate must read
// the UHD off this shape, else the potato run always reads NO_IGPU.
const LIVE_ARENA = [
  'arena: presenter surface: Intel(R) UHD Graphics (32.0.101.7088 ) 1280x720',
  'arena per-pass: adapter="Intel(R) UHD Graphics" driver="32.0.101.7088 " resolution=1280x720 frames=60 sky=0.000ms layers=0.000ms particles=0.000ms decals=0.000ms ibl=7.480ms fog=0.000ms gtao=0.000ms shadow=0.000ms forward=1.631ms validation_errors=0',
  "arena frame-split: sim=0.714ms present=75.758ms fighters=12.463ms per frame (mean over 60 frames) cpu_ms p50=12.644 p99=2210.562 interval_ms p50=13.042 p99=2691.790 gpu_ms p50=2.194 p99=2.457 (forward p50=1.631 blit p50=0.564; 59 frames) acquire=0.082ms present_mode=Immediate",
].join("\n");

const CANNED_TASKLIST = [
  '"arena.exe","1234","Console","1","1,048,576 K"',
  '"arena.exe","1234","Console","1","1,310,720 K"',
  '"cargo.exe","9999","Console","1","500,000 K"',
].join("\n");

test("potato: live arena per-pass output names the UHD and reads integrated", () => {
  assert.equal(parsePerPassAdapter(LIVE_ARENA), "Intel(R) UHD Graphics");
  assert.equal(arenaAdapterName(LIVE_ARENA), "Intel(R)_UHD_Graphics");
  const measured = { p50_ms: 13.042, p99_ms: 2691.79, peak_ws_mb: 1235 };
  const s = summarize({ output: LIVE_ARENA, tasklistText: CANNED_TASKLIST, bar: loadBar(), floor: false });
  assert.equal(s.adapter, "Intel(R)_UHD_Graphics");
  assert.equal(s.hasIntegrated, true);
  assert.equal(s.measured.p50_ms, 13.042);
  assert.equal(
    s.line,
    "potato: adapter=Intel(R)_UHD_Graphics p50_ms=13.042 p99_ms=2691.790 peak_ws_mb=1280",
  );
  assert.equal(s.exit, 0);
  const again = summarize({ output: LIVE_ARENA, tasklistText: CANNED_TASKLIST, bar: loadBar(), floor: false });
  assert.equal(s.line, again.line);
  assert.deepEqual(checkBars(measured, loadBar()).map((v) => v.status), ["PASS", "FAIL", "PASS"]);
});

test("potato: kind comes from the name, and a software adapter never passes", () => {
  assert.equal(classifyAdapterKind("Intel(R) UHD Graphics"), "integrated");
  assert.equal(classifyAdapterKind("Intel(R) Iris(R) Xe Graphics"), "integrated");
  assert.equal(classifyAdapterKind("AMD Radeon(TM) Graphics"), "integrated");
  assert.equal(classifyAdapterKind("NVIDIA GeForce RTX 3050 6GB Laptop GPU"), "discrete");
  assert.equal(classifyAdapterKind("AMD Radeon RX 6600"), "discrete");
  assert.equal(classifyAdapterKind("Microsoft Basic Render Driver"), "cpu");
  assert.equal(classifyAdapterKind(""), "unknown");
  const software = [
    'arena per-pass: adapter="Microsoft Basic Render Driver" driver="\\\\.\\GLOBALROOT" frames=60 validation_errors=0',
    "arena frame-split: sim=1ms present=2ms fighters=0ms per frame (mean over 600 frames) cpu_ms p50=5.000 p99=9.000 interval_ms p50=16.000 p99=20.000 gpu_ms=unsupported acquire=0ms present_mode=vsync",
  ].join("\n");
  assert.equal(arenaAdapterName(software), "Microsoft_Basic_Render_Driver");
  assert.equal(findIntegratedAdapter(software), null);
  assert.equal(
    decideExit({ hasIntegrated: false, verdicts: checkBars({ p50_ms: 16, p99_ms: 20, peak_ws_mb: 1000 }, loadBar()), floor: false }),
    2,
  );
});

test("potato: an explicit kind word wins over the live name", () => {
  const both = [
    "adapter: RTX-3050 Dx12 discrete vendor=0x10DE",
    'arena per-pass: adapter="Intel(R) UHD Graphics" driver="32.0" frames=60 validation_errors=0',
  ].join("\n");
  assert.equal(arenaAdapterName(both), "RTX-3050_Dx12_discrete");
  assert.equal(parseAdapterTypeWord(both), "discrete");
  assert.equal(findIntegratedAdapter(both), null);
  assert.equal(parseAdapterTypeWord(LIVE_ARENA), null);
  const uhd = [CANNED_ARENA, 'arena per-pass: adapter="RTX-3050" frames=60'].join("\n");
  assert.equal(findIntegratedAdapter(uhd), "Intel-UHD_Dx12_integrated");
});

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

// A gate number from a loaded box is a filed number, never a claim of speed
// (quiet.mjs rule). The run stamps its own load so the report says which it is.
test("potato: the load stamp counts cargo and rustc from the same tasklist text", () => {
  // The canned sample already carries one cargo.exe line and one arena.exe.
  assert.deepEqual(parseBuildLoad(CANNED_TASKLIST), { cargo: 1, rustc: 0, loaded: true });
  const arenaOnly = '"arena.exe","1234","Console","1","1,310,720 K"';
  assert.deepEqual(parseBuildLoad(arenaOnly), { cargo: 0, rustc: 0, loaded: false });
  const loaded = [
    CANNED_TASKLIST,
    '"cargo.exe","4242","Console","1","120,000 K"',
    '"cargo.exe","4243","Console","1","90,000 K"',
    '"rustc.exe","4244","Console","1","4,000,000 K"',
  ].join("\n");
  assert.deepEqual(parseBuildLoad(loaded), { cargo: 3, rustc: 1, loaded: true });
  assert.deepEqual(parseBuildLoad(""), { cargo: 0, rustc: 0, loaded: false });
  assert.deepEqual(parseBuildLoad(null), { cargo: 0, rustc: 0, loaded: false });
});

test("potato: the load line reads QUIET or LOADED with the counts", () => {
  assert.equal(formatLoadText({ cargo: 0, rustc: 0, loaded: false }), "QUIET");
  assert.equal(
    formatLoadText({ cargo: 2, rustc: 1, loaded: true }),
    "LOADED 2 cargo + 1 rustc running (filed numbers, not a speed claim)",
  );
  assert.equal(formatLoadLine({ cargo: 0, rustc: 0, loaded: false }), "potato load: QUIET");
  assert.equal(
    formatLoadLine({ cargo: 1, rustc: 0, loaded: true }),
    "potato load: LOADED 1 cargo + 0 rustc running (filed numbers, not a speed claim)",
  );
});

test("potato: the report carries the load stamp, and UNKNOWN when it never sampled", () => {
  const base = {
    at: "2026-10-17T00:00:00.000Z",
    cmd: "arena.exe --scene arena",
    env: "E2040_ADAPTER=integrated E2040_LIMITS=downlevel",
    adapter: "Intel(R)_UHD_Graphics",
    line: "potato: adapter=Intel(R)_UHD_Graphics p50_ms=7.235 p99_ms=20.239 peak_ws_mb=1508",
    verdicts: checkBars({ p50_ms: 7.235, p99_ms: 20.239, peak_ws_mb: 1508 }, loadBar()),
    hasIntegrated: true,
    missing: false,
    timedOut: false,
  };
  const quiet = formatReport({ ...base, load: { cargo: 0, rustc: 0, loaded: false } });
  assert.ok(quiet.includes("load: QUIET"), quiet);
  const loaded = formatReport({ ...base, load: { cargo: 3, rustc: 1, loaded: true } });
  assert.ok(
    loaded.includes("load: LOADED 3 cargo + 1 rustc running (filed numbers, not a speed claim)"),
    loaded,
  );
  const unknown = formatReport(base);
  assert.ok(unknown.includes("load: UNKNOWN"), unknown);
});

// The row's Accept is about the tool, not only its pure helpers: "no
// integrated adapter prints NO_IGPU and exits 2; --floor exits 1 when p50
// is over". These tests drive the real tool entry point with injected live
// inputs (no arena run, no tasklist, no wall clock) and a temp report dir.
const DISCRETE_ARENA = [
  "adapter: RTX-3050 Dx12 discrete vendor=0x10DE",
  "arena frame-split: sim=1ms present=2ms fighters=0ms per frame (mean over 300 frames) cpu_ms p50=5.000 p99=9.000 interval_ms p50=16.000 p99=20.000 gpu_ms=unsupported acquire=0ms present_mode=vsync",
].join("\n");

const OVER_ARENA = [
  "adapter: Intel-UHD Dx12 integrated vendor=0x8086",
  "arena frame-split: sim=1ms present=2ms fighters=0ms per frame (mean over 300 frames) cpu_ms p50=40.000 p99=50.000 interval_ms p50=60.000 p99=90.000 gpu_ms=unsupported acquire=0ms present_mode=vsync",
].join("\n");

function fakeLive(output, tasklistText = CANNED_TASKLIST) {
  return {
    output,
    tasklistText,
    missing: false,
    timedOut: false,
    load: { cargo: 0, rustc: 0, loaded: false },
  };
}

async function drive(argv, live, reportDir) {
  const lines = [];
  const errors = [];
  const code = await potato.run(argv, {
    runLive: async () => live,
    log: (line) => lines.push(line),
    errorLog: (line) => errors.push(line),
    reportDir,
    now: () => new Date("2026-10-17T00:00:00.000Z"),
  });
  return { code, lines, errors };
}

function tempReportDir() {
  return mkdtempSync(joinPath(tmpdir(), "potato-check-"));
}

test("potato: the tool prints NO_IGPU and returns 2 when no integrated adapter shows", async () => {
  const dir = tempReportDir();
  try {
    const { code, lines } = await drive([], fakeLive(DISCRETE_ARENA), dir);
    assert.equal(code, 2, "no integrated adapter exits 2");
    assert.ok(lines.includes("NO_IGPU"), lines.join("\n"));
    assert.ok(lines[0].startsWith("potato: adapter=RTX-3050_Dx12_discrete"), lines[0]);
    assert.equal(lines[0], "potato: adapter=RTX-3050_Dx12_discrete p50_ms=16.000 p99_ms=20.000 peak_ws_mb=1280");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("potato: the tool returns 1 with --floor when p50 is over, else 0", async () => {
  const dir = tempReportDir();
  try {
    const hard = await drive(["--floor"], fakeLive(OVER_ARENA), dir);
    assert.equal(hard.code, 1, "--floor exits 1 on a non-PASS bar");
    const p50 = hard.lines.find((line) => line.startsWith("p50_ms "));
    assert.ok(p50?.includes("60.000"), p50);
    assert.ok(p50?.endsWith("FAIL"), p50);
    const soft = await drive([], fakeLive(OVER_ARENA), dir);
    assert.equal(soft.code, 0, "without --floor a FAIL only prints");
    assert.ok(soft.lines.some((line) => line.endsWith("FAIL")), soft.lines.join("\n"));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("potato: the tool prints the same canned potato line bytes twice and writes the report", async () => {
  const dir = tempReportDir();
  try {
    const first = await drive([], fakeLive(CANNED_ARENA), dir);
    const second = await drive([], fakeLive(CANNED_ARENA), dir);
    assert.equal(first.lines[0], "potato: adapter=Intel-UHD_Dx12_integrated p50_ms=42.125 p99_ms=67.500 peak_ws_mb=1280");
    assert.deepEqual(first.lines, second.lines, "same inputs give the same bytes twice");
    assert.equal(first.code, 0);
    const file = joinPath(dir, "potato-2026-10-17.md");
    assert.ok(first.lines.some((line) => line === `report: ${file}`), first.lines.join("\n"));
    const report = readFileSync(file, "utf8");
    assert.ok(report.includes(first.lines[0]), report);
    assert.ok(report.includes("load: QUIET"), report);
    assert.ok(first.lines.includes("potato load: QUIET"), first.lines.join("\n"));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("potato: a missing arena prints UNKNOWN and the report says MISSING", async () => {
  const dir = tempReportDir();
  try {
    const { code, lines } = await drive([], { ...fakeLive("", ""), missing: true }, dir);
    assert.equal(code, 2, "no adapter evidence reads NO_IGPU exit 2");
    assert.equal(lines[0], "potato: adapter=UNKNOWN p50_ms=UNKNOWN p99_ms=UNKNOWN peak_ws_mb=UNKNOWN");
    const report = readFileSync(joinPath(dir, "potato-2026-10-17.md"), "utf8");
    assert.ok(report.includes("arena: MISSING"), report);
    assert.ok(report.includes("potato: adapter=UNKNOWN"), report);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("potato: --help prints the usage line and exits 0", async () => {
  const { code, lines } = await drive(["--help"], fakeLive(CANNED_ARENA), tempReportDir());
  assert.equal(code, 0);
  assert.equal(lines[0], "usage: node tools/potato-check.mjs [--floor]");
});

test("potato: an unreadable bar file exits 2 with the cause", async () => {
  const dir = tempReportDir();
  try {
    const lines = [];
    const errors = [];
    const code = await potato.run([], {
      runLive: async () => fakeLive(CANNED_ARENA),
      loadBar: () => {
        throw new Error("potato bar p50_ms is not a number");
      },
      log: (line) => lines.push(line),
      errorLog: (line) => errors.push(line),
      reportDir: dir,
    });
    assert.equal(code, 2, "a bad bar file fails closed");
    assert.ok(errors.some((line) => line.includes("potato bar p50_ms is not a number")), errors.join("\n"));
    assert.deepEqual(lines, [], "nothing is measured when the bar file is bad");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// A frame number sampled while 26 builds run is a filed number, not a
// measurement (quiet.mjs rule; the tool's own header says so). So `--floor`
// refuses to judge such a run (exit 3) instead of printing a false FAIL the
// owner would read as a product verdict. Every bar line still prints its
// PASS/FAIL: nothing is hidden, only the verdict is withheld. A quiet box
// keeps the row's exit 1 / 0 byte for byte.
test("potato: the --floor verdict is withheld on a LOADED box, exit 3", async () => {
  const dir = tempReportDir();
  try {
    const loaded = { ...fakeLive(OVER_ARENA), load: { cargo: 3, rustc: 1, loaded: true } };
    const held = await drive(["--floor"], loaded, dir);
    assert.equal(held.code, 3, "--floor refuses to judge while builds run");
    assert.ok(
      held.lines.some((line) => line.startsWith("potato floor: WITHHELD")),
      held.lines.join("\n"),
    );
    const p50 = held.lines.find((line) => line.startsWith("p50_ms "));
    assert.ok(p50?.includes("60.000"), p50);
    assert.ok(p50?.endsWith("FAIL"), p50);
    const report = readFileSync(joinPath(dir, "potato-2026-10-17.md"), "utf8");
    assert.ok(report.includes("floor: WITHHELD"), report);
    const quiet = await drive(["--floor"], fakeLive(OVER_ARENA), dir);
    assert.equal(quiet.code, 1, "a quiet box still exits 1 on an over-bar p50");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("potato: decideExit withholds a floor verdict only when the box was loaded", () => {
  const bar = loadBar();
  const over = checkBars({ p50_ms: 60, p99_ms: 70, peak_ws_mb: 1000 }, bar);
  const pass = checkBars({ p50_ms: 42.125, p99_ms: 67.5, peak_ws_mb: 1280 }, bar);
  assert.equal(decideExit({ hasIntegrated: true, verdicts: over, floor: true, loaded: true }), 3);
  assert.equal(decideExit({ hasIntegrated: true, verdicts: pass, floor: true, loaded: true }), 3);
  assert.equal(decideExit({ hasIntegrated: true, verdicts: over, floor: true, loaded: false }), 1);
  assert.equal(decideExit({ hasIntegrated: true, verdicts: pass, floor: true, loaded: false }), 0);
  assert.equal(decideExit({ hasIntegrated: true, verdicts: over, floor: false, loaded: true }), 0);
  assert.equal(
    decideExit({ hasIntegrated: false, verdicts: over, floor: true, loaded: true }),
    2,
    "no integrated adapter still wins",
  );
});

test("potato: summarize carries the run load stamp into the verdict", () => {
  const bar = loadBar();
  const quiet = summarize({ output: CANNED_ARENA, tasklistText: CANNED_TASKLIST, bar, floor: true });
  assert.equal(quiet.loaded, false);
  assert.equal(quiet.exit, 0);
  const loaded = summarize({ output: OVER_ARENA, tasklistText: CANNED_TASKLIST, bar, floor: true, loaded: true });
  assert.equal(loaded.loaded, true);
  assert.equal(loaded.exit, 3);
  assert.ok(loaded.line.includes("p50_ms=60.000"), loaded.line);
  assert.equal(loaded.verdicts[0].status, "FAIL", "the bar verdict still prints");
});

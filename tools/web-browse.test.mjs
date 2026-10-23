// web-browse drills: the decisions of the headless Chrome check (which flag sets, which files are served
// and with what type, what counts as a drawn canvas, what each verdict says) run on fixtures. One extra
// test drives a real Chrome against a mock shell page when WEB_BROWSE_CHROME names a browser; it is
// skipped otherwise.
//   node --test tools/web-browse.test.mjs
import test from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateSync } from "node:zlib";
import {
  BASE_FLAGS, FLAG_SETS, NO_GPU_SCRIPT, analyzePixels, badMimeVerdict, decodePng, destroyVerdict, flagLine, goldenVerdict, growHashVerdict, isDrawn, loadVerdict, lossVerdict, noWebGpuVerdict, parseArgs, quotaVerdict,
  memoryVerdict, readPage, receiptOf, resolveRequest, serve, verdict, visibilityVerdict,
} from "./web-browse.mjs";

// ---- a PNG encoder for fixtures: every filter type, so the decoder's unfilter paths all run ----
function encodePng(width, height, rgba) {
  const bpp = 4, stride = width * bpp;
  const rows = [];
  for (let y = 0; y < height; y++) {
    const f = y % 5;
    const line = Buffer.alloc(stride + 1);
    line[0] = f;
    for (let x = 0; x < stride; x++) {
      const cur = rgba[y * stride + x];
      const a = x >= bpp ? rgba[y * stride + x - bpp] : 0, b = y > 0 ? rgba[(y - 1) * stride + x] : 0, c = x >= bpp && y > 0 ? rgba[(y - 1) * stride + x - bpp] : 0;
      let pred = 0;
      if (f === 1) pred = a; else if (f === 2) pred = b; else if (f === 3) pred = (a + b) >> 1;
      else if (f === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); pred = pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      line[x + 1] = (cur - pred) & 255;
    }
    rows.push(line);
  }
  const chunk = (type, data) => { const h = Buffer.alloc(8); h.writeUInt32BE(data.length, 0); h.write(type, 4, "latin1"); return Buffer.concat([h, data, Buffer.alloc(4)]); }; // CRC left zero: the decoder does not check it
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4); ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(Buffer.concat(rows))), chunk("IEND", Buffer.alloc(0))]);
}

const image = (w, h, pixel) => { const b = Buffer.alloc(w * h * 4); for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) b.set(pixel(x, y), (y * w + x) * 4); return b; };

test("the PNG decoder returns the pixels the encoder was given, through every filter type", () => {
  const rgba = image(13, 11, (x, y) => [(x * 19 + y * 7) & 255, (x * x + y) & 255, (x ^ y) * 9 & 255, 255]);
  const png = decodePng(encodePng(13, 11, rgba));
  assert.equal(png.width, 13);
  assert.equal(png.height, 11);
  assert.equal(png.channels, 4);
  assert.ok(png.data.equals(rgba));
});

test("the PNG decoder refuses what it cannot read", () => {
  assert.throws(() => decodePng(Buffer.from("not a png at all, not at all, not at all")), /not a PNG/);
  const png = encodePng(4, 4, image(4, 4, () => [1, 2, 3, 255]));
  png[24] = 16; // bit depth 16 in the IHDR
  assert.throws(() => decodePng(png), /unsupported PNG/);
});

test("analyzePixels tells a flat colour, the black page and a drawn frame apart", () => {
  const stats = (pixel) => analyzePixels(decodePng(encodePng(40, 30, image(40, 30, pixel))));
  const black = stats(() => [0, 0, 0, 255]);
  const flat = stats(() => [120, 90, 160, 255]);
  const drawn = stats((x, y) => (x > 30 && y < 12 ? [250, 40, 90, 255] : x < 10 && y > 24 ? [20, 20, 20, 255] : [120, 90, 160, 255]));
  assert.equal(black.distinct, 1);
  assert.equal(black.meanLuma, 0);
  assert.equal(isDrawn(black), false, "the black page is not a drawn canvas");
  assert.equal(flat.dominantShare, 1);
  assert.equal(isDrawn(flat), false, "one flat colour is not a drawn canvas");
  assert.equal(drawn.distinct, 3);
  assert.ok(drawn.meanLuma > 20);
  assert.equal(isDrawn(drawn), false, "three colours are too few: a frame has overlay text over the clear colour");
  const withText = stats((x, y) => [120 + ((x * y) % 7) * 5, 90, 160, 255]);
  assert.equal(isDrawn(withText), true);
});

test("request paths stay inside the served directory", () => {
  const dir = mkdtempSync(join(tmpdir(), "web-browse-srv-"));
  try {
    assert.equal(resolveRequest(dir, "/"), join(dir, "index.html"));
    assert.equal(resolveRequest(dir, "/engine_bg.wasm?x=1"), join(dir, "engine_bg.wasm"));
    assert.equal(resolveRequest(dir, "/../secret.txt"), null);
    assert.equal(resolveRequest(dir, "/%2e%2e/secret.txt"), null);
    assert.equal(resolveRequest(dir, "/a/../../secret.txt"), null);
    assert.equal(resolveRequest(dir, "/%00"), null);
    assert.equal(resolveRequest(dir, "/%E0%A4%A"), null);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the server binds loopback, serves .wasm as application/wasm and 404s the rest", async () => {
  const dir = mkdtempSync(join(tmpdir(), "web-browse-srv-"));
  writeFileSync(join(dir, "index.html"), "<p>hi</p>");
  writeFileSync(join(dir, "engine_bg.wasm"), Buffer.from([0, 0x61, 0x73, 0x6d]));
  writeFileSync(join(dir, "engine.js"), "export {}");
  const s = await serve(dir);
  try {
    const base = `http://127.0.0.1:${s.port}`;
    const wasm = await fetch(`${base}/engine_bg.wasm`);
    assert.equal(wasm.headers.get("content-type"), "application/wasm");
    assert.equal((await wasm.arrayBuffer()).byteLength, 4);
    assert.match((await fetch(`${base}/engine.js`)).headers.get("content-type"), /^text\/javascript/);
    assert.match((await fetch(`${base}/`)).headers.get("content-type"), /^text\/html/);
    assert.equal((await fetch(`${base}/nope.js`)).status, 404);
    assert.equal((await fetch(`${base}/..%2f..%2fetc/passwd`)).status, 404);
    assert.deepEqual(s.log.map((l) => l.status), [200, 200, 200, 404, 404]);
  } finally {
    await s.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the first flag set is the one the plan names and every set asks for WebGPU", () => {
  assert.equal(flagLine(FLAG_SETS[0]), `${BASE_FLAGS.join(" ")} --enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader`);
  assert.equal(new Set(FLAG_SETS.map((s) => s.name)).size, FLAG_SETS.length);
  assert.equal(new Set(FLAG_SETS.map((s) => s.flags.join(" "))).size, FLAG_SETS.length);
  for (const s of FLAG_SETS) assert.ok(s.flags.includes("--enable-unsafe-webgpu"), s.name);
  assert.ok(BASE_FLAGS.includes("--headless=new") && BASE_FLAGS.includes("--no-sandbox"));
  assert.ok(!BASE_FLAGS.concat(...FLAG_SETS.map((s) => s.flags)).some((f) => /remote-debugging-address|remote-allow-origins/.test(f)), "no flag opens the debugging port to the network");
});

const goodLine = { ok: true, adapter: { vendor: "google", architecture: "swiftshader", description: "SwiftShader" } };
const ran = (over = {}) => ({ timedOut: false, report: goodLine, reportText: JSON.stringify(goodLine), stats: { meanLuma: 90, distinct: 500, dominantShare: 0.7 }, framesAdvanced: true, recovered: true, ...over });

test("verdict: an adapter run is ok only when drawn, moving and recovered", () => {
  assert.deepEqual(verdict(ran()), { ok: true, code: "adapter-ok", why: "google swiftshader SwiftShader" });
  assert.equal(verdict(ran({ stats: { meanLuma: 0, distinct: 1, dominantShare: 1 } })).code, "blank-canvas");
  assert.equal(verdict(ran({ stats: null })).code, "blank-canvas");
  assert.equal(verdict(ran({ framesAdvanced: false })).code, "frozen");
  assert.equal(verdict(ran({ recovered: false })).code, "no-recovery");
});

const lossReceipt = (over = {}) => ({ ok: true, frames: 40, losses: 1, lastLoss: { reason: "destroyed", cause: "forced", message: "device destroyed" }, ...over });

test("lossVerdict: one forced loss, the browser's reason, the cause and a frame counter that moves", () => {
  assert.deepEqual(lossVerdict(lossReceipt()), { ok: true, problems: [] });
  assert.match(lossVerdict(null).problems[0], /no receipt JSON/);
  assert.match(lossVerdict(lossReceipt({ losses: 0 })).problems[0], /counts 0 losses/);
  assert.match(lossVerdict(lossReceipt({ lastLoss: undefined })).problems.join(), /no lastLoss/);
  assert.match(lossVerdict(lossReceipt({ lastLoss: { reason: "", cause: "forced" } })).problems.join(), /no GPUDeviceLostInfo reason/);
  assert.match(lossVerdict(lossReceipt({ lastLoss: { reason: "unknown", cause: "browser" } })).problems.join(), /cause is browser, expected forced/);
  assert.match(lossVerdict(lossReceipt({ frames: 0 })).problems.join(), /counts no frame/);
});

test("destroyVerdict: a requested destroy stops the page and starts no new generation", () => {
  assert.deepEqual(destroyVerdict({ generationBefore: "1", state: "stopped", generation: "1" }), { ok: true, problems: [] });
  assert.match(destroyVerdict({ generationBefore: "1", state: "running", generation: "1" }).problems[0], /expected stopped/);
  assert.match(destroyVerdict({ generationBefore: "1", state: "stopped", generation: "2" }).problems[0], /must not retry/);
  assert.match(destroyVerdict(undefined).problems[0], /did not report/);
});

test("visibilityVerdict: one visibilitychange is exactly one skipped frame", () => {
  assert.deepEqual(visibilityVerdict({ before: 0, after: 1 }), { ok: true, problems: [] });
  assert.match(visibilityVerdict({ before: 0, after: 0 }).problems[0], /from 0 to 0, expected 1/);
  assert.match(visibilityVerdict({ before: 2, after: 5 }).problems[0], /from 2 to 5, expected 3/);
  assert.match(visibilityVerdict({ before: -1, after: -1 }).problems[0], /never showed hidden_skips/);
  assert.match(visibilityVerdict(undefined).problems[0], /did not report/);
});

test("growHashVerdict: a still frozen frame, and a memory grow that leaves its hash alone", () => {
  const cap = (hash, over = {}) => ({ ok: true, frame: 40, hash, blank: false, ...over });
  assert.deepEqual(growHashVerdict({ still: [cap("aa"), cap("aa")], afterGrow: cap("aa") }), { ok: true, problems: [] });
  assert.match(growHashVerdict({ still: [cap("aa"), cap("aa")], afterGrow: cap("bb") }).problems[0], /changed the frame hash: aa before, bb after/);
  assert.match(growHashVerdict({ still: [cap("aa"), cap("cc")], afterGrow: cap("cc") }).problems[0], /not still.*prove nothing/);
  assert.match(growHashVerdict({ still: [cap("aa"), cap("aa", { blank: true })], afterGrow: cap("aa") }).problems[0], /blank/);
  assert.match(growHashVerdict({ still: [cap("aa"), null], afterGrow: cap("aa") }).problems[0], /fewer than three/);
  assert.match(growHashVerdict(undefined).problems[0], /fewer than three/);
});

test("verdict: the requested destroy is part of the run, the loss receipt only of an arena run", () => {
  // A page without the arena (the square shell) has no receipt to read.
  assert.equal(verdict(ran({ loss: null })).ok, true);
  assert.equal(verdict(ran({ destroyed: { generationBefore: "1", state: "stopped", generation: "1" } })).ok, true);
  assert.equal(verdict(ran({ destroyed: { generationBefore: "1", state: "running", generation: "2" } })).code, "requested-destroy");
});

test("verdict: a refusal carries the page's own reason, a dead page says timeout", () => {
  const refused = { ok: false, reason: "no-adapter", message: "WebGPU is here but the browser gave no graphics adapter.", detail: "No suitable adapter" };
  const v = verdict(ran({ report: refused, reportText: JSON.stringify(refused) }));
  assert.equal(v.ok, false);
  assert.equal(v.code, "no-adapter");
  assert.match(v.why, /No suitable adapter/);
  assert.equal(verdict({ timedOut: true, waitedMs: 90000 }).code, "timeout");
  assert.equal(verdict(ran({ report: null, reportText: "starting" })).code, "no-report");
  assert.equal(verdict(ran({ report: { hello: 1 } })).code, "bad-line");
});

const GOLDEN_LINE = "golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK 412 ms";
const GOLDEN_TEXT = `${GOLDEN_LINE}
tape 600 ticks in 41.3 ms, 0.0688 ms per tick`;

const fatalRun = (over = {}) => ({
  timedOut: false, state: "fatal", fatalDisplay: "block",
  fatalText: "This browser cannot run the game\nThis browser has no WebGPU (navigator.gpu is missing). navigator.gpu is undefined. (fallback=no-webgpu)\nChrome 113 or newer\nEdge 113 or newer\nSafari 26 or newer\nFirefox 141 or newer on Windows",
  report: { ok: false, reason: "no-webgpu", fallback: "no-webgpu" }, reportText: '{"ok":false,"reason":"no-webgpu","fallback":"no-webgpu"}', exceptions: [],
  golden: GOLDEN_TEXT, goldenState: "ok", ...over,
});

test("noWebGpuVerdict: the loud message with the versions, the reason and no exception", () => {
  assert.deepEqual(noWebGpuVerdict(fatalRun()), { ok: true, problems: [] });
});

test("goldenVerdict: only the OK shape with data-golden=ok passes, and it keeps the tick line", () => {
  const good = goldenVerdict(GOLDEN_TEXT, "ok");
  assert.deepEqual(good, { ok: true, line: GOLDEN_LINE, tickLine: "tape 600 ticks in 41.3 ms, 0.0688 ms per tick", why: "" });
  const mismatch = "golden MISMATCH got d57b5c8ae3096615 state 2aca9338670aa0cb corpus b37fc6d66df2d417, expected d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 (412 ms)";
  assert.match(goldenVerdict(mismatch, "mismatch").why, /ended as mismatch: golden MISMATCH got d57b5c8ae3096615/);
  assert.equal(goldenVerdict(mismatch, "ok").ok, false, "the shape is checked even when the attribute says ok");
  assert.equal(goldenVerdict("golden ERROR tape: boom", "error").ok, false);
  assert.match(goldenVerdict("golden: running the 600-tick tape", null).why, /never finished/);
  assert.equal(goldenVerdict(undefined, null).ok, false);
  assert.equal(goldenVerdict(GOLDEN_LINE.replace(" ms", ""), "ok").ok, false);
  assert.equal(goldenVerdict(GOLDEN_LINE.replace("d57b5c8ae3096614", "d57b"), "ok").ok, false);
});

test("noWebGpuVerdict: the golden line still has to print when there is no WebGPU", () => {
  assert.match(noWebGpuVerdict(fatalRun({ goldenState: null, golden: "golden: running" })).problems.join(), /never finished/);
  assert.match(noWebGpuVerdict(fatalRun({ goldenState: "mismatch", golden: "golden MISMATCH got x" })).problems.join(), /ended as mismatch/);
});

test("noWebGpuVerdict: each broken piece is named", () => {
  assert.match(noWebGpuVerdict(fatalRun({ fatalDisplay: "none" })).problems.join(), /not shown/);
  assert.match(noWebGpuVerdict(fatalRun({ fatalText: "Chrome 113 Edge 113 Safari 26" })).problems.join(), /lacks Firefox 141/);
  assert.match(noWebGpuVerdict(fatalRun({ state: "running" })).problems.join(), /expected fatal/);
  assert.match(noWebGpuVerdict(fatalRun({ report: { ok: false, reason: "no-adapter" } })).problems.join(), /reason no-webgpu/);
  // QT11: the page and the line name the fallback, not only the reason.
  assert.match(noWebGpuVerdict(fatalRun({ fatalText: "Chrome 113 Edge 113 Safari 26 Firefox 141" })).problems.join(), /does not say fallback=no-webgpu/);
  assert.match(noWebGpuVerdict(fatalRun({ report: { ok: false, reason: "no-webgpu" } })).problems.join(), /no "fallback":"no-webgpu"/);
  assert.match(noWebGpuVerdict(fatalRun({ exceptions: ["TypeError: x"] })).problems.join(), /uncaught exceptions: TypeError/);
  assert.equal(noWebGpuVerdict(fatalRun({ timedOut: true })).ok, false);
});

test("parseArgs", () => {
  assert.match(parseArgs([]).error, /--chrome/);
  const a = parseArgs(["--chrome", "google-chrome", "--dir", "d", "--out", "o", "--require-adapter", "--only", "2"]);
  assert.deepEqual([a.chrome, a.dir, a.out, a.requireAdapter, a.only, a.error], ["google-chrome", "d", "o", true, 2, null]);
  assert.equal(parseArgs(["--chrome", "c"]).all, false);
  assert.equal(parseArgs(["--chrome", "c", "--all"]).all, true);
  assert.match(parseArgs(["--chrome", "c", "--wat"]).error, /unknown argument/);
  assert.match(parseArgs(["--chrome", "c", "--only", "x"]).error, /--only/);
});

// ---- one real browser, a mock shell page ----
const CHROME = process.env.WEB_BROWSE_CHROME;
const MOCK = `<!doctype html><body data-state="starting"><canvas id="canvas" width="64" height="64"></canvas><div id="hud">frame 0 | -- ms</div><pre id="report">starting</pre><pre id="golden">golden: running</pre>
<div id="fatal" style="display:none">Chrome 113 Edge 113 Safari 26 Firefox 141</div><script>
const c = document.getElementById('canvas').getContext('2d'); let n = 0;
document.getElementById('golden').textContent = 'golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK 412 ms'; document.body.dataset.golden = 'ok';
if (typeof navigator.gpu === 'undefined') {
  document.getElementById('fatal').style.display = 'block'; document.getElementById('report').textContent = JSON.stringify({ ok: false, reason: 'no-webgpu', fallback: 'no-webgpu' }); document.getElementById('fatal').textContent += ' (fallback=no-webgpu)'; document.body.dataset.state = 'fatal';
} else {
  document.body.dataset.generation = '1';
  window.loseDevice = () => { document.body.dataset.generation = '2'; };
  document.getElementById('report').textContent = JSON.stringify({ ok: true, adapter: { vendor: 'mock' } }); document.body.dataset.state = 'running';
  setInterval(() => { document.getElementById('hud').textContent = 'frame ' + (++n) + ' | 16.0 ms'; document.body.style.background = 'rgb(' + (n % 255) + ',90,160)'; }, 50);
}
</script></body>`;

test("a real Chrome reads the mock page back, with and without navigator.gpu", { skip: !CHROME }, async () => {
  const dir = mkdtempSync(join(tmpdir(), "web-browse-mock-"));
  writeFileSync(join(dir, "index.html"), MOCK);
  mkdirSync(join(dir, "out"));
  const s = await serve(dir);
  const flags = [...BASE_FLAGS, "--enable-unsafe-webgpu"];
  try {
    const url = `http://127.0.0.1:${s.port}/index.html`;
    const withGpu = await readPage({ chrome: CHROME, flags, url, logFile: join(dir, "out", "a.log"), waitMs: 30000 });
    if (withGpu.hasGpu) {
      assert.equal(withGpu.state, "running");
      assert.equal(withGpu.report.ok, true);
      assert.equal(withGpu.framesAdvanced, true);
    }
    const without = await readPage({ chrome: CHROME, flags, url, logFile: join(dir, "out", "b.log"), beforeLoad: NO_GPU_SCRIPT, waitMs: 30000 });
    assert.equal(without.hasGpu, false);
    assert.deepEqual(noWebGpuVerdict(without), { ok: true, problems: [] });
    assert.ok(without.screenshot.length > 100);
  } finally {
    await s.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

// ---- scripted input (AIS2) ----
test("inputCommands maps events to CDP calls and waits, in order", async () => {
  const { inputCommands } = await import("./web-browse.mjs");
  const cmds = inputCommands([{ kind: "key", key: "a" }, { kind: "click", x: 10, y: 20 }, { kind: "wait", ms: 50 }, { kind: "key_down", key: "ArrowLeft" }, { kind: "key_up", key: "ArrowLeft" }]);
  assert.deepEqual(cmds.map((c) => c.wait ?? `${c.method}:${c.params.type}`), [
    "Input.dispatchKeyEvent:keyDown", "Input.dispatchKeyEvent:keyUp",
    "Input.dispatchMouseEvent:mouseMoved", "Input.dispatchMouseEvent:mousePressed", "Input.dispatchMouseEvent:mouseReleased",
    50, "Input.dispatchKeyEvent:rawKeyDown", "Input.dispatchKeyEvent:keyUp",
  ]);
  assert.equal(cmds[0].params.code, "KeyA");
  assert.equal(cmds[0].params.text, "a");
  assert.equal(cmds[1].params.text, undefined);
  assert.deepEqual([cmds[3].params.x, cmds[3].params.y, cmds[3].params.button], [10, 20, "left"]);
});

test("inputCommands refuses what it cannot map", async () => {
  const { inputCommands } = await import("./web-browse.mjs");
  assert.throws(() => inputCommands([{ kind: "tap" }]), /unknown input kind/);
  assert.throws(() => inputCommands([{ kind: "key", key: "NoSuchKey" }]), /unknown key/);
  assert.throws(() => inputCommands([{ kind: "click", x: 1 }]), /integer x and y/);
  assert.throws(() => inputCommands([{ kind: "wait", ms: 99999 }]), /0\.\.=2000/);
});

test("parseArgs takes --input and refuses it without a file", () => {
  assert.equal(parseArgs(["--chrome", "c", "--input", "e.json"]).input, "e.json");
  assert.match(parseArgs(["--chrome", "c", "--input"]).error, /--input needs/);
});

// ---- the arena frame (WEB5): the GPU copy of the canvas, its hash, its PNG and the receipt ----
test("fnv1a64 is FNV-1a 64: the published vectors and the same digest the render crate prints", async () => {
  const { fnv1a64 } = await import("./web-browse.mjs");
  assert.equal(fnv1a64(Buffer.from("a")), "af63dc4c8601ec8c");
  assert.equal(fnv1a64(Buffer.alloc(0)), "cbf29ce484222325");
  // The same four bytes are a unit test in crates/render/src/present/gpu_present/capture.rs.
  assert.equal(fnv1a64(Buffer.from([9, 8, 7, 255])), "f247a9ba2081b132");
  assert.notEqual(fnv1a64(Buffer.from([9, 8, 7, 254])), fnv1a64(Buffer.from([9, 8, 7, 255])));
});

test("encodePng writes a PNG the decoder reads back byte for byte, with real CRCs", async () => {
  const { encodePng: write } = await import("./web-browse.mjs");
  const w = 7, h = 5, rgba = Buffer.alloc(w * h * 4);
  for (let i = 0; i < rgba.length; i++) rgba[i] = (i * 37 + (i >> 3)) & 255;
  const png = write(w, h, rgba);
  const back = decodePng(png);
  assert.deepEqual([back.width, back.height, back.channels], [w, h, 4]);
  assert.ok(Buffer.from(back.data).equals(rgba));
  // Every chunk's CRC is the CRC-32 of its type and data.
  const { crc32 } = await import("node:zlib");
  let off = 8, chunks = 0;
  while (off < png.length) {
    const len = png.readUInt32BE(off);
    assert.equal(png.readUInt32BE(off + 8 + len), crc32(png.subarray(off + 4, off + 8 + len)) >>> 0, `chunk at ${off}`);
    off += 12 + len; chunks++;
  }
  assert.equal(chunks, 3, "IHDR, IDAT, IEND");
});

test("encodePng refuses bytes that are not the shape it was told", async () => {
  const { encodePng: write } = await import("./web-browse.mjs");
  assert.throws(() => write(4, 4, Buffer.alloc(10)), /not a 4x4 RGBA image/);
  assert.throws(() => write(0, 4, Buffer.alloc(0)), /not a 0x4 RGBA image/);
});

test("arenaVerdict: a drawn, intact capture with a clean receipt passes, and each fault is named", async () => {
  const { arenaVerdict } = await import("./web-browse.mjs");
  const good = {
    capture: "ready", captureLine: { hash: "00ff00ff00ff00ff", blank: false }, hashOk: true, nodeHash: "00ff00ff00ff00ff",
    stats: { width: 8, height: 8, distinct: 900, dominantShare: 0.05, meanLuma: 150 },
    receipt: { ok: true, tier: "web", frames: 120, frameErrors: 0, validationErrors: 0, latePipelines: 0 }, receiptText: "{}",
    tierText: "tier=web why=default scale=100 shadow=1024x2", splitText: "", pipelinesText: "pipelines=24 ms=412",
  };
  assert.deepEqual(arenaVerdict(good), { ok: true, problems: [] });
  assert.deepEqual(arenaVerdict(null), { ok: true, problems: [] }, "a page without the arena is not judged here");
  const bad = (patch) => arenaVerdict({ ...good, ...patch }).problems.join(" | ");
  assert.match(bad({ capture: null }), /never finished/);
  assert.match(bad({ capture: "error" }), /ended as error/);
  assert.match(bad({ capture: "blank", captureLine: { hash: "x", blank: true } }), /ended as blank.*\(blank\)/);
  assert.match(bad({ hashOk: false, nodeHash: "1111111111111111" }), /hash to 1111111111111111, the page said 00ff00ff00ff00ff/);
  assert.match(bad({ stats: { width: 8, height: 8, distinct: 1, dominantShare: 1, meanLuma: 0 } }), /copied canvas is blank/);
  assert.match(bad({ receipt: { ...good.receipt, validationErrors: 2, firstError: "bind group" } }), /validation errors 2: bind group/);
  assert.match(bad({ receipt: { ...good.receipt, frameErrors: 1 } }), /frame errors 1/);
  assert.match(bad({ receipt: { ...good.receipt, frames: 0 } }), /counts no frame/);
  assert.match(bad({ receipt: null, receiptText: "garbage" }), /receipt is not JSON: garbage/);
  // WEB13: a pipeline built after the loader fails the run loudly; WEB8: the page must name its tier.
  assert.match(bad({ receipt: { ...good.receipt, latePipelines: 2 } }), /late_compile: 2 pipeline\(s\) built after the loader/);
  assert.match(bad({ receipt: { ...good.receipt, tier: undefined } }), /receipt names no tier/);
  assert.match(bad({ tierText: "arena: starting" }), /prints no tier= line/);
  assert.match(bad({ tierText: "tier=potato why=query" }), /tier line says tier=potato, the receipt says tier=web/);
  assert.match(bad({ pipelinesText: "pipelines: waiting" }), /no pipelines=<n> ms=<t> line/);
});

test("verdict: for the arena page the GPU copy decides, a black screenshot does not fail it", async () => {
  const arena = { capture: "ready", captureLine: { hash: "aa", blank: false }, hashOk: true, nodeHash: "aa", stats: { width: 8, height: 8, distinct: 500, dominantShare: 0.1, meanLuma: 120 }, receipt: { ok: true, tier: "web", frames: 40, frameErrors: 0, validationErrors: 0, latePipelines: 0 }, receiptText: "{}", tierText: "tier=web why=default", pipelinesText: "pipelines=24 ms=400" };
  const run = { timedOut: false, report: { ok: true, adapter: {} }, stats: { width: 8, height: 8, distinct: 1, dominantShare: 1, meanLuma: 0 }, framesAdvanced: true, recovered: true, loss: lossReceipt() };
  assert.equal(verdict({ ...run, arena }).ok, true, "black screenshot, drawn GPU copy");
  // QT7: the visibility drill is part of an arena run.
  assert.equal(verdict({ ...run, arena, visibility: { before: 0, after: 0 } }).code, "visibility");
  assert.equal(verdict({ ...run, arena, visibility: { before: 0, after: 1 } }).ok, true);
  // QW8: the arena run that recovered must also show the loss in the rebuilt page's receipt.
  assert.equal(verdict({ ...run, arena, loss: lossReceipt({ losses: 0 }) }).code, "loss-receipt");
  assert.equal(verdict({ ...run, arena, loss: null }).code, "loss-receipt");
  assert.equal(verdict(run).code, "blank-canvas", "without the arena the screenshot decides");
  const failed = verdict({ ...run, arena: { ...arena, receipt: { ...arena.receipt, validationErrors: 3 } } });
  assert.equal(failed.ok, false);
  assert.equal(failed.code, "arena");
  assert.match(failed.why, /validation errors 3/);
});

test("readArena brings the pixels over the wire intact and makes the PNG (mock page, no browser)", async () => {
  const { readArena, fnv1a64, encodePng: write } = await import("./web-browse.mjs");
  const w = 6, h = 4, rgba = Buffer.alloc(w * h * 4);
  for (let i = 0; i < rgba.length; i++) rgba[i] = (i * 53 + 11) & 255;
  const line = JSON.stringify({ ok: true, frame: 31, width: w, height: h, hash: fnv1a64(rgba), blank: false });
  const receipt = JSON.stringify({ ok: true, frames: 31, frameErrors: 0, validationErrors: 0 });
  const nodes = { receipt, passes: "sky=0.1ms", init: "arena ready in 800 ms", capture: line };
  const evalJs = async (expr) => {
    if (expr.includes("dataset.mode")) return true;
    if (expr.includes("dataset.capture")) return "ready";
    if (expr.includes("FileReader")) return rgba.toString("base64");
    const id = /getElementById\('(\w+)'\)/.exec(expr)?.[1];
    return nodes[id] ?? "";
  };
  const a = await readArena({ evalJs, sleep: async () => {} }, 1000);
  assert.equal(a.capture, "ready");
  assert.equal(a.hashOk, true);
  assert.equal(a.nodeHash, fnv1a64(rgba));
  assert.deepEqual([a.width, a.height, a.bytes], [w, h, rgba.length]);
  assert.ok(Buffer.from(decodePng(a.png).data).equals(rgba), "the PNG holds the very bytes");
  assert.equal(a.receipt.frames, 31);
  // Bytes of the wrong count are refused rather than hashed.
  const short = await readArena({ evalJs: async (e) => (e.includes("FileReader") ? rgba.subarray(0, 12).toString("base64") : evalJs(e)), sleep: async () => {} }, 1000);
  assert.equal(short.hashOk, false);
  assert.equal(short.png, undefined);
  // A page that is not the arena is skipped.
  assert.equal(await readArena({ evalJs: async () => false, sleep: async () => {} }), null);
  void write;
});

test("readArena waits for the A/B page's on arm before it reads the receipt (mock page)", async () => {
  const { readArena, fnv1a64 } = await import("./web-browse.mjs");
  const w = 2, h = 2, rgba = Buffer.alloc(w * h * 4, 7);
  const line = JSON.stringify({ ok: true, frame: 67, width: w, height: h, hash: fnv1a64(rgba), blank: false });
  const warm = JSON.stringify({ ok: true, frames: 31 });
  const done = JSON.stringify({ ok: true, frames: 151, perf: { mode: "ab", off: { frames: 60 }, on: { frames: 46 } } });
  let reads = 0;
  const nodes = { passes: "", init: "", capture: line };
  const evalJs = async (expr) => {
    if (expr.includes("dataset.mode")) return true;
    if (expr.includes("dataset.capture")) return "ready";
    if (expr.includes("FileReader")) return rgba.toString("base64");
    if (expr.includes("getElementById('receipt')")) return reads++ === 0 ? warm : done;
    const id = /getElementById\('(\w+)'\)/.exec(expr)?.[1];
    return nodes[id] ?? "";
  };
  const a = await readArena({ evalJs, sleep: async () => {} }, 1000, { waitForPerf: true });
  assert.equal(a.receipt.perf.mode, "ab");
  assert.ok(reads >= 2, "it polled past the warm-up receipt");
  // Without the flag it reports the first receipt it read (the plain page, which prints no A/B).
  reads = 0;
  const quick = await readArena({ evalJs, sleep: async () => {} }, 1000);
  assert.equal(quick.receipt.perf, undefined);
});

test("QW10: readArena gives up on a page that never prints its perf, on a clock the test drives", async () => {
  const { readArena } = await import("./web-browse.mjs");
  const captured = JSON.stringify({ ok: true, frame: 5, width: 2, height: 2, hash: "0000000000000000", blank: false });
  const stuck = JSON.stringify({ ok: true, frames: 5 });
  let reads = 0;
  let clock = 1_000;
  const now = () => {
    clock += 500;
    return clock;
  };
  const evalJs = async (expr) => {
    if (expr.includes("dataset.mode")) return true;
    if (expr.includes("dataset.capture")) return "ready";
    if (expr.includes("FileReader")) return Buffer.alloc(16, 7).toString("base64");
    if (expr.includes("getElementById('receipt')")) {
      reads += 1;
      return stuck;
    }
    if (expr.includes("getElementById('capture')")) return captured;
    return "";
  };
  const a = await readArena({ evalJs, sleep: async () => {}, now }, 1000, { waitForPerf: true });
  assert.equal(a.receipt.perf, undefined, "no perf object ever arrives");
  assert.ok(reads > 500 && reads < 700, `it stops on the five-minute cap, not on a frame counter (${reads} polls)`);
});

test("parseArgs takes --page-query and refuses an empty one", () => {
  assert.equal(parseArgs(["--chrome", "c"]).pageQuery, null);
  assert.equal(parseArgs(["--chrome", "c", "--page-query", "bundle=ab&freeze"]).pageQuery, "bundle=ab&freeze");
  assert.match(parseArgs(["--chrome", "c", "--page-query", "?"]).error, /--page-query/);
  assert.match(parseArgs(["--chrome", "c", "--page-query"]).error, /--page-query/);
});

test("writeArena leaves the PNG and the facts side by side, and nothing for a page without the arena", async () => {
  const { writeArena, encodePng: write } = await import("./web-browse.mjs");
  const dir = mkdtempSync(join(tmpdir(), "arena-out-"));
  try {
    writeArena(dir, "none", null);
    const png = write(2, 2, Buffer.alloc(16, 200));
    writeArena(dir, "arena-capture", { capture: "ready", png, receipt: { frames: 3 } });
    const { readFileSync, readdirSync } = await import("node:fs");
    assert.deepEqual(readdirSync(dir).sort(), ["arena-capture.json", "arena-capture.png"]);
    assert.ok(readFileSync(join(dir, "arena-capture.png")).equals(png));
    const facts = JSON.parse(readFileSync(join(dir, "arena-capture.json"), "utf8"));
    assert.equal(facts.capture, "ready");
    assert.equal(facts.png, undefined, "the bytes are in the PNG, not repeated in the JSON");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ---- the net duel (WEB9, WEB12) ----
test("parseArgs takes the net duel keys and refuses the half-given ones", () => {
  const ok = parseArgs(["--chrome", "c", "--server", "ws://127.0.0.1:7050", "--expect-hash", "00ab00ab00ab00ab", "--expect-transport", "websocket", "--net-timeout", "30"]);
  assert.equal(ok.error, null);
  assert.deepEqual([ok.server, ok.expectHash, ok.expectTransport, ok.netTimeout], ["ws://127.0.0.1:7050", "00ab00ab00ab00ab", "websocket", 30]);
  assert.match(parseArgs(["--chrome", "c", "--server", "http://h:1"]).error, /--server/);
  assert.match(parseArgs(["--chrome", "c", "--server"]).error, /--server/);
  assert.match(parseArgs(["--chrome", "c", "--wt", "https://h:2/e2040"]).error, /go with --server/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--wt", "ws://h:2"]).error, /--wt/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--cert", "abcd"]).error, /--cert/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--expect-hash", "xyz"]).error, /--expect-hash/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--expect-transport", "quic"]).error, /--expect-transport/);
  assert.equal(parseArgs(["--chrome", "c"]).server, null, "without --server the full check runs");
});

test("netUrl puts the keys in the query, encoded", async () => {
  const { netUrl } = await import("./web-browse.mjs");
  const cert = "ab".repeat(32);
  assert.equal(netUrl(8099, { server: "ws://127.0.0.1:7050" }), "http://127.0.0.1:8099/index.html?server=ws%3A%2F%2F127.0.0.1%3A7050");
  const u = new URL(netUrl(8099, { server: "ws://h:1", wt: "https://h:2/e2040", cert }));
  assert.deepEqual([u.searchParams.get("server"), u.searchParams.get("wt"), u.searchParams.get("cert")], ["ws://h:1", "https://h:2/e2040", cert]);
});

test("netVerdict accepts only a finished duel on the expected hash, ticks and leg", async () => {
  const { netVerdict } = await import("./web-browse.mjs");
  const line = "net: transport=websocket ticks=600 hash=0123456789abcdef peers=1";
  const good = netVerdict(line, "ok", { hash: "0123456789ABCDEF", transport: "websocket" });
  assert.equal(good.ok, true);
  assert.deepEqual(good.parsed, { transport: "websocket", ticks: 600, hash: "0123456789abcdef", peers: 1 });
  assert.equal(netVerdict(line, "ok").ok, true, "no expectation, only the shape");
  const wrongHash = netVerdict(line, "ok", { hash: "fedcba9876543210" });
  assert.equal(wrongHash.ok, false);
  assert.match(wrongHash.problems[0], /the server's is fedcba9876543210/);
  assert.match(netVerdict(line, "ok", { transport: "webtransport" }).problems[0], /expected webtransport/);
  assert.match(netVerdict(line.replace("ticks=600", "ticks=599"), "ok").problems[0], /ticks 599, expected 600/);
  assert.match(netVerdict(line.replace("peers=1", "peers=0"), "ok").problems[0], /peers 0/);
  const failed = netVerdict("net: failed transport=none reason=no leg connected: websocket leg failed: closed", "failed");
  assert.equal(failed.ok, false);
  assert.match(failed.problems[0], /ended as failed.*websocket leg failed/);
  assert.match(netVerdict("", null).problems[0], /never finished/);
  assert.match(netVerdict("net: idle", "ok").problems[0], /not the net shape/);
});

test("readNetPage reads the line a mock page prints (real Chrome, WEB_BROWSE_CHROME)", { skip: !process.env.WEB_BROWSE_CHROME }, async () => {
  const { readNetPage, netVerdict } = await import("./web-browse.mjs");
  const dir = mkdtempSync(join(tmpdir(), "net-page-"));
  const html = `<!doctype html><body><pre id="net">net: connecting</pre><script>
    const q = new URLSearchParams(location.search);
    setTimeout(() => { document.getElementById('net').textContent = 'net: transport=websocket ticks=600 hash=0123456789abcdef peers=1'; document.body.dataset.net = 'ok'; }, 300);
    document.title = q.get('server');
  </script></body>`;
  writeFileSync(join(dir, "index.html"), html);
  const server = await serve(dir);
  try {
    const url = `http://127.0.0.1:${server.port}/index.html?server=ws%3A%2F%2Fh%3A1`;
    const r = await readNetPage({ chrome: process.env.WEB_BROWSE_CHROME, url, logFile: join(dir, "chrome.log"), waitMs: 20000 });
    assert.equal(r.state, "ok");
    assert.equal(netVerdict(r.text, r.state, { hash: "0123456789abcdef" }).ok, true);
  } finally {
    await server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("parseArgs takes the one-port join and the live seat keys, and refuses the odd ones", () => {
  const ok = parseArgs(["--chrome", "c", "--join", "127.0.0.1:7060", "--cert", "ab".repeat(32), "--play", "--ticks", "3600", "--token", "424242", "--drop-at-tick", "150"]);
  assert.equal(ok.error, null);
  assert.deepEqual([ok.join, ok.server, ok.play, ok.liveKeys, ok.ticks, ok.token, ok.dropAtTick], ["127.0.0.1:7060", null, true, false, 3600, 424242, 150]);
  const keys = parseArgs(["--chrome", "c", "--server", "ws://h:1", "--live-keys"]);
  assert.deepEqual([keys.play, keys.liveKeys, keys.error], [true, true, null], "--live-keys is a live seat");
  assert.equal(parseArgs(["--chrome", "c", "--join", "[::1]:7050"]).error, null);
  assert.match(parseArgs(["--chrome", "c", "--join", "nohost"]).error, /--join needs host:port/);
  assert.match(parseArgs(["--chrome", "c", "--join", "h:1", "--server", "ws://h:1"]).error, /not both/);
  assert.match(parseArgs(["--chrome", "c", "--join", "h:1", "--wt", "https://h:1/e2040"]).error, /one port/);
  assert.match(parseArgs(["--chrome", "c", "--play"]).error, /go with --server or --join/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--ticks", "0"]).error, /--ticks/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--token", "0"]).error, /--token/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--drop-at-tick", "x"]).error, /--drop-at-tick/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--drop-at-tick", "9", "--expect-hash", "00ab00ab00ab00ab"]).error, /ends the page early/);
});

test("netUrl carries the one-port join, the match length, the token and the live flag", async () => {
  const { netUrl } = await import("./web-browse.mjs");
  const cert = "ab".repeat(32);
  const u = new URL(netUrl(8099, { join: "127.0.0.1:7060", cert, ticks: 3600, token: 424242, play: true }));
  assert.deepEqual([u.searchParams.get("join"), u.searchParams.get("server"), u.searchParams.get("cert"), u.searchParams.get("ticks"), u.searchParams.get("token")], ["127.0.0.1:7060", null, cert, "3600", "424242"]);
  assert.equal(u.searchParams.has("play"), true);
  assert.equal(new URL(netUrl(8099, { server: "ws://h:1" })).searchParams.has("play"), false, "the proof script is the default");
});

test("liveKeyPlan holds W throughout, presses a fight's worth of keys and releases W at the end", async () => {
  const { liveKeyPlan } = await import("./web-browse.mjs");
  const plan = liveKeyPlan(5000);
  assert.deepEqual(plan.map((s) => s.atMs), [...plan.map((s) => s.atMs)].sort((x, y) => x - y), "in time order");
  assert.equal(plan.filter((s) => s.key === "w" && s.kind === "key_down").length, 1, "W goes down once");
  assert.deepEqual(plan.at(-1), { atMs: 5000, kind: "key_up", key: "w" });
  for (const key of ["f", "e", "q", "Space"]) assert.ok(plan.some((s) => s.key === key && s.kind === "key_down"), key);
  const downs = new Map();
  for (const s of plan) downs.set(s.key, (downs.get(s.key) ?? 0) + (s.kind === "key_down" ? 1 : -1));
  assert.ok([...downs.values()].every((n) => n === 0), "every key that goes down comes up");
  assert.ok(plan.every((s) => s.atMs <= 5000));
});

test("readNetPage presses the plan and drops the page at the tick (real Chrome, WEB_BROWSE_CHROME)", { skip: !process.env.WEB_BROWSE_CHROME }, async () => {
  const { readNetPage, liveKeyPlan } = await import("./web-browse.mjs");
  const dir = mkdtempSync(join(tmpdir(), "net-keys-"));
  const html = `<!doctype html><body><pre id="net">net: seated</pre><script>
    document.body.dataset.net = 'running';
    const seen = new Set();
    addEventListener('keydown', (e) => { seen.add(e.code); document.title = [...seen].sort().join(','); });
    let tick = 0;
    setInterval(() => { tick += 5; document.body.dataset.netTick = String(tick); if (['KeyW', 'KeyF', 'KeyE', 'KeyQ', 'Space'].every((c) => seen.has(c))) { document.getElementById('net').textContent = 'keys ' + document.title; document.body.dataset.net = 'ok'; } }, 50);
  </script></body>`;
  writeFileSync(join(dir, "index.html"), html);
  const server = await serve(dir);
  const run = (extra) => readNetPage({ chrome: process.env.WEB_BROWSE_CHROME, url: `http://127.0.0.1:${server.port}/index.html`, logFile: join(dir, "chrome.log"), waitMs: 20000, ...extra });
  try {
    const pressed = await run({ keys: liveKeyPlan(4000) });
    assert.equal(pressed.state, "ok", pressed.text);
    assert.match(pressed.text, /KeyE.*KeyF.*KeyQ.*KeyW|Space/, "the page heard the real key events");
    const left = await run({ dropAtTick: 60 });
    assert.ok(left.dropped && left.dropped.tick >= 60, JSON.stringify(left.dropped));
    assert.notEqual(left.state, "ok");
  } finally {
    await server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

test("netVerdict can demand a loud failure that names the blocked leg", async () => {
  const { netVerdict } = await import("./web-browse.mjs");
  const text = "net: failed transport=none reason=no leg connected: webtransport leg failed: no answer within 5 s; websocket leg failed: the socket could not connect";
  const want = { failure: ["webtransport leg failed", "websocket leg failed"] };
  assert.equal(netVerdict(text, "failed", want).ok, true);
  assert.match(netVerdict(text, "failed", { failure: ["quic leg failed"] }).problems[0], /lacks "quic leg failed"/);
  assert.match(netVerdict("net: transport=websocket ticks=600 hash=0123456789abcdef peers=1", "ok", want).problems[0], /expected the duel to fail/);
  assert.match(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--expect-failure", "x", "--expect-hash", "00ab00ab00ab00ab"]).error, /does not go with/);
  assert.deepEqual(parseArgs(["--chrome", "c", "--server", "ws://h:1", "--expect-failure", "a", "--expect-failure", "b"]).expectFailure, ["a", "b"]);
});

// ---- how the module arrived (QW3, QW12) ----
test("the server can type .wasm wrongly on purpose and only .wasm", async () => {
  const dir = mkdtempSync(join(tmpdir(), "web-browse-mime-"));
  writeFileSync(join(dir, "engine_bg.wasm"), Buffer.from([0, 0x61, 0x73, 0x6d]));
  writeFileSync(join(dir, "engine.js"), "export {}");
  const s = await serve(dir, { wasmType: "text/plain" });
  try {
    const base = `http://127.0.0.1:${s.port}`;
    assert.equal((await fetch(`${base}/engine_bg.wasm`)).headers.get("content-type"), "text/plain");
    assert.match((await fetch(`${base}/engine.js`)).headers.get("content-type"), /^text\/javascript/);
    assert.deepEqual(s.log.map((l) => l.type), ["text/plain", "text/javascript; charset=utf-8"]);
  } finally {
    await s.close();
    rmSync(dir, { recursive: true, force: true });
  }
});

const LOAD = { bundle: "network", bytesFetched: 3399353, downloadMs: 812, compileMs: 95, initMs: 40.4, firstFrameMs: 4650.2, cache: "stored", persisted: true };

test("loadVerdict: network first, three timings, a stored copy, then bundle=cache with 0 bytes fetched", () => {
  assert.deepEqual(loadVerdict(LOAD, { bundle: "cache", bytesFetched: 0 }), { ok: true, problems: [] });
  assert.match(loadVerdict(null, null).problems[0], /no load stats/);
  assert.match(loadVerdict({ ...LOAD, bundle: "cache" }, { bundle: "cache", bytesFetched: 0 }).problems[0], /first load of a fresh profile says bundle=cache/);
  assert.match(loadVerdict({ ...LOAD, cache: "quota" }, { bundle: "cache", bytesFetched: 0 }).problems[0], /not stored: cache=quota/);
  assert.match(loadVerdict({ ...LOAD, compileMs: undefined }, { bundle: "cache", bytesFetched: 0 }).problems[0], /compileMs is undefined/);
  assert.match(loadVerdict(LOAD, null).problems[0], /second load .* did not report/);
  assert.match(loadVerdict(LOAD, { bundle: "network", bytesFetched: 3399353 }).problems.join(";"), /bundle=network, not cache.*fetched 3399353 bytes, not 0/);
});

test("quotaVerdict and badMimeVerdict name what is wrong", () => {
  assert.equal(quotaVerdict({ cache: "quota" }).ok, true);
  assert.match(quotaVerdict({ cache: "stored" }).problems[0], /cache=stored, not quota/);
  assert.match(quotaVerdict(null).problems[0], /cache=nothing/);
  const fatal = { state: "fatal", fatalDisplay: "block", fatalText: "This browser cannot run the game\nThe engine did not start: Error: engine_bg.wasm is served as \"text/plain\", not application/wasm" };
  assert.deepEqual(badMimeVerdict(fatal), { ok: true, problems: [] });
  assert.match(badMimeVerdict({ ...fatal, state: "starting", fatalDisplay: "none" }).problems.join(";"), /expected fatal.*display:none/);
  assert.match(badMimeVerdict({ ...fatal, fatalText: "This browser cannot run the game" }).problems[0], /lacks "The engine did not start"/);
});

test("receiptOf carries the load timings and the reload facts, and stays null for a page without them", () => {
  const r = receiptOf({ wasmMemoryBytes: 1, commit: "c" }, 4700, "set", null, LOAD, { bundle: "cache", bytesFetched: 0, downloadMs: 3 });
  assert.deepEqual(r.load, { bundle: "network", bytesFetched: 3399353, downloadMs: 812, compileMs: 95, initMs: 40, firstFrameMs: 4650, cache: "stored", persisted: true });
  assert.deepEqual(r.reload, { bundle: "cache", bytesFetched: 0 });
  assert.equal(r.ttffMs, 4700);
  assert.deepEqual([receiptOf({}, 1, "s").load, receiptOf({}, 1, "s").reload], [null, null]);
});

test("parseArgs takes --bad-mime", () => {
  assert.equal(parseArgs(["--chrome", "c"]).badMime, false);
  assert.equal(parseArgs(["--chrome", "c", "--bad-mime"]).badMime, true);
});

test("memoryVerdict: a forced grow of 16 pages is exactly 16 pages and the frames keep coming", () => {
  assert.deepEqual(memoryVerdict({ before: 247, after: 263, moved: true }), { ok: true, problems: [] });
  assert.match(memoryVerdict({ before: 247, after: 250, moved: true }).problems[0], /from 247 to 250 pages/);
  assert.match(memoryVerdict({ before: 247, after: 263, moved: false }).problems[0], /frame counter stopped/);
  assert.match(memoryVerdict(null).problems[0], /no memory readings/);
  assert.match(memoryVerdict({ before: null }).problems[0], /no memory readings/);
});

test("parseArgs takes --soak seconds", () => {
  assert.equal(parseArgs(["--chrome", "c"]).soak, 0);
  assert.equal(parseArgs(["--chrome", "c", "--soak", "600"]).soak, 600);
  assert.match(parseArgs(["--chrome", "c", "--soak", "x"]).error, /--soak/);
});

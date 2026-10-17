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
  BASE_FLAGS, FLAG_SETS, NO_GPU_SCRIPT, analyzePixels, decodePng, flagLine, goldenVerdict, isDrawn, noWebGpuVerdict, parseArgs, readPage,
  resolveRequest, serve, verdict,
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
  fatalText: "This browser cannot run the game\nChrome 113 or newer\nEdge 113 or newer\nSafari 26 or newer\nFirefox 141 or newer on Windows",
  report: { ok: false, reason: "no-webgpu" }, reportText: '{"ok":false,"reason":"no-webgpu"}', exceptions: [],
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
  document.getElementById('fatal').style.display = 'block'; document.getElementById('report').textContent = JSON.stringify({ ok: false, reason: 'no-webgpu' }); document.body.dataset.state = 'fatal';
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
    receipt: { ok: true, frames: 120, frameErrors: 0, validationErrors: 0 }, receiptText: "{}",
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
});

test("verdict: for the arena page the GPU copy decides, a black screenshot does not fail it", async () => {
  const arena = { capture: "ready", captureLine: { hash: "aa", blank: false }, hashOk: true, nodeHash: "aa", stats: { width: 8, height: 8, distinct: 500, dominantShare: 0.1, meanLuma: 120 }, receipt: { ok: true, frames: 40, frameErrors: 0, validationErrors: 0 }, receiptText: "{}" };
  const run = { timedOut: false, report: { ok: true, adapter: {} }, stats: { width: 8, height: 8, distinct: 1, dominantShare: 1, meanLuma: 0 }, framesAdvanced: true, recovered: true };
  assert.equal(verdict({ ...run, arena }).ok, true, "black screenshot, drawn GPU copy");
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

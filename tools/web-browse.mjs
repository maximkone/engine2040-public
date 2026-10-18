// Headless Chrome check of dist/web (WEB4): serve it on 127.0.0.1, load it, read back what the page says.
//   node tools/web-browse.mjs --dir dist/web --chrome google-chrome --out out/web [--require-adapter] [--only <n>]
// Two proofs, in one browser each:
//  1. The WebGPU run. Flag sets that ask Chrome for a software (SwiftShader) adapter are tried in order until
//     one gives an adapter. For that set the check reads the JSON line from #report (ok, adapter, limits),
//     takes a screenshot and tests that the canvas is not blank, that frames keep coming, and that the
//     page survives a destroyed device (window.loseDevice -> data-generation 2). If no set gives an adapter
//     the exact reason of every attempt is written down (attempts.json) and the run says NO ADAPTER.
//  2. The no-WebGPU run. navigator.gpu is removed before the page starts; the loud message with the browser
//     versions must show, the line must say ok:false reason no-webgpu, and nothing may throw.
//  3. The net duel (WEB9, WEB12), only with --server or --join: `node tools/web-browse.mjs --dir dist/web --chrome google-chrome
//     --out out/web-net --server ws://127.0.0.1:7050 [--expect-hash <hex>] [--expect-transport websocket]
//     [--wt https://host:port/e2040 --cert <sha256 hex>] [--net-timeout <s>]`. Loads the page at `?server=...` in a Chrome
//     with no GPU flags (the duel needs none), waits for `net: transport=.. ticks=.. hash=.. peers=..` in #net and checks it:
//     600 ticks (or --ticks N), the hash you expect, the transport you expect. Nothing else runs in that mode. Exit 0 proven, 1 not.
//     `--join host:port [--cert ..]` opens the page by the one-port shorthand instead. `--play` makes the seat a live one and
//     `--live-keys` presses real keys at it (a fight's worth: walk, swing, turn, dodge) so the tape is not the script;
//     `--token N` pins the seat token and `--drop-at-tick N` takes the page away once the server tick reaches N (exit 0
//     when it dropped, 1 when the duel ended first), so a second run with the same token can take the seat back.
// Exit: 0 all proven (a missing adapter is only a failure with --require-adapter), 1 a proof failed,
// 2 could not run (no Chrome, bad arguments), 3 no adapter and --require-adapter was given.
// Only loopback is served and visited. The Chrome processes started here are ours and are closed here.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { extname, join, normalize, resolve, sep } from "node:path";
import { crc32, deflateSync, inflateSync } from "node:zlib";
import { loadLine } from "../crates/web/loader.js";

// ---- flag sets ----
// Every set starts from BASE. The first set is the one the plan names; the others vary the pieces that decide
// whether Chrome finds a Vulkan driver on a machine with no GPU: which ANGLE backend, whether the Vulkan
// feature is on, and whether the SwiftShader adapter is forced or merely allowed.
export const BASE_FLAGS = ["--headless=new", "--no-sandbox", "--disable-dev-shm-usage", "--no-first-run", "--no-default-browser-check", "--hide-scrollbars", "--mute-audio", "--window-size=1280,720"];
export const FLAG_SETS = [
  { name: "swiftshader-angle", flags: ["--enable-unsafe-webgpu", "--enable-features=Vulkan", "--use-angle=swiftshader", "--use-webgpu-adapter=swiftshader"] },
  { name: "swiftshader-vulkan", flags: ["--enable-unsafe-webgpu", "--enable-features=Vulkan", "--use-vulkan=swiftshader", "--use-angle=vulkan", "--disable-vulkan-surface", "--use-webgpu-adapter=swiftshader"] },
  { name: "adapter-only", flags: ["--enable-unsafe-webgpu", "--use-webgpu-adapter=swiftshader"] },
  { name: "swiftshader-unsafe", flags: ["--enable-unsafe-webgpu", "--enable-unsafe-swiftshader", "--enable-features=Vulkan", "--use-angle=swiftshader", "--use-webgpu-adapter=swiftshader", "--ignore-gpu-blocklist", "--disable-gpu-sandbox"] },
  { name: "lavapipe", flags: ["--enable-unsafe-webgpu", "--enable-features=Vulkan", "--use-vulkan=native", "--use-angle=vulkan", "--disable-vulkan-surface", "--ignore-gpu-blocklist", "--disable-gpu-sandbox"] },
  { name: "vulkan-feature-only", flags: ["--enable-unsafe-webgpu", "--enable-features=Vulkan", "--ignore-gpu-blocklist"] },
  { name: "bare", flags: ["--enable-unsafe-webgpu"] },
];
export const flagLine = (set) => [...BASE_FLAGS, ...set.flags].join(" ");

// ---- static server on loopback ----
export const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".wasm": "application/wasm", ".json": "application/json", ".png": "image/png", ".txt": "text/plain; charset=utf-8", ".css": "text/css; charset=utf-8" };

// The file a request path maps to inside `dir`, or null when it escapes `dir`.
export function resolveRequest(dir, urlPath) {
  let p;
  try { p = decodeURIComponent(urlPath.split("?")[0]); } catch { return null; }
  if (p.includes("\0") || p.split(/[\\/]+/).includes("..")) return null;
  const rel = normalize(p === "/" ? "/index.html" : p).replace(/^[/\\]+/, "");
  const full = resolve(dir, rel);
  const base = resolve(dir);
  return full === base || full.startsWith(base + sep) ? full : null;
}

// `wasmType` overrides the Content-Type of .wasm files: the drill for a host that types the module wrongly (QW3).
export async function serve(dir, { wasmType = null } = {}) {
  const log = [];
  const server = createServer((req, res) => {
    const file = resolveRequest(dir, req.url ?? "/");
    const ok = file && existsSync(file) && statSync(file).isFile();
    const type = ok ? ((wasmType && extname(file) === ".wasm" ? wasmType : MIME[extname(file)]) ?? "application/octet-stream") : null;
    log.push({ path: req.url, status: ok ? 200 : 404, type });
    if (!ok) { res.writeHead(404); res.end("not found"); return; }
    res.writeHead(200, { "Content-Type": type, "Cache-Control": "no-store" });
    res.end(readFileSync(file));
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, log, close: () => new Promise((r) => server.close(r)) };
}

// ---- PNG: just enough to tell a drawn canvas from a blank one ----
export function decodePng(buf) {
  if (buf.length < 33 || buf.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG");
  let off = 8, w = 0, h = 0, depth = 0, color = 0, interlace = 0;
  const idat = [];
  while (off + 8 <= buf.length) {
    const len = buf.readUInt32BE(off), type = buf.toString("latin1", off + 4, off + 8), data = buf.subarray(off + 8, off + 8 + len);
    if (type === "IHDR") { w = data.readUInt32BE(0); h = data.readUInt32BE(4); depth = data[8]; color = data[9]; interlace = data[12]; }
    else if (type === "IDAT") idat.push(data);
    else if (type === "IEND") break;
    off += 12 + len;
  }
  if (depth !== 8 || (color !== 2 && color !== 6) || interlace !== 0) throw new Error(`unsupported PNG (depth ${depth}, color type ${color}, interlace ${interlace})`);
  const bpp = color === 6 ? 4 : 3, stride = w * bpp;
  const raw = inflateSync(Buffer.concat(idat));
  if (raw.length !== h * (stride + 1)) throw new Error("PNG data has the wrong length");
  const px = Buffer.alloc(h * stride);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (stride + 1)], src = y * (stride + 1) + 1, dst = y * stride;
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? px[dst + x - bpp] : 0, b = y > 0 ? px[dst - stride + x] : 0, c = x >= bpp && y > 0 ? px[dst - stride + x - bpp] : 0;
      let v = raw[src + x];
      if (f === 1) v += a; else if (f === 2) v += b; else if (f === 3) v += (a + b) >> 1;
      else if (f === 4) { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c; }
      else if (f !== 0) throw new Error(`bad PNG filter ${f}`);
      px[dst + x] = v & 255;
    }
  }
  return { width: w, height: h, channels: bpp, data: px };
}

// Colour statistics of a decoded PNG. `distinct` counts exact RGB values (capped), `dominantShare` is the
// biggest single colour's share of all pixels, `meanLuma` is 0..255.
export function analyzePixels({ width, height, channels, data }) {
  const counts = new Map();
  let luma = 0;
  const n = width * height;
  for (let i = 0; i < n; i++) {
    const o = i * channels, r = data[o], g = data[o + 1], b = data[o + 2];
    luma += 0.2126 * r + 0.7152 * g + 0.0722 * b;
    const k = (r << 16) | (g << 8) | b;
    counts.set(k, (counts.get(k) ?? 0) + 1);
  }
  let top = 0;
  for (const c of counts.values()) if (c > top) top = c;
  return { width, height, distinct: counts.size, dominantShare: n ? top / n : 1, meanLuma: n ? luma / n : 0 };
}

// Drawn = not the black page, not one flat colour. The shell clears to a mid-bright colour and draws a square
// and two text boxes over it, so a working frame is bright on average and holds several colours.
export const isDrawn = (s) => s.meanLuma >= 20 && s.distinct >= 4 && s.dominantShare < 0.98;

// ---- the arena frame (WEB5) ----
// A headless canvas screenshot can come out black, so the arena page asks the GPU for the pixels itself: it copies the
// swapchain image into a buffer, maps it, and leaves window.arenaCapture = { line, rgba }. The check below takes those
// bytes, proves they are the ones the page hashed, tests that they are not blank and turns them into a PNG.

// FNV-1a 64 over bytes as 16 lowercase hex digits: the digest the render crate prints for a capture.
export function fnv1a64(buf) {
  const prime = 0x100000001b3n, mask = (1n << 64n) - 1n;
  let h = 0xcbf29ce484222325n;
  for (let i = 0; i < buf.length; i++) h = ((h ^ BigInt(buf[i])) * prime) & mask;
  return h.toString(16).padStart(16, "0");
}

// A PNG (8-bit RGBA, no interlace, filter 0) of tight RGBA rows.
export function encodePng(width, height, rgba) {
  const stride = width * 4;
  if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1 || rgba.length !== stride * height) throw new Error(`encodePng: ${rgba.length} bytes are not a ${width}x${height} RGBA image`);
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y++) Buffer.from(rgba.buffer, rgba.byteOffset + y * stride, stride).copy(raw, y * (stride + 1) + 1);
  const chunk = (type, data) => {
    const len = Buffer.alloc(4), crc = Buffer.alloc(4), t = Buffer.from(type, "latin1");
    len.writeUInt32BE(data.length);
    crc.writeUInt32BE(crc32(Buffer.concat([t, data])) >>> 0);
    return Buffer.concat([len, t, data, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0); ihdr.writeUInt32BE(height, 4); ihdr[8] = 8; ihdr[9] = 6;
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", ihdr), chunk("IDAT", deflateSync(raw, { level: 6 })), chunk("IEND", Buffer.alloc(0))]);
}

// What the arena page proved: the GPU copy arrived intact and shows a drawn frame, and the page's own receipt counts
// no validation error of any kind and at least one frame. `a` is readArena's result; null (not an arena page) is fine.
export function arenaVerdict(a) {
  if (!a) return { ok: true, problems: [] };
  const problems = [];
  if (a.capture !== "ready") problems.push(`the GPU copy of the canvas ended as ${a.capture ?? "never finished"}${a.captureLine?.blank ? " (blank)" : ""}`);
  else {
    if (!a.hashOk) problems.push(`the bytes that arrived hash to ${a.nodeHash}, the page said ${a.captureLine?.hash}`);
    if (!a.stats || !isDrawn(a.stats)) problems.push(`the copied canvas is blank (${JSON.stringify(a.stats)})`);
  }
  const rc = a.receipt;
  if (!rc) problems.push(`the page's receipt is not JSON: ${String(a.receiptText).slice(0, 160)}`);
  else {
    if (rc.validationErrors !== 0) problems.push(`validation errors ${rc.validationErrors}${rc.firstError ? `: ${rc.firstError}` : ""}`);
    if (rc.frameErrors !== 0) problems.push(`frame errors ${rc.frameErrors}${rc.firstError ? `: ${rc.firstError}` : ""}`);
    if (!(rc.frames > 0)) problems.push("the receipt counts no frame");
    // WEB13: a pipeline built after the loader is a stutter the page names loudly; the run fails on it.
    if (rc.latePipelines > 0) problems.push(`late_compile: ${rc.latePipelines} pipeline(s) built after the loader (${String(a.pipelinesText).slice(0, 200)})`);
    // WEB8: the page names its tier, the ready-line tier and the receipt tier agree, and the one-second split has all four fields.
    if (typeof rc.tier !== "string" || !rc.tier) problems.push("the receipt names no tier");
    else if (!/^tier=[a-z]+ /.test(String(a.tierText))) problems.push(`the page prints no tier= line: ${String(a.tierText).slice(0, 120)}`);
    else if (!String(a.tierText).startsWith(`tier=${rc.tier} `)) problems.push(`the tier line says ${String(a.tierText).split(" ")[0]}, the receipt says tier=${rc.tier}`);
    if (!/^pipelines=\d+ ms=\d+/.test(String(a.pipelinesText))) problems.push(`the page prints no pipelines=<n> ms=<t> line: ${String(a.pipelinesText).slice(0, 120)}`);
  }
  return { ok: problems.length === 0, problems };
}

// ---- what a run proved ----
export function verdict(r) {
  if (r.timedOut) return { ok: false, code: "timeout", why: `the page never left data-state=starting within ${r.waitedMs} ms` };
  const line = r.report;
  if (!line) return { ok: false, code: "no-report", why: `#report is not JSON: ${String(r.reportText).slice(0, 200)}` };
  if (line.ok === false) return { ok: false, code: line.reason ?? "failed", why: `${line.message ?? ""} ${line.detail ?? ""}`.trim() };
  if (line.ok !== true) return { ok: false, code: "bad-line", why: "the line has no ok field" };
  if (r.arena) {
    // The arena page checks its own pixels through the GPU; a headless screenshot of its canvas may be black.
    const av = arenaVerdict(r.arena);
    if (!av.ok) return { ok: false, code: "arena", why: av.problems.join("; ") };
  } else if (!r.stats || !isDrawn(r.stats)) return { ok: false, code: "blank-canvas", why: `adapter ok but the screenshot is blank (${JSON.stringify(r.stats)})` };
  if (!r.framesAdvanced) return { ok: false, code: "frozen", why: "adapter ok but the frame counter does not move" };
  if (r.recovered === false) return { ok: false, code: "no-recovery", why: "the page did not come back after the device was destroyed" };
  if (r.arena && r.recovered) {
    const lv = lossVerdict(r.loss);
    if (!lv.ok) return { ok: false, code: "loss-receipt", why: lv.problems.join("; ") };
  }
  if (r.arena && r.visibility) {
    const vv = visibilityVerdict(r.visibility);
    if (!vv.ok) return { ok: false, code: "visibility", why: vv.problems.join("; ") };
  }
  if (r.destroyed) {
    const dv = destroyVerdict(r.destroyed);
    if (!dv.ok) return { ok: false, code: "requested-destroy", why: dv.problems.join("; ") };
  }
  return { ok: true, code: "adapter-ok", why: `${line.adapter?.vendor || "?"} ${line.adapter?.architecture || ""} ${line.adapter?.description || line.adapter?.name || ""}`.trim() };
}

// The golden line the page prints: the engine itself ran in the wasm module and got the frozen numbers. The first line
// of #golden is `golden <golden> state <state> corpus <corpus> OK <n> ms`; `data-golden` on <body> says ok, mismatch or error.
export const GOLDEN_OK = /^golden [0-9a-f]{16} state [0-9a-f]{16} corpus [0-9a-f]{16} OK \d+ ms$/;
export function goldenVerdict(text, state) {
  const lines = String(text ?? "").split("\n");
  const first = lines[0] ?? "";
  if (state !== "ok") return { ok: false, line: first, tickLine: lines[1] ?? null, why: `golden check ended as ${state ?? "never finished"}: ${first.slice(0, 300)}` };
  if (!GOLDEN_OK.test(first)) return { ok: false, line: first, tickLine: lines[1] ?? null, why: `data-golden is ok but the line is not the OK shape: ${first.slice(0, 300)}` };
  return { ok: true, line: first, tickLine: lines[1] ?? null, why: "" };
}

// The no-WebGPU run is right when the loud message shows and the line names the reason.
export function noWebGpuVerdict(r) {
  const problems = [];
  if (r.timedOut) problems.push("the page never left data-state=starting");
  if (r.state !== "fatal") problems.push(`data-state is ${r.state}, expected fatal`);
  if (r.fatalDisplay !== "block") problems.push(`the message is display:${r.fatalDisplay}, not shown`);
  for (const v of ["Chrome 113", "Edge 113", "Safari 26", "Firefox 141"]) if (!String(r.fatalText).includes(v)) problems.push(`the message lacks ${v}`);
  if (!r.report || r.report.ok !== false || r.report.reason !== "no-webgpu") problems.push(`the line is not ok:false reason no-webgpu: ${String(r.reportText).slice(0, 160)}`);
  // QT11: the reason is named on the page (not a blank canvas, not a generic error) and in the line as fallback=<reason>.
  if (!String(r.fatalText).includes("fallback=no-webgpu")) problems.push(`the message does not say fallback=no-webgpu: ${String(r.fatalText).replace(/\s+/g, " ").slice(0, 160)}`);
  if (r.report?.fallback !== "no-webgpu") problems.push(`the line has no "fallback":"no-webgpu": ${String(r.reportText).slice(0, 160)}`);
  if (r.exceptions.length) problems.push(`uncaught exceptions: ${r.exceptions.join(" | ")}`);
  const g = goldenVerdict(r.golden, r.goldenState);
  if (!g.ok) problems.push(g.why);
  return { ok: problems.length === 0, problems };
}

// ---- device loss (QW8) ----
// A forced loss (window.loseDevice) is a loss: the rebuilt page's receipt counts it and carries the browser's own GPUDeviceLostInfo
// reason and message with who ended the device, and its frame counter moves. A requested destroy (window.destroyDevice) is not a
// loss: the page stops (data-state=stopped) and never starts a new device generation.
export function lossVerdict(rc) {
  if (!rc) return { ok: false, problems: ["no receipt JSON after the forced loss"] };
  const problems = [];
  if (rc.losses !== 1) problems.push(`the receipt counts ${rc.losses} losses after one forced loss`);
  const n = rc.lastLoss;
  if (!n) problems.push("the receipt has no lastLoss");
  else {
    if (typeof n.reason !== "string" || !n.reason) problems.push("lastLoss carries no GPUDeviceLostInfo reason");
    if (n.cause !== "forced") problems.push(`lastLoss cause is ${n.cause}, expected forced`);
  }
  if (!(rc.frames > 0)) problems.push("the rebuilt page counts no frame in its receipt");
  return { ok: problems.length === 0, problems };
}

export function destroyVerdict(d) {
  if (!d) return { ok: false, problems: ["the requested-destroy drill did not report"] };
  const problems = [];
  if (d.state !== "stopped") problems.push(`after a requested destroy data-state is ${d.state}, expected stopped`);
  if (d.generation !== d.generationBefore) problems.push(`a requested destroy started generation ${d.generation} (was ${d.generationBefore}): it must not retry`);
  return { ok: problems.length === 0, problems };
}

// QT7: a visibilitychange (the tab came back from hidden) makes the next frame a skipped frame, counted as hidden_skips in the split line.
export function visibilityVerdict(v) {
  if (!v) return { ok: false, problems: ["the visibility drill did not report"] };
  const problems = [];
  if (!(v.before >= 0)) problems.push("the split line never showed hidden_skips before the drill");
  else if (v.after !== v.before + 1) problems.push(`one visibilitychange moved hidden_skips from ${v.before} to ${v.after}, expected ${v.before + 1}`);
  return { ok: problems.length === 0, problems };
}

// QW14: with the animation frozen (?freeze) every frame is the same picture. Two captures with nothing between them must agree
// (else the check proves nothing), and a capture after memory.grow must equal them: a grow leaves the frame hash unchanged.
export function growHashVerdict(f) {
  if (!f?.still?.[0] || !f.still[1] || !f.afterGrow) return { ok: false, problems: ["the frozen page gave fewer than three captures"] };
  const [a, b] = f.still;
  const problems = [];
  if (a.blank || b.blank || f.afterGrow.blank) problems.push("a frozen capture is blank");
  if (a.hash !== b.hash) problems.push(`the frozen frame is not still (${a.hash} then ${b.hash}), so a grow check would prove nothing`);
  else if (f.afterGrow.hash !== b.hash) problems.push(`memory.grow changed the frame hash: ${b.hash} before, ${f.afterGrow.hash} after`);
  return { ok: problems.length === 0, problems };
}

// ---- how the module arrived (QW3, QW12) ----
// The page publishes window.loadStats = { bundle: network|cache, bytesFetched, downloadMs, compileMs, initMs, firstFrameMs, cache, persisted }.
// A fresh profile loads from the network, stores the copy under its hash and reports three timings; the same page loaded again in that
// profile (`again`) reads the copy: bundle=cache and no wasm byte fetched.
export function loadVerdict(first, again) {
  const problems = [];
  if (!first) return { ok: false, problems: ["the page published no load stats (window.loadStats)"] };
  if (first.bundle !== "network") problems.push(`the first load of a fresh profile says bundle=${first.bundle}`);
  if (!(first.bytesFetched > 0)) problems.push(`the first load fetched ${first.bytesFetched} bytes`);
  for (const k of ["downloadMs", "compileMs", "initMs", "firstFrameMs"]) if (!(Number.isFinite(first[k]) && first[k] >= 0)) problems.push(`${k} is ${first[k]}`);
  if (first.cache !== "stored") problems.push(`the copy was not stored: cache=${first.cache}`);
  if (!again) problems.push("the second load in the same profile did not report");
  else {
    if (again.bundle !== "cache") problems.push(`the second load says bundle=${again.bundle}, not cache`);
    if (again.bytesFetched !== 0) problems.push(`the second load fetched ${again.bytesFetched} bytes, not 0`);
  }
  return { ok: problems.length === 0, problems };
}

// With Cache.put forced to throw QuotaExceededError the page still loads and says so.
export const QUOTA_SCRIPT = "Cache.prototype.put = () => Promise.reject(new DOMException('forced', 'QuotaExceededError'));";
export const quotaVerdict = (load) => (load?.cache === "quota" ? { ok: true, problems: [] } : { ok: false, problems: [`with Cache.put refusing, the page says cache=${load?.cache ?? "nothing"}, not quota`] });

// A host that types the wasm wrongly must give the fatal page with the reason, not a blank page.
export function badMimeVerdict(r) {
  const problems = [];
  if (r.state !== "fatal") problems.push(`data-state is ${r.state}, expected fatal`);
  if (r.fatalDisplay !== "block") problems.push(`the fatal message is display:${r.fatalDisplay}`);
  for (const t of ["The engine did not start", "text/plain", "application/wasm"]) if (!String(r.fatalText).includes(t)) problems.push(`the fatal text lacks "${t}": ${String(r.fatalText).slice(0, 200)}`);
  return { ok: problems.length === 0, problems };
}

// ---- Chrome over CDP ----
export async function connectCdp(url) {
  const ws = new WebSocket(url);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error(`cannot connect to ${url}`)); });
  let nextId = 0;
  const pending = new Map(), handlers = new Map();
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id !== undefined) {
      const p = pending.get(m.id);
      if (!p) return;
      pending.delete(m.id);
      if (m.error) p.rej(new Error(`${p.method}: ${m.error.message}`)); else p.res(m.result);
    } else for (const f of handlers.get(m.method) ?? []) f(m.params);
  };
  ws.onclose = () => { for (const p of pending.values()) p.rej(new Error(`${p.method}: connection closed`)); pending.clear(); };
  return {
    send: (method, params = {}, timeoutMs = 30000) => new Promise((res, rej) => {
      const id = ++nextId;
      const t = setTimeout(() => { pending.delete(id); rej(new Error(`${method}: no answer in ${timeoutMs} ms`)); }, timeoutMs);
      pending.set(id, { method, res: (v) => { clearTimeout(t); res(v); }, rej: (e) => { clearTimeout(t); rej(e); } });
      ws.send(JSON.stringify({ id, method, params }));
    }),
    on: (method, fn) => handlers.set(method, [...(handlers.get(method) ?? []), fn]),
    close: () => { try { ws.close(); } catch { /* already closed */ } },
  };
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export async function launchChrome(chrome, flags, logFile) {
  const profile = mkdtempSync(join(tmpdir(), "web-browse-"));
  const args = [...flags, "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--enable-logging=stderr", "about:blank"];
  const proc = spawn(chrome, args, { stdio: ["ignore", "ignore", "pipe"] });
  proc.stderr.on("data", (d) => appendFileSync(logFile, d));
  let exited = false;
  proc.on("exit", () => { exited = true; });
  let port = null;
  for (let i = 0; i < 100 && !port && !exited; i++) {
    await sleep(200);
    try { port = Number(readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0]) || null; } catch { /* not written yet */ }
  }
  if (!port) { try { proc.kill(); } catch { /* gone */ } rmSync(profile, { recursive: true, force: true }); throw new Error(`Chrome did not open a debugging port (${exited ? "it exited" : "timeout"}); see ${logFile}`); }
  const list = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = list.find((t) => t.type === "page");
  if (!page) throw new Error("Chrome has no page target");
  const cdp = await connectCdp(page.webSocketDebuggerUrl);
  return {
    cdp,
    port,
    async close() {
      try { await cdp.send("Browser.close", {}, 5000); } catch { /* closing drops the socket */ }
      cdp.close();
      for (let i = 0; i < 50 && !exited; i++) await sleep(200);
      if (!exited) { try { proc.kill(); } catch { /* gone */ } }
      // Windows keeps the profile locked a moment after Chrome exits; a leftover temp folder is harmless.
      try { rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 }); } catch { /* left in the temp directory */ }
    },
  };
}

// The browse receipt: what the ready line said about wasm memory plus the time to the first frame.
// WEB8/WEB13: the arena page's tier, validation count and pipeline warm-up ride along (null on the plain shell page).
// `arena` is readArena's result: its receipt is the page's own final count, newer than the ready line.
// QW3/QW12: `load` is the page's window.loadStats of the first load, `reload` the same page loaded again in the same profile;
// the receipt keeps the three timings (download, compile, first frame) apart from the harness's own ttff, and the cache facts.
export const receiptOf = (report, firstFrameMs, set, arena = null, load = null, reload = null) => ({
  schema: 1, set, wasmMemoryBytes: report?.wasmMemoryBytes ?? null, ttffMs: firstFrameMs ?? null, commit: report?.commit ?? null,
  load: load && { bundle: load.bundle, bytesFetched: load.bytesFetched, downloadMs: load.downloadMs, compileMs: load.compileMs, initMs: Math.round(load.initMs), firstFrameMs: Math.round(load.firstFrameMs), cache: load.cache, persisted: load.persisted },
  reload: reload && { bundle: reload.bundle, bytesFetched: reload.bytesFetched },
  tier: arena?.receipt?.tier ?? report?.tier ?? null,
  validation: arena?.receipt?.validationErrors ?? report?.validation ?? null,
  pipelines: report?.pipelines ?? null, pipelinesMs: report?.pipelinesMs ?? null,
  latePipelines: arena?.receipt?.latePipelines ?? report?.latePipelines ?? null,
});

const parseLine =(text) => { try { const v = JSON.parse(text); return v && typeof v === "object" ? v : null; } catch { return null; } };

// ---- scripted input (AIS2 browser.input): events become Chrome DevTools Input.* calls ----
// An event is { kind, key?, x?, y?, ms? }. kind: key (down then up), key_down, key_up, mouse_move, click, wait.
export const INPUT_KINDS = ["key", "key_down", "key_up", "mouse_move", "click", "wait"];
const KEY_CODES = { ArrowLeft: 37, ArrowUp: 38, ArrowRight: 39, ArrowDown: 40, Space: 32, Enter: 13, Escape: 27, Tab: 9, Shift: 16, Control: 17, Alt: 18 };
function keyParams(name) {
  if (name === "Space") return { key: " ", code: "Space", windowsVirtualKeyCode: 32, text: " " };
  if (/^[A-Za-z]$/.test(name)) return { key: name, code: `Key${name.toUpperCase()}`, windowsVirtualKeyCode: name.toUpperCase().charCodeAt(0), text: name };
  if (/^[0-9]$/.test(name)) return { key: name, code: `Digit${name}`, windowsVirtualKeyCode: name.charCodeAt(0), text: name };
  if (name in KEY_CODES) return { key: name, code: name, windowsVirtualKeyCode: KEY_CODES[name], ...(name === "Enter" ? { text: "\r" } : {}) };
  throw new Error(`unknown key "${name}"`);
}
// The CDP calls (and waits) for a list of events, in order. Throws on an event it cannot map.
export function inputCommands(events) {
  const out = [];
  const point = (e) => {
    if (!Number.isInteger(e.x) || !Number.isInteger(e.y)) throw new Error(`${e.kind} needs integer x and y`);
    return { x: e.x, y: e.y };
  };
  for (const e of events) {
    if (e.kind === "key" || e.kind === "key_down" || e.kind === "key_up") {
      const p = keyParams(e.key);
      const { text, ...up } = p;
      if (e.kind !== "key_up") out.push({ method: "Input.dispatchKeyEvent", params: { type: text ? "keyDown" : "rawKeyDown", ...p } });
      if (e.kind !== "key_down") out.push({ method: "Input.dispatchKeyEvent", params: { type: "keyUp", ...up } });
    } else if (e.kind === "mouse_move") {
      out.push({ method: "Input.dispatchMouseEvent", params: { type: "mouseMoved", ...point(e) } });
    } else if (e.kind === "click") {
      const p = point(e);
      out.push({ method: "Input.dispatchMouseEvent", params: { type: "mouseMoved", ...p } });
      out.push({ method: "Input.dispatchMouseEvent", params: { type: "mousePressed", ...p, button: "left", buttons: 1, clickCount: 1 } });
      out.push({ method: "Input.dispatchMouseEvent", params: { type: "mouseReleased", ...p, button: "left", buttons: 0, clickCount: 1 } });
    } else if (e.kind === "wait") {
      if (!Number.isInteger(e.ms) || e.ms < 0 || e.ms > 2000) throw new Error("wait needs ms in 0..=2000");
      out.push({ wait: e.ms });
    } else throw new Error(`unknown input kind "${e.kind}" (legal: ${INPUT_KINDS.join(", ")})`);
  }
  return out;
}

// The arena page's own proof, read after the first frames: wait for the page's GPU copy of the canvas, bring the bytes over,
// check them against the page's hash and make the PNG. null on a page that is not the arena (no data-mode=arena).
export async function readArena({ evalJs, sleep: nap }, waitMs = 150000) {
  if (!(await evalJs("document.body.dataset.mode === 'arena'"))) return null;
  const t0 = Date.now();
  let capture = null;
  while (Date.now() - t0 < waitMs) {
    capture = await evalJs("document.body.dataset.capture ?? null");
    if (capture) break;
    await nap(500);
  }
  const text = (id) => evalJs(`document.getElementById('${id}')?.textContent ?? ''`);
  const out = { capture, waitedMs: Date.now() - t0, receiptText: await text("receipt"), passes: await text("passes"), init: await text("init"), captureText: await text("capture"), tierText: await text("tier"), splitText: await text("split"), pipelinesText: await text("pipelines") };
  out.receipt = parseLine(out.receiptText);
  out.captureLine = parseLine(out.captureText);
  if (capture === "ready" || capture === "blank") {
    const packed = await evalJs(`(async () => { const c = window.arenaCapture; if (!c) return null;
      const url = await new Promise((res, rej) => { const r = new FileReader(); r.onload = () => res(r.result); r.onerror = rej; r.readAsDataURL(new Blob([c.rgba])); });
      return url.slice(url.indexOf(',') + 1); })()`);
    const line = out.captureLine;
    if (packed && line) {
      const rgba = Buffer.from(packed, "base64");
      out.bytes = rgba.length;
      out.width = line.width; out.height = line.height;
      if (rgba.length === line.width * line.height * 4) {
        out.nodeHash = fnv1a64(rgba);
        out.hashOk = out.nodeHash === line.hash;
        out.stats = analyzePixels({ width: line.width, height: line.height, channels: 4, data: rgba });
        out.png = encodePng(line.width, line.height, rgba);
      } else out.hashOk = false;
    }
  }
  return out;
}

// Load `url` in a fresh Chrome and read the page back. `beforeLoad` is a script run before any page script.
// `needGolden: false` stops waiting at the first frame or the fatal page (a page that cannot load never prints a golden line).
export async function readPage({ chrome, flags, url, logFile, beforeLoad = null, waitMs = 90000, afterReady = null, inputEvents = null, needGolden = true }) {
  const b = await launchChrome(chrome, flags, logFile);
  const { cdp } = b;
  const consoleLines = [], exceptions = [];
  try {
    cdp.on("Runtime.consoleAPICalled", (p) => consoleLines.push(`${p.type}: ${(p.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ")}`.slice(0, 600)));
    cdp.on("Runtime.exceptionThrown", (p) => exceptions.push(String(p.exceptionDetails?.exception?.description ?? p.exceptionDetails?.text ?? "exception").slice(0, 400)));
    cdp.on("Log.entryAdded", (p) => consoleLines.push(`log ${p.entry.level}: ${p.entry.text}`.slice(0, 600)));
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
    if (beforeLoad) await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: beforeLoad });
    const evalJs = async (expression) => {
      const r = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      if (r.exceptionDetails) throw new Error(`evaluate: ${r.exceptionDetails.text} ${r.exceptionDetails.exception?.description ?? ""}`);
      return r.result.value;
    };
    await cdp.send("Page.navigate", { url });
    const t0 = Date.now();
    let state = null, goldenState = null, firstFrameMs = null;
    while (Date.now() - t0 < waitMs) {
      await sleep(250);
      try { [state, goldenState] = await evalJs("[document.body.dataset.state ?? null, document.body.dataset.golden ?? null]"); } catch { state = null; }
      // Time to first frame (WEB11): the first poll that sees the first frame on the canvas, good to one poll (250 ms).
      if (firstFrameMs === null && (state === "running" || state === "fatal")) firstFrameMs = Date.now() - t0;
      // The golden check runs after the adapter line, so wait for both.
      if ((state === "running" || state === "fatal") && (goldenState || !needGolden)) break;
    }
    const timedOut = state !== "running" && state !== "fatal";
    const read = () => evalJs(`(() => { const $ = (id) => document.getElementById(id); const c = $('canvas'); return {
      state: document.body.dataset.state, generation: document.body.dataset.generation ?? null,
      hud: $('hud').textContent, reportText: $('report').textContent,
      golden: $('golden').textContent, goldenState: document.body.dataset.golden ?? null,
      fatalDisplay: getComputedStyle($('fatal')).display, fatalText: $('fatal').innerText,
      hasGpu: typeof navigator.gpu !== 'undefined', canvas: [c.width, c.height], ua: navigator.userAgent }; })()`);
    const first = await read();
    const load = await evalJs(LOAD_STATS_JS);
    const probe = await evalJs(`(async () => { if (typeof navigator.gpu === 'undefined') return { gpu: false };
      try { const a = await navigator.gpu.requestAdapter(); if (!a) return { gpu: true, adapter: null };
        const i = a.info ?? (a.requestAdapterInfo ? await a.requestAdapterInfo() : {});
        return { gpu: true, adapter: { vendor: i.vendor, architecture: i.architecture, device: i.device, description: i.description, isFallback: a.isFallbackAdapter ?? null, features: [...a.features].length } };
      } catch (e) { return { gpu: true, error: String(e) }; } })()`);
    const out = { waitedMs: Date.now() - t0, firstFrameMs, timedOut, state: first.state, generation: first.generation, hud: first.hud, reportText: first.reportText, report: parseLine(first.reportText), golden: first.golden, goldenState: first.goldenState, fatalDisplay: first.fatalDisplay, fatalText: first.fatalText, hasGpu: first.hasGpu, canvas: first.canvas, ua: first.ua, load, probe, console: consoleLines, exceptions, screenshot: null, stats: null, framesAdvanced: null, recovered: null };
    if (!timedOut) {
      out.screenshot = Buffer.from((await cdp.send("Page.captureScreenshot", { format: "png" })).data, "base64");
      if (state === "running") {
        try { out.stats = analyzePixels(decodePng(out.screenshot)); } catch (e) { out.statsError = String(e); }
        const frames = (hud) => Number(/^frame (\d+)/.exec(hud)?.[1] ?? 0);
        const before = frames((await read()).hud);
        await sleep(1000);
        out.framesAdvanced = frames((await read()).hud) > before;
        out.arena = await readArena({ evalJs, sleep });
        if (inputEvents?.length) {
          // Scripted input replays from page load, in order, then one more screenshot shows what it did.
          for (const c of inputCommands(inputEvents)) {
            if (c.wait !== undefined) await sleep(c.wait); else await cdp.send(c.method, c.params);
          }
          await sleep(300);
          out.inputCount = inputEvents.length;
          out.inputScreenshot = Buffer.from((await cdp.send("Page.captureScreenshot", { format: "png" })).data, "base64");
        }
        if (afterReady) Object.assign(out, await afterReady({ evalJs, read, sleep }));
      }
    }
    return out;
  } finally {
    await b.close();
  }
}

// The page's load stats once its cache copy is settled (null on a page without them).
const LOAD_STATS_JS = "(async () => { await window.loadSettled; return window.loadStats ?? null; })()";

// Load the same page again in the same profile (the Cache API copy from the first load is still there) and read its load stats.
const reloadProbe = async ({ evalJs, sleep: nap }) => {
  await evalJs("window.oldDocument = true");
  evalJs("location.reload()").catch(() => { /* the page goes away under the call */ });
  const t0 = Date.now();
  while (Date.now() - t0 < 90000) {
    await nap(500);
    try {
      if (await evalJs("!window.oldDocument && window.loadStats?.firstFrameMs != null")) return { reload: await evalJs(LOAD_STATS_JS) };
    } catch { /* between documents */ }
  }
  return { reload: null };
};

// Destroy the device and wait for generation 2 with frames moving again.
const recoveryProbe = async ({ evalJs, read, sleep: nap }) => {
  await evalJs("window.loseDevice()");
  const t0 = Date.now();
  let r = null;
  while (Date.now() - t0 < 30000) {
    await nap(300);
    r = await read();
    if (r.generation === "2" && r.state === "running") break;
  }
  const ok = r?.generation === "2" && r.state === "running";
  // When it did not come back, ask the browser directly whether it still hands out an adapter: a plain
  // `null` there means the browser, not the page, could not recover.
  const direct = ok ? null : await evalJs("navigator.gpu.requestAdapter().then((a) => (a ? 'adapter' : 'null'), (e) => String(e))").catch((e) => String(e));
  const frames = (hud) => Number(/^frame (\d+)/.exec(hud)?.[1] ?? 0);
  let moving = false;
  if (ok) { const a = frames(r.hud); await nap(800); moving = frames((await read()).hud) > a; }
  // QW8: the rebuilt page's receipt (published every 15 frames) counts the loss and carries the browser's reason.
  let loss = null;
  if (ok) {
    for (const t1 = Date.now(); Date.now() - t1 < 20000; await nap(500)) {
      loss = parseLine(await evalJs("document.getElementById('receipt')?.textContent ?? ''"));
      if (loss?.losses >= 1) break;
    }
  }
  return { recovered: ok && moving, loss, recoveryState: r ? { state: r.state, generation: r.generation, hud: r.hud, adapterAfterLoss: direct } : null };
};

// QT7: tell the page its tab came back (the same event a hidden tab fires) and read the skipped frame in the split line.
const visibilityProbe = async ({ evalJs, sleep: nap }) => {
  const skips = async () => Number(/hidden_skips=(\d+)/.exec(await evalJs("document.getElementById('split')?.textContent ?? ''"))?.[1] ?? -1);
  let before = -1;
  for (const t0 = Date.now(); Date.now() - t0 < 30000 && before < 0; await nap(500)) before = await skips();
  await evalJs("document.dispatchEvent(new Event('visibilitychange'))");
  let after = before;
  for (const t0 = Date.now(); Date.now() - t0 < 60000 && after <= before; await nap(500)) after = await skips();
  return { visibility: { before, after } };
};

// QW8: a requested destroy (the page ending its device on purpose) stops the page and never starts a new generation.
const destroyProbe = async ({ evalJs, read, sleep: nap }) => {
  const before = await read();
  await evalJs("window.destroyDevice()");
  for (const t0 = Date.now(); Date.now() - t0 < 10000; await nap(300)) if ((await read()).state === "stopped") break;
  await nap(1500); // long enough for a restart to show itself, which it must not
  const after = await read();
  return { destroyed: { generationBefore: before.generation, state: after.state, generation: after.generation, hud: after.hud } };
};

// QW14: a forced memory.grow (16 pages) must leave the frames coming; with --soak the page then stays open for `soakSec` and every
// 60 s reading of window.memoryLog is kept. The wasm-bindgen views refresh by themselves after a grow; the page caches none.
const GROW_PAGES = 16;
const memoryProbe = (soakSec) => async ({ evalJs, read, sleep: nap }) => {
  const before = await evalJs("window.sampleMemory?.() ?? null");
  const after = await evalJs(`window.growMemory?.(${GROW_PAGES}) ?? null`);
  const frames = (hud) => Number(/^frame (\d+)/.exec(hud)?.[1] ?? 0);
  const f0 = frames((await read()).hud);
  await nap(1000);
  const moved = frames((await read()).hud) > f0;
  if (soakSec > 0) await nap(soakSec * 1000);
  return { memory: { before, after, moved, log: await evalJs("window.memoryLog ?? []") } };
};

// After the first frames: the forced memory grow, the device-loss drill, then the second load of the same page in the same profile.
const afterReadyProbes = (soakSec = 0) => async (ctx) => ({ ...(await memoryProbe(soakSec)(ctx)), ...(await visibilityProbe(ctx)), ...(await recoveryProbe(ctx)), ...(await reloadProbe(ctx)), ...(await destroyProbe(ctx)) });

// QW14: on a ?freeze page, capture the canvas twice, grow the memory, capture again. The captures are the page's own GPU copies.
const captureLine = async (evalJs) => parseLine(await evalJs("document.getElementById('capture')?.textContent ?? ''"));
async function takeCapture({ evalJs, sleep: nap }) {
  const was = (await captureLine(evalJs))?.frame ?? -1;
  await evalJs("window.requestArenaCapture()");
  for (const t0 = Date.now(); Date.now() - t0 < 90000; await nap(500)) {
    const line = await captureLine(evalJs);
    if (line && line.frame !== was) return line;
  }
  return null;
}
export const frozenGrowProbe = async (ctx) => {
  for (const t0 = Date.now(); Date.now() - t0 < 120000 && !(await ctx.evalJs("document.body.dataset.capture ?? null")); await ctx.sleep(500));
  const a = await takeCapture(ctx);
  const b = await takeCapture(ctx);
  const pages = await ctx.evalJs(`window.growMemory?.(${GROW_PAGES}) ?? null`);
  const afterGrow = await takeCapture(ctx);
  return { frozen: { still: [a, b], afterGrow, pages } };
};

// A forced grow adds exactly the pages asked for and the frame counter keeps moving.
export function memoryVerdict(m) {
  if (!m || m.before == null) return { ok: false, problems: ["the page published no memory readings (window.sampleMemory)"] };
  const problems = [];
  if (m.after !== m.before + GROW_PAGES) problems.push(`memory.grow(${GROW_PAGES}) took the module from ${m.before} to ${m.after} pages`);
  if (!m.moved) problems.push("the frame counter stopped after the memory grew");
  return { ok: problems.length === 0, problems };
}

export const NO_GPU_SCRIPT = "Object.defineProperty(Navigator.prototype, 'gpu', { get() { return undefined; }, configurable: true });";

const tailOf = (file, n = 40) => { try { return readFileSync(file, "utf8").trimEnd().split(/\r?\n/).slice(-n).join("\n"); } catch { return ""; } };

// The arena page's GPU copy as `<prefix>.png` (made from the bytes the page handed over) and its facts as `<prefix>.json`.
export function writeArena(dir, prefix, arena) {
  if (!arena) return;
  if (arena.png) writeFileSync(join(dir, `${prefix}.png`), arena.png);
  const { png, ...facts } = arena;
  writeFileSync(join(dir, `${prefix}.json`), `${JSON.stringify(facts, null, 2)}\n`);
}

export function parseArgs(argv) {
  const o = { input: null, dir: "dist/web", chrome: null, out: "out/web", requireAdapter: false, all: false, only: null, extra: process.env.WEB_CHROME_EXTRA ?? "", error: null, server: null, join: null, wt: null, cert: null, ticks: null, play: false, liveKeys: false, token: null, dropAtTick: null, expectHash: null, expectTransport: null, expectFailure: [], netTimeout: 90, badMime: false, soak: 0 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir") o.dir = argv[++i];
    else if (a === "--chrome") o.chrome = argv[++i];
    else if (a === "--out") o.out = argv[++i];
    else if (a === "--require-adapter") o.requireAdapter = true;
    else if (a === "--all") o.all = true;
    else if (a === "--only") o.only = Number(argv[++i]);
    else if (a === "--input") o.input = argv[++i] ?? null;
    else if (a === "--server") o.server = argv[++i] ?? null;
    else if (a === "--join") o.join = argv[++i] ?? null;
    else if (a === "--ticks") o.ticks = Number(argv[++i]);
    else if (a === "--play") o.play = true;
    else if (a === "--live-keys") { o.play = true; o.liveKeys = true; }
    else if (a === "--token") o.token = Number(argv[++i]);
    else if (a === "--drop-at-tick") o.dropAtTick = Number(argv[++i]);
    else if (a === "--wt") o.wt = argv[++i] ?? null;
    else if (a === "--cert") o.cert = argv[++i] ?? null;
    else if (a === "--expect-hash") o.expectHash = argv[++i] ?? null;
    else if (a === "--expect-transport") o.expectTransport = argv[++i] ?? null;
    else if (a === "--expect-failure") o.expectFailure.push(argv[++i] ?? "");
    else if (a === "--bad-mime") o.badMime = true;
    else if (a === "--soak") o.soak = Number(argv[++i]);
    else if (a === "--net-timeout") o.netTimeout = Number(argv[++i]);
    else o.error = `unknown argument ${a}`;
  }
  if (!o.chrome) o.error ??= "--chrome <path to Chrome> is required";
  if (o.only !== null && !(o.only >= 0)) o.error ??= "--only needs a flag set index";
  if (argv.includes("--input") && !o.input) o.error ??= "--input needs a JSON file of events";
  if (argv.includes("--server") && !/^wss?:\/\/[^/\s]+/.test(o.server ?? "")) o.error ??= "--server needs ws://host:port (or wss://host)";
  if (argv.includes("--join") && !/^(\[[0-9a-f:.]+\]|[A-Za-z0-9.-]+):\d{1,5}$/i.test(o.join ?? "")) o.error ??= "--join needs host:port";
  if (o.join && o.server) o.error ??= "give --server or --join, not both";
  if ((o.wt || o.cert) && !o.server && !o.join) o.error ??= "--wt and --cert go with --server (or --join)";
  if (o.join && o.wt) o.error ??= "--join serves both legs on its one port: --wt goes with --server";
  if (o.ticks !== null && !(Number.isInteger(o.ticks) && o.ticks >= 1 && o.ticks <= 100000)) o.error ??= "--ticks needs a match length of 1 to 100000";
  if (o.token !== null && !(Number.isSafeInteger(o.token) && o.token >= 1)) o.error ??= "--token needs a positive whole number";
  if (o.dropAtTick !== null && !(Number.isInteger(o.dropAtTick) && o.dropAtTick >= 1)) o.error ??= "--drop-at-tick needs a server tick (1 or more)";
  if ((o.play || o.token !== null || o.dropAtTick !== null || o.ticks !== null) && !o.server && !o.join) o.error ??= "--play, --live-keys, --token, --ticks and --drop-at-tick go with --server or --join";
  if (o.dropAtTick !== null && (o.expectHash || o.expectTransport || o.expectFailure.length)) o.error ??= "--drop-at-tick ends the page early: it does not go with --expect-hash, --expect-transport or --expect-failure";
  if (o.wt && !/^https:\/\//.test(o.wt)) o.error ??= "--wt needs https://host:port/e2040";
  if (o.cert && !/^[0-9a-f]{64}$/i.test(o.cert)) o.error ??= "--cert needs 64 hex digits (the SHA-256 of the certificate)";
  if (o.expectHash && !/^[0-9a-f]{16}$/i.test(o.expectHash)) o.error ??= "--expect-hash needs 16 hex digits";
  if (o.expectTransport && !["websocket", "webtransport"].includes(o.expectTransport)) o.error ??= "--expect-transport is websocket or webtransport";
  if (o.expectFailure.some((t) => !t) || (o.expectFailure.length && (o.expectHash || o.expectTransport))) o.error ??= "--expect-failure needs text and does not go with --expect-hash or --expect-transport";
  if (!(o.soak >= 0)) o.error ??= "--soak needs seconds (0 or more)";
  if (!(o.netTimeout >= 1)) o.error ??= "--net-timeout needs seconds";
  return o;
}

// ---- the net duel (WEB9, WEB12) ----
// The page joins a native dedicated server (`?server=ws://host:port`), plays 600 scripted ticks and prints one line to #net:
//   net: transport=<webtransport|websocket> ticks=<n> hash=<16 hex> peers=<n>        or        net: failed transport=<leg|none> reason=<text>
// `data-net` on <body> is running, ok or failed. The hash is the server's state hash of the last tick, which the page's own
// simulation reached too; `net_serve --reference` prints the same number offline.
export const NET_OK = /^net: transport=(webtransport|websocket) ticks=(\d+) hash=([0-9a-f]{16}) peers=(\d+)$/;
export const NET_TICKS = 600;

// The page address of a net run: the static server's origin plus the `?server=` (or `?join=`) keys and the session keys.
export function netUrl(port, { server = null, join = null, wt = null, cert = null, ticks = null, play = false, token = null }) {
  const q = new URLSearchParams();
  if (join) q.set("join", join); else q.set("server", server);
  if (wt) q.set("wt", wt);
  if (cert) q.set("cert", cert);
  if (ticks !== null) q.set("ticks", String(ticks));
  if (token !== null) q.set("token", String(token));
  if (play) q.set("play", "");
  return `http://127.0.0.1:${port}/index.html?${q}`;
}

// What a person does in a fight, as time-stamped key events (W held throughout; a swing, a turn, a dodge, a turn the other
// way, a swing again, every 2.4 s). The page's keyboard path turns them into the seat's input, so the tape is nobody's script.
// No Shift: CDP names it "Shift" and the page wants the code ShiftLeft.
export function liveKeyPlan(durationMs, period = 2400) {
  const beat = [[0, "key_down", "w"], [300, "key_down", "f"], [380, "key_up", "f"], [600, "key_down", "e"], [900, "key_up", "e"], [1200, "key_down", "Space"], [1280, "key_up", "Space"], [1500, "key_down", "q"], [1800, "key_up", "q"], [2000, "key_down", "f"], [2080, "key_up", "f"]];
  const plan = [];
  for (let base = 0; base < durationMs; base += period) {
    for (const [at, kind, key] of beat) if (base + at < durationMs && !(base > 0 && key === "w")) plan.push({ atMs: base + at, kind, key });
  }
  plan.push({ atMs: durationMs, kind: "key_up", key: "w" });
  return plan.sort((x, y) => x.atMs - y.atMs);
}

// What the page's net line proved against what the run expects. `want`: { hash, transport, ticks }.
export function netVerdict(text, state, want = {}) {
  const line = String(text ?? "").split("\n")[0].trim();
  if (want.failure?.length) {
    // A run that must fail, loudly: the page says failed and names every text asked for (for example the blocked leg).
    const problems = [];
    if (state !== "failed") problems.push(`expected the duel to fail, it ended as ${state ?? "never finished"}: ${line.slice(0, 300)}`);
    for (const t of want.failure) if (!line.includes(t)) problems.push(`the failure text lacks "${t}": ${line.slice(0, 400)}`);
    return { ok: problems.length === 0, line, parsed: null, problems };
  }
  if (state !== "ok") return { ok: false, line, parsed: null, problems: [`the net duel ended as ${state ?? "never finished"}: ${line.slice(0, 400)}`] };
  const m = NET_OK.exec(line);
  if (!m) return { ok: false, line, parsed: null, problems: [`data-net is ok but the line is not the net shape: ${line.slice(0, 300)}`] };
  const parsed = { transport: m[1], ticks: Number(m[2]), hash: m[3], peers: Number(m[4]) };
  const problems = [];
  if (parsed.ticks !== (want.ticks ?? NET_TICKS)) problems.push(`ticks ${parsed.ticks}, expected ${want.ticks ?? NET_TICKS}`);
  if (want.hash && parsed.hash !== String(want.hash).toLowerCase()) problems.push(`hash ${parsed.hash}, the server's is ${String(want.hash).toLowerCase()}`);
  if (want.transport && parsed.transport !== want.transport) problems.push(`transport ${parsed.transport}, expected ${want.transport}`);
  if (!(parsed.peers >= 1)) problems.push(`peers ${parsed.peers}`);
  return { ok: problems.length === 0, line, parsed, problems };
}

// Load the net page in a Chrome with the base flags only and wait for the line. Returns { text, state, console, exceptions, waitedMs, dropped }.
// `keys`: a liveKeyPlan, pressed as real key events once the match has started (data-net-tick appears). `dropAtTick`: leave the page
// (navigate to about:blank, which closes its sockets) once the server tick reaches it; `dropped` is then { tick } and the duel was not finished.
export async function readNetPage({ chrome, flags = BASE_FLAGS, url, logFile, waitMs = 90000, keys = null, dropAtTick = null }) {
  const b = await launchChrome(chrome, flags, logFile);
  const { cdp } = b;
  const consoleLines = [], exceptions = [];
  try {
    cdp.on("Runtime.consoleAPICalled", (p) => consoleLines.push(`${p.type}: ${(p.args ?? []).map((a) => a.value ?? a.description ?? "").join(" ")}`.slice(0, 600)));
    cdp.on("Runtime.exceptionThrown", (p) => exceptions.push(String(p.exceptionDetails?.exception?.description ?? p.exceptionDetails?.text ?? "exception").slice(0, 400)));
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    const evalJs = async (expression) => {
      const r = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      if (r.exceptionDetails) throw new Error(`evaluate: ${r.exceptionDetails.text} ${r.exceptionDetails.exception?.description ?? ""}`);
      return r.result.value;
    };
    await cdp.send("Page.navigate", { url });
    const t0 = Date.now();
    let state = null, text = "", tick = null, dropped = null, pressing = null;
    const stop = { done: false };
    while (Date.now() - t0 < waitMs) {
      await sleep(dropAtTick === null ? 250 : 50);
      try { [state, text, tick] = await evalJs("[document.body?.dataset.net ?? null, document.getElementById('net')?.textContent ?? '', document.body?.dataset.netTick ?? null]"); } catch { state = null; }
      if (state === "ok" || state === "failed") break;
      const live = tick === null ? null : Number(tick);
      if (keys && !pressing && live !== null) pressing = pressKeys(cdp, keys, stop);
      if (dropAtTick !== null && live !== null && live >= dropAtTick) {
        dropped = { tick: live };
        await cdp.send("Page.navigate", { url: "about:blank" });
        await sleep(300);
        break;
      }
    }
    stop.done = true;
    await pressing?.catch(() => {});
    return { text, state, console: consoleLines, exceptions, waitedMs: Date.now() - t0, dropped };
  } finally {
    await b.close();
  }
}

// Press a liveKeyPlan on the page in real time. Stops when `stop.done`; every held key is released at the end.
async function pressKeys(cdp, plan, stop) {
  const t0 = Date.now(), held = new Set();
  for (const step of plan) {
    const wait = step.atMs - (Date.now() - t0);
    if (wait > 0) await sleep(wait);
    if (stop.done) break;
    for (const c of inputCommands([{ kind: step.kind, key: step.key }])) if (c.method) await cdp.send(c.method, c.params);
    if (step.kind === "key_down") held.add(step.key); else held.delete(step.key);
  }
  for (const key of held) for (const c of inputCommands([{ kind: "key_up", key }])) await cdp.send(c.method, c.params).catch(() => {});
}

// `--server`: only the net duel runs. Prints NET LINE and the verdict; writes net.json (and the Chrome log) to --out.
async function netMain(a) {
  const dir = resolve(a.dir), out = resolve(a.out);
  if (!existsSync(join(dir, "index.html"))) { console.error(`web-browse: ${dir} has no index.html (run node tools/web-build.mjs)`); return 2; }
  mkdirSync(out, { recursive: true });
  const server = await serve(dir);
  try {
    const url = netUrl(server.port, a);
    const logFile = join(out, "chrome-net.log");
    writeFileSync(logFile, "");
    let r;
    // A live match runs about as long as its ticks at 60 a second; the key plan covers it with a margin.
    const keys = a.liveKeys ? liveKeyPlan((a.ticks ?? NET_TICKS) * 17 + 20000) : null;
    try { r = await readNetPage({ chrome: a.chrome, url, logFile, waitMs: a.netTimeout * 1000, keys, dropAtTick: a.dropAtTick }); } catch (e) { console.error(`web-browse: ${e.message ?? e}`); return 2; }
    if (a.dropAtTick !== null) {
      console.log(r.dropped ? `net duel: DROPPED the page at server tick ${r.dropped.tick} (asked for ${a.dropAtTick})` : `net duel: FAILED - the duel ended as ${r.state ?? "never started"} before tick ${a.dropAtTick}: ${r.text.slice(0, 300)}`);
      return r.dropped ? 0 : 1;
    }
    const v = netVerdict(r.text, r.state, { hash: a.expectHash, transport: a.expectTransport, failure: a.expectFailure, ticks: a.ticks ?? undefined });
    console.log(`NET LINE ${v.line}`);
    console.log(`net duel: ${v.ok ? (v.parsed ? `OK (${v.parsed.transport}, ${v.parsed.ticks} ticks, hash ${v.parsed.hash}${a.expectHash ? " equals the server's" : ""})` : "OK (failed loudly, as asked)") : `FAILED - ${v.problems.join("; ")}`}`);
    if (!v.ok) {
      for (const l of r.console.slice(0, 12)) console.log(`    console ${l.replace(/\s+/g, " ").slice(0, 400)}`);
      for (const l of r.exceptions.slice(0, 3)) console.log(`    exception ${l.replace(/\s+/g, " ").slice(0, 400)}`);
    }
    writeFileSync(join(out, "net.json"), `${JSON.stringify({ url, verdict: v, expected: { hash: a.expectHash, transport: a.expectTransport }, waitedMs: r.waitedMs, console: r.console.slice(0, 60), exceptions: r.exceptions }, null, 2)}\n`);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### net duel\n\`${v.line}\`\n${v.ok ? "ok" : `**FAILED** ${v.problems.join("; ")}`}\n`);
    return v.ok ? 0 : 1;
  } finally {
    await server.close();
  }
}

// `--bad-mime`: a host that types engine_bg.wasm as text/plain. The page must show the fatal message with the reason.
async function badMimeMain(a) {
  const dir = resolve(a.dir), out = resolve(a.out);
  if (!existsSync(join(dir, "index.html"))) { console.error(`web-browse: ${dir} has no index.html (run node tools/web-build.mjs)`); return 2; }
  mkdirSync(out, { recursive: true });
  const server = await serve(dir, { wasmType: "text/plain" });
  try {
    const logFile = join(out, "chrome-bad-mime.log");
    writeFileSync(logFile, "");
    let r;
    try { r = await readPage({ chrome: a.chrome, flags: BASE_FLAGS, url: `http://127.0.0.1:${server.port}/index.html`, logFile, waitMs: 40000, needGolden: false }); } catch (e) { console.error(`web-browse: ${e.message ?? e}`); return 2; }
    const v = badMimeVerdict(r);
    console.log(`BAD MIME fatal text: ${String(r.fatalText).replace(/\s+/g, " ").slice(0, 400)}`);
    console.log(`wrong MIME type: ${v.ok ? "OK (fatal page with the reason, not a blank page)" : `FAILED - ${v.problems.join("; ")}`}`);
    if (r.screenshot) writeFileSync(join(out, "bad-mime.png"), r.screenshot);
    return v.ok ? 0 : 1;
  } finally {
    await server.close();
  }
}

export async function main(argv) {
  const a = parseArgs(argv);
  if (a.error) { console.error(`web-browse: ${a.error}`); return 2; }
  if (a.server || a.join) return netMain(a);
  if (a.badMime) return badMimeMain(a);
  const dir = resolve(a.dir), out = resolve(a.out);
  if (!existsSync(join(dir, "index.html"))) { console.error(`web-browse: ${dir} has no index.html (run node tools/web-build.mjs)`); return 2; }
  mkdirSync(out, { recursive: true });
  let inputEvents = null;
  if (a.input) {
    try {
      inputEvents = JSON.parse(readFileSync(resolve(a.input), "utf8"));
      if (!Array.isArray(inputEvents)) throw new Error("the file is not a JSON array");
      inputCommands(inputEvents); // refuse a bad script before any browser starts
    } catch (e) { console.error(`web-browse: --input ${a.input}: ${e.message}`); return 2; }
  }
  const server = await serve(dir);
  const url = `http://127.0.0.1:${server.port}/index.html`;
  const attempts = [];
  let winner = null;
  const extraFlags = a.extra.split(/\s+/).filter(Boolean);
  const sets = [...FLAG_SETS, ...(extraFlags.length ? [{ name: "extra", flags: extraFlags }] : [])];
  try {
    for (const [i, set] of sets.entries()) {
      if (a.only !== null && i !== a.only) continue;
      const logFile = join(out, `chrome-${set.name}.log`);
      writeFileSync(logFile, "");
      const flags = [...BASE_FLAGS, ...set.flags];
      let r, v;
      try {
        r = await readPage({ chrome: a.chrome, flags, url, logFile, afterReady: afterReadyProbes(a.soak), inputEvents });
        v = verdict(r);
      } catch (e) {
        r = null; v = { ok: false, code: "run-error", why: String(e.message ?? e) };
      }
      const entry = { set: set.name, flags: flags.join(" "), verdict: v, state: r?.state ?? null, hasGpu: r?.hasGpu ?? null, probe: r?.probe ?? null, hud: r?.hud ?? null, stats: r?.stats ?? null, framesAdvanced: r?.framesAdvanced ?? null, recovered: r?.recovered ?? null, recoveryState: r?.recoveryState ?? null, console: (r?.console ?? []).slice(0, 30), exceptions: r?.exceptions ?? [], chromeLog: tailOf(logFile) };
      attempts.push(entry);
      console.log(`[${i}] ${set.name}: ${v.ok ? "ADAPTER OK" : v.code} - ${v.why}`);
      if (!v.ok) {
        // What a person reading only the job log needs to see why: the page's own errors, what the browser said
        // about WebGPU, and the last lines Chrome itself logged.
        if (entry.probe) console.log(`    navigator.gpu probe: ${JSON.stringify(entry.probe)}`);
        if (entry.recoveryState) console.log(`    after the device was destroyed: ${JSON.stringify(entry.recoveryState)}`);
        for (const l of entry.console.slice(0, 8)) console.log(`    console ${l.replace(/\s+/g, " ").slice(0, 400)}`);
        for (const l of entry.exceptions.slice(0, 3)) console.log(`    exception ${l.replace(/\s+/g, " ").slice(0, 400)}`);
        for (const l of entry.chromeLog.split("\n").filter((x) => /gpu|vulkan|webgpu|swiftshader|dawn|angle|adapter/i.test(x)).slice(-6)) console.log(`    chrome ${l.slice(0, 300)}`);
      }
      if (v.ok && !winner) {
        winner = { set, flags: flags.join(" "), report: r.report, reportText: r.reportText, stats: r.stats, golden: r.golden, goldenState: r.goldenState, firstFrameMs: r.firstFrameMs, arena: r.arena, load: r.load, reload: r.reload, memory: r.memory, loss: r.loss, destroyed: r.destroyed, visibility: r.visibility };
        writeFileSync(join(out, "ready.png"), r.screenshot);
        writeFileSync(join(out, "report.json"), `${r.reportText}\n`);
        writeArena(out, "arena-capture", r.arena);
        // The numbers tools/web-budget.mjs gates on (WEB11).
        writeFileSync(join(out, "receipt.json"), `${JSON.stringify({ ...receiptOf(r.report, r.firstFrameMs, set.name, r.arena, r.load, r.reload), inputEvents: r.inputCount ?? 0 }, null, 2)}\n`);
        if (r.inputScreenshot) writeFileSync(join(out, "input.png"), r.inputScreenshot);
        writeFileSync(join(out, "flag-line.txt"), `${a.chrome} ${flags.join(" ")}\n`);
      }
      if (!v.ok) writeArena(out, `attempt-${set.name}-arena`, r?.arena);
      if (v.ok && !a.all) break; // --all keeps going so every flag set's outcome is on record
      if (r?.screenshot) writeFileSync(join(out, `attempt-${set.name}.png`), r.screenshot);
    }

    if (winner) {
      console.log(`FLAG LINE ${a.chrome} ${winner.flags}`);
      console.log(`READY LINE ${winner.reportText}`);
      console.log(`CANVAS ${JSON.stringify(winner.stats)}`);
      if (winner.arena) {
        console.log(`ARENA CAPTURE ${winner.arena.captureText}`);
        console.log(`ARENA HASH page ${winner.arena.captureLine?.hash} node ${winner.arena.nodeHash} ${winner.arena.hashOk ? "EQUAL" : "DIFFERENT"}; ${winner.arena.init}`);
        console.log(`ARENA RECEIPT ${winner.arena.receiptText}`);
        console.log(`ARENA PASSES ${winner.arena.passes}`);
        console.log(`ARENA TIER ${winner.arena.tierText}`);
        console.log(`ARENA SPLIT ${winner.arena.splitText || "(the first second is not over yet)"}`);
        console.log(`ARENA PIPELINES ${winner.arena.pipelinesText}`);
      }
      if (winner.loss) console.log(`LOSS RECEIPT after the forced loss: losses ${winner.loss.losses}, lastLoss ${JSON.stringify(winner.loss.lastLoss)}, frames ${winner.loss.frames}`);
      if (winner.destroyed) console.log(`REQUESTED DESTROY ${JSON.stringify(winner.destroyed)}`);
      if (winner.visibility) console.log(`VISIBILITY hidden_skips ${winner.visibility.before} -> ${winner.visibility.after}`);
      const gw = goldenVerdict(winner.golden, winner.goldenState);
      console.log(`GOLDEN ${gw.ok ? "" : "FAILED "}${gw.line}`);
      if (gw.tickLine) console.log(`WASM TICK ${gw.tickLine}`);
    }

    // QW14: the frozen page, with the flags of the winning set: a memory grow must leave the frame hash alone.
    let frozenV = null;
    if (winner?.arena) {
      try {
        const fr = await readPage({ chrome: a.chrome, flags: [...BASE_FLAGS, ...winner.set.flags], url: `${url}?freeze`, logFile: join(out, "chrome-frozen.log"), afterReady: frozenGrowProbe, waitMs: 120000 });
        frozenV = growHashVerdict(fr.frozen);
        console.log(`FROZEN GROW ${frozenV.ok ? "OK" : "FAILED"}: captures ${JSON.stringify((fr.frozen?.still ?? []).map((c) => c?.hash))} then ${fr.frozen?.afterGrow?.hash} after memory.grow to ${fr.frozen?.pages} pages${frozenV.ok ? "" : ` - ${frozenV.problems.join("; ")}`}`);
      } catch (e) {
        frozenV = { ok: false, problems: [String(e.message ?? e)] };
        console.log(`FROZEN GROW FAILED: ${frozenV.problems[0]}`);
      }
    }

    // The no-WebGPU page, with the flags of the winning set (or the first one when none won).
    const base = [...BASE_FLAGS, ...(winner ? winner.set.flags : sets[0].flags)];
    let nogpu, nogpuV;
    try {
      nogpu = await readPage({ chrome: a.chrome, flags: base, url, logFile: join(out, "chrome-no-webgpu.log"), beforeLoad: `${NO_GPU_SCRIPT}
${QUOTA_SCRIPT}`, waitMs: 60000 });
      nogpuV = noWebGpuVerdict(nogpu);
      if (nogpu.screenshot) writeFileSync(join(out, "no-webgpu.png"), nogpu.screenshot);
    } catch (e) {
      nogpuV = { ok: false, problems: [String(e.message ?? e)] };
    }
    const gn = goldenVerdict(nogpu?.golden, nogpu?.goldenState);
    console.log(`NO-WEBGPU GOLDEN ${gn.ok ? "" : "FAILED "}${gn.line}`);
    console.log(`no-WebGPU page: ${nogpuV.ok ? "OK (loud message shown, reason no-webgpu, nothing thrown)" : `FAILED - ${nogpuV.problems.join("; ")}`}`);

    // QW3/QW12: the winner's load facts (three timings, then the second load from the cache) and the forced-quota page.
    const loadV = winner ? loadVerdict(winner.load, winner.reload) : null;
    const quotaV = quotaVerdict(nogpu?.load);
    const memV = winner ? memoryVerdict(winner.memory) : null;
    if (winner) {
      const log = winner.memory?.log ?? [];
      const peak = Math.max(0, ...log.map((x) => x.pages));
      console.log(`MEMORY ${memV.ok ? "" : "FAILED "}pages ${winner.memory?.before} -> ${winner.memory?.after} after a forced grow of ${GROW_PAGES}; peak ${peak} pages = ${peak * 65536} bytes (+25% = ${Math.ceil(peak * 65536 * 1.25)}); readings ${JSON.stringify(log.map((x) => x.pages))}${memV.ok ? "" : " - " + memV.problems.join("; ")}`);
    }
    if (winner) console.log(`LOAD ${loadV.ok ? "" : "FAILED "}${loadLine(winner.load ?? {}, winner.load?.initMs, winner.load?.firstFrameMs)} | again: ${JSON.stringify(winner.reload)}${loadV.ok ? "" : ` - ${loadV.problems.join("; ")}`}`);
    console.log(`QUOTA PAGE ${quotaV.ok ? "OK (Cache.put refused with QuotaExceededError, the page still loaded and said cache=quota)" : `FAILED - ${quotaV.problems.join("; ")}`}`);

    const goldenWinner = winner ? goldenVerdict(winner.golden, winner.goldenState) : null;
    const summary = { adapter: Boolean(winner), frozenGrow: frozenV, golden: { withAdapter: goldenWinner, withoutWebGpu: gn }, winner: winner ? { name: winner.set.name, flags: winner.flags } : null, noWebGpuPage: nogpuV, attempts, served: server.log };
    writeFileSync(join(out, "attempts.json"), `${JSON.stringify(summary, null, 2)}\n`);
    const md = [
      "### web-browse",
      winner ? `adapter ok with: \`${winner.flags}\`` : "**NO ADAPTER on this runner** with any flag set; reasons in attempts.json",
      winner ? `\`\`\`\n${winner.reportText}\n\`\`\`\ncanvas stats: ${JSON.stringify(winner.stats)}` : attempts.map((x) => `- ${x.set}: ${x.verdict.code} - ${x.verdict.why}`).join("\n"),
      `no-WebGPU page: ${nogpuV.ok ? "ok" : `FAILED ${nogpuV.problems.join("; ")}`}`,
      `golden (adapter page): \`${goldenWinner ? goldenWinner.line : "no adapter page"}\` ${goldenWinner?.tickLine ?? ""}`,
      `golden (no-WebGPU page): \`${gn.line}\``,
    ].join("\n");
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${md}\n`);

    if (!nogpuV.ok || !quotaV.ok || (loadV && !loadV.ok) || (memV && !memV.ok) || (frozenV && !frozenV.ok)) return 1;
    if (goldenWinner && !goldenWinner.ok) {
      console.log(`GOLDEN FAILED on the adapter page: ${goldenWinner.why}`);
      return 1;
    }
    if (!winner) {
      console.log("NO ADAPTER: every flag set was tried; see attempts.json for each reason.");
      return a.requireAdapter ? 3 : 0;
    }
    return 0;
  } finally {
    await server.close();
  }
}

if (import.meta.main) main(process.argv.slice(2)).then((c) => process.exit(c), (e) => { console.error(e); process.exit(2); });

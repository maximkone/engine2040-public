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
// Exit: 0 all proven (a missing adapter is only a failure with --require-adapter), 1 a proof failed,
// 2 could not run (no Chrome, bad arguments), 3 no adapter and --require-adapter was given.
// Only loopback is served and visited. The Chrome processes started here are ours and are closed here.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { extname, join, normalize, resolve, sep } from "node:path";
import { crc32, deflateSync, inflateSync } from "node:zlib";

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

export async function serve(dir) {
  const log = [];
  const server = createServer((req, res) => {
    const file = resolveRequest(dir, req.url ?? "/");
    const ok = file && existsSync(file) && statSync(file).isFile();
    log.push({ path: req.url, status: ok ? 200 : 404, type: ok ? (MIME[extname(file)] ?? "application/octet-stream") : null });
    if (!ok) { res.writeHead(404); res.end("not found"); return; }
    res.writeHead(200, { "Content-Type": MIME[extname(file)] ?? "application/octet-stream", "Cache-Control": "no-store" });
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
  if (r.exceptions.length) problems.push(`uncaught exceptions: ${r.exceptions.join(" | ")}`);
  const g = goldenVerdict(r.golden, r.goldenState);
  if (!g.ok) problems.push(g.why);
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
export const receiptOf = (report, firstFrameMs, set) => ({ schema: 1, set, wasmMemoryBytes: report?.wasmMemoryBytes ?? null, ttffMs: firstFrameMs ?? null, commit: report?.commit ?? null });

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
  const out = { capture, waitedMs: Date.now() - t0, receiptText: await text("receipt"), passes: await text("passes"), init: await text("init"), captureText: await text("capture") };
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
export async function readPage({ chrome, flags, url, logFile, beforeLoad = null, waitMs = 90000, afterReady = null, inputEvents = null }) {
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
      if ((state === "running" || state === "fatal") && goldenState) break;
    }
    const timedOut = state !== "running" && state !== "fatal";
    const read = () => evalJs(`(() => { const $ = (id) => document.getElementById(id); const c = $('canvas'); return {
      state: document.body.dataset.state, generation: document.body.dataset.generation ?? null,
      hud: $('hud').textContent, reportText: $('report').textContent,
      golden: $('golden').textContent, goldenState: document.body.dataset.golden ?? null,
      fatalDisplay: getComputedStyle($('fatal')).display, fatalText: $('fatal').innerText,
      hasGpu: typeof navigator.gpu !== 'undefined', canvas: [c.width, c.height], ua: navigator.userAgent }; })()`);
    const first = await read();
    const probe = await evalJs(`(async () => { if (typeof navigator.gpu === 'undefined') return { gpu: false };
      try { const a = await navigator.gpu.requestAdapter(); if (!a) return { gpu: true, adapter: null };
        const i = a.info ?? (a.requestAdapterInfo ? await a.requestAdapterInfo() : {});
        return { gpu: true, adapter: { vendor: i.vendor, architecture: i.architecture, device: i.device, description: i.description, isFallback: a.isFallbackAdapter ?? null, features: [...a.features].length } };
      } catch (e) { return { gpu: true, error: String(e) }; } })()`);
    const out = { waitedMs: Date.now() - t0, firstFrameMs, timedOut, state: first.state, generation: first.generation, hud: first.hud, reportText: first.reportText, report: parseLine(first.reportText), golden: first.golden, goldenState: first.goldenState, fatalDisplay: first.fatalDisplay, fatalText: first.fatalText, hasGpu: first.hasGpu, canvas: first.canvas, ua: first.ua, probe, console: consoleLines, exceptions, screenshot: null, stats: null, framesAdvanced: null, recovered: null };
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
  return { recovered: ok && moving, recoveryState: r ? { state: r.state, generation: r.generation, hud: r.hud, adapterAfterLoss: direct } : null };
};

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
  const o = { input: null, dir: "dist/web", chrome: null, out: "out/web", requireAdapter: false, all: false, only: null, extra: process.env.WEB_CHROME_EXTRA ?? "", error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--dir") o.dir = argv[++i];
    else if (a === "--chrome") o.chrome = argv[++i];
    else if (a === "--out") o.out = argv[++i];
    else if (a === "--require-adapter") o.requireAdapter = true;
    else if (a === "--all") o.all = true;
    else if (a === "--only") o.only = Number(argv[++i]);
    else if (a === "--input") o.input = argv[++i] ?? null;
    else o.error = `unknown argument ${a}`;
  }
  if (!o.chrome) o.error ??= "--chrome <path to Chrome> is required";
  if (o.only !== null && !(o.only >= 0)) o.error ??= "--only needs a flag set index";
  if (argv.includes("--input") && !o.input) o.error ??= "--input needs a JSON file of events";
  return o;
}

export async function main(argv) {
  const a = parseArgs(argv);
  if (a.error) { console.error(`web-browse: ${a.error}`); return 2; }
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
        r = await readPage({ chrome: a.chrome, flags, url, logFile, afterReady: recoveryProbe, inputEvents });
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
        winner = { set, flags: flags.join(" "), report: r.report, reportText: r.reportText, stats: r.stats, golden: r.golden, goldenState: r.goldenState, firstFrameMs: r.firstFrameMs, arena: r.arena };
        writeFileSync(join(out, "ready.png"), r.screenshot);
        writeFileSync(join(out, "report.json"), `${r.reportText}\n`);
        writeArena(out, "arena-capture", r.arena);
        // The numbers tools/web-budget.mjs gates on (WEB11).
        writeFileSync(join(out, "receipt.json"), `${JSON.stringify({ ...receiptOf(r.report, r.firstFrameMs, set.name), inputEvents: r.inputCount ?? 0 }, null, 2)}\n`);
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
      }
      const gw = goldenVerdict(winner.golden, winner.goldenState);
      console.log(`GOLDEN ${gw.ok ? "" : "FAILED "}${gw.line}`);
      if (gw.tickLine) console.log(`WASM TICK ${gw.tickLine}`);
    }

    // The no-WebGPU page, with the flags of the winning set (or the first one when none won).
    const base = [...BASE_FLAGS, ...(winner ? winner.set.flags : sets[0].flags)];
    let nogpu, nogpuV;
    try {
      nogpu = await readPage({ chrome: a.chrome, flags: base, url, logFile: join(out, "chrome-no-webgpu.log"), beforeLoad: NO_GPU_SCRIPT, waitMs: 60000 });
      nogpuV = noWebGpuVerdict(nogpu);
      if (nogpu.screenshot) writeFileSync(join(out, "no-webgpu.png"), nogpu.screenshot);
    } catch (e) {
      nogpuV = { ok: false, problems: [String(e.message ?? e)] };
    }
    const gn = goldenVerdict(nogpu?.golden, nogpu?.goldenState);
    console.log(`NO-WEBGPU GOLDEN ${gn.ok ? "" : "FAILED "}${gn.line}`);
    console.log(`no-WebGPU page: ${nogpuV.ok ? "OK (loud message shown, reason no-webgpu, nothing thrown)" : `FAILED - ${nogpuV.problems.join("; ")}`}`);

    const goldenWinner = winner ? goldenVerdict(winner.golden, winner.goldenState) : null;
    const summary = { adapter: Boolean(winner), golden: { withAdapter: goldenWinner, withoutWebGpu: gn }, winner: winner ? { name: winner.set.name, flags: winner.flags } : null, noWebGpuPage: nogpuV, attempts, served: server.log };
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

    if (!nogpuV.ok) return 1;
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

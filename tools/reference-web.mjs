// reference-web: capture the web tier's ten reference shots from the real page.
//
// The page draws on the browser's animation clock, so this tool drives that
// clock instead of sleeping: it replaces requestAnimationFrame and
// performance.now before the page loads, queues each callback, and feeds the
// page one exact 60 Hz timestamp per arena frame. The page still renders its
// own scene through WebGPU; the tool only chooses the tick and reads the
// pixels back through the page's own requestArenaCapture API.
//
// The page owns the camera. Today it is the procedural ring orbit, a function
// of the page clock; the fragment records that name (`procedural-arena`) so a
// web row never claims the native `arena600` camera by accident. A later page
// camera hook can pass `--camera arena600` once it really exists.
//
// Usage from the repo root:
//   node tools/reference-web.mjs --scratch out/ref-web
//   node tools/reference-web.mjs --scratch out/ref-web --passes 2
//   node tools/reference-web.mjs --write-content
//   node tools/reference-web.mjs --help
//
// Exit codes: 0 verified and written - 1 a capture or PNG failed - 2 usage.
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { decodePng, fnvHex } from "./reference-captures.mjs";
import { connectCdp, encodePng, launchChrome, serve } from "./web-browse.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const TIER = "web";
const FRAME_MS = 1000 / 60;
const DEFAULT_TICKS = [0, 60, 120, 180, 240, 300, 360, 420, 480, 540];
const DEFAULT_WIDTH = 640;
const DEFAULT_HEIGHT = 360;
const DEFAULT_SEED = 2040;
const DEFAULT_CAMERA = "procedural-arena";
const DEFAULT_SHOT_PREFIX = "web";
const DEFAULT_TIMEOUT_MS = 90_000;
/// How long the driver waits for one requested capture, in tries of one frozen frame plus a
/// real pause: a copy maps on the browser's own clock, and a software adapter can need more
/// wall time than a burst of frames gives it.
const CAPTURE_TRIES = 240;
const CAPTURE_PAUSE_MS = 20;

/// The page-side clock shim. It runs before every page script, so the arena's
/// warm-up frames all see time 0 and the first queued frame is the origin.
export const CLOCK_SHIM = `(() => {
  let elapsed = 0;
  try {
    Object.defineProperty(performance, "now", {
      configurable: true,
      value: () => elapsed,
    });
  } catch (error) {
    window.__referenceClockError = String(error);
  }
  const queue = [];
  window.requestAnimationFrame = (callback) => {
    queue.push(callback);
    window.__referenceQueued = queue.length;
    return queue.length;
  };
  window.cancelAnimationFrame = () => {};
  window.__referenceFrame = (ms) => {
    const callback = queue.shift();
    window.__referenceQueued = queue.length;
    if (!callback) return false;
    elapsed = ms;
    callback(ms);
    return true;
  };
  window.__referenceQueued = 0;
})();`;

const READ_RGBA = `(async () => {
  const capture = window.arenaCapture;
  if (!capture) return null;
  const url = await new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error ?? new Error("read failed"));
    reader.readAsDataURL(new Blob([capture.rgba]));
  });
  return url.slice(url.indexOf(",") + 1);
})()`;

const READ_REPORT = `(() => {
  const text = document.getElementById("report")?.textContent ?? "";
  try {
    const value = JSON.parse(text);
    return value && typeof value === "object" ? value : null;
  } catch {
    return null;
  }
})()`;

/// A page that stopped before its first frame (usually no WebGPU adapter).
export class PageFatal extends Error {
  constructor(message) {
    super(message);
    this.name = "PageFatal";
  }
}

/// Parse the tool's flags. Without `--scratch`, writing the real manifest folder
/// needs the explicit `--write-content`, so a mistake cannot touch content early.
export function parseArgs(argv) {
  const args = {
    pageDir: "dist/web",
    scratch: null,
    writeContent: false,
    browser: null,
    width: DEFAULT_WIDTH,
    height: DEFAULT_HEIGHT,
    seed: DEFAULT_SEED,
    camera: DEFAULT_CAMERA,
    shotPrefix: DEFAULT_SHOT_PREFIX,
    ticks: [...DEFAULT_TICKS],
    passes: 1,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    help: false,
  };
  for (let at = 0; at < argv.length; at += 1) {
    const flag = argv[at];
    const value = () => {
      const next = argv[at + 1];
      if (!next || next.startsWith("--")) throw new Error(`${flag} needs a value`);
      at += 1;
      return next;
    };
    if (flag === "--help" || flag === "-h") args.help = true;
    else if (flag === "--scratch") args.scratch = value();
    else if (flag === "--write-content") args.writeContent = true;
    else if (flag === "--browser") args.browser = value();
    else if (flag === "--dir") args.pageDir = value();
    else if (flag === "--width") args.width = positiveInt(flag, value());
    else if (flag === "--height") args.height = positiveInt(flag, value());
    else if (flag === "--seed") args.seed = nonNegativeInt(flag, value());
    else if (flag === "--camera") args.camera = nonEmpty(flag, value());
    else if (flag === "--shot-prefix") args.shotPrefix = nonEmpty(flag, value());
    else if (flag === "--ticks") args.ticks = parseTicks(value());
    else if (flag === "--passes") args.passes = positiveInt(flag, value());
    else if (flag === "--timeout") args.timeoutMs = positiveInt(flag, value());
    else throw new Error(`unknown flag ${flag}`);
  }
  if (args.scratch && args.writeContent) {
    throw new Error("--scratch and --write-content name two different output folders");
  }
  return args;
}

function nonEmpty(flag, raw) {
  if (typeof raw !== "string" || raw.trim().length === 0) throw new Error(`${flag} needs a name`);
  return raw.trim();
}

function positiveInt(flag, raw) {
  const value = integer(flag, raw);
  if (value < 1) throw new Error(`${flag} must be at least 1`);
  return value;
}

function nonNegativeInt(flag, raw) {
  const value = integer(flag, raw);
  if (value < 0) throw new Error(`${flag} must not be negative`);
  return value;
}

function integer(flag, raw) {
  if (!/^\d+$/.test(String(raw))) throw new Error(`${flag} needs a whole number`);
  return Number(raw);
}

/// A comma-separated, strictly ascending tick list. Ten ticks is the tier's
/// reference shape; the verifier rejects a short or long fragment, not this
/// parser, so development can capture one tick without editing code.
export function parseTicks(raw) {
  const parts = String(raw).split(",").map((part) => part.trim());
  if (parts.length === 0 || parts.some((part) => !/^\d+$/.test(part))) {
    throw new Error("--ticks needs whole numbers separated by commas");
  }
  const ticks = parts.map(Number);
  for (let at = 1; at < ticks.length; at += 1) {
    if (ticks[at] <= ticks[at - 1]) throw new Error("--ticks must ascend without repeats");
  }
  return ticks;
}

/// The shot name for one tick: `web-t000`, `web-t060`, ... (at least three digits).
export function shotName(prefix, tick) {
  return `${prefix}-t${String(tick).padStart(3, "0")}`;
}

/// The output set root. A scratch run mirrors `content/reference` under the
/// scratch folder, so its fragment can be verified with the same tool.
export function outputRoot(args) {
  if (args.scratch) return resolve(ROOT, args.scratch);
  if (!args.writeContent) {
    throw new Error("refusing to write content/reference without --write-content (use --scratch to test)");
  }
  return join(ROOT, "content", "reference");
}

/// The page URL. Only the web tier, the deterministic clock and the flat ladder are needed:
/// the seed is filed with the row, and the fps ladder is off so a slow capture frame cannot
/// resize the canvas in the middle of the set; the page's scene is procedural.
export function pageUrl(port, seed = DEFAULT_SEED) {
  return `http://127.0.0.1:${port}/?tier=web&seed=${seed}&ladder=off`;
}

/// The HUD line is `frame 12 | ...`; the frame counter is how the driver tells
/// an arena callback from the page's other animation-frame callbacks.
export function parseHudFrame(text) {
  const match = /^frame\s+(\d+)\b/.exec(String(text ?? "").trim());
  return match ? Number(match[1]) : null;
}

/// Parse a JSON line, treating an empty page placeholder as absent.
export function parseLine(text) {
  const raw = String(text ?? "").trim();
  if (raw.length === 0 || raw === "null") return null;
  try {
    const value = JSON.parse(raw);
    return value && typeof value === "object" ? value : null;
  } catch {
    throw new Error(`page line is not JSON: ${raw.slice(0, 120)}`);
  }
}

/// A name for the adapter fact the page publishes. Description is the most
/// specific; the vendor/architecture fallback keeps a row honest when Chrome
/// hides the description.
export function adapterLabel(report) {
  const adapter = report?.adapter ?? {};
  for (const field of ["description", "name"]) {
    if (typeof adapter[field] === "string" && adapter[field].trim().length > 0) {
      return adapter[field].trim();
    }
  }
  const fallback = [adapter.vendor, adapter.architecture]
    .filter((part) => typeof part === "string" && part.trim().length > 0)
    .join("/");
  return fallback.trim();
}

/// The adapter line from the page's `tier=` text, a fallback when the ready
/// report has no adapter name.
export function tierAdapter(text) {
  const match = /\badapter=([^\s]+)/.exec(String(text ?? ""));
  return match ? match[1] : "";
}

/// The exact command a reviewer can run to reproduce the set. The tool path is
/// relative to the repo when it lives there.
export function commandLine(argv, execPath = process.execPath, toolPath = fileURLToPath(import.meta.url)) {
  const quote = (value) => (/[\s"]/u.test(value) ? `"${value.replaceAll('"', '\\"')}"` : value);
  const inside = relative(ROOT, toolPath);
  const tool = inside.startsWith("..") ? toolPath : inside;
  return [execPath === process.execPath ? "node" : execPath, tool, ...argv].map(quote).join(" ");
}

/// The source string a row carries: browser, adapter and command line.
export function sourceLine(browser, adapter, command) {
  return `browser (${browser}) adapter=${adapter} cmd=${command}`;
}

/// Build the fragment `tools/reference-captures.mjs` verifies. The extra
/// `pixel_fnv`, `frame` and `steps` fields are proof for the reviewer; the
/// merge tool files only the fields the manifest names.
export function buildFragment({
  browser,
  camera,
  seed,
  width,
  height,
  adapter,
  driver,
  command,
  shots,
  passes = 1,
}) {
  return {
    schema: 1,
    tier: TIER,
    camera,
    seed,
    width,
    height,
    adapter,
    driver,
    command,
    source: sourceLine(browser, adapter, command),
    passes,
    shots,
  };
}

/// Verify one captured PNG has the declared canvas and digest. This is the same
/// definition `crates/playtest/src/look/reference.rs` uses on load.
export function verifyPng(png, shot) {
  const decoded = decodePng(png);
  if (decoded.width !== shot.width || decoded.height !== shot.height) {
    throw new Error(
      `${shot.shot}: PNG is ${decoded.width}x${decoded.height}, declared ${shot.width}x${shot.height}`,
    );
  }
  const pixel = fnvHex(decoded.rgba);
  if (pixel === "0000000000000000") throw new Error(`${shot.shot}: zero pixel digest`);
  if (shot.pixel_fnv && pixel !== shot.pixel_fnv) {
    throw new Error(`${shot.shot}: decoded pixels hash ${pixel}, declared ${shot.pixel_fnv}`);
  }
  return { width: decoded.width, height: decoded.height, pixel_fnv: pixel };
}

/// The exact page-clock driver. `runTo` runs one arena frame per tick, while
/// `frozenAt` holds the last timestamp so the capture copy and receipt can
/// publish without moving the camera.
export class ArenaClock {
  constructor(evalJs, frame) {
    if (!Number.isInteger(frame) || frame < 0) throw new Error("ArenaClock needs a frame number");
    this.evalJs = evalJs;
    this.frame = frame;
    this.nextTick = 0;
  }

  async stepAt(ms) {
    const result = await this.evalJs(
      `(() => { const ok = window.__referenceFrame(${ms}); ` +
        `return [ok, document.getElementById("hud")?.textContent ?? ""]; })()`,
    );
    if (!Array.isArray(result) || !result[0]) throw new Error(`no animation frame queued at ${ms} ms`);
    const frame = parseHudFrame(result[1]);
    if (frame === null) throw new Error("page HUD has no frame number");
    const advanced = frame > this.frame;
    this.frame = frame;
    return advanced;
  }

  async arenaStepAt(ms) {
    for (let tries = 0; tries < 8; tries += 1) {
      if (await this.stepAt(ms)) return;
    }
    throw new Error(`no arena frame reached at ${ms} ms`);
  }

  async runTo(tick) {
    if (!Number.isInteger(tick) || tick < this.nextTick) {
      throw new Error(`cannot run from tick ${this.nextTick} to ${tick}`);
    }
    while (this.nextTick < tick) {
      const ms = this.nextTick * FRAME_MS;
      await this.arenaStepAt(ms);
      this.nextTick += 1;
    }
  }

  async target(tick) {
    await this.runTo(tick);
    const ms = tick * FRAME_MS;
    await this.evalJs("window.requestArenaCapture()");
    await this.arenaStepAt(ms);
    this.nextTick = tick + 1;
    return this.frame;
  }

  async frozenAt(tick) {
    await this.arenaStepAt(tick * FRAME_MS);
  }
}

/// Match the page's WebGPU adapter facts to Chromium's own GPU list, so the
/// fragment can name the full card (and its driver) instead of the short
/// vendor/architecture the page exposes. `[]` or no match keeps the page name.
export function gpuDeviceFor(report, devices) {
  const facts = report?.adapter ?? {};
  const needles = [facts.description, facts.name, facts.device, facts.architecture, facts.vendor]
    .filter((part) => typeof part === "string" && part.trim().length > 2)
    .map((part) => part.trim().toLowerCase());
  for (const device of devices ?? []) {
    const text = `${device.deviceString ?? ""} ${device.driverVendor ?? ""}`.toLowerCase();
    if (needles.some((needle) => text.includes(needle))) return device;
  }
  return null;
}

/// Chromium's browser-target GPU list over CDP. A page-target call is refused,
/// and Edge may not expose it; an empty list is fine.
export async function systemGpu(port) {
  const version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  const cdp = await connectCdp(version.webSocketDebuggerUrl);
  try {
    const info = await cdp.send("SystemInfo.getInfo");
    return Array.isArray(info?.gpu?.devices) ? info.gpu.devices : [];
  } finally {
    cdp.close();
  }
}

/// Read the ready report and the browser's own adapter description.
async function pageFacts(evalJs, version, devices) {
  const report = await evalJs(READ_REPORT);
  if (!report || report.ok !== true) throw new PageFatal("page published no ready report");
  if (report.tier !== TIER) throw new PageFatal(`page chose tier ${report.tier}, not ${TIER}`);
  const tierText = await evalJs('document.getElementById("tier")?.textContent ?? ""');
  const pageAdapter = adapterLabel(report) || tierAdapter(tierText);
  const device = gpuDeviceFor(report, devices);
  const adapter = String(device?.deviceString ?? pageAdapter).trim();
  if (adapter.length === 0) throw new PageFatal("page report names no adapter");
  return {
    report,
    adapter,
    driver: String(device?.driverVersion ?? version.product ?? "unknown driver").trim(),
    browser: String(version.product ?? "unknown browser"),
    userAgent: String(version.userAgent ?? ""),
  };
}

async function waitForPage(evalJs, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    let state = null;
    let queued = 0;
    try {
      const value = await evalJs(
        "({ state: document.body?.dataset.state ?? null, queued: window.__referenceQueued ?? 0 })",
      );
      state = value?.state ?? null;
      queued = Number(value?.queued ?? 0);
    } catch {
      // The document is between the first load and a cross-origin-isolation reload.
    }
    if (state === "running" && queued > 0) return;
    if (state === "fatal") {
      let detail = "";
      try {
        detail = await evalJs(
          '`${document.getElementById("fatal")?.innerText ?? ""} ${document.body?.dataset.fatal ?? ""}`',
        );
      } catch {
        // The fatal page is already enough.
      }
      throw new PageFatal(String(detail).trim().slice(0, 500) || "page stopped before its first frame");
    }
    await sleep(100);
  }
  throw new PageFatal(`page did not reach its first frame in ${timeoutMs} ms`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/// Launch one browser and load the page under the clock shim.
async function openPage({ browserPath, flags, pageDir, width, height, seed, timeoutMs }) {
  const server = await serve(pageDir);
  const logFile = join(tmpdir(), `reference-web-${process.pid}-${Date.now()}.log`);
  let browser = null;
  try {
    browser = await launchChrome(browserPath, flags, logFile);
    const { cdp } = browser;
    const exceptions = [];
    cdp.on("Runtime.exceptionThrown", (params) => {
      const text = params?.exceptionDetails?.exception?.description ?? params?.exceptionDetails?.text ?? "";
      if (text) exceptions.push(String(text).slice(0, 300));
    });
    const evalJs = async (expression) => {
      const result = await cdp.send("Runtime.evaluate", {
        expression,
        returnByValue: true,
        awaitPromise: true,
      });
      if (result.exceptionDetails) {
        const detail = result.exceptionDetails.exception?.description ?? result.exceptionDetails.text;
        throw new Error(`page evaluate failed: ${String(detail).slice(0, 300)}`);
      }
      return result.result.value;
    };
    const devices = await systemGpu(browser.port).catch(() => []);
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    const version = await cdp.send("Browser.getVersion");
    await cdp.send("Emulation.setDeviceMetricsOverride", {
      width,
      height,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await cdp.send("Page.addScriptToEvaluateOnNewDocument", { source: CLOCK_SHIM });
    await cdp.send("Page.navigate", { url: pageUrl(server.port, seed) });
    await waitForPage(evalJs, timeoutMs);
    const facts = await pageFacts(evalJs, version, devices);
    if (facts.report.canvas?.width !== width || facts.report.canvas?.height !== height) {
      throw new PageFatal(
        `page canvas is ${facts.report.canvas?.width}x${facts.report.canvas?.height}, wanted ${width}x${height}`,
      );
    }
    return {
      evalJs,
      ...facts,
      exceptions,
      async close() {
        await browser.close();
        await server.close();
      },
    };
  } catch (error) {
    if (browser) await browser.close().catch(() => {});
    await server.close().catch(() => {});
    throw error;
  }
}

async function readReceiptAt(evalJs, clock, tick, targetFrame) {
  for (let tries = 0; tries < 40; tries += 1) {
    const receipt = parseLine(await evalJs('document.getElementById("receipt")?.textContent ?? ""'));
    if (receipt && Number(receipt.frames) >= targetFrame) {
      if (Number(receipt.steps) !== tick) {
        throw new Error(`tick ${tick}: receipt says ${receipt.steps} steps at frame ${receipt.frames}`);
      }
      return receipt;
    }
    await clock.frozenAt(tick);
  }
  throw new Error(`tick ${tick}: receipt never reached frame ${targetFrame}`);
}

async function captureShot({ evalJs, clock, tick, width, height, seed, shot }) {
  const beforeLine = parseLine(await evalJs("window.arenaCapture?.line ?? null"));
  const beforeFrame = Number(beforeLine?.frame ?? -1);
  const before = clock.frame;
  await clock.target(tick);
  const targetFrame = clock.frame;
  if (targetFrame <= before) throw new Error(`${shot}: the target frame did not draw`);

  let line = null;
  for (let tries = 0; tries < CAPTURE_TRIES; tries += 1) {
    await clock.frozenAt(tick);
    line = parseLine(await evalJs("window.arenaCapture?.line ?? null"));
    if (line && Number(line.frame) >= targetFrame && Number(line.frame) > beforeFrame) break;
    // The page keeps an ask until a frame can arm it, but a driver that never sees a line says
    // so again: the clock is frozen at this tick, so another ask cannot move the camera.
    if (tries === 15 || tries === 30) await evalJs("window.requestArenaCapture()");
    // A copy maps on the browser's clock, not the shim's; leave it real time to land.
    await sleep(CAPTURE_PAUSE_MS);
  }
  if (!line || Number(line.frame) < targetFrame || Number(line.frame) <= beforeFrame) {
    const debug = await evalJs(
      `({ hud: document.getElementById("hud")?.textContent ?? "", ` +
        `capture: document.getElementById("capture")?.textContent ?? "", ` +
        `receipt: document.getElementById("receipt")?.textContent ?? "", ` +
        `split: document.getElementById("split")?.textContent ?? "", ` +
        `tier: document.getElementById("tier")?.textContent ?? "", ` +
        `queued: window.__referenceQueued ?? -1, ` +
        `state: window.arenaCaptureState ?? "none", ` +
        `hasCapture: !!window.arenaCapture, ` +
        `hasRequest: typeof window.requestArenaCapture })`,
    );
    throw new Error(`${shot}: no capture line after frame ${targetFrame} (${JSON.stringify(debug)})`);
  }
  if (line.ok !== true) throw new Error(`${shot}: page capture failed (${JSON.stringify(line)})`);
  if (line.width !== width || line.height !== height) {
    throw new Error(`${shot}: capture is ${line.width}x${line.height}, wanted ${width}x${height}`);
  }
  if (line.blank === true) throw new Error(`${shot}: page capture is blank`);

  const packed = await evalJs(READ_RGBA);
  if (typeof packed !== "string" || packed.length === 0) throw new Error(`${shot}: page has no pixels`);
  const rgba = Buffer.from(packed, "base64");
  if (rgba.length !== width * height * 4) {
    throw new Error(`${shot}: readback is ${rgba.length} bytes, wanted ${width * height * 4}`);
  }
  const pixel = fnvHex(rgba);
  if (pixel === "0000000000000000") throw new Error(`${shot}: readback has a zero pixel digest`);
  if (pixel !== line.hash) throw new Error(`${shot}: readback hashes ${pixel}, page says ${line.hash}`);

  const png = encodePng(width, height, rgba);
  const verified = verifyPng(png, { shot, width, height, pixel_fnv: pixel });
  const pngFnv = fnvHex(png);
  if (pngFnv === "0000000000000000") throw new Error(`${shot}: PNG has a zero byte digest`);
  const receipt = await readReceiptAt(evalJs, clock, tick, targetFrame);

  return {
    shot,
    png: `${TIER}/${shot}.png`,
    tick,
    width,
    height,
    seed,
    pixel_fnv: verified.pixel_fnv,
    png_fnv: pngFnv,
    bytes: png.length,
    frame: targetFrame,
    steps: Number(receipt.steps),
    pngBytes: png,
  };
}

async function capturePass({ page, ticks, width, height, seed, shotPrefix }) {
  const hud = await page.evalJs('document.getElementById("hud")?.textContent ?? ""');
  const frame = parseHudFrame(hud);
  if (frame === null) throw new Error("page HUD has no starting frame");
  const clock = new ArenaClock(page.evalJs, frame);
  const shots = [];
  for (const tick of ticks) {
    const name = shotName(shotPrefix, tick);
    shots.push(
      await captureShot({
        evalJs: page.evalJs,
        clock,
        tick,
        width,
        height,
        seed,
        shot: name,
      }),
    );
  }
  return shots;
}

function comparePasses(first, second) {
  if (first.shots.length !== second.shots.length) throw new Error("the two passes captured different shot counts");
  for (let at = 0; at < first.shots.length; at += 1) {
    const left = first.shots[at];
    const right = second.shots[at];
    if (left.tick !== right.tick || left.shot !== right.shot) {
      throw new Error(`pass 2 shot ${right.shot} does not match ${left.shot}`);
    }
    if (left.pixel_fnv !== right.pixel_fnv || !left.pngBytes.equals(right.pngBytes)) {
      throw new Error(
        `tick ${left.tick}: pass 2 differs (` +
          `pixel ${left.pixel_fnv}/${right.pixel_fnv}, bytes ${left.bytes}/${right.bytes})`,
      );
    }
  }
}

/// The installed Chromium browser. Chrome is preferred; Edge is the fallback.
export function findBrowser(explicit = null) {
  if (explicit) {
    const path = resolve(ROOT, explicit);
    if (!existsSync(path)) throw new Error(`no browser at ${path}`);
    return path;
  }
  const roots = [
    process.env["PROGRAMFILES"],
    process.env["PROGRAMFILES(X86)"],
    process.env["LOCALAPPDATA"],
  ].filter(Boolean);
  const candidates = [
    ...roots.map((root) => join(root, "Google", "Chrome", "Application", "chrome.exe")),
    ...roots.map((root) => join(root, "Microsoft", "Edge", "Application", "msedge.exe")),
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  ];
  for (const path of candidates) if (existsSync(path)) return path;
  throw new Error("no Chrome or Edge found; pass --browser <path>");
}

const BASE_FLAGS = [
  "--headless=new",
  "--no-sandbox",
  "--disable-dev-shm-usage",
  "--no-first-run",
  "--no-default-browser-check",
  "--hide-scrollbars",
  "--mute-audio",
];

/// Browser flag sets, in the order they are tried: the real GPU first, the
/// software adapter second. A page with no adapter fails fast and moves on.
export function browserFlagSets(width, height) {
  const size = `--window-size=${width},${height}`;
  return [
    { name: "hardware", flags: [...BASE_FLAGS, size, "--enable-unsafe-webgpu"] },
    {
      name: "swiftshader",
      flags: [...BASE_FLAGS, size, "--enable-unsafe-webgpu", "--use-webgpu-adapter=swiftshader"],
    },
  ];
}

async function runPass({ args, browserPath, flagSet, pass }) {
  const page = await openPage({
    browserPath,
    flags: flagSet.flags,
    pageDir: resolve(ROOT, args.pageDir),
    width: args.width,
    height: args.height,
    seed: args.seed,
    timeoutMs: args.timeoutMs,
  });
  try {
    console.log(`reference-web: pass ${pass} loaded ${flagSet.name}, adapter=${page.adapter}`);
    const shots = await capturePass({
      page,
      ticks: args.ticks,
      width: args.width,
      height: args.height,
      seed: args.seed,
      shotPrefix: args.shotPrefix,
    });
    return {
      shots,
      adapter: page.adapter,
      driver: page.driver,
      browser: page.browser,
      exceptions: page.exceptions,
      report: page.report,
    };
  } finally {
    await page.close();
  }
}

async function captureAll(args, browserPath, flagsets) {
  let chosen = null;
  let first = null;
  for (let pass = 1; pass <= args.passes; pass += 1) {
    const candidates = chosen ? [chosen] : flagsets;
    let result = null;
    let last = null;
    for (const flagSet of candidates) {
      try {
        result = await runPass({ args, browserPath, flagSet, pass });
        chosen = flagSet;
        break;
      } catch (error) {
        last = error;
        if (!(error instanceof PageFatal)) throw error;
        console.error(`reference-web: ${flagSet.name} page failed: ${error.message}`);
      }
    }
    if (!result) throw last ?? new Error("no browser flags worked");
    if (first === null) first = result;
    else {
      comparePasses(first, result);
      console.log(`reference-web: pass ${pass} equals pass 1 on every shot`);
    }
  }
  return first;
}

function usage() {
  return [
    "node tools/reference-web.mjs --scratch <dir> [options]",
    "node tools/reference-web.mjs --write-content [options]",
    "",
    "Captures the web tier's ten real 640x360 shots from dist/web in headless",
    "Chrome over CDP. Each shot is one camera-path tick (0..540 by 60).",
    "",
    "  --scratch <dir>      write <dir>/web/*.png and <dir>/web/fragment.json",
    "  --write-content      write content/reference/web (needed outside scratch)",
    "  --passes <n>         capture n times and require equal bytes (default 1)",
    "  --ticks <a,b,...>    ticks to capture (default 0,60,...,540)",
    "  --dir <dir>          built page folder (default dist/web)",
    "  --browser <path>     Chrome or Edge executable",
    "  --camera <name>      camera name filed with the rows (page camera: procedural-arena)",
    "  --shot-prefix <name> shot name prefix (default web)",
    "  --width <px>         canvas width (default 640)",
    "  --height <px>        canvas height (default 360)",
    "  --seed <n>           seed filed with the rows (default 2040)",
    "  --timeout <ms>       page load timeout (default 90000)",
    "",
    "The page owns the camera. Its procedural orbit camera is filed as",
    "procedural-arena; pass --camera only when the page really provides that",
    "camera. A full tier needs all ten ticks; the merge verifier checks it.",
  ].join("\n");
}

async function main(argv) {
  let args;
  try {
    args = parseArgs(argv);
  } catch (error) {
    console.error(`reference-web: ${error.message}`);
    console.error(usage());
    return 2;
  }
  if (args.help) {
    console.log(usage());
    return 0;
  }
  const pageDir = resolve(ROOT, args.pageDir);
  if (!existsSync(join(pageDir, "index.html"))) {
    console.error(`reference-web: no built page at ${pageDir}`);
    return 2;
  }
  let root;
  try {
    root = outputRoot(args);
    const browserPath = findBrowser(args.browser);
    const flagsets = browserFlagSets(args.width, args.height);
    const command = commandLine(argv);
    const first = await captureAll(args, browserPath, flagsets);
    const tierDir = join(root, TIER);
    mkdirSync(tierDir, { recursive: true });
    const shots = first.shots.map(({ pngBytes, ...shot }) => shot);
    const fragment = buildFragment({
      browser: first.browser,
      camera: args.camera,
      seed: args.seed,
      width: args.width,
      height: args.height,
      adapter: first.adapter,
      driver: first.driver,
      command,
      shots,
      passes: args.passes,
    });
    for (const shot of first.shots) {
      writeFileSync(join(tierDir, `${shot.shot}.png`), shot.pngBytes);
      console.log(
        `reference ${TIER}/${shot.shot} pixel_fnv=${shot.pixel_fnv} png_fnv=${shot.png_fnv} ` +
          `bytes=${shot.bytes} tick=${shot.tick} frame=${shot.frame} steps=${shot.steps}`,
      );
    }
    writeFileSync(join(tierDir, "fragment.json"), `${JSON.stringify(fragment, null, 2)}\n`);
    console.log(
      `reference-web: tier=${TIER} shots=${shots.length} camera=${args.camera} ` +
        `adapter=${first.adapter} driver=${first.driver} browser=${first.browser} ` +
        `passes=${args.passes} out=${relative(ROOT, tierDir)}`,
    );
    return 0;
  } catch (error) {
    console.error(`reference-web: ${error.message}`);
    return 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
}

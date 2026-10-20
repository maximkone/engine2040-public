// KAM8: build the kam2-web wasm module, serve target/kam2web and prove the page
// draws the KAM2 village in a headless Chrome.
//   node tools/kam2-web.mjs [--frames 120] [--chrome <path>] [--no-build] [--out target/kam2web.png]
// Steps: cargo builds `kam2-web` for wasm32-unknown-unknown with the size profile,
// wasm-bindgen writes kam2web.js plus kam2web_bg.wasm, the page, the two atlases and
// the six cameos (copied out of content/kam2/, never read from there at runtime) are
// copied beside them, a loopback static server serves the folder and Chrome (CDP)
// loads `?frames=N`. The page stops itself at exactly N frames, so the screenshot is
// a frozen, deterministic frame of the running match.
// Then the two keys of the milestone are pressed for real: Space holds the match
// (the tick stops) and releases it, an arrow pans the view.
// KAM11 adds the sidebar drill on a live page: cell 2 arms the farm, a click on
// ground the sim's own `placement` accepts raises the house count with an empty
// order line, a click on a road tile prints the sim's refusal, and the page with
// the sidebar is saved as target/kam2ui.png.
// Exit 0 and `KAM8 OK frames=<n> png=<path>` plus
// `KAM11 OK houses=<n> cells=5 placed=<kind> refused=<text> png=<path> bytes=<n>`
// when every proof passes and no page exception was thrown; 1 a proof failed,
// 2 could not run (no Chrome, no build).
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { BASE_FLAGS, analyzePixels, decodePng, isDrawn, launchChrome, serve } from "./web-browse.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(root, "target", "kam2web");
const DEFAULT_PNG = join(root, "target", "kam2web.png");
const UI_PNG = join(root, "target", "kam2ui.png");
const CHROME_LOG = join(root, "target", "kam2web-chrome.log");
const BUILT_WASM = join(root, "target", "wasm32-unknown-unknown", "web", "kam2_web.wasm");
// The six committed cameos, 96x72 each, served under cameos/<id>.png.
const CAMEOS = {
  cottage: "content/kam2/house/house.cottage.cameo.png",
  warehouse: "content/kam2/buildings/warehouse/house.warehouse.cameo.png",
  farm: "content/kam2/buildings/farm/house.farm.cameo.png",
  mill: "content/kam2/buildings/mill/house.mill.cameo.png",
  bakery: "content/kam2/buildings/bakery/house.bakery.cameo.png",
  woodcutter: "content/kam2/buildings/woodcutter/house.woodcutter.cameo.png",
};

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tail = (text, n = 30) => String(text).trimEnd().split(/\r?\n/).slice(-n).join("\n");

function run(cmd, args) {
  const r = spawnSync(cmd, args, { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (r.error) throw new Error(`${cmd} could not start: ${r.error.message}`);
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed (exit ${r.status})\n${tail(`${r.stdout ?? ""}${r.stderr ?? ""}`)}`);
  return r.stdout ?? "";
}

function findChrome() {
  if (process.env.KAM8_CHROME) return process.env.KAM8_CHROME;
  const candidates = [
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
  ];
  return candidates.find((path) => existsSync(path)) ?? "google-chrome";
}

function copy(from, to) {
  const source = join(root, from);
  if (!existsSync(source)) throw new Error(`${from} is missing`);
  cpSync(source, join(OUT_DIR, to));
}

function build() {
  console.log("[kam2-web] cargo build --profile web --target wasm32-unknown-unknown -p kam2-web");
  run("cargo", ["build", "--profile", "web", "--target", "wasm32-unknown-unknown", "-p", "kam2-web"]);
  if (!existsSync(BUILT_WASM)) throw new Error(`${BUILT_WASM} was not written`);
  rmSync(OUT_DIR, { recursive: true, force: true });
  mkdirSync(join(OUT_DIR, "soldier"), { recursive: true });
  mkdirSync(join(OUT_DIR, "house"), { recursive: true });
  mkdirSync(join(OUT_DIR, "cameos"), { recursive: true });
  console.log("[kam2-web] wasm-bindgen --target web");
  run("wasm-bindgen", [BUILT_WASM, "--target", "web", "--no-typescript", "--out-dir", OUT_DIR, "--out-name", "kam2web"]);
  copy("crates/kam2-web/page/index.html", "index.html");
  copy("crates/kam2-web/page/app.js", "app.js");
  copy("content/kam2/soldier/manifest.json", "soldier/manifest.json");
  copy("content/kam2/soldier/unit.soldier.atlas.png", "soldier/unit.soldier.atlas.png");
  copy("content/kam2/house/manifest.json", "house/manifest.json");
  copy("content/kam2/house/house.cottage.atlas.png", "house/house.cottage.atlas.png");
  for (const [id, from] of Object.entries(CAMEOS)) copy(from, `cameos/${id}.png`);
}

async function evaluate(cdp, expression) {
  const result = await cdp.send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
  if (result.exceptionDetails) {
    throw new Error(`evaluate: ${result.exceptionDetails.text} ${result.exceptionDetails.exception?.description ?? ""}`.trim());
  }
  return result.result.value;
}

const keyEvent = (type, key, code, vk, text) => ({ type, key, code, windowsVirtualKeyCode: vk, ...(text ? { text } : {}) });

async function press(cdp, key, code, vk, text) {
  await cdp.send("Input.dispatchKeyEvent", keyEvent("keyDown", key, code, vk, text));
  await cdp.send("Input.dispatchKeyEvent", keyEvent("keyUp", key, code, vk));
}

async function moveMouse(cdp, x, y) {
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
}

async function clickAt(cdp, x, y) {
  await moveMouse(cdp, x, y);
  await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1 });
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", buttons: 0, clickCount: 1 });
}

async function waitFor(cdp, expression, ok, timeoutMs, label) {
  const start = Date.now();
  let last = null;
  while (Date.now() - start < timeoutMs) {
    await sleep(200);
    try {
      last = await evaluate(cdp, expression);
    } catch {
      continue;
    }
    if (ok(last)) return last;
  }
  throw new Error(`${label} within ${timeoutMs} ms (last ${JSON.stringify(last)})`);
}

function parseArgs(argv) {
  const options = { frames: 120, chrome: null, build: true, out: DEFAULT_PNG };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--frames") options.frames = Number(argv[++i]);
    else if (arg === "--chrome") options.chrome = argv[++i];
    else if (arg === "--no-build") options.build = false;
    else if (arg === "--out") options.out = resolve(argv[++i] ?? DEFAULT_PNG);
    else if (arg === "--help") {
      console.log("usage: node tools/kam2-web.mjs [--frames 120] [--chrome <path>] [--no-build] [--out target/kam2web.png]");
      process.exit(0);
    } else throw new Error(`unknown argument ${arg}`);
  }
  if (!Number.isInteger(options.frames) || options.frames < 1) throw new Error("--frames needs a whole number of 1 or more");
  return options;
}

async function prove(options) {
  const chrome = options.chrome ?? findChrome();
  if (options.build) build();
  if (!existsSync(join(OUT_DIR, "kam2web_bg.wasm"))) throw new Error(`${OUT_DIR} holds no build; drop --no-build`);

  const server = await serve(OUT_DIR);
  const browser = await launchChrome(chrome, BASE_FLAGS, CHROME_LOG);
  try {
    const { cdp } = browser;
    const exceptions = [];
    cdp.on("Runtime.exceptionThrown", (p) => exceptions.push(String(p.exceptionDetails?.exception?.description ?? p.exceptionDetails?.text ?? "exception").slice(0, 400)));
    await cdp.send("Page.enable");
    await cdp.send("Runtime.enable");
    await cdp.send("Log.enable");
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });

    // 1. The picture: exactly `frames` animation frames, then the page freezes itself.
    const url = `http://127.0.0.1:${server.port}/index.html?frames=${options.frames}`;
    await cdp.send("Page.navigate", { url });
    const state = await waitFor(cdp, "[document.body.dataset.state ?? null, document.getElementById('report')?.textContent ?? '']", (v) => v && (v[0] === "frozen" || v[0] === "fatal"), 120000, "the page never reached its frame budget");
    if (state[0] === "fatal") throw new Error(`the page failed: ${state[1]}`);
    const frames = await evaluate(cdp, "window.kam2Frames");
    if (frames !== options.frames) throw new Error(`the page ran ${frames} frames, asked ${options.frames}`);
    const report = JSON.parse(state[1]);
    if (report.ok !== true) throw new Error(`the report is not ok: ${state[1]}`);
    if (!/^[0-9a-f]{16}$/.test(report.hash)) throw new Error(`the report hash is not 16 hex digits: ${report.hash}`);
    if (!(report.tick > 0)) throw new Error(`the wasm module played no tick in ${frames} frames`);

    const shot = Buffer.from((await cdp.send("Page.captureScreenshot", { format: "png" })).data, "base64");
    writeFileSync(options.out, shot);
    const stats = analyzePixels(decodePng(shot));
    if (!isDrawn(stats)) throw new Error(`the screenshot is blank (${JSON.stringify(stats)})`);

    // 2. The keys, on a live page: Space holds and releases, an arrow pans.
    await cdp.send("Page.navigate", { url: `http://127.0.0.1:${server.port}/index.html` });
    await waitFor(cdp, "window.kam2 ? window.kam2.frames() : -1", (n) => n >= 10, 120000, "the live page never drew 10 frames");
    await press(cdp, " ", "Space", 32, " ");
    await sleep(300);
    const held = await evaluate(cdp, "window.kam2.tick()");
    await sleep(600);
    const stillHeld = await evaluate(cdp, "window.kam2.tick()");
    if (stillHeld !== held) throw new Error(`Space did not hold the match: tick ${held} then ${stillHeld}`);
    const paused = await evaluate(cdp, "window.kam2.paused()");
    if (paused !== true) throw new Error("the page says it is not paused after Space");
    await press(cdp, " ", "Space", 32, " ");
    await sleep(600);
    const resumed = await evaluate(cdp, "window.kam2.tick()");
    if (resumed <= stillHeld) throw new Error(`Space did not release the match: tick ${stillHeld} then ${resumed}`);
    const panBefore = await evaluate(cdp, "window.kam2.pan()");
    await cdp.send("Input.dispatchKeyEvent", keyEvent("rawKeyDown", "ArrowLeft", "ArrowLeft", 37));
    await sleep(400);
    await cdp.send("Input.dispatchKeyEvent", keyEvent("keyUp", "ArrowLeft", "ArrowLeft", 37));
    const panAfter = await evaluate(cdp, "window.kam2.pan()");
    if (!(panAfter[0] > panBefore[0])) throw new Error(`ArrowLeft did not pan: ${JSON.stringify(panBefore)} then ${JSON.stringify(panAfter)}`);

    // 3. The build sidebar (KAM11): 120 frames have been drawn by now. Hold the
    // match so only the page's own click can move the house count.
    await waitFor(cdp, "window.kam2.frames()", (n) => n >= 120, 120000, "the live page never drew 120 frames");
    await press(cdp, " ", "Space", 32, " ");
    await sleep(300);
    if ((await evaluate(cdp, "window.kam2.paused()")) !== true) throw new Error("the match did not hold before the build drill");
    const cells = await evaluate(cdp, "document.querySelectorAll('#build-grid .cell').length");
    if (cells !== 5) throw new Error(`the sidebar holds ${cells} build cells, not 5`);
    await waitFor(cdp, "Array.from(document.querySelectorAll('#build-grid img')).map((i) => [i.naturalWidth, i.naturalHeight])", (sizes) => Array.isArray(sizes) && sizes.length === 5 && sizes.every(([w, h]) => w === 96 && h === 72), 30000, "the five 96x72 cameos never loaded");
    const sidebar = await evaluate(cdp, "(() => { const s = document.getElementById('sidebar'); return { width: getComputedStyle(s).width, background: getComputedStyle(s).backgroundColor }; })()");
    if (sidebar.width !== "176px") throw new Error(`the sidebar is ${sidebar.width} wide, not 176px`);
    const before = await evaluate(cdp, "window.kam2.houses()");
    const site = await evaluate(cdp, `(() => {
      for (let z = 0; z < 12; z += 1) for (let x = 0; x < 16; x += 1) {
        if (window.kam2.placement("farm", x, z) !== "") continue;
        const p = window.kam2.pixelOf(x, z);
        if (p && p.x > 30 && p.x < 1090 && p.y > 30 && p.y < 660) return { x, z, px: p.x, py: p.y };
      }
      return null;
    })()`);
    if (!site) throw new Error("the sim accepts no farm tile anywhere on the field");
    const cell = await evaluate(cdp, "(() => { const r = document.querySelectorAll('#build-grid .cell')[1].getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) }; })()");
    await clickAt(cdp, cell.x, cell.y);
    await sleep(250);
    const armed = await evaluate(cdp, "window.kam2.armed()");
    if (armed !== "farm") throw new Error(`cell 2 armed ${JSON.stringify(armed)}, not farm`);
    await clickAt(cdp, site.px, site.py);
    await sleep(400);
    const placedLine = await evaluate(cdp, "window.kam2.order()");
    if (placedLine !== "") throw new Error(`the farm at (${site.x},${site.z}) was refused: ${placedLine}`);
    const after = await evaluate(cdp, "window.kam2.houses()");
    if (!(after > before)) throw new Error(`the house count did not rise: ${before} then ${after} after the farm`);

    // The refusal: the same armed farm over the village road.
    const road = await evaluate(cdp, `(() => {
      for (let x = 0; x < 16; x += 1) { const p = window.kam2.pixelOf(x, 6); if (p && p.x > 30 && p.x < 1090) return { x, z: 6, px: p.x, py: p.y }; }
      return null;
    })()`);
    if (!road) throw new Error("no road tile of row 6 is on screen");
    const refusal = await evaluate(cdp, `window.kam2.placement("farm", ${road.x}, ${road.z})`);
    if (!refusal) throw new Error(`the road tile (${road.x},6) is not refused`);
    await moveMouse(cdp, road.px, road.py);
    await sleep(250);
    await clickAt(cdp, road.px, road.py);
    await sleep(400);
    const shown = await evaluate(cdp, "document.getElementById('order-line').textContent");
    if (!shown) throw new Error("the refused build printed no words");
    if (shown !== refusal) throw new Error(`the order line says "${shown}", the sim says "${refusal}"`);
    const afterRefusal = await evaluate(cdp, "window.kam2.houses()");
    if (afterRefusal !== after) throw new Error(`the refused build moved the count: ${after} then ${afterRefusal}`);

    const uiShot = Buffer.from((await cdp.send("Page.captureScreenshot", { format: "png" })).data, "base64");
    writeFileSync(UI_PNG, uiShot);
    const uiBytes = statSync(UI_PNG).size;
    const uiStats = analyzePixels(decodePng(uiShot));
    if (!isDrawn(uiStats)) throw new Error(`the sidebar screenshot is blank (${JSON.stringify(uiStats)})`);
    console.log(`KAM11 OK houses=${after} cells=${cells} placed=${armed} refused=${shown} png=${UI_PNG} bytes=${uiBytes}`);

    const finalState = await evaluate(cdp, "[document.body.dataset.state, document.getElementById('fatal').innerText]");
    if (finalState[0] === "fatal") throw new Error(`the page went fatal during the key drill: ${finalState[1]}`);
    if (exceptions.length) throw new Error(`uncaught page exceptions: ${exceptions.join(" | ")}`);

    const bytes = statSync(options.out).size;
    console.log(`KAM8 receipt frames=${frames} tick=${report.tick} hash=${report.hash} houses=${report.houses} units=${report.units} webgpu=${report.webgpu} bytes=${bytes}`);
    console.log(`KAM8 OK frames=${frames} png=${options.out}`);
    return { frames, report, bytes };
  } finally {
    await browser.close();
    await server.close();
  }
}

const options = (() => {
  try {
    return parseArgs(process.argv.slice(2));
  } catch (error) {
    console.error(`kam2-web: ${error.message}`);
    process.exit(2);
  }
})();

try {
  await prove(options);
} catch (error) {
  console.error(`KAM8 FAIL ${error.message}`);
  process.exit(1);
}

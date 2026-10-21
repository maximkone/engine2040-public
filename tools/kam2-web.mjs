// KAM8: build the kam2-web wasm module, serve target/kam2web and prove the page
// draws the KAM2 village in a headless Chrome.
//   node tools/kam2-web.mjs [--frames 120] [--chrome <path>] [--no-build] [--out target/kam2web.png] [--wasm <path>]
// The wasm may come from elsewhere: KAM2_WASM names the module to bind and
// KAM2_NO_CARGO=1 leaves the cargo build out, so a seat can prove the page
// against a wasm it built in its own target directory.
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
// KAM13 adds the map drill on the same page: the farm is disarmed, a click on a
// side-0 soldier picks it, the panel names it, the panel is 160x72, the three
// command buttons are 48x48, the minimap is 160x120 with the window the camera
// looks at, and the page is saved as target/kam2ui3.png.
// KAM14 adds the pointer-contract drill on a page that froze after its frame
// budget, so the field stands still while the mouse alone gives every order:
// the farm is placed and the armed cell is cancelled with the right button, a
// swordsman is picked, the right button sends it walking, the stop button
// stands it still, a foe under the cursor gets a crosshair and is struck
// without moving the selection, the right button on the foe's own ground
// prints the sim's refusal, shift adds a second soldier, a click on empty
// ground clears the selection, a house is picked like a unit, a click on the
// minimap moves the camera and the rectangle on it follows by exactly what the
// camera moved, and the page is saved as target/kam2ui4.png.
// Exit 0 and `KAM8 OK frames=<n> png=<path>` plus
// `KAM11 OK houses=<n> cells=5 placed=<kind> refused=<text> png=<path> bytes=<n>`
// and
// `KAM13 OK selected=<kind> panel=<w>x<h> buttons=<n> minimap=<w>x<h> rect=<x>,<y>,<w>,<h> tests=<n> png=<path> bytes=<n>`
// plus `KAM13 SOURCES selection=crate minimap=crate commands=crate`
// and
// `KAM14 OK farm=<x>,<z> soldier=<id> walk=<x>,<z> foe=<id> refused=<text> picked=<n> house=<id> road=<text> pan=<x>,<y> window=<x>,<y>,<w>,<h> png=<path> bytes=<n>`
// when every proof passes and no page exception was thrown; 1 a proof failed,
// 2 could not run (no Chrome, no build).
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { BASE_FLAGS, analyzePixels, decodePng, isDrawn, launchChrome, serve } from "./web-browse.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT_DIR = join(root, "target", "kam2web");
const DEFAULT_PNG = join(root, "target", "kam2web.png");
const UI_PNG = join(root, "target", "kam2ui.png");
const UI3_PNG = join(root, "target", "kam2ui3.png");
const UI4_PNG = join(root, "target", "kam2ui4.png");
const CHROME_LOG = join(root, "target", "kam2web-chrome.log");
// The wasm the tool binds. KAM2_WASM points the tool at a build made in
// another target directory, and KAM2_NO_CARGO=1 leaves the cargo build out
// when that build is already there: the proof must not depend on the shared
// target directory's lock.
const BUILT_WASM = process.env.KAM2_WASM
  ? resolve(process.env.KAM2_WASM)
  : join(root, "target", "wasm32-unknown-unknown", "web", "kam2_web.wasm");
const NO_CARGO = process.env.KAM2_NO_CARGO === "1";
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

// The number of tests the crate carries: every `#[test]` of its own sources.
function crateTests() {
  const dir = join(root, "crates", "kam2-web", "src");
  let count = 0;
  for (const name of readdirSync(dir)) {
    if (!name.endsWith(".rs")) continue;
    const text = readFileSync(join(dir, name), "utf8");
    count += (text.match(/#\[test\]/g) ?? []).length;
  }
  return count;
}

// Where the three parts of the sidebar live: the panel, the minimap and the
// commands are the crate's own code, never the page's.
function sourcesLine() {
  const parts = [
    ["selection", "crates/kam2-web/src/selection.rs", "pub fn select_at"],
    ["minimap", "crates/kam2-web/src/minimap.rs", "pub fn minimap_json"],
    ["commands", "crates/kam2-web/src/selection.rs", "pub enum Order"],
  ];
  const found = [];
  for (const [name, file, anchor] of parts) {
    const text = readFileSync(join(root, file), "utf8");
    if (!text.includes(anchor)) throw new Error(`the ${name} of the sidebar is not in ${file} (${anchor})`);
    found.push(`${name}=crate`);
  }
  return `KAM13 SOURCES ${found.join(" ")}`;
}

function build(wasm) {
  if (NO_CARGO) {
    console.log(`[kam2-web] KAM2_NO_CARGO=1: binding the wasm at ${wasm}`);
  } else {
    console.log("[kam2-web] cargo build --profile web --target wasm32-unknown-unknown -p kam2-web");
    run("cargo", ["build", "--profile", "web", "--target", "wasm32-unknown-unknown", "-p", "kam2-web"]);
  }
  if (!existsSync(wasm)) throw new Error(`${wasm} was not written`);
  rmSync(OUT_DIR, { recursive: true, force: true });
  mkdirSync(join(OUT_DIR, "soldier"), { recursive: true });
  mkdirSync(join(OUT_DIR, "house"), { recursive: true });
  mkdirSync(join(OUT_DIR, "cameos"), { recursive: true });
  console.log("[kam2-web] wasm-bindgen --target web");
  run("wasm-bindgen", [wasm, "--target", "web", "--no-typescript", "--out-dir", OUT_DIR, "--out-name", "kam2web"]);
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

// The right button: Chrome answers it with the contextmenu the page listens
// for, so the page's own order path runs on a real pointer event.
async function rightClickAt(cdp, x, y) {
  await moveMouse(cdp, x, y);
  await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "right", buttons: 2, clickCount: 1 });
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "right", buttons: 0, clickCount: 1 });
}

// Shift and the left button: the page's shift adds to the selection. The
// modifier bit of the protocol is 8 for shift.
async function shiftClickAt(cdp, x, y) {
  await moveMouse(cdp, x, y);
  await cdp.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", buttons: 1, clickCount: 1, modifiers: 8 });
  await cdp.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", buttons: 0, clickCount: 1, modifiers: 8 });
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
  const options = { frames: 120, chrome: null, build: true, out: DEFAULT_PNG, wasm: null };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--frames") options.frames = Number(argv[++i]);
    else if (arg === "--chrome") options.chrome = argv[++i];
    else if (arg === "--no-build") options.build = false;
    else if (arg === "--out") options.out = resolve(argv[++i] ?? DEFAULT_PNG);
    else if (arg === "--wasm") options.wasm = resolve(argv[++i] ?? BUILT_WASM);
    else if (arg === "--help") {
      console.log("usage: node tools/kam2-web.mjs [--frames 120] [--chrome <path>] [--no-build] [--out target/kam2web.png] [--wasm <path>]");
      process.exit(0);
    } else throw new Error(`unknown argument ${arg}`);
  }
  if (!Number.isInteger(options.frames) || options.frames < 1) throw new Error("--frames needs a whole number of 1 or more");
  return options;
}

async function prove(options) {
  const chrome = options.chrome ?? findChrome();
  if (options.build) build(options.wasm ?? BUILT_WASM);
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
    const state = await waitFor(cdp, "[document.body.dataset.state ?? null, document.getElementById('report')?.textContent ?? '', document.getElementById('fatal')?.innerText ?? '']", (v) => v && (v[0] === "frozen" || v[0] === "fatal"), 120000, "the page never reached its frame budget");
    if (state[0] === "fatal") throw new Error(`the page failed: ${state[2] || state[1]}`);
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
    await waitFor(cdp, "Array.from(document.querySelectorAll('#build-grid img')).map((i) => [i.naturalWidth, i.naturalHeight])", (sizes) => Array.isArray(sizes) && sizes.length === 5 && sizes.every(([w, h]) => w > 0 && h > 0), 30000, "the five cameos never loaded");
    const cellSize = await evaluate(cdp, "(() => { const i = document.querySelector('#build-grid img'); return [i.clientWidth, i.clientHeight]; })()");
    if (cellSize[0] !== 64 || cellSize[1] !== 48) throw new Error(`a cameo draws ${cellSize.join("x")}, not 64x48`);
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

    // The refusal: the same armed farm over the village road. The first road tile
    // the sim refuses as unbuildable ground is the one the page tries, so a unit
    // that happens to stand on the road cannot stand in for the road rule.
    const road = await evaluate(cdp, `(() => {
      for (let x = 0; x < 16; x += 1) {
        const text = window.kam2.placement("farm", x, 6);
        if (!text.startsWith("NotBuildable")) continue;
        const p = window.kam2.pixelOf(x, 6);
        if (p && p.x > 30 && p.x < 1090) return { x, z: 6, px: p.x, py: p.y, text };
      }
      return null;
    })()`);
    if (!road) throw new Error("no road tile of row 6 is refused as unbuildable ground");
    const refusal = road.text;
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

    // 4. The KAM13 sidebar: the minimap, the selection panel and the three
    // command buttons, on the same live page. The farm is disarmed first, so
    // the click below picks a soldier instead of placing a house.
    await press(cdp, "Escape", "Escape", 27);
    await sleep(250);
    const armedAfter = await evaluate(cdp, "window.kam2.armed()");
    if (armedAfter !== null) throw new Error(`Escape left the ${armedAfter} cell armed`);

    const soldier = await evaluate(cdp, `(() => {
      const map = window.kam2.map();
      const unit = map.units.find((u) => u.side === 0);
      if (!unit) return null;
      const pixel = window.kam2.pixelOf(unit.x, unit.z);
      if (!pixel) return null;
      const id = window.kam2.pick(unit.x, unit.z);
      const state = id ? window.kam2.unit(id) : null;
      return { x: unit.x, z: unit.z, px: pixel.x, py: pixel.y, id, name: state ? state.name : "" };
    })()`);
    if (!soldier || !soldier.id) throw new Error("the page found no side-0 soldier to pick");
    await moveMouse(cdp, soldier.px, soldier.py);
    await sleep(250);
    await clickAt(cdp, soldier.px, soldier.py);
    await sleep(400);
    const panelName = await evaluate(cdp, "document.getElementById('sel-name').textContent");
    if (panelName !== soldier.name) {
      throw new Error(`the panel says "${panelName}", the module says "${soldier.name}"`);
    }
    const selection = await evaluate(cdp, "JSON.stringify(window.kam2.selection())");
    if (!selection.includes(soldier.id)) {
      throw new Error(`the selection holds ${selection}, not ${soldier.id}`);
    }

    // The panel and the buttons are border-box, so the box the plan names is
    // the border box; the minimap is a canvas, so its surface is the size.
    const panelSize = await evaluate(cdp, "(() => { const p = document.getElementById('selection-panel'); return [p.offsetWidth, p.offsetHeight]; })()");
    if (panelSize[0] !== 160 || panelSize[1] !== 72) {
      throw new Error(`the selection panel is ${panelSize.join("x")}, not 160x72`);
    }
    const commands = await evaluate(cdp, "Array.from(document.querySelectorAll('#command-row .command')).map((b) => [b.offsetWidth, b.offsetHeight])");
    if (commands.length !== 3) throw new Error(`the command row holds ${commands.length} buttons, not 3`);
    if (!commands.every(([w, h]) => w === 48 && h === 48)) {
      throw new Error(`a command button is ${JSON.stringify(commands)}, not 48x48`);
    }
    const mapSize = await evaluate(cdp, "(() => { const m = document.getElementById('minimap'); return [m.clientWidth, m.clientHeight]; })()");
    if (mapSize[0] !== 160 || mapSize[1] !== 120) {
      throw new Error(`the minimap is ${mapSize.join("x")}, not 160x120`);
    }
    const mapBuffer = await evaluate(cdp, "(() => { const m = document.getElementById('minimap'); return [m.width, m.height]; })()");
    if (mapBuffer[0] !== 160 || mapBuffer[1] !== 120) {
      throw new Error(`the minimap surface is ${mapBuffer.join("x")}, not 160x120`);
    }
    const rect = await evaluate(cdp, "window.kam2.map().rect");
    if (!Array.isArray(rect) || rect.length !== 4) {
      throw new Error(`the minimap window is ${JSON.stringify(rect)}`);
    }

    const ui3Shot = Buffer.from((await cdp.send("Page.captureScreenshot", { format: "png" })).data, "base64");
    writeFileSync(UI3_PNG, ui3Shot);
    const ui3Bytes = statSync(UI3_PNG).size;
    const ui3Stats = analyzePixels(decodePng(ui3Shot));
    if (!isDrawn(ui3Stats)) throw new Error(`the KAM13 screenshot is blank (${JSON.stringify(ui3Stats)})`);
    const tests = crateTests();
    console.log(`KAM13 OK selected=${panelName} panel=${panelSize.join("x")} buttons=${commands.length} minimap=${mapSize.join("x")} rect=${rect.join(",")} tests=${tests} png=${UI3_PNG} bytes=${ui3Bytes}`);
    console.log(sourcesLine());

    // 5. The KAM14 pointer contract: the mouse alone gives every order. The
    // page is fresh and frozen after its frame budget, so the field stands
    // still and no key is touched anywhere in this run.
    await cdp.send("Page.navigate", { url: `http://127.0.0.1:${server.port}/index.html?frames=${options.frames}` });
    const pointerState = await waitFor(cdp, "[document.body.dataset.state ?? null, document.getElementById('fatal')?.innerText ?? '']", (v) => v && (v[0] === "frozen" || v[0] === "fatal"), 120000, "the pointer page never reached its frame budget");
    if (pointerState[0] === "fatal") throw new Error(`the pointer page failed: ${pointerState[1]}`);

    // 5a. Place a farm with the mouse: arm the cell, then click the ground
    // the sim itself accepts.
    const farmCell = await evaluate(cdp, "(() => { const r = document.querySelectorAll('#build-grid .cell')[1].getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) }; })()");
    await clickAt(cdp, farmCell.x, farmCell.y);
    await sleep(250);
    const armedFarm = await evaluate(cdp, "window.kam2.armed()");
    if (armedFarm !== "farm") throw new Error(`the mouse armed ${JSON.stringify(armedFarm)}, not farm`);
    const housesBefore = await evaluate(cdp, "window.kam2.houses()");
    const farmSite = await evaluate(cdp, "(() => { for (let z = 0; z < 12; z += 1) for (let x = 0; x < 16; x += 1) { if (window.kam2.placement('farm', x, z) !== '') continue; const p = window.kam2.pixelOf(x, z); if (p && p.x > 30 && p.x < 1090 && p.y > 30 && p.y < 660) return { x, z, px: p.x, py: p.y }; } return null; })()");
    if (!farmSite) throw new Error("the sim accepts no farm tile on the pointer page");
    await clickAt(cdp, farmSite.px, farmSite.py);
    await sleep(400);
    const farmOrder = await evaluate(cdp, "window.kam2.order()");
    if (farmOrder !== "") throw new Error(`the farm at (${farmSite.x},${farmSite.z}) was refused: ${farmOrder}`);
    const housesAfter = await evaluate(cdp, "window.kam2.houses()");
    if (!(housesAfter > housesBefore)) throw new Error(`the house count did not rise: ${housesBefore} then ${housesAfter}`);

    // The armed cell is cancelled with the right button, as the plan says.
    await rightClickAt(cdp, farmSite.px, farmSite.py);
    await sleep(250);
    const disarmed = await evaluate(cdp, "window.kam2.armed()");
    if (disarmed !== null) throw new Error(`the right button left the ${disarmed} cell armed`);

    // 5b. Pick a swordsman with the mouse: the panel names the sim's own unit
    // and the selection holds its id.
    const swordsman = await evaluate(cdp, "(() => { const map = window.kam2.map(); for (const unit of map.units) { if (unit.side !== 0) continue; const p = window.kam2.pixelOf(unit.x, unit.z); if (!p || p.x <= 30 || p.x >= 1090 || p.y <= 30 || p.y >= 660) continue; const id = window.kam2.pick(unit.x, unit.z); if (!id) continue; const state = window.kam2.unit(id); return { x: unit.x, z: unit.z, px: p.x, py: p.y, id, name: state ? state.name : '' }; } return null; })()");
    if (!swordsman) throw new Error("the pointer page found no side-0 soldier to pick");
    await clickAt(cdp, swordsman.px, swordsman.py);
    await sleep(400);
    const picked = await evaluate(cdp, "JSON.stringify(window.kam2.selection())");
    if (!picked.includes(swordsman.id)) throw new Error(`the selection holds ${picked}, not ${swordsman.id}`);
    const soldierPanel = await evaluate(cdp, "document.getElementById('sel-name').textContent");
    if (soldierPanel !== swordsman.name) throw new Error(`the panel says "${soldierPanel}", the module says "${swordsman.name}"`);

    // 5c. The right button attack-moves the picked soldier: the sim takes the
    // order, so the order line stays empty and the soldier walks.
    const open = await evaluate(cdp, "(() => { const ids = window.kam2.selection(); const unit = window.kam2.unit(ids[ids.length - 1]); for (const [dx, dz] of [[0, 1], [1, 0], [-1, 0], [0, -1]]) { const x = unit.x + dx; const z = unit.z + dz; if (x < 0 || z < 0 || x > 15 || z > 11) continue; if (window.kam2.pick(x, z) !== '') continue; const p = window.kam2.pixelOf(x, z); if (!p || p.x <= 30 || p.x >= 1090 || p.y <= 30 || p.y >= 660) continue; return { x, z, px: p.x, py: p.y }; } return null; })()");
    if (!open) throw new Error("no free ground beside the soldier to walk to");
    await rightClickAt(cdp, open.px, open.py);
    await sleep(400);
    const walkOrder = await evaluate(cdp, "window.kam2.order()");
    if (walkOrder !== "") throw new Error(`the walk to (${open.x},${open.z}) was refused: ${walkOrder}`);
    const walking = await evaluate(cdp, `window.kam2.unit(${JSON.stringify(swordsman.id)})`);
    if (!walking || walking.moving !== true) throw new Error(`the soldier did not take the walk order: ${JSON.stringify(walking)}`);

    // 5d. The stop button stands the soldier still and forgets the path.
    const stopButton = await evaluate(cdp, "(() => { const r = document.querySelector('#command-row .command[data-order=\"stop\"]').getBoundingClientRect(); return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) }; })()");
    await clickAt(cdp, stopButton.x, stopButton.y);
    await sleep(400);
    const stopOrder = await evaluate(cdp, "window.kam2.order()");
    if (stopOrder !== "") throw new Error(`the stop order was refused: ${stopOrder}`);
    const stopped = await evaluate(cdp, `window.kam2.unit(${JSON.stringify(swordsman.id)})`);
    if (!stopped || stopped.moving !== false) throw new Error(`the soldier still walks after the stop: ${JSON.stringify(stopped)}`);

    // 5e. A foe under the cursor: the cursor is a crosshair, the left button
    // holds the soldier against it, and the selection does not move.
    const foe = await evaluate(cdp, "(() => { const map = window.kam2.map(); for (const unit of map.units) { if (unit.side === 0) continue; const p = window.kam2.pixelOf(unit.x, unit.z); if (!p || p.x <= 30 || p.x >= 1090 || p.y <= 30 || p.y >= 660) continue; const id = window.kam2.pick(unit.x, unit.z); if (!id) continue; return { x: unit.x, z: unit.z, px: p.x, py: p.y, id }; } return null; })()");
    if (!foe) throw new Error("the pointer page found no foe in sight");
    await moveMouse(cdp, foe.px, foe.py);
    await sleep(250);
    const cursor = await evaluate(cdp, "document.getElementById('stage').style.cursor");
    if (cursor !== "crosshair") throw new Error(`the cursor over a foe is "${cursor}", not crosshair`);
    const hashBefore = await evaluate(cdp, "window.kam2.hash()");
    await clickAt(cdp, foe.px, foe.py);
    await sleep(400);
    const holdOrder = await evaluate(cdp, "window.kam2.order()");
    if (holdOrder !== "") throw new Error(`the hold order was refused: ${holdOrder}`);
    const hashAfter = await evaluate(cdp, "window.kam2.hash()");
    if (hashAfter === hashBefore) throw new Error("the hold order left the match untouched");
    const heldSelection = await evaluate(cdp, "JSON.stringify(window.kam2.selection())");
    if (heldSelection !== picked) throw new Error(`the hold order moved the selection: ${picked} then ${heldSelection}`);

    // 5f. The right button on the foe's own ground: the sim refuses in its
    // own words.
    await rightClickAt(cdp, foe.px, foe.py);
    await sleep(400);
    const occupied = await evaluate(cdp, "window.kam2.order()");
    if (occupied !== "Occupied: that ground is taken") throw new Error(`the ground of the foe printed "${occupied}"`);

    // 5g. Shift and the left button add to the selection.
    const second = await evaluate(cdp, "(() => { const map = window.kam2.map(); const ids = window.kam2.selection(); const mine = window.kam2.unit(ids[ids.length - 1]); for (const unit of map.units) { if (unit.side !== mine.side) continue; if (unit.x === mine.x && unit.z === mine.z) continue; const p = window.kam2.pixelOf(unit.x, unit.z); if (!p || p.x <= 30 || p.x >= 1090 || p.y <= 30 || p.y >= 660) continue; const id = window.kam2.pick(unit.x, unit.z); if (!id) continue; return { x: unit.x, z: unit.z, px: p.x, py: p.y, id }; } return null; })()");
    if (!second) throw new Error("the pointer page found no second soldier to add");
    await shiftClickAt(cdp, second.px, second.py);
    await sleep(400);
    const both = await evaluate(cdp, "JSON.stringify(window.kam2.selection())");
    const bothIds = JSON.parse(both);
    if (bothIds.length !== 2) throw new Error(`the shift click left ${bothIds.length} soldiers picked: ${both}`);
    if (!bothIds.includes(swordsman.id) || !bothIds.includes(second.id)) throw new Error(`the shift click picked ${both}`);

    // 5h. A click on empty ground clears the selection.
    const empty = await evaluate(cdp, "(() => { const map = window.kam2.map(); for (let z = 0; z < map.mapH; z += 1) for (let x = 0; x < map.mapW; x += 1) { if (window.kam2.pick(x, z) !== '') continue; const p = window.kam2.pixelOf(x, z); if (!p || p.x <= 30 || p.x >= 1090 || p.y <= 30 || p.y >= 660) continue; return { x, z, px: p.x, py: p.y }; } return null; })()");
    if (!empty) throw new Error("the pointer page found no empty ground to click");
    await clickAt(cdp, empty.px, empty.py);
    await sleep(400);
    const cleared = await evaluate(cdp, "JSON.stringify(window.kam2.selection())");
    if (cleared !== "[]") throw new Error(`a click on empty ground left ${cleared} picked`);
    const emptyPanel = await evaluate(cdp, "document.getElementById('sel-name').textContent");
    if (emptyPanel !== "nothing picked") throw new Error(`the panel says "${emptyPanel}" after the ground was clicked`);

    // 5i. A house is picked like a unit, and the panel shows the sim's own
    // numbers for it.
    const house = await evaluate(cdp, "(() => { const map = window.kam2.map(); for (const marker of map.houses) { const p = window.kam2.pixelOf(marker.x, marker.z); if (!p || p.x <= 30 || p.x >= 1090 || p.y <= 30 || p.y >= 660) continue; const id = window.kam2.pick(marker.x, marker.z); if (!id || !id.startsWith('h')) continue; const state = window.kam2.house(id); return { x: marker.x, z: marker.z, px: p.x, py: p.y, id, name: state ? state.name : '' }; } return null; })()");
    if (!house) throw new Error("the pointer page found no house to pick");
    await clickAt(cdp, house.px, house.py);
    await sleep(400);
    const houseSelection = await evaluate(cdp, "JSON.stringify(window.kam2.selection())");
    if (!houseSelection.includes(house.id)) throw new Error(`the selection holds ${houseSelection}, not ${house.id}`);
    const housePanel = await evaluate(cdp, "document.getElementById('sel-name').textContent");
    if (housePanel !== house.name) throw new Error(`the panel says "${housePanel}", the module says "${house.name}"`);

    // 5j. A click on the minimap moves the camera, and the rectangle on it
    // follows the camera by exactly what the camera moved.
    const lookBefore = await evaluate(cdp, "(() => { const m = window.kam2.map(); return { pan: window.kam2.pan(), look: m.look, centre: m.centre }; })()");
    const box = await evaluate(cdp, "(() => { const r = document.getElementById('minimap').getBoundingClientRect(); return { left: r.left, top: r.top, width: r.width, height: r.height }; })()");
    await clickAt(cdp, Math.round(box.left + box.width * 0.25), Math.round(box.top + box.height * 0.25));
    await sleep(400);
    const lookAfter = await evaluate(cdp, "(() => { const m = window.kam2.map(); return { pan: window.kam2.pan(), look: m.look, centre: m.centre }; })()");
    const dx = lookAfter.pan[0] - lookBefore.pan[0];
    const dz = lookAfter.pan[1] - lookBefore.pan[1];
    if (Math.abs(dx) < 0.001 && Math.abs(dz) < 0.001) throw new Error("the minimap click did not move the camera");
    if (Math.abs(lookAfter.look[0] - lookBefore.look[0] - dx * 10) > 0.05) throw new Error(`the window moved ${lookAfter.look[0] - lookBefore.look[0]} px for a camera move of ${dx}`);
    if (Math.abs(lookAfter.look[1] - lookBefore.look[1] - dz * 10) > 0.05) throw new Error(`the window moved ${lookAfter.look[1] - lookBefore.look[1]} px for a camera move of ${dz}`);
    const centred = [(lookAfter.look[0] + lookAfter.look[2] / 2) / 10, (lookAfter.look[1] + lookAfter.look[3] / 2) / 10];
    if (Math.abs(centred[0] - lookAfter.centre[0]) > 0.01 || Math.abs(centred[1] - lookAfter.centre[1]) > 0.01) {
      throw new Error(`the window sits on ${centred.join(",")}, the camera on ${lookAfter.centre.join(",")}`);
    }

    // 5k. The page with the sidebar, a house picked and a refused build with
    // its reason: the armed farm over the village road.
    const roadTile = await evaluate(cdp, "(() => { for (let x = 0; x < 16; x += 1) { const text = window.kam2.placement('farm', x, 6); if (!text.startsWith('NotBuildable')) continue; const p = window.kam2.pixelOf(x, 6); if (p && p.x > 30 && p.x < 1090 && p.y > 30 && p.y < 660) return { x, z: 6, px: p.x, py: p.y, text }; } return null; })()");
    if (!roadTile) throw new Error("no road tile of row 6 is refused as unbuildable ground");
    await clickAt(cdp, farmCell.x, farmCell.y);
    await sleep(250);
    if ((await evaluate(cdp, "window.kam2.armed()")) !== "farm") throw new Error("the farm cell did not arm again");
    await clickAt(cdp, roadTile.px, roadTile.py);
    await sleep(400);
    const roadOrder = await evaluate(cdp, "window.kam2.order()");
    if (roadOrder !== roadTile.text) throw new Error(`the road printed "${roadOrder}", the sim says "${roadTile.text}"`);

    const ui4Shot = Buffer.from((await cdp.send("Page.captureScreenshot", { format: "png" })).data, "base64");
    writeFileSync(UI4_PNG, ui4Shot);
    const ui4Bytes = statSync(UI4_PNG).size;
    const ui4Stats = analyzePixels(decodePng(ui4Shot));
    if (!isDrawn(ui4Stats)) throw new Error(`the KAM14 screenshot is blank (${JSON.stringify(ui4Stats)})`);
    console.log(`KAM14 OK farm=${farmSite.x},${farmSite.z} soldier=${swordsman.id} walk=${open.x},${open.z} foe=${foe.id} refused=${occupied} picked=${bothIds.length} house=${house.id} road=${roadOrder} pan=${lookAfter.pan.map((n) => Number(n.toFixed(2))).join(",")} window=${lookAfter.look.join(",")} png=${UI4_PNG} bytes=${ui4Bytes}`);

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

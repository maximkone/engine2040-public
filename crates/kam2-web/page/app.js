// KAM11 page: the wasm module owns the match and the fixed 10 Hz tick; this file
// draws its flat 2.5D frame with Canvas2D, fills the build sidebar from the
// module's own cell list, and paints the footprint the cursor would place.
//
// The page reads one Float32Array per frame (layout in crates/kam2-web/src/scene_flat.rs):
// ground diamonds, then depth-ordered billboards (soldier cells, cottage cells).
// It converts the scene's linear colours to sRGB once per frame and draws the
// packed atlases the same way the native window does. The build cells, the wares
// and every refusal come from the wasm module (`houseKinds`, `stockOf`,
// `placement`, `build`): the page paints and clicks, the sim decides. Nothing
// here feeds a value back into the sim.

import init, { Kam2Web } from "./kam2web.js";

const stage = document.getElementById("stage");
const ctx = stage.getContext("2d", { alpha: false });
const report = document.getElementById("report");
const hudLine = document.getElementById("hud-line");
const hudState = document.getElementById("hud-state");
const wares = document.getElementById("wares");
const grid = document.getElementById("build-grid");
const orderLine = document.getElementById("order-line");
const fatal = document.getElementById("fatal");

// Presentation scale: the scene is built for 1280x720 at 100% zoom (the same
// answer the window's camera gives); the page shows it at 125% so the 16x12
// field fills the canvas width.
const VIEW_SCALE = 1.25;
const PAN_STEP = 0.12;
const REPORT_EVERY = 10;

const GROUND_STRIDE = 11;
const BILLBOARD_STRIDE = 12;
const KIND_UNIT = 1;
const KIND_HOUSE = 2;
// Tiles per ground row: scene_flat writes one diamond per tile in row order.
const MAP_W = 16;
const WARE_NAMES = ["wood", "stone", "grain", "flour", "bread"];

const keys = new Set();
const state = {
  frames: 0,
  tick: 0,
  hash: "",
  webgpu: "unknown",
  error: null,
  limit: 0,
  frame: null,
  cells: [],
  armed: null,
  hover: null,
  hoverOk: false,
  order: "",
};
const params = new URLSearchParams(location.search);
const frameLimit = Number(params.get("frames") ?? 0);
state.limit = Number.isFinite(frameLimit) && frameLimit > 0 ? frameLimit : 0;

let game = null;
let soldier = null;
let cottage = null;
let last = 0;

const byte = (linear) => {
  const v = linear <= 0.0031308 ? 12.92 * linear : 1.055 * Math.pow(linear, 1 / 2.4) - 0.055;
  return Math.max(0, Math.min(255, Math.round(v * 255)));
};

const rgb = (r, g, b) => `rgb(${r} ${g} ${b})`;

function fail(reason) {
  state.error = String(reason);
  document.body.dataset.state = "fatal";
  fatal.textContent = `KAM2 did not start: ${state.error}`;
  fatal.style.display = "block";
  console.error(state.error);
}

async function loadArt(manifestPath) {
  const url = new URL(manifestPath, location.href);
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${manifestPath} answered ${response.status}`);
  const manifest = await response.json();
  const image = new Image();
  image.src = new URL(manifest.atlas, url).href;
  await image.decode();
  return { manifest, image };
}

async function probeWebgpu() {
  if (!navigator.gpu) return "no-webgpu";
  try {
    const adapter = await navigator.gpu.requestAdapter();
    if (!adapter) return "no-adapter";
    const info = adapter.info ?? {};
    return [info.vendor, info.architecture].filter(Boolean).join("/") || "adapter";
  } catch (error) {
    return `error:${String(error)}`;
  }
}

function backdrop() {
  const sky = ctx.createLinearGradient(0, 0, 0, stage.height);
  sky.addColorStop(0, "#0d1218");
  sky.addColorStop(0.45, "#18211f");
  sky.addColorStop(0.72, "#121a1c");
  sky.addColorStop(1, "#080b0f");
  ctx.fillStyle = sky;
  ctx.fillRect(0, 0, stage.width, stage.height);
  const glow = ctx.createRadialGradient(640, 340, 40, 640, 340, 720);
  glow.addColorStop(0, "rgba(143, 168, 78, 0.10)");
  glow.addColorStop(1, "rgba(143, 168, 78, 0)");
  ctx.fillStyle = glow;
  ctx.fillRect(0, 0, stage.width, stage.height);
}

function sceneTransform() {
  const s = VIEW_SCALE;
  ctx.setTransform(s, 0, 0, s, (1 - s) * stage.width * 0.5, (1 - s) * stage.height * 0.5);
}

function drawGround(frame, base, count) {
  let o = base;
  for (let i = 0; i < count; i += 1, o += GROUND_STRIDE) {
    const fill = rgb(byte(frame[o + 8]), byte(frame[o + 9]), byte(frame[o + 10]));
    ctx.beginPath();
    ctx.moveTo(frame[o], frame[o + 1]);
    ctx.lineTo(frame[o + 2], frame[o + 3]);
    ctx.lineTo(frame[o + 4], frame[o + 5]);
    ctx.lineTo(frame[o + 6], frame[o + 7]);
    ctx.closePath();
    ctx.fillStyle = fill;
    ctx.strokeStyle = fill;
    ctx.lineWidth = 1;
    ctx.fill();
    ctx.stroke();
  }
}

function drawBillboard(frame, o) {
  const kind = frame[o];
  const tx = frame[o + 4];
  const ty = frame[o + 5];
  const w = frame[o + 6];
  const h = frame[o + 7];
  const art = kind === KIND_HOUSE ? cottage : soldier;
  const [pivotX, pivotY] = art.manifest.pivot;
  const feetX = tx + (pivotX / art.manifest.tile_w) * w;
  const feetY = ty + (pivotY / art.manifest.tile_h) * h;

  ctx.beginPath();
  if (kind === KIND_HOUSE) {
    ctx.ellipse(feetX, feetY, 38, 12, 0, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(0, 0, 0, 0.26)";
    ctx.fill();
  } else {
    ctx.ellipse(feetX, feetY, 12, 5.5, 0, 0, Math.PI * 2);
    ctx.fillStyle = "rgba(0, 0, 0, 0.32)";
    ctx.fill();
    ctx.strokeStyle = rgb(frame[o + 1], frame[o + 2], frame[o + 3]);
    ctx.lineWidth = 2;
    ctx.stroke();
  }

  const image = art.image;
  const sx = frame[o + 8] * image.width;
  const sy = frame[o + 9] * image.height;
  const sw = frame[o + 10] * image.width;
  const sh = frame[o + 11] * image.height;
  ctx.drawImage(image, sx, sy, sw, sh, tx, ty, w, h);
}

// One ground diamond of the frame as [x0, y0, x1, y1, x2, y2, x3, y3], top,
// right, bottom and left corner in framebuffer pixels.
function groundDiamond(frame, x, z) {
  if (!frame || x < 0 || z < 0 || x >= MAP_W) return null;
  const rows = (frame[0] | 0) / MAP_W;
  if (z >= rows) return null;
  const o = 4 + (z * MAP_W + x) * GROUND_STRIDE;
  return frame.subarray(o, o + 8);
}

// The four screen corners of a `w` by `h` footprint with its top-left at (x, z),
// taken from the diamonds the renderer already drew.
function footprintCorners(frame, x, z, w, h) {
  const top = groundDiamond(frame, x, z);
  const right = groundDiamond(frame, x + w - 1, z);
  const bottom = groundDiamond(frame, x + w - 1, z + h - 1);
  const left = groundDiamond(frame, x, z + h - 1);
  if (!top || !right || !bottom || !left) return null;
  return [
    [top[0], top[1]],
    [right[2], right[3]],
    [bottom[4], bottom[5]],
    [left[6], left[7]],
  ];
}

function drawFootprint() {
  if (!state.armed || !state.hover) return;
  const corners = footprintCorners(
    state.frame,
    state.hover.x,
    state.hover.z,
    state.armed.w,
    state.armed.h,
  );
  if (!corners) return;
  ctx.beginPath();
  ctx.moveTo(corners[0][0], corners[0][1]);
  for (const corner of corners.slice(1)) ctx.lineTo(corner[0], corner[1]);
  ctx.closePath();
  ctx.fillStyle = state.hoverOk ? "rgba(72, 158, 74, 0.42)" : "rgba(186, 46, 38, 0.42)";
  ctx.fill();
  ctx.lineWidth = 1.5;
  ctx.strokeStyle = state.hoverOk ? "rgba(128, 240, 134, 0.95)" : "rgba(255, 126, 116, 0.95)";
  ctx.stroke();
}

function drawFrame(frame) {
  backdrop();
  sceneTransform();
  const grounds = frame[0] | 0;
  const billboards = frame[1] | 0;
  drawGround(frame, 4, grounds);
  const base = 4 + grounds * GROUND_STRIDE;
  for (let i = 0; i < billboards; i += 1) {
    drawBillboard(frame, base + i * BILLBOARD_STRIDE);
  }
  drawFootprint();
}

function applyPan() {
  let dx = 0;
  let dz = 0;
  if (keys.has("ArrowLeft")) dx += PAN_STEP;
  if (keys.has("ArrowRight")) dx -= PAN_STEP;
  if (keys.has("ArrowUp")) dz += PAN_STEP;
  if (keys.has("ArrowDown")) dz -= PAN_STEP;
  if (dx !== 0 || dz !== 0) game.pan(dx, dz);
}

// Pointer to framebuffer pixels: the canvas is stretched to the window with
// object-fit contain, so the drawn bitmap keeps its 1280x720 shape and is
// centred inside the element.
function framebufferPoint(event) {
  const rect = stage.getBoundingClientRect();
  const scale = Math.min(rect.width / stage.width, rect.height / stage.height);
  if (!(scale > 0)) return null;
  const offsetX = (rect.width - stage.width * scale) / 2;
  const offsetY = (rect.height - stage.height * scale) / 2;
  return [
    (event.clientX - rect.left - offsetX) / scale,
    (event.clientY - rect.top - offsetY) / scale,
  ];
}

// Framebuffer pixels to the scene space the frame was built in: the draw
// transform scales by VIEW_SCALE about the canvas centre.
function scenePoint(event) {
  const fb = framebufferPoint(event);
  if (!fb) return null;
  const s = VIEW_SCALE;
  return [
    (fb[0] - (1 - s) * stage.width * 0.5) / s,
    (fb[1] - (1 - s) * stage.height * 0.5) / s,
  ];
}

function clientPoint(sceneX, sceneY) {
  const rect = stage.getBoundingClientRect();
  const scale = Math.min(rect.width / stage.width, rect.height / stage.height);
  const offsetX = (rect.width - stage.width * scale) / 2;
  const offsetY = (rect.height - stage.height * scale) / 2;
  const s = VIEW_SCALE;
  const fbX = s * sceneX + (1 - s) * stage.width * 0.5;
  const fbY = s * sceneY + (1 - s) * stage.height * 0.5;
  return {
    x: Math.round(rect.left + offsetX + fbX * scale),
    y: Math.round(rect.top + offsetY + fbY * scale),
  };
}

function insideDiamond(px, py, diamond) {
  let sign = 0;
  for (let corner = 0; corner < 4; corner += 1) {
    const a = (corner % 4) * 2;
    const b = ((corner + 1) % 4) * 2;
    const cross =
      (diamond[b] - diamond[a]) * (py - diamond[a + 1]) -
      (diamond[b + 1] - diamond[a + 1]) * (px - diamond[a]);
    if (cross === 0) continue;
    const side = cross > 0 ? 1 : -1;
    if (sign === 0) sign = side;
    else if (side !== sign) return false;
  }
  return true;
}

// The tile under a point of the frame's own scene space; the diamonds are the
// ones the player sees, so a panned or scaled view still lands on its tile.
function tileUnder(px, py) {
  const frame = state.frame;
  if (!frame) return null;
  const grounds = frame[0] | 0;
  for (let i = 0; i < grounds; i += 1) {
    const diamond = frame.subarray(4 + i * GROUND_STRIDE, 4 + i * GROUND_STRIDE + 8);
    if (insideDiamond(px, py, diamond)) {
      return { x: i % MAP_W, z: (i / MAP_W) | 0 };
    }
  }
  return null;
}

function applyCursor() {
  if (!state.armed) {
    stage.style.cursor = "default";
    return;
  }
  stage.style.cursor = state.hoverOk ? "crosshair" : "not-allowed";
}

function setOrder(text) {
  state.order = text;
  orderLine.textContent = text;
}

function refreshHover(event) {
  if (!state.armed || !game) {
    state.hover = null;
    state.hoverOk = false;
    applyCursor();
    return;
  }
  const point = scenePoint(event);
  const tile = point ? tileUnder(point[0], point[1]) : null;
  state.hover = tile;
  state.hoverOk = tile ? game.placement(state.armed.id, tile.x, tile.z) === "" : false;
  applyCursor();
}

function arm(cell) {
  state.armed = cell;
  state.hover = null;
  state.hoverOk = false;
  for (const other of state.cells) {
    other.button.setAttribute("aria-pressed", String(other === cell));
  }
  applyCursor();
  publish();
}

function disarm() {
  if (!state.armed) return;
  state.armed = null;
  state.hover = null;
  state.hoverOk = false;
  for (const cell of state.cells) cell.button.setAttribute("aria-pressed", "false");
  applyCursor();
  publish();
}

function placeAtPointer(event) {
  if (!state.armed || !game) return;
  const point = scenePoint(event);
  const tile = point ? tileUnder(point[0], point[1]) : null;
  if (!tile) return;
  setOrder(game.build(state.armed.id, tile.x, tile.z));
  state.hover = tile;
  state.hoverOk = state.order === "";
  applyCursor();
  publish();
}

function buildGrid() {
  for (const [slot, kind] of JSON.parse(Kam2Web.houseKinds()).entries()) {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "cell";
    button.title = `${kind.label} (${kind.w} x ${kind.h}) - ${kind.cost}`;
    button.dataset.kind = kind.id;
    button.setAttribute("aria-pressed", "false");
    const image = document.createElement("img");
    image.src = `cameos/${kind.id}.png`;
    image.alt = kind.label;
    image.width = 64;
    image.height = 48;
    const key = document.createElement("span");
    key.className = "key";
    key.textContent = String(slot + 1);
    button.append(image, key);
    const cell = { id: kind.id, label: kind.label, w: kind.w, h: kind.h, button };
    button.addEventListener("click", () => arm(cell));
    state.cells.push(cell);
    grid.append(button);
  }
}

function wareStrip() {
  const counts = Array.from(game.stockOf(0));
  const parts = WARE_NAMES.map((name, slot) => {
    const span = document.createElement("span");
    const label = document.createElement("b");
    label.textContent = name;
    span.append(label, document.createTextNode(` ${counts[slot] ?? 0}`));
    return span;
  });
  wares.replaceChildren(...parts);
}

function publish() {
  state.tick = game.ticks;
  state.hash = game.hash;
  const paused = game.paused;
  hudState.textContent = paused ? "PAUSED" : "RUN";
  hudState.className = paused ? "tag paused" : "tag";
  hudLine.textContent = `tick ${state.tick} · hash ${state.hash} · houses ${game.houses} · units ${game.units}`;
  wareStrip();
  orderLine.textContent = state.order;
  report.textContent = JSON.stringify({
    ok: state.error === null,
    frames: state.frames,
    tick: state.tick,
    hash: state.hash,
    houses: game.houses,
    units: game.units,
    paused,
    stock: game.stock,
    armed: state.armed ? state.armed.id : null,
    order: state.order,
    webgpu: state.webgpu,
    canvas: "canvas2d",
  });
}

function tickFrame(now) {
  const dt = last === 0 ? 0 : now - last;
  last = now;
  game.advance(dt);
  applyPan();
  const frame = game.frame();
  state.frame = frame;
  drawFrame(frame);
  state.frames += 1;
  window.kam2Frames = state.frames;
  if (state.frames === 1) {
    document.body.dataset.state = "running";
  }
  if (state.frames % REPORT_EVERY === 0) publish();
  if (state.limit > 0 && state.frames >= state.limit) {
    document.body.dataset.state = "frozen";
    return;
  }
  requestAnimationFrame(tickFrame);
}

function onKeyDown(event) {
  if (event.code === "Space") {
    event.preventDefault();
    game.togglePause();
    publish();
    return;
  }
  if (event.code === "Escape") {
    event.preventDefault();
    disarm();
    return;
  }
  if (event.key >= "1" && event.key <= "5") {
    const cell = state.cells[Number(event.key) - 1];
    if (cell) {
      event.preventDefault();
      arm(cell);
    }
    return;
  }
  if (event.code.startsWith("Arrow")) {
    event.preventDefault();
    keys.add(event.code);
  }
}

function onKeyUp(event) {
  keys.delete(event.code);
}

async function start() {
  try {
    state.webgpu = await probeWebgpu();
    [soldier, cottage] = await Promise.all([
      loadArt("soldier/manifest.json"),
      loadArt("house/manifest.json"),
    ]);
    await init();
    game = new Kam2Web();
    buildGrid();
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    window.kam2 = {
      frames: () => state.frames,
      tick: () => game.ticks,
      hash: () => game.hash,
      paused: () => game.paused,
      houses: () => game.houses,
      units: () => game.units,
      pan: () => Array.from(game.panOffset),
      webgpu: () => state.webgpu,
      armed: () => (state.armed ? state.armed.id : null),
      order: () => orderLine.textContent,
      placement: (kind, x, z) => game.placement(kind, x, z),
      stock: (side) => Array.from(game.stockOf(side ?? 0)),
      // Client pixel of a tile centre, for the headless pointer drill.
      pixelOf: (x, z) => {
        const diamond = groundDiamond(state.frame, x, z);
        if (!diamond) return null;
        const cx = (diamond[0] + diamond[2] + diamond[4] + diamond[6]) / 4;
        const cy = (diamond[1] + diamond[3] + diamond[5] + diamond[7]) / 4;
        return clientPoint(cx, cy);
      },
    };
    addEventListener("keydown", onKeyDown);
    addEventListener("keyup", onKeyUp);
    addEventListener("blur", () => keys.clear());
    stage.addEventListener("mousemove", refreshHover);
    stage.addEventListener("mouseleave", () => {
      state.hover = null;
      state.hoverOk = false;
      applyCursor();
    });
    stage.addEventListener("mousedown", (event) => {
      if (event.button !== 0 || !state.armed) return;
      event.preventDefault();
      refreshHover(event);
      placeAtPointer(event);
    });
    stage.addEventListener("contextmenu", (event) => {
      if (!state.armed) return;
      event.preventDefault();
      disarm();
    });
    document.addEventListener("visibilitychange", () => {
      last = 0;
    });
    publish();
    requestAnimationFrame(tickFrame);
  } catch (error) {
    fail(error && error.message ? error.message : error);
  }
}

addEventListener("error", (event) => {
  if (state.error === null) fail(event.message);
});
addEventListener("unhandledrejection", (event) => {
  if (state.error === null) fail(event.reason && event.reason.message ? event.reason.message : event.reason);
});

start();

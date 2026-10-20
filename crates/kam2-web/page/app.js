// KAM8 page: the wasm module owns the match and the fixed 10 Hz tick; this file
// draws its flat 2.5D frame with Canvas2D and forwards the two keys.
//
// The page reads one Float32Array per frame (layout in crates/kam2-web/src/scene_flat.rs):
// ground diamonds, then depth-ordered billboards (soldier cells, cottage cells).
// It converts the scene's linear colours to sRGB once per frame and draws the
// packed atlases the same way the native window does. Nothing here feeds a value
// back into the sim.

import init, { Kam2Web } from "./kam2web.js";

const stage = document.getElementById("stage");
const ctx = stage.getContext("2d", { alpha: false });
const report = document.getElementById("report");
const hudLine = document.getElementById("hud-line");
const hudState = document.getElementById("hud-state");
const hudStock = document.getElementById("hud-stock");
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

const keys = new Set();
const state = { frames: 0, tick: 0, hash: "", webgpu: "unknown", error: null, limit: 0 };
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
  fatal.textContent = `KAM8 did not start: ${state.error}`;
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

function publish() {
  state.tick = game.ticks;
  state.hash = game.hash;
  const paused = game.paused;
  hudState.textContent = paused ? "PAUSED" : "RUN";
  hudState.className = paused ? "tag paused" : "tag";
  hudLine.textContent = `tick ${state.tick} · hash ${state.hash} · houses ${game.houses} · units ${game.units} · frames ${state.frames}`;
  hudStock.textContent = game.stock;
  report.textContent = JSON.stringify({
    ok: state.error === null,
    frames: state.frames,
    tick: state.tick,
    hash: state.hash,
    houses: game.houses,
    units: game.units,
    paused,
    stock: game.stock,
    webgpu: state.webgpu,
    canvas: "canvas2d",
  });
}

function tickFrame(now) {
  const dt = last === 0 ? 0 : now - last;
  last = now;
  game.advance(dt);
  applyPan();
  drawFrame(game.frame());
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
    ctx.imageSmoothingEnabled = true;
    ctx.imageSmoothingQuality = "high";
    window.kam2 = {
      frames: () => state.frames,
      tick: () => game.ticks,
      hash: () => game.hash,
      paused: () => game.paused,
      pan: () => Array.from(game.panOffset),
      webgpu: () => state.webgpu,
    };
    addEventListener("keydown", onKeyDown);
    addEventListener("keyup", onKeyUp);
    addEventListener("blur", () => keys.clear());
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

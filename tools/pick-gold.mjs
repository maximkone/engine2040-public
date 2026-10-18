// EV21 gold pointing eval writer: 100 pixel tasks over one fixed scene, answered
// by this file's own ray maths (rotation matrices and slabs, written apart from
// crates/endpoint/src/pick.rs so the two can disagree). Output is
// fixtures/pick/gold.txt; the Rust test reads it and must match 100/100.
//   node tools/pick-gold.mjs          rewrite the fixture
//   node tools/pick-gold.mjs --check  exit 1 when the fixture differs
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const GOLD_PATH = join(ROOT, "fixtures", "pick", "gold.txt");

// Camera in cm and centidegrees; actors in meters (dyadic values, so f32 == f64).
const CAMERA = { eye: [0, 600, -1200], yaw: 0, pitch: -2500, fov: 6000, w: 1280, h: 720 };
const ACTORS = [
  [1, "crate", -3.5, 0.5, 2, 1, 1, 1],
  [2, "barrel", 0, 0.75, 4, 1, 1.5, 1],
  [3, "crate", 3.5, 0.5, 3, 1, 1, 1],
  [4, "pillar", -6, 2, 8, 1, 4, 1],
  [5, "wall", 0, 1, 12, 8, 2, 0.5],
  [6, "altar", 6, 0.5, 7, 2, 1, 2],
];
const FAR_CM = 10000;
const TASKS = 100;

const rad = (cdeg) => (cdeg / 100) * (Math.PI / 180);

// Ray through the centre of pixel (px, py): camera space, then pitch, then yaw.
function rayOf(px, py) {
  const { w, h, fov, pitch, yaw } = CAMERA;
  const focal = h / 2 / Math.tan(rad(fov) / 2);
  const cam = [(px + 0.5 - w / 2) / focal, -(py + 0.5 - h / 2) / focal, 1];
  const [sp, cp, sy, cy] = [Math.sin(rad(pitch)), Math.cos(rad(pitch)), Math.sin(rad(yaw)), Math.cos(rad(yaw))];
  const tilted = [cam[0], cam[1] * cp + cam[2] * sp, -cam[1] * sp + cam[2] * cp];
  const world = [tilted[0] * cy + tilted[2] * sy, tilted[1], -tilted[0] * sy + tilted[2] * cy];
  const len = Math.hypot(...world);
  return world.map((v) => v / len);
}

// Entry distance of the ray into a box (cm), or null; `margin` reports a graze.
function slab(actor, dir) {
  const [, , x, y, z, sx, sy, sz] = actor;
  const c = [x, y, z].map((v) => v * 100);
  const hf = [sx, sy, sz].map((v) => v * 50);
  let near = 0;
  let far = Infinity;
  for (let k = 0; k < 3; k += 1) {
    const lo = c[k] - hf[k];
    const hi = c[k] + hf[k];
    if (Math.abs(dir[k]) < 1e-12) {
      if (CAMERA.eye[k] < lo || CAMERA.eye[k] > hi) return null;
      continue;
    }
    const a = (lo - CAMERA.eye[k]) / dir[k];
    const b = (hi - CAMERA.eye[k]) / dir[k];
    near = Math.max(near, Math.min(a, b));
    far = Math.min(far, Math.max(a, b));
  }
  return near <= far && far > 0 ? { near, far } : null;
}

// Pixel under a world point (cm), the inverse of rayOf; null behind the eye.
function pixelOf(point) {
  const { w, h, fov, pitch, yaw, eye } = CAMERA;
  const focal = h / 2 / Math.tan(rad(fov) / 2);
  const d = point.map((v, k) => v - eye[k]);
  const [sp, cp, sy, cy] = [Math.sin(rad(pitch)), Math.cos(rad(pitch)), Math.sin(rad(yaw)), Math.cos(rad(yaw))];
  const yawed = [d[0] * cy - d[2] * sy, d[1], d[0] * sy + d[2] * cy];
  const cam = [yawed[0], yawed[1] * cp - yawed[2] * sp, yawed[1] * sp + yawed[2] * cp];
  if (cam[2] <= 1e-9) return null;
  const px = Math.floor(w / 2 + (focal * cam[0]) / cam[2]);
  const py = Math.floor(h / 2 - (focal * cam[1]) / cam[2]);
  return px >= 0 && px < w && py >= 0 && py < h ? [px, py] : null;
}

// Round half away from zero, like Rust's f64::round.
const cm = (v) => Math.sign(v) * Math.round(Math.abs(v));
const nearHalf = (v) => Math.abs(Math.abs(v % 1) - 0.5) < 1e-6;

// Answer for one pixel, or null when the pixel grazes an edge or a rounding tie.
function answer(px, py) {
  const dir = rayOf(px, py);
  let best = null;
  for (const actor of ACTORS) {
    const hit = slab(actor, dir);
    if (!hit) continue;
    if (hit.far - hit.near < 1e-3) return null;
    if (!best || hit.near < best.t) best = { t: hit.near, id: actor[0] };
  }
  if (best) return `id ${best.id}`;
  const t = dir[1] < 0 ? Math.min(-CAMERA.eye[1] / dir[1], FAR_CM) : FAR_CM;
  const point = CAMERA.eye.map((e, k) => e + dir[k] * t);
  if (point.some(nearHalf)) return null;
  return `pt ${point.map(cm).join(" ")}`;
}

export function generate() {
  const { eye, yaw, pitch, fov, w, h } = CAMERA;
  const lines = [
    "# EV21 gold pointing eval. Written by tools/pick-gold.mjs (its own ray maths,",
    "# apart from crates/endpoint/src/pick.rs); do not edit by hand.",
    "# camera: eye in cm, angles in centidegrees. actor: id name x y z sx sy sz",
    "# (meters, box centered on x y z). task: pixel x, pixel y, answer (an entity",
    "# id or a floor point in integer cm).",
    `camera ${eye.join(",")} ${yaw} ${pitch} ${fov} ${w}x${h}`,
    ...ACTORS.map((a) => `actor ${a.join(" ")}`),
  ];
  let state = 2040;
  const next = (n) => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state % n;
  };
  let tasks = 0;
  while (tasks < TASKS) {
    // Every other task aims at a random point inside a random box (hits,
    // occlusions); the rest are uniform pixels (floor points and sky).
    let px = next(w);
    let py = next(h);
    if (tasks % 2 === 1) {
      const [, , x, y, z, sx, sy, sz] = ACTORS[next(ACTORS.length)];
      const inside = [x, y, z].map((c, k) => (c + [sx, sy, sz][k] * ((next(80) + 10) / 100 - 0.5)) * 100);
      const aimed = pixelOf(inside);
      if (aimed === null) continue;
      [px, py] = aimed;
    }
    const got = answer(px, py);
    if (got === null) continue;
    lines.push(`task ${px} ${py} ${got}`);
    tasks += 1;
  }
  return `${lines.join("\n")}\n`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const text = generate();
  if (process.argv.includes("--check")) {
    process.exit(readFileSync(GOLD_PATH, "utf8") === text ? 0 : 1);
  }
  mkdirSync(dirname(GOLD_PATH), { recursive: true });
  writeFileSync(GOLD_PATH, text);
  console.log(`wrote ${GOLD_PATH}`);
}

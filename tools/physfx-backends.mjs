// RZ10: the RZ1 eight-ragdoll soak on two physics backends, printed as one table.
//
// Usage:
//   node tools/physfx-backends.mjs [--ticks 180] [--warmup 30] [--json]
//
// The scene is the plan's soak (crates/physfx/src/tests.rs, the budget test): 8 humanoid
// ragdolls of 14 capsules each, 200 debris boxes and 6 severed limb capsules over one
// static floor, stepped at the fixed 60 Hz tick. The same scene definition feeds both
// backends, so the two rows differ only in the engine.
//
// Backends: Rapier and Jolt, both as WebAssembly in this Node process (the row says
// "beside Rapier wasm"; Jolt is the single-thread `wasm-compat` build, MIT). The donors
// are installed under `tools/vendor` (gitignored) with
// `npm install --prefix tools/vendor --no-save --no-package-lock jolt-physics@1.1.0
// @dimforge/rapier3d-compat@0.21.0`; when one is missing the tool says so loudly and
// exits 1, so a runner cannot print a table with half of it. Nothing here is authority:
// these are float bodies, never hashed.
//
// Output: one table, `backend engine bodies mean_ms p95_ms`. Exit 0 = both backends ran
// and the table printed; exit 1 = a backend failed to load or step; exit 2 = bad usage.
import { createRequire } from "node:module";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const VENDOR = join(ROOT, "tools", "vendor", "node_modules");

/** One bone of the test humanoid: name, parent, local offset (meters), capsule (radius, half-height). */
export const BONES = [
  ["pelvis", null, [0.0, 1.1, 0.0], [0.14, 0.05]],
  ["spine", 0, [0.0, 0.25, 0.0], [0.13, 0.05]],
  ["chest", 1, [0.0, 0.25, 0.0], [0.15, 0.08]],
  ["head", 2, [0.0, 0.38, 0.0], [0.11, 0.04]],
  ["upper_arm_l", 2, [-0.25, -0.1, 0.0], [0.05, 0.1]],
  ["lower_arm_l", 4, [0.0, -0.28, 0.0], [0.04, 0.1]],
  ["upper_arm_r", 2, [0.25, -0.1, 0.0], [0.05, 0.1]],
  ["lower_arm_r", 6, [0.0, -0.28, 0.0], [0.04, 0.1]],
  ["thigh_l", 0, [-0.1, -0.25, 0.0], [0.08, 0.15]],
  ["shin_l", 8, [0.0, -0.4, 0.0], [0.06, 0.15]],
  ["foot_l", 9, [0.0, -0.3, 0.0], [0.06, 0.03]],
  ["thigh_r", 0, [0.1, -0.25, 0.0], [0.08, 0.15]],
  ["shin_r", 11, [0.0, -0.4, 0.0], [0.06, 0.15]],
  ["foot_r", 12, [0.0, -0.3, 0.0], [0.06, 0.03]],
];

/** The plan soak: 8 ragdolls, 200 debris boxes, 6 severed limbs. */
export const RAGDOLLS = 8;
export const DEBRIS = 200;
export const LIMBS = 6;
export const DEFAULT_TICKS = 180;
export const DEFAULT_WARMUP = 30;
/** A flesh bone weighs what one cubic meter weighs (engine-physics material table). */
const DENSITY = 1000;
/** The fixed step the physical layer runs at. */
const DT = 1 / 60;
const GRAVITY = -9.81;

/** The world centre of every bone of ragdoll `slot`; the root local offset carries the 1.1 m
 * pelvis, lifted 0.5 m like the plan soak. */
export function boneWorld(slot) {
  const world = [];
  for (let i = 0; i < BONES.length; i++) {
    const parent = BONES[i][1];
    const off = BONES[i][2];
    const base = parent === null ? [slot * 2.5 - 8.75, 0.5, 0.0] : world[parent];
    world.push([base[0] + off[0], base[1] + off[1], base[2] + off[2]]);
  }
  return world;
}

/** The world centres of `count` debris boxes on the plan soak's half-meter grid. */
export function debrisWorld(count) {
  return Array.from({ length: count }, (_, i) => [
    (i % 20) * 0.5 - 4.75,
    2.0,
    4.0 + Math.floor(i / 20) * 0.5,
  ]);
}

/** The world centres of `count` severed limb capsules, thrown along -z. */
export function limbWorld(count) {
  return Array.from({ length: count }, (_, i) => [i - 2.5, 2.5, -3.0]);
}

/** The number of dynamic bodies the scene spawns (the same for both backends). */
export function sceneBodies() {
  return RAGDOLLS * BONES.length + DEBRIS + LIMBS;
}

/** `ms` for every entry of `samples` rounded to three decimals. */
function round3(x) {
  return Math.round(x * 1000) / 1000;
}

/** The p95 of a list of numbers (nearest-rank; the list is copied and sorted). */
export function p95(samples) {
  const sorted = [...samples].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))];
}

/** The one table: a header line and one row per backend, columns aligned. */
export function formatTable(rows) {
  const head = ["backend", "engine", "bodies", "mean_ms", "p95_ms"];
  const body = rows.map((r) => [r.backend, r.engine, String(r.bodies), round3(r.meanMs).toFixed(3), round3(r.p95Ms).toFixed(3)]);
  const widths = head.map((h, c) => Math.max(h.length, ...body.map((r) => r[c].length)));
  const line = (cells) => cells.map((cell, c) => cell.padEnd(widths[c])).join("  ").trimEnd();
  return [line(head), ...body.map(line)].join("\n");
}

/** Parse the command line: the tick count, the warm-up and `--json`. */
export function parseArgs(argv) {
  const out = { ticks: DEFAULT_TICKS, warmup: DEFAULT_WARMUP, json: false, help: false, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--ticks") {
      const n = Number(argv[++i]);
      if (!Number.isInteger(n) || n < 1) out.error = "--ticks needs a positive integer";
      else out.ticks = n;
    } else if (a === "--warmup") {
      const n = Number(argv[++i]);
      if (!Number.isInteger(n) || n < 0) out.error = "--warmup needs a non-negative integer";
      else out.warmup = n;
    } else if (a === "--json") out.json = true;
    else if (a === "--help" || a === "-h") out.help = true;
    else out.error = `unknown flag ${a}`;
    if (out.error) break;
  }
  return out;
}

/** The version string of a vendored donor, or null when it is not installed. */
export function donorVersion(spec) {
  try {
    const req = createRequire(join(VENDOR, "anchor.cjs"));
    let dir = dirname(req.resolve(spec));
    while (!existsSync(join(dir, "package.json"))) {
      const up = dirname(dir);
      if (up === dir) return null;
      dir = up;
    }
    return JSON.parse(readFileSync(join(dir, "package.json"), "utf8")).version;
  } catch {
    return null;
  }
}

/** The module namespace of one vendored ESM/CJS donor; throws when it is not installed. */
async function loadDonor(spec) {
  const req = createRequire(join(VENDOR, "anchor.cjs"));
  return await import(pathToFileURL(req.resolve(spec)).href);
}

/** Mean and p95 of `ticks` steps after `warmup` steps, as milliseconds per tick. */
function timed(warmup, ticks, step) {
  for (let t = 0; t < warmup; t++) step();
  const samples = [];
  for (let t = 0; t < ticks; t++) {
    const before = performance.now();
    step();
    samples.push(performance.now() - before);
  }
  const mean = samples.reduce((a, b) => a + b, 0) / samples.length;
  return { meanMs: mean, p95Ms: p95(samples) };
}

/** The scene on Rapier (`@dimforge/rapier3d-compat`), one impulse joint per bone pair. */
export async function runRapier({ ticks, warmup }) {
  const mod = await loadDonor("@dimforge/rapier3d-compat");
  const RAPIER = mod.default ?? mod;
  await RAPIER.init();
  const world = new RAPIER.World({ x: 0, y: GRAVITY, z: 0 });
  world.createCollider(RAPIER.ColliderDesc.cuboid(50, 0.5, 50).setTranslation(0, -0.5, 0));
  for (let slot = 0; slot < RAGDOLLS; slot++) {
    const centres = boneWorld(slot);
    const bodies = [];
    for (let i = 0; i < BONES.length; i++) {
      const [radius, half] = BONES[i][3];
      const desc = RAPIER.RigidBodyDesc.dynamic().setTranslation(centres[i][0], centres[i][1], centres[i][2]);
      const body = world.createRigidBody(desc);
      world.createCollider(RAPIER.ColliderDesc.capsule(half, radius).setDensity(DENSITY), body);
      bodies.push(body);
    }
    for (let i = 0; i < BONES.length; i++) {
      const parent = BONES[i][1];
      if (parent === null) continue;
      const anchor = {
        x: centres[i][0] - centres[parent][0],
        y: centres[i][1] - centres[parent][1],
        z: centres[i][2] - centres[parent][2],
      };
      world.createImpulseJoint(RAPIER.JointData.spherical(anchor, { x: 0, y: 0, z: 0 }), bodies[parent], bodies[i], true);
    }
    bodies[0].setLinvel({ x: 0, y: 0, z: 1 }, true);
    bodies[0].applyImpulse({ x: 0, y: 0, z: 40 + 10 * slot }, true);
  }
  for (const [x, y, z] of debrisWorld(DEBRIS)) {
    const body = world.createRigidBody(RAPIER.RigidBodyDesc.dynamic().setTranslation(x, y, z));
    world.createCollider(RAPIER.ColliderDesc.cuboid(0.1, 0.1, 0.1).setDensity(DENSITY), body);
  }
  for (const [x, y, z] of limbWorld(LIMBS)) {
    const body = world.createRigidBody(RAPIER.RigidBodyDesc.dynamic().setTranslation(x, y, z).setLinvel(0, 1, -1));
    world.createCollider(RAPIER.ColliderDesc.capsule(0.15, 0.06).setDensity(DENSITY), body);
  }
  const timing = timed(warmup, ticks, () => world.step());
  return { backend: "rapier", engine: `rapier3d-compat ${donorVersion("@dimforge/rapier3d-compat") ?? "?"} (wasm)`, bodies: sceneBodies(), ...timing };
}

/** The scene on Jolt (`jolt-physics` wasm-compat), one point constraint per bone pair. */
export async function runJolt({ ticks, warmup }) {
  const mod = await loadDonor("jolt-physics/wasm-compat");
  const initJolt = mod.default ?? mod;
  const Jolt = await initJolt();
  const settings = new Jolt.JoltSettings();
  const objects = new Jolt.ObjectLayerPairFilterTable(2);
  objects.EnableCollision(0, 1);
  objects.EnableCollision(1, 1);
  const broad = new Jolt.BroadPhaseLayerInterfaceTable(2, 2);
  broad.MapObjectToBroadPhaseLayer(0, new Jolt.BroadPhaseLayer(0));
  broad.MapObjectToBroadPhaseLayer(1, new Jolt.BroadPhaseLayer(1));
  settings.mObjectLayerPairFilter = objects;
  settings.mBroadPhaseLayerInterface = broad;
  settings.mObjectVsBroadPhaseLayerFilter = new Jolt.ObjectVsBroadPhaseLayerFilterTable(broad, 2, objects, 2);
  const jolt = new Jolt.JoltInterface(settings);
  Jolt.destroy(settings);
  const system = jolt.GetPhysicsSystem();
  const bodies = system.GetBodyInterface();
  const add = (shape, pos, dynamic) => {
    const creation = new Jolt.BodyCreationSettings(
      shape,
      new Jolt.RVec3(pos[0], pos[1], pos[2]),
      dynamic ? Jolt.Quat.prototype.sIdentity() : new Jolt.Quat(0, 0, 0, 1),
      dynamic ? Jolt.EMotionType_Dynamic : Jolt.EMotionType_Static,
      dynamic ? 1 : 0,
    );
    const body = bodies.CreateBody(creation);
    Jolt.destroy(creation);
    bodies.AddBody(body.GetID(), dynamic ? Jolt.EActivation_Activate : Jolt.EActivation_DontActivate);
    return body;
  };
  add(new Jolt.BoxShape(new Jolt.Vec3(50, 0.5, 50), 0.05, null), [0, -0.5, 0], false);
  for (let slot = 0; slot < RAGDOLLS; slot++) {
    const centres = boneWorld(slot);
    const handles = [];
    for (let i = 0; i < BONES.length; i++) {
      const [radius, half] = BONES[i][3];
      handles.push(add(new Jolt.CapsuleShape(half, radius, null), centres[i], true));
    }
    for (let i = 0; i < BONES.length; i++) {
      const parent = BONES[i][1];
      if (parent === null) continue;
      const point = new Jolt.PointConstraintSettings();
      point.mSpace = Jolt.EConstraintSpace_WorldSpace;
      point.mPoint1 = new Jolt.RVec3(centres[i][0], centres[i][1], centres[i][2]);
      point.mPoint2 = new Jolt.RVec3(centres[i][0], centres[i][1], centres[i][2]);
      const constraint = bodies.CreateConstraint(point, handles[parent].GetID(), handles[i].GetID());
      system.AddConstraint(constraint);
    }
    bodies.SetLinearVelocity(handles[0].GetID(), new Jolt.Vec3(0, 0, 1));
    bodies.AddImpulse(handles[0].GetID(), new Jolt.Vec3(0, 0, 40 + 10 * slot));
  }
  const box = new Jolt.BoxShape(new Jolt.Vec3(0.1, 0.1, 0.1), 0.05, null);
  for (const pos of debrisWorld(DEBRIS)) add(box, pos, true);
  const limb = new Jolt.CapsuleShape(0.15, 0.06, null);
  for (const pos of limbWorld(LIMBS)) {
    const body = add(limb, pos, true);
    bodies.SetLinearVelocity(body.GetID(), new Jolt.Vec3(0, 1, -1));
  }
  system.OptimizeBroadPhase();
  const timing = timed(warmup, ticks, () => jolt.Step(DT, 1));
  return { backend: "jolt", engine: `jolt-physics ${donorVersion("jolt-physics") ?? "?"} (wasm-compat)`, bodies: sceneBodies(), ...timing };
}

const USAGE = [
  "usage: node tools/physfx-backends.mjs [--ticks N] [--warmup N] [--json]",
  "  --ticks N   measured ticks per backend (default 180)",
  "  --warmup N  ticks before the timer starts (default 30)",
  "  --json      one JSON object per line instead of the table",
].join("\n");

/** Install hint for a donor that is not under tools/vendor. */
function installHint() {
  return "run `npm install --prefix tools/vendor --no-save --no-package-lock jolt-physics@1.1.0 @dimforge/rapier3d-compat@0.21.0`";
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(USAGE);
    return 0;
  }
  if (args.error) {
    console.error(`physfx-backends: ${args.error}\n${USAGE}`);
    return 2;
  }
  const rows = [];
  let failed = false;
  for (const [name, run] of [["rapier", runRapier], ["jolt", runJolt]]) {
    try {
      rows.push(await run(args));
    } catch (error) {
      failed = true;
      console.error(`physfx-backends: ${name} backend did not run: ${error && error.message ? error.message : error}`);
      console.error(`physfx-backends: ${name} ${installHint()}`);
    }
  }
  if (args.json) {
    for (const row of rows) console.log(JSON.stringify(row));
  } else {
    console.log(`physfx backends: scene ${RAGDOLLS} ragdolls + ${DEBRIS} debris + ${LIMBS} limbs (${sceneBodies()} dynamic bodies), ${args.ticks} ticks at 60 Hz`);
    if (rows.length > 0) console.log(formatTable(rows));
  }
  return failed || rows.length < 2 ? 1 : 0;
}

const invoked = Boolean(process.argv[1]) && import.meta.url === pathToFileURL(process.argv[1]).href;
if (invoked) process.exitCode = await main();

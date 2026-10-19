// Tests for tools/physfx-backends.mjs: the shared scene is the RZ1 scene (checked against
// the Rust source), the table formats, and the argument parser. The engines themselves are
// donors under tools/vendor and are exercised by the runner job, not here.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  BONES,
  DEBRIS,
  DEFAULT_TICKS,
  DEFAULT_WARMUP,
  LIMBS,
  RAGDOLLS,
  boneWorld,
  debrisWorld,
  donorVersion,
  formatTable,
  limbWorld,
  p95,
  parseArgs,
  sceneBodies,
} from "./physfx-backends.mjs";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));

test("the JS scene is the Rust scene: every bone rows matches crates/physfx/src/tests.rs", () => {
  const source = readFileSync(join(ROOT, "crates", "physfx", "src", "tests.rs"), "utf8");
  const rows = [...source.matchAll(/^\s*\("(\w+)",\s*(None|Some\((\d+)\)),\s*\[([^\]]+)\],\s*\(([^)]+)\)\),/gm)];
  assert.equal(rows.length, BONES.length, "tests.rs has a different number of bones");
  rows.forEach((m, i) => {
    const offset = m[4].split(",").map((s) => Number(s.trim()));
    const [radius, half] = m[5].split(",").map((s) => Number(s.trim()));
    assert.equal(m[1], BONES[i][0], `bone ${i} name`);
    assert.equal(m[3] === undefined ? null : Number(m[3]), BONES[i][1], `bone ${i} parent`);
    assert.deepEqual(offset, BONES[i][2], `bone ${i} offset`);
    assert.deepEqual([radius, half], BONES[i][3], `bone ${i} capsule`);
  });
  // The plan soak counts come from the budget test in the same file.
  assert.match(source, /for slot in 0\.\.8u8/, "tests.rs no longer soaks 8 ragdolls");
  assert.match(source, /debris_field\(200, 2\.0\)/, "tests.rs no longer soaks 200 debris");
  assert.match(source, /for part in 0\.\.6u32/, "tests.rs no longer soaks 6 limbs");
  assert.equal(RAGDOLLS, 8);
  assert.equal(DEBRIS, 200);
  assert.equal(LIMBS, 6);
});

test("boneWorld lays the humanoid out from the root down", () => {
  const world = boneWorld(0);
  assert.equal(world.length, BONES.length);
  assert.deepEqual(world[0], [-8.75, 1.6, 0], "pelvis");
  assert.deepEqual(world[3], [-8.75, 2.48, 0], "head: pelvis + 0.25 + 0.25 + 0.38");
  assert.equal(world[9][0], -8.85, "shin_l x: pelvis - 0.1 (thigh)");
  assert.ok(Math.abs(world[9][1] - 0.95) < 1e-9, "shin_l y: pelvis - 0.25 (thigh) - 0.4 (shin)");
  assert.equal(world[9][2], 0);
  const shifted = boneWorld(1);
  assert.deepEqual(shifted[0], [world[0][0] + 2.5, world[0][1], world[0][2]], "slot 1 is one grid step over");
});

test("the debris grid and the limbs are where the plan soak puts them", () => {
  const debris = debrisWorld(DEBRIS);
  assert.equal(debris.length, 200);
  assert.deepEqual(debris[0], [-4.75, 2.0, 4.0]);
  assert.deepEqual(debris[19], [4.75, 2.0, 4.0], "20 per row");
  assert.deepEqual(debris[20], [-4.75, 2.0, 4.5], "the next row steps z by 0.5");
  assert.equal(debrisWorld(0).length, 0);
  const limbs = limbWorld(LIMBS);
  assert.equal(limbs.length, 6);
  assert.deepEqual(limbs[0], [-2.5, 2.5, -3.0]);
  assert.deepEqual(limbs[5], [2.5, 2.5, -3.0]);
});

test("sceneBodies is 8 x 14 + 200 + 6", () => {
  assert.equal(sceneBodies(), 318);
});

test("formatTable lines up the columns and rounds to three decimals", () => {
  const table = formatTable([
    { backend: "rapier", engine: "rapier3d-compat 0.21.0 (wasm)", bodies: 318, meanMs: 6.4837, p95Ms: 7.1 },
    { backend: "jolt", engine: "jolt-physics 1.1.0 (wasm-compat)", bodies: 318, meanMs: 8.635, p95Ms: 9.2345 },
  ]);
  const [head, first, second] = table.split("\n");
  assert.match(head, /^backend\s+engine\s+bodies\s+mean_ms\s+p95_ms$/);
  assert.ok(first.startsWith("rapier"));
  assert.ok(second.startsWith("jolt"));
  assert.match(first, /318\s+6\.484\s+7\.100$/);
  assert.match(second, /318\s+8\.635\s+9\.235$/);
  assert.equal(table.split("\n").length, 3);
});

test("p95 is the nearest rank", () => {
  assert.equal(p95([1, 2, 3, 4, 5]), 5);
  assert.equal(p95([5, 4, 3, 2, 1]), 5);
  assert.equal(p95([1]), 1);
  assert.equal(p95(Array.from({ length: 100 }, (_, i) => i)), 95);
});

test("parseArgs reads the flags, refuses a bad tick count and defaults to the plan soak", () => {
  assert.deepEqual(parseArgs([]), { ticks: DEFAULT_TICKS, warmup: DEFAULT_WARMUP, json: false, help: false, error: null });
  assert.equal(DEFAULT_TICKS, 180);
  assert.deepEqual(parseArgs(["--ticks", "60", "--warmup", "0", "--json"]), { ticks: 60, warmup: 0, json: true, help: false, error: null });
  assert.equal(parseArgs(["--help"]).help, true);
  assert.match(parseArgs(["--ticks", "0"]).error, /positive integer/);
  assert.match(parseArgs(["--ticks", "x"]).error, /positive integer/);
  assert.match(parseArgs(["--warmup", "-1"]).error, /non-negative integer/);
  assert.match(parseArgs(["--nope"]).error, /unknown flag --nope/);
});

test("donorVersion is null for a package that is not installed", () => {
  assert.equal(donorVersion("not-a-real-physics-donor-xyz"), null);
});

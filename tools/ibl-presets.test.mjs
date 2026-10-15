// tools/ibl-presets.test.mjs drills: the IBL sky-preset fixture validator.
// Donor idea-only: bevyengine/bevy (Apache-2.0) bakes one IBL set per sky
// (sky-to-IBL bake per environment) instead of sharing one fixture bake;
// each preset in fixtures/ibl-presets.json names its own IBL bake for the
// same reason. No donor code is copied: the ranges below mirror
// crates/render/src/sky.rs (turbidity 1.5..=10, albedo 0..=1, capture edge
// at most 256). Never runs cargo.
//   node --test tools/ibl-presets.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURE = join(ROOT, "fixtures", "ibl-presets.json");

const TURBIDITY_MIN = 1.5;
const TURBIDITY_MAX = 10.0;
const CAPTURE_MAX_DIM = 256;

function loadFixture() {
  return JSON.parse(readFileSync(FIXTURE, "utf8"));
}

function validatePresets(doc) {
  assert.equal(doc.version, 1, "fixture version is 1");
  assert.ok(Array.isArray(doc.presets), "presets is an array");
  assert.equal(doc.presets.length, 3, "3 sky presets, one IBL bake each");
  const names = new Set();
  const bakes = new Set();
  for (const p of doc.presets) {
    assert.ok(p.name && typeof p.name === "string", "preset names itself");
    assert.ok(p.ibl_bake && typeof p.ibl_bake === "string", `${p.name} names its own IBL bake`);
    assert.ok(!names.has(p.name), `duplicate preset name ${p.name}`);
    assert.ok(!bakes.has(p.ibl_bake), `shared IBL bake ${p.ibl_bake}: each sky bakes its own`);
    names.add(p.name);
    bakes.add(p.ibl_bake);
    const dir = p.sun_dir;
    assert.ok(Array.isArray(dir) && dir.length === 3, `${p.name} sun_dir has 3 parts`);
    assert.ok(dir.every((c) => Number.isFinite(c)), `${p.name} sun_dir is finite`);
    const len2 = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    assert.ok(len2 >= 1e-12, `${p.name} sun_dir is nonzero`);
    assert.ok(Number.isFinite(p.turbidity) && p.turbidity >= TURBIDITY_MIN && p.turbidity <= TURBIDITY_MAX, `${p.name} turbidity in 1.5..=10`);
    assert.ok(Array.isArray(p.ground_albedo) && p.ground_albedo.length === 3, `${p.name} albedo has 3 parts`);
    assert.ok(p.ground_albedo.every((c) => Number.isFinite(c) && c >= 0 && c <= 1), `${p.name} albedo in 0..=1`);
  }
  assert.ok(Number.isInteger(doc.capture_w) && doc.capture_w > 0 && doc.capture_w <= CAPTURE_MAX_DIM, "capture width in 1..=256");
  assert.ok(Number.isInteger(doc.capture_h) && doc.capture_h > 0 && doc.capture_h <= CAPTURE_MAX_DIM, "capture height in 1..=256");
  return { names: [...names], bakes: [...bakes] };
}

test("fixture holds 3 sky presets, each with its own IBL bake", () => {
  const seen = validatePresets(loadFixture());
  assert.equal(new Set(seen.names).size, 3, "3 preset names: 1 shared rig before -> 3");
  assert.equal(new Set(seen.bakes).size, 3, "3 IBL bakes: 1 shared bake before -> 3");
});

test("validator refuses a shared IBL bake across two skies", () => {
  const doc = loadFixture();
  doc.presets[1].ibl_bake = doc.presets[0].ibl_bake;
  assert.throws(() => validatePresets(doc), /shared IBL bake/);
});

test("validator refuses out-of-range turbidity", () => {
  const doc = loadFixture();
  doc.presets[2].turbidity = 99;
  assert.throws(() => validatePresets(doc), /turbidity/);
});

test("validator refuses a duplicate preset name", () => {
  const doc = loadFixture();
  doc.presets[2].name = doc.presets[0].name;
  assert.throws(() => validatePresets(doc), /duplicate preset name/);
});


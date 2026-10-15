// tools/sky-presets.test.mjs drills: the analytic sky-preset fixture validator.
// Donor idea-only: Hosek-Wilkie analytic sky (turbidity-driven daylight model,
// one parameter set per sky condition) inspires one analytic preset per arena
// condition instead of a single shared gradient rig; no donor coefficients or
// code are copied. Companion to fixtures/ibl-presets.json: each sky preset
// names its own IBL bake, so sky and IBL can never drift apart. Ranges below
// mirror crates/render/src/sky.rs (turbidity 1.5..=10, albedo 0..=1, capture
// edge at most 256). Never runs cargo.
//   node --test tools/sky-presets.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const FIXTURE = join(ROOT, "fixtures", "sky-presets.json");
const IBL_FIXTURE = join(ROOT, "fixtures", "ibl-presets.json");

const TURBIDITY_MIN = 1.5;
const TURBIDITY_MAX = 10.0;
const CAPTURE_MAX_DIM = 256;

function loadJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function validatePresets(doc, iblDoc) {
  assert.equal(doc.version, 1, "fixture version is 1");
  assert.equal(doc.model, "analytic", "sky model is analytic");
  assert.ok(Array.isArray(doc.presets), "presets is an array");
  assert.equal(doc.presets.length, 3, "3 analytic sky presets, one IBL bake each");
  const knownBakes = new Set(iblDoc.presets.map((p) => p.ibl_bake));
  const names = new Set();
  const bakes = new Set();
  for (const p of doc.presets) {
    assert.ok(p.name && typeof p.name === "string", "preset names itself");
    assert.ok(p.ibl_bake && typeof p.ibl_bake === "string", `${p.name} names its own IBL bake`);
    assert.ok(!names.has(p.name), `duplicate preset name ${p.name}`);
    assert.ok(!bakes.has(p.ibl_bake), `shared IBL bake ${p.ibl_bake}: each sky bakes its own`);
    assert.ok(knownBakes.has(p.ibl_bake), `unknown IBL bake ${p.ibl_bake}: sky must name a bake from fixtures/ibl-presets.json`);
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
    assert.ok(Number.isFinite(p.exposure_ev100), `${p.name} exposure_ev100 is finite`);
  }
  assert.ok(Number.isInteger(doc.capture_w) && doc.capture_w > 0 && doc.capture_w <= CAPTURE_MAX_DIM, "capture width in 1..=256");
  assert.ok(Number.isInteger(doc.capture_h) && doc.capture_h > 0 && doc.capture_h <= CAPTURE_MAX_DIM, "capture height in 1..=256");
  return { names: [...names], bakes: [...bakes] };
}

test("fixture holds 3 analytic sky presets, each with its own IBL bake", () => {
  const seen = validatePresets(loadJson(FIXTURE), loadJson(IBL_FIXTURE));
  assert.equal(new Set(seen.names).size, 3, "3 preset names: 1 gradient rig before -> 3 analytic");
  assert.equal(new Set(seen.bakes).size, 3, "3 IBL bakes: 1 shared bake before -> 3");
});

test("validator refuses a shared IBL bake across two skies", () => {
  const doc = loadJson(FIXTURE);
  doc.presets[1].ibl_bake = doc.presets[0].ibl_bake;
  assert.throws(() => validatePresets(doc, loadJson(IBL_FIXTURE)), /shared IBL bake/);
});

test("validator refuses an IBL bake missing from the companion fixture", () => {
  const doc = loadJson(FIXTURE);
  doc.presets[2].ibl_bake = "no-such-bake";
  assert.throws(() => validatePresets(doc, loadJson(IBL_FIXTURE)), /unknown IBL bake/);
});

test("validator refuses out-of-range turbidity", () => {
  const doc = loadJson(FIXTURE);
  doc.presets[2].turbidity = 99;
  assert.throws(() => validatePresets(doc, loadJson(IBL_FIXTURE)), /turbidity/);
});

test("validator refuses a duplicate preset name", () => {
  const doc = loadJson(FIXTURE);
  doc.presets[2].name = doc.presets[0].name;
  assert.throws(() => validatePresets(doc, loadJson(IBL_FIXTURE)), /duplicate preset name/);
});

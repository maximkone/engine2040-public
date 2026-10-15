// tools/sky-presets3.test.mjs covers day/dusk/night content presets as distinct IBL bakes.
// Companion to tools/sky-preset.test.mjs (wave-2 file, left untouched): day stays, dusk and night extend the ladder. Never runs cargo.
//   node --test tools/sky-preset*.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

function load(file) {
  const url = new URL(`../content/sky/${file}`, import.meta.url);
  return JSON.parse(readFileSync(url, "utf8"));
}

function checkSane(p) {
  assert.equal(p.kind, "sky_ibl", `${p.name} kind is sky_ibl`);
  assert.ok(p.sun_elevation_deg >= -90 && p.sun_elevation_deg <= 85, `${p.name} sun elevation sane`);
  assert.ok(p.sun_azimuth_deg >= 0 && p.sun_azimuth_deg <= 360, `${p.name} sun azimuth sane`);
  assert.equal(p.sh_order, 2, `${p.name} SH order 2`);
  assert.equal(p.sh_coeffs, 9, `${p.name} SH coeffs 9`);
  assert.equal(p.sh_coeffs, (p.sh_order + 1) * (p.sh_order + 1), `${p.name} SH coeffs match order`);
  assert.ok(p.exposure_ev100 >= 4 && p.exposure_ev100 <= 16, `${p.name} exposure sane`);
  assert.ok(p.turbidity >= 1.5 && p.turbidity <= 10, `${p.name} turbidity bounds`);
  assert.ok(p.capture_w > 0 && p.capture_w <= 256 && p.capture_h > 0 && p.capture_h <= 256, `${p.name} capture bounds`);
  assert.ok(p.albedo_ground >= 0 && p.albedo_ground <= 1, `${p.name} albedo bounds`);
  assert.ok(Number.isFinite(p.budget_ms) && p.budget_ms > 0, `${p.name} budget positive`);
}

test("day/dusk/night presets parse with sane sun/SH/exposure and stay distinct", () => {
  const day = load("preset_day.json");
  const dusk = load("preset_dusk.json");
  const night = load("preset_night.json");
  assert.equal(day.name, "day");
  assert.equal(dusk.name, "dusk");
  assert.equal(night.name, "night");
  for (const p of [day, dusk, night]) checkSane(p);
  const elev = [day.sun_elevation_deg, dusk.sun_elevation_deg, night.sun_elevation_deg];
  assert.equal(new Set(elev).size, 3, "sun elevations distinct");
  const expo = [day.exposure_ev100, dusk.exposure_ev100, night.exposure_ev100];
  assert.equal(new Set(expo).size, 3, "exposures distinct");
  const azim = [day.sun_azimuth_deg, dusk.sun_azimuth_deg, night.sun_azimuth_deg];
  assert.equal(new Set(azim).size, 3, "sun azimuths distinct");
  assert.ok(day.sun_elevation_deg > dusk.sun_elevation_deg, "dusk sun lower than day");
  assert.ok(dusk.sun_elevation_deg > night.sun_elevation_deg, "night sun lowest");
  assert.ok(day.exposure_ev100 > dusk.exposure_ev100, "dusk darker than day");
  assert.ok(dusk.exposure_ev100 > night.exposure_ev100, "night darkest");
});

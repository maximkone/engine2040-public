import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
test("day sky preset parses with sane ranges", () => {
  const url = new URL("../content/sky/preset_day.json", import.meta.url);
  const raw = readFileSync(url, "utf8");
  const p = JSON.parse(raw);
  assert.equal(p.name, "day");
  assert.ok(p.sun_elevation_deg >= 5 && p.sun_elevation_deg <= 85, "sun elevation day range");
  assert.ok(p.sun_azimuth_deg >= 0 && p.sun_azimuth_deg <= 360, "sun azimuth range");
  assert.equal(p.sh_order, 2);
  assert.equal(p.sh_coeffs, 9);
  assert.equal(p.sh_coeffs, (p.sh_order + 1) * (p.sh_order + 1));
  assert.ok(p.exposure_ev100 >= 8 && p.exposure_ev100 <= 16, "exposure sane");
  assert.ok(p.turbidity >= 1.5 && p.turbidity <= 10, "turbidity bounds");
  assert.ok(p.capture_w <= 256 && p.capture_h <= 256, "capture bounds");
});

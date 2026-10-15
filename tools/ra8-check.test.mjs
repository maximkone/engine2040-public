// Run: node --test tools/ra8-check.test.mjs
// RA8/q1 checker gate: q1 PNG count plus manifest presence for 3 fighter captures.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { checkRa8, EXPECTED_RA8 } from "./ra8-check.mjs";
test("missing is a finding, not a throw (empty dir, today q1 state)", () => {
  const dir = mkdtempSync(join(tmpdir(), "ra8-empty-"));
  const r = checkRa8(dir);
  assert.equal(r.pngCount, 0);
  assert.equal(r.manifestPresent, false);
  assert.equal(r.manifestStatus, "missing");
  assert.ok(r.findings.length >= 2);
  assert.ok(r.findings.some((f) => f.includes("skipped:")));
  assert.equal(EXPECTED_RA8.fighterPng, 3);
});
test("present case passes (3 pngs plus one ra8 manifest)", () => {
  const dir = mkdtempSync(join(tmpdir(), "ra8-full-"));
  writeFileSync(join(dir, "v1-hero.png"), "a");
  writeFileSync(join(dir, "v2-arena-day.png"), "b");
  writeFileSync(join(dir, "v3-fight.png"), "c");
  writeFileSync(join(dir, "ra8-look.json"), "{}");
  const r = checkRa8(dir);
  assert.equal(r.pngCount, 3);
  assert.deepEqual(r.pngFiles, ["v1-hero.png", "v2-arena-day.png", "v3-fight.png"]);
  assert.equal(r.ra8Count, 1);
  assert.equal(r.manifestPresent, true);
  assert.equal(r.manifestStatus, "present");
  assert.equal(r.findings.length, 0);
});
test("live q1 today: 0 top-level PNGs, manifest missing", () => {
  const r = checkRa8();
  assert.equal(r.pngCount, 0, "q1 png count " + r.pngCount);
  assert.equal(r.manifestPresent, false);
  assert.equal(r.manifestStatus, "missing");
  assert.equal(r.expectedPng, 3);
  assert.ok(r.findings.some((f) => f.includes("skipped:")));
});


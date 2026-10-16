import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { checkQ1, EXPECTED_Q1, SYNTH_REL, ORBIT_REL } from "./q1-check.mjs";

// The live target/q1 tree is untracked local captures, absent on a fresh
// machine, so every case below builds its own fixture root instead.
function makeFixture({ synth = 4, orbit = 30, manifests = true, ra8 = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "q1-"));
  const synthDir = join(root, SYNTH_REL);
  const orbitDir = join(root, ORBIT_REL);
  mkdirSync(synthDir, { recursive: true });
  mkdirSync(orbitDir, { recursive: true });
  for (let i = 0; i < synth; i++) writeFileSync(join(synthDir, "synth_" + i + ".png"), "");
  for (let i = 0; i < orbit; i++) writeFileSync(join(orbitDir, "orbit_" + i + ".png"), "");
  if (manifests) {
    const body = ra8 ? "ra8 reference-gpu look manifest" : "cpu staged stills manifest";
    writeFileSync(join(synthDir, "manifest.json"), body);
    writeFileSync(join(orbitDir, "manifest.json"), body);
  }
  return root;
}

test("q1 fixture pins the expected capture counts", () => {
  const r = checkQ1(makeFixture());
  assert.equal(r.synthPng, 4, "synth png count " + r.synthPng);
  assert.equal(r.orbitPng, 30, "orbit png count " + r.orbitPng);
  assert.equal(r.totalPng, 34, "total png count " + r.totalPng);
  assert.equal(r.manifests, 2, "manifest count " + r.manifests);
  assert.equal(r.ra8Manifests, 0, "RA8 manifests pinned as gap, got " + r.ra8Manifests);
  assert.deepEqual(r.gaps, ["skipped: RA8 reference-GPU look manifest absent"]);
  assert.equal(EXPECTED_Q1.synthPng, 4);
  assert.equal(EXPECTED_Q1.orbitPng, 30);
  assert.equal(EXPECTED_Q1.ra8Manifests, 0);
});

test("missing capture dirs warn and report zeros, never throw", () => {
  const r = checkQ1(mkdtempSync(join(tmpdir(), "q1-empty-")));
  assert.equal(r.synthPng, 0);
  assert.equal(r.orbitPng, 0);
  assert.equal(r.manifests, 0);
  assert.ok(r.gaps.length > 0, "gaps name the missing captures");
});

test("an ra8 manifest counts toward the ra8 pin", () => {
  const r = checkQ1(makeFixture({ ra8: true }));
  assert.equal(r.ra8Manifests, 2);
  assert.ok(r.gaps.some((g) => g === "ra8 2 != 0"), "gaps were " + JSON.stringify(r.gaps));
});

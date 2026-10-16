// Run: node --test tools/iqa-gate.test.mjs
// Pins the PQ4 gate: equal 10000/pass, +20% sand <9800/fail, rerun identical, donor keys.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DEFAULT_THRESHOLD, sandFrames, scorePair } from "./iqa-gate.mjs";
const TOOL = fileURLToPath(new URL("./iqa-gate.mjs", import.meta.url));
const KEYS = ["ssim_x10000", "ssimulacra2", "butteraugli"];
test("equal sand scores 10000 with verdict pass", () => {
  const { ref, w, h } = sandFrames();
  const j = scorePair(ref, ref, w, h);
  assert.equal(j.ssim_x10000, 10000);
  assert.equal(j.verdict, "pass");
  assert.equal(j.mean_de_x100, 0);
  for (const k of KEYS) assert.ok(k in j);
  assert.equal(j.ssimulacra2, null);
  assert.equal(j.butteraugli, null);
});
test("+20% sand scores below 9800 with verdict fail", () => {
  const { ref, cap, w, h } = sandFrames();
  const j = scorePair(ref, cap, w, h, DEFAULT_THRESHOLD);
  assert.ok(j.ssim_x10000 < 9800, `ssim ${j.ssim_x10000}`);
  assert.equal(j.verdict, "fail");
  assert.ok(j.mean_de_x100 > 0);
});
test("rerun of the same pair is byte-identical", () => {
  const { ref, cap, w, h } = sandFrames();
  assert.equal(JSON.stringify(scorePair(ref, cap, w, h)), JSON.stringify(scorePair(ref, cap, w, h)));
});
test("selfcheck exits 0 with donor keys and stable stdout", () => {
  const r = spawnSync(process.execPath, [TOOL, "--selfcheck"], { encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr);
  const j = JSON.parse(r.stdout);
  for (const k of KEYS) assert.ok(k in j);
  assert.ok(j.ssim_x10000 < 9800);
  const again = spawnSync(process.execPath, [TOOL, "--selfcheck"], { encoding: "utf8" });
  assert.equal(again.stdout, r.stdout);
});

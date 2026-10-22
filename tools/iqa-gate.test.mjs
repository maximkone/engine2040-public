// Run: node --test tools/iqa-gate.test.mjs
// Pins the PQ4 gate: equal 10000/pass, +20% sand <9800/fail, rerun identical, donor keys.
// FLIP beside SSIM (LF15): equal 0, +20% sand 1268 and a 1 px shift 321 (the
// same pins as crates/playtest/src/look/metric.rs), and the E7 table matches.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DEFAULT_THRESHOLD, FLIP_ONE, SRGB_E7, SSIMULACRA2_ONE, flipX10000, sandFrames, scorePair, ssimulacra2X10000 } from "./iqa-gate.mjs";
const TOOL = fileURLToPath(new URL("./iqa-gate.mjs", import.meta.url));
const KEYS = ["ssim_x10000", "ssimulacra2", "butteraugli"];
test("equal sand scores 10000 with verdict pass", () => {
  const { ref, w, h } = sandFrames();
  const j = scorePair(ref, ref, w, h);
  assert.equal(j.ssim_x10000, 10000);
  assert.equal(j.verdict, "pass");
  assert.equal(j.mean_de_x100, 0);
  for (const k of KEYS) assert.ok(k in j);
  assert.equal(j.ssimulacra2, SSIMULACRA2_ONE);
  assert.equal(j.butteraugli, null);
});
test("+20% sand scores below 9800 with verdict fail", () => {
  const { ref, cap, w, h } = sandFrames();
  const j = scorePair(ref, cap, w, h, DEFAULT_THRESHOLD);
  assert.ok(j.ssim_x10000 < 9800, `ssim ${j.ssim_x10000}`);
  assert.equal(j.verdict, "fail");
  assert.ok(j.mean_de_x100 > 0);
});
test("ssimulacra2 twin: identical 10000, the sand pair near the Rust donor", () => {
  const { ref, cap, w, h } = sandFrames();
  assert.equal(ssimulacra2X10000(ref, ref, w, h), SSIMULACRA2_ONE);
  const worse = ssimulacra2X10000(ref, cap, w, h);
  assert.ok(worse < SSIMULACRA2_ONE, `ssimulacra2 ${worse}`);
  assert.ok(worse > 0, `ssimulacra2 ${worse}`);
  // The Rust donor is float and not bit-reproducible across machines, so the
  // twin and the donor agree inside a band, never on one exact number.
  assert.ok(worse >= 6200 && worse <= 6600, `ssimulacra2 ${worse}, outside the band`);
  assert.equal(ssimulacra2X10000(ref, cap, w, h), worse);
});
test("ssimulacra2 twin refuses a pair it cannot score", () => {
  const { ref, w, h } = sandFrames();
  const small = Buffer.alloc(32 * 24 * 4, 128);
  assert.throws(() => ssimulacra2X10000(ref, small, w, h), /size mismatch/);
  const tiny = Buffer.alloc(4 * 4 * 4, 128);
  assert.throws(() => ssimulacra2X10000(tiny, tiny, 4, 4), /under 8x8/);
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
test("flip: equal sand 0, +20% sand 1268 and a 1 px shift 321 (the Rust pins)", () => {
  const { ref, cap, w, h } = sandFrames();
  assert.equal(flipX10000(ref, ref, w, h).flip_x10000, 0);
  const bright = flipX10000(cap, ref, w, h);
  assert.equal(bright.flip_x10000, 1268);
  assert.equal(bright.flip_max_x10000, 2270);
  assert.equal(bright.pixels, w * h);
  assert.ok(bright.flip_x10000 <= FLIP_ONE);
  const shifted = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const p = (y * w + x) * 4, q = (y * w + Math.min(x + 1, w - 1)) * 4; shifted.set(ref.subarray(q, q + 3), p); shifted[p + 3] = 255; }
  assert.equal(flipX10000(shifted, ref, w, h).flip_x10000, 321);
  assert.equal(JSON.stringify(flipX10000(cap, ref, w, h)), JSON.stringify(flipX10000(cap, ref, w, h)));
  assert.ok("flip_x10000" in scorePair(ref, cap, w, h));
});
test("the E7 sRGB table matches metric.rs", () => {
  const src = readFileSync(new URL("../crates/playtest/src/look/metric.rs", import.meta.url), "utf8");
  const block = src.match(/SRGB_LINEAR_E7\s*:\s*\[u32;\s*256\]\s*=\s*\[([\s\S]*?)\];/)[1];
  const rust = block.replace(/_/g, "").match(/\d+/g).map(Number);
  assert.equal(rust.length, 256);
  assert.deepEqual(SRGB_E7, rust);
});

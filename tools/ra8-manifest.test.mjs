// Run: node --test tools/ra8-manifest.test.mjs
// RA8 manifest generator gate: validates required fields, tick coverage,
// hash format; generates a real sample from target/q1/captures (read-only).
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync, readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { validateRa8Manifest, generateRa8Manifest, hashBytes } from "./ra8-manifest.mjs";
const HERE = dirname(fileURLToPath(import.meta.url));
const Q1_CAPTURES = join(HERE, "..", "target", "q1", "captures");
function minimal() {
  const h = hashBytes(Buffer.from("ra8-sample"));
  return {
    schema: 1, kind: "ra8-look", source_dir: Q1_CAPTURES,
    renderer: "headless-synth", device: "staged", seed: 2040,
    width: 64, height: 48, fps: 30,
    frames: [
      { tick: 0, file: "frame_000000.png", bytes: h.bytes, sha256: h.sha256, png_fnv: h.png_fnv, checkpoint: "a5853bc7deb368a6" },
      { tick: 30, file: "frame_000001.png", bytes: h.bytes, sha256: h.sha256, png_fnv: h.png_fnv },
    ],
    sequence_fnv: "cbf29ce484222325", honesty: "test sample",
  };
}
test("valid manifest passes (required fields, tick coverage, hash format)", () => {
  const v = validateRa8Manifest(minimal());
  assert.equal(v.ok, true, JSON.stringify(v.errors));
});
test("missing required field fails", () => {
  const m = minimal(); delete m.renderer;
  const v = validateRa8Manifest(m);
  assert.equal(v.ok, false);
  assert.ok(v.errors.some((e) => e.includes("renderer")));
});
test("bad hash format fails", () => {
  const m = minimal(); m.frames[0].sha256 = "ZZZ";
  const v = validateRa8Manifest(m);
  assert.equal(v.ok, false);
  assert.ok(v.errors.some((e) => e.includes("sha256")));
});
test("duplicate tick fails tick coverage", () => {
  const m = minimal(); m.frames[1].tick = 0;
  const v = validateRa8Manifest(m);
  assert.equal(v.ok, false);
  assert.ok(v.errors.some((e) => e.includes("tick")));
});
test("generates sample from disk PNGs, validates, quotes sample", () => {
  const dir = mkdtempSync(join(tmpdir(), "ra8-src-"));
  const ticks = [0, 30, 60, 90];
  const srcFrames = ticks.map((tick, i) => {
    const name = `frame_${String(i).padStart(6, "0")}.png`;
    const png = Buffer.from(`synth-frame-${i}-tick-${tick}`);
    writeFileSync(join(dir, name), png);
    const h = hashBytes(png);
    return { tick, file: name, bytes: h.bytes, sha256: h.sha256, png_fnv: h.png_fnv };
  });
  writeFileSync(join(dir, "manifest.json"), JSON.stringify({
    renderer: "test-synth", seed: 2040, width: 64, height: 48, fps: 30,
    frames: srcFrames,
  }));
  const m = generateRa8Manifest(dir, {});
  const v = validateRa8Manifest(m);
  assert.equal(v.ok, true, JSON.stringify(v.errors));
  assert.equal(m.frames.length, 4);
  assert.deepEqual(m.frames.map((f) => f.tick), [0, 30, 60, 90]);
  const out = join(mkdtempSync(join(tmpdir(), "ra8-")), "ra8-sample.json");
  writeFileSync(out, JSON.stringify(m, null, 2));
  const back = JSON.parse(readFileSync(out, "utf8"));
  assert.equal(back.sequence_fnv, m.sequence_fnv);
  console.log("RA8_SAMPLE " + JSON.stringify({ frames: m.frames.length, sequence_fnv: m.sequence_fnv, first_sha256: m.frames[0].sha256, honesty: m.honesty }));
  if (existsSync(Q1_CAPTURES)) {
    const real = generateRa8Manifest(Q1_CAPTURES, {});
    const rv = validateRa8Manifest(real);
    assert.equal(rv.ok, true, JSON.stringify(rv.errors));
  }
});
test("never fakes: tampered source hash is rejected", () => {
  const dir = mkdtempSync(join(tmpdir(), "ra8-tamper-"));
  const png = Buffer.from("real-bytes");
  writeFileSync(join(dir, "frame_000000.png"), png);
  const h = hashBytes(png);
  assert.equal(h.sha256.length, 64);
  writeFileSync(join(dir, "manifest.json"), JSON.stringify({
    renderer: "x", seed: 1, width: 8, height: 8, fps: 30,
    frames: [{ tick: 0, file: "frame_000000.png", bytes: h.bytes, sha256: "0".repeat(64), png_fnv: h.png_fnv }],
  }));
  assert.throws(() => generateRa8Manifest(dir, {}), /mismatch/);
});
test("refuses to write into target/", () => {
  const node = process.execPath;
  const script = join(HERE, "ra8-manifest.mjs");
  assert.throws(() => execFileSync(node, [script, "--src", Q1_CAPTURES, "--out", join(HERE, "..", "target", "ra8.json")], { stdio: "pipe" }), /./);
});

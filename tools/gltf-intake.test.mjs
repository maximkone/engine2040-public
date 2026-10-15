// tools/gltf-intake.mjs drills: the licence gate and the pure helpers.
// Importing the tool must not need tools/node (heavy deps stay lazy);
// intake/validateBytes still need the install, so drills cover the
// offline surface only: licence lookup, read-only paths, hashes, counts.
//   node --test tools/gltf-intake.test.mjs
import assert from "node:assert/strict";
import { mkdtempSync as mkdtempRaw, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  ACCEPTED_LICENCE,
  checkBufferBounds,
  countTriangles,
  insideDir,
  resolveLicence,
  sha256,
} from "./gltf-intake.mjs";

// Remove every temp folder this file makes when it exits (test temp folders piled up in %TEMP%).
const madeDirs = [];
const mkdtempSync = (prefix, options) => { const dir = mkdtempRaw(prefix, options); madeDirs.push(dir); return dir; };
process.on("exit", () => { for (const dir of madeDirs) try { rmSync(dir, { recursive: true, force: true }); } catch { /* still in use: leave it */ } });

test("the gate accepts only CC0-1.0", () => {
  assert.equal(ACCEPTED_LICENCE, "CC0-1.0");
});

test("--licence wins; a missing or broken manifest resolves UNKNOWN", () => {
  assert.equal(resolveLicence(null, "MIT"), "MIT", "explicit flag wins");
  assert.equal(resolveLicence(join(tmpdir(), "no-such-manifest.json"), ""), "UNKNOWN");
  const dir = mkdtempSync(join(tmpdir(), "intake-manifest-"));
  const bad = join(dir, "bad.json");
  writeFileSync(bad, "{not json", "utf8");
  assert.equal(resolveLicence(bad, ""), "UNKNOWN", "broken JSON refuses the copy");
});

test("a CC0 sources[] mark passes; any other mark refuses", () => {
  const dir = mkdtempSync(join(tmpdir(), "intake-manifest-"));
  const cc0 = join(dir, "cc0.json");
  writeFileSync(cc0, JSON.stringify({ sources: [{ spdx: "CC0-1.0" }] }), "utf8");
  assert.equal(resolveLicence(cc0, ""), "CC0-1.0", "0 CC0 marks before -> 1 CC0 licence out");
  const gpl = join(dir, "gpl.json");
  writeFileSync(gpl, JSON.stringify({ spdx: "GPL-3.0-only" }), "utf8");
  assert.equal(resolveLicence(gpl, ""), "GPL-3.0-only", "non-CC0 resolves as written so intake refuses it");
});

test("outputs inside content/fighters are refused; siblings pass", () => {
  const dir = mkdtempSync(join(tmpdir(), "intake-dirs-"));
  assert.equal(insideDir(join(dir, "out.glb"), dir), true, "child path is inside");
  assert.equal(insideDir(dir, dir), true, "the dir itself is inside");
  assert.equal(insideDir(join(dir, "..", "escape.glb"), dir), false, "dot-dot escape is outside");
  assert.equal(insideDir(join(tmpdir(), "elsewhere.glb"), dir), false, "a sibling tree is outside");
});

test("sha256 matches the known vector", () => {
  assert.equal(
    sha256(Buffer.from("abc")),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
  );
});

test("triangle counts add indexed thirds plus POSITION thirds", () => {
  const stub = {
    getRoot: () => ({
      listMeshes: () => [{
        listPrimitives: () => [
          { getIndices: () => ({ getCount: () => 6 }), getAttribute: () => null },
          { getIndices: () => null, getAttribute: (n) => (n === "POSITION" ? { getCount: () => 9 } : null) },
        ],
      }],
    }),
  };
  assert.equal(countTriangles(stub), 5, "6 indices (2 tris) + 9 verts (3 tris) = 5");
});

test("checkBufferBounds vectors: inside true, outsides false, negatives false", () => {
  assert.deepEqual(checkBufferBounds(64, 8, 32, 4, 16), { ok: true, reason: "inside" }, "view 8..40 in 64, accessor 4..20 in 32");
  assert.deepEqual(checkBufferBounds(64, 8, 32, 4, 29), { ok: false, reason: "accessor-outside-view" }, "4+29=33 over view len 32");
  assert.deepEqual(checkBufferBounds(64, 50, 20, 0, 8), { ok: false, reason: "view-outside-buffer" }, "50+20=70 over buffer 64");
  assert.deepEqual(checkBufferBounds(-1, 8, 32, 4, 16), { ok: false, reason: "negative-or-fraction" }, "negative buffer refuses");
  assert.deepEqual(checkBufferBounds(64, -2, 32, 4, 16), { ok: false, reason: "negative-or-fraction" }, "negative view offset refuses");
  assert.deepEqual(checkBufferBounds(64, 8, 32, 4.5, 16), { ok: false, reason: "negative-or-fraction" }, "fraction offset refuses");
  assert.deepEqual(checkBufferBounds(64, 0, 64, 0, 64), { ok: true, reason: "inside" }, "exact fit 0..64 is inside");
});

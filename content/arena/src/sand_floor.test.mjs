// Run: node --test content/arena/src/sand_floor.test.mjs
// Sand floor trim guard: the 1.7 MB single-line glTF stalls root grep,
// so every search passes path/include and ignores the minified blob.
// This file trims dead IMG_* bufferView names (no caller outside the file)
// while keeping geometry (20016 B prefix) plus image slices intact.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const SRC_DIR = dirname(fileURLToPath(import.meta.url));
const GLTF_PATH = join(SRC_DIR, "sand_floor.gltf");

// Geometry view lengths: 5 views sum to the 20016 B prefix (HEAD-identical).
const GEO_VIEW_BYTES = [3468, 3468, 2312, 4624, 6144];
// Image slice lengths: albedo 2k, normal 2k, orm (staged SHAs in manifest).
const IMG_VIEW_BYTES = [457152, 863847, 664];

export function loadDoc(path = GLTF_PATH) {
  return JSON.parse(readFileSync(path, "utf8"));
}

// List trim problems in one parsed glTF doc. Empty means trimmed and good.
export function findTrimProblems(doc) {
  const bad = [];
  if (!doc || !Array.isArray(doc.bufferViews)) {
    bad.push("bufferViews array");
    return bad;
  }
  for (const [i, v] of doc.bufferViews.entries()) {
    if (v && typeof v.name === "string" && v.name.startsWith("IMG_")) {
      bad.push(`bufferView ${i} dead name ${v.name}`);
    }
  }
  return bad;
}

test("sand_floor parses with 3 images over 8 views", () => {
  const doc = loadDoc();
  assert.equal(doc.asset?.version, "2.0");
  assert.equal((doc.images ?? []).length, 3);
  assert.equal((doc.bufferViews ?? []).length, 8);
  assert.equal((doc.accessors ?? []).length, 5);
  assert.equal((doc.meshes ?? []).length, 1);
  const lens = doc.bufferViews.map((v) => v.byteLength);
  assert.deepEqual(lens.slice(0, 5), GEO_VIEW_BYTES);
  assert.deepEqual(lens.slice(5), IMG_VIEW_BYTES);
  assert.equal(doc.buffers?.[0]?.byteLength, 1341679);
  // Texture indices resolve, sampler repeats, TEXCOORD_0 on the primitive.
  assert.deepEqual(doc.images.map((im) => im.bufferView), [5, 6, 7]);
  assert.equal(doc.samplers?.[0]?.wrapS, 10497);
  const prim = doc.meshes[0]?.primitives?.[0];
  assert.ok(prim?.attributes?.TEXCOORD_0 !== undefined);
});

test("sand_floor carries no dead IMG_* bufferView names", () => {
  const doc = loadDoc();
  assert.deepEqual(findTrimProblems(doc), []);
});

test("validators refuse IMG names and bad shapes", () => {
  const good = { bufferViews: [{ byteLength: 1 }, { byteLength: 2 }] };
  assert.deepEqual(findTrimProblems(good), []);
  const withDead = {
    bufferViews: [{ byteLength: 1, name: "IMG_sand_albedo_2k" }],
  };
  assert.ok(findTrimProblems(withDead).length > 0);
  assert.deepEqual(findTrimProblems(null), ["bufferViews array"]);
  assert.deepEqual(findTrimProblems({}), ["bufferViews array"]);
});

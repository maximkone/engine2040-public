// Run: node --test content/arena/src/banner.test.mjs
// Banner search guard: the 7.8 MB single-line glTF stalls root grep,
// so the file stays pretty-printed multiline and carries no dead IMG_*
// bufferView names (no caller outside the file). Geometry (21264 B over
// 15 views) plus image slices stay intact; the loader
// (`crates/game/src/kit_load.rs:66` Banner => "banner.gltf") parses JSON,
// so whitespace never changes staged meshes.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const SRC_DIR = dirname(fileURLToPath(import.meta.url));
const GLTF_PATH = join(SRC_DIR, "banner.gltf");

// Geometry view lengths: 3 meshes x 5 views (pole, bar, cloth).
const GEO_VIEW_BYTES = [
  1800, 1800, 1200, 2400, 2304,
  1800, 1800, 1200, 2400, 2304,
  420, 420, 280, 560, 576,
];
// Image slice lengths: cloth albedo 1k (1024x986, under the potato cap), cloth
// normal 2k, orm pair.
const IMG_VIEW_BYTES = [153322, 2656496, 664, 664];

export function loadDoc(path = GLTF_PATH) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function loadRaw(path = GLTF_PATH) {
  return readFileSync(path, "utf8");
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

// List format problems in the raw file text. Empty means multiline and good.
export function findFormatProblems(raw) {
  const bad = [];
  if (typeof raw !== "string" || !raw.includes("\n")) {
    bad.push("single-line blob");
    return bad;
  }
  if (raw.split("\n").length < 100) bad.push("too few lines");
  return bad;
}

test("banner parses with 4 images over 19 views", () => {
  const doc = loadDoc();
  assert.equal(doc.asset?.version, "2.0");
  assert.equal((doc.meshes ?? []).length, 3);
  assert.equal((doc.accessors ?? []).length, 15);
  assert.equal((doc.bufferViews ?? []).length, 19);
  assert.equal((doc.images ?? []).length, 4);
  assert.equal((doc.materials ?? []).length, 2);
  const lens = doc.bufferViews.map((v) => v.byteLength);
  assert.deepEqual(lens.slice(0, 15), GEO_VIEW_BYTES);
  assert.deepEqual(lens.slice(15), IMG_VIEW_BYTES);
  assert.equal(doc.buffers?.[0]?.byteLength, 2832412);
  assert.deepEqual(doc.images.map((im) => im.bufferView), [15, 16, 17, 18]);
  const names = doc.meshes.map((m) => m.name);
  assert.deepEqual(names, ["banner_pole", "banner_bar", "banner_cloth"]);
});

test("banner stays pretty-printed multiline", () => {
  const raw = loadRaw();
  assert.deepEqual(findFormatProblems(raw), []);
});

test("banner carries no dead IMG_* bufferView names", () => {
  const doc = loadDoc();
  assert.deepEqual(findTrimProblems(doc), []);
});

test("validators refuse single-line blobs, IMG names and bad shapes", () => {
  assert.deepEqual(findFormatProblems('{"a":1}'), ["single-line blob"]);
  assert.deepEqual(findFormatProblems("a\n".repeat(100)), []);
  assert.deepEqual(findTrimProblems({ bufferViews: [{ byteLength: 1 }] }), []);
  const withDead = {
    bufferViews: [{ byteLength: 1, name: "IMG_cloth_orm" }],
  };
  assert.ok(findTrimProblems(withDead).length > 0);
  assert.deepEqual(findTrimProblems(null), ["bufferViews array"]);
  assert.deepEqual(findTrimProblems({}), ["bufferViews array"]);
});

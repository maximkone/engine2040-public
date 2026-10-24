// Run: node --test tools/polyhaven-glb.test.mjs
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { wrapGltf } from "./polyhaven-glb.mjs";

// A GLB is a 12-byte header then chunks: length, type, bytes. The types are the
// spec's ASCII magic in little-endian ("JSON", "BIN\0").
const GLB_MAGIC = 0x46546c67;
const JSON_CHUNK = 0x4e4f534a;
const BIN_CHUNK = 0x004e4942;

// One buffer, one external texture: the smallest document the wrapper accepts.
const gltfWith = (extra = {}) => ({
  asset: { version: "2.0" },
  buffers: [{ uri: "scene.bin", byteLength: 8 }],
  bufferViews: [{ buffer: 0, byteOffset: 0, byteLength: 8 }],
  images: [{ name: "diffuse", uri: "diffuse.png" }],
  ...extra,
});

const fixture = (doc = gltfWith()) => {
  const dir = mkdtempSync(join(tmpdir(), "polyhaven-glb-"));
  writeFileSync(join(dir, "scene.gltf"), JSON.stringify(doc));
  writeFileSync(join(dir, "scene.bin"), Buffer.from([1, 2, 3, 4, 5, 6, 7, 8]));
  writeFileSync(join(dir, "diffuse.png"), Buffer.from([0x89, 0x50, 0x4e, 0x47]));
  return dir;
};

const chunks = (glb) => {
  const jsonLen = glb.readUInt32LE(12), binAt = 20 + jsonLen;
  return {
    json: JSON.parse(glb.subarray(20, 20 + jsonLen).toString("utf8")),
    binLen: glb.readUInt32LE(binAt),
    bin: glb.subarray(binAt + 8, binAt + 8 + glb.readUInt32LE(binAt)),
  };
};

test("wrapGltf writes one GLB with a JSON chunk and a BIN chunk of the concatenated files", () => {
  const dir = fixture();
  try {
    const glb = wrapGltf(dir, "scene.gltf");
    assert.equal(glb.readUInt32LE(0), GLB_MAGIC, "glTF magic");
    assert.equal(glb.readUInt32LE(4), 2, "GLB version 2");
    assert.equal(glb.readUInt32LE(8), glb.length, "the header length is the whole file");
    assert.equal(glb.readUInt32LE(16), JSON_CHUNK, "first chunk is JSON");
    // The BIN chunk carries the buffer, then the image bytes, and pads to 4.
    const { binLen, bin, json } = chunks(glb);
    assert.equal(binLen % 4, 0, "the BIN chunk is padded to 4 bytes");
    assert.equal(bin.length, binLen);
    assert.deepEqual([...bin.subarray(0, 8)], [1, 2, 3, 4, 5, 6, 7, 8], "the buffer bytes come first");
    assert.deepEqual([...bin.subarray(8, 12)], [0x89, 0x50, 0x4e, 0x47], "then the image bytes");
    // No external uri survives: the document now points into the blob.
    assert.equal(json.buffers[0].uri, undefined, "the buffer uri is dropped");
    assert.equal(json.buffers[0].byteLength, binLen, "and its length is the blob");
    assert.equal(json.images[0].uri, undefined, "the image uri is dropped");
    assert.equal(json.images[0].mimeType, "image/png");
    const view = json.bufferViews[json.images[0].bufferView];
    assert.deepEqual([view.buffer, view.byteOffset, view.byteLength], [0, 8, 4], "the image has its own bufferView after the buffer");
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("the same three inputs give the same bytes, and a second wrap of the same tree does not drift", () => {
  const dir = fixture();
  try {
    assert.deepEqual(wrapGltf(dir, "scene.gltf"), wrapGltf(dir, "scene.gltf"), "no clock and no map order in the output");
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("wrapGltf refuses a document it cannot pack instead of writing a broken GLB", () => {
  const twoBuffers = fixture(gltfWith({ buffers: [{ uri: "scene.bin" }, { uri: "scene.bin" }] }));
  try { assert.throws(() => wrapGltf(twoBuffers, "scene.gltf"), /want exactly one buffer, got 2/); }
  finally { rmSync(twoBuffers, { recursive: true, force: true }); }
  const embedded = fixture(gltfWith({ buffers: [{ uri: "data:application/octet-stream;base64,AAAA" }] }));
  try { assert.throws(() => wrapGltf(embedded, "scene.gltf"), /already embedded/); }
  finally { rmSync(embedded, { recursive: true, force: true }); }
  const noUri = fixture(gltfWith({ images: [{ name: "diffuse" }] }));
  try { assert.throws(() => wrapGltf(noUri, "scene.gltf"), /has no external uri/); }
  finally { rmSync(noUri, { recursive: true, force: true }); }
});

test("a texture type the GLB cannot name is refused, not guessed", () => {
  const dir = fixture(gltfWith({ images: [{ name: "diffuse", uri: "diffuse.ktx2" }] }));
  try {
    writeFileSync(join(dir, "diffuse.ktx2"), Buffer.from([0, 0, 0, 0]));
    assert.throws(() => wrapGltf(dir, "scene.gltf"), /is not a known type/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

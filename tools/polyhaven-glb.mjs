// polyhaven-glb: wrap a Poly Haven `.gltf` + `.bin` + texture files into one
// self-contained `.glb`, so the model intake (`crates/assets/src/gen_intake.rs`)
// reads the same bytes on every machine with no external URIs.
//
// Owner: content lane. Consumers: SW121 and the next Poly Haven rows.
// Determinism: the same three inputs give the same bytes (no clock, no map
// order: images are appended in the order the glTF lists them). Units: bytes.
//
// Usage: node tools/polyhaven-glb.mjs <dir> <model.gltf> <out.glb>
//   <dir> holds the .gltf, its .bin and the relative texture paths the glTF names.
// Refuses: a missing buffer or image, a buffer URI that is not a file, or an
// input already packed (a .glb in, a .glb out is not this tool's job).
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const GLB_MAGIC = 0x46546c67; // "glTF"
const JSON_CHUNK = 0x4e4f534a; // "JSON"
const BIN_CHUNK = 0x004e4942; // "BIN\0"
const MIME = { ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png" };

function padded(buf, fill) {
  const pad = (4 - (buf.length % 4)) % 4;
  return pad === 0 ? buf : Buffer.concat([buf, Buffer.alloc(pad, fill)]);
}

/** Pack a glTF document plus its external files into one GLB. */
export function wrapGltf(dir, gltfName) {
  const gltf = JSON.parse(readFileSync(join(dir, gltfName), "utf8"));
  if (gltf.buffers?.length !== 1) {
    throw new Error(`${gltfName}: want exactly one buffer, got ${gltf.buffers?.length ?? 0}`);
  }
  const buffer = gltf.buffers[0];
  if (!buffer.uri || buffer.uri.startsWith("data:")) {
    throw new Error(`${gltfName}: buffer is already embedded`);
  }
  const parts = [readFileSync(join(dir, buffer.uri))];
  let offset = parts[0].length;
  const appended = [];
  gltf.images = (gltf.images ?? []).map((image) => {
    if (!image.uri || image.uri.startsWith("data:")) {
      throw new Error(`${gltfName}: image ${image.name ?? "?"} has no external uri`);
    }
    const ext = image.uri.slice(image.uri.lastIndexOf(".")).toLowerCase();
    const mimeType = MIME[ext];
    if (!mimeType) throw new Error(`${gltfName}: image ${image.uri} is not a known type`);
    const bytes = readFileSync(join(dir, image.uri));
    const start = offset;
    parts.push(bytes);
    offset += bytes.length;
    // Each image gets its own bufferView in the shared binary blob.
    const view = { buffer: 0, byteOffset: start, byteLength: bytes.length };
    appended.push(view);
    return { ...image, uri: undefined, mimeType, bufferView: gltf.bufferViews.length + appended.length - 1 };
  });
  gltf.bufferViews = [...(gltf.bufferViews ?? []), ...appended];
  // The GLB BIN chunk becomes buffer 0, so its uri goes and its length is the blob.
  const bin = padded(Buffer.concat(parts), 0);
  delete buffer.uri;
  buffer.byteLength = bin.length;
  // Stable key order keeps the wrap byte-identical across runs.
  const json = padded(Buffer.from(JSON.stringify(gltf), "utf8"), 0x20);
  const total = 12 + 8 + json.length + 8 + bin.length;
  const out = Buffer.alloc(total);
  let at = 0;
  out.writeUInt32LE(GLB_MAGIC, at);
  out.writeUInt32LE(2, at + 4);
  out.writeUInt32LE(total, at + 8);
  at += 12;
  out.writeUInt32LE(json.length, at);
  out.writeUInt32LE(JSON_CHUNK, at + 4);
  json.copy(out, at + 8);
  at += 8 + json.length;
  out.writeUInt32LE(bin.length, at);
  out.writeUInt32LE(BIN_CHUNK, at + 4);
  bin.copy(out, at + 8);
  return out;
}

if (process.argv[1]?.endsWith("polyhaven-glb.mjs")) {
  const [dir, gltfName, outName] = process.argv.slice(2);
  if (!dir || !gltfName || !outName) {
    console.error("usage: node tools/polyhaven-glb.mjs <dir> <model.gltf> <out.glb>");
    process.exit(2);
  }
  const glb = wrapGltf(dir, gltfName);
  writeFileSync(join(dir, outName), glb);
  console.log(`polyhaven-glb: ${outName} ${glb.length} bytes`);
}

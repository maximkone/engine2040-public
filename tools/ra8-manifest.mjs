// RA8 look manifest generator for q1 captures.
// Generates manifests from real PNG bytes on disk, never fakes evidence.
// Read-only on target/q1; refuses to write into target/.
//
// Usage:
//   node tools/ra8-manifest.mjs --src target/q1/captures --out $env:TEMP/ra8.json
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { join, resolve, basename } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { fnv1a64Hex } from "./fnv1a64.mjs";
export const RA8_SCHEMA = 1;
export const RA8_KIND = "ra8-look";
const HEX64 = /^[0-9a-f]{64}$/;
const HEX16 = /^[0-9a-f]{16}$/;
function err(list, msg) { list.push(msg); }
export function validateRa8Manifest(m) {
  const errors = [];
  if (!m || typeof m !== "object") return { ok: false, errors: ["manifest must be an object"] };
  if (m.schema !== RA8_SCHEMA) err(errors, "schema must be " + RA8_SCHEMA);
  if (m.kind !== RA8_KIND) err(errors, "kind must be ra8-look");
  for (const f of ["source_dir", "renderer", "honesty", "sequence_fnv"]) {
    if (typeof m[f] !== "string" || m[f].length === 0) err(errors, f + " must be a non-empty string");
  }
  for (const f of ["seed", "width", "height", "fps"]) {
    if (!Number.isInteger(m[f]) || m[f] < 0) err(errors, f + " must be an integer >= 0");
  }
  if (m.width === 0) err(errors, "width must be > 0");
  if (m.height === 0) err(errors, "height must be > 0");
  if (typeof m.sequence_fnv !== "string" || !HEX16.test(m.sequence_fnv)) err(errors, "sequence_fnv must match hex16");
  if (!Array.isArray(m.frames) || m.frames.length === 0) {
    err(errors, "frames must be a non-empty array");
    return { ok: errors.length === 0, errors };
  }
  const seen = new Set();
  let prev = -1;
  for (let i = 0; i < m.frames.length; i++) {
    const fr = m.frames[i];
    const at = "frames[" + i + "]";
    if (!fr || typeof fr !== "object") { err(errors, at + " must be an object"); continue; }
    if (!Number.isInteger(fr.tick) || fr.tick < 0) err(errors, at + ".tick must be an integer >= 0");
    else {
      if (seen.has(fr.tick)) err(errors, at + ".tick duplicates " + fr.tick);
      seen.add(fr.tick);
      if (fr.tick <= prev) err(errors, at + ".tick must be sorted ascending");
      prev = fr.tick;
    }
    if (typeof fr.file !== "string" || !fr.file.toLowerCase().endsWith(".png")) err(errors, at + ".file must be a .png path");
    if (!Number.isInteger(fr.bytes) || fr.bytes <= 0) err(errors, at + ".bytes must be an integer > 0");
    if (typeof fr.sha256 !== "string" || !HEX64.test(fr.sha256)) err(errors, at + ".sha256 must match hex64");
    if (typeof fr.png_fnv !== "string" || !HEX16.test(fr.png_fnv)) err(errors, at + ".png_fnv must match hex16");
    if (fr.checkpoint !== undefined && (typeof fr.checkpoint !== "string" || !HEX16.test(fr.checkpoint))) err(errors, at + ".checkpoint must match hex16 when present");
  }
  return { ok: errors.length === 0, errors };
}
export function hashBytes(bytes) {
  const buf = Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes);
  const sha256 = createHash("sha256").update(buf).digest("hex");
  return { bytes: buf.length, sha256, png_fnv: fnv1a64Hex(buf) };
}
export function refuseTargetOut(outPath) {
  const norm = resolve(outPath).replace(/\\/g, "/").toLowerCase();
  if (norm.includes("/target/") || norm.endsWith("/target")) throw new Error("refusing to write into target/: " + outPath);
}
export function generateRa8Manifest(srcDir, opts = {}) {
  const dir = resolve(srcDir);
  let entries;
  try { entries = readdirSync(dir, { withFileTypes: true }); }
  catch (e) { throw new Error("cannot read src dir " + dir + ": " + String((e && e.message) || e)); }
  const pngNames = entries.filter((d) => d.isFile() && d.name.toLowerCase().endsWith(".png")).map((d) => d.name).sort();
  if (pngNames.length === 0) throw new Error("skipped: no .png frames in " + dir);
  let source = null;
  try { source = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8")); } catch { source = null; }
  const byFile = new Map();
  if (source && Array.isArray(source.frames)) {
    for (const r of source.frames) { if (r && typeof r.file === "string") byFile.set(basename(r.file), r); }
  }
  const frames = pngNames.map((name) => {
    const full = join(dir, name);
    const st = statSync(full);
    const bytes = readFileSync(full);
    if (bytes.length === 0) throw new Error("empty png " + full);
    if (st.size !== bytes.length) throw new Error("size mismatch " + full);
    const h = hashBytes(bytes);
    const src = byFile.get(name);
    if (src) {
      if (typeof src.sha256 === "string" && src.sha256 !== h.sha256) throw new Error("sha256 mismatch for " + name);
      if (typeof src.png_fnv === "string" && src.png_fnv !== h.png_fnv) throw new Error("png_fnv mismatch for " + name);
      if (Number.isInteger(src.bytes) && src.bytes !== h.bytes) throw new Error("bytes mismatch for " + name);
    }
    const tick = src && Number.isInteger(src.tick) ? src.tick : -1;
    if (tick < 0) throw new Error("skipped: no source tick for " + name);
    const row = { tick, file: name, bytes: h.bytes, sha256: h.sha256, png_fnv: h.png_fnv };
    if (src && typeof src.checkpoint === "string" && HEX16.test(src.checkpoint)) row.checkpoint = src.checkpoint;
    return row;
  });
  frames.sort((a, b) => a.tick - b.tick);
  const renderer = opts.renderer ?? (source && source.renderer) ?? "unknown";
  const seed = opts.seed ?? (source && source.seed) ?? 0;
  const width = opts.width ?? (source && source.width) ?? 0;
  const height = opts.height ?? (source && source.height) ?? 0;
  const fps = opts.fps ?? (source && source.fps) ?? 30;
  const device = opts.device ?? "staged";
  const honesty = opts.honesty ?? ("hashed from disk PNGs in " + dir + "; no reference-GPU pixels invented");
  const sequence_fnv = fnv1a64Hex(Buffer.from(frames.map((f) => f.sha256).join("\n"), "utf8"));
  const manifest = {
    schema: RA8_SCHEMA, kind: RA8_KIND, source_dir: dir,
    renderer: String(renderer), device: String(device),
    seed, width, height, fps, frames,
    tick_coverage: { count: frames.length, first: frames[0].tick, last: frames[frames.length - 1].tick },
    sequence_fnv, honesty, generated_at: new Date().toISOString(),
  };
  const v = validateRa8Manifest(manifest);
  if (!v.ok) throw new Error("generated manifest invalid: " + v.errors.join("; "));
  return manifest;
}
function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--src") out.src = argv[++i];
    else if (a === "--out") out.out = argv[++i];
    else if (a === "--renderer") out.renderer = argv[++i];
    else if (a === "--device") out.device = argv[++i];
    else if (a === "--seed") out.seed = Number(argv[++i]);
    else if (a === "--help" || a === "-h") out.help = true;
  }
  return out;
}
if (import.meta.main) {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || !args.src || !args.out) {
    console.log("usage: node tools/ra8-manifest.mjs --src <capturesDir> --out <json> [--renderer <n>] [--device <n>] [--seed <n>]");
    process.exit(args.help ? 0 : 2);
  }
  try {
    refuseTargetOut(args.out);
    const m = generateRa8Manifest(args.src, args);
    writeFileSync(args.out, JSON.stringify(m, null, 2) + "\n");
    console.log(JSON.stringify({ ok: true, out: resolve(args.out), frames: m.frames.length, sequence_fnv: m.sequence_fnv }));
  } catch (e) { console.error("ra8-manifest: " + String((e && e.message) || e)); process.exit(1); }
}

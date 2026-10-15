// frame-triage.mjs - J1 showcase triage (fs + PNG pixels via zlib stdlib, never cargo).
// The gate reads pixels: each frame counts fighter-kit pixels (isFighterPixel from
// fighter-count.mjs) and passes only above FIGHTER_FLOOR_PIXELS. The manifest's claimed
// fighter count is reported and never decides. Orbit dims pin 1280x720; red boxes come
// from red-pixel count. No new deps.
import { existsSync, readFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadFrame } from './frame-v2v3.mjs';
import { decodePng, isFighterPixel } from './fighter-count.mjs';
export const PNG_MAGIC = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
export const ORBIT_TRUTH = { width: 1280, height: 720, fighters: 0, boxesMin: 2, boxesMax: 3 };
// Fighter-kit pixels a frame needs before it passes (strictly above). No real capture has
// fighter pixels yet: every target/reports/staged-fight*/captures set counts fighter=0 with
// fighter-count.mjs, so no measured set fits this number. It is a stated floor: a visible
// fighter at showcase scale covers far more than 50 pixels, and 50 rejects a stray pixel.
export const FIGHTER_FLOOR_PIXELS = 50;
export function readIhdrDimensions(filePath) {
  const buf = readFileSync(filePath);
  if (buf.length < 33 || !buf.subarray(0, 8).equals(PNG_MAGIC)) { throw new Error('not a PNG'); }
  if (buf.toString('ascii', 12, 16) !== 'IHDR') { throw new Error('missing IHDR'); }
  const width = buf.readUInt32BE(16);
  const height = buf.readUInt32BE(20);
  if (!width || !height) { throw new Error('bad IHDR dims'); }
  return { width: width, height: height, bytes: buf.length };
}
export function countRedPixels(pixels) {
  let red = 0;
  const n = Math.floor(pixels.length / 4);
  for (let i = 0; i < n; i += 1) {
    const o = i * 4;
    const r = pixels[o];
    const g = pixels[o + 1];
    const b = pixels[o + 2];
    if (r > 140 && r > g + 40 && r > b + 40) { red += 1; }
  }
  return red;
}
export function boxesFromRed(red) {
  if (red < 500) { return { boxesMin: 0, boxesMax: 0 }; }
  if (red < 2000) { return { boxesMin: 1, boxesMax: 2 }; }
  if (red < 5000) { return { boxesMin: 2, boxesMax: 3 }; }
  return { boxesMin: 3, boxesMax: 3 };
}
export function fightersFromManifest(filePath) {
  try {
    const dir = dirname(filePath);
    const manPath = join(dir, 'manifest.json');
    if (!existsSync(manPath)) { return 0; }
    const man = JSON.parse(readFileSync(manPath, 'utf8'));
    if (typeof man.fighters === 'number' && Number.isFinite(man.fighters)) { return Math.max(0, Math.floor(man.fighters)); }
    const frames = man.frames;
    if (!Array.isArray(frames)) { return 0; }
    const base = basename(filePath);
    for (const e of frames) {
      if (e && e.file === base && typeof e.fighters === 'number' && Number.isFinite(e.fighters)) { return Math.max(0, Math.floor(e.fighters)); }
    }
    return 0;
  } catch {
    return 0;
  }
}
/** Fighter-kit pixels in one PNG, counted on decoded RGBA. Throws on an undecodable file. */
export function fighterPixelCount(filePath) {
  const { data } = decodePng(readFileSync(filePath), filePath);
  let fighter = 0;
  for (let i = 0; i < data.length; i += 4) {
    if (isFighterPixel(data[i], data[i + 1], data[i + 2])) { fighter += 1; }
  }
  return fighter;
}
export function triageFrame(filePath) {
  const d = readIhdrDimensions(filePath);
  const frame = loadFrame(filePath);
  const red = countRedPixels(frame.pixels);
  const boxes = boxesFromRed(red);
  return {
    file: filePath,
    width: d.width,
    height: d.height,
    bytes: d.bytes,
    decoded: true,
    claimedFighters: fightersFromManifest(filePath),
    fighterPixels: fighterPixelCount(filePath),
    boxesMin: boxes.boxesMin,
    boxesMax: boxes.boxesMax,
    redPixels: red,
  };
}
export function triageOrbitDir(dirAbs, fileNames) {
  return fileNames.map(function (n) { return triageFrame(join(dirAbs, n)); });
}
/**
 * True only for a row built by decoding a frame. triageFrame marks a decoded row; the
 * counts must fit inside the decoded area. A made-up row with a high fighterPixels but
 * no decoded pixels fails, so the gate never goes green on an unopened frame.
 */
export function hasDecodedPixels(r) {
  if (!r || typeof r !== 'object' || r.decoded !== true) { return false; }
  if (!Number.isFinite(r.width) || !Number.isFinite(r.height) || r.width <= 0 || r.height <= 0) { return false; }
  if (!Number.isFinite(r.bytes) || r.bytes <= 0) { return false; }
  if (!Number.isFinite(r.fighterPixels) || !Number.isFinite(r.redPixels)) { return false; }
  const total = r.width * r.height;
  if (!Number.isFinite(total) || total <= 0) { return false; }
  if (r.fighterPixels < 0 || r.fighterPixels > total) { return false; }
  if (r.redPixels < 0 || r.redPixels > total) { return false; }
  return true;
}
/**
 * The frame gate over triage rows. A frame passes only when its decoded fighter-kit
 * pixel count is above `floor`. The set passes only when it has frames and every frame
 * passes.
 */
export function gateRows(rows, floor = FIGHTER_FLOOR_PIXELS) {
  const judged = rows.map(function (r) { return Object.assign({}, r, { pass: hasDecodedPixels(r) && r.fighterPixels > floor }); });
  return { pass: judged.length > 0 && judged.every(function (r) { return r.pass; }), floor: floor, rows: judged };
}
const isMain = import.meta.main;
if (isMain) {
  const dir = join(fileURLToPath(import.meta.url), '..', '..', 'target', 'showcase', '2026-10-08', 'orbit');
  const names = ['frame_000000.png', 'frame_000001.png', 'frame_000002.png'];
  let gate = null;
  try {
    gate = gateRows(triageOrbitDir(dir, names));
  } catch (err) {
    process.stderr.write('frame-triage: ' + (err && err.message ? err.message : err) + '\n');
    process.stdout.write('GATE FAIL frames=0\n');
    process.exitCode = 1;
  }
  if (gate !== null) {
    for (const r of gate.rows) {
      process.stdout.write(r.file + ' | ' + r.width + 'x' + r.height + ' | fighterPixels=' + r.fighterPixels + ' | claimedFighters=' + r.claimedFighters + ' | boxes=' + r.boxesMin + '-' + r.boxesMax + ' | red=' + r.redPixels + ' | ' + (r.pass ? 'pass' : 'fail') + '\n');
    }
    process.stdout.write('GATE ' + (gate.pass ? 'PASS' : 'FAIL') + ' floor=' + gate.floor + ' frames=' + gate.rows.length + '\n');
  }
}

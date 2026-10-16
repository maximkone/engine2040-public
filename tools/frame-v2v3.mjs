// frame-v2v3.mjs - V2 arena-day + V3 fight-action classifiers (standalone, never cargo).
// Fresh PNG decode (8-bit gray/RGB/RGBA, non-interlaced, node zlib only) plus two
// pixel heuristics over orbit-style captures. No new deps. Standalone: it does not
// import frame-triage.mjs or frame-moved.mjs; the decode below is reimplemented.
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { inflateSync } from 'node:zlib';
export const PNG_HEAD = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
export const V2_TRUTH = { minSkyFrac: 0.25, minSkyErnst: 0.03, bandFrac: 0.4 };
export const V3_TRUTH = { minMeanDelta: 8, minChangedFrac: 0.05, changeFloor: 12 };
function paethGuess(left, above, upperLeft) {
  const base = left + above - upperLeft;
  const dLeft = Math.abs(base - left);
  const dAbove = Math.abs(base - above);
  const dUpper = Math.abs(base - upperLeft);
  if (dLeft <= dAbove && dLeft <= dUpper) { return left; }
  if (dAbove <= dUpper) { return above; }
  return upperLeft;
}
export function decodeFrame(buf) {
  if (buf.length < 8 || !buf.subarray(0, 8).equals(PNG_HEAD)) { throw new Error('not a PNG'); }
  let at = 8;
  let picW = 0;
  let picH = 0;
  let picColor = -1;
  let gotHeader = false;
  const parts = [];
  while (at + 8 <= buf.length) {
    const size = buf.readUInt32BE(at);
    const kind = buf.toString('ascii', at + 4, at + 8);
    const body = at + 8;
    if (body + size + 4 > buf.length) { throw new Error('truncated chunk'); }
    if (kind === 'IHDR') {
      if (size !== 13) { throw new Error('bad IHDR'); }
      picW = buf.readUInt32BE(body);
      picH = buf.readUInt32BE(body + 4);
      const depth = buf[body + 8];
      picColor = buf[body + 9];
      const comp = buf[body + 10];
      const filt = buf[body + 11];
      const lace = buf[body + 12];
      if (depth !== 8 || comp !== 0 || filt !== 0 || lace !== 0) { throw new Error('only 8-bit non-interlaced PNGs'); }
      if (picColor !== 0 && picColor !== 2 && picColor !== 6) { throw new Error('only gray/RGB/RGBA PNGs'); }
      if (picW === 0 || picH === 0) { throw new Error('bad IHDR dims'); }
      gotHeader = true;
    } else if (kind === 'IDAT') {
      parts.push(buf.subarray(body, body + size));
    } else if (kind === 'IEND') {
      break;
    }
    at = body + size + 4;
  }
  if (!gotHeader) { throw new Error('missing IHDR'); }
  const lanes = picColor === 6 ? 4 : picColor === 2 ? 3 : 1;
  const rowLen = picW * lanes;
  let flat;
  try {
    flat = inflateSync(Buffer.concat(parts));
  } catch {
    throw new Error('bad IDAT');
  }
  if (flat.length !== picH * (rowLen + 1)) { throw new Error('bad scanlines'); }
  const pixels = Buffer.alloc(picW * picH * 4);
  let above = Buffer.alloc(rowLen);
  for (let y = 0; y < picH; y += 1) {
    const base = y * (rowLen + 1);
    const mode = flat[base];
    const scan = flat.subarray(base + 1, base + 1 + rowLen);
    if (mode > 4) { throw new Error('bad filter'); }
    const now = Buffer.alloc(rowLen);
    for (let i = 0; i < rowLen; i += 1) {
      const west = i >= lanes ? now[i - lanes] : 0;
      const north = above[i];
      const northWest = i >= lanes ? above[i - lanes] : 0;
      let guess = 0;
      if (mode === 1) { guess = west; } else if (mode === 2) { guess = north; } else if (mode === 3) { guess = (west + north) >> 1; } else if (mode === 4) { guess = paethGuess(west, north, northWest); }
      now[i] = (scan[i] + guess) & 0xff;
    }
    for (let x = 0; x < picW; x += 1) {
      const o = (y * picW + x) * 4;
      if (lanes === 4) {
        pixels[o] = now[x * 4];
        pixels[o + 1] = now[x * 4 + 1];
        pixels[o + 2] = now[x * 4 + 2];
        pixels[o + 3] = now[x * 4 + 3];
      } else if (lanes === 3) {
        pixels[o] = now[x * 3];
        pixels[o + 1] = now[x * 3 + 1];
        pixels[o + 2] = now[x * 3 + 2];
        pixels[o + 3] = 0xff;
      } else {
        pixels[o] = now[x];
        pixels[o + 1] = now[x];
        pixels[o + 2] = now[x];
        pixels[o + 3] = 0xff;
      }
    }
    above = now;
  }
  return { width: picW, height: picH, pixels: pixels };
}
export function loadFrame(filePath) {
  const raw = readFileSync(filePath);
  const pic = decodeFrame(raw);
  return { file: filePath, bytes: raw.length, width: pic.width, height: pic.height, pixels: pic.pixels };
}
function isSkyPixel(r, g, b) {
  return b > r + 8 && b > g + 4 && b >= 100;
}
export function v2ArenaDay(frame) {
  const bandRows = Math.max(1, Math.floor(frame.height * V2_TRUTH.bandFrac));
  let sky = 0;
  let surplus = 0;
  const total = bandRows * frame.width;
  for (let y = 0; y < bandRows; y += 1) {
    for (let x = 0; x < frame.width; x += 1) {
      const o = (y * frame.width + x) * 4;
      const r = frame.pixels[o];
      const g = frame.pixels[o + 1];
      const b = frame.pixels[o + 2];
      const top = Math.max(r, g);
      surplus += (b - top) / 255;
      if (isSkyPixel(r, g, b)) { sky += 1; }
    }
  }
  const skyFrac = sky / total;
  const skyErnst = surplus / total;
  const day = skyFrac >= V2_TRUTH.minSkyFrac && skyErnst >= V2_TRUTH.minSkyErnst;
  return {
    width: frame.width,
    height: frame.height,
    skyFrac: Math.round(skyFrac * 10000) / 10000,
    skyErnst: Math.round(skyErnst * 10000) / 10000,
    label: day ? 'arena-day' : 'not-arena-day',
    why: day ? 'open day sky over the band' : 'no open day sky (dome/cubes stand-in)'
  };
}
export function v3FightDelta(before, after) {
  if (before.width !== after.width || before.height !== after.height) { throw new Error('size mismatch'); }
  const n = before.width * before.height;
  let sum = 0;
  let moved = 0;
  for (let i = 0; i < n; i += 1) {
    const o = i * 4;
    const d = (Math.abs(before.pixels[o] - after.pixels[o]) + Math.abs(before.pixels[o + 1] - after.pixels[o + 1]) + Math.abs(before.pixels[o + 2] - after.pixels[o + 2])) / 3;
    sum += d;
    if (d > V3_TRUTH.changeFloor) { moved += 1; }
  }
  const meanDelta = sum / n;
  const changedFrac = moved / n;
  const fight = meanDelta >= V3_TRUTH.minMeanDelta && changedFrac >= V3_TRUTH.minChangedFrac;
  return {
    width: before.width,
    height: before.height,
    meanDelta: Math.round(meanDelta * 100) / 100,
    changedFrac: Math.round(changedFrac * 10000) / 10000,
    label: fight ? 'fight-action' : 'static',
    why: fight ? 'fighters in motion between frames' : 'no fight motion (slow orbit stand-in)'
  };
}
export function classifyPair(pathA, pathB) {
  const a = loadFrame(pathA);
  const b = loadFrame(pathB);
  return { a: pathA, b: pathB, v2a: v2ArenaDay(a), v2b: v2ArenaDay(b), v3: v3FightDelta(a, b) };
}
const isMain = import.meta.main;
if (isMain) {
  const here = dirname(fileURLToPath(import.meta.url));
  const root = join(here, '..');
  const orbit = join(root, 'target', 'showcase', '2026-10-08', 'orbit');
  const args = process.argv.slice(2).filter(function (s) { return s !== '--json'; });
  const asJson = process.argv.includes('--json');
  const fa = args[0] || join(orbit, 'frame_000000.png');
  const fb = args[1] || join(orbit, 'frame_000001.png');
  const out = classifyPair(fa, fb);
  const NL = String.fromCharCode(10);
  if (asJson) {
    process.stdout.write(JSON.stringify(out, null, 2) + NL);
  } else {
    process.stdout.write('V2 sky-Ernst ' + fa + ' skyFrac=' + out.v2a.skyFrac + ' ernst=' + out.v2a.skyErnst + ' label=' + out.v2a.label + NL);
    process.stdout.write('V2 sky-Ernst ' + fb + ' skyFrac=' + out.v2b.skyFrac + ' ernst=' + out.v2b.skyErnst + ' label=' + out.v2b.label + NL);
    process.stdout.write('V3 motion-delta ' + fa + ' -> ' + fb + ' mean=' + out.v3.meanDelta + ' changed=' + out.v3.changedFrac + ' label=' + out.v3.label + NL);
  }
}


// rdc-diff.mjs - RenderDoc replay-to-texture pixel-diff harness (tools only).
// PORT(baldurk/renderdoc@v1.45:docs/python_api/examples/renderdoc/save_texture.py):
// OpenCaptureFile/OpenFile -> load frame PNGs; SetFrameEvent -> select frame
// index; TextureSave + SaveTexture(PNG) -> save diff texture; pixel compare
// -> differing count. Decode via fighter-count.mjs; integer diff mirrors
// crates/proof/src/gpu_live.rs:145 (max channel abs-diff above epsilon).
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { deflateSync } from 'node:zlib';
import { decodePng } from './fighter-count.mjs';
import { FIGHTER_FLOOR_PIXELS } from './frame-triage.mjs';
export const DIFF_W = 1280;
export const DIFF_H = 720;
const CRC = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) { c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; }
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) { c = CRC[(c ^ b) & 0xff] ^ (c >>> 8); }
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}
/** Encode RGBA bytes as 8-bit PNG (color type 6, filter 0, no interlace). */
export function encodePng(width, height, data) {
  if (data.length !== width * height * 4) { throw new Error('encode size mismatch'); }
  const src = Buffer.isBuffer(data) ? data : Buffer.from(data);
  const stride = width * 4;
  const raw = Buffer.alloc(height * (stride + 1));
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0;
    src.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(raw, { level: 1 })), chunk('IEND', Buffer.alloc(0))]);
}
/** Masked integer pixel diff; mirror of gpu_live.rs frame_diff_count. */
export function diffPixels(a, b, width, height, epsilon = 0, mask = null) {
  if (a.length !== b.length) { throw new Error('length mismatch'); }
  if (a.length === 0) { throw new Error('empty frame'); }
  if (a.length !== width * height * 4) { throw new Error('size mismatch'); }
  if (mask !== null && mask.length !== width * height) { throw new Error('mask mismatch'); }
  if (Buffer.isBuffer(a) && Buffer.isBuffer(b) ? a.equals(b) : Buffer.from(a).equals(Buffer.from(b))) { return 0; }
  let diff = 0;
  const n = width * height;
  for (let i = 0; i < n; i += 1) {
    if (mask !== null && mask[i] !== 0) { continue; }
    const o = i * 4;
    let wide = 0;
    for (let k = 0; k < 4; k += 1) { const d = Math.abs(a[o + k] - b[o + k]); if (d > wide) { wide = d; } }
    if (wide > epsilon) { diff += 1; }
  }
  return diff;
}
/** Resolve a capture input (dir or PNG file) to one frame file. */
export function pickFrame(input, index = 0) {
  if (statSync(input).isDirectory()) {
    const names = readdirSync(input).filter((n) => n.toLowerCase().endsWith('.png')).sort();
    if (names.length === 0) { throw new Error('no PNG frames'); }
    if (index < 0 || index >= names.length) { throw new Error('frame index out of range'); }
    return join(input, names[index]);
  }
  return input;
}
/** Replay one capture pair: load frames, diff, save diff texture, return count. */
export function replayPair(dirA, dirB, outPng, frameIndex = 0, epsilon = 0) {
  const a = decodePng(readFileSync(pickFrame(dirA, frameIndex)), String(dirA));
  const b = decodePng(readFileSync(pickFrame(dirB, frameIndex)), String(dirB));
  if (a.width !== b.width || a.height !== b.height) { throw new Error('size mismatch'); }
  const count = diffPixels(a.data, b.data, a.width, a.height, epsilon, null);
  const n = a.width * a.height;
  const tex = Buffer.alloc(n * 4);
  for (let i = 0; i < n; i += 1) {
    const o = i * 4;
    let wide = 0;
    for (let k = 0; k < 4; k += 1) { const d = Math.abs(a.data[o + k] - b.data[o + k]); if (d > wide) { wide = d; } }
    const hit = wide > epsilon ? 255 : 0;
    tex[o] = hit; tex[o + 1] = 0; tex[o + 2] = hit; tex[o + 3] = 255;
  }
  writeFileSync(outPng, encodePng(a.width, a.height, tex));
  return { count: count, width: a.width, height: a.height, out: outPng };
}
/** Self-check on a 1280x720 pair: identical -> 0, one spike -> 1. */
export function selfCheck() {
  const t0 = Date.now();
  const dir = mkdtempSync(join(tmpdir(), 'rdc-diff-'));
  try {
    const n = DIFF_W * DIFF_H;
    const base = Buffer.alloc(n * 4, 128);
    for (let i = 3; i < base.length; i += 4) { base[i] = 255; }
    const spike = Buffer.from(base);
    spike[0] = (spike[0] + 100) & 255;
    const dirA = join(dir, 'a');
    const dirB = join(dir, 'b');
    mkdirSync(dirA);
    mkdirSync(dirB);
    writeFileSync(join(dirA, 'frame-0.png'), encodePng(DIFF_W, DIFF_H, base));
    writeFileSync(join(dirB, 'frame-0.png'), encodePng(DIFF_W, DIFF_H, base));
    const same = replayPair(dirA, dirB, join(dir, 'diff-same.png'));
    writeFileSync(join(dirB, 'frame-0.png'), encodePng(DIFF_W, DIFF_H, spike));
    const one = replayPair(dirA, dirB, join(dir, 'diff-spike.png'));
    const ms = Date.now() - t0;
    process.stdout.write('identical=' + same.count + ' spike=' + one.count + ' ms=' + ms + '\n');
    if (same.count !== 0 || one.count !== 1) { throw new Error('self-check diff mismatch'); }
    if (FIGHTER_FLOOR_PIXELS !== 50) { throw new Error('floor moved'); }
    if (ms > 5000) { throw new Error('self-check too slow'); }
    return { identical: same.count, spike: one.count, ms: ms };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
if (import.meta.main) {
  const args = process.argv.slice(2);
  if (args.includes('--help') || args.includes('-h')) { process.stdout.write('Usage: node tools/rdc-diff.mjs <dirA|pngA> <dirB|pngB> <outPng> [--frame N] [--epsilon N] | --self-check\n'); process.exit(0); }
  if (args.includes('--self-check')) { selfCheck(); }
  else {
    const pos = args.filter((a) => !a.startsWith('--'));
    const fi = args.includes('--frame') ? Number(args[args.indexOf('--frame') + 1]) : 0;
    const ep = args.includes('--epsilon') ? Number(args[args.indexOf('--epsilon') + 1]) : 0;
    if (pos.length !== 3) { process.stderr.write('need <dirA> <dirB> <outPng>\n'); process.exit(2); }
    const r = replayPair(pos[0], pos[1], pos[2], fi, ep);
    process.stdout.write('diff=' + r.count + ' pixels=' + (r.width * r.height) + ' out=' + r.out + '\n');
  }
}

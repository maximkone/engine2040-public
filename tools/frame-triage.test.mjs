// Run: node --test tools/frame-triage.test.mjs
// The frame gate reads pixels. Synthetic PNGs in a temp dir pin it: a blank set with a manifest
// claiming fighters fails, fighter-kit pixels pass, every frame must pass, and a frame passes
// only above FIGHTER_FLOOR_PIXELS. The J1 orbit pin runs only when its capture set is present.
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateSync } from 'node:zlib';
import test from 'node:test';
import { FIGHTER_FLOOR_PIXELS, ORBIT_TRUTH, gateRows, hasDecodedPixels, triageOrbitDir } from './frame-triage.mjs';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const ORBIT = join(ROOT, 'target', 'showcase', '2026-10-08', 'orbit');
const ORBIT_SKIP = existsSync(ORBIT) ? false : 'no J1 orbit set at target/showcase/2026-10-08/orbit in this tree';
const NAMES = ['frame_000000.png', 'frame_000001.png', 'frame_000002.png'];
const W = 64;
const H = 48;
const FIGHTER_RGB = [60, 120, 200]; // B >= 140, B - R >= 50, 70 <= G <= 210
const BLANK_RGB = [128, 128, 128]; // neither box kit nor fighter kit

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k += 1) { c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; }
  return c >>> 0;
});
function crc32(buf) {
  let c = 0xffffffff;
  for (const byte of buf) { c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8); }
  return (c ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([length, body, crc]);
}
/** Write an 8-bit RGB PNG with no filter and no interlace; `pixel(x, y)` gives [r, g, b]. */
function writePng(file, pixel) {
  const stride = W * 3;
  const raw = Buffer.alloc(H * (stride + 1));
  for (let y = 0; y < H; y += 1) {
    const row = y * (stride + 1);
    for (let x = 0; x < W; x += 1) {
      const [r, g, b] = pixel(x, y);
      raw[row + 1 + x * 3] = r;
      raw[row + 2 + x * 3] = g;
      raw[row + 3 + x * 3] = b;
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(W, 0);
  ihdr.writeUInt32BE(H, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 2; // colour type RGB
  writeFileSync(file, Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]));
}
const solid = (rgb) => () => rgb;
/** Build a temp frame set (entries are [name, pixel]) plus manifest.json, run `body(dir)`, clean up. */
function withFrames(entries, manifest, body) {
  const dir = mkdtempSync(join(tmpdir(), 'frame-triage-'));
  try {
    for (const [name, pixel] of entries) { writePng(join(dir, name), pixel); }
    writeFileSync(join(dir, 'manifest.json'), JSON.stringify(manifest));
    body(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('blank frame set with a manifest claiming fighters fails', function () {
  withFrames(NAMES.map((n) => [n, solid(BLANK_RGB)]), { fighters: 3 }, function (dir) {
    const gate = gateRows(triageOrbitDir(dir, NAMES));
    assert.equal(gate.pass, false);
    for (const r of gate.rows) {
      assert.equal(r.fighterPixels, 0);
      assert.equal(r.claimedFighters, 3, 'the claim is reported, not used');
      assert.equal(r.pass, false);
    }
  });
});

test('frame set with fighter-kit pixels passes, whatever the manifest says', function () {
  withFrames(NAMES.map((n) => [n, solid(FIGHTER_RGB)]), { fighters: 0 }, function (dir) {
    const gate = gateRows(triageOrbitDir(dir, NAMES));
    assert.equal(gate.pass, true);
    for (const r of gate.rows) {
      assert.equal(r.fighterPixels, W * H);
      assert.equal(r.pass, true);
    }
  });
});

test('one blank frame fails a set whose other frames show fighters', function () {
  const entries = [
    [NAMES[0], solid(FIGHTER_RGB)],
    [NAMES[1], solid(FIGHTER_RGB)],
    [NAMES[2], solid(BLANK_RGB)],
  ];
  withFrames(entries, {}, function (dir) {
    const gate = gateRows(triageOrbitDir(dir, NAMES));
    assert.equal(gate.pass, false);
    assert.deepEqual(gate.rows.map((r) => r.pass), [true, true, false]);
  });
});

test('a frame passes only strictly above the floor', function () {
  for (const [count, want] of [[FIGHTER_FLOOR_PIXELS, false], [FIGHTER_FLOOR_PIXELS + 1, true]]) {
    const pixel = (x, y) => (y * W + x < count ? FIGHTER_RGB : BLANK_RGB);
    withFrames([[NAMES[0], pixel]], {}, function (dir) {
      const gate = gateRows(triageOrbitDir(dir, [NAMES[0]]));
      assert.equal(gate.rows[0].fighterPixels, count);
      assert.equal(gate.pass, want, `${count} fighter pixels against floor ${FIGHTER_FLOOR_PIXELS}`);
    });
  }
});

test('an empty frame list never passes', function () {
  assert.equal(gateRows([]).pass, false);
});

test('decoded fighter-kit frames carry pixel proof and pass', function () {
  withFrames(NAMES.map((n) => [n, solid(FIGHTER_RGB)]), { fighters: 0 }, function (dir) {
    const gate = gateRows(triageOrbitDir(dir, NAMES));
    assert.equal(gate.pass, true);
    for (const r of gate.rows) {
      assert.equal(r.decoded, true);
      assert.equal(hasDecodedPixels(r), true);
      assert.equal(r.fighterPixels, W * H);
      assert.equal(r.pass, true);
    }
  });
});

test('an unopened frame never passes, even with a high fighter count', function () {
  const ghost = { file: 'ghost.png', fighterPixels: FIGHTER_FLOOR_PIXELS + 100, claimedFighters: 3 };
  assert.equal(hasDecodedPixels(ghost), false);
  assert.equal(gateRows([ghost]).pass, false);
  const plausible = { file: 'ghost.png', decoded: false, width: W, height: H, bytes: 100, redPixels: 0, fighterPixels: FIGHTER_FLOOR_PIXELS + 100, claimedFighters: 0 };
  assert.equal(hasDecodedPixels(plausible), false);
  assert.equal(gateRows([plausible]).pass, false);
});

test('orbit frames read IHDR 1280x720', { skip: ORBIT_SKIP }, function () {
  const rows = triageOrbitDir(ORBIT, NAMES);
  assert.equal(rows.length, 3);
  for (const r of rows) {
    assert.equal(r.width, 1280);
    assert.equal(r.height, 720);
    assert.ok(r.bytes > 1000);
  }
});

test('box-count heuristic recorded 2-3', { skip: ORBIT_SKIP }, function () {
  const rows = triageOrbitDir(ORBIT, NAMES);
  for (const r of rows) {
    assert.equal(r.boxesMin, 2);
    assert.equal(r.boxesMax, 3);
    assert.ok(r.boxesMax >= r.boxesMin);
  }
  assert.equal(ORBIT_TRUTH.boxesMin, 2);
  assert.equal(ORBIT_TRUTH.boxesMax, 3);
});

test('EXPECT-J1-FAIL: fighter pixels pinned 0, flips green on fighter render', { skip: ORBIT_SKIP }, function (t) {
  const rows = triageOrbitDir(ORBIT, NAMES);
  for (const r of rows) { assert.equal(r.fighterPixels, 0); }
  assert.equal(ORBIT_TRUTH.fighters, 0);
  t.diagnostic('EXPECT-J1-FAIL: orbit shows fighter pixels=0 red boxes=2-3; when fighters render, update ORBIT_TRUTH.fighters and assert the gate passes');
});

// rdc-diff.test.mjs - capture-diff gate: identical 0, one spike 1, changed clears floor.
// PORT(baldurk/renderdoc@v1.45:docs/python_api/examples/renderdoc/save_texture.py):
// OpenCaptureFile/OpenFile -> load frame PNGs; SetFrameEvent -> select frame
// index; TextureSave + SaveTexture(PNG) -> save diff texture; pixel compare
// -> differing count. Floor is FIGHTER_FLOOR_PIXELS from frame-triage.mjs.
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { decodePng } from './fighter-count.mjs';
import { FIGHTER_FLOOR_PIXELS } from './frame-triage.mjs';
import { encodePng, replayPair } from './rdc-diff.mjs';
const W = 1280;
const H = 720;
function solid(w, h, v = 128) {
  const buf = Buffer.alloc(w * h * 4, v);
  for (let i = 3; i < buf.length; i += 4) { buf[i] = 255; }
  return buf;
}
function withDirs(body) {
  const root = mkdtempSync(join(tmpdir(), 'rdc-diff-test-'));
  try {
    const a = join(root, 'a');
    const b = join(root, 'b');
    mkdirSync(a);
    mkdirSync(b);
    body(root, a, b);
  } finally { rmSync(root, { recursive: true, force: true }); }
}
test('identical replay pair diffs to 0 and writes a diff texture', () => {
  withDirs((root, a, b) => {
    const base = solid(W, H);
    writeFileSync(join(a, 'frame-0.png'), encodePng(W, H, base));
    writeFileSync(join(b, 'frame-0.png'), encodePng(W, H, base));
    const out = join(root, 'diff.png');
    const r = replayPair(a, b, out);
    assert.equal(r.count, 0);
    assert.equal(r.width, W);
    assert.equal(r.height, H);
    const tex = decodePng(readFileSync(out), 'diff');
    assert.equal(tex.width, W);
    assert.equal(tex.height, H);
  });
});
test('one spiked pixel diffs to 1', () => {
  withDirs((root, a, b) => {
    const base = solid(W, H);
    const spike = Buffer.from(base);
    spike[0] = (spike[0] + 100) & 255;
    writeFileSync(join(a, 'frame-0.png'), encodePng(W, H, base));
    writeFileSync(join(b, 'frame-0.png'), encodePng(W, H, spike));
    const r = replayPair(a, b, join(root, 'diff.png'));
    assert.equal(r.count, 1);
  });
});
test('changed frame clears FIGHTER_FLOOR_PIXELS', () => {
  withDirs((root, a, b) => {
    const w = 64;
    const h = 48;
    const base = solid(w, h);
    const moved = Buffer.from(base);
    for (let i = 0; i < 64; i += 1) { moved[i * 4] = (moved[i * 4] + 100) & 255; }
    writeFileSync(join(a, 'frame-0.png'), encodePng(w, h, base));
    writeFileSync(join(b, 'frame-0.png'), encodePng(w, h, moved));
    const r = replayPair(a, b, join(root, 'diff.png'));
    assert.equal(r.count, 64);
    assert.ok(r.count > FIGHTER_FLOOR_PIXELS);
    assert.equal(FIGHTER_FLOOR_PIXELS, 50);
  });
});

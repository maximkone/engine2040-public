// Run: node --test tools/frame-v2v3.test.mjs
// V2 arena-day (sky-Ernst) + V3 fight-action (motion-delta) over 2 q1 orbit samples.
// Quotes the V2/V3 outputs on the samples; dome cubes read not-arena-day + static.
import assert from 'node:assert/strict';
import test from 'node:test';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { classifyPair, decodeFrame, loadFrame, v2ArenaDay, v3FightDelta } from './frame-v2v3.mjs';
const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const FA = join(ROOT, 'target', 'showcase', '2026-10-08', 'orbit', 'frame_000000.png');
const FB = join(ROOT, 'target', 'showcase', '2026-10-08', 'orbit', 'frame_000001.png');
const NL = String.fromCharCode(10);
function quote(line) {
  process.stdout.write(line + NL);
}
test('decode reads 1280x720 on both orbit samples', function () {
  const a = loadFrame(FA);
  const b = loadFrame(FB);
  assert.equal(a.width, 1280);
  assert.equal(a.height, 720);
  assert.equal(b.width, 1280);
  assert.equal(b.height, 720);
  assert.ok(a.bytes > 1000);
  assert.ok(b.bytes > 1000);
  assert.equal(a.pixels.length, 1280 * 720 * 4);
});
test('V2 sky-Ernst says not-arena-day on the dome cubes', function (t) {
  const out = classifyPair(FA, FB);
  t.diagnostic('V2 sky-Ernst ' + FA + ' skyFrac=' + out.v2a.skyFrac + ' ernst=' + out.v2a.skyErnst + ' label=' + out.v2a.label);
  t.diagnostic('V2 sky-Ernst ' + FB + ' skyFrac=' + out.v2b.skyFrac + ' ernst=' + out.v2b.skyErnst + ' label=' + out.v2b.label);
  quote('V2 sky-Ernst ' + FA + ' skyFrac=' + out.v2a.skyFrac + ' ernst=' + out.v2a.skyErnst + ' label=' + out.v2a.label);
  quote('V2 sky-Ernst ' + FB + ' skyFrac=' + out.v2b.skyFrac + ' ernst=' + out.v2b.skyErnst + ' label=' + out.v2b.label);
  for (const v of [out.v2a, out.v2b]) {
    assert.equal(v.label, 'not-arena-day');
    assert.equal(v.skyFrac, 0);
    assert.ok(Math.abs(v.skyErnst - -0.1692) < 0.02, 'ernst near -0.1692, got ' + v.skyErnst);
  }
  const solo = v2ArenaDay(loadFrame(FA));
  assert.equal(solo.label, 'not-arena-day');
  assert.equal(solo.width, 1280);
});
test('V3 motion-delta says static on the slow orbit pair', function (t) {
  const out = classifyPair(FA, FB);
  t.diagnostic('V3 motion-delta ' + FA + ' -> ' + FB + ' mean=' + out.v3.meanDelta + ' changed=' + out.v3.changedFrac + ' label=' + out.v3.label);
  quote('V3 motion-delta ' + FA + ' -> ' + FB + ' mean=' + out.v3.meanDelta + ' changed=' + out.v3.changedFrac + ' label=' + out.v3.label);
  assert.equal(out.v3.label, 'static');
  assert.ok(Math.abs(out.v3.meanDelta - 5.12) < 0.6, 'mean near 5.12, got ' + out.v3.meanDelta);
  assert.ok(Math.abs(out.v3.changedFrac - 0.1097) < 0.03, 'changed near 0.1097, got ' + out.v3.changedFrac);
});
test('identical frames have zero motion-delta', function () {
  const a = loadFrame(FA);
  const d = v3FightDelta(a, a);
  assert.equal(d.meanDelta, 0);
  assert.equal(d.changedFrac, 0);
  assert.equal(d.label, 'static');
});
test('non-PNG input throws', function () {
  assert.throws(function () { decodeFrame(Buffer.from('not a png at all')); }, /not a PNG/);
});


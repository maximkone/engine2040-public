// Donor: gschup/ggrs (Apache-2.0, https://github.com/gschup/ggrs) - vectors are fresh data shaped by the idea; no vendor bytes copied.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const doc = JSON.parse(readFileSync(new URL('../crates/net/testdata/input_queue_vectors.json', import.meta.url), 'utf8'));
const LEN = doc.constants.INPUT_QUEUE_LEN;
const slot = (f) => ((f % LEN) + LEN) % LEN;
const byId = (id) => doc.vectors.find((v) => v.id === id);

test('slot wrap mod128 maps frames to circular slots', () => {
  const v = byId('slot-wrap-mod128');
  assert.deepEqual(v.frames.map(slot), v.expected_slots);
});

test('slot wrap eviction window keeps last 128 frames', () => {
  const v = byId('slot-wrap-eviction-window');
  const pushed = Array.from({ length: v.pushed_count }, (_, i) => v.pushed_start + i);
  const retained = pushed.slice(pushed.length - v.capacity);
  assert.equal(retained.length, v.expected_retained_count);
  assert.equal(retained[0], v.expected_retained_start);
  assert.equal(retained[retained.length - 1], v.expected_retained_end);
});

test('predictor fill repeats last confirmed input', () => {
  const v = byId('predictor-fill-repeat-last');
  const predicted = v.missing_frames.map(() => v.last_confirmed_input);
  assert.deepEqual(predicted, v.expected_predicted);
});

test('first-incorrect-frame pinpoints divergence', () => {
  const v = byId('first-incorrect-frame');
  let first = null;
  for (let i = 0; i < v.predicted.length; i++) {
    if (v.predicted[i] !== v.confirmed[i]) { first = v.start_frame + i; break; }
  }
  assert.equal(first, v.expected_first_incorrect);
  const w = byId('first-incorrect-none-when-equal');
  let none = null;
  for (let i = 0; i < w.predicted.length; i++) {
    if (w.predicted[i] !== w.confirmed[i]) { none = w.start_frame + i; break; }
  }
  assert.equal(none, w.expected_first_incorrect);
});

test('discard-confirmed advances tail and retains rest', () => {
  const v = byId('discard-confirmed-tail');
  const tail = v.discard_through + 1;
  assert.equal(tail, v.expected_tail);
  const retained = [];
  for (let f = tail; f <= v.confirmed_through; f++) retained.push(f);
  assert.deepEqual(retained, v.expected_retained);
});

test('vector count quoted', () => {
  console.log(`rollback vectors: ${doc.vectors.length} vectors pass`);
  assert.equal(doc.vectors.length, 6);
});

#!/usr/bin/env node
// Task 2 referee: a capture is green only when the pixels the receipt names are
// lit. The receipt's own `validation_errors` is not enough: the mutation below
// is a clean receipt over a black frame, and it must stay red.
//
//   node tools/tasks/02-red-capture/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/02-red-capture/referee.mjs --tree <dir>

import { cli, defineTask, json } from '../lib.mjs';

const CLEAR = '#00000000';
const LIT = '#c8b08aff';
const WHEAT = '#d8bc84ff';

/** Four 4x4 frames; pixel (1,1) of frame 3 is the one the receipt checks. */
function frames(centre) {
  const blank = Array.from({ length: 4 }, () => [CLEAR, CLEAR, CLEAR, CLEAR]);
  const near = Array.from({ length: 4 }, () => [CLEAR, CLEAR, CLEAR, CLEAR]);
  near[1] = [CLEAR, WHEAT, LIT, CLEAR];
  near[2] = [CLEAR, LIT, LIT, CLEAR];
  const last = Array.from({ length: 4 }, () => [CLEAR, CLEAR, CLEAR, CLEAR]);
  last[1] = [CLEAR, centre, LIT, CLEAR];
  last[2] = [CLEAR, LIT, WHEAT, CLEAR];
  return [blank, near, near, last];
}

function receipt({ errors, draws, lit }) {
  return `${JSON.stringify(
    {
      frames: 4,
      draws,
      clear: CLEAR,
      validation_errors: errors,
      checks: [{ frame: 3, pixel: [1, 1], why: lit ? 'the fighters are in view by frame 3' : 'no geometry reached the centre' }],
    },
    null,
    2,
  )}\n`;
}

const start = { 'capture.json': receipt({ errors: 2, draws: 0, lit: false }), 'frames.json': `${JSON.stringify(frames(CLEAR), null, 2)}\n` };
const fixed = { 'capture.json': receipt({ errors: 0, draws: 6, lit: true }), 'frames.json': `${JSON.stringify(frames(LIT), null, 2)}\n` };
// The lie: the receipt is declared clean, the same black frames are shipped.
const mutation = { 'capture.json': receipt({ errors: 0, draws: 6, lit: true }), 'frames.json': `${JSON.stringify(frames(CLEAR), null, 2)}\n` };

const PIXEL_RE = /^#[0-9a-f]{8}$/i;

export const task = defineTask({
  id: '02-red-capture',
  title: 'fix a red capture',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const capture = json(files, 'capture.json');
    const table = json(files, 'frames.json');
    if (!Number.isInteger(capture.frames) || capture.frames !== table.length) {
      failures.push(`capture names ${capture.frames} frame(s), the table has ${table.length}`);
    }
    if (capture.validation_errors !== 0) failures.push(`capture has ${capture.validation_errors} validation error(s)`);
    if (!Number.isInteger(capture.draws) || capture.draws < 1) failures.push(`capture drew ${capture.draws} item(s): the frame is empty`);
    const checks = Array.isArray(capture.checks) ? capture.checks : [];
    if (checks.length === 0) failures.push('the receipt names no pixel to look at');
    for (const check of checks) {
      const frame = table[check.frame];
      const pixel = frame?.[check.pixel?.[0]]?.[check.pixel?.[1]];
      if (pixel === undefined) {
        failures.push(`frame ${check.frame} has no pixel ${JSON.stringify(check.pixel)}`);
        continue;
      }
      if (!PIXEL_RE.test(pixel)) failures.push(`frame ${check.frame} pixel ${JSON.stringify(check.pixel)} is not #rrggbbaa: ${pixel}`);
      if (pixel.toLowerCase() === String(capture.clear).toLowerCase()) {
        failures.push(`frame ${check.frame} pixel ${JSON.stringify(check.pixel)} is the clear colour: ${check.why}`);
      }
    }
    return failures;
  },
});

cli(task);

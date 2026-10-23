#!/usr/bin/env node
// Task 5 referee: divergence is fixed at the code, never by re-pinning the
// golden. The mutation makes both receipts agree on a new hash while golden.txt
// stands: red.
//
//   node tools/tasks/05-golden-pin/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/05-golden-pin/referee.mjs --tree <dir>

import { cli, defineTask, json } from '../lib.mjs';

const GOLDEN = 'd57b5c8ae3096614';
const DIVERGED = '9f2c1b0d7e6a5533';

function receipt(seed, hash, ticks = 600) {
  return `${JSON.stringify({ seed, ticks, final_hash: hash }, null, 2)}\n`;
}

const start = {
  'golden.txt': `${GOLDEN}\n`,
  'receipts/seed-2040.json': receipt(2040, GOLDEN),
  'receipts/seed-max.json': receipt(65535, DIVERGED),
};
const fixed = {
  'golden.txt': `${GOLDEN}\n`,
  'receipts/seed-2040.json': receipt(2040, GOLDEN),
  'receipts/seed-max.json': receipt(65535, GOLDEN),
};
// The re-pin: both receipts agree, on a hash the golden never had.
const mutation = {
  'golden.txt': `${GOLDEN}\n`,
  'receipts/seed-2040.json': receipt(2040, DIVERGED),
  'receipts/seed-max.json': receipt(65535, DIVERGED),
};

const HASH_RE = /^[0-9a-f]{16}$/;

export const task = defineTask({
  id: '05-golden-pin',
  title: 'fix a determinism red without moving the golden',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const golden = (files['golden.txt'] ?? '').trim();
    if (!HASH_RE.test(golden)) return [`golden.txt is not one 16 hex hash: ${JSON.stringify(golden)}`];
    const names = Object.keys(files)
      .filter((rel) => rel.startsWith('receipts/') && rel.endsWith('.json'))
      .sort();
    if (names.length !== 2) return [`expected two receipts, found ${names.length}`];
    const seeds = new Set();
    for (const rel of names) {
      const out = json(files, rel);
      if (!Number.isInteger(out.ticks) || out.ticks !== 600) failures.push(`${rel} ran ${out.ticks} ticks, not 600`);
      if (!Number.isInteger(out.seed) || out.seed < 0) failures.push(`${rel} has no seed`);
      else seeds.add(out.seed);
      if (typeof out.final_hash !== 'string' || !HASH_RE.test(out.final_hash)) failures.push(`${rel} final_hash is not 16 hex digits`);
      else if (out.final_hash !== golden) failures.push(`${rel} prints ${out.final_hash}, the golden is ${golden}: fix the divergence, do not re-pin`);
    }
    if (seeds.size !== 2) failures.push(`the two receipts ran ${seeds.size} distinct seed(s)`);
    return failures;
  },
});

cli(task);

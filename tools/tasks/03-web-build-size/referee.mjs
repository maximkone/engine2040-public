#!/usr/bin/env node
// Task 3 referee: the web build is smaller only when both halves are there.
// The mutation drops the wasm artifact and reports a small total: red.
//
//   node tools/tasks/03-web-build-size/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/03-web-build-size/referee.mjs --tree <dir>

import { cli, defineTask, json } from '../lib.mjs';

const REQUIRED = ['engine.js', 'engine_bg.wasm'];

function build({ sizes, magic, flags = ['web', 'wasm32'] }) {
  return `${JSON.stringify(
    {
      budget_bytes: 921600,
      entries: Object.fromEntries(Object.entries(sizes).map(([name, bytes]) => [name, { bytes }])),
      wasm_magic: magic,
      flags,
    },
    null,
    2,
  )}\n`;
}

const start = { 'build.json': build({ sizes: { 'engine.js': 700000, 'engine_bg.wasm': 300000 }, magic: '0061736d' }) };
const fixed = { 'build.json': build({ sizes: { 'engine.js': 620000, 'engine_bg.wasm': 250000 }, magic: '0061736d' }) };
const mutation = { 'build.json': build({ sizes: { 'engine.js': 520000 }, magic: '00000000' }) };

export const task = defineTask({
  id: '03-web-build-size',
  title: 'make the web build smaller',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const out = json(files, 'build.json');
    const entries = out.entries ?? {};
    for (const name of REQUIRED) {
      if (!(name in entries)) failures.push(`the build has no ${name}: a smaller total with a missing half is not a win`);
    }
    let total = 0;
    for (const [name, entry] of Object.entries(entries)) {
      const bytes = entry?.bytes;
      if (!Number.isInteger(bytes) || bytes <= 0) failures.push(`${name} has no size`);
      else total += bytes;
    }
    if (!Number.isInteger(out.budget_bytes) || out.budget_bytes <= 0) failures.push('build.json has no budget_bytes');
    else if (total > out.budget_bytes) failures.push(`the build is ${total} bytes, the budget is ${out.budget_bytes}`);
    if (out.wasm_magic !== '0061736d') failures.push(`wasm magic is ${JSON.stringify(out.wasm_magic)}, not 0061736d: the wasm artifact is not real`);
    if (!Array.isArray(out.flags) || !out.flags.includes('wasm32')) failures.push('the wasm32 flag is gone: the web build no longer targets wasm');
    return failures;
  },
});

cli(task);

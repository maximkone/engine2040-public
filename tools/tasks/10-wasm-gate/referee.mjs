#!/usr/bin/env node
// Task 10 referee: the std::time body sits behind a not(wasm32) gate, the wasm
// twin never touches std::time, and the wasm-check leg is green. The mutation
// puts the attribute on with the wrong polarity: red.
//
//   node tools/tasks/10-wasm-gate/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/10-wasm-gate/referee.mjs --tree <dir>

import { cli, defineTask, json } from '../lib.mjs';

const FILE = 'clock.rs.txt';
const DESKTOP_GATE = '#[cfg(not(target_arch = "wasm32"))]';
const WASM_GATE = '#[cfg(target_arch = "wasm32")]';

const BODY = [
  '    let start = std::time::Instant::now();',
  '    start.elapsed().as_millis() as u64',
];
const TWIN = [
  WASM_GATE,
  'pub fn now_millis() -> u64 {',
  '    // The browser clock arrives through the shim.',
  '    shim_now_millis()',
  '}',
];

function source({ gate, twin }) {
  const lines = ['// Fixture: the desktop clock and its browser twin.', ''];
  if (gate !== undefined) lines.push(gate);
  lines.push('pub fn now_millis() -> u64 {', ...BODY, '}');
  if (twin) lines.push('', ...TWIN);
  lines.push('');
  return lines.join('\n');
}

function web({ leg }) {
  return `${JSON.stringify(
    {
      legs: [
        { name: 'wasm-check', status: leg },
        { name: 'dist/web', status: 'green' },
      ],
    },
    null,
    2,
  )}\n`;
}

const start = { [FILE]: source({}), 'web.json': web({ leg: 'red' }) };
const fixed = { [FILE]: source({ gate: DESKTOP_GATE, twin: true }), 'web.json': web({ leg: 'green' }) };
// Gated as wasm-only: the desktop build loses its clock and wasm still gets std::time.
const mutation = { [FILE]: source({ gate: WASM_GATE }), 'web.json': web({ leg: 'green' }) };

export const task = defineTask({
  id: '10-wasm-gate',
  title: 'gate a desktop clock for the wasm build',
  variants: { start, fixed, mutation },
  checks(files) {
    const failures = [];
    const text = files[FILE];
    if (text === undefined) return [`${FILE} is missing`];
    const lines = text.split('\n');
    const use = lines.findIndex((line) => line.includes('Instant::now'));
    if (use < 0) return [`${FILE} does not use Instant::now: this task gates a real clock`];
    let signature = -1;
    for (let index = use; index >= 0; index -= 1) {
      if (lines[index].startsWith('pub fn now_millis')) {
        signature = index;
        break;
      }
    }
    if (signature < 0) failures.push('the clock function has no signature');
    else {
      const above = lines[signature - 1]?.trim();
      if (above !== DESKTOP_GATE) failures.push(`the clock body is gated with ${JSON.stringify(above)}, not ${DESKTOP_GATE}`);
    }
    const twin = lines.findIndex((line) => line.trim() === WASM_GATE);
    if (twin < 0) failures.push(`there is no ${WASM_GATE} twin`);
    else {
      const wasmBody = lines.slice(twin + 1).join('\n');
      if (/std::time|Instant/.test(wasmBody)) failures.push('the wasm twin still reads std::time: the browser has no such clock');
      if (!/shim_now_millis|js_now/.test(wasmBody)) failures.push('the wasm twin takes no clock from the shim');
    }
    const out = json(files, 'web.json');
    const leg = (out.legs ?? []).find((entry) => entry?.name === 'wasm-check');
    if (leg === undefined) failures.push('web.json has no wasm-check leg');
    else if (leg.status !== 'green') failures.push(`the wasm-check leg is ${JSON.stringify(leg.status)}, not green`);
    return failures;
  },
});

cli(task);

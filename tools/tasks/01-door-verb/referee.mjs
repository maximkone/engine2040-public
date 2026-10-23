#!/usr/bin/env node
// Task 1 referee: a door verb is manifest + handler + doc, and the handler runs.
//
//   node tools/tasks/01-door-verb/referee.mjs --apply start|fixed|mutation --dest <dir>
//   node tools/tasks/01-door-verb/referee.mjs --tree <dir>

import { cli, defineTask, importFrom, json } from '../lib.mjs';

const VERB = 'scene.frame';

const BASE = [
  { name: 'scene.read', handler: 'scene.read', doc: 'read the scene revision' },
  { name: 'tx.preview', handler: 'tx.preview', doc: 'preview a change' },
];
const FRAME = { name: VERB, handler: VERB, doc: 'frame: put the camera on one item' };

function manifest(verbs) {
  return `${JSON.stringify({ verbs }, null, 2)}\n`;
}

function handlers({ withFrame }) {
  const lines = [
    '// The door handlers: one per verb in verbs.json, nothing else.',
    'export const handlers = {',
    "  'scene.read': () => ({ ok: true, revision: 1 }),",
    "  'tx.preview': (args) => ({ ok: true, approval: 'single-use', change: args.change ?? null }),",
  ];
  if (withFrame) lines.push(`  '${VERB}': (args) => ({ ok: true, framed: args.name }),`);
  lines.push('};', '');
  return lines.join('\n');
}

const start = { 'verbs.json': manifest(BASE), 'handlers.mjs': handlers({ withFrame: false }) };
const fixed = { 'verbs.json': manifest([...BASE, FRAME]), 'handlers.mjs': handlers({ withFrame: true }) };
// The half-wired verb: the manifest lists it, the handler map does not.
const mutation = { 'verbs.json': manifest([...BASE, FRAME]), 'handlers.mjs': handlers({ withFrame: false }) };

export const task = defineTask({
  id: '01-door-verb',
  title: 'add a door verb',
  variants: { start, fixed, mutation },
  async checks(files) {
    const failures = [];
    const verbs = json(files, 'verbs.json');
    const list = Array.isArray(verbs.verbs) ? verbs.verbs : [];
    if (list.length === 0) failures.push('verbs.json lists no verbs');
    const source = files['handlers.mjs'] ?? '';
    for (const verb of list) {
      if (typeof verb.name !== 'string' || verb.name.trim() === '') failures.push('verbs.json has a verb with no name');
      if (typeof verb.doc !== 'string' || verb.doc.trim() === '') failures.push(`verb ${verb.name} has no doc line`);
      if (verb.name !== verb.handler) failures.push(`verb ${verb.name} names handler ${verb.handler}: the door keeps them equal`);
      if (!source.includes(`'${verb.handler}':`)) failures.push(`verbs.json lists ${verb.handler} but handlers.mjs has no handler`);
    }
    const names = list.map((verb) => verb.name);
    if (new Set(names).size !== names.length) failures.push('verbs.json lists a verb twice');
    if (!names.includes(VERB)) failures.push(`verbs.json does not list ${VERB}`);
    if (failures.length > 0) return failures;
    const mod = await importFrom(files, 'handlers.mjs');
    const handler = mod.handlers?.[VERB];
    if (typeof handler !== 'function') return [`${VERB} has no handler function`];
    let answer;
    try {
      answer = handler({ name: 'arena' });
    } catch (error) {
      return [`${VERB} threw on { name: 'arena' }: ${error.message}`];
    }
    if (answer?.ok !== true || answer?.framed !== 'arena') {
      failures.push(`${VERB} does not frame the named item: ${JSON.stringify(answer)}`);
    }
    return failures;
  },
});

cli(task);

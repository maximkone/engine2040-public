import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..');
const skill = readFileSync(join(here, 'SKILL.md'), 'utf8');

test('frontmatter names acquire-codebase', () => {
  assert.ok(skill.includes('name: acquire-codebase'));
});

test('trigger covers unfamiliar crates before reading', () => {
  assert.match(skill, /entering unfamiliar crates/i);
  assert.match(skill, /before reading code/i);
});

test('seven check markers present', () => {
  for (const m of ['intent', 'scan', 'architecture', 'boundaries', 'determinism', 'tests', 'ask-user']) {
    assert.match(skill, new RegExp(m, 'i'), `missing marker ${m}`);
  }
});

test('report carries seven headings', () => {
  for (const h of ['### Intent', '### Scan', '### Architecture', '### Boundaries', '### Determinism', '### Tests', '### ASK-USER']) {
    assert.ok(skill.includes(h), `missing heading ${h}`);
  }
});

test('read-only guard present', () => {
  assert.match(skill, /read-only/i);
  assert.match(skill, /never checkout, rebase, or edit/i);
});

test('board slice focus exits 0 on this tree', { timeout: 90000 }, () => {
  const r = spawnSync(process.execPath, [join(root, 'tools', 'board-slice.mjs'), '--focus'], { encoding: 'utf8', timeout: 60000 });
  assert.equal(r.status, 0);
});

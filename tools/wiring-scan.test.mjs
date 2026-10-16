import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkRise, baselineCounts, baselineOf, baselineTotal, countByCrate, formatReport, helpText, scanIslands } from './wiring-scan.mjs';

function makeFixture() {
  const root = mkdtempSync(join(tmpdir(), 'wiring-'));
  const aSrc = join(root, 'crate-a', 'src');
  const bSrc = join(root, 'crate-b', 'src');
  mkdirSync(aSrc, { recursive: true });
  mkdirSync(bSrc, { recursive: true });
  const aLib = [
    'pub fn used_elsewhere() {}',
    'pub fn used_only_in_test() {}',
    'pub fn never_used() {}',
    '#[cfg(test)]',
    'mod tests {',
    '  fn t() { super::used_only_in_test(); }',
    '}',
    ''
  ].join('\n');
  const bLib = [
    'fn call_it() { used_elsewhere(); }',
    ''
  ].join('\n');
  writeFileSync(join(aSrc, 'lib.rs'), aLib);
  writeFileSync(join(bSrc, 'lib.rs'), bLib);
  return root;
}

function cleanup(dir) {
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch (err) { console.warn(`WARN wiring-scan-test: cleanup skipped for ${dir}: ${err?.message ?? err}`); }
}

test('fixture reports test-only and unused fns, not the shared one', () => {
  const root = makeFixture();
  try {
    const islands = scanIslands(root);
    assert.equal(islands.length, 2);
    const names = islands.map(function (r) { return r.name; });
    assert.deepEqual(names, ['used_only_in_test', 'never_used']);
    for (const r of islands) {
      assert.equal(r.kind, 'fn');
      assert.equal(r.crate, 'crate-a');
    }
  } finally {
    cleanup(root);
  }
});

// Committed fixture under tools/fixtures/wiring: same three-fn shape on disk.
test('wiring_committed_fixture_reports_last_two', () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const root = join(here, 'fixtures', 'wiring');
  const islands = scanIslands(root);
  assert.equal(islands.length, 2);
  const names = islands.map(function (r) { return r.name; });
  assert.deepEqual(names, ['used_only_in_test', 'never_used']);
});

test('same fixture twice gives byte-equal output', () => {
  const root = makeFixture();
  try {
    const first = formatReport(scanIslands(root));
    const second = formatReport(scanIslands(root));
    assert.equal(first, second);
    assert.ok(first.startsWith('islands: 2\n'));
  } finally {
    cleanup(root);
  }
});

test('empty dir reports islands zero', () => {
  const root = mkdtempSync(join(tmpdir(), 'wiring-empty-'));
  try {
    const islands = scanIslands(root);
    assert.equal(islands.length, 0);
    assert.equal(formatReport(islands), 'islands: 0\n');
  } finally {
    cleanup(root);
  }
});

// JSON bytes stay equal for the same fixture.
test('wiring_json_byte_equal_x2', () => {
  const root = makeFixture();
  try {
    const first = JSON.stringify(scanIslands(root));
    const second = JSON.stringify(scanIslands(root));
    assert.equal(first, second);
    assert.equal(JSON.parse(first).length, 2);
  } finally {
    cleanup(root);
  }
});

// Top ten lists bigger crates first, ties by name.
test('wiring_top_ten_orders_counts', () => {
  const islands = [
    { crate: 'crate-b', file: 'crate-b/src/lib.rs', line: 1, kind: 'fn', name: 'b1' },
    { crate: 'crate-b', file: 'crate-b/src/lib.rs', line: 2, kind: 'fn', name: 'b2' },
    { crate: 'crate-a', file: 'crate-a/src/lib.rs', line: 1, kind: 'fn', name: 'a1' },
    { crate: 'crate-a', file: 'crate-a/src/lib.rs', line: 2, kind: 'fn', name: 'a2' },
    { crate: 'crate-c', file: 'crate-c/src/lib.rs', line: 1, kind: 'fn', name: 'c1' }
  ];
  const counts = countByCrate(islands);
  assert.deepEqual(counts, { 'crate-b': 2, 'crate-a': 2, 'crate-c': 1 });
  const text = formatReport(islands);
  const tops = text.split('\n').filter(function (line) { return line.startsWith('top '); });
  assert.deepEqual(tops, ['top 2 crate-a', 'top 2 crate-b', 'top 1 crate-c']);
});

// Only crates above baseline count as risen.
test('wiring_baseline_rise_detects', () => {
  const risen = checkRise({ 'crate-a': 3, 'crate-b': 2, 'crate-c': 1 }, { 'crate-a': 2, 'crate-b': 2, 'crate-c': 5 });
  assert.deepEqual(risen, ['crate-a']);
  assert.deepEqual(checkRise({ 'crate-new': 1 }, {}), ['crate-new']);
});

// Committed baseline pin: baselineOf shape plus on-disk file shape.
test('wiring_baseline_pin_shape', () => {
  const islands = [
    { crate: 'crate-a', file: 'crate-a/src/lib.rs', line: 1, kind: 'fn', name: 'a1' },
    { crate: 'crate-b', file: 'crate-b/src/lib.rs', line: 1, kind: 'fn', name: 'b1' }
  ];
  const base = baselineOf(islands);
  assert.equal(base.total, 2);
  assert.deepEqual(base.by_crate, { 'crate-a': 1, 'crate-b': 1 });
  assert.equal(baselineTotal(base), 2);
  assert.deepEqual(baselineCounts(base), { 'crate-a': 1, 'crate-b': 1 });
  // A rise above the pinned shape is detected.
  assert.deepEqual(checkRise({ 'crate-a': 2, 'crate-b': 1 }, baselineCounts(base)), ['crate-a']);
  assert.equal(baselineTotal({ 'crate-a': 2 }) + 0, 2);
});

test('wiring_help_lists_flags', () => {
  const text = helpText();
  assert.ok(text.includes('--json'));
  assert.ok(text.includes('--baseline'));
  assert.ok(text.includes('--check'));
  assert.ok(text.includes('--help'));
  assert.ok(text.includes('wiring-scan.json'));
});

test('wiring_baseline_file_committed', async () => {
  const { readFileSync } = await import('node:fs');
  const { dirname, join } = await import('node:path');
  const { fileURLToPath } = await import('node:url');
  const here = dirname(fileURLToPath(import.meta.url));
  const raw = readFileSync(join(here, 'wiring-scan.baseline.json'), 'utf8');
  const baseline = JSON.parse(raw);
  assert.equal(typeof baseline.total, 'number');
  assert.ok(baseline.total > 0);
  assert.equal(typeof baseline.by_crate, 'object');
  assert.ok(Object.keys(baseline.by_crate).length > 0);
  assert.equal(baselineTotal(baseline), baseline.total);
});

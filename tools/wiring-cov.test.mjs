import assert from 'node:assert/strict';
import test from 'node:test';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { INSTALL_LINE, formatSummary, groupZeroHits, hasLlvmCov } from './wiring-cov.mjs';

// Canned llvm-cov export JSON: zero-hit fns in render + game, one hot fn.
function canned() {
  return {
    data: [{
      functions: [
        { name: 'render_cold', count: 0, filenames: ['C:/engine2040/crates/render/src/cold.rs'] },
        { name: 'render_hot', count: 41, filenames: ['C:/engine2040/crates/render/src/hot.rs'] },
        { name: 'game_cold_b', count: 0, filenames: ['crates/game/src/b.rs'] },
        { name: 'game_cold_a', count: 0, filenames: ['crates/game/src/a.rs'] },
      ],
    }],
  };
}

test('canned fixture gives the same grouped zero-hit list x2', () => {
  const a = groupZeroHits(canned());
  const b = groupZeroHits(canned());
  assert.deepEqual(a, b);
  assert.deepEqual(a, {
    'engine-game': ['game_cold_a', 'game_cold_b'],
    'engine-render': ['render_cold'],
  });
});

test('hot functions never count and lists sort by crate then name', () => {
  const groups = groupZeroHits(canned());
  assert.ok(!Object.values(groups).flat().includes('render_hot'));
  assert.deepEqual(Object.keys(groups), ['engine-game', 'engine-render']);
  assert.deepEqual(groups['engine-game'], ['game_cold_a', 'game_cold_b']);
});

test('summary names counts and the output path', () => {
  const line = formatSummary({ 'engine-game': ['a', 'b'] }, 'out.json');
  assert.equal(line, 'wiring-cov: 2 zero-hit fns in 1 crates -> out.json');
});

test('without cargo-llvm-cov the tool prints the install line and exits 2', () => {
  if (hasLlvmCov()) {
    return;
  }
  const dir = mkdtempSync(join(tmpdir(), 'wiring-cov-'));
  try {
    const r = spawnSync(process.execPath, ['tools/wiring-cov.mjs', '--out', join(dir, 'w.json')], { encoding: 'utf8' });
    assert.equal(r.status, 2);
    assert.match(`${r.stdout}${r.stderr}`, new RegExp(INSTALL_LINE.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('canned json file round-trips through --from-json', () => {
  const dir = mkdtempSync(join(tmpdir(), 'wiring-cov-'));
  try {
    const src = join(dir, 'raw.json');
    const out = join(dir, 'w.json');
    writeFileSync(src, JSON.stringify(canned()));
    const r = spawnSync(process.execPath, ['tools/wiring-cov.mjs', '--from-json', src, '--out', out], { encoding: 'utf8' });
    assert.equal(r.status, 0);
    const written = JSON.parse(readFileSync(out, 'utf8'));
    assert.deepEqual(written.crates, groupZeroHits(canned()));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

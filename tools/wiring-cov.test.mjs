import assert from 'node:assert/strict';
import test from 'node:test';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  ARENA_ARGS,
  INSTALL_LINE,
  ROW_CRATES,
  analyze,
  cargoPlan,
  formatSummary,
  groupZeroHits,
  hasLlvmCov,
  runPlan,
} from './wiring-cov.mjs';

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

// A generic fn is one llvm-cov row per instantiation.
function withGenerics() {
  return {
    data: [{
      functions: [
        { name: 'render::lerp', count: 0, filenames: ['crates/render/src/math.rs'] },
        { name: 'render::lerp', count: 9, filenames: ['crates/render/src/math.rs'] },
        { name: 'render::dead', count: 0, filenames: ['crates/render/src/math.rs'] },
        { name: 'render::dead', count: 0, filenames: ['crates/render/src/math.rs'] },
        { name: 'new', count: 0, filenames: ['crates/render/src/a.rs'] },
        { name: 'new', count: 5, filenames: ['crates/render/src/b.rs'] },
        { name: 'net_cold', count: 0, filenames: ['crates/net/src/x.rs'] },
        { name: 'vendor_cold', count: 0, filenames: ['C:/Users/me/.cargo/registry/src/dep/lib.rs'] },
        { name: 'ai_cold', count: 0, filenames: ['crates/ai/src/q.rs'] },
      ],
    }],
  };
}

test('one live instantiation keeps a generic fn off the list', () => {
  const g = groupZeroHits(withGenerics());
  assert.ok(!g['engine-render'].includes('render::lerp'));
  assert.ok(g['engine-render'].includes('render::dead'));
  // Same short name in two files stays two fns: only a.rs is cold.
  assert.equal(g['engine-render'].filter((n) => n === 'new').length, 1);
});

test('default scope is the five row crates; null scope keeps the rest', () => {
  assert.deepEqual(ROW_CRATES, ['engine-ai', 'engine-anim', 'engine-audio', 'engine-game', 'engine-render']);
  const scoped = groupZeroHits(withGenerics());
  assert.deepEqual(Object.keys(scoped), ['engine-ai', 'engine-render']);
  const all = groupZeroHits(withGenerics(), { crates: null });
  assert.deepEqual(Object.keys(all), ['engine-ai', 'engine-net', 'engine-render', 'unknown']);
  assert.deepEqual(groupZeroHits(withGenerics(), { crates: ['engine-net'] }), { 'engine-net': ['net_cold'] });
});

test('totals count every fn per crate, zero-hit ones included', () => {
  const { totals } = analyze(withGenerics());
  assert.deepEqual(totals['engine-render'], { zero: 2, total: 4 });
  assert.deepEqual(totals['engine-ai'], { zero: 1, total: 1 });
});

test('a report with no data lists nothing and does not throw', () => {
  assert.deepEqual(analyze({}), { crates: {}, totals: {} });
  assert.deepEqual(analyze(null), { crates: {}, totals: {} });
  assert.deepEqual(analyze({ data: [{ functions: [{ count: 0 }, { name: '', count: 0 }] }] }), { crates: {}, totals: {} });
});

test('cargo plan runs the arena bin with the app feature, never cargo test', () => {
  const plan = cargoPlan('raw.json');
  assert.equal(plan.length, 3);
  assert.deepEqual(plan[0], ['llvm-cov', 'clean', '--workspace']);
  assert.deepEqual(plan[1], [
    'llvm-cov', 'run', '--no-report', '-p', 'engine-game', '--features', 'app', '--bin', 'arena', '--',
    '--scene', 'arena', '--fighters', '8', '--frames', '300', '--exit',
  ]);
  assert.deepEqual(plan[2], ['llvm-cov', 'report', '--json', '--output-path', 'raw.json']);
  assert.ok(plan.every((a) => !a.includes('test')));
  assert.deepEqual(ARENA_ARGS.slice(0, 2), ['--scene', 'arena']);
});

test('the planned bin target really needs the feature the plan passes', () => {
  // Cargo refuses `--bin arena` without `app`; the plan must not drift from the manifest.
  const toml = readFileSync(new URL('../crates/game/Cargo.toml', import.meta.url), 'utf8');
  const block = toml.split('[[bin]]').find((b) => /^\s*name = "arena"\s*$/m.test(b.split(/\r?\n\r?\n/)[0])) ?? '';
  assert.match(block, /required-features = \["app"\]/);
  const run = cargoPlan('raw.json')[1];
  assert.equal(run[run.indexOf('--features') + 1], 'app');
  assert.equal(run[run.indexOf('--bin') + 1], 'arena');
});

test('runPlan stops at the first failing step and names it', () => {
  const seen = [];
  const bad = runPlan(cargoPlan('r.json'), (args) => { seen.push(args[1]); return args[1] === 'run' ? 101 : 0; });
  assert.deepEqual(seen, ['clean', 'run']);
  assert.equal(bad.ok, false);
  assert.equal(bad.status, 101);
  assert.match(bad.step, /^cargo llvm-cov run --no-report -p engine-game/);
  const seen2 = [];
  assert.deepEqual(runPlan(cargoPlan('r.json'), (args) => { seen2.push(args[1]); return 0; }), { ok: true });
  assert.deepEqual(seen2, ['clean', 'run', 'report']);
});

test('fake cargo writing the raw export leads to the same report as the fixture', () => {
  const dir = mkdtempSync(join(tmpdir(), 'wiring-cov-'));
  try {
    const raw = join(dir, 'raw.json');
    const res = runPlan(cargoPlan(raw), (args) => {
      if (args[1] === 'report') writeFileSync(args[args.indexOf('--output-path') + 1], JSON.stringify(canned()));
      return 0;
    });
    assert.equal(res.ok, true);
    assert.deepEqual(groupZeroHits(JSON.parse(readFileSync(raw, 'utf8'))), groupZeroHits(canned()));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('cli report carries scope, totals and honours --all and --crates', () => {
  const dir = mkdtempSync(join(tmpdir(), 'wiring-cov-'));
  try {
    const src = join(dir, 'raw.json');
    writeFileSync(src, JSON.stringify(withGenerics()));
    const run = (...flags) => {
      const out = join(dir, `out-${flags.join('')}.json`);
      const r = spawnSync(process.execPath, ['tools/wiring-cov.mjs', '--from-json', src, '--out', out, ...flags], { encoding: 'utf8' });
      return { r, report: r.status === 0 ? JSON.parse(readFileSync(out, 'utf8')) : null };
    };
    const base = run();
    assert.equal(base.r.status, 0);
    assert.deepEqual(base.report.scope, ROW_CRATES);
    assert.deepEqual(Object.keys(base.report.crates), ['engine-ai', 'engine-render']);
    assert.deepEqual(base.report.totals['engine-render'], { zero: 2, total: 4 });
    assert.match(base.r.stdout, /^wiring-cov: 3 zero-hit fns in 2 crates -> /);
    assert.equal(run('--all').report.scope, 'all');
    assert.deepEqual(Object.keys(run('--all').report.crates), ['engine-ai', 'engine-net', 'engine-render', 'unknown']);
    assert.deepEqual(Object.keys(run('--crates', 'engine-net').report.crates), ['engine-net']);
    const both = run('--all', '--crates', 'engine-net');
    assert.equal(both.r.status, 2);
    assert.match(both.r.stderr, /exclude each other/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('an unreadable or non-json --from-json file exits 2 with a one-line reason', () => {
  const dir = mkdtempSync(join(tmpdir(), 'wiring-cov-'));
  try {
    const out = join(dir, 'w.json');
    const missing = spawnSync(process.execPath, ['tools/wiring-cov.mjs', '--from-json', join(dir, 'nope.json'), '--out', out], { encoding: 'utf8' });
    assert.equal(missing.status, 2);
    assert.match(missing.stderr, /^wiring-cov: cannot read .*nope\.json/);
    const junk = join(dir, 'junk.json');
    writeFileSync(junk, '{not json');
    const bad = spawnSync(process.execPath, ['tools/wiring-cov.mjs', '--from-json', junk, '--out', out], { encoding: 'utf8' });
    assert.equal(bad.status, 2);
    assert.match(bad.stderr, /^wiring-cov: cannot read /);
    assert.doesNotMatch(bad.stderr, /at .*\.mjs:\d+/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

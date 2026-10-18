import assert from 'node:assert/strict';
import test from 'node:test';
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkRise, baselineCounts, baselineOf, baselineTotal, countByCrate, formatReport, helpText, parseAddedLines, scanIslands, acquireRunLock, loadVerdict, saveVerdict, treeKey } from './wiring-scan.mjs';

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

// ---- HEAD + dirty-file verdict cache and the one-run-per-tool lock ----

function gitRepo() {
  const repo = mkdtempSync(join(tmpdir(), 'verdict-repo-'));
  const git = (...args) => execFileSync('git', args, { cwd: repo, stdio: ['ignore', 'ignore', 'ignore'] });
  writeFileSync(join(repo, 'a.rs'), 'pub fn a() {}\n');
  git('init', '-q');
  git('add', 'a.rs');
  git('-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'one');
  return repo;
}

test('verdict key and cache: HEAD plus the dirty-file hash, a dirty edit moves it', () => {
  const repo = gitRepo();
  try {
    const clean = treeKey(repo);
    assert.equal(clean.split('|').length, 3, `HEAD, dirty hash, caller key from ${clean}`);
    assert.equal(treeKey(repo), clean, 'an unchanged tree keeps its key');
    saveVerdict(repo, 'demo', clean, { out: 'PASS demo\n', err: '', code: 0 });
    assert.equal(loadVerdict(repo, 'demo', clean).out, 'PASS demo\n', 'a keyed entry replays');
    writeFileSync(join(repo, 'a.rs'), 'pub fn a() {}\npub fn b() {}\n');
    const dirty = treeKey(repo);
    assert.notEqual(dirty, clean, 'a dirty edit moves the key');
    assert.equal(loadVerdict(repo, 'demo', dirty), null, 'a moved key never replays a stale verdict');
    assert.notEqual(treeKey(repo, 'check'), treeKey(repo, 'report'), 'the caller mode is in the key');
  } finally { cleanup(repo); }
});

test('run lock: a dead holder is taken over now, a live holder is waited on', async () => {
  const repo = gitRepo();
  try {
    const lines = [];
    mkdirSync(join(repo, 'target', 'locks'), { recursive: true });
    writeFileSync(join(repo, 'target', 'locks', 'dead.lock'), '{"pid":999999}');
    acquireRunLock(repo, 'dead', (m) => lines.push(m)).release();
    assert.deepEqual(lines, [], 'a dead holder is taken over, never waited on');
    const url = new URL('./wiring-scan.mjs', import.meta.url).href;
    const child = `import { writeFileSync } from 'node:fs'; import { join } from 'node:path';
import { acquireRunLock } from ${JSON.stringify(url)};
const repo = process.argv[1]; acquireRunLock(repo, 'demo');
writeFileSync(join(repo, 'held'), '1'); setTimeout(() => process.exit(0), 900);`;
    const holder = spawn(process.execPath, ['--input-type=module', '-e', child, repo], { stdio: 'ignore' });
    for (let i = 0; i < 200 && !existsSync(join(repo, 'held')); i += 1) await new Promise((r) => setTimeout(r, 25));
    const start = Date.now();
    acquireRunLock(repo, 'demo', (m) => lines.push(m)).release();
    const waited = Date.now() - start;
    await new Promise((r) => holder.on('exit', r));
    assert.ok(lines.some((l) => l.startsWith('waiting for lock: demo (held by pid ')), `a wait line, got ${JSON.stringify(lines)}`);
    assert.ok(waited >= 200, `the second run waited for the live holder, waited ${waited}ms`);
  } finally { cleanup(repo); }
});

// ---- AD5 diff gate -------------------------------------------------------

const TOOL = join(dirname(fileURLToPath(import.meta.url)), 'wiring-scan.mjs');

test('parseAddedLines reads the new side of each hunk and skips deleted files', () => {
  const diff = [
    'diff --git a/crates/a/src/lib.rs b/crates/a/src/lib.rs',
    '--- a/crates/a/src/lib.rs',
    '+++ b/crates/a/src/lib.rs',
    '@@ -3,0 +4,2 @@ fn x',
    '+pub fn one() {}',
    '+pub fn two() {}',
    '@@ -9 +11 @@',
    '-old',
    '+new',
    '@@ -20,2 +22,0 @@',
    '--- a/crates/gone.rs',
    '+++ /dev/null',
    '@@ -1,3 +0,0 @@',
  ].join('\n');
  const added = parseAddedLines(diff);
  assert.deepEqual([...added.get('crates/a/src/lib.rs')], [4, 5, 11]);
  assert.equal(added.size, 1);
});

// A temp git repo with one committed crate pair, so a test edits the tree
// after the base commit and runs the CLI on it.
function makeRepo() {
  const top = mkdtempSync(join(tmpdir(), 'wiring-diff-'));
  const git = function (...args) { return execFileSync('git', ['-C', top, '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', '-c', 'core.autocrlf=false', ...args], { encoding: 'utf8' }); };
  mkdirSync(join(top, 'crates', 'a', 'src'), { recursive: true });
  mkdirSync(join(top, 'crates', 'b', 'src'), { recursive: true });
  mkdirSync(join(top, 'docs'), { recursive: true });
  writeFileSync(join(top, 'crates', 'a', 'src', 'lib.rs'), 'pub fn old_orphan() {}\n');
  writeFileSync(join(top, 'crates', 'b', 'src', 'lib.rs'), 'fn call_it() {}\n');
  writeFileSync(join(top, 'docs', 'SPRINT.md'), '- [ ] ZZ9 next: the consumer row\n- [x] ZZ1 done: closed\n');
  git('init', '-q');
  git('add', '-A');
  git('commit', '-q', '-m', 'base');
  const run = function (...args) {
    const r = spawnSync(process.execPath, [TOOL, join(top, 'crates'), '--diff', ...args], { encoding: 'utf8' });
    return { code: r.status, out: r.stdout };
  };
  const put = function (crate, file, text) { writeFileSync(join(top, 'crates', crate, 'src', file), text); };
  return { top, run, put };
}

test('diff gate: an uncalled new pub fn FAILs, the same fn called from another file PASSes', () => {
  const repo = makeRepo();
  try {
    repo.put('a', 'lib.rs', 'pub fn old_orphan() {}\npub fn fresh_fn() {}\n');
    const bad = repo.run();
    assert.equal(bad.code, 1, bad.out);
    assert.match(bad.out, /unwired a a\/src\/lib\.rs:2 fn fresh_fn/);
    assert.doesNotMatch(bad.out, /old_orphan/, 'an old orphan is not in the diff');
    repo.put('b', 'lib.rs', 'fn call_it() { fresh_fn(); }\n');
    const good = repo.run();
    assert.equal(good.code, 0, good.out);
    assert.match(good.out, /^PASS wiring-scan --diff/m);
    assert.deepEqual([good.out, repo.run().out], [good.out, good.out], 'same tree, same output');
  } finally {
    cleanup(repo.top);
  }
});

test('diff gate: an open UNLOCKS row exempts new items, a closed or unknown row does not', () => {
  const repo = makeRepo();
  try {
    repo.put('a', 'lib.rs', 'pub fn old_orphan() {}\npub struct Waiting;\n');
    const ok = repo.run('--unlocks=ZZ9');
    assert.equal(ok.code, 0, ok.out);
    assert.match(ok.out, /unlocked a a\/src\/lib\.rs:2 struct Waiting/);
    assert.match(ok.out, /wait for ZZ9/);
    for (const row of ['ZZ1', 'NOPE7']) {
      const refused = repo.run('--unlocks=' + row);
      assert.equal(refused.code, 1, refused.out);
      assert.match(refused.out, /not open on the board: /);
    }
  } finally {
    cleanup(repo.top);
  }
});

test('diff gate: untracked files count and the four unwired shapes of BLOCKERS all FAIL', () => {
  const repo = makeRepo();
  try {
    // px36: two helpers that only call each other. rt19: a helper only a test calls.
    // fix5: a type only its own file builds. fix3c: a test-only island of fns.
    repo.put('a', 'servo.rs', [
      'pub fn capture_bias() { strike_gait_biased(); }',
      'pub fn strike_gait_biased() { capture_bias(); }',
      'pub fn reflect_screen_for_forward() {}',
      'pub struct Runner;',
      'fn build() -> Runner { Runner }',
      'pub fn move_row() {}',
      'pub fn shake_for_trauma() { move_row(); }',
      '#[cfg(test)]',
      'mod tests { fn t() { super::reflect_screen_for_forward(); super::shake_for_trauma(); } }',
      '',
    ].join('\n'));
    const r = repo.run();
    assert.equal(r.code, 1, r.out);
    for (const name of ['capture_bias', 'strike_gait_biased', 'reflect_screen_for_forward', 'Runner', 'move_row', 'shake_for_trauma']) {
      assert.match(r.out, new RegExp('unwired a a/src/servo\\.rs:\\d+ (fn|struct) ' + name + '\\n'), name);
    }
  } finally {
    cleanup(repo.top);
  }
});

test('--orphans is the default report and --help lists the diff flags', () => {
  const root = makeFixture();
  try {
    const plain = execFileSync(process.execPath, [TOOL, root], { encoding: 'utf8' });
    const orphans = execFileSync(process.execPath, [TOOL, root, '--orphans'], { encoding: 'utf8' });
    assert.equal(orphans, plain);
    assert.match(plain, /^islands: 2$/m);
    for (const flag of ['--diff', '--unlocks', '--orphans']) assert.match(helpText(), new RegExp(flag));
  } finally {
    cleanup(root);
  }
});

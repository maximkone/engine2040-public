import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import {
  REWRITE_LINES,
  checkByPath,
  checkGenre,
  checkProof,
  checkRewrites,
  genreScope,
  parseGenreLedger,
  parseNameStatus,
  parseNumstat,
  parsePatch,
  proofFinding,
  ratchetVerdict,
  review,
  rewriteSignature,
  rustfmtVerdict,
} from './review-gate.mjs';

// One test per rule. Every rule takes plain data, so no test needs git, a
// working tree or rustfmt: the ratchet and rustfmt legs are stubbed rows.

test('R1: a merge or an empty message fails; dirty overlap only warns', () => {
  const merge = checkByPath({ isMerge: true, message: 'merge branch', paths: ['a.rs'] });
  assert.equal(merge.ok, false);
  assert.match(merge.line, /FAIL R1 commit by path: the commit is a merge/);

  const empty = checkByPath({ isMerge: false, message: '  \n', paths: ['a.rs'] });
  assert.equal(empty.ok, false);
  assert.match(empty.line, /the message is empty/);

  const overlap = checkByPath({
    isMerge: false,
    message: 'tools: add a gate',
    paths: ['tools/a.mjs', 'tools/b.mjs'],
    dirtyPaths: ['tools/a.mjs'],
  });
  assert.equal(overlap.ok, true);
  assert.deepEqual(overlap.warnings, [
    'WARN R1 dirty overlap: tools/a.mjs is also uncommitted in the working tree',
  ]);
});

test('R2: a rewrite-shaped diff fails and names the file', () => {
  assert.equal(REWRITE_LINES, 200);
  const path = 'crates/game/src/windowed.rs';
  const shaped = { path, added: 2400, deleted: 2350, before: 3600, after: 3650 };
  const out = checkRewrites([shaped]);
  assert.equal(out.ok, false);
  assert.match(out.line, /FAIL R2 no whole-file rewrite/);
  assert.match(out.line, new RegExp(path.replace(/\./g, '\\.')));
  assert.match(out.line, /3600 -> 3650/);

  assert.equal(rewriteSignature({ path, added: 2400, deleted: 2350, before: 100, after: 2400 }), null);
  assert.equal(rewriteSignature({ path, added: 200, deleted: 900, before: 900, after: 900 }), null);
  assert.equal(checkRewrites([{ path, added: 30, deleted: 12, before: 500, after: 518 }]).ok, true);
});

test('R3: an added genre word fails and names it; ledger, comments, strings and tests pass', () => {
  const ledger = new Set(['input/src/fighter.rs']);
  const files = [
    {
      path: 'crates/combat/src/swing.rs',
      key: 'combat/src/swing.rs',
      before: 'pub fn swing() {}\n',
      after: 'pub type FighterSnap = u8;\npub fn swing() {}\n',
      added: [{ line: 1, text: 'pub type FighterSnap = u8;' }],
    },
  ];
  const out = checkGenre({ files, ledger });
  assert.equal(out.ok, false);
  assert.match(out.line, /FAIL R3 no new genre name/);
  assert.match(out.line, /crates\/combat\/src\/swing\.rs:1 adds FighterSnap/);

  assert.equal(genreScope('crates/combat/src/swing.rs'), true);
  assert.equal(genreScope('crates/combat/tests/it/swing.rs'), false);
  assert.equal(genreScope('crates/game/src/lib.rs'), false);
  assert.equal(genreScope('crates/arena/src/lib.rs'), false);

  const listed = [{ ...files[0], path: 'crates/input/src/fighter.rs', key: 'input/src/fighter.rs' }];
  assert.equal(checkGenre({ files: listed, ledger }).ok, true);

  const quiet = [
    {
      path: 'crates/ui/src/menus.rs',
      key: 'ui/src/menus.rs',
      before: '',
      after: '// Fighter layout\nconst LABEL: &str = "Gladiator";\n#[cfg(test)]\nmod t { fn f() -> FighterSnap { todo!() } }\n',
      added: [],
    },
  ];
  assert.equal(checkGenre({ files: quiet, ledger }).ok, true);

  const grown = [{ ...files[0], before: 'pub struct FighterSnap;\n', after: 'pub struct FighterSnap;\npub fn hit() {}\n' }];
  assert.equal(checkGenre({ files: grown, ledger }).ok, true);
});

test('R4: the ratchet leg reads its RESULT line, skips with no .rs', () => {
  const skipped = ratchetVerdict({ touchedRs: false });
  assert.equal(skipped.ok, true);
  assert.match(skipped.line, /PASS R4 ratchet: skipped, no \.rs file in the commit/);

  const pass = ratchetVerdict({
    touchedRs: true,
    output: 'crate  kloc  score\nrender  53.1  88\nRESULT PASS: score 90 (A), 0 ratchet regressions over 1 crate(s)\n',
  });
  assert.equal(pass.ok, true);
  assert.match(pass.line, /RESULT PASS: score 90/);

  const fail = ratchetVerdict({ touchedRs: true, output: 'RESULT FAIL: 3 ratchet regression(s)\n' });
  assert.equal(fail.ok, false);
  assert.match(fail.line, /FAIL R4 ratchet \(working tree\): RESULT FAIL: 3/);

  assert.equal(ratchetVerdict({ touchedRs: true, output: 'no summary here' }).ok, false);
});

test('R5: rustfmt rows are data; a bad row fails, missing rustfmt skips', () => {
  const bad = rustfmtVerdict({
    rows: [
      { path: 'crates/web/src/shell.rs', status: 1 },
      { path: 'crates/web/src/limits.rs', status: 1 },
    ],
  });
  assert.equal(bad.ok, false);
  assert.match(bad.line, /FAIL R5 rustfmt: crates\/web\/src\/shell\.rs is not formatted/);
  assert.match(bad.line, /1 more/);

  assert.equal(rustfmtVerdict({ rows: [{ path: 'crates/web/src/shell.rs', status: 0 }] }).ok, true);
  assert.match(rustfmtVerdict({ available: false }).line, /skipped, rustfmt is not on PATH/);
  assert.match(rustfmtVerdict({ rows: [] }).line, /skipped, no \.rs file/);
});

test('R6: a message without a proof line fails; Proof: or a command plus verdict passes', () => {
  const missing = checkProof({ message: 'tools: add the review gate\n\nIt prints one line per rule.\n' });
  assert.equal(missing.ok, false);
  assert.match(missing.line, /FAIL R6 proof line: no proof line/);

  assert.equal(proofFinding('tools: x\n\nProof: cargo test -p engine-core --lib (12 passed)\n'), null);
  assert.equal(proofFinding('Ran `node --test tools/review-gate.test.mjs` (9 passed).'), null);
  assert.equal(proofFinding('Ran `node --test tools/x.test.mjs`; it passed.'), null);
  assert.notEqual(proofFinding('Ran `node --test tools/x.test.mjs`.'), null);
});

test('a clean commit passes every rule', () => {
  const out = review({
    isMerge: false,
    message: 'tools: add the gate\n\nProof: `node --test tools/review-gate.test.mjs` (9 passed).\n',
    paths: ['tools/review-gate.mjs'],
    dirtyPaths: [],
    rewrites: [],
    genreFiles: [],
    ledger: new Set(['input/src/fighter.rs']),
    touchedRs: false,
    ratchetOutput: '',
    rustfmt: { available: false },
  });
  assert.equal(out.ok, true);
  assert.equal(out.lines.length, 6);
  assert.equal(out.broken, 0);
  assert.ok(out.lines.every((line) => line.startsWith('PASS ')));
});

test('the git parsers read name-status, numstat and added patch lines', () => {
  const rows = parseNameStatus('M\ttools/a.mjs\nR100\told/a.rs\tcrates/a/src/b.rs\n');
  assert.deepEqual(rows, [
    { status: 'M', paths: ['tools/a.mjs'] },
    { status: 'R', paths: ['old/a.rs', 'crates/a/src/b.rs'] },
  ]);

  const stats = parseNumstat('64\t7\tcrates/destruction/src/bonded.rs\n-\t-\tassets/x.bin\n');
  assert.deepEqual(stats.get('crates/destruction/src/bonded.rs'), { added: 64, deleted: 7 });
  assert.deepEqual(stats.get('assets/x.bin'), { added: 0, deleted: 0 });

  const patch = parsePatch(
    [
      'diff --git a/crates/a/src/b.rs b/crates/a/src/b.rs',
      'index 1111111..2222222 100644',
      '--- a/crates/a/src/b.rs',
      '+++ b/crates/a/src/b.rs',
      '@@ -10,2 +10,3 @@ pub fn f() {',
      ' context',
      '+pub struct FighterSnap;',
      '-old line',
      ' context',
      '',
    ].join('\n'),
  );
  assert.deepEqual(patch.get('crates/a/src/b.rs'), [{ line: 11, text: 'pub struct FighterSnap;' }]);
});

test('the GE2 ledger comes from workspace-check, one list', () => {
  const ledger = parseGenreLedger(readFileSync(new URL('./workspace-check.mjs', import.meta.url), 'utf8'));
  assert.ok(ledger.size > 50, `ledger has ${ledger.size} entries`);
  assert.ok(ledger.has('input/src/fighter.rs'));
  assert.throws(() => parseGenreLedger('const genreLedger = new Set([]);'), /parsed only 0 entries/);
  assert.throws(() => parseGenreLedger('nothing'), /cannot read the GE2 genre ledger/);
});

// golden-matrix.test.mjs - unit tests for golden-matrix.mjs, no cargo.
import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { causeNote, compareRagdoll, compareRows, compareTapes, formatMatrix, formatRagdoll, formatRows, parseArgs, parseRagdoll, parseRows, parseTape, run } from './golden-matrix.mjs';
function makeTape(opts) {
  const o = opts || {};
  const seed = o.seed !== undefined ? BigInt(o.seed) : 19n;
  const ticks = o.ticks !== undefined ? o.ticks : 3;
  const inputs = o.inputs !== undefined ? o.inputs : [];
  const legacyOpt = o.legacy !== undefined ? o.legacy : o.legacyFn;
  const stateOpt = o.state !== undefined ? o.state : o.stateFn;
  const legacyFn = typeof legacyOpt === 'function'
    ? legacyOpt
    : (legacyOpt !== undefined && legacyOpt !== null
      ? function () { return BigInt(legacyOpt); }
      : function (t) { return BigInt(1000 + t); });
  const stateFn = typeof stateOpt === 'function'
    ? stateOpt
    : (stateOpt !== undefined && stateOpt !== null
      ? function () { return BigInt(stateOpt); }
      : function (t) { return BigInt(2000 + t * 13); });
  const count = inputs.length;
  const buf = Buffer.alloc(28 + count * 16 + ticks * 16);
  buf.write('E2040RP\0', 0, 'ascii');
  buf.writeUInt16LE(1, 8);
  buf.writeUInt16LE(1, 10);
  buf.writeBigUInt64LE(seed, 12);
  buf.writeUInt32LE(ticks, 20);
  buf.writeUInt32LE(count, 24);
  for (let i = 0; i < count; i += 1) {
    const inp = inputs[i];
    const off = 28 + i * 16;
    buf.writeUInt32LE(inp.tick >>> 0, off);
    buf.writeUInt32LE(inp.body >>> 0, off + 4);
    buf.writeFloatLE(inp.vx, off + 8);
    buf.writeFloatLE(inp.vy, off + 12);
  }
  const base = 28 + count * 16;
  for (let t = 0; t < ticks; t += 1) {
    buf.writeBigUInt64LE(BigInt(legacyFn(t)), base + t * 16);
    buf.writeBigUInt64LE(BigInt(stateFn(t)), base + t * 16 + 8);
  }
  return buf;
}
function entriesFor(buffers, names) {
  const entries = buffers.map(function (b, i) {
    return { name: names[i], tape: parseTape(b) };
  });
  const results = entries.slice(1).map(function (e) { return compareTapes(entries[0].tape, e.tape); });
  return { entries: entries, results: results };
}
test('parseTape round-trips seed, ticks, inputs and checkpoints', function () {
  const inputs = [{ tick: 2, body: 9, vx: -5.25, vy: 8.0 }, { tick: 4, body: 1, vx: 1.5, vy: -2.0 }];
  const buf = makeTape({ seed: 19n, ticks: 5, inputs: inputs });
  const tape = parseTape(buf);
  assert.equal(tape.format, 1);
  assert.equal(tape.fixture, 1);
  assert.equal(tape.seed, 19n);
  assert.equal(typeof tape.seed, 'bigint');
  assert.equal(tape.ticks, 5);
  assert.equal(tape.inputs.length, 2);
  assert.equal(tape.inputs[0].length, 32);
  assert.equal(tape.checkpoints.length, 5);
  assert.equal(tape.checkpoints[0].legacy, 1000n);
  assert.equal(tape.checkpoints[4].state, BigInt(2000 + 4 * 13));
  const raw = Buffer.from(tape.inputs[0], 'hex');
  assert.equal(raw.readUInt32LE(0), 2);
  assert.equal(raw.readUInt32LE(4), 9);
  assert.equal(raw.readFloatLE(8), -5.25);
});
test('parseTape rejects bad tapes with not a tape', function () {
  const good = makeTape({ seed: 19n, ticks: 2, inputs: [{ tick: 0, body: 1, vx: 1, vy: 2 }] });
  assert.throws(function () { parseTape(Buffer.alloc(0)); }, function (err) { return err instanceof Error && err.message.indexOf('not a tape') === 0; });
  assert.throws(function () { parseTape(Buffer.alloc(27)); }, function (err) { return err instanceof Error && err.message.indexOf('not a tape') === 0; });
  assert.throws(function () { parseTape(good.subarray(0, good.length - 1)); }, function (err) { return err instanceof Error && err.message.indexOf('not a tape') === 0; });
  const badMagic = Buffer.from(good);
  badMagic[0] = badMagic[0] ^ 255;
  assert.throws(function () { parseTape(badMagic); }, function (err) { return err instanceof Error && err.message.indexOf('not a tape') === 0; });
  const badFormat = Buffer.from(good);
  badFormat.writeUInt16LE(2, 8);
  assert.throws(function () { parseTape(badFormat); }, function (err) { return err instanceof Error && err.message.indexOf('not a tape') === 0; });
  const badFixture = Buffer.from(good);
  badFixture.writeUInt16LE(2, 10);
  assert.throws(function () { parseTape(badFixture); }, function (err) { return err instanceof Error && err.message.indexOf('not a tape') === 0; });
  const tooLong = Buffer.concat([good, Buffer.alloc(1)]);
  assert.throws(function () { parseTape(tooLong); }, function (err) { return err instanceof Error && err.message.indexOf('not a tape') === 0; });
});
test('two equal tapes match and end RESULT PASS', function () {
  const a = makeTape({ seed: 19n, ticks: 4, inputs: [{ tick: 1, body: 2, vx: 0.5, vy: -1.5 }] });
  const b = Buffer.from(a);
  const r = compareTapes(parseTape(a), parseTape(b));
  assert.equal(r.equal, true);
  assert.equal(r.comparable, true);
  const g = entriesFor([a, b], ['a.e2040', 'b.e2040']);
  const out = formatMatrix(g.entries, g.results);
  assert.match(out, /MATCH b\.e2040 vs a\.e2040/);
  assert.ok(out.endsWith('RESULT PASS'));
});
test('state lane diverge from tick 57 names tick and fails', function () {
  const ticks = 120;
  const baseFn = function (t) { return BigInt(5000 + t); };
  const lateFn = function (t) { return t < 57 ? BigInt(5000 + t) : BigInt(9000 + t); };
  const a = makeTape({ seed: 19n, ticks: ticks, state: baseFn });
  const b = makeTape({ seed: 19n, ticks: ticks, state: lateFn });
  const r = compareTapes(parseTape(a), parseTape(b));
  assert.equal(r.equal, false);
  assert.equal(r.comparable, true);
  assert.equal(r.firstDivergentTick, 57);
  assert.equal(r.lane, 'state');
  const g = entriesFor([a, b], ['a.e2040', 'b.e2040']);
  const out = formatMatrix(g.entries, g.results);
  assert.match(out, /first bad tick 57 \(state lane\)/);
  assert.ok(out.endsWith('RESULT FAIL'));
});
test('legacy lane diverge at tick 0 cites start state', function () {
  const a = makeTape({ seed: 19n, ticks: 3 });
  const b = makeTape({ seed: 19n, ticks: 3, legacy: function (t) { return t === 0 ? 9999n : BigInt(1000 + t); } });
  const r = compareTapes(parseTape(a), parseTape(b));
  assert.equal(r.equal, false);
  assert.equal(r.firstDivergentTick, 0);
  assert.equal(r.lane, 'legacy');
  assert.match(r.note, /start state/);
  assert.match(causeNote(0), /start state/);
});
test('different seed or inputs are SKIP with no tick named', function () {
  const a = makeTape({ seed: 19n, ticks: 3, inputs: [{ tick: 0, body: 1, vx: 1, vy: 1 }] });
  const otherSeed = makeTape({ seed: 20n, ticks: 3, inputs: [{ tick: 0, body: 1, vx: 1, vy: 1 }] });
  const rs = compareTapes(parseTape(a), parseTape(otherSeed));
  assert.equal(rs.comparable, false);
  assert.equal(rs.firstDivergentTick, null);
  const gs = entriesFor([a, otherSeed], ['a.e2040', 'b.e2040']);
  const outs = formatMatrix(gs.entries, gs.results);
  assert.match(outs, /SKIP b\.e2040:/);
  assert.doesNotMatch(outs, /first bad tick/);
  const otherInputs = makeTape({ seed: 19n, ticks: 3, inputs: [{ tick: 0, body: 2, vx: 1, vy: 1 }] });
  const ri = compareTapes(parseTape(a), parseTape(otherInputs));
  assert.equal(ri.comparable, false);
  assert.equal(ri.firstDivergentTick, null);
  const gi = entriesFor([a, otherInputs], ['a.e2040', 'c.e2040']);
  const outi = formatMatrix(gi.entries, gi.results);
  assert.match(outi, /SKIP c\.e2040:/);
  assert.doesNotMatch(outi, /first bad tick/);
});
test('same inputs give the same output text twice', function () {
  const a = makeTape({ seed: 19n, ticks: 3 });
  const b = Buffer.from(a);
  const g = entriesFor([a, b], ['a.e2040', 'b.e2040']);
  const first = formatMatrix(g.entries, g.results);
  const second = formatMatrix(g.entries, g.results);
  assert.equal(first, second);
});
test('arch digest diverge names tick plus cause class', function () {
  const ticks = 12;
  const baseFn = function (t) { return BigInt(5000 + t); };
  const a = makeTape({ seed: 19n, ticks: ticks, state: baseFn });
  const fma = makeTape({ seed: 19n, ticks: ticks, state: function (t) { return t < 5 ? BigInt(5000 + t) : BigInt(5000 + t + 1); } });
  const rf = compareTapes(parseTape(a), parseTape(fma));
  assert.equal(rf.firstDivergentTick, 5);
  assert.equal(rf.cause, 'fma');
  assert.match(rf.note, /cause=fma/);
  const g = entriesFor([a, fma], ['linux.e2040', 'windows.e2040']);
  const out = formatMatrix(g.entries, g.results);
  assert.match(out, /first bad tick 5/);
  assert.match(out, /cause=fma/);
  const libm = makeTape({ seed: 19n, ticks: ticks, state: function (t) { return t < 5 ? BigInt(5000 + t) : BigInt(5000 + t + 300); } });
  assert.equal(compareTapes(parseTape(a), parseTape(libm)).cause, 'libm');
  const tie = makeTape({ seed: 19n, ticks: ticks, state: function (t) { return t < 5 ? BigInt(5000 + t) : BigInt(5000 + t + 2654435769); } });
  assert.equal(compareTapes(parseTape(a), parseTape(tie)).cause, 'sort-tie');
});
function hex(v) {
  return BigInt(v).toString(16).padStart(16, '0');
}
// The two lines `matrix_line` prints: digest row, then block row.
function makeRows(opts) {
  const o = opts || {};
  const host = o.host !== undefined ? o.host : 'linux-x86_64';
  const every = o.every !== undefined ? o.every : 500;
  const blocks = o.blocks !== undefined ? o.blocks : [11n, 22n, 33n, 44n];
  const golden = o.golden !== undefined ? o.golden : 0xd57b5c8ae3096614n;
  const corpus = o.corpus !== undefined ? o.corpus : 0xb37fc6d66df2d417n;
  return host + ' golden=' + hex(golden) + ' corpus=' + hex(corpus) + ' blocks=' + blocks.length + '\n'
    + host + ' every=' + every + ' cut_checkpoints=' + blocks.map(hex).join(',') + '\n';
}
function rowEntries(texts, names) {
  const entries = texts.map(function (t, i) { return { name: names[i], row: parseRows(t) }; });
  const results = entries.slice(1).map(function (e) { return compareRows(entries[0].row, e.row); });
  return { entries: entries, results: results };
}
test('parseRows reads both lines, a BOM and CRLF', function () {
  const row = parseRows(makeRows({ host: 'windows-x86_64', blocks: [1n, 2n, 3n] }));
  assert.equal(row.host, 'windows-x86_64');
  assert.equal(row.golden, 0xd57b5c8ae3096614n);
  assert.equal(row.corpus, 0xb37fc6d66df2d417n);
  assert.equal(row.every, 500);
  assert.deepEqual(row.checkpoints, [1n, 2n, 3n]);
  const crlf = '﻿' + makeRows({ blocks: [7n] }).replace(/\n/g, '\r\n');
  assert.deepEqual(parseRows(crlf).checkpoints, [7n]);
});
test('parseRows refuses a row that is not what matrix_line prints', function () {
  const good = makeRows();
  const bad = function (text) {
    assert.throws(function () { parseRows(text); }, function (err) { return err instanceof Error && err.message.indexOf('not a matrix row') === 0; });
  };
  bad('');
  bad(good.split('\n')[0] + '\n');
  bad(good.replace('golden=d57b5c8ae3096614', 'golden=D57B5C8AE3096614'));
  bad(good.replace('golden=d57b5c8ae3096614', 'golden=d57b5c8a'));
  bad(good.replace('blocks=4', 'blocks=5'));
  bad(good.replace('every=500', 'every=0'));
  bad(good.replace('every=500', 'every=x'));
  bad(good.replace('linux-x86_64 every', 'linux-aarch64 every'));
  bad(good + good);
});
test('equal rows match and the rows verdict is PASS', function () {
  const g = rowEntries([makeRows(), makeRows(), makeRows({ host: 'windows-x86_64' })], ['a.txt', 'b.txt', 'c.txt']);
  assert.ok(g.results.every(function (r) { return r.equal && r.comparable; }));
  const out = formatRows(g.entries, g.results);
  assert.match(out, /MATCH b\.txt vs a\.txt/);
  assert.match(out, /MATCH c\.txt vs a\.txt/);
  assert.ok(out.endsWith('CORPUS RESULT PASS'));
});
test('a moved corpus block names the block, its cases and a cause', function () {
  const moved = makeRows({ host: 'linux-aarch64', blocks: [11n, 22n, 34n, 45n] });
  const r = compareRows(parseRows(makeRows()), parseRows(moved));
  assert.equal(r.equal, false);
  assert.equal(r.firstDivergentBlock, 2);
  assert.equal(r.firstCase, 1000);
  assert.equal(r.cause, 'fma');
  assert.match(r.note, /first bad corpus block 2 \(cases 1000\.\.=1499\) \[cause=fma\]/);
  const far = makeRows({ blocks: [11n, 22n, 33n ^ 0x8000000000000000n, 44n] });
  assert.equal(compareRows(parseRows(makeRows()), parseRows(far)).cause, 'sort-tie');
  const g = rowEntries([makeRows(), moved], ['linux.txt', 'arm.txt']);
  const out = formatRows(g.entries, g.results);
  assert.match(out, /DIVERGE arm\.txt vs linux\.txt \(linux-aarch64 vs linux-x86_64\): first bad corpus block 2/);
  assert.ok(out.endsWith('CORPUS RESULT FAIL'));
});
test('equal blocks with a moved digest still fail and name the digest', function () {
  const base = parseRows(makeRows());
  const corpus = compareRows(base, parseRows(makeRows({ corpus: 0xb37fc6d66df2d418n })));
  assert.equal(corpus.equal, false);
  assert.equal(corpus.firstDivergentBlock, null);
  assert.match(corpus.note, /corpus digest differs: b37fc6d66df2d417 vs b37fc6d66df2d418/);
  const golden = compareRows(base, parseRows(makeRows({ golden: 1n })));
  assert.equal(golden.equal, false);
  assert.match(golden.note, /golden digest differs: d57b5c8ae3096614 vs 0000000000000001/);
});
test('rows with another block count or width are SKIP, not a block', function () {
  const base = parseRows(makeRows());
  const fewer = compareRows(base, parseRows(makeRows({ blocks: [11n, 22n] })));
  assert.equal(fewer.comparable, false);
  assert.equal(fewer.equal, false);
  assert.match(fewer.note, /corpus shape differs/);
  const wider = compareRows(base, parseRows(makeRows({ every: 250 })));
  assert.equal(wider.comparable, false);
  const g = rowEntries([makeRows(), makeRows({ blocks: [11n] })], ['a.txt', 'b.txt']);
  const out = formatRows(g.entries, g.results);
  assert.match(out, /SKIP b\.txt:/);
  assert.doesNotMatch(out, /first bad corpus block/);
  assert.ok(out.endsWith('CORPUS RESULT FAIL'));
});
test('parseArgs splits tapes from rows and refuses one-file groups', function () {
  assert.deepEqual(parseArgs(['a', 'b']), { tapes: ['a', 'b'], rows: [], ragdoll: [], error: null });
  assert.deepEqual(parseArgs(['a', 'b', '--rows', 'x', 'y', 'z']), { tapes: ['a', 'b'], rows: ['x', 'y', 'z'], ragdoll: [], error: null });
  assert.deepEqual(parseArgs(['--rows', 'x', 'y']), { tapes: [], rows: ['x', 'y'], ragdoll: [], error: null });
  assert.match(parseArgs([]).error, /^usage:/);
  assert.match(parseArgs(['a']).error, /^usage:/);
  assert.match(parseArgs(['a', 'b', '--rows', 'x']).error, /^usage:/);
  assert.match(parseArgs(['a', 'b', '--rows', 'x', '--rows', 'y']).error, /only one --rows group/);
  assert.match(parseArgs(['a', 'b', '--frob']).error, /unknown flag --frob/);
});
test('run compares tapes and rows over files and sets the exit code', function () {
  const dir = mkdtempSync(join(tmpdir(), 'golden-matrix-'));
  try {
    const put = function (name, data) {
      const p = join(dir, name);
      writeFileSync(p, data);
      return p;
    };
    const tapeA = put('a.e2040', makeTape({ seed: 2040n, ticks: 40 }));
    const tapeB = put('b.e2040', makeTape({ seed: 2040n, ticks: 40 }));
    const tapeC = put('c.e2040', makeTape({ seed: 2040n, ticks: 40, state: function (t) { return t < 17 ? BigInt(2000 + t * 13) : BigInt(7000 + t); } }));
    const rowA = put('a.txt', makeRows());
    const rowB = put('b.txt', makeRows({ host: 'windows-x86_64' }));
    const rowC = put('c.txt', makeRows({ host: 'linux-aarch64', blocks: [11n, 22n, 33n, 99n] }));
    const same = run([tapeA, tapeB, '--rows', rowA, rowB]);
    assert.equal(same.code, 0, same.err + same.out);
    assert.match(same.out, /RESULT PASS/);
    assert.match(same.out, /CORPUS RESULT PASS/);
    const tapeOnly = run([tapeA, tapeB]);
    assert.equal(tapeOnly.code, 0);
    assert.doesNotMatch(tapeOnly.out, /CORPUS/);
    const badTape = run([tapeA, tapeB, tapeC, '--rows', rowA, rowB]);
    assert.equal(badTape.code, 1);
    assert.match(badTape.out, /first bad tick 17 \(state lane\)/);
    assert.match(badTape.out, /CORPUS RESULT PASS/);
    const badRows = run([tapeA, tapeB, '--rows', rowA, rowB, rowC]);
    assert.equal(badRows.code, 1);
    assert.match(badRows.out, /first bad corpus block 3 \(cases 1500\.\.=1999\)/);
    assert.match(badRows.out, /\nRESULT PASS\n/);
    const missing = run([tapeA, join(dir, 'gone.e2040')]);
    assert.equal(missing.code, 2);
    assert.match(missing.err, /not a tape: cannot read .*gone\.e2040/);
    const notRow = run(['--rows', rowA, tapeA]);
    assert.equal(notRow.code, 2);
    assert.match(notRow.err, /not a matrix row/);
    assert.equal(run([tapeA]).code, 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
// The text `ragdoll_line` prints: a summary line, then one line of body digests per tick.
// `digest(tick, body)` gives each body digest; a tweak function moves chosen ones.
function makeRagdoll(opts) {
  const o = opts || {};
  const host = o.host !== undefined ? o.host : 'linux-x86_64';
  const ticks = o.ticks !== undefined ? o.ticks : 6;
  const bodies = o.bodies !== undefined ? o.bodies : 4;
  const digest = o.digest !== undefined ? o.digest : function (t, b) { return BigInt(1000 + t * 100 + b); };
  const contacts = o.contacts !== undefined ? o.contacts : 12;
  const finalState = o.finalState !== undefined ? o.finalState : 0xabcdef0123456789n;
  const overall = o.overall !== undefined ? o.overall : 0x1122334455667788n;
  const lines = [host + ' ragdoll=' + hex(overall) + ' ticks=' + ticks + ' bodies=' + bodies + ' contacts=' + contacts + ' final_state=' + hex(finalState)];
  for (let t = 0; t < ticks; t += 1) {
    const list = [];
    for (let b = 0; b < bodies; b += 1) {
      list.push(hex(digest(t, b)));
    }
    lines.push(host + ' tick=' + t + ' bodies=' + list.join(','));
  }
  return lines.join('\n') + '\n';
}
function ragdollEntries(texts, names) {
  const entries = texts.map(function (t, i) { return { name: names[i], row: parseRagdoll(t) }; });
  const results = entries.slice(1).map(function (e) { return compareRagdoll(entries[0].row, e.row); });
  return { entries: entries, results: results };
}
test('parseRagdoll reads the summary, every tick line, a BOM and CRLF', function () {
  const row = parseRagdoll(makeRagdoll({ host: 'wasi-wasm32', ticks: 3, bodies: 2 }));
  assert.equal(row.host, 'wasi-wasm32');
  assert.equal(row.digest, 0x1122334455667788n);
  assert.equal(row.ticks, 3);
  assert.equal(row.bodies, 2);
  assert.equal(row.contacts, 12n);
  assert.equal(row.finalState, 0xabcdef0123456789n);
  assert.equal(row.perTick.length, 3);
  assert.deepEqual(row.perTick[2], [1200n, 1201n]);
  const crlf = '﻿' + makeRagdoll({ ticks: 2, bodies: 1 }).replace(/\n/g, '\r\n');
  assert.deepEqual(parseRagdoll(crlf).perTick, [[1000n], [1100n]]);
});
test('parseRagdoll refuses a row that is not what ragdoll_line prints', function () {
  const good = makeRagdoll();
  const bad = function (text) {
    assert.throws(function () { parseRagdoll(text); }, function (err) { return err instanceof Error && err.message.indexOf('not a ragdoll row') === 0; });
  };
  bad('');
  bad(good.split('\n')[0] + '\n');
  bad(good.replace('ragdoll=1122334455667788', 'ragdoll=1122334455'));
  bad(good.replace('ticks=6', 'ticks=7'));
  bad(good.replace('ticks=6', 'ticks=0'));
  bad(good.replace('bodies=4 contacts', 'bodies=5 contacts'));
  bad(good.replace('tick=2 ', 'tick=3 '));
  bad(good.replace('linux-x86_64 tick=1', 'linux-aarch64 tick=1'));
  bad(good.replace(hex(1100n), hex(1100n).toUpperCase().replace('0', 'G')));
  bad(good + good);
});
test('equal ragdoll rows match and the ragdoll verdict is PASS', function () {
  const g = ragdollEntries([makeRagdoll(), makeRagdoll(), makeRagdoll({ host: 'wasi-wasm32' })], ['a.txt', 'b.txt', 'c.txt']);
  assert.ok(g.results.every(function (r) { return r.equal && r.comparable; }));
  const out = formatRagdoll(g.entries, g.results);
  assert.match(out, /MATCH b\.txt vs a\.txt/);
  assert.match(out, /MATCH c\.txt vs a\.txt/);
  assert.match(out, /c\.txt host=wasi-wasm32 ragdoll=1122334455667788 ticks=6 bodies=4 contacts=12 final_state=abcdef0123456789/);
  assert.ok(out.endsWith('RAGDOLL RESULT PASS'));
});
test('a moved body names the first tick and the first body that differ', function () {
  // From tick 3 on, body 2 differs; from tick 4 on, body 0 differs as well.
  const moved = makeRagdoll({
    host: 'wasi-wasm32',
    digest: function (t, b) {
      if (b === 2 && t >= 3) { return BigInt(9000 + t); }
      if (b === 0 && t >= 4) { return BigInt(8000 + t); }
      return BigInt(1000 + t * 100 + b);
    }
  });
  const r = compareRagdoll(parseRagdoll(makeRagdoll()), parseRagdoll(moved));
  assert.equal(r.equal, false);
  assert.equal(r.comparable, true);
  assert.equal(r.firstTick, 3);
  assert.equal(r.firstBody, 2);
  assert.match(r.note, /first bad tick 3 body 2 \(skeleton order; 1 of 4 bodies differ on that tick\)/);
  const same = compareRagdoll(parseRagdoll(makeRagdoll()), parseRagdoll(makeRagdoll()));
  assert.equal(same.equal, true);
  const g = ragdollEntries([makeRagdoll(), moved], ['linux.txt', 'wasm.txt']);
  const out = formatRagdoll(g.entries, g.results);
  assert.match(out, /DIVERGE wasm\.txt vs linux\.txt \(wasi-wasm32 vs linux-x86_64\): first bad tick 3 body 2/);
  assert.ok(out.endsWith('RAGDOLL RESULT FAIL'));
  // Two bodies moved on the same first tick: the lowest index is named and both are counted.
  const two = makeRagdoll({ digest: function (t, b) { return (t >= 1 && (b === 1 || b === 3)) ? BigInt(7000 + t) : BigInt(1000 + t * 100 + b); } });
  const rt = compareRagdoll(parseRagdoll(makeRagdoll()), parseRagdoll(two));
  assert.equal(rt.firstTick, 1);
  assert.equal(rt.firstBody, 1);
  assert.match(rt.note, /2 of 4 bodies differ/);
});
test('equal transforms with other contacts or a moved state still fail and say so', function () {
  const base = parseRagdoll(makeRagdoll());
  const contacts = compareRagdoll(base, parseRagdoll(makeRagdoll({ contacts: 13 })));
  assert.equal(contacts.equal, false);
  assert.equal(contacts.firstTick, null);
  assert.match(contacts.note, /every transform matches but contacts differ: 12 vs 13/);
  const state = compareRagdoll(base, parseRagdoll(makeRagdoll({ finalState: 0xabcdef0123456788n })));
  assert.equal(state.equal, false);
  assert.match(state.note, /final state hash differs: abcdef0123456789 vs abcdef0123456788/);
  const overall = compareRagdoll(base, parseRagdoll(makeRagdoll({ overall: 1n })));
  assert.equal(overall.equal, false);
  assert.match(overall.note, /ragdoll digest differs: 1122334455667788 vs 0000000000000001/);
});
test('ragdoll rows with another tick or body count are SKIP, not a tick', function () {
  const base = parseRagdoll(makeRagdoll());
  const shorter = compareRagdoll(base, parseRagdoll(makeRagdoll({ ticks: 5 })));
  assert.equal(shorter.comparable, false);
  assert.equal(shorter.equal, false);
  assert.match(shorter.note, /ragdoll shape differs/);
  const wider = compareRagdoll(base, parseRagdoll(makeRagdoll({ bodies: 5 })));
  assert.equal(wider.comparable, false);
  const g = ragdollEntries([makeRagdoll(), makeRagdoll({ bodies: 5 })], ['a.txt', 'b.txt']);
  const out = formatRagdoll(g.entries, g.results);
  assert.match(out, /SKIP b\.txt:/);
  assert.doesNotMatch(out, /first bad tick/);
  assert.ok(out.endsWith('RAGDOLL RESULT FAIL'));
});
test('parseArgs reads the ragdoll group beside tapes and rows in any order', function () {
  assert.deepEqual(parseArgs(['--ragdoll', 'x', 'y']), { tapes: [], rows: [], ragdoll: ['x', 'y'], error: null });
  assert.deepEqual(parseArgs(['a', 'b', '--rows', 'r', 's', '--ragdoll', 'x', 'y', 'z']), { tapes: ['a', 'b'], rows: ['r', 's'], ragdoll: ['x', 'y', 'z'], error: null });
  assert.deepEqual(parseArgs(['a', 'b', '--ragdoll', 'x', 'y', '--rows', 'r', 's']), { tapes: ['a', 'b'], rows: ['r', 's'], ragdoll: ['x', 'y'], error: null });
  assert.match(parseArgs(['--ragdoll', 'x']).error, /^usage:/);
  assert.match(parseArgs(['a', 'b', '--ragdoll', 'x', 'y', '--ragdoll', 'z', 'w']).error, /only one --ragdoll group/);
  assert.match(parseArgs(['a', 'b', '--rows', 'x', 'y', '--ragdoll', 'p', 'q', '--rows', 'z', 'w']).error, /only one --rows group/);
});
test('run compares ragdoll rows over files and sets the exit code', function () {
  const dir = mkdtempSync(join(tmpdir(), 'golden-matrix-ragdoll-'));
  try {
    const put = function (name, data) {
      const p = join(dir, name);
      writeFileSync(p, data);
      return p;
    };
    const tapeA = put('a.e2040', makeTape({ seed: 2040n, ticks: 40 }));
    const tapeB = put('b.e2040', makeTape({ seed: 2040n, ticks: 40 }));
    const ragA = put('ra.txt', makeRagdoll());
    const ragB = put('rb.txt', makeRagdoll({ host: 'wasi-wasm32' }));
    const ragC = put('rc.txt', makeRagdoll({ host: 'linux-aarch64', digest: function (t, b) { return (t >= 4 && b === 3) ? 5n : BigInt(1000 + t * 100 + b); } }));
    const same = run([tapeA, tapeB, '--ragdoll', ragA, ragB]);
    assert.equal(same.code, 0, same.err + same.out);
    assert.match(same.out, /\nRESULT PASS\n/);
    assert.match(same.out, /RAGDOLL RESULT PASS/);
    assert.doesNotMatch(same.out, /CORPUS/);
    const bad = run([tapeA, tapeB, '--ragdoll', ragA, ragB, ragC]);
    assert.equal(bad.code, 1);
    assert.match(bad.out, /first bad tick 4 body 3/);
    assert.match(bad.out, /\nRESULT PASS\n/);
    assert.match(bad.out, /RAGDOLL RESULT FAIL/);
    const notRow = run(['--ragdoll', ragA, tapeA]);
    assert.equal(notRow.code, 2);
    assert.match(notRow.err, /not a ragdoll row/);
    const missing = run(['--ragdoll', ragA, join(dir, 'gone.txt')]);
    assert.equal(missing.code, 2);
    assert.match(missing.err, /not a ragdoll row: cannot read .*gone\.txt/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

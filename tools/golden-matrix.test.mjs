// golden-matrix.test.mjs - unit tests for golden-matrix.mjs, no cargo.
import assert from 'node:assert/strict';
import test from 'node:test';
import { causeNote, compareTapes, formatMatrix, parseTape } from './golden-matrix.mjs';
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

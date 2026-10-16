// golden-matrix.mjs - compare input/checkpoint tapes across machines, no cargo.
// Reads tapes from `engine2040 --seed <s> --ticks <n> --record <path>`:
// 8 bytes magic `E2040RP\0`, u16 format (1), u16 fixture (1), u64 seed,
// u32 ticks, u32 input count, then 16 bytes per input (u32 tick, u32 body,
// f32 vx, f32 vy), then 16 bytes per checkpoint (u64 legacy, u64 state),
// all little endian, no trailing bytes (header 28 bytes).
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
export const TAPE_MAGIC = Buffer.from([69, 50, 48, 52, 48, 82, 80, 0]);
export const TAPE_FORMAT = 1;
export const TAPE_FIXTURE = 1;
export const TAPE_HEADER = 28;
export function parseTape(buf) {
  const bytes = Buffer.isBuffer(buf) ? buf : Buffer.from(buf);
  if (bytes.length < TAPE_HEADER) {
    throw new Error('not a tape: short header, got ' + bytes.length + ' bytes');
  }
  if (!bytes.subarray(0, 8).equals(TAPE_MAGIC)) {
    throw new Error('not a tape: bad magic');
  }
  const format = bytes.readUInt16LE(8);
  const fixture = bytes.readUInt16LE(10);
  if (format !== TAPE_FORMAT || fixture !== TAPE_FIXTURE) {
    throw new Error('not a tape: unsupported version format=' + format + ' fixture=' + fixture);
  }
  const seed = bytes.readBigUInt64LE(12);
  const ticks = bytes.readUInt32LE(20);
  const count = bytes.readUInt32LE(24);
  const want = TAPE_HEADER + count * 16 + ticks * 16;
  if (bytes.length !== want) {
    throw new Error('not a tape: length mismatch, got ' + bytes.length + ' want ' + want);
  }
  const inputs = [];
  for (let i = 0; i < count; i += 1) {
    const off = TAPE_HEADER + i * 16;
    inputs.push(bytes.subarray(off, off + 16).toString('hex'));
  }
  const checkpoints = [];
  const base = TAPE_HEADER + count * 16;
  for (let t = 0; t < ticks; t += 1) {
    const off = base + t * 16;
    checkpoints.push({
      legacy: bytes.readBigUInt64LE(off),
      state: bytes.readBigUInt64LE(off + 8)
    });
  }
  return { format: format, fixture: fixture, seed: seed, ticks: ticks, inputs: inputs, checkpoints: checkpoints };
}
export function classifyCause(a, b) {
  const x = BigInt(a) ^ BigInt(b);
  if (x === 0n) {
    return null;
  }
  const bigA = BigInt(a);
  const bigB = BigInt(b);
  const d = bigA > bigB ? bigA - bigB : bigB - bigA;
  let pop = 0;
  let v = x;
  while (v > 0n) {
    pop += Number(v & 1n);
    v >>= 1n;
  }
  if (d <= 2n && pop <= 2) {
    return 'fma';
  }
  if (x < 256n || d < 1024n) {
    return 'libm';
  }
  return 'sort-tie';
}
export function causeNote(tick, cause) {
  const tag = cause ? ' [cause=' + cause + ']' : '';
  if (tick === 0) {
    return 'start state differs: seed, init order or struct layout' + tag;
  }
  return 'tick ' + tick + ' differs after ' + (tick - 1) + ' matched: bisect that tick\'s systems for sort ties, libm calls or fused multiply-add' + tag;
}
export function compareTapes(base, other) {
  if (base.format !== other.format || base.fixture !== other.fixture) {
    return { equal: false, comparable: false, firstDivergentTick: null, lane: null, note: 'versions differ: format ' + base.format + '/' + other.format + ' fixture ' + base.fixture + '/' + other.fixture };
  }
  if (base.seed !== other.seed) {
    return { equal: false, comparable: false, firstDivergentTick: null, lane: null, note: 'seed differs: ' + String(base.seed) + ' vs ' + String(other.seed) };
  }
  if (base.ticks !== other.ticks) {
    return { equal: false, comparable: false, firstDivergentTick: null, lane: null, note: 'tick count differs: ' + base.ticks + ' vs ' + other.ticks };
  }
  if (base.inputs.length !== other.inputs.length) {
    return { equal: false, comparable: false, firstDivergentTick: null, lane: null, note: 'inputs differ: count ' + base.inputs.length + ' vs ' + other.inputs.length };
  }
  for (let i = 0; i < base.inputs.length; i += 1) {
    if (base.inputs[i] !== other.inputs[i]) {
      return { equal: false, comparable: false, firstDivergentTick: null, lane: null, note: 'inputs differ: input ' + i + ' differs' };
    }
  }
  for (let t = 0; t < base.ticks; t += 1) {
    const a = base.checkpoints[t];
    const b = other.checkpoints[t];
    const legacyDiff = a.legacy !== b.legacy;
    const stateDiff = a.state !== b.state;
    if (legacyDiff || stateDiff) {
      const lane = legacyDiff && stateDiff ? 'both' : (legacyDiff ? 'legacy' : 'state');
      let cause = null;
      if (legacyDiff && stateDiff) {
        const cs = classifyCause(a.state, b.state);
        const cl = classifyCause(a.legacy, b.legacy);
        cause = (cs === 'sort-tie' || cl === 'sort-tie') ? 'sort-tie' : ((cs === 'libm' || cl === 'libm') ? 'libm' : 'fma');
      } else {
        cause = classifyCause(lane === 'legacy' ? a.legacy : a.state, lane === 'legacy' ? b.legacy : b.state);
      }
      return { equal: false, comparable: true, firstDivergentTick: t, lane: lane, cause: cause, note: causeNote(t, cause) };
    }
  }
  return { equal: true, comparable: true, firstDivergentTick: null, lane: null, note: '' };
}
function lastStateHex(tape) {
  if (tape.checkpoints.length === 0) {
    return '0000000000000000';
  }
  return tape.checkpoints[tape.checkpoints.length - 1].state.toString(16).padStart(16, '0');
}
export function formatMatrix(entries, results) {
  const lines = [];
  lines.push('golden-matrix: ' + entries.length + ' tapes');
  for (const e of entries) {
    lines.push(e.name + ' seed=' + String(e.tape.seed) + ' ticks=' + e.tape.ticks + ' inputs=' + e.tape.inputs.length + ' last_state=' + lastStateHex(e.tape));
  }
  const first = entries.length > 0 ? entries[0].name : '';
  for (let i = 1; i < entries.length; i += 1) {
    const r = results[i - 1];
    const name = entries[i].name;
    if (!r.comparable) {
      lines.push('SKIP ' + name + ': ' + r.note);
    } else if (r.equal) {
      lines.push('MATCH ' + name + ' vs ' + first);
    } else {
      lines.push('DIVERGE ' + name + ' vs ' + first + ': first bad tick ' + r.firstDivergentTick + ' (' + r.lane + ' lane) cause=' + r.cause + ' - ' + r.note);
    }
  }
  const pass = results.every(function (r) { return r.equal; });
  lines.push(pass ? 'RESULT PASS' : 'RESULT FAIL');
  return lines.join('\n');
}
const isMain = import.meta.main;
if (isMain) {
  const args = process.argv.slice(2);
  if (args.length < 2) {
    process.stderr.write('usage: node tools/golden-matrix.mjs <a.e2040> <b.e2040> [c.e2040 ...]\n');
    process.exitCode = 2;
  } else {
    try {
      const entries = args.map(function (p) {
        let buf = null;
        try {
          buf = readFileSync(p);
        } catch (err) {
          throw new Error('not a tape: cannot read ' + p + ': ' + (err && err.message ? err.message : String(err)));
        }
        try {
          return { name: p, tape: parseTape(buf) };
        } catch (err) {
          const msg = err && err.message ? err.message : String(err);
          const head = msg.indexOf('not a tape') === 0 ? msg : 'not a tape: ' + msg;
          throw new Error(head + ' (' + p + ')');
        }
      });
      const base = entries[0].tape;
      const results = entries.slice(1).map(function (e) { return compareTapes(base, e.tape); });
      process.stdout.write(formatMatrix(entries, results) + '\n');
      process.exitCode = results.every(function (r) { return r.equal; }) ? 0 : 1;
    } catch (err) {
      process.stderr.write((err && err.message ? err.message : String(err)) + '\n');
      process.exitCode = 2;
    }
  }
}

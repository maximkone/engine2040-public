// golden-matrix.mjs - compare input/checkpoint tapes, corpus rows and ragdoll rows across machines, no cargo.
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
// Corpus rows: the two lines `matrix_line` prints on one machine. Line one is
// `<host> golden=<16hex> corpus=<16hex> blocks=<n>`, line two is
// `<host> every=<cases> cut_checkpoints=<16hex>,<16hex>,...` with n digests.
export function parseRows(text) {
  const lines = String(text).replace(/^﻿/, '').split(/\r?\n/).filter(function (l) { return l.length > 0; });
  if (lines.length !== 2) {
    throw new Error('not a matrix row: want 2 lines (digest row, block row), got ' + lines.length);
  }
  const digest = /^(\S+) golden=([0-9a-f]{16}) corpus=([0-9a-f]{16}) blocks=(\d+)$/.exec(lines[0]);
  if (!digest) {
    throw new Error('not a matrix row: bad digest row "' + lines[0] + '"');
  }
  const blocks = /^(\S+) every=(\d+) cut_checkpoints=([0-9a-f]{16}(?:,[0-9a-f]{16})*)$/.exec(lines[1]);
  if (!blocks) {
    throw new Error('not a matrix row: bad block row "' + lines[1] + '"');
  }
  if (digest[1] !== blocks[1]) {
    throw new Error('not a matrix row: host ' + digest[1] + ' on the digest row but ' + blocks[1] + ' on the block row');
  }
  const every = Number(blocks[2]);
  if (!(every >= 1)) {
    throw new Error('not a matrix row: every must be at least 1, got ' + blocks[2]);
  }
  const checkpoints = blocks[3].split(',').map(function (h) { return BigInt('0x' + h); });
  if (checkpoints.length !== Number(digest[4])) {
    throw new Error('not a matrix row: blocks=' + digest[4] + ' but the block row lists ' + checkpoints.length);
  }
  return { host: digest[1], golden: BigInt('0x' + digest[2]), corpus: BigInt('0x' + digest[3]), every: every, checkpoints: checkpoints };
}
function hex16(v) {
  return v.toString(16).padStart(16, '0');
}
// Block digests first (they name the case range), then the two pinned digests.
export function compareRows(base, other) {
  if (base.every !== other.every || base.checkpoints.length !== other.checkpoints.length) {
    return { equal: false, comparable: false, firstDivergentBlock: null, firstCase: null, cause: null, note: 'corpus shape differs: every ' + base.every + '/' + other.every + ' blocks ' + base.checkpoints.length + '/' + other.checkpoints.length };
  }
  for (let b = 0; b < base.checkpoints.length; b += 1) {
    if (base.checkpoints[b] !== other.checkpoints[b]) {
      const cause = classifyCause(base.checkpoints[b], other.checkpoints[b]);
      const first = b * base.every;
      const last = first + base.every - 1;
      return { equal: false, comparable: true, firstDivergentBlock: b, firstCase: first, cause: cause, note: 'first bad corpus block ' + b + ' (cases ' + first + '..=' + last + ') [cause=' + cause + ']: bisect that block for sort ties, libm calls, or fused multiply-add' };
    }
  }
  if (base.golden !== other.golden) {
    return { equal: false, comparable: true, firstDivergentBlock: null, firstCase: null, cause: null, note: 'golden digest differs: ' + hex16(base.golden) + ' vs ' + hex16(other.golden) };
  }
  if (base.corpus !== other.corpus) {
    return { equal: false, comparable: true, firstDivergentBlock: null, firstCase: null, cause: null, note: 'corpus digest differs: ' + hex16(base.corpus) + ' vs ' + hex16(other.corpus) };
  }
  return { equal: true, comparable: true, firstDivergentBlock: null, firstCase: null, cause: null, note: '' };
}
export function formatRows(entries, results) {
  const lines = [];
  lines.push('golden-matrix: ' + entries.length + ' corpus rows');
  for (const e of entries) {
    lines.push(e.name + ' host=' + e.row.host + ' golden=' + hex16(e.row.golden) + ' corpus=' + hex16(e.row.corpus) + ' blocks=' + e.row.checkpoints.length);
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
      lines.push('DIVERGE ' + name + ' vs ' + first + ' (' + entries[i].row.host + ' vs ' + entries[0].row.host + '): ' + r.note);
    }
  }
  lines.push(results.every(function (r) { return r.equal; }) ? 'CORPUS RESULT PASS' : 'CORPUS RESULT FAIL');
  return lines.join('\n');
}
// Ragdoll rows: what `ragdoll_line` prints on one machine. Line one is
// `<host> ragdoll=<16hex> ticks=<n> bodies=<n> contacts=<n> final_state=<16hex>`,
// then one line per tick, counted from 0: `<host> tick=<t> bodies=<16hex>,<16hex>,...`
// with one digest per body in skeleton order.
export function parseRagdoll(text) {
  const lines = String(text).replace(/^﻿/, '').split(/\r?\n/).filter(function (l) { return l.length > 0; });
  if (lines.length < 2) {
    throw new Error('not a ragdoll row: want a summary line and a line per tick, got ' + lines.length + ' lines');
  }
  const sum = /^(\S+) ragdoll=([0-9a-f]{16}) ticks=(\d+) bodies=(\d+) contacts=(\d+) final_state=([0-9a-f]{16})$/.exec(lines[0]);
  if (!sum) {
    throw new Error('not a ragdoll row: bad summary line "' + lines[0] + '"');
  }
  const host = sum[1];
  const ticks = Number(sum[3]);
  const bodies = Number(sum[4]);
  if (!(ticks >= 1) || !(bodies >= 1)) {
    throw new Error('not a ragdoll row: ticks and bodies must be at least 1, got ticks=' + sum[3] + ' bodies=' + sum[4]);
  }
  if (lines.length !== 1 + ticks) {
    throw new Error('not a ragdoll row: ticks=' + ticks + ' but ' + (lines.length - 1) + ' tick lines');
  }
  const perTick = [];
  for (let t = 0; t < ticks; t += 1) {
    const m = /^(\S+) tick=(\d+) bodies=([0-9a-f]{16}(?:,[0-9a-f]{16})*)$/.exec(lines[t + 1]);
    if (!m) {
      throw new Error('not a ragdoll row: bad tick line "' + lines[t + 1] + '"');
    }
    if (m[1] !== host) {
      throw new Error('not a ragdoll row: host ' + host + ' on the summary line but ' + m[1] + ' on tick line ' + t);
    }
    if (Number(m[2]) !== t) {
      throw new Error('not a ragdoll row: tick line ' + t + ' says tick=' + m[2]);
    }
    const digests = m[3].split(',').map(function (h) { return BigInt('0x' + h); });
    if (digests.length !== bodies) {
      throw new Error('not a ragdoll row: bodies=' + bodies + ' but tick ' + t + ' lists ' + digests.length);
    }
    perTick.push(digests);
  }
  return { host: host, digest: BigInt('0x' + sum[2]), ticks: ticks, bodies: bodies, contacts: BigInt(sum[5]), finalState: BigInt('0x' + sum[6]), perTick: perTick };
}
// Body digests first (they name the tick and the body), then what the transforms do not carry.
export function compareRagdoll(base, other) {
  if (base.ticks !== other.ticks || base.bodies !== other.bodies) {
    return { equal: false, comparable: false, firstTick: null, firstBody: null, note: 'ragdoll shape differs: ticks ' + base.ticks + '/' + other.ticks + ' bodies ' + base.bodies + '/' + other.bodies };
  }
  for (let t = 0; t < base.ticks; t += 1) {
    let first = -1;
    let count = 0;
    for (let b = 0; b < base.bodies; b += 1) {
      if (base.perTick[t][b] !== other.perTick[t][b]) {
        count += 1;
        if (first < 0) {
          first = b;
        }
      }
    }
    if (first >= 0) {
      const note = 'first bad tick ' + t + ' body ' + first + ' (skeleton order; ' + count + ' of ' + base.bodies + ' bodies differ on that tick): Rapier stepped differently here, look for fused multiply-add, SIMD, or a contact/joint order that depends on the machine';
      return { equal: false, comparable: true, firstTick: t, firstBody: first, note: note };
    }
  }
  if (base.contacts !== other.contacts) {
    return { equal: false, comparable: true, firstTick: null, firstBody: null, note: 'every transform matches but contacts differ: ' + base.contacts + ' vs ' + other.contacts };
  }
  if (base.finalState !== other.finalState) {
    return { equal: false, comparable: true, firstTick: null, firstBody: null, note: 'every transform matches but the final state hash differs: ' + hex16(base.finalState) + ' vs ' + hex16(other.finalState) + ' (velocities or solver counters)' };
  }
  if (base.digest !== other.digest) {
    return { equal: false, comparable: true, firstTick: null, firstBody: null, note: 'ragdoll digest differs: ' + hex16(base.digest) + ' vs ' + hex16(other.digest) };
  }
  return { equal: true, comparable: true, firstTick: null, firstBody: null, note: '' };
}
export function formatRagdoll(entries, results) {
  const lines = [];
  lines.push('golden-matrix: ' + entries.length + ' ragdoll rows');
  for (const e of entries) {
    lines.push(e.name + ' host=' + e.row.host + ' ragdoll=' + hex16(e.row.digest) + ' ticks=' + e.row.ticks + ' bodies=' + e.row.bodies + ' contacts=' + e.row.contacts + ' final_state=' + hex16(e.row.finalState));
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
      lines.push('DIVERGE ' + name + ' vs ' + first + ' (' + entries[i].row.host + ' vs ' + entries[0].row.host + '): ' + r.note);
    }
  }
  lines.push(results.every(function (r) { return r.equal; }) ? 'RAGDOLL RESULT PASS' : 'RAGDOLL RESULT FAIL');
  return lines.join('\n');
}
const USAGE = 'usage: node tools/golden-matrix.mjs <a.e2040> <b.e2040> [c.e2040 ...] [--rows <a.txt> <b.txt> [c.txt ...]] [--ragdoll <a.txt> <b.txt> [c.txt ...]]';
const GROUP_FLAGS = { '--rows': 'rows', '--ragdoll': 'ragdoll' };
// Tape files first, then the corpus row files after `--rows` and the ragdoll row files after
// `--ragdoll` (either order); each group needs two or more files.
export function parseArgs(argv) {
  const groups = { tapes: [], rows: [], ragdoll: [] };
  const seen = new Set();
  let current = 'tapes';
  for (const a of argv) {
    if (a.indexOf('--') !== 0) {
      groups[current].push(a);
      continue;
    }
    const group = GROUP_FLAGS[a];
    if (group === undefined) {
      return { tapes: [], rows: [], ragdoll: [], error: 'unknown flag ' + a + '\n' + USAGE };
    }
    if (seen.has(a)) {
      return { tapes: [], rows: [], ragdoll: [], error: 'only one ' + a + ' group is allowed\n' + USAGE };
    }
    seen.add(a);
    current = group;
  }
  const sizes = [groups.tapes.length, groups.rows.length, groups.ragdoll.length];
  if (sizes.every(function (n) { return n === 0; }) || sizes.some(function (n) { return n === 1; })) {
    return { tapes: [], rows: [], ragdoll: [], error: USAGE };
  }
  return { tapes: groups.tapes, rows: groups.rows, ragdoll: groups.ragdoll, error: null };
}
function readNamed(p, kind, parse) {
  let text = null;
  try {
    text = readFileSync(p);
  } catch (err) {
    throw new Error('not a ' + kind + ': cannot read ' + p + ': ' + (err && err.message ? err.message : String(err)));
  }
  try {
    return parse(text);
  } catch (err) {
    const msg = err && err.message ? err.message : String(err);
    const head = msg.indexOf('not a ' + kind) === 0 ? msg : 'not a ' + kind + ': ' + msg;
    throw new Error(head + ' (' + p + ')');
  }
}
// Exit code 0 = every file agrees, 1 = a divergence or a skipped pair, 2 = usage or an unreadable file.
// Each file group (tapes, corpus rows, ragdoll rows) is compared against its own first file.
export function run(argv) {
  const args = parseArgs(argv);
  if (args.error) {
    return { code: 2, out: '', err: args.error + '\n' };
  }
  try {
    const sections = [];
    let pass = true;
    if (args.tapes.length > 0) {
      const entries = args.tapes.map(function (p) { return { name: p, tape: readNamed(p, 'tape', parseTape) }; });
      const results = entries.slice(1).map(function (e) { return compareTapes(entries[0].tape, e.tape); });
      sections.push(formatMatrix(entries, results));
      pass = pass && results.every(function (r) { return r.equal; });
    }
    if (args.rows.length > 0) {
      const entries = args.rows.map(function (p) { return { name: p, row: readNamed(p, 'matrix row', function (b) { return parseRows(b.toString('utf8')); }) }; });
      const results = entries.slice(1).map(function (e) { return compareRows(entries[0].row, e.row); });
      sections.push(formatRows(entries, results));
      pass = pass && results.every(function (r) { return r.equal; });
    }
    if (args.ragdoll.length > 0) {
      const entries = args.ragdoll.map(function (p) { return { name: p, row: readNamed(p, 'ragdoll row', function (b) { return parseRagdoll(b.toString('utf8')); }) }; });
      const results = entries.slice(1).map(function (e) { return compareRagdoll(entries[0].row, e.row); });
      sections.push(formatRagdoll(entries, results));
      pass = pass && results.every(function (r) { return r.equal; });
    }
    return { code: pass ? 0 : 1, out: sections.join('\n') + '\n', err: '' };
  } catch (err) {
    return { code: 2, out: '', err: (err && err.message ? err.message : String(err)) + '\n' };
  }
}
const isMain = import.meta.main;
if (isMain) {
  const done = run(process.argv.slice(2));
  process.stdout.write(done.out);
  process.stderr.write(done.err);
  process.exitCode = done.code;
}

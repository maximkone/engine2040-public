// Unit tests for the pure parts of clip.mjs. Run: node --test tools/motion/clip.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import {
  parseArgs,
  kimodoPython,
  genArgs,
  parseGenResult,
  gateReceipt,
  parseBakeLine,
  provenanceRow,
  summaryLine,
  SLIP_MAX,
} from './clip.mjs';

const SLASH = ['--prompt', 'A person raises a sword and brings it down in a strong overhead strike.',
  '--name', 'slash_over', '--seed', '71235', '--seconds', '2.0'];

test('parseArgs reads the documented command line', () => {
  const o = parseArgs([...SLASH, '--install']);
  assert.equal(o.name, 'slash_over');
  assert.equal(o.seed, 71235);
  assert.equal(o.seconds, 2.0);
  assert.equal(o.install, true);
  assert.equal(o.allowSlip, false);
  assert.equal(o.slipMax, SLIP_MAX);
  assert.equal(o.prompt, 'A person raises a sword and brings it down in a strong overhead strike.');
});

test('parseArgs takes --flag=value and defaults seconds to 2', () => {
  const o = parseArgs(['--prompt=A person walks forward.', '--name=walk_fwd', '--seed=7']);
  assert.equal(o.seconds, 2);
  assert.equal(o.name, 'walk_fwd');
  assert.equal(o.seed, 7);
});

test('parseArgs refuses bad input with the reason', () => {
  assert.throws(() => parseArgs(['--name', 'x_y', '--seed', '1']), /missing --prompt/);
  assert.throws(() => parseArgs([...SLASH, '--name', 'Slash-Over']), /must match/);
  assert.throws(() => parseArgs([...SLASH, '--seed', '-3']), /whole number/);
  assert.throws(() => parseArgs([...SLASH, '--seed', '1.5']), /whole number/);
  assert.throws(() => parseArgs([...SLASH, '--seconds', '0.1']), /between 0.5 and 10/);
  assert.throws(() => parseArgs([...SLASH, '--seconds', 'abc']), /between/);
  assert.throws(() => parseArgs([...SLASH, '--bogus', '1']), /unknown flag --bogus/);
  assert.throws(() => parseArgs([...SLASH, '--install=yes']), /takes no value/);
  assert.throws(() => parseArgs([...SLASH, 'yes']), /unexpected argument/);
  assert.throws(() => parseArgs(['--prompt', '   ', '--name', 'a_b', '--seed', '1']), /empty/);
  assert.throws(() => parseArgs([...SLASH, '--seed']), /needs a value/);
});

test('parseArgs reads the batch flags and checks the pick is inside the batch', () => {
  const o = parseArgs([...SLASH, '--samples', '4', '--pick', '1']);
  assert.equal(o.samples, 4);
  assert.equal(o.pick, 1);
  assert.equal(parseArgs(SLASH).samples, 1);
  assert.equal(parseArgs(SLASH).pick, 0);
  assert.throws(() => parseArgs([...SLASH, '--samples', '17']), /1..16/);
  assert.throws(() => parseArgs([...SLASH, '--samples', '2', '--pick', '2']), /0..1/);
  assert.throws(() => parseArgs([...SLASH, '--pick', '1']), /0..0/);
});

test('parseArgs --help short-circuits', () => {
  assert.deepEqual(parseArgs(['--help']), { help: true });
});

test('kimodoPython points at the venv on Windows and on POSIX', () => {
  assert.equal(
    kimodoPython('C:\\forge-media\\kimodo', 'win32'),
    path.join('C:\\forge-media\\kimodo', 'venv', 'Scripts', 'python.exe'),
  );
  assert.equal(kimodoPython('/opt/kimodo', 'linux'), path.join('/opt/kimodo', 'venv', 'bin', 'python'));
});

test('genArgs builds the kimodo_gen.py call with the seed and seconds as strings', () => {
  const a = genArgs({ prompt: 'p', seed: 71235, seconds: 2, outDir: '/o', stem: 'slash_over' });
  assert.equal(path.basename(a[0]), 'kimodo_gen.py');
  assert.deepEqual(a.slice(1), ['--prompt', 'p', '--seed', '71235', '--seconds', '2', '--samples', '1', '--pick', '0', '--out-dir', '/o', '--stem', 'slash_over']);
});

test('parseGenResult takes the last JSON line with ok, skipping log noise', () => {
  const out = [
    'Loaded model: Kimodo-SOMA-RP-v1.2',
    '{"partial": true}',
    '{"ok": false, "status": "missing_prompt"}',
    '{"ok": true, "status": "generated", "seed": 71235}',
    '',
  ].join('\n');
  assert.deepEqual(parseGenResult(out), { ok: true, status: 'generated', seed: 71235 });
  assert.equal(parseGenResult('no json here'), null);
  assert.equal(parseGenResult(''), null);
});

const RECEIPT_OK = { avg_slip: { leftfoot: 0.000389, rightfoot: 0.00014 } };
const RECEIPT_SLIP = { avg_slip: { leftfoot: 0.133841, rightfoot: 0.116123 } };

test('gateReceipt passes a clip under the slip gate and prints the numbers', () => {
  const g = gateReceipt(RECEIPT_OK);
  assert.equal(g.ok, true);
  assert.match(g.message, /pass: left 0.000389 m\/s, right 0.000140 m\/s/);
});

test('gateReceipt fails a sliding clip and names the number', () => {
  const g = gateReceipt(RECEIPT_SLIP);
  assert.equal(g.ok, false);
  assert.match(g.message, /left 0.133841 m\/s/);
  assert.match(g.message, /right 0.116123 m\/s/);
  assert.match(g.message, /allow-slip/);
});

test('gateReceipt fails when only one foot is over the gate', () => {
  assert.equal(gateReceipt({ avg_slip: { leftfoot: 0.01, rightfoot: 0.2 } }).ok, false);
  assert.equal(gateReceipt({ avg_slip: { leftfoot: 0.2, rightfoot: 0.01 } }).ok, false);
});

test('gateReceipt treats exactly the gate as a pass and just over it as a fail', () => {
  assert.equal(gateReceipt({ avg_slip: { leftfoot: 0.08, rightfoot: 0.08 } }).ok, true);
  assert.equal(gateReceipt({ avg_slip: { leftfoot: 0.080001, rightfoot: 0.0 } }).ok, false);
});

test('gateReceipt keeps a sliding clip with --allow-slip, marked FAILED', () => {
  const g = gateReceipt(RECEIPT_SLIP, { allowSlip: true });
  assert.equal(g.ok, true);
  assert.match(g.message, /FAILED, kept by --allow-slip/);
});

test('gateReceipt refuses a receipt with no slip numbers', () => {
  assert.equal(gateReceipt({}).ok, false);
  assert.equal(gateReceipt({ avg_slip: { leftfoot: 'x', rightfoot: 0 } }).ok, false);
});

test('parseBakeLine reads the line motion_bake prints', () => {
  const line = 'motion_bake: clip=slash_over seed=none source=C:/x/slash_over.npz source_sha256=1a8ca2b8 ' +
    'ticks=119 qclip_hash=0x4b3b8bb4fb746161 avg_slip=0.000389/0.000140 packed=103823 ' +
    'retarget_digest=0x16aead24055062c9 out=target/motion/slash_over/slash_over.am5p receipt=target/motion/slash_over/slash_over.receipt.json\n';
  const b = parseBakeLine(`Compiling noise\n${line}`);
  assert.equal(b.clip, 'slash_over');
  assert.equal(b.ticks, 119);
  assert.deepEqual(b.avgSlip, [0.000389, 0.00014]);
  assert.equal(b.packed, 103823);
  assert.equal(b.qclipHash, '0x4b3b8bb4fb746161');
  assert.equal(parseBakeLine('error: something'), null);
});

test('provenanceRow writes the fixtures table row with steps', () => {
  const row = provenanceRow({
    name: 'slash_over',
    prompt: 'A person raises a sword and brings it down in a strong overhead strike.',
    seed: 71235,
    steps: 100,
    ticks: 119,
    slip: { leftfoot: 0.000389, rightfoot: 0.00014 },
  });
  assert.equal(
    row,
    '| `slash_over.am5p` | `slash_over` | A person raises a sword and brings it down in a strong overhead strike. | 71235 | 100 | 119 | 0.000389 / 0.000140 |',
  );
});

test('provenanceRow writes the content table row without steps', () => {
  const row = provenanceRow(
    { name: 'parry', prompt: 'A person parries.', seed: 71234, steps: 100, ticks: 119, slip: { leftfoot: 0.080097, rightfoot: 0.06819 } },
    'content',
  );
  assert.equal(row, '| `parry.am5p` | `parry` | A person parries. | 71234 | 119 | 0.080097 / 0.068190 |');
});

test('summaryLine carries ticks, slip, bytes, sha256, gate and install state', () => {
  const s = summaryLine({
    name: 'slash_over',
    ticks: 119,
    slip: { leftfoot: 0.000389, rightfoot: 0.00014 },
    bytes: 103823,
    sha256: 'abc',
    gate: 'pass',
    installed: false,
  });
  assert.equal(s, 'clip slash_over: ticks=119 slip=0.000389/0.000140 bytes=103823 sha256=abc gate=pass installed=no');
});

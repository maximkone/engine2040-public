// Run: node --test tools/pe-exports.test.mjs
// GS8 reads a PE export table. Synthetic PE32 and PE32+ images pin the parser,
// a text file pins the exit-2 path, and kernel32 pins CreateFileW on Windows.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { listExports, parseExports } from './pe-exports.mjs';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const TOOL = join(ROOT, 'tools', 'pe-exports.mjs');
const KERNEL32 = 'C:\\Windows\\System32\\kernel32.dll';
const KERNEL_SKIP = process.platform !== 'win32'
  ? 'needs Windows for kernel32.dll'
  : (existsSync(KERNEL32) ? false : 'no kernel32.dll in this tree');

/** Build a minimal one-section PE image exporting `names`. */
function makePe(magic, names) {
  const is32p = magic === 0x20b;
  const sizeOpt = is32p ? 240 : 224;
  const eLfanew = 64;
  const vaddr = 0x2000;
  const rawPtr = 512;
  const dllName = Buffer.from('test.dll\0', 'ascii');
  const nameBufs = names.map(function (n) { return Buffer.from(n + '\0', 'ascii'); });
  let rel = 40;
  const funcRel = rel;
  rel += names.length * 4;
  const ptrRel = rel;
  rel += names.length * 4;
  const ordRel = rel;
  rel += names.length * 2;
  const dllRel = rel;
  rel += dllName.length;
  const strRels = nameBufs.map(function (b) {
    const r = rel;
    rel += b.length;
    return r;
  });
  const rawSize = 256;
  const buf = Buffer.alloc(rawPtr + rawSize);
  buf[0] = 0x4d;
  buf[1] = 0x5a;
  buf.writeUInt32LE(eLfanew, 0x3c);
  buf.writeUInt32LE(0x00004550, eLfanew);
  const coff = eLfanew + 4;
  buf.writeUInt16LE(0x8664, coff);
  buf.writeUInt16LE(1, coff + 2);
  buf.writeUInt16LE(sizeOpt, coff + 16);
  const opt = coff + 20;
  buf.writeUInt16LE(magic, opt);
  if (is32p) {
    buf.writeUInt32LE(16, opt + 108);
    buf.writeUInt32LE(vaddr, opt + 112);
    buf.writeUInt32LE(40, opt + 116);
  } else {
    buf.writeUInt32LE(16, opt + 92);
    buf.writeUInt32LE(vaddr, opt + 96);
    buf.writeUInt32LE(40, opt + 100);
  }
  const sec = opt + sizeOpt;
  Buffer.from('.edata\0\0').copy(buf, sec);
  buf.writeUInt32LE(rawSize, sec + 8);
  buf.writeUInt32LE(vaddr, sec + 12);
  buf.writeUInt32LE(rawSize, sec + 16);
  buf.writeUInt32LE(rawPtr, sec + 20);
  const dir = rawPtr;
  buf.writeUInt32LE(vaddr + dllRel, dir + 12);
  buf.writeUInt32LE(1, dir + 16);
  buf.writeUInt32LE(names.length, dir + 20);
  buf.writeUInt32LE(names.length, dir + 24);
  buf.writeUInt32LE(vaddr + funcRel, dir + 28);
  buf.writeUInt32LE(vaddr + ptrRel, dir + 32);
  buf.writeUInt32LE(vaddr + ordRel, dir + 36);
  for (let i = 0; i < names.length; i += 1) {
    buf.writeUInt32LE(vaddr + strRels[i], rawPtr + ptrRel + i * 4);
    buf.writeUInt16LE(i, rawPtr + ordRel + i * 2);
    buf.writeUInt32LE(vaddr, rawPtr + funcRel + i * 4);
  }
  dllName.copy(buf, rawPtr + dllRel);
  nameBufs.forEach(function (b, i) { b.copy(buf, rawPtr + strRels[i]); });
  return buf;
}

function withTempFile(bytes, body) {
  const dir = mkdtempSync(join(tmpdir(), 'pe-exports-'));
  const file = join(dir, 'sample.bin');
  writeFileSync(file, bytes);
  try {
    body(file);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('PE32 image lists its export names in order', function () {
  assert.deepEqual(parseExports(makePe(0x10b, ['Alpha', 'Beta'])), ['Alpha', 'Beta']);
});

test('PE32+ image lists its export names', function () {
  assert.deepEqual(parseExports(makePe(0x20b, ['Gamma'])), ['Gamma']);
});

test('text bytes are not a PE file', function () {
  assert.throws(function () { parseExports(Buffer.from('hello')); }, /not a PE/);
  assert.throws(function () { parseExports(Buffer.alloc(128)); }, /not a PE/);
});

test('CLI prints one name per line for a PE file', function () {
  withTempFile(makePe(0x10b, ['Alpha', 'Beta']), function (file) {
    const r = spawnSync(process.execPath, [TOOL, file], { encoding: 'utf8' });
    assert.equal(r.status, 0);
    assert.deepEqual(r.stdout.split('\n').filter(function (l) { return l.length > 0; }), ['Alpha', 'Beta']);
  });
});

test('CLI exits 2 with a message for a non-PE file', function () {
  withTempFile(Buffer.from('not a pe'), function (file) {
    const r = spawnSync(process.execPath, [TOOL, file], { encoding: 'utf8' });
    assert.equal(r.status, 2);
    assert.match(r.stderr, /not a PE/i);
  });
});

test('CLI exits 2 with a message for a missing file', function () {
  const missing = join(ROOT, 'tools', 'no-such-pe-exports.dll');
  const r = spawnSync(process.execPath, [TOOL, missing], { encoding: 'utf8' });
  assert.equal(r.status, 2);
  assert.ok(r.stderr.length > 0);
});

test('truncated export table is not a PE file', function () {
  const full = makePe(0x10b, ['Alpha']);
  assert.throws(function () { parseExports(full.subarray(0, 520)); }, /not a PE/);
  withTempFile(full.subarray(0, 520), function (file) {
    const r = spawnSync(process.execPath, [TOOL, file], { encoding: 'utf8' });
    assert.equal(r.status, 2);
    assert.match(r.stderr, /not a PE/i);
  });
});

test('CLI exits 2 with a message for a missing arg', function () {
  const r = spawnSync(process.execPath, [TOOL], { encoding: 'utf8' });
  assert.equal(r.status, 2);
  assert.match(r.stderr, /usage/i);
});

test('listExports reads names from a file', function () {
  withTempFile(makePe(0x10b, ['One']), function (file) {
    assert.deepEqual(listExports(file), ['One']);
  });
});

test('kernel32.dll lists CreateFileW', { skip: KERNEL_SKIP }, function () {
  assert.ok(listExports(KERNEL32).includes('CreateFileW'));
});

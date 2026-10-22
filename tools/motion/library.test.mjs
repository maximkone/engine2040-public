// Tests for the prompt library and the cached/missing report. No network, no GPU, no Kimodo.
// Run: node --test tools/motion/library.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import {
  DEFAULT_LIBRARY,
  ID_RE,
  PROMPT_MAX,
  parseLibrary,
  loadLibrary,
  readNpzStrings,
  readCacheTexts,
  cacheReport,
} from './library.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const CLIP = path.join(HERE, 'clip.mjs');

// The four shipped clips' prompts. The library must carry them verbatim: kaggle_encode.py
// rebuilds the whole cache from the library, so a dropped line would lose a shipped clip.
const SHIPPED = [
  'A person raises a sword and brings it down in a strong overhead strike.',
  'A person lunges forward and thrusts a spear.',
  'A person parries an attack with a sword and then strikes back.',
  'A person swings a sword in a fast horizontal slash.',
];

// Build a numpy-style .npz (zip) in memory: a unicode array for 'texts' and a float stub for 'emb'.
function npyStrings(strings) {
  const width = Math.max(1, ...strings.map((s) => [...s].length));
  const dict = `{'descr': '<U${width}', 'fortran_order': False, 'shape': (${strings.length},), }`;
  const pad = (64 - ((10 + dict.length + 1) % 64)) % 64;
  const header = Buffer.from(dict + ' '.repeat(pad) + '\n', 'latin1');
  const len = Buffer.alloc(2);
  len.writeUInt16LE(header.length);
  const body = Buffer.alloc(strings.length * width * 4);
  strings.forEach((s, i) => {
    [...s].forEach((ch, j) => body.writeUInt32LE(ch.codePointAt(0), (i * width + j) * 4));
  });
  return Buffer.concat([Buffer.from('\x93NUMPY\x01\x00', 'latin1'), len, header, body]);
}

function zipOf(members) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const m of members) {
    const name = Buffer.from(m.name, 'utf8');
    const raw = m.data;
    const body = m.deflate ? zlib.deflateRawSync(raw) : raw;
    const method = m.deflate ? 8 : 0;
    const crc = zlib.crc32(raw);
    const lh = Buffer.alloc(30);
    lh.writeUInt32LE(0x04034b50, 0);
    lh.writeUInt16LE(20, 4);
    lh.writeUInt16LE(method, 8);
    lh.writeUInt32LE(crc, 14);
    lh.writeUInt32LE(body.length, 18);
    lh.writeUInt32LE(raw.length, 22);
    lh.writeUInt16LE(name.length, 26);
    locals.push(lh, name, body);
    const ch = Buffer.alloc(46);
    ch.writeUInt32LE(0x02014b50, 0);
    ch.writeUInt16LE(20, 4);
    ch.writeUInt16LE(20, 6);
    ch.writeUInt16LE(method, 10);
    ch.writeUInt32LE(crc, 16);
    ch.writeUInt32LE(body.length, 20);
    ch.writeUInt32LE(raw.length, 24);
    ch.writeUInt16LE(name.length, 28);
    ch.writeUInt32LE(offset, 42);
    centrals.push(ch, name);
    offset += 30 + name.length + body.length;
  }
  const cdSize = centrals.reduce((n, b) => n + b.length, 0);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(members.length, 8);
  eocd.writeUInt16LE(members.length, 10);
  eocd.writeUInt32LE(cdSize, 12);
  eocd.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, ...centrals, eocd]);
}

function fakeNpz(texts, { deflate = false } = {}) {
  return zipOf([
    { name: 'texts.npy', data: npyStrings(texts), deflate },
    { name: 'emb.npy', data: Buffer.alloc(64), deflate },
  ]);
}

function tmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 'rb5-library-'));
}

test('the library file parses with no errors and holds at least 150 prompts', () => {
  const text = fs.readFileSync(DEFAULT_LIBRARY, 'utf8');
  const { entries, errors } = parseLibrary(text);
  assert.deepEqual(errors, []);
  assert.ok(entries.length >= 150, `want >= 150 prompts, got ${entries.length}`);
  assert.equal(loadLibrary(DEFAULT_LIBRARY).length, entries.length);
});

test('every id is lower case, digits or underscore, and ids and prompt texts are unique', () => {
  const { entries } = parseLibrary(fs.readFileSync(DEFAULT_LIBRARY, 'utf8'));
  for (const e of entries) {
    assert.match(e.id, ID_RE, `bad id ${e.id}`);
    assert.ok(e.prompt.length <= PROMPT_MAX, `${e.id} is too long`);
  }
  assert.equal(new Set(entries.map((e) => e.id)).size, entries.length, 'ids repeat');
  assert.equal(new Set(entries.map((e) => e.prompt)).size, entries.length, 'prompt texts repeat');
});

test('the library carries the four shipped clip prompts verbatim', () => {
  const prompts = new Set(parseLibrary(fs.readFileSync(DEFAULT_LIBRARY, 'utf8')).entries.map((e) => e.prompt));
  for (const p of SHIPPED) assert.ok(prompts.has(p), `missing shipped prompt: ${p}`);
});

test('every RB5 category is present', () => {
  const ids = parseLibrary(fs.readFileSync(DEFAULT_LIBRARY, 'utf8')).entries.map((e) => e.id);
  const has = (re) => ids.some((id) => re.test(id));
  const want = {
    'cuts, horizontal': /^slash_h_/,
    'cuts, overhead': /^slash_over/,
    'cuts, diagonal': /^slash_diag_/,
    'cuts, rising': /^slash_rise/,
    'thrusts': /^thrust_/,
    'blocks, high': /^block_high_/,
    'blocks, low': /^block_low_/,
    'blocks, shield': /^block_shield_/,
    'parries': /^parry/,
    'shield bashes': /^shield_bash_/,
    'feints': /^feint_/,
    'dodges and sidesteps': /^dodge_/,
    'staggers, light': /^stagger_light_/,
    'staggers, heavy': /^stagger_heavy_/,
    'staggers, from front': /^stagger_(light|heavy)_front$/,
    'staggers, from back': /^stagger_(light|heavy)_back$/,
    'knockdowns': /^knockdown_/,
    'get-ups': /^getup_/,
    'guard idles': /^guard_idle_/,
    'taunts': /^taunt_/,
    'walking in guard': /^walk_guard/,
    'running in guard': /^run_guard/,
  };
  for (const [name, re] of Object.entries(want)) assert.ok(has(re), `no ${name} prompt (${re})`);
  const deaths = ids.filter((id) => /^death_/.test(id));
  assert.ok(deaths.length >= 4, `want several deaths, got ${deaths.length}`);
  for (const fam of ['slash_h', 'slash_over', 'slash_diag', 'slash_rise', 'thrust', 'block_high', 'parry', 'shield_bash']) {
    assert.ok(ids.includes(`${fam}_light`), `no light ${fam}`);
    assert.ok(ids.includes(`${fam}_heavy`), `no heavy ${fam}`);
  }
});

test('parseLibrary skips comments and blanks and refuses each bad line with its number', () => {
  const good = '# header\n\nslash_h\tA person slashes.\n  # indented comment\n';
  const ok = parseLibrary(good);
  assert.deepEqual(ok.errors, []);
  assert.deepEqual(ok.entries, [{ id: 'slash_h', prompt: 'A person slashes.', line: 3 }]);

  const bad = [
    ['no tab', 'slash_h A person slashes.\n', /line 1: want id<TAB>prompt/],
    ['upper case id', 'Slash_H\tA person slashes.\n', /line 1: id "Slash_H" must be/],
    ['dash in id', 'slash-h\tA person slashes.\n', /must be lower case/],
    ['empty prompt', 'slash_h\t   \n', /line 1: id slash_h has an empty prompt/],
    ['too long', `slash_h\t${'x'.repeat(PROMPT_MAX + 1)}\n`, /past the 500 cap/],
    ['repeated id', 'a_b\tOne person.\na_b\tTwo people.\n', /line 2: id a_b repeats line 1/],
    ['repeated prompt', 'a_b\tOne person.\nc_d\tOne person.\n', /line 2: id c_d has the same prompt text as line 1/],
  ];
  for (const [name, text, re] of bad) {
    const { errors } = parseLibrary(text);
    assert.equal(errors.length, 1, `${name}: one error`);
    assert.match(errors[0], re, name);
  }
});

test('readNpzStrings reads back unicode texts from stored and deflated npz members', () => {
  const texts = ['A person slashes.', 'Café: a lunge é 剣 🗡'];
  assert.deepEqual(readNpzStrings(fakeNpz(texts)), texts);
  assert.deepEqual(readNpzStrings(fakeNpz(texts, { deflate: true })), texts);
});

test('readNpzStrings refuses a file that is not an npz', () => {
  assert.throws(() => readNpzStrings(Buffer.from('not a zip at all, just text')), /not a zip/);
  assert.throws(() => readNpzStrings(zipOf([{ name: 'emb.npy', data: Buffer.alloc(8) }])), /no texts.npy/);
});

test('the cached/missing report works on a fake cache dir', () => {
  const dir = tmpDir();
  try {
    const lib = path.join(dir, 'lib.txt');
    fs.writeFileSync(lib, 'a_one\tA person one.\nb_two\tA person two.\nc_three\tA person three.\n');
    const entries = loadLibrary(lib);

    // No cache file: everything is missing, and the cache reads as null.
    assert.equal(readCacheTexts(path.join(dir, 'emb.npz')), null);
    const none = cacheReport(entries, null);
    assert.deepEqual(none.cached, []);
    assert.deepEqual(none.missing.map((e) => e.id), ['a_one', 'b_two', 'c_three']);

    // A cache holding two of the three texts (plus one unrelated text).
    fs.writeFileSync(path.join(dir, 'emb.npz'), fakeNpz(['A person two.', 'A person one.', 'Unrelated.']));
    const rep = cacheReport(entries, readCacheTexts(path.join(dir, 'emb.npz')));
    assert.equal(rep.total, 3);
    assert.deepEqual(rep.cached.map((e) => e.id), ['a_one', 'b_two']);
    assert.deepEqual(rep.missing.map((e) => e.id), ['c_three']);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('clip.mjs --list-library exits 0 when all cached, 1 when some missing, 2 on a bad library', () => {
  const dir = tmpDir();
  try {
    const lib = path.join(dir, 'lib.txt');
    fs.writeFileSync(lib, 'a_one\tA person one.\nb_two\tA person two.\n');
    fs.writeFileSync(path.join(dir, 'emb.npz'), fakeNpz(['A person one.', 'A person two.']));
    // Keep the child's env free of a stray KIMODO_HOME or KIMODO_CACHE from this machine.
    const env = { ...process.env };
    delete env.KIMODO_HOME;
    delete env.KIMODO_CACHE;
    const run = (args) => spawnSync(process.execPath, [CLIP, '--list-library', ...args], { encoding: 'utf8', env });

    const all = run([lib, '--kimodo-home', dir]);
    assert.equal(all.status, 0, all.stderr);
    assert.match(all.stdout, /cached: 2 of 2/);
    assert.match(all.stdout, /missing: 0/);

    fs.writeFileSync(path.join(dir, 'emb.npz'), fakeNpz(['A person one.']));
    const some = run([lib, '--kimodo-home', dir]);
    assert.equal(some.status, 1, some.stderr);
    assert.match(some.stdout, /cached: 1 of 2/);
    assert.match(some.stdout, /missing: 1\n  b_two/);

    fs.writeFileSync(lib, 'Bad_ID\tA person.\n');
    const bad = run([lib, '--kimodo-home', dir]);
    assert.equal(bad.status, 2);
    assert.match(bad.stderr, /must be lower case/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

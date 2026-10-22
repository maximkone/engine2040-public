// The Kimodo prompt library: parse content/motion/combat_prompts.txt, read the texts
// out of an emb.npz with no Python, and report which ids are cached.
//
// Line format: id<TAB>prompt. Ids are lower case letters, digits and underscore.
// Blank lines and lines starting with # are skipped. Pure except the two file readers.

import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(HERE, '..', '..');
export const DEFAULT_LIBRARY = path.join(ROOT, 'content', 'motion', 'combat_prompts.txt');
export const ID_RE = /^[a-z0-9_]{1,60}$/;
export const PROMPT_MAX = 500; // same cap as clip.mjs and kimodo_gen.py

// Parse the library text. Returns { entries: [{ id, prompt, line }], errors: [string] }.
// Every rule is checked; a file with any error should not be encoded.
export function parseLibrary(text) {
  const entries = [];
  const errors = [];
  const ids = new Map();
  const prompts = new Map();
  String(text).split(/\r?\n/).forEach((raw, i) => {
    const line = i + 1;
    const body = raw.trim();
    if (!body || body.startsWith('#')) return;
    const tab = raw.indexOf('\t');
    if (tab < 0) {
      errors.push(`line ${line}: want id<TAB>prompt`);
      return;
    }
    const id = raw.slice(0, tab).trim();
    const prompt = raw.slice(tab + 1).trim();
    if (!ID_RE.test(id)) {
      errors.push(`line ${line}: id ${JSON.stringify(id)} must be lower case letters, digits or underscore (1..60)`);
      return;
    }
    if (!prompt) {
      errors.push(`line ${line}: id ${id} has an empty prompt`);
      return;
    }
    if (prompt.length > PROMPT_MAX) {
      errors.push(`line ${line}: id ${id} prompt is ${prompt.length} characters, past the ${PROMPT_MAX} cap`);
      return;
    }
    if (ids.has(id)) {
      errors.push(`line ${line}: id ${id} repeats line ${ids.get(id)}`);
      return;
    }
    if (prompts.has(prompt)) {
      errors.push(`line ${line}: id ${id} has the same prompt text as line ${prompts.get(prompt)}`);
      return;
    }
    ids.set(id, line);
    prompts.set(prompt, line);
    entries.push({ id, prompt, line });
  });
  return { entries, errors };
}

// Read and parse a library file. Throws one Error listing every problem.
export function loadLibrary(file = DEFAULT_LIBRARY) {
  const { entries, errors } = parseLibrary(fs.readFileSync(file, 'utf8'));
  if (errors.length) throw new Error(`${file} is not a valid prompt library:\n  ${errors.join('\n  ')}`);
  if (entries.length === 0) throw new Error(`${file} has no prompts`);
  return entries;
}

// The texts stored in an .npz (numpy zip) member, as strings. Supports stored and deflated
// entries and '<U' unicode arrays, which is what kaggle_encode.py writes for 'texts'.
export function readNpzStrings(buf, member = 'texts') {
  const eocd = findEocd(buf);
  const count = buf.readUInt16LE(eocd + 10);
  let p = buf.readUInt32LE(eocd + 16);
  const found = new Map();
  for (let i = 0; i < count; i++) {
    if (buf.readUInt32LE(p) !== 0x02014b50) throw new Error('npz central directory is damaged');
    const method = buf.readUInt16LE(p + 10);
    const csize = buf.readUInt32LE(p + 20);
    const nlen = buf.readUInt16LE(p + 28);
    const xlen = buf.readUInt16LE(p + 30);
    const clen = buf.readUInt16LE(p + 32);
    const loff = buf.readUInt32LE(p + 42);
    found.set(buf.toString('utf8', p + 46, p + 46 + nlen), { method, csize, loff });
    p += 46 + nlen + xlen + clen;
  }
  const entry = found.get(`${member}.npy`);
  if (!entry) throw new Error(`npz has no ${member}.npy member`);
  const start = entry.loff + 30 + buf.readUInt16LE(entry.loff + 26) + buf.readUInt16LE(entry.loff + 28);
  let data = buf.subarray(start, start + entry.csize);
  if (entry.method === 8) data = zlib.inflateRawSync(data);
  else if (entry.method !== 0) throw new Error(`npz member ${member}.npy uses unsupported compression ${entry.method}`);

  if (data.toString('latin1', 0, 6) !== '\x93NUMPY') throw new Error(`${member}.npy is not an npy array`);
  const major = data[6];
  const hlen = major === 1 ? data.readUInt16LE(8) : data.readUInt32LE(8);
  const hstart = major === 1 ? 10 : 12;
  const header = data.toString('latin1', hstart, hstart + hlen);
  const descr = /'descr':\s*'([^']+)'/.exec(header);
  const unicode = descr && /^[<|=]U(\d+)$/.exec(descr[1]);
  if (!unicode) throw new Error(`${member}.npy is not a unicode array (descr ${descr ? descr[1] : 'missing'})`);
  if (descr[1][0] === '>') throw new Error(`${member}.npy is big-endian, not supported`);
  const shape = /'shape':\s*\((\d+)/.exec(header);
  if (!shape) throw new Error(`${member}.npy has no 1-D shape`);
  const n = Number(shape[1]);
  const width = Number(unicode[1]) * 4; // UTF-32LE code points
  const body = hstart + hlen;
  if (body + n * width > data.length) throw new Error(`${member}.npy is shorter than its shape says`);
  const out = [];
  for (let i = 0; i < n; i++) {
    let s = '';
    const at = body + i * width;
    for (let j = 0; j < width; j += 4) {
      const cp = data.readUInt32LE(at + j);
      if (cp === 0) break;
      s += String.fromCodePoint(cp);
    }
    out.push(s);
  }
  return out;
}

function findEocd(buf) {
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 65557); i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) return i;
  }
  throw new Error('file is not a zip (no end of central directory)');
}

// Texts in the cache file. Returns null when the file is absent (every id is then missing).
export function readCacheTexts(cacheFile) {
  if (!fs.existsSync(cacheFile)) return null;
  return readNpzStrings(fs.readFileSync(cacheFile), 'texts');
}

// Split the library into cached and missing, by exact prompt text (how kimodo_gen.py matches).
export function cacheReport(entries, cachedTexts) {
  const have = new Set(cachedTexts || []);
  return {
    total: entries.length,
    cached: entries.filter((e) => have.has(e.prompt)),
    missing: entries.filter((e) => !have.has(e.prompt)),
  };
}

#!/usr/bin/env node
// Project memory: a JSONL store of the lessons a build leaves behind.
//
// A row is one **decision**, one **failed fix** or one **tuned value**, and it
// carries the tape hash (or golden) it was taken under, the source path it is
// about and a UTC stamp. The hash is the point: a lesson without the tape it
// was learned on cannot be re-checked, so `record` refuses one.
//
//   node tools/memory/memory.mjs record --kind failed-fix \
//       --hash d57b5c8ae3096614 --path crates/anim/src/motion_bake.rs \
//       --text "avg_slip reads the local foot offset; the pack carries local
//              offsets, so the planted-foot meter is avg_world_slip" \
//       [--utc 2026-10-23T13:05:00Z] [--evidence be75d2c32]
//   node tools/memory/memory.mjs query [--kind K] [--path P] [--hash H] [--json]
//   node tools/memory/memory.mjs --selftest
//
// The store defaults to tools/memory/memory.jsonl next to this file; `--store`
// points at another one. Query prints one JSON row per line, sorted by UTC
// stamp then id, so the same store always answers the same way.
//
// The pure helpers are exported for tests; the CLI is the last function.

import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const GOLDEN = 'd57b5c8ae3096614';
export const ROOT = path.resolve(HERE, '..', '..');
export const DEFAULT_STORE = path.join(HERE, 'memory.jsonl');
export const KINDS = ['decision', 'failed-fix', 'tuned-value'];
export const HASH_RE = /^(?:0x)?[0-9a-f]{16}$/i;
const STAMP_RE = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;

const USAGE = `usage: node tools/memory/memory.mjs <command> [options]

  record  --kind <decision|failed-fix|tuned-value> --hash <16 hex> --path <repo/path>
          --text "<lesson>" [--utc <YYYY-MM-DDTHH:MM:SSZ>] [--evidence <run id|commit>]
          [--store <file>]
  query   [--kind <k>] [--path <substring>] [--hash <h>] [--json] [--store <file>]
  --selftest   replay a fixed fixture twice and compare the answers
  --help       this text

A row without a tape hash is refused: a lesson that cannot be re-checked under
the hash it was learned on is not memory, it is opinion.`;

export class MemoryError extends Error {}

/** The UTC stamp a row carries: explicit, never a wall clock in a fixture. */
export function utcNow(now = new Date()) {
  return now.toISOString().replace(/\.\d{3}Z$/, 'Z');
}

/** Stable row id: the canonical fields, hashed, so one row has one id. */
export function rowId(row) {
  const canonical = [row.kind, row.hash, row.path, row.utc, row.text, row.evidence ?? ''].join('\u0000');
  return createHash('sha256').update(canonical).digest('hex').slice(0, 12);
}

/** Validate one row and return it with its id; throws MemoryError on a refusal. */
export function validateRow(row) {
  if (!KINDS.includes(row.kind)) {
    throw new MemoryError(`kind must be one of ${KINDS.join(', ')}, got ${JSON.stringify(row.kind)}`);
  }
  if (row.hash === undefined || row.hash === null || String(row.hash).trim() === '') {
    throw new MemoryError('row has no tape hash');
  }
  if (!HASH_RE.test(String(row.hash))) {
    throw new MemoryError(`hash is not 16 hex digits (0x optional): ${row.hash}`);
  }
  if (typeof row.path !== 'string' || row.path.trim() === '') {
    throw new MemoryError('row has no source path');
  }
  const sourcePath = row.path.replaceAll('\\', '/');
  if (path.isAbsolute(sourcePath) || sourcePath.startsWith('../')) {
    throw new MemoryError(`path must be repo-relative: ${row.path}`);
  }
  if (typeof row.text !== 'string' || row.text.trim() === '') {
    throw new MemoryError('row has no text');
  }
  const utc = row.utc === undefined || row.utc === null ? utcNow() : String(row.utc);
  if (!STAMP_RE.test(utc)) {
    throw new MemoryError(`utc must be YYYY-MM-DDTHH:MM:SSZ, got ${utc}`);
  }
  const hash = String(row.hash).replace(/^0x/i, '').toLowerCase();
  const out = {
    kind: row.kind,
    hash,
    path: sourcePath,
    utc,
    text: row.text.trim(),
  };
  if (row.evidence !== undefined && row.evidence !== null && String(row.evidence).trim() !== '') {
    out.evidence = String(row.evidence).trim();
  }
  out.id = rowId(out);
  return out;
}

/** Rows from JSONL text; blank lines are skipped, bad JSON is a refusal. */
export function parseRows(text, where = 'store') {
  const rows = [];
  for (const [index, line] of text.split('\n').entries()) {
    const trimmed = line.trim();
    if (trimmed === '') continue;
    let row;
    try {
      row = JSON.parse(trimmed);
    } catch (error) {
      throw new MemoryError(`${where}:${index + 1}: not JSON (${error.message})`);
    }
    rows.push(validateRow(row));
  }
  return rows;
}

/** Read a store file; a missing store is an empty memory, not an error. */
export function readStore(file) {
  if (!fs.existsSync(file)) return [];
  return parseRows(fs.readFileSync(file, 'utf8'), path.basename(file));
}

/** Append one validated row: one JSON line, one newline, never a rewrite. */
export function appendRow(file, row) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.appendFileSync(file, `${JSON.stringify(row)}\n`);
  return row;
}

/** Rows matching every filter given, sorted by UTC stamp then id. */
export function filterRows(rows, filters = {}) {
  const wantedHash = filters.hash === undefined ? undefined : String(filters.hash).replace(/^0x/i, '').toLowerCase();
  return rows
    .filter((row) => filters.kind === undefined || row.kind === filters.kind)
    .filter((row) => filters.path === undefined || row.path.includes(String(filters.path).replaceAll('\\', '/')))
    .filter((row) => wantedHash === undefined || row.hash === wantedHash)
    .sort((a, b) => a.utc.localeCompare(b.utc) || a.id.localeCompare(b.id));
}

/** The query answer: one JSON line per row, nothing else on stdout. */
export function renderRows(rows) {
  return rows.map((row) => JSON.stringify(row)).join('\n');
}

function parseArgs(argv) {
  const out = { _: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === '--json') out.json = true;
    else if (arg === '--selftest') out.selftest = true;
    else if (arg === '--help' || arg === '-h') out.help = true;
    else if (arg.startsWith('--')) {
      const key = arg.slice(2);
      const value = argv[index + 1];
      if (value === undefined || (value.startsWith('--') && value !== '--')) {
        throw new MemoryError(`${arg} needs a value`);
      }
      out[key] = value;
      index += 1;
    } else {
      out._.push(arg);
    }
  }
  return out;
}

function commandRecord(args) {
  const row = validateRow({
    kind: args.kind,
    hash: args.hash,
    path: args.path,
    text: args.text,
    utc: args.utc,
    evidence: args.evidence,
  });
  const store = path.resolve(args.store ?? DEFAULT_STORE);
  appendRow(store, row);
  process.stdout.write(`${JSON.stringify(row)}\n`);
  return 0;
}

function commandQuery(args) {
  const store = path.resolve(args.store ?? DEFAULT_STORE);
  const rows = filterRows(readStore(store), { kind: args.kind, path: args.path, hash: args.hash });
  const answer = renderRows(rows);
  if (answer !== '') process.stdout.write(`${answer}\n`);
  const filters = ['kind', 'path', 'hash'].filter((key) => args[key] !== undefined).map((key) => `${key}=${args[key]}`);
  process.stderr.write(`memory: ${rows.length} row(s)${filters.length ? ` match ${filters.join(' ')}` : ''}\n`);
  return 0;
}

/** A fixed fixture: two decisions, one failed fix, one tuned value. */
const FIXTURE = [
  {
    kind: 'decision',
    hash: GOLDEN,
    path: 'crates/game/src/bin/arena.rs',
    text: 'commit a shared-tree file by path and by hunks: a whole-file write swept another lane\'s work',
    utc: '2026-10-23T09:40:00Z',
    evidence: '294738a77',
  },
  {
    kind: 'failed-fix',
    hash: GOLDEN,
    path: 'crates/game/src/console.rs',
    text: 'git add one path did not scope the commit: the shared index carried a foreign staged removal',
    utc: '2026-10-23T12:18:00Z',
    evidence: '0a08078c5',
  },
  {
    kind: 'failed-fix',
    hash: GOLDEN,
    path: 'crates/anim/src/motion_bake.rs',
    text: 'the local foot offset is not slip; the pack is local, so the planted-foot meter is the world one',
    utc: '2026-10-23T13:05:00Z',
    evidence: 'be75d2c32',
  },
  {
    kind: 'tuned-value',
    hash: GOLDEN,
    path: 'crates/anim/src/readable_lod.rs',
    text: 'ARROW_DRAW_CHORD 0.120432 (0.12 plus the Taylor d^3/4), LEAN_VISIBLE_MIN 0.02 rad',
    utc: '2026-10-23T13:30:00Z',
    evidence: 'a3d25e82c',
  },
];

/** One selftest replay: record the fixture, answer three queries, as text. */
function replay(store) {
  const lines = [];
  for (const row of FIXTURE) {
    const saved = validateRow(row);
    appendRow(store, saved);
    lines.push(JSON.stringify(saved));
  }
  const queries = [
    {},
    { kind: 'failed-fix' },
    { path: 'crates/anim' },
    { hash: GOLDEN },
    { hash: '0x0000000000000000' },
  ];
  const answers = [];
  for (const filters of queries) {
    const rows = filterRows(readStore(store), filters);
    answers.push(renderRows(rows));
  }
  return { recorded: lines, answers };
}

function commandSelftest() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'e2040-memory-'));
  const first = replay(path.join(dir, 'first.jsonl'));
  const second = replay(path.join(dir, 'second.jsonl'));
  fs.rmSync(dir, { recursive: true, force: true });
  const rows = first.recorded;
  for (const line of rows) {
    const row = JSON.parse(line);
    process.stdout.write(`row ${row.id} ${row.kind} ${row.hash} ${row.path}\n`);
  }
  const equal = JSON.stringify(first) === JSON.stringify(second);
  const counts = first.answers.map((answer) => (answer === '' ? 0 : answer.split('\n').length));
  process.stdout.write(`queries: all ${counts[0]}, failed-fix ${counts[1]}, path ${counts[2]}, hash ${counts[3]}, other ${counts[4]}\n`);
  if (!equal) {
    process.stdout.write('FAIL memory selftest: the two replays answered differently\n');
    return 1;
  }
  // A row with no hash must be refused, and the refusal must not touch a store.
  let refused = false;
  try {
    validateRow({ kind: 'decision', path: 'crates/anim/src/readable.rs', text: 'no hash' });
  } catch (error) {
    refused = error instanceof MemoryError && /tape hash/.test(error.message);
  }
  if (!refused) {
    process.stdout.write('FAIL memory selftest: a row with no tape hash was accepted\n');
    return 1;
  }
  process.stdout.write(`PASS memory selftest: ${rows.length} rows x2 equal, no-hash row refused\n`);
  return 0;
}

function main(argv) {
  const args = parseArgs(argv);
  if (args.help) {
    process.stdout.write(`${USAGE}\n`);
    return 0;
  }
  if (args.selftest) return commandSelftest();
  const command = args._[0];
  if (command === 'record') return commandRecord(args);
  if (command === 'query') return commandQuery(args);
  throw new MemoryError(command === undefined ? 'no command (record, query or --selftest)' : `unknown command ${command}`);
}

const invoked = process.argv[1] ? path.resolve(process.argv[1]) === fileURLToPath(import.meta.url) : false;
if (invoked) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`memory: refuse: ${error.message}\n`);
    process.exitCode = 1;
  }
}

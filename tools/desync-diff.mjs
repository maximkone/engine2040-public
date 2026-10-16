// Factorio-style desync report over canonical system bytes.
//
// Usage:
//   node tools/desync-diff.mjs --expected <bin> --actual <bin> --system <name> --tick <u32> --out <dir>
//   node tools/desync-diff.mjs --expected <bin> --actual <bin> --system <name> --tick <u32> --out <dir> [--expected-hash <hex> --actual-hash <hex>]
//
// Inputs are canonical byte dumps from `system_canonical_bytes` (engine-arena
// `world|space|combat|rng|ai`, engine-proof `world|space|clock`): the FNV-1a64
// of the dump equals the system's diagnostic hash. The tool prints the first
// diverging byte and writes a report dir (`report.json`, `expected.bin`,
// `actual.bin`, `diff.txt`) so a desync names its tick plus system, never a
// bare hash. Exit 0 writes a report on a divergence, exit 1 means identical
// bytes (nothing to dump), exit 2 is a usage or IO error.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { fnv1a64Hex } from "./fnv1a64.mjs";
import { chain } from "./tape-checksum.mjs";

const SYSTEM_NAMES = ["world", "space", "combat", "rng", "ai", "clock"];
const MAX_DIFF_ROWS = 16;

function usage() {
  return [
    "usage: node tools/desync-diff.mjs --expected <bin> --actual <bin> --system <name> --tick <u32> --out <dir>",
    `  system is one of: ${SYSTEM_NAMES.join("|")}`,
  ].join("\n");
}

function fail(message) {
  process.stderr.write(`desync-diff: ${message}\n`);
  process.exitCode = 2;
  throw new Error(message);
}

// Pure CLI arg validation (exported for tests): throws the usage line on
// anything but the required flags plus the optional hash pair.
export function parseDesyncArgs(argv) {
  const args = { expected: null, actual: null, system: null, tick: null, out: null, expectedHash: null, actualHash: null };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const next = argv[i + 1];
    if (flag === "--expected" || flag === "--actual" || flag === "--system" || flag === "--tick" || flag === "--out" || flag === "--expected-hash" || flag === "--actual-hash") {
      if (next === undefined || next.startsWith("-")) throw new Error(`${usage()}\nmissing value for ${flag}`);
      i += 1;
      if (flag === "--expected") args.expected = next;
      else if (flag === "--actual") args.actual = next;
      else if (flag === "--system") args.system = next;
      else if (flag === "--tick") args.tick = next;
      else if (flag === "--out") args.out = next;
      else if (flag === "--expected-hash") args.expectedHash = next.toLowerCase();
      else args.actualHash = next.toLowerCase();
    } else if (flag === "--help" || flag === "-h") {
      throw new Error(usage());
    } else {
      throw new Error(`${usage()}\nunknown argument: ${flag}`);
    }
  }
  for (const key of ["expected", "actual", "system", "tick", "out"]) {
    if (!args[key]) throw new Error(`${usage()}\nmissing required --${key}`);
  }
  if (!SYSTEM_NAMES.includes(args.system)) throw new Error(`${usage()}\nunknown system: ${args.system}`);
  if (!/^\d+$/.test(args.tick) || Number(args.tick) > 4294967295) throw new Error(`${usage()}\n--tick must be a u32`);
  for (const hex of [args.expectedHash, args.actualHash]) {
    if (hex !== null && !/^[0-9a-f]{16}$/.test(hex)) throw new Error(`${usage()}\n--*-hash must be 16 lowercase hex digits`);
  }
  return args;
}

// First index where the dumps differ, or -1 when they are byte-identical.
// A length mismatch diverges at the shared-prefix end.
export function firstDivergingByte(expected, actual) {
  const a = expected instanceof Uint8Array ? expected : new Uint8Array(expected);
  const b = actual instanceof Uint8Array ? actual : new Uint8Array(actual);
  const shared = Math.min(a.length, b.length);
  for (let i = 0; i < shared; i += 1) {
    if (a[i] !== b[i]) return i;
  }
  if (a.length !== b.length) return shared;
  return -1;
}

// Up to MAX_DIFF_ROWS diverging offsets with both byte values, for diff.txt.
export function diffRows(expected, actual, limit = MAX_DIFF_ROWS) {
  const a = expected instanceof Uint8Array ? expected : new Uint8Array(expected);
  const b = actual instanceof Uint8Array ? actual : new Uint8Array(actual);
  const rows = [];
  const shared = Math.min(a.length, b.length);
  for (let i = 0; i < shared && rows.length < limit; i += 1) {
    if (a[i] !== b[i]) rows.push({ offset: i, expected: a[i], actual: b[i] });
  }
  if (rows.length < limit && a.length !== b.length) {
    rows.push({ offset: shared, expected: a.length > shared ? a[shared] : null, actual: b.length > shared ? b[shared] : null, truncated: true });
  }
  return rows;
}

// Write the Factorio-style report dir and return its summary. Always writes
// report.json plus both dumps; diff.txt lists the first diverging offsets.
export function writeDesyncReport({ outDir, tick, system, expectedBytes, actualBytes, expectedHash, actualHash }) {
  const expected = expectedBytes instanceof Uint8Array ? Buffer.from(expectedBytes) : Buffer.from(expectedBytes);
  const actual = actualBytes instanceof Uint8Array ? Buffer.from(actualBytes) : Buffer.from(actualBytes);
  const offset = firstDivergingByte(expected, actual);
  const computedExpected = expectedHash ?? chain(null, expected);
  const computedActual = actualHash ?? chain(null, actual);
  mkdirSync(outDir, { recursive: true });
  writeFileSync(join(outDir, "expected.bin"), expected);
  writeFileSync(join(outDir, "actual.bin"), actual);
  const rows = offset === -1 ? [] : diffRows(expected, actual);
  const diffText = offset === -1
    ? `tick ${tick} system ${system}: identical (${expected.length} bytes)\n`
    : [
        `tick ${tick} system ${system}: first divergence at byte ${offset} (expected ${expected.length} bytes, actual ${actual.length} bytes)`,
        ...rows.map((row) => {
          const exp = row.expected === null ? "eof" : `0x${row.expected.toString(16).padStart(2, "0")}`;
          const got = row.actual === null ? "eof" : `0x${row.actual.toString(16).padStart(2, "0")}`;
          return `  +${row.offset}: expected ${exp}, got ${got}`;
        }),
        "",
      ].join("\n");
  writeFileSync(join(outDir, "diff.txt"), diffText);
  const report = {
    schema_version: 1,
    tick: Number(tick),
    system,
    expected_hash: computedExpected,
    actual_hash: computedActual,
    expected_len: expected.length,
    actual_len: actual.length,
    first_byte: offset,
  };
  writeFileSync(join(outDir, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
  return { offset, report };
}

function readDump(path, cwd) {
  const abs = resolve(cwd, path);
  try {
    return readFileSync(abs);
  } catch {
    fail(`missing file: ${path}`);
  }
  return Buffer.alloc(0);
}

function main(argv, cwd) {
  let args;
  try {
    args = parseDesyncArgs(argv);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 2;
    throw error;
  }
  const expected = readDump(args.expected, cwd);
  const actual = readDump(args.actual, cwd);
  if (args.expectedHash && fnv1a64Hex(expected) !== args.expectedHash) {
    fail(`expected dump hash mismatch: file is ${fnv1a64Hex(expected)}, flag says ${args.expectedHash}`);
  }
  if (args.actualHash && fnv1a64Hex(actual) !== args.actualHash) {
    fail(`actual dump hash mismatch: file is ${fnv1a64Hex(actual)}, flag says ${args.actualHash}`);
  }
  const outDir = resolve(cwd, args.out);
  const { offset, report } = writeDesyncReport({
    outDir,
    tick: Number(args.tick),
    system: args.system,
    expectedBytes: expected,
    actualBytes: actual,
  });
  if (offset === -1) {
    process.stdout.write(`no divergence at tick ${args.tick} (system ${args.system}): ${expected.length} identical bytes\nREPORT: ${outDir}\n`);
    process.exitCode = 1;
    return;
  }
  process.stdout.write(`desync at tick ${args.tick} (system ${args.system}): first byte ${offset}: expected ${report.expected_hash}, got ${report.actual_hash}\nREPORT: ${outDir}\n`);
}

const isMain = import.meta.main;
if (isMain) {
  try {
    main(process.argv.slice(2), process.cwd());
  } catch {
    if (process.exitCode !== 2 && process.exitCode !== 1) process.exitCode = 2;
  }
}



// tape-checksum.mjs - per-tick FNV-1a64 chain plus first-divergence report.
// Checksum discipline follows ggrs SyncTestSession (checksum each tick):
// https://github.com/gschup/ggrs/blob/main/src/sessions/sync_test_session.rs
// MIT OR Apache-2.0 donor; fresh JS below, zero vendor bytes.
// Mirrors crates/core/src/hash.rs FNV64_OFFSET/PRIME (h=(h^byte)*prime) and
// tools/fnv1a64.mjs FNV_OFFSET/PRIME/MASK. Consumer: tools/desync-diff.mjs
// imports chain for the tick-7 pin.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
const FNV_OFFSET = 0xcbf29ce484222325n;
const FNV_PRIME = 0x100000001b3n;
const FNV_MASK = 0xffffffffffffffffn;
function toBytes(entry) {
  if (typeof entry === "string") return Buffer.from(entry, "utf8");
  if (Array.isArray(entry)) return Buffer.from(entry);
  return Buffer.from(JSON.stringify(entry) ?? "", "utf8");
}
export function chain(prevHex, bytes) {
  let h = prevHex === null || prevHex === undefined ? FNV_OFFSET : BigInt(`0x${prevHex}`);
  const buf = bytes instanceof Uint8Array ? bytes : Buffer.from(bytes);
  for (const b of buf) h = ((h ^ BigInt(b)) * FNV_PRIME) & FNV_MASK;
  return h.toString(16).padStart(16, "0");
}
export function tape(ticks) {
  const out = [];
  let prev = null;
  for (const t of ticks) {
    prev = chain(prev, toBytes(t));
    out.push(prev);
  }
  return out;
}
export function compare(aHex, bHex) {
  const n = Math.min(aHex.length, bHex.length);
  for (let i = 0; i < n; i += 1) if (aHex[i] !== bHex[i]) return { equal: false, tick: i };
  if (aHex.length !== bHex.length) return { equal: false, tick: n };
  return { equal: true, tick: null };
}
function loadTape(path, cwd) {
  const raw = JSON.parse(readFileSync(resolve(cwd, path), "utf8"));
  if (!Array.isArray(raw)) throw new Error(`not a tape: ${path} must hold a JSON array`);
  return raw;
}
function main(argv, cwd) {
  const at = argv.indexOf("--tapes");
  if (at === -1 || argv.length !== at + 3) {
    process.stderr.write("usage: node tools/tape-checksum.mjs --tapes <a.json> <b.json>\n");
    process.exitCode = 2;
    return;
  }
  const a = tape(loadTape(argv[at + 1], cwd));
  const b = tape(loadTape(argv[at + 2], cwd));
  const r = compare(a, b);
  if (r.equal) {
    process.stdout.write(`first-divergence: equal (${a.length} ticks)\n`);
    return;
  }
  process.stdout.write(`first-divergence: ${r.tick}\n`);
  process.stdout.write(`a[${r.tick}]=${a[r.tick] ?? "eof"} b[${r.tick}]=${b[r.tick] ?? "eof"}\n`);
}
const isMain = import.meta.main;
if (isMain) {
  try {
    main(process.argv.slice(2), process.cwd());
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    if (process.exitCode === 0) process.exitCode = 2;
  }
}

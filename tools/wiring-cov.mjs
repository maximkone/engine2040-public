// wiring-cov.mjs - which functions does the real arena never run?
// Nightly/report-only: instrumented `cargo llvm-cov run` of the arena binary
// for a short scripted run, then zero-hit functions grouped per crate.
// donor: taiki-e/cargo-llvm-cov --json (run the bin, never `cargo test`,
// so test-only callers do not count). Never runs in the 40-seat loop.
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const INSTALL_LINE = 'cargo install cargo-llvm-cov';
const DEFAULT_OUT = join('target', 'reports', 'wiring-cov.json');
const DEFAULT_RAW = join('target', 'reports', 'wiring-cov-raw.json');
const ARENA_ARGS = ['--scene', 'arena', '--fighters', '8', '--frames', '300', '--exit'];

export function hasLlvmCov() {
  const r = spawnSync('cargo', ['llvm-cov', '--version'], { encoding: 'utf8' });
  return r.status === 0;
}

function crateOf(filename) {
  const m = /crates[\\/]+([A-Za-z0-9_-]+)[\\/]/.exec(filename ?? '');
  return m ? `engine-${m[1]}` : 'unknown';
}

// llvm-cov export JSON: { data: [ { functions: [ { name, count, filenames } ] } ] }.
// Zero-hit only, grouped per crate, every list sorted for byte-identical x2.
export function groupZeroHits(report) {
  const groups = new Map();
  const blocks = report?.data ?? [];
  for (const block of blocks) {
    for (const fn of block?.functions ?? []) {
      if ((fn?.count ?? 0) !== 0) continue;
      const name = String(fn?.name ?? '');
      if (!name) continue;
      const crate = crateOf(fn?.filenames?.[0]);
      if (!groups.has(crate)) groups.set(crate, new Set());
      groups.get(crate).add(name);
    }
  }
  const out = {};
  for (const [crate, names] of [...groups.entries()].sort(([a], [b]) => (a < b ? -1 : 1))) {
    out[crate] = [...names].sort();
  }
  return out;
}

export function formatSummary(groups, outPath) {
  const crates = Object.keys(groups).length;
  const fns = Object.values(groups).reduce((n, xs) => n + xs.length, 0);
  return `wiring-cov: ${fns} zero-hit fns in ${crates} crates -> ${outPath}`;
}

function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

function realRun(outPath) {
  execFileSync('cargo', ['llvm-cov', 'run', '--bin', 'arena', '--', ...ARENA_ARGS], { stdio: 'inherit' });
  execFileSync('cargo', ['llvm-cov', 'report', '--json', '--output-path', DEFAULT_RAW], { stdio: 'inherit' });
  return groupZeroHits(readJson(DEFAULT_RAW));
}

function argAfter(argv, flag) {
  const i = argv.indexOf(flag);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : null;
}

export function main(argv = process.argv.slice(2)) {
  const fromJson = argAfter(argv, '--from-json');
  const outPath = argAfter(argv, '--out') ?? DEFAULT_OUT;
  let groups;
  if (fromJson) {
    groups = groupZeroHits(readJson(fromJson));
  } else {
    if (!hasLlvmCov()) {
      console.error(`wiring-cov: cargo-llvm-cov not installed: ${INSTALL_LINE}`);
      process.exit(2);
    }
    groups = realRun(outPath);
  }
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, `${JSON.stringify({ generated: 'wiring-cov', crates: groups }, null, 2)}\n`);
  console.log(formatSummary(groups, outPath));
}

if (import.meta.main) {
  main();
}

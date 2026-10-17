// wiring-cov.mjs - which functions does the real arena never run?
// Nightly/report-only: instrumented `cargo llvm-cov run` of the arena binary
// for a short scripted run, then zero-hit functions grouped per crate.
// donor: taiki-e/cargo-llvm-cov --json (run the bin, never `cargo test`,
// so test-only callers do not count). Never runs in the 40-seat loop.
//
// Use:  node tools/wiring-cov.mjs [--out <report.json>] [--all | --crates a,b]
//       node tools/wiring-cov.mjs --from-json <llvm-cov-export.json> [--out ..]
// Exit: 0 report written, 1 a cargo step failed, 2 cargo-llvm-cov missing or
// bad flags (the install line goes to stderr; a NOOP, not a failure).
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const INSTALL_LINE = 'cargo install cargo-llvm-cov';
export const REPORT_REL = 'target/reports/wiring-cov.json';
export const RAW_REL = 'target/reports/wiring-cov-raw.json';
// The row's five crates: the ones a player sees, hears or fights.
export const ROW_CRATES = ['engine-ai', 'engine-anim', 'engine-audio', 'engine-game', 'engine-render'];
// Same flags as FC1. The arena bin needs the `app` feature or cargo refuses it.
export const ARENA_ARGS = ['--scene', 'arena', '--fighters', '8', '--frames', '300', '--exit'];
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

export function hasLlvmCov() {
  const r = spawnSync('cargo', ['llvm-cov', '--version'], { encoding: 'utf8' });
  return r.status === 0;
}

function crateOf(filename) {
  const m = /crates[\\/]+([A-Za-z0-9_-]+)[\\/]/.exec(filename ?? '');
  return m ? `engine-${m[1]}` : 'unknown';
}

// llvm-cov export JSON: { data: [ { functions: [ { name, count, filenames } ] } ] }.
// A generic fn is one row per instantiation: a fn is zero-hit only when every
// instantiation of (file, name) is zero, so one live copy keeps it off the list.
// `crates` null keeps every crate; else only the listed ones. Lists are sorted
// so two runs over the same fixture are byte-identical.
export function analyze(report, { crates = ROW_CRATES } = {}) {
  const keep = crates ? new Set(crates) : null;
  const fns = new Map();
  for (const block of report?.data ?? []) {
    for (const fn of block?.functions ?? []) {
      const name = String(fn?.name ?? '');
      if (!name) continue;
      const file = String(fn?.filenames?.[0] ?? '');
      const crate = crateOf(file);
      if (keep && !keep.has(crate)) continue;
      const key = `${crate}\u0000${file}\u0000${name}`;
      const row = fns.get(key) ?? { crate, name, hits: 0 };
      row.hits += Number(fn?.count ?? 0) || 0;
      fns.set(key, row);
    }
  }
  const zero = new Map();
  const total = new Map();
  for (const { crate, name, hits } of fns.values()) {
    total.set(crate, (total.get(crate) ?? 0) + 1);
    if (hits !== 0) continue;
    if (!zero.has(crate)) zero.set(crate, new Set());
    zero.get(crate).add(name);
  }
  const byName = ([a], [b]) => (a < b ? -1 : a > b ? 1 : 0);
  const out = { crates: {}, totals: {} };
  for (const [crate, names] of [...zero.entries()].sort(byName)) {
    out.crates[crate] = [...names].sort();
  }
  for (const [crate, n] of [...total.entries()].sort(byName)) {
    out.totals[crate] = { zero: out.crates[crate]?.length ?? 0, total: n };
  }
  return out;
}

export function groupZeroHits(report, opts) {
  return analyze(report, opts).crates;
}

export function formatSummary(groups, outPath) {
  const crates = Object.keys(groups).length;
  const fns = Object.values(groups).reduce((n, xs) => n + xs.length, 0);
  return `wiring-cov: ${fns} zero-hit fns in ${crates} crates -> ${outPath}`;
}

// The three cargo calls of a real run, in order. `clean` first so old profile
// data cannot leak into this run; `--no-report` so only the json export prints.
export function cargoPlan(rawPath) {
  return [
    ['llvm-cov', 'clean', '--workspace'],
    ['llvm-cov', 'run', '--no-report', '-p', 'engine-game', '--features', 'app', '--bin', 'arena', '--', ...ARENA_ARGS],
    ['llvm-cov', 'report', '--json', '--output-path', rawPath],
  ];
}

// Runs the plan with an injected runner so tests never build anything.
// Returns { ok: true } or { ok: false, step, status }.
export function runPlan(plan, run) {
  for (const args of plan) {
    const status = run(args);
    if (status !== 0) return { ok: false, step: `cargo ${args.join(' ')}`, status };
  }
  return { ok: true };
}

function realRunner(args) {
  return spawnSync('cargo', args, { cwd: ROOT, stdio: 'inherit' }).status;
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (err) {
    console.error(`wiring-cov: cannot read ${path}: ${err.message}`);
    process.exit(2);
  }
}

function argAfter(argv, flag) {
  const i = argv.indexOf(flag);
  return i >= 0 && i + 1 < argv.length ? argv[i + 1] : null;
}

function atRoot(p) {
  return isAbsolute(p) ? p : join(ROOT, p);
}

function bad(message) {
  console.error(`wiring-cov: ${message}`);
  process.exit(2);
}

export function main(argv = process.argv.slice(2)) {
  if (argv.includes('--help') || argv.includes('-h')) {
    console.log('usage: node tools/wiring-cov.mjs [--out <report.json>] [--all | --crates a,b] [--from-json <export.json>]');
    return;
  }
  const fromJson = argAfter(argv, '--from-json');
  const outPath = argAfter(argv, '--out') ?? REPORT_REL;
  const crateArg = argAfter(argv, '--crates');
  if (argv.includes('--all') && crateArg) bad('--all and --crates exclude each other');
  if (argv.includes('--crates') && !crateArg) bad('--crates needs a comma list, e.g. engine-render,engine-game');
  if (argv.includes('--from-json') && !fromJson) bad('--from-json needs a file');
  const scope = argv.includes('--all') ? null : crateArg ? crateArg.split(',').map((c) => c.trim()).filter(Boolean) : ROW_CRATES;
  let raw;
  if (fromJson) {
    raw = readJson(atRoot(fromJson));
  } else {
    if (!hasLlvmCov()) {
      console.error(`wiring-cov: cargo-llvm-cov not installed: ${INSTALL_LINE}`);
      process.exit(2);
    }
    mkdirSync(dirname(atRoot(RAW_REL)), { recursive: true });
    const res = runPlan(cargoPlan(atRoot(RAW_REL)), realRunner);
    if (!res.ok) {
      console.error(`wiring-cov: step failed (status ${res.status}): ${res.step}`);
      process.exit(1);
    }
    raw = readJson(atRoot(RAW_REL));
  }
  const { crates, totals } = analyze(raw, { crates: scope });
  const report = {
    generated: 'wiring-cov',
    scope: scope ?? 'all',
    run: fromJson ? 'from-json' : `arena ${ARENA_ARGS.join(' ')}`,
    note: 'a headless fallback run never reaches the windowed render path: run on a box with a display',
    crates,
    totals,
  };
  mkdirSync(dirname(atRoot(outPath)), { recursive: true });
  writeFileSync(atRoot(outPath), `${JSON.stringify(report, null, 2)}\n`);
  console.log(formatSummary(crates, outPath));
}

if (import.meta.main) {
  main();
}

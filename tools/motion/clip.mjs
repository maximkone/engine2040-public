#!/usr/bin/env node
// One command: a text prompt in, a combat motion pack out.
//
//   node tools/motion/clip.mjs --prompt "A person raises a sword and brings it down in a strong overhead strike." \
//       --name slash_over --seed 71235 [--seconds 2] [--install] [--allow-slip]
//
// Steps: gen (Kimodo venv, from the cached text embedding) -> bake (tools/motion_bake)
// -> read the receipt -> foot-slip gate -> with --install, copy the pack into
// content/fighters/packs/clips/ and fixtures/clips/combat/ and print the PROVENANCE rows.
//
// The Kimodo side is kimodo_gen.py, run by the venv python at KIMODO_HOME
// (default C:\forge-media\kimodo). The pure helpers are exported for clip.test.mjs.

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const ROOT = path.resolve(HERE, '..', '..');

export const DEFAULT_KIMODO_HOME = 'C:\\forge-media\\kimodo';
export const SLIP_MAX = 0.08; // m/s, average foot speed; over this fails the gate
export const NAME_RE = /^[a-z][a-z0-9_]{1,40}$/;
export const SECONDS_MIN = 0.5;
export const SECONDS_MAX = 10;
export const PROMPT_MAX = 500;

const USAGE = `usage: node tools/motion/clip.mjs --prompt "<text>" --name <name> --seed <int>
         [--seconds <s>] [--samples <n>] [--pick <k>] [--install] [--allow-slip]
         [--slip-max <m/s>] [--kimodo-home <dir>]

  --prompt    the motion in words; must already be in the Kimodo embedding cache
  --name      clip name, lower case letters, digits, underscore (e.g. slash_over)
  --seed      integer seed; the same prompt, seed and seconds give the same bytes
  --seconds   clip length, default 2.0
  --samples   candidates drawn from the seed (a batch), 1..16, default 1
  --pick      which candidate to keep, 0-based, default 0 (shipped slash_over: --samples 4 --pick 1)
  --install   copy the pack into content/fighters/packs/clips/ and fixtures/clips/combat/
  --allow-slip  keep a clip that fails the foot-slip gate (the numbers still print)
  --slip-max  gate in m/s, default ${SLIP_MAX}
  --kimodo-home  default $KIMODO_HOME or ${DEFAULT_KIMODO_HOME}`;

const FLAGS_BOOL = new Set(['install', 'allow-slip', 'help']);
const FLAGS_VALUE = new Set(['prompt', 'name', 'seed', 'seconds', 'samples', 'pick', 'slip-max', 'kimodo-home']);

// Parse argv (without node and script). Returns a plain options object or throws.
export function parseArgs(argv) {
  const raw = {};
  for (let i = 0; i < argv.length; i++) {
    const tok = argv[i];
    if (!tok.startsWith('--')) throw new Error(`unexpected argument ${JSON.stringify(tok)}`);
    let key = tok.slice(2);
    let value;
    const eq = key.indexOf('=');
    if (eq >= 0) {
      value = key.slice(eq + 1);
      key = key.slice(0, eq);
    }
    if (FLAGS_BOOL.has(key)) {
      if (value !== undefined) throw new Error(`--${key} takes no value`);
      raw[key] = true;
      continue;
    }
    if (!FLAGS_VALUE.has(key)) throw new Error(`unknown flag --${key}`);
    if (value === undefined) {
      value = argv[i + 1];
      if (value === undefined || value.startsWith('--')) throw new Error(`--${key} needs a value`);
      i++;
    }
    raw[key] = value;
  }
  if (raw.help) return { help: true };
  for (const need of ['prompt', 'name', 'seed']) {
    if (raw[need] === undefined) throw new Error(`missing --${need}\n${USAGE}`);
  }
  const prompt = raw.prompt.trim();
  if (!prompt) throw new Error('--prompt is empty; describe the motion in words');
  if (prompt.length > PROMPT_MAX) throw new Error(`--prompt is ${prompt.length} characters, past the ${PROMPT_MAX} cap`);
  if (!NAME_RE.test(raw.name)) {
    throw new Error(`--name ${JSON.stringify(raw.name)} must match ${NAME_RE} (lower case, digits, underscore)`);
  }
  if (!/^\d+$/.test(raw.seed) || Number(raw.seed) > 2147483647) {
    throw new Error(`--seed ${JSON.stringify(raw.seed)} must be a whole number 0..2147483647`);
  }
  const seconds = raw.seconds === undefined ? 2.0 : Number(raw.seconds);
  if (!Number.isFinite(seconds) || seconds < SECONDS_MIN || seconds > SECONDS_MAX) {
    throw new Error(`--seconds ${raw.seconds} must be between ${SECONDS_MIN} and ${SECONDS_MAX}`);
  }
  const samples = raw.samples === undefined ? 1 : Number(raw.samples);
  if (!Number.isInteger(samples) || samples < 1 || samples > 16) {
    throw new Error(`--samples ${raw.samples} must be a whole number 1..16`);
  }
  const pick = raw.pick === undefined ? 0 : Number(raw.pick);
  if (!Number.isInteger(pick) || pick < 0 || pick >= samples) {
    throw new Error(`--pick ${raw.pick} must be a whole number 0..${samples - 1} for --samples ${samples}`);
  }
  const slipMax = raw['slip-max'] === undefined ? SLIP_MAX : Number(raw['slip-max']);
  if (!Number.isFinite(slipMax) || slipMax <= 0) throw new Error(`--slip-max ${raw['slip-max']} must be a positive number`);
  return {
    prompt,
    name: raw.name,
    seed: Number(raw.seed),
    seconds,
    samples,
    pick,
    install: raw.install === true,
    allowSlip: raw['allow-slip'] === true,
    slipMax,
    kimodoHome: raw['kimodo-home'] || undefined,
  };
}

// The venv python that runs kimodo_gen.py.
export function kimodoPython(home, platform = process.platform) {
  return platform === 'win32'
    ? path.join(home, 'venv', 'Scripts', 'python.exe')
    : path.join(home, 'venv', 'bin', 'python');
}

// Arguments for kimodo_gen.py.
export function genArgs({ prompt, seed, seconds, samples = 1, pick = 0, outDir, stem }) {
  return [
    path.join(HERE, 'kimodo_gen.py'),
    '--prompt', prompt,
    '--seed', String(seed),
    '--seconds', String(seconds),
    '--samples', String(samples),
    '--pick', String(pick),
    '--out-dir', outDir,
    '--stem', stem,
  ];
}

// The result line kimodo_gen.py prints last on stdout. Null when there is none.
export function parseGenResult(stdout) {
  const lines = String(stdout || '').split(/\r?\n/).map((l) => l.trim()).filter((l) => l.startsWith('{'));
  for (let i = lines.length - 1; i >= 0; i--) {
    try {
      const obj = JSON.parse(lines[i]);
      if (obj && typeof obj === 'object' && 'ok' in obj) return obj;
    } catch {
      // not JSON, keep looking
    }
  }
  return null;
}

// Foot-slip gate over a bake receipt. Fails when either foot's average speed is over slipMax.
export function gateReceipt(receipt, { slipMax = SLIP_MAX, allowSlip = false } = {}) {
  const slip = receipt && receipt.avg_slip;
  const left = Number(slip && slip.leftfoot);
  const right = Number(slip && slip.rightfoot);
  if (!Number.isFinite(left) || !Number.isFinite(right)) {
    return { ok: false, left, right, slipMax, message: 'receipt has no avg_slip numbers' };
  }
  const over = left > slipMax || right > slipMax;
  const numbers = `left ${left.toFixed(6)} m/s, right ${right.toFixed(6)} m/s (gate ${slipMax} m/s)`;
  if (!over) return { ok: true, left, right, slipMax, message: `slip gate pass: ${numbers}` };
  if (allowSlip) {
    return { ok: true, left, right, slipMax, message: `slip gate FAILED, kept by --allow-slip: ${numbers}` };
  }
  return {
    ok: false,
    left,
    right,
    slipMax,
    message: `foot slip over the gate: ${numbers}. The clip slides on the ground; ` +
      're-roll the seed or prompt, or pass --allow-slip to keep it with the numbers visible.',
  };
}

// The stdout line motion_bake prints on success. Null when it does not match.
export function parseBakeLine(stdout) {
  const m = /motion_bake: clip=(\S+) seed=(\S+) source=(\S+) source_sha256=([0-9a-f]+) ticks=(\d+) qclip_hash=(0x[0-9a-f]+) avg_slip=([\d.]+)\/([\d.]+) packed=(\d+) retarget_digest=(0x[0-9a-f]+) out=(\S+) receipt=(\S+)/.exec(
    String(stdout || ''),
  );
  if (!m) return null;
  return {
    clip: m[1],
    seed: m[2],
    source: m[3],
    sourceSha256: m[4],
    ticks: Number(m[5]),
    qclipHash: m[6],
    avgSlip: [Number(m[7]), Number(m[8])],
    packed: Number(m[9]),
    retargetDigest: m[10],
    out: m[11],
    receipt: m[12],
  };
}

// One row for the PROVENANCE tables. 'fixtures' matches fixtures/clips/combat/PROVENANCE.md
// (with steps); 'content' matches content/fighters/packs/PROVENANCE.md (no steps column).
export function provenanceRow({ name, prompt, seed, steps, ticks, slip }, table = 'fixtures') {
  const l = Number(slip.leftfoot).toFixed(6);
  const r = Number(slip.rightfoot).toFixed(6);
  if (table === 'content') {
    return `| \`${name}.am5p\` | \`${name}\` | ${prompt} | ${seed} | ${ticks} | ${l} / ${r} |`;
  }
  return `| \`${name}.am5p\` | \`${name}\` | ${prompt} | ${seed} | ${steps} | ${ticks} | ${l} / ${r} |`;
}

// The one summary line the command ends with.
export function summaryLine({ name, ticks, slip, bytes, sha256, gate, installed }) {
  const l = Number(slip.leftfoot).toFixed(6);
  const r = Number(slip.rightfoot).toFixed(6);
  return `clip ${name}: ticks=${ticks} slip=${l}/${r} bytes=${bytes} sha256=${sha256} gate=${gate} installed=${installed ? 'yes' : 'no'}`;
}

export function sha256File(file) {
  return createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

class ClipError extends Error {}

function fail(message) {
  throw new ClipError(message);
}

function runOrFail(cmd, args, options, what, fix) {
  const res = spawnSync(cmd, args, options);
  if (res.error) fail(`${what} did not start (${res.error.message}). ${fix}`);
  return res;
}

function main(argv) {
  let opts;
  try {
    opts = parseArgs(argv);
  } catch (err) {
    console.error(`clip: ${err.message}`);
    process.exit(2);
  }
  if (opts.help) {
    console.log(USAGE);
    return 0;
  }

  const home = opts.kimodoHome || process.env.KIMODO_HOME || DEFAULT_KIMODO_HOME;
  const py = kimodoPython(home);
  if (!fs.existsSync(py)) {
    fail(`no Kimodo venv python at ${py}. Fix: set KIMODO_HOME to the folder that holds venv\\ and src\\ (default ${DEFAULT_KIMODO_HOME}).`);
  }

  const outDir = path.join(ROOT, 'target', 'motion', opts.name);
  fs.mkdirSync(outDir, { recursive: true });

  // 1. gen. stderr is shown live so the Kaggle step prints on a cache miss.
  const pyPath = [path.join(home, 'src'), process.env.PYTHONPATH].filter(Boolean).join(path.delimiter);
  const gen = runOrFail(
    py,
    genArgs({ prompt: opts.prompt, seed: opts.seed, seconds: opts.seconds, samples: opts.samples, pick: opts.pick, outDir, stem: opts.name }),
    {
      cwd: ROOT,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'inherit'],
      maxBuffer: 64 * 1024 * 1024,
      env: { ...process.env, KIMODO_HOME: home, PYTHONPATH: pyPath, PYTHONIOENCODING: 'utf-8' },
    },
    'kimodo_gen.py',
    'Fix: see the messages above.',
  );
  const result = parseGenResult(gen.stdout);
  if (gen.status === 2) {
    fail(`prompt is not in the Kimodo embedding cache. Follow the steps printed above (tools/motion/kaggle_encode.py), then run this again.`);
  }
  if (gen.status !== 0 || !result || !result.ok) {
    fail(`kimodo_gen.py failed (exit ${gen.status}). Fix: read the message above; the venv is ${py}.`);
  }
  console.error(`gen: ${result.frames} frames, ${result.steps} steps, seed ${result.seed}, bvh ${result.bvh_sha256.slice(0, 12)}`);

  // 2. bake, with the engine's own tool.
  const pack = path.join(outDir, `${opts.name}.am5p`);
  const receiptPath = path.join(outDir, `${opts.name}.receipt.json`);
  const bake = runOrFail(
    'cargo',
    ['run', '--quiet', '--manifest-path', path.join(ROOT, 'tools', 'motion_bake', 'Cargo.toml'), '--',
      'bake', '--source', result.npz, '--name', opts.name, '--out', pack, '--receipt', receiptPath],
    { cwd: ROOT, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 64 * 1024 * 1024 },
    'cargo run motion_bake',
    'Fix: run `cargo run --manifest-path tools/motion_bake/Cargo.toml -- bake --help` and read the error above.',
  );
  if (bake.status !== 0) {
    fail(`motion_bake failed (exit ${bake.status}). Fix: read the cargo error above; the source is ${result.npz}.`);
  }
  const bakeLine = parseBakeLine(bake.stdout);
  if (!bakeLine) fail('motion_bake printed no receipt line. Fix: check tools/motion_bake/src/main.rs still prints "motion_bake: clip=...".');

  // 3. read the receipt and gate it.
  const receipt = JSON.parse(fs.readFileSync(receiptPath, 'utf8'));
  if (receipt.pack_magic !== 'AM5P') fail(`receipt pack_magic is ${receipt.pack_magic}, want AM5P`);
  const gate = gateReceipt(receipt, { slipMax: opts.slipMax, allowSlip: opts.allowSlip });
  console.error(gate.message);
  if (!gate.ok) {
    fail(`${gate.message}\nNothing was installed. Pack and receipt stay in ${outDir}.`);
  }

  const packBytes = fs.statSync(pack).size;
  const packSha = sha256File(pack);
  const slip = { leftfoot: receipt.avg_slip.leftfoot, rightfoot: receipt.avg_slip.rightfoot };

  // 4. optional install.
  let installed = false;
  if (opts.install) {
    const contentDir = path.join(ROOT, 'content', 'fighters', 'packs', 'clips');
    const fixtureDir = path.join(ROOT, 'fixtures', 'clips', 'combat');
    fs.mkdirSync(contentDir, { recursive: true });
    fs.mkdirSync(fixtureDir, { recursive: true });
    fs.copyFileSync(pack, path.join(contentDir, `${opts.name}.am5p`));
    fs.copyFileSync(pack, path.join(fixtureDir, `${opts.name}.am5p`));
    fs.copyFileSync(receiptPath, path.join(fixtureDir, `${opts.name}.receipt.json`));
    installed = true;
    const row = { name: opts.name, prompt: opts.prompt, seed: opts.seed, steps: result.steps, ticks: receipt.ticks, slip };
    console.log('PROVENANCE row for fixtures/clips/combat/PROVENANCE.md:');
    console.log(provenanceRow(row, 'fixtures'));
    console.log('PROVENANCE row for content/fighters/packs/PROVENANCE.md:');
    console.log(provenanceRow(row, 'content'));
    console.log(`installed: ${path.relative(ROOT, path.join(contentDir, `${opts.name}.am5p`))}, ${path.relative(ROOT, path.join(fixtureDir, `${opts.name}.am5p`))}, ${path.relative(ROOT, path.join(fixtureDir, `${opts.name}.receipt.json`))}`);
  }

  console.log(summaryLine({
    name: opts.name,
    ticks: receipt.ticks,
    slip,
    bytes: packBytes,
    sha256: packSha,
    gate: gate.ok && gate.message.includes('FAILED') ? 'kept' : 'pass',
    installed,
  }));
  console.log(`pack: ${pack}`);
  console.log(`receipt: ${receiptPath}`);
  return 0;
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (err) {
    if (err instanceof ClipError) {
      console.error(`clip: ${err.message}`);
      process.exitCode = 1;
    } else {
      throw err;
    }
  }
}

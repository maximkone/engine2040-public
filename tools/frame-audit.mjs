#!/usr/bin/env node
// frame-audit.mjs - one verdict per captured frame and per law: shadow, material, scale, attach.
//
//   node tools/frame-audit.mjs <capture-dir> [--json] [--frame N] [--quiet]
//
// The window's `view=ok` judges bodies as boxes and said ok on a frame with no shadows, a black
// sword and shield and a spear standing out of a lying body. This reads the picture instead:
// `<capture-dir>/view-audit.jsonl` (written by arena.exe beside every captured PNG: posed parts,
// joints, spears, camera) is projected onto each `frame_*.png` and four laws judge the pixels and
// the numbers together (tools/frame-audit-laws.mjs):
//   shadow    each body stands in a darker footprint than the floor around it
//   material  no drawn part renders near-black and flat
//   scale     blade, shield, spear and stature sit in the kit's real range
//   attach    grip in the hand, shield on the arm, spear held by the living or stuck in its victim
// Output, one line per frame, the reasons under every failing one, then `audit=ok` or
// `audit=FAIL(<laws>)`:
//   frame 2: shadow=FAIL(slot0,slot1) material=FAIL(gladius_blade@0) scale=ok attach=FAIL(spear@1)
// `audit=` is advisory beside the window's `view=`: the laws are tuned on real frames, not yet
// part of the `view=` gate (docs/book/ref-tools.md, frame-audit.mjs).
// Exit: 0 every law ok or not applicable, 1 a law failed, 2 usage, missing file or bad PNG.
// No cargo, no GPU: fs + zlib only.
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { decodePng } from './fighter-count.mjs';
import { LAWS, auditFrame, formatFrame, pixelSlabLaw } from './frame-audit-laws.mjs';

export const AUDIT_FILE = 'view-audit.jsonl';
// Failing frames that print their reasons; the rest print one line (a long capture stays readable).
const MAX_EXPLAINED = 8;
const FRAME_FILE = /^frame_\d+\.png$/;

export function parseRows(text) {
  const rows = [];
  for (const [i, line] of text.split('\n').entries()) {
    if (line.trim() === '') continue;
    try {
      rows.push(JSON.parse(line));
    } catch (err) {
      throw new Error(`${AUDIT_FILE} line ${i + 1}: not JSON (${err.message})`, { cause: err });
    }
  }
  return rows;
}

const frameIndex = (file) => Number((/(\d+)\.png$/.exec(file) || [])[1] ?? NaN);

// A capture with no audit rows (an arena.exe from before the audit): the pixels alone can still
// show a black slab. Every other law reads n/a and the verdict says so.
export function auditPixelsOnly(dir, { only = null } = {}) {
  const na = { status: 'n/a', subjects: [], detail: [], judged: 0 };
  const frames = [];
  for (const file of readdirSync(dir).filter((f) => FRAME_FILE.test(f)).sort()) {
    const index = frameIndex(file);
    if (only !== null && index !== only) continue;
    const material = pixelSlabLaw(decodePng(readFileSync(join(dir, file)), file));
    frames.push({ index, file, shadow: na, material, scale: na, attach: na, camera: na, ok: material.status !== 'FAIL' });
  }
  const failed = frames.some((f) => f.material.status === 'FAIL') ? ['material'] : [];
  return { frames, failed, pixelOnly: true };
}

// Audit every row of one capture dir. Returns { frames: [result], failed: [law] }.
export function auditDir(dir, { only = null } = {}) {
  const path = join(dir, AUDIT_FILE);
  if (!existsSync(path)) {
    if (existsSync(dir) && readdirSync(dir).some((f) => FRAME_FILE.test(f))) return auditPixelsOnly(dir, { only });
    throw new Error(`${dir}: no frame_*.png and no ${AUDIT_FILE}`);
  }
  const rows = parseRows(readFileSync(path, 'utf8'));
  if (rows.length === 0) throw new Error(`${path}: no rows`);
  const frames = [];
  for (const row of rows) {
    const index = frameIndex(row.file);
    if (only !== null && index !== only) continue;
    const png = join(dir, row.file);
    if (!existsSync(png)) throw new Error(`${png}: the audit row names a frame that is not on disk`);
    const img = decodePng(readFileSync(png), row.file);
    frames.push({ index, ...auditFrame(row, img) });
  }
  const failed = LAWS.filter((l) => frames.some((f) => f[l].status === 'FAIL'));
  if (frames.some((f) => f.camera.status === 'FAIL')) failed.push('camera');
  return { frames, failed };
}

export function render({ frames, failed, pixelOnly = false }, { quiet = false } = {}) {
  const out = [];
  let explained = 0;
  for (const f of frames) {
    if (quiet && f.ok) continue;
    out.push(formatFrame(f.index, f));
    if (f.ok) continue;
    explained += 1;
    if (explained === MAX_EXPLAINED + 1) out.push('  (reasons for the rest: --frame N)');
    if (explained > MAX_EXPLAINED) continue;
    for (const law of [...LAWS, 'camera']) {
      if (f[law].status !== 'FAIL') continue;
      for (const d of f[law].detail) out.push(`  ${law} ${d}`);
    }
  }
  const bad = frames.filter((f) => !f.ok).length;
  const note = pixelOnly ? ' (pixel-only: no view-audit.jsonl, capture again with the current arena.exe for shadow, scale and attach)' : '';
  const verdict = failed.length === 0 ? (pixelOnly ? 'ok(material only)' : 'ok') : `FAIL(${failed.join(',')})`;
  out.push(`audit=${verdict} frames=${frames.length} failed_frames=${bad}${note}`);
  return out.join('\n');
}

const HELP = `frame-audit.mjs - per frame, per law verdict on a capture dir
  node tools/frame-audit.mjs <capture-dir> [--json] [--frame N] [--quiet]
  --json     print the full result (every law, every reason) as JSON
  --frame N  audit only frame_00000N.png
  --quiet    print only the failing frames
  Reads <capture-dir>/view-audit.jsonl and frame_*.png. Laws: shadow material scale attach.
  Exit 0 ok, 1 a law failed, 2 usage or a missing file.`;

export function main(argv) {
  const args = argv.slice(2);
  if (args.includes('--help') || args.includes('-h')) {
    console.log(HELP);
    return 0;
  }
  const flags = new Set(['--json', '--quiet']);
  let only = null;
  const dirs = [];
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === '--frame') {
      only = Number(args[++i]);
      if (!Number.isInteger(only) || only < 0) {
        console.error('frame-audit: --frame needs a frame number');
        return 2;
      }
    } else if (flags.has(args[i])) {
      continue;
    } else if (args[i].startsWith('--')) {
      console.error(`frame-audit: unknown flag ${args[i]}\n${HELP}`);
      return 2;
    } else {
      dirs.push(args[i]);
    }
  }
  if (dirs.length !== 1) {
    console.error(`frame-audit: give one capture dir\n${HELP}`);
    return 2;
  }
  let result;
  try {
    result = auditDir(resolve(dirs[0]), { only });
  } catch (err) {
    console.error(`frame-audit: ${err.message}`);
    return 2;
  }
  if (result.frames.length === 0) {
    console.error('frame-audit: no frame matched');
    return 2;
  }
  if (args.includes('--json')) console.log(JSON.stringify(result, null, 1));
  else console.log(render(result, { quiet: args.includes('--quiet') }));
  return result.failed.length === 0 ? 0 : 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = main(process.argv);
}

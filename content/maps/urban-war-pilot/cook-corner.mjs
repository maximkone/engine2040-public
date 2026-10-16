// CT56.b corner cook intake — deterministic, node stdlib only.
// Reads the frozen CT9 kit (src glTFs + report.json + manifest.json) and the
// CT56.a splitter selection (variant.json + corner.json), checks every actor
// against the frozen CT2 cook (file SHA, bytes, LOD0 budget, hull gate), and
// emits corner-cook.json. Fails non-zero on any mismatch. Byte-identical
// output on every run: fixed key order, LF newlines, no timestamps.
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const DIR = dirname(fileURLToPath(import.meta.url));
const KIT = join(DIR, '..');
// CT2 triangle budget for props (content/maps/REPORT.md §3, docs/plan/content.md).
const PROP_MAX_TRIS = 5000;

const fail = (msg) => {
  process.stderr.write(`cook-corner: FAIL ${msg}\n`);
  process.exit(1);
};

const variant = JSON.parse(readFileSync(join(DIR, 'variant.json'), 'utf8'));
const corner = JSON.parse(readFileSync(join(DIR, 'corner.json'), 'utf8'));
if (corner.variant !== variant.variant) fail('corner.variant != variant.variant');
const report = JSON.parse(readFileSync(join(KIT, 'report.json'), 'utf8'));
const manifest = JSON.parse(readFileSync(join(KIT, 'manifest.json'), 'utf8'));
const cookedByAsset = {};
for (const row of report.assets) cookedByAsset[row.asset] = row;

const actors = corner.actors.map((place) => {
  const cooked = cookedByAsset[place.asset];
  if (!cooked) fail(`unknown asset ${place.asset}`);
  const file = `src/${place.asset}.gltf`;
  const ref = manifest.files[file];
  if (!ref) fail(`no manifest row ${file}`);
  const bytes = readFileSync(join(KIT, file));
  const sha = createHash('sha256').update(bytes).digest('hex');
  if (sha !== ref.sha256) fail(`SHA drift ${file}`);
  if (bytes.length !== ref.bytes) fail(`bytes drift ${file}`);
  if (cooked.hulls_contain !== true) fail(`hull escape ${place.asset}`);
  if (cooked.lod0_tris > PROP_MAX_TRIS) fail(`budget breach ${place.asset} ${cooked.lod0_tris}`);
  return {
    name: place.name,
    asset: place.asset,
    file,
    sha256: sha,
    bytes: bytes.length,
    lod0_tris: cooked.lod0_tris,
    cook_digest: cooked.digest,
    hulls_contain: true,
    x: place.x,
    y: place.y,
    z: place.z,
    sx: place.sx,
    sy: place.sy,
    sz: place.sz,
  };
});
const totalLod0 = actors.reduce((sum, a) => sum + a.lod0_tris, 0);
const totalBytes = actors.reduce((sum, a) => sum + a.bytes, 0);
const worst = actors.reduce((top, a) => (a.lod0_tris > top.lod0_tris ? a : top), actors[0]);
const out = {
  corner: corner.corner,
  variant: corner.variant,
  kit: 'CT9 frozen',
  prop_max_tris: PROP_MAX_TRIS,
  actors,
  totals: { actors: actors.length, lod0_tris: totalLod0, src_bytes: totalBytes },
  worst_asset: { asset: worst.asset, lod0_tris: worst.lod0_tris },
  budget_verdict: 'hold',
  note: 'CT56.b splitter: 3 corner actors resolve against the frozen CT9 CT2 cook, budgets hold, no new geometry, no engine hunk',
};
writeFileSync(join(DIR, 'corner-cook.json'), `${JSON.stringify(out, null, 2)}\n`);
process.stdout.write(
  `cook-corner: GREEN actors=${actors.length} lod0=${totalLod0} bytes=${totalBytes} worst=${worst.asset}@${worst.lod0_tris}\n`,
);

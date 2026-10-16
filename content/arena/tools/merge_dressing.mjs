// Staging glue + the one real arena loader: merged placement + dressing.
// LOADER CONTRACT: `loadMergedArena(root?)` reads `placement.json` (11-actor
// parade, ids 1..11) + `dressing.json` (6 dressing actors, ids 101+) under
// root, concatenates in id order, validates (unique ids/names, known assets,
// disjoint XZ footprints, inside +/-99 m, resting y) and returns
// `{ placement, dressing, merged, canonical, sha }`. Throws on any failure
// (CLI wraps to `MERGE-DRESSING FAIL` + exit 1); the CC0 generator
// `gen.mjs` imports this (never the reverse) for its post-write check and
// never writes or deletes `dressing.json` (no-clobber guard lives there).
// The window presenter stages exactly this merged set — this module is what
// it calls, not a side proof. Node stdlib only, no randomness.
//
// Run from the repo root:
//   node .lanes/ct53/content/arena/tools/merge_dressing.mjs
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const KNOWN = new Set([
  "sand_floor",
  "stands",
  "walls",
  "gate_tunnel",
  "torch",
  "banner",
  "crate",
  "debris",
]);

function fail(msg) {
  console.error(`MERGE-DRESSING FAIL: ${msg}`);
  process.exit(1);
}

function load(rel, root = ROOT) {
  let raw;
  try {
    raw = readFileSync(join(root, rel), "utf8");
  } catch (err) {
    fail(`read ${rel}: ${err.message}`);
  }
  let doc;
  try {
    doc = JSON.parse(raw);
  } catch (err) {
    fail(`parse ${rel}: ${err.message}`);
  }
  if (!Array.isArray(doc.actors)) fail(`${rel} has no actors array`);
  return doc.actors;
}

function num(v, what) {
  if (typeof v !== "number" || !Number.isFinite(v)) fail(`${what} is not finite`);
  return v;
}

// The one real loader. Throws (via fail) on any contract breach so both the
// CLI and the `gen.mjs` post-write check share one code path.
export function loadMergedArena(root = ROOT) {
  const placement = load("placement.json", root);
  const dressing = load("dressing.json", root);
  if (placement.length !== 11) fail(`placement actors = ${placement.length}, want 11`);
  if (dressing.length !== 6) fail(`dressing actors = ${dressing.length}, want 6`);
  for (const a of dressing) {
    if (!Number.isInteger(a.id) || a.id < 101) {
      fail(`dressing id ${JSON.stringify(a.id)} out of the 101+ dressing range (parade owns 1..11)`);
    }
  }

  const merged = [...placement, ...dressing].sort((a, b) => a.id - b.id);
  const ids = new Set();
  const names = new Set();
  for (const a of merged) {
    if (!Number.isInteger(a.id)) fail(`actor id not an integer: ${JSON.stringify(a).slice(0, 60)}`);
    if (ids.has(a.id)) fail(`duplicate id ${a.id}`);
    ids.add(a.id);
    if (typeof a.name !== "string" || a.name.length === 0) fail(`actor ${a.id} has no name`);
    if (names.has(a.name)) fail(`duplicate name ${a.name}`);
    names.add(a.name);
    if (!KNOWN.has(a.asset)) fail(`actor ${a.name} has unknown asset ${a.asset}`);
    const x = num(a.x, `${a.name}.x`);
    const y = num(a.y, `${a.name}.y`);
    const z = num(a.z, `${a.name}.z`);
    const sx = num(a.sx, `${a.name}.sx`);
    const sy = num(a.sy, `${a.name}.sy`);
    const sz = num(a.sz, `${a.name}.sz`);
    if (sx <= 0 || sy <= 0 || sz <= 0) fail(`actor ${a.name} has non-positive extent`);
    if (y < 0) fail(`actor ${a.name} floats below y=0`);
    if (Math.abs(x) + sx / 2 > 99 || Math.abs(z) + sz / 2 > 99) {
      fail(`actor ${a.name} leaves the +/-99 m envelope`);
    }
  }

  const fails = [];
  for (let i = 0; i < merged.length; i++) {
    for (let j = i + 1; j < merged.length; j++) {
      const a = merged[i];
      const b = merged[j];
      const overlap =
        a.x - a.sx / 2 < b.x + b.sx / 2 &&
        b.x - b.sx / 2 < a.x + a.sx / 2 &&
        a.z - a.sz / 2 < b.z + b.sz / 2 &&
        b.z - b.sz / 2 < a.z + a.sz / 2;
      if (overlap) fails.push(`${a.name}#${a.id} x ${b.name}#${b.id}`);
    }
  }

  const canonical = JSON.stringify(merged.map((a) => ({
    id: a.id,
    name: a.name,
    asset: a.asset,
    x: a.x,
    y: a.y,
    z: a.z,
    sx: a.sx,
    sy: a.sy,
    sz: a.sz,
  })));
  const sha = createHash("sha256").update(canonical).digest("hex");
  if (fails.length > 0) fail(`overlaps: ${fails.join("; ")}`);
  return { placement, dressing, merged, canonical, sha };
}

// CLI entry: runs only when executed directly (`node merge_dressing.mjs`).
// When imported by `gen.mjs` argv[1] names the importer, so this stays quiet.
if (process.argv[1]?.endsWith("merge_dressing.mjs")) {
  const { merged, sha } = loadMergedArena();
  console.log(`MERGED actors=${merged.length} fails=NONE sha=${sha}`);
}

// buildings_receipt.mjs - verify the ART3 building set (five kinds + the cottage) and print
// its receipts.
//
// Reads content/kam2/buildings/<kind>/{manifest,source}.json and content/kam2/house/,
// checks every atlas hash and size against its source.json, checks one scale for every kind
// (pixels per tile, read back from the render settings), checks the cameo atlas of every
// kind against the style lock (96x72, pitch 35, yaw 45, transparent, one per kind), and
// keeps each kind's atlas plus cameo under the 400 KB cap. Prints:
//
//   KAM2 BUILDINGS OK kinds=5 cells=10 bytes=<n> nativescale=<px>
//   KAM2 HOUSE OK cells=3 atlas=<w>x<h> bytes=<n> cameo=<n>
//   KAM2 SCALE OK kinds=6 scale=<min>..<max> px-per-tile=<n>
//   KAM2 CAMEOS OK kinds=6 cells=<n> bytes=<n>
//
// Usage from the repo root: node tools/kam2/buildings_receipt.mjs
// Exit: 0 all checks pass - 1 a check failed.
import { createHash } from "node:crypto";
import { existsSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

import { decodePng } from "./pack_atlas.mjs";

const BUILDINGS = resolve("content/kam2/buildings");
const HOUSE = resolve("content/kam2/house");
const MAX_BYTES = 400 * 1024;
const MAX_SCALE_SPREAD_PX = 1;
const CAMEOS = { cell: [96, 72], pitch_deg: 35, yaws: [45, 135] };

// The sim's own footprints, from HouseKind::footprint in crates/kam2-sim/src/economy.rs.
const KINDS = [
  { kind: "warehouse", dir: join(BUILDINGS, "warehouse"), footprint: [3, 3], states: ["finished", "half_built"] },
  { kind: "farm", dir: join(BUILDINGS, "farm"), footprint: [2, 2], states: ["finished", "half_built"] },
  { kind: "mill", dir: join(BUILDINGS, "mill"), footprint: [2, 2], states: ["finished", "half_built"] },
  { kind: "bakery", dir: join(BUILDINGS, "bakery"), footprint: [2, 2], states: ["finished", "half_built"] },
  { kind: "woodcutter", dir: join(BUILDINGS, "woodcutter"), footprint: [2, 2], states: ["finished", "half_built"] },
  {
    kind: "cottage",
    dir: HOUSE,
    footprint: [2, 2],
    states: ["finished", "half_built", "damaged"],
  },
];

const errors = [];
const read = (path) => JSON.parse(readFileSync(path, "utf8"));
const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

let buildingCells = 0;
let buildingBytes = 0;
let houseAtlas = null;
let scaleMin = Infinity;
let scaleMax = -Infinity;
const cameoBytes = [];
const cameoIds = new Set();

for (const spec of KINDS) {
  const dir = spec.dir;
  const manifestPath = join(dir, "manifest.json");
  const sourcePath = join(dir, "source.json");
  if (!existsSync(manifestPath) || !existsSync(sourcePath)) {
    errors.push(`${spec.kind}: manifest.json or source.json missing`);
    continue;
  }
  const manifest = read(manifestPath);
  const source = read(sourcePath);
  const atlasPath = join(dir, manifest.atlas);
  if (!existsSync(atlasPath)) {
    errors.push(`${spec.kind}: atlas missing: ${manifest.atlas}`);
    continue;
  }
  const atlasBytes = statSync(atlasPath).size;
  const atlasHash = sha256(atlasPath);
  if (atlasBytes !== source.atlas.bytes) {
    errors.push(`${spec.kind}: atlas bytes ${atlasBytes} != source ${source.atlas.bytes}`);
  }
  if (atlasHash !== source.atlas.sha256) errors.push(`${spec.kind}: atlas sha256 differs from source.json`);
  if (manifest.id !== `house.${spec.kind}`) errors.push(`${spec.kind}: id ${manifest.id} is not house.${spec.kind}`);
  if (manifest.kind !== "building") errors.push(`${spec.kind}: kind is not building`);
  if (JSON.stringify(manifest.footprint) !== JSON.stringify(spec.footprint)) {
    errors.push(`${spec.kind}: footprint ${manifest.footprint} != sim ${spec.footprint}`);
  }
  if (JSON.stringify(manifest.pivot) !== JSON.stringify(source.pivot.px)) {
    errors.push(`${spec.kind}: manifest pivot ${manifest.pivot} != source ${source.pivot.px}`);
  }
  const states = manifest.anims.map((anim) => anim.name).join(",");
  if (states !== spec.states.join(",")) errors.push(`${spec.kind}: states ${states}`);
  manifest.anims.forEach((anim, row) => {
    if (anim.row !== row) errors.push(`${spec.kind}: state ${anim.name} row ${anim.row} != ${row}`);
  });

  // Cell convention: one square cell, the render size, the atlas cell and the source agree.
  const cell = [manifest.tile_w, manifest.tile_h];
  if (cell[0] !== cell[1]) errors.push(`${spec.kind}: cell ${cell} is not square`);
  if (JSON.stringify(cell) !== JSON.stringify(source.atlas.cell)) {
    errors.push(`${spec.kind}: cell ${cell} != source ${source.atlas.cell}`);
  }
  const pad = manifest.padding;
  const rows = spec.states.length;
  const cols = manifest.facing_count;
  const wantAtlas = [cell[0] + pad * 2, (cell[1] + pad * 2) * rows];
  if (JSON.stringify(manifest.atlas_size) !== JSON.stringify(source.atlas.size)) {
    errors.push(`${spec.kind}: atlas_size ${manifest.atlas_size} != source ${source.atlas.size}`);
  }
  if (manifest.atlas_size[0] !== wantAtlas[0] || manifest.atlas_size[1] !== wantAtlas[1] || cols !== 1) {
    errors.push(`${spec.kind}: atlas ${manifest.atlas_size} is not ${cols} x ${rows} cells of ${cell} + ${pad} px`);
  }
  if (source.render.size !== cell[0]) errors.push(`${spec.kind}: render size ${source.render.size} != cell ${cell[0]}`);

  // Scale: pixels per tile read back from the render itself (cell px / world span).
  const pxPerTile = cell[0] / source.render.ortho_scale_world_units;
  const claimed = source.scale.used_px_per_kit_cell;
  if (Math.abs(pxPerTile - claimed) > 0.05) {
    errors.push(`${spec.kind}: render gives ${pxPerTile.toFixed(3)} px per tile, source says ${claimed}`);
  }
  scaleMin = Math.min(scaleMin, pxPerTile);
  scaleMax = Math.max(scaleMax, pxPerTile);

  // Cameo: one per kind, style-lock angles, 96x72 cell, transparent, inside the byte cap.
  const cameo = manifest.cameo;
  if (!cameo || typeof cameo !== "object") {
    errors.push(`${spec.kind}: manifest has no cameo object`);
  } else {
    if (cameo.id !== `${manifest.id}.cameo`) errors.push(`${spec.kind}: cameo id ${cameo.id}`);
    if (JSON.stringify(cameo.cell) !== JSON.stringify(CAMEOS.cell)) {
      errors.push(`${spec.kind}: cameo cell ${cameo.cell} != ${CAMEOS.cell}`);
    }
    if (cameo.padding !== pad) errors.push(`${spec.kind}: cameo padding ${cameo.padding} != ${pad}`);
    const cameoPath = join(dir, cameo.atlas);
    if (!existsSync(cameoPath)) {
      errors.push(`${spec.kind}: cameo missing: ${cameo.atlas}`);
    } else {
      const bytes = statSync(cameoPath).size;
      const hash = sha256(cameoPath);
      const decoded = decodePng(readFileSync(cameoPath), cameoPath);
      const want = [CAMEOS.cell[0] + pad * 2, CAMEOS.cell[1] + pad * 2];
      if (decoded.width !== want[0] || decoded.height !== want[1]) {
        errors.push(`${spec.kind}: cameo PNG ${decoded.width}x${decoded.height} != ${want}`);
      }
      if (JSON.stringify(cameo.size) !== JSON.stringify(want)) {
        errors.push(`${spec.kind}: cameo size ${cameo.size} != ${want}`);
      }
      if (bytes !== source.cameo.bytes) errors.push(`${spec.kind}: cameo bytes ${bytes} != source ${source.cameo.bytes}`);
      if (hash !== source.cameo.sha256) errors.push(`${spec.kind}: cameo sha256 differs from source.json`);
      if (source.cameo.pitch_deg !== CAMEOS.pitch_deg) {
        errors.push(`${spec.kind}: cameo pitch ${source.cameo.pitch_deg} != ${CAMEOS.pitch_deg}`);
      }
      if (!CAMEOS.yaws.includes(source.cameo.yaw_deg)) {
        errors.push(`${spec.kind}: cameo yaw ${source.cameo.yaw_deg} is not 45 deg off S`);
      }
      if (source.cameo.transparent !== true) errors.push(`${spec.kind}: cameo not marked transparent`);
      for (const [x, y] of [
        [pad, pad],
        [want[0] - pad - 1, pad],
        [pad, want[1] - pad - 1],
        [want[0] - pad - 1, want[1] - pad - 1],
      ]) {
        const alpha = decoded.pixels[(y * decoded.width + x) * 4 + 3];
        if (alpha !== 0) errors.push(`${spec.kind}: cameo pixel ${x},${y} alpha ${alpha} is not transparent`);
      }
      if (atlasBytes + bytes >= MAX_BYTES) {
        errors.push(`${spec.kind}: atlas+cameo ${atlasBytes + bytes} bytes is over the 400 KB cap`);
      }
      cameoBytes.push(bytes);
      cameoIds.add(cameo.id);
    }
  }

  if (spec.kind === "cottage") {
    houseAtlas = { cells: spec.states.length, size: manifest.atlas_size, bytes: atlasBytes };
  } else {
    buildingCells += manifest.anims.reduce(
      (sum, anim) => sum + anim.frames * (anim.facings ?? manifest.facing_count),
      0,
    );
    buildingBytes += atlasBytes;
  }
}

const spread = scaleMax - scaleMin;
if (!(spread <= MAX_SCALE_SPREAD_PX)) {
  errors.push(`scale spread ${spread.toFixed(3)} px per tile is over ${MAX_SCALE_SPREAD_PX} px`);
}
if (cameoIds.size !== KINDS.length) errors.push(`cameos ${cameoIds.size} != kinds ${KINDS.length}`);

if (errors.length) {
  for (const error of errors) process.stderr.write(`E_BUILDINGS ${error}\n`);
  process.exit(1);
}
const scale = (value) => value.toFixed(3);
process.stdout.write(
  `KAM2 BUILDINGS OK kinds=${KINDS.length - 1} cells=${buildingCells} bytes=${buildingBytes} ` +
    `nativescale=${Number(scaleMin.toFixed(2))}\n`,
);
process.stdout.write(
  `KAM2 HOUSE OK cells=${houseAtlas.cells} atlas=${houseAtlas.size[0]}x${houseAtlas.size[1]} ` +
    `bytes=${houseAtlas.bytes} cameo=${cameoBytes[cameoBytes.length - 1]}\n`,
);
process.stdout.write(
  `KAM2 SCALE OK kinds=${KINDS.length} scale=${scale(scaleMin)}..${scale(scaleMax)} ` +
    `px-per-tile=${Number(scaleMin.toFixed(2))}\n`,
);
process.stdout.write(
  `KAM2 CAMEOS OK kinds=${cameoIds.size} cells=${cameoIds.size} bytes=${cameoBytes.reduce((a, b) => a + b, 0)}\n`,
);

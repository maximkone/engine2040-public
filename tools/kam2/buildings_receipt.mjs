// buildings_receipt.mjs - verify the ART3 building atlases and print their receipt.
//
// Reads content/kam2/buildings/<kind>/{manifest,source}.json, checks the atlas hash and
// size against source.json, checks one scale for every kind, then prints:
//
//   KAM2 BUILDINGS OK kinds=<n> cells=<n> bytes=<n> nativescale=<px>
//
// Usage from the repo root: node tools/kam2/buildings_receipt.mjs
// Exit: 0 all checks pass - 1 a check failed.
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

const ROOT = resolve("content/kam2/buildings");
const FOOTPRINTS = { warehouse: [3, 3], farm: [2, 2], mill: [2, 2], bakery: [2, 2], woodcutter: [2, 2] };
const MAX_BYTES = 400 * 1024;

const errors = [];
const kinds = readdirSync(ROOT, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .sort();

let cells = 0;
let bytes = 0;
let nativescale = null;

for (const kind of kinds) {
  const dir = join(ROOT, kind);
  const manifestPath = join(dir, "manifest.json");
  const sourcePath = join(dir, "source.json");
  if (!existsSync(manifestPath) || !existsSync(sourcePath)) {
    errors.push(`${kind}: manifest.json or source.json missing`);
    continue;
  }
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const source = JSON.parse(readFileSync(sourcePath, "utf8"));
  const atlasPath = join(dir, manifest.atlas);
  if (!existsSync(atlasPath)) {
    errors.push(`${kind}: atlas missing: ${manifest.atlas}`);
    continue;
  }
  const atlasBytes = statSync(atlasPath).size;
  const atlasHash = createHash("sha256").update(readFileSync(atlasPath)).digest("hex");
  if (atlasBytes !== source.atlas.bytes) errors.push(`${kind}: atlas bytes ${atlasBytes} != source ${source.atlas.bytes}`);
  if (atlasHash !== source.atlas.sha256) errors.push(`${kind}: atlas sha256 differs from source.json`);
  if (atlasBytes >= MAX_BYTES) errors.push(`${kind}: atlas ${atlasBytes} bytes is over the 400 KB cap`);
  if (!manifest.id.startsWith("house.")) errors.push(`${kind}: id ${manifest.id} is not house.<kind>`);
  if (manifest.kind !== "building") errors.push(`${kind}: kind is not building`);
  if (JSON.stringify(manifest.footprint) !== JSON.stringify(FOOTPRINTS[kind])) {
    errors.push(`${kind}: footprint ${manifest.footprint} != sim ${FOOTPRINTS[kind]}`);
  }
  if (JSON.stringify(manifest.pivot) !== JSON.stringify(source.pivot.px)) {
    errors.push(`${kind}: manifest pivot ${manifest.pivot} != source ${source.pivot.px}`);
  }
  const states = manifest.anims.map((anim) => anim.name).join(",");
  if (states !== "finished,half_built") errors.push(`${kind}: states ${states}`);
  const px = source.scale.used_px_per_kit_cell;
  if (nativescale === null) nativescale = px;
  else if (nativescale !== px) errors.push(`${kind}: scale ${px} != ${nativescale}`);
  cells += manifest.anims.reduce((sum, anim) => sum + anim.frames * (anim.facings ?? manifest.facing_count), 0);
  bytes += atlasBytes;
}

if (kinds.length !== Object.keys(FOOTPRINTS).length) {
  errors.push(`kinds ${kinds.length} != ${Object.keys(FOOTPRINTS).length}`);
}
for (const kind of Object.keys(FOOTPRINTS)) {
  if (!kinds.includes(kind)) errors.push(`${kind}: no folder`);
}
const expectedBytes = kinds.reduce((sum, kind) => {
  const source = JSON.parse(readFileSync(join(ROOT, kind, "source.json"), "utf8"));
  return sum + source.atlas.bytes;
}, 0);
if (bytes !== expectedBytes) errors.push(`total ${bytes} != source total ${expectedBytes}`);

if (errors.length) {
  for (const error of errors) process.stderr.write(`E_BUILDINGS ${error}\n`);
  process.exit(1);
}
process.stdout.write(
  `KAM2 BUILDINGS OK kinds=${kinds.length} cells=${cells} bytes=${bytes} nativescale=${nativescale}\n`,
);

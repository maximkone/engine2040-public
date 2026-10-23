// Read the JSON chunk of a GLB and report skin and material completeness.
// Magenta in the renderer means a primitive with no material or skin, so a
// fighter with every primitive materialled and one skin cannot shade magenta.
// Usage: node check.mjs <fighter.glb> [more.glb ...]
import { readFileSync } from "node:fs";

const files = process.argv.slice(2);
if (files.length === 0) {
  console.error("usage: node check.mjs <fighter.glb> [more.glb ...]");
  process.exit(2);
}

let bad = 0;
for (const f of files) {
  const buf = readFileSync(f);
  if (buf.readUInt32LE(0) !== 0x46546c67) {
    console.log(`FAIL ${f}: not a GLB`);
    bad++;
    continue;
  }
  const jsonLen = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + jsonLen).toString("utf8").replace(/\0+$/, ""));
  const meshes = json.meshes ?? [];
  const prims = meshes.flatMap((m) => m.primitives ?? []);
  const mats = json.materials ?? [];
  const skins = json.skins ?? [];
  const nodes = json.nodes ?? [];
  const noMat = prims.filter((p) => p.material === undefined).length;
  const texless = prims.filter((p) => {
    if (p.material === undefined) return true;
    const m = mats[p.material];
    return !m?.pbrMetallicRoughness?.baseColorTexture;
  }).length;
  const skinnedNodes = nodes.filter((n) => n.skin !== undefined).length;
  const joints = skins.reduce((n, s) => n + (s.joints?.length ?? 0), 0);
  const ok = prims.length > 0 && noMat === 0 && skinnedNodes > 0 && joints > 0;
  if (!ok) bad++;
  console.log(
    `${ok ? "PASS" : "FAIL"} ${f.split(/[\\/]/).pop()}: meshes ${meshes.length} prims ${prims.length} ` +
      `materials ${mats.length} primsWithoutMaterial ${noMat} primsWithoutBaseColorTex ${texless} ` +
      `skins ${skins.length} joints ${joints} skinnedNodes ${skinnedNodes}`,
  );
}
console.log(bad === 0 ? "PASS glb: every fighter has a material and a skin" : `FAIL glb: ${bad} fighter(s) incomplete`);
process.exit(bad === 0 ? 0 : 1);

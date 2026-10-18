// Run: node --test tools/pilot-scout.test.mjs
// CT78: every number in content/maps/PILOT-SCOUT.md equals the pilot files and CT9 textures it comes from.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import test from "node:test";
import { fileDigest, imageSize, readJson, REPO, rgba8MipBytes } from "./content-lib.mjs";

const MAPS = join(REPO, "content", "maps");
const PILOT = join(MAPS, "urban-war-pilot");
const doc = readFileSync(join(MAPS, "PILOT-SCOUT.md"), "utf8");
const corner = readJson(join(PILOT, "corner.json"));
const cook = readJson(join(PILOT, "corner-cook.json"));
const district = readJson(join(MAPS, "placement_district.json")).actors;

test("the scout names the variant, the corner and its three actors", () => {
  assert.equal(readJson(join(PILOT, "variant.json")).variant, "district-corner");
  assert.equal(corner.actors.length, 3);
  for (const text of [corner.variant, corner.corner, ...corner.actors.map((a) => `\`${a.name}\` (${a.asset})`)]) assert.ok(doc.includes(text), text);
});

test("the corner actors keep the coordinates and sizes of the district placement", () => {
  for (const a of corner.actors) {
    const same = district.find((d) => d.name === a.name);
    for (const key of ["asset", "x", "y", "z", "sx", "sy", "sz"]) assert.equal(a[key], same[key], `${a.name}.${key}`);
  }
});

test("source bytes and LOD0 triangles are the corner cook's totals and the files on disk", () => {
  const files = cook.actors.map((a) => fileDigest(join(MAPS, a.file)));
  assert.deepEqual(files.map((f) => f.bytes), cook.actors.map((a) => a.bytes), "the cook receipt matches the disk");
  const bytes = files.reduce((s, f) => s + f.bytes, 0);
  const tris = cook.actors.reduce((s, a) => s + a.lod0_tris, 0);
  assert.ok(cook.actors.every((a) => a.hulls_contain && a.lod0_tris <= cook.prop_max_tris));
  assert.ok(doc.includes(`| glTF source bytes, 3 actors | ${bytes} |`), `${bytes} bytes`);
  assert.ok(doc.includes(`| LOD0 triangles, 3 actors | ${tris} |`), `${tris} tris`);
});

test("the VRAM guess is the RGBA8 plus mips bound of the three albedos and the two shared data maps", () => {
  const tex = readFileSync(join(MAPS, "tools", "placement_data.rs"), "utf8");
  const albedos = new Set(corner.actors.map((a) => new RegExp(`\\("${a.asset}", "([a-z_]+)"\\)`).exec(tex)[1]));
  assert.deepEqual([...albedos].sort(), ["canvas_albedo", "plaster_albedo", "stone_albedo"]);
  const bound = (name) => {
    const size = imageSize(join(MAPS, "textures", `${name}.png`));
    return rgba8MipBytes(size.w, size.h);
  };
  const albedo = [...albedos].map(bound);
  const data = ["flat_normal", "metal_roughness"].map(bound);
  const total = [...albedo, ...data].reduce((s, n) => s + n, 0);
  assert.ok(albedo.every((n) => n === albedo[0]) && data.every((n) => n === data[0]), "512 albedos and 256 data maps");
  assert.ok(doc.includes(`3 x ${albedo[0]} |`) && doc.includes(`2 x ${data[0]} |`));
  assert.ok(doc.includes(`${total} bytes (${(total / 1048576).toFixed(2)} MiB)`), `${total}`);
});

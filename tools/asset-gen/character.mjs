// One command, one character: concept -> base mesh -> our style -> the 30 m
// gate -> our 22-bone rig. The gate has teeth: a silhouette that does not
// read stops the chain and says why.
//
//   node tools/asset-gen/character.mjs --concept content/generated/concepts/ashkin_gunner_02.jpg \
//     --name ashkin_gunner --faction ashkin --out content/generated/characters/ashkin_gunner
//
// The steps are the tools in this repo, called in order:
//   tools/asset-gen/sf3d.mjs            the one local 3D lane
//   tools/blender/style_cook.py         our palette, our budget, our LODs
//   tools/blender/mesh_silhouette.py    the 30 m read test
//   tools/blender/rig22.py              our 22-bone template
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
function fail(msg) {
  console.error(`character: ${msg}`);
  process.exit(1);
}

const concept = arg("concept");
const name = arg("name");
const faction = arg("faction", "ashkin");
const outDir = resolve(ROOT, arg("out", `content/generated/characters/${name || "unnamed"}`));
const tris = arg("tris", "2000");
const texture = arg("texture", "256");
if (!concept || !name) fail("--concept <image> and --name <id> are required");

const conceptPath = resolve(ROOT, concept);
if (!existsSync(conceptPath)) fail(`no concept image at ${conceptPath}`);

function blender() {
  if (process.env.BLENDER && existsSync(process.env.BLENDER)) return process.env.BLENDER;
  const roots = [
    "C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe",
    "C:\\Program Files\\Blender Foundation\\Blender 5.0\\blender.exe",
    "C:\\Program Files\\Blender Foundation\\Blender 4.2\\blender.exe",
  ];
  for (const candidate of roots) if (existsSync(candidate)) return candidate;
  fail("no Blender found: set BLENDER to the blender.exe path");
}
const BLENDER = blender();

// The gate band, calibrated on the six blockouts: the two heavies that read
// as rectangles scored 0.7443 and 0.7519, the readable ones 0.5283 to 0.6604.
const FILL_MIN = 0.40;
const FILL_MAX = 0.70;

mkdirSync(outDir, { recursive: true });
const meshDir = join(outDir, "mesh");
const cookDir = join(outDir, "cook");
const gateDir = join(outDir, "gate");
const rigDir = join(outDir, "rig");

function run(label, command, args) {
  console.log(`-> ${label}`);
  const started = Date.now();
  const output = execFileSync(command, args, {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
    timeout: 4 * 60 * 60 * 1000,
  });
  const seconds = ((Date.now() - started) / 1000).toFixed(1);
  console.log(`   done in ${seconds}s`);
  return { output, seconds: Number(seconds) };
}

// 1. the base mesh, from the one local 3D lane
const mesh = run("base mesh (sf3d)", process.execPath, [
  join(ROOT, "tools", "asset-gen", "sf3d.mjs"),
  "--image", conceptPath, "--out", meshDir, "--texture", texture,
]);
const baseMesh = join(meshDir, "mesh.glb");
if (!existsSync(baseMesh)) fail(`sf3d wrote no mesh.glb in ${meshDir}`);

// 2. our style: palette, budget, atlas, AO, LODs
const cook = run("style cook", BLENDER, [
  "-b", "--factory-startup", "--python-exit-code", "1",
  "-P", join(ROOT, "tools", "blender", "style_cook.py"),
  "--", baseMesh, cookDir, "--faction", faction, "--tris", tris,
]);
const cookJson = JSON.parse(readFileSync(join(cookDir, "cook.json"), "utf8"));

// 3. the gate
const gate = run("30 m gate", BLENDER, [
  "-b", "--factory-startup", "--python-exit-code", "1",
  "-P", join(ROOT, "tools", "blender", "mesh_silhouette.py"),
  "--", join(cookDir, "lod0.glb"), gateDir,
]);
const silhouette = JSON.parse(readFileSync(join(gateDir, "silhouette.json"), "utf8"));

const fill = silhouette.bbox_fill;
const gateVerdict = fill >= FILL_MIN && fill <= FILL_MAX
  ? "pass"
  : (fill > FILL_MAX ? "fail_too_solid" : "fail_too_sparse");

// 4. our rig
const rig = run("22-bone rig", BLENDER, [
  "-b", "--factory-startup", "--python-exit-code", "1",
  "-P", join(ROOT, "tools", "blender", "rig22.py"),
  "--", join(cookDir, "lod0.glb"), join(rigDir, "rigged.glb"),
]);
const rigMetrics = JSON.parse(rig.output.trim().split("\n").pop());

const character = {
  name,
  faction,
  concept: conceptPath,
  out: outDir,
  gate: {
    verdict: gateVerdict,
    bbox_fill: fill,
    fill_band: [FILL_MIN, FILL_MAX],
    height_30m_px: silhouette.height_30m_px,
    fill_30m: silhouette.fill_30m,
    triangles: silhouette.triangles,
    note: "a silhouette passes when it reads at 90 pixels tall: faction first, class second",
  },
  cook: {
    lods: cookJson.lods,
    atlas: cookJson.atlas,
    ao: cookJson.ao,
    scale_applied: cookJson.scale_applied,
    source_triangles: cookJson.source_triangles,
  },
  rig: {
    bones: rigMetrics.bones,
    weighted_vertices: rigMetrics.weighted_vertices,
    vertices: rigMetrics.vertices,
    distance_stats: rigMetrics.distance_stats,
    landmarks: rigMetrics.landmarks,
  },
  seconds: { mesh: mesh.seconds, cook: cook.seconds, gate: gate.seconds, rig: rig.seconds },
  generated_at: new Date().toISOString(),
};
writeFileSync(join(outDir, "character.json"), JSON.stringify(character, null, 2) + "\n", "utf8");

console.log(`\nGATE ${gateVerdict}  fill ${fill} (band ${FILL_MIN}-${FILL_MAX})  ` +
  `${silhouette.height_30m_px} px at 30 m  ${cookJson.lods[0].triangles} tris at LOD0`);
console.log(`WROTE ${join(outDir, "character.json")}`);
if (gateVerdict !== "pass") {
  console.error(`character: the silhouette does not read (${gateVerdict}). ` +
    `Look at ${join(gateDir, "silhouette_30m.png")} before painting anything.`);
  process.exit(3);
}

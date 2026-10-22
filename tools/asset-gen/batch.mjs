// The whole sheet through the lane, one character at a time.
//
//   node tools/asset-gen/batch.mjs --help                 usage, no work
//   node tools/asset-gen/batch.mjs --dry-run              the plan; no work, no writes
//   node tools/asset-gen/batch.mjs --concepts content/generated/concepts --out content/generated/characters --yes
//
// It calls tools/asset-gen/character.mjs for each concept, so the machine is
// the same one, and it writes a summary table: the gate verdict, the fill,
// the triangle count and the rig coverage for every class. Nothing runs
// without --yes: the plan prints and the exit code is 2.
//
//   --mesh-dir <dir>   use the base meshes already in <dir>/<name>/0/mesh.glb
//   --only <id>        one class
//   --no-mesh          do not generate a base mesh, use what is there
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));

const USAGE = `usage: node tools/asset-gen/batch.mjs [--concepts <dir>] [--out <dir>] [--mesh-dir <dir>]
         [--only <id>] [--no-mesh] [--skip-existing] [--dry-run | --yes]

  Runs tools/asset-gen/character.mjs for each concept picture: a base mesh (sf3d),
  the Blender cook, the 30 m gate and the 22-bone rig, then writes batch.json and
  batch.md under --out. This script calls no paid service, but each concept is
  hours of local Blender and sf3d work and rewrites its folder under --out.
  Nothing runs without --yes.

  --concepts <dir>   default content/generated/concepts
  --out <dir>        default content/generated/characters
  --mesh-dir <dir>   default content/generated: reuse <dir>/gunner_base_<id>/0/mesh.glb
                     or <dir>/base_<id>/0/mesh.glb when they exist
  --only <id>        one class
  --no-mesh          skip a class that has no base mesh to reuse
  --skip-existing    skip a class whose character.json is already there
  --dry-run          print the plan; no work, no writes; exit 0
  --yes              run the plan; without it the plan prints and the exit code is 2
  --help             this text; exit 0`;

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const has = (name) => process.argv.includes(`--${name}`);

// Help is answered before anything else is read or made.
if (has("help")) {
  console.log(USAGE);
  process.exit(0);
}

const conceptsDir = resolve(ROOT, arg("concepts", "content/generated/concepts"));
const outRoot = resolve(ROOT, arg("out", "content/generated/characters"));
const meshRoot = resolve(ROOT, arg("mesh-dir", "content/generated"));
const only = arg("only");
const noMesh = has("no-mesh");

if (!existsSync(conceptsDir)) {
  console.error(`batch: no concepts at ${conceptsDir}`);
  process.exit(1);
}

const files = readdirSync(conceptsDir)
  .filter((n) => /^concept_.+\.(jpg|png)$/i.test(n))
  .filter((n) => !only || n.includes(only));
if (!files.length) {
  console.error(`batch: no concept_*.jpg in ${conceptsDir}`);
  process.exit(1);
}

// What one concept will do: already done, skipped for want of a mesh, or run
// (with the base mesh to reuse, when there is one). Reads only.
function decide(file) {
  const id = basename(file).replace(/^concept_/i, "").replace(/\.(jpg|png)$/i, "");
  const concept = join(conceptsDir, file);
  const out = join(outRoot, id);
  // Work already done is not done again: the mesh is the expensive part.
  if (has("skip-existing") && existsSync(join(out, "character.json"))) {
    return { id, concept, out, action: "done" };
  }
  // A base mesh that already exists is the expensive part: reuse it.
  const base = join(meshRoot, `gunner_base_${id}`, "0", "mesh.glb");
  const conceptsBase = join(meshRoot, `base_${id}`, "0", "mesh.glb");
  const known = existsSync(base) ? base : (existsSync(conceptsBase) ? conceptsBase : null);
  if (!known && noMesh) return { id, concept, out, action: "no-mesh" };
  return { id, concept, out, action: "run", mesh: known };
}

const plan = files.map(decide);

// The dry run and the default without --yes both stop here, before the out
// folder is made or any character is started.
if (has("dry-run") || !has("yes")) {
  const runs = plan.filter((p) => p.action === "run");
  console.log(`${has("dry-run") ? "dry run" : "plan"}: ${runs.length} concept(s) to run, ` +
    `${plan.length - runs.length} skipped, into ${outRoot}`);
  for (const p of plan) {
    if (p.action === "done") console.log(`  skip ${p.id}: already done`);
    else if (p.action === "no-mesh") console.log(`  skip ${p.id}: no base mesh and --no-mesh`);
    else {
      const mesh = p.mesh ? ` --mesh ${p.mesh}` : " (makes the base mesh with sf3d)";
      console.log(`  ${p.id}: node tools/asset-gen/character.mjs --concept ${p.concept} --name ${p.id} --out ${p.out}${mesh}`);
    }
  }
  if (has("dry-run")) process.exit(0);
  console.error(`nothing was run: add --yes to run ${runs.length} concept(s) (local Blender and sf3d, hours of work)`);
  process.exit(2);
}

mkdirSync(outRoot, { recursive: true });

const rows = [];
for (const p of plan) {
  if (p.action === "done") {
    console.log(`skip ${p.id}: already done`);
    const done = JSON.parse(readFileSync(join(p.out, "character.json"), "utf8"));
    rows.push({
      id: p.id,
      verdict: done.gate?.verdict ?? "unknown",
      fill: done.gate?.bbox_fill ?? null,
      fill30: done.gate?.fill_30m ?? null,
      tris: done.cook?.lods?.[0]?.triangles ?? null,
      rig: done.rig?.weighted_vertices ?? null,
      vertices: done.rig?.vertices ?? null,
      error: null,
    });
    continue;
  }
  if (p.action === "no-mesh") {
    console.log(`skip ${p.id}: no base mesh and --no-mesh`);
    continue;
  }

  const args = [join(ROOT, "tools", "asset-gen", "character.mjs"),
    "--concept", p.concept, "--name", p.id, "--out", p.out];
  if (p.mesh) args.push("--mesh", p.mesh);

  console.log(`\n=== ${p.id}`);
  let result = null;
  let failure = null;
  try {
    const stdout = execFileSync(process.execPath, args, {
      cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, timeout: 4 * 60 * 60 * 1000,
    });
    const line = stdout.trim().split("\n").find((l) => l.startsWith("GATE "));
    if (line) result = line.replace(/^GATE /, "").trim();
  } catch (err) {
    failure = (err.stdout || "") + (err.stderr || "");
    const line = String(failure).trim().split("\n").find((l) => l.startsWith("GATE "));
    if (line) result = line.replace(/^GATE /, "").trim();
  }

  const charFile = join(p.out, "character.json");
  let character = null;
  if (existsSync(charFile)) character = JSON.parse(readFileSync(charFile, "utf8"));
  rows.push({
    id: p.id,
    verdict: character?.gate?.verdict ?? (result ? result.split(/\s+/)[0] : "error"),
    fill: character?.gate?.bbox_fill ?? null,
    fill30: character?.gate?.fill_30m ?? null,
    tris: character?.cook?.lods?.[0]?.triangles ?? null,
    rig: character?.rig?.weighted_vertices ?? null,
    vertices: character?.rig?.vertices ?? null,
    error: character ? null : String(failure || result || "no result").slice(0, 400),
  });
  console.log(`${p.id}: ${rows[rows.length - 1].verdict} fill ${rows[rows.length - 1].fill}`);
}

const summary = {
  generated_at: new Date().toISOString(),
  concepts_dir: conceptsDir,
  out: outRoot,
  count: rows.length,
  passed: rows.filter((r) => r.verdict === "pass").length,
  rows,
};
writeFileSync(join(outRoot, "batch.json"), JSON.stringify(summary, null, 2) + "\n", "utf8");

const lines = [
  `| class | gate | fill | fill@30m | LOD0 tris | rigged verts |`,
  `|---|---|---|---|---|---|`,
];
for (const r of rows) {
  lines.push(`| ${r.id} | ${r.verdict} | ${r.fill ?? "-"} | ${r.fill30 ?? "-"} | ${r.tris ?? "-"} | ${r.rig ?? "-"}/${r.vertices ?? "-"} |`);
}
writeFileSync(join(outRoot, "batch.md"), lines.join("\n") + "\n", "utf8");
console.log(`\n${summary.passed}/${summary.count} passed the gate`);
console.log(`WROTE ${join(outRoot, "batch.json")} and batch.md`);

// The whole sheet through the lane, one character at a time.
//
//   node tools/asset-gen/batch.mjs --concepts content/generated/concepts --out content/generated/characters
//
// It calls tools/asset-gen/character.mjs for each concept, so the machine is
// the same one, and it writes a summary table: the gate verdict, the fill,
// the triangle count and the rig coverage for every class.
//
//   --mesh-dir <dir>   use the base meshes already in <dir>/<name>/0/mesh.glb
//   --only <id>        one class
//   --no-mesh          do not generate a base mesh, use what is there
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const has = (name) => process.argv.includes(`--${name}`);

const conceptsDir = resolve(ROOT, arg("concepts", "content/generated/concepts"));
const outRoot = resolve(ROOT, arg("out", "content/generated/characters"));
const meshRoot = resolve(ROOT, arg("mesh-dir", "content/generated"));
const only = arg("only");
const noMesh = has("no-mesh");

if (!existsSync(conceptsDir)) {
  console.error(`batch: no concepts at ${conceptsDir}`);
  process.exit(1);
}
mkdirSync(outRoot, { recursive: true });

const files = readdirSync(conceptsDir)
  .filter((n) => /^concept_.+\.(jpg|png)$/i.test(n))
  .filter((n) => !only || n.includes(only));
if (!files.length) {
  console.error(`batch: no concept_*.jpg in ${conceptsDir}`);
  process.exit(1);
}

const rows = [];
for (const file of files) {
  const id = basename(file).replace(/^concept_/i, "").replace(/\.(jpg|png)$/i, "");
  const concept = join(conceptsDir, file);
  const out = join(outRoot, id);
  // Work already done is not done again: the mesh is the expensive part.
  if (has("skip-existing") && existsSync(join(out, "character.json"))) {
    console.log(`skip ${id}: already done`);
    const done = JSON.parse(readFileSync(join(out, "character.json"), "utf8"));
    rows.push({
      id,
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
  const args = [join(ROOT, "tools", "asset-gen", "character.mjs"),
    "--concept", concept, "--name", id, "--out", out];

  // A base mesh that already exists is the expensive part: reuse it.
  const base = join(meshRoot, `gunner_base_${id}`, "0", "mesh.glb");
  const conceptsBase = join(meshRoot, `base_${id}`, "0", "mesh.glb");
  const known = existsSync(base) ? base : (existsSync(conceptsBase) ? conceptsBase : null);
  if (known) {
    args.push("--mesh", known);
  } else if (noMesh) {
    console.log(`skip ${id}: no base mesh and --no-mesh`);
    continue;
  }

  console.log(`\n=== ${id}`);
  let result = null;
  let failure = null;
  try {
    const out = execFileSync(process.execPath, args, {
      cwd: ROOT, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, timeout: 4 * 60 * 60 * 1000,
    });
    const line = out.trim().split("\n").find((l) => l.startsWith("GATE "));
    if (line) result = line.replace(/^GATE /, "").trim();
  } catch (err) {
    failure = (err.stdout || "") + (err.stderr || "");
    const line = String(failure).trim().split("\n").find((l) => l.startsWith("GATE "));
    if (line) result = line.replace(/^GATE /, "").trim();
  }

  const charFile = join(out, "character.json");
  let character = null;
  if (existsSync(charFile)) character = JSON.parse(readFileSync(charFile, "utf8"));
  rows.push({
    id,
    verdict: character?.gate?.verdict ?? (result ? result.split(/\s+/)[0] : "error"),
    fill: character?.gate?.bbox_fill ?? null,
    fill30: character?.gate?.fill_30m ?? null,
    tris: character?.cook?.lods?.[0]?.triangles ?? null,
    rig: character?.rig?.weighted_vertices ?? null,
    vertices: character?.rig?.vertices ?? null,
    error: character ? null : String(failure || result || "no result").slice(0, 400),
  });
  console.log(`${id}: ${rows[rows.length - 1].verdict} fill ${rows[rows.length - 1].fill}`);
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

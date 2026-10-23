// blender-cook.mjs: the one pinned Blender the geometry cooks may drive (GEO1a).
//   node tools/dcc/blender-cook.mjs --check [--exe <path>] [--pin <file>]
// Hashes the exe file first, then runs it once headless and reads its version. Prints
// `ok <version> <cold_start_seconds>`, exit 0. Exit 2 `blender.version_mismatch` when the
// exe's sha256 is not the pinned one or its version does not start with the pin's
// version_prefix; exit 3 `blender.missing` when the exe path does not exist; exit 1 for a
// usage error or a run that failed or timed out (`blender.run_failed`). The pin is
// tools/dcc/blender.pin.json unless --pin names another file.
//
//   node tools/dcc/blender-cook.mjs --recipe <file> --out <glb> [--script <py>]
//
// GEO14 adds the `bake` op: a piece with ambient occlusion, curvature grime and a
// bevel normal. The recipe guards run before the pinned check (so a no-UV piece never
// starts Blender), the pinned Blender bakes and exports, then each map rides the
// existing `engine-assets` KTX2 cook through tools/dcc/ktx2-cook, and the sidecar
// carries the GLB digest, the PNG digests and the KTX2 set digest.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..", "..");
const RUN_TIMEOUT_MS = 60_000;
const BAKE_TIMEOUT_MS = 300_000;
const KTX2_TIMEOUT_MS = 600_000;
const VERSION_EXPR = "import bpy;print('E2040-VERSION', bpy.app.version_string)";

/** sha256 of a file as lowercase hex, streamed (the exe is over 100 MB). */
export function sha256File(path) {
  return new Promise((resolvePromise, reject) => {
    const hash = createHash("sha256");
    createReadStream(path)
      .on("data", (chunk) => hash.update(chunk))
      .on("error", reject)
      .on("end", () => resolvePromise(hash.digest("hex")));
  });
}

/** The version string from Blender's stdout, or null when the marker line is missing. */
export function parseVersion(stdout) {
  const line = String(stdout).split(/\r?\n/).find((l) => l.startsWith("E2040-VERSION "));
  return line ? line.slice("E2040-VERSION ".length).trim() : null;
}

/** The checked result as { code, text }: the caller prints text and exits with code. */
export async function check({ exe, pinPath }) {
  const pin = JSON.parse(readFileSync(pinPath, "utf8"));
  const path = exe ?? pin.exe;
  if (!existsSync(path)) return { code: 3, text: `blender.missing: ${path}` };
  const sha = await sha256File(path);
  if (sha !== pin.exe_sha256) {
    return { code: 2, text: `blender.version_mismatch: sha256 ${sha} is not the pinned ${pin.exe_sha256}` };
  }
  const started = performance.now();
  const run = spawnSync(
    path,
    ["-b", "--factory-startup", "-noaudio", "--python-exit-code", "1", "--python-expr", VERSION_EXPR],
    { encoding: "utf8", timeout: RUN_TIMEOUT_MS },
  );
  const seconds = ((performance.now() - started) / 1000).toFixed(2);
  const version = run.status === 0 ? parseVersion(run.stdout) : null;
  if (!version) {
    const why = run.error ? run.error.message : `exit ${run.status}`;
    return { code: 1, text: `blender.run_failed: ${why}` };
  }
  if (!version.startsWith(pin.version_prefix)) {
    return { code: 2, text: `blender.version_mismatch: ${version} does not start with ${pin.version_prefix}` };
  }
  return { code: 0, text: `ok ${version} ${seconds}`, version };
}

function flag(argv, name) {
  const at = argv.indexOf(name);
  return at >= 0 ? argv[at + 1] : undefined;
}

/** The op names the recipe runner supports: anything else is refused before
 * Blender starts (GEO1b, GEO14). */
export const ALLOWED_RECIPE_OPS = ["cube", "bevel", "mirror", "array", "bake"];

/** The three bake flags, one per map (GEO14). */
export const BAKE_MAPS = ["ao", "grime", "normal"];

/**
 * The glTF document behind a `.gltf` or `.glb`, or throws. Reads the JSON chunk
 * of a GLB so the UV guard never needs Blender.
 */
export function readGltfDocument(path) {
  const bytes = readFileSync(path);
  if (bytes.length >= 20 && bytes.subarray(0, 4).toString("latin1") === "glTF") {
    const length = bytes.readUInt32LE(12);
    if (length === 0 || 20 + length > bytes.length) throw new Error("short glb json chunk");
    return JSON.parse(bytes.subarray(20, 20 + length).toString("utf8"));
  }
  return JSON.parse(bytes.toString("utf8"));
}

/**
 * Object names whose primitives carry no TEXCOORD_0. The name is the node that
 * places the mesh, else the mesh name, so a refusal names what a person sees.
 */
export function objectsWithoutUv(doc) {
  const meshes = Array.isArray(doc.meshes) ? doc.meshes : [];
  const nodes = Array.isArray(doc.nodes) ? doc.nodes : [];
  const missing = [];
  const lacksUv = (mesh) =>
    !Array.isArray(mesh?.primitives) ||
    mesh.primitives.length === 0 ||
    mesh.primitives.some((prim) => prim?.attributes?.TEXCOORD_0 === undefined);
  const named = new Set();
  nodes.forEach((node) => {
    if (node?.mesh === undefined) return;
    named.add(node.mesh);
    const mesh = meshes[node.mesh];
    if (lacksUv(mesh)) missing.push(node.name ?? mesh?.name ?? `mesh ${node.mesh}`);
  });
  meshes.forEach((mesh, index) => {
    if (named.has(index)) return;
    if (lacksUv(mesh)) missing.push(mesh?.name ?? `mesh ${index}`);
  });
  return [...new Set(missing)];
}

/**
 * The recipe guards as { code, text }: 0 means cook it. A `python` key and an
 * op outside the allowed set are refused the same way, with text
 * `recipe.unknown_op` (exit 4), before any process spawn. A bake recipe names a
 * source piece; a piece with a primitive that has no UV refuses as
 * `recipe.no_uv: object "<name>"` before the pinned check and Blender start.
 */
export function guardRecipe(recipe, { root = ROOT } = {}) {
  if (recipe && typeof recipe === "object" && "python" in recipe) {
    return { code: 4, text: "recipe.unknown_op: a python key is never read" };
  }
  const ops = recipe && Array.isArray(recipe.ops) ? recipe.ops : [];
  if (ops.length === 0) {
    return { code: 4, text: "recipe.unknown_op: the recipe holds no ops" };
  }
  for (const op of ops) {
    const name = op && typeof op === "object" ? op.op : undefined;
    if (!ALLOWED_RECIPE_OPS.includes(name)) {
      return { code: 4, text: `recipe.unknown_op: ${JSON.stringify(name ?? null)}` };
    }
  }
  const bake = ops.filter((op) => op.op === "bake");
  if (bake.length > 0) {
    if (ops.length !== 1) return { code: 4, text: "recipe.bad_bake: a bake recipe holds one bake op" };
    const op = bake[0];
    if (typeof op.src !== "string" || !/\.(gltf|glb)$/i.test(op.src)) {
      return { code: 4, text: "recipe.bad_bake: src wants a .gltf or .glb path" };
    }
    if (isAbsolute(op.src) || op.src.split(/[\\/]/).includes("..")) {
      return { code: 4, text: "recipe.bad_bake: src must stay under the tree" };
    }
    const size = op.size ?? 256;
    if (!Number.isInteger(size) || size < 16 || size > 1024) {
      return { code: 4, text: "recipe.bad_bake: size wants 16..1024" };
    }
    let flags = 0;
    for (const name of BAKE_MAPS) {
      const value = op[name] ?? false;
      if (typeof value !== "boolean") return { code: 4, text: `recipe.bad_bake: ${name} wants true or false` };
      flags += value ? 1 : 0;
    }
    if (flags === 0) return { code: 4, text: "recipe.no_maps: name ao, grime or normal" };
    const target = resolve(root, op.src);
    if (!existsSync(target)) return { code: 4, text: `recipe.bad_src: ${op.src}` };
    let doc;
    try {
      doc = readGltfDocument(target);
    } catch (error) {
      return { code: 4, text: `recipe.bad_src: ${op.src}: ${error.message}` };
    }
    if (!Array.isArray(doc.meshes) || doc.meshes.length === 0) {
      return { code: 4, text: `recipe.no_mesh: ${op.src}` };
    }
    const missing = objectsWithoutUv(doc);
    if (missing.length > 0) {
      return { code: 4, text: `recipe.no_uv: object "${missing[0]}" has no TEXCOORD_0` };
    }
  }
  return { code: 0, text: "ok" };
}

/** What a bake recipe asked for, or null for a geometry recipe. */
export function bakePlan(recipe) {
  const op = recipe?.ops?.find((entry) => entry?.op === "bake");
  if (!op) return null;
  return {
    src: op.src,
    size: op.size ?? 256,
    bevel: op.bevel ?? 0.01,
    maps: BAKE_MAPS.filter((name) => op[name] === true),
  };
}

/**
 * Cook one baked map into KTX2 through the existing `engine-assets` cook. The
 * wrapper crate lives under tools/dcc so no file under crates/ changes; cargo
 * builds it once into target/ktx2-cook.
 */
function cookKtx2({ inPath, outPath, kind }) {
  const manifest = join(HERE, "ktx2-cook", "Cargo.toml");
  const run = spawnSync(
    "cargo",
    ["run", "--quiet", "--manifest-path", manifest, "--", "--in", inPath, "--out", outPath, "--kind", kind],
    {
      encoding: "utf8",
      timeout: KTX2_TIMEOUT_MS,
      env: { ...process.env, CARGO_TARGET_DIR: join(ROOT, "target", "ktx2-cook") },
    },
  );
  if (run.status !== 0) {
    const why = run.error ? run.error.message : `exit ${run.status}: ${run.stderr ?? ""}`;
    return { code: 1, text: `ktx2.cook_failed: ${why}` };
  }
  return { code: 0, text: String(run.stdout ?? "").trim() };
}

/** The maps a bake plan writes besides the GLB: every requested map plus the ORM
 * pack when occlusion or grime is on. */
export function bakedMapNames(plan) {
  const names = [];
  if (plan.maps.includes("ao")) names.push("ao");
  if (plan.maps.includes("grime")) names.push("grime");
  if (plan.maps.includes("ao") || plan.maps.includes("grime")) names.push("orm");
  if (plan.maps.includes("normal")) names.push("normal");
  return names;
}

/** Cook one recipe into one GLB (GEO1b): the recipe guards run before any
 * spawn, the pinned check runs before Blender, the pinned Blender runs the
 * recipe script and the digests land in `<out>.json`. A bake recipe then cooks
 * its maps through KTX2 and the sidecar carries the map shas too. Returns
 * { code, text, sidecar, stdout }. */
export async function cookRecipe({ recipePath, outPath, scriptPath, pinPath, exe }) {
  const pin = JSON.parse(readFileSync(pinPath, "utf8"));
  const blender = exe ?? pin.exe;
  const recipeText = readFileSync(recipePath, "utf8");
  let recipe = null;
  try {
    recipe = JSON.parse(recipeText);
  } catch (error) {
    return { code: 4, text: `recipe.unknown_op: unreadable json: ${error.message}` };
  }
  const guard = guardRecipe(recipe);
  if (guard.code !== 0) return { ...guard };
  const first = await check({ exe, pinPath });
  if (first.code !== 0) return { ...first };
  const script = scriptPath ?? join(HERE, "blender", "run_recipe.py");
  const out = outPath ?? `${recipePath}.glb`;
  const plan = bakePlan(recipe);
  const run = spawnSync(
    blender,
    [
      "-b", "--factory-startup", "-noaudio", "--python-exit-code", "1",
      "--threads", "1", "--disable-autoexec", "-P", script,
      "--", recipePath, out,
    ],
    { encoding: "utf8", timeout: plan ? BAKE_TIMEOUT_MS : RUN_TIMEOUT_MS },
  );
  if (run.status !== 0) {
    const why = run.error ? run.error.message : `exit ${run.status}: ${run.stderr ?? ""}`;
    return { code: 1, text: `blender.run_failed: ${why}` };
  }
  const sidecar = {
    blender_version: createHash("sha256").update(String(first.version ?? "")).digest("hex"),
    ...await digestsOfThree(recipeText, script, out),
  };
  if (plan) {
    const dir = dirname(out);
    const stem = basename(out).replace(/\.glb$/i, "");
    const maps = {};
    for (const name of bakedMapNames(plan)) {
      const png = join(dir, `${stem}_${name}.png`);
      if (!existsSync(png)) return { code: 1, text: `recipe.missing_map: ${png}` };
      maps[name] = { png, bytes: statSync(png).size, sha256: await sha256File(png) };
    }
    for (const name of ["orm", "normal"]) {
      if (!maps[name]) continue;
      const ktx2 = join(dir, `${stem}_${name}.ktx2`);
      const cooked = cookKtx2({ inPath: maps[name].png, outPath: ktx2, kind: name });
      if (cooked.code !== 0) return { ...cooked };
      maps[name].kind = name;
      maps[name].ktx2 = ktx2;
      maps[name].ktx2_bytes = statSync(ktx2).size;
      maps[name].ktx2_sha256 = await sha256File(ktx2);
    }
    const cooked = ["orm", "normal"].filter((name) => maps[name]?.ktx2_sha256);
    sidecar.bytes = statSync(out).size;
    sidecar.bake = { src: plan.src, size: plan.size, bevel: plan.bevel, maps: plan.maps };
    sidecar.maps = maps;
    if (cooked.length > 0) {
      const digest = createHash("sha256");
      for (const name of cooked) digest.update(`${name} ${maps[name].ktx2_sha256}\n`);
      sidecar.ktx2_set = digest.digest("hex");
    }
  }
  writeFileSync(`${out}.json`, `${JSON.stringify(sidecar, null, 2)}\n`);
  const line = sidecar.ktx2_set ? `ok ${out} glb=${sidecar.output} ktx2=${sidecar.ktx2_set}` : `ok ${out}`;
  return { code: 0, text: line, sidecar, stdout: run.stdout ?? "" };
}

/** recipe, script and output digests for the sidecar: three sha256 values. */
async function digestsOfThree(recipeText, scriptPath, outPath) {
  return {
    recipe: createHash("sha256").update(recipeText).digest("hex"),
    script: await sha256File(scriptPath),
    output: await sha256File(outPath),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const argv = process.argv.slice(2);
  if (argv.includes("--check")) {
    const { code, text } = await check({
      exe: flag(argv, "--exe"),
      pinPath: flag(argv, "--pin") ?? join(HERE, "blender.pin.json"),
    });
    (code === 0 ? console.log : console.error)(text);
    process.exit(code);
  }
  if (argv.includes("--recipe")) {
    const { code, text } = await cookRecipe({
      recipePath: flag(argv, "--recipe"),
      outPath: flag(argv, "--out"),
      scriptPath: flag(argv, "--script"),
      pinPath: flag(argv, "--pin") ?? join(HERE, "blender.pin.json"),
      exe: flag(argv, "--exe"),
    });
    (code === 0 ? console.log : console.error)(text);
    process.exit(code);
  }
  console.error(
    "usage: node tools/dcc/blender-cook.mjs --check [--exe <path>] [--pin <file>]\n" +
    "   or: node tools/dcc/blender-cook.mjs --recipe <file> --out <glb> [--script <py>] [--exe <path>]",
  );
  process.exit(1);
}

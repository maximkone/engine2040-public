// blender-cook.mjs: the one pinned Blender the geometry cooks may drive (GEO1a).
//   node tools/dcc/blender-cook.mjs --check [--exe <path>] [--pin <file>]
// Hashes the exe file first, then runs it once headless and reads its version. Prints
// `ok <version> <cold_start_seconds>`, exit 0. Exit 2 `blender.version_mismatch` when the
// exe's sha256 is not the pinned one or its version does not start with the pin's
// version_prefix; exit 3 `blender.missing` when the exe path does not exist; exit 1 for a
// usage error or a run that failed or timed out (`blender.run_failed`). The pin is
// tools/dcc/blender.pin.json unless --pin names another file.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const RUN_TIMEOUT_MS = 60_000;
const VERSION_EXPR = "import bpy;print('E2040-VERSION', bpy.app.version_string)";

/** sha256 of a file as lowercase hex, streamed (the exe is over 100 MB). */
export function sha256File(path) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    createReadStream(path)
      .on("data", (chunk) => hash.update(chunk))
      .on("error", reject)
      .on("end", () => resolve(hash.digest("hex")));
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
 * Blender starts (GEO1b). */
export const ALLOWED_RECIPE_OPS = ["cube", "bevel", "mirror", "array"];

/** The recipe guards as { code, text }: 0 means cook it. A `python` key and an
 * op outside the allowed set are refused the same way, with text
 * `recipe.unknown_op` (exit 4), before any process spawn. */
export function guardRecipe(recipe) {
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
  return { code: 0, text: "ok" };
}

/** Cook one recipe into one GLB (GEO1b): the pinned check runs first, the
 * recipe guards run before any spawn, then the pinned Blender runs the recipe
 * script and the four digests land in `<out>.json`. Returns
 * { code, text, sidecar, stdout }. */
export async function cookRecipe({ recipePath, outPath, scriptPath, pinPath, exe }) {
  const pin = JSON.parse(readFileSync(pinPath, "utf8"));
  const blender = exe ?? pin.exe;
  const first = await check({ exe, pinPath });
  if (first.code !== 0) return { ...first };
  const recipeText = readFileSync(recipePath, "utf8");
  let recipe = null;
  try {
    recipe = JSON.parse(recipeText);
  } catch (error) {
    return { code: 4, text: `recipe.unknown_op: unreadable json: ${error.message}` };
  }
  const guard = guardRecipe(recipe);
  if (guard.code !== 0) return { ...guard };
  const script = scriptPath ?? join(HERE, "blender", "run_recipe.py");
  const out = outPath ?? `${recipePath}.glb`;
  const run = spawnSync(
    blender,
    [
      "-b", "--factory-startup", "-noaudio", "--python-exit-code", "1",
      "--threads", "1", "--disable-autoexec", "-P", script,
      "--", recipePath, out,
    ],
    { encoding: "utf8", timeout: RUN_TIMEOUT_MS },
  );
  if (run.status !== 0) {
    const why = run.error ? run.error.message : `exit ${run.status}: ${run.stderr ?? ""}`;
    return { code: 1, text: `blender.run_failed: ${why}` };
  }
  const sidecar = {
    blender_version: createHash("sha256").update(String(first.version ?? "")).digest("hex"),
    ...await digestsOfThree(recipeText, script, out),
  };
  writeFileSync(`${out}.json`, `${JSON.stringify(sidecar, null, 2)}\n`);
  return { code: 0, text: `ok ${out}`, sidecar, stdout: run.stdout ?? "" };
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

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
//   node tools/dcc/blender-cook.mjs --batch <batch.json> [--crash-at <n>]
//
// GEO14 adds the `bake` op: a piece with ambient occlusion, curvature grime and a
// bevel normal. The recipe guards run before the pinned check (so a no-UV piece never
// starts Blender), the pinned Blender bakes and exports, then each map rides the
// existing `engine-assets` KTX2 cook through tools/dcc/ktx2-cook, and the sidecar
// carries the GLB digest, the PNG digests and the KTX2 set digest.
//
// GEO15 adds the batch: one warm Blender serves every recipe. `--batch` names a JSON
// manifest `{"recipes":[{"recipe":<path>,"out":<glb>},...]}`; every recipe is guarded
// before anything starts, the exe hash is checked once, then `run_recipe.py --serve`
// cooks each recipe on its main thread and reloads factory settings between them.
// The gate is digest equality with the fresh-process cook. A crash mid-batch restarts
// the process once and resumes at the in-flight recipe; a second crash fails the
// batch. `--crash-at <n>` kills the warm process just before recipe n (index from 0)
// so a test can prove the restart path without waiting for a real crash.
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import net from "node:net";
import { basename, dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..", "..");
const RUN_TIMEOUT_MS = 60_000;
const BAKE_TIMEOUT_MS = 300_000;
const KTX2_TIMEOUT_MS = 600_000;
const BRIDGE_TIMEOUT_MS = 60_000;
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

/** The pin and exe file check without starting Blender: the batch hashes once
 * and reads the version from the warm process it already starts. */
export async function verifyPinned({ exe, pinPath }) {
  const pin = JSON.parse(readFileSync(pinPath, "utf8"));
  const path = exe ?? pin.exe;
  if (!existsSync(path)) return { code: 3, text: `blender.missing: ${path}` };
  const sha = await sha256File(path);
  if (sha !== pin.exe_sha256) {
    return { code: 2, text: `blender.version_mismatch: sha256 ${sha} is not the pinned ${pin.exe_sha256}` };
  }
  return { code: 0, text: "ok", exe: path, pin };
}

/** The checked result as { code, text }: the caller prints text and exits with code. */
export async function check({ exe, pinPath }) {
  const verified = await verifyPinned({ exe, pinPath });
  if (verified.code !== 0) return verified;
  const { pin, exe: path } = verified;
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
  const done = await completeSidecar({ recipeText, scriptPath: script, outPath: out, plan, version: first.version });
  return { ...done, stdout: run.stdout ?? "" };
}

/** The sidecar a cook writes: the GLB, recipe and script digests, and for a
 * bake the map shas plus the KTX2 set. Shared by one cold cook and a batch. */
async function completeSidecar({ recipeText, scriptPath, outPath, plan, version }) {
  if (!existsSync(outPath)) return { code: 1, text: `blender.run_failed: no output ${outPath}` };
  const sidecar = {
    blender_version: createHash("sha256").update(String(version ?? "")).digest("hex"),
    ...await digestsOfThree(recipeText, scriptPath, outPath),
  };
  if (plan) {
    const dir = dirname(outPath);
    const stem = basename(outPath).replace(/\.glb$/i, "");
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
    sidecar.bytes = statSync(outPath).size;
    sidecar.bake = { src: plan.src, size: plan.size, bevel: plan.bevel, maps: plan.maps };
    sidecar.maps = maps;
    if (cooked.length > 0) {
      const digest = createHash("sha256");
      for (const name of cooked) digest.update(`${name} ${maps[name].ktx2_sha256}\n`);
      sidecar.ktx2_set = digest.digest("hex");
    }
  }
  writeFileSync(`${outPath}.json`, `${JSON.stringify(sidecar, null, 2)}\n`);
  const line = sidecar.ktx2_set ? `ok ${outPath} glb=${sidecar.output} ktx2=${sidecar.ktx2_set}` : `ok ${outPath}`;
  return { code: 0, text: line, sidecar };
}

/** recipe, script and output digests for the sidecar: three sha256 values. */
async function digestsOfThree(recipeText, scriptPath, outPath) {
  return {
    recipe: createHash("sha256").update(recipeText).digest("hex"),
    script: await sha256File(scriptPath),
    output: await sha256File(outPath),
  };
}

// ---------------------------------------------------------------------------
// Warm batch (GEO15).
// ---------------------------------------------------------------------------

/** Spawn `run_recipe.py --serve` and read its `bridge.listening <port> <version>` line. */
function launchBridge({ blender, scriptPath }) {
  const child = spawn(
    blender,
    [
      "-b", "--factory-startup", "-noaudio", "--python-exit-code", "1",
      "--threads", "1", "--disable-autoexec", "-P", scriptPath,
      "--", "--serve",
    ],
    { stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
  );
  let stderr = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr = (stderr + chunk).slice(-65536);
  });
  const banner = new Promise((resolvePromise, reject) => {
    let buffered = "";
    const timer = setTimeout(() => {
      stop();
      reject(new Error("bridge.banner_timeout: the warm Blender printed no port"));
    }, BRIDGE_TIMEOUT_MS);
    const stop = () => {
      clearTimeout(timer);
      child.stdout.off("data", onData);
      child.off("exit", onExit);
    };
    const onData = (chunk) => {
      buffered += chunk;
      const at = buffered.indexOf("\n");
      if (at < 0) return;
      const line = buffered.slice(0, at).trim();
      const match = /^bridge\.listening (\d+) (.+)$/.exec(line);
      stop();
      // The bridge keeps printing one recipe line per cook; keep the pipe drained.
      child.stdout.on("data", () => {});
      if (!match) {
        reject(new Error(`bridge.bad_banner: ${line}`));
        return;
      }
      resolvePromise({ port: Number(match[1]), version: match[2].trim() });
    };
    const onExit = (code) => {
      stop();
      const last = stderr.trim().split("\n").pop();
      reject(new Error(`bridge.exited: the warm Blender left with ${code}${last ? `: ${last}` : ""}`));
    };
    child.stdout.setEncoding("utf8");
    child.stdout.on("data", onData);
    child.once("exit", onExit);
  });
  return { child, banner };
}

/** One request in flight at a time; a death or timeout rejects it. */
class WarmBridge {
  constructor(child, socket, version) {
    this.child = child;
    this.socket = socket;
    this.version = version;
    this.nextId = 1;
    this.pending = null;
    this.closed = null;
    let buffered = "";
    socket.setEncoding("utf8");
    socket.on("data", (chunk) => {
      buffered += chunk;
      let at;
      while ((at = buffered.indexOf("\n")) >= 0) {
        const line = buffered.slice(0, at).trim();
        buffered = buffered.slice(at + 1);
        if (!line) continue;
        let message;
        try {
          message = JSON.parse(line);
        } catch {
          continue;
        }
        const pending = this.pending;
        if (!pending) continue;
        this.pending = null;
        clearTimeout(pending.timer);
        pending.resolve(message);
      }
    });
    socket.on("error", () => this.shut());
    socket.on("close", () => this.shut());
    child.once("exit", () => this.shut());
  }

  shut() {
    if (this.closed) return;
    this.closed = new Error("bridge.closed: the warm Blender exited");
    const pending = this.pending;
    if (!pending) return;
    this.pending = null;
    clearTimeout(pending.timer);
    pending.reject(this.closed);
  }

  request(recipePath, outPath, timeoutMs) {
    if (this.closed) return Promise.reject(this.closed);
    const id = this.nextId++;
    return new Promise((resolvePromise, reject) => {
      const timer = setTimeout(() => {
        if (!this.pending) return;
        this.pending = null;
        reject(new Error("bridge.timeout: a warm recipe ran past its limit"));
        this.socket.destroy();
      }, timeoutMs);
      this.pending = { resolve: resolvePromise, reject, timer };
      this.socket.write(`${JSON.stringify({ id, recipe: recipePath, out: outPath })}\n`);
    });
  }

  async close() {
    if (!this.closed) {
      this.socket.write(`${JSON.stringify({ cmd: "quit" })}\n`);
      await new Promise((resolvePromise) => {
        const timer = setTimeout(resolvePromise, 5000);
        this.socket.once("close", () => {
          clearTimeout(timer);
          resolvePromise();
        });
      });
    }
    this.socket.destroy();
    if (this.child.exitCode === null) {
      const exited = new Promise((resolvePromise) => this.child.once("exit", resolvePromise));
      this.child.kill();
      await Promise.race([exited, new Promise((resolvePromise) => setTimeout(resolvePromise, 5000))]);
    }
  }
}

async function connectWarm({ exe, scriptPath, pin }) {
  const launched = launchBridge({ blender: exe, scriptPath });
  let banner;
  try {
    banner = await launched.banner;
  } catch (error) {
    launched.child.kill();
    throw error;
  }
  if (!banner.version.startsWith(pin.version_prefix)) {
    launched.child.kill();
    throw Object.assign(new Error("version"), {
      code: 2,
      text: `blender.version_mismatch: ${banner.version} does not start with ${pin.version_prefix}`,
    });
  }
  const socket = net.createConnection({ host: "127.0.0.1", port: banner.port });
  await new Promise((resolvePromise, reject) => {
    socket.once("connect", resolvePromise);
    socket.once("error", reject);
  });
  return new WarmBridge(launched.child, socket, banner.version);
}

function killChild(child) {
  if (child.exitCode !== null) return Promise.resolve();
  return new Promise((resolvePromise) => {
    child.once("exit", resolvePromise);
    child.kill();
  });
}

/**
 * Cook a manifest of recipes on one warm Blender. Every recipe is guarded and
 * the exe hashed before the process starts; the version comes from the warm
 * process itself, so a batch pays one startup instead of one per recipe. A
 * crash restarts once at the in-flight recipe and reports the restart; a
 * second crash fails the batch. Returns { code, text, restarts, seconds }.
 */
export async function cookBatch({ batchPath, pinPath, scriptPath, exe, crashAt = -1 }) {
  let manifest;
  try {
    manifest = JSON.parse(readFileSync(batchPath, "utf8"));
  } catch (error) {
    return { code: 4, text: `batch.unreadable: ${error.message}` };
  }
  const entries = Array.isArray(manifest?.recipes) ? manifest.recipes : [];
  if (entries.length === 0) {
    return { code: 4, text: 'batch.empty: name recipes as [{"recipe":<path>,"out":<glb>}]' };
  }
  const recipes = [];
  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index];
    if (!entry || typeof entry.recipe !== "string" || typeof entry.out !== "string") {
      return { code: 4, text: `batch.bad_item: recipe ${index} wants recipe and out paths` };
    }
    const recipePath = resolve(entry.recipe);
    const outPath = resolve(entry.out);
    let recipeText;
    try {
      recipeText = readFileSync(recipePath, "utf8");
    } catch (error) {
      return { code: 4, text: `batch recipe ${index}: recipe.unreadable: ${error.message}` };
    }
    let recipe;
    try {
      recipe = JSON.parse(recipeText);
    } catch (error) {
      return { code: 4, text: `batch recipe ${index}: recipe.unknown_op: unreadable json: ${error.message}` };
    }
    const guard = guardRecipe(recipe);
    if (guard.code !== 0) return { ...guard, text: `batch recipe ${index}: ${guard.text}` };
    recipes.push({ index, recipePath, outPath, recipeText, plan: bakePlan(recipe) });
  }
  const verified = await verifyPinned({ exe, pinPath });
  if (verified.code !== 0) return verified;
  const script = scriptPath ?? join(HERE, "blender", "run_recipe.py");
  const started = performance.now();
  const sidecars = [];
  let restarts = 0;
  let bridge = null;
  let pendingCrash = crashAt;
  try {
    bridge = await connectWarm({ exe: verified.exe, scriptPath: script, pin: verified.pin });
    for (const item of recipes) {
      mkdirSync(dirname(item.outPath), { recursive: true });
      let reply;
      for (;;) {
        if (item.index === pendingCrash) {
          pendingCrash = -1;
          await killChild(bridge.child);
        }
        try {
          reply = await bridge.request(item.recipePath, item.outPath, item.plan ? BAKE_TIMEOUT_MS : RUN_TIMEOUT_MS);
          break;
        } catch (error) {
          if (restarts >= 1) return { code: 1, text: `blender.batch_crashed: ${error.message}` };
          restarts += 1;
          process.stderr.write(`blender.batch_restarted before recipe ${item.index}: ${error.message}\n`);
          try {
            bridge = await connectWarm({ exe: verified.exe, scriptPath: script, pin: verified.pin });
          } catch (again) {
            return { code: again.code ?? 1, text: again.text ?? `blender.batch_crashed: ${again.message}` };
          }
        }
      }
      if (!reply.ok) {
        return { code: reply.code ?? 1, text: `batch recipe ${item.index}: ${reply.text ?? "blender.recipe_failed"}` };
      }
      const done = await completeSidecar({
        recipeText: item.recipeText,
        scriptPath: script,
        outPath: item.outPath,
        plan: item.plan,
        version: bridge.version,
      });
      if (done.code !== 0) return { ...done, text: `batch recipe ${item.index}: ${done.text}` };
      sidecars.push(done.sidecar);
    }
  } catch (error) {
    return { code: error.code ?? 1, text: error.text ?? `blender.run_failed: ${error.message}` };
  } finally {
    if (bridge) await bridge.close();
  }
  const seconds = ((performance.now() - started) / 1000).toFixed(2);
  return {
    code: 0,
    text: `ok batch ${recipes.length} restarts=${restarts} seconds=${seconds}`,
    restarts,
    seconds: Number(seconds),
    sidecars,
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
  if (argv.includes("--batch")) {
    const crash = flag(argv, "--crash-at");
    const crashAt = crash === undefined ? -1 : Number(crash);
    if (crash !== undefined && (!Number.isInteger(crashAt) || crashAt < 0)) {
      console.error("bad --crash-at: wants a recipe index 0 or more");
      process.exit(1);
    }
    const result = await cookBatch({
      batchPath: flag(argv, "--batch"),
      pinPath: flag(argv, "--pin") ?? join(HERE, "blender.pin.json"),
      scriptPath: flag(argv, "--script"),
      exe: flag(argv, "--exe"),
      crashAt,
    });
    (result.code === 0 ? console.log : console.error)(result.text);
    process.exit(result.code);
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
    "   or: node tools/dcc/blender-cook.mjs --recipe <file> --out <glb> [--script <py>] [--exe <path>]\n" +
    "   or: node tools/dcc/blender-cook.mjs --batch <batch.json> [--crash-at <n>] [--exe <path>]",
  );
  process.exit(1);
}

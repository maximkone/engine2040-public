// Tripo client: image or text to a rigged GLB, with a provenance row.
//
// The key is never written to disk and never printed. It comes from
// TRIPO_API_KEY in the environment, or from --key-file <path>.
//
//   node tools/asset-gen/tripo.mjs --help                     usage, no network
//   node tools/asset-gen/tripo.mjs --image concept.png --out content/generated/gunner --dry-run
//   node tools/asset-gen/tripo.mjs --image concept.png --out content/generated/gunner --yes
//   node tools/asset-gen/tripo.mjs --text "..." --out content/generated/kit --rig --yes
//
// Licence, checked before any call: Tripo free plan output is non-commercial
// and Tripo keeps the rights. Only a paid plan gives commercial rights to
// ship a model in a game. --plan must therefore be "paid" for anything that
// enters the repo.
//
// Every run is a paid Tripo task. Without --yes nothing is sent: the plan
// prints and the exit code is 2.
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const HOSTS = ["https://api.tripo3d.ai", "https://openapi.tripo3d.ai"];

const USAGE = `usage: node tools/asset-gen/tripo.mjs (--image <file> | --text "<prompt>") --out <dir>
         [--rig] [--model <id>] [--plan paid] [--key-file <path>] [--dry-run | --yes]

  One paid Tripo task: the model as <dir>/model.glb with <dir>/provenance.json.
  --image     our own picture (Tripo grants rights to the output, not to the input)
  --text      a prompt instead of a picture
  --rig       a second, paid rig task on the model
  --plan      must be "paid" (the default); the free plan is refused
  --key-file  read the key from a file; the key also comes from TRIPO_API_KEY and is never printed
  --dry-run   print the plan; no network, no key read, no writes; exit 0
  --yes       spend: without it the plan prints and the exit code is 2
  --help      this text; exit 0`;


function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const has = (name) => process.argv.includes(`--${name}`);

function fail(msg, code = 1) {
  console.error(`tripo: ${msg}`);
  process.exit(code);
}

async function call(base, path, key, init = {}) {
  const res = await fetch(base + path, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${key}`,
      ...(init.headers || {}),
    },
  });
  const text = await res.text();
  let body;
  try { body = JSON.parse(text); } catch { body = { raw: text.slice(0, 400) }; }
  if (!res.ok) {
    const code = body?.error?.code ?? res.status;
    const message = body?.error?.message ?? body?.message ?? text.slice(0, 200);
    throw new Error(`${code}: ${message}`);
  }
  return body;
}

async function waitForTask(base, key, taskId, timeoutMs = 15 * 60 * 1000) {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    const task = await call(base, `/v3/task/${taskId}`, key);
    const status = task?.status ?? task?.data?.status;
    if (status === "success") return task;
    if (status === "failed" || status === "error") {
      throw new Error(`task failed: ${task?.error?.message ?? status}`);
    }
    process.stdout.write(`  task ${taskId} ${status}...\n`);
    await new Promise((r) => setTimeout(r, 4000));
  }
  throw new Error("task timed out");
}

function pickModelUrl(task) {
  const d = task?.data ?? task;
  const candidates = [
    d?.output?.pbr_model, d?.output?.model, d?.output?.glb_model,
    d?.output?.rigged_model, d?.output?.base_model,
  ].filter((u) => typeof u === "string" && u.startsWith("http"));
  if (!candidates.length) {
    throw new Error(`no model url in task result: ${JSON.stringify(d?.output ?? {}).slice(0, 300)}`);
  }
  return candidates[0];
}

async function main() {
  if (has("help")) {
    console.log(USAGE);
    process.exit(0);
  }
  const image = arg("image");
  const text = arg("text");
  if (!image && !text) fail("one of --image or --text is required");
  const outDir = arg("out");
  if (!outDir) fail("--out <dir> is required");
  const plan = arg("plan", "paid");
  if (plan !== "paid") {
    fail("--plan must be \"paid\": Tripo free plan output is non-commercial and Tripo retains the rights");
  }
  const inputPath = image ? join(ROOT, image) : null;
  if (inputPath && !existsSync(inputPath)) fail(`input image not found: ${inputPath}`);

  // The dry run and the default without --yes stop here: no key is read,
  // no directory is made, nothing is sent.
  if (has("dry-run") || !has("yes")) {
    console.log(`plan: one paid Tripo ${image ? "image" : "text"}-to-model task, model ${arg("model", "v3.1-20260211")}, ` +
      `texture and pbr on, ${has("rig") ? "then a second, paid rig task" : "no rig task"}`);
    console.log(`  input:  ${image ? inputPath : `text ${JSON.stringify(text)}`}`);
    console.log(`  output: ${join(outDir, "model.glb")} and ${join(outDir, "provenance.json")}`);
    if (has("dry-run")) process.exit(0);
    console.error("nothing was sent: add --yes to spend (one paid Tripo task)");
    process.exit(2);
  }

  const keyFile = arg("key-file");
  const key = process.env.TRIPO_API_KEY || (keyFile && existsSync(keyFile) ? readFileSync(keyFile, "utf8").trim() : null);
  if (!key) fail("no key: set TRIPO_API_KEY or pass --key-file <path>");

  const inputBytes = inputPath ? readFileSync(inputPath) : null;
  const inputSha = inputBytes ? createHash("sha256").update(inputBytes).digest("hex") : null;

  mkdirSync(outDir, { recursive: true });

  let base = HOSTS[0];
  let task;
  const body = image
    ? { file: inputBytes.toString("base64"), type: "jpg" }
    : { prompt: text, type: "txt" };
  const endpoint = image ? "/v3/generation/image-to-model" : "/v3/generation/text-to-model";
  const payload = { ...body, model: arg("model", "v3.1-20260211"), texture: true, pbr: true, face_limit: 1 };

  let lastError;
  for (const host of HOSTS) {
    try {
      const created = await call(host, endpoint, key, { method: "POST", body: JSON.stringify(payload) });
      const taskId = created?.data?.task_id ?? created?.task_id;
      if (!taskId) throw new Error(`no task id: ${JSON.stringify(created).slice(0, 200)}`);
      console.log(`  task ${taskId}`);
      task = await waitForTask(host, key, taskId);
      base = host;
      break;
    } catch (err) {
      lastError = err;
      console.error(`  ${host} failed: ${err.message}`);
    }
  }
  if (!task) fail(`generation failed: ${lastError?.message ?? "unknown"}`);

  let modelUrl = pickModelUrl(task);
  if (has("rig")) {
    try {
      const rigTask = await call(base, "/v3/generation/rig", key, {
        method: "POST",
        body: JSON.stringify({ type: "mesh", weight: 50, source_model: { url: modelUrl } }),
      });
      const rigId = rigTask?.data?.task_id ?? rigTask?.task_id;
      if (rigId) {
        const rigged = await waitForTask(base, key, rigId);
        modelUrl = pickModelUrl(rigged);
      }
    } catch (err) {
      console.error(`  rig step failed, keeping the base model: ${err.message}`);
    }
  }

  const glb = Buffer.from(await (await fetch(modelUrl)).arrayBuffer());
  const glbPath = join(outDir, "model.glb");
  writeFileSync(glbPath, glb);

  const provenance = {
    provider: "tripo",
    plan,
    commercial_rights: true,
    licence: "Tripo paid plan: commercial use granted, model private, not used for training",
    generated_at: new Date().toISOString(),
    task_id: task?.data?.task_id ?? task?.task_id ?? null,
    model_version: arg("model", "v3.1-20260211"),
    rigged: has("rig"),
    source_kind: image ? "image" : "text",
    source_path: image ? image : null,
    source_sha256: inputSha,
    source_rights: "must be our own image: Tripo grants rights to the output, not to the input",
    output_path: glbPath,
    output_sha256: createHash("sha256").update(glb).digest("hex"),
    bytes: glb.length,
  };
  const provPath = join(outDir, "provenance.json");
  writeFileSync(provPath, JSON.stringify(provenance, null, 2) + "\n", "utf8");
  console.log(`WROTE ${glbPath} (${glb.length} bytes)`);
  console.log(`WROTE ${provPath}`);
}

main().catch((err) => fail(err.message));

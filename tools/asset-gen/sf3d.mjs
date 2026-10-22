// The one local 3D lane: Stability AI SF3D, run on this PC, no key, no cash.
//
//   node tools/asset-gen/sf3d.mjs --image concept.png --out content/generated/gunner
//   node tools/asset-gen/sf3d.mjs --image content/generated/concepts --out content/generated/batch
//
// What actually works on this PC (installed and proved 2026-10-21):
//   python: the uv-managed 3.10.20 (gpytoolbox 0.2.0 has no cp312/cp313
//   wheel, and this box has 3.12 and 3.13). Venv at <SF3D>\.venv.
//   torch: plain PyPI with --only-binary=:all:. The PyPI Windows wheel is
//   the CPU build, so this box runs the mesh on the CPU; download.pytorch.org
//   is unreliable from here (a 2.4 GB wheel arrived with a wrong SHA-256).
//   extensions: pip install --no-build-isolation, because their setup.py
//   imports torch and pip's build isolation hides it. USE_NATIVE_ARCH=0 and
//   CPPFLAGS=-include utility. Plain rembg, never rembg[gpu].
//   weights: tools/asset-gen/fetch-weights.mjs, which resumes across a bad
//   network and can pull the mirror. This tool passes the local directory
//   with --pretrained-model, so no hub call happens at run time.
//   real flags: run.py IMAGE --output-dir DIR --texture-resolution N
//               --foreground-ratio F --remesh_option none|triangle|quad
//               --batch_size N --device cuda|cpu --pretrained-model DIR
//   output: <out>/<n>/mesh.glb with UVs and material maps, and input.png.
//   VRAM floor about 6 GB: on a 6 GB card run --texture-resolution 512.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join, dirname, basename, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const SF3D = process.env.SF3D_DIR || join("C:\\", "forge-media", "models", "stable-fast-3d");
const PYTHON = process.env.SF3D_PYTHON || join(SF3D, ".venv", "Scripts", "python.exe");
function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
function fail(msg) {
  console.error(`sf3d: ${msg}`);
  process.exit(1);
}

// Local only: no key and no money, so there is no --yes. --help is answered
// before any path is checked or any python is run.
if (process.argv.includes("--help")) {
  console.log(`usage: node tools/asset-gen/sf3d.mjs --image <file|dir> --out <dir> [--texture 512] [--ratio 0.85]

  Local only: Stability AI SF3D on this PC, no key, no cash. Writes <out>/<name>.glb
  per image plus provenance.json. Needs the SF3D install (SF3D_DIR, SF3D_PYTHON) and
  the local weights (SF3D_WEIGHTS); it runs the mesh on this PC's GPU or CPU.
  --texture N   texture resolution (default 512)
  --ratio F     foreground ratio (default 0.85)
  --help        this text; exit 0`);
  process.exit(0);
}

const image = arg("image");
const outDir = arg("out");
if (!image || !outDir) fail("--image <file|dir> and --out <dir> are required");
const texture = arg("texture", "512");
const ratio = arg("ratio", "0.85");

if (!existsSync(join(SF3D, "run.py"))) {
  fail(
    "SF3D is not installed.\n" +
      "  git clone https://github.com/Stability-AI/stable-fast-3d " + SF3D + "\n" +
      "  cd " + SF3D + " && py -3.11 -m venv .venv && .venv\\Scripts\\pip install -U pip wheel ninja\n" +
      "  .venv\\Scripts\\pip install torch --index-url https://download.pytorch.org/whl/cu121\n" +
      "  .venv\\Scripts\\pip install -r requirements.txt\n" +
      "  set USE_NATIVE_ARCH=0 && .venv\\Scripts\\pip install ./texture_baker ./uv_unwrapper\n" +
      "  weights: accept the licence on huggingface.co/stabilityai/stable-fast-3d, then huggingface-cli login",
  );
}
if (!existsSync(PYTHON)) fail(`no python at ${PYTHON}: make the 3.11 venv, or set SF3D_PYTHON`);

const inputPath = isAbsolute(image) ? image : join(ROOT, image);
if (!existsSync(inputPath)) fail(`input not found: ${inputPath}`);
const isDir = statSync(inputPath).isDirectory();
const images = isDir
  ? readdirSync(inputPath).filter((n) => /\.(png|jpg|jpeg|webp)$/i.test(n)).map((n) => join(inputPath, n))
  : [inputPath];
if (!images.length) fail("no images found");

mkdirSync(outDir, { recursive: true });
const work = join(outDir, "sf3d-out");
mkdirSync(work, { recursive: true });

// Device: CUDA when this torch build has it, CPU otherwise. The PyPI wheel on
// Windows is the CPU build, so this box runs the mesh on the CPU; the Kaggle
// notebook runs the same code on a T4 for volume.
const probe = execFileSync(PYTHON, ["-c", "import torch; print('cuda' if torch.cuda.is_available() else 'cpu')"], { encoding: "utf8" });
const device = probe.trim().split(/\r?\n/).pop().trim();
console.log(`  device: ${device}`);

console.log(`  ${images.length} image(s), texture ${texture}`);

// Local weights when they are on disk: no hub call, no token, no licence
// check at run time. fetch-weights.mjs puts them here.
const WEIGHTS = process.env.SF3D_WEIGHTS || join("C:\\", "forge-media", "models", "sf3d-weights");
const haveWeights = existsSync(join(WEIGHTS, "model.safetensors"));
if (haveWeights) console.log(`  weights: ${WEIGHTS}`);
else console.log(`  weights: none in ${WEIGHTS}, the run will ask the hub`);

const run = execFileSync(
  PYTHON,
  [join(SF3D, "run.py"), ...images, "--output-dir", work, "--texture-resolution", texture,
   "--foreground-ratio", ratio, "--remesh_option", "none", "--batch_size", "1", "--device", device,
   ...(haveWeights ? ["--pretrained-model", WEIGHTS] : [])],
  { cwd: SF3D, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 60 * 60 * 1000, maxBuffer: 64 * 1024 * 1024,
    // The hub is never needed: the model weights are local and the DINOv2
    // encoder is cached. Asking the hub anyway means a stale cache lock
    // from an earlier run can block every run behind it, silently.
    env: { ...process.env,
           HF_HUB_OFFLINE: "1",
           TRANSFORMERS_OFFLINE: "1",
           HF_HUB_DISABLE_TELEMETRY: "1",
           HF_HUB_ENABLE_HF_TRANSFER: "0",
           SF3D_HF_TRANSFER: process.env.SF3D_HF_TRANSFER ?? "0" } },
);
console.log((run.stdout || "").split(/\r?\n/).slice(-10).join("\n"));

const meshes = [];
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (name.endsWith(".glb")) meshes.push(p);
  }
};
walk(work);
if (!meshes.length) fail("SF3D produced no mesh");

const written = [];
for (const mesh of meshes) {
  const name = basename(mesh).replace(/\.glb$/i, "") || "mesh";
  const dest = join(outDir, `${name}.glb`);
  const bytes = readFileSync(mesh);
  writeFileSync(dest, bytes);
  written.push({
    name,
    path: dest,
    bytes: bytes.length,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  });
  console.log(`WROTE ${dest} (${bytes.length} bytes)`);
}

writeFileSync(join(outDir, "provenance.json"), JSON.stringify({
  provider: "sf3d",
  lane: "local stable-fast-3d (the one local lane)",
  licence: "Stability AI Community Licence: commercial at or under $1M revenue, attribution required",
  attribution_required: "Powered by Stability AI",
  commercial_use: true,
  weights: "stabilityai/stable-fast-3d, fetched by tools/asset-gen/fetch-weights.mjs",
  model: "stabilityai/stable-fast-3d",
  weights_source: haveWeights ? WEIGHTS : "hub",
  texture_resolution: Number(texture),
  foreground_ratio: Number(ratio),
  source_images: images.map((p) => ({
    path: p,
    sha256: createHash("sha256").update(readFileSync(p)).digest("hex"),
  })),
  source_rights: "our own images only: a mesh made from someone else's picture is still derived from it",
  meshes: written,
  generated_at: new Date().toISOString(),
}, null, 2) + "\n", "utf8");
console.log(`WROTE ${join(outDir, "provenance.json")}`);
console.log("next: our Blender retopo, palette paint and 22-bone rig, then the engine cook");

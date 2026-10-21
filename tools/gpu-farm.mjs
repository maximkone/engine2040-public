// tools/gpu-farm.mjs: run one cargo line on a Kaggle GPU kernel.
//
// Why (owner 2026-10-21: "can we reuse kaggle for speedup of cpu or gpu use?"):
// the PC's GPU and cargo pool are shared by every session, so the GPU test legs
// wait. A Kaggle kernel gets a T4 (two per account) and about 30 GPU-hours a
// week, and the kernel API lets a session push, poll and pull the result with
// no browser in the loop.
//
// What it is for: the legs that need a real GPU (wgpu adapter tests, the live
// frame, the arena window) and the long renders the PC cannot afford. What it
// is NOT for: cargo builds (GitHub runners are free and faster), the golden
// (no GPU needed) and look-scoring captures (a T4 frame is not the local RTX
// 3050 frame, so it can never stand in for that reference).
//
// Usage:
//   node tools/gpu-farm.mjs --crate engine-render --filter auto_exposure --name gs44-meter
//   node tools/gpu-farm.mjs --crate engine-render --line "cargo test -p engine-render --features window --lib contact_shadow" --name gs43
//   node tools/gpu-farm.mjs --status <name>
//
// Credentials: the Kaggle CLI reads them itself (~/.kaggle/access_token, or
// kaggle.json, or KAGGLE_USERNAME and KAGGLE_KEY). The tool never reads a
// secret and never writes one into the tree.
//
// Owner: engine2040. Units: one kernel, one cargo line, one receipt.

import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const PUBLIC_REPO = "https://github.com/maximkone/engine2040-public";
const KERNEL_DIR = "target/gpu-farm";
const REPORT_DIR = "target/reports/gpu-farm";
const API = "https://api.kaggle.com/v1/kernels.KernelsApiService";

// The Kaggle credential on this PC is a bearer token in the user's Kaggle
// folder. The tool reads it to call the Kaggle API and never prints, stores
// or copies it; the kernels stay private.
function apiToken() {
  const file = join(homedir(), ".kaggle", "access_token");
  if (existsSync(file)) return readFileSync(file, "utf8").trim();
  return "";
}

async function apiCall(method, body) {
  const res = await fetch(`${API}/${method}`, {
    method: "POST",
    headers: { Authorization: `Bearer ${apiToken()}`, "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${method}: HTTP ${res.status}`);
  return res.json();
}

// Where the kernel is: RUNNING, COMPLETE, ERROR or anything else Kaggle says.
export async function kernelStatus(ref) {
  const [owner, slug] = ref.split("/");
  const data = await apiCall("GetKernelSessionStatus", { user_name: owner, kernel_slug: slug });
  return String(data.status ?? "UNKNOWN");
}

function parseArgs(argv) {
  const out = { name: "farm", crate: "", filter: "", line: "", repo: PUBLIC_REPO, ref: "main", status: "", watch: false, pull: false, dry: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--crate") out.crate = argv[++i] ?? "";
    else if (a === "--filter") out.filter = argv[++i] ?? "";
    else if (a === "--line") out.line = argv[++i] ?? "";
    else if (a === "--repo") out.repo = argv[++i] ?? "";
    else if (a === "--ref") out.ref = argv[++i] ?? "";
    else if (a === "--name") out.name = argv[++i] ?? "";
    else if (a === "--status") out.status = argv[++i] ?? "";
    else if (a === "--watch") out.watch = true;
else if (a === "--pull") out.pull = true;
    else if (a === "--dry") out.dry = true;
    else if (a === "--help" || a === "-h") out.help = true;
  }
  return out;
}

// The cargo line the kernel runs. One plain line: the runner refuses more.
export function cargoLine({ crate, filter, line }) {
  if (line) return line.trim();
  if (!crate) return "";
  return filter
    ? `cargo test -p ${crate} --lib ${filter}`
    : `cargo test -p ${crate} --lib`;
}

// The kernel body: a Linux image with a T4. Kaggle script kernels take
// python (bash was retired), so the shell work lives in a Python file that
// runs it and keeps the exit code.
export function kernelScript({ repo, ref, line }) {
  const bash = `set -uo pipefail
LOG=/kaggle/working/receipt.txt
: > "$LOG"
{
  echo "kernel: $(date -u +%Y-%m-%dT%H:%MZ)"
  echo "line: ${line}"
  echo "nvidia: $(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)"
  echo "---"
} >> "$LOG"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >> "$LOG" 2>&1
apt-get install -y -qq libvulkan1 libvulkan-dev mesa-vulkan-drivers libasound2-dev pkg-config libegl1 libgles2 >> "$LOG" 2>&1

# What the driver gives us: the Vulkan ICD and the EGL libraries decide
# whether wgpu can reach the T4 at all. Kaggle mounts the CUDA driver with
# compute capability only, so the graphics library may be missing; when it
# is there, the ICD file is written by hand and the T4 becomes a GPU rig.
{
  echo "--- gpu probe ---"
  echo "icd: $(ls /usr/share/vulkan/icd.d/ 2>/dev/null | tr '\\n' ' ')"
  echo "nvidia libs: $(ls /usr/local/lib /usr/lib/x86_64-linux-gnu 2>/dev/null | grep -i -E 'libGLX_nvidia|libEGL_nvidia' | tr '\\n' ' ')"
  echo "---"
} >> "$LOG"

for candidate in /usr/local/lib/libGLX_nvidia.so.0 /usr/lib/x86_64-linux-gnu/libGLX_nvidia.so.0; do
  if [ -f "$candidate" ]; then
    echo "nvidia vulkan icd: $candidate" >> "$LOG"
    printf '{\\n  "file_format_version": "1.0.0",\\n  "ICD": {\\n    "library_path": "%s",\\n    "api_version": "1.3.242"\\n  }\\n}\\n' "$candidate" > /usr/share/vulkan/icd.d/nvidia_icd.json
    ldconfig 2>> "$LOG"
    break
  fi
done

if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >> "$LOG" 2>&1
  source "$HOME/.cargo/env"
fi
export CARGO_INCREMENTAL=0
export RUST_BACKTRACE=1
export RUST_LOG=wgpu_core=info,wgpu_hal=info
export WGPU_BACKTRACE=1
export EGL_PLATFORM=surfaceless

# The clone lives outside the output folder: Kaggle lists every file of
# /kaggle/working, and a repo tree buries the receipt under thousands.
rm -rf /kaggle/working/engine2040 /tmp/engine2040
cd /tmp
if [ ! -d engine2040 ]; then
  git clone --depth 1 --branch ${ref} ${repo} engine2040 >> "$LOG" 2>&1 || {
    echo "branch ${ref} not found, taking the default branch" >> "$LOG"
    rm -rf engine2040
    git clone --depth 1 ${repo} engine2040 >> "$LOG" 2>&1 || {
      echo "clone failed" >> "$LOG"; cat "$LOG"; exit 3; }
  }
fi
cd engine2040
echo "--- commit ---" >> "$LOG"
git rev-parse --short HEAD >> "$LOG" 2>&1

echo "--- cargo ---" >> "$LOG"
bash -c "${line}" >> "$LOG" 2>&1
CODE=$?
echo "--- exit: $CODE ---" >> "$LOG"
cat "$LOG"
exit $CODE
`;
  return `# GPU farm kernel: one cargo line on a T4.
import pathlib
import subprocess
import sys

LOG = pathlib.Path("/kaggle/working/receipt.txt")
SCRIPT = r"""${bash.replace(/"""/g, '\\"\\"\\"')}"""

proc = subprocess.run(["bash", "-lc", SCRIPT])
text = LOG.read_text() if LOG.exists() else ""
with LOG.open("a") as handle:
    handle.write(f"--- kernel exit: {proc.returncode} ---\\n")
print(text[-4000:])
sys.exit(proc.returncode)
`;
}

// Kaggle slugs: lowercase, non-alphanumerics become hyphens, no doubles.
export function slugOf(text) {
  return String(text)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

// The Kaggle owner: from the pushed kernels on disk, from KAGGLE_OWNER, or
// from the owner given on the command line. Kaggle wants owner/slug.
export function ownerSlug(explicit = "") {
  if (explicit) return explicit;
  if (process.env.KAGGLE_OWNER) return process.env.KAGGLE_OWNER;
  const root = KERNEL_DIR;
  if (existsSync(root)) {
    const names = readdirSync(root).sort();
    for (const name of names.reverse()) {
      const meta = join(root, name, "kernel-metadata.json");
      if (!existsSync(meta)) continue;
      const id = JSON.parse(readFileSync(meta, "utf8")).id ?? "";
      const owner = id.split("/")[0];
      if (owner) return owner;
    }
  }
  return "";
}

export function kernelMetadata({ name, line, owner }) {
  const title = `engine2040 gpu farm ${name}`;
  const slug = slugOf(title);
  return {
    id: `${owner}/${slug}`,
    title,
    code_file: "main.py",
    language: "python",
    kernel_type: "script",
    is_private: true,
    enable_gpu: true,
    enable_internet: true,
    dataset_sources: [],
    kernel_sources: [],
    competition_sources: [],
    notebook_sources: [],
    source: line,
  };
}

// The kernel ref the API wants: owner/slug, read from the pushed metadata.
export function kernelRef(name, owner = "") {
  const meta = join(KERNEL_DIR, name, "kernel-metadata.json");
  if (existsSync(meta)) {
    const data = JSON.parse(readFileSync(meta, "utf8"));
    if (data.id) return data.id;
  }
  return `${owner}/${slugOf(`engine2040 gpu farm ${name}`)}`;
}

// The output files of a finished kernel session, with their download links.
export async function sessionOutput(ref) {
  const [owner, slug] = ref.split("/");
  const files = [];
  let raw = {};
  let pageToken = "";
  for (let page = 0; page < 20; page += 1) {
    const data = await apiCall("ListKernelSessionOutput", {
      user_name: owner,
      kernel_slug: slug,
      page_size: 200,
      page_token: pageToken,
    });
    raw = data;
    const walk = (value) => {
      if (Array.isArray(value)) {
        for (const item of value) walk(item);
        return;
      }
      if (value && typeof value === "object") {
        const name = value.name ?? value.fileName ?? value.title;
        const url = value.url ?? value.downloadUrl ?? value.download_url;
        if (name && url) files.push({ name: String(name), url: String(url) });
        for (const item of Object.values(value)) walk(item);
      }
    };
    walk(data.files ?? data.outputFiles ?? []);
    pageToken = String(data.nextPageToken ?? "");
    if (!pageToken) break;
  }
  return { files, raw };
}

export function reportPath(name) {
  return join(REPORT_DIR, `${name}.json`);
}

// What the kernel receipt says: the last `--- exit: N ---` line decides.
export function receiptVerdict(text) {
  if (!text) return "no-receipt";
  if (text.includes("--- exit: 0 ---")) return "pass";
  if (text.includes("--- kernel exit: 0 ---")) return "pass";
  if (text.includes("--- exit:") || text.includes("--- kernel exit:")) return "fail";
  return "incomplete";
}

// Pull the kernel output and write the receipt next to the reports. Kaggle
// lists every file of the working folder (the whole cloned tree), so only the
// receipt is downloaded.
export async function pullReceipt(name, line, dir) {
  const out = join(KERNEL_DIR, `${name}-out`);
  mkdirSync(out, { recursive: true });
  const { files, raw } = await sessionOutput(kernelRef(name));
  for (const file of files) {
    const base = file.name.split("/").pop() ?? file.name;
    if (base !== "receipt.txt") continue;
    const res = await fetch(file.url);
    if (!res.ok) continue;
    writeFileSync(join(out, base), Buffer.from(await res.arrayBuffer()));
  }
  if (typeof raw.log === "string" && raw.log) {
    writeFileSync(join(out, "kernel.log"), raw.log);
  }
  const receipt = join(out, "receipt.txt");
  const text = existsSync(receipt) ? readFileSync(receipt, "utf8") : "";
  mkdirSync(REPORT_DIR, { recursive: true });
  writeFileSync(
    reportPath(name),
    JSON.stringify({ name, line, state: receiptVerdict(text), receipt: text.slice(-4000) }, null, 1),
  );
  return text;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log("gpu-farm: push one cargo line to a Kaggle GPU kernel and pull the receipt.");
    console.log("  --crate <c> --filter <f> | --line <cargo line>  --name <n> [--repo <url>] [--ref <branch>]");
    console.log("  --status <n>  --watch  --pull  --dry");
    return 0;
  }
  const line = cargoLine(args);
  if (!line && !args.status && !args.watch && !args.pull) {
    console.error("gpu-farm: no cargo line (pass --crate, or --line)");
    return 2;
  }
  if (!apiToken()) {
    console.log("gpu-farm: NOOP, no Kaggle credential on this PC.");
    console.log("  the token lives in ~/.kaggle/access_token (the Kaggle CLI reads it too)");
    console.log(`  would run on a T4: ${line}`);
    return 2;
  }
  if (args.watch) {
    const dir = join(KERNEL_DIR, args.name);
    const ref = kernelRef(args.name);
    try {
      for (let i = 0; i < 180; i += 1) {
        const status = await kernelStatus(ref);
        console.log(`watch ${args.name}: ${status}`);
        if (/complete/i.test(status)) {
          const text = await pullReceipt(args.name, cargoLine(args), dir);
          console.log(`gpu-farm: ${args.name} -> ${reportPath(args.name)} (${receiptVerdict(text)})`);
          console.log(text.slice(-1500));
          return receiptVerdict(text) === "pass" ? 0 : 1;
        }
        if (/error/i.test(status)) {
          const text = await pullReceipt(args.name, cargoLine(args), dir);
          console.log(`gpu-farm: ${args.name} failed on Kaggle (${receiptVerdict(text)})`);
          console.log(text.slice(-1500));
          return 1;
        }
        await new Promise((resolve) => setTimeout(resolve, 60000));
      }
    } catch (e) {
      console.log(`gpu-farm: watch failed: ${String(e?.stack ?? e).slice(0, 400)}`);
      return 1;
    }
    console.log("gpu-farm: still running after 3 hours; pull it later");
    return 3;
  }
  if (args.status) {
    try {
      console.log(await kernelStatus(kernelRef(args.status)));
    } catch (e) {
      console.log(`gpu-farm: status failed: ${String(e?.message ?? e).trim()}`);
      return 2;
    }
    return 0;
  }
  if (args.pull) {
    const dir = join(KERNEL_DIR, args.name);
    try {
      const text = await pullReceipt(args.name, line, dir);
      console.log(`gpu-farm: ${args.name} -> ${reportPath(args.name)} (${receiptVerdict(text)})`);
      console.log(text.slice(-1200));
      return receiptVerdict(text) === "pass" ? 0 : 1;
    } catch (e) {
      console.log(`gpu-farm: pull failed: ${String(e?.message ?? e).trim()}`);
      return 2;
    }
  }
  const dir = join(KERNEL_DIR, args.name);
  mkdirSync(dir, { recursive: true });
  let owner = "";
  try {
    owner = ownerSlug();
  } catch (e) {
    console.log(`gpu-farm: cannot read the Kaggle owner: ${String(e.stdout ?? e.message).trim()}`);
    return 2;
  }
  writeFileSync(join(dir, "main.py"), kernelScript({ repo: args.repo, ref: args.ref, line }));
  writeFileSync(
    join(dir, "kernel-metadata.json"),
    JSON.stringify(kernelMetadata({ name: args.name, line, owner }), null, 1),
  );
  if (args.dry) {
    console.log(`gpu-farm: dry. kernel at ${dir}`);
    console.log(`gpu-farm: line ${line}`);
    return 0;
  }
  // The Kaggle CLI kills the parent process tree when it pushes a kernel on
  // this PC (the node process dies with exit 255 and no output, in every
  // stdio mode; the same command from the shell works). So the tool prepares
  // the kernel and prints the one line to run; the shell runs it.
  console.log(`gpu-farm: kernel ready at ${dir}`);
  console.log(`  run: kaggle kernels push -p ${dir}`);
  console.log(`  then: node tools/gpu-farm.mjs --watch --name ${args.name}`);
  return 0;
}

if (process.argv[1] && process.argv[1].endsWith("gpu-farm.mjs")) {
  try {
    process.exitCode = await main();
  } catch (e) {
    console.log(`gpu-farm: crashed: ${String(e?.stack ?? e).slice(0, 600)}`);
    process.exitCode = 1;
  }
}

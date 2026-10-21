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
// Credentials: KAGGLE_USERNAME and KAGGLE_KEY in the environment, or
// ~/.kaggle/kaggle.json. Without them the tool prints the line it would run
// and exits 2 (NOOP): no credential is ever written into the tree.
//
// Owner: engine2040. Units: one kernel, one cargo line, one receipt.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const PUBLIC_REPO = "https://github.com/maximkone/engine2040-public";
const KERNEL_DIR = "target/gpu-farm";
const REPORT_DIR = "target/reports/gpu-farm";

function parseArgs(argv) {
  const out = { name: "farm", crate: "", filter: "", line: "", repo: PUBLIC_REPO, ref: "master", status: "", dry: false };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--crate") out.crate = argv[++i] ?? "";
    else if (a === "--filter") out.filter = argv[++i] ?? "";
    else if (a === "--line") out.line = argv[++i] ?? "";
    else if (a === "--repo") out.repo = argv[++i] ?? "";
    else if (a === "--ref") out.ref = argv[++i] ?? "";
    else if (a === "--name") out.name = argv[++i] ?? "";
    else if (a === "--status") out.status = argv[++i] ?? "";
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

// The kernel script: a Linux image with a T4. It installs the toolchain, the
// graphics libraries the crate needs, clones the public mirror, runs the line
// and writes a receipt the tool pulls back.
export function kernelScript({ repo, ref, line }) {
  return `#!/bin/bash
# GPU farm kernel: one cargo line on a T4.
set -uo pipefail
LOG=/kaggle/working/receipt.txt
: > "$LOG"
{
  echo "kernel: $(date -u +%Y-%m-%dT%H:%MZ)"
  echo "line: ${line}"
  echo "nvidia: $(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)"
  echo "vulkan: $(command -v vulkaninfo >/dev/null && echo yes || echo no)"
  echo "---"
} >> "$LOG"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >> "$LOG" 2>&1
apt-get install -y -qq libvulkan1 libvulkan-dev mesa-vulkan-drivers libasound2-dev pkg-config >> "$LOG" 2>&1

if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >> "$LOG" 2>&1
  source "$HOME/.cargo/env"
fi
export CARGO_INCREMENTAL=0
export RUST_BACKTRACE=1

cd /kaggle/working
if [ ! -d engine2040 ]; then
  git clone --depth 1 --branch ${ref} ${repo} engine2040 >> "$LOG" 2>&1 || {
    echo "clone failed" >> "$LOG"; cat "$LOG"; exit 3; }
fi
cd engine2040

echo "--- cargo ---" >> "$LOG"
bash -c "${line}" >> "$LOG" 2>&1
CODE=$?
echo "--- exit: $CODE ---" >> "$LOG"
cat "$LOG"
exit $CODE
`;
}

export function kernelMetadata({ name, line }) {
  return {
    id: `maximkone/engine2040-gpu-farm-${name}`,
    title: `engine2040 gpu farm ${name}`,
    code_file: "run.sh",
    language: "bash",
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

function credentials() {
  const file = join(homedir(), ".kaggle", "kaggle.json");
  if (process.env.KAGGLE_USERNAME && process.env.KAGGLE_KEY) {
    return { user: process.env.KAGGLE_USERNAME, key: process.env.KAGGLE_KEY };
  }
  if (existsSync(file)) {
    const raw = JSON.parse(readFileSync(file, "utf8"));
    if (raw.username && raw.key) return { user: raw.username, key: raw.key };
  }
  return null;
}

function kaggle(args, opts = {}) {
  return execFileSync("kaggle", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], ...opts });
}

export function reportPath(name) {
  return join(REPORT_DIR, `${name}.json`);
}

// What the kernel receipt says: the last `--- exit: N ---` line decides.
export function receiptVerdict(text) {
  if (!text) return "no-receipt";
  if (text.includes("--- exit: 0 ---")) return "pass";
  if (text.includes("--- exit:")) return "fail";
  return "incomplete";
}

// Pull the kernel output and write the receipt next to the reports.
export function pullReceipt(name, line, dir) {
  const out = join(KERNEL_DIR, `${name}-out`);
  mkdirSync(out, { recursive: true });
  kaggle(["kernels", "output", dir, "-p", "-o", out]);
  const receipt = join(out, "receipt.txt");
  const text = existsSync(receipt) ? readFileSync(receipt, "utf8") : "";
  mkdirSync(REPORT_DIR, { recursive: true });
  writeFileSync(
    reportPath(name),
    JSON.stringify({ name, line, state: receiptVerdict(text), receipt: text.slice(-4000) }, null, 1),
  );
  return text;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log("gpu-farm: push one cargo line to a Kaggle GPU kernel and pull the receipt.");
    console.log("  --crate <c> --filter <f> | --line <cargo line>  --name <n> [--repo <url>] [--ref <branch>]");
    console.log("  --status <n>   --dry");
    return 0;
  }
  const line = cargoLine(args);
  if (!line) {
    console.error("gpu-farm: no cargo line (pass --crate, or --line)");
    return 2;
  }
  const creds = credentials();
  if (!creds) {
    console.log("gpu-farm: NOOP, no Kaggle credentials.");
    console.log("  needs KAGGLE_USERNAME and KAGGLE_KEY, or ~/.kaggle/kaggle.json");
    console.log(`  would run on a T4: ${line}`);
    return 2;
  }
  if (args.status) {
    const dir = join(KERNEL_DIR, args.status);
    try {
      console.log(kaggle(["kernels", "status", dir]));
    } catch (e) {
      console.log(`gpu-farm: status failed: ${String(e.stdout ?? e.message).trim()}`);
      return 2;
    }
    return 0;
  }
  if (args.pull) {
    const dir = join(KERNEL_DIR, args.name);
    try {
      const text = pullReceipt(args.name, line, dir);
      console.log(`gpu-farm: ${args.name} -> ${reportPath(args.name)} (${receiptVerdict(text)})`);
      console.log(text.slice(-1200));
      return receiptVerdict(text) === "pass" ? 0 : 1;
    } catch (e) {
      console.log(`gpu-farm: pull failed: ${String(e.stdout ?? e.stderr ?? e.message).trim()}`);
      return 2;
    }
  }
  const dir = join(KERNEL_DIR, args.name);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "run.sh"), kernelScript({ repo: args.repo, ref: args.ref, line }));
  writeFileSync(join(dir, "kernel-metadata.json"), JSON.stringify(kernelMetadata({ name: args.name, line }), null, 1));
  if (args.dry) {
    console.log(`gpu-farm: dry. kernel at ${dir}`);
    console.log(`gpu-farm: line ${line}`);
    return 0;
  }
  let pushOut = "";
  try {
    pushOut = kaggle(["kernels", "push", "-p", dir]);
  } catch (e) {
    const msg = String(e.stdout ?? "") + String(e.stderr ?? "");
    console.log(`gpu-farm: push failed: ${msg.trim()}`);
    mkdirSync(REPORT_DIR, { recursive: true });
    writeFileSync(reportPath(args.name), JSON.stringify({ name: args.name, line, state: "push-failed", error: msg.trim() }, null, 1));
    return 1;
  }
  console.log(pushOut.trim());
  console.log("gpu-farm: pushed. Poll with: node tools/gpu-farm.mjs --status " + args.name);
  return 0;
}

if (process.argv[1] && process.argv[1].endsWith("gpu-farm.mjs")) {
  process.exitCode = main();
}

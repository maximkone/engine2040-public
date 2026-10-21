// Keeps the weights fetch alive across network drops and machine restarts.
//
// The fetcher resumes from its .part file, so a restart costs nothing. This
// wrapper restarts the fetcher whenever it exits before the weights are
// complete, and the Windows task scheduler runs the wrapper on a timer, so a
// dead network or a rebooted machine recovers on its own.
//
//   node tools/asset-gen/fetch-weights-watch.mjs
import { spawn } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const FETCH = join(HERE, "fetch-weights.mjs");
const DEST = process.env.SF3D_WEIGHTS || "C:\\forge-media\\models\\sf3d-weights";
const STATE = join(DEST, "fetch-state.json");
const NEEDED = [
  { name: "config.yaml", size: 2405 },
  { name: "model.safetensors", size: 4024289892 },
];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function complete() {
  return NEEDED.every((t) => {
    const p = join(DEST, t.name);
    return existsSync(p) && statSync(p).size === t.size;
  });
}

function lastError() {
  try {
    const s = JSON.parse(readFileSync(STATE, "utf8"));
    return s["model.safetensors"]?.lastError ?? "unknown";
  } catch {
    return "no state file";
  }
}

// Another fetcher may already own the .part file. Do not fight it.
function lockedByOther() {
  const lock = join(DEST, "fetch.lock");
  if (!existsSync(lock)) return false;
  let pid = 0;
  try { pid = Number(readFileSync(lock, "utf8").trim()); } catch { pid = 0; }
  if (!pid) return false;
  try { process.kill(pid, 0); return true; } catch (err) { return err.code === "EPERM"; }
}

let attempt = 0;
while (!complete()) {
  attempt += 1;
  if (lockedByOther()) {
    console.log("[watch] another fetcher owns the download, waiting 60s");
    await sleep(60000);
    continue;
  }
  console.log(`[watch] start attempt ${attempt} (last error: ${lastError()})`);
  const child = spawn(process.execPath, [FETCH], { stdio: "inherit" });
  const code = await new Promise((resolve) => child.on("exit", resolve));
  if (complete()) {
    console.log("[watch] weights complete");
    process.exit(0);
  }
  console.log(`[watch] fetcher exited ${code}, waiting 20s before the next attempt`);
  await sleep(20000);
}
console.log("[watch] weights already complete");

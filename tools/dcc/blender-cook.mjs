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
import { createReadStream, existsSync, readFileSync } from "node:fs";
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
  return { code: 0, text: `ok ${version} ${seconds}` };
}

function flag(argv, name) {
  const at = argv.indexOf(name);
  return at >= 0 ? argv[at + 1] : undefined;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const argv = process.argv.slice(2);
  if (!argv.includes("--check")) {
    console.error("usage: node tools/dcc/blender-cook.mjs --check [--exe <path>] [--pin <file>]");
    process.exit(1);
  }
  const { code, text } = await check({
    exe: flag(argv, "--exe"),
    pinPath: flag(argv, "--pin") ?? join(HERE, "blender.pin.json"),
  });
  (code === 0 ? console.log : console.error)(text);
  process.exit(code);
}

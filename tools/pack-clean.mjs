// One-week compat shim: `pack-clean` lives in tools/ship.mjs now.
// Transparent: adds no output, forwards argv, propagates exit code.
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const r = spawnSync(process.execPath, [join(dirname(fileURLToPath(import.meta.url)), "ship.mjs"), "pack", ...process.argv.slice(2)], { stdio: "inherit" });
process.exit(r.status ?? 1);

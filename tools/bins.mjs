#!/usr/bin/env node
// bins: are the release binaries the runner, perf and walker seats run
// present and built after the last code commit? Prints one line per binary
// and, for each missing or stale one, the exact cargo command that rebuilds
// it. Runner and perf seats may run that command (the seat guard allows
// `cargo build --release` for them, through the cargo slots); every other
// seat reads the line and reports BLOCKED.
//   node tools/bins.mjs          lines + exit 0 when all are fresh, else 1
//   node tools/bins.mjs --json   the same as JSON
// 2026-10-10: target/ was wiped, staged_fight.exe and perf.exe were gone,
// and no seat owned rebuilding them, so runner and perf reported BLOCKED.
import { existsSync, statSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const ENV = "$env:CARGO_BUILD_JOBS='10'; $env:RUSTC_WRAPPER=''; "; // 10 jobs: one build holds the target lock alone, 2 left 10 of 12 cores idle (Maxim 2026-10-16)

export const BINS = [
  { exe: "engine2040", build: "cargo build --release -p engine2040 --bin engine2040" },
  { exe: "staged_fight", build: "cargo build --release -p engine-game --features app --bin staged_fight" },
  { exe: "perf", build: "cargo build --release -p engine-perf --bin perf" },
  { exe: "arena", build: "cargo build --release -p engine-game --features app --bin arena" },
  { exe: "e2040-editor", build: "cargo build --release -p engine-editor --features ui --bin e2040-editor" },
];

// Code commits since the binary was written (crates/ and the root manifests).
// Speed fast path (2026-10-12, builder-build-r1): one `git log` for the
// oldest binary instead of one per binary. Four binaries shared one history
// scan, so `node tools/bins.mjs` ran 4 git processes (~15 s on a loaded box).
// One scan with commit timestamps plus an in-JS per-binary count keeps the
// same STALE/FRESH shape with 1 process. Same output shape, same STALE-on-
// failure. Timestamps are second-granular, so the per-binary count errs
// STALE on a same-second edge (a needless rebuild, never a missed one).
const commitsSinceBatch = (oldestMs, root) => {
  const out = execFileSync("git", ["log", "--format=%ct", `--since=${new Date(oldestMs).toISOString()}`, "--", "crates", "Cargo.toml", "Cargo.lock", "src"], { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  return out.split("\n").map((l) => l.trim()).filter(Boolean).map(Number).filter(Number.isFinite);
};

export const binStatus = (root = ROOT, bins = BINS) => {
  const pending = bins.map(({ exe, build }) => {
    const path = join(root, "target", "release", `${exe}.exe`);
    if (!existsSync(path)) return { exe, path, state: "MISSING", commits: null, build: ENV + build };
    const ms = statSync(path).mtimeMs;
    return { exe, path, build, ms };
  });
  const existing = pending.filter((r) => r.state !== "MISSING");
  let stamps = [];
  let failed = false;
  if (existing.length > 0) {
    const oldest = Math.min(...existing.map((r) => r.ms));
    try {
      stamps = commitsSinceBatch(oldest, root);
    } catch (e) {
      console.error(`bins: git log failed for ${root}: ${e.message} -- marking STALE`);
      failed = true;
    }
  }
  return pending.map((r) => {
    if (r.state === "MISSING") return { exe: r.exe, path: r.path, state: "MISSING", commits: null, build: r.build };
    const ms = r.ms;
    const commits = failed ? 1 : stamps.filter((ct) => ct >= Math.floor(ms / 1000)).length;
    return { exe: r.exe, path: r.path, state: commits ? "STALE" : "FRESH", commits, built: new Date(ms).toISOString(), build: commits ? ENV + r.build : null };
  });
};

const main = () => {
  const rows = binStatus();
  if (process.argv.includes("--json")) {
    process.stdout.write(`${JSON.stringify(rows, null, 2)}\n`);
  } else {
    for (const r of rows) {
      const why = r.state === "STALE" ? ` (${r.commits} code commits since ${r.built})` : r.state === "FRESH" ? ` (built ${r.built})` : "";
      console.log(`${r.state.padEnd(7)} ${r.exe}${why}`);
      if (r.build) console.log(`  rebuild: ${r.build}`);
    }
  }
  process.exitCode = rows.every((r) => r.state === "FRESH") ? 0 : 1;
};

if (import.meta.main) main();

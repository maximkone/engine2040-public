// Lead brief: farm state + dirty-tree order + claims. Fast, no cargo.
// Usage: node tools/lead-brief.mjs  (prints <1.5KB, called by sprint-loop.mjs)
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const ROOT = join(import.meta.dirname ?? ".", "..");
const run = (cmd, args) => {
  try { return execFileSync(cmd, args, { cwd: ROOT, encoding: "utf8", timeout: 8000 }).trim(); }
  catch { return ""; }
};

// 1. Farm: first 11 lines of target/farm-last.txt (PASS/FAIL per job + run URL).
let farm = "farm: unknown (node tools/farm.mjs never ran)";
try {
  const t = readFileSync(join(ROOT, "target", "farm-last.txt"), "utf8").split("\n");
  farm = t.slice(0, 11).join("\n").slice(0, 900);
} catch { /* no farm yet */ }

// 2. Dirty tree: count + ordered paths (landing order = what to land/clean first).
const raw = run("git", ["status", "--short", "--untracked-files=all"]);
const dirty = raw ? raw.split("\n").filter(Boolean) : [];
const dirtyHead = dirty.slice(0, 15).join("\n");

// 3. Claims: active holders from target/queue/claims.txt tail (who owns which crate).
let claims = "claims: none";
try {
  const lines = readFileSync(join(ROOT, "target", "queue", "claims.txt"), "utf8").trim().split("\n");
  claims = lines.slice(-8).join("\n").slice(0, 600);
} catch { /* no claims */ }

// 4. Queue depth: ready packet count (fast readdir, no parse).
let ready = "?";
try { ready = String(readdirSync(join(ROOT, "target", "queue", "ready")).length); } catch { ready = "0"; }

console.log([
  `FARM: ${farm.split("\n")[0]}`,
  ...farm.split("\n").slice(1, 10).filter((l) => /^(PASS|FAILURE|RESULT)/.test(l)),
  `DIRTY: ${dirty.length} file(s)${dirty.length ? " (land oldest first):\n" + dirtyHead : " (tree clean)"}`,
  `CLAIMS (tail):\n${claims}`,
  `READY: ${ready} packet(s) in target/queue/ready/`,
  `RULE: heavy gates run on GitHub (farm + gh-check); locally run only: cargo check -p <crate> / cargo test -p <crate> --lib <one test>.`,
].join("\n"));

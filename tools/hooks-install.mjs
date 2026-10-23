// hooks-install: point git at the tracked hooks in tools/hooks.
// A fresh clone has core.hooksPath unset and only runs .git/hooks, so the
// rust-quality ratchet (pre-commit) and the licence gate (pre-push) are
// silently skipped. This tool sets `core.hooksPath=tools/hooks` in this repo's
// local config only: git runs the tracked scripts in place. It never copies
// into or overwrites .git/hooks, and on Unix it sets the hooks' executable bit.
//   node tools/hooks-install.mjs           install, then verify
//   node tools/hooks-install.mjs --check   report only; exit 1 when not installed
// Exit: 0 installed or clean, 1 --check found a gap, 2 refused or bad arguments.
// Refuses to install while a tracked hook is missing, so a broken tree cannot
// point git at a half-empty hooks dir.
// Drills run against a fixture repo with E2040_HOOKS_ROOT (hooks-install.test.mjs).
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const HOOKS_PATH = "tools/hooks";
export const HOOKS = Object.freeze(["pre-commit", "pre-push"]);
const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(process.env.E2040_HOOKS_ROOT ?? join(HERE, ".."));

const HELP = `usage: node tools/hooks-install.mjs [--check]

  no flag   set this repo's core.hooksPath to tools/hooks and verify both hooks
  --check   exit 1 when the hooks are not installed (changes nothing)
  --help    print this text, exit 0

  Git runs tools/hooks/pre-commit and tools/hooks/pre-push in place; nothing is
  copied into or overwritten under .git/hooks. On Unix the executable bit is set.
  Exit: 0 installed or clean, 1 --check found a gap, 2 refused or bad arguments.`;

const git = (root, args) =>
  spawnSync("git", ["-C", root, ...args], { encoding: "utf8", timeout: 60_000, maxBuffer: 8 * 1024 * 1024 });

// Git stores core.hooksPath either relative to the worktree root or absolute.
// Windows paths compare case-insensitively and may arrive with either slash.
function samePath(a, b) {
  const norm = (p) => resolve(p).replace(/\\/g, "/").replace(/\/+$/, "");
  const left = norm(a);
  const right = norm(b);
  return process.platform === "win32" ? left.toLowerCase() === right.toLowerCase() : left === right;
}

export function inspect(root = ROOT) {
  const hooksDir = join(root, HOOKS_PATH);
  const missing = HOOKS.filter((hook) => !existsSync(join(hooksDir, hook)));
  const base = { installed: false, reason: "", missing, hooksDir, configured: "" };
  const repo = git(root, ["rev-parse", "--is-inside-work-tree"]);
  if (repo.error || repo.status !== 0 || repo.stdout.trim() !== "true") {
    return { ...base, reason: "not a git work tree" };
  }
  const cfg = git(root, ["config", "--get", "core.hooksPath"]);
  const configured = cfg.status === 0 ? cfg.stdout.trim() : "";
  if (missing.length) return { ...base, reason: `tracked hooks missing: ${missing.join(", ")}`, configured };
  if (!configured) return { ...base, reason: "core.hooksPath is unset", configured };
  if (!samePath(resolve(root, configured), hooksDir)) {
    return { ...base, reason: `core.hooksPath is ${configured}, not ${HOOKS_PATH}`, configured };
  }
  return { installed: true, reason: "", missing, hooksDir, configured };
}

export function main(argv = process.argv.slice(2), { root = ROOT } = {}) {
  const flags = argv.filter((a) => a.startsWith("-"));
  const words = argv.filter((a) => !a.startsWith("-"));
  const unknown = flags.filter((f) => f !== "--check" && f !== "--help" && f !== "-h");
  if (unknown.length || words.length) {
    console.error(`unknown argument: ${[...unknown, ...words].join(" ")}`);
    console.error(HELP);
    return 2;
  }
  if (flags.includes("--help") || flags.includes("-h")) {
    console.log(HELP);
    return 0;
  }
  const before = inspect(root);
  if (flags.includes("--check")) {
    if (before.installed) {
      console.log(`hooks: installed (core.hooksPath=${HOOKS_PATH}; ${HOOKS.join(", ")})`);
      return 0;
    }
    console.log(`hooks: not installed (${before.reason}) — run node tools/hooks-install.mjs`);
    return 1;
  }
  if (before.reason === "not a git work tree") {
    console.error(`refused: ${root} is not a git work tree, so core.hooksPath has nowhere to point.`);
    return 2;
  }
  if (before.missing.length) {
    console.error(`refused: tracked hooks missing: ${before.missing.join(", ")} under ${HOOKS_PATH}; restore them, then install.`);
    return 2;
  }
  if (before.configured && before.configured !== HOOKS_PATH) {
    console.log(`note: replacing core.hooksPath=${before.configured} with ${HOOKS_PATH}`);
  }
  const set = git(root, ["config", "--local", "core.hooksPath", HOOKS_PATH]);
  if (set.error || set.status !== 0) {
    console.error(`refused: cannot set core.hooksPath: ${(set.stderr || set.error?.message || "").trim()}`);
    return 2;
  }
  if (process.platform !== "win32") {
    for (const hook of HOOKS) {
      const file = join(root, HOOKS_PATH, hook);
      const mode = statSync(file).mode & 0o777;
      if ((mode & 0o111) !== 0o111) chmodSync(file, mode | 0o111);
    }
  }
  const after = inspect(root);
  if (!after.installed) {
    console.error(`refused: install did not verify (${after.reason}).`);
    return 2;
  }
  const mode = process.platform === "win32" ? "" : ", executable";
  console.log(`hooks: installed (core.hooksPath=${HOOKS_PATH}; ${HOOKS.join(", ")}${mode})`);
  console.log("nothing was copied into .git/hooks; git runs the tracked hooks in place");
  return 0;
}

// Run the CLI only when this file is the entry point; tests import the names.
const isMain = process.argv[1] !== undefined
  && resolve(process.argv[1]).toLowerCase() === fileURLToPath(import.meta.url).toLowerCase();
if (isMain) process.exitCode = main();

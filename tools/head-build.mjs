// head-build: build the release arena from HEAD alone, for frame proofs of landed work.
//
// Owner 2026-10-24: agents ran 2+ hours, partly because each built its own release exe in a private
// snapshot dir (a cold full build: syn, serde, rapier) and the shared tree rarely builds while 20
// writers have half-done files in it. This is the one sanctioned HEAD build:
//
//   node tools/head-build.mjs            export HEAD to target/head-src, build, print the exe path
//   node tools/arena-run.mjs --exe target/head/arena-<sha>.exe -- <arena args>
//
// target/head-src keeps its own index (target/head-src.index), so each export rewrites only the
// files HEAD changed and cargo rebuilds only what they touch. The build shares target/ (registry
// crates stay warm) and the PC's compile-slot pool. The exe is copied to target/head/arena-<sha>.exe,
// so a dirty-tree build that lands in target/release later cannot be mistaken for it.
// Exit 0 built (or already built for this sha), 1 the build failed, 2 usage.
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const SRC = join(ROOT, "target", "head-src");
const OUT = join(ROOT, "target", "head");

const git = (args, env = {}) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28, env: { ...process.env, ...env } }).trim();

// Bring `src` to the tree of `rev`, writing only the files that differ from what its own index last
// wrote there (git read-tree -m -u against a private index; the shared index is never touched).
export const exportTree = (rev, src = SRC, repo = ROOT) => {
  mkdirSync(src, { recursive: true });
  const env = { ...process.env, GIT_INDEX_FILE: `${src}.index`, GIT_WORK_TREE: src };
  const run = (args, input) => execFileSync("git", ["--git-dir", join(repo, ".git"), "-c", "core.autocrlf=false", ...args], { cwd: src, encoding: "utf8", env, input, maxBuffer: 1 << 28 });
  if (!existsSync(`${src}.index`)) run(["read-tree", rev]);
  try { run(["update-index", "-q", "--refresh"]); } catch { /* a stale stat line only means read-tree compares content */ }
  // Nobody edits src, but a tool can write into it (git lfs rewrote tools/hooks/pre-push there on the
  // first export): put such a file back first, or read-tree -m -u refuses it as not up to date.
  const stray = run(["ls-files", "-m", "-z"]).split("\0").filter(Boolean);
  if (stray.length) run(["checkout-index", "-f", "-u", "-z", "--stdin"], stray.join("\0"));
  run(["read-tree", "-m", "-u", rev]);
  // A file missing from src (the first export, or one deleted by hand) is written; present ones are left
  // alone (checkout-index -a exits 1 on every file that already exists, so only the missing ones go to it).
  const missing = run(["ls-files", "-d", "-z"]).split("\0").filter(Boolean);
  if (missing.length) run(["checkout-index", "-u", "-z", "--stdin"], missing.join("\0"));
};

export const exeFor = (sha) => join(OUT, `arena-${sha}.exe`);

const main = () => {
  const sha = git(["rev-parse", "--short=9", "HEAD"]);
  const exe = exeFor(sha);
  if (existsSync(exe)) {
    console.log(`head-build: ${exe} is already built for HEAD ${sha}`);
    return 0;
  }
  exportTree("HEAD");
  console.log(`head-build: HEAD ${sha} exported to target/head-src; building the release arena (shared target/, slot pool)`);
  const cargo = spawnSync("cargo", ["build", "--release", "-p", "engine-game", "--features", "app", "--bin", "arena", "--manifest-path", join(SRC, "Cargo.toml")], {
    cwd: SRC,
    stdio: "inherit",
    env: { ...process.env, CARGO_TARGET_DIR: join(ROOT, "target") },
  });
  if (cargo.status !== 0) {
    console.log(`head-build: the build of HEAD ${sha} failed (exit ${cargo.status}): HEAD itself does not build`);
    return 1;
  }
  mkdirSync(OUT, { recursive: true });
  copyFileSync(join(ROOT, "target", "release", "arena.exe"), exe);
  console.log(`head-build: ${exe}`);
  console.log(`run it: node tools/arena-run.mjs --exe ${exe.slice(ROOT.length + 1).split("\\").join("/")} -- <arena args>`);
  return 0;
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main());

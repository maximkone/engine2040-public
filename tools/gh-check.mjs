// GitHub as the workforce (Maxim 2026-10-16, "make github our workforce"): run ONE narrow cargo command on a GitHub runner
// against the helper's working tree (HEAD plus every dirty and untracked file), without the PC's one build lock.
//   node tools/gh-check.mjs cargo test -p engine-net --lib interest_64_room_converges
//   node tools/gh-check.mjs cargo clippy -p engine-input --all-targets -- -D warnings
//   node tools/gh-check.mjs --wait wip-xxxx          pick up a check that was still running (exit 3 prints this line)
// How: the HEAD snapshot (denylist and LFS applied, like the public export) is built once per HEAD into the mirror clone's object
// store (from the nearest cached ancestor's base plus the diff, not from scratch); each call overlays the dirty files, commits that
// tree on a throwaway wip-* branch of the public mirror, dispatches .github/workflows/check.yml on it, waits, prints the command's
// output tail and deletes the branch. Up to 20 run at once, and calls may start together. A line like
// [local 12.3s: fetch 0.0 sweep 0.0 base 3.1 overlay 2.4 commit 0.1 push 5.0 dispatch 1.5] says where the local half went.
// GH_CHECK_DRY=1 stops after the push (no dispatch, the branch is deleted): it measures the local half for free.
// Exit 0 = the command passed, 1 = it failed (tail printed), 2 = the check could not run, 3 = still running (not a failure).
// On auto (the default; owner 2026-10-17 "all AIs work like that, heavy stuff on auto on git") the helper never has to decide:
//   - lines that need Windows or a window|app|ui|device feature only to build (every check and clippy, and tests whose code
//     opens no GPU or window) run on a Windows runner; only GPU, window and sound-device tests (a gpu/window/wgpu/... test
//     filter or target, a file that probes an adapter, device-feature tests) go to the PC: a runner has no adapter;
//   - reuse (owner 2026-10-21, "reuse the fuk out of it, save our poor cpu"): a result is kept by the line, the runner and the
//     bytes it builds (its crates with their dependencies, the manifests, the lock, content/ and fixtures/, and any path its code
//     names outside them). The same line on unchanged inputs returns the kept result at once (a pass for 24 h, a fail for
//     30 min; --fresh runs it again), and a line already running for another session is joined, not run twice;
//   - a run that could not start or finished with no command output is tried once more, then the same line runs on the PC
//     (never --workspace: that stays forbidden local);
//   - a cargo test that PASSED with 0 tests (a filter that only matches Windows-only or GPU tests) is not a proof: it runs on the PC.
//   --remote-only never falls back (exit 2 instead); --local skips GitHub; --budget <s> (default 100, env GH_CHECK_BUDGET) is how
//   long one call waits: OpenCode and Claude Code tool calls die at 120 s, so a longer check returns exit 3 with its --wait line
//   and the wip branch stays until a call sees the result (leaked branches older than 3 h are swept by the next call).
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { applyHunks, parseDiff } from "./commit-mine.mjs";
import { lowerPriority, poolEnv } from "./jobserver.mjs";
// Public-gate safe (2026-10-16): no static import of ./public-export.mjs here.
// That file is in its own export DENY list, so the public checkout lacks it and
// every public gate-tool test fails with ERR_MODULE_NOT_FOUND. The small pure
// pieces (PUBLIC_REPO + DENY/denied) are mirrored below; the heavy snapshot
// builders load lazily inside buildBase(). Mirror of tools/public-export.mjs
// DENY (audit 2026-10-16): if that list changes, copy it here.
const PUBLIC_REPO = "maximkone/engine2040-public";
const DENY = [
  ".opencode/", "opencode.jsonc", "arsenal.json", "orders.csv", "team/", "sprint/", "archive/", "clients/",
  "docs/archive/", "docs/ROYALTY.md", "docs/EMPIRE-INBOX.md", "AGENTS.md", "AGENTS.test.mjs", "CLAUDE.md",
  "tools/public-export.mjs", "tools/public-export.test.mjs",
];
const denied = (rel) => {
  if (/^docs\/archive\/SPRINT-done-[^/]*\.md$/.test(rel)) return false;
  return DENY.some((d) => (d.endsWith("/") ? rel.startsWith(d) : rel === d));
};

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const WORK = join(ROOT, "target", "gh-check");
const JOBS = join(WORK, "jobs");
const CLONE = join(ROOT, "target", "public-export", "repo");
const STARTED = Date.now();
const RUN_CAP_MS = 30 * 60_000; // check.yml times out at 40 min; nobody waits for a check longer than 30
const LEAK_MS = 3 * 3_600_000; // a wip branch or job file older than this is a leak of a killed call
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const run = (cmd, args, o = {}) => execFileSync(cmd, args, { encoding: "utf8", maxBuffer: 1 << 28, stdio: ["pipe", "pipe", "pipe"], ...o });
const gdir = () => join(CLONE, ".git");
const g = (args, o = {}) => run("git", ["--git-dir", gdir(), "-c", "core.autocrlf=false", ...args], o).trim();

// GitHub's rate limits (owner 2026-10-24: 20 agents dispatching at once got "HTTP 403: API rate limit exceeded"). A limited call
// is not a broken check: wait and try again. RATE_WAITS_MS are the waits between tries; POLL_MS is one run-status read.
export const RATE_WAITS_MS = [30_000, 60_000, 120_000];
const POLL_MS = 20_000;
export const rateLimited = (e) => /rate limit|secondary rate|HTTP 429|abuse detection/i.test(String(e?.stderr ?? e?.message ?? e));
export const withRateRetry = async (call, waits = RATE_WAITS_MS, wait = sleep) => {
  for (let i = 0; ; i++) {
    try { return await call(); } catch (e) {
      if (!rateLimited(e) || i >= waits.length) throw e;
      console.log(`GitHub rate limit; trying again in ${waits[i] / 1000} s`);
      await wait(waits[i]);
    }
  }
};

// One plain cargo check, test or clippy line, or one cargo-mutants slice (okMutants), goes to the runner; the workflow refuses the rest again.
export const okCmd = (cmd) => (/^cargo (check|test|clippy) /.test(cmd) || okMutants(cmd)) && !/[&;|`<>]|\$\(/.test(cmd);

// cargo-mutants (AF5, owner 2026-10-24): one crate and one slice of its mutants per line, never the whole workspace. The slice is
// --shard k/n or -f <file> (a plain path); -t/--timeout takes seconds. Anything else is refused: --workspace, --all, -o (the workflow
// reads mutants.out in the checkout), --in-place, a bare word, a `--` for the test binary.
const MUTANTS_FLAGS = new Set(["-p", "--package", "--shard", "-f", "--file", "-t", "--timeout"]);
export const okMutants = (cmd) => {
  const w = words(cmd);
  if (w.length < 4 || w[0] !== "cargo" || w[1] !== "mutants") return false;
  const pkgs = [];
  let slices = 0;
  for (let i = 2; i < w.length; i++) {
    const eq = /^(--[a-z]+)=(.*)$/.exec(w[i]); // --shard=1/8 is --shard 1/8
    const flag = eq ? eq[1] : w[i];
    if (!MUTANTS_FLAGS.has(flag)) return false;
    const val = eq ? eq[2] : w[++i];
    if (val === undefined || val.startsWith("-")) return false;
    if (flag === "-p" || flag === "--package") {
      if (!/^[a-z][a-z0-9_-]*$/.test(val)) return false;
      pkgs.push(val);
    } else if (flag === "--shard") {
      // cargo-mutants counts shards from 0: k/n with k < n (0/20 .. 19/20); 20/20 is its own error
      const s = /^(\d+)\/([1-9]\d*)$/.exec(val);
      if (!s || Number(s[1]) >= Number(s[2]) || Number(s[2]) > 256) return false;
      slices++;
    } else if (flag === "-f" || flag === "--file") {
      if (!/^[A-Za-z0-9_][A-Za-z0-9_./-]*$/.test(val) || val.split("/").includes("..")) return false;
      slices++;
    } else if (!/^[1-9]\d*$/.test(val)) return false;
  }
  return pkgs.length === 1 && slices > 0;
};
// A cargo-mutants line is heavy: it goes to a GitHub runner, never to this PC (not even as the fallback).
export const isMutants = (cmd) => words(cmd)[0] === "cargo" && words(cmd)[1] === "mutants";

// The words of a plain command line (a quoted run is one word, quotes removed).
export const words = (s) => (String(s).match(/"[^"]*"|'[^']*'|\S+/g) ?? []).map((w) => w.replace(/^(["'])(.*)\1$/, "$2"));

export const DEFAULT_BUDGET_S = 100;
// Printed by --help: every helper asks for the flags, so they are one command
// away instead of a read of this file.
export const USAGE = [
  "gh-check: run ONE cargo check|test|clippy line on a GitHub runner (or this PC) against your working tree, or one cargo mutants slice on GitHub.",
  "",
  "  node tools/gh-check.mjs [flags] cargo <check|test|clippy> <a cargo line>",
  "  node tools/gh-check.mjs [flags] cargo mutants -p <crate> --shard <k/n> | -f <file>   (one slice of one crate's mutants: GitHub only)",
  "  node tools/gh-check.mjs --wait wip-xxxx [--budget <s>]   pick up a check that was still running",
  "",
  "Flags (all before the cargo word):",
  "  --budget <s>     how long one call waits (default 100, max 1740). A short call returns exit 3 and a --wait line.",
  "  --only <paths>   build HEAD plus just these comma-separated paths; other writers' dirty files stay out.",
  "                   <path>#N takes only that hunk of a shared file (commit-mine --list numbers them): prove what you --pick",
  "  --at <sha>       build that commit alone, no working-tree file: bisect a test that is red on master",
  "  --fresh          run again instead of returning the kept result",
  "  --remote-only    never fall back to this PC (exit 2 instead)",
  "  --local          run on this PC in the shared compile pool, no GitHub",
  "  --wait <job>     read the result of a check an earlier call left running",
  "",
  "Exit: 0 pass, 1 fail, 2 could not run, 3 still running (not a failure). OpenCode shows every",
  "non-zero as 'Exited with code 1', so read the last line, not the number.",
  "",
  "Heavy work belongs on the runner; --local is the override for a line that needs this PC.",
].join("\n");
// node tools/gh-check.mjs [--budget <s>] [--remote-only | --local] [--fresh] [--only <path,path>] cargo <check|test|clippy> ...   |   --wait <job> [--budget <s>]
export const parseArgs = (argv, env = {}) => {
  const out = { cmd: "", wait: null, mode: "auto", fresh: false, only: null, at: null, budget: Number(env.GH_CHECK_BUDGET) >= 1 ? Math.min(Number(env.GH_CHECK_BUDGET), 1740) : DEFAULT_BUDGET_S, error: null };
  let i = 0;
  for (; i < argv.length && argv[i].startsWith("-"); i++) {
    const a = argv[i];
    if (a === "--wait") { out.wait = argv[++i] ?? ""; if (!/^wip-[a-z0-9]+$/.test(out.wait)) out.error = "--wait needs the job name the check printed (wip-...)"; }
    else if (a === "--budget") { const n = Number(argv[++i]); if (n >= 1) out.budget = Math.min(n, 1740); else out.error = "--budget needs seconds (1 to 1740)"; }
    else if (a === "--remote-only") out.mode = "remote";
    else if (a === "--local") out.mode = "local";
    else if (a === "--fresh") out.fresh = true;
    else if (a === "--only") {
      // Comma- or space-separated: every bare word up to `cargo` is a path (a space list was refused as the cargo line).
      const words = [String(argv[++i] ?? "")];
      while (i + 1 < argv.length && !argv[i + 1].startsWith("-") && argv[i + 1] !== "cargo") words.push(argv[++i]);
      out.only = words.flatMap((w) => w.split(",")).map((x) => x.trim().split("\\").join("/")).filter(Boolean);
      if (!out.only.length) out.error = "--only needs paths, comma-separated";
    }
    else if (a === "--at") { out.at = argv[++i] ?? ""; if (!/^[0-9a-f]{7,40}$/.test(out.at)) out.error = "--at needs a commit sha (7 to 40 hex)"; }
    else { out.error = `unknown flag ${a}`; break; }
  }
  if (out.at && out.only && !out.error) out.error = "--at builds that commit alone; it takes no --only";
  out.cmd = argv.slice(i).join(" ").trim();
  // cargo has none of these, so one after the cargo line is ours put in the wrong place: cargo would only fail on it remotely
  const rest = argv.slice(i); const dash = rest.indexOf("--");
  const late = (dash < 0 ? rest : rest.slice(0, dash)).find((a) => /^--(?:at|budget|fresh|only|wait|remote-only)$/.test(a));
  if (late && !out.error) out.error = `${late} is a gh-check flag: put it before the cargo line (node tools/gh-check.mjs ${late} ... cargo ...)`;
  return out;
};

// ---- preflight: what --only cannot know until the remote build fails ----
// Two --only mistakes cost a whole remote run (minutes) to learn, so both are refused here in under a second:
// a directory (an overlay entry must be a file), and a `mod x;` / `#[path = ".."]` whose file is in neither
// the --only list nor HEAD (the overlay then builds HEAD's copy without it and cargo fails with E0583).

// The file a `mod name;` in `file` resolves to, in Rust's own order: `dir/name.rs` then `dir/name/mod.rs`,
// where `dir` is the declaring file's folder except for `mod.rs`, `lib.rs` and `main.rs`, which own theirs.
export const modFileOf = (file, name) => {
  const p = file.split("\\").join("/");
  const base = /(?:^|\/)(?:mod|lib|main)\.rs$/.test(p) ? p.replace(/[^/]+$/, "") : `${p.replace(/\.rs$/, "")}/`;
  return [`${base}${name}.rs`, `${base}${name}/mod.rs`];
};

// Every module file a changed .rs declares, one candidate list per declaration: `mod x;` accepts `dir/x.rs`
// then `dir/x/mod.rs` (rustc's own order), and only both missing is a missing module. A comment line is
// skipped, so a commented-out `mod x;` is not a missing file.
export const declaredModFiles = (file, text) => {
  const out = [];
  const src = String(text).split("\n");
  const dir = file.split("\\").join("/").replace(/[^/]+$/, "");
  let pending = null; // a `#[path = "..."]` on its own line applies to the next `mod x;`
  for (let i = 0; i < src.length; i++) {
    const line = src[i];
    if (/^\s*(?:\/\/|\/\*|\*)/.test(line)) continue;
    const pathed = /#\[path\s*=\s*"([^"]+)"\]/.exec(line);
    const decl = /(?:^|\s)(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/.exec(line);
    if (!decl) { if (pathed) pending = pathed[1]; continue; }
    const path = pathed?.[1] ?? pending; pending = null;
    if (path) { out.push([dir + path.replace(/^\.\//, "")]); continue; }
    out.push(modFileOf(file, decl[1]));
  }
  return out;
};

// Problems that make an --only set fail remotely, each with the fix in the message. `headPaths` is the set of
// paths at HEAD (a null means read them from `root`). One git call, and only when a declaration names
// something the --only list does not already cover.
export const preflightOnly = (only, { root = ROOT, headPaths = null } = {}) => {
  const problems = [];
  const listed = (only ?? []).map((o) => String(o).replace(/#\d+$/, ""));
  for (const p of listed) {
    const abs = join(root, p);
    // A path that is gone is the caller's `D` entry, not a directory mistake.
    if (existsSync(abs) && statSync(abs).isDirectory()) {
      problems.push(`--only ${p} is a directory: --only takes files, so name the files you changed (git status --short ${p} lists them, and every new .rs your mod line needs)`);
    }
  }
  // Module files: only the changed .rs of this overlay can declare one, and only one the overlay lacks matters.
  const rs = [...new Set(listed.filter((p) => p.endsWith(".rs")))];
  if (!rs.length) return problems;
  // A `<path>#N` entry ships HEAD plus those hunks (pickedText), so only their `mod` lines count: the working-tree file also holds
  // other writers' `mod` lines, whose untracked files this overlay never carries (the preflight demanded them all, 2026-10-24).
  const picks = new Map();
  for (const o of only ?? []) { const m = /^(.*)#(\d+)$/.exec(String(o)); if (m) picks.set(m[1], [...(picks.get(m[1]) ?? []), Number(m[2])]); }
  const groups = [];
  for (const p of rs) {
    if (!existsSync(join(root, p)) || statSync(join(root, p)).isDirectory()) continue; // a removed or directory entry has no modules
    let text; try { text = picks.has(p) ? pickedText(p, picks.get(p), root) : readFileSync(join(root, p), "utf8"); } catch { continue; } // a bad #N fails later with its own message
    for (const deps of declaredModFiles(p, text)) {
      if (!deps.some((d) => listed.includes(d)) && !groups.some((g) => g.from === p && g.deps.join() === deps.join())) groups.push({ from: p, deps });
    }
  }
  if (!groups.length) return problems;
  const head = headPaths ?? headSet(root);
  for (const { from, deps } of groups) {
    if (deps.some((d) => head.has(d))) continue; // HEAD supplies it and the overlay leaves it alone
    problems.push(`--only ${from} declares a module at ${deps[0]}, which is in neither --only nor HEAD: add it (--only ${[...(only ?? []).map(String), deps[0]].join(",")})`);
  }
  return problems;
};

// Every path at HEAD, one call. A file only has to exist there: the overlay updates the listed ones and
// buildBase supplies the rest of HEAD.
let headCache = null;
const headSet = (root = ROOT) => {
  if (headCache && headCache.root === root) return headCache.set;
  const set = new Set(run("git", ["ls-tree", "-r", "--name-only", "HEAD"], { cwd: root }).split("\n").filter(Boolean));
  headCache = { root, set };
  return set;
};

// clippy without --no-deps lints every dependency too, so a foreign crate's warning turns a crate's own
// clippy line red. `-p` names the crate, so the flag is safe exactly there.
export const withNoDeps = (cmd) => {
  const c = String(cmd);
  if (!/^cargo\s+clippy\b/.test(c.trim())) return { cmd: c, added: false };
  const parts = c.trim().split(/\s+/);
  if (parts.includes("--no-deps")) return { cmd: c, added: false };
  if (!parts.some((w) => w === "-p" || w === "--package" || /^--package=|^-p./.test(w))) return { cmd: c, added: false };
  const dash = parts.indexOf("--");
  if (dash >= 0) parts.splice(dash, 0, "--no-deps"); else parts.push("--no-deps");
  return { cmd: parts.join(" "), added: true };
};

// ---- where a line runs: the Linux runner, the Windows runner, or the PC ----
// Cargo features that need a GPU, a window, a sound device or Windows itself (render `window`, game `app`, editor `ui`, audio
// `device`): the Linux runner cannot build them, the Windows runner can. tools/gh-check.test.mjs checks this list against every
// Cargo.toml.
export const LOCAL_FEATURES = new Set(["window", "app", "ui", "device", "gpu"]);
// A test filter or target naming one of these is a GPU or window test: only the PC has an adapter and a desktop (runners skip
// such tests and still "pass").
const GPU_WORD = /gpu|window|wgpu|winit|swapchain|adapter|vulkan|dx12/i;
const WIN_WORD = /win32/i;
// A source file that probes for an adapter or prints SKIPPED holds tests that skip on a runner; a Windows-only file holds tests
// that vanish on Linux and run on the Windows runner.
const GPU_SRC = /request_adapter|\bSKIPPED\b|\bno (?:GPU|adapter)\b/;
const WIN_SRC = /cfg\(windows\)|target_os\s*=\s*"windows"/;
const VALUE_FLAGS = new Set(["-p", "--package", "--bin", "--test", "--example", "--bench", "--features", "-F", "--target", "--profile", "--manifest-path", "-j", "--jobs", "--exclude", "--color", "--message-format", "--target-dir", "--config"]);
const NAME_FLAGS = new Set(["--bin", "--test", "--example", "--bench"]);
const walkRs = (dir, out = []) => {
  for (const e of existsSync(dir) ? readdirSync(dir, { withFileTypes: true }) : []) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walkRs(p, out); else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};
// The first source file of the packages that defines a test fn named like a filter and holds `pattern` code, else null.
const localSource = (pkgs, filters, root, pattern) => {
  if (!pkgs.length || !filters.length) return null;
  const base = join(root, "crates");
  for (const d of existsSync(base) ? readdirSync(base) : []) {
    let name; try { name = /^name\s*=\s*"([^"]+)"/m.exec(readFileSync(join(base, d, "Cargo.toml"), "utf8"))?.[1]; } catch { continue; }
    if (!pkgs.includes(name)) continue;
    for (const f of walkRs(join(base, d))) {
      const src = readFileSync(f, "utf8");
      if (pattern.test(src) && filters.some((x) => [...src.matchAll(/\bfn\s+(\w+)/g)].some((m) => m[1].includes(x)))) return f.slice(root.length + 1).split("\\").join("/");
    }
  }
  return null;
};
// { gpu, win }: gpu = why only the PC can prove the line; win = why it needs the Windows runner; both null = the Linux runner.
export const classify = (cmd, root = ROOT) => {
  const out = { gpu: null, win: null };
  const w = words(cmd);
  if (w[0] !== "cargo") return out;
  const pkgs = [], filters = [];
  let afterDash = false, device = false;
  for (let i = 2; i < w.length; i++) {
    const t = w[i];
    if (afterDash) { if (!t.startsWith("-")) filters.push(t); continue; }
    if (t === "--") { afterDash = true; continue; }
    if (t === "--all-features") { out.win ??= "--all-features builds the window and device features (Windows builds them)"; device = true; continue; }
    const feat = t === "--features" || t === "-F" ? w[i + 1] : /^--features=|^-F./.test(t) ? t.replace(/^--features=|^-F/, "") : null;
    if (feat) {
      const hit = feat.split(/[,\s]+/).find((f) => LOCAL_FEATURES.has(f.split("/").pop()));
      if (hit) out.win ??= `feature ${hit} needs a GPU, a window or Windows to build (the Windows runner builds it)`;
      device ||= feat.split(/[,\s]+/).some((f) => f.split("/").pop() === "device");
    }
    const named = NAME_FLAGS.has(t) ? w[i + 1] : /^--(?:bin|test|example|bench)=/.test(t) ? t.split("=")[1] : null;
    if (named && GPU_WORD.test(named)) out.gpu ??= `target ${named} is a GPU or window test`;
    else if (named && WIN_WORD.test(named)) out.win ??= `target ${named} is a Windows-only test`;
    if (t === "-p" || t === "--package") { pkgs.push(w[++i] ?? ""); continue; }
    if (/^--package=/.test(t)) { pkgs.push(t.slice(10)); continue; }
    if (VALUE_FLAGS.has(t)) { i++; continue; }
    if (!t.startsWith("-")) filters.push(t);
  }
  if (w[1] !== "test" || w.includes("--no-run")) return out; // compiling only: the features above are all that matters
  if (device) out.gpu ??= "feature device tests need a sound device (runners have none)";
  const hit = filters.find((f) => GPU_WORD.test(f));
  if (hit) out.gpu ??= `test filter ${hit} is a GPU or window test`;
  const winHit = filters.find((f) => WIN_WORD.test(f));
  if (winHit) out.win ??= `test filter ${winHit} is a Windows-only test`;
  const gpuFile = localSource(pkgs, filters, root, GPU_SRC);
  if (gpuFile) out.gpu ??= `${gpuFile} holds GPU or window code (its tests skip on a runner)`;
  const winFile = localSource(pkgs, filters, root, WIN_SRC);
  if (winFile) out.win ??= `${winFile} holds Windows-only code (its tests vanish on Linux)`;
  return out;
};
// null = the line may run on the Linux runner, else why not (a feature or Windows reason first, then a GPU one).
export const localOnly = (cmd, root = ROOT) => { const c = classify(cmd, root); return c.win ?? c.gpu; };
// The runner a line goes to when it may leave the PC.
export const runnerOs = (cmd, root = ROOT) => (classify(cmd, root).win ? "windows-latest" : "ubuntu-latest");

// Tests that passed over all `test result:` lines of a cargo test log; null when the log has none.
export const testsRan = (log) => {
  let n = null;
  for (const m of String(log).matchAll(/test result: (?:ok|FAILED)\. (\d+) passed/g)) n = (n ?? 0) + Number(m[1]);
  return n;
};

// The source files a cargo error tail points at (`--> path:line`), slash-normalized, in order.
export const errorFiles = (text) => [...new Set([...String(text).matchAll(/-->\s+([^\s:]+\.rs):\d+/g)].map((m) => m[1].split("\\").join("/")))];
// Those of them that are dirty (uncommitted) in this tree: the likely cause when someone
// else's half-done edit broke the build of everyone's line (owner 2026-10-24: most runs that
// day failed on other agents' work in progress).
export const dirtyErrorFiles = (text, root = ROOT) => {
  const files = errorFiles(text);
  if (!files.length) return [];
  let status = "";
  try { status = run("git", ["-C", root, "status", "--porcelain", "--", ...files]); } catch { return []; }
  const dirty = new Set(status.split("\n").filter(Boolean).map((l) => l.slice(3).trim().split("\\").join("/")));
  return files.filter((f) => dirty.has(f));
};

// Crates that keep their tests behind a feature (owner 2026-10-24: a "PASS, 0 tests ran" on these
// crates was the usual trap). Kept small on purpose: `app` sends a line to the slower Windows
// runner, so the feature is added only after a run without it proved nothing.
export const TEST_FEATURES = { "engine-physfx": "rapier", "engine-game": "app", "engine-endpoint": "mcp" };
// The same `cargo test` line with its crate's test feature added, or null when the line already
// names features, is not a test of one of those crates, or has nothing to add.
export const withTestFeatures = (cmd) => {
  const w = words(cmd);
  if (w[0] !== "cargo" || w[1] !== "test" || w.some((t) => /^(?:--features|-F|--all-features)(?:=|$)|^-F./.test(t))) return null;
  const at = w.findIndex((t) => t === "-p" || t === "--package");
  const feat = at >= 0 ? TEST_FEATURES[w[at + 1]] : undefined;
  if (!feat) return null;
  const dash = cmd.indexOf(" -- ");
  return dash >= 0 ? `${cmd.slice(0, dash)} --features ${feat}${cmd.slice(dash)}` : `${cmd} --features ${feat}`;
};

// ---- the route: GitHub first, the PC second ----
// remote(cmd, os) -> { code, retry?, why?, noTests? }; local(cmd) -> code. Pure over those two, so the tests fake them.
export const route = async ({ cmd, mode = "auto", remote, local, say = console.log, root = ROOT }) => {
  const c = classify(cmd, root);
  const forced = mode === "local" ? "--local given" : mode === "auto" ? c.gpu : null;
  if (forced) { say(`local, not GitHub: ${forced}`); return local(cmd); }
  const os = c.win ? "windows-latest" : "ubuntu-latest";
  if (c.win) say(`Windows runner: ${c.win}`);
  let res = { code: 2 };
  for (let attempt = 1; attempt <= 2; attempt++) {
    res = await remote(cmd, os);
    if (res.code !== 2 || !res.retry) break;
    if (attempt < 2) say(`the check could not run${res.why ? ` (${res.why})` : ""}; trying once more`);
  }
  // 0 tests ran on a crate whose tests sit behind a feature: run the line once more with it.
  const featured = res.code === 0 && res.noTests ? withTestFeatures(cmd) : null;
  if (featured) {
    say(`0 tests ran: this crate keeps its tests behind a feature; running \`${featured}\``);
    const again = await remote(featured, runnerOs(featured, root));
    return afterRemote({ res: again, cmd: featured, mode, local, say });
  }
  return afterRemote({ res, cmd, mode, local, say });
};
// A finished remote result into the exit code: a vacuous pass or a run that could not happen goes to the PC unless --remote-only.
export const afterRemote = async ({ res, cmd, mode, local, say }) => {
  if (res.code === 0 && res.noTests) {
    say("PASS but 0 tests ran on the runner: the filter matches only GPU, window or Windows-only tests (or nothing), so it proves nothing");
    if (mode === "remote") return 2;
    say("running it on the PC instead"); return local(cmd);
  }
  if (res.code === 2 && mode !== "remote") {
    if (isMutants(cmd)) { say("GitHub could not run it; cargo mutants never runs on this PC"); return 2; }
    say("GitHub could not run it; running it on the PC instead"); return local(cmd);
  }
  return res.code;
};

// The same line on this PC: stdio straight through, cargo's own lock decides when it runs. Never workspace-wide.
// A kept result of the same line on the same inputs answers at once; the same line already building here is waited for.
export const runLocal = (cmd, say = console.log, { fresh = false, root = ROOT } = {}) => {
  if (isMutants(cmd)) { say("refused: cargo mutants never runs on this PC (it is heavy): it goes to a GitHub runner, one -p crate and one --shard k/n per line"); return 2; }
  if (/\s--(?:workspace|all)(?=\s|$)/.test(cmd)) { say("refused: a workspace-wide cargo line never runs on this PC; give one crate (-p <crate>) or fix what stopped GitHub"); return 2; }
  const key = reuseKey(cmd, "pc", root);
  const lock = key && join(RESULTS, `${key}.lock`);
  if (key && !fresh) {
    for (let waited = 0; ownerAlive(lock) && waited < RUN_CAP_MS; waited += 5000) {
      if (!waited) say(`the same line on the same inputs is already building on this PC (pid ${readFileSync(lock, "utf8").trim()}): waiting for its result`);
      sleepSync(5000);
    }
    const kept = keptResult(key);
    if (kept) { sayKept(kept, say); return kept.code; }
  }
  let mine = false;
  if (lock) { mkdirSync(RESULTS, { recursive: true }); try { writeFileSync(lock, String(process.pid), { flag: "wx" }); mine = true; } catch { /* a racing twin: run anyway */ } }
  try {
    say(`[local] ${cmd}`);
    const [bin, ...args] = words(cmd);
    lowerPriority();
    const r = spawnSync(bin, args, { cwd: root, stdio: "inherit", env: { ...process.env, ...poolEnv() } });
    // A child that was killed has no exit code and carries a signal: that is not a build failure. On this PC a
    // long build run as a supervised agent command dies that way, and the call would end at exit 255 with nothing
    // printed. Say it here, so the log says what happened.
    if (r.status === null && r.signal) say(`the build was killed (${r.signal}): a long build must run detached, see skills/github-check/SKILL.md`);
    const code = r.error ? 2 : r.status === 0 ? 0 : 1;
    if (key && reuseKey(cmd, "pc", root) === key) keepResult(key, { code, where: "pc", cmd });
    return code;
  } finally { if (mine) rmSync(lock, { force: true }); }
};

// ---- reuse: a result is kept by what the line builds ----
const RESULTS = join(WORK, "results");
// A pass is kept a day; a fail half an hour (a flake or the PC's load can turn it).
export const KEEP_MS = { 0: 24 * 3_600_000, 1: 30 * 60_000 };
// Read by every crate build besides its own folder; content/ and fixtures/ are read by include_str! and by tests under computed
// names, so they always count.
const KEY_FILES = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "rust-toolchain.toml"];
const KEY_DIRS = ["content", "fixtures"];
const SKIP_DIRS = new Set(["target", "node_modules", ".git"]);

// Workspace packages of crates/*: name -> { dir, deps: { normal, dev } } (build deps count as normal; renamed deps by `package`).
export const crateGraph = (root = ROOT) => {
  const g = new Map(); const base = join(root, "crates");
  for (const d of existsSync(base) ? readdirSync(base) : []) {
    let text; try { text = readFileSync(join(base, d, "Cargo.toml"), "utf8"); } catch { continue; }
    const name = /^\s*name\s*=\s*"([^"]+)"/m.exec(text)?.[1];
    if (name) g.set(name, { dir: `crates/${d}`, text });
  }
  for (const v of g.values()) {
    const normal = new Set(), dev = new Set();
    let kind = null;
    for (const line of v.text.split(/\r?\n/)) {
      const h = /^\s*\[([^\]]+)\]/.exec(line);
      if (h) {
        const m = /(?:^|\.)(dependencies|dev-dependencies|build-dependencies)(?:\.(.+))?$/.exec(h[1].replace(/[\s"']/g, ""));
        kind = m ? (m[1] === "dev-dependencies" ? dev : normal) : null;
        if (m?.[2]) { if (g.has(m[2])) kind.add(m[2]); kind = null; } // [dependencies.engine-x]: its keys are not deps
        continue;
      }
      if (!kind) continue;
      const k = /^\s*([A-Za-z0-9_-]+)\s*[.=]/.exec(line)?.[1];
      if (k && g.has(k)) kind.add(k);
      const renamed = /\bpackage\s*=\s*"([^"]+)"/.exec(line)?.[1];
      if (renamed && g.has(renamed)) kind.add(renamed);
    }
    v.deps = { normal, dev }; delete v.text;
  }
  return g;
};

// The folders a line builds: its packages and their dev deps, then normal deps all the way down. null = not keyable.
export const closureDirs = (pkgs, g) => {
  if (!pkgs.length || pkgs.some((p) => !g.has(p))) return null;
  const seen = new Set(pkgs); const stack = [...pkgs];
  for (const p of pkgs) for (const d of g.get(p).deps.dev) if (!seen.has(d)) { seen.add(d); stack.push(d); }
  while (stack.length) for (const d of g.get(stack.pop()).deps.normal) if (!seen.has(d)) { seen.add(d); stack.push(d); }
  return [...seen].map((p) => g.get(p).dir).sort();
};

// Paths the crates' code names outside their folders ("../../docs/x.md" in an include_str! or a test's read): part of the key.
const outsidePaths = (dirs, root) => {
  const out = new Set();
  const inside = (rel) => dirs.some((d) => rel === d || rel.startsWith(`${d}/`));
  for (const d of dirs) {
    for (const f of walkRs(join(root, d))) {
      for (const m of readFileSync(f, "utf8").matchAll(/"((?:\.\.\/)+[^"\s{}]+)"/g)) {
        for (const base of [dirname(f), join(root, d)]) {
          const rel = relative(root, resolve(base, m[1])).split("\\").join("/");
          if (!rel.startsWith("..") && !inside(rel) && existsSync(join(root, rel))) out.add(rel);
        }
      }
    }
  }
  return [...out];
};

// path:size:mtime of every file under the paths, sorted and hashed: an edit anywhere in them changes it.
const fingerprint = (root, paths) => {
  const rows = [];
  const walk = (rel) => {
    let st; try { st = statSync(join(root, rel)); } catch { rows.push(`${rel}:-`); return; }
    if (!st.isDirectory()) { rows.push(`${rel}:${st.size}:${st.mtimeMs}`); return; }
    for (const e of readdirSync(join(root, rel))) if (!SKIP_DIRS.has(e)) walk(`${rel}/${e}`);
  };
  for (const p of paths) walk(p);
  const h = createHash("sha256");
  for (const r of rows.sort()) h.update(`${r}\n`);
  return h.digest("hex");
};

// The reuse key of a line on a runner ("pc", "ubuntu-latest", "windows-latest"), or null for a line without -p (the workspace
// or the root package: too wide to key).
export const reuseKey = (cmd, where, root = ROOT) => {
  const w = words(cmd); const pkgs = [];
  for (let i = 2; i < w.length && w[i] !== "--"; i++) {
    if (w[i] === "-p" || w[i] === "--package") pkgs.push(w[++i] ?? "");
    else if (/^--package=/.test(w[i])) pkgs.push(w[i].slice(10));
    else if (/^--(?:workspace|all|manifest-path)(?:=|$)/.test(w[i])) return null;
  }
  let dirs; try { dirs = closureDirs(pkgs, crateGraph(root)); } catch { return null; }
  if (!dirs) return null;
  const paths = [...KEY_FILES, ...KEY_DIRS, ...dirs, ...outsidePaths(dirs, root)];
  return createHash("sha256").update(`${where}\n${w.join(" ")}\n${fingerprint(root, paths)}`).digest("hex").slice(0, 32);
};

const resultPath = (key) => join(RESULTS, `${key}.json`);
export const keptResult = (key, now = Date.now()) => {
  try { const r = JSON.parse(readFileSync(resultPath(key), "utf8")); return now - r.at < (KEEP_MS[r.code] ?? 0) ? r : null; } catch { return null; }
};
export const keepResult = (key, r) => {
  if (!key || !(r.code in KEEP_MS) || r.noTests) return;
  mkdirSync(RESULTS, { recursive: true });
  const tmp = `${resultPath(key)}.${process.pid}`;
  writeFileSync(tmp, JSON.stringify({ ...r, at: Date.now() })); renameSync(tmp, resultPath(key));
};
const sayKept = (r, say) => {
  if (r.tail) say(r.tail);
  say(`REUSED ${r.code === 0 ? "PASS" : "FAIL"} from ${Math.round((Date.now() - r.at) / 60_000)} min ago (${r.where}${r.url ? `, run ${r.url}` : ""}): nothing this line builds changed since. --fresh runs it again.`);
};
const ownerAlive = (lock) => {
  let pid; try { pid = Number(readFileSync(lock, "utf8")); } catch { return false; }
  try { process.kill(pid, 0); return true; } catch (e) { return e.code === "EPERM"; }
};
const sleepSync = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

// ---- the runner ----
const withIndex = (name, fn, work = WORK) => {
  const idx = join(work, `${name}.index`); mkdirSync(work, { recursive: true }); rmSync(idx, { force: true });
  try { return fn({ GIT_INDEX_FILE: idx }); } finally { rmSync(idx, { force: true }); }
};

// Housekeeping one call does for all (a fetch, a sweep): at most once per everyMs, never two at once, the other calls skip it.
// fn returning false (a failed fetch) does not count as done.
const oncePer = async (name, everyMs, fn) => {
  const stamp = join(WORK, `${name}.at`), lock = join(WORK, `${name}.lock`); mkdirSync(WORK, { recursive: true });
  try { if (Date.now() - statSync(stamp).mtimeMs < everyMs) return false; } catch { /* never done */ }
  if (existsSync(lock) && !ownerAlive(lock)) rmSync(lock, { force: true });
  try { writeFileSync(lock, String(process.pid), { flag: "wx" }); } catch { return false; }
  try { const r = await fn(); if (r === false) return false; writeFileSync(stamp, ""); return true; } finally { rmSync(lock, { force: true }); }
};

// ---- the base snapshot: HEAD as a git tree object inside the mirror clone, built once per HEAD by the first caller ----
// A dozen sessions commit every few minutes, so nearly every call meets a new HEAD. Rebuilding the whole public tree for it (copy,
// denylist, LFS, scan, add: 13 to 20 s) was a quarter of the local half, so a new HEAD starts from the nearest ancestor's cached
// base and replays `git diff` through the same per-file rules (public-export.mjs buildPaths). tools/gh-check.test.mjs proves that
// both ways give one tree id.
export const BASE_LIMIT = 2000; // more changed paths than this: the whole build is as fast and has nothing to get wrong
const BASE_FORMAT = 1; // bump it when the incremental build changes: older bases stop being starting points
const STEER = /(?:^|\/)\.git(?:ignore|attributes)$/; // they decide what `git add` skips and converts: a change to one rebuilds everything
// The exporter's text plus BASE_FORMAT: a base built under other rules is never a starting point.
const rulesStamp = () => `${BASE_FORMAT}:${createHash("sha256").update(readFileSync(join(dirname(fileURLToPath(import.meta.url)), "public-export.mjs"), "utf8").replace(/\r\n/g, "\n")).digest("hex").slice(0, 16)}`;
const ctxNow = () => ({ root: ROOT, work: WORK, gd: gdir() });
const refuseSecrets = (s) => { const secret = s.problems.filter((p) => /secret|token|key/i.test(p)); if (secret.length) throw new Error(`snapshot refused: ${secret[0]}`); };
const treeKnown = (gd, tree) => { try { run("git", ["--git-dir", gd, "cat-file", "-e", `${tree}^{tree}`]); return true; } catch { return false; } };

// The newest cached base (made under today's rules, every LFS object resolved) whose commit is an ancestor of `full`: its .meta, or null.
const nearestBase = (ctx, full, stamp) => {
  const metas = new Map();
  for (const f of existsSync(ctx.work) ? readdirSync(ctx.work) : []) {
    if (!/^base-[0-9a-f]+\.meta$/.test(f)) continue;
    try { const m = JSON.parse(readFileSync(join(ctx.work, f), "utf8")); if (m.rules === stamp && m.lfsMissing === 0 && m.full && m.full !== full) metas.set(m.full, m); } catch { /* a half-written meta */ }
  }
  if (!metas.size) return null;
  for (const c of run("git", ["rev-list", "--max-count=1500", full], { cwd: ctx.root }).split("\n")) { const m = metas.get(c); if (m && treeKnown(ctx.gd, m.tree)) return m; }
  return null;
};

// The paths of `files` that git add -A would skip (ignored), judged by the .gitignore files buildPaths extracted into `tree`.
const ignoredPaths = (ctx, tree, files) => {
  if (!files.length) return [];
  try { return run("git", ["--git-dir", ctx.gd, "--work-tree", tree, "check-ignore", "--no-index", "--stdin", "-z"], { cwd: tree, input: `${files.join("\0")}\0` }).split("\0").filter(Boolean); }
  catch (e) { if (e.status === 1) return []; throw e; }
};

// prev.tree plus the diff prev..full, null when a whole build is the better answer. Same rules per file as the whole build: the files go through
// buildPaths (denylist, LFS, README, old name), the secret scan runs on them, and git add (ignore rules, attributes) puts them in the index.
const incrementalBase = async (ctx, full, prev) => {
  const raw = run("git", ["diff", "--raw", "-z", "--no-renames", "--no-ext-diff", prev.full, full], { cwd: ctx.root, maxBuffer: 1 << 28 }).split("\0");
  const del = [], put = [];
  for (let i = 0; i + 1 < raw.length; i += 2) {
    const m = /^:\d+ (\d+) \w+ \w+ ([A-Z])/.exec(raw[i]); const p = raw[i + 1];
    if (!m || STEER.test(p)) return null;
    if (m[2] === "D") del.push(p); else if (m[1] === "100644" || m[1] === "100755") put.push(p); else return null; // a symlink or a submodule: the whole build
  }
  if (!del.length && !put.length) return { tree: prev.tree, paths: 0, lfsMissing: 0 };
  if (del.length + put.length > BASE_LIMIT) return null;
  const { buildPaths, scan } = await import("./public-export.mjs");
  const name = `inc-${full.slice(0, 12)}-${process.pid}`; const out = join(ctx.work, name); mkdirSync(out, { recursive: true });
  try {
    const b = buildPaths({ out, paths: put, root: ctx.root, rev: full });
    refuseSecrets(scan(b.tree));
    const skip = new Set(ignoredPaths(ctx, b.tree, b.files)); const add = b.files.filter((f) => !skip.has(f));
    const tree = withIndex(name, (env) => {
      const e = { ...process.env, ...env };
      run("git", ["--git-dir", ctx.gd, "read-tree", prev.tree], { env: e });
      const gone = [...new Set([...del, ...put])].map((p) => `0 ${"0".repeat(40)}\t${p}\0`).join(""); // all out first: a file that became a folder and back
      if (gone) run("git", ["--git-dir", ctx.gd, "update-index", "-z", "--index-info"], { env: e, input: gone });
      if (add.length) run("git", ["--literal-pathspecs", "--git-dir", ctx.gd, "--work-tree", b.tree, "-c", "core.autocrlf=false", "add", "--pathspec-from-file=-", "--pathspec-file-nul"], { env: e, cwd: b.tree, input: add.join("\0") });
      return run("git", ["--git-dir", ctx.gd, "write-tree"], { env: e }).trim();
    }, ctx.work);
    return { tree, paths: del.length + put.length, lfsMissing: b.lfsMissing.length };
  } finally { rmSync(out, { recursive: true, force: true }); }
};

// The whole public tree of `full` (copy, denylist, LFS, scan) added to an empty index.
const wholeBase = async (ctx, head, full) => {
  const { buildTree, scan } = await import("./public-export.mjs");
  const out = join(ctx.work, `base-${head}`);
  try {
    const b = buildTree({ out, root: ctx.root, rev: full }); refuseSecrets(scan(b.tree));
    const tree = withIndex(`base-${head}`, (env) => {
      run("git", ["--git-dir", ctx.gd, "--work-tree", b.tree, "-c", "core.autocrlf=false", "add", "-A"], { env: { ...process.env, ...env } });
      return run("git", ["--git-dir", ctx.gd, "write-tree"], { env: { ...process.env, ...env } }).trim();
    }, ctx.work);
    return { tree, paths: null, lfsMissing: b.lfsMissing.length };
  } finally { rmSync(out, { recursive: true, force: true }); }
};

// The base tree id of commit `full` (cached as base-<head>.tree, which tools/web-run.mjs reads too; base-<head>.meta says how it was made).
export const buildBase = async (head, full, ctx = ctxNow(), say = console.log) => {
  const cache = join(ctx.work, `base-${head}.tree`); mkdirSync(ctx.work, { recursive: true });
  const cached = () => readFileSync(cache, "utf8").trim();
  if (existsSync(cache)) return cached();
  const lock = join(ctx.work, `base-${head}.lock`);
  for (let tries = 0; ; tries++) { // one builder per HEAD; the others wait for its result
    try { writeFileSync(lock, String(process.pid), { flag: "wx" }); break; } catch {
      for (let i = 0; i < 1200 && !existsSync(cache) && ownerAlive(lock); i++) await sleep(250);
      if (existsSync(cache)) return cached();
      if (ownerAlive(lock) || tries >= 3) throw new Error("the HEAD snapshot did not appear in 5 minutes");
      rmSync(lock, { force: true }); // its builder died
    }
  }
  try {
    const stamp = rulesStamp(); const prev = nearestBase(ctx, full, stamp); let built = null;
    if (prev) {
      try { built = await incrementalBase(ctx, full, prev); if (built) say(`base ${head}: ${built.paths} changed paths on top of ${prev.full.slice(0, 9)}`); }
      catch (e) { if (/^snapshot refused/.test(e.message)) throw e; say(`base ${head}: the incremental build failed (${whyLine(e)}), building it whole`); }
    }
    built ??= await wholeBase(ctx, head, full);
    writeFileSync(join(ctx.work, `base-${head}.meta`), JSON.stringify({ full, tree: built.tree, rules: stamp, lfsMissing: built.lfsMissing, paths: built.paths }));
    writeFileSync(cache, built.tree); return built.tree;
  } finally { rmSync(lock, { force: true }); }
};

// HEAD plus just the numbered hunks of one file, numbered as `commit-mine --list` numbers them: `--only <path>#N` proves exactly
// what `commit-mine --pick <path>#N` commits, while other writers' hunks in the same file stay out.
export const pickedText = (p, ns, cwd = ROOT) => {
  const { hunks } = parseDiff(run("git", ["diff", "-U0", "--no-color", "--no-ext-diff", "HEAD", "--", p], { cwd }));
  const chosen = ns.map((n) => hunks[n - 1]);
  if (chosen.some((h) => !h)) throw new Error(`--only ${p}#N: the file has ${hunks.length} hunks (node tools/commit-mine.mjs --list ${p})`);
  return applyHunks(run("git", ["show", `HEAD:${p}`], { cwd }), chosen);
};

// Dirty and untracked files of the working tree as index lines "mode sha<TAB>path" (mode 0 = removed); only: just these paths,
// where an entry `<path>#N` takes that file as HEAD plus its picked hunks (pickedText).
// All the blobs go through ONE git call (a process per file took 20 s for 270 files); a file with CRLF is hashed from an LF copy.
const overlayLines = (only = null) => {
  const raw = run("git", ["status", "--porcelain=v1", "-z", "--untracked-files=all"], { cwd: ROOT }).split("\0");
  const picks = new Map();
  for (const o of only ?? []) { const m = /^(.*)#(\d+)$/.exec(o); if (m) picks.set(m[1], [...(picks.get(m[1]) ?? []), Number(m[2])]); }
  const paths = only?.map((o) => o.replace(/#\d+$/, ""));
  const want = (p) => !paths || paths.includes(p); const rm = (p) => `0 ${"0".repeat(40)}\t${p}`;
  const lines = [], todo = []; const tmp = join(WORK, `ov-${process.pid}`);
  try {
    for (let i = 0; i < raw.length; i++) {
      const e = raw[i]; if (e.length < 4) continue;
      const x = e[0], y = e[1], p = e.slice(3);
      if (x === "R" || x === "C") { if (want(raw[i + 1])) lines.push(rm(raw[i + 1])); i++; }
      if (denied(p) || !want(p)) continue;
      const src = join(ROOT, p);
      if (x === "D" || y === "D" || !existsSync(src) || statSync(src).isDirectory()) { lines.push(rm(p)); continue; }
      let buf = picks.has(p) ? Buffer.from(pickedText(p, picks.get(p))) : readFileSync(src), file = src;
      if (picks.has(p)) { mkdirSync(tmp, { recursive: true }); file = join(tmp, `pick-${todo.length}`); writeFileSync(file, buf); }
      if (!buf.includes(0) && buf.includes(13)) { mkdirSync(tmp, { recursive: true }); file = join(tmp, String(todo.length)); writeFileSync(file, buf.toString("utf8").replace(/\r\n/g, "\n")); }
      if (/[\n"]/.test(file)) { lines.push(`100644 ${g(["hash-object", "-w", "--stdin"], { input: readFileSync(file) })}\t${p}`); continue; } // --stdin-paths cannot spell this name
      lines.push({ p, at: todo.length }); todo.push(file.split("\\").join("/"));
    }
    const shas = todo.length ? g(["hash-object", "-w", "--no-filters", "--stdin-paths"], { input: `${todo.join("\n")}\n` }).split("\n") : [];
    return lines.map((l) => (typeof l === "string" ? l : `100644 ${shas[l.at].trim()}\t${l.p}`));
  } finally { rmSync(tmp, { recursive: true, force: true }); }
};

// ---- jobs: a check that outlives one call keeps its state in target/gh-check/jobs/<id>.json ----
const jobPath = (id) => join(JOBS, `${id}.json`);
const saveJob = (job) => { mkdirSync(JOBS, { recursive: true }); writeFileSync(jobPath(job.id), JSON.stringify(job)); };
const loadJob = (id) => { try { return JSON.parse(readFileSync(jobPath(id), "utf8")); } catch { return null; } };
const dropBranch = (...ids) => { if (ids.length) { try { run("git", ["--git-dir", gdir(), "push", "-q", "origin", "--delete", ...ids]); } catch { /* the branch is a throwaway */ } } };
const dropJob = (id) => rmSync(jobPath(id), { force: true });
// wip-<8 chars of Date.now() in base 36><3 random chars>: the age of a branch is in its name.
export const newId = (now = Date.now()) => `wip-${now.toString(36)}${Math.random().toString(36).slice(2, 5).padEnd(3, "0")}`;
export const idAgeMs = (id, now = Date.now()) => { const t = parseInt(String(id).slice(4, 12), 36); return /^wip-[0-9a-z]{8}/.test(id) && Number.isFinite(t) ? now - t : 0; };
// Throwaway prefixes every check tool pushes to the public mirror. `wip-` and `web-build-` encode their
// age in the name; `docs-probe-`, `golden-matrix-*` and the older `wip-am24-*` lanes do not, so the
// name alone cannot date them and the sweeper reads the commit date instead. Without this list those
// families live forever: `idAgeMs` answers 0 for a name it cannot parse, which reads as "not stale".
export const THROWAWAY_PREFIXES = ["wip-", "web-build-", "docs-probe-", "golden-matrix-"];
/** True when `name` is a branch one of the check tools pushed, not a real branch of the mirror. */
export const isThrowawayBranch = (name) => THROWAWAY_PREFIXES.some((p) => String(name).startsWith(p));
// Branches and job files of killed calls (the finally of a killed process never runs): older than LEAK_MS, gone.
const sweep = () => {
  try {
    // Ask the mirror itself, not the local refs: a wip branch is pushed straight to the remote and a
    // stale local clone lags behind it. Every throwaway family is read, not only `wip-`, because a
    // branch whose name carries no timestamp answered idAgeMs 0 and read as "not stale" forever.
    const heads = g(["ls-remote", "--heads", "origin"])
      .split("\n")
      .map((line) => line.trim().split(/\s+/))
      .filter(([sha, ref]) => sha && ref)
      .map(([sha, ref]) => ({ sha, name: ref.replace("refs/heads/", "") }))
      .filter(({ name }) => name !== "main" && isThrowawayBranch(name));
    const now = Date.now();
    const stale = [];
    for (const { sha, name } of heads) {
      // A name that carries its own age stays exact and needs no round trip.
      const named = idAgeMs(name, now);
      if (named > 0) {
        if (named > LEAK_MS) stale.push(name);
        continue;
      }
      // Anything else is dated by its commit, which is the only age a `wip-am24-*` lane has.
      try {
        const unix = Number(g(["show", "-s", "--format=%ct", sha]).trim()) * 1000;
        if (Number.isFinite(unix) && unix > 0 && now - unix > LEAK_MS) stale.push(name);
      } catch { /* an unreadable commit just stays */ }
    }
    dropBranch(...stale);
    for (const f of existsSync(JOBS) ? readdirSync(JOBS) : []) if (Date.now() - statSync(join(JOBS, f)).mtimeMs > LEAK_MS) rmSync(join(JOBS, f), { force: true });
  } catch { /* a sweep never fails a check */ }
};

const lapOf = (job) => `${Math.round((Date.now() - job.startedAt) / 1000)}s`;
// One status read serves every waiting call on this PC (13 calls each reading their own branch every 10 s ran the repo's Actions
// API out of its hourly quota): the newest check.yml runs are kept in RUNS for POLL_MS and each call finds its branch there.
// A run missing from that list two minutes after its dispatch (it fell past the newest 100) is read by its branch.
const RUNS = join(WORK, "runs.json");
export const runFromList = (runs, branch) => runs.find((r) => r.headBranch === branch) ?? null;
const recentRuns = () => {
  try { const kept = JSON.parse(readFileSync(RUNS, "utf8")); if (Date.now() - kept.at < POLL_MS) return kept.runs; } catch { /* nothing kept yet */ }
  const runs = JSON.parse(run("gh", ["run", "list", "-R", PUBLIC_REPO, "--workflow", "check.yml", "--json", "databaseId,status,conclusion,headBranch", "-L", "100"]) || "[]");
  try { mkdirSync(WORK, { recursive: true }); const tmp = `${RUNS}.${process.pid}`; writeFileSync(tmp, JSON.stringify({ at: Date.now(), runs })); renameSync(tmp, RUNS); } catch { /* another call wrote it first */ }
  return runs;
};
const runOf = (job) => runFromList(recentRuns(), job.id)
  ?? (Date.now() - job.startedAt > 120_000 ? JSON.parse(run("gh", ["run", "list", "-R", PUBLIC_REPO, "--workflow", "check.yml", "--branch", job.id, "--json", "databaseId,status,conclusion", "-L", "1"]) || "[]")[0] ?? null : null);
// The log lines of check.yml's "run the command" step. 2026-10-24: GitHub served jobs with no steps, so
// `gh run view --log` named every line "UNKNOWN STEP" and every run read as "no command output"; the step
// is then found by its script's first line (`case "$CMD" in`) up to the next step or the job cleanup.
export const commandLines = (log) => {
  const lines = log.split("\n");
  const named = lines.filter((l) => l.includes("\trun the command\t"));
  if (named.length) return named;
  const start = lines.findIndex((l) => l.includes('##[group]Run case "$CMD" in'));
  if (start < 0) return [];
  const end = lines.findIndex((l, i) => i > start && (l.includes("##[group]Run ") || /Z Post job cleanup\./.test(l)));
  return lines.slice(start, end < 0 ? lines.length : end);
};
// Wait for the run of a job until it completes or the call's budget (counted from the start of this process) ends.
// Returns { code, retry?, why?, noTests? }; code 3 = still running (the branch and job file stay).
const settle = async (job, budgetMs) => {
  const until = STARTED + budgetMs;
  let last = null, lost = 0;
  for (;;) {
    if (Date.now() - job.startedAt > RUN_CAP_MS) { console.log(`[${lapOf(job)}] no result after 30 minutes (run ${job.rid ?? "not started"})`); return { code: 2, why: "no result in 30 minutes", retry: false }; }
    if (Date.now() + 10_000 > until || lost >= 6) {
      saveJob(job);
      console.log(`[${lapOf(job)}] STILL RUNNING on the runner, not a failure (${job.cmd}). Do other work, then run: node tools/gh-check.mjs --wait ${job.id}`);
      return { code: 3 };
    }
    await sleep(POLL_MS);
    try {
      last = runOf(job); lost = 0; job.rid = last?.databaseId ?? job.rid;
      if (last?.status === "completed") break;
    } catch (e) { if (rateLimited(e)) await sleep(RATE_WAITS_MS[0]); else lost++; }
  }
  let log = ""; try { log = run("gh", ["run", "view", String(job.rid), "-R", PUBLIC_REPO, "--log"]); } catch (e) { log = String(e.stdout ?? ""); }
  const mine = commandLines(log).map((l) => l.split("\t").slice(2).join("\t").replace(/^\S+Z\s/, "").replace(/\x1b\[[0-9;]*m/g, ""));
  const url = `https://github.com/${PUBLIC_REPO}/actions/runs/${job.rid}`;
  if (!mine.length || last.conclusion === "cancelled") { console.log(`[${lapOf(job)}] the runner gave no command output (${last.conclusion}; setup failed or the run was cancelled): ${url}`); return { code: 2, why: `run ${last.conclusion} with no command output`, retry: true }; }
  const tail = mine.slice(-70).join("\n");
  console.log(tail);
  const ok = last.conclusion === "success";
  console.log(`[${lapOf(job)}] ${ok ? "PASS" : "FAIL"} on the runner: ${job.cmd} (run ${url})`);
  if (!ok && !job.only && !job.at) {
    const dirty = dirtyErrorFiles(tail);
    if (dirty.length) console.log(`note: the errors point at files that are uncommitted in the shared tree (${dirty.join(", ")}). If they are not yours, another AI is mid-edit: prove yours with node tools/gh-check.mjs --only <your,paths> ${job.cmd}`);
  }
  const res = { code: ok ? 0 : 1, noTests: ok && /^cargo test /.test(job.cmd) && !/\s--no-run(?=\s|$)/.test(job.cmd) && testsRan(mine.join("\n")) === 0 };
  // Kept only when nothing the line builds changed while it ran (the key read again now).
  if (job.key && reuseKey(job.cmd, job.os ?? "ubuntu-latest") === job.key) keepResult(job.key, { ...res, where: job.os ?? "ubuntu-latest", cmd: job.cmd, url, tail: mine.slice(-30).join("\n") });
  return res;
};
const finish = (job, res) => { if (res.code !== 3) { dropBranch(job.id); dropJob(job.id); } return res; };
// A run of the same line on the same inputs that another call started and that is still going.
const liveJob = (key) => {
  for (const f of existsSync(JOBS) ? readdirSync(JOBS) : []) {
    const j = loadJob(f.replace(/\.json$/, ""));
    if (j?.key === key && Date.now() - j.startedAt < RUN_CAP_MS) return j;
  }
  return null;
};

// The line of a failed git or gh call that says what went wrong (git push prints "To <url>" first and the reason after it).
export const whyLine = (e) => {
  const lines = String(e?.stderr ?? e?.message ?? e).split("\n").map((l) => l.trim()).filter(Boolean);
  return lines.find((l) => /error|rejected|fatal|denied|refused/i.test(l)) ?? lines[0] ?? "unknown";
};

// A fetch that lost the mirror race is worth another try; anything else is a real failure.
export const raceLost = (why) => /incorrect old value|cannot lock ref|failed to update ref|reference is not a tree/i.test(String(why));

// The mirror's main moves while other lanes push, so a fetch can lose the race and say
// "incorrect old value provided". The runner checks out the wip branch tree, so the parent
// commit is only the diff view: retry the fetch, then carry on with the last known origin/main.
const fetchOrigin = async () => {
  for (let attempt = 1; ; attempt++) { // main only: every other branch is a wip-* of another call, whose files this clone has no use for
    try { run("git", ["--git-dir", gdir(), "fetch", "-q", "--no-tags", "origin", "main"]); return true; }
    catch (e) {
      const why = whyLine(e);
      if (attempt >= 4 || !raceLost(why)) { console.log(`the fetch did not go through, carrying on with the last origin/main: ${why}`); return false; }
      console.log(`the fetch lost the mirror race, retrying: ${why}`);
      await sleep(1000 * attempt);
    }
  }
};

// The parent the wip commit hangs on: the mirror's main when it is known, else the local HEAD.
const mirrorParent = () => {
  for (const ref of ["refs/remotes/origin/main", "HEAD"]) {
    try { const sha = run("git", ["--git-dir", gdir(), "rev-parse", "--verify", `${ref}^{commit}`]).trim(); if (sha) return sha; } catch { /* try the next one */ }
  }
  return "";
};

// The newest pushed tree also stays on the mirror under HAVE: a wip branch is deleted at once, and a git push sends everything the remote
// does not advertise, so the dirty files of other sessions (95 MB of generated assets at times) went up again on every call (60 to 170 s,
// HTTP 408). One local pusher moves it at a time (a lock); ci.yml runs on main only. HAVE saves bytes only as a PARENT of the wip commit:
// git push leaves out what the new commit's parents reach, not what every advertised ref reaches (pack-objects marks edge trees only), so
// a HAVE beside the commit still sent 865 objects / 87 MB a push, and 9 with it as the second parent (lead 2026-10-24).
// HAVE_PUSHED is this PC's note of the last commit it moved HAVE to; the next wip commit takes it as that second parent.
const HAVE = "refs/gh-check/have";
const HAVE_PUSHED = "refs/gh-check/have-pushed";
/** The second parent for the next wip commit: HAVE_PUSHED, unless there is none or it is the first parent. */
export const haveParent = (first, dir = gdir()) => {
  try { const sha = run("git", ["--git-dir", dir, "rev-parse", "--verify", "--quiet", `${HAVE_PUSHED}^{commit}`]).trim(); return sha && sha !== first ? sha : ""; }
  catch { return ""; }
};
export const transient = (why) => /RPC failed|REFUSED_STREAM|stream error|HTTP [45]\d\d|returned error: [45]\d\d|Could not (?:connect|resolve)|timed out|reset by peer|connection reset|hung up|early EOF|unexpected disconnect|cannot lock ref|failed to update ref|remote unpack|unable to write file|could not write pack|index-pack|pack-objects/i.test(String(why));
// A lost pack race prints a 150-character GitHub object path that reads like a
// failure and fills the caller's context; the retry line says what happened,
// and only the last attempt (which throws) keeps git's own words.
export const pushNote = (why) => /unable to write file|could not write pack|index-pack|pack-objects|remote unpack/i.test(String(why))
  ? "the mirror lost the pack race (GitHub side); trying again"
  : `the push failed (${why}); trying again`;
/** The `[local ...]` entries for one push: the transfer's own seconds, then the wait when it was over a second. */
export const pushLaps = ({ seconds, waitedMs }) => [`push ${seconds.toFixed(1)}`, ...(waitedMs >= 1000 ? [`pushwait ${(waitedMs / 1000).toFixed(1)}`] : [])];

// One push at a time. With 19 checks pushing together the mirror took the pack race and every call paid
// 5 to 170 s (2.1 GB of pack-objects on this PC, 2026-10-24); a waiter says so once, then waits its turn.
// The wait is not charged to the push's own seconds, so the lap line still measures the transfer.
// One push at a time, and a waiter waits instead of failing. The uplink is a phone hotspot: 76 MB of new
// content plus 81 parallel pushes meant no push finished, so HAVE never moved and every push re-sent it all
// (HTTP 408, 650-800 s, lead 2026-10-24). One at a time, the first push lands the big delta once and the rest
// are small. A wait past the cap (a holder hung for 20 min) pushes without a slot; it never fails a run.
const PUSH_SLOTS = 1;
const PUSH_LOCK = join(WORK, "push.lock");
const slotPath = (i) => (i === 0 ? PUSH_LOCK : `${PUSH_LOCK}.${i}`);
const PUSH_LOCK_CAP_MS = 1_200_000;
let heldSlot = null;
/** Where the first push slot lives, so a test (or a stuck caller) can name it. */
export const pushLockPath = () => PUSH_LOCK;
export const pushLock = async ({ capMs = PUSH_LOCK_CAP_MS, say = console.log, slots = PUSH_SLOTS } = {}) => {
  mkdirSync(WORK, { recursive: true });
  const t0 = Date.now(); let said = false;
  for (;;) {
    for (let i = 0; i < slots; i++) {
      const p = slotPath(i);
      if (existsSync(p) && !ownerAlive(p)) rmSync(p, { force: true });
      try { writeFileSync(p, String(process.pid), { flag: "wx" }); heldSlot = p; return Date.now() - t0; } catch { /* slot taken */ }
    }
    if (!said) { say(`waiting for a push slot (${slots} pushes at a time, so the mirror does not lose the pack race)`); said = true; }
    if (Date.now() - t0 > capMs) { say(`no push slot after ${Math.round(capMs / 1000)} s: pushing without one`); heldSlot = null; return Date.now() - t0; }
    await sleep(250);
  }
};
/** Frees the slot this process holds (no-op when it pushed without one). */
export const pushUnlock = () => { if (heldSlot) rmSync(heldSlot, { force: true }); heldSlot = null; };

// Pushes the wip commit; a transient network or ref-lock error is tried again (3 pushes, 2 s and 4 s apart).
// Returns the seconds the transfer took and the seconds spent waiting for the push lock.
const pushWip = async (commit, id) => {
  const lock = join(WORK, "have.lock"), off = join(WORK, "have.off"); let mine = false;
  try { if (Date.now() - statSync(off).mtimeMs > 3_600_000) rmSync(off, { force: true }); } catch { /* no off switch */ }
  if (!existsSync(off)) {
    if (existsSync(lock) && !ownerAlive(lock)) rmSync(lock, { force: true });
    try { writeFileSync(lock, String(process.pid), { flag: "wx" }); mine = true; } catch { /* another call moves HAVE */ }
  }
  const wip = `refs/heads/${id}`; const refs = [`${commit}:${wip}`, ...(mine ? [`+${commit}:${HAVE}`] : [])];
  const noteHave = () => { if (!mine) return; try { run("git", ["--git-dir", gdir(), "update-ref", HAVE_PUSHED, commit]); } catch { /* the next commit goes without it */ } };
  let waitedMs = 0, held = false;
  try {
    waitedMs = await pushLock(); held = true;
    const t0 = Date.now();
    for (let attempt = 1; ; attempt++) {
      // No LFS lock check: nobody locks here, and its extra API call failed whole pushes (http2 REFUSED_STREAM, 2026-10-24).
      try { run("git", ["--git-dir", gdir(), "-c", "lfs.locksverify=false", "push", "--porcelain", "origin", ...refs]); noteHave(); return { seconds: (Date.now() - t0) / 1000, waitedMs }; }
      catch (e) {
        const rows = String(e.stdout ?? "").split("\n").filter((l) => l.includes("\t")); const bad = (ref) => rows.some((l) => l.startsWith("!") && l.includes(`:${ref}\t`));
        if (rows.some((l) => !l.startsWith("!") && l.includes(`:${wip}\t`))) { if (bad(HAVE)) writeFileSync(off, ""); else noteHave(); return { seconds: (Date.now() - t0) / 1000, waitedMs }; } // the wip branch is there; a refused HAVE is switched off for an hour
        const why = whyLine(e); if (attempt >= 3 || !transient(why)) throw e;
        console.log(`${pushNote(why)} in ${2 * attempt} s`); await sleep(2000 * attempt);
      }
    }
  } finally { if (held) pushUnlock(); if (mine) rmSync(lock, { force: true }); }
};

// One fresh check: snapshot, push, dispatch, wait. A failure before the dispatch is { code: 2, retry: true }.
// A kept result answers first; a live twin is joined instead of dispatched again.
// only: build HEAD plus just these working-tree paths (others' unfinished files stay out); such a run is not kept.
// at: build that commit alone, no working-tree file (a bisect of a red test); such a run is not kept either.
const startRemote = async (cmd, os, budgetMs, fresh = false, only = null, at = null) => {
  const key = only || at ? null : reuseKey(cmd, os);
  if (key && !fresh) {
    const kept = keptResult(key);
    if (kept) { sayKept(kept, console.log); return { code: kept.code }; }
    const twin = liveJob(key);
    if (twin) { console.log(`joined ${twin.id}: another call runs the same line on the same inputs`); return finish(twin, await settle(twin, budgetMs)); }
  }
  if (!existsSync(gdir())) { console.log("no mirror clone yet: run node tools/farm.mjs once"); return { code: 2, why: "no mirror clone", retry: false }; }
  const rev = (...a) => run("git", ["rev-parse", ...a], { cwd: ROOT }).trim();
  let full = rev(at ? `${at}^{commit}` : "HEAD"), head = rev("--short", full);
  const job = { id: newId(), cmd, os, key, head, startedAt: Date.now(), only: only ?? null, at: at ?? null };
  let pushed = false, res = { code: 2, retry: true };
  // Seconds per phase of the local half, printed once ([local 12s: fetch 3.0 base 4.1 ...]): where a slow call went.
  const laps = []; const lap = async (name, fn) => { const t0 = Date.now(); try { return await fn(); } finally { laps.push(`${name} ${((Date.now() - t0) / 1000).toFixed(1)}`); } };
  const lapLine = () => console.log(`[local ${lapOf(job)}: ${laps.join(" ")}]`);
  try {
    await lap("fetch", () => oncePer("fetch", 60_000, fetchOrigin)); // origin/main is only the wip commit's parent: a minute old is fine
    await lap("sweep", () => oncePer("sweep", 600_000, async () => sweep()));
    let tree;
    for (let again = 0; ; again++) {
      const base = await lap("base", () => buildBase(head, full));
      tree = await lap("overlay", async () => withIndex(job.id, (env) => {
        const e = { ...process.env, ...env };
        run("git", ["--git-dir", gdir(), "read-tree", base], { env: e });
        const lines = at ? [] : overlayLines(only);
        if (only) console.log(`--only: HEAD ${head} plus ${lines.length} of your paths`);
        if (lines.length) run("git", ["--git-dir", gdir(), "update-index", "-z", "--add", "--replace", "--index-info"], { env: e, input: lines.map((l) => `${l}\0`).join("") });
        return run("git", ["--git-dir", gdir(), "write-tree"], { env: e }).trim();
      }));
      if (at) break;
      const now = rev("HEAD"); if (now === full || again >= 2) break;
      console.log(`HEAD moved to ${now.slice(0, 9)} while the snapshot was built: again`); // the dirty files are judged against the new HEAD
      full = now; head = job.head = rev("--short", full);
    }
    const env = { ...process.env, GIT_AUTHOR_NAME: "maximkone", GIT_AUTHOR_EMAIL: "maximkone@users.noreply.github.com", GIT_COMMITTER_NAME: "maximkone", GIT_COMMITTER_EMAIL: "maximkone@users.noreply.github.com" };
    const commit = await lap("commit", async () => { const parent = mirrorParent(); const have = haveParent(parent); return run("git", ["--git-dir", gdir(), "commit-tree", tree, ...(parent ? ["-p", parent] : []), ...(have ? ["-p", have] : []), "-m", `wip ${job.id} of ${head}: ${cmd}`], { env }).trim(); });
    // Not lap(): lap appends after the call returns, and this has to report the transfer alone, not the lock wait.
    {
      const pr = await pushWip(commit, job.id);
      laps.push(...pushLaps(pr));
    }
    pushed = true;
    if (process.env.GH_CHECK_DRY) { lapLine(); console.log("GH_CHECK_DRY: pushed, not dispatched"); return { code: 2, why: "dry run", retry: false }; } // measures the local half without a GitHub run (use with --remote-only)
    console.log(`[${lapOf(job)}] sent ${job.id} (${at ? `commit ${head} alone` : `HEAD ${head} + working tree`}) to the ${os} runner: ${cmd}`);
    // The os input only when it is not the default, so a mirror whose check.yml predates it still takes Linux lines.
    try { await lap("dispatch", () => withRateRetry(async () => run("gh", ["workflow", "run", "check.yml", "-R", PUBLIC_REPO, "--ref", job.id, "-f", `cmd=${cmd}`, ...(os === "ubuntu-latest" ? [] : ["-f", `os=${os}`])]))); saveJob(job); lapLine(); }
    catch (e) { lapLine(); const why = whyLine(e); console.log(`dispatch failed (is check.yml on the mirror's main? run node tools/farm.mjs once): ${why}`); res = { code: 2, why: "dispatch failed", retry: true }; return res; }
    res = await settle(job, budgetMs);
    return res;
  } catch (e) { lapLine(); console.log(`could not run the check: ${whyLine(e)}`); res = { code: 2, why: "the snapshot or push failed", retry: true }; return res; }
  finally { if (pushed) finish(job, res); }
};

// Pick up a check an earlier call left running.
const waitRemote = async (id, budgetMs) => {
  const job = loadJob(id);
  if (!job) { console.log(`no job ${id} (finished, swept after 3 h, or never started): run the cargo line again`); return { code: 2, why: "unknown job", retry: false }; }
  let res = { code: 2, retry: true };
  try { res = await settle(job, budgetMs); return res; }
  catch (e) { console.log(`could not read the check: ${String(e?.stderr ?? e?.message ?? e).split("\n")[0]}`); res = { code: 3 }; return res; }
  finally { finish(job, res); }
};

export const main = async (argv, env = process.env) => {
  if (argv.length === 1 && ["--help", "-h", "help"].includes(argv[0])) { console.log(USAGE); return 0; }
  const a = parseArgs(argv, env);
  if (a.error) { console.log(`refused: ${a.error}\nnode tools/gh-check.mjs --help lists the flags`); return 2; }
  const budgetMs = a.budget * 1000;
  if (a.wait) {
    const job = loadJob(a.wait);
    const res = await waitRemote(a.wait, budgetMs);
    return job ? afterRemote({ res, cmd: job.cmd, mode: "auto", local: (c) => runLocal(c, console.log, { fresh: a.fresh }), say: console.log }) : res.code;
  }
  if (!okCmd(a.cmd)) { console.log("refused: give one plain `cargo check|test|clippy ...` command, such as `node tools/gh-check.mjs cargo test -p engine-net --lib one_filter`, or one cargo mutants slice, such as `cargo mutants -p engine-core --shard 1/8` (`node tools/gh-check.mjs --help` lists the flags)"); return 2; }
  if (isMutants(a.cmd) && a.mode === "local") { console.log("refused: cargo mutants never runs on this PC: drop --local (the slice goes to a GitHub runner)"); return 2; }
  // A dependency's lint is not this crate's: --no-deps is added here, not left to the caller to remember.
  const nd = withNoDeps(a.cmd);
  if (nd.added) console.log(`note: clippy with -p now adds --no-deps, so a dependency's warning cannot fail this line: ${nd.cmd}`);
  a.cmd = nd.cmd;
  // --only mistakes that cost a whole remote run are refused here, before the fetch and the push.
  const problems = a.only ? preflightOnly(a.only) : [];
  if (problems.length) { console.log("refused: " + problems.join("\nrefused: ")); return 2; }
  return route({ cmd: a.cmd, mode: a.only || a.at || env.GH_CHECK_DRY ? "remote" : a.mode, remote: (c, os) => startRemote(c, os, budgetMs, a.fresh, a.only, a.at), local: (c) => runLocal(c, console.log, { fresh: a.fresh }) });
};
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main(process.argv.slice(2)).then((c) => process.exit(c));

// GitHub as the workforce (Maxim 2026-10-16, "make github our workforce"): run ONE narrow cargo command on a GitHub runner
// against the helper's working tree (HEAD plus every dirty and untracked file), without the PC's one build lock.
//   node tools/gh-check.mjs cargo test -p engine-net --lib interest_64_room_converges
//   node tools/gh-check.mjs cargo clippy -p engine-input --all-targets -- -D warnings
//   node tools/gh-check.mjs --wait wip-xxxx          pick up a check that was still running (exit 3 prints this line)
// How: the HEAD snapshot (denylist and LFS applied, like the public export) is built once per HEAD into the mirror clone's object
// store; each call overlays the dirty files, commits that tree on a throwaway wip-* branch of the public mirror, dispatches
// .github/workflows/check.yml on it, waits, prints the command's output tail and deletes the branch. Up to 20 run at once.
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
import { lowerPriority, poolEnv } from "./jobserver.mjs";
// Public-gate safe (2026-10-16): no static import of ./public-export.mjs here.
// That file is in its own export DENY list, so the public checkout lacks it and
// every public gate-tool test fails with ERR_MODULE_NOT_FOUND. The small pure
// pieces (PUBLIC_REPO + DENY/denied) are mirrored below; the heavy snapshot
// builders load lazily inside baseTree(). Mirror of tools/public-export.mjs
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

// Only one plain cargo check, test or clippy line goes to the runner (the workflow refuses the rest again).
export const okCmd = (cmd) => /^cargo (check|test|clippy) /.test(cmd) && !/[&;|`<>]|\$\(/.test(cmd);

// The words of a plain command line (a quoted run is one word, quotes removed).
export const words = (s) => (String(s).match(/"[^"]*"|'[^']*'|\S+/g) ?? []).map((w) => w.replace(/^(["'])(.*)\1$/, "$2"));

export const DEFAULT_BUDGET_S = 100;
// node tools/gh-check.mjs [--budget <s>] [--remote-only | --local] [--fresh] [--only <path,path>] cargo <check|test|clippy> ...   |   --wait <job> [--budget <s>]
export const parseArgs = (argv, env = {}) => {
  const out = { cmd: "", wait: null, mode: "auto", fresh: false, only: null, budget: Number(env.GH_CHECK_BUDGET) >= 1 ? Math.min(Number(env.GH_CHECK_BUDGET), 1740) : DEFAULT_BUDGET_S, error: null };
  let i = 0;
  for (; i < argv.length && argv[i].startsWith("-"); i++) {
    const a = argv[i];
    if (a === "--wait") { out.wait = argv[++i] ?? ""; if (!/^wip-[a-z0-9]+$/.test(out.wait)) out.error = "--wait needs the job name the check printed (wip-...)"; }
    else if (a === "--budget") { const n = Number(argv[++i]); if (n >= 1) out.budget = Math.min(n, 1740); else out.error = "--budget needs seconds (1 to 1740)"; }
    else if (a === "--remote-only") out.mode = "remote";
    else if (a === "--local") out.mode = "local";
    else if (a === "--fresh") out.fresh = true;
    else if (a === "--only") { out.only = String(argv[++i] ?? "").split(",").map((x) => x.trim().split("\\").join("/")).filter(Boolean); if (!out.only.length) out.error = "--only needs paths, comma-separated"; }
    else { out.error = `unknown flag ${a}`; break; }
  }
  out.cmd = argv.slice(i).join(" ").trim();
  // cargo has none of these, so one after the cargo line is ours put in the wrong place: cargo would only fail on it remotely
  const rest = argv.slice(i); const dash = rest.indexOf("--");
  const late = (dash < 0 ? rest : rest.slice(0, dash)).find((a) => /^--(?:budget|fresh|only|wait|remote-only)$/.test(a));
  if (late && !out.error) out.error = `${late} is a gh-check flag: put it before the cargo line (node tools/gh-check.mjs ${late} ... cargo ...)`;
  return out;
};

// ---- where a line runs: the Linux runner, the Windows runner, or the PC ----
// Cargo features that need a GPU, a window, a sound device or Windows itself (render `window`, game `app`, editor `ui`, audio
// `device`): the Linux runner cannot build them, the Windows runner can. tools/gh-check.test.mjs checks this list against every
// Cargo.toml.
export const LOCAL_FEATURES = new Set(["window", "app", "ui", "device"]);
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
  return afterRemote({ res, cmd, mode, local, say });
};
// A finished remote result into the exit code: a vacuous pass or a run that could not happen goes to the PC unless --remote-only.
export const afterRemote = async ({ res, cmd, mode, local, say }) => {
  if (res.code === 0 && res.noTests) {
    say("PASS but 0 tests ran on the runner: the filter matches only GPU, window or Windows-only tests (or nothing), so it proves nothing");
    if (mode === "remote") return 2;
    say("running it on the PC instead"); return local(cmd);
  }
  if (res.code === 2 && mode !== "remote") { say("GitHub could not run it; running it on the PC instead"); return local(cmd); }
  return res.code;
};

// The same line on this PC: stdio straight through, cargo's own lock decides when it runs. Never workspace-wide.
// A kept result of the same line on the same inputs answers at once; the same line already building here is waited for.
export const runLocal = (cmd, say = console.log, { fresh = false, root = ROOT } = {}) => {
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
const withIndex = (name, fn) => {
  const idx = join(WORK, `${name}.index`); mkdirSync(WORK, { recursive: true }); rmSync(idx, { force: true });
  try { return fn({ GIT_INDEX_FILE: idx }); } finally { rmSync(idx, { force: true }); }
};

// The HEAD snapshot as a git tree object inside the mirror clone, built once per HEAD by the first caller; the others wait for it.
const baseTree = async (head) => {
  const cache = join(WORK, `base-${head}.tree`); mkdirSync(WORK, { recursive: true });
  if (existsSync(cache)) return readFileSync(cache, "utf8").trim();
  const lock = join(WORK, `base-${head}.lock`);
  try { writeFileSync(lock, String(process.pid), { flag: "wx" }); } catch {
    for (let i = 0; i < 60 && !existsSync(cache); i++) await sleep(5000);
    if (existsSync(cache)) return readFileSync(cache, "utf8").trim();
    rmSync(lock, { force: true }); throw new Error("the HEAD snapshot did not appear in 5 minutes");
  }
  try {
    const { buildTree, scan } = await import("./public-export.mjs");
    const out = join(WORK, `base-${head}`); const b = buildTree({ out });
    const s = scan(b.tree); const secret = s.problems.filter((p) => /secret|token|key/i.test(p));
    if (secret.length) throw new Error(`snapshot refused: ${secret[0]}`);
    const tree = withIndex(`base-${head}`, (env) => {
      run("git", ["--git-dir", gdir(), "--work-tree", b.tree, "-c", "core.autocrlf=false", "add", "-A"], { env: { ...process.env, ...env } });
      return run("git", ["--git-dir", gdir(), "write-tree"], { env: { ...process.env, ...env } }).trim();
    });
    rmSync(out, { recursive: true, force: true }); writeFileSync(cache, tree); return tree;
  } finally { rmSync(lock, { force: true }); }
};

// Dirty and untracked files of the working tree as index lines "mode sha<TAB>path" (mode 0 = removed).
const overlayLines = () => {
  const raw = run("git", ["status", "--porcelain=v1", "-z", "--untracked-files=all"], { cwd: ROOT }).split("\0");
  const lines = [];
  for (let i = 0; i < raw.length; i++) {
    const e = raw[i]; if (e.length < 4) continue;
    const x = e[0], y = e[1], p = e.slice(3);
    if (x === "R" || x === "C") { lines.push(`0 ${"0".repeat(40)}\t${raw[i + 1]}`); i++; }
    if (denied(p)) continue;
    const src = join(ROOT, p);
    if (x === "D" || y === "D" || !existsSync(src) || statSync(src).isDirectory()) { lines.push(`0 ${"0".repeat(40)}\t${p}`); continue; }
    let buf = readFileSync(src); if (!buf.includes(0) && buf.includes(13)) buf = Buffer.from(buf.toString("utf8").replace(/\r\n/g, "\n"));
    lines.push(`100644 ${g(["hash-object", "-w", "--stdin"], { input: buf })}\t${p}`);
  }
  return lines;
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
// Branches and job files of killed calls (the finally of a killed process never runs): older than LEAK_MS, gone.
const sweep = () => {
  try {
    const stale = g(["ls-remote", "--heads", "origin", "wip-*"]).split("\n").map((l) => l.split("\t")[1]?.replace("refs/heads/", "")).filter((id) => id && idAgeMs(id) > LEAK_MS);
    dropBranch(...stale);
    for (const f of existsSync(JOBS) ? readdirSync(JOBS) : []) if (Date.now() - statSync(join(JOBS, f)).mtimeMs > LEAK_MS) rmSync(join(JOBS, f), { force: true });
  } catch { /* a sweep never fails a check */ }
};

const lapOf = (job) => `${Math.round((Date.now() - job.startedAt) / 1000)}s`;
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
    await sleep(10_000);
    try {
      const list = JSON.parse(run("gh", ["run", "list", "-R", PUBLIC_REPO, "--workflow", "check.yml", "--branch", job.id, "--json", "databaseId,status,conclusion", "-L", "1"]) || "[]");
      lost = 0; last = list[0] ?? null; job.rid = last?.databaseId ?? job.rid;
      if (last?.status === "completed") break;
    } catch { lost++; }
  }
  let log = ""; try { log = run("gh", ["run", "view", String(job.rid), "-R", PUBLIC_REPO, "--log"]); } catch (e) { log = String(e.stdout ?? ""); }
  const mine = log.split("\n").filter((l) => l.includes("\trun the command\t")).map((l) => l.split("\t").slice(2).join("\t").replace(/^\S+Z\s/, "").replace(/\x1b\[[0-9;]*m/g, ""));
  const url = `https://github.com/${PUBLIC_REPO}/actions/runs/${job.rid}`;
  if (!mine.length || last.conclusion === "cancelled") { console.log(`[${lapOf(job)}] the runner gave no command output (${last.conclusion}; setup failed or the run was cancelled): ${url}`); return { code: 2, why: `run ${last.conclusion} with no command output`, retry: true }; }
  const tail = mine.slice(-70).join("\n");
  console.log(tail);
  const ok = last.conclusion === "success";
  console.log(`[${lapOf(job)}] ${ok ? "PASS" : "FAIL"} on the runner: ${job.cmd} (run ${url})`);
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
  for (let attempt = 1; ; attempt++) {
    try { run("git", ["--git-dir", gdir(), "fetch", "-q", "origin"]); return true; }
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

// One fresh check: snapshot, push, dispatch, wait. A failure before the dispatch is { code: 2, retry: true }.
// A kept result answers first; a live twin is joined instead of dispatched again.
// only: build HEAD plus just these working-tree paths (others' unfinished files stay out); such a run is not kept.
const startRemote = async (cmd, os, budgetMs, fresh = false, only = null) => {
  const key = only ? null : reuseKey(cmd, os);
  if (key && !fresh) {
    const kept = keptResult(key);
    if (kept) { sayKept(kept, console.log); return { code: kept.code }; }
    const twin = liveJob(key);
    if (twin) { console.log(`joined ${twin.id}: another call runs the same line on the same inputs`); return finish(twin, await settle(twin, budgetMs)); }
  }
  if (!existsSync(gdir())) { console.log("no mirror clone yet: run node tools/farm.mjs once"); return { code: 2, why: "no mirror clone", retry: false }; }
  const head = run("git", ["rev-parse", "--short", "HEAD"], { cwd: ROOT }).trim();
  const job = { id: newId(), cmd, os, key, head, startedAt: Date.now() };
  let pushed = false, res = { code: 2, retry: true };
  try {
    await fetchOrigin();
    sweep();
    const base = await baseTree(head);
    const tree = withIndex(job.id, (env) => {
      const e = { ...process.env, ...env };
      run("git", ["--git-dir", gdir(), "read-tree", base], { env: e });
      const lines = overlayLines().filter((l) => !only || only.includes(l.split("	")[1]));
      if (only) console.log(`--only: HEAD ${head} plus ${lines.length} of your paths`);
      if (lines.length) run("git", ["--git-dir", gdir(), "update-index", "--add", "--index-info"], { env: e, input: `${lines.join("\n")}\n` });
      return run("git", ["--git-dir", gdir(), "write-tree"], { env: e }).trim();
    });
    const env = { ...process.env, GIT_AUTHOR_NAME: "maximkone", GIT_AUTHOR_EMAIL: "maximkone@users.noreply.github.com", GIT_COMMITTER_NAME: "maximkone", GIT_COMMITTER_EMAIL: "maximkone@users.noreply.github.com" };
    const parent = mirrorParent();
    const commit = run("git", ["--git-dir", gdir(), "commit-tree", tree, ...(parent ? ["-p", parent] : []), "-m", `wip ${job.id} of ${head}: ${cmd}`], { env }).trim();
    run("git", ["--git-dir", gdir(), "push", "-q", "origin", `${commit}:refs/heads/${job.id}`]); pushed = true;
    console.log(`[${lapOf(job)}] sent ${job.id} (HEAD ${head} + working tree) to the ${os} runner: ${cmd}`);
    // The os input only when it is not the default, so a mirror whose check.yml predates it still takes Linux lines.
    try { run("gh", ["workflow", "run", "check.yml", "-R", PUBLIC_REPO, "--ref", job.id, "-f", `cmd=${cmd}`, ...(os === "ubuntu-latest" ? [] : ["-f", `os=${os}`])]); saveJob(job); }
    catch (e) { const why = whyLine(e); console.log(`dispatch failed (is check.yml on the mirror's main? run node tools/farm.mjs once): ${why}`); res = { code: 2, why: "dispatch failed", retry: true }; return res; }
    res = await settle(job, budgetMs);
    return res;
  } catch (e) { console.log(`could not run the check: ${whyLine(e)}`); res = { code: 2, why: "the snapshot or push failed", retry: true }; return res; }
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
  const a = parseArgs(argv, env);
  if (a.error) { console.log(`refused: ${a.error}`); return 2; }
  const budgetMs = a.budget * 1000;
  if (a.wait) {
    const job = loadJob(a.wait);
    const res = await waitRemote(a.wait, budgetMs);
    return job ? afterRemote({ res, cmd: job.cmd, mode: "auto", local: (c) => runLocal(c, console.log, { fresh: a.fresh }), say: console.log }) : res.code;
  }
  if (!okCmd(a.cmd)) { console.log("refused: give one plain `cargo check|test|clippy ...` command"); return 2; }
  return route({ cmd: a.cmd, mode: a.only ? "remote" : a.mode, remote: (c, os) => startRemote(c, os, budgetMs, a.fresh, a.only), local: (c) => runLocal(c, console.log, { fresh: a.fresh }) });
};
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main(process.argv.slice(2)).then((c) => process.exit(c));

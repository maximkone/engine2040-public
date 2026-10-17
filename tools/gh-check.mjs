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
//   - GPU, window and Windows-only lines (features window|app|ui|device, a gpu/window/wgpu/... test filter) go straight to the PC:
//     the Linux runner has no adapter, so such tests would skip and "pass";
//   - a run that could not start or finished with no command output is tried once more, then the same line runs on the PC
//     (never --workspace: that stays forbidden local);
//   - a cargo test that PASSED with 0 tests (a filter that only matches Windows-only or GPU tests) is not a proof: it runs on the PC.
//   --remote-only never falls back (exit 2 instead); --local skips GitHub; --budget <s> (default 100, env GH_CHECK_BUDGET) is how
//   long one call waits: OpenCode and Claude Code tool calls die at 120 s, so a longer check returns exit 3 with its --wait line
//   and the wip branch stays until a call sees the result (leaked branches older than 3 h are swept by the next call).
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
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
// node tools/gh-check.mjs [--budget <s>] [--remote-only | --local] cargo <check|test|clippy> ...   |   --wait <job> [--budget <s>]
export const parseArgs = (argv, env = {}) => {
  const out = { cmd: "", wait: null, mode: "auto", budget: Number(env.GH_CHECK_BUDGET) >= 1 ? Math.min(Number(env.GH_CHECK_BUDGET), 1740) : DEFAULT_BUDGET_S, error: null };
  let i = 0;
  for (; i < argv.length && argv[i].startsWith("-"); i++) {
    const a = argv[i];
    if (a === "--wait") { out.wait = argv[++i] ?? ""; if (!/^wip-[a-z0-9]+$/.test(out.wait)) out.error = "--wait needs the job name the check printed (wip-...)"; }
    else if (a === "--budget") { const n = Number(argv[++i]); if (n >= 1) out.budget = Math.min(n, 1740); else out.error = "--budget needs seconds (1 to 1740)"; }
    else if (a === "--remote-only") out.mode = "remote";
    else if (a === "--local") out.mode = "local";
    else { out.error = `unknown flag ${a}`; break; }
  }
  out.cmd = argv.slice(i).join(" ").trim();
  return out;
};

// ---- what must run on the PC ----
// Cargo features that need a GPU, a window, a sound device or Windows itself (render `window`, game `app`, editor `ui`, audio
// `device`): the Linux runner cannot build or run them. tools/gh-check.test.mjs checks this list against every Cargo.toml.
export const LOCAL_FEATURES = new Set(["window", "app", "ui", "device"]);
// A test filter or target naming one of these is a GPU, window or Windows-only test (they skip on the runner and still pass).
const LOCAL_WORD = /gpu|window|wgpu|winit|swapchain|adapter|vulkan|dx12|win32/i;
// A source file that probes for an adapter, prints SKIPPED or is Windows-only holds tests that skip or vanish on the runner.
const LOCAL_SRC = /request_adapter|\bSKIPPED\b|\bno (?:GPU|adapter)\b|cfg\(windows\)|target_os\s*=\s*"windows"/;
const VALUE_FLAGS = new Set(["-p", "--package", "--bin", "--test", "--example", "--bench", "--features", "-F", "--target", "--profile", "--manifest-path", "-j", "--jobs", "--exclude", "--color", "--message-format", "--target-dir", "--config"]);
const NAME_FLAGS = new Set(["--bin", "--test", "--example", "--bench"]);
const walkRs = (dir, out = []) => {
  for (const e of existsSync(dir) ? readdirSync(dir, { withFileTypes: true }) : []) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walkRs(p, out); else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};
// The first source file of the packages that defines a test fn named like a filter and holds LOCAL_SRC code, else null.
const localSource = (pkgs, filters, root) => {
  if (!pkgs.length || !filters.length) return null;
  const base = join(root, "crates");
  for (const d of existsSync(base) ? readdirSync(base) : []) {
    let name; try { name = /^name\s*=\s*"([^"]+)"/m.exec(readFileSync(join(base, d, "Cargo.toml"), "utf8"))?.[1]; } catch { continue; }
    if (!pkgs.includes(name)) continue;
    for (const f of walkRs(join(base, d))) {
      const src = readFileSync(f, "utf8");
      if (LOCAL_SRC.test(src) && filters.some((x) => [...src.matchAll(/\bfn\s+(\w+)/g)].some((m) => m[1].includes(x)))) return f.slice(root.length + 1).split("\\").join("/");
    }
  }
  return null;
};
// null = the line may run on GitHub, else why it must run here.
export const localOnly = (cmd, root = ROOT) => {
  const w = words(cmd);
  if (w[0] !== "cargo") return null;
  const pkgs = [], filters = [];
  let afterDash = false;
  for (let i = 2; i < w.length; i++) {
    const t = w[i];
    if (afterDash) { if (!t.startsWith("-")) filters.push(t); continue; }
    if (t === "--") { afterDash = true; continue; }
    if (t === "--all-features") return "--all-features builds the window and device features (no GPU or Windows on the runner)";
    const feat = t === "--features" || t === "-F" ? w[i + 1] : /^--features=|^-F./.test(t) ? t.replace(/^--features=|^-F/, "") : null;
    if (feat) {
      const hit = feat.split(/[,\s]+/).find((f) => LOCAL_FEATURES.has(f.split("/").pop()));
      if (hit) return `feature ${hit} needs a GPU, a window or Windows (the runner is Linux without a GPU)`;
    }
    const named = NAME_FLAGS.has(t) ? w[i + 1] : /^--(?:bin|test|example|bench)=/.test(t) ? t.split("=")[1] : null;
    if (named && LOCAL_WORD.test(named)) return `target ${named} is a GPU, window or Windows-only test`;
    if (t === "-p" || t === "--package") { pkgs.push(w[++i] ?? ""); continue; }
    if (/^--package=/.test(t)) { pkgs.push(t.slice(10)); continue; }
    if (VALUE_FLAGS.has(t)) { i++; continue; }
    if (!t.startsWith("-")) filters.push(t);
  }
  if (w[1] !== "test") return null; // check and clippy only compile: the features above are all that matters
  const hit = filters.find((f) => LOCAL_WORD.test(f));
  if (hit) return `test filter ${hit} is a GPU, window or Windows-only test`;
  const file = localSource(pkgs, filters, root);
  return file ? `${file} holds GPU, window or Windows-only code (its tests skip or vanish on the runner)` : null;
};

// Tests that passed over all `test result:` lines of a cargo test log; null when the log has none.
export const testsRan = (log) => {
  let n = null;
  for (const m of String(log).matchAll(/test result: (?:ok|FAILED)\. (\d+) passed/g)) n = (n ?? 0) + Number(m[1]);
  return n;
};

// ---- the route: GitHub first, the PC second ----
// remote(cmd) -> { code, retry?, why?, noTests? }; local(cmd) -> code. Pure over those two, so the tests fake them.
export const route = async ({ cmd, mode = "auto", remote, local, say = console.log }) => {
  const forced = mode === "local" ? "--local given" : mode === "auto" ? localOnly(cmd) : null;
  if (forced) { say(`local, not GitHub: ${forced}`); return local(cmd); }
  let res = { code: 2 };
  for (let attempt = 1; attempt <= 2; attempt++) {
    res = await remote(cmd);
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
export const runLocal = (cmd, say = console.log) => {
  if (/\s--(?:workspace|all)(?=\s|$)/.test(cmd)) { say("refused: a workspace-wide cargo line never runs on this PC; give one crate (-p <crate>) or fix what stopped GitHub"); return 2; }
  say(`[local] ${cmd}`);
  const [bin, ...args] = words(cmd);
  const r = spawnSync(bin, args, { cwd: ROOT, stdio: "inherit" });
  return r.error ? 2 : r.status === 0 ? 0 : 1;
};

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
  console.log(mine.slice(-70).join("\n"));
  const ok = last.conclusion === "success";
  console.log(`[${lapOf(job)}] ${ok ? "PASS" : "FAIL"} on the runner: ${job.cmd} (run ${url})`);
  return { code: ok ? 0 : 1, noTests: ok && /^cargo test /.test(job.cmd) && !/\s--no-run(?=\s|$)/.test(job.cmd) && testsRan(mine.join("\n")) === 0 };
};
const finish = (job, res) => { if (res.code !== 3) { dropBranch(job.id); dropJob(job.id); } return res; };

// One fresh check: snapshot, push, dispatch, wait. A failure before the dispatch is { code: 2, retry: true }.
const startRemote = async (cmd, budgetMs) => {
  if (!existsSync(gdir())) { console.log("no mirror clone yet: run node tools/farm.mjs once"); return { code: 2, why: "no mirror clone", retry: false }; }
  const head = run("git", ["rev-parse", "--short", "HEAD"], { cwd: ROOT }).trim();
  const job = { id: newId(), cmd, head, startedAt: Date.now() };
  let pushed = false, res = { code: 2, retry: true };
  try {
    run("git", ["--git-dir", gdir(), "fetch", "-q", "origin"]);
    sweep();
    const base = await baseTree(head);
    const tree = withIndex(job.id, (env) => {
      const e = { ...process.env, ...env };
      run("git", ["--git-dir", gdir(), "read-tree", base], { env: e });
      const lines = overlayLines();
      if (lines.length) run("git", ["--git-dir", gdir(), "update-index", "--add", "--index-info"], { env: e, input: `${lines.join("\n")}\n` });
      return run("git", ["--git-dir", gdir(), "write-tree"], { env: e }).trim();
    });
    const env = { ...process.env, GIT_AUTHOR_NAME: "maximkone", GIT_AUTHOR_EMAIL: "maximkone@users.noreply.github.com", GIT_COMMITTER_NAME: "maximkone", GIT_COMMITTER_EMAIL: "maximkone@users.noreply.github.com" };
    const commit = run("git", ["--git-dir", gdir(), "commit-tree", tree, "-p", "refs/remotes/origin/main", "-m", `wip ${job.id} of ${head}: ${cmd}`], { env }).trim();
    run("git", ["--git-dir", gdir(), "push", "-q", "origin", `${commit}:refs/heads/${job.id}`]); pushed = true;
    console.log(`[${lapOf(job)}] sent ${job.id} (HEAD ${head} + working tree) to the runner: ${cmd}`);
    try { run("gh", ["workflow", "run", "check.yml", "-R", PUBLIC_REPO, "--ref", job.id, "-f", `cmd=${cmd}`]); }
    catch (e) { const why = String(e.stderr ?? e.message).split("\n")[0]; console.log(`dispatch failed (is check.yml on the mirror's main? run node tools/farm.mjs once): ${why}`); res = { code: 2, why: "dispatch failed", retry: true }; return res; }
    res = await settle(job, budgetMs);
    return res;
  } catch (e) { console.log(`could not run the check: ${String(e?.stderr ?? e?.message ?? e).split("\n")[0]}`); res = { code: 2, why: "the snapshot or push failed", retry: true }; return res; }
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
    return job ? afterRemote({ res, cmd: job.cmd, mode: "auto", local: (c) => runLocal(c), say: console.log }) : res.code;
  }
  if (!okCmd(a.cmd)) { console.log("refused: give one plain `cargo check|test|clippy ...` command"); return 2; }
  return route({ cmd: a.cmd, mode: a.mode, remote: (c) => startRemote(c, budgetMs), local: (c) => runLocal(c) });
};
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main(process.argv.slice(2)).then((c) => process.exit(c));

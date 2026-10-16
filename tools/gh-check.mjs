// GitHub as the workforce (Maxim 2026-10-16, "make github our workforce"): run ONE narrow cargo command on a GitHub runner
// against the helper's working tree (HEAD plus every dirty and untracked file), without the PC's one build lock.
//   node tools/gh-check.mjs cargo test -p engine-net --lib interest_64_room_converges
//   node tools/gh-check.mjs cargo clippy -p engine-input --all-targets -- -D warnings
// How: the HEAD snapshot (denylist and LFS applied, like the public export) is built once per HEAD into the mirror clone's object
// store; each call overlays the dirty files, commits that tree on a throwaway wip-* branch of the public mirror, dispatches
// .github/workflows/check.yml on it, waits, prints the command's output tail and deletes the branch. Up to 20 run at once.
// Exit 0 = the command passed on the runner, 1 = it failed (tail printed), 2 = the check could not run.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { PUBLIC_REPO, buildTree, denied, scan } from "./public-export.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const WORK = join(ROOT, "target", "gh-check");
const CLONE = join(ROOT, "target", "public-export", "repo");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const run = (cmd, args, o = {}) => execFileSync(cmd, args, { encoding: "utf8", maxBuffer: 1 << 28, stdio: ["pipe", "pipe", "pipe"], ...o });
const gdir = () => join(CLONE, ".git");
const g = (args, o = {}) => run("git", ["--git-dir", gdir(), "-c", "core.autocrlf=false", ...args], o).trim();

// Only one plain cargo check, test or clippy line goes to the runner (the workflow refuses the rest again).
export const okCmd = (cmd) => /^cargo (check|test|clippy) /.test(cmd) && !/[&;|`<>]|\$\(/.test(cmd);

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

export const main = async (argv) => {
  const cmd = argv.join(" ").trim();
  if (!okCmd(cmd)) { console.log("refused: give one plain `cargo check|test|clippy ...` command"); return 2; }
  if (!existsSync(gdir())) { console.log("no mirror clone yet: run node tools/farm.mjs once"); return 2; }
  const head = run("git", ["rev-parse", "--short", "HEAD"], { cwd: ROOT }).trim();
  const id = `wip-${Date.now().toString(36)}${Math.random().toString(36).slice(2, 5)}`;
  const t0 = Date.now(); const lap = () => `${Math.round((Date.now() - t0) / 1000)}s`;
  let pushed = false;
  try {
    run("git", ["--git-dir", gdir(), "fetch", "-q", "origin"]);
    const base = await baseTree(head);
    const tree = withIndex(id, (env) => {
      const e = { ...process.env, ...env };
      run("git", ["--git-dir", gdir(), "read-tree", base], { env: e });
      const lines = overlayLines();
      if (lines.length) run("git", ["--git-dir", gdir(), "update-index", "--add", "--index-info"], { env: e, input: `${lines.join("\n")}\n` });
      return run("git", ["--git-dir", gdir(), "write-tree"], { env: e }).trim();
    });
    const env = { ...process.env, GIT_AUTHOR_NAME: "maximkone", GIT_AUTHOR_EMAIL: "maximkone@users.noreply.github.com", GIT_COMMITTER_NAME: "maximkone", GIT_COMMITTER_EMAIL: "maximkone@users.noreply.github.com" };
    const commit = run("git", ["--git-dir", gdir(), "commit-tree", tree, "-p", "refs/remotes/origin/main", "-m", `wip ${id} of ${head}: ${cmd}`], { env }).trim();
    run("git", ["--git-dir", gdir(), "push", "-q", "origin", `${commit}:refs/heads/${id}`]); pushed = true;
    console.log(`[${lap()}] sent ${id} (HEAD ${head} + working tree) to the runner: ${cmd}`);
    try { run("gh", ["workflow", "run", "check.yml", "-R", PUBLIC_REPO, "--ref", id, "-f", `cmd=${cmd}`]); }
    catch (e) { console.log(`dispatch failed (is check.yml on the mirror's main? run node tools/farm.mjs once): ${String(e.stderr ?? e.message).split("\n")[0]}`); return 2; }
    let rid = null, last = null;
    for (let i = 0; i < 180; i++) {
      await sleep(10_000);
      const list = JSON.parse(run("gh", ["run", "list", "-R", PUBLIC_REPO, "--workflow", "check.yml", "--branch", id, "--json", "databaseId,status,conclusion", "-L", "1"]) || "[]");
      last = list[0] ?? null; rid = last?.databaseId ?? rid;
      if (last?.status === "completed") break;
    }
    if (!last || last.status !== "completed") { console.log(`[${lap()}] no result after 30 minutes (run ${rid ?? "not started"})`); return 2; }
    let log = ""; try { log = run("gh", ["run", "view", String(rid), "-R", PUBLIC_REPO, "--log"]); } catch (e) { log = String(e.stdout ?? ""); }
    const mine = log.split("\n").filter((l) => l.includes("\trun the command\t")).map((l) => l.split("\t").slice(2).join("\t").replace(/^\S+Z\s/, "").replace(/\x1b\[[0-9;]*m/g, ""));
    console.log(mine.slice(-70).join("\n"));
    console.log(`[${lap()}] ${last.conclusion === "success" ? "PASS" : "FAIL"} on the runner: ${cmd} (run https://github.com/${PUBLIC_REPO}/actions/runs/${rid})`);
    return last.conclusion === "success" ? 0 : 1;
  } catch (e) { console.log(`could not run the check: ${String(e?.stderr ?? e?.message ?? e).split("\n")[0]}`); return 2; }
  finally { if (pushed) { try { run("git", ["--git-dir", gdir(), "push", "-q", "origin", "--delete", id]); } catch { /* the branch is a throwaway */ } } }
};
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main(process.argv.slice(2)).then((c) => process.exit(c));

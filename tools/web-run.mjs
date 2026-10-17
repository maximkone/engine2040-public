// Run the web-build workflow (build, clippy, tests, headless Chrome) on an ISOLATED snapshot: HEAD plus only the paths you name.
//   node tools/web-run.mjs [--budget <s>] [--file <tree path>=<local file>] <path> [<path> ...]
//   node tools/web-run.mjs --wait web-build-xxxx [--budget <s>]
// Why not tools/gh-check.mjs: that overlays every dirty file of the shared tree, so another seat's half-written crate turns your
// run red. Here the tree is the committed HEAD snapshot (public-export rules: denylist, LFS bytes) plus exactly the files you list
// (a directory means every file in it, recursively; a path that does not exist locally is removed from the snapshot).
// `--file a/b=local` puts the bytes of another local file at a/b (for example a Cargo.lock that is HEAD plus your own hunk).
// How: the tree is committed on a throwaway `web-build-<id>` branch of the public mirror; the push starts .github/workflows/web-build.yml
// by itself (push: branches web-build-*). The tool waits, prints each job and its failed steps, and deletes the branch when the run ended.
// Exit 0 = the run succeeded, 1 = it failed, 2 = could not run, 3 = still running after the budget (not a failure: run the --wait line).
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const PUBLIC_REPO = "maximkone/engine2040-public";
const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const CLONE = join(ROOT, "target", "public-export", "repo");
const WORK = join(ROOT, "target", "web-run");
const GIT_DIR = join(CLONE, ".git");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const run = (cmd, args, o = {}) => execFileSync(cmd, args, { encoding: "utf8", maxBuffer: 1 << 28, stdio: ["pipe", "pipe", "pipe"], ...o });
const g = (args, o = {}) => run("git", ["--git-dir", GIT_DIR, "-c", "core.autocrlf=false", ...args], o).trim();

export const newBranch = (now = Date.now()) => `web-build-${now.toString(36)}${Math.random().toString(36).slice(2, 5).padEnd(3, "0")}`;
export const validBranch = (b) => /^web-build-[0-9a-z]{9,}$/.test(String(b));

// node tools/web-run.mjs ...  ->  { paths, files: [[treePath, localFile]], wait, budget, error }
export function parseArgs(argv) {
  const o = { paths: [], files: [], wait: null, budget: 100, error: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--wait") { o.wait = argv[++i] ?? ""; if (!validBranch(o.wait)) o.error ??= "--wait needs the branch the run printed (web-build-...)"; }
    else if (a === "--budget") { const n = Number(argv[++i]); if (n >= 1) o.budget = Math.min(n, 1740); else o.error ??= "--budget needs seconds (1 to 1740)"; }
    else if (a === "--file") {
      const [dst, ...src] = String(argv[++i] ?? "").split("=");
      if (!dst || !src.length) o.error ??= "--file needs <tree path>=<local file>"; else o.files.push([dst, src.join("=")]);
    } else if (a.startsWith("-")) o.error ??= `unknown flag ${a}`;
    else o.paths.push(a.replace(/\\/g, "/").replace(/^\.\//, ""));
  }
  if (!o.wait && !o.paths.length && !o.files.length) o.error ??= "name the paths to overlay on HEAD (the files your change touches)";
  for (const p of [...o.paths, ...o.files.map((f) => f[0])]) if (p.startsWith("/") || p.split("/").includes("..") || /^[A-Za-z]:/.test(p)) o.error ??= `${p}: a path inside the repo, please`;
  return o;
}

// Every file under a path of the working tree as repo-relative forward-slash paths; a path with nothing there is [] (it is removed).
const expand = (p) => {
  const abs = join(ROOT, p);
  if (!existsSync(abs)) return [];
  if (!statSync(abs).isDirectory()) return [p];
  return readdirSync(abs, { withFileTypes: true }).flatMap((e) => (e.name === ".git" || e.name === "target" ? [] : expand(`${p}/${e.name}`)));
};

const withIndex = (name, fn) => {
  const idx = join(WORK, `${name}.index`); mkdirSync(WORK, { recursive: true }); rmSync(idx, { force: true });
  try { return fn({ ...process.env, GIT_INDEX_FILE: idx }); } finally { rmSync(idx, { force: true }); }
};

// The HEAD snapshot as a tree object in the mirror clone: reuse the one tools/gh-check.mjs cached for this HEAD, else build it here.
const baseTree = async (head) => {
  const shared = join(ROOT, "target", "gh-check", `base-${head}.tree`);
  if (existsSync(shared)) return readFileSync(shared, "utf8").trim();
  const own = join(WORK, `base-${head}.tree`);
  if (existsSync(own)) return readFileSync(own, "utf8").trim();
  const { buildTree, scan } = await import("./public-export.mjs");
  const out = join(WORK, `base-${head}`); mkdirSync(out, { recursive: true });
  const b = buildTree({ out });
  const s = scan(b.tree);
  const secret = s.problems.filter((p) => /secret|token|key/i.test(p));
  if (secret.length) throw new Error(`snapshot refused: ${secret[0]}`);
  const tree = withIndex(`base-${head}`, (env) => {
    run("git", ["--git-dir", GIT_DIR, "--work-tree", b.tree, "-c", "core.autocrlf=false", "add", "-A"], { env });
    return run("git", ["--git-dir", GIT_DIR, "write-tree"], { env }).trim();
  });
  rmSync(out, { recursive: true, force: true }); mkdirSync(dirname(own), { recursive: true }); writeFileSync(own, tree);
  return tree;
};

// "mode sha<TAB>path" lines for update-index (mode 0 = remove), LF line endings like the public export.
const lines = (paths, files) => {
  const out = [];
  const put = (treePath, src) => {
    let buf = readFileSync(src);
    if (!buf.includes(0) && buf.includes(13)) buf = Buffer.from(buf.toString("utf8").replace(/\r\n/g, "\n"));
    out.push(`100644 ${g(["hash-object", "-w", "--stdin"], { input: buf })}\t${treePath}`);
  };
  for (const p of paths) {
    const found = expand(p);
    if (!found.length) out.push(`0 ${"0".repeat(40)}\t${p}`);
    for (const f of found) put(f, join(ROOT, f));
  }
  for (const [dst, src] of files) put(dst, resolve(ROOT, src));
  return out;
};

const lap = (t0) => `${Math.round((Date.now() - t0) / 1000)}s`;
const gh = (args) => run("gh", args);

// Print the run's jobs and failed steps; returns the conclusion.
const report = (rid, branch) => {
  const url = `https://github.com/${PUBLIC_REPO}/actions/runs/${rid}`;
  let data = null;
  try { data = JSON.parse(gh(["run", "view", String(rid), "-R", PUBLIC_REPO, "--json", "conclusion,jobs"])); } catch { /* the summary is best effort */ }
  for (const j of data?.jobs ?? []) {
    console.log(`  job ${j.name}: ${j.conclusion || j.status}`);
    for (const s of j.steps ?? []) if (s.conclusion && s.conclusion !== "success" && s.conclusion !== "skipped") console.log(`    step ${s.name}: ${s.conclusion}`);
  }
  console.log(`run ${url} (${branch})`);
  return data?.conclusion ?? null;
};

const tailFailed = (rid) => {
  try { console.log(run("gh", ["run", "view", String(rid), "-R", PUBLIC_REPO, "--log-failed"]).split("\n").slice(-80).join("\n")); }
  catch (e) { console.log(`no failed-step log yet: ${String(e.stderr ?? e.message).split("\n")[0]}`); }
};

const settle = async (branch, t0, budgetMs, started) => {
  let rid = null;
  for (;;) {
    try {
      const list = JSON.parse(gh(["run", "list", "-R", PUBLIC_REPO, "--workflow", "web-build.yml", "--branch", branch, "--json", "databaseId,status,conclusion", "-L", "1"]) || "[]");
      const last = list[0];
      if (last) {
        rid = last.databaseId;
        if (last.status === "completed") break;
      }
    } catch { /* a poll that fails is tried again */ }
    if (Date.now() - started + 20_000 > budgetMs) {
      console.log(`[${lap(t0)}] STILL RUNNING on the runner${rid ? ` (run ${rid})` : ""}, not a failure. Run: node tools/web-run.mjs --wait ${branch}`);
      return 3;
    }
    await sleep(15_000);
  }
  const conclusion = report(rid, branch);
  if (conclusion !== "success") tailFailed(rid);
  console.log(`[${lap(t0)}] ${conclusion === "success" ? "PASS" : "FAIL"}: web-build on ${branch}`);
  try { run("git", ["--git-dir", GIT_DIR, "push", "-q", "origin", "--delete", branch]); } catch { /* a throwaway branch */ }
  return conclusion === "success" ? 0 : 1;
};

export async function main(argv) {
  const a = parseArgs(argv);
  if (a.error) { console.log(`refused: ${a.error}`); return 2; }
  const t0 = Date.now(), started = Date.now(), budgetMs = a.budget * 1000;
  if (!existsSync(GIT_DIR)) { console.log("no mirror clone yet: run node tools/farm.mjs once"); return 2; }
  if (a.wait) return settle(a.wait, t0, budgetMs, started);
  const branch = newBranch();
  try {
    const head = run("git", ["rev-parse", "--short", "HEAD"], { cwd: ROOT }).trim();
    g(["fetch", "-q", "origin"]);
    const base = await baseTree(head);
    const tree = withIndex(branch, (env) => {
      run("git", ["--git-dir", GIT_DIR, "read-tree", base], { env });
      const ls = lines(a.paths, a.files);
      if (ls.length) run("git", ["--git-dir", GIT_DIR, "update-index", "--add", "--index-info"], { env, input: `${ls.join("\n")}\n` });
      return run("git", ["--git-dir", GIT_DIR, "write-tree"], { env }).trim();
    });
    const env = { ...process.env, GIT_AUTHOR_NAME: "maximkone", GIT_AUTHOR_EMAIL: "maximkone@users.noreply.github.com", GIT_COMMITTER_NAME: "maximkone", GIT_COMMITTER_EMAIL: "maximkone@users.noreply.github.com" };
    const commit = run("git", ["--git-dir", GIT_DIR, "commit-tree", tree, "-p", "refs/remotes/origin/main", "-m", `web-run ${branch} of ${head} + ${a.paths.length + a.files.length} path(s)`], { env }).trim();
    run("git", ["--git-dir", GIT_DIR, "push", "-q", "origin", `${commit}:refs/heads/${branch}`]);
    console.log(`[${lap(t0)}] sent ${branch} (HEAD ${head} + ${[...a.paths, ...a.files.map((f) => f[0])].join(" ")}); the push starts web-build.yml`);
  } catch (e) { console.log(`could not start the run: ${String(e?.stderr ?? e?.message ?? e).split("\n")[0]}`); return 2; }
  return settle(branch, t0, budgetMs, started);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) main(process.argv.slice(2)).then((c) => process.exit(c));

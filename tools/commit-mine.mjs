// commit-mine: commit only YOUR changes from a tree other AIs are editing at the same time.
//
// Owner 2026-10-24: the hand-rolled "temp GIT_INDEX_FILE + git apply --cached" dance cost every
// agent time and once put a hunk into the wrong function (two functions shared the same context
// lines, so `git apply` found the context in the wrong place and master stopped building).
// This tool does the same job safely:
//   - hunks are zero-context (`git diff -U0`), applied at their exact HEAD line numbers, so a
//     hunk can never land somewhere else;
//   - before the commit, the file in the temp index is compared with the file computed here
//     from HEAD plus exactly the picked hunks; any difference refuses the commit;
//   - the temp index is built from HEAD and the commit is refused if HEAD moved meanwhile
//     (a stale tree would undo another AI's commit).
//
//   node tools/commit-mine.mjs --list <path>...                  number the hunks against HEAD
//   node tools/commit-mine.mjs -m "<msg>" --pick <path>#1,<path>#3 [--whole <path>,...] [--push]
//
// `--whole` takes a file as it is on disk (a new file, one only you touched, a binary or LFS file,
// or a deletion when the path is gone); `--pick` takes
// the numbered hunks `--list` printed. Messages longer than one line: pass `-F <file>`.
// Exit 0 committed (or listed), 1 refused (nothing committed), 2 usage.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const git = (args, { cwd, env, input } = {}) =>
  execFileSync("git", args, { cwd, env: { ...process.env, ...env }, input, encoding: "utf8", maxBuffer: 1 << 28, stdio: ["pipe", "pipe", "pipe"] });

const norm = (s) => s.replace(/\r\n/g, "\n");

// One file's zero-context diff against HEAD: { header, hunks: [{ at, del, add: [...lines], text }] }.
export const parseDiff = (diff) => {
  const lines = norm(diff).split("\n");
  const header = [];
  const hunks = [];
  let cur = null;
  for (const line of lines) {
    const m = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (m) {
      cur = { at: Number(m[1]), del: m[2] === undefined ? 1 : Number(m[2]), add: [], text: [line] };
      hunks.push(cur);
      continue;
    }
    if (!cur) {
      if (line) header.push(line);
      continue;
    }
    if (line.startsWith("\\")) continue;
    if (line.startsWith("+")) cur.add.push(line.slice(1));
    if (line.startsWith("+") || line.startsWith("-")) cur.text.push(line);
  }
  return { header, hunks };
};

// HEAD's text plus exactly `hunks` (zero-context, at their HEAD line numbers).
export const applyHunks = (headText, hunks) => {
  const out = norm(headText).split("\n");
  for (const h of [...hunks].sort((a, b) => b.at - a.at)) {
    if (h.del === 0) out.splice(h.at, 0, ...h.add);
    else out.splice(h.at - 1, h.del, ...h.add);
  }
  return out.join("\n");
};

const diffOf = (path, cwd) => git(["diff", "-U0", "--no-color", "--no-ext-diff", "HEAD", "--", path], { cwd });

// The picked hunks as patch text with their `+` positions recomputed from the picked hunks
// alone. `git apply --unidiff-zero` places a pure insertion by its new-file line, and the diff's
// new-file lines assume every earlier hunk applied too: skip one and the next lands off by its
// size (found by this tool's own check on 2026-10-24).
export const renumber = (chosen) => {
  let offset = 0;
  return [...chosen]
    .sort((a, b) => a.at - b.at)
    .flatMap((h) => {
      // Old side: `-N,0` already means "after line N". New side: a deletion's `+M,0` names the
      // line before it, the way git writes it.
      const newStart = (h.del === 0 ? h.at + 1 : h.at) + offset;
      offset += h.add.length - h.del;
      const old = h.del === 1 ? `${h.at}` : `${h.at},${h.del}`;
      const add = h.add.length;
      const neu = add === 1 ? `${newStart}` : add === 0 ? `${newStart - 1},0` : `${newStart},${add}`;
      const tail = h.text[0].replace(/^@@ [^@]* @@/, "");
      return [`@@ -${old} +${neu} @@${tail}`, ...h.text.slice(1)];
    });
};

export const list = (paths, cwd, say = console.log) => {
  for (const path of paths) {
    const { hunks } = parseDiff(diffOf(path, cwd));
    if (!hunks.length) say(`${path}: no change against HEAD`);
    hunks.forEach((h, i) => {
      const first = h.text.slice(1).find((l) => l.trim().length > 1) ?? "";
      say(`${path}#${i + 1}  ${h.text[0]}  ${first.slice(0, 100)}`);
    });
  }
};

// Pull (merge, no rebase) and push master, up to three times when another push wins the race. A pre-push hook's
// refusal is no race: it stops at once and names the hook's reason (lead 2026-10-24: one AI's unparsable Cargo.toml
// made cargo deny refuse every push in the shared tree, and each AI read "the push lost three races").
export const pushMaster = (cwd, tries = 3) => {
  let why = "";
  for (let attempt = 1; attempt <= tries; attempt++) {
    try {
      git(["pull", "--no-rebase", "--no-edit", "origin", "master"], { cwd });
      git(["push", "origin", "master"], { cwd });
      return { ok: true, why: "" };
    } catch (e) {
      const lines = `${e?.stderr ?? ""}\n${e?.stdout ?? ""}`.split(/\r?\n/).map((l) => l.replace(/\x1b\[[0-9;]*m/g, "").trim()).filter(Boolean);
      const refused = lines.find((l) => /^refused:|hook declined/i.test(l));
      if (refused) return { ok: false, why: `the pre-push hook refused it: ${refused} (${lines.find((l) => /^error/i.test(l) && !/failed to push/.test(l)) ?? "run git push to read it"})` };
      why = lines.find((l) => /rejected|cannot lock ref|non-fast-forward/i.test(l)) ?? lines.at(-1) ?? "";
      // Another AI pushed in between: pull again and retry.
    }
  }
  return { ok: false, why: `the push lost ${tries} races (${why})` };
};

export const parseArgs = (argv) => {
  const out = { list: null, msg: null, msgFile: null, pick: [], whole: [], push: false, gapOk: false, error: null };
  // `--whole a b` takes both: a bare path after --pick/--whole joins that list (2026-10-24: the second
  // path was silently dropped and a commit left its new test file behind, breaking HEAD's build).
  let last = null;
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--list") out.list = argv.slice(i + 1).filter((p) => !p.startsWith("--"));
    else if (a === "-m") { out.msg = argv[++i] ?? null; last = null; }
    else if (a === "-F") { out.msgFile = argv[++i] ?? null; last = null; }
    else if (a === "--pick" || a === "--whole") { last = out[a.slice(2)]; last.push(...(argv[++i] ?? "").split(",").filter(Boolean)); }
    else if (a === "--push") { out.push = true; last = null; }
    else if (a === "--gap-ok") { out.gapOk = true; last = null; }
    else if (last && !a.startsWith("-")) last.push(...a.split(",").filter(Boolean));
    else out.error ??= `unknown argument ${a}`;
    if (out.list) break;
  }
  if (!out.list && !out.msg && !out.msgFile) out.error = "give --list <paths>, or -m/-F with --pick and/or --whole";
  if (!out.list && !out.pick.length && !out.whole.length) out.error ??= "nothing to commit: give --pick <path>#N or --whole <path>";
  for (const p of out.pick) if (!/#\d+$/.test(p)) out.error ??= `--pick wants <path>#N, got ${p}`;
  return out;
};

// The newest green gh-check --only set (target/gh-check/proved.jsonl, last 3 h) that covers every path of this
// commit, and the files it also carried that are still uncommitted: the proof built with them, so the commit
// may not build without them (SW23 proved with combat's new presentation.rs, committed without it, and
// engine-game's tests stopped compiling for every agent). `<path>#N` entries are skipped: other writers keep
// hunks in a shared file. live: the dirty and untracked paths (git status), injectable for tests.
export const proofGap = (paths, { cwd, now = Date.now(), live = null } = {}) => {
  const root = git(["rev-parse", "--show-toplevel"], { cwd }).trim();
  const file = join(root, "target", "gh-check", "proved.jsonl");
  if (!existsSync(file)) return null;
  const want = [...new Set(paths.map((p) => String(p).replace(/#\d+$/, "")))];
  const rows = readFileSync(file, "utf8").split("\n").filter(Boolean).slice(-500).reverse();
  const dirty = live ?? new Set(git(["status", "--porcelain=v1", "--untracked-files=all"], { cwd: root }).split("\n").filter((l) => l.length > 3).map((l) => l.slice(3).replace(/^"|"$/g, "")));
  for (const line of rows) {
    let r; try { r = JSON.parse(line); } catch { continue; }
    if (now - (r.t ?? 0) > 3 * 3600_000) break;
    const only = (r.only ?? []).map(String);
    const whole = only.filter((p) => !/#\d+$/.test(p));
    const covered = new Set(only.map((p) => p.replace(/#\d+$/, "")));
    if (!want.every((p) => covered.has(p))) continue;
    const extra = whole.filter((p) => !want.includes(p) && dirty.has(p));
    return extra.length ? { run: r.run, cmd: r.cmd, extra } : null;
  }
  return null;
};

// Commit the picked hunks and whole files. Returns { code, sha?, why? }.
export const commitMine = ({ msg, msgFile, pick, whole, cwd, push = false, gapOk = false, say = console.log }) => {
  const byPath = new Map();
  for (const p of pick) {
    const [path, n] = [p.replace(/#\d+$/, ""), Number(p.split("#").pop())];
    byPath.set(path, [...(byPath.get(path) ?? []), n]);
  }
  if (!gapOk) {
    const gap = proofGap([...byPath.keys(), ...whole], { cwd });
    if (gap) return { code: 1, why: `your green proof (run ${gap.run}: ${gap.cmd}) also carried ${gap.extra.join(", ")}, still uncommitted: without them this commit may not build at HEAD. Add them (--whole, or --pick your hunks), or prove again with --only just this commit's files. --gap-ok only when those files are another agent's unfinished work that your commit does not use.` };
  }
  const root = git(["rev-parse", "--show-toplevel"], { cwd }).trim();
  const dir = join(root, "target", "commit-mine");
  mkdirSync(dir, { recursive: true });
  const index = join(dir, `index-${process.pid}`);
  const env = { GIT_INDEX_FILE: index };
  try {
    const head = git(["rev-parse", "HEAD"], { cwd }).trim();
    git(["read-tree", "HEAD"], { cwd, env });
    const expect = new Map();
    for (const [path, ns] of byPath) {
      const diff = diffOf(path, cwd);
      const { header, hunks } = parseDiff(diff);
      if (header.some((l) => /^(new|deleted) file mode/.test(l))) return { code: 1, why: `${path} is a new or deleted file: use --whole` };
      const chosen = ns.map((n) => hunks[n - 1]);
      if (chosen.some((h) => !h)) return { code: 1, why: `${path} has ${hunks.length} hunks; --list shows their numbers` };
      const patch = [...header, ...renumber(chosen), ""].join("\n");
      git(["apply", "--cached", "--unidiff-zero", "--whitespace=nowarn", "-"], { cwd, env, input: patch });
      expect.set(path, applyHunks(git(["show", `HEAD:${path}`], { cwd }), chosen));
    }
    // A whole file goes through git's own filters (an LFS file stages its pointer, 2026-10-24: the
    // text compare refused every LFS and binary path), so it is checked by blob id; a whole path
    // gone from disk is a deletion.
    const blobs = new Map();
    const gone = new Set();
    for (const path of whole) {
      if (!existsSync(join(root, path))) {
        git(["rm", "--cached", "-q", "--", path], { cwd, env });
        gone.add(path);
        continue;
      }
      git(["add", "--", path], { cwd, env });
      blobs.set(path, git(["hash-object", "--", path], { cwd }).trim());
    }
    for (const [path, want] of expect) {
      const got = norm(git(["show", `:${path}`], { cwd, env }));
      if (got.replace(/\n+$/, "") !== want.replace(/\n+$/, "")) return { code: 1, why: `${path}: the staged file is not HEAD plus your hunks; nothing committed` };
    }
    for (const [path, blob] of blobs) {
      if (git(["rev-parse", `:${path}`], { cwd, env }).trim() !== blob) return { code: 1, why: `${path}: the staged blob is not the file on disk; nothing committed` };
    }
    for (const path of gone) {
      if (git(["ls-files", "--", path], { cwd, env }).trim()) return { code: 1, why: `${path}: the deletion did not stage; nothing committed` };
    }
    const paths = [...expect.keys(), ...blobs.keys(), ...gone];
    if (git(["rev-parse", "HEAD"], { cwd }).trim() !== head) return { code: 1, why: "HEAD moved while staging (another AI committed); run it again" };
    const file = msgFile ?? join(dir, `msg-${process.pid}.txt`);
    if (!msgFile) writeFileSync(file, msg);
    git(["commit", "-q", "-F", file], { cwd, env });
    const sha = git(["rev-parse", "--short", "HEAD"], { cwd }).trim();
    say(`committed ${sha}: ${paths.join(", ")}`);
    // The shared index never saw this commit: left alone, a new file reads as a staged deletion
    // there, and the next plain `git commit` by anyone would delete it. Point the shared index's
    // entries for these paths at the new HEAD (the working files stay as they are).
    try {
      git(["reset", "-q", "--", ...paths], { cwd });
    } catch {
      say("note: could not resync the shared index for these paths; run git reset -q -- <paths>");
    }
    // The commit stands from here on: a push that loses a race is retried, never re-committed.
    if (push) { const p = pushMaster(cwd); say(p.ok ? "pushed master" : `commit kept locally: ${p.why}; push it later (git pull --no-rebase --no-edit; git push)`); }
    return { code: 0, sha };
  } catch (err) {
    return { code: 1, why: String(err.stderr || err.message).trim().split("\n").slice(-3).join(" ") };
  } finally {
    rmSync(index, { force: true });
  }
};

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const a = parseArgs(process.argv.slice(2));
  if (a.list) {
    list(a.list, process.cwd());
    process.exit(0);
  }
  if (a.error) {
    console.log(`commit-mine: ${a.error}`);
    process.exit(2);
  }
  // A busy tree moves HEAD under us (another AI committed): rebuild from the new HEAD and retry.
  let res = { code: 1, why: "" };
  for (let attempt = 1; attempt <= 3; attempt++) {
    res = commitMine({ ...a, cwd: process.cwd() });
    if (!res.code || !/HEAD moved|cannot lock ref/.test(res.why ?? "")) break;
    console.log(`commit-mine: HEAD moved while staging; trying again (${attempt}/3)`);
  }
  if (res.code) console.log(`commit-mine: refused: ${res.why}`);
  process.exit(res.code);
}

// review-gate: the AGENTS.md review rules as one gate over one commit.
// Usage: node tools/review-gate.mjs [<commit-ish>=HEAD]
// Prints one line per rule R1-R6: PASS, or FAIL with the first broken path or
// line; WARN lines never fail (the shared tree makes dirty overlap common).
// R4 and R5 read the working tree, not the commit, and say so: another lane's
// uncommitted edit can colour them, the way it does for any local gate.
// Exit 0 pass, 1 a rule broke, 2 usage or git error.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { genreHits } from "./sim-source-scan.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// A rewrite signature: a big delete and a big re-add that barely changes the
// file length. It is the post-hoc trace of a whole-file write over someone
// else's dirty file, which loses their hunks.
export const REWRITE_LINES = 200;
export const REWRITE_KEEP = 0.1;

const USAGE = "usage: node tools/review-gate.mjs [<commit-ish>=HEAD]";

// ------------------------------------------------------------------ parsing

/** Undo git's C-style path quoting: `"a/b\tc.rs"` -> `a/b\tc.rs`. */
export function gitPath(raw) {
  const text = String(raw ?? "");
  if (!text.startsWith('"') || !text.endsWith('"')) return text;
  const body = text.slice(1, -1);
  return body.replace(/\\([0-7]{3}|.)/g, (_, esc) =>
    /^[0-7]{3}$/.test(esc) ? String.fromCharCode(parseInt(esc, 8)) : ({ t: "\t", n: "\n", r: "\r" }[esc] ?? esc),
  );
}

/** `git show --name-status --format=` rows: `{ status: "M", paths: ["a.rs"] }`.
 * A rename or copy carries the old path then the new one. */
export function parseNameStatus(text) {
  const rows = [];
  for (const line of String(text ?? "").split(/\r?\n/)) {
    if (!line.trim() || !line.includes("\t")) continue;
    const [status, ...paths] = line.split("\t");
    const letter = status.trim()[0];
    if (!letter || !paths.length) continue;
    rows.push({ status: letter, paths: paths.map(gitPath).filter(Boolean) });
  }
  return rows;
}

/** `git show --numstat --format=` rows keyed by new path: `{ added, deleted }`.
 * A binary file reads `-` and counts as 0. */
export function parseNumstat(text) {
  const out = new Map();
  for (const line of String(text ?? "").split(/\r?\n/)) {
    const parts = line.split("\t");
    if (parts.length < 3) continue;
    const added = Number(parts[0]);
    const deleted = Number(parts[1]);
    let path = parts[parts.length - 1];
    const arrow = path.lastIndexOf(" => ");
    if (arrow >= 0) path = path.slice(arrow + 4);
    out.set(gitPath(path), {
      added: Number.isFinite(added) ? added : 0,
      deleted: Number.isFinite(deleted) ? deleted : 0,
    });
  }
  return out;
}

/** Added lines of a `git show -p` patch per new path: `[{ line, text }]`, where
 * `line` is the line number in the new file. Deleted files collect nothing. */
export function parsePatch(text) {
  const files = new Map();
  let current = null;
  let newLine = 0;
  for (const raw of String(text ?? "").split("\n")) {
    if (raw.startsWith("diff --git ") || raw.startsWith("diff --cc ")) {
      current = null;
      continue;
    }
    if (raw.startsWith("+++ ")) {
      const path = gitPath(raw.slice(4).trim()).replace(/^b\//, "");
      current = path === "/dev/null" ? null : (files.get(path) ?? []);
      if (current) files.set(path, current);
      newLine = 0;
      continue;
    }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(raw);
    if (hunk) {
      newLine = Number(hunk[1]);
      continue;
    }
    if (!current || raw.startsWith("\\")) continue;
    if (raw.startsWith("+")) {
      current.push({ line: newLine, text: raw.slice(1) });
      newLine += 1;
    } else if (raw.startsWith(" ")) {
      newLine += 1;
    }
  }
  return files;
}

// ------------------------------------------------------------------- rules

/** R1: a reviewable commit is no merge, has a message, and touches its own
 * paths. A path the working tree also changes only warns: the shared tree
 * leaves other lanes' hunks in place, so overlap is common and not a defect. */
export function checkByPath({ isMerge = false, message = "", paths = [], dirtyPaths = [] }) {
  const broken = [];
  if (isMerge) broken.push("the commit is a merge; review the squashed change instead");
  if (!String(message).trim()) broken.push("the message is empty");
  if (broken.length) {
    return { ok: false, line: `FAIL R1 commit by path: ${broken.join("; ")}`, warnings: [] };
  }
  const dirty = new Set(dirtyPaths);
  const warnings = paths
    .filter((path) => dirty.has(path))
    .map((path) => `WARN R1 dirty overlap: ${path} is also uncommitted in the working tree`);
  return { ok: true, line: `PASS R1 commit by path: ${paths.length} path(s), message present`, warnings };
}

/** R2: the rewrite signature of one file, or null. */
export function rewriteSignature({ path, added = 0, deleted = 0, before = 0, after = 0 }) {
  if (!(added > REWRITE_LINES && deleted > REWRITE_LINES)) return null;
  if (!(before > 0)) return null;
  if (Math.abs(after - before) >= before * REWRITE_KEEP) return null;
  return `${path} deletes ${deleted} and re-adds ${added} of ${before} lines (${before} -> ${after} lines)`;
}

/** R2 over rows of `{ path, added, deleted, before, after }`. */
export function checkRewrites(rows = []) {
  for (const row of rows) {
    const hit = rewriteSignature(row);
    if (hit) return { ok: false, line: `FAIL R2 no whole-file rewrite: ${hit}` };
  }
  return { ok: true, line: `PASS R2 no whole-file rewrite: ${rows.length} modified file(s) checked` };
}

/** The GE2 ledger set, read from tools/workspace-check.mjs so there is one
 * list. A parse that finds too few entries is stale: the gate refuses to run
 * with an empty ledger. */
export function parseGenreLedger(source) {
  const block = /const genreLedger = new Set\(\[([\s\S]*?)\]\);/.exec(String(source ?? ""));
  if (!block) throw new Error("cannot read the GE2 genre ledger from tools/workspace-check.mjs");
  const files = new Set([...block[1].matchAll(/"([^"]+)"/g)].map((match) => match[1]));
  if (files.size < 20) throw new Error(`GE2 genre ledger parsed only ${files.size} entries; the format moved`);
  return files;
}

/** R3 for one Rust file: the genre identifiers the commit adds to it, if the
 * ledger does not list it. Comments, strings and test modules do not count.
 * `added` (patch rows `{ line, text }`) only locates the first new name. */
export function genreDelta({ path = "", key = path, before = "", after = "", added = [], ledger }) {
  if (ledger.has(key)) return null;
  const known = new Set(genreHits(before));
  const names = genreHits(after).filter((name) => !known.has(name));
  if (!names.length) return null;
  const at = added.find((row) => names.some((name) => genreHits(row.text).includes(name)));
  return { path, names, line: at?.line };
}

/** R3 over rows of `{ path, key, before, after, added }` (full text at both
 * sides). */
export function checkGenre({ files = [], ledger }) {
  for (const file of files) {
    const hit = genreDelta({ ...file, ledger });
    if (hit) {
      const where = hit.line ? `${hit.path}:${hit.line}` : hit.path;
      return {
        ok: false,
        line: `FAIL R3 no new genre name: ${where} adds ${hit.names.join(", ")} (GE2 ledger; use a neutral name or alias)`,
      };
    }
  }
  return { ok: true, line: `PASS R3 no new genre names: ${files.length} Rust file(s), ${ledger.size} ledger entries` };
}

/** R4: the ratchet speaks for itself; this reads its RESULT line. It follows
 * the working tree, not the commit, so the label says so. */
export function ratchetVerdict({ touchedRs = false, output = "" } = {}) {
  if (!touchedRs) return { ok: true, line: "PASS R4 ratchet: skipped, no .rs file in the commit" };
  const result = String(output)
    .split(/\r?\n/)
    .filter((line) => line.startsWith("RESULT "))
    .pop();
  if (result && result.startsWith("RESULT PASS")) {
    return { ok: true, line: `PASS R4 ratchet (working tree): ${result}` };
  }
  return { ok: false, line: `FAIL R4 ratchet (working tree): ${result ?? "no RESULT line"}` };
}

/** R5 over `{ path, status }` rows from `rustfmt --check`. Missing rustfmt is a
 * skip, never a pass for files it did not read. */
export function rustfmtVerdict({ available = true, rows = [] } = {}) {
  if (!available) return { ok: true, line: "PASS R5 rustfmt: skipped, rustfmt is not on PATH" };
  if (!rows.length) return { ok: true, line: "PASS R5 rustfmt: skipped, no .rs file in the commit" };
  const bad = rows.filter((row) => row.status !== 0);
  if (bad.length) {
    const first = bad[0];
    return {
      ok: false,
      line: `FAIL R5 rustfmt: ${first.path} is not formatted (rustfmt --edition 2021 --check)${bad.length > 1 ? `; ${bad.length - 1} more` : ""}`,
    };
  }
  return { ok: true, line: `PASS R5 rustfmt: ${rows.length} file(s) formatted` };
}

/** R6: a proof line carries `Proof:` (the paper's first review rule) or the
 * message pairs a backticked command with a pass/fail word. */
export function proofFinding(message) {
  const text = String(message ?? "");
  if (/(Proof|proof):/.test(text)) return null;
  const command = /`[^`\n]+`/.test(text);
  const verdict = /\b(?:pass|passes|passed|fail|fails|failed|green|red)\b/i.test(text);
  if (command && verdict) return null;
  return "no proof line: add `Proof: <command> (<n> passed)` or a backticked command with a pass/fail word";
}

/** R6 over `{ message }`. */
export function checkProof({ message = "" } = {}) {
  const finding = proofFinding(message);
  return finding ? { ok: false, line: `FAIL R6 proof line: ${finding}` } : { ok: true, line: "PASS R6 proof line: the message names its proof" };
}

// ------------------------------------------------------------------ review

/** All six rules over plain data, no git: `{ isMerge, message, paths,
 * dirtyPaths, rewrites, genreFiles, ledger, touchedRs, ratchetOutput,
 * rustfmt }`. Returns `{ ok, lines }` with WARN lines after their rule. */
export function review(data = {}) {
  const results = [
    checkByPath(data),
    checkRewrites(data.rewrites ?? []),
    checkGenre({ files: data.genreFiles ?? [], ledger: data.ledger ?? new Set() }),
    ratchetVerdict({ touchedRs: data.touchedRs ?? false, output: data.ratchetOutput ?? "" }),
    rustfmtVerdict(data.rustfmt ?? {}),
    checkProof(data),
  ];
  const lines = [];
  for (const result of results) {
    lines.push(result.line);
    for (const warning of result.warnings ?? []) lines.push(warning);
  }
  const broken = results.filter((result) => !result.ok).length;
  return { ok: broken === 0, lines, broken };
}

// --------------------------------------------------------------------- git

function run(cmd, args, { allowFail = false } = {}) {
  const result = spawnSync(cmd, args, { cwd: ROOT, encoding: "utf8", maxBuffer: 512 * 1024 * 1024 });
  if (result.error) {
    if (allowFail) return { status: 1, stdout: "", stderr: String(result.error.message) };
    throw new Error(`${cmd} ${args.join(" ")}: ${result.error.message}`);
  }
  if (result.status !== 0 && !allowFail) {
    throw new Error(`${cmd} ${args.join(" ")}: ${String(result.stderr ?? "").trim() || `exit ${result.status}`}`);
  }
  return result;
}

function git(args, options) {
  return run("git", args, options);
}

/** Working tree paths that differ from HEAD, renames keyed by new path. */
export function dirtyPathsFrom(porcelain) {
  const out = new Set();
  for (const line of String(porcelain ?? "").split(/\r?\n/)) {
    if (line.length < 4) continue;
    let path = line.slice(3);
    const arrow = path.indexOf(" -> ");
    if (arrow >= 0) path = path.slice(arrow + 4);
    out.add(gitPath(path.trim()));
  }
  return out;
}

function lineCount(text) {
  if (!text) return 0;
  const body = text.endsWith("\n") ? text.slice(0, -1) : text;
  return body.split("\n").length;
}

function fileAt(sha, path) {
  const result = git(["show", `${sha}:${path}`], { allowFail: true });
  return result.status === 0 ? result.stdout : null;
}

/** Text rows for R2: only a modified file can carry the rewrite signature. */
function rewriteRows(sha, rows, stats) {
  const out = [];
  for (const row of rows) {
    if (row.status !== "M") continue;
    const path = row.paths[row.paths.length - 1];
    const counts = stats.get(path);
    if (!counts || counts.added <= REWRITE_LINES || counts.deleted <= REWRITE_LINES) continue;
    const beforeText = fileAt(`${sha}^`, path);
    if (beforeText === null) continue;
    const afterText = fileAt(sha, path) ?? "";
    out.push({ path, added: counts.added, deleted: counts.deleted, before: lineCount(beforeText), after: lineCount(afterText) });
  }
  return out;
}

/** The files the GE2 ledger gate scans: `crates/<pkg>/src/**`, minus the arena,
 * game and viewer packages the ledger leaves out by design. */
export function genreScope(path) {
  const match = /^crates\/([^/]+)\/src\/.+\.rs$/.exec(String(path));
  return Boolean(match) && !["arena", "game", "viewer"].includes(match[1]);
}

function genreRows(sha, rows, ledger, patch) {
  const out = [];
  for (const row of rows) {
    if (row.status === "D") continue;
    const path = row.paths[row.paths.length - 1];
    if (!genreScope(path)) continue;
    const key = path.replace(/^crates\//, "");
    if (ledger.has(key)) continue;
    const from = row.status === "R" ? row.paths[0] : path;
    out.push({
      path,
      key,
      before: fileAt(`${sha}^`, from) ?? "",
      after: fileAt(sha, path) ?? "",
      added: patch.get(path) ?? [],
    });
  }
  return out;
}

function rustfmtRows(paths) {
  if (!paths.length) return { available: true, rows: [] };
  const probe = run("rustfmt", ["--version"], { allowFail: true });
  if (probe.status !== 0) return { available: false, rows: [] };
  const rows = [];
  for (const path of paths) {
    const result = run("rustfmt", ["--edition", "2021", "--check", path], { allowFail: true });
    rows.push({ path, status: result.status ?? 1 });
  }
  return { available: true, rows };
}

export function main(argv = []) {
  if (argv.includes("-h") || argv.includes("--help")) {
    console.log(USAGE);
    return 0;
  }
  const target = argv[0] ?? "HEAD";
  if (argv.length > 1) {
    console.error(USAGE);
    return 2;
  }

  try {
    const resolved = git(["rev-parse", "--verify", `${target}^{commit}`]);
    const sha = resolved.stdout.trim();
    const short = git(["rev-parse", "--short", sha]).stdout.trim();
    const parents = git(["rev-list", "--parents", "-n", "1", sha]).stdout.trim().split(/\s+/);
    const isMerge = parents.length > 2;
    const message = git(["show", "-s", "--format=%B", sha]).stdout;
    const rows = parseNameStatus(git(["show", "--name-status", "--format=", sha]).stdout).map((row) => ({
      ...row,
      path: row.paths[row.paths.length - 1],
    }));
    const shown = git(["show", "--numstat", "-p", "--format=", sha]).stdout;
    const stats = parseNumstat(shown);
    const patch = parsePatch(shown);
    const dirty = dirtyPathsFrom(git(["status", "--porcelain"]).stdout);
    const ledger = parseGenreLedger(readFileSync(join(ROOT, "tools", "workspace-check.mjs"), "utf8"));

    const rsPaths = rows.filter((row) => row.status !== "D" && row.path.endsWith(".rs")).map((row) => row.path);
    const touchedRs = rsPaths.length > 0;
    const ratchet = touchedRs
      ? run("node", ["tools/rust-quality.mjs"], { allowFail: true })
      : { status: 0, stdout: "" };
    const rustfmt = rustfmtRows(rsPaths.filter((path) => existsSync(join(ROOT, path))));

    const verdict = review({
      isMerge,
      message,
      paths: rows.map((row) => row.path),
      dirtyPaths: [...dirty],
      rewrites: rewriteRows(sha, rows, stats),
      genreFiles: genreRows(sha, rows, ledger, patch),
      ledger,
      touchedRs,
      ratchetOutput: `${ratchet.stdout ?? ""}\n${ratchet.stderr ?? ""}`,
      rustfmt,
    });

    console.log(`review-gate ${short}: ${rows.length} path(s)`);
    for (const line of verdict.lines) console.log(line);
    console.log(verdict.ok ? `RESULT PASS: ${verdict.lines.filter((line) => line.startsWith("PASS ")).length} rule leg(s)` : `RESULT FAIL: ${verdict.broken} rule(s) broken`);
    return verdict.ok ? 0 : 1;
  } catch (error) {
    console.error(`review-gate: ${error.message}`);
    return 2;
  }
}

if (import.meta.main) {
  process.exitCode = main(process.argv.slice(2));
}

// ticked-rows-audit: which TICKED board rows lost the code they were ticked for.
//
// Auto-backup sweeps and dead-code purges deleted Rust that done rows still depend on
// (MP5 lagcomp.rs, HR2 live.rs). This is a static, cargo-free read of the repo. For every
// `- [x] ID` row it checks, from the row text alone:
//   crate-missing        a backticked `cargo test ... -p <crate>` names a package nobody has
//   test-target-missing  `--test <name>` has no tests/<name>.rs, tests/<name>/ or [[test]]
//   test-target-moved    the same, but tests/it/<name>.rs (or a sibling dir) exists: stale command
//   filter-missing       no `fn <name>` / `mod <name>` / file or dir name holds the test filter
//                        (cargo filters are substring matches; `a::b` needs every segment)
//   path-missing         a `crates/<c>/src/<file>.rs` path in the row is not in git any more
//   path-moved           the same, but a file of that name lives elsewhere in the crate: stale path
// Evidence names the commit that removed the file or last changed the filter, when git knows it.
// It never runs cargo and never edits a row. Commands without -p are skipped.
//
// Rows come from board-slice (live board plus its shards, then the done rows moved to
// docs/archive/SPRINT-done-*.md, read whole so their Accept lines and paths count), then the
// two older moved-row snapshots board-slice does not read (see HISTORY_FILES).
//
//   node tools/ticked-rows-audit.mjs [--json] [--no-history] [--root <dir>]
//
// Exit codes: 0 no suspect row - 1 at least one suspect row - 2 usage or git failure.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { cliArgs, usageFail } from "./argv.mjs";
import { archiveDoneFiles, loadBoardText, parseBoard } from "./board-slice.mjs";
import { crateRows, packageName } from "./crate-map.mjs";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const PREFIX = "ticked-rows-audit";
const GIT_TIMEOUT_MS = 30_000;

const HELP = `ticked-rows-audit.mjs - which ticked board rows lost their code (static, no cargo)

Usage:
  node tools/ticked-rows-audit.mjs [--json] [--no-history] [--root <dir>]

For every "- [x] ID" row (board, shards, docs/archive/SPRINT-done-*.md, older snapshots) it checks:
  crate-missing        cargo test -p <crate>: no such package
  test-target-missing  cargo test --test <name>: no tests/<name>.rs, tests/<name>/ or [[test]]
  test-target-moved    same, but the test file now lives under tests/it/ (stale command)
  filter-missing       cargo test ... <filter>: no fn / mod / file name holds the filter
  path-missing         crates/<c>/src/<file>.rs named in the row is not in git
  path-moved           same, but a file of that name lives elsewhere in the crate (stale path)
Output: one line per suspect row "ID  reasons  evidence", then a SUMMARY line.

Options:
  --json         machine output: { root, archived, ticked, commands, paths, unchecked,
                 suspects: [{ id, source, findings: [{ reason, evidence }] }] }
  --no-history   skip docs/archive/SPRINT-2026-10-05.md and docs/SPRINT-ARCHIVE.md (older moved
                 rows that board-slice does not read; read by default)
  --root <dir>   audit another checkout (default: this repo)
  -h, --help     print this help and exit 0

Exit codes: 0 no suspect row - 1 at least one suspect row - 2 usage or git failure.
`;

// ---- rows -------------------------------------------------------------------------------

// Older moved-row snapshots: board-slice never reads them, but their rows are ticked too.
const HISTORY_FILES = ["docs/archive/SPRINT-2026-10-05.md", "docs/SPRINT-ARCHIVE.md"];

const lf = (t) => (t.includes("\r") ? t.replace(/\r\n/g, "\n") : t);

// Every ticked row once, as { id, source, text }. The live board wins over the archives
// (same first-wins rule as board-slice's archived rows); `text` joins the row's lines.
export function tickedRows(root = ROOT, { history = true } = {}) {
  const board = parseBoard(lf(loadBoardText(root)));
  const seen = new Set(board.tasks.keys());
  const rows = [];
  for (const t of board.tasks.values()) {
    if (t.done) rows.push({ id: t.id, source: "board", text: t.lines.join(" ") });
  }
  const extra = history ? HISTORY_FILES.map((f) => join(root, f)).filter((f) => existsSync(f)) : [];
  for (const file of [...archiveDoneFiles(root), ...extra]) {
    const source = relative(root, file).replaceAll("\\", "/");
    for (const t of parseBoard(lf(readFileSync(file, "utf8"))).tasks.values()) {
      if (!t.done || seen.has(t.id)) continue;
      seen.add(t.id);
      rows.push({ id: t.id, source, text: t.lines.join(" ") });
    }
  }
  return rows;
}

// ---- cargo test commands ----------------------------------------------------------------

// cargo flags that take the next token as a value (so it is never a test filter).
const VALUE_FLAGS = new Set([
  "-F", "-j", "-Z", "--bench", "--bin", "--color", "--config", "--example", "--exclude", "--features",
  "--jobs", "--manifest-path", "--message-format", "--profile", "--target", "--target-dir",
]);
// libtest flags (after `--`) that take a value.
const HARNESS_VALUE_FLAGS = new Set([
  "--color", "--format", "--logfile", "--report-time", "--shuffle-seed", "--skip", "--test-threads", "-Z",
]);
const FILTER_SHAPE = /^[A-Za-z_]\w*(?:::\w*)*$/;
const unquote = (t) => t.replace(/^["']+|["']+$/g, "");

function parseOne(fragment) {
  if (/[<>${}]/.test(fragment)) return null; // `cargo test -p <crate>` is a template, not a command
  const tok = fragment.trim().split(/\s+/).map(unquote).filter(Boolean);
  let i = tok.indexOf("cargo");
  if (i < 0) return null;
  i += 1;
  if (tok[i]?.startsWith("+")) i += 1;
  if (tok[i] !== "test") return null;
  const cmd = { text: tok.slice(tok.indexOf("cargo")).join(" "), crates: [], targets: [], lib: false, exact: false, filters: [] };
  const addFilter = (t) => {
    if (t && FILTER_SHAPE.test(t) && !cmd.filters.includes(t)) cmd.filters.push(t);
  };
  const addCrate = (t) => {
    const name = (t ?? "").replace(/@.*$/, "");
    if (name && !/[*?[]/.test(name) && !cmd.crates.includes(name)) cmd.crates.push(name);
  };
  let harness = false;
  for (i += 1; i < tok.length; i += 1) {
    const t = tok[i];
    if (harness) {
      if (HARNESS_VALUE_FLAGS.has(t)) i += 1;
      else if (t === "--exact") cmd.exact = true;
      else if (!t.startsWith("-")) addFilter(t);
    } else if (t === "--") harness = true;
    else if (t === "-p" || t === "--package") { addCrate(tok[i + 1]); i += 1; }
    else if (/^--package=./.test(t)) addCrate(t.slice(10));
    else if (/^-p=?.+/.test(t)) addCrate(t.replace(/^-p=?/, ""));
    else if (t === "--test") { if (tok[i + 1]) cmd.targets.push(tok[i + 1]); i += 1; }
    else if (/^--test=./.test(t)) cmd.targets.push(t.slice(7));
    else if (t === "--lib" || t === "--doc") cmd.lib = true;
    else if (VALUE_FLAGS.has(t)) i += 1;
    else if (!t.startsWith("-")) addFilter(t);
  }
  return cmd;
}

// The cargo test commands inside one row's backticked spans, whitespace-normalized and
// deduplicated. A span may chain commands with && ; | and may carry an env prefix.
export function cargoTestCommands(text) {
  const out = [];
  const seen = new Set();
  for (const span of text.matchAll(/`([^`]+)`/g)) {
    if (!/\bcargo\b/.test(span[1])) continue;
    for (const fragment of span[1].split(/&&|\|\||;|\|/)) {
      const cmd = parseOne(fragment);
      if (!cmd || seen.has(cmd.text)) continue;
      seen.add(cmd.text);
      out.push(cmd);
    }
  }
  return out;
}

// `crates/<c>/src/<file>.rs` (also tests/benches/examples) paths named in the row text.
// Not preceded by a path character, so `donor/crates/x/src/y.rs` is not ours.
const PATH_RE = /(?<![\w./\\-])crates\/[\w-]+\/(?:src|tests|benches|examples)\/[\w/.-]*?\.rs(?![\w])/g;
export function sourcePaths(text) {
  return [...new Set([...text.matchAll(PATH_RE)].map((m) => m[0]))];
}

// ---- name pools -------------------------------------------------------------------------

const stripLineComments = (src) => src.replace(/^[ \t]*\/\/.*$/gm, "");

// { fns, mods, fnText, modText, files }: the fn names, the mod / file / dir names of a set of
// source files. `*Text` joins the names with spaces so a substring test never straddles two.
function poolOf(entries, read) {
  const fns = new Set();
  const mods = new Set();
  for (const { path, rel } of entries) {
    const src = read(path);
    if (src != null) {
      const code = stripLineComments(src);
      for (const m of code.matchAll(/\bfn\s+(?:r#)?([A-Za-z_]\w*)/g)) fns.add(m[1]);
      for (const m of code.matchAll(/\bmod\s+([A-Za-z_]\w*)/g)) mods.add(m[1]);
    }
    for (const part of rel.split("/")) {
      const name = part.replace(/\.rs$/, "");
      if (name && !["mod", "lib", "main"].includes(name)) mods.add(name);
    }
  }
  return { fns, mods, fnText: ` ${[...fns].join(" ")} `, modText: ` ${[...mods].join(" ")} `, files: entries.length };
}

// Cargo's filter is a substring test on `mod::path::fn`. Segments before the last name
// modules; the last names a fn or a module; a trailing `::` makes the last one a module.
// `exact` (libtest --exact) turns every substring test into equality.
export function filterMatches(pool, filter, exact = false) {
  const segs = filter.split("::");
  const trailing = segs.at(-1) === "";
  const real = segs.filter(Boolean);
  if (!real.length) return true;
  const has = (set, text, s) => (exact ? set.has(s) : text.includes(s));
  const inMods = (s) => has(pool.mods, pool.modText, s);
  const inAny = (s) => inMods(s) || has(pool.fns, pool.fnText, s);
  return real.every((s, k) => (k === real.length - 1 && !trailing ? inAny(s) : inMods(s)));
}

// ---- the audit --------------------------------------------------------------------------

const SCOPE_DIRS = ["src", "tests", "examples", "benches"];

// ctx: { files: Set<string> (repo-relative, forward slashes), packages: Map<name, dir>,
//        read(path) -> string|null, deleted: Set<string> (tracked, gone from disk; optional),
//        removedBy(path) / lastTouch(needle, dir) -> {sha, subject} | false (no such commit) | null (unknown) }
//        (the last two are optional history hints)
export function auditRows(rows, ctx) {
  const rs = [...ctx.files].filter((f) => f.endsWith(".rs"));
  const hint = (h) => (h ? ` ${h.sha} "${h.subject}"` : "");
  const under = (dir, sub) => (dir === "." ? `${sub}/` : `${dir}/${sub}/`);
  const entriesUnder = (dir, sub) => {
    const prefix = under(dir, sub);
    return rs.filter((f) => f.startsWith(prefix)).map((path) => ({ path, rel: path.slice(prefix.length) }));
  };
  const cargoToml = (dir) => ctx.read(dir === "." ? "Cargo.toml" : `${dir}/Cargo.toml`) ?? "";

  // Files of integration test target `name`: tests/<name>.rs, tests/<name>/**, or a [[test]] path.
  const targetEntries = (dir, name) => {
    const single = `${under(dir, "tests")}${name}.rs`;
    const entries = ctx.files.has(single) ? [{ path: single, rel: `${name}.rs` }] : [];
    entries.push(...entriesUnder(dir, `tests/${name}`));
    if (entries.length) return entries;
    const block = [...cargoToml(dir).matchAll(/\[\[test\]\]([^[]*)/g)].map((m) => m[1]).find((b) => new RegExp(`name\\s*=\\s*"${name}"`).test(b));
    const path = block && /path\s*=\s*"([^"]+)"/.exec(block)?.[1];
    if (!path) return []; // a [[test]] without path is tests/<name>.rs again, which is not there
    const full = dir === "." ? path : `${dir}/${path}`;
    const folder = full.endsWith("/main.rs") ? full.slice(0, -"main.rs".length) : null;
    if (folder) return rs.filter((f) => f.startsWith(folder)).map((p) => ({ path: p, rel: p.slice(folder.length) }));
    return ctx.files.has(full) ? [{ path: full, rel: full.split("/").at(-1) }] : [];
  };

  const poolCache = new Map();
  const poolFor = (key, build) => {
    if (!poolCache.has(key)) poolCache.set(key, poolOf(build(), (p) => ctx.read(p)));
    return poolCache.get(key);
  };

  // The pools one command's filters are tried against, one per resolved crate.
  const scopePools = (cmd, dir) => {
    if (cmd.targets.length || cmd.lib) {
      const parts = [];
      if (cmd.lib) parts.push(["src", () => entriesUnder(dir, "src")]);
      for (const t of cmd.targets) parts.push([`test:${t}`, () => targetEntries(dir, t)]);
      return [poolFor(`${dir}|${parts.map((p) => p[0]).join(",")}`, () => parts.flatMap((p) => p[1]()))];
    }
    return [poolFor(`${dir}|all`, () => SCOPE_DIRS.flatMap((s) => entriesUnder(dir, s)))];
  };

  // History hint for a gone path: removed by <commit>, never in git, or nothing (git unavailable).
  const goneNote = (path) => {
    if (ctx.deleted?.has(path)) return "deleted in the working tree, not committed yet";
    const gone = ctx.removedBy?.(path);
    if (gone) return `removed by${hint(gone)}`;
    return gone === false ? "never in git history" : "";
  };
  const crateOf = (path) => `${path.split("/").slice(0, 2).join("/")}/`;
  const sameName = (path) => {
    const base = `/${path.split("/").at(-1)}`;
    return [...ctx.files].filter((f) => f.endsWith(base));
  };

  const crateFindings = (cmd) => {
    const findings = [];
    const resolved = [];
    for (const name of cmd.crates) {
      const dir = ctx.packages.get(name);
      if (dir === undefined) {
        const short = name.replace(/^engine-/, "");
        const note = goneNote(`crates/${short}/Cargo.toml`);
        findings.push({ reason: "crate-missing", evidence: `-p ${name}: no such package${note ? ` (crates/${short}/Cargo.toml ${note})` : ""}` });
      } else resolved.push({ name, dir });
    }
    if (!resolved.length) return findings;
    let readable = resolved;
    for (const target of cmd.targets) {
      if (resolved.some((c) => targetEntries(c.dir, target).length)) continue;
      // The one-binary layout moved tests/<name>.rs to tests/it/<name>.rs: say so.
      const now = resolved.flatMap((c) => entriesUnder(c.dir, "tests").filter((e) => e.rel.endsWith(`/${target}.rs`) || e.rel.includes(`/${target}/`)).map((e) => e.path));
      const where = resolved.map((c) => c.dir).join(", ");
      findings.push(now.length
        ? { reason: "test-target-moved", evidence: `--test ${target}: no such test binary in ${where}, now ${now.slice(0, 2).join(", ")} (${cmd.text})` }
        : { reason: "test-target-missing", evidence: `--test ${target}: no tests/${target}.rs, tests/${target}/ or [[test]] in ${where} (${cmd.text})` });
      readable = [];
    }
    if (!readable.length) return findings;
    const pools = readable.flatMap((c) => scopePools(cmd, c.dir));
    const scope = readable.map((c) => (cmd.targets.length ? cmd.targets.map((t) => `${under(c.dir, "tests")}${t}`) : [cmd.lib ? under(c.dir, "src").slice(0, -1) : c.dir]).join("+")).join(", ");
    for (const filter of cmd.filters) {
      if (pools.some((p) => filterMatches(p, filter, cmd.exact))) continue;
      const bad = filter.split("::").filter(Boolean).find((s) => !pools.some((p) => filterMatches(p, s, cmd.exact))) ?? filter;
      const dir = readable[0].dir === "." ? "src" : readable[0].dir;
      const last = ctx.lastTouch?.(bad, dir);
      const lastNote = last ? `; last changed in${hint(last)}` : last === false ? `; no commit under ${dir} ever held it` : "";
      const fnCount = pools.reduce((n, p) => n + p.fns.size, 0);
      const files = pools.reduce((n, p) => n + p.files, 0);
      findings.push({ reason: "filter-missing", evidence: `${cmd.text}: no fn, mod or file name holds "${bad}" in ${scope} (${fnCount} fns, ${files} files)${lastNote}` });
    }
    return findings;
  };

  const pathFindings = (text) => {
    const findings = [];
    for (const path of sourcePaths(text)) {
      if (ctx.files.has(path) || ctx.files.has(`${path.slice(0, -3)}/mod.rs`)) continue;
      const same = sameName(path);
      const near = same.filter((f) => f.startsWith(crateOf(path)));
      const note = goneNote(path);
      if (near.length) {
        findings.push({ reason: "path-moved", evidence: `${path}: now ${near.slice(0, 3).join(", ")}${note ? ` (old path ${note})` : ""}` });
      } else {
        const far = same.slice(0, 3);
        findings.push({ reason: "path-missing", evidence: `${path}: ${note || "not in git"}${far.length ? `; same file name in another crate: ${far.join(", ")}` : ""}` });
      }
    }
    return findings;
  };

  const suspects = [];
  let commands = 0;
  let paths = 0;
  let unchecked = 0;
  for (const row of rows) {
    const cmds = cargoTestCommands(row.text).filter((c) => c.crates.length);
    const named = sourcePaths(row.text);
    commands += cmds.length;
    paths += named.length;
    if (!cmds.length && !named.length) unchecked += 1; // names nothing this tool can look up
    const all = [...cmds.flatMap(crateFindings), ...pathFindings(row.text)];
    const findings = all.filter((f, k) => all.findIndex((o) => o.evidence === f.evidence) === k);
    if (findings.length) suspects.push({ id: row.id, source: row.source, findings });
  }
  return { ticked: rows.length, commands, paths, unchecked, suspects };
}

// ---- git and disk -----------------------------------------------------------------------

function git(root, args) {
  const env = { ...process.env };
  for (const k of ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"]) delete env[k];
  const r = spawnSync("git", args, { cwd: root, encoding: "utf8", shell: false, timeout: GIT_TIMEOUT_MS, env, maxBuffer: 256 * 1024 * 1024 });
  return r.error || r.status !== 0 ? null : r.stdout;
}

// The newest commit of a `--format=%h<TAB>%s` log: { sha, subject }, false when the log is
// empty (no such commit), null when git failed (unknown).
const firstCommit = (out) => {
  if (out === null) return null;
  const line = out.split("\n").find(Boolean);
  if (!line) return false;
  const [sha, ...rest] = line.split("\t");
  return { sha, subject: rest.join("\t").slice(0, 80) };
};

const memo = (fn) => {
  const seen = new Map();
  return (...args) => {
    const key = JSON.stringify(args);
    if (!seen.has(key)) seen.set(key, fn(...args));
    return seen.get(key);
  };
};

// Files that exist now: tracked or untracked-not-ignored, minus tracked ones deleted on disk.
export function buildContext(root = ROOT) {
  const listed = git(root, ["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
  const deleted = git(root, ["ls-files", "-z", "--deleted"]);
  if (listed === null || deleted === null) return null;
  const gone = new Set(deleted.split("\0").filter(Boolean));
  const files = new Set(listed.split("\0").filter((f) => f && !gone.has(f)));
  const packages = new Map();
  for (const r of crateRows(root)) if (r.name) packages.set(r.name, `crates/${r.dir}`);
  const top = join(root, "Cargo.toml");
  if (existsSync(top)) {
    const name = packageName(readFileSync(top, "utf8"));
    if (name) packages.set(name, ".");
  }
  return {
    files,
    deleted: gone,
    packages,
    read: (path) => {
      try {
        return readFileSync(join(root, path), "utf8");
      } catch {
        return null;
      }
    },
    removedBy: memo((path) => firstCommit(git(root, ["log", "-1", "--diff-filter=D", "--format=%h%x09%s", "--", path]))),
    lastTouch: memo((needle, dir) => firstCommit(git(root, ["log", "-1", `-S${needle}`, "--format=%h%x09%s", "--", dir]))),
  };
}

// ---- output -----------------------------------------------------------------------------

export function formatReport(report, { archived = 0 } = {}) {
  const lines = report.suspects.map((s) => {
    const reasons = [...new Set(s.findings.map((f) => f.reason))].join(",");
    return `${s.id.padEnd(8)}  ${reasons}  ${s.findings.map((f) => f.evidence).join(" | ")}`;
  });
  const byReason = new Map();
  for (const s of report.suspects) {
    for (const reason of new Set(s.findings.map((f) => f.reason))) byReason.set(reason, (byReason.get(reason) ?? 0) + 1);
  }
  const split = [...byReason].sort().map(([reason, n]) => `${reason} ${n}`).join(", ");
  lines.push(
    `SUMMARY ${report.suspects.length} suspect row(s) of ${report.ticked} ticked${split ? ` [${split}]` : ""} (${report.commands} cargo test commands with -p, ${report.paths} source paths checked; ${report.unchecked} rows name neither, so nothing to check; ${archived} rows read from docs/archive)`,
  );
  return `${lines.join("\n")}\n`;
}

// Returns the exit code; output goes through stdout.write and the process ends on its own,
// so a piped reader never sees a cut-off report.
function main(argv) {
  let json = false;
  let history = true;
  let root = ROOT;
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--json") json = true;
    else if (a === "--no-history") history = false;
    else if (a === "--root") {
      if (!argv[i + 1]) usageFail(PREFIX, "--root needs a directory", HELP);
      root = resolve(argv[(i += 1)]);
    } else if (a === "-h" || a === "--help") {
      process.stdout.write(HELP);
      return 0;
    } else usageFail(PREFIX, `unknown argument ${a}`, HELP);
  }
  if (!existsSync(join(root, "docs", "SPRINT.md"))) {
    process.stderr.write(`${PREFIX}: error: board missing: ${join(root, "docs", "SPRINT.md")}\n`);
    return 2;
  }
  const ctx = buildContext(root);
  if (!ctx) {
    process.stderr.write(`${PREFIX}: error: git ls-files failed in ${root} (not a git checkout?)\n`);
    return 2;
  }
  const rows = tickedRows(root, { history });
  const report = auditRows(rows, ctx);
  const archived = rows.filter((r) => r.source !== "board").length;
  if (json) process.stdout.write(`${JSON.stringify({ root, archived, ...report }, null, 2)}\n`);
  else process.stdout.write(formatReport(report, { archived }));
  return report.suspects.length ? 1 : 0;
}

if (import.meta.main) {
  process.exitCode = main(cliArgs());
}

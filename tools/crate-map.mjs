// crate-map: docs/CRATE-MAP.md, one row per crates/<dir>, from its Cargo.toml and lib.rs.
//   node tools/crate-map.mjs          write docs/CRATE-MAP.md
//   node tools/crate-map.mjs --check  exit 1 when docs/CRATE-MAP.md is not what the crates say
//   node tools/crate-map.mjs --baseline [--index | --with <path,path>] [--verbose]
//                                     print the size numbers of HEAD, of the staged
//                                     commit (--index) or of HEAD plus a lane's paths
//                                     (--with), so a lane can read the size it lands;
//                                     --verbose adds the per-name lock listing
// Job = the first `//!` line of src/lib.rs. Depends on = engine crates in [dependencies]
// (dev-dependencies do not count). Used by = the reverse. Instability = Ce/(Ca+Ce). Exit 0 PASS/written, 1 STALE, 2 usage.
// Weak features = the flags the feature scan finds thin (no other crate asks for them and
// one place in their own crate reads them).
// --baseline measures the size of the engine: the crates, the physical lines of Rust
// under crates/<dir>/src, the names that carry more than one version in Cargo.lock and
// the dead feature flags. No number is written in this file: every one is measured
// from the tree.
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { classify, featureRows, weakFeatures } from "./feature-scan.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
export const OUT = "docs/CRATE-MAP.md";
export const JOB_MAX = 100;

// TOML tables as { header: [lines] }. Handles [x] and [[x]] headers.
const tables = (text) => {
  const out = {};
  let cur = "";
  for (const line of text.replace(/\r\n/g, "\n").split("\n")) {
    const h = /^\[\[?([^\]]+)\]\]?\s*$/.exec(line.trim());
    if (h) { cur = h[1].trim(); out[cur] ??= []; continue; }
    if (cur) out[cur].push(line);
  }
  return out;
};

export const packageName = (toml) => /^name\s*=\s*"([^"]+)"/m.exec((tables(toml).package ?? []).join("\n"))?.[1] ?? "";

// Keys of [dependencies]; `alias = { package = "engine-x" }` counts as engine-x.
export const dependencyNames = (toml) => (tables(toml).dependencies ?? []).flatMap((line) => {
  const key = /^\s*([\w-]+)(?:\.\w+)?\s*=/.exec(line)?.[1];
  if (!key) return [];
  return [/package\s*=\s*"([^"]+)"/.exec(line)?.[1] ?? key];
});

// Physical lines of the Rust under crates/<dir>/src, blank lines included (the
// count `wc -l` gives). `tests/` inside src counts: it is compiled code. Pure
// helper for tests.
export const sourceLines = (cratesDir, dir) => {
  const src = join(cratesDir, dir, "src");
  if (!existsSync(src)) return { files: 0, lines: 0 };
  let files = 0;
  let lines = 0;
  const walk = (p) => {
    for (const e of readdirSync(p, { withFileTypes: true })) {
      const q = join(p, e.name);
      if (e.isDirectory()) walk(q);
      else if (e.name.endsWith(".rs")) {
        files += 1;
        lines += readFileSync(q, "utf8").split("\n").length - 1;
      }
    }
  };
  walk(src);
  return { files, lines };
};

// Multi-version names in Cargo.lock: what `cargo tree --duplicates` prints,
// without calling cargo. One pass over the lock.
export const lockVersions = (root = ROOT) => {
  const lockPath = join(root, "Cargo.lock");
  if (!existsSync(lockPath)) return { packages: 0, names: 0, copies: 0, byName: [] };
  const versions = new Map();
  let packages = 0;
  for (const block of readFileSync(lockPath, "utf8").split("[[package]]").slice(1)) {
    const name = /name = "([^"]+)"/.exec(block)?.[1];
    const version = /version = "([^"]+)"/.exec(block)?.[1];
    if (!name || !version) continue;
    packages += 1;
    if (!versions.has(name)) versions.set(name, new Set());
    versions.get(name).add(version);
  }
  const byName = [...versions.entries()]
    .filter(([, v]) => v.size > 1)
    .map(([name, v]) => ({ name, copies: v.size, versions: [...v].sort() }))
    .sort((a, b) => b.copies - a.copies || a.name.localeCompare(b.name));
  return { packages, names: byName.length, copies: byName.reduce((a, r) => a + r.copies - 1, 0), byName };
};

// Feature flags that are dead: declared in a manifest, asked for by no other
// crate and read by no `cfg(feature = ...)` in their own crate. The rule and
// its readers belong to tools/feature-scan.mjs, so this is that scan's dead
// bucket rather than a second walk with a second opinion. Its `--commands`
// half fills a field the dead rule ignores, so it stays off here (it walks
// every doc of the repo to fill it).
export const deadFeatures = (root = ROOT) => featureRows(root, { commands: false }).filter((row) => classify(row) === "dead");

// The committed tree: the baseline is a snapshot of a commit, so the counts
// come from HEAD and not from a working tree that other sessions are editing.
// `git archive` and `tar` read the committed bytes into a temp folder (the way
// tools/public-export.mjs reads a tree), and the same helpers measure it, so
// there is one rule and not two. Null when git or tar is not there; the
// caller then measures the working tree and says so.
// `rev` may be any tree-ish, so the same reader measures a tree built from the
// index or from HEAD plus a lane's paths.
export const committedTree = (root = ROOT, rev = "HEAD") => {
  const dir = mkdtempSync(join(tmpdir(), "crate-map-head-"));
  const tar = join(dir, "tree.tar");
  try {
    execFileSync("git", ["-C", root, "-c", "core.autocrlf=false", "-c", "core.eol=lf", "archive", "--format=tar", "-o", tar, rev, "crates", "Cargo.lock"], { stdio: "ignore" });
    // Relative names with cwd: GNU tar reads a drive letter in -f or -C as a
    // remote host:path (C:\... -> host "C") and fails, while Windows bsdtar
    // takes it. The archive is in the temp dir, so neither name needs a colon.
    execFileSync("tar", ["-xf", "tree.tar", "-C", "."], { cwd: dir, stdio: "ignore" });
    rmSync(tar, { force: true });
    return dir;
  } catch {
    rmSync(dir, { recursive: true, force: true });
    return null;
  }
};

// The tree the next commit lands: git runs a hook with GIT_INDEX_FILE on the
// commit's own index, so `write-tree` answers exactly what will be committed.
export const indexTree = (root = ROOT) =>
  execFileSync("git", ["-C", root, "write-tree"], { encoding: "utf8" }).trim();

// HEAD plus the working-tree state of `paths` (tracked, changed or new; a
// deleted path drops out), in a throwaway index. This is what `git commit --
// <paths>` would land, so a lane can read the size its commit will have before
// it commits, which HEAD alone cannot answer.
export const stagedTree = (paths, root = ROOT) => {
  const listed = (Array.isArray(paths) ? paths : String(paths ?? "").split(",")).map((p) => p.trim()).filter(Boolean);
  if (!listed.length) throw new Error("--with needs paths (comma-separated, repo-relative)");
  const dir = mkdtempSync(join(tmpdir(), "crate-map-with-"));
  const env = { ...process.env, GIT_INDEX_FILE: join(dir, "index") };
  const git = (...args) => execFileSync("git", ["-C", root, ...args], { env, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  try {
    git("read-tree", "HEAD");
    git("add", "-A", "-f", "--", ...listed);
    return git("write-tree").trim();
  } finally { rmSync(dir, { recursive: true, force: true }); }
};


const clip = (s, max) => (s.length > max ? `${s.slice(0, max - 3)}...` : s);

// One row per crates/<dir> that has a Cargo.toml, sorted by dir.
export const crateRows = (root = ROOT) => {
  const crates = join(root, "crates");
  const dirs = existsSync(crates) ? readdirSync(crates).filter((d) => existsSync(join(crates, d, "Cargo.toml"))).sort() : [];
  const rows = dirs.map((d) => {
    const toml = readFileSync(join(crates, d, "Cargo.toml"), "utf8");
    const lib = join(crates, d, "src", "lib.rs");
    const doc = existsSync(lib) ? readFileSync(lib, "utf8").split(/\r?\n/).find((l) => /^\/\/!\s*\S/.test(l)) : undefined;
    const job = doc ? doc.replace(/^\/\/!\s*/, "").trim() : "(no lib.rs //! line)";
    // PW51: the measured size of the crate rides the row, so the baseline
    // counts and the map read from one walk.
    const src = sourceLines(crates, d);
    return { dir: d, name: packageName(toml), job: clip(job, JOB_MAX).replace(/\|/g, "/"), deps: dependencyNames(toml), srcFiles: src.files, srcLines: src.lines };
  });
  const names = new Set(rows.map((r) => r.name));
  for (const r of rows) r.deps = [...new Set(r.deps)].filter((n) => names.has(n)).sort();
  for (const r of rows) r.usedBy = rows.filter((o) => o.deps.includes(r.name)).map((o) => o.name).sort();
  return rows;
};

// Coupling alarm (CQ21): instability = Ce / (Ca + Ce), with Ce = fan-out
// (Depends on count) and Ca = fan-in (Used by count). A crate nothing
// touches and that touches nothing scores 0. Pure helper for tests.
export const instability = (row) => {
  const outgoing = row.deps.length;
  const incoming = row.usedBy.length;
  return outgoing + incoming === 0 ? 0 : outgoing / (outgoing + incoming);
};

// Short names keep the table under the 7 KB budget; the Package column keeps the full name.
const list = (names) => (names.length ? names.map((n) => n.replace(/^engine-/, "")).join(", ") : "-");

// One line per weak flag: the flag and the one place that reads it.
const weakLine = (weak) => `- \`${weak.key}\` - \`${weak.where}\`${weak.target ? ` (\`${weak.target}\` needs it)` : ""}`;

export const renderCrateMap = (rows, weak = []) => [
  "# Crate map",
  "",
  "Generated by `node tools/crate-map.mjs` from each `crates/<dir>/Cargo.toml` and its `lib.rs` `//!` line. Do not edit by hand.",
  "`Depends on` = engine crates in `[dependencies]`; `Used by` = the reverse. Both drop the `engine-` prefix.",
  "`Instability` = Ce / (Ca + Ce): fan-out over fan-in plus fan-out. 0 is stable, 1 is unstable.",
  "`Weak features` = the flags `node tools/feature-scan.mjs` finds thin: no other crate asks for them and one place reads them.",
  "",
  "| Package | Dir | Job (lib.rs) | Depends on | Used by | Instability |",
  "|---|---|---|---|---|---|",
  ...rows.map((r) => `| \`${r.name}\` | \`crates/${r.dir}\` | ${r.job} | ${list(r.deps)} | ${list(r.usedBy)} | ${instability(r).toFixed(2)} |`),
  "",
  "## Weak features",
  "",
  "No other crate's manifest asks for these flags and exactly one place in their own crate",
  "reads them. One edit and they are dead, so they need a second reader before they grow.",
  "",
  ...(weak.length ? weak.map(weakLine) : ["None."]),
  "",
].join("\n");

export const generate = (root = ROOT) => renderCrateMap(crateRows(root), weakFeatures(root));

if (import.meta.main) {
  const args = process.argv.slice(2);
  const usage = "usage: node tools/crate-map.mjs [--check | --baseline [--index | --with <path,path>] [--verbose]]";
  if (args[0] === "--baseline") {
    const withAt = args.indexOf("--with");
    const withValue = withAt < 0 ? null : args[withAt + 1];
    const index = args.includes("--index");
    const verbose = args.includes("--verbose");
    const known = new Set(["--baseline", "--index", "--with", "--verbose", withValue]);
    const bad = args.some((a) => !known.has(a))
      || (withAt >= 0 && (!withValue || withValue.startsWith("--")))
      || (index && withAt >= 0);
    if (bad) {
      console.log(usage);
      process.exit(2);
    }
    // The size of the engine, measured. Three readings: the committed tree
    // (HEAD), the index (the commit a hook is about to make) and HEAD plus a
    // lane's paths (what `git commit -- <paths>` will land, known before the
    // commit).
    let tree;
    try {
      if (index) tree = committedTree(ROOT, indexTree());
      else if (withValue) tree = committedTree(ROOT, stagedTree(withValue));
      else tree = committedTree();
    } catch (error) {
      console.log(`could not read the tree: ${String(error?.message ?? error).split("\n")[0]}`);
      process.exit(2);
    }
    const source = index ? "the staged commit" : withValue ? `HEAD plus ${withValue}` : "HEAD";
    const rows = crateRows(tree ?? ROOT);
    const lock = lockVersions(tree ?? ROOT);
    const dead = deadFeatures(tree ?? ROOT);
    console.log(`measured: ${source}${tree ? "" : " (working tree: the tree could not be read)"}`);
    console.log(`crates: ${rows.length}`);
    console.log(`src lines: ${rows.reduce((a, r) => a + r.srcLines, 0)}`);
    console.log(`lock packages: ${lock.packages}`);
    console.log(`multi-version lock names: ${lock.names}`);
    console.log(`extra version copies: ${lock.copies}`);
    console.log(`dead feature flags: ${dead.length}`);
    // The four numbers are the answer; the per-name lock listing is ~90 lines
    // and pushes the numbers out of a caller's context, so it waits for
    // --verbose. The doc names the command for the numbers, not the listing.
    if (verbose) {
      for (const r of lock.byName) console.log(`  ${r.name} (${r.copies}): ${r.versions.join(" ")}`);
      for (const f of dead) console.log(`  dead feature: ${f.key}`);
    }
    if (tree && tree !== ROOT) rmSync(tree, { recursive: true, force: true });
    process.exit(0);
  }
  if (args.length > 1 || (args.length === 1 && args[0] !== "--check")) { console.log(usage); process.exit(2); }
  const rows = crateRows();
  const text = renderCrateMap(rows, weakFeatures());
  const file = join(ROOT, OUT);
  if (args[0] === "--check") {
    const have = existsSync(file) ? readFileSync(file, "utf8").replace(/\r\n/g, "\n") : "";
    console.log(have === text ? `PASS ${OUT}` : `STALE ${OUT}: run node tools/crate-map.mjs`);
    process.exit(have === text ? 0 : 1);
  }
  writeFileSync(file, text);
  console.log(`wrote ${OUT}: ${rows.length} rows, ${text.length} bytes`);
}

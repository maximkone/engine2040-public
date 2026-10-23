// crate-map: rows come from Cargo.toml and lib.rs; the rendered table has one row per crate.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { crateRows, committedTree, deadFeatures, dependencyNames, indexTree, instability, lockVersions, packageName, renderCrateMap, sourceLines, stagedTree } from "./crate-map.mjs";

// alpha uses beta (dotted key), a non-engine crate, and a dev-dependency on gamma.
// gamma uses beta through a renamed key. delta has no lib.rs.
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "crate-map-"));
  const files = {
    "crates/alpha/Cargo.toml": '[package]\nname = "engine-alpha"\n\n[dependencies]\nengine-beta.workspace = true\nserde = "1"\n\n[dev-dependencies]\nengine-gamma = { path = "../gamma" }\n',
    "crates/alpha/src/lib.rs": "// not a doc line\n//! alpha: the first doc line.\n//! second line.\n",
    "crates/beta/Cargo.toml": '[package]\nname = "engine-beta"\n\n[features]\nx = []\n',
    "crates/beta/src/lib.rs": "//! beta: the thing | with a pipe.\n",
    "crates/gamma/Cargo.toml": '[package]\nname = "engine-gamma"\n\n[dependencies]\nbeta-alias = { package = "engine-beta", path = "../beta" }\n',
    "crates/gamma/src/lib.rs": "pub fn f() {}\n",
    "crates/delta/Cargo.toml": '[package]\nname = "engine-delta"\n',
  };
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), body);
  }
  return root;
}

test("names, job lines and engine dependencies come from the manifests", () => {
  assert.equal(packageName('[package]\nname = "engine-x"\n[[bin]]\nname = "tool"\n'), "engine-x");
  assert.deepEqual(dependencyNames('[dependencies]\nengine-a.workspace = true\nb = { package = "engine-c", path = "../c" }\n'), ["engine-a", "engine-c"]);
  const root = fixture();
  try {
    const rows = Object.fromEntries(crateRows(root).map((r) => [r.dir, r]));
    assert.equal(rows.alpha.name, "engine-alpha");
    assert.equal(rows.alpha.job, "alpha: the first doc line.", "first //! line, not the // comment");
    assert.deepEqual(rows.alpha.deps, ["engine-beta"], "dev-dependencies and outside crates do not count");
    assert.deepEqual(rows.gamma.deps, ["engine-beta"], "renamed engine dependency counts");
    assert.deepEqual(rows.beta.usedBy, ["engine-alpha", "engine-gamma"]);
    assert.equal(rows.delta.job, "(no lib.rs //! line)");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("the table has one row per crate and escapes pipes in the job", () => {
  const root = fixture();
  try {
    const text = renderCrateMap(crateRows(root));
    const rows = text.split("\n").filter((l) => l.startsWith("| `"));
    assert.equal(rows.length, 4);
    assert.ok(text.includes("beta: the thing / with a pipe."), "pipe replaced so the table holds");
    assert.match(text, /\| `engine-alpha` \| `crates\/alpha` \| alpha: the first doc line\. \| beta \| - \| 1\.00 \|/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("instability is Ce/(Ca+Ce) and the table prints it per crate", () => {
  const root = fixture();
  try {
    const rows = Object.fromEntries(crateRows(root).map((r) => [r.dir, r]));
    assert.equal(instability(rows.beta), 0, "used by two, depends on none: fully stable");
    assert.equal(instability(rows.alpha), 1, "depends on one, used by none: fully unstable");
    assert.equal(instability(rows.delta), 0, "no coupling either way: stable");
    const text = renderCrateMap(crateRows(root));
    assert.ok(text.includes("Instability"), "the header names the alarm column");
    assert.match(text, /\| `engine-alpha` \| `crates\/alpha` \| alpha: the first doc line\. \| beta \| - \| 1\.00 \|/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

// ---- PW51: the measured side of the compaction baseline ----

test("sourceLines counts every .rs under a crate's src, nested folders included", () => {
  const root = mkdtempSync(join(tmpdir(), "source-lines-"));
  try {
    const files = {
      "crates/a/src/lib.rs": "fn main() {}\n",
      "crates/a/src/inner/mod.rs": "// one\n// two\n",
      "crates/a/src/notes.md": "not rust, not counted\n",
      "crates/b/Cargo.toml": '[package]\nname = "engine-b"\n',
    };
    for (const [path, body] of Object.entries(files)) {
      mkdirSync(join(root, path, ".."), { recursive: true });
      writeFileSync(join(root, path), body);
    }
    assert.deepEqual(sourceLines(join(root, "crates"), "a"), { files: 2, lines: 3 }, "physical lines, blank lines included, nested folders walked");
    assert.deepEqual(sourceLines(join(root, "crates"), "b"), { files: 0, lines: 0 }, "a crate with no src/ counts zero");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("lockVersions counts the packages and the names that carry more than one version", () => {
  const root = mkdtempSync(join(tmpdir(), "lock-versions-"));
  try {
    writeFileSync(join(root, "Cargo.lock"), [
      "[[package]]", 'name = "a"', 'version = "1.0.0"', "",
      "[[package]]", 'name = "a"', 'version = "2.0.0"', "",
      "[[package]]", 'name = "b"', 'version = "3.0.0"', "",
      "[[package]]", 'name = "c"', 'version = "0.1.0"', "",
      "[[package]]", 'name = "c"', 'version = "0.2.0"', "",
      "[[package]]", 'name = "c"', 'version = "0.3.0"', "",
    ].join("\n"));
    const lock = lockVersions(root);
    assert.equal(lock.packages, 6, "six package blocks in the fixture");
    assert.equal(lock.names, 2, "a and c carry more than one version, b does not");
    assert.equal(lock.copies, 3, "one extra copy of a plus two of c");
    assert.deepEqual(lock.byName.map((r) => r.name), ["c", "a"], "most copies first, then the name");
    assert.deepEqual(lock.byName[0].versions, ["0.1.0", "0.2.0", "0.3.0"]);
    const bare = mkdtempSync(join(tmpdir(), "lock-bare-"));
    try { assert.deepEqual(lockVersions(bare), { packages: 0, names: 0, copies: 0, byName: [] }, "no Cargo.lock counts nothing"); }
    finally { rmSync(bare, { recursive: true, force: true }); }
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("deadFeatures finds a declared flag nobody asks for and nobody reads", () => {
  const root = mkdtempSync(join(tmpdir(), "dead-features-"));
  try {
    const files = {
      "crates/a/Cargo.toml": '[package]\nname = "engine-a"\n\n[features]\ndead = []\nread = []\nasked = []\n',
      "crates/a/src/lib.rs": '#[cfg(feature = "read")]\npub fn f() {}\n',
      "crates/b/Cargo.toml": '[package]\nname = "engine-b"\n\n[dependencies]\nengine-a = { path = "../a", features = ["asked"] }\n',
      "crates/b/src/lib.rs": "pub fn g() {}\n",
    };
    for (const [path, body] of Object.entries(files)) {
      mkdirSync(join(root, path, ".."), { recursive: true });
      writeFileSync(join(root, path), body);
    }
    // read = one cfg in its own src (weak, not dead); asked = another crate asks.
    assert.deepEqual(deadFeatures(root).map((f) => f.key), ["engine-a/dead"]);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("every crate row carries its measured src files and src lines", () => {
  const root = fixture();
  try {
    const rows = Object.fromEntries(crateRows(root).map((r) => [r.dir, r]));
    assert.equal(rows.alpha.srcFiles, 1);
    assert.equal(rows.alpha.srcLines, 3, "alpha's lib.rs is three physical lines");
    assert.equal(rows.beta.srcLines, 1, "a single ///! line");
    assert.equal(rows.delta.srcFiles, 0, "no src/: nothing to count");
    const total = crateRows(root).reduce((a, r) => a + r.srcLines, 0);
    assert.equal(total, 5, "the sum the baseline records: 3 + 1 + 1");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("committedTree reads the committed tree and leaves the working tree alone", () => {
  const root = mkdtempSync(join(tmpdir(), "crate-map-head-"));
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });
  let tree = null;
  try {
    mkdirSync(join(root, "crates", "a", "src"), { recursive: true });
    writeFileSync(join(root, "crates", "a", "Cargo.toml"), '[package]\nname = "engine-a"\n');
    writeFileSync(join(root, "crates", "a", "src", "lib.rs"), "fn committed() {}\n");
    writeFileSync(join(root, "Cargo.lock"), '[[package]]\nname = "a"\nversion = "1.0.0"\n');
    git("init", "-q");
    git("-c", "user.email=t@t", "-c", "user.name=t", "add", "-A");
    git("-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "one");
    // A file that is only in the working tree must not be in the snapshot.
    writeFileSync(join(root, "crates", "a", "src", "dirty.rs"), "fn dirty() {}\n");

    tree = committedTree(root);
    assert.ok(tree, "the committed tree is read");
    assert.equal(existsSync(join(tree, "crates", "a", "src", "lib.rs")), true, "the committed file is there");
    assert.equal(existsSync(join(tree, "crates", "a", "src", "dirty.rs")), false, "the uncommitted file is not");
    assert.equal(existsSync(join(tree, "Cargo.lock")), true, "the lock is there");
    assert.equal(crateRows(tree).length, 1, "the snapshot holds the committed crates");
    assert.equal(lockVersions(tree).packages, 1, "the snapshot holds the committed lock");
  } finally {
    if (tree) rmSync(tree, { recursive: true, force: true });
    rmSync(root, { recursive: true, force: true });
  }
});

test("--with reads HEAD plus a lane's working paths, so the numbers are known before the commit", () => {
  const root = mkdtempSync(join(tmpdir(), "crate-map-with-"));
  const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" });
  git("init", "-q");
  try {
    mkdirSync(join(root, "crates", "a", "src"), { recursive: true });
    writeFileSync(join(root, "crates", "a", "Cargo.toml"), '[package]\nname = "engine-a"\n');
    writeFileSync(join(root, "crates", "a", "src", "lib.rs"), "fn one() {}\n");
    writeFileSync(join(root, "Cargo.lock"), '[[package]]\nname = "a"\nversion = "1.0.0"\n');
    git("-c", "user.email=t@t", "-c", "user.name=t", "add", "-A");
    git("-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "one");
    const before = crateRows(committedTree(root)).reduce((n, r) => n + r.srcLines, 0);
    assert.equal(before, 1, "HEAD counts one line");

    // The lane edits lib.rs and adds a new file, and nothing is staged yet.
    writeFileSync(join(root, "crates", "a", "src", "lib.rs"), "fn one() {}\nfn two() {}\n");
    writeFileSync(join(root, "crates", "a", "src", "new.rs"), "fn three() {}\n");
    const withLib = committedTree(root, stagedTree(["crates/a/src/lib.rs"], root));
    assert.equal(crateRows(withLib).reduce((n, r) => n + r.srcLines, 0), 2, "the edit is measured");
    rmSync(withLib, { recursive: true, force: true });
    const withBoth = committedTree(root, stagedTree("crates/a/src/lib.rs,crates/a/src/new.rs", root));
    assert.equal(crateRows(withBoth).reduce((n, r) => n + r.srcLines, 0), 3, "a new file is measured too");
    rmSync(withBoth, { recursive: true, force: true });
    assert.equal(crateRows(committedTree(root)).reduce((n, r) => n + r.srcLines, 0), 1, "HEAD itself is untouched");

    // --index reads exactly what the index stages, and a staged deletion is seen.
    assert.equal(crateRows(committedTree(root, indexTree(root))).reduce((n, r) => n + r.srcLines, 0), 1, "nothing staged yet");
    git("add", "crates/a/src/lib.rs");
    const staged = committedTree(root, indexTree(root));
    assert.equal(crateRows(staged).reduce((n, r) => n + r.srcLines, 0), 2, "the staged edit is measured");
    rmSync(staged, { recursive: true, force: true });
    assert.throws(() => stagedTree([], root), /--with needs paths/, "an empty --with is refused");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

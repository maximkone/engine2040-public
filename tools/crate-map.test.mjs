// crate-map: rows come from Cargo.toml and lib.rs; the rendered table has one row per crate.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { crateRows, dependencyNames, instability, packageName, renderCrateMap } from "./crate-map.mjs";

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

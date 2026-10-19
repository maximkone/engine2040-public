import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { analyzeText, compare, coverageOf, innerDocumented, isDocumented, measure } from "./docs-coverage.mjs";
test("empty doc markers count undocumented, prose counts documented", () => {
  assert.equal(isDocumented(["/// Prose.", "pub fn a() {}"], 1), true);
  assert.equal(isDocumented(["///", "pub fn a() {}"], 1), false);
  assert.equal(isDocumented(["///   ", "pub fn a() {}"], 1), false);
  assert.equal(isDocumented(["/** prose */", "pub fn a() {}"], 1), true);
  assert.equal(isDocumented(["/** */", "pub fn a() {}"], 1), false);
  assert.equal(isDocumented(['#[doc = "prose"]', "pub fn a() {}"], 1), true);
  assert.equal(isDocumented(['#[doc = ""]', "pub fn a() {}"], 1), false);
  assert.equal(isDocumented(["/// Prose.", "", "pub fn a() {}"], 2), false);
  assert.equal(isDocumented(["/// Prose.", "#[must_use]", "pub fn a() {}"], 2), true);
  assert.equal(isDocumented(["/**", " * prose", " */", "pub fn a() {}"], 3), true);
  assert.equal(isDocumented(["/**", " *", " */", "pub fn a() {}"], 3), false);
});
test("a doc comment above a multi-line attribute group documents the item", () => {
  const group = [
    "/// Args for the verbs that take nothing.",
    "#[derive(Debug, Default)]",
    "#[expect(",
    "    clippy::empty_structs_with_brackets,",
    '    reason = "serde reads the wire shape `{}`; a unit struct reads null"',
    ")]",
    "pub struct NoArgs {}",
  ];
  assert.equal(isDocumented(group, group.length - 1), true, "the walk steps over the whole group");
  // A balanced code line after an attribute is not a group continuation: the
  // item below it stays undocumented.
  const code = ["#[derive(Debug)]", "fn helper() { let v = [1, 2]; }", "pub struct Bare;"];
  assert.equal(isDocumented(code, 2), false);
  const a = analyzeText([...group, "", "/// Kept.", "pub struct Kept;"].join("\n"));
  assert.equal(a.total, 2);
  assert.equal(a.undocumented, 0);
});
test("analyzeText counts pub items, skips strings and test regions", () => {
  const text = ["/// Adds one.", "pub fn documented() {}", "pub fn missing() {}", 'let s = "pub fn fake() {}";', "#[cfg(test)]", "mod tests {", "/// Tested.", "pub fn in_test() {}", "}"].join("\n");
  const a = analyzeText(text);
  assert.equal(a.total, 2);
  assert.equal(a.documented, 1);
  assert.equal(a.undocumented, 1);
  assert.match(a.missing[0].text, /pub fn missing/);
});
test("coverageOf handles empty crates", () => {
  assert.equal(coverageOf(0, 0), 100);
  assert.equal(coverageOf(4, 3), 75);
});
test("compare fails on new undocumented items and on removed API", () => {
  const base = { crates: { a: { total: 10, documented: 8, undocumented: 2, coverage: 80 } } };
  assert.deepEqual(compare(base, new Map([["a", { name: "a", total: 10, documented: 8, undocumented: 2, coverage: 80 }]])), { regressions: [], improvements: [] });
  const worse = compare(base, new Map([["a", { name: "a", total: 11, documented: 8, undocumented: 3, coverage: 72.72 }]]));
  assert.equal(worse.regressions.length, 1);
  assert.match(worse.regressions[0], /undocumented public items 2 -> 3/);
  const cut = compare(base, new Map([["a", { name: "a", total: 9, documented: 8, undocumented: 1, coverage: 88.88 }]]));
  assert.equal(cut.regressions.length, 1);
  assert.match(cut.regressions[0], /public items removed 10 -> 9/);
  const fresh = compare({ crates: {} }, new Map([["n", { name: "n", total: 5, documented: 5, undocumented: 0, coverage: 100 }]]));
  assert.match(fresh.regressions[0], /new crate with no recorded baseline/);
});
test("measure runs over a temp fixture crate", () => {
  const root = mkdtempSync(join(tmpdir(), "docs-cov-"));
  try {
    mkdirSync(join(root, "crates", "fake", "src"), { recursive: true });
    writeFileSync(join(root, "crates", "fake", "Cargo.toml"), '[package]\nname = "engine-fake"\n');
    writeFileSync(join(root, "crates", "fake", "src", "lib.rs"), "/// Documented.\npub fn yes() {}\npub fn no() {}\n");
    const m = measure(root);
    assert.equal(m.get("engine-fake").total, 2);
    assert.equal(m.get("engine-fake").documented, 1);
    assert.equal(m.get("engine-fake").coverage, 50);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
test("a pub mod is documented by its own inner doc comment, not by an empty one", () => {
  assert.equal(innerDocumented("//! Prose about the module.\n\nuse x;\n"), true);
  assert.equal(innerDocumented("#![allow(dead_code)]\n\n//! Prose after an inner attribute.\n"), true);
  assert.equal(innerDocumented("/*!\n * Block prose.\n */\n"), true);
  assert.equal(innerDocumented("//!\n//!   \nuse x;\n"), false);
  assert.equal(innerDocumented("use x;\n//! too late\n"), false);
  assert.equal(innerDocumented("// plain comment\npub fn a() {}\n"), false);
  const text = ["pub mod yes;", "pub mod no;", "/// Own prose.", "pub mod own;"].join("\n");
  const a = analyzeText(text, (name) => name === "yes");
  assert.equal(a.total, 3);
  assert.equal(a.documented, 2);
  assert.match(a.missing[0].text, /pub mod no;/);
});
test("measure resolves pub mod docs beside lib.rs, a.rs/b.rs and a/mod.rs", () => {
  const root = mkdtempSync(join(tmpdir(), "docs-cov-"));
  try {
    const src = join(root, "crates", "fake", "src");
    mkdirSync(join(src, "a", "deep"), { recursive: true });
    writeFileSync(join(root, "crates", "fake", "Cargo.toml"), '[package]\nname = "engine-fake"\n');
    writeFileSync(join(src, "lib.rs"), "pub mod a;\npub mod bare;\n");
    writeFileSync(join(src, "a.rs"), "//! Module a.\npub mod deep;\n");
    writeFileSync(join(src, "bare.rs"), "use x;\n");
    writeFileSync(join(src, "a", "deep", "mod.rs"), "//! Deep.\n");
    const c = measure(root).get("engine-fake");
    assert.equal(c.total, 3);
    assert.equal(c.undocumented, 1);
    assert.match(c.missing[0], /lib\.rs:2: pub mod bare;/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

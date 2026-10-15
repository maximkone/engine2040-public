import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { analyzeText, compare, coverageOf, isDocumented, measure } from "./docs-coverage.mjs";
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

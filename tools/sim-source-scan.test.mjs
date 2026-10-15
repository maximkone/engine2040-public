import assert from "node:assert/strict";
import test from "node:test";
import {
  PLATFORM_MATH_METHODS,
  deriveFixedApi,
  genreHits,
  maskRust,
  platformMathHits,
  productionCode,
} from "./sim-source-scan.mjs";

const FIXED_MINI = "use engine_core::fixed::Fixed;\n";
const apiOf = (src) => deriveFixedApi(src);

test("maskRust blanks line comments but keeps length and newlines", () => {
  const src = "let a = 1; // x.sin()\nlet b = 2;\n";
  const masked = maskRust(src);
  assert.equal(masked.length, src.length);
  assert.equal(masked.split("\n").length, 3);
  assert.doesNotMatch(masked, /x\.sin/);
  assert.match(masked, /let a/);
});

test("maskRust blanks block comments and strings at the same offsets", () => {
  const src = '/* a.sin() */ let s = "b.cos()"; let c = \'x\';\n';
  const masked = maskRust(src);
  assert.equal(masked.length, src.length);
  assert.doesNotMatch(masked, /a\.sin|b\.cos/);
  assert.match(masked, /let s/);
});

test("productionCode stops at the first real cfg(test)", () => {
  assert.equal(productionCode("fn a() {}\n#[cfg(test)]\nfn b() {}\n").includes("fn b"), false);
  assert.equal(productionCode("fn a() {}\n// #[cfg(test)]\nfn b() {}\n").includes("fn b"), true);
});

test("genreHits ignores comments and strings", () => {
  const src = "// Fighter\n/* Gladiator */\nfn f() -> &'static str { \"Fighter\" }\n";
  assert.deepEqual(genreHits(src), []);
});

test("genreHits counts code identifiers (failure case)", () => {
  assert.deepEqual(genreHits("use crate::delta::FighterSnap;\n"), ["FighterSnap"]);
  assert.deepEqual(genreHits("struct Gladiator;\n"), ["Gladiator"]);
});

test("deriveFixedApi reads consts, ctors and self methods", () => {
  const api = apiOf("impl Fixed {\n pub const ONE: Self = Self(1);\n pub fn from_int(v: i64) -> Self { Self(v) }\n pub fn sin(self) -> Self { self }\n pub fn raw(self) -> i64 { 0 }\n}\n");
  assert.equal(api.consts.has("ONE"), true);
  assert.equal(api.ctors.has("from_int"), true);
  assert.equal(api.methods.has("sin"), true);
  assert.equal(api.methods.has("raw"), false);
});

test("platformMathHits passes proven Fixed, fails f32 (failure case)", () => {
  const api = apiOf("impl Fixed {\n pub const PI: Self = Self(1);\n pub fn from_int(v: i64) -> Self { Self(v) }\n pub fn sin(self) -> Self { self }\n}\n");
  const ok = `${FIXED_MINI}fn f(t: Fixed) -> Fixed { t.sin() }\n`;
  assert.deepEqual(platformMathHits(ok, api), []);
  const bad = `${FIXED_MINI}fn f(x: f32) -> f32 { x.sin() }\n`;
  const hits = platformMathHits(bad, api);
  assert.equal(hits.length, 1);
  assert.equal(hits[0].method, "sin");
  assert.equal(hits[0].line, 2);
});

test("platformMathHits lists the wide net, not a snapshot", () => {
  for (const name of ["sin", "powf", "log10", "cbrt", "atan2"]) {
    assert.equal(PLATFORM_MATH_METHODS.includes(name), true, name);
  }
  assert.ok(PLATFORM_MATH_METHODS.length > 10);
});

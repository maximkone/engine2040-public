import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deriveFixedApi } from "./sim-source-scan.mjs";
import { HELP, analyzeFile, cfgTestModPaths, cfgTestMods, cloneKey, compare, functions, hitIsOwned, isPassthrough, lex, measure, onlyBase, packages, scopeRatchetToOwned, score, testLines, testOnlyFile, testPath } from "./rust-quality.mjs";

const lib = { crate: "engine-x", lib: true, bin: false, sim: false, hot: false, core: false, testFile: false };
const sim = { ...lib, crate: "engine-physics", sim: true, hot: true };

test("lexer blanks strings, chars and comments but keeps lifetimes and layout", () => {
  const src = 'fn f<\'a>(s: &\'a str) -> char { let c = \'}\'; let t = "{ .unwrap() }"; // x.unwrap()\n c }';
  const { code, comments } = lex(src);
  assert.equal(code.length, src.length);
  assert.equal(code.includes("unwrap"), false);
  assert.match(code, /&'a str/);
  assert.match(comments[0], /x\.unwrap/);
  assert.equal(functions(code).length, 1);
  const raw = lex('let s = r#"a "quoted" { brace"#; let b = b\'{\';');
  assert.equal(raw.code.includes("{"), false);
});

test("functions see bodies whose signature holds a bracketed `;` and skip bodiless declarations", () => {
  const { code } = lex(
    [
      "fn quat(a: [f32; 4]) -> [f32; 4] {",
      "    [a[3], a[0], a[1], a[2]]",
      "}",
      "trait T {",
      "    fn decl(&self) -> [u8; 4];",
      "    fn bounded<V: Into<[u8; 2]>>(&self, v: V) -> u8 { 0 }",
      "}",
    ].join("\n"),
  );
  assert.deepEqual(functions(code).map((f) => f.name), ["quat", "bounded"]);
});

test("a file opening with #![cfg(test)] after its docs is test code; a later one is not", () => {
  assert.equal(testOnlyFile("//! helper\n//! more\n#![cfg(test)]\n\nfn f() {}\n"), true);
  assert.equal(testOnlyFile("// note\n\n#![cfg(test)]\n"), true);
  assert.equal(testOnlyFile("fn f() {}\n#![cfg(test)]\n"), false);
  assert.equal(testOnlyFile("#[cfg(test)]\nmod tests {}\n"), false);
});

test("test paths and #[cfg(test)] file modules are test code", () => {
  for (const p of ["src/tests/blast.rs", "src/a/tests/x.rs", "src/lod_test.rs", "src/wiring_tests.rs", "src/tests.rs"]) {
    assert.equal(testPath(p), true, p);
  }
  for (const p of ["src/latest.rs", "src/contests/x.rs", "src/attest.rs", "src/lib.rs"]) {
    assert.equal(testPath(p), false, p);
  }
  const lib_rs = "pub mod a;\n#[cfg(test)]\nmod stub;\n#[cfg(test)]\npub(crate) mod suite;\n#[cfg(test)]\nmod inline { }\nmod live;\n";
  assert.deepEqual(cfgTestMods(lib_rs), ["stub", "suite"]);
  assert.deepEqual(cfgTestModPaths("src/lib.rs", lib_rs), ["src/stub", "src/suite"]);
  assert.deepEqual(cfgTestModPaths("src/mind.rs", "#[cfg(test)]\nmod conformance;\n"), ["src/mind/conformance"]);
  assert.deepEqual(cfgTestModPaths("src/a/mod.rs", "#[cfg(test)]\nmod t;\n"), ["src/a/t"]);
});

test("test modules, #[test] fns and tests.rs files are not production code", () => {
  const src = [
    "pub fn a() -> u8 { 1 }",
    "#[cfg(test)]",
    "mod tests {",
    "    #[test]",
    "    fn t() { Some(1).unwrap(); }",
    "}",
    "pub fn b() -> u32 { x.unwrap() }",
  ].join("\n");
  const mask = testLines(lex(src).code);
  assert.deepEqual([...mask], [0, 1, 1, 1, 1, 1, 0]);
  const r = analyzeFile(src, lib);
  assert.equal(r.metrics.unwrap, 1, "only the production unwrap counts");
  assert.equal(r.metrics.tests, 1);
  assert.equal(analyzeFile(src, { ...lib, testFile: true }).metrics.unwrap, 0);
});

test("undoc_pub sees a doc comment above a multi-line attribute", () => {
  const wrapped = [
    "/// Documented.",
    "#[derive(Debug)]",
    "#[expect(",
    "    clippy::empty_structs_with_brackets,",
    '    reason = "wire shape"',
    ")]",
    "pub struct A {}",
  ].join("\n");
  assert.equal(analyzeFile(wrapped, lib).metrics.undoc_pub, 0);
  const bare = wrapped.replace("/// Documented.\n", "");
  assert.equal(analyzeFile(bare, lib).metrics.undoc_pub, 1);
});

test("production debt metrics each catch their defect and pass the clean form", () => {
  const ids = new Set(["GP19"]);
  const bad = [
    "#[allow(dead_code)]",
    "pub fn f(x: u64) -> Result<u32, String> { // GP19 staged rig",
    "    println!(\"hi\");",
    "    let h = 0xcbf2_9ce4_8422_2325u64;",
    "    Ok(x as u32)",
    "}",
  ].join("\n");
  const m = analyzeFile(bad, lib, ids).metrics;
  assert.equal(m.allow_no_reason, 1);
  assert.equal(m.dead_code_allows, 1);
  assert.equal(m.string_errors, 1);
  assert.equal(m.board_ids, 1);
  assert.equal(m.print_in_lib, 1);
  assert.equal(m.hash_reimpl, 1);
  assert.equal(m.casts, 1);
  assert.equal(m.undoc_pub, 1);
  const good = [
    "#[expect(clippy::cast_possible_truncation, reason = \"ids fit in u32 by construction\")]",
    "/// Converts a staged id.",
    "pub fn f(x: u64) -> Result<u32, IdError> { u32::try_from(x).map_err(|_| IdError) }",
  ].join("\n");
  const g = analyzeFile(good, lib, ids).metrics;
  for (const k of ["allow_no_reason", "dead_code_allows", "string_errors", "board_ids", "print_in_lib", "hash_reimpl", "casts", "undoc_pub"]) assert.equal(g[k], 0, k);
  assert.equal(analyzeFile(bad, { ...lib, core: true }, ids).metrics.hash_reimpl, 0, "engine-core owns the hash");
});

test("simulation crates flag nondeterminism and hot-path allocation unless waived", () => {
  const src = [
    "use std::collections::HashMap;",
    "use std::time::Instant; // det-ok: profiling only, never hashed",
    "pub fn step(world: &mut World) {",
    "    let scratch: Vec<u32> = Vec::new();",
    "    let names = format!(\"{}\", 1); // alloc-ok: cold error path",
    "}",
    "pub fn build(world: &World) -> Vec<u32> { Vec::new() }",
  ].join("\n");
  const m = analyzeFile(src, sim).metrics;
  assert.equal(m.nondet, 1);
  assert.equal(m.hot_alloc, 1, "only the unwaived allocation inside step counts");
  assert.equal(m.waivers, 2);
  assert.equal(analyzeFile(src, lib).metrics.nondet, 0, "presentation crates may use HashMap");
});

test("simulation crates count platform float math, but not Fixed sin/cos, tests or presentation crates", () => {
  const planted = "pub fn step(x: f32) -> f32 {\n    x.sin() + x.powf(2.0)\n}\n";
  assert.equal(analyzeFile(planted, sim).metrics.float_math, 2);
  assert.deepEqual(analyzeFile(planted, sim).hits.float_math, [1, 1]);
  assert.equal(analyzeFile(planted, lib).metrics.float_math, 0, "presentation crates may use libm");
  assert.equal(analyzeFile(planted, { ...sim, ownMath: true }).metrics.float_math, 0, "Fixed's own file is exempt");
  assert.equal(analyzeFile(planted, { ...sim, testFile: true }).metrics.float_math, 0);
  assert.equal(analyzeFile(`fn a() {}\n#[cfg(test)]\nmod t {\n    fn b(x: f32) -> f32 { x.sin() }\n}\n`, sim).metrics.float_math, 0);
  const fixedApi = deriveFixedApi("impl Fixed { pub fn sin(self) -> Fixed { self } }\n");
  const proven = "use engine_core::fixed::Fixed;\npub fn turn(a: Fixed) -> Fixed { a.sin() }\n";
  assert.equal(analyzeFile(proven, { ...sim, fixedApi }).metrics.float_math, 0, "a receiver proven Fixed passes");
  assert.equal(analyzeFile(proven, sim).metrics.float_math, 1, "without the Fixed source nothing is proven");
});

test("a planted f32 sin() in the arena step fails the ratchet and a clean arena passes", () => {
  const root = mkdtempSync(join(tmpdir(), "rq-arena-"));
  try {
    mkdirSync(join(root, "crates", "arena", "src"), { recursive: true });
    writeFileSync(join(root, "crates", "arena", "Cargo.toml"), '[package]\nname = "engine-arena"\n');
    const lib_rs = join(root, "crates", "arena", "src", "lib.rs");
    const baseline = { crates: { "engine-arena": { float_math: 0 } } };
    writeFileSync(lib_rs, "pub fn step(x: i32) -> i32 { x + 1 }\n");
    const clean = measure(root, null, false).crates;
    assert.equal(clean.get("engine-arena").metrics.float_math, 0);
    assert.deepEqual(compare(baseline, clean).regressions, []);
    writeFileSync(lib_rs, "pub fn step(x: f32) -> f32 { x.sin() }\n");
    const dirty = measure(root, null, false).crates;
    assert.equal(dirty.get("engine-arena").metrics.float_math, 1);
    assert.match(dirty.get("engine-arena").hits.float_math[0], /lib\.rs:1:/);
    assert.deepEqual(compare(baseline, dirty).regressions.map((r) => r.replace(/ \(.*$/, "")), ["engine-arena: float_math 0 -> 1"]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("pass-through relays and long functions are measured, delegation and idiom are not", () => {
  const code = lex([
    "pub fn follow_stage(a: Vec3, b: &Params) -> Pose { frame_pair(a, b) }",
    "pub fn builder() -> AppBuilder { AppBuilder::default() }",
    "impl Rig { pub fn lock(&self, t: u8) -> u8 { self.d.lock(t) } }",
    "impl From<u8> for Id { fn from(v: u8) -> Self { make(v) } }",
  ].join("\n")).code;
  const relays = functions(code).filter(isPassthrough).map((f) => f.name);
  assert.deepEqual(relays, ["follow_stage"]);
  const long = `pub fn big() {\n${"    let x = 1;\n".repeat(120)}}`;
  assert.equal(analyzeFile(long, lib).metrics.long_fns, 1);
});

test("clone keys survive renamed locals but not different calls or constants", () => {
  const a = cloneKey("let mut h = 0xcbf29ce484222325u64; for b in bytes { h ^= u64::from(*b); h = h.wrapping_mul(0x100000001b3); } h");
  const b = cloneKey("let mut acc = 0xcbf2_9ce4_8422_2325u64; for byte in bytes { acc ^= u64::from(*byte); acc = acc.wrapping_mul(0x100000001b3); } acc");
  const c = cloneKey("let mut h = 0xcbf29ce484222325u64; for b in bytes { h ^= u64::from(*b); h = h.wrapping_add(0x100000001b3); } h");
  assert.equal(a.key, b.key);
  assert.notEqual(a.key, c.key);
});

test("ratchet fails on any rise, reports drops, and admits only grade-A new crates", () => {
  const base = { crates: { "engine-x": { unwrap: 2, casts: 10, tests: 5 } } };
  const metrics = (o) => ({ ...Object.fromEntries(["unwrap", "panic_sites", "casts", "allow_no_reason", "dead_code_allows", "print_in_lib", "string_errors", "board_ids", "hash_reimpl", "dup_fns", "passthrough", "long_fns", "big_files", "hot_alloc", "nondet", "waivers", "undoc_pub", "unsafe", "tests"].map((k) => [k, 0])), loc: 1000, lines: 1000, pub_items: 10, ...o });
  const worse = compare(base, new Map([["engine-x", { metrics: metrics({ unwrap: 3, casts: 8, tests: 4 }) }]]));
  assert.equal(worse.regressions.length, 2, "unwrap rose and tests fell");
  assert.equal(worse.improvements.length, 1);
  const fresh = compare(base, new Map([["engine-new", { metrics: metrics({ tests: 12 }) }]]));
  assert.equal(fresh.regressions.length, 0);
  const dirty = compare(base, new Map([["engine-new", { metrics: metrics({ tests: 12, board_ids: 1 }) }]]));
  assert.match(dirty.regressions[0], /new crate/);
  assert.equal(score(metrics({ tests: 12 })).score, 100);
});

test("--help prints usage and exits 0 without the workspace scan", () => {
  assert.match(HELP, /Usage:/, "help text exists for --help");
  assert.match(HELP, /--crate/, "help documents the fast --crate path");
  const tool = join(dirname(fileURLToPath(import.meta.url)), "rust-quality.mjs");
  const r = spawnSync(process.execPath, [tool, "--help"], { encoding: "utf8" });
  assert.equal(r.status, 0, `expected exit 0, got ${r.status}: ${r.stderr}`);
  assert.match(r.stdout, /Usage:/);
  assert.match(r.stdout, /--crate/);
  assert.doesNotMatch(r.stdout, /production code per crate/, "help must not run the workspace scan");
  assert.doesNotMatch(r.stdout, /RESULT (PASS|FAIL)/, "help must exit before the ratchet");
});

test("land scope: owned-clean plus foreign-staged-debt passes", () => {
  const base = { crates: { "engine-render": { passthrough: 1 }, "engine-physics": { unwrap: 0 } } };
  const metrics = (o) => ({ ...Object.fromEntries(["unwrap", "panic_sites", "casts", "allow_no_reason", "dead_code_allows", "print_in_lib", "string_errors", "board_ids", "hash_reimpl", "dup_fns", "passthrough", "long_fns", "big_files", "hot_alloc", "nondet", "waivers", "undoc_pub", "unsafe", "tests"].map((k) => [k, 0])), loc: 1000, lines: 1000, pub_items: 10, ...o });
  const crates = new Map([
    ["engine-render", { metrics: metrics({ passthrough: 2 }), hits: { passthrough: ["crates/render/src/twin_shadow.rs:274: pub fn relay(a: Vec3) -> Pose { frame_pair(a) }"] } }],
    ["engine-physics", { metrics: metrics({ unwrap: 0 }), hits: {} }],
  ]);
  const { regressions } = compare(base, crates);
  assert.equal(regressions.length, 1, "foreign render debt regresses the global ratchet");
  const owned = ["crates/physics/src/lib.rs"];
  const scoped = scopeRatchetToOwned(regressions, crates, owned);
  assert.equal(scoped.blockers.length, 0, "owned-clean packet keeps no blocker");
  assert.equal(scoped.foreign.length, 1, "foreign debt is kept as a warning, never a refusal");
  assert.equal(hitIsOwned("crates/render/src/twin_shadow.rs:274: relay", owned), false);
  assert.equal(hitIsOwned("crates/physics/src/lib.rs:10: clean", owned), true);
});

test("land scope: owned regression fails", () => {
  const base = { crates: { "engine-physics": { unwrap: 0 } } };
  const metrics = (o) => ({ ...Object.fromEntries(["unwrap", "panic_sites", "casts", "allow_no_reason", "dead_code_allows", "print_in_lib", "string_errors", "board_ids", "hash_reimpl", "dup_fns", "passthrough", "long_fns", "big_files", "hot_alloc", "nondet", "waivers", "undoc_pub", "unsafe", "tests"].map((k) => [k, 0])), loc: 1000, lines: 1000, pub_items: 10, ...o });
  const crates = new Map([
    ["engine-physics", { metrics: metrics({ unwrap: 1 }), hits: { unwrap: ["crates/physics/src/lib.rs:10: x.unwrap()"] } }],
  ]);
  const { regressions } = compare(base, crates);
  assert.equal(regressions.length, 1);
  const scoped = scopeRatchetToOwned(regressions, crates, ["crates/physics/src/lib.rs"]);
  assert.equal(scoped.blockers.length, 1, "owned debt still refuses, never a warning");
  assert.equal(scoped.foreign.length, 0);
});

test("land scope: clean tree passes", () => {
  const base = { crates: { "engine-physics": { unwrap: 0 } } };
  const metrics = (o) => ({ ...Object.fromEntries(["unwrap", "panic_sites", "casts", "allow_no_reason", "dead_code_allows", "print_in_lib", "string_errors", "board_ids", "hash_reimpl", "dup_fns", "passthrough", "long_fns", "big_files", "hot_alloc", "nondet", "waivers", "undoc_pub", "unsafe", "tests"].map((k) => [k, 0])), loc: 1000, lines: 1000, pub_items: 10, ...o });
  const crates = new Map([
    ["engine-physics", { metrics: metrics({ unwrap: 0 }), hits: {} }],
  ]);
  const { regressions } = compare(base, crates);
  assert.equal(regressions.length, 0);
  const scoped = scopeRatchetToOwned(regressions, crates, ["crates/physics/src/lib.rs"]);
  assert.equal(scoped.blockers.length, 0);
  assert.equal(scoped.foreign.length, 0);
});

test("onlyBase names baseline rows with no crate, and the real baseline has none", () => {
  const base = { crates: { "engine-core": {}, "engine-viewer": {} } };
  assert.deepEqual(onlyBase(base, new Set(["engine-core"])), ["engine-viewer"]);
  assert.deepEqual(onlyBase({}, new Set()), []);
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  const real = JSON.parse(readFileSync(join(root, "tools", "rust-quality.baseline.json"), "utf8"));
  assert.deepEqual(onlyBase(real, new Set(packages(root).map((p) => p.name))), [], "a baseline row names a crate that is gone");
});

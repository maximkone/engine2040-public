import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deriveFixedApi } from "./sim-source-scan.mjs";
import * as rq from "./rust-quality.mjs";
import { HELP, analyzeFile, cfgTestModPaths, cfgTestMods, cloneKey, compare, compareCloneMembers, compareFiles, fileLineCount, filePins, functions, hitIsOwned, isPassthrough, lex, measure, onlyBase, packages, scopeRatchetToOwned, score, testLines, testOnlyFile, testPath } from "./rust-quality.mjs";

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

test("the board-id prefilter keeps every real row id (E51)", () => {
  const ids = new Set(["OPT9", "E51", "GA7"]);
  const src = [
    "// OPT9 ops: compile cache spike",
    "pub fn a() {} // E51 speed: measured hotspot bundle",
    "// a plain comment with no row id",
    "// ga7 lower case is not a row id",
    "pub fn b() {} // GA7 is a row id",
  ].join("\n");
  const hits = analyzeFile(src, lib, ids).hits.board_ids;
  assert.equal(hits.length, 3, "OPT9, E51 and GA7 are named");
});

test("the clone group owner order does not depend on the machine locale (E51)", () => {
  const paths = [
    "crates/core/src/a.rs:1",
    "crates/ai/src/b.rs:2",
    "crates/ai/src/a.rs:3",
    "crates/zz/src/c.rs:4",
    "crates/core/src/z.rs:5",
  ];
  for (const a of paths) {
    for (const b of paths) {
      const mine = compareCloneMembers({ core: 0, at: a }, { core: 0, at: b });
      const locale = a.localeCompare(b);
      assert.equal(Math.sign(mine), Math.sign(locale), `${a} vs ${b}`);
    }
  }
  assert.ok(compareCloneMembers({ core: 1, at: "z" }, { core: 0, at: "a" }) < 0, "engine-core owns the group first");
});

test("the package list is read once per root (E51)", () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..");
  assert.equal(packages(root), packages(root), "the second call reuses the list");
  assert.ok(packages(root).some((p) => p.name === "engine-core"));
});

// AF2: every rule id the tool reports names the exact edit that clears it, so
// an agent fixes a red gate without reading the rule source. The fixture below
// fires every rule id of the metric table, one file per group of defects.
test("every rule id names its fix on the seeded fixture", () => {
  assert.ok(rq.METRICS, "the tool must list its rule ids");
  assert.ok(rq.RULE_FIXES, "the tool must list a fix per rule id");
  const root = mkdtempSync(join(tmpdir(), "rq-fix-"));
  try {
    mkdirSync(join(root, "crates", "physics", "src"), { recursive: true });
    mkdirSync(join(root, "docs"), { recursive: true });
    mkdirSync(join(root, "tools"), { recursive: true });
    writeFileSync(join(root, "crates", "physics", "Cargo.toml"), '[package]\nname = "engine-physics"\n');
    writeFileSync(join(root, "docs", "SPRINT.md"), "- [ ] SEED1 the seeded fixture\n");
    writeFileSync(join(root, "crates", "physics", "src", "lib.rs"), [
      "use std::collections::HashMap;",
      "",
      "#[allow(dead_code)]",
      "#[allow(clippy::too_many_arguments)]",
      "pub fn staged(id: u64) -> Result<u32, String> {",
      "    // SEED1 staged rig",
      "    let map: HashMap<u32, u32> = HashMap::new();",
      "    let hash = 0xcbf2_9ce4_8422_2325u64;",
      "    let raw = id as u32;",
      "    let value = map.get(&raw).unwrap();",
      "    let other = map.get(&raw).expect(\"seeded\");",
      "    println!(\"{value} {other}\");",
      "    let angle = value as f32;",
      "    let sine = angle.sin();",
      "    let _ = hash;",
      "    Ok(sine as u32)",
      "}",
      "",
      "// det-ok: seeded fixture only, never hashed",
      "",
      "pub unsafe fn raw_read(ptr: *const u8) -> u8 {",
      "    *ptr",
      "}",
      "",
      "pub fn follow_stage(a: Vec3, b: &Params) -> Pose { frame_pair(a, b) }",
      "",
      "pub fn step(world: &mut World) {",
      "    let scratch: Vec<u32> = Vec::new();",
      "    let _ = scratch;",
      "}",
      "",
    ].join("\n"));
    // The same body in two files: one owner, one duplicate.
    const mix = (name, local) => [
      `pub fn ${name}(seed: u32) -> u32 {`,
      `    let mut ${local} = 0xcbf2_9ce4_8422_2325u64;`,
      "    for byte in [seed, seed] {",
      `        ${local} ^= u64::from(byte);`,
      `        ${local} = ${local}.wrapping_mul(0x100000001b3);`,
      "    }",
      `    ${local} as u32`,
      "}",
      "",
    ].join("\n");
    writeFileSync(join(root, "crates", "physics", "src", "dup_a.rs"), mix("mix_a", "h"));
    writeFileSync(join(root, "crates", "physics", "src", "dup_b.rs"), mix("mix_b", "acc"));
    // A file over 1500 production lines.
    const long = Array.from({ length: 1600 }, (_, i) => `    let pad${i} = ${i};`).join("\n");
    writeFileSync(join(root, "crates", "physics", "src", "big.rs"), `pub fn big_body() {\n${long}\n}\n`);
    const zero = Object.fromEntries(Object.keys(rq.METRICS).map((k) => [k, 0]));
    const baseline = { crates: { "engine-physics": { ...zero, tests: 5 }, engine2040: zero } };
    writeFileSync(join(root, "tools", "rust-quality.baseline.json"), `${JSON.stringify(baseline, null, 1)}\n`);

    // Every rule id has its own fix text.
    for (const rule of Object.keys(rq.METRICS)) {
      assert.equal(typeof rq.RULE_FIXES[rule], "string", `${rule} needs a fix`);
      assert.ok(rq.RULE_FIXES[rule].trim().length > 0, `${rule} fix text must not be empty`);
    }
    assert.equal(new Set(Object.values(rq.RULE_FIXES)).size, Object.keys(rq.METRICS).length, "every rule names its own edit");

    const { crates } = measure(root, null, false);
    const seeded = crates.get("engine-physics");
    const { regressions } = compare(baseline, crates);

    // Every rule id fails on the fixture and its line ends with the fix.
    for (const rule of Object.keys(rq.METRICS)) {
      const line = regressions.find((r) => r.startsWith(`engine-physics: ${rule} `));
      assert.ok(line, `the seeded fixture must fail ${rule}`);
      const tail = line.slice(line.lastIndexOf(" fix: ") + 6);
      assert.ok(tail.trim().length > 0, `${rule} must end with a fix tail: ${line}`);
      assert.equal(tail, rq.RULE_FIXES[rule], `${rule} must carry its own fix`);
    }
    // Every hit of every crate names its fix too.
    for (const [name, c] of crates) {
      for (const [rule, hits] of Object.entries(c.hits ?? {})) {
        for (const h of hits) {
          const tail = h.slice(h.lastIndexOf(" fix: ") + 6);
          assert.ok(tail.trim().length > 0, `${name} ${rule} hit must end with a fix tail: ${h}`);
          assert.equal(tail, rq.RULE_FIXES[rule], `${name} ${rule} hit must carry its own fix`);
        }
      }
    }
    // The tool prints the same fix on every FAIL line.
    const tool = join(dirname(fileURLToPath(import.meta.url)), "rust-quality.mjs");
    const run = spawnSync(process.execPath, [tool, "--no-cache"], { cwd: root, encoding: "utf8" });
    const printed = `${run.stdout ?? ""}${run.stderr ?? ""}`.split("\n");
    const failLines = printed.filter((l) => l.startsWith("[FAIL]") || l.startsWith("RESULT FAIL"));
    assert.ok(failLines.length >= Object.keys(rq.METRICS).length, `the fixture must fail every rule, got ${failLines.length} FAIL lines`);
    for (const l of failLines) {
      const tail = l.slice(l.lastIndexOf(" fix: ") + 6);
      assert.ok(tail.trim().length > 0, `printed FAIL line must end with a fix tail: ${l}`);
    }
    assert.ok(seeded.metrics.hot_alloc > 0, "the fixture allocates in a hot function");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// CX2: a file over 3000 lines cannot grow. The seeded fixture writes a
// 3,001-line file, pins it in the baseline, then grows it by one line: the
// tool must FAIL and name the path and both counts.
test("a file over the 3000-line cap cannot grow, and a file under it that crosses fails too", () => {
  const root = mkdtempSync(join(tmpdir(), "rq-cap-"));
  try {
    mkdirSync(join(root, "crates", "physics", "src"), { recursive: true });
    mkdirSync(join(root, "tools"), { recursive: true });
    writeFileSync(join(root, "crates", "physics", "Cargo.toml"), '[package]\nname = "engine-physics"\n');
    // 3,001 lines: one over the cap.
    const body = Array.from({ length: 3001 }, (_, i) => `pub fn f${i}() -> u32 { ${i} }`).join("\n");
    const src = join(root, "crates", "physics", "src", "big.rs");
    writeFileSync(src, `${body}\n`);
    const zero = Object.fromEntries(Object.keys(rq.METRICS).map((k) => [k, 0]));
    const baselinePath = join(root, "tools", "rust-quality.baseline.json");
    const writeBaseline = (files) => {
      const baseline = { version: 1, crates: { "engine-physics": { ...zero, tests: 0 } }, files, accepted: [] };
      writeFileSync(baselinePath, `${JSON.stringify(baseline, null, 1)}\n`);
    };
    writeBaseline({ "crates/physics/src/big.rs": 3001 });

    // The measured file is 3,001 lines: it sits on its pin, so it passes.
    const { fileLines } = measure(root, null, false);
    assert.equal(fileLines.get("crates/physics/src/big.rs"), 3001);
    assert.equal(fileLineCount(`${body}\n`), 3001, "the line count is the physical line count");
    assert.deepEqual(Object.keys(filePins(fileLines)), ["crates/physics/src/big.rs"], "only the over-cap file holds a pin");
    assert.deepEqual(compareFiles({ "crates/physics/src/big.rs": 3001 }, fileLines).regressions, []);

    // Grow it by one line: the tool FAILs and names the path and both counts.
    writeFileSync(src, `${body}\npub fn grown() -> u32 { 7 }\n`);
    const grown = measure(root, null, false).fileLines;
    assert.equal(grown.get("crates/physics/src/big.rs"), 3002);
    const { regressions } = compareFiles({ "crates/physics/src/big.rs": 3001 }, grown);
    assert.equal(regressions.length, 1, "the grown file is the one regression");
    assert.match(regressions[0], /crates\/physics\/src\/big\.rs/, "the line names the path");
    assert.match(regressions[0], /3001/, "the line names the count before");
    assert.match(regressions[0], /3002/, "the line names the count after");
    assert.ok(regressions[0].endsWith(` fix: ${rq.FILE_LINE_FIX}`), "the line names the edit that clears it");

    // A file that was under the cap and crosses it fails too.
    const under = compareFiles({}, new Map([["crates/physics/src/new.rs", 3001]]));
    assert.equal(under.regressions.length, 1, "crossing the cap fails");
    assert.match(under.regressions[0], /crossed the 3000-line cap/);
    assert.match(under.regressions[0], /crates\/physics\/src\/new\.rs/);

    // A file under the cap may grow freely.
    const under2 = compareFiles({}, new Map([["crates/physics/src/small.rs", 2999]]));
    assert.deepEqual(under2.regressions, [], "a file under the cap is not pinned");

    // The tool itself reports the regression and exits 1.
    const tool = join(dirname(fileURLToPath(import.meta.url)), "rust-quality.mjs");
    const run = spawnSync(process.execPath, [tool, "--no-cache"], { cwd: root, encoding: "utf8" });
    const printed = `${run.stdout ?? ""}${run.stderr ?? ""}`;
    assert.equal(run.status, 1, `the grown file must fail the gate: ${printed}`);
    assert.match(printed, /\[FAIL\] ratchet crates\/physics\/src\/big\.rs: file lines 3001 -> 3002/);
    assert.match(printed, /RESULT FAIL: /);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

// CX2 escape hatch: a file pin can be raised with a reason, the way a crate
// baseline can. The refusal names the escape hatch, and the bump is logged in
// the baseline's "accepted" list.
test("a file pin can be raised with a reason, and the refusal names the escape hatch", () => {
  const root = mkdtempSync(join(tmpdir(), "rq-accept-file-"));
  try {
    mkdirSync(join(root, "crates", "geo", "src"), { recursive: true });
    mkdirSync(join(root, "tools"), { recursive: true });
    writeFileSync(join(root, "crates", "geo", "Cargo.toml"), '[package]\nname = "engine-geo"\n');
    // The sim-config gate reads the canonical copy: the fixture carries one.
    writeFileSync(join(root, "tools", "sim-clippy.toml"), "disallowed-types = []\n");
    // 3,001 lines of private helpers: a file over the cap that trips no rule.
    const body = Array.from({ length: 3001 }, (_, i) => `fn helper_${i}() -> u32 { ${i} }`).join("\n");
    const src = join(root, "crates", "geo", "src", "big.rs");
    writeFileSync(src, `${body}\n`);
    const zero = Object.fromEntries(Object.keys(rq.METRICS).map((k) => [k, 0]));
    const baselinePath = join(root, "tools", "rust-quality.baseline.json");
    const writeBaseline = (files) => {
      // The fixture file is over the 1500-line production bar too, so the
      // baseline already holds that count: only the file pin is the subject.
      const baseline = { version: 1, crates: { "engine-geo": { ...zero, big_files: 1, tests: 0 } }, files, accepted: [] };
      writeFileSync(baselinePath, `${JSON.stringify(baseline, null, 1)}\n`);
    };
    writeBaseline({ "crates/geo/src/big.rs": 3001 });

    const tool = join(dirname(fileURLToPath(import.meta.url)), "rust-quality.mjs");

    // Grow the file by one line: the gate refuses and names the escape hatch.
    const grown = `${body}\nfn helper_grown() -> u32 { 7 }\n`;
    writeFileSync(src, grown);
    const failed = spawnSync(process.execPath, [tool, "--no-cache"], { cwd: root, encoding: "utf8" });
    const failedOut = `${failed.stdout ?? ""}${failed.stderr ?? ""}`;
    assert.equal(failed.status, 1, `the grown file must fail the gate: ${failedOut}`);
    assert.match(failedOut, /\[FAIL\] ratchet crates\/geo\/src\/big\.rs: file lines 3001 -> 3002/, "the refusal names the path and both counts");
    assert.match(failedOut, /--accept-file crates\/geo\/src\/big\.rs/, "the refusal names the escape hatch command");
    assert.match(failedOut, /--reason/, "the refusal names the reason flag");

    // A bump without a reason is refused: the reason is what makes it honest.
    const noReason = spawnSync(process.execPath, [tool, "--accept-file", "crates/geo/src/big.rs"], { cwd: root, encoding: "utf8" });
    assert.equal(noReason.status, 2, "a bump without a reason is a usage error");
    assert.match(`${noReason.stdout ?? ""}${noReason.stderr ?? ""}`, /--reason/);

    // The bump raises the pin to the file's real line count and logs the reason.
    const accepted = spawnSync(process.execPath, [tool, "--no-cache", "--accept-file", "crates/geo/src/big.rs", "--reason", "the generated table must hold its rows until the split lands"], { cwd: root, encoding: "utf8" });
    const acceptedOut = `${accepted.stdout ?? ""}${accepted.stderr ?? ""}`;
    assert.equal(accepted.status, 0, `the bump must land: ${acceptedOut}`);
    assert.match(acceptedOut, /ACCEPTED crates\/geo\/src\/big\.rs/);

    const after = JSON.parse(readFileSync(baselinePath, "utf8"));
    assert.equal(after.files["crates/geo/src/big.rs"], 3002, "the pin holds the file's real line count");
    const log = (after.accepted ?? []).filter((a) => a.file === "crates/geo/src/big.rs");
    assert.equal(log.length, 1, "the bump is logged once");
    assert.equal(log[0].from, 3001, "the log names the old count");
    assert.equal(log[0].to, 3002, "the log names the new count");
    assert.match(log[0].reason, /the generated table/, "the log keeps the reason");

    // The tree run is green again: the pin matches the file.
    const green = spawnSync(process.execPath, [tool, "--no-cache"], { cwd: root, encoding: "utf8" });
    const greenOut = `${green.stdout ?? ""}${green.stderr ?? ""}`;
    assert.equal(green.status, 0, `the bumped tree must pass: ${greenOut}`);
    assert.match(greenOut, /RESULT PASS/);

    // A file that is not in the tree cannot be bumped.
    const missing = spawnSync(process.execPath, [tool, "--no-cache", "--accept-file", "crates/geo/src/gone.rs", "--reason", "the file does not exist here"], { cwd: root, encoding: "utf8" });
    assert.equal(missing.status, 2, "a bump for a file that is not there is a usage error");
    assert.match(`${missing.stdout ?? ""}${missing.stderr ?? ""}`, /not a file in the tree/);

    // A file that crossed the cap (it had no pin) can be pinned with a reason.
    writeBaseline({});
    const crossed = spawnSync(process.execPath, [tool, "--no-cache"], { cwd: root, encoding: "utf8" });
    const crossedOut = `${crossed.stdout ?? ""}${crossed.stderr ?? ""}`;
    assert.equal(crossed.status, 1, "a file that crossed the cap fails");
    assert.match(crossedOut, /crates\/geo\/src\/big\.rs: file lines 3002 \(crossed the 3000-line cap\)/);
    assert.match(crossedOut, /--accept-file crates\/geo\/src\/big\.rs/, "the crossing refusal names the escape hatch too");
    const pinned = spawnSync(process.execPath, [tool, "--no-cache", "--accept-file", "crates/geo/src/big.rs", "--reason", "the new file lands over the cap on purpose"], { cwd: root, encoding: "utf8" });
    assert.equal(pinned.status, 0, `the crossing bump must land: ${pinned.stdout ?? ""}${pinned.stderr ?? ""}`);
    const afterPin = JSON.parse(readFileSync(baselinePath, "utf8"));
    assert.equal(afterPin.files["crates/geo/src/big.rs"], 3002, "the new file holds a pin at its real count");
    const pinLog = (afterPin.accepted ?? []).filter((a) => a.file === "crates/geo/src/big.rs").at(-1);
    assert.equal(pinLog.from, null, "the log names that the file had no pin before");
    assert.equal(pinLog.to, 3002, "the log names the new count");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

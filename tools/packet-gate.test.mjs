import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { auditLand, auditPatch, auditResult, unlockRows, untrackedPaths } from "./packet-gate.mjs";
import { ALLOC, CAST, HASH_CONST, NONDET } from "./rust-quality.mjs";

const patch = (oldPath, newPath, line = "+fn run() { true }") => [
  `diff --git a/${oldPath ?? newPath} b/${newPath ?? oldPath}`,
  oldPath ? `--- a/${oldPath}` : "--- /dev/null",
  newPath ? `+++ b/${newPath}` : "+++ /dev/null",
  "@@ -0,0 +1 @@",
  line,
  "",
].join("\n");

// A private caller in another file: the AD5 guard needs one, and `fn g` is no pub item.
const caller = (path, target) => patch(null, path, `+fn g() {\n+    let _ = ${target};\n+}\n`);

test("owned code and tests pass only their declared packet role", () => {
  const src = patch(null, "crates/combat/src/action.rs");
  const testFile = patch(null, "crates/combat/tests/action.rs");
  assert.equal(auditPatch(src + testFile, { owned: ["crates/combat/"], role: "writer" }).ok, true);
  assert.match(auditPatch(src + testFile, { owned: ["crates/combat/"], role: "code" }).errors.join(" "), /changed tests/);
  assert.match(auditPatch(src + testFile, { owned: ["crates/combat/"], role: "tests" }).errors.join(" "), /changed implementation/);
});

test("outside ownership and lookalike prefix cannot be approved", () => {
  const result = auditPatch(patch(null, "crates/combat-extra/src/a.rs"), { owned: ["crates/combat/"], role: "writer" });
  assert.equal(result.ok, false);
  assert.match(result.errors.join(" "), /outside owned paths/);
});

test("added placeholders fail; existing context and tracked TODOs do not", () => {
  const broken = auditPatch(patch(null, "crates/combat/src/a.rs", "+todo!()"), { owned: ["crates/combat/"], role: "writer" });
  assert.match(broken.errors.join(" "), /placeholder/);
  const tracked = auditPatch(patch(null, "crates/combat/src/a.rs", "+// TODO GP12: add a follow-up"), { owned: ["crates/combat/"], role: "writer" });
  assert.equal(tracked.ok, true);
});

test("deleted tests, binary patches and empty patches cannot be green", () => {
  assert.match(auditPatch(patch("crates/combat/tests/a.rs", null, "-test"), { owned: ["crates/combat/"], role: "writer" }).errors.join(" "), /deleted a test/);
  assert.equal(auditPatch("diff --git a/content/x.png b/content/x.png\nBinary files differ\n", { owned: ["content/"], role: "writer" }).ok, false);
  assert.equal(auditPatch("", { owned: ["crates/combat/"], role: "writer" }).ok, false);
});

// Whole-file patch: every line added, with the post-patch file on disk.
function lane(files) {
  const root = mkdtempSync(join(tmpdir(), "packet-gate-"));
  let text = "";
  for (const [path, body] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), body);
    const lines = body.split("\n");
    text += [`diff --git a/${path} b/${path}`, "--- /dev/null", `+++ b/${path}`, `@@ -0,0 +1,${lines.length} @@`, ...lines.map((l) => `+${l}`), ""].join("\n");
  }
  return { root, text, done: () => rmSync(root, { recursive: true, force: true }) };
}
const ids = new Set(["GP19"]);
const rust = (files, owned) => {
  const l = lane(files);
  try { return auditPatch(l.text, { owned, role: "writer", root: l.root, ids }); } finally { l.done(); }
};

test("new production Rust meets the bar; the same code in tests or bins may not need to", () => {
  const bad = rust({ "crates/render/src/a.rs": "pub fn f(v: Option<u8>) -> u8 {\n    println!(\"x\");\n    v.unwrap()\n}\n" }, ["crates/render/"]);
  assert.match(bad.errors.join("\n"), /rust-unwrap/);
  assert.match(bad.errors.join("\n"), /rust-print/);
  const inTests = rust({ "crates/render/src/a.rs": "fn f() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { Some(1).unwrap(); println!(\"ok\"); }\n}\n" }, ["crates/render/"]);
  assert.equal(inTests.ok, true, inTests.errors.join("\n"));
  const bin = rust({ "crates/game/src/bin/tool.rs": "fn main() { println!(\"usage\"); }\n" }, ["crates/game/"]);
  assert.equal(bin.ok, true, bin.errors.join("\n"));
});

test("lint escapes need expect with a reason; dbg never lands", () => {
  const bare = rust({ "crates/render/src/a.rs": "#[allow(clippy::too_many_arguments)]\nfn f() {}\n" }, ["crates/render/"]);
  assert.match(bare.errors.join("\n"), /rust-allow/);
  const silent = rust({ "crates/render/src/a.rs": "#[expect(clippy::too_many_arguments)]\nfn f() {}\n" }, ["crates/render/"]);
  assert.match(silent.errors.join("\n"), /rust-expect-reason/);
  const ok = rust({ "crates/render/src/a.rs": "#[expect(clippy::too_many_arguments, reason = \"flat kernel inputs\")]\nfn f() {}\n" }, ["crates/render/"]);
  assert.equal(ok.ok, true, ok.errors.join("\n"));
  const wrapped = rust({ "crates/render/src/a.rs": "#[expect(\n    clippy::too_many_arguments,\n    reason = \"rustfmt wraps long attributes\"\n)]\nfn f() {}\n" }, ["crates/render/"]);
  assert.equal(wrapped.ok, true, wrapped.errors.join("\n"));
  assert.match(rust({ "crates/render/tests/a.rs": "fn t() { dbg!(1); }\n" }, ["crates/render/"]).errors.join("\n"), /rust-dbg/);
});

test("simulation crates refuse nondeterminism unless waived; presentation crates may hash", () => {
  const src = "use std::collections::HashMap;\nfn f() {}\n";
  assert.match(rust({ "crates/physics/src/a.rs": src }, ["crates/physics/"]).errors.join("\n"), /rust-nondet/);
  const waived = rust({ "crates/physics/src/a.rs": "use std::collections::HashMap; // det-ok: debug names, never hashed\nfn f() {}\n" }, ["crates/physics/"]);
  assert.equal(waived.ok, true, waived.errors.join("\n"));
  assert.equal(rust({ "crates/render/src/a.rs": src }, ["crates/render/"]).ok, true);
});

test("core owns hashing and the alloc probe; copies elsewhere fail", () => {
  const fnv = "fn h(b: &[u8]) -> u64 {\n    let mut h = 0xcbf2_9ce4_8422_2325u64;\n    for x in b { h = (h ^ u64::from(*x)).wrapping_mul(0x100000001b3); }\n    h\n}\n";
  assert.match(rust({ "crates/anim/src/a.rs": fnv }, ["crates/anim/"]).errors.join("\n"), /rust-hash-copy/);
  assert.equal(rust({ "crates/core/src/a.rs": fnv }, ["crates/core/"]).ok, true);
  const probe = "struct P;\nunsafe impl GlobalAlloc for P {}\n";
  assert.match(rust({ "crates/audio/src/a.rs": probe }, ["crates/audio/"]).errors.join("\n"), /rust-alloc-probe-copy/);
});

test("comments carry no board IDs, typed errors only, and hot fns do not allocate", () => {
  assert.match(rust({ "crates/render/src/a.rs": "// GP19 staged rig\npub fn f() {}\n" }, ["crates/render/"]).errors.join("\n"), /rust-board-id/);
  assert.match(rust({ "crates/render/src/a.rs": "/// Parse.\npub fn f() -> Result<u8, String> { Ok(1) }\n" }, ["crates/render/"]).errors.join("\n"), /rust-string-error/);
  const hot = "fn step(out: &mut Vec<u32>) {\n    let scratch: Vec<u32> = Vec::new();\n    out.extend(scratch);\n}\nfn build() -> Vec<u32> { Vec::new() }\n";
  const r = rust({ "crates/arena/src/a.rs": hot }, ["crates/arena/"]);
  assert.equal(r.errors.filter((e) => /rust-hot-alloc/.test(e)).length, 1, r.errors.join("\n"));
  const cold = "fn step(out: &mut Vec<u32>) {\n    let scratch: Vec<u32> = Vec::new(); // alloc-ok: empty Vec never allocates\n    out.extend(scratch);\n}\n";
  assert.equal(rust({ "crates/arena/src/a.rs": cold }, ["crates/arena/"]).ok, true);
});

test("a new function over 100 lines fails and a new cast is reported", () => {
  const long = `pub fn big(x: u64) -> u32 {\n${"    let _ = 1;\n".repeat(110)}    x as u32\n}\n`;
  const r = rust({ "crates/render/src/a.rs": long }, ["crates/render/"]);
  assert.match(r.errors.join("\n"), /rust-long-fn/);
  assert.match(r.warnings.join("\n"), /1 new numeric `as` cast/);
});

test("bad scopes and unknown role refuse instead of silently passing", () => {
  assert.throws(() => auditPatch(patch(null, "x.rs"), { owned: ["../"], role: "writer" }), /invalid owned path/);
  assert.throws(() => auditPatch(patch(null, "x.rs"), { owned: ["x.rs"], role: "review" }), /invalid role/);
});

test("bar constants come from rust-quality instead of local copies", () => {
  const src = readFileSync(new URL("./packet-gate.mjs", import.meta.url), "utf8");
  assert.match(src, /from\s+["']\.\/rust-quality\.mjs["']/);
  for (const name of ["LONG_FN_LINES", "BIG_FILE_LINES", "HOT_FN", "ALLOC", "NONDET", "HASH_CONST", "CAST"]) {
    assert.match(src, new RegExp(`\\b${name}\\b`), `${name} is used`);
    assert.doesNotMatch(src, new RegExp(`^\\s*const\\s+${name}\\s*=`, "m"), `${name} is imported, not redefined`);
  }
  assert.match(String(ALLOC), /Rc::new/, "shared ALLOC covers Rc/Arc (the drift)");
  assert.match(String(CAST), /as\\s/);
  assert.match(String(NONDET), /HashMap/);
  assert.match(String(HASH_CONST), /cbf29ce484222325/i);
});

const baselineSection = [
  "diff --git a/tools/rust-quality.baseline.json b/tools/rust-quality.baseline.json",
  "--- a/tools/rust-quality.baseline.json",
  "+++ b/tools/rust-quality.baseline.json",
  "@@ -0,0 +1 @@",
  "+{}",
  "",
].join("\n");

test("a commit adding ratchet debt is refused unless its landing moves the baseline", () => {
  const cast = patch(null, "crates/render/src/a.rs", "+/// Scale a count.\n+fn f(x: u64) -> u32 {\n+    x as u32\n+}\n");
  const silent = auditPatch(cast, { owned: ["crates/render/"], role: "writer" });
  assert.equal(silent.ok, false);
  assert.match(silent.errors.join("\n"), /ratchet-baseline/);
  const open = auditPatch(`${cast}\n${baselineSection}`, {
    owned: ["crates/render/", "tools/rust-quality.baseline.json"],
    role: "writer",
  });
  assert.equal(open.ok, true, open.errors.join("\n"));
});

test("new expect and panic sites need the same baseline landing", () => {
  const ex = patch(null, "crates/core/src/a.rs", "+/// Get the slot.\n+fn f(v: Option<u8>) -> u8 {\n+    v.expect(\"slot checked above\")\n+}\n");
  const silent = auditPatch(ex, { owned: ["crates/core/"], role: "writer" });
  assert.equal(silent.ok, false);
  assert.match(silent.errors.join("\n"), /rust-expect-site/);
  const open = auditPatch(`${ex}\n${baselineSection}`, {
    owned: ["crates/core/", "tools/rust-quality.baseline.json"],
    role: "writer",
  });
  assert.equal(open.ok, true, open.errors.join("\n"));
});

test("auditLand refuses ratchet debt without a baseline touch, passes with one", () => {
  assert.equal(auditLand({ regressions: [] }).ok, true);
  const refused = auditLand({ regressions: ["engine-ai: casts 107 -> 112"], baselineTouched: false });
  assert.equal(refused.ok, false);
  assert.match(refused.errors.join("\n"), /--accept/);
  const landed = auditLand({ regressions: ["engine-ai: casts 107 -> 112"], baselineTouched: true });
  assert.equal(landed.ok, true);
});

test("an untracked new file is scanned like a hunk, and a clean one passes with an empty diff", () => {
  const bad = lane({ "crates/render/src/a.rs": "#[allow(clippy::too_many_arguments)]\nfn f() {}\n" });
  try {
    const r = auditPatch("", { owned: ["crates/render/"], role: "writer", root: bad.root, ids, untracked: ["crates/render/src/a.rs"] });
    assert.equal(r.ok, false);
    assert.match(r.errors.join("\n"), /rust-allow/);
  } finally { bad.done(); }
  const good = lane({ "crates/render/src/b.rs": "/// One.\nfn f() -> u8 {\n    1\n}\n" });
  try {
    const r = auditPatch("", { owned: ["crates/render/"], role: "writer", root: good.root, ids, untracked: ["crates/render/src/b.rs"] });
    assert.equal(r.ok, true, r.errors.join("\n"));
  } finally { good.done(); }
});

test("an empty diff with no new file is still refused", () => {
  const r = auditPatch("", { owned: ["crates/render/"], role: "writer", untracked: [] });
  assert.equal(r.ok, false);
  assert.match(r.errors.join("\n"), /no usable git patch/);
});

test("a binary untracked file is refused, not read as text", () => {
  const l = lane({});
  try {
    mkdirSync(join(l.root, "content"), { recursive: true });
    writeFileSync(join(l.root, "content/x.png"), Buffer.from([0x89, 0x50, 0x00, 0x47]));
    const r = auditPatch("", { owned: ["content/"], role: "writer", root: l.root, untracked: ["content/x.png"] });
    assert.equal(r.ok, false);
    assert.match(r.errors.join("\n"), /binary/);
  } finally { l.done(); }
});

test("an owned path with no change and no file on disk fails; an existing one with no change does not", () => {
  const l = lane({ "crates/render/src/a.rs": "/// One.\nfn f() {}\n" });
  try {
    const gone = auditPatch(l.text, { owned: ["crates/render/", "crates/render/src/gone.rs"], role: "writer", root: l.root, ids });
    assert.equal(gone.ok, false);
    assert.match(gone.errors.join("\n"), /crates\/render\/src\/gone\.rs: owned path has no change and no file on disk/);
    const idle = auditPatch(l.text, { owned: ["crates/render/", "crates/render/src/a.rs"], role: "writer", root: l.root, ids });
    assert.equal(idle.ok, true, idle.errors.join("\n"));
  } finally { l.done(); }
});

test("untrackedPaths lists only new files under the owned scope of a git tree", () => {
  const dir = mkdtempSync(join(tmpdir(), "packet-gate-git-"));
  try {
    execFileSync("git", ["init", "-q"], { cwd: dir });
    mkdirSync(join(dir, "crates/render/src"), { recursive: true });
    writeFileSync(join(dir, "crates/render/src/new.rs"), "pub fn f() {}\n");
    writeFileSync(join(dir, "other.txt"), "x\n");
    assert.deepEqual(untrackedPaths(["crates/render/"], dir), ["crates/render/src/new.rs"]);
    assert.deepEqual(untrackedPaths([], dir), []);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("auditResult: a DONE needs a proof that names a command or file result, and no FAIL", () => {
  const reply = (line) => `Changed: x.\n\n${line}\n`;
  assert.deepEqual(auditResult(reply("RESULT: DONE - torches cast shadows | proof: cargo test -p engine-render --lib torch 3 passed")), { ok: true, errors: [] });
  const cite = auditResult(reply("RESULT: DONE - LF17 packet | proof: geaen | 11.3 Poses | paths: target/queue/ready/0-lf17.md"));
  assert.equal(cite.ok, false);
  assert.match(cite.errors.join(" "), /proof names no command or file result/);
  assert.match(auditResult(reply("RESULT: DONE - x")).errors.join(" "), /proof is empty/);
  assert.match(auditResult("Ran out of time.\n").errors.join(" "), /no RESULT line/);
  assert.match(auditResult(reply("RESULT: DONE - cut FAIL | proof: node --test tools/x.test.mjs")).errors.join(" "), /RESULT text says FAIL after DONE/);
  assert.match(auditResult("VERDICT: FAIL - no caller\nRESULT: DONE - judged | proof: cargo test -p x 1 passed\n").errors.join(" "), /DONE after a FAIL verdict line/);
  // only a DONE claims a result: the keeper's own BLOCKED line cites the keeper
  assert.equal(auditResult(reply("RESULT: BLOCKED - parked | proof: the keeper")).ok, true);
});

test("GS4 fake-pass guard: a new test passes, an edited assertion plus code fails, TESTS: waives", () => {
  const addOnly = [
    "diff --git a/crates/combat/tests/f.rs b/crates/combat/tests/f.rs",
    "--- a/crates/combat/tests/f.rs",
    "+++ b/crates/combat/tests/f.rs",
    "@@ -0,0 +1,3 @@",
    "+#[test]",
    "+fn t() { assert_eq!(1, 1); }",
    "",
  ].join("\n");
  assert.equal(auditPatch(addOnly, { owned: ["crates/combat/"], role: "writer" }).ok, true);
  const editBoth = [
    "diff --git a/crates/combat/tests/f.rs b/crates/combat/tests/f.rs",
    "--- a/crates/combat/tests/f.rs",
    "+++ b/crates/combat/tests/f.rs",
    "@@ -1,2 +1,2 @@",
    "-    assert_eq!(f(), 5);",
    "+    assert_eq!(f(), 6);",
    "diff --git a/crates/combat/src/f.rs b/crates/combat/src/f.rs",
    "--- a/crates/combat/src/f.rs",
    "+++ b/crates/combat/src/f.rs",
    "@@ -1,2 +1,2 @@",
    "-/// Get f.",
    "-fn f() -> u8 { 5 }",
    "+/// Get f.",
    "+fn f() -> u8 { 6 }",
    "",
  ].join("\n");
  const fail = auditPatch(editBoth, { owned: ["crates/combat/"], role: "writer" });
  assert.equal(fail.ok, false);
  assert.match(fail.errors.join("\n"), /FAKE-PASS-RISK: 1 assertion lines edited/);
  const waived = auditPatch(editBoth, { owned: ["crates/combat/"], role: "writer", testsMark: true });
  assert.equal(waived.ok, true, waived.errors.join("\n"));
});

test("GS4 fake-pass guard: test-only assertion edits pass, JS asserts count too", () => {
  const testOnly = [
    "diff --git a/crates/combat/tests/f.rs b/crates/combat/tests/f.rs",
    "--- a/crates/combat/tests/f.rs",
    "+++ b/crates/combat/tests/f.rs",
    "@@ -1,2 +1,2 @@",
    "-    assert_eq!(f(), 5);",
    "+    assert_eq!(f(), 6);",
    "",
  ].join("\n");
  assert.equal(auditPatch(testOnly, { owned: ["crates/combat/"], role: "writer" }).ok, true);
  const jsBoth = [
    "diff --git a/tools/owned.test.mjs b/tools/owned.test.mjs",
    "--- a/tools/owned.test.mjs",
    "+++ b/tools/owned.test.mjs",
    "@@ -1,2 +1,2 @@",
    "-assert.equal(f(), 5);",
    "+assert.equal(f(), 6);",
    "diff --git a/tools/owned.mjs b/tools/owned.mjs",
    "--- a/tools/owned.mjs",
    "+++ b/tools/owned.mjs",
    "@@ -1,2 +1,2 @@",
    "-export const f = () => 5;",
    "+export const f = () => 6;",
    "",
  ].join("\n");
  const fail = auditPatch(jsBoth, { owned: ["tools/"], role: "writer" });
  assert.equal(fail.ok, false);
  assert.match(fail.errors.join("\n"), /FAKE-PASS-RISK/);
});

test("GS4 fake-pass guard: --tests waives the canned diff on the CLI", () => {
  const dir = mkdtempSync(join(tmpdir(), "packet-gate-gs4-"));
  try {
    const file = join(dir, "edit.patch");
    writeFileSync(file, [
      "diff --git a/crates/combat/tests/f.rs b/crates/combat/tests/f.rs",
      "--- a/crates/combat/tests/f.rs",
      "+++ b/crates/combat/tests/f.rs",
      "@@ -1,2 +1,2 @@",
      "-    assert_eq!(f(), 5);",
      "+    assert_eq!(f(), 6);",
      "diff --git a/crates/combat/src/f.rs b/crates/combat/src/f.rs",
      "--- a/crates/combat/src/f.rs",
      "+++ b/crates/combat/src/f.rs",
      "@@ -1,2 +1,2 @@",
      "-/// Get f.",
      "-fn f() -> u8 { 5 }",
      "+/// Get f.",
      "+fn f() -> u8 { 6 }",
      "",
    ].join("\n"));
    const gate = join(dirname(fileURLToPath(import.meta.url)), "packet-gate.mjs");
    const shut = spawnSync(process.execPath, [gate, "--patch", file, "--owned", "crates/combat/"], { encoding: "utf8" });
    assert.equal(shut.status, 1);
    assert.match(shut.stdout + shut.stderr, /FAKE-PASS-RISK/);
    const open = spawnSync(process.execPath, [gate, "--patch", file, "--owned", "crates/combat/", "--tests"], { encoding: "utf8" });
    assert.equal(open.status, 0, open.stdout + open.stderr);
    assert.match(open.stdout, /RESULT PASS/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("packet-gate --result audits a reply file and exits 1 on a book cite", () => {
  const dir = mkdtempSync(join(tmpdir(), "packet-gate-result-"));
  try {
    const run = (text) => {
      const file = join(dir, "r.md");
      writeFileSync(file, text);
      const r = spawnSync(process.execPath, [join(dirname(fileURLToPath(import.meta.url)), "packet-gate.mjs"), "--result", file], { encoding: "utf8" });
      return { code: r.status, out: r.stdout + r.stderr };
    };
    const good = run("RESULT: DONE - x | proof: node --test tools/x.test.mjs -> 3 pass\n");
    assert.equal(good.code, 0);
    assert.match(good.out, /RESULT PASS/);
    const bad = run("RESULT: DONE - x | proof: geaen | 15.1 Components\n");
    assert.equal(bad.code, 1);
    assert.match(bad.out, /FAIL .*proof names no command or file result/);
    assert.match(bad.out, /RESULT FAIL/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("AD5 unwired guard: an uncalled new pub item fails; a caller, an UNLOCKS row, --unlocks pass", () => {
  const def = patch(null, "crates/render/src/a.rs", "+/// Solo.\n+pub fn solo() -> u8 {\n+    1\n+}\n");
  const alone = auditPatch(def, { owned: ["crates/render/"], role: "writer" });
  assert.equal(alone.ok, false);
  assert.match(alone.errors.join("\n"), /UNWIRED-RISK: solo has no non-test caller outside crates\/render\/src\/a\.rs/);
  const opts = { owned: ["crates/render/"], role: "writer" };
  assert.equal(auditPatch(def + caller("crates/render/src/b.rs", "crate::a::solo"), opts).ok, true);
  assert.equal(auditPatch(def, { ...opts, unlocks: ["PX36"] }).ok, true);
  assert.deepEqual(unlockRows("## Proof\nUNLOCKS: FIX3\nUNLOCKS: E42b\n"), ["FIX3", "E42b"]);
  const dir = mkdtempSync(join(tmpdir(), "packet-gate-unlocks-"));
  try {
    writeFileSync(join(dir, "p.patch"), def);
    writeFileSync(join(dir, "row.md"), "## Proof\nUNLOCKS: AD5\n");
    const gate = join(dirname(fileURLToPath(import.meta.url)), "packet-gate.mjs");
    const run = (...extra) => spawnSync(process.execPath, [gate, "--patch", join(dir, "p.patch"), "--owned", "crates/render/", "--root", dir, ...extra], { encoding: "utf8" });
    assert.equal(run().status, 1, "an uncalled pub item is refused");
    assert.equal(run("--unlocks", join(dir, "row.md")).status, 0, "the packet's UNLOCKS: row waives it");
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("AD5 unwired guard: px36 shape (two new fns calling only each other) fails", () => {
  const r = rust({ "crates/combat/src/bias.rs": "/// Capture bias.\npub fn capture_bias(x: u8) -> u8 {\n    strike_gait_biased(x)\n}\n/// Biased gait.\npub fn strike_gait_biased(x: u8) -> u8 {\n    capture_bias(x)\n}\n", "crates/combat/tests/bias.rs": "fn t() { crate::bias::capture_bias(1); crate::bias::strike_gait_biased(1); }\n" }, ["crates/combat/"]);
  assert.equal(r.ok, false);
  assert.match(r.errors.join("\n"), /UNWIRED-RISK: capture_bias has no non-test caller/);
  assert.match(r.errors.join("\n"), /UNWIRED-RISK: strike_gait_biased has no non-test caller/);
});

test("AD5 unwired guard: rt19 shape (helper referenced only from tests/) fails", () => {
  const r = rust({ "crates/render/src/reflect.rs": "/// Screen ray.\npub fn reflect_screen_for_forward() -> u8 {\n    1\n}\n", "crates/render/tests/reflect.rs": "fn t() { assert_eq!(reflect_screen_for_forward(), 1); }\n" }, ["crates/render/"]);
  assert.equal(r.ok, false);
  assert.match(r.errors.join("\n"), /UNWIRED-RISK: reflect_screen_for_forward has no non-test caller/);
});

test("AD5 unwired guard: fix5 shape (struct and fn used only inside #[cfg(test)]) fails", () => {
  const r = rust({ "crates/anim/src/blend.rs": "/// Blend runner.\npub struct Runner {\n    pub n: u8,\n}\n/// Eval one.\npub fn eval(r: &Runner) -> u8 {\n    r.n\n}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { let r = super::Runner { n: 1 }; assert_eq!(super::eval(&r), 1); }\n}\n" }, ["crates/anim/"]);
  assert.equal(r.ok, false);
  assert.match(r.errors.join("\n"), /UNWIRED-RISK: Runner has no non-test caller/);
  assert.match(r.errors.join("\n"), /UNWIRED-RISK: eval has no non-test caller/);
});

test("AD5 unwired guard: fix3c shape (test-only island of new pub items) fails", () => {
  const r = rust({ "crates/combat/src/trauma.rs": "/// Trauma kind.\npub enum Trauma {\n    Cut,\n}\n/// Landed-blow shake.\npub fn impact_shake(t: Trauma) -> u8 {\n    match t {\n        Trauma::Cut => 1,\n    }\n}\n", "crates/combat/tests/trauma.rs": "#[test]\nfn t() { assert_eq!(impact_shake(Trauma::Cut), 1); }\n" }, ["crates/combat/"]);
  assert.equal(r.ok, false);
  assert.match(r.errors.join("\n"), /UNWIRED-RISK: Trauma has no non-test caller/);
  assert.match(r.errors.join("\n"), /UNWIRED-RISK: impact_shake has no non-test caller/);
});

test("AD5 --orphans reports an existing orphan and exits 0 without touching it", () => {
  const body = "/// Used.\npub fn used() -> u8 {\n    1\n}\npub fn orphan() -> u8 {\n    2\n}\n";
  const l = lane({ "crates/demo/src/a.rs": body, "crates/demo/src/lib.rs": "fn run() -> u8 {\n    crate::a::used()\n}\n" });
  try {
    const gate = join(dirname(fileURLToPath(import.meta.url)), "packet-gate.mjs");
    const r = spawnSync(process.execPath, [gate, "--orphans", "--root", l.root], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stdout + r.stderr);
    assert.match(r.stdout, /ORPHAN crates\/demo\/src\/a\.rs:5 pub fn orphan/);
    assert.doesNotMatch(r.stdout, /pub fn used/);
    assert.match(r.stdout, /RESULT PASS: 1 orphan/);
    assert.equal(readFileSync(join(l.root, "crates/demo/src/a.rs"), "utf8"), body);
  } finally { l.done(); }
});

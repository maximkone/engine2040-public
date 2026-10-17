import assert from "node:assert/strict";
import test from "node:test";
import { parseBoardTasks, largeBlobViolations, LFS_SIZE_LIMIT, hasHelpFlag, WORKSPACE_CHECK_HELP, testLayoutViolations, OWN_TEST_BINARY, allowedEdges } from "./workspace-check.mjs";
import { parseBoard, needsOf } from "./board-slice.mjs";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { deriveFixedApi, genreHits, maskRust, platformMathHits, productionCode } from "./sim-source-scan.mjs";

const board = (lines) => lines.join("\n");

test("a clean board parses with no errors", () => {
  const { tasks, errors } = parseBoardTasks(board([
    "- [x] PF6 done row",
    "- [ ] GM3 open row, needs: PF6.",
    "- [ ] OPT7 another row",
  ]));
  assert.equal(tasks.size, 3);
  assert.deepEqual(errors, []);
  assert.equal(tasks.get("GM3").done, false);
  assert.equal(tasks.get("PF6").done, true);
});

test("a duplicate task id is an error", () => {
  const { errors } = parseBoardTasks(board(["- [ ] PF6 one", "- [ ] PF6 two"]));
  assert.deepEqual(errors, ["board: duplicate task PF6"]);
});

test("a needs: naming an unknown task is an error", () => {
  const { errors } = parseBoardTasks(board(["- [ ] GM3 row, needs: NOPE9."]));
  assert.deepEqual(errors, ["board: GM3 needs unknown task NOPE9"]);
});

test("a ticked task needing an open one is an error; open-needs-open is fine", () => {
  const bad = parseBoardTasks(board(["- [x] GM3 row, needs: PF6.", "- [ ] PF6 base."]));
  assert.deepEqual(bad.errors, ["board: GM3 is ticked but needs open PF6"]);
  const good = parseBoardTasks(board(["- [ ] GM3 row, needs: PF6.", "- [ ] PF6 base."]));
  assert.deepEqual(good.errors, []);
});

test("indented continuation lines join the task text before needs parsing", () => {
  const { tasks, errors } = parseBoardTasks(board([
    "- [x] GM3 row",
    "  continued, needs: PF6.",
    "- [ ] PF6 base.",
  ]));
  assert.deepEqual(errors, ["board: GM3 is ticked but needs open PF6"]);
  assert.match(tasks.get("GM3").text, /continued/);
});

test("a blank line ends the current task; lowercase words are not deps", () => {
  const { errors } = parseBoardTasks(board(["- [ ] GM3 row", "", "needs: nothing here."]));
  assert.deepEqual(errors, []);
});

test("suffixed ids (letter after digit) parse", () => {
  const { tasks, errors } = parseBoardTasks(board(["- [ ] SP13a spinoff, needs: SP9."]));
  assert.deepEqual(errors, ["board: SP13a needs unknown task SP9"]);
  assert.equal(tasks.has("SP13a"), true);
});

test("LFS size guard: a newly tracked non-LFS file over 5 MB fails", () => {
  const bad = largeBlobViolations([{ path: "content/arena/src/new.gltf", bytes: 6 * 1024 * 1024, lfs: false }]);
  assert.equal(bad.length, 1);
  assert.match(bad[0], /new\.gltf/);
  assert.match(bad[0], /LFS/);
});

test("LFS size guard: one byte over the limit fails", () => {
  assert.equal(largeBlobViolations([{ path: "x.bin", bytes: LFS_SIZE_LIMIT + 1, lfs: false }]).length, 1);
});

test("LFS size guard: LFS-tracked, small, and exactly-5 MB files pass", () => {
  assert.deepEqual(largeBlobViolations([
    { path: "content/arena/hdri/a.hdr", bytes: 26 * 1024 * 1024, lfs: true },
    { path: "content/arena/dressing.json", bytes: 1024, lfs: false },
    { path: "content/edge.bin", bytes: LFS_SIZE_LIMIT, lfs: false },
  ]), []);
});

test("--help is detected before the audit runs", () => {
  assert.equal(hasHelpFlag(["--help"]), true);
  assert.equal(hasHelpFlag(["-h"]), true);
  assert.equal(hasHelpFlag([]), false);
  assert.equal(hasHelpFlag(["--locked"]), false);
  assert.match(WORKSPACE_CHECK_HELP, /Usage:/);
  assert.match(WORKSPACE_CHECK_HELP, /--help/);
});

test("parity: parseBoardTasks agrees with board-slice parseBoard/needsOf", () => {
  const text = board([
    "### Section (owner team, `docs/plan/frontier.md`)",
    "- [x] PF6 done row",
    "- [ ] GM3 open row,",
    "  continued, needs: PF6.",
    "## Other",
    "- [ ] OPT7 another row, needs: none.",
    "- [ ] GM3 repeat row",
  ]);
  const { tasks, order } = parseBoard(text);
  const checked = parseBoardTasks(text);
  assert.equal(checked.tasks.size, tasks.size);
  for (const id of order) {
    assert.equal(checked.tasks.get(id).done, tasks.get(id).done);
    const direct = needsOf(tasks.get(id)).sort();
    const viaText = (checked.tasks.get(id).text.match(/needs:\s*([^.]*)\./)?.[1] ?? "")
      .split(",").map((s) => s.trim().split(/\s+/)[0]).filter((s) => /^[A-Z]/.test(s)).sort();
    assert.deepEqual(viaText, direct);
  }
  assert.deepEqual(checked.errors, ["board: duplicate task GM3"]);
});

// ---- determinism gate: Fixed math passes, platform float math still fails ----

const FIXED_SRC = readFileSync(new URL("../crates/core/src/fixed.rs", import.meta.url), "utf8");
const FIXED_API = deriveFixedApi(FIXED_SRC);
const USE_FIXED = "use engine_core::fixed::Fixed;\n";
const mathHits = (rust) => platformMathHits(rust, FIXED_API);

test("determinism: the Fixed surface is read from fixed.rs, not hardcoded", () => {
  for (const name of ["ONE", "ZERO", "PI"]) assert.equal(FIXED_API.consts.has(name), true, name);
  assert.equal(FIXED_API.consts.has("SCALE"), false, "SCALE is an i64, not a Fixed");
  for (const name of ["from_int", "from_bits", "atan2"]) assert.equal(FIXED_API.ctors.has(name), true, name);
  for (const name of ["sin", "cos", "tan", "abs", "clamp", "sqrt"]) assert.equal(FIXED_API.methods.has(name), true, name);
  assert.equal(FIXED_API.methods.has("sin_cos"), false, "tuple result, not Fixed");
  assert.equal(FIXED_API.selfFns.has("sin_cos"), true);
  assert.equal(FIXED_API.selfFns.has("exp"), false, "Fixed has no exp");
  const mini = "impl Fixed {\n pub const K: Self = Self(1);\n pub fn make(v: i64) -> Self { Self(v) }\n pub fn twice(self) -> Self { self }\n pub fn raw(self) -> i64 { 0 }\n}\n";
  const api = deriveFixedApi(mini);
  assert.deepEqual([...api.consts], ["K"]);
  assert.deepEqual([...api.ctors], ["make"]);
  assert.deepEqual([...api.methods], ["twice"]);
  assert.deepEqual([...api.selfFns].sort(), ["raw", "twice"]);
});

test("determinism: Fixed trig through params, lets, calls and groups passes", () => {
  // Shape of the MP10 interp.rs slerp, the call sites that tripped the gate.
  const src = `${USE_FIXED}
fn half_angle(yaw: i16) -> Fixed {
    Fixed::from_int(i64::from(yaw)) * Fixed::PI / Fixed::from_int(3600)
}
pub fn from_yaw(yaw: i16) -> (Fixed, Fixed) {
    let (s, c) = half_angle(yaw).sin_cos();
    (s, c)
}
pub fn slerp(a: Fixed, b: Fixed, t: Fixed) -> Fixed {
    let t = t.clamp(Fixed::ZERO, Fixed::ONE);
    let theta = Fixed::from_int(3) * Fixed::PI
        / Fixed::from_int(3600);
    let sin_theta = theta.sin();
    let wa = ((Fixed::ONE - t) * theta).sin() / sin_theta;
    let wb = (t * theta).sin() / sin_theta;
    a * wa + b * wb
}
`;
  assert.deepEqual(mathHits(src), []);
});

test("determinism: Fixed paths, chains, fields and file consts pass", () => {
  const src = `${USE_FIXED}
const STEP: Fixed = Fixed::from_int(2);
pub struct Rig { angle: Fixed }
impl Rig {
    pub fn facing(&self) -> Fixed { self.angle.cos() }
    pub fn pi_sin() -> Fixed { Fixed::PI.sin() }
    pub fn two(v: Fixed) -> Fixed { v.abs().sin() + Fixed::from_int(2).tan() + STEP.cos() }
}
`;
  assert.deepEqual(mathHits(src), []);
});

test("determinism: an f32 or f64 receiver still fails, even beside Fixed code", () => {
  const cases = {
    "typed f32 param": "fn f(x: f32) -> f32 { x.sin() }",
    "typed f64 param": "fn f(x: f64) -> f64 { x.powf(2.0) }",
    "untyped float literal": "fn f() -> f32 { let a = 2.0; a.cos() }",
    "result of an f32 fn": "fn deg() -> f32 { 1.0 }\nfn f() -> f32 { let a = deg(); a.exp() }",
    "f32 group": "fn f(x: f32, y: f32) -> f32 { (x * y).sin() }",
    "float literal": "fn f() -> f32 { 1.5_f32.tan() }",
    "Fixed converted to float": "fn f(t: Fixed) -> f32 { t.lossy().sin() }",
    "cast to float": "fn f(t: Fixed) -> f32 { (t as f32).sin() }",
    "method Fixed does not have": "fn f(t: Fixed) -> Fixed { t.exp() }",
    "unknown call": "fn f() -> f32 { mystery(1.0).ln() }",
    "indexed array": "fn f(v: [f32; 3]) -> f32 { v[0].hypot(1.0) }",
    "tuple field": "fn f(p: (f32, f32)) -> f32 { p.0.sin() }",
    "macro result": "fn f(x: Fixed) -> f32 { wrap!(x).sin() }",
    "shadowed to a float": "fn f(t: Fixed) -> f32 { let t = t.lossy(); t.sin() }",
    "closure param": "fn f(t: Fixed, v: &[f32]) { for t in v { t.sin(); } }",
  };
  for (const [name, body] of Object.entries(cases)) {
    assert.equal(mathHits(`${USE_FIXED}${body}\n`).length, 1, name);
  }
});

test("determinism: a name that is Fixed in one fn and f32 in another only fails the f32 one", () => {
  const src = `${USE_FIXED}fn a(t: Fixed) -> Fixed { t.sin() }
fn b(t: f32) -> f32 {
    t.sin()
}
`;
  const hits = mathHits(src);
  assert.equal(hits.length, 1);
  assert.equal(hits[0].line, 4);
  assert.equal(hits[0].receiver, "t");
  assert.equal(hits[0].method, "sin");
});

test("determinism: without a real Fixed import nothing is exempt", () => {
  assert.equal(mathHits("fn f(t: Fixed) -> Fixed { t.sin() }").length, 1);
  const own = "use something::Other;\nstruct Fixed(f32);\nimpl Fixed { fn s(&self) -> f32 { self.0.sin() } }\n";
  assert.equal(mathHits(own).length, 1);
  assert.equal(platformMathHits(`${USE_FIXED}fn f(t: Fixed) -> Fixed { t.sin() }`).length, 1, "no API means no proof");
});

test("determinism: comments and strings never count; a real cfg(test) ends the scan", () => {
  const quiet = `${USE_FIXED}// x.sin() in a comment
/// doc: y.cos()
/* z.exp() /* nested w.ln() */ */
fn f() -> &'static str { "q.powf(2.0)" }
fn g() -> &'static str { r#"r.hypot(1.0)"# }
`;
  assert.deepEqual(mathHits(quiet), []);
  const tricked = `${USE_FIXED}// #[cfg(test)] in a comment must not hide the rest\nfn f(x: f32) -> f32 { x.sin() }\n`;
  assert.equal(mathHits(tricked).length, 1);
  const tests = `${USE_FIXED}fn ok(t: Fixed) -> Fixed { t.sin() }\n#[cfg(test)]\nmod tests { fn t(x: f32) -> f32 { x.sin() } }\n`;
  assert.deepEqual(mathHits(tests), []);
});

test("maskRust keeps offsets and newlines and tells lifetimes from char literals", () => {
  const src = "fn f<'a>(x: &'a str) -> char { let q = '\\''; let c = 'x'; // note 'z'\n \"s\\\"t\" }\n";
  const masked = maskRust(src);
  assert.equal(masked.length, src.length);
  assert.equal(masked.split("\n").length, src.split("\n").length);
  assert.match(masked, /<'a>/);
  assert.match(masked, /&'a str/);
  assert.doesNotMatch(masked, /'x'|note|'z'|s\\"t/);
  assert.equal(productionCode("fn a() {}\n// #[cfg(test)]\nfn b() {}\n#[cfg(test)]\nfn c() {}\n").includes("fn b"), true);
  assert.equal(productionCode("fn a() {}\n#[cfg(test)]\nfn c() {}\n").includes("fn c"), false);
});

test("determinism: the MP10 interp.rs lands clean (the false positives that reverted it)", { skip: existsSync(new URL("../crates/net/src/interp.rs", import.meta.url)) ? false : "missing crates/net/src/interp.rs; skipping check (no source to scan)" }, () => {
  const src = readFileSync(new URL("../crates/net/src/interp.rs", import.meta.url), "utf8");
  assert.deepEqual(mathHits(src), []);
});

// ---- GE2 genre ledger: identifiers in code only ---------------------------

test("GE2: genre words in comments, doc lines and strings do not count", () => {
  const src = `// Fighter slot
/// FighterSnap docs and Gladiator lore
/* gladiator */
fn f() -> &'static str { "Fighter" }
fn g() -> &'static str { r"Gladiator" }
`;
  assert.deepEqual(genreHits(src), []);
});

test("GE2: genre identifiers in code count, whole-token, with their names", () => {
  assert.deepEqual(genreHits("use crate::delta::FighterSnap;\n"), ["FighterSnap"]);
  assert.deepEqual(genreHits("pub type BodySnap = FighterSnap;\n"), ["FighterSnap"]);
  assert.deepEqual(genreHits("struct Gladiator; fn gladiator_hp() {}\n"), ["Gladiator", "gladiator_hp"]);
  assert.deepEqual(genreHits("fn f() { let x = MaxFighters; }\n"), ["MaxFighters"]);
});

test("GE2: neutral names, lowercase fighter words and test-only uses pass", () => {
  assert.deepEqual(genreHits("use crate::delta::{BodySnap, FullSnap};\nfn of(fighter: &BodySnap) { fighters.len(); }\n"), []);
  assert.deepEqual(genreHits("fn f() {}\n#[cfg(test)]\nmod t { fn g() -> FighterSnap { todo!() } }\n"), []);
  assert.deepEqual(genreHits("// #[cfg(test)] is only a comment\nfn f() -> FighterSnap { todo!() }\n"), ["FighterSnap"]);
});

test("GE2: the MP10 interp.rs names no genre identifier", { skip: existsSync(new URL("../crates/net/src/interp.rs", import.meta.url)) ? false : "missing crates/net/src/interp.rs; skipping check (no source to scan)" }, () => {
  const src = readFileSync(new URL("../crates/net/src/interp.rs", import.meta.url), "utf8");
  assert.deepEqual(genreHits(src), []);
});

test("GE2: playtest replay.rs names no genre identifier (neutral BodyTape)", { skip: existsSync(new URL("../crates/playtest/src/replay.rs", import.meta.url)) ? false : "missing crates/playtest/src/replay.rs; skipping check (no source to scan)" }, () => {
  const src = readFileSync(new URL("../crates/playtest/src/replay.rs", import.meta.url), "utf8");
  assert.deepEqual(genreHits(src), []);
});

test("workspace-check runs both gates through the shared scanner", () => {
  const gate = readFileSync(new URL("./workspace-check.mjs", import.meta.url), "utf8");
  assert.match(gate, /platformMathHits\(/);
  assert.match(gate, /genreHits\(/);
  assert.doesNotMatch(gate, /split\("#\[cfg\(test\)\]"\)\[0\]/, "raw cfg(test) split would bypass comment masking");
});

test("documented edges: every optional engine-endpoint manifest edge has one allow entry", () => {
  const EDGE = /^(engine-[a-z-]+)\s*=\s*\{\s*workspace\s*=\s*true,\s*optional\s*=\s*true/gm;
  const names = (toml) => [...toml.matchAll(EDGE)].map((m) => m[1]);
  assert.deepEqual(names("engine-physics = { workspace = true, optional = true }\n"), ["engine-physics"]);
  assert.deepEqual(names("engine-core = { workspace = true }\n"), [], "a required edge is not optional");
  // The audit reads the real manifest: the doc row must grow with it (PX40 physics pack).
  const toml = readFileSync(new URL("../crates/endpoint/Cargo.toml", import.meta.url), "utf8");
  const allowed = new Set(allowedEdges.get("engine-endpoint"));
  for (const dep of names(toml)) {
    assert.equal(allowed.has(dep), true, `${dep} is an engine-endpoint manifest edge, so it needs one allow entry`);
  }
});

test("test layout guard: a new top-level test file in a merged crate fails with the fix", () => {
  const found = testLayoutViolations([{ name: "net", files: [{ file: "fresh_vectors.rs", text: "#[test] fn a() {}" }] }]);
  assert.equal(found.length, 1);
  assert.match(found[0], /crates\/net\/tests\/fresh_vectors\.rs/);
  assert.match(found[0], /tests\/it\//);
});

test("test layout guard: a listed own-process file and a #[global_allocator] file pass", () => {
  assert.ok(OWN_TEST_BINARY["game/crash_hook"]);
  assert.deepEqual(testLayoutViolations([
    { name: "game", files: [{ file: "crash_hook.rs", text: "" }] },
    { name: "physics", files: [{ file: "ragdoll_tape.rs", text: "#[global_allocator]\nstatic A: X = X;" }] },
  ]), []);
});

test("test layout guard: the real tree has no stray top-level test file in a merged crate", () => {
  const crates = new URL("../crates/", import.meta.url);
  if (!existsSync(crates)) return;
  const layout = [];
  for (const name of readdirSync(crates)) {
    const tests = new URL(`../crates/${name}/tests/`, import.meta.url);
    if (!existsSync(new URL("it/main.rs", tests))) continue;
    layout.push({ name, files: readdirSync(tests).filter((n) => n.endsWith(".rs")).map((file) => ({ file, text: readFileSync(new URL(file, tests), "utf8") })) });
  }
  assert.ok(layout.length >= 10, `expected the 10 merged crates, found ${layout.length}`);
  assert.deepEqual(testLayoutViolations(layout), []);
});

test("wiring cache: changed .rs or scanner inputs bust the cache (no stale numbers)", async () => {
  const { mkdtempSync, mkdirSync: mkDir, writeFileSync: writeTmp, rmSync: rmTmp } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const gateUrl = new URL("./workspace-check.mjs", import.meta.url);
  const gateSrc = readFileSync(gateUrl, "utf8");
  assert.match(gateSrc, /cached\?\.fp === fp/, "loader must require an exact fingerprint match");
  const shimSrc = gateSrc
    .replace("const WIRING_CACHE =", "export const WIRING_CACHE =")
    .replace("function wiringFingerprint(", "export function wiringFingerprint(")
    .replace("function loadWiringCache(", "export function loadWiringCache(")
    .replace("function saveWiringCache(", "export function saveWiringCache(");
  assert.match(shimSrc, /export function wiringFingerprint\(/, "shim must export the real fingerprint");
  const shimUrl = new URL("./.wiring-cache-shim.test-tmp.mjs", import.meta.url);
  writeTmp(shimUrl, shimSrc);
  try {
    const shim = await import(`${shimUrl.href}?t=${Date.now()}`);
    const repoDir = mkdtempSync(join(tmpdir(), "wiring-cache-"));
    try {
      mkDir(join(repoDir, "tools"), { recursive: true });
      mkDir(join(repoDir, "crates", "demo", "src"), { recursive: true });
      writeTmp(join(repoDir, "tools", "wiring-scan.mjs"), "// scanner v1\n");
      writeTmp(join(repoDir, "crates", "demo", "src", "lib.rs"), "pub fn alpha() {}\n");
      const fp1 = shim.wiringFingerprint(repoDir);
      assert.ok(fp1.includes("tools/wiring-scan.mjs:"), "fingerprint keys on the scanner");
      assert.match(fp1, /demo.*lib\.rs:/, "fingerprint keys on scanned .rs files");
      shim.saveWiringCache(repoDir, fp1, "islands: 1");
      assert.equal(shim.loadWiringCache(repoDir, fp1), "islands: 1", "unchanged inputs hit the cache");
      writeTmp(join(repoDir, "crates", "demo", "src", "lib.rs"), "pub fn alpha() {}\npub fn beta_fresh_item() {}\n");
      const fp2 = shim.wiringFingerprint(repoDir);
      assert.notEqual(fp2, fp1, "changed .rs must change the fingerprint");
      assert.equal(shim.loadWiringCache(repoDir, fp2), null, "stale cache must not print stale numbers");
      writeTmp(join(repoDir, "tools", "wiring-scan.mjs"), "// scanner v2\n");
      const fp3 = shim.wiringFingerprint(repoDir);
      assert.notEqual(fp3, fp2, "scanner edit must change the fingerprint");
      assert.equal(shim.loadWiringCache(repoDir, fp3), null, "stale cache must not survive a scanner edit");
    } finally {
      rmTmp(repoDir, { recursive: true, force: true });
    }
  } finally {
    rmTmp(shimUrl, { force: true });
  }
});

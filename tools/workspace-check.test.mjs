import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { parseBoardTasks, largeBlobViolations, LFS_SIZE_LIMIT, hasHelpFlag, WORKSPACE_CHECK_HELP, testLayoutViolations, OWN_TEST_BINARY, allowedEdges, parseDeclaredModules, parseIncludedFiles, parseTestModules, sourceFilesUnder, romeSourceAudit, romeSourceViolations, romeContentFiles, codexRefsNamed, codexRefList, unwiredModulesLine, LAYERS, SIMULATION_CRATES, PRESENTATION_CRATES, PLATFORM_CRATES, GPU_BOUNDARY_LEGACY, layerEdgeViolations, gpuBoundaryFindings, ioBoundaryFindings } from "./workspace-check.mjs";
import { mentions, pathTokens, tokensMention, rowsWithReason, summaryLine, scan as scanUnwired } from "./unwired-code-gate.mjs";import { parseBoard, needsOf } from "./board-slice.mjs";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { deriveFixedApi, genreHits, ioHits, maskRust, platformMathHits, productionCode, productionSource } from "./sim-source-scan.mjs";
import { scanIslandsFrom } from "./wiring-scan.mjs";

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
  assert.match(gate, /genreFindings\(/);
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

test("module reachability: one parse per file answers every lookup (E51)", () => {
  const names = parseDeclaredModules([
    "mod plain;",
    "  pub mod published;",
    "pub(crate) mod scoped;",
    "pub(in crate::x) mod nested;",
    "let s = \"mod quoted;\";",
  ].join("\n"));
  assert.deepEqual([...names].sort(), ["nested", "plain", "published", "scoped"]);
  assert.equal(names.has("quoted"), false, "a string is not a module declaration");
});

test("module reachability: an include!() names the file it pulls in (E51)", () => {
  const names = parseIncludedFiles([
    'include!("editor/commands_coalesce.rs");',
    'include!("plain.rs");',
  ].join("\n"));
  assert.deepEqual([...names].sort(), ["commands_coalesce", "plain"]);
});

test("the shared source list is the wiring file set: tests directories stay out (E51)", () => {
  const dir = mkdtempSync(join(tmpdir(), "ws-sources-"));
  try {
    mkdirSync(join(dir, "demo", "src"), { recursive: true });
    mkdirSync(join(dir, "demo", "tests"), { recursive: true });
    writeFileSync(join(dir, "demo", "src", "lib.rs"), "pub fn a() {}\n");
    writeFileSync(join(dir, "demo", "src", "mod.rs"), "pub fn b() {}\n");
    writeFileSync(join(dir, "demo", "tests", "it.rs"), "fn t() {}\n");
    const files = sourceFilesUnder(dir).map((f) => f.replace(/\\/g, "/"));
    assert.equal(files.length, 2, "the tests directory is not scanned");
    assert.ok(files.some((f) => f.endsWith("demo/src/lib.rs")));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the in-process wiring scan answers like the tool it replaces (E51)", () => {
  // fileURLToPath, not pathname: on Linux a pathname keeps no leading slash (home/...), a relative path that is not there.
  const root = fileURLToPath(new URL("../crates/", import.meta.url));
  if (!existsSync(root)) return;
  const files = sourceFilesUnder(root);
  assert.ok(files.length > 100, `expected the crate sources, found ${files.length}`);
  const entries = files.map((file) => ({ file, text: readFileSync(file, "utf8") }));
  const islands = scanIslandsFrom(entries, root);
  assert.equal(typeof islands.length, "number");
  const tool = spawnSync(process.execPath, [fileURLToPath(new URL("./wiring-scan.mjs", import.meta.url)), root], { encoding: "utf8", maxBuffer: 1 << 26 });
  const line = (tool.stdout ?? "").split("\n")[0].trim();
  assert.equal(line, `islands: ${islands.length}`, "the in-process scan must count what the tool counts");
});

// ---- PW48: the unwired-code gate gets a caller ------------------------------

// One tree scan for the whole group: the scan reads every crate file, and
// three tests want the same answer.
let unwiredScan = null;
function unwiredTreeScan() {
  if (unwiredScan !== null) return unwiredScan;
  const root = fileURLToPath(new URL("../crates/", import.meta.url));
  if (!existsSync(root)) return null;
  unwiredScan = scanUnwired(root);
  return unwiredScan;
}

// The path token index is the fast answer; `mentions` is the readable
// reference. They must agree, or the index hides a caller.
test("unwired gate: the path token index answers like the reference (PW48)", () => {
  const shapes = [
    ["engine_ai::behavior::tick", ["engine_ai"], "behavior", []],
    ["engine_ai::behaviorx::tick", ["engine_ai"], "behavior", []],
    ["engine_ai::{behavior, other}", ["engine_ai"], "behavior", []],
    ["engine_ai::{behavior as b, other}", ["engine_ai"], "behavior", []],
    ["engine_ai::{other, behavior as b}", ["engine_ai"], "behavior", []],
    ["foo_engine_ai::behavior", ["engine_ai"], "behavior", []],
    ["engine_ai :: behavior", ["engine_ai"], "behavior", []],
    ["engine_ai::LoadQueue", ["engine_ai"], "load_queue", ["LoadQueue"]],
    ["engine_ai::behavior!", ["engine_ai"], "behavior", []],
    ["use engine_ai::behavior;", ["engine_ai"], "behavior", []],
    ["engine_ai::{a, engine_render::thing}", ["engine_render"], "thing", []],
    ["engine_ai::{a, engine_render::thing}", ["engine_ai"], "a", []],
    ["engine_ai::{a}", ["engine_ai"], "b", []],
    ["engine_ai::{a::b, c}", ["engine_ai"], "a", []],
  ];
  for (const [text, idents, module, reExported] of shapes) {
    const reference = mentions(text, idents, module, reExported);
    const index = tokensMention(pathTokens(text), idents, module, reExported);
    assert.equal(index, reference, `the index and the reference must agree on ${JSON.stringify(text)}`);
  }
});

test("unwired gate: the summary line counts the rows it is given (PW48)", () => {
  const result = { publicCount: 10, crateCount: 3, rows: [] };
  const rows = [{ reason: "no-caller" }, { reason: "no-caller" }, { reason: "behind-features:import" }];
  assert.equal(
    summaryLine(result, rows),
    "SUMMARY unwired modules: 3 of 10 public modules in 3 crates have no caller outside their own crate",
  );
  const sharp = rowsWithReason({ rows }, "no-caller");
  assert.equal(sharp.length, 2, "the sharp half keeps the feature-gated modules out");
  assert.equal(summaryLine(result, sharp),
    "SUMMARY unwired modules: 2 of 10 public modules in 3 crates have no caller outside their own crate");
});

test("unwired gate: the sharp half is a strict subset of the whole list (PW48)", () => {
  const result = unwiredTreeScan();
  if (!result) return;
  const sharp = rowsWithReason(result, "no-caller");
  assert.ok(sharp.length > 0, "the tree holds dead public modules today");
  assert.ok(sharp.length < result.rows.length, "the feature-gated modules are a separate class");
  for (const row of sharp) assert.equal(row.reason, "no-caller", "the sharp half holds only no-caller rows");
  for (const row of result.rows) {
    if (row.reason !== "no-caller") assert.ok(row.reason.startsWith("behind-features:"), "every other row names its features");
  }
});

test("unwired gate: the self-test still proves the known dead modules (PW48)", () => {
  const tool = spawnSync(process.execPath, [fileURLToPath(new URL("./unwired-code-gate.mjs", import.meta.url)), "--self-test"], { encoding: "utf8", maxBuffer: 1 << 26 });
  assert.equal(tool.status, 0, "the self-test must pass");
  assert.match(tool.stdout, /PASS unwired-code-gate: 10 known dead modules found, 4 live modules not flagged, fixture ok/);
  assert.match(tool.stdout, /index-matches-reference \d+ pairs, 0 differences/, "the index must answer like the reference");
});

test("unwired gate: the audit prints one report-only line with the live count (PW48)", () => {
  const root = fileURLToPath(new URL("../crates/", import.meta.url));
  if (!existsSync(root)) return;
  const line = unwiredModulesLine(root);
  assert.match(line, /^WARN unwired modules \(report-only\): SUMMARY unwired modules: \d+ of \d+ public modules in \d+ crates have no caller outside their own crate$/);
  const numbers = line.match(/SUMMARY unwired modules: (\d+) of (\d+) public modules in (\d+) crates/);
  const sharp = Number(numbers[1]);
  const publicCount = Number(numbers[2]);
  const crateCount = Number(numbers[3]);
  assert.ok(sharp > 0, "the tree holds dead public modules today");
  assert.ok(sharp < publicCount, "not every public module is dead");
  assert.ok(crateCount > 1, "the tree holds many crates");
  // The count comes from the scan, not from a constant: the sharp rows of a
  // fresh scan must give the same numbers.
  const result = unwiredTreeScan();
  const fresh = rowsWithReason(result, "no-caller");
  assert.equal(fresh.length, sharp, "the printed count is the scan's own count");
  assert.equal(result.publicCount, publicCount, "the printed total is the scan's own total");
  assert.equal(result.crateCount, crateCount, "the printed crate count is the scan's own count");
});

// ---- CDX6: a Rome number needs a source -----------------------------------

// A tree with Rome tables in it, and the codex index when one is wanted.
function romeTree(files, index = null) {
  const root = mkdtempSync(join(tmpdir(), "rome-lint-"));
  const romeDir = join(root, "content", "rome");
  mkdirSync(romeDir, { recursive: true });
  for (const [name, text] of Object.entries(files)) {
    const file = join(romeDir, name);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, text);
  }
  if (index) {
    const indexDir = join(root, "content", "local", "codex");
    mkdirSync(indexDir, { recursive: true });
    writeFileSync(join(indexDir, "index.jsonl"), index);
  }
  return root;
}

test("rome source lint: an entry with a number and no ref is reported by path and key", () => {
  const root = romeTree({ "camp.toml": "# a camp\n\n[ditch.one_night]\nwidth_ft = 5\ndepth_ft = 3\n" });
  try {
    const audit = romeSourceAudit(root);
    assert.equal(audit.violations.length, 1);
    assert.match(audit.violations[0], /content\/rome\/camp\.toml/, "the violation names the path");
    assert.match(audit.violations[0], /ditch\.one_night/, "the violation names the key");
    assert.match(audit.violations[0], /no ref/, "the violation says what is missing");
    assert.equal(audit.entries, 1);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rome source lint: a ref with the number passes and an entry with no number needs none", () => {
  const root = romeTree({
    "camp.toml": "[units]\nmm_per_ft = 296\nref = \"rome.md section 5\"\n\n[kit.lorica]\ncertainty = \"attested\"\n",
  });
  try {
    const audit = romeSourceAudit(root);
    assert.deepEqual(audit.violations, []);
    assert.equal(audit.entries, 1, "only the section with a number is judged");
    assert.equal(audit.prose, 1, "the ref names a source in prose, not a codex passage");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rome source lint: a draft table is not game data", () => {
  const root = romeTree({
    "tech/iron.draft.toml": "draft = true\ntable = \"iron\"\n\n[iron.bar]\nbeats = 6\n",
    "iron.toml": "draft = true\n\n[iron.bar]\nbeats = 6\n",
  });
  try {
    const audit = romeSourceAudit(root);
    assert.deepEqual(audit.violations, []);
    assert.equal(audit.files, 0, "both drafts are left out");
    assert.equal(romeContentFiles(root).length, 0);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rome source lint: a ref in the codex passage form must be a passage the index holds", () => {
  const root = romeTree({
    "tech/iron.toml": "[iron.bar]\nbeats = 6\nref = \"vegetius.3.8\"\n",
    "tech/axe.toml": "[iron.axe]\nbeats = 4\nref = \"vegetius.3.9\"\n",
  }, `${JSON.stringify({ ref: "vegetius.3.8" })}\n`);
  try {
    const audit = romeSourceAudit(root);
    assert.equal(audit.violations.length, 1);
    assert.match(audit.violations[0], /content\/rome\/tech\/axe\.toml/);
    assert.match(audit.violations[0], /iron\.axe/);
    assert.match(audit.violations[0], /vegetius\.3\.9/);
    assert.match(audit.violations[0], /codex index does not hold/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rome source lint: with no codex index the ref-list leg is skipped, not failed", () => {
  const root = romeTree({ "tech/iron.toml": "[iron.bar]\nbeats = 6\nref = \"vegetius.3.9\"\n" });
  try {
    const audit = romeSourceAudit(root);
    assert.deepEqual(audit.violations, []);
    assert.equal(audit.refs, null, "there is no list to judge by");
    assert.equal(audit.indexExists, false);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("rome source lint: a prose ref names no codex passage, a codex ref does", () => {
  assert.deepEqual(codexRefsNamed("vegetius.3.8"), ["vegetius.3.8"]);
  assert.deepEqual(codexRefsNamed("see caesar.bg.4.17 for the frontage"), ["caesar.bg.4.17"]);
  assert.deepEqual(codexRefsNamed("rome.md section 5, Vegetius 3.8; Hyginus 49"), []);
  assert.deepEqual(codexRefsNamed("rome.md"), []);
  const list = codexRefList(`${JSON.stringify({ ref: "vegetius.3.8" })}\nnot json\n${JSON.stringify({ ref: "caesar.bg.4.17" })}\n`);
  assert.deepEqual([...list].sort(), ["caesar.bg.4.17", "vegetius.3.8"]);
});

test("rome source lint: the shipped Rome tables hold no unsourced number", () => {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const audit = romeSourceAudit(root);
  assert.deepEqual(audit.violations, []);
  assert.ok(audit.entries > 0, "the Rome tables hold numbers");
  assert.ok(audit.files >= 3, "camp, line and kit are read");
});

test("rome source lint: one table without a ref is refused by path and key", () => {
  const found = romeSourceViolations("content/rome/camp.toml", "[ditch.light]\nwidth_ft = 9\n");
  assert.equal(found.length, 1);
  assert.match(found[0], /content\/rome\/camp\.toml/);
  assert.match(found[0], /ditch\.light/);
});

test("workspace-check runs the Rome source lint", () => {
  const gate = readFileSync(new URL("./workspace-check.mjs", import.meta.url), "utf8");
  assert.match(gate, /romeSourceAudit\(repoDir\)/);
  assert.match(gate, /SKIP rome codex refs/);
  assert.match(gate, /PASS rome sources/);
});

// ---- PW52: the layer rules ------------------------------------------------

test("layer rules: the layer map covers every crate of the workspace (PW52)", () => {
  const crates = fileURLToPath(new URL("../crates/", import.meta.url));
  if (!existsSync(crates)) return;
  const dirs = readdirSync(crates, { withFileTypes: true })
    .filter((e) => e.isDirectory() && existsSync(join(crates, e.name, "Cargo.toml")))
    .map((e) => e.name);
  const placed = new Set(Object.values(LAYERS).flat());
  for (const dir of dirs) {
    // The crate name is in the manifest, not in the folder name.
    const manifest = readFileSync(join(crates, dir, "Cargo.toml"), "utf8");
    const found = /^name\s*=\s*"([^"]+)"/m.exec(manifest);
    assert.ok(found, `${dir} has no crate name`);
    if (!placed.has(found[1])) assert.fail(`${found[1]} is in no layer of the layer map`);
  }
});

test("layer rules: a simulation crate on a presentation or platform edge is named (PW52)", () => {
  const pkg = (name, deps) => ({ name, dependencies: deps.map((d) => (typeof d === "string" ? { name: d, kind: "normal" } : d)) });
  const found = layerEdgeViolations([
    pkg("engine-arena", ["engine-core", "engine-render"]),
    pkg("engine-combat", [{ name: "engine-provider", kind: "normal" }]),
    pkg("engine-ecs", ["engine-core"]),
  ]);
  assert.deepEqual(found, ["engine-arena -> engine-render", "engine-combat -> engine-provider"]);
});

test("layer rules: a dev edge is a test edge and stays legal (PW52)", () => {
  const found = layerEdgeViolations([
    { name: "engine-arena", dependencies: [{ name: "engine-provider", kind: "dev" }] },
    { name: "engine-combat", dependencies: [{ name: "engine-audio", kind: "normal" }] },
  ]);
  assert.deepEqual(found, ["engine-combat -> engine-audio"]);
});

test("layer rules: the live tree has no simulation crate on a presentation edge (PW52)", () => {
  const meta = spawnSync("cargo", ["metadata", "--locked", "--format-version", "1"], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 });
  if (meta.status !== 0) return;
  const packages = JSON.parse(meta.stdout).packages.map((p) => ({
    name: p.name,
    dependencies: p.dependencies.filter((d) => d.path).map((d) => ({ name: d.name, kind: d.kind ?? "normal" })),
  }));
  const found = layerEdgeViolations(packages);
  assert.deepEqual(found, [], "a simulation crate may not import a presentation or a platform crate");
});

test("gpu boundary: a new file outside present/ that names wgpu:: fails (PW52)", () => {
  const files = [
    { rel: "render/src/present/gpu_present/frame.rs", src: "let d: wgpu::Device;" },
    { rel: "render/src/gpu.rs", src: "let d: wgpu::Device;" },
  ];
  const found = gpuBoundaryFindings(files, []);
  assert.deepEqual(found.added, ["render/src/gpu.rs"]);
  assert.deepEqual(found.offenders, ["render/src/gpu.rs"]);
  assert.deepEqual(found.stale, []);
});

test("gpu boundary: a file under present/ that names wgpu:: is legal (PW52)", () => {
  const files = [{ rel: "render/src/present/gpu_present/frame.rs", src: "let d: wgpu::Device;" }];
  const found = gpuBoundaryFindings(files, []);
  assert.deepEqual(found.added, []);
  assert.deepEqual(found.offenders, []);
});

test("gpu boundary: a known file that stops naming wgpu:: is stale, not new (PW52)", () => {
  const files = [{ rel: "render/src/gpu.rs", src: "let d: wgpu::Device;" }];
  const found = gpuBoundaryFindings(files, ["render/src/gpu.rs", "render/src/old.rs"]);
  assert.deepEqual(found.added, []);
  assert.deepEqual(found.stale, ["render/src/old.rs"]);
});

test("gpu boundary: the live tree names no new file outside present/ (PW52)", () => {
  const root = fileURLToPath(new URL("../crates/", import.meta.url));
  if (!existsSync(root)) return;
  const dir = join(root, "render", "src");
  const files = [];
  const walk = (d) => readdirSync(d, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(d, e.name)) : e.name.endsWith(".rs") ? [join(d, e.name)] : []);
  for (const file of walk(dir)) {
    files.push({ rel: `render/src/${file.slice(dir.length + 1).split("\\").join("/")}`, src: readFileSync(file, "utf8") });
  }
  const found = gpuBoundaryFindings(files);
  assert.deepEqual(found.added, [], "no new file outside present/ may name wgpu::");
  assert.equal(found.offenders.length, GPU_BOUNDARY_LEGACY.length, "the live count is the known count");
});

test("io boundary: a simulation crate that reads a file in production code is named (PW52)", () => {
  const files = [{
    rel: "arena/src/lib.rs",
    crate: "engine-arena",
    src: "fn load() { let t = std::fs::read_to_string(\"a\"); }\n",
  }];
  const found = ioBoundaryFindings(files);
  assert.equal(found.length, 1);
  assert.match(found[0], /arena\/src\/lib\.rs/);
  assert.match(found[0], /fs at line 1/);
});

test("io boundary: IO inside a test module does not count (PW52)", () => {
  const src = `fn load() { let t = std::fs::read_to_string("a"); }
#[cfg(test)]
mod tests {
  fn t() { let t = std::fs::read_to_string("b"); }
}
`;
  assert.equal(ioHits(src).length, 1, "only the production read counts");
  assert.equal(ioHits(src)[0].line, 1);
  assert.equal(ioHits(src)[0].kind, "fs");
});

test("io boundary: a crate outside the simulation set may do IO (PW52)", () => {
  const files = [{
    rel: "assets/src/archive.rs",
    crate: "engine-assets",
    src: "fn load() { let t = std::fs::read_to_string(\"a\"); }\n",
  }];
  assert.deepEqual(ioBoundaryFindings(files), []);
});

test("io boundary: the live tree has no IO in a simulation crate outside its tests (PW52)", () => {
  const root = fileURLToPath(new URL("../crates/", import.meta.url));
  if (!existsSync(root)) return;
  const files = [];
  for (const dir of readdirSync(root, { withFileTypes: true })) {
    if (!dir.isDirectory()) continue;
    const crate = `engine-${dir.name}`;
    if (!SIMULATION_CRATES.has(crate)) continue;
    const src = join(root, dir.name, "src");
    if (!existsSync(src)) continue;
    const walk = (d) => readdirSync(d, { withFileTypes: true }).flatMap((e) =>
      e.isDirectory() ? walk(join(d, e.name)) : e.name.endsWith(".rs") ? [join(d, e.name)] : []);
    for (const file of walk(src)) {
      const rel = `${dir.name}/src/${file.slice(src.length + 1).split("\\").join("/")}`;
      if (rel.includes("/bin/")) continue;
      files.push({ rel, crate, src: readFileSync(file, "utf8") });
    }
  }
  const isTestModule = (rel) => {
    const name = rel.split("/").pop().replace(/\.rs$/, "");
    const parts = rel.split("/");
    const parent = parts.slice(0, -1).join("/");
    const lib = join(root, parent, "lib.rs");
    if (!existsSync(lib)) return false;
    return parseTestModules(readFileSync(lib, "utf8")).has(name);
  };
  const found = ioBoundaryFindings(files, SIMULATION_CRATES, isTestModule);
  assert.deepEqual(found, [], "a simulation crate does no IO outside its tests");
});

test("the layer map and the audit agree on the crates (PW52)", () => {
  const gate = readFileSync(new URL("./workspace-check.mjs", import.meta.url), "utf8");
  assert.match(gate, /layerEdgeViolations\(/);
  assert.match(gate, /gpuBoundaryFindings\(/);
  assert.match(gate, /ioBoundaryFindings\(/);
  assert.match(gate, /PASS layer rules: no simulation crate imports a presentation crate/);
  assert.match(gate, /WARN gpu boundary: \$\{gpu\.offenders\.length\} files outside present\/ name wgpu:: \(report-only\)/);
  assert.match(gate, /PASS gpu boundary: no new file outside present\/ names wgpu::/);
  assert.match(gate, /PASS io boundary: no simulation crate does IO outside its tests/);
});

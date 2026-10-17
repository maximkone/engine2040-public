import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { BASELINE, compare, evalCfg, main, scanText, scanTree, snapshot, table, tightened, totals } from "./wasm-gate.mjs";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");

// A repo-shaped temp tree: { "crates/core/src/lib.rs": "..." }.
function tree(files) {
  const root = mkdtempSync(join(tmpdir(), "wasm-gate-"));
  mkdirSync(join(root, "tools"));
  for (const [rel, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, rel)), { recursive: true });
    writeFileSync(join(root, rel), text);
  }
  return root;
}
const run = (root, ...argv) => {
  const lines = [];
  const code = main(argv, { root, out: (l) => lines.push(l) });
  return { code, text: lines.join("\n") };
};
const at = (hits) => hits.map((h) => `${h.line}:${h.id}`);

const EVERY_HAZARD = [
  "fn all() {",
  '    let a = std::fs::read("a");', // 2 fs
  "    let t = Instant::now();", // 3 clock
  "    let w = SystemTime::now();", // 4 clock
  "    std::thread::spawn(|| ());", // 5 thread
  '    let h = thread::Builder::new().name("x".into()).spawn(|| ());', // 6 thread
  '    let s = UdpSocket::bind("0.0.0.0:0");', // 7 socket
  "    let c = TcpStream::connect(addr);", // 8 socket
  "    let e = std::env::current_exe();", // 9 exe
  "    let d = std::env::current_dir();", // 10 exe
  "    let f = pollster::block_on(fut);", // 11 block
  "    device.poll(wgpu::PollType::wait_indefinitely());", // 12 block
  "}",
  "",
].join("\n");

test("a fixture file with every hazard is reported with its line", () => {
  const { hits } = scanText(EVERY_HAZARD);
  assert.deepEqual(at(hits), ["2:fs", "3:clock", "4:clock", "5:thread", "6:thread", "7:socket", "8:socket", "9:exe", "10:exe", "11:block", "12:block"]);
  assert.equal(hits[0].snippet, 'let a = std::fs::read("a");');
});

test("a Builder imported by name is still a thread hazard, counted once", () => {
  const { hits } = scanText('fn f() {\n    Builder::new()\n        .name("x".into())\n        .spawn(|| ());\n    std::thread::Builder::new().spawn(|| ());\n}\n');
  assert.deepEqual(at(hits), ["2:thread", "5:thread"]);
});

test("use std::{fs, io} is an fs hazard, a poll that does not wait is not a block hazard", () => {
  const { hits } = scanText("use std::{\n    fs,\n    io,\n};\nfn f() { device.poll(wgpu::PollType::Poll); }\n");
  assert.deepEqual(at(hits), ["1:fs"]);
});

test("comments and strings are not code", () => {
  const { hits } = scanText('// std::fs::read(path)\n/* Instant::now() */\nfn f() -> &\'static str { "std::fs::read UdpSocket" }\n');
  assert.deepEqual(hits, []);
});

test("test code is skipped: cfg(test) modules, #[test] fns and a file that is cfg(test)", () => {
  const src = [
    "fn real() { std::fs::read(p); }", // 1
    "#[cfg(test)]",
    "mod tests {",
    "    fn t() { std::fs::read(p); Instant::now(); }",
    "}",
    "#[test]",
    "fn alone() { std::fs::read(p); }",
    "",
  ].join("\n");
  assert.deepEqual(at(scanText(src).hits), ["1:fs"]);
  assert.equal(scanText("#![cfg(test)]\nfn helper() { std::fs::read(p); }\n").skipped, true);
});

test("items that are off for the browser are skipped, items that may be on are not", () => {
  const src = [
    '#[cfg(not(target_arch = "wasm32"))]', // 1
    "fn native_only() { std::fs::read(p); }", // 2 skipped
    '#[cfg(not(target_family = "wasm"))]',
    "use std::fs;", // 4 skipped
    "#[cfg(windows)]",
    "fn win() { std::fs::read(p); }", // 6 skipped
    '#[cfg(all(unix, feature = "x"))]',
    "fn nix() { std::fs::read(p); }", // 8 skipped
    '#[cfg(all(not(target_arch = "wasm32"), feature = "x"))]',
    "mod native {",
    "    fn f() { std::fs::read(p); }", // 11 skipped
    "}",
    '#[cfg(feature = "x")]',
    "fn feature_on() { std::fs::read(p); }", // 14 reported: a feature may be on
    '#[cfg(any(test, feature = "x"))]',
    "fn maybe() { std::fs::read(p); }", // 16 reported
    '#[cfg(target_arch = "wasm32")]',
    "fn web() { std::fs::read(p); }", // 18 reported
    '#[cfg(not(any(target_arch = "wasm32", windows)))]',
    "fn neither() { std::fs::read(p); }", // 20 skipped
    "fn plain() { std::fs::read(p); }", // 21 reported
    "",
  ].join("\n");
  assert.deepEqual(at(scanText(src).hits), ["14:fs", "16:fs", "18:fs", "21:fs"]);
});

test("a statement and a use under an off cfg end at their semicolon", () => {
  const src = ['fn f() {', '    #[cfg(not(target_arch = "wasm32"))]', "    let t = Instant::now();", "    let u = Instant::now();", "}", ""].join("\n");
  assert.deepEqual(at(scanText(src).hits), ["4:clock"]);
});

test("evalCfg: known browser values, unknown features", () => {
  assert.equal(evalCfg('target_arch = "wasm32"'), true);
  assert.equal(evalCfg('target_os = "unknown"'), true);
  assert.equal(evalCfg('target_os = "linux"'), false);
  assert.equal(evalCfg("test"), false);
  assert.equal(evalCfg("unix"), false);
  assert.equal(evalCfg('feature = "x"'), null);
  assert.equal(evalCfg('not(feature = "x")'), null);
  assert.equal(evalCfg('all(feature = "x", windows)'), false);
  assert.equal(evalCfg('any(feature = "x", target_family = "wasm")'), true);
  assert.equal(evalCfg("not("), null, "a broken predicate is unknown, never a crash");
});

test("a false cfg on mod x; takes out x.rs and the x/ directory, a false leading #![cfg] the file", () => {
  const root = tree({
    "crates/core/src/lib.rs": '#[cfg(not(target_arch = "wasm32"))]\nmod native;\n#[cfg(test)]\nmod proofs;\nmod web;\nfn f() { std::fs::read(p); }\n',
    "crates/core/src/native.rs": "fn n() { std::fs::read(p); }\n",
    "crates/core/src/native/deep.rs": "fn n() { std::fs::read(p); }\n",
    "crates/core/src/proofs.rs": "fn n() { std::fs::read(p); }\n",
    "crates/core/src/web.rs": "fn w() { std::fs::read(p); }\n",
    "crates/core/src/off.rs": '#![cfg(not(target_arch = "wasm32"))]\nfn n() { std::fs::read(p); }\nfn m() { std::fs::read(p); }\n',
  });
  try {
    assert.deepEqual([...scanTree(root).keys()].sort(), ["crates/core/src/lib.rs", "crates/core/src/web.rs"]);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("tests, benches, examples, tests.rs and src/bin are not web-path code unless a bin is listed", () => {
  const hazard = "fn f() { std::fs::read(p); }\n";
  const root = tree({
    "crates/render/src/lib.rs": hazard,
    "crates/render/src/tests.rs": hazard,
    "crates/render/src/gate_tests.rs": hazard,
    "crates/render/src/tests/a.rs": hazard,
    "crates/render/src/bin/viewer.rs": hazard,
    "crates/render/src/bin/arena/mod.rs": hazard,
    "crates/render/src/main.rs": hazard,
    "crates/render/tests/it/main.rs": hazard,
    "crates/render/benches/b.rs": hazard,
    "crates/render/examples/e.rs": hazard,
    "crates/editor/src/lib.rs": hazard, // not a web-path crate
  });
  try {
    assert.deepEqual([...scanTree(root).keys()], ["crates/render/src/lib.rs"]);
    const listed = [...scanTree(root, { bins: ["crates/render/src/bin/arena"] }).keys()].sort();
    assert.deepEqual(listed, ["crates/render/src/bin/arena/mod.rs", "crates/render/src/lib.rs"]);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("totals and the table count per crate and hazard", () => {
  const root = tree({ "crates/render/src/lib.rs": EVERY_HAZARD, "crates/core/src/lib.rs": "fn f() { std::fs::read(p); }\n" });
  try {
    const files = scanTree(root);
    assert.deepEqual(totals(files).get("render"), { fs: 1, clock: 2, thread: 2, socket: 2, exe: 2, block: 2 });
    assert.deepEqual(totals(files).get("core"), { fs: 1, clock: 0, thread: 0, socket: 0, exe: 0, block: 0 });
    assert.match(table(files), /^render\s+1\s+2\s+2\s+2\s+2\s+2\s+11$/m);
    assert.match(table(files), /^all\s+2\s+2\s+2\s+2\s+2\s+2\s+12$/m);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

const GATE_FILE = "crates/game/src/lib.rs";
const gateTree = (text) => tree({ [GATE_FILE]: text });
const withBaseline = (root, files) => writeFileSync(join(root, BASELINE), JSON.stringify({ version: 1, files }));

test("ratchet: the same counts pass, a raised count fails and names the lines", () => {
  const root = gateTree("fn a() { std::fs::read(p); }\nfn b() { std::fs::read(q); }\n");
  try {
    withBaseline(root, { [GATE_FILE]: { fs: 2 } });
    const same = run(root);
    assert.equal(same.code, 0);
    assert.match(same.text, /RESULT PASS/);
    withBaseline(root, { [GATE_FILE]: { fs: 1 } });
    const raised = run(root);
    assert.equal(raised.code, 1);
    assert.match(raised.text, /\[FAIL\] ratchet count rose crates\/game\/src\/lib\.rs: fs 1 -> 2/);
    assert.match(raised.text, /crates\/game\/src\/lib\.rs:1: fn a\(\)/);
    assert.match(raised.text, /crates\/game\/src\/lib\.rs:2: fn b\(\)/);
    assert.match(raised.text, /RESULT FAIL: 1 ratchet regression/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("ratchet: a hazard kind the baseline does not list fails even when the file total fell", () => {
  const root = gateTree("fn a() { Instant::now(); }\n");
  try {
    withBaseline(root, { [GATE_FILE]: { fs: 3 } });
    const swapped = run(root);
    assert.equal(swapped.code, 1);
    assert.match(swapped.text, /count rose crates\/game\/src\/lib\.rs: clock 0 -> 1/);
    assert.match(swapped.text, /\[IMPROVED\] crates\/game\/src\/lib\.rs: fs 3 -> 0/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("ratchet: a file the baseline does not know fails", () => {
  const root = tree({ [GATE_FILE]: "fn a() {}\n", "crates/net/src/lib.rs": "fn s() { UdpSocket::bind(a); }\n" });
  try {
    withBaseline(root, {});
    const fresh = run(root);
    assert.equal(fresh.code, 1);
    assert.match(fresh.text, /new file with hazards crates\/net\/src\/lib\.rs: socket 0 -> 1/);
    assert.match(fresh.text, /crates\/net\/src\/lib\.rs:1: fn s\(\)/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("ratchet: a lowered count passes and prints the improvement line, --tighten lowers the baseline", () => {
  const root = gateTree("fn a() { std::fs::read(p); }\n");
  try {
    withBaseline(root, { [GATE_FILE]: { fs: 3 } });
    const lowered = run(root);
    assert.equal(lowered.code, 0);
    assert.match(lowered.text, /\[IMPROVED\] crates\/game\/src\/lib\.rs: fs 3 -> 1/);
    assert.match(lowered.text, /\[INFO\] 1 count\(s\) fell below the baseline: run --tighten/);
    const tight = run(root, "--tighten");
    assert.equal(tight.code, 0);
    assert.deepEqual(JSON.parse(readFileSync(join(root, BASELINE), "utf8")).files, { [GATE_FILE]: { fs: 1 } });
    assert.match(run(root).text, /RESULT PASS/);
    assert.doesNotMatch(run(root).text, /IMPROVED/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("tightened drops fixed files and never adds or raises", () => {
  const root = tree({ [GATE_FILE]: "fn a() { std::fs::read(p); Instant::now(); }\n", "crates/net/src/lib.rs": "fn s() { UdpSocket::bind(a); }\n" });
  try {
    const files = scanTree(root);
    const baseline = { files: { [GATE_FILE]: { fs: 5, clock: 1 }, "crates/gone/src/lib.rs": { fs: 2 } } };
    assert.deepEqual(tightened(baseline, files), { [GATE_FILE]: { fs: 1, clock: 1 } });
    assert.deepEqual(compare({ files: snapshot(files) }, files), { regressions: [], improvements: [] });
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("--init writes the baseline once and refuses a second time", () => {
  const root = gateTree("fn a() { std::fs::read(p); }\n");
  try {
    const first = run(root, "--init");
    assert.equal(first.code, 0);
    assert.deepEqual(JSON.parse(readFileSync(join(root, BASELINE), "utf8")).files, { [GATE_FILE]: { fs: 1 } });
    assert.throws(() => run(root, "--init"), /exists/);
    assert.equal(run(root).code, 0);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("a missing baseline and an unknown flag are usage errors, not a pass", () => {
  const root = gateTree("fn a() {}\n");
  try {
    assert.throws(() => run(root), /missing/);
    assert.throws(() => run(root, "--accept"), /unknown argument/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("the committed baseline holds for this tree", () => {
  assert.ok(existsSync(join(REPO, BASELINE)), `${BASELINE} is committed`);
  const result = run(REPO);
  assert.equal(result.code, 0, result.text);
});

test("the clock seam crate hides its native Instant from the browser scan", () => {
  const files = scanTree(REPO);
  assert.equal([...files.keys()].some((f) => f.startsWith("crates/clock/")), false);
});

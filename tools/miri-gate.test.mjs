import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { MAP, WORKFLOW, forbidRollout, main, problems, scanText, scanTree } from "./miri-gate.mjs";

const LINE = "cargo miri test -p engine-core --lib alloc_probe";
const FLOW = `steps:\n  - run: ${LINE}\n`;
const SITES = "fn a() {\n    // SAFETY: fine\n    unsafe { f() };\n}\n";

// A repo-shaped temp tree: { "crates/core/src/lib.rs": "..." }.
function tree(files) {
  const root = mkdtempSync(join(tmpdir(), "miri-gate-"));
  for (const [rel, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, rel)), { recursive: true });
    writeFileSync(join(root, rel), text);
  }
  return root;
}
const crate = (files) => ({ core: files });
const file = (name, text) => ({ file: name, ...scanText(text) });
const map = (sites) => ({ sites });
const run = (root, ...argv) => {
  const lines = [];
  const code = main(argv, { root, out: (l) => lines.push(l) });
  return { code, text: lines.join("\n") };
};

test("scanText counts unsafe blocks, fns, impls, traits and externs with their lines", () => {
  const text = [
    "unsafe impl Send for A {}",
    "unsafe fn f() {}",
    "fn g() { unsafe { f() } }",
    "unsafe trait T {}",
    'unsafe extern "C" { fn h(); }',
  ].join("\n");
  assert.deepEqual(scanText(text).sites.map((s) => s.line), [1, 2, 3, 4, 5]);
});

test("scanText ignores comments, strings, doc text and the lint name", () => {
  const text = [
    "// unsafe { in a comment }",
    '/// unsafe fn in docs',
    'let s = "unsafe { in a string }";',
    '#[expect(unsafe_code, reason = "x")]',
    "#![forbid(unsafe_code)]",
    "let is_unsafe = true; let unsafety = 1;",
  ].join("\n");
  const scan = scanText(text);
  assert.equal(scan.sites.length, 0);
  assert.equal(scan.forbid, true);
});

test("scanText finds cfg(miri) suppressions in all their shapes", () => {
  const text = [
    "#[cfg(miri)]",
    "#[cfg(not(miri))]",
    "#[cfg(all(test, not(miri)))]",
    "#[cfg_attr(miri, ignore)]",
    "if cfg!(miri) {}",
    "#[cfg(test)]",
    "// #[cfg(miri)] in a comment",
  ].join("\n");
  assert.deepEqual(scanText(text).suppressions.map((s) => s.line), [1, 2, 3, 4, 5]);
});

test("problems is empty when source, map and workflow agree", () => {
  const files = crate([file("crates/core/src/a.rs", SITES)]);
  assert.deepEqual(problems(files, map({ "crates/core/src/a.rs": { count: 1, miri: LINE } }), FLOW), []);
  const skipped = map({ "crates/core/src/a.rs": { count: 1, skip: "foreign code" } });
  assert.deepEqual(problems(files, skipped, "steps: []\n"), []);
});

test("a new unsafe site, or a changed count, fails until the map is audited", () => {
  const files = crate([file("crates/core/src/a.rs", SITES)]);
  assert.match(problems(files, map({}), FLOW)[0], /a\.rs: 1 unsafe site\(s\) not in/);
  const two = crate([file("crates/core/src/a.rs", `${SITES}${SITES}`)]);
  assert.match(problems(two, map({ "crates/core/src/a.rs": { count: 1, miri: LINE } }), FLOW)[0], /2 unsafe site\(s\) but .* says 1/);
});

test("a map entry for a file without sites is stale", () => {
  const files = crate([file("crates/core/src/a.rs", "fn a() {}\n")]);
  const found = problems(files, map({ "crates/core/src/a.rs": { count: 1, miri: LINE } }), FLOW);
  assert.match(found[0], /no unsafe site any more/);
  const gone = problems(crate([]), map({ "crates/core/src/gone.rs": { count: 1, miri: LINE } }), FLOW);
  assert.match(gone[0], /gone\.rs has no unsafe site any more/);
});

test("an entry needs exactly one of a miri line and a skip reason", () => {
  const files = crate([file("crates/core/src/a.rs", SITES)]);
  const entry = (e) => problems(files, map({ "crates/core/src/a.rs": { count: 1, ...e } }), FLOW);
  assert.match(entry({})[0], /exactly one of "miri"/);
  assert.match(entry({ miri: LINE, skip: "why" })[0], /exactly one of "miri"/);
  assert.match(entry({ skip: "  " })[0], /exactly one of "miri"/);
  assert.match(entry({ miri: "cargo test -p engine-core" })[0], /must start with "cargo miri test -p "/);
});

test("a miri line the workflow does not run fails", () => {
  const files = crate([file("crates/core/src/a.rs", SITES)]);
  const found = problems(files, map({ "crates/core/src/a.rs": { count: 1, miri: LINE } }), "steps: []\n");
  assert.match(found[0], /miri\.yml does not run `cargo miri test -p engine-core/);
  const commented = problems(files, map({ "crates/core/src/a.rs": { count: 1, miri: LINE } }), `# ${LINE}\n`);
  assert.match(commented[0], /does not run/);
});

test("cfg(miri) in Rust and suppression flags in the workflow fail, comments do not", () => {
  const files = crate([file("crates/core/src/a.rs", `${SITES}#[cfg_attr(miri, ignore)]\nfn t() {}\n`)]);
  const entries = map({ "crates/core/src/a.rs": { count: 1, miri: LINE } });
  assert.match(problems(files, entries, FLOW)[0], /a\.rs:5: cfg\(miri\) is a suppression/);
  const clean = crate([file("crates/core/src/a.rs", SITES)]);
  for (const bad of ["MIRIFLAGS: -Zmiri-disable-isolation", "run: x -Zmiri-ignore-leaks", "continue-on-error: true"]) {
    const found = problems(clean, entries, `${FLOW}${bad}\n`);
    assert.ok(found.some((p) => /suppression, the gate allows none/.test(p)), bad);
  }
  assert.deepEqual(problems(clean, entries, `# no MIRIFLAGS, no continue-on-error\n${FLOW}`), []);
});

test("forbidRollout lists crates with no unsafe site and no forbid yet", () => {
  const crates = {
    clean: [file("crates/clean/src/lib.rs", "fn a() {}\n")],
    forbidden: [file("crates/forbidden/src/lib.rs", "#![forbid(unsafe_code)]\n")],
    risky: [file("crates/risky/src/lib.rs", "fn a() {}\n"), file("crates/risky/src/b.rs", SITES)],
    benchy: [file("crates/benchy/src/lib.rs", "#![forbid(unsafe_code)]\n"), file("crates/benchy/src/main.rs", "fn main() {}\n")],
    tool: [file("crates/tool/build.rs", "fn main() {}\n")],
  };
  assert.deepEqual(forbidRollout(crates), ["clean", "benchy"]);
});

test("scanTree reads crates, skips target folders, and main reports and exits on problems", () => {
  const root = tree({
    "crates/core/Cargo.toml": "[package]\n",
    "crates/core/src/lib.rs": "fn a() {}\n",
    "crates/core/src/a.rs": SITES,
    "crates/core/target/debug/gen.rs": SITES,
    [MAP]: JSON.stringify({ sites: { "crates/core/src/a.rs": { count: 1, miri: LINE } } }),
    [WORKFLOW]: FLOW,
  });
  assert.deepEqual(Object.keys(scanTree(root)), ["core"]);
  const ok = run(root, "--list");
  assert.equal(ok.code, 0, ok.text);
  assert.match(ok.text, /unsafe sites: 1 in 1 files/);
  assert.match(ok.text, /a\.rs: 1 \(lines 3\)/);
  assert.match(ok.text, /forbid-rollout .*: none/);
  writeFileSync(join(root, "crates/core/src/b.rs"), SITES);
  const bad = run(root);
  assert.equal(bad.code, 1);
  assert.match(bad.text, /FAIL crates\/core\/src\/b\.rs: 1 unsafe site/);
  assert.equal(run(tree({})).code, 1);
});

test("this repository passes its own Miri gate", () => {
  const result = run(join(dirname(new URL(import.meta.url).pathname.replace(/^\/(\w:)/, "$1")), ".."));
  assert.equal(result.code, 0, result.text);
});

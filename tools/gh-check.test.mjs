// Run: node --test tools/gh-check.test.mjs
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DEFAULT_BUDGET_S, KEEP_MS, LOCAL_FEATURES, buildBase, classify, closureDirs, crateGraph, idAgeMs, isThrowawayBranch, keepResult, keptResult, localOnly, main, newId, okCmd, parseArgs, raceLost, reuseKey, route, runLocal, runnerOs, testsRan, words } from "./gh-check.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

test("only one plain cargo check, test or clippy line goes to the runner", () => {
  assert.ok(okCmd("cargo test -p engine-net --lib interest_64_room_converges"));
  assert.ok(okCmd("cargo clippy -p engine-input --all-targets -- -D warnings"));
  assert.ok(okCmd("cargo check -p engine-render --features window"));
  for (const bad of ["cargo build -p x", "cargo test -p x && curl evil", "cargo test -p x; ls", "cargo test -p x | cat", "cargo test $(id)", "rm -rf /", "cargo test -p x > f"]) assert.equal(okCmd(bad), false, bad);
});

test("words keep a quoted run as one word", () => {
  assert.deepEqual(words('cargo test -p x -- --exact "a b"'), ["cargo", "test", "-p", "x", "--", "--exact", "a b"]);
});

test("flags come before the cargo line; a long check returns instead of dying at the 120 s tool timeout", () => {
  assert.deepEqual(parseArgs(["cargo", "test", "-p", "x"], {}), { cmd: "cargo test -p x", wait: null, mode: "auto", fresh: false, only: null, budget: DEFAULT_BUDGET_S, error: null });
  assert.ok(DEFAULT_BUDGET_S < 120, "the default budget must end before OpenCode's and Claude Code's 120 s tool timeout");
  assert.equal(parseArgs(["--budget", "840", "cargo", "test", "-p", "x"], {}).budget, 840);
  assert.equal(parseArgs(["--budget", "99999", "cargo", "test"], {}).budget, 1740, "capped under the 30 minute run cap");
  assert.equal(parseArgs(["cargo", "test"], { GH_CHECK_BUDGET: "300" }).budget, 300);
  assert.equal(parseArgs(["--remote-only", "cargo", "test"], {}).mode, "remote");
  assert.equal(parseArgs(["--local", "cargo", "test"], {}).mode, "local");
  assert.equal(parseArgs(["--wait", "wip-mgabc123def", "--budget", "50"], {}).wait, "wip-mgabc123def");
  for (const bad of [["--wait", "x"], ["--wait"], ["--budget", "0", "cargo", "test"], ["--budget", "abc", "cargo", "test"], ["--frobnicate", "cargo", "test"]]) assert.ok(parseArgs(bad, {}).error, bad.join(" "));
  // a flag after the cargo line belongs to cargo
  assert.equal(parseArgs(["cargo", "test", "-p", "x", "--local"], {}).mode, "auto");
  // ...but gh-check's own flags after it are refused before any push, not handed to cargo to fail on a runner
  assert.match(parseArgs(["cargo", "test", "-p", "x", "--lib", "f", "--budget", "840"], {}).error ?? "", /--budget is a gh-check flag: put it before/);
  assert.match(parseArgs(["cargo", "test", "-p", "x", "--fresh"], {}).error ?? "", /--fresh is a gh-check flag/);
  assert.equal(parseArgs(["cargo", "test", "-p", "x", "--", "--budget"], {}).error, null, "after -- it is the test binary's");
});

test("GPU, window and Windows-only lines stay on the PC; plain crate lines go to GitHub", () => {
  for (const line of [
    "cargo test -p engine-net --lib interest_64_room_converges",
    "cargo test -p engine-core --lib",
    "cargo test -p engine-core",
    "cargo clippy -p engine-input --all-targets -- -D warnings",
    "cargo check -p engine-core",
    "cargo test --workspace --all-targets",
  ]) assert.equal(localOnly(line, ROOT), null, line);
  for (const [line, why] of [
    ["cargo test -p engine-render --features window --lib x", /feature window/],
    ["cargo test -p engine-render --features=v1,window", /feature window/],
    ["cargo test -p engine-game -F app --lib", /feature app/],
    ["cargo check -p engine-game --features engine-render/window", /feature engine-render\/window/],
    ["cargo test -p engine-editor --features ui", /feature ui/],
    ["cargo test -p engine-audio --features device", /feature device/],
    ["cargo test -p engine-game --all-features", /--all-features/],
    ["cargo test -p engine-proof --lib gpu_live", /filter gpu_live/],
    ["cargo test -p engine-game --lib -- --exact windowed::tests::x", /filter windowed/],
    ["cargo test -p engine-render --test gpu_frames", /target gpu_frames/],
    ["cargo clippy -p engine-game --all-targets --features app -- -D warnings", /feature app/],
  ]) assert.match(localOnly(line, ROOT) ?? "", why, line);
});

test("a test fn that lives in a file probing for an adapter or Windows is local; its neighbours are not", () => {
  const root = mkdtempSync(join(tmpdir(), "ghcheck-"));
  try {
    mkdirSync(join(root, "crates", "foo", "src"), { recursive: true });
    mkdirSync(join(root, "crates", "bar", "src"), { recursive: true });
    writeFileSync(join(root, "crates", "foo", "Cargo.toml"), '[package]\nname = "engine-foo"\n');
    writeFileSync(join(root, "crates", "bar", "Cargo.toml"), '[package]\nname = "engine-bar"\n');
    writeFileSync(join(root, "crates", "foo", "src", "blit.rs"), '#[test] fn blit_matches_reference() { let a = request_adapter(); if a.is_none() { println!("SKIPPED"); return; } }\n');
    writeFileSync(join(root, "crates", "foo", "src", "math.rs"), "#[test] fn add_is_commutative() {}\n");
    writeFileSync(join(root, "crates", "foo", "src", "crash.rs"), '#[cfg(windows)] mod w { #[test] fn dump_written() {} }\n');
    writeFileSync(join(root, "crates", "bar", "src", "blit.rs"), "// the same words elsewhere do not count for another crate\n#[test] fn blit_matches_reference() {}\n");
    assert.match(localOnly("cargo test -p engine-foo --lib blit_matches", root) ?? "", /crates\/foo\/src\/blit\.rs/);
    assert.match(localOnly("cargo test -p engine-foo --lib dump_written", root) ?? "", /crates\/foo\/src\/crash\.rs/);
    assert.equal(localOnly("cargo test -p engine-foo --lib add_is", root), null);
    assert.equal(localOnly("cargo test -p engine-foo --lib", root), null, "no filter: the whole crate runs, GitHub proves what can run there");
    assert.equal(localOnly("cargo test -p engine-bar --lib blit_matches", root), null);
    assert.equal(localOnly("cargo clippy -p engine-foo --all-targets", root), null, "clippy only compiles");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("LOCAL_FEATURES names every Cargo feature that pulls a GPU, window, device or Windows dependency", () => {
  const heavy = /dep:(?:wgpu|winit|windows|cpal|gilrs|egui-wgpu|egui-winit)\b|\/window\b|\/device\b/;
  let seen = 0;
  for (const dir of readdirSync(join(ROOT, "crates"), { withFileTypes: true }).filter((d) => d.isDirectory())) {
    let text; try { text = readFileSync(join(ROOT, "crates", dir.name, "Cargo.toml"), "utf8"); } catch { continue; }
    const section = /^\[features\]\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(text)?.[1] ?? "";
    for (const m of section.matchAll(/^([\w-]+)\s*=\s*\[(.*)\]/gm)) {
      if (!heavy.test(m[2])) continue;
      seen++;
      assert.ok(LOCAL_FEATURES.has(m[1]), `crates/${dir.name} feature ${m[1]} pulls GPU/window/device code: add it to LOCAL_FEATURES in tools/gh-check.mjs`);
    }
  }
  assert.ok(seen >= 4, `expected the window, app, ui and device features, saw ${seen}`);
});

test("a cargo test that passed with 0 tests proves nothing", () => {
  assert.equal(testsRan("running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 41 filtered out; finished in 0.00s\n"), 0);
  assert.equal(testsRan("test result: ok. 0 passed; 0 failed\ntest result: ok. 3 passed; 0 failed\n"), 3, "doc tests with 0 do not hide the lib tests");
  assert.equal(testsRan("test result: FAILED. 2 passed; 1 failed\n"), 2);
  assert.equal(testsRan("Finished dev profile\n"), null, "no result line (check, clippy, --no-run)");
});

test("a job name carries its age so a leaked branch can be swept", () => {
  const t = Date.parse("2026-10-17T10:00:00Z");
  const id = newId(t);
  assert.match(id, /^wip-[0-9a-z]{11}$/);
  assert.equal(idAgeMs(id, t + 4 * 3_600_000), 4 * 3_600_000);
  assert.equal(idAgeMs("wip-zz", t), 0, "an id of another shape is never swept");
  assert.equal(idAgeMs("main", t), 0);
});

test("every throwaway branch family is sweepable, not only wip-", () => {
  // The 2026-10-22 branch pile: 41 branches on the mirror, because a family whose name carries no
  // timestamp answered idAgeMs 0 and read as "not stale". These are the families the tools push.
  for (const name of ["wip-mvju6qvvn1r", "web-build-mvcsnbgjuc2", "docs-probe-mvcky5cv", "golden-matrix-dsn2-mvcofsdk", "wip-am24-after"]) {
    assert.ok(isThrowawayBranch(name), `${name} is a branch a check tool pushed`);
  }
  // Real branches are never swept, however old they look.
  for (const name of ["main", "master", "recovery/lane-work-2026-10-22", "feature-x"]) {
    assert.ok(!isThrowawayBranch(name), `${name} is a real branch and stays`);
  }
  // A family that does not encode its age is still datable by the commit, which is what the sweeper reads.
  assert.equal(idAgeMs("wip-am24-after"), 0, "no timestamp in the name");
  assert.ok(isThrowawayBranch("wip-am24-after"), "so the commit date decides it");
});

const fakes = (remoteResults, localCode = 0) => {
  const calls = { remote: 0, local: [], said: [], os: [] };
  return {
    calls,
    remote: async (cmd, os) => { calls.os.push(os); return remoteResults[Math.min(calls.remote++, remoteResults.length - 1)]; },
    local: (cmd) => { calls.local.push(cmd); return localCode; },
    say: (s) => calls.said.push(s),
  };
};
const CMD = "cargo test -p engine-net --lib interest";

test("route: a pass or a fail on the runner is final", async () => {
  let f = fakes([{ code: 0 }]);
  assert.equal(await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say }), 0);
  assert.deepEqual([f.calls.remote, f.calls.local.length], [1, 0]);
  f = fakes([{ code: 1 }]);
  assert.equal(await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say }), 1, "a red stays red: no second chance on the PC");
  assert.deepEqual([f.calls.remote, f.calls.local.length], [1, 0]);
});

test("route: exit 2 twice runs the same line on the PC; one retry may rescue it", async () => {
  let f = fakes([{ code: 2, retry: true, why: "push failed" }], 0);
  assert.equal(await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say }), 0);
  assert.deepEqual([f.calls.remote, f.calls.local], [2, [CMD]]);
  assert.ok(f.calls.said.some((s) => /trying once more/.test(s)));
  f = fakes([{ code: 2, retry: true }], 1);
  assert.equal(await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say }), 1, "the PC's own result is the answer");
  f = fakes([{ code: 2, retry: true }, { code: 0 }]);
  assert.equal(await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say }), 0);
  assert.deepEqual([f.calls.remote, f.calls.local.length], [2, 0]);
  f = fakes([{ code: 2, retry: false, why: "no result in 30 minutes" }]);
  await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say });
  assert.deepEqual([f.calls.remote, f.calls.local.length], [1, 1], "a timeout is not retried for another 30 minutes");
});

test("route: GPU and window tests never touch GitHub; feature builds go to the Windows runner, the rest to Linux", async () => {
  const line = "cargo test -p engine-proof --lib gpu_live";
  const f = fakes([{ code: 0 }], 0);
  assert.equal(await route({ cmd: line, remote: f.remote, local: f.local, say: f.say }), 0);
  assert.deepEqual([f.calls.remote, f.calls.local], [0, [line]]);
  const w = fakes([{ code: 0 }], 0);
  assert.equal(await route({ cmd: "cargo clippy -p engine-game --all-targets --features app -- -D warnings", remote: w.remote, local: w.local, say: w.say }), 0);
  assert.deepEqual([w.calls.os, w.calls.local.length], [["windows-latest"], 0]);
  assert.ok(w.calls.said.some((s) => /Windows runner: feature app/.test(s)));
  const l = fakes([{ code: 0 }], 0);
  await route({ cmd: CMD, remote: l.remote, local: l.local, say: l.say });
  assert.deepEqual(l.calls.os, ["ubuntu-latest"]);
  const g = fakes([{ code: 0 }], 0);
  await route({ cmd: CMD, mode: "local", remote: g.remote, local: g.local, say: g.say });
  assert.deepEqual([g.calls.remote, g.calls.local], [0, [CMD]], "--local skips GitHub");
});

test("route: a pass with 0 tests is rerun on the PC, or refused under --remote-only", async () => {
  let f = fakes([{ code: 0, noTests: true }], 0);
  assert.equal(await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say }), 0);
  assert.deepEqual(f.calls.local, [CMD]);
  assert.ok(f.calls.said.some((s) => /0 tests ran/.test(s)));
  f = fakes([{ code: 0, noTests: true }]);
  assert.equal(await route({ cmd: CMD, mode: "remote", remote: f.remote, local: f.local, say: f.say }), 2);
  assert.equal(f.calls.local.length, 0);
});

test("route: --remote-only never falls back, and a check still running is neither a pass nor a fall back", async () => {
  let f = fakes([{ code: 2, retry: true }]);
  assert.equal(await route({ cmd: CMD, mode: "remote", remote: f.remote, local: f.local, say: f.say }), 2);
  assert.deepEqual([f.calls.remote, f.calls.local.length], [2, 0]);
  f = fakes([{ code: 3 }]);
  assert.equal(await route({ cmd: CMD, remote: f.remote, local: f.local, say: f.say }), 3);
  assert.deepEqual([f.calls.remote, f.calls.local.length], [1, 0], "exit 3 is not retried and not run twice");
});

test("a workspace-wide line never runs on the PC, even as the fallback", () => {
  const said = [];
  for (const line of ["cargo test --workspace --all-targets", "cargo clippy --all -- -D warnings"]) assert.equal(runLocal(line, (s) => said.push(s)), 2, line);
  assert.equal(said.length, 2);
  assert.match(said[0], /workspace-wide/);
});

test("a fetch that lost the mirror race is retried; a real failure is not", () => {
  assert.ok(raceLost("fetching ref refs/remotes/origin/main failed: incorrect old value provided"));
  assert.ok(raceLost("cannot lock ref 'refs/remotes/origin/main'"));
  assert.ok(raceLost("failed to update ref 'refs/remotes/origin/main'"));
  assert.equal(raceLost("could not read Username for 'https://github.com': terminal prompts disabled"), false);
  assert.equal(raceLost("not a git repository"), false);
});

test("main refuses what is not one plain cargo line, and an unknown job, before any network call", async () => {
  const log = console.log; console.log = () => {};
  try {
    assert.equal(await main(["rm", "-rf", "/"], {}), 2);
    assert.equal(await main(["cargo", "build", "-p", "x"], {}), 2);
    assert.equal(await main(["cargo", "test", "-p", "x", "&&", "id"], {}), 2);
    assert.equal(await main(["--wait", "nonsense"], {}), 2);
    assert.equal(await main(["--wait", "wip-zzzzzzzzzzz"], {}), 2, "no such job file");
  } finally { console.log = log; }
});

test("classify: compiling a window|app|ui|device feature is Windows-runner work; GPU, window and sound tests are the PC's", () => {
  assert.equal(classify("cargo clippy -p engine-game --all-targets --features app -- -D warnings", ROOT).gpu, null, "clippy only compiles");
  assert.match(classify("cargo clippy -p engine-game --features app", ROOT).win ?? "", /feature app/);
  assert.equal(classify("cargo test -p engine-game --features app --no-run", ROOT).gpu, null, "--no-run only compiles");
  assert.match(classify("cargo test -p engine-proof --lib gpu_live", ROOT).gpu ?? "", /filter gpu_live/);
  assert.match(classify("cargo test -p engine-audio --features device", ROOT).gpu ?? "", /sound device/);
  assert.equal(classify("cargo check -p engine-audio --features device", ROOT).gpu, null, "a device build needs no device");
  assert.equal(runnerOs("cargo check -p engine-core", ROOT), "ubuntu-latest");
  assert.equal(runnerOs("cargo test -p engine-game --all-features --no-run", ROOT), "windows-latest");
  const root = mkdtempSync(join(tmpdir(), "ghcheck-"));
  try {
    mkdirSync(join(root, "crates", "foo", "src"), { recursive: true });
    writeFileSync(join(root, "crates", "foo", "Cargo.toml"), '[package]\nname = "engine-foo"\n');
    writeFileSync(join(root, "crates", "foo", "src", "blit.rs"), '#[test] fn blit_matches_reference() { let a = request_adapter(); }\n');
    writeFileSync(join(root, "crates", "foo", "src", "crash.rs"), '#[cfg(windows)] mod w { #[test] fn dump_written() {} }\n');
    assert.deepEqual([classify("cargo test -p engine-foo --lib dump_written", root).gpu, runnerOs("cargo test -p engine-foo --lib dump_written", root)], [null, "windows-latest"], "Windows-only code runs on the Windows runner");
    assert.match(classify("cargo test -p engine-foo --lib blit_matches", root).gpu ?? "", /blit\.rs/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

// A tiny workspace: foo -> bar (normal, table form) -> qux; foo -dev-> baz -dev-> zap (a dev dep of a dep is not built);
// foo -> renamed (package = "engine-ren"); lone stands apart.
const tinyWorkspace = () => {
  const root = mkdtempSync(join(tmpdir(), "ghreuse-"));
  const crate = (dir, toml, src = "pub fn f() {}\n") => {
    mkdirSync(join(root, "crates", dir, "src"), { recursive: true });
    writeFileSync(join(root, "crates", dir, "Cargo.toml"), toml);
    writeFileSync(join(root, "crates", dir, "src", "lib.rs"), src);
  };
  writeFileSync(join(root, "Cargo.toml"), '[workspace]\nmembers = ["crates/*"]\n');
  writeFileSync(join(root, "Cargo.lock"), "# lock\n");
  mkdirSync(join(root, "content"), { recursive: true }); writeFileSync(join(root, "content", "a.toml"), "a = 1\n");
  mkdirSync(join(root, "docs"), { recursive: true }); writeFileSync(join(root, "docs", "x.md"), "x\n"); writeFileSync(join(root, "docs", "y.md"), "y\n");
  crate("foo", '[package]\nname = "engine-foo"\n\n[dependencies.engine-bar]\npath = "../bar"\n\n[dependencies]\nren = { package = "engine-ren", path = "../ren" }\n\n[dev-dependencies]\nengine-baz.workspace = true\n', 'const X: &str = include_str!("../../../docs/x.md");\n');
  crate("bar", '[package]\nname = "engine-bar"\n\n[target.\'cfg(windows)\'.dependencies]\nengine-qux = { path = "../qux" }\n');
  crate("qux", '[package]\nname = "engine-qux"\n');
  crate("baz", '[package]\nname = "engine-baz"\n\n[dev-dependencies]\nengine-zap = { path = "../zap" }\n');
  crate("zap", '[package]\nname = "engine-zap"\n');
  crate("ren", '[package]\nname = "engine-ren"\n');
  crate("lone", '[package]\nname = "engine-lone"\n');
  return root;
};
const touch = (file, text) => writeFileSync(file, text); // a new size is a new key, whatever the clock does

test("reuse: a line is keyed by the crates it builds, not by the rest of the tree", () => {
  const root = tinyWorkspace();
  try {
    assert.deepEqual(closureDirs(["engine-foo"], crateGraph(root)), ["crates/bar", "crates/baz", "crates/foo", "crates/qux", "crates/ren"]);
    assert.equal(closureDirs(["engine-nope"], crateGraph(root)), null);
    const cmd = "cargo test -p engine-foo --lib";
    const k = reuseKey(cmd, "pc", root);
    assert.match(k ?? "", /^[0-9a-f]{32}$/);
    assert.equal(reuseKey(cmd, "pc", root), k, "same inputs, same key");
    assert.notEqual(reuseKey(cmd, "ubuntu-latest", root), k, "the runner is part of the key");
    assert.notEqual(reuseKey("cargo test -p engine-foo --lib x", "pc", root), k, "the line is part of the key");
    touch(join(root, "crates", "lone", "src", "lib.rs"), "pub fn g() { /* another crate */ }\n");
    touch(join(root, "docs", "y.md"), "y changed\n");
    touch(join(root, "crates", "zap", "src", "lib.rs"), "pub fn z() { /* a dev dep of a dep */ }\n");
    assert.equal(reuseKey(cmd, "pc", root), k, "edits outside what the line builds keep the key");
    for (const f of [["crates", "qux", "src", "lib.rs"], ["docs", "x.md"], ["content", "a.toml"], ["Cargo.lock"]]) {
      const before = reuseKey(cmd, "pc", root);
      touch(join(root, ...f), `changed ${f.join("/")} and longer\n`);
      assert.notEqual(reuseKey(cmd, "pc", root), before, `${f.join("/")} is an input of the line`);
    }
    assert.equal(reuseKey("cargo test --workspace", "pc", root), null);
    assert.equal(reuseKey("cargo test --lib", "pc", root), null, "no -p: the root package, too wide to key");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("reuse: a kept pass answers the same line at once, a fail only for half an hour, and --fresh runs it again", () => {
  const root = tinyWorkspace();
  const cmd = `cargo test -p engine-foo --lib reuse_probe_${process.pid}`;
  const key = reuseKey(cmd, "pc", root);
  try {
    keepResult(key, { code: 0, where: "pc", cmd });
    const said = [];
    assert.equal(runLocal(cmd, (s) => said.push(s), { root }), 0);
    assert.ok(said.some((s) => /^REUSED PASS/.test(s)), said.join("\n"));
    assert.ok(!said.some((s) => /^\[local\]/.test(s)), "nothing was built");
    assert.equal(keptResult(key, Date.now() + KEEP_MS[0] + 1), null, "a pass is kept a day");
    keepResult(key, { code: 1, where: "pc", cmd });
    assert.equal(keptResult(key)?.code, 1);
    assert.equal(keptResult(key, Date.now() + KEEP_MS[1] + 1), null, "a fail is kept half an hour");
    keepResult(key, { code: 0, noTests: true, where: "pc", cmd });
    assert.equal(keptResult(key)?.code, 1, "a vacuous pass is never kept");
    assert.equal(parseArgs(["--fresh", "cargo", "check", "-p", "x"]).fresh, true);
  } finally {
    rmSync(join(ROOT, "target", "gh-check", "results", `${key}.json`), { force: true });
    rmSync(root, { recursive: true, force: true });
  }
});

test("a failed push names its reason, not git's 'To <url>' header", async () => {
  const { whyLine } = await import("./gh-check.mjs");
  assert.equal(whyLine({ stderr: "To https://github.com/x.git\n ! [remote rejected] wip-1 -> wip-1 (failed)\nerror: failed to push some refs\n" }), "! [remote rejected] wip-1 -> wip-1 (failed)");
  assert.equal(whyLine({ message: "plain" }), "plain");
});

test("--only takes comma-separated paths (backslashes become slashes) and refuses an empty list", () => {
  assert.deepEqual(parseArgs(["--only", "crates\\game\\src\\a.rs, crates/game/src/b.rs", "cargo", "test", "-p", "x"]).only, ["crates/game/src/a.rs", "crates/game/src/b.rs"]);
  assert.match(parseArgs(["--only", "", "cargo", "test", "-p", "x"]).error ?? "", /--only needs paths/);
  assert.equal(parseArgs(["cargo", "test", "-p", "x"]).only, null);
});

// A new HEAD's base is built on the nearest cached one (gh-check.mjs incrementalBase) or whole (wholeBase). Both sit behind the exported
// buildBase, so this test drives buildBase and gh-check.mjs needs no export for it. tools/public-export.mjs is denied, so the public
// checkout has no exporter: there the test is skipped (its import would fail), not red.
const exporterHere = existsSync(join(ROOT, "tools", "public-export.mjs"));
test("a new HEAD's snapshot built on the old base is the tree a whole build gives: one tree id, denied paths out, nothing missing", { skip: exporterHere ? false : "no exporter in the public checkout" }, async () => {
  const root = mkdtempSync(join(tmpdir(), "ghbase-"));
  const repo = join(root, "repo");
  try {
    mkdirSync(repo);
    const git = (...args) => execFileSync("git", ["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false", "-c", "user.name=ghcheck", "-c", "user.email=ghcheck@example.invalid", ...args], { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
    const put = (rel, text) => { mkdirSync(dirname(join(repo, rel)), { recursive: true }); writeFileSync(join(repo, rel), text); };
    const drop = (rel) => rmSync(join(repo, rel), { force: true });
    const commit = (msg) => { git("add", "-A"); git("commit", "-q", "-m", msg); return git("rev-parse", "HEAD"); };
    git("init", "-q");

    // commit A: a text rule with a CRLF file, a denied roster, a file to rename, one to rename into a denied folder, one to delete, a tools README with one script
    put(".gitattributes", "*.txt text\n");
    put("README.md", "# fixture\n");
    put("src/lib.txt", "one\ntwo\n");
    put("crlf.txt", "alpha\r\nbeta\r\n");
    put("docs/guide.md", "guide v1\n");
    put("docs/old-name.md", "renamed later\n");
    put("docs/moved-out.md", "moved into team later\n");
    put("docs/gone.md", "deleted later\n");
    put("team/roster.md", "private roster\n");
    put("tools/README.md", "# tools\n1 live scripts\n- `tools/tool-a.mjs` one\n");
    put("tools/tool-a.mjs", "export const a = 1;\n");
    const fullA = commit("A");

    // commit B: a change, a rename, a rename into a denied folder, a delete, new files, denied files added and changed, a CRLF change, a new script
    put("docs/guide.md", "guide v2\n");
    drop("docs/old-name.md"); put("docs/new-name.md", "renamed later\n");
    drop("docs/moved-out.md"); put("team/moved.md", "moved into team later\n");
    drop("docs/gone.md");
    put("team/roster.md", "private roster v2\n");
    put("AGENTS.md", "internal\n");
    put("src/added.mjs", "export const added = true;\n");
    put("crlf.txt", "alpha\r\ngamma\r\n");
    put("tools/tool-b.mjs", "export const b = 2;\n");
    const fullB = commit("B");

    // buildBase(head, full, ctx, say): B finds A's base in `work` and replays the diff; `fresh` has no base, so B is built whole
    const ctx = (work) => ({ root: repo, work, gd: join(repo, ".git") });
    const base = async (full, work) => { const said = []; const tree = await buildBase(full.slice(0, 9), full, ctx(work), (s) => said.push(s)); return { tree, said }; };
    const work = join(root, "work"), fresh = join(root, "fresh");
    const a = await base(fullA, work);
    const inc = await base(fullB, work);
    const whole = await base(fullB, fresh);
    assert.deepEqual(a.said, [], "A has no earlier base: a whole build");
    assert.equal(inc.said.length, 1, `B is built on A's base: ${inc.said.join(" | ")}`);
    assert.match(inc.said[0], /changed paths on top of /);
    assert.deepEqual(whole.said, []);

    const ls = (tree) => git("ls-tree", "-r", tree);
    const names = (tree) => ls(tree).split("\n").map((l) => l.split("\t")[1]);
    assert.equal(ls(inc.tree), ls(whole.tree), "the same paths, modes and blobs");
    assert.equal(inc.tree, whole.tree, "one git tree id");
    assert.deepEqual(names(inc.tree).sort(), [".gitattributes", "README.md", "crlf.txt", "docs/guide.md", "docs/new-name.md", "src/added.mjs", "src/lib.txt", "tools/README.md", "tools/tool-a.mjs", "tools/tool-b.mjs"], "denied and deleted or renamed-away paths are out, nothing is missing");
    assert.equal(git("show", `${inc.tree}:crlf.txt`).includes("\r"), false, "the text rule stores the CRLF file as LF");
    assert.ok(!names(a.tree).includes("team/roster.md"), "a denied path is out of the first base too");
  } finally { rmSync(root, { recursive: true, force: true, maxRetries: 3 }); }
});

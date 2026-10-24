// Run: node --test tools/gh-check.test.mjs
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DEFAULT_BUDGET_S, KEEP_MS, LOCAL_FEATURES, afterRemote, buildBase, changesOf, classify, closureDirs, commandLines, compileRedOf, crateGraph, declaredModFiles, haveParent, headRedFor, idAgeMs, isMutants, isThrowawayBranch, keepResult, keptResult, localOnly, main, masterRedLine, modFileOf, newId, noteHeadRed, okCmd, okMutants, parseArgs, pickedText, preflightOnly, pushLaps, knownRedsNote, pushLock, pushUnlock, pushLockPath, pushNote, raceLost, rateLimited, reuseKey, route, runFromList, runLocal, runnerOs, testsRan, withNoDeps, withRateRetry, words } from "./gh-check.mjs";
import { changeOf } from "./commit-mine.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

test("a lost pack race logs one short line, a network error keeps its reason", () => {
  const pack = "remote: error: unable to write file /data/repositories/x/objects/pack/pack-abc.pack: No such file or directory";
  assert.match(pushNote(pack), /lost the pack race/);
  assert.doesNotMatch(pushNote(pack), /No such file/, "the long GitHub path stays out of the retry line");
  assert.match(pushNote("fatal: unable to access 'https://github.com/x': Could not resolve host"), /Could not resolve host/);
});

test("only one plain cargo check, test or clippy line goes to the runner", () => {
  assert.ok(okCmd("cargo test -p engine-net --lib interest_64_room_converges"));
  assert.ok(okCmd("cargo clippy -p engine-input --all-targets -- -D warnings"));
  assert.ok(okCmd("cargo check -p engine-render --features window"));
  for (const bad of ["cargo build -p x", "cargo test -p x && curl evil", "cargo test -p x; ls", "cargo test -p x | cat", "cargo test $(id)", "rm -rf /", "cargo test -p x > f"]) assert.equal(okCmd(bad), false, bad);
});

test("cargo mutants: one crate and one slice (--shard k/n or -f) goes to the runner; the whole workspace, no slice, a shell line or an unlisted flag never does", () => {
  for (const ok of ["cargo mutants -p engine-core --shard 1/8 --timeout 120", "cargo mutants -p engine-core --shard 0/20", "cargo mutants --package engine-combat --shard=19/20", "cargo mutants -p engine-core -f crates/core/src/lib.rs", "cargo mutants -p engine-core --file crates/core/src/x.rs -t 60"]) assert.equal(okCmd(ok), true, ok);
  assert.equal(isMutants("cargo mutants -p engine-core --shard 1/8"), true);
  assert.equal(isMutants("cargo test -p engine-core"), false);
  for (const bad of ["cargo mutants", "cargo mutants --shard 1/8", "cargo mutants -p engine-core", "cargo mutants --workspace", "cargo mutants --all -p engine-core --shard 1/8", "cargo mutants -p engine-core --workspace --shard 1/8", "cargo mutants -p engine-core -p engine-combat --shard 1/8", "cargo mutants -p engine-core --shard 9/8", "cargo mutants -p engine-core --shard 8/8", "cargo mutants -p engine-core --shard 1", "cargo mutants -p engine-core -f ../../x.rs", "cargo mutants -p engine-core -f crates/core/src/*.rs", "cargo mutants -p engine-core --shard 1/8 --output /tmp/x", "cargo mutants -p engine-core --shard 1/8 --in-place", "cargo mutants -p engine-core --shard 1/8 -- --nocapture", "cargo mutants -p engine-core --shard 1/8 --timeout 0", "cargo mutants -p engine-core --shard 1/8; rm -rf x", "cargo mutants -p engine-core --shard 1/8 | tee log", "cargo mutants -p engine-core --shard $(id)"]) assert.equal(okCmd(bad), false, bad);
  assert.equal(okMutants("cargo mutants -p engine-core --shard 1/8 --timeout 120"), true);
});

test("cargo mutants never runs on this PC: --local refuses it, and a GitHub failure never falls back to the PC", async () => {
  const mutants = "cargo mutants -p engine-core --shard 1/8";
  const said = [];
  assert.equal(runLocal(mutants, (s) => said.push(s)), 2, "refused before any build starts");
  assert.match(said.join("\n"), /never runs on this PC/);
  let ranHere = false;
  const local = async () => { ranHere = true; return 0; };
  assert.equal(await afterRemote({ res: { code: 2, retry: false }, cmd: mutants, mode: "auto", local, say: () => {} }), 2);
  assert.equal(ranHere, false, "no PC fallback for a mutants line");
  const log = console.log; console.log = () => {};
  try { assert.equal(await main(["--local", "cargo", "mutants", "-p", "engine-core", "--shard", "1/8"], {}), 2); } finally { console.log = log; }
});

test("words keep a quoted run as one word", () => {
  assert.deepEqual(words('cargo test -p x -- --exact "a b"'), ["cargo", "test", "-p", "x", "--", "--exact", "a b"]);
});

test("flags come before the cargo line; a long check returns instead of dying at the 120 s tool timeout", () => {
  assert.deepEqual(parseArgs(["cargo", "test", "-p", "x"], {}), { cmd: "cargo test -p x", wait: null, mode: "auto", fresh: false, only: null, at: null, budget: DEFAULT_BUDGET_S, error: null });
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

test("--help prints the flags once and exits 0, so no helper has to read this tool", async () => {
  const said = [];
  const log = console.log;
  console.log = (s) => said.push(String(s));
  try {
    assert.equal(await main(["--help"]), 0);
    assert.equal(await main(["-h"]), 0);
    assert.equal(await main(["help"]), 0);
  } finally { console.log = log; }
  const text = said[0];
  for (const flag of ["--budget", "--only", "--fresh", "--remote-only", "--local", "--wait"]) assert.match(text, new RegExp(flag));
  assert.match(said[2], /Exited with code 1/, "the harness's flat exit code is named");
  assert.match(said[0], /Exit: 0 pass, 1 fail, 2 could not run, 3 still running/);
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

test("route: 0 tests on a feature-gated crate reruns once with its test feature, on the right runner", async () => {
  const { withTestFeatures } = await import("./gh-check.mjs");
  assert.equal(withTestFeatures("cargo test -p engine-physfx --lib hit"), "cargo test -p engine-physfx --lib hit --features rapier");
  assert.equal(withTestFeatures("cargo test -p engine-game --lib spear_look -- --nocapture"), "cargo test -p engine-game --lib spear_look --features app -- --nocapture");
  assert.equal(withTestFeatures("cargo test -p engine-physfx --features rapier --lib"), null, "a line that names features is left alone");
  assert.equal(withTestFeatures("cargo test -p engine-net --lib x"), null, "only the listed crates");
  assert.equal(withTestFeatures("cargo check -p engine-physfx"), null, "only test lines");
  const f = fakes([{ code: 0, noTests: true }, { code: 0 }]);
  const line = "cargo test -p engine-game --lib spear_look";
  assert.equal(await route({ cmd: line, remote: f.remote, local: f.local, say: f.say }), 0);
  assert.deepEqual([f.calls.remote, f.calls.local.length], [2, 0], "the rerun is remote, not the PC");
  assert.deepEqual(f.calls.os, ["ubuntu-latest", "windows-latest"], "app goes to the Windows runner");
  assert.ok(f.calls.said.some((s) => /keeps its tests behind a feature/.test(s)));
  const g = fakes([{ code: 0, noTests: true }], 0);
  await route({ cmd: CMD, remote: g.remote, local: g.local, say: g.say });
  assert.deepEqual([g.calls.remote, g.calls.local], [1, [CMD]], "other crates keep the PC fallback");
});

test("a failed tail names the files its errors point at", async () => {
  const { errorFiles } = await import("./gh-check.mjs");
  const tail = [
    "error[E0432]: unresolved import `crate::vfx_soft`",
    "  --> crates\\render\\src\\particle_draw_gpu.rs:12:12",
    "error[E0308]: mismatched types",
    "   --> crates/endpoint/src/packs/view/scene.rs:161:13",
    "  --> crates\\render\\src\\particle_draw_gpu.rs:40:1",
  ].join("\n");
  assert.deepEqual(errorFiles(tail), ["crates/render/src/particle_draw_gpu.rs", "crates/endpoint/src/packs/view/scene.rs"]);
  assert.deepEqual(errorFiles("test result: ok. 3 passed"), []);
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
  const spaced = parseArgs(["--only", "a.rs", "b.rs#2", "--budget", "60", "cargo", "test", "-p", "x"]);
  assert.deepEqual([spaced.only, spaced.budget, spaced.cmd, spaced.error], [["a.rs", "b.rs#2"], 60, "cargo test -p x", null], "space-separated paths up to the next flag or cargo");
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

test("a rate-limited GitHub call waits and tries again; any other error and the last limited try still throw", async () => {
  const limit = Object.assign(new Error("gh failed"), { stderr: "HTTP 403: API rate limit exceeded for user ID 1." });
  assert.ok(rateLimited(limit) && rateLimited("You have exceeded a secondary rate limit") && !rateLimited(new Error("HTTP 404: Not Found")));
  const waited = [];
  const wait = async (ms) => { waited.push(ms); };
  let tries = 0;
  assert.equal(await withRateRetry(async () => { tries++; if (tries < 3) throw limit; return "sent"; }, [1, 2, 3], wait), "sent");
  assert.deepEqual(waited, [1, 2], "one wait per limited try, in order");
  await assert.rejects(withRateRetry(async () => { throw new Error("HTTP 404: Not Found"); }, [1], wait), /404/, "a real error is not retried");
  await assert.rejects(withRateRetry(async () => { throw limit; }, [5], wait), /gh failed/, "the limit still holding after the last wait throws");
  assert.deepEqual(waited, [1, 2, 5]);
});
test("one shared run list answers each waiting call by its own branch", () => {
  const runs = [{ headBranch: "wip-b", databaseId: 2, status: "in_progress" }, { headBranch: "wip-a", databaseId: 1, status: "completed", conclusion: "success" }];
  assert.equal(runFromList(runs, "wip-a").databaseId, 1);
  assert.equal(runFromList(runs, "wip-b").status, "in_progress");
  assert.equal(runFromList(runs, "wip-c"), null, "a run not listed yet is not started, not lost");
});
test("--only <path>#N proves HEAD plus that hunk alone: another writer's hunk in the same file stays out", () => {
  const repo = mkdtempSync(join(tmpdir(), "ghpick-"));
  const git = (...args) => execFileSync("git", ["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false", "-c", "user.name=ghcheck", "-c", "user.email=ghcheck@example.invalid", ...args], { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  try {
    git("init", "-q");
    const lines = Array.from({ length: 10 }, (_, i) => `line ${i + 1}`);
    writeFileSync(join(repo, "a.rs"), `${lines.join("\n")}\n`);
    git("add", "a.rs"); git("commit", "-q", "-m", "base");
    const dirty = [...lines]; dirty[1] = "theirs"; dirty[7] = "mine";
    writeFileSync(join(repo, "a.rs"), `${dirty.join("\n")}\n`);
    const want = [...lines]; want[7] = "mine";
    assert.equal(pickedText("a.rs", [2], repo), `${want.join("\n")}\n`, "hunk 2 (line 8) only, trailing newline kept");
    assert.throws(() => pickedText("a.rs", [3], repo), /has 2 hunks/);
  } finally { rmSync(repo, { recursive: true, force: true, maxRetries: 3 }); }
});
test("--at <sha> builds one commit alone: a sha, never with --only, and never after the cargo line", () => {
  const a = parseArgs(["--at", "e2f172fcf", "cargo", "test", "-p", "engine-game", "--lib", "x"]);
  assert.equal(a.error, null);
  assert.equal(a.at, "e2f172fcf");
  assert.equal(a.cmd, "cargo test -p engine-game --lib x");
  assert.match(parseArgs(["--at", "HEAD~3", "cargo", "check", "-p", "x"]).error, /commit sha/);
  assert.match(parseArgs(["--at", "e2f172fcf", "--only", "a.rs", "cargo", "check", "-p", "x"]).error, /no --only/);
  assert.match(parseArgs(["cargo", "test", "-p", "x", "--at", "e2f172fcf"]).error, /--at is a gh-check flag/);
});
test("the command step is found when GitHub serves the job without step names", () => {
  const unknown = [
    "run\tUNKNOWN STEP\t2026-10-24T16:58:57Z ##[group]Run actions/checkout@v5",
    "run\tUNKNOWN STEP\t2026-10-24T16:59:32Z ##[group]Run case \"$CMD\" in",
    "run\tUNKNOWN STEP\t2026-10-24T17:04:03Z test result: ok. 3 passed; 0 failed",
    "run\tUNKNOWN STEP\t2026-10-24T17:04:05Z Post job cleanup.",
  ].join("\n");
  assert.equal(commandLines(unknown).length, 2);
  assert.match(commandLines(unknown)[1], /3 passed/);
  const named = "run\tset up\tx\nrun\trun the command\ttest result: ok. 1 passed\nrun\tPost\ty";
  assert.deepEqual(commandLines(named), ["run\trun the command\ttest result: ok. 1 passed"]);
  assert.deepEqual(commandLines("run\tUNKNOWN STEP\tno step here"), []);
});
// ---- preflight: the two --only mistakes that used to cost a whole remote run ----

test("--only refuses a directory with the fix in the message, and passes a file list", () => {
  const dir = "crates/game/src";
  const bad = preflightOnly([dir, "crates/gone/src/none.rs"], { headPaths: new Set() });
  assert.equal(bad.length, 1, "one problem, not one per path");
  assert.match(bad[0], /is a directory: --only takes files/);
  assert.match(bad[0], /git status --short/, "the message names how to get the file list");
  // A path that is gone is a removal, not a directory mistake.
  assert.deepEqual(preflightOnly(["crates/gone/src/none.rs"], { headPaths: new Set() }), []);
});

test("a new mod file neither in --only nor at HEAD is refused with the add line; either one alone is enough", () => {
  const root = mkdtempSync(join(tmpdir(), "gh-only-"));
  try {
    mkdirSync(join(root, "src"), { recursive: true });
    writeFileSync(join(root, "src/lib.rs"), "mod depth;\npub mod kept;\n");
    writeFileSync(join(root, "src/depth.rs"), "");
    const from = "src/lib.rs", dep = "src/depth.rs", at = new Set([from, "src/kept.rs"]);
    const missing = preflightOnly([from], { root, headPaths: at });
    assert.equal(missing.length, 1, "the new file only, not the sibling that is already at HEAD");
    assert.match(missing[0], /declares a module at src\/depth\.rs, which is in neither --only nor HEAD/);
    assert.match(missing[0], /--only src\/lib\.rs,src\/depth\.rs/, "the fix is the full --only line");
    assert.deepEqual(preflightOnly([from, dep], { root, headPaths: at }), [], "listed in --only: fine");
    assert.deepEqual(preflightOnly([from], { root, headPaths: new Set([...at, dep]) }), [], "already at HEAD: fine");
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test("a module file resolves the way rustc resolves it: sibling folder, mod.rs, and an explicit #[path]", () => {
  assert.deepEqual(modFileOf("crates/a/src/lib.rs", "b"), ["crates/a/src/b.rs", "crates/a/src/b/mod.rs"]);
  assert.deepEqual(modFileOf("crates/a/src/b.rs", "c"), ["crates/a/src/b/c.rs", "crates/a/src/b/c/mod.rs"]);
  assert.deepEqual(modFileOf("crates/a/src/b/mod.rs", "c"), ["crates/a/src/b/c.rs", "crates/a/src/b/c/mod.rs"]);
  // One candidate list per declaration: `mod f {}` is an inline module, `// mod g;` is a comment.
  assert.deepEqual(declaredModFiles("crates/a/src/b.rs", "pub mod c;\nmod d;\npub(crate) mod e;\nmod f {}\n// mod g;"), [
    ["crates/a/src/b/c.rs", "crates/a/src/b/c/mod.rs"],
    ["crates/a/src/b/d.rs", "crates/a/src/b/d/mod.rs"],
    ["crates/a/src/b/e.rs", "crates/a/src/b/e/mod.rs"],
  ]);
  assert.deepEqual(declaredModFiles("crates/a/src/lib.rs", '#[path = "odd/place.rs"]\nmod weird;'), [["crates/a/src/odd/place.rs"]]);
});

// ---- clippy: a dependency's lint is not this crate's ----

test("clippy with -p adds --no-deps once, before the -- that starts the lint flags", () => {
  assert.deepEqual(withNoDeps("cargo clippy -p engine-ai -- -D warnings"), { cmd: "cargo clippy -p engine-ai --no-deps -- -D warnings", added: true });
  assert.deepEqual(withNoDeps("cargo clippy --package engine-ai --all-targets"), { cmd: "cargo clippy --package engine-ai --all-targets --no-deps", added: true });
  assert.equal(withNoDeps("cargo clippy -p engine-ai --no-deps").added, false, "already there");
  assert.equal(withNoDeps("cargo clippy --all-targets").added, false, "no -p: --no-deps would be refused by cargo");
  assert.equal(withNoDeps("cargo test -p engine-ai").added, false, "not clippy");
  assert.equal(withNoDeps("cargo clippy -p a --no-deps -- -D warnings").added, false, "added once, not twice");
});

// ---- the push slots: a few pushes at a time, never a failed run ----

test("full push slots make the next push wait and say so, then push without one; a dead slot is taken over", async () => {
  const lock = pushLockPath();
  mkdirSync(dirname(lock), { recursive: true });
  const said = []; const say = (m) => said.push(m);
  const slots = [lock, `${lock}.1`];
  // Our own pid is alive, so both slots are held: the wait says so once, and past the cap it pushes anyway.
  for (const s of slots) writeFileSync(s, String(process.pid));
  try {
    assert.equal(typeof (await pushLock({ capMs: 600, say, slots: 2 })), "number", "past the cap it resolves, never rejects");
    assert.equal(said.length, 2, "one wait line, then one 'without one' line, not one per poll");
    assert.match(said[0], /waiting for a push slot \(2 pushes at a time/);
    assert.match(said[1], /pushing without one/);
    pushUnlock();
    for (const s of slots) assert.ok(existsSync(s), "a push without a slot frees nobody else's slot");
  } finally { for (const s of slots) rmSync(s, { force: true }); }
  // A slot whose owner is gone is cleared and taken at once.
  writeFileSync(lock, "999999");
  try { assert.equal(typeof (await pushLock({ say, slots: 1 })), "number", "acquired, and it reports the wait in ms"); pushUnlock(); assert.ok(!existsSync(lock), "unlock frees the slot it took"); }
  finally { rmSync(lock, { force: true }); }
});

test("the local line reports the push transfer, and the lock wait only when it was over a second", () => {
  assert.deepEqual(pushLaps({ seconds: 29.54, waitedMs: 120 }), ["push 29.5"], "no wait line under a second");
  assert.deepEqual(pushLaps({ seconds: 5.04, waitedMs: 12_600 }), ["push 5.0", "pushwait 12.6"], "a waited turn is named");
});

test("a wip commit built after a HAVE push lists HAVE_PUSHED as its second parent", () => {
  // A bare temp repo stands in for the mirror: the point is the shape of commit-tree's -p list, not a network call.
  const dir = mkdtempSync(join(tmpdir(), "gh-have-"));
  try {
    execFileSync("git", ["init", "-q", "--bare", dir]);
    const env = { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@e", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@e" };
    const git = (args, input) => execFileSync("git", ["--git-dir", dir, ...args], { encoding: "utf8", input, env }).trim();
    const tree = git(["hash-object", "-w", "-t", "tree", "--stdin"], "");
    const first = git(["commit-tree", tree, "-m", "main"]);
    assert.equal(haveParent(first, dir), "", "nothing pushed yet: one parent");
    const pushed = git(["commit-tree", tree, "-m", "an earlier wip"]);
    git(["update-ref", "refs/gh-check/have-pushed", pushed]);
    assert.equal(haveParent(first, dir), pushed, "the earlier wip is the second parent");
    assert.equal(haveParent(pushed, dir), "", "never its own parent");
    // The exact call startRemote makes, so the parent order is the one git records.
    const wip = git(["commit-tree", tree, "-p", first, ...(haveParent(first, dir) ? ["-p", haveParent(first, dir)] : []), "-m", "wip"]);
    assert.deepEqual(git(["rev-list", "--parents", "-n", "1", wip]).split(" ").slice(1), [first, pushed], "parent 1 is the mirror's main, parent 2 is HAVE_PUSHED");
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test("a wasm check line installs the wasm32 target before the command runs", () => {
  // Runs 40125053939 and 40125845878 died on E0463 `can't find crate for core`: the
  // runner image has the native rust-std only, so a --target wasm32 line needs the add.
  const wf = readFileSync(join(ROOT, ".github", "workflows", "check.yml"), "utf8").replace(/\r\n/g, "\n");
  const add = wf.indexOf("rustup target add wasm32-unknown-unknown");
  assert.ok(add > 0, "check.yml adds the wasm32 target");
  assert.match(wf, /if: contains\(inputs\.cmd, '--target wasm32-unknown-unknown'\)/, "only when the command asks for it");
  assert.ok(add < wf.indexOf("- name: run the command"), "installed before the step that runs the command");
});

test("a mod inside an inline mod block resolves one folder down, and the block's closing brace restores the top", () => {
  const src = ["mod a;", "#[cfg(test)]", "mod tests {", "    mod b;", "    #[path = \"c_t.rs\"]", "    mod c;", "}", "mod d;", ""].join("\n");
  assert.deepEqual(declaredModFiles("crates/physics/src/ragdoll.rs", src), [
    ["crates/physics/src/ragdoll/a.rs", "crates/physics/src/ragdoll/a/mod.rs"],
    ["crates/physics/src/ragdoll/tests/b.rs", "crates/physics/src/ragdoll/tests/b/mod.rs"],
    ["crates/physics/src/ragdoll/tests/c_t.rs"],
    ["crates/physics/src/ragdoll/d.rs", "crates/physics/src/ragdoll/d/mod.rs"],
  ]);
  assert.deepEqual(declaredModFiles("crates/x/src/lib.rs", "mod tests {\n    mod b;\n}\n"), [["crates/x/src/tests/b.rs", "crates/x/src/tests/b/mod.rs"]]);
});

test("a failed run whose every failure is a known master red says so; a new failure is named as the caller's", () => {
  const dir = mkdtempSync(join(tmpdir(), "reds-")); const file = join(dir, "r.json");
  writeFileSync(file, JSON.stringify({ head: "abc1234", at: Date.now(), tests: ["hit_track::tests::a", "outline::tests::b"] }));
  const log = "test hit_track::tests::a ... FAILED\ntest x::y ... ok\n";
  try {
    assert.match(knownRedsNote(log, { file }), /every failure here is a known master red at abc1234/);
    assert.match(knownRedsNote(`${log}test mine::t ... FAILED\n`, { file }), /1 of these 2 failures .*yours: mine::t/);
    assert.equal(knownRedsNote("test mine::t ... FAILED\n", { file }), "", "nothing known failed: no note");
    assert.equal(knownRedsNote(log, { file, now: Date.now() + 7 * 3600_000 }), "", "a list older than 6 h says nothing");
    assert.deepEqual(parseArgs(["--master-reds", "123", "abc1234"]).masterReds, { run: "123", head: "abc1234" });
    assert.match(parseArgs(["--master-reds", "x", "y"]).error, /run id/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

const tmpRepo = (prefix) => {
  const repo = mkdtempSync(join(tmpdir(), prefix));
  const git = (...args) => execFileSync("git", ["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false", "-c", "user.name=ghcheck", "-c", "user.email=ghcheck@example.invalid", ...args], { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  git("init", "-q");
  return { repo, git };
};

test("a green --only run records what each code path changes, so commit-mine can match a commit to it", () => {
  const { repo, git } = tmpRepo("ghchg-");
  try {
    writeFileSync(join(repo, "a.rs"), "line 1\nline 2\nline 3\n");
    writeFileSync(join(repo, "notes.md"), "x\n");
    git("add", "-A"); git("commit", "-q", "-m", "base");
    writeFileSync(join(repo, "a.rs"), "theirs\nline 1\nline 2\nmine\nline 3\n");
    writeFileSync(join(repo, "b.rs"), "pub fn b() {}\n");
    writeFileSync(join(repo, "notes.md"), "y\n");
    const ch = changesOf(["a.rs#2", "b.rs", "notes.md"], git("rev-parse", "HEAD"), repo);
    assert.deepEqual(Object.keys(ch).sort(), ["a.rs", "b.rs"], "only the files a build compiles");
    assert.equal(ch["a.rs"].a, changeOf("x\n", "x\nmine\n").a, "a.rs#2 is the picked hunk alone, not the other writer's");
    assert.deepEqual([ch["a.rs"].na, ch["a.rs"].nd, ch["b.rs"].na, ch["b.rs"].nd], [1, 0, 1, 0], "a new file adds its lines and drops none");
  } finally { rmSync(repo, { recursive: true, force: true, maxRetries: 3 }); }
});

test("a compile error two runs with other --only sets did not carry refuses the lines that build that crate, until it changes", () => {
  const { repo, git } = tmpRepo("ghred-");
  const file = join(repo, "head-reds.json");
  try {
    const crate = (dir, name, deps = "") => {
      mkdirSync(join(repo, "crates", dir, "src"), { recursive: true });
      writeFileSync(join(repo, "crates", dir, "Cargo.toml"), `[package]\nname = "${name}"\n\n[dependencies]\n${deps}`);
      writeFileSync(join(repo, "crates", dir, "src", "lib.rs"), "pub fn x() {}\n");
    };
    crate("render", "engine-render"); crate("game", "engine-game", 'engine-render = { path = "../render" }\n'); crate("combat", "engine-combat");
    writeFileSync(join(repo, "Cargo.toml"), "[workspace]\n"); writeFileSync(join(repo, "Cargo.lock"), "# lock\n");
    git("add", "-A"); git("commit", "-q", "-m", "base");
    const full = git("rev-parse", "HEAD");
    const out = ["error[E0583]: file not found for module `weather`", "   --> crates\\render\\src\\lib.rs:261:1", "    |", "error: could not compile `engine-render` (lib) due to 3 previous errors"].join("\n");
    const red = compileRedOf(out);
    assert.deepEqual([red.code, red.file, red.line, red.dir], ["E0583", "crates/render/src/lib.rs", 261, "crates/render"]);
    assert.equal(compileRedOf("test a ... FAILED\nthread 'a' panicked at crates/x/src/a.rs:1:1"), null, "a test failure is not a compile red");
    const job = (only, rid) => ({ full, head: "abc1234", only, rid });
    assert.equal(noteHeadRed(job(["crates/game/src/a.rs"], 1), red, { file, root: repo }).sets.length, 1);
    const gameLine = "cargo test -p engine-game --lib x";
    assert.equal(headRedFor(full, gameLine, ["crates/game/src/b.rs"], { file, root: repo }), null, "one run alone may be its own change breaking a caller");
    noteHeadRed(job(["crates/game/src/a.rs#2"], 1), red, { file, root: repo });
    assert.equal(headRedFor(full, gameLine, ["crates/game/src/b.rs"], { file, root: repo }), null, "the same files again confirm nothing");
    noteHeadRed(job(["crates/combat/src/c.rs"], 2), red, { file, root: repo });
    const hit = headRedFor(full, gameLine, ["crates/game/src/b.rs"], { file, root: repo });
    assert.equal(hit?.crate, "engine-render", "confirmed: game builds on render's lib");
    assert.match(masterRedLine(hit), /MASTER IS RED, NOT YOURS: engine-render does not compile at master \(since abc1234\): crates\/render\/src\/lib\.rs:261 error\[E0583\] file not found for module `weather` \(runs 1, 2 /);
    assert.equal(headRedFor(full, "cargo test -p engine-combat --lib", null, { file, root: repo }), null, "a crate that does not build on render runs");
    assert.equal(headRedFor(full, "cargo check -p engine-game", ["crates/render/src/lib.rs"], { file, root: repo }), null, "carrying the red file is fixing it");
    assert.equal(headRedFor(full, "cargo test --workspace --lib", null, { file, root: repo })?.wide, true, "a workspace line is told, not refused");
    writeFileSync(join(repo, "crates", "game", "src", "lib.rs"), "pub fn y() {}\n"); git("commit", "-q", "-am", "game");
    assert.equal(headRedFor(git("rev-parse", "HEAD"), "cargo check -p engine-render", null, { file, root: repo })?.crate, "engine-render", "a commit elsewhere leaves it red");
    writeFileSync(join(repo, "crates", "render", "src", "lib.rs"), "pub fn fixed() {}\n"); git("commit", "-q", "-am", "fix");
    const fixed = git("rev-parse", "HEAD");
    assert.equal(headRedFor(fixed, "cargo check -p engine-render", null, { file, root: repo }), null, "the fix moves render's tree: lines run again");
    const testRed = compileRedOf(out.replace("(lib)", "(lib test)"));
    noteHeadRed({ full: fixed, head: "def5678", only: ["x.rs"], rid: 3 }, testRed, { file, root: repo });
    noteHeadRed({ full: fixed, head: "def5678", only: ["y.rs"], rid: 4 }, testRed, { file, root: repo });
    assert.equal(headRedFor(fixed, "cargo test -p engine-render --lib", null, { file, root: repo })?.lib, false, "a red in render's tests refuses render's test lines");
    assert.equal(headRedFor(fixed, gameLine, null, { file, root: repo }), null, "game builds render's lib, not its tests");
    assert.equal(headRedFor(fixed, "cargo check -p engine-render", null, { file, root: repo }), null, "a check without --tests does not build them");
  } finally { rmSync(repo, { recursive: true, force: true, maxRetries: 3 }); }
});
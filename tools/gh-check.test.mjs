// Run: node --test tools/gh-check.test.mjs
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { DEFAULT_BUDGET_S, LOCAL_FEATURES, idAgeMs, localOnly, main, newId, okCmd, parseArgs, route, runLocal, testsRan, words } from "./gh-check.mjs";

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
  assert.deepEqual(parseArgs(["cargo", "test", "-p", "x"], {}), { cmd: "cargo test -p x", wait: null, mode: "auto", budget: DEFAULT_BUDGET_S, error: null });
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

const fakes = (remoteResults, localCode = 0) => {
  const calls = { remote: 0, local: [], said: [] };
  return {
    calls,
    remote: async () => remoteResults[Math.min(calls.remote++, remoteResults.length - 1)],
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

test("route: GPU, window and Windows-only lines never touch GitHub", async () => {
  const line = "cargo test -p engine-render --features window --lib present";
  const f = fakes([{ code: 0 }], 0);
  assert.equal(await route({ cmd: line, remote: f.remote, local: f.local, say: f.say }), 0);
  assert.deepEqual([f.calls.remote, f.calls.local], [0, [line]]);
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

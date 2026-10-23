// Drills for tools/hooks-install.mjs (gap 7): the installer points a repo at the
// tracked hooks, --check fails before and passes after, nothing is written into
// .git/hooks, and a missing tracked hook refuses the install. Every drill uses
// its own temp repo under target/ with an isolated git config, so the shared
// tree's git config is never read or written. No network.
//   node --test tools/hooks-install.test.mjs
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
} from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { HOOKS, HOOKS_PATH } from "./hooks-install.mjs";

const REPO = join(dirname(fileURLToPath(import.meta.url)), "..");
const TOOL = join(REPO, "tools", "hooks-install.mjs");

// Point git at an empty config so a global core.hooksPath on this PC cannot
// make a fresh fixture look installed.
const fixtureEnv = (root) => ({
  ...process.env,
  E2040_HOOKS_ROOT: root,
  GIT_CONFIG_GLOBAL: join(root, "gitconfig-global-missing"),
  GIT_CONFIG_NOSYSTEM: "1",
});
const git = (root, args) => spawnSync("git", ["-C", root, ...args], { encoding: "utf8", env: fixtureEnv(root) });
const install = (root, args = []) =>
  spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8", env: fixtureEnv(root), timeout: 60_000 });
const out = (r) => `${r.stdout ?? ""}${r.stderr ?? ""}`;
const hookListing = (root) => {
  const dir = join(root, ".git", "hooks");
  return existsSync(dir) ? readdirSync(dir).sort() : [];
};

function fixture({ hooks = HOOKS } = {}) {
  const root = join(REPO, "target", `hooks-install-drill-${process.pid}-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(root, "tools", "hooks"), { recursive: true });
  const init = git(root, ["init", "--quiet"]);
  assert.equal(init.status, 0, out(init));
  for (const hook of HOOKS) {
    if (!hooks.includes(hook)) continue;
    const dest = join(root, "tools", "hooks", hook);
    copyFileSync(join(REPO, "tools", "hooks", hook), dest);
    // A Unix checkout can hand out 0644; install mode owns the executable bit.
    if (process.platform !== "win32") chmodSync(dest, 0o644);
  }
  return { root, before: hookListing(root) };
}

test("--check fails on a fresh repo, passes once core.hooksPath points at tools/hooks, and install writes nothing into .git/hooks", () => {
  const { root, before } = fixture();
  try {
    const early = install(root, ["--check"]);
    assert.equal(early.status, 1, out(early));
    assert.match(out(early), /hooks: not installed/);

    // The literal gap-7 fix: set core.hooksPath, change nothing else.
    const set = git(root, ["config", "--local", "core.hooksPath", HOOKS_PATH]);
    assert.equal(set.status, 0, out(set));
    const configured = install(root, ["--check"]);
    assert.equal(configured.status, 0, out(configured));
    assert.match(out(configured), /hooks: installed/);

    const unset = git(root, ["config", "--local", "--unset", "core.hooksPath"]);
    assert.equal(unset.status, 0, out(unset));
    assert.equal(install(root, ["--check"]).status, 1, "an unset core.hooksPath is the fresh-clone state");

    // Install mode sets the relative path (no home path), and verifies.
    const done = install(root);
    assert.equal(done.status, 0, out(done));
    assert.match(out(done), /hooks: installed/);
    const value = git(root, ["config", "--local", "--get", "core.hooksPath"]);
    assert.equal(value.status, 0, out(value));
    assert.equal(value.stdout.trim(), HOOKS_PATH);

    assert.deepEqual(hookListing(root), before, "install wrote nothing into .git/hooks");
    for (const hook of HOOKS) {
      assert.ok(!existsSync(join(root, ".git", "hooks", hook)), `.git/hooks/${hook} stays absent`);
    }
    if (process.platform !== "win32") {
      for (const hook of HOOKS) {
        const mode = statSync(join(root, "tools", "hooks", hook)).mode & 0o777;
        assert.equal(mode & 0o111, 0o111, `install sets the executable bit on ${hook}`);
      }
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("install refuses while a tracked hook is missing and leaves core.hooksPath alone", () => {
  const { root } = fixture({ hooks: ["pre-commit"] });
  try {
    const refused = install(root);
    assert.equal(refused.status, 2, out(refused));
    assert.match(out(refused), /refused: tracked hooks missing: pre-push/);
    const value = git(root, ["config", "--local", "--get", "core.hooksPath"]);
    assert.notEqual(value.status, 0, "no core.hooksPath was written");
    assert.ok(!existsSync(join(root, ".git", "hooks", "pre-push")));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("both tracked hooks are POSIX sh with no home paths", (t) => {
  for (const hook of HOOKS) {
    const file = join(REPO, "tools", "hooks", hook);
    const text = readFileSync(file, "utf8");
    assert.match(text, /^#!\/bin\/sh\n/, `${hook} starts with a POSIX shebang`);
    assert.doesNotMatch(text, /[A-Za-z]:[\\/]Users[\\/]/, `${hook} carries no home path`);
    const sh = spawnSync("sh", ["-n", file], { encoding: "utf8" });
    if (sh.error?.code === "ENOENT") t.diagnostic("sh is not on PATH: syntax check skipped");
    else assert.equal(sh.status, 0, `${hook} parses as sh:\n${out(sh)}`);
  }
  const prePush = readFileSync(join(REPO, "tools", "hooks", "pre-push"), "utf8");
  assert.match(prePush, /git lfs pre-push/);
  assert.match(prePush, /cargo deny check licenses/);
  assert.match(prePush, /node tools\/license-gate\.mjs/);
});

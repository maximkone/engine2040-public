// Run: node --test tools/release.test.mjs
// TG5: the version check and the tag gate. A tag is only made when the version,
// the changelog and the lock agree and the tree is clean; the git calls are
// injected, so the test never makes a tag.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { check, lockVersion, main, newestChangelogVersion, tagDecision, workspaceVersion } from "./release.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const TOML = `[package]
name = "engine2040"
version.workspace = true

[workspace.package]
version = "0.1.0"
edition = "2021"
`;

const CHANGELOG = `# Changelog

## 0.1.0

Start.
`;

const LOCK = `version = 4

[[package]]
name = "ab_glyph"
version = "0.2.29"

[[package]]
name = "engine2040"
version = "0.1.0"
dependencies = [
 "engine-core",
]
`;

test("the workspace version is read from [workspace.package], not [package]", () => {
  assert.equal(workspaceVersion(TOML), "0.1.0");
  assert.equal(workspaceVersion("[package]\nversion.workspace = true\n"), null);
});

test("the newest changelog heading is the first '## <version>' line", () => {
  assert.equal(newestChangelogVersion(CHANGELOG), "0.1.0");
  assert.equal(newestChangelogVersion("## 0.2.0\n\nnew\n\n## 0.1.0\n"), "0.2.0");
  assert.equal(newestChangelogVersion("## Unreleased\n\n## 0.1.0\n"), "0.1.0");
  assert.equal(newestChangelogVersion("no headings"), null);
});

test("the lock version is the engine2040 entry, not its neighbours", () => {
  assert.equal(lockVersion(LOCK), "0.1.0");
  assert.equal(lockVersion(LOCK, "ab_glyph"), "0.2.29");
  assert.equal(lockVersion(LOCK, "absent"), null);
});

test("three agreeing files pass; one disagreement names itself", () => {
  const ok = check({ toml: TOML, changelog: CHANGELOG, lock: LOCK });
  assert.deepEqual(ok, { ok: true, version: "0.1.0", changelogVersion: "0.1.0", locked: "0.1.0", problems: [] });
  const staleHeading = check({ toml: TOML, changelog: "## 0.0.9\n", lock: LOCK });
  assert.equal(staleHeading.ok, false);
  assert.match(staleHeading.problems.join("\n"), /CHANGELOG\.md newest heading is 0\.0\.9, workspace version is 0\.1\.0/);
  const staleLock = check({ toml: TOML, changelog: CHANGELOG, lock: LOCK.replace('"engine2040"\nversion = "0.1.0"', '"engine2040"\nversion = "0.2.0"') });
  assert.equal(staleLock.ok, false);
  assert.match(staleLock.problems.join("\n"), /Cargo\.lock engine2040 is 0\.2\.0/);
  const missing = check({ toml: "", changelog: "", lock: "" });
  assert.equal(missing.problems.length, 3);
});

test("a tag needs a green check, a clean tree and a free name", () => {
  const green = check({ toml: TOML, changelog: CHANGELOG, lock: LOCK });
  const red = check({ toml: TOML, changelog: "## 0.0.9\n", lock: LOCK });
  assert.equal(tagDecision({ result: green, clean: true, tagExists: false, tag: "v0.1.0" }).ok, true);
  assert.match(tagDecision({ result: red, clean: true, tagExists: false, tag: "v0.1.0" }).why, /check is red/);
  assert.match(tagDecision({ result: green, clean: false, tagExists: false, tag: "v0.1.0" }).why, /tree is dirty/);
  assert.match(tagDecision({ result: green, clean: true, tagExists: true, tag: "v0.1.0" }).why, /already exists/);
});

// One fake git: it answers status/tag --list and records the tag it would make.
function fakeGit({ dirty = false, tags = [] } = {}) {
  const calls = [];
  const run = (args) => {
    calls.push(args.join(" "));
    if (args[0] === "status") return { status: 0, stdout: dirty ? " M Cargo.toml\n" : "", stderr: "" };
    if (args[0] === "tag" && args[1] === "--list") return { status: 0, stdout: tags.includes(args[2]) ? `${args[2]}\n` : "", stderr: "" };
    if (args[0] === "tag" && args[1] === "-a") return { status: 0, stdout: "", stderr: "" };
    return { status: 1, stdout: "", stderr: "unexpected git call" };
  };
  return { calls, run };
}

function tree() {
  const dir = mkdtempSync(join(tmpdir(), "release-"));
  writeFileSync(join(dir, "Cargo.toml"), TOML);
  writeFileSync(join(dir, "Cargo.lock"), LOCK);
  writeFileSync(join(dir, "CHANGELOG.md"), CHANGELOG);
  return dir;
}

test("--check reads the three real files and exits 0", () => {
  const dir = tree();
  try {
    const lines = [];
    assert.equal(main(["--check"], { root: dir, say: (l) => lines.push(l), git: fakeGit().run }), 0);
    assert.match(lines.join("\n"), /release: check OK/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--check exits 1 when the changelog lags the workspace version", () => {
  const dir = tree();
  try {
    writeFileSync(join(dir, "CHANGELOG.md"), "## 0.0.9\n");
    const lines = [];
    assert.equal(main(["--check"], { root: dir, say: (l) => lines.push(l), git: fakeGit().run }), 1);
    assert.match(lines.join("\n"), /check FAILED/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--tag makes one annotated tag on a clean, green tree", () => {
  const dir = tree();
  try {
    const g = fakeGit();
    assert.equal(main(["--tag"], { root: dir, say: () => {}, git: g.run }), 0);
    assert.deepEqual(g.calls, ["status --porcelain", "tag --list v0.1.0", "tag -a v0.1.0 -m engine2040 0.1.0"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("--tag refuses a dirty tree and an existing tag without tagging", () => {
  const dir = tree();
  try {
    const dirty = fakeGit({ dirty: true });
    const lines = [];
    assert.equal(main(["--tag"], { root: dir, say: (l) => lines.push(l), git: dirty.run }), 1);
    assert.match(lines.join("\n"), /tree is dirty/);
    assert.equal(dirty.calls.some((c) => c.startsWith("tag -a")), false);

    const taken = fakeGit({ tags: ["v0.1.0"] });
    const more = [];
    assert.equal(main(["--tag"], { root: dir, say: (l) => more.push(l), git: taken.run }), 1);
    assert.match(more.join("\n"), /already exists/);
    assert.equal(taken.calls.some((c) => c.startsWith("tag -a")), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("no mode and an unknown flag exit 2", () => {
  assert.equal(main([], { say: () => {} }), 2);
  assert.equal(main(["--bump"], { say: () => {} }), 2);
  assert.equal(main(["--help"], { say: () => {} }), 0);
});

test("this repo's own files agree today", () => {
  const lines = [];
  assert.equal(main(["--check"], { root: ROOT, say: (l) => lines.push(l) }), 0, lines.join("\n"));
});

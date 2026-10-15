import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseOwner } from "./sprint-lock.mjs";

const TOOL = fileURLToPath(new URL("./sprint-lock.mjs", import.meta.url));
const run = (args, cwd) => spawnSync(process.execPath, [TOOL, ...args], { encoding: "utf8", cwd });
const freshDir = () => mkdtempSync(join(tmpdir(), "sprint-lock-"));

test("parseOwner: a fresh interactive lease gets a session token", () => {
  assert.match(parseOwner("acquire", []), /^orchestrator#[0-9a-f]{4}$/);
  assert.match(parseOwner("takeover", []), /^orchestrator#[0-9a-f]{4}$/);
  assert.match(parseOwner("acquire", ["--owner", "orchestrator", "--id", "go2"]), /^orchestrator#[0-9a-f]{4}$/);
});

test("parseOwner: named owners pass through; other commands keep plain orchestrator", () => {
  assert.equal(parseOwner("acquire", ["keeper"]) , "keeper");
  assert.equal(parseOwner("takeover", ["keeper"]), "keeper");
  assert.equal(parseOwner("refresh", []), "orchestrator");
  assert.equal(parseOwner("refresh", ["orchestrator#abcd"]), "orchestrator#abcd");
  assert.equal(parseOwner("release", ["keeper"]), "keeper");
});

test("parseOwner: a flag-like owner throws the usage error", () => {
  assert.throws(() => parseOwner("acquire", ["--owner", "--bogus"]), /Bad owner "--bogus"/);
});

test("CLI in a temp cwd: acquire, status, refresh, release round-trip", () => {
  const dir = freshDir();
  try {
    assert.match(run(["status"], dir).stdout.trim(), /^FREE$/);
    const a = run(["acquire", "tester"], dir);
    assert.equal(a.status, 0);
    assert.match(a.stdout, /ACQUIRED target\/sprint\.lock for tester/);
    assert.match(run(["status"], dir).stdout, /^HELD: tester since /);
    const clash = run(["acquire", "other"], dir);
    assert.equal(clash.status, 1);
    assert.match(clash.stderr, /LOCKED by tester since /);
    assert.equal(run(["refresh", "tester"], dir).status, 0);
    const wrongRefresh = run(["refresh", "other"], dir);
    assert.equal(wrongRefresh.status, 1);
    assert.match(wrongRefresh.stderr, /refresh refused/);
    const wrongRelease = run(["release", "other"], dir);
    assert.equal(wrongRelease.status, 1);
    assert.match(wrongRelease.stderr, /release refused/);
    const rel = run(["release", "tester"], dir);
    assert.equal(rel.status, 0);
    assert.match(rel.stdout.trim(), /^RELEASED$/);
    assert.match(run(["status"], dir).stdout.trim(), /^FREE$/);
    assert.equal(existsSync(join(dir, "target", "sprint.lock")), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI in a temp cwd: a stale lock reports STALE and acquire replaces it", () => {
  const dir = freshDir();
  try {
    mkdirSync(join(dir, "target"), { recursive: true });
    writeFileSync(join(dir, "target", "sprint.lock"), "dead since 2020-01-01T00:00:00.000Z\n");
    utimesSync(join(dir, "target", "sprint.lock"), new Date("2020-01-01"), new Date("2020-01-01"));
    assert.match(run(["status"], dir).stdout, /^STALE: dead since /);
    const a = run(["acquire", "newowner"], dir);
    assert.equal(a.status, 0);
    assert.match(a.stdout, /replacing stale lock/);
    assert.match(readFileSync(join(dir, "target", "sprint.lock"), "utf8"), /^newowner since /);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI in a temp cwd: takeover replaces any lock noisily", () => {
  const dir = freshDir();
  try {
    assert.equal(run(["acquire", "holder"], dir).status, 0);
    const t = run(["takeover", "rescuer"], dir);
    assert.equal(t.status, 0);
    assert.match(t.stdout, /\[WARN\] taking over lock/);
    assert.match(t.stdout, /takeover/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

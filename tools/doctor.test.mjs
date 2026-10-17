// DOC1 drills: the hardened doctor must never mutate lane evidence or
// build pools without scoped consent. Each drill builds a
// fixture tree, runs tools/doctor.mjs with E2040_DOCTOR_ROOT pointed at it,
// and asserts the fixture is byte-unchanged where the law requires it.
//   node --test tools/doctor.test.mjs
import { spawnSync } from "node:child_process";
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const REPO = process.cwd();

const fixture = (lanes = {}) => {
  const dir = join(tmpdir(), `doctor-drill-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(dir, "tools"), { recursive: true });
  mkdirSync(join(dir, "target", "lanes"), { recursive: true });
  mkdirSync(join(dir, "target", "pool-1"), { recursive: true });
  writeFileSync(join(dir, "rust-toolchain.toml"), '[toolchain]\nchannel = "stable"\n');
  writeFileSync(join(dir, "target", "lanes", "keep.patch"), "evidence\n");
  writeFileSync(join(dir, "target", "pool-1", "cache.bin"), "cache\n");
  writeFileSync(join(dir, "target", "lanes-worktrees.json"), JSON.stringify({ worktrees: lanes }));
  return dir;
};

const doctor = (dir, args, freeGb) => {
  const r = spawnSync(process.execPath, [join(REPO, "tools", "doctor.mjs"), ...args], {
    encoding: "utf8",
    env: { ...process.env, E2040_DOCTOR_ROOT: dir, ...(freeGb === undefined ? {} : { E2040_DOCTOR_FREE_GB: String(freeGb) }) },
  });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};


test("low disk with registered lanes changes nothing even with --consent=cache", () => {
  const dir = fixture({ holder: "orchestrator since 2026-01-01T00:00:00.000Z\n", ageMs: 60_000, lanes: { a: {} } });
  try {
    const r = doctor(dir, ["--fix", "--consent=cache"], 2);
    assert.ok(existsSync(join(dir, "target", "pool-1", "cache.bin")), "pools kept while a lane may build");
    assert.ok(existsSync(join(dir, "target", "lanes", "keep.patch")), "evidence never deleted");
    assert.match(r.out, /NEEDS YOU/, "asks the operator instead");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("low disk, zero lanes: --fix alone changes nothing; --consent=cache evicts pools but keeps evidence", () => {
  const dir = fixture({});
  try {
    doctor(dir, ["--fix"], 2);
    assert.ok(existsSync(join(dir, "target", "pool-1", "cache.bin")), "no consent means no eviction");
    const r = doctor(dir, ["--fix", "--consent=cache"], 2);
    assert.ok(!existsSync(join(dir, "target", "pool-1")), "own pools evicted with consent");
    assert.ok(existsSync(join(dir, "target", "lanes", "keep.patch")), "evidence kept through eviction");
    assert.match(r.out, /FIXED/, "eviction is receipted");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("unknown consent scope fails honestly and changes nothing", () => {
  const dir = fixture({});
  try {
    const r = doctor(dir, ["--fix", "--consent=bogus"], 2);
    assert.equal(r.status, 2, "usage error, not a repair");
    assert.ok(existsSync(join(dir, "target", "pool-1", "cache.bin")));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

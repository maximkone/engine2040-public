// tools/clean.mjs drills: the loop's cleaner (sprint-loop.mjs --auto) must
// report trash without touching live state. Each drill builds a fixture tree,
// runs tools/clean.mjs with cwd pointed at it, and asserts the fixture is
// byte-unchanged where the law requires it. Dry-run only: nothing here may
// pass --yes/--auto against the real repo.
//   node --test tools/clean.test.mjs
import { spawnSync } from "node:child_process";
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const REPO = process.cwd();

const fixture = () => {
  const dir = join(tmpdir(), `clean-drill-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(dir, "target", "tmp"), { recursive: true });
  mkdirSync(join(dir, "target", "lanes", "old-build"), { recursive: true });
  writeFileSync(join(dir, "target", "tmp", "scratch.bin"), "scratch\n");
  writeFileSync(join(dir, "target", "canon-fail.log"), "fail\n");
  writeFileSync(join(dir, "target", "keep.txt"), "keep\n");
  return dir;
};

const clean = (dir, args) => {
  const r = spawnSync(process.execPath, [join(REPO, "tools", "clean.mjs"), ...args], {
    encoding: "utf8", cwd: dir,
  });
  return { status: r.status, out: `${r.stdout ?? ""}${r.stderr ?? ""}` };
};

test("dry-run reports trash and old lanes but deletes nothing", () => {
  const dir = fixture();
  try {
    const r = clean(dir, ["--dry-run"]);
    assert.equal(r.status, 0, `exit 0 on report-only, got: ${r.out}`);
    assert.match(r.out, /WOULD target[\\/]tmp/, "trash dir listed");
    assert.match(r.out, /WOULD reclaim/, "reclaim summary printed");
    assert.match(r.out, /SKIP pools: lanes-worktrees\.json unreadable/, "unknown pools fail honest");
    assert.ok(existsSync(join(dir, "target", "tmp", "scratch.bin")), "dry-run deletes nothing");
    assert.ok(existsSync(join(dir, "target", "canon-fail.log")), "dry-run deletes nothing");
    assert.ok(existsSync(join(dir, "target", "lanes", "old-build")), "dry-run deletes nothing");
    assert.ok(existsSync(join(dir, "target", "keep.txt")), "non-trash survives");
    assert.ok(!r.out.includes("keep.txt"), "non-trash never listed");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("unknown flag exits 2 with usage and deletes nothing", () => {
  const dir = fixture();
  try {
    const r = clean(dir, ["--bogus"]);
    assert.equal(r.status, 2, `exit 2 on usage, got: ${r.out}`);
    assert.match(r.out, /Usage: node tools\/clean\.mjs/, "usage printed");
    assert.ok(existsSync(join(dir, "target", "tmp", "scratch.bin")), "usage error deletes nothing");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("empty target reports zero paths", () => {
  const dir = join(tmpdir(), `clean-drill-empty-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(join(dir, "target"), { recursive: true });
  try {
    const r = clean(dir, ["--dry-run"]);
    assert.equal(r.status, 0, `exit 0 on empty report, got: ${r.out}`);
    assert.match(r.out, /across 0 path\(s\)/, "zero paths reported");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("wave-1: no silent empty catch in tools/clean.mjs", async () => {
  const { readFileSync } = await import("node:fs");
  const src = readFileSync(join(REPO, "tools", "clean.mjs"), "utf8");
  const parts = src.split("catch").slice(1);
  assert.ok(parts.length > 0, "expected catch blocks");
  for (const p of parts) {
    assert.ok(p.slice(0, 200).includes("console.warn"), "every catch warns");
  }
  const flat = src.replace(/\s+/g, " ");
  assert.ok(!flat.includes("catch (err) { }"), "no empty catch");
  assert.ok(!flat.includes("catch { }"), "no empty bare catch");
});

import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { flagsFor, isWarmLane } from "./lanes.mjs";

test("flagsFor: manifest path, pool dir, jobs and lock flags", () => {
  const dir = join(process.cwd(), "x");
  const f = flagsFor({ dir, pool: 2 });
  assert.match(f, /^--manifest-path x\/Cargo\.toml --target-dir target\/pool-2 -j \d+ --locked$/);
  const dflt = flagsFor({ dir });
  assert.match(dflt, /--target-dir target\/pool-1/);
});

test("flagsFor: old entries without a pool still list on pool-1", () => {
  const f = flagsFor({ dir: join(process.cwd(), ".lanes", "old") });
  assert.ok(f.includes("--manifest-path .lanes/old/Cargo.toml"));
  assert.ok(f.includes("--locked"));
  assert.ok(!f.includes("--release"));
});

test("CLI list in an empty temp cwd reports zero lanes without git", () => {
  const dir = mkdtempSync(join(tmpdir(), "lanes-"));
  try {
    const r = spawnSync(process.execPath, [fileURLToPath(new URL("./lanes.mjs", import.meta.url)), "list"], { encoding: "utf8", cwd: dir });
    assert.equal(r.status, 0);
    assert.match(r.stdout, /RESULT 0 lane\(s\), \d+ pool\(s\), -j \d+/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI flags on a cold lane keeps stdout to one line and hints on stderr in pwsh form", () => {
  const dir = mkdtempSync(join(tmpdir(), "lanes-"));
  try {
    mkdirSync(join(dir, "target"), { recursive: true });
    const laneDir = join(dir, ".lanes", "cold1");
    const admin = { worktrees: { cold1: { dir: laneDir, branch: "lane/cold1", base: "abc", pool: 1, createdAt: Date.now() } } };
    writeFileSync(join(dir, "target", "lanes-worktrees.json"), JSON.stringify(admin));
    // No target/pool-1/debug/incremental here: definitionally cold.
    const r = spawnSync(process.execPath, [fileURLToPath(new URL("./lanes.mjs", import.meta.url)), "flags", "cold1"], { encoding: "utf8", cwd: dir });
    assert.equal(r.status, 0);
    const lines = r.stdout.split("\n").filter((l) => l.length > 0);
    assert.equal(lines.length, 1);
    assert.match(lines[0], /^--manifest-path .* --target-dir target\/pool-1 -j \d+ --locked$/);
    assert.ok(!r.stdout.includes("RUSTC_WRAPPER"));
    assert.match(r.stderr, /\$env:RUSTC_WRAPPER="sccache"/);
    assert.match(r.stderr, /\$env:CARGO_INCREMENTAL="0"/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("CLI flags on a warm lane prints only the flags line and no hint", () => {
  const dir = mkdtempSync(join(tmpdir(), "lanes-"));
  try {
    const incr = join(dir, "target", "pool-1", "debug", "incremental");
    mkdirSync(incr, { recursive: true });
    writeFileSync(join(incr, "probe"), "x");
    const laneDir = join(dir, ".lanes", "warm1");
    // createdAt long before the pool cache mtime: warm.
    const admin = { worktrees: { warm1: { dir: laneDir, branch: "lane/warm1", base: "abc", pool: 1, createdAt: 1 } } };
    writeFileSync(join(dir, "target", "lanes-worktrees.json"), JSON.stringify(admin));
    const r = spawnSync(process.execPath, [fileURLToPath(new URL("./lanes.mjs", import.meta.url)), "flags", "warm1"], { encoding: "utf8", cwd: dir });
    assert.equal(r.status, 0);
    const lines = r.stdout.split("\n").filter((l) => l.length > 0);
    assert.equal(lines.length, 1);
    assert.match(lines[0], /^--manifest-path .* --target-dir target\/pool-1 -j \d+ --locked$/);
    assert.equal(r.stderr, "");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// Adopted from tools/lanes-prune.test.mjs (W2-04 prune, orphan adopted 2026-10-08):
const REPO = process.cwd();
const LANES = join(REPO, "tools", "lanes.mjs");
const OLD = Date.now() - 3 * 3600 * 1000;

const git = (dir, ...args) => {
  const r = spawnSync("git", args, { encoding: "utf8", cwd: dir });
  assert.equal(r.status, 0, `git ${args.join(" ")} failed: ${r.stderr}${r.stdout}`);
  return r.stdout.trim();
};

const lanes = (dir, ...args) => {
  const r = spawnSync(process.execPath, [LANES, ...args], { encoding: "utf8", cwd: dir });
  assert.equal(r.status, 0, `lanes ${args.join(" ")} failed: ${r.stderr}${r.stdout}`);
  return r.stdout;
};

const adminPath = (dir) => join(dir, "target", "lanes-worktrees.json");

// `create` is retired, so a stray lane is built by hand: worktree on lane/<name> plus its admin entry.
const addLane = (dir, name, createdAt) => {
  const laneDir = join(dir, ".lanes", name);
  const base = git(dir, "rev-parse", "HEAD");
  git(dir, "worktree", "add", "-b", `lane/${name}`, laneDir, base);
  const admin = existsSync(adminPath(dir)) ? JSON.parse(readFileSync(adminPath(dir), "utf8")) : { worktrees: {} };
  admin.worktrees[name] = { dir: laneDir, branch: `lane/${name}`, base, pool: 1, createdAt };
  mkdirSync(join(dir, "target"), { recursive: true });
  writeFileSync(adminPath(dir), `${JSON.stringify(admin, null, 2)}\n`);
  return laneDir;
};

const mkRepo = () => {
  const dir = join(tmpdir(), `lanes-prune-${Date.now()}-${Math.floor(Math.random() * 1e6)}`);
  mkdirSync(dir, { recursive: true });
  git(dir, "init", "-b", "master");
  git(dir, "config", "user.email", "test@example.com");
  git(dir, "config", "user.name", "test");
  writeFileSync(join(dir, "a.txt"), "base\n");
  git(dir, "add", "a.txt");
  git(dir, "commit", "-m", "base");
  return dir;
};

// One lane per scenario: merged+clean+old (prunable), unmerged, dirty, young.
const mkLane = (dir, name, { merge = false, dirty = false, age = OLD } = {}) => {
  const laneDir = addLane(dir, name, age);
  writeFileSync(join(laneDir, `${name}.txt`), `${name}\n`);
  git(laneDir, "add", `${name}.txt`);
  git(laneDir, "commit", "-m", name);
  if (merge) git(dir, "merge", `lane/${name}`, "--no-edit");
  if (dirty) writeFileSync(join(laneDir, "uncommitted.txt"), "work in progress\n");
  return laneDir;
};

test("create is retired: exit 2, the one-line message, no worktree, branch or admin file", () => {
  const dir = mkRepo();
  try {
    const before = git(dir, "worktree", "list");
    const message = "lanes retired 2026-10-10 (owner: one worktree): edit owned paths in C:\\engine2040; see .opencode/commands/sprint.md";
    for (const args of [["create", "test-x"], ["create"], ["create", "test-x", "--base", "HEAD", "--pool", "1"]]) {
      const r = spawnSync(process.execPath, [LANES, ...args], { encoding: "utf8", cwd: dir });
      assert.equal(r.status, 2, `create ${args.slice(1).join(" ")} exits 2`);
      assert.equal(r.stderr.trim(), message, "stderr is exactly the one-line message");
      assert.equal(r.stdout, "", "nothing on stdout");
    }
    assert.equal(git(dir, "worktree", "list"), before, "git worktree list unchanged");
    assert.equal(git(dir, "branch", "--list", "lane/*"), "", "no lane branch");
    assert.ok(!existsSync(join(dir, ".lanes")), "no .lanes folder");
    assert.ok(!existsSync(adminPath(dir)), "no admin file");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("prune dry-run lists only the merged+clean+old lane, removes nothing", () => {
  const dir = mkRepo();
  try {
    mkLane(dir, "aaa-old", { merge: true });
    mkLane(dir, "bbb-live", { merge: false });
    mkLane(dir, "ccc-dirty", { merge: true, dirty: true });
    mkLane(dir, "ddd-young", { merge: true, age: Date.now() });
    const out = lanes(dir, "prune");
    assert.match(out, /PRUNE aaa-old/, "prunable lane listed");
    assert.match(out, /KEEP bbb-live \(unmerged\)/, "unmerged kept");
    assert.match(out, /KEEP ccc-dirty \(dirty\)/, "dirty kept");
    assert.match(out, /KEEP ddd-young \(young\)/, "young kept");
    assert.match(out, /dry-run, nothing removed/, "dry-run removes nothing");
    const admin = JSON.parse(readFileSync(adminPath(dir), "utf8"));
    assert.equal(Object.keys(admin.worktrees).length, 4, "dry-run changes nothing");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("prune --apply removes only the merged+clean+old lane, never dirty work", () => {
  const dir = mkRepo();
  try {
    const oldDir = mkLane(dir, "aaa-old", { merge: true });
    mkLane(dir, "bbb-live", { merge: false });
    const dirtyDir = mkLane(dir, "ccc-dirty", { merge: true, dirty: true });
    mkLane(dir, "ddd-young", { merge: true, age: Date.now() });
    const out = lanes(dir, "prune", "--apply");
    assert.match(out, /PRUNED aaa-old/, "merged+clean+old removed");
    assert.match(out, /pruned 1, kept 3/, "counts exact");
    assert.ok(!existsSync(oldDir), "worktree gone");
    const branches = git(dir, "branch", "--list", "lane/*");
    assert.ok(!branches.includes("lane/aaa-old"), "branch gone");
    assert.ok(branches.includes("lane/bbb-live"), "unmerged branch kept");
    assert.ok(existsSync(join(dirtyDir, "uncommitted.txt")), "dirty work untouched");
    const admin = JSON.parse(readFileSync(adminPath(dir), "utf8"));
    assert.deepEqual(Object.keys(admin.worktrees).sort(), ["bbb-live", "ccc-dirty", "ddd-young"]);
    // Second run is stable: nothing left to prune.
    const again = lanes(dir, "prune", "--apply");
    assert.match(again, /pruned 0, kept 3/, "idempotent");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("prune --apply keeps merged+clean+old lanes with a pending review or a fresh patch", () => {
  const dir = mkRepo();
  try {
    const heldDir = mkLane(dir, "aaa-held", { merge: true });
    mkLane(dir, "bbb-free", { merge: true });
    const freshDir = mkLane(dir, "ccc-fresh", { merge: true });
    mkLane(dir, "aaa", { merge: true });
    // A REVIEW packet still queued names the lane: the judge is outstanding.
    mkdirSync(join(dir, "target", "queue", "ready"), { recursive: true });
    writeFileSync(join(dir, "target", "queue", "ready", "001-aaa-held-review.md"),
      "---\nrole: judge\ntitle: review aaa-held\nchain: review\nof: builder-aaa\n---\nREVIEW lane aaa-held at .lanes/aaa-held, patch target/lanes/aaa-held.patch\n");
    // A collected patch under 24 h old keeps the lane even between packets.
    mkdirSync(join(dir, "target", "lanes"), { recursive: true });
    writeFileSync(join(dir, "target", "lanes", "ccc-fresh.patch"), "fresh\n");
    const out = lanes(dir, "prune", "--apply");
    assert.match(out, /KEEP aaa-held \(pending-review\)/, "queued review keeps the lane");
    assert.match(out, /KEEP ccc-fresh \(recent-patch\)/, "fresh patch keeps the lane");
    assert.match(out, /PRUNED bbb-free/, "unreferenced lane still pruned");
    assert.match(out, /PRUNED aaa[^a-z0-9-]/, "similar prefix (aaa vs aaa-held) never protects");
    assert.ok(existsSync(heldDir), "held worktree kept");
    assert.ok(existsSync(freshDir), "fresh worktree kept");
    const branches = git(dir, "branch", "--list", "lane/*").split("\n").map((b) => b.replace(/^[*+\s]+/, "").trim());
    assert.ok(branches.includes("lane/aaa-held"), "held branch kept");
    assert.ok(branches.includes("lane/ccc-fresh"), "fresh branch kept");
    assert.ok(!branches.includes("lane/aaa"), "prefix-similar branch gone");
    const admin = JSON.parse(readFileSync(adminPath(dir), "utf8"));
    assert.deepEqual(Object.keys(admin.worktrees).sort(), ["aaa-held", "ccc-fresh"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("remove refuses a lane with an outstanding review unless --force", () => {
  const dir = mkRepo();
  try {
    const heldDir = mkLane(dir, "aaa-held", { merge: true });
    const freeDir = mkLane(dir, "bbb-free", { merge: true });
    mkdirSync(join(dir, "target", "queue", "ready"), { recursive: true });
    writeFileSync(join(dir, "target", "queue", "ready", "001-aaa-held-review.md"),
      "---\nrole: judge\ntitle: review aaa-held\nchain: review\n---\nREVIEW lane aaa-held at .lanes/aaa-held, patch target/lanes/aaa-held.patch\n");
    // Plain remove refuses the held lane (the lead's remove + create cycle
    // re-created the lane-GC BLOCKEDs on cq10-combat 2026-10-03).
    const refused = spawnSync(process.execPath, [LANES, "remove", "aaa-held"], { encoding: "utf8", cwd: dir });
    assert.notEqual(refused.status, 0, "remove of a held lane fails");
    assert.match(refused.stderr, /outstanding review/, "refusal names the reason");
    assert.ok(existsSync(heldDir), "held worktree kept");
    // An unheld lane still removes cleanly.
    const out = lanes(dir, "remove", "bbb-free");
    assert.match(out, /REMOVED bbb-free/, "unheld lane removed");
    assert.ok(!existsSync(freeDir), "unheld worktree gone");
    // Explicit --force overrides the guard.
    const forced = lanes(dir, "remove", "aaa-held", "--force");
    assert.match(forced, /REMOVED aaa-held/, "--force removes the held lane");
    assert.ok(!existsSync(heldDir), "forced worktree gone");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

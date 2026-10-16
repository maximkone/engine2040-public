import assert from "node:assert/strict";
import test from "node:test";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { isWarmLane } from "./lanes.mjs";

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

test("a live claim keeps its lane and its -suffix lanes; a stale claim or a prefix match does not", () => {
  const dir = mkRepo();
  try {
    const claimedDir = mkLane(dir, "win6-top", { merge: true });
    mkLane(dir, "win10", { merge: true });
    mkLane(dir, "old1", { merge: true });
    mkdirSync(join(dir, "target", "queue"), { recursive: true });
    const now = new Date().toISOString();
    const past = new Date(Date.now() - 3 * 3600 * 1000).toISOString();
    writeFileSync(join(dir, "target", "queue", "claims.txt"),
      `WIN6 | builder-finish-r1 | ${now} | crates/game/src/windowed.rs\nOLD1 | builder | ${past} | crates/a\n`);
    const out = lanes(dir, "prune", "--apply");
    assert.match(out, /KEEP win6-top \(live-claim\)/, "WIN6 claim holds win6-top");
    assert.match(out, /PRUNED win10/, "win10 is not held by a WIN6 claim");
    assert.match(out, /PRUNED old1/, "a claim over 2 h old holds nothing");
    assert.ok(existsSync(claimedDir), "claimed worktree kept");
    const refused = spawnSync(process.execPath, [LANES, "remove", "win6-top"], { encoding: "utf8", cwd: dir });
    assert.notEqual(refused.status, 0, "plain remove of a claimed lane fails");
    assert.match(refused.stderr, /live-claim/, "refusal names the claim");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// Strict disk-pressure mode (--min-age-h): creation, last commit and every file older than N hours,
// and lanes git lists but target/lanes-worktrees.json lost are in scope too.
const HOURS = 3600 * 1000;
const ageFiles = (dir, hours) => {
  const past = new Date(Date.now() - hours * HOURS);
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (e.name === ".git") continue;
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else utimesSync(p, past, past);
    }
  };
  walk(dir);
};
const mkAgedLane = (dir, name, { commitHoursAgo = 30, filesHoursAgo = 30, createdHoursAgo = 30 } = {}) => {
  const laneDir = addLane(dir, name, Date.now() - createdHoursAgo * HOURS);
  writeFileSync(join(laneDir, `${name}.txt`), `${name}\n`);
  git(laneDir, "add", `${name}.txt`);
  const when = new Date(Date.now() - commitHoursAgo * HOURS).toISOString();
  const r = spawnSync("git", ["commit", "-m", name], {
    encoding: "utf8", cwd: laneDir, env: { ...process.env, GIT_AUTHOR_DATE: when, GIT_COMMITTER_DATE: when },
  });
  assert.equal(r.status, 0, `commit failed: ${r.stderr}`);
  git(dir, "merge", `lane/${name}`, "--no-edit");
  ageFiles(laneDir, filesHoursAgo);
  return laneDir;
};

test("prune --min-age-h keeps a lane whose files or last commit are young and takes git-listed lanes missing from the admin file", () => {
  const dir = mkRepo();
  try {
    const oldDir = mkAgedLane(dir, "aaa-old");
    const touchedDir = mkAgedLane(dir, "bbb-touched");
    // Same bytes, new mtime: git status stays clean, the file clock says someone is working here.
    utimesSync(join(touchedDir, "bbb-touched.txt"), new Date(), new Date());
    const freshDir = mkAgedLane(dir, "ccc-newcommit", { commitHoursAgo: 1 });
    const orphanDir = mkAgedLane(dir, "ddd-orphan");
    const admin = JSON.parse(readFileSync(adminPath(dir), "utf8"));
    delete admin.worktrees["ddd-orphan"];
    writeFileSync(adminPath(dir), `${JSON.stringify(admin, null, 2)}\n`);
    const dry = lanes(dir, "prune", "--min-age-h", "24");
    assert.match(dry, /PRUNE aaa-old .*>24h/, "old lane listed");
    assert.match(dry, /KEEP bbb-touched \(young-files\)/, "a file touched in the last 24 h keeps the lane");
    assert.match(dry, /KEEP ccc-newcommit \(young-commit\)/, "a commit in the last 24 h keeps the lane");
    assert.match(dry, /PRUNE ddd-orphan/, "a lane git lists but the admin file lost is in scope");
    assert.match(dry, /dry-run, nothing removed/);
    assert.ok(existsSync(oldDir) && existsSync(orphanDir), "dry-run removes nothing");
    // --dry-run beats --apply: the flag auto-clean.ps1 -DryRun passes can never remove a lane.
    const both = lanes(dir, "prune", "--min-age-h", "24", "--apply", "--dry-run");
    assert.match(both, /PRUNE aaa-old/, "dry-run still lists the lane");
    assert.match(both, /dry-run, nothing removed/);
    assert.ok(existsSync(oldDir) && existsSync(orphanDir), "--apply with --dry-run removes nothing");
    // The default 2 h mode does not look at files or commits: bbb-touched would go.
    assert.match(lanes(dir, "prune"), /PRUNE bbb-touched/, "default mode ignores file times");
    const out = lanes(dir, "prune", "--min-age-h", "24", "--apply", "--skip", "ddd-orphan");
    assert.match(out, /PRUNED aaa-old/);
    assert.match(out, /KEEP ddd-orphan \(live-process\)/, "--skip keeps a lane a live process names");
    assert.ok(!existsSync(oldDir), "old worktree gone");
    assert.ok(existsSync(touchedDir) && existsSync(freshDir) && existsSync(orphanDir), "young and skipped lanes stay");
    const again = lanes(dir, "prune", "--min-age-h", "24", "--apply");
    assert.match(again, /PRUNED ddd-orphan/, "without --skip the orphan goes");
    assert.ok(!existsSync(orphanDir), "orphan worktree gone");
    const branches = git(dir, "branch", "--list", "lane/*").split("\n").map((b) => b.replace(/^[*+\s]+/, "").trim());
    assert.deepEqual(branches.sort(), ["lane/bbb-touched", "lane/ccc-newcommit"]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("prune rejects a bad --min-age-h", () => {
  const dir = mkRepo();
  try {
    const r = spawnSync(process.execPath, [LANES, "prune", "--min-age-h", "abc"], { encoding: "utf8", cwd: dir });
    assert.equal(r.status, 2);
    assert.match(r.stderr, /positive number of hours/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
test("a stale patch with no queued packet no longer protects a lane", () => {
  const dir = mkRepo();
  try {
    const oldDir = mkLane(dir, "aaa-stale", { merge: true });
    mkdirSync(join(dir, "target", "lanes"), { recursive: true });
    const patch = join(dir, "target", "lanes", "aaa-stale.patch");
    writeFileSync(patch, "stale\n");
    const past = new Date(Date.now() - 25 * 3600 * 1000);
    utimesSync(patch, past, past);
    const out = lanes(dir, "prune", "--apply");
    assert.match(out, /PRUNED aaa-stale/, "stale patch does not protect");
    assert.ok(!existsSync(oldDir), "worktree gone");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("prune takes a frozen lane but keeps intent-to-add dirt with no patch or fresh files", () => {
  const dir = mkRepo();
  try {
    const frozenDir = mkLane(dir, "aaa-frozen", { merge: true });
    writeFileSync(join(frozenDir, "new-stuff.txt"), "fresh work\n");
    git(frozenDir, "add", "--intent-to-add", "--all");
    ageFiles(frozenDir, 3);
    const rawDir = mkLane(dir, "bbb-raw", { merge: true });
    writeFileSync(join(rawDir, "wip.txt"), "uncollected\n");
    const busyDir = mkLane(dir, "ccc-busy", { merge: true });
    writeFileSync(join(busyDir, "more.txt"), "more work\n");
    git(busyDir, "add", "--intent-to-add", "--all");
    mkdirSync(join(dir, "target", "lanes"), { recursive: true });
    writeFileSync(join(dir, "target", "lanes", "aaa-frozen.patch"), "frozen\n");
    // Collected 2 days ago: past the 24 h recent-patch hold, inside the 7 d
    // salvage window, so a judge can still read it.
    const twoDays = new Date(Date.now() - 2 * 24 * 3600 * 1000);
    utimesSync(join(dir, "target", "lanes", "aaa-frozen.patch"), twoDays, twoDays);
    writeFileSync(join(dir, "target", "lanes", "ccc-busy.patch"), "busy\n");
    // Busy's patch is old enough to lose the 24 h hold, so only its freshly
    // touched files protect it.
    utimesSync(join(dir, "target", "lanes", "ccc-busy.patch"), twoDays, twoDays);
    const out = lanes(dir, "prune", "--apply");
    assert.match(out, /PRUNED aaa-frozen/, "frozen lane prunes");
    assert.match(out, /KEEP bbb-raw \(dirty\)/, "uncollected dirt is never dropped");
    assert.match(out, /KEEP ccc-busy \(dirty\)/, "freshly touched files keep the lane");
    assert.ok(!existsSync(frozenDir), "frozen worktree gone");
    assert.ok(existsSync(join(dir, "target", "lanes", "aaa-frozen.patch")), "the patch carries the work on");
    assert.ok(existsSync(join(rawDir, "wip.txt")), "raw work untouched");
    const branches = git(dir, "branch", "--list", "lane/*");
    assert.ok(!branches.includes("lane/aaa-frozen"), "frozen branch gone");
    assert.ok(branches.includes("lane/bbb-raw"), "raw branch kept");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("diff and remove work on lanes git lists but the admin file lost", () => {
  const dir = mkRepo();
  try {
    const orphanDir = mkLane(dir, "aaa-orphan", { merge: true });
    writeFileSync(join(orphanDir, "extra.txt"), "orphan work\n");
    const admin = JSON.parse(readFileSync(adminPath(dir), "utf8"));
    delete admin.worktrees["aaa-orphan"];
    writeFileSync(adminPath(dir), `${JSON.stringify(admin, null, 2)}\n`);
    const out = lanes(dir, "diff", "aaa-orphan");
    assert.match(out, /PATCH target[\\/]lanes[\\/]aaa-orphan\.patch/, "orphan diff collects");
    assert.ok(existsSync(join(dir, "target", "lanes", "aaa-orphan.patch")), "orphan patch written");
    // The work is collected but uncommitted, so plain remove refuses: the
    // orphan cleanup flow is diff, then --force with the patch kept.
    const removed = lanes(dir, "remove", "aaa-orphan", "--force");
    assert.match(removed, /REMOVED aaa-orphan/, "orphan removes");
    assert.ok(!existsSync(orphanDir), "orphan worktree gone");
    assert.ok(!git(dir, "branch", "--list", "lane/*").includes("lane/aaa-orphan"), "orphan branch gone");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a pipeless claim holds its lane while fresh and releases it when old", () => {
  const dir = mkRepo();
  try {
    const freshDir = mkLane(dir, "aaa-pipe", { merge: true });
    mkLane(dir, "bbb-old", { merge: true });
    mkdirSync(join(dir, "target", "queue"), { recursive: true });
    const now = new Date().toISOString();
    const past = new Date(Date.now() - 3 * 3600 * 1000).toISOString();
    writeFileSync(join(dir, "target", "queue", "claims.txt"),
      `AAA-PIPE ${now} crates/a\nBBB-OLD ${past} crates/b\n`);
    const out = lanes(dir, "prune", "--apply");
    assert.match(out, /KEEP aaa-pipe \(live-claim\)/, "a fresh pipeless claim holds its lane");
    assert.match(out, /PRUNED bbb-old/, "a pipeless claim over 2 h old holds nothing");
    assert.ok(existsSync(freshDir), "held worktree kept");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("corrupt lanes admin fails loud, never an empty list", () => {
  const dir = mkdtempSync(join(tmpdir(), "lanes-"));
  try {
    mkdirSync(join(dir, "target"), { recursive: true });
    for (const bad of [`{ not json`, `{"worktrees": null}`, `[]`]) {
      writeFileSync(join(dir, "target", "lanes-worktrees.json"), bad);
      const r = spawnSync(process.execPath, [fileURLToPath(new URL("./lanes.mjs", import.meta.url)), "list"], { encoding: "utf8", cwd: dir });
      assert.notEqual(r.status, 0, `corrupt admin must fail: ${bad}`);
      assert.match(r.stderr, /lanes admin/, "names the admin file");
      assert.ok(!r.stdout.includes("RESULT 0 lane"), "never silently reports empty");
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("isWarmLane never claims warm when the cache state is unreadable", () => {
  assert.equal(isWarmLane({ pool: 9999 }), false, "missing cache is cold, not warm");
  assert.equal(isWarmLane({ pool: 9999, createdAt: 1 }), false, "missing cache with a date is cold");
  const src = readFileSync(fileURLToPath(new URL("./lanes.mjs", import.meta.url)), "utf8");
  assert.ok(!src.includes("call it warm"), "no warm-on-error lie remains");
});

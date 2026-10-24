import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { applyHunks, commitMine, parseArgs, parseDiff, proofGap } from "./commit-mine.mjs";

const git = (cwd, ...args) => execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] });

// A repo whose file has two functions with the SAME body lines: the case that put a hunk into
// the wrong function on 2026-10-24.
const repo = () => {
  const dir = mkdtempSync(join(tmpdir(), "commit-mine-"));
  git(dir, "init", "-q", "-b", "master");
  git(dir, "config", "user.email", "t@t");
  git(dir, "config", "user.name", "t");
  git(dir, "config", "core.autocrlf", "false");
  const body = (name) => [`fn ${name}() {`, "    dir: f.dir,", "    age: 0,", "    latched: false,", "    speed: 0,", "}"];
  writeFileSync(join(dir, "a.rs"), [...body("strike"), "", ...body("anchor"), ""].join("\n"));
  git(dir, "add", "a.rs");
  git(dir, "commit", "-q", "-m", "base");
  return dir;
};

test("parseDiff and applyHunks rebuild HEAD plus the chosen hunks exactly", () => {
  const head = "a\nb\nc\nd\n";
  const diff = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,0 +2 @@\n+NEW\n@@ -3 +4 @@\n-c\n+C\n";
  const { header, hunks } = parseDiff(diff);
  assert.equal(header.length, 3);
  assert.equal(hunks.length, 2);
  assert.equal(applyHunks(head, hunks), "a\nNEW\nb\nC\nd\n");
  assert.equal(applyHunks(head, [hunks[1]]), "a\nb\nC\nd\n", "skipping hunk 1 keeps hunk 2 at its HEAD line");
});

test("a picked hunk lands in the right function even when two functions share every line", () => {
  const dir = repo();
  try {
    const text = readFileSync(join(dir, "a.rs"), "utf8").split("\n");
    // Mine: a field in `anchor` (second function). Someone else's: a field in `strike`.
    text.splice(10, 0, "    yank: false,");
    text.splice(3, 0, "    other: 1,");
    writeFileSync(join(dir, "a.rs"), text.join("\n"));
    const hunks = parseDiff(git(dir, "diff", "-U0", "HEAD", "--", "a.rs")).hunks;
    assert.equal(hunks.length, 2);
    const mine = hunks.findIndex((h) => h.add.includes("    yank: false,")) + 1;
    const res = commitMine({ msg: "mine", pick: [`a.rs#${mine}`], whole: [], cwd: dir, say: () => {} });
    assert.equal(res.code, 0, res.why);
    const committed = git(dir, "show", "HEAD:a.rs").split("\n");
    const anchorAt = committed.indexOf("fn anchor() {");
    assert.ok(committed.indexOf("    yank: false,") > anchorAt, "yank went into anchor");
    assert.ok(!committed.includes("    other: 1,"), "the other AI's hunk stayed out");
    assert.ok(readFileSync(join(dir, "a.rs"), "utf8").includes("    other: 1,"), "and stayed on disk");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("replacements and several picks around a skipped hunk land at their HEAD lines", () => {
  const dir = repo();
  try {
    const t = readFileSync(join(dir, "a.rs"), "utf8").split("\n");
    t[11] = "    speed: 9,"; // mine: a replacement in anchor
    t.splice(4, 0, "    theirs: 2,"); // someone else's insertion in strike
    t.splice(1, 0, "    first: 0,"); // mine: an insertion at the top of strike
    writeFileSync(join(dir, "a.rs"), t.join("\n"));
    const hunks = parseDiff(git(dir, "diff", "-U0", "HEAD", "--", "a.rs")).hunks;
    const n = (s) => hunks.findIndex((h) => h.add.includes(s)) + 1;
    const res = commitMine({ msg: "two of mine", pick: [`a.rs#${n("    first: 0,")}`, `a.rs#${n("    speed: 9,")}`], whole: [], cwd: dir, say: () => {} });
    assert.equal(res.code, 0, res.why);
    const want = [
      "fn strike() {", "    first: 0,", "    dir: f.dir,", "    age: 0,", "    latched: false,", "    speed: 0,", "}", "",
      "fn anchor() {", "    dir: f.dir,", "    age: 0,", "    latched: false,", "    speed: 9,", "}", "",
    ].join("\n");
    assert.equal(git(dir, "show", "HEAD:a.rs"), want);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("a picked deletion after a skipped insertion removes the right line", () => {
  const dir = repo();
  try {
    const t = readFileSync(join(dir, "a.rs"), "utf8").split("\n");
    t.splice(9, 1); // mine: delete `age: 0,` from anchor
    t.splice(2, 0, "    theirs: 3,"); // someone else's insertion in strike
    writeFileSync(join(dir, "a.rs"), t.join("\n"));
    const hunks = parseDiff(git(dir, "diff", "-U0", "HEAD", "--", "a.rs")).hunks;
    const mine = hunks.findIndex((h) => h.add.length === 0) + 1;
    const res = commitMine({ msg: "del", pick: [`a.rs#${mine}`], whole: [], cwd: dir, say: () => {} });
    assert.equal(res.code, 0, res.why);
    const lines = git(dir, "show", "HEAD:a.rs").split("\n");
    assert.equal(lines.filter((l) => l === "    age: 0,").length, 1, "one age line left (strike's)");
    assert.ok(!lines.includes("    theirs: 3,"));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("whole files and bad picks", () => {
  const dir = repo();
  try {
    writeFileSync(join(dir, "b.rs"), "new\n");
    assert.equal(commitMine({ msg: "new file", pick: ["b.rs#1"], whole: [], cwd: dir, say: () => {} }).code, 1, "a new file wants --whole");
    const res = commitMine({ msg: "new file", pick: [], whole: ["b.rs"], cwd: dir, say: () => {} });
    assert.equal(res.code, 0, res.why);
    assert.equal(git(dir, "show", "HEAD:b.rs"), "new\n");
    assert.equal(git(dir, "status", "--short", "--", "b.rs"), "", "the shared index follows the commit: no staged deletion left behind");
    writeFileSync(join(dir, "a.rs"), readFileSync(join(dir, "a.rs"), "utf8") + "tail\n");
    assert.equal(commitMine({ msg: "x", pick: ["a.rs#9"], whole: [], cwd: dir, say: () => {} }).code, 1, "a hunk number that does not exist refuses");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("whole binary, filtered and deleted files commit as git stores them", () => {
  const dir = repo();
  try {
    // A clean filter stands in for LFS: the staged blob differs from the disk bytes.
    git(dir, "config", "filter.up.clean", "tr a-z A-Z");
    writeFileSync(join(dir, ".gitattributes"), "*.big filter=up\n");
    writeFileSync(join(dir, "c.big"), "pointer me\n");
    writeFileSync(join(dir, "d.ogg"), Buffer.from([0x4f, 0x67, 0x67, 0x53, 0x00, 0xff, 0xfe, 0x80]));
    const res = commitMine({ msg: "assets", pick: [], whole: [".gitattributes", "c.big", "d.ogg"], cwd: dir, say: () => {} });
    assert.equal(res.code, 0, res.why);
    assert.equal(git(dir, "show", "HEAD:c.big"), "POINTER ME\n", "the filter ran, as for an LFS pointer");
    assert.equal(git(dir, "rev-parse", "HEAD:d.ogg").trim(), git(dir, "hash-object", "d.ogg").trim(), "binary bytes kept");
    rmSync(join(dir, "d.ogg"));
    const del = commitMine({ msg: "drop", pick: [], whole: ["d.ogg"], cwd: dir, say: () => {} });
    assert.equal(del.code, 0, del.why);
    assert.equal(git(dir, "ls-tree", "--name-only", "HEAD", "--", "d.ogg"), "", "the deletion committed");
    assert.equal(git(dir, "status", "--short", "--", "d.ogg", "c.big"), "");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("arguments", () => {
  assert.deepEqual(parseArgs(["--list", "a.rs", "b.rs"]).list, ["a.rs", "b.rs"]);
  const a = parseArgs(["-m", "msg", "--pick", "a.rs#1,a.rs#3", "--whole", "c.md", "--push"]);
  assert.deepEqual([a.pick, a.whole, a.push, a.error], [["a.rs#1", "a.rs#3"], ["c.md"], true, null]);
  assert.match(parseArgs(["-m", "msg"]).error, /nothing to commit/);
  assert.match(parseArgs(["-m", "msg", "--pick", "a.rs"]).error, /#N/);
  const spaced = parseArgs(["-m", "msg", "--whole", "a.rs", "a_tests.rs", "--push"]);
  assert.deepEqual([spaced.whole, spaced.push, spaced.error], [["a.rs", "a_tests.rs"], true, null], "a second path after --whole is taken, not dropped");
  assert.match(parseArgs(["-m", "msg", "--push", "stray.rs"]).error, /unknown argument stray.rs/);
});

test("a commit that leaves out a still-dirty file its green proof carried is refused; --gap-ok and a full commit pass", () => {
  const dir = repo();
  try {
    mkdirSync(join(dir, "target", "gh-check"), { recursive: true });
    const now = Date.now();
    const row = (t, only) => JSON.stringify({ t, head: "x", only, cmd: "cargo test -p a --lib", run: 7 });
    writeFileSync(join(dir, "target", "gh-check", "proved.jsonl"), [row(now - 4 * 3600_000, ["a.rs", "old.rs"]), row(now - 60_000, ["a.rs", "new.rs", "lib.rs#2"])].join("\n") + "\n");
    const live = new Set(["a.rs", "new.rs", "lib.rs", "old.rs"]);
    assert.deepEqual(proofGap(["a.rs"], { cwd: dir, now, live })?.extra, ["new.rs"], "the proof built with new.rs; a lib.rs#N pick is never demanded");
    assert.equal(proofGap(["a.rs", "new.rs"], { cwd: dir, now, live }), null, "the whole proved set is committed");
    assert.equal(proofGap(["a.rs"], { cwd: dir, now, live: new Set(["a.rs"]) }), null, "new.rs already on HEAD: no gap");
    assert.equal(proofGap(["zzz.rs"], { cwd: dir, now, live }), null, "no proof covers this commit");
    writeFileSync(join(dir, "a.rs"), "fn changed() {}\n");
    writeFileSync(join(dir, "new.rs"), "pub const X: u8 = 1;\n");
    const refused = commitMine({ msg: "m", pick: [], whole: ["a.rs"], cwd: dir, say: () => {} });
    assert.equal(refused.code, 1);
    assert.match(refused.why, /also carried new\.rs/);
    assert.equal(commitMine({ msg: "m", pick: [], whole: ["a.rs"], cwd: dir, gapOk: true, say: () => {} }).code, 0, "--gap-ok commits");
    assert.deepEqual(parseArgs(["-m", "x", "--whole", "a.rs", "--gap-ok"]).gapOk, true);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { exportTree } from "./head-build.mjs";

test("the HEAD export writes every file once, then only what the next commit changed", () => {
  const root = mkdtempSync(join(tmpdir(), "headbuild-"));
  const repo = join(root, "repo");
  const src = join(root, "src");
  const git = (...args) => execFileSync("git", ["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false", "-c", "user.name=hb", "-c", "user.email=hb@example.invalid", ...args], { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  try {
    execFileSync("git", ["init", "-q", repo]);
    writeFileSync(join(repo, "a.rs"), "a1\n");
    writeFileSync(join(repo, "b.rs"), "b1\n");
    writeFileSync(join(repo, "gone.rs"), "x\n");
    git("add", "."); git("commit", "-q", "-m", "one");
    exportTree("HEAD", src, repo);
    assert.equal(readFileSync(join(src, "a.rs"), "utf8"), "a1\n");
    const old = new Date(Date.now() - 3_600_000);
    utimesSync(join(src, "a.rs"), old, old);
    writeFileSync(join(repo, "b.rs"), "b2\n");
    git("rm", "-q", "gone.rs"); git("add", "."); git("commit", "-q", "-m", "two");
    writeFileSync(join(repo, "a.rs"), "dirty in the shared tree\n"); // never exported: HEAD only
    exportTree("HEAD", src, repo);
    assert.equal(readFileSync(join(src, "b.rs"), "utf8"), "b2\n", "a changed file is rewritten");
    assert.equal(readFileSync(join(src, "a.rs"), "utf8"), "a1\n", "the export is HEAD, not the dirty tree");
    assert.equal(statSync(join(src, "a.rs")).mtimeMs, old.getTime(), "an unchanged file keeps its mtime (cargo stays warm)");
    assert.equal(existsSync(join(src, "gone.rs")), false, "a file HEAD removed is removed");
    exportTree("HEAD", src, repo); // nothing changed: a no-op, not an error on the files already there
    writeFileSync(join(src, "b.rs"), "a tool wrote here\n");
    writeFileSync(join(repo, "b.rs"), "b3\n");
    git("add", "."); git("commit", "-q", "-m", "three");
    exportTree("HEAD", src, repo);
    assert.equal(readFileSync(join(src, "b.rs"), "utf8"), "b3\n", "a stray write in src never blocks the next export");
    rmSync(join(src, "b.rs"));
    exportTree("HEAD", src, repo);
    assert.equal(readFileSync(join(src, "b.rs"), "utf8"), "b3\n", "a file deleted by hand comes back");
  } finally { rmSync(root, { recursive: true, force: true, maxRetries: 3 }); }
});

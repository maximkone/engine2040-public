// Run: node --test tools/argv.test.mjs
// argv is the shared shell-safe spawn seam: junk-file detection stays a
// pure filename filter, dry runs never spawn, and real spawns keep argv
// arrays (no shell string). Pure function plus dry-run paths, so no repo
// file is touched.
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { junkFiles, runArgv } from "./argv.mjs";

test("junkFiles flags shell junk and nothing else", () => {
  const dir = mkdtempSync(join(tmpdir(), "argv-"));
  for (const n of ["=x", "chk-1.out", "board.md", "chk-.txt"]) writeFileSync(join(dir, n), "x");
  assert.deepEqual(junkFiles(dir).sort(), ["=x", "chk-1.out"]);
});

test("dry run never spawns and echoes the argv", () => {
  const r = runArgv("cargo", ["check", "-p", "x"], { dryRun: true });
  assert.equal(r.ok, true);
  assert.equal(r.status, 0);
  assert.equal(r.out, JSON.stringify(["cargo", "check", "-p", "x"]));
});

test("a failing command reports ok false with its status", () => {
  const r = runArgv(process.execPath, ["--no-such-flag-xyz"], {});
  assert.equal(r.ok, false);
  assert.notEqual(r.status, 0);
});
import { formatResultLine, proofProblem } from "./argv.mjs";

test("proofProblem: a RESULT proof names a command or a file result, never a book cite", () => {
  for (const bad of ["geaen | 15.1 Components", "Game Engine Architecture ch. 15.1", "see the book, p. 140", "it works", "<where>", "<command result, file or URL>", "", "   ", undefined]) {
    assert.ok(proofProblem(bad), `rejected: ${JSON.stringify(bad)}`);
  }
  assert.match(proofProblem(""), /^is empty/);
  assert.match(proofProblem("geaen | 15.1 Components"), /^names no command or file result/);
  for (const good of [
    "cargo test -p engine-render --lib torch 3 passed", "node --test tools/land.test.mjs", "rg -n gone crates/a/src/b.rs -> no hit", "Select-String -Path docs/a.md -Pattern x",
    "Test-Path target/reports/x.md", "git diff --stat", "pwsh -File x", "python tool", "crates/render/src/shadow.rs:42", "target/reports/pass-truth.md exists",
    "tests pass -> 6", "https://example.com/spec", "tools/land.test.mjs 12 pass", "final.json written",
  ]) assert.equal(proofProblem(good), null, good);
});

test("formatResultLine scripts the RESULT receipt deterministically", () => {
  const a = formatResultLine("done", "  lane x ", "node --test -> 3 passed");
  assert.equal(a, "RESULT: DONE - lane x | proof: node --test -> 3 passed");
  assert.equal(formatResultLine("DONE", "lane x", "node --test -> 3 passed"), a);
  assert.throws(() => formatResultLine("MAYBE", "x", "y"));
  assert.throws(() => formatResultLine("DONE", "", "y"));
});

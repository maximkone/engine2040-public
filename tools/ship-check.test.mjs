// HX10: the ship build must be reproducible, so the tree is named by content
// hash. These tests hold the tool to that: the same tree hashes the same, one
// changed byte moves the hash, a rename moves it, and a folder that cannot be
// hashed fails loudly.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { main } from "./ship-check.mjs";

// A temp tree: { "sub/a.txt": "bytes" }. The fixtures stay out of the repo.
function tree(files) {
  const root = mkdtempSync(join(tmpdir(), "ship-check-"));
  for (const [rel, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, rel)), { recursive: true });
    writeFileSync(join(root, rel), text);
  }
  return root;
}

// One run of the tool: the exit code, the printed lines and the error lines.
const run = (...argv) => {
  const lines = [];
  const errors = [];
  const code = main(argv, { out: (line) => lines.push(line), err: (line) => errors.push(line) });
  return { code, lines, text: lines.join("\n"), errors: errors.join("\n") };
};
// The last printed line carries the tree hash.
const treeLine = (text) => text.split("\n").at(-1);
// The line of one file: its content hash and its path.
const fileLine = (text, rel) => text.split("\n").find((line) => line.endsWith(`  ${rel}`));

test("the same tree prints the same tree hash", (t) => {
  const root = tree({ "a.txt": "alpha", "sub/b.txt": "beta" });
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const first = run("--print", root);
  const second = run("--print", root);
  assert.equal(first.code, 0);
  assert.equal(first.text, second.text);
  assert.match(treeLine(first.text), /^tree [0-9a-f]{64}$/);
  assert.equal(run("--hash", root).text, run("--hash", root).text);
});

test("one changed byte changes the tree hash and only that file's line", (t) => {
  const root = tree({ "a.txt": "alpha", "b.txt": "beta" });
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const before = run("--print", root);
  writeFileSync(join(root, "a.txt"), "alphb");
  const after = run("--print", root);
  assert.notEqual(treeLine(before.text), treeLine(after.text));
  assert.notEqual(fileLine(before.text, "a.txt"), fileLine(after.text, "a.txt"));
  assert.equal(fileLine(before.text, "b.txt"), fileLine(after.text, "b.txt"));
});

test("a renamed file changes the tree hash", (t) => {
  const root = tree({ "a.txt": "alpha" });
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const before = run("--print", root);
  renameSync(join(root, "a.txt"), join(root, "b.txt"));
  const after = run("--print", root);
  // The bytes are the same, so the content hash is the same; the path moved.
  assert.equal(fileLine(before.text, "a.txt").split("  ")[0], fileLine(after.text, "b.txt").split("  ")[0]);
  assert.notEqual(treeLine(before.text), treeLine(after.text));
});

test("the printed lines are sorted by path", (t) => {
  const root = tree({ "z.txt": "z", "m/b.txt": "b", "a.txt": "a", "m/a.txt": "c" });
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const { code, lines } = run("--print", root);
  assert.equal(code, 0);
  const paths = lines.slice(0, -1).map((line) => line.split("  ")[1]);
  assert.deepEqual(paths, ["a.txt", "m/a.txt", "m/b.txt", "z.txt"]);
});

test("a missing folder exits non-zero and names the folder", (t) => {
  const root = mkdtempSync(join(tmpdir(), "ship-check-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const missing = join(root, "not-there");
  const { code, errors } = run("--print", missing);
  assert.notEqual(code, 0);
  assert.ok(errors.includes(resolve(missing)), errors);
});

test("an empty folder is refused", (t) => {
  const root = mkdtempSync(join(tmpdir(), "ship-check-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const { code, errors } = run("--print", root);
  assert.notEqual(code, 0);
  assert.ok(errors.includes(resolve(root)), errors);
});

test("the hash flag prints only the tree hash", (t) => {
  const root = tree({ "a.txt": "alpha" });
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const { code, lines } = run("--hash", root);
  assert.equal(code, 0);
  assert.equal(lines.length, 1);
  assert.match(lines[0], /^[0-9a-f]{64}$/);
});

// HX10: the ship build must be reproducible, so the tree is named by content
// hash. These tests hold the tool to that: the same tree hashes the same, one
// changed byte moves the hash, a rename moves it, and a folder that cannot be
// hashed fails loudly. A Windows PE image carries two fields the linker writes
// fresh in every build (the time stamp and the CodeView debug GUID), so those
// are zeroed before the hash and the PDB is left out of the tree.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { contentHash, isPeImage, main, normalizePeImage } from "./ship-check.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

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

// --- a Windows PE image -------------------------------------------------
//
// A small PE image with the shape the normalizer reads: the DOS header, the
// PE header, one section, a debug directory with one CodeView entry and the
// code behind it. The fields the linker writes fresh (the time stamp, the
// GUID and the age) are parameters, so a test can build the same image twice
// with only those fields moved.
function peImage({ timeStamp = 0x11223344, guid = 1, age = 7, code = "CODE" } = {}) {
  const buf = Buffer.alloc(0x600);
  buf.writeUInt16LE(0x5a4d, 0); // MZ
  buf.writeUInt32LE(0x80, 0x3c); // e_lfanew
  buf.write("PE\0\0", 0x80, "latin1");
  const coff = 0x84;
  buf.writeUInt16LE(0x8664, coff); // machine: x86-64
  buf.writeUInt16LE(1, coff + 2); // one section
  buf.writeUInt32LE(timeStamp, coff + 4); // the time stamp
  buf.writeUInt16LE(240, coff + 16); // the optional header size
  const opt = coff + 20;
  buf.writeUInt16LE(0x20b, opt); // PE32+
  buf.writeUInt32LE(16, opt + 108); // the number of data directories
  const debug = opt + 112 + 6 * 8; // the debug directory entry
  buf.writeUInt32LE(0x2000, debug); // its RVA
  buf.writeUInt32LE(28, debug + 4); // its size: one entry
  const section = opt + 240;
  buf.write(".text\0\0\0", section, "latin1");
  buf.writeUInt32LE(0x200, section + 8); // the virtual size
  buf.writeUInt32LE(0x2000, section + 12); // the virtual address
  buf.writeUInt32LE(0x200, section + 16); // the raw size
  buf.writeUInt32LE(0x400, section + 20); // the raw pointer
  buf.write(code, 0x200, "latin1");
  const entry = 0x400;
  buf.writeUInt32LE(timeStamp, entry + 4); // the entry's own time stamp
  buf.writeUInt32LE(2, entry + 12); // the type: CodeView
  buf.writeUInt32LE(24, entry + 16); // the size of the record
  buf.writeUInt32LE(0x2000, entry + 20); // its RVA
  buf.writeUInt32LE(0x500, entry + 24); // its file offset
  buf.write("RSDS", 0x500, "latin1");
  buf.writeUInt32LE(guid, 0x504);
  buf.writeUInt32LE(guid * 7, 0x508);
  buf.writeUInt32LE(age, 0x510);
  return buf;
}

test("the linker fields the normalizer zeroes are the ones that move", () => {
  const image = peImage();
  assert.ok(isPeImage(image), "the fixture reads as a PE image");
  const normalized = normalizePeImage(image);
  // The COFF time stamp is zero.
  assert.equal(normalized.readUInt32LE(0x88), 0);
  // The GUID and the age are zero; the RSDS signature stays.
  assert.equal(normalized.toString("latin1", 0x500, 0x504), "RSDS");
  assert.equal(normalized.readUInt32LE(0x504), 0);
  assert.equal(normalized.readUInt32LE(0x508), 0);
  assert.equal(normalized.readUInt32LE(0x510), 0);
  // Every other byte is untouched.
  const copy = Buffer.from(image);
  copy.writeUInt32LE(0, 0x88); // the COFF time stamp
  copy.writeUInt32LE(0, 0x404); // the debug entry's own time stamp
  copy.fill(0, 0x504, 0x514); // the GUID and the age
  assert.deepEqual(normalized, copy);
});

test("two builds that differ only in the linker fields hash the same", (t) => {
  const root = mkdtempSync(join(tmpdir(), "ship-check-pe-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const first = peImage({ timeStamp: 0x11111111, guid: 1, age: 3 });
  const second = peImage({ timeStamp: 0x22222222, guid: 9, age: 5 });
  writeFileSync(join(root, "a.exe"), first);
  writeFileSync(join(root, "b.exe"), second);
  assert.notDeepEqual(first, second, "the raw bytes differ");
  // The tool hashes the image with those fields zeroed, so the two agree.
  assert.equal(contentHash(join(root, "a.exe")), contentHash(join(root, "b.exe")));
  const a = tree({});
  t.after(() => rmSync(a, { recursive: true, force: true }));
  writeFileSync(join(a, "game.exe"), first);
  const before = run("--print", a);
  writeFileSync(join(a, "game.exe"), second);
  const after = run("--print", a);
  assert.equal(treeLine(before.text), treeLine(after.text));
  // A changed code byte still moves the hash.
  writeFileSync(join(a, "game.exe"), peImage({ timeStamp: 0x22222222, guid: 9, age: 5, code: "DONE" }));
  assert.notEqual(treeLine(after.text), treeLine(run("--print", a).text));
});

test("a file that is not a PE image is hashed as it is", () => {
  const buf = Buffer.from("not a pe image at all");
  assert.equal(isPeImage(buf), false);
  assert.deepEqual(normalizePeImage(buf), buf);
});

test("a debug file is named apart and does not move the tree hash", (t) => {
  const root = tree({ "a.exe": "bytes", "a.pdb": "debug bytes" });
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const before = run("--print", root);
  assert.match(before.text, /^excluded {2}a\.pdb$/m);
  assert.doesNotMatch(before.text, /^excluded {2}a\.exe$/m);
  // The debug file does not move the tree hash.
  writeFileSync(join(root, "a.pdb"), "other debug bytes");
  assert.equal(treeLine(before.text), treeLine(run("--print", root).text));
  // The code does.
  writeFileSync(join(root, "a.exe"), "other bytes");
  assert.notEqual(treeLine(before.text), treeLine(run("--print", root).text));
});

test("the ship workflow is not cancelled by a newer push", () => {
  const workflow = readFileSync(join(ROOT, ".github", "workflows", "ship-check.yml"), "utf8").replace(/\r\n/g, "\n");
  // Two cold builds take longer than the gap between two pushes to the
  // mirror. A job that every newer push cancels would never finish, so the
  // proof would never land.
  assert.match(workflow, /cancel-in-progress: false/);
  assert.doesNotMatch(workflow, /cancel-in-progress: true/);
});

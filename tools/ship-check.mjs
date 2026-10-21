#!/usr/bin/env node
// HX10: the ship build must be reproducible, so its file tree is named by
// content hash. This walks a folder, hashes every file's bytes with SHA-256
// and prints one line per file (the content hash and the path relative to the
// folder root) plus a tree hash over all of them.
//
//   node tools/ship-check.mjs --print [dir]   the tree and the tree hash
//   node tools/ship-check.mjs --hash [dir]    only the tree hash, one line
//
// The tree hash covers the relative paths as well as the content hashes, so a
// renamed file moves it. Paths are sorted without the locale and written with
// "/", so the same tree prints the same bytes on every machine. The tool
// reads bytes only: it never looks at the clock, the locale or the timezone.
//
// A Windows PE image carries two fields that the linker writes fresh in every
// build: the COFF time stamp and the CodeView debug record (the PDB GUID and
// its age). The debug directory points at a PDB, and the PDB is not part of
// the shipped bundle, so a PE image is hashed with those fields zeroed and a
// `.pdb` file is left out of the tree and named in the receipt. Every other
// byte is compared as it is: the code, the imports and the data must match.
//
// Exit codes: 0 ok - 1 the tree cannot be hashed (no such folder, no files)
// - 2 usage.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

// One file's content hash: SHA-256 over its bytes.
export function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

// --- the Windows PE image -----------------------------------------------
//
// The layout the normalizer needs, straight from the PE format:
//   0x3c            e_lfanew: the offset of the PE header
//   pe + 4          the COFF header: machine, section count, time stamp
//   pe + 24         the optional header: magic, then the data directories
//   pe + 24 + opt   the section table: one 40 byte entry per section
const COFF_TIME_STAMP = 4; // offset in the COFF header
const DEBUG_DIRECTORY = 6; // the index of the debug directory in the data directories
const CODEVIEW = 2; // the debug directory entry type that holds the PDB GUID
const DEBUG_ENTRY = 28; // the size of one IMAGE_DEBUG_DIRECTORY
const CV_SIGNATURE = 4; // the RSDS signature before the GUID
const CV_GUID = 16; // the GUID: fresh in every link
const CV_AGE = 4; // the age after the GUID: fresh in every link

// True when the bytes start like a PE image: the MZ header and the PE
// signature behind e_lfanew.
export function isPeImage(buf) {
  if (buf.length < 0x40 || buf.readUInt16LE(0) !== 0x5a4d) return false;
  const pe = buf.readUInt32LE(0x3c);
  if (pe + 24 > buf.length) return false;
  return buf.readUInt32LE(pe) === 0x00004550;
}

// The file offset of an RVA, through the section table. -1 when the RVA
// belongs to no section.
function rvaToOffset(buf, pe, rva) {
  const coff = pe + 4;
  const sections = buf.readUInt16LE(coff + 2);
  const optSize = buf.readUInt16LE(coff + 16);
  const table = coff + 20 + optSize;
  for (let i = 0; i < sections; i++) {
    const entry = table + i * 40;
    if (entry + 40 > buf.length) break;
    const virtualSize = buf.readUInt32LE(entry + 8);
    const virtualAddress = buf.readUInt32LE(entry + 12);
    const rawSize = buf.readUInt32LE(entry + 16);
    const rawPointer = buf.readUInt32LE(entry + 20);
    const span = Math.max(virtualSize, rawSize);
    if (rva >= virtualAddress && rva < virtualAddress + span) return rawPointer + (rva - virtualAddress);
  }
  return -1;
}

// A copy of the image with the fields the linker writes fresh zeroed: the
// COFF time stamp, the time stamp of every debug directory entry, and the
// GUID and age of every CodeView record. Everything else is byte for byte
// the same, so two builds of the same code hash the same.
export function normalizePeImage(buf) {
  if (!isPeImage(buf)) return buf;
  const out = Buffer.from(buf);
  const pe = out.readUInt32LE(0x3c);
  const coff = pe + 4;
  out.writeUInt32LE(0, coff + COFF_TIME_STAMP);
  const opt = coff + 20;
  if (opt + 2 > out.length) return out;
  const magic = out.readUInt16LE(opt);
  // PE32+ keeps the data directories at optional + 112, PE32 at optional + 96.
  const ddBase = magic === 0x20b ? opt + 112 : opt + 96;
  const ddEntry = ddBase + DEBUG_DIRECTORY * 8;
  if (ddEntry + 8 > out.length) return out;
  const debugRva = out.readUInt32LE(ddEntry);
  const debugSize = out.readUInt32LE(ddEntry + 4);
  if (debugRva === 0 || debugSize === 0) return out;
  const dir = rvaToOffset(out, pe, debugRva);
  if (dir < 0) return out;
  const count = Math.floor(debugSize / DEBUG_ENTRY);
  for (let i = 0; i < count; i++) {
    const entry = dir + i * DEBUG_ENTRY;
    if (entry + DEBUG_ENTRY > out.length) break;
    out.writeUInt32LE(0, entry + COFF_TIME_STAMP);
    if (out.readUInt32LE(entry + 12) !== CODEVIEW) continue;
    const size = out.readUInt32LE(entry + 16);
    const raw = out.readUInt32LE(entry + 24);
    if (raw === 0 || size < CV_SIGNATURE + CV_GUID + CV_AGE) continue;
    if (raw + CV_SIGNATURE + CV_GUID + CV_AGE > out.length) continue;
    out.fill(0, raw + CV_SIGNATURE, raw + CV_SIGNATURE + CV_GUID);
    out.writeUInt32LE(0, raw + CV_SIGNATURE + CV_GUID);
  }
  return out;
}

// The content hash of one file: SHA-256 over its bytes, with the linker's
// fresh fields zeroed in a PE image. A file that is not a PE image is hashed
// as it is.
export function contentHash(path) {
  const bytes = readFileSync(path);
  return createHash("sha256").update(normalizePeImage(bytes)).digest("hex");
}

// The debug files that carry no code: a PDB names the build that made it, so
// it is left out of the tree and named in the receipt.
export function isDebugFile(rel) {
  return rel.toLowerCase().endsWith(".pdb");
}

// Path compare without the locale: code-unit order, so the printed order is
// the same on every machine.
function byPath(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

// Every regular file under `root`, with its path relative to `root` (always
// with "/" separators) and its content hash. Symlinks and other odd entries
// are skipped: what they point at is not part of the tree. The debug files
// are skipped too and named apart, so the receipt says what was left out.
export function walkTree(root) {
  const files = [];
  const excluded = [];
  const stack = [""];
  while (stack.length > 0) {
    const relDir = stack.pop();
    const absDir = relDir === "" ? root : join(root, ...relDir.split("/"));
    for (const entry of readdirSync(absDir, { withFileTypes: true })) {
      const rel = relDir === "" ? entry.name : `${relDir}/${entry.name}`;
      if (entry.isDirectory()) stack.push(rel);
      else if (entry.isFile()) {
        if (isDebugFile(rel)) excluded.push(rel);
        else files.push({ rel, hash: contentHash(join(absDir, entry.name)) });
      }
    }
  }
  files.sort((a, b) => byPath(a.rel, b.rel));
  excluded.sort(byPath);
  return { files, excluded };
}

// The tree hash: SHA-256 over "path\0content-hash\n" for every file, in path
// order. The path is inside the hash, so a rename moves the tree hash.
export function treeHash(files) {
  const hash = createHash("sha256");
  for (const file of [...files].sort((a, b) => byPath(a.rel, b.rel))) {
    hash.update(`${file.rel}\0${file.hash}\n`, "utf8");
  }
  return hash.digest("hex");
}

// The printed tree: one line per file, then the tree hash line. The files
// left out are named under the tree, so the receipt says what was not
// compared.
export function printTree(files, excluded = [], tree = treeHash(files)) {
  const lines = files.map((file) => `${file.hash}  ${file.rel}`);
  for (const rel of excluded) lines.push(`excluded  ${rel}`);
  lines.push(`tree ${tree}`);
  return lines.join("\n");
}

export const HELP = `ship-check.mjs — name a file tree by content hash

Usage:
  node tools/ship-check.mjs --print [dir]   the tree (one line per file) and the tree hash
  node tools/ship-check.mjs --hash [dir]    only the tree hash, one line

Every regular file under <dir> is hashed with SHA-256; the tree hash covers
the relative path and the content hash of each file, in path order, so a
changed byte or a renamed file moves it. With no <dir> the current folder is
used. Exit codes: 0 ok - 1 no such folder or no files - 2 usage.
`;

export function main(argv, { cwd = process.cwd(), out = console.log, err = console.error } = {}) {
  let print = false;
  let hash = false;
  let dir = null;
  for (const arg of argv) {
    if (arg === "--print") print = true;
    else if (arg === "--hash") hash = true;
    else if (arg === "--help" || arg === "-h") {
      out(HELP.trimEnd());
      return 0;
    } else if (arg.startsWith("-")) {
      err(`ship-check: unknown flag ${arg}\n${HELP}`);
      return 2;
    } else if (dir === null) dir = arg;
    else {
      err(`ship-check: at most one <dir>\n${HELP}`);
      return 2;
    }
  }
  if (print && hash) {
    err(`ship-check: --print and --hash do not go together\n${HELP}`);
    return 2;
  }
  if (!print && !hash) {
    err(`ship-check: need --print or --hash\n${HELP}`);
    return 2;
  }
  const root = resolve(cwd, dir ?? ".");
  let stat;
  try {
    stat = statSync(root);
  } catch {
    err(`ship-check: no such folder: ${root}`);
    return 1;
  }
  if (!stat.isDirectory()) {
    err(`ship-check: not a folder: ${root}`);
    return 1;
  }
  const { files, excluded } = walkTree(root);
  if (files.length === 0) {
    err(`ship-check: no files under ${root}`);
    return 1;
  }
  const tree = treeHash(files);
  if (hash) {
    out(tree);
    return 0;
  }
  for (const file of files) out(`${file.hash}  ${file.rel}`);
  for (const rel of excluded) out(`excluded  ${rel}`);
  out(`tree ${tree}`);
  return 0;
}

if (import.meta.main) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    console.error(`ship-check: ${error.message}`);
    process.exitCode = 2;
  }
}

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
// Exit codes: 0 ok - 1 the tree cannot be hashed (no such folder, no files)
// - 2 usage.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, resolve } from "node:path";

// One file's content hash: SHA-256 over its bytes.
export function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

// Path compare without the locale: code-unit order, so the printed order is
// the same on every machine.
function byPath(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

// Every regular file under `root`, with its path relative to `root` (always
// with "/" separators) and its content hash. Symlinks and other odd entries
// are skipped: what they point at is not part of the tree.
export function walkTree(root) {
  const files = [];
  const stack = [""];
  while (stack.length > 0) {
    const relDir = stack.pop();
    const absDir = relDir === "" ? root : join(root, ...relDir.split("/"));
    for (const entry of readdirSync(absDir, { withFileTypes: true })) {
      const rel = relDir === "" ? entry.name : `${relDir}/${entry.name}`;
      if (entry.isDirectory()) stack.push(rel);
      else if (entry.isFile()) files.push({ rel, hash: sha256File(join(absDir, entry.name)) });
    }
  }
  files.sort((a, b) => byPath(a.rel, b.rel));
  return files;
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

// The printed tree: one line per file, then the tree hash line.
export function printTree(files, tree = treeHash(files)) {
  return [...files.map((file) => `${file.hash}  ${file.rel}`), `tree ${tree}`].join("\n");
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
  const files = walkTree(root);
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

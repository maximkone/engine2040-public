// modmap: one line per tracked Rust file under crates/, grouped by crate, written to
// .opencode/modmap.md (AGENTS.md "Files inside a crate": Grep it, never read it whole).
//   node tools/modmap.mjs            print the map to stdout
//   node tools/modmap.mjs --write    rewrite .opencode/modmap.md
//   node tools/modmap.mjs --check    exit 1 when .opencode/modmap.md differs from a fresh map
// Exit codes: 0 ok, 1 stale map (--check), 2 usage.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const MAP = join(ROOT, ".opencode", "modmap.md");

// Package name from a crate's Cargo.toml text ("" when absent).
export function packageName(cargoToml) {
  const m = /^\s*name\s*=\s*"([^"]+)"/m.exec(cargoToml);
  return m ? m[1] : "";
}

// The file's own first `//!` line, trimmed; "(no //! line)" when it has none.
export function leadingDoc(text) {
  for (const line of text.split(/\r?\n/)) {
    const t = line.trim();
    if (t.startsWith("//!")) {
      const doc = t.slice(3).trim();
      if (doc) return doc;
      continue;
    }
    if (t === "" || t.startsWith("//") || t.startsWith("#![")) continue;
    break;
  }
  return "(no //! line)";
}

// lib.rs and main.rs first, then plain path order.
export function fileOrder(a, b) {
  const rank = (p) => (/^src\/(lib|main)\.rs$/.test(p) ? 0 : 1);
  return rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0);
}

// crates: [{ dir, pkg, files: [{ path (crate-relative), lines, doc }] }] -> markdown.
export function render(crates) {
  const fileCount = crates.reduce((n, c) => n + c.files.length, 0);
  const out = [
    "# modmap: engine2040",
    `_${crates.length} crates, ${fileCount} Rust files. One line per file: \`crate/path\` · lines · the file's own leading \`//!\` line. Read it with Grep (a crate name or a word), never whole. Generated from tracked files by \`node tools/modmap.mjs --write\`; do not edit by hand._`,
    "",
  ];
  for (const c of crates) {
    const lines = c.files.reduce((n, f) => n + f.lines, 0);
    out.push(`## ${c.dir} (${c.pkg}, ${c.files.length} files, ${lines} lines)`);
    for (const f of [...c.files].sort((a, b) => fileOrder(a.path, b.path))) {
      out.push(`- \`${c.dir}/${f.path}\` · ${f.lines} · ${f.doc}`);
    }
    out.push("");
  }
  return out.join("\n");
}

export function collect(root = ROOT) {
  const tracked = execFileSync("git", ["ls-files", "-z", "--", "crates/*.rs"], { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 })
    .split("\0")
    .filter(Boolean);
  const byCrate = new Map();
  for (const rel of tracked) {
    const parts = rel.split("/");
    if (parts.length < 3) continue;
    const dir = parts[1];
    const full = join(root, rel);
    if (!existsSync(full)) continue; // deleted in the working tree, not yet committed
    if (!byCrate.has(dir)) byCrate.set(dir, []);
    const text = readFileSync(full, "utf8");
    byCrate.get(dir).push({ path: parts.slice(2).join("/"), lines: text.split(/\r?\n/).length - (text.endsWith("\n") ? 1 : 0), doc: leadingDoc(text) });
  }
  return [...byCrate.keys()].sort().flatMap((dir) => {
    const toml = join(root, "crates", dir, "Cargo.toml");
    if (!existsSync(toml)) return [];
    return [{ dir, pkg: packageName(readFileSync(toml, "utf8")), files: byCrate.get(dir) }];
  });
}

function main(argv) {
  const mode = argv[0] ?? "--print";
  if (!["--print", "--write", "--check", "--help", "-h"].includes(mode) || argv.length > 1) {
    process.stderr.write("usage: node tools/modmap.mjs [--write | --check]\n");
    return 2;
  }
  if (mode === "--help" || mode === "-h") {
    process.stdout.write("usage: node tools/modmap.mjs [--write | --check]\n");
    return 0;
  }
  const text = render(collect());
  if (mode === "--print") process.stdout.write(text);
  if (mode === "--write") {
    writeFileSync(MAP, text);
    process.stdout.write(`modmap: wrote ${MAP}\n`);
  }
  if (mode === "--check") {
    const old = existsSync(MAP) ? readFileSync(MAP, "utf8").replace(/\r\n/g, "\n") : "";
    if (old !== text) {
      process.stdout.write("modmap: STALE, run node tools/modmap.mjs --write\n");
      return 1;
    }
    process.stdout.write("modmap: fresh\n");
  }
  return 0;
}

if (import.meta.main) process.exitCode = main(process.argv.slice(2));

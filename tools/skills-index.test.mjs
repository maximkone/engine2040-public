// Run: node --test tools/skills-index.test.mjs
// Skills index parity and skill-file hygiene: header N matches skills/*/SKILL.md,
// rows cover folders and name no missing folder, frontmatter is loadable and
// trigger-shaped, files stay short, and cites cannot rot (no path:line, every
// cited repo path exists unless the line says "cut", and a "cut" line names no
// path that exists). No SKILL.md tells the reader to run plain cargo test or
// clippy: those go through node tools/gh-check.mjs.
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const DIR = join(ROOT, "skills");
const INDEX = join(DIR, "SKILLS.md");
const MAX_SKILL_LINES = 250;
const MAX_DESCRIPTION = 300;

export function headerCount(text) {
  const m = /(\d+) skills in skills\//.exec(text);
  return m ? Number(m[1]) : NaN;
}

export function listSkills(dir = DIR) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && existsSync(join(dir, e.name, "SKILL.md")))
    .map((e) => e.name)
    .sort();
}

export function missingRows(text, names) {
  return names.filter((n) => !text.includes("| " + n + " |"));
}

// First cell of every table row in the "when" tables (before the old-name map).
export function indexRowNames(text) {
  const cut = text.indexOf("## Old name");
  const body = cut < 0 ? text : text.slice(0, cut);
  const names = [];
  for (const line of body.split(/\r?\n/)) {
    const m = /^\| ([a-z0-9-]+) \|/.exec(line);
    if (m && m[1] !== "skill") names.push(m[1]);
  }
  return names;
}

export function frontmatter(lines, file) {
  const errs = [];
  if ((lines[0] || "").trim() !== "---") return { errs: [file + ":1 missing opening ---"], vals: {} };
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === "---");
  if (end < 0) return { errs: [file + ":1 missing closing ---"], vals: {} };
  const vals = {};
  for (let i = 1; i < end; i++) {
    const line = lines[i];
    const c = line.indexOf(":");
    if (c < 0) continue;
    const key = line.slice(0, c).trim();
    const val = line.slice(c + 1).trim();
    if (key === "name" || key === "description") vals[key] = val.replace(/^(["'])(.*)\1$/, "$2");
    const quoted = val.startsWith('"') || val.startsWith("'");
    if (val && !quoted && val.includes(": ")) errs.push(file + ":" + (i + 1) + " unquoted colon-space");
  }
  for (const k of ["name", "description"]) if (!vals[k]) errs.push(file + ":1 empty " + k);
  return { errs, vals };
}

export function frontmatterErrors(lines, file) {
  return frontmatter(lines, file).errs;
}

// Line cites rot; ban `foo.rs:123` style in skill files.
export function lineCites(line) {
  return [...line.matchAll(/\b[\w./-]+\.(?:rs|wgsl|mjs|md|toml|yml|json):\d+/g)].map((m) => m[0]);
}

// Backticked repo paths (optionally `path::symbol`) on one line.
export function citedPaths(line) {
  const out = [];
  for (const m of line.matchAll(/`((?:crates|tools|docs|content|fixtures|tests|skills|examples|\.github|\.cargo)\/[^`\s:*<>{}]+?)(?:::[^`]*)?`/g)) {
    out.push(m[1].replace(/[),.;]+$/, ""));
  }
  return out;
}

// The line without backticked spans, so a path such as crates/combat/src/cut.rs is not the word "cut".
export function prose(line) {
  return line.replace(/`[^`]*`/g, " ");
}

// Cite errors for one line (no file prefix). A "cut" line must name no path that exists;
// any other line must name no path that is missing. A git-ignored output dir is exempt.
export function citeErrors(line, exists = (p) => existsSync(join(ROOT, p))) {
  if (/git-ignored/i.test(line)) return [];
  const cut = /\bcut\b/i.test(prose(line));
  const errs = [];
  for (const p of citedPaths(line)) {
    const there = exists(p);
    if (cut && there) errs.push(`says cut but ${p} exists`);
    if (!cut && !there) errs.push(`missing ${p}`);
  }
  return errs;
}

// A line that tells the reader to run a plain cargo test or clippy (not through gh-check).
export function plainCargoLine(line) {
  if (line.includes("node tools/gh-check.mjs")) return false;
  return /^\s*(?:[-*+]\s+|\d+\.\s+)?`?cargo (?:test|clippy)\b/.test(line);
}

function markdownFiles(dir) {
  const out = [];
  for (const n of readdirSync(dir)) {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) out.push(...markdownFiles(p));
    else if (n.endsWith(".md")) out.push(p);
  }
  return out;
}

test("helpers: header, rows, frontmatter, cites", () => {
  assert.equal(headerCount("# Skills - 26 skills in skills/"), 26);
  assert.deepEqual(missingRows("a | ok | b", ["ok", "nope"]), ["nope"]);
  assert.deepEqual(indexRowNames("| skill | when |\n| a-b | x |\n## Old name\n| zz | y |"), ["a-b"]);
  assert.ok(frontmatterErrors(["---", "name: ", "description: x", "---"], "f")[0].includes("empty name"));
  assert.ok(frontmatterErrors(["---", "name: a", "description: one: two", "---"], "f")[0].includes("colon-space"));
  assert.deepEqual(lineCites("see `crates/a/src/b.rs:12` and c.wgsl:4"), ["crates/a/src/b.rs:12", "c.wgsl:4"]);
  assert.deepEqual(citedPaths("open `crates/a/src/b.rs::sym` then `tools/x.mjs`."), ["crates/a/src/b.rs", "tools/x.mjs"]);
});

test("header N matches skills/*/SKILL.md count", () => {
  const n = headerCount(readFileSync(INDEX, "utf8"));
  const names = listSkills();
  assert.equal(n, names.length, "header N=" + n + " but disk has " + names.length);
});

test("every skill folder has a row, and no row names a missing folder", () => {
  const text = readFileSync(INDEX, "utf8");
  const names = listSkills();
  assert.deepEqual(missingRows(text, names), [], "folders with no SKILLS.md row");
  const stale = indexRowNames(text).filter((n) => !names.includes(n));
  assert.deepEqual(stale, [], "SKILLS.md rows that name no folder");
});

test("no skills hide in .opencode/skills (one index, visible to every AI)", () => {
  const oc = join(ROOT, ".opencode", "skills");
  const found = existsSync(oc) ? listSkills(oc) : [];
  assert.deepEqual(found, [], "move these into skills/ and index them");
});

test("every SKILL.md: loadable frontmatter, name = folder, trigger-shaped description, short", () => {
  const errs = [];
  for (const n of listSkills()) {
    const file = join(DIR, n, "SKILL.md");
    const lines = readFileSync(file, "utf8").split(/\r?\n/);
    const { errs: fm, vals } = frontmatter(lines, file);
    errs.push(...fm);
    if (vals.name && vals.name !== n) errs.push(`${n}: name '${vals.name}' is not the folder name`);
    if (vals.description && !vals.description.startsWith("Use when")) errs.push(`${n}: description must start with "Use when"`);
    if (vals.description && vals.description.length > MAX_DESCRIPTION) errs.push(`${n}: description ${vals.description.length} > ${MAX_DESCRIPTION} chars`);
    if (lines.length > MAX_SKILL_LINES) errs.push(`${n}: SKILL.md ${lines.length} > ${MAX_SKILL_LINES} lines (move tables to reference.md)`);
  }
  assert.deepEqual(errs, [], errs.join("\n"));
});

test("skill files never cite path:line; every cited repo path exists, and a cut line names none that exists", () => {
  const errs = [];
  for (const n of listSkills()) {
    for (const file of markdownFiles(join(DIR, n))) {
      const rel = file.slice(ROOT.length + 1);
      readFileSync(file, "utf8").split(/\r?\n/).forEach((line, i) => {
        for (const c of lineCites(line)) errs.push(`${rel}:${i + 1} line cite ${c}`);
        for (const e of citeErrors(line)) errs.push(`${rel}:${i + 1} ${e}`);
      });
    }
  }
  assert.deepEqual(errs, [], errs.join("\n"));
});

test("fixture: a cut line that names an existing path fails; a missing path on a cut line passes", () => {
  const exists = (p) => p === "crates/combat/src/cut.rs";
  assert.deepEqual(citeErrors("Limb loss: the cut tables `crates/combat/src/cut.rs`", exists), ["says cut but crates/combat/src/cut.rs exists"]);
  assert.deepEqual(citeErrors("The old file was cut; `crates/gone/x.rs` is gone.", exists), []);
  assert.deepEqual(citeErrors("Owns `crates/combat/src/cut.rs`", exists), []);
  assert.deepEqual(citeErrors("Owns `crates/gone/x.rs`", exists), ["missing crates/gone/x.rs"]);
});

test("fixture: a line that runs plain cargo test or clippy fails; a gh-check line and cargo check pass", () => {
  assert.equal(plainCargoLine("- `cargo test -p engine-net --lib x`"), true);
  assert.equal(plainCargoLine("cargo clippy -p engine-net --all-targets"), true);
  assert.equal(plainCargoLine("  1. `cargo test --locked -p x`"), true);
  assert.equal(plainCargoLine("- `node tools/gh-check.mjs cargo test -p engine-net --lib x`"), false);
  assert.equal(plainCargoLine("- `cargo check -p engine-net` to iterate"), false);
  assert.equal(plainCargoLine("A plain `cargo test` is denied in OpenCode."), false);
});

test("no SKILL.md line tells the reader to run plain cargo test or clippy (use node tools/gh-check.mjs)", () => {
  const errs = [];
  for (const n of listSkills()) {
    const file = join(DIR, n, "SKILL.md");
    const rel = file.slice(ROOT.length + 1);
    readFileSync(file, "utf8").split(/\r?\n/).forEach((line, i) => {
      if (plainCargoLine(line)) errs.push(`${rel}:${i + 1} plain cargo test/clippy: run it as node tools/gh-check.mjs cargo ...`);
    });
  }
  assert.deepEqual(errs, [], errs.join("\n"));
});

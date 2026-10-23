// Run: node tools/skills-lint.mjs [--json] [--init] [--tighten] [skills dir]
//
// The repo-side skill gate. It replaces the deleted center agentlint (E90):
// every skills/<name>/SKILL.md needs loadable front matter whose name is the
// folder, a description, a row in skills/SKILLS.md and every `references/<x>`
// it names on disk. A SKILL.md stays under 4096 bytes, or under its pinned size
// in tools/skills-lint.baseline.json, which may only shrink (`--tighten`).
//
// Exit 0 pass, 1 fail. `--json` prints one object instead of lines.
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const BASELINE = join(HERE, "skills-lint.baseline.json");

export const MAX_BYTES = 4096;
export const MAX_DESCRIPTION = 500;
const INDEX = "SKILLS.md";

// Bytes counted with LF endings, so a CRLF checkout does not fail a file.
export function skillBytes(text) {
  return Buffer.byteLength(text.replace(/\r\n/g, "\n"), "utf8");
}

export function listSkillDirs(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && existsSync(join(dir, e.name, "SKILL.md")))
    .map((e) => e.name)
    .sort();
}

// Front matter between the first two `---` lines, as { errs, values }.
export function frontmatter(text, file) {
  const lines = text.split(/\r?\n/);
  const errs = [];
  if ((lines[0] || "").trim() !== "---") return { errs: [file + ": missing opening ---"], values: {} };
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === "---");
  if (end < 0) return { errs: [file + ": missing closing ---"], values: {} };
  const values = {};
  for (let i = 1; i < end; i++) {
    const c = lines[i].indexOf(":");
    if (c < 0) continue;
    const key = lines[i].slice(0, c).trim();
    const val = lines[i].slice(c + 1).trim().replace(/^(["'])(.*)\1$/, "$2");
    if (key === "name" || key === "description") values[key] = val;
  }
  if (!values.name) errs.push(file + ": empty name");
  if (!values.description) errs.push(file + ": empty description");
  return { errs, values };
}

// Every relative references/<x> mention on one line, without trailing punctuation.
export function referenceMentions(line) {
  const out = [];
  for (const m of line.matchAll(/references\/([A-Za-z0-9._-]+(?:\/[A-Za-z0-9._-]+)*)/g)) {
    out.push(m[1].replace(/[.,;:]+$/, ""));
  }
  return out;
}

// Missing reference targets in one file; `exists` takes a path under skill references/.
export function referenceErrors(file, text, exists) {
  const errs = [];
  text.split(/\r?\n/).forEach((line, i) => {
    for (const name of referenceMentions(line)) {
      if (!exists(name)) errs.push(`${file}:${i + 1} missing references/${name}`);
    }
  });
  return errs;
}

// Skill rows of the index tables, before the old-name map (that map lists dead names).
export function indexRows(text) {
  const rows = [];
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    if (/^## Old name/i.test(lines[i])) break;
    const m = /^\|\s*([a-z0-9-]+)\s*\|/.exec(lines[i]);
    if (m && m[1] !== "skill" && !/^-+$/.test(m[1])) rows.push({ name: m[1], line: i + 1 });
  }
  return rows;
}

// One skill's own errors, each prefixed by its SKILL.md path.
export function skillErrors(name, text, file, pinned = MAX_BYTES) {
  const errs = [];
  const bytes = skillBytes(text);
  if (bytes > pinned) errs.push(`${file}: ${bytes} bytes > ${MAX_BYTES}` + (pinned < MAX_BYTES ? ` (baseline ${pinned})` : ""));
  const fm = frontmatter(text, file);
  errs.push(...fm.errs);
  if (fm.values.name && fm.values.name !== name) errs.push(`${file}: front matter name '${fm.values.name}' is not the folder`);
  const d = fm.values.description;
  if (d && d.length > MAX_DESCRIPTION) errs.push(`${file}: description ${d.length} > ${MAX_DESCRIPTION} chars`);
  return errs;
}

function markdownUnder(dir) {
  const out = [];
  for (const n of readdirSync(dir)) {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) out.push(...markdownUnder(p));
    else if (n.endsWith(".md")) out.push(p);
  }
  return out;
}

// Lint one skills dir. `baseline.bytes` pins a skill's size; a pin only goes down.
export function lint(dir, baseline = {}, options = {}) {
  const root = options.root || dirname(dir);
  const label = (p) => relative(root, p).split(sep).join("/");
  const pinned = baseline.bytes || {};
  const names = listSkillDirs(dir);
  const errors = [];
  let largest = { name: "", bytes: 0 };
  const over = [];

  for (const name of names) {
    const file = join(dir, name, "SKILL.md");
    const text = readFileSync(file, "utf8");
    const bytes = skillBytes(text);
    if (bytes > largest.bytes) largest = { name, bytes };
    if (bytes > MAX_BYTES) over.push(name);
    errors.push(...skillErrors(name, text, label(file), pinned[name] ?? MAX_BYTES));
    const exists = (p) => existsSync(join(dir, name, "references", p));
    for (const f of markdownUnder(join(dir, name))) {
      errors.push(...referenceErrors(label(f), readFileSync(f, "utf8"), exists));
    }
  }

  const indexFile = join(dir, INDEX);
  if (!existsSync(indexFile)) {
    errors.push(label(indexFile) + ": missing index file");
  } else {
    const rows = indexRows(readFileSync(indexFile, "utf8"));
    for (const name of names) {
      if (!rows.some((r) => r.name === name)) errors.push(label(indexFile) + ": no row for " + name);
    }
    for (const r of rows) {
      if (!names.includes(r.name)) errors.push(label(indexFile) + ":" + r.line + " row '" + r.name + "' names no skill dir");
    }
  }

  return { ok: errors.length === 0, skills: names.length, largest, over: over.length, errors };
}

export function readBaseline(file = BASELINE) {
  return existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : {};
}

// Pin every skill at or above the cap, so it may only shrink from here.
export function baselineFrom(dir) {
  const bytes = {};
  for (const name of listSkillDirs(dir)) {
    const n = skillBytes(readFileSync(join(dir, name, "SKILL.md"), "utf8"));
    if (n > MAX_BYTES) bytes[name] = n;
  }
  return {
    note: "Pinned SKILL.md bytes above " + MAX_BYTES + "; a pin only goes down. Lower them with `node tools/skills-lint.mjs --tighten` as skills are split.",
    bytes,
  };
}

export function tighten(dir, baseline) {
  const next = { ...(baseline.bytes || {}) };
  for (const name of Object.keys(next)) {
    const file = join(dir, name, "SKILL.md");
    if (!existsSync(file)) {
      delete next[name];
      continue;
    }
    const n = skillBytes(readFileSync(file, "utf8"));
    if (n <= MAX_BYTES) delete next[name];
    else next[name] = Math.min(next[name], n);
  }
  return { ...baseline, bytes: next };
}

function print(json, report, dirLabel) {
  if (json) {
    console.log(JSON.stringify({ ...report, dir: dirLabel }));
    return;
  }
  for (const e of report.errors) console.log(e);
  const head = report.ok ? "PASS skills-lint" : "FAIL skills-lint";
  const tail = `: ${report.skills} skills, largest ${report.largest.bytes} bytes (${report.largest.name})`;
  const pinned = report.over ? `, ${report.over} over ${MAX_BYTES} pinned` : "";
  console.log(head + tail + pinned + (report.ok ? "" : `, ${report.errors.length} failures`));
}

function main(argv) {
  const json = argv.includes("--json");
  const init = argv.includes("--init");
  const doTighten = argv.includes("--tighten");
  const dirArg = argv.find((a) => !a.startsWith("--"));
  const dir = dirArg ? join(ROOT, dirArg) : join(ROOT, "skills");

  if (init) {
    if (existsSync(BASELINE)) {
      console.log("FAIL skills-lint: " + relative(ROOT, BASELINE).split(sep).join("/") + " exists; use --tighten");
      return 1;
    }
    writeFileSync(BASELINE, JSON.stringify(baselineFrom(dir), null, 2) + "\n");
    console.log("wrote " + relative(ROOT, BASELINE).split(sep).join("/"));
    return 0;
  }

  let baseline = readBaseline();
  if (doTighten) {
    baseline = tighten(dir, baseline);
    writeFileSync(BASELINE, JSON.stringify(baseline, null, 2) + "\n");
  }

  const report = lint(dir, baseline, { root: ROOT });
  print(json, report, relative(ROOT, dir).split(sep).join("/"));
  return report.ok ? 0 : 1;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exitCode = main(process.argv.slice(2));

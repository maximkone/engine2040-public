// Run: node --test tools/skills-index.test.mjs
// Skills index parity: header N matches skills/*/SKILL.md, rows cover folders, frontmatter loads.
import assert from "node:assert/strict";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const DIR = join(ROOT, "skills");
const INDEX = join(DIR, "SKILLS.md");
export function headerCount(text) {
  const m = /(\d+) skills in skills\//.exec(text);
  return m ? Number(m[1]) : NaN;
}
export function listSkills(dir = DIR) {
  return readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory() && existsSync(join(dir, e.name, "SKILL.md"))).map((e) => e.name).sort();
}
export function missingRows(text, names) {
  return names.filter((n) => !text.includes("| " + n + " |"));
}
export function frontmatterErrors(lines, file) {
  const errs = [];
  if ((lines[0] || "").trim() !== "---") return [file + ":1 missing opening ---"];
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === "---");
  if (end < 0) return [file + ":1 missing closing ---"];
  const vals = {};
  for (let i = 1; i < end; i++) {
    const line = lines[i];
    const c = line.indexOf(":");
    if (c < 0) continue;
    const key = line.slice(0, c).trim();
    const val = line.slice(c + 1).trim();
    if (key === "name" || key === "description") vals[key] = val;
    const quoted = val.startsWith('"') || val.startsWith("'");
    if (val && !quoted && val.includes(": ")) errs.push(file + ":" + (i + 1) + " unquoted colon-space");
  }
  for (const k of ["name", "description"]) if (!vals[k]) errs.push(file + ":1 empty " + k);
  return errs;
}
test("header parses N skills in skills/", () => {
  assert.equal(headerCount("# Skills - 14 agents, 85 skills in skills/ and 8 in .opencode/skills/"), 85);
});
test("missingRows names gaps, frontmatter flags colon-space", () => {
  assert.deepEqual(missingRows("a | ok | b", ["ok", "nope"]), ["nope"]);
  assert.ok(frontmatterErrors(["---", "name: ", "description: x", "---"], "f")[0].includes("empty name"));
  assert.ok(frontmatterErrors(["---", "name: a", "description: one: two", "---"], "f")[0].includes("colon-space"));
});
test("header N matches skills/*/SKILL.md count", () => {
  const text = readFileSync(INDEX, "utf8");
  const names = listSkills();
  const n = headerCount(text);
  console.log("skills index: header " + n + ", disk " + names.length);
  assert.equal(n, names.length, "header N=" + n + " but disk has " + names.length);
});
test("every skill folder has a table row", () => {
  const text = readFileSync(INDEX, "utf8");
  const missing = missingRows(text, listSkills());
  console.log("skills rows: missing " + missing.length);
  assert.deepEqual(missing, [], "missing SKILLS.md rows:\n" + missing.join("\n"));
});
test("every SKILL.md frontmatter loads (name, description, no bare colon-space)", () => {
  const errs = [];
  for (const n of listSkills()) {
    const file = join(DIR, n, "SKILL.md");
    errs.push(...frontmatterErrors(readFileSync(file, "utf8").split(/\r?\n/), file));
  }
  console.log("skills frontmatter: checked " + listSkills().length + ", errors " + errs.length);
  assert.deepEqual(errs, [], "bad frontmatter:\n" + errs.join("\n"));
});

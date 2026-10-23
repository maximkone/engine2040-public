// Run: node --test tools/skills-lint.test.mjs
// Fixtures live in a temp dir; the real skills tree is never touched.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { lint, MAX_DESCRIPTION } from "./skills-lint.mjs";

const SKILL = (name, body = "") => `---\nname: ${name}\ndescription: "Use when a fixture needs a skill."\n---\n\n# ${name}\n${body}`;

// Build <tmp>/skills/<name>/SKILL.md for each entry, plus the index rows.
function fixture(skills, rows) {
  const root = mkdtempSync(join(tmpdir(), "skills-lint-"));
  const dir = join(root, "skills");
  mkdirSync(dir, { recursive: true });
  for (const [name, text] of Object.entries(skills)) {
    mkdirSync(join(dir, name, "references"), { recursive: true });
    writeFileSync(join(dir, name, "SKILL.md"), text);
  }
  const list = rows ?? Object.keys(skills);
  const table = ["# Skills", "", "| skill | when |", "|---|---|", ...list.map((n) => `| ${n} | a fixture row |`)];
  writeFileSync(join(dir, "SKILLS.md"), table.join("\n") + "\n");
  return { root, dir };
}

function runText(skills, rows) {
  const { root, dir } = fixture(skills, rows);
  const report = lint(dir, {}, { root });
  rmSync(root, { recursive: true, force: true });
  return report;
}

test("a good fixture passes", () => {
  const { root, dir } = fixture({
    "alpha-skill": SKILL("alpha-skill", "\nRead `references/notes.md` first.\n"),
    "beta-skill": SKILL("beta-skill"),
  });
  writeFileSync(join(dir, "alpha-skill", "references", "notes.md"), "# notes\n");
  const report = lint(dir, {}, { root });
  rmSync(root, { recursive: true, force: true });
  assert.deepEqual(report.errors, []);
  assert.equal(report.ok, true);
  assert.equal(report.skills, 2);
  assert.equal(report.largest.name, "alpha-skill");
});

test("a SKILL.md over the byte cap fails with its size", () => {
  const big = SKILL("big-skill", "\n" + "x".repeat(5000) + "\n");
  const report = runText({ "big-skill": big });
  assert.equal(report.ok, false);
  assert.equal(report.errors.length, 1);
  assert.match(report.errors[0], /skills[\\/]big-skill[\\/]SKILL\.md: 5\d+ bytes > 4096$/);
});

test("a name that is not the folder, a missing description or a long one fails", () => {
  const wrong = SKILL("other-name");
  assert.match(runText({ "wrong-name": wrong }).errors.join("\n"), /wrong-name[\\/]SKILL\.md: front matter name 'other-name' is not the folder/);
  const empty = "---\nname: no-desc\n---\n\n# no-desc\n";
  assert.match(runText({ "no-desc": empty }).errors.join("\n"), /no-desc[\\/]SKILL\.md: empty description/);
  const long = SKILL("long-desc").replace('"Use when a fixture needs a skill."', "Use when " + "y".repeat(MAX_DESCRIPTION));
  assert.match(runText({ "long-desc": long }).errors.join("\n"), new RegExp("description \\d+ > " + MAX_DESCRIPTION + " chars"));
});

test("an index that misses a row, or names a gone folder, fails", () => {
  const missing = runText({ "hidden-skill": SKILL("hidden-skill") }, []);
  assert.match(missing.errors.join("\n"), /SKILLS\.md: no row for hidden-skill/);
  const stale = runText({ "only-skill": SKILL("only-skill") }, ["only-skill", "gone-skill"]);
  assert.match(stale.errors.join("\n"), /SKILLS\.md:6 row 'gone-skill' names no skill dir/);
});

test("a references/ path that is not on disk fails with file and line", () => {
  const text = SKILL("ref-skill", "\nRead `references/gone.md` for the detail.\n");
  const report = runText({ "ref-skill": text });
  assert.equal(report.ok, false);
  assert.deepEqual(report.errors, ["skills/ref-skill/SKILL.md:8 missing references/gone.md"]);
});

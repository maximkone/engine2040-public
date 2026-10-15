import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const skillPath = join(dir, "SKILL.md");

test("skill file exists at the listed path", () => {
  assert.equal(existsSync(skillPath), true, "SKILL.md must exist beside the test");
});

const text = readFileSync(skillPath, "utf8");
const lines = text.split(/\r?\n/);

test("frontmatter carries name systematic-debug", () => {
  assert.match(text, /name:\s*systematic-debug/);
});

test("iron law marker is present", () => {
  assert.match(text, /NO FIXES WITHOUT ROOT CAUSE/);
});

test("four phase headings are present", () => {
  assert.match(text, /Root Cause/);
  assert.match(text, /Pattern/);
  assert.match(text, /Hypothesis/);
  assert.match(text, /Implementation/);
});

test("3-strikes stop marker is present", () => {
  assert.match(text, /3-strikes stop/);
});

test("cross-links to TDD and verification skills", () => {
  assert.match(text, /test-driven-development/);
  assert.match(text, /verification-before-completion/);
});

test("body stays within 120 lines", () => {
  assert.ok(lines.length <= 120, `SKILL.md has ${lines.length} lines, cap is 120`);
});

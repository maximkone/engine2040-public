// Run: node --test tools/evidence-pack.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const SKILL = join(ROOT, "skills", "evidence-pack", "SKILL.md");
const EXAMPLE = join(ROOT, "skills", "evidence-pack", "EXAMPLE.md");

const read = (p) => readFileSync(p, "utf8");
const lines = (p) => read(p).split("\n").length;

test("both skill files exist at the packet paths", () => {
  assert.equal(existsSync(SKILL), true, "skills/evidence-pack/SKILL.md exists");
  assert.equal(existsSync(EXAMPLE), true, "skills/evidence-pack/EXAMPLE.md exists");
});

test("frontmatter names the skill", () => {
  assert.match(read(SKILL), /^name: evidence-pack/m);
});

test("pack law marker is present", () => {
  assert.match(read(SKILL), /NO CLAIM WITHOUT/);
});

test("all 5 pack-part markers are present", () => {
  const text = read(SKILL).toLowerCase();
  assert.ok(text.includes("command"), "command part");
  assert.ok(text.includes("exit"), "exit part");
  assert.ok(text.includes("sha") || text.includes("hash"), "SHA/hash part");
  assert.ok(text.includes("excerpt"), "excerpt part");
  assert.ok(text.includes("verdict"), "verdict part");
});

test("replay marker is present", () => {
  assert.match(read(SKILL).toLowerCase(), /replay/);
});

test("DRAFT stop marker is present", () => {
  assert.match(read(SKILL), /DRAFT/);
});

test("EXAMPLE carries all 5 parts filled", () => {
  const text = read(EXAMPLE);
  const low = text.toLowerCase();
  assert.ok(low.includes("node --test tools/fnv1a64.test.mjs"), "filled command line");
  assert.ok(/(^|\s)exit:? 0/m.test(low), "filled exit code");
  assert.ok(/[0-9a-f]{64}/.test(low), "filled output SHA");
  assert.ok(low.includes("pass 7") && low.includes("fail 0"), "filled excerpt");
  assert.ok(/PASS 7 pass 0 fail/.test(text), "filled verdict");
});

test("size cap: both files fit 150 lines combined", () => {
  const total = lines(SKILL) + lines(EXAMPLE);
  assert.ok(total <= 150, `SKILL.md plus EXAMPLE.md is ${total} lines, cap is 150`);
});

// Run: node --test skills/engine-review/SKILL.test.mjs
// Guards reason-less FAIL verdicts: every FAIL carries its reason on the same line.
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const SKILL = fileURLToPath(new URL("./SKILL.md", import.meta.url));

function skillText() {
  return readFileSync(SKILL, "utf8").replace(/\r\n/g, "\n");
}

function failLines(text) {
  return text.split("\n").filter((line) => line.includes("FAIL")); // keep FAIL reason lines only
}

function bareFailLines(text) {
  return failLines(text).filter((line) => !/reason/i.test(line));
}

test("every FAIL line carries its reason on the same line", () => {
  const text = skillText();
  const lines = failLines(text);
  assert.ok(lines.length > 0, "expected FAIL verdict lines with reason in SKILL.md");
  const bare = bareFailLines(text);
  assert.equal(bare.length, 0, `bare FAIL without reason: ${bare.join(" | ").slice(0, 300)}`);
});

test("CHECK and VERDICT templates require FAIL with reason", () => {
  const text = skillText();
  assert.match(text, /CHECK: FAIL - <reason>/);
  assert.match(text, /FAIL - <reason> \| BLOCKED - <reason>/);
});

test("bare FAIL verdicts are rejected with reason", () => {
  const text = skillText();
  assert.match(text, /Every FAIL.*reason.*bare FAIL.*rejected/);
  assert.match(text, /a bare FAIL.*no reason is rejected/);
});

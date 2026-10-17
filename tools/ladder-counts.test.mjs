// Run: node --test tools/ladder-counts.test.mjs
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { LADDER, countRungs, render } from "./ladder-counts.mjs";

const SAMPLE = [
  "- CARD | P1 | idea | arXiv | not run | -",
  "- CARD ACCEPTED | P2 | idea two | arXiv | not run | -",
  " - SPIKE | P? | spike | repo | numbers | -",
  "- LANDED | P3 | landed | repo | numbers | row X, abc1234",
  "- REJECTED | P4 | dead | repo | - | reason",
  "- CARD | no pillar so not a line",
  "prose - CARD | P1 | mid-line is not a rung",
].join("\n");

test("counts each rung line by its first word", () => {
  const c = countRungs(SAMPLE);
  assert.equal(c.get("CARD"), 2);
  assert.equal(c.get("SPIKE"), 1);
  assert.equal(c.get("LANDED"), 1);
  assert.equal(c.get("REJECTED"), 1);
  assert.equal(c.get("BAR"), 0);
});

test("render prints one line per rung plus the total", () => {
  const out = render(countRungs(SAMPLE));
  assert.match(out, /^CARD\s+2$/m);
  assert.match(out, /^TOTAL\s+5$/m);
});

test("the real ladder has open CARD and SPIKE lines and counts x2 identically", () => {
  const text = readFileSync(LADDER, "utf8");
  const a = render(countRungs(text));
  assert.equal(a, render(countRungs(text)));
  const c = countRungs(text);
  assert.ok(c.get("CARD") > 0 && c.get("SPIKE") > 0, a);
});

// Run: node --test docs/INDEX.test.mjs
// Speed fix f08: Start chain mandated five docs (94,642B) before work.
// Quick start names ROADMAP + ARCHITECTURE first (~33.9KB).
import assert from "node:assert/strict";
import { readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const INDEX_PATH = join(ROOT, "docs", "INDEX.md");

test("quick start names two docs first", () => {
  const head = readFileSync(INDEX_PATH, "utf8").split("\n").slice(0, 8).join("\n");
  assert.match(head, /Quick start:/, "header names the quick start");
  assert.match(head, /ROADMAP\.md.*ARCHITECTURE\.md/, "quick start is ROADMAP then ARCHITECTURE");
  assert.doesNotMatch(head, /Start: `PLAN-REVIEW\.md`/, "old five-doc Start chain is gone");
});

test("quick-start pair stays under the old five-doc total", () => {
  const road = statSync(join(ROOT, "docs", "ROADMAP.md")).size;
  const arch = statSync(join(ROOT, "docs", "ARCHITECTURE.md")).size;
  const quick = road + arch;
  assert.ok(quick < 94642, `quick start ${quick} bytes should be under old 94642`);
});

test("full context chain is still listed after the quick start", () => {
  const text = readFileSync(INDEX_PATH, "utf8").replace(/\r\n/g, "\n");
  assert.ok(text.includes("PLAN-REVIEW.md"), "PLAN-REVIEW stays listed");
  assert.ok(text.includes("VISION.md"), "VISION stays listed");
  assert.ok(text.includes("AI-NATIVE.md"), "AI-NATIVE stays listed");
});

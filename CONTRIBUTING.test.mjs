// Run: node --test CONTRIBUTING.test.mjs
// Speed fix f09: docs/INDEX.md Execution line must use an explicit
// root-relative path for the root-only CONTRIBUTING.md, so a docs-relative
// resolver never implies docs/CONTRIBUTING.md.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, normalize } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const ROOT = dirname(fileURLToPath(import.meta.url));
const INDEX_PATH = join(ROOT, "docs", "INDEX.md");

function executionLine() {
  const text = readFileSync(INDEX_PATH, "utf8");
  const line = text.split("\n").find((l) => l.startsWith("Execution:"));
  assert.ok(line, "docs/INDEX.md must keep an Execution line");
  return line;
}

test("Execution line uses an explicit root path for CONTRIBUTING.md", () => {
  const line = executionLine();
  assert.ok(line.includes("`../CONTRIBUTING.md`"), "Execution line must contain `../CONTRIBUTING.md`, got: " + line);
});

test("no bare CONTRIBUTING.md token implies a docs path", () => {
  const line = executionLine();
  const bare = line.includes("`CONTRIBUTING.md`");
  assert.equal(bare, false, "bare `CONTRIBUTING.md` implies docs/CONTRIBUTING.md, got: " + line);
});

test("explicit path resolves to the root-only file", () => {
  assert.equal(existsSync(join(ROOT, "CONTRIBUTING.md")), true, "root CONTRIBUTING.md must exist");
  assert.equal(existsSync(join(ROOT, "docs", "CONTRIBUTING.md")), false, "docs/CONTRIBUTING.md must stay missing (root-only)");
  const abs = normalize(join(ROOT, "docs", "../CONTRIBUTING.md"));
  assert.equal(existsSync(abs), true, "docs/../CONTRIBUTING.md must resolve to the root file");
});

// The AI door chapter quotes no tool total and no schema budget: both move with every
// new verb, so the page points at the two files that pin them (E71).
// Run: node --test tools/ai-door-book.test.mjs
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (rel) => readFileSync(join(ROOT, rel), "utf8").replace(/\r\n/g, "\n");

test("ai-door.md names the pin test and the budget constant and quotes no stale total", () => {
  const book = read("docs/book/ai-door.md");
  for (const name of ["tool_count_pin.rs", "tool_budget_exhaustion.rs", "TOOL_LIST_BUDGET_CHARS", "CORE_TOOL_NAMES"]) {
    assert.ok(book.includes(name), `the chapter points at ${name}`);
  }
  assert.doesNotMatch(book, /exactly (twenty|thirteen|\d+) tools/i, "no exact tool total in prose");
  assert.doesNotMatch(book, /: (twenty|thirteen) MCP tools/i, "no total in the title");
  assert.doesNotMatch(book, /\b\d{2},\d{3}\b|\b\d{2}_\d{3}\b/, "no schema budget number in prose");
  assert.doesNotMatch(book, /tool_names\(\) -> \[/, "no array length in the tool_names signature");
});

test("the files the chapter points at exist and still own the names", () => {
  const mcp = read("crates/endpoint/src/mcp.rs");
  assert.match(mcp, /pub const TOOL_LIST_BUDGET_CHARS: usize = /);
  assert.match(mcp, /pub const CORE_TOOL_NAMES: \[&str; \d+\]/);
  for (const rel of ["crates/endpoint/tests/it/tool_count_pin.rs", "crates/endpoint/tests/it/tool_budget_exhaustion.rs"]) {
    assert.ok(existsSync(join(ROOT, rel)), `${rel} exists`);
  }
});

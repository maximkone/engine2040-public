// Run: node --test Cargo.test.mjs
// Bare cargo commands resolve default-members only: this pins the fast
// default to the root package and the documented `-p`/`--workspace`
// paths, so a bare `cargo test` runs one suite instead of twenty-nine.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoDir = dirname(fileURLToPath(import.meta.url));

export function readDefaultMembers(text) {
  const block = /default-members\s*=\s*\[([^\]]*)\]/.exec(text);
  if (!block) throw new Error("Cargo.toml has no default-members block");
  return [...block[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

function loadCargoToml() {
  return readFileSync(join(repoDir, "Cargo.toml"), "utf8");
}

test("default is the root package only, not the old twenty-nine", () => {
  assert.deepEqual(readDefaultMembers(loadCargoToml()), ["."]);
});

test("the opt-in shells stay out of the default set", () => {
  const members = new Set(readDefaultMembers(loadCargoToml()));
  assert.ok(!members.has("crates/editor"), "crates/editor must stay out of default-members (use -p)");
  assert.ok(!members.has("crates/endpoint"), "crates/endpoint must stay out of default-members (use -p)");
});

test("the fast -p path is documented beside the member list", () => {
  const text = loadCargoToml();
  assert.match(text, /cargo check\/test -p <crate>/);
  assert.match(text, /cargo test -p engine-editor/);
  assert.match(text, /cargo build -p engine-endpoint --features mcp/);
  assert.match(text, /cargo \.\.\. --workspace/);
});

test("the exclusion note names the real out-of-default set", () => {
  const text = loadCargoToml();
  assert.match(text, /engine-chaos/);
  assert.match(text, /crates\/\*/);
});

test("every default member path exists", () => {
  for (const member of readDefaultMembers(loadCargoToml())) {
    const dir = member === "." ? repoDir : join(repoDir, member);
    assert.ok(existsSync(dir), `default member missing on disk: ${member}`);
  }
});

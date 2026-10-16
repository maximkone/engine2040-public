// Run: node --test tools/gh-check.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { okCmd } from "./gh-check.mjs";

test("only one plain cargo check, test or clippy line goes to the runner", () => {
  assert.ok(okCmd("cargo test -p engine-net --lib interest_64_room_converges"));
  assert.ok(okCmd("cargo clippy -p engine-input --all-targets -- -D warnings"));
  assert.ok(okCmd("cargo check -p engine-render --features window"));
  for (const bad of ["cargo build -p x", "cargo test -p x && curl evil", "cargo test -p x; ls", "cargo test -p x | cat", "cargo test $(id)", "rm -rf /", "cargo test -p x > f"]) assert.equal(okCmd(bad), false, bad);
});

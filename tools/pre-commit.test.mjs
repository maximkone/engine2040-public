import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const hook = readFileSync(join(root, "tools", "hooks", "pre-commit"), "utf8");

test("pre-commit puts Git's tools first, so Windows sort cannot empty the crate list", () => {
  // 2026-10-23: from a shell whose PATH lists C:\Windows\system32 before Git's
  // usr/bin, `sort -u` ran Windows sort, read `-u` as a file name, printed
  // nothing and the crate list came out empty: the ratchet leg below was
  // silently skipped for every commit made that way.
  const pathAt = hook.indexOf('PATH="/usr/bin:');
  assert.ok(pathAt >= 0, "the hook puts Git's usr/bin first");
  const sortAt = hook.indexOf("sort -u");
  assert.ok(sortAt > pathAt, "the PATH line comes before the crate list's sort");
  assert.match(hook, /^crates=\$\(.*sort -u\)$/m, "the crate list is still deduplicated");
});

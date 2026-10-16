// OPT36 unify test: real tree passes, seeded extra feature fails.
import assert from "node:assert/strict";
import test from "node:test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkUnify, findViolations, parseDeps } from "./feature-unify.mjs";
const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
test("real tree passes", () => {
  assert.deepEqual(checkUnify(repoDir), []);
});
test("seeded extra feature fails", () => {
  const union = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png"] }');
  const member = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png", "avif"] }');
  const bad = findViolations("seed", member, union);
  assert.ok(bad.length === 1 && bad[0].includes("avif"), "seeded avif must fail");
});
test("subset passes", () => {
  const union = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png", "hdr"] }');
  const member = parseDeps('[dependencies]\nimage = { version = "0.25", features = ["png"] }');
  assert.deepEqual(findViolations("seed", member, union), []);
});

// Arch fitness test: a seeded cycle fails, the real graph passes.
// Rule from docs/ARCHITECTURE.md:9. Short plain English.
import assert from "node:assert/strict";
import test from "node:test";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { findCycle, loadAllowedEdges } from "./arch-fitness.mjs";

test("seeded cycle fails as a -> b -> a", () => {
  const cycle = findCycle(new Map([["a", ["b"]], ["b", ["a"]]]));
  assert.ok(cycle, "seeded cycle must fail");
  assert.equal(cycle.join(" -> "), "a -> b -> a");
  console.log(`FAIL cycle: ${cycle.join(" -> ")}`);
});

test("acyclic chain passes", () => {
  const cycle = findCycle(new Map([["a", ["b"]], ["b", ["c"]], ["c", []]]));
  assert.equal(cycle, null);
});

test("self loop fails", () => {
  const cycle = findCycle({ a: ["a"] });
  assert.ok(cycle);
  assert.equal(cycle.join(" -> "), "a -> a");
});

test("real allowedEdges graph passes", () => {
  const repoDir = join(dirname(fileURLToPath(import.meta.url)), "..");
  const edges = loadAllowedEdges(repoDir);
  assert.ok(edges.size > 10, "real graph has rows");
  assert.equal(findCycle(edges), null);
});

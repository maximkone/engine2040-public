import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { GOLD_PATH, generate } from "./pick-gold.mjs";

test("pick-gold: the fixture is exactly what the writer produces, twice", () => {
  const first = generate();
  assert.equal(generate(), first, "deterministic");
  assert.equal(readFileSync(GOLD_PATH, "utf8"), first, "fixtures/pick/gold.txt is stale: run node tools/pick-gold.mjs");
});

test("pick-gold: 100 tasks, a mix of entity ids and floor points", () => {
  const tasks = generate().split("\n").filter((l) => l.startsWith("task "));
  assert.equal(tasks.length, 100);
  const ids = tasks.filter((l) => l.split(" ")[3] === "id").length;
  assert.ok(ids >= 10 && ids <= 90, `${ids} entity answers`);
  assert.ok(generate().includes("actor 1 crate") && generate().includes("actor 3 crate"), "a duplicated name is in the scene");
});

// Run: node --test tools/dep-rebuild-count.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { DEFAULT_LANES, copyCounts, dependencyUnits, laneArgs, parseArgs, parseCompiling, verdict } from "./dep-rebuild-count.mjs";

test("the lane command is the affected gate's compile step plus --no-run", () => {
  assert.deepEqual(laneArgs({ crates: ["engine-physics"], jobs: 4 }), [
    "test", "-j", "4", "--locked", "-p", "engine-physics", "--all-targets", "--no-run",
  ]);
  assert.deepEqual(laneArgs({ crates: ["a", "b"], jobs: 2 }), [
    "test", "-j", "2", "--locked", "-p", "a", "-p", "b", "--all-targets", "--no-run",
  ]);
});

test("a cargo Compiling line reads as name and version", () => {
  const out = parseCompiling([
    "    Checking engine-core v0.1.0 (/c/core)",
    "   Compiling serde v1.0.229",
    "   Compiling serde_json v1.0.151",
    "warning: something",
  ].join("\n"));
  assert.deepEqual(out, ["serde v1.0.229", "serde_json v1.0.151"]);
});

test("the lane's own crate is not one of its dependency units", () => {
  const units = dependencyUnits(["engine-physics v0.1.0", "serde v1.0.229"], "engine-physics");
  assert.deepEqual(units, ["serde v1.0.229"]);
});

test("a dependency wanted by two lanes with different features is compiled twice", () => {
  // Cargo prints a Compiling line only when it builds a unit: the second lane
  // rebuilt serde_json (another feature set) and reused serde.
  const copies = copyCounts([
    ["serde_json v1.0.151", "serde v1.0.229"],
    ["serde_json v1.0.151"],
    ["serde v1.0.229"],
  ]);
  assert.equal(copies.get("serde_json"), 2, "serde_json has two copies");
  assert.equal(copies.get("serde"), 2, "serde was built twice");
});

test("the verdict keeps the hack only when the count drops by a third", () => {
  assert.equal(verdict({ before: 100, after: 60 }).keep, true);
  assert.equal(verdict({ before: 100, after: 66 }).keep, true, "66 of 100 is a third less");
  assert.equal(verdict({ before: 100, after: 70 }).keep, false, "a third is the bar: 70 is not enough");
  assert.equal(verdict({ before: 100, after: 150 }).keep, false);
  assert.equal(verdict({ before: 0, after: 0 }).keep, false);
  assert.match(verdict({ before: 100, after: 60 }).reason, /40% less/);
});

test("the arguments are read without surprises", () => {
  const a = parseArgs(["--lane", "engine-core", "--jobs", "2", "--json"]);
  assert.equal(a.error, null);
  assert.equal(a.jobs, 2);
  assert.equal(a.json, true);
  assert.deepEqual(a.crates, ["engine-core"], "the named lanes replace the default set");
  assert.equal(parseArgs([]).crates.length, 5, "the default is five lanes");
  assert.equal(parseArgs(["--jobs", "0"]).error, "--jobs needs a whole number of 1 or more");
  assert.equal(parseArgs(["--lane"]).error, "--lane needs a crate name");
  assert.equal(parseArgs(["--nope"]).error, "unknown flag --nope");
  assert.deepEqual(DEFAULT_LANES.length, 5, "the row counts five lane builds");
  assert.equal(parseArgs([]).force, false);
  assert.equal(parseArgs(["--force"]).force, true);
});

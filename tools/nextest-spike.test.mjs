// Run: node --test tools/nextest-spike.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { failedTests, laneArgs, nextestArgs, parseArgs, passedTests, readNextestVersion, retriedTests, testArgs, verdict } from "./nextest-spike.mjs";

const CARGO_TEST_OUTPUT = [
  "running 3 tests",
  "test engine_core::fixed::add ... ok",
  "test engine_core::rng::next ... FAILED",
  "test engine_core::hash::of ... ok",
  "failures:",
  "    engine_core::rng::next",
].join("\n");

const NEXTEST_OUTPUT = [
  "    PASS [   0.004s] engine-core engine_core::fixed::add",
  "    FAIL [   0.006s] engine-core engine_core::rng::next",
  "    PASS [   0.003s] engine-core engine_core::hash::of",
  "    TRY 2 FAIL [   0.006s] engine-core engine_core::rng::next",
  "Summary [   0.013s] 3 tests run: 2 passed, 1 failed",
].join("\n");

test("the lane is one crate, one filter and four jobs", () => {
  assert.deepEqual(laneArgs({ crate: "engine-core", filter: "", jobs: 4 }), ["-j", "4", "--locked", "-p", "engine-core", "--lib"]);
  assert.deepEqual(testArgs({ crate: "engine-core", filter: "fixed", jobs: 4 }), ["test", "-j", "4", "--locked", "-p", "engine-core", "--lib", "fixed"]);
  assert.deepEqual(nextestArgs({ crate: "engine-core", filter: "", jobs: 2 }), ["nextest", "run", "-j", "2", "--locked", "-p", "engine-core", "--lib"]);
});

test("a missing nextest is cargo's own error, never a guess", () => {
  const fake = (cmd, args) => ({ status: 101, stdout: "", stderr: "error: no such command: `nextest`\n" });
  const got = readNextestVersion(fake);
  assert.equal(got.ok, false);
  assert.match(got.error, /no such command/);
  assert.equal(got.version, null);
});

test("an installed nextest reports its version", () => {
  const fake = () => ({ status: 0, stdout: "cargo-nextest 0.9.90\n", stderr: "" });
  const got = readNextestVersion(fake);
  assert.equal(got.ok, true);
  assert.equal(got.version, "cargo-nextest 0.9.90");
});

test("the failed and passed tests are read from both runners", () => {
  assert.deepEqual(failedTests(CARGO_TEST_OUTPUT), ["engine_core::rng::next"]);
  assert.deepEqual(passedTests(CARGO_TEST_OUTPUT), ["engine_core::fixed::add", "engine_core::hash::of"]);
  assert.deepEqual(failedTests(NEXTEST_OUTPUT), ["engine-core engine_core::rng::next"]);
  assert.equal(passedTests(NEXTEST_OUTPUT).length, 2);
});

test("the retries are reported, never hidden", () => {
  const retries = retriedTests(NEXTEST_OUTPUT);
  assert.equal(retries.length, 1);
  assert.equal(retries[0].try, 2);
  assert.equal(retries[0].result, "FAIL");
  assert.match(retries[0].test, /engine_core::rng::next/);
});

test("the verdict keeps nextest only when it is faster with the same failures", () => {
  const ok = verdict({ testSeconds: 100, nextestSeconds: 60, testFailures: ["a"], nextestFailures: ["a"], retries: [] });
  assert.equal(ok.keep, true);
  const slower = verdict({ testSeconds: 60, nextestSeconds: 100, testFailures: [], nextestFailures: [], retries: [] });
  assert.equal(slower.keep, false);
  assert.match(slower.reasons.join(" "), /not faster/);
  const different = verdict({ testSeconds: 100, nextestSeconds: 60, testFailures: ["a"], nextestFailures: ["b"], retries: [] });
  assert.equal(different.keep, false);
  assert.match(different.reasons.join(" "), /failures differ/);
});

test("the arguments are read without surprises", () => {
  assert.equal(parseArgs([]).error, "--crate needs a crate name");
  assert.equal(parseArgs(["--crate"]).error, "--crate needs a crate name");
  assert.equal(parseArgs(["--crate", "engine-core", "--jobs", "0"]).error, "--jobs needs a whole number of 1 or more");
  assert.equal(parseArgs(["--crate", "engine-core", "--nope"]).error, "unknown flag --nope");
  const ok = parseArgs(["--crate", "engine-core", "--filter", "fixed", "--jobs", "2"]);
  assert.equal(ok.error, null);
  assert.equal(ok.crate, "engine-core");
  assert.equal(ok.filter, "fixed");
  assert.equal(ok.jobs, 2);
  assert.equal(parseArgs(["--crate", "engine-core"]).force, false);
  assert.equal(parseArgs(["--crate", "engine-core", "--force"]).force, true);
});

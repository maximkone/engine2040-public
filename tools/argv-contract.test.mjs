// argv contract: pins the shared argv seam export shapes (AW3).
// Run: node --test tools/argv-contract.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import * as argv from "./argv.mjs";

test("argv exposes the shared seam functions", () => {
  assert.equal(typeof argv.junkFiles, "function");
  assert.equal(typeof argv.fail, "function");
  assert.equal(typeof argv.usageFail, "function");
  assert.equal(typeof argv.runArgv, "function");
  assert.equal(typeof argv.formatResultLine, "function");
});

test("runArgv dry-run echoes argv without spawning", () => {
  const r = argv.runArgv("cargo", ["check", "-p", "x"], { dryRun: true });
  assert.equal(r.ok, true);
  assert.equal(r.status, 0);
  assert.equal(r.out, JSON.stringify(["cargo", "check", "-p", "x"]));
});

test("formatResultLine normalizes the RESULT receipt", () => {
  assert.equal(argv.formatResultLine("done", "lane x", "node --test -> ok"), "RESULT: DONE - lane x | proof: node --test -> ok");
  assert.throws(() => argv.formatResultLine("MAYBE", "x", "y"));
});

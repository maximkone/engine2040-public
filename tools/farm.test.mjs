// Run: node --test tools/farm.test.mjs
import assert from "node:assert/strict";
import test from "node:test";
import { errorLines } from "./farm.mjs";

test("errorLines keeps defect lines per job and drops the noise", () => {
  const log = [
    "test (render)\ttest render\t2026-10-16T06:45:19.6Z \u001b[1m\u001b[91merror\u001b[0m: could not compile `engine-render`",
    "test (render)\ttest render\t2026-10-16T06:45:20.1Z    Compiling engine-core v0.1.0",
    "quick\tfmt\t2026-10-16T06:45:21.0Z Diff in /home/runner/work/x/src/lib.rs:1:",
    "bad line without tabs",
  ].join("\n");
  const by = errorLines(log);
  assert.deepEqual([...by.keys()], ["test (render)", "quick"]);
  assert.equal(by.get("test (render)").length, 1);
  assert.match(by.get("test (render)")[0], /^error: could not compile/);
  assert.match(by.get("quick")[0], /^Diff in /);
});

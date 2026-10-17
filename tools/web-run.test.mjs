// node --test tools/web-run.test.mjs: the argument contract of the isolated web-build runner.
import test from "node:test";
import assert from "node:assert/strict";
import { newBranch, parseArgs, validBranch } from "./web-run.mjs";

test("paths and --file pairs are collected, backslashes and ./ are normalised", () => {
  const a = parseArgs(["--budget", "840", "crates\\web", "./tools/web-input-check.mjs", "--file", "Cargo.lock=C:/tmp/lock=x"]);
  assert.equal(a.error, null);
  assert.deepEqual(a.paths, ["crates/web", "tools/web-input-check.mjs"]);
  assert.deepEqual(a.files, [["Cargo.lock", "C:/tmp/lock=x"]]);
  assert.equal(a.budget, 840);
});

test("a run with nothing to overlay is refused, so no one sends the dirty shared tree by accident", () => {
  assert.match(parseArgs([]).error, /name the paths/);
});

test("paths outside the repo and unknown flags are refused", () => {
  assert.match(parseArgs(["../x"]).error, /inside the repo/);
  assert.match(parseArgs(["/etc/passwd"]).error, /inside the repo/);
  assert.match(parseArgs(["C:/x"]).error, /inside the repo/);
  assert.match(parseArgs(["--wat", "a"]).error, /unknown flag/);
  assert.match(parseArgs(["--file", "nope", "a"]).error, /--file needs/);
  assert.match(parseArgs(["--budget", "0", "a"]).error, /--budget needs/);
});

test("--wait takes only a branch this tool made", () => {
  const b = newBranch(1_800_000_000_000);
  assert.ok(validBranch(b), b);
  assert.equal(parseArgs(["--wait", b]).error, null);
  assert.match(parseArgs(["--wait", "main"]).error, /--wait needs/);
  assert.match(parseArgs(["--wait", "web-build-a"]).error, /--wait needs/);
});

// tools/quiet.test.mjs: dedicated parseArgs pin for the perf quiet window.
// bins-quiet.test.mjs covers the happy path; this file pins the edges:
// missing separator, times clamp, --out handling, and arg passthrough.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseArgs } from "./quiet.mjs";

test("quiet: no separator or no command is refused", () => {
  assert.equal(parseArgs([]), null);
  assert.equal(parseArgs(["perf.exe"]), null);
  assert.equal(parseArgs(["--times", "3"]), null);
  assert.equal(parseArgs(["--"]), null);
  assert.equal(parseArgs(["--times", "3", "--out", "x.csv"]), null);
});

test("quiet: minimal command defaults to once with no out file", () => {
  assert.deepEqual(parseArgs(["--", "perf.exe"]), { times: 1, out: null, cmd: "perf.exe", args: [] });
});

test("quiet: times clamps to 1..10 and non-numbers fall back to 1", () => {
  assert.equal(parseArgs(["--times", "0", "--", "a"]).times, 1);
  assert.equal(parseArgs(["--times", "-4", "--", "a"]).times, 1);
  assert.equal(parseArgs(["--times", "abc", "--", "a"]).times, 1);
  assert.equal(parseArgs(["--times", "--", "--", "a"]).times, 1);
  assert.equal(parseArgs(["--times", "1", "--", "a"]).times, 1);
  assert.equal(parseArgs(["--times", "10", "--", "a"]).times, 10);
  assert.equal(parseArgs(["--times", "11", "--", "a"]).times, 10);
  assert.equal(parseArgs(["--times", "99", "--", "a"]).times, 10);
});

test("quiet: out file is passed through and args follow the command", () => {
  assert.deepEqual(
    parseArgs(["--times", "2", "--out", "target/p-{n}.csv", "--", "perf.exe", "-v", "--fast"]),
    { times: 2, out: "target/p-{n}.csv", cmd: "perf.exe", args: ["-v", "--fast"] },
  );
  assert.deepEqual(parseArgs(["--out", "o.csv", "--", "a", "b", "c"]), {
    times: 1,
    out: "o.csv",
    cmd: "a",
    args: ["b", "c"],
  });
});

test("quiet: a second separator belongs to the command args", () => {
  assert.deepEqual(parseArgs(["--", "a", "--", "b"]), { times: 1, out: null, cmd: "a", args: ["--", "b"] });
});

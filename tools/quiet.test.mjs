// tools/quiet.test.mjs: dedicated parseArgs pin for the perf quiet window.
// bins-quiet.test.mjs covers the happy path; this file pins the edges:
// missing separator, times clamp, --out handling, and arg passthrough.
// 2026-10-17: also pins the load sidecar and --require-quiet, so a LOADED
// window cannot be read as a claimable one from target/reports/ alone.
import { test } from "node:test";
import assert from "node:assert/strict";
import { parseArgs, verdict, sidecarPath, sidecarText, refused, wantQuiet } from "./quiet.mjs";

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

// Pure helpers only here: no window, no cargo, no timing.
test("quiet: the verdict is QUIET only when every slot is held and no build runs", () => {
  assert.equal(
    verdict({ held: 3, max: 3, running: 0, others: [] }),
    "QUIET (3 of 3 cargo slots held, no cargo or rustc running)",
  );
  assert.match(
    verdict({ held: 3, max: 3, running: 1, others: [] }),
    /^LOADED 0 of 3 slots busy, 1 cargo\/rustc running/,
  );
  assert.match(
    verdict({ held: 2, max: 3, running: 0, others: [{ seat: "perf", cmd: "perf.exe" }] }),
    /^LOADED 1 of 3 slots busy, 0 cargo\/rustc running \(perf/,
  );
  assert.match(verdict({ held: 1, max: 3, running: 2, others: [] }), /^LOADED 0 of 3 slots busy, 2 cargo\/rustc running/);
});

test("quiet: the load sidecar replaces {n} with load and carries time and command", () => {
  assert.equal(sidecarPath("target/reports/perf-{n}.csv"), "target/reports/perf-load.csv");
  assert.equal(sidecarPath("target/reports/perf-{n}.txt"), "target/reports/perf-load.txt");
  assert.equal(sidecarPath("out.txt"), "out.txt");
  const line = verdict({ held: 2, max: 3, running: 1, others: [] });
  assert.equal(
    sidecarText(line, "perf.exe", ["--scene", "arena"], "2026-10-17T03:00:00Z"),
    `${line}\n2026-10-17T03:00:00Z\nperf.exe --scene arena\n`,
  );
});

test("quiet: a LOADED verdict is refused under --require-quiet, a QUIET one is not", () => {
  const quiet = verdict({ held: 3, max: 3, running: 0, others: [] });
  const loaded = verdict({ held: 3, max: 3, running: 4, others: [] });
  assert.equal(refused(loaded, true), true);
  assert.equal(refused(quiet, true), false);
  assert.equal(refused(loaded, false), false);
  assert.equal(refused(quiet, false), false);
});

test("quiet: --require-quiet is read before the separator only", () => {
  assert.equal(wantQuiet(["--require-quiet", "--out", "o-{n}.csv", "--", "perf.exe"]), true);
  assert.equal(wantQuiet(["--out", "o-{n}.csv", "--", "perf.exe"]), false);
  assert.equal(wantQuiet(["--", "arena.exe", "--require-quiet"]), false);
  assert.equal(wantQuiet([]), false);
});

test("quiet: parseArgs keeps its four keys and passes --require-quiet through", () => {
  assert.deepEqual(Object.keys(parseArgs(["--require-quiet", "--out", "o-{n}.csv", "--", "a"])), ["times", "out", "cmd", "args"]);
  assert.deepEqual(parseArgs(["--require-quiet", "--", "a"]), { times: 1, out: null, cmd: "a", args: [] });
});

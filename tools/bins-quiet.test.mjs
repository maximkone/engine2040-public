// tools/bins.mjs and tools/quiet.mjs: the loop's binary freshness check and
// the perf seat's quiet window (2026-10-10).
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { binStatus, BINS } from "./bins.mjs";
import { parseArgs } from "./quiet.mjs";
import { RUN_BUILD } from "../.opencode/seat-guard.mjs";

test("bins: a missing binary is MISSING with a rebuild line the runner guard allows", () => {
  const root = mkdtempSync(join(tmpdir(), "bins-"));
  const [row] = binStatus(root, [BINS[1]]);
  assert.equal(row.state, "MISSING");
  assert.match(row.build, /cargo build --release -p engine-game --features app --bin staged_fight$/);
  assert.ok(RUN_BUILD.test(row.build), "the printed rebuild line passes the run-seat cargo rule");
});

test("bins: a present binary outside git is STALE with rebuild and stderr warn", () => {
  const root = mkdtempSync(join(tmpdir(), "bins-"));
  mkdirSync(join(root, "target", "release"), { recursive: true });
  writeFileSync(join(root, "target", "release", "perf.exe"), "");
  let err = "";
  const orig = console.error;
  console.error = (...a) => { err += a.join(" ") + "\n"; };
  try {
    const [row] = binStatus(root, [BINS[2]]);
    assert.equal(row.state, "STALE");
    assert.match(row.build, /--bin perf$/);
    assert.match(err, /marking STALE/);
  } finally {
    console.error = orig;
  }
});

test("quiet: args split at --, times clamp to 1..10, no command is refused", () => {
  assert.deepEqual(parseArgs(["--times", "3", "--out", "target/p-{n}.csv", "--", "perf.exe", "-v"]), { times: 3, out: "target/p-{n}.csv", cmd: "perf.exe", args: ["-v"] });
  assert.equal(parseArgs(["--times", "99", "--", "a"]).times, 10);
  assert.equal(parseArgs(["--", "a"]).times, 1);
  assert.equal(parseArgs(["perf.exe"]), null);
  assert.equal(parseArgs(["--"]), null);
});

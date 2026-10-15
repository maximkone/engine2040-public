// Run: node --test tools/bins.test.mjs
// bins dedicated pin: BINS shape plus MISSING and FRESH rows (bins-quiet also
// covers quiet.mjs; this file pins bins.mjs alone for the no-test gap).
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { BINS, binStatus } from "./bins.mjs";

test("bins: BINS lists the five loop binaries with rebuild lines", () => {
  assert.equal(BINS.map((b) => b.exe).join(","), "engine2040,staged_fight,perf,arena,e2040-editor");
  for (const b of BINS) assert.match(b.build, /^cargo build --release -p engine/);
});

test("bins: a missing binary is MISSING with its rebuild line", () => {
  const root = mkdtempSync(join(tmpdir(), "bins-"));
  const [row] = binStatus(root, [BINS[1]]);
  assert.equal(row.state, "MISSING");
  assert.match(row.build, /--bin staged_fight$/);
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

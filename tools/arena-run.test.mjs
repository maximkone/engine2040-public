import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { isStale, release, tryLock } from "./arena-run.mjs";

test("one holder at a time; a dead or old holder's lock is taken over", () => {
  const dir = mkdtempSync(join(tmpdir(), "arena-run-"));
  const lock = join(dir, "arena-window.lock");
  try {
    assert.equal(tryLock(lock), true, "a free lock is taken");
    assert.equal(tryLock(lock), false, "a live holder (this process) keeps it");
    release(lock);
    assert.equal(tryLock(lock), true, "released, it is free again");
    release(lock);
    // A holder whose process is gone.
    writeFileSync(lock, JSON.stringify({ pid: 2 ** 22 + 7, at: "x" }));
    assert.equal(isStale(lock), true, "a dead pid is stale");
    assert.equal(tryLock(lock), true);
    release(lock);
    // A live holder that has held it too long.
    writeFileSync(lock, JSON.stringify({ pid: process.pid, at: "x" }));
    const old = (Date.now() - 16 * 60 * 1000) / 1000;
    utimesSync(lock, old, old);
    assert.equal(isStale(lock), true, "older than 15 minutes is stale");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

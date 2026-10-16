// Run: node --test tools/presence.test.mjs
// Pins tools/presence.mjs: heartbeat JSON shape, bad-session rejects,
// list sorting + STALE flags + corrupt skips, CLI round-trip.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { STALE_MS, heartbeat, list } from "./presence.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const BIN = join(HERE, "presence.mjs");
const freshDir = () => mkdtempSync(join(tmpdir(), "presence-test-"));

test("STALE_MS is 300000", () => {
  assert.equal(STALE_MS, 300000);
});

test("heartbeat writes session/pid/wave/files/heartbeat JSON", () => {
  const dir = freshDir();
  const rec = heartbeat({ dir, session: "bob", pid: 4242, wave: "w1", files: ["a", "b"] });
  assert.equal(rec.session, "bob");
  assert.equal(rec.pid, 4242);
  const raw = JSON.parse(readFileSync(join(dir, "bob.json"), "utf8"));
  assert.equal(raw.session, "bob");
  assert.equal(raw.pid, 4242);
  assert.equal(raw.wave, "w1");
  assert.deepEqual(raw.files, ["a", "b"]);
  assert.ok(Date.parse(raw.heartbeat) > 0);
});

test("heartbeat rejects bad sessions", () => {
  const dir = freshDir();
  for (const bad of ["", "../evil", "-lead", "has space"]) {
    assert.throws(() => heartbeat({ dir, session: bad }), /bad --session/);
  }
});

test("list sorts and flags fresh vs STALE, skips corrupt/nodate", () => {
  const dir = freshDir();
  heartbeat({ dir, session: "zebra", pid: 1 });
  heartbeat({ dir, session: "apple", pid: 2 });
  heartbeat({ dir, session: "oldie", pid: 3 });
  const old = JSON.parse(readFileSync(join(dir, "oldie.json"), "utf8"));
  old.heartbeat = new Date(Date.now() - STALE_MS - 1000).toISOString();
  writeFileSync(join(dir, "oldie.json"), JSON.stringify(old));
  writeFileSync(join(dir, "corrupt.json"), "{not json");
  writeFileSync(join(dir, "nodate.json"), JSON.stringify({ session: "nodate", pid: 9 }));
  writeFileSync(join(dir, "sessionless.json"), JSON.stringify({ pid: 8 }));
  const rows = list(dir);
  assert.deepEqual(rows.map((r) => r.session), ["apple", "nodate", "oldie", "zebra"]);
  const byName = Object.fromEntries(rows.map((r) => [r.session, r]));
  assert.equal(byName.apple.stale, false);
  assert.ok(byName.apple.ageMs < STALE_MS);
  assert.equal(byName.oldie.stale, true);
  assert.ok(byName.oldie.ageMs > STALE_MS);
  assert.equal(byName.nodate.stale, true);
  assert.ok(Number.isNaN(byName.nodate.ageMs));
});

test("CLI heartbeat then list round-trip; bad session exits 2", () => {
  const dir = freshDir();
  execFileSync("node", [BIN, "heartbeat", "--session", "cli-bob", "--dir", dir, "--pid", "4242"], { encoding: "utf8" });
  const out = execFileSync("node", [BIN, "list", "--dir", dir], { encoding: "utf8" });
  assert.match(out, /cli-bob pid=4242/);
  let code = 0;
  try {
    execFileSync("node", [BIN, "heartbeat", "--session", "../evil", "--dir", dir], { encoding: "utf8" });
  } catch (err) {
    code = err.status;
  }
  assert.equal(code, 2);
});

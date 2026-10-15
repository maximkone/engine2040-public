// Run: node --test tools/p4-netsim.test.mjs
// p4-netsim runs its CLI on import, so pin it through subprocess only.
// Short runs (--ticks 20 --reconnect off) keep the gate fast and stable.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "p4-netsim.mjs");

function run(args) {
  const r = spawnSync(process.execPath, [SCRIPT, ...args], { encoding: "utf8" });
  return { status: r.status, out: String(r.stdout ?? ""), err: String(r.stderr ?? "") };
}

function stripWall(out) {
  return out
    .split("\n")
    .filter((line) => !line.includes("wall_ms="))
    .join("\n");
}

test("clean short run passes with every client converged", () => {
  const r = run(["--ticks", "20", "--reconnect", "off"]);
  assert.equal(r.status, 0);
  assert.match(r.out, /p4-netsim PASS/);
  assert.match(r.out, /converged_freshest=64\/64/);
  assert.match(r.out, /exact_after_settle=64\/64/);
  assert.match(r.out, /desync events=0/);
});

test("corrupted packet fails as a desync for that client", () => {
  const r = run(["--ticks", "20", "--reconnect", "off", "--corrupt", "17:10"]);
  assert.equal(r.status, 1);
  assert.match(r.out, /corrupted=1/);
  assert.match(r.out, /client 17 tick 10.*packet digest mismatch/);
  assert.match(r.out, /p4-netsim FAIL: desync events=1/);
});

test("unknown argument exits 2 with the usage error", () => {
  const r = run(["--bogus"]);
  assert.equal(r.status, 2);
  assert.match(r.err, /unknown argument --bogus/);
});

test("short reconnect run exits 2 with the tick guard", () => {
  const r = run(["--ticks", "10", "--reconnect", "on"]);
  assert.equal(r.status, 2);
  assert.match(r.err, /reconnect needs at least 400 ticks/);
});

test("two clean runs agree on the converged lines", () => {
  const a = run(["--ticks", "20", "--reconnect", "off"]);
  const b = run(["--ticks", "20", "--reconnect", "off"]);
  assert.equal(a.status, 0);
  assert.equal(b.status, 0);
  assert.equal(stripWall(a.out), stripWall(b.out));
});

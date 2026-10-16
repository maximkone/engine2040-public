// cargo-guard tests: red dies at cap with E_BUILD_TIMEOUT, green untouched,
// message cap trims the tail (E_BUILD_TIMEOUT greppable receipt).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { isErrorLine, parseLimits, trimTail } from "./cargo-guard.mjs";
const GUARD = fileURLToPath(new URL("./cargo-guard.mjs", import.meta.url));
const run = (args, env) => spawnSync(process.execPath, [GUARD, ...args],
  { encoding: "utf8", env: { ...process.env, ...env }, timeout: 15000 });
test("limits default to 1s/0; error lines detected", () => {
  assert.deepEqual(parseLimits({}), { timeLimitSec: 1, msgLimit: 0 });
  assert.deepEqual(parseLimits({ CARGO_TIME_LIMIT: "0" }), { timeLimitSec: 0, msgLimit: 0 });
  assert.ok(isErrorLine("error: boom") && isErrorLine("error[12]"));
  assert.ok(!isErrorLine("info: ok"));
});
test("trimTail keeps head, uncapped intact", () => {
  assert.deepEqual(trimTail(["a", "b", "c"], 0), ["a", "b", "c"]);
  assert.deepEqual(trimTail(["a", "b", "c"], 2), ["a", "b"]);
});
test("red dies at cap with E_BUILD_TIMEOUT", () => {
  const red = [process.execPath, "-e", "console.log('error: boom'); setInterval(()=>{}, 1000)"];
  const t0 = Date.now();
  const r = run(red, { CARGO_TIME_LIMIT: "1", CARGO_MSG_LIMIT: "0" });
  const wall = (Date.now() - t0) / 1000;
  assert.notEqual(r.status, 0);
  assert.match(`${r.stdout}${r.stderr}`, /E_BUILD_TIMEOUT/);
  assert.match(`${r.stderr}`, /CARGO_TIME_LIMIT=1/);
  assert.ok(wall < 6, `red wall ${wall}s over cap+5s grace`);
});
test("green untouched uncapped, trimmed to 50 capped", () => {
  const green = (n) => [process.execPath, "-e", `for (let i=0;i<${n};i++) console.log('info: line '+i)`];
  const g = run(green(5), { CARGO_TIME_LIMIT: "1", CARGO_MSG_LIMIT: "0" });
  assert.equal(g.status, 0);
  assert.equal(g.stdout.trim().split("\n").length, 5);
  const c = run(green(200), { CARGO_MSG_LIMIT: "50" });
  assert.equal(c.status, 0);
  assert.equal(c.stdout.trim().split("\n").length, 50);
});

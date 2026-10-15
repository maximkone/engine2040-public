// Run: node --test .github/workflows/ci.test.mjs
// CI runner trim: full release build + tests run once (ubuntu), others smoke.
// Before: gates 2 + window-compile 2 + arm-golden 1 = 5 runners.
// After:  gates 1 + window-compile 1 + arm-golden 1 = 3 runners.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const CI = join(HERE, "ci.yml");

function runnerCount(text) {
  const lists = [...text.matchAll(/os:\s*\[([^\]]+)\]/g)].map((m) =>
    m[1].split(",").map((s) => s.trim()).filter(Boolean),
  );
  return lists.reduce((n, l) => n + l.length, 0);
}

function jobBlock(text, name) {
  const start = text.indexOf(`\n  ${name}:`);
  assert.notEqual(start, -1, `job ${name} exists`);
  const rest = text.slice(start + name.length + 4);
  const nextJob = rest.search(/\n  [a-z][a-z0-9-]*:/);
  const end = nextJob === -1 ? text.length : start + name.length + 4 + nextJob;
  return text.slice(start, end);
}

test("total expanded runners is 3 (full once, smoke others)", () => {
  const text = readFileSync(CI, "utf8");
  assert.equal(runnerCount(text), 3, `expected 3 runners, lists: ${[...text.matchAll(/os:\s*\[[^\]]+\]/g)].map((m) => m[0]).join(" | ")}`);
});

test("full suite (release build + tests) runs once on ubuntu only", () => {
  const text = readFileSync(CI, "utf8");
  const gates = jobBlock(text, "gates");
  assert.match(gates, /cargo build --locked --workspace --release/);
  assert.match(gates, /cargo test --locked --workspace --all-targets/);
  assert.match(gates, /os:\s*\[ubuntu-latest\]/);
  assert.doesNotMatch(gates, /windows-latest/);
  const jobsWithFullTest = ["gates", "window-compile", "arm-golden"].filter((j) =>
    jobBlock(text, j).includes("cargo test --locked --workspace --all-targets"),
  );
  assert.deepEqual(jobsWithFullTest, ["gates"]);
});

test("windows smoke (window check) and ARM golden stay", () => {
  const text = readFileSync(CI, "utf8");
  const wc = jobBlock(text, "window-compile");
  assert.match(wc, /windows-latest/);
  assert.doesNotMatch(wc, /ubuntu-latest/);
  assert.match(wc, /cargo check --locked --workspace --all-targets --features window/);
  const arm = jobBlock(text, "arm-golden");
  assert.match(arm, /ubuntu-24\.04-arm/);
});

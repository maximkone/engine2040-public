// Run: node --test tools/lead-brief.test.mjs
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

const TOOL = fileURLToPath(new URL("./lead-brief.mjs", import.meta.url));

function brief() {
  return execFileSync(process.execPath, [TOOL], { encoding: "utf8", timeout: 15000 });
}

test("brief exits 0 and prints every section plus the farm-first rule", () => {
  const out = brief();
  for (const head of ["FARM:", "DIRTY:", "CLAIMS:", "ROT:", "READY:", "RULE: FARM-FIRST:"]) {
    assert.ok(out.includes(head), "missing " + head);
  }
  assert.ok(out.includes("RECEIPT:"), "missing RECEIPT:");
  console.log(out.split("\n").slice(0, 3).join("\n"));
});

test("brief stays small and caps live claims to the last 8 lines", () => {
  const out = brief();
  assert.ok(Buffer.byteLength(out, "utf8") < 4096, "brief grew past 4KB: " + out.length);
  const src = readFileSync(TOOL, "utf8");
  assert.match(src, /slice\(-8\)/);
});

test("brief stays cargo-free and git-only", () => {
  const src = readFileSync(TOOL, "utf8");
  assert.match(src, /Fast, no cargo/);
  assert.ok(!src.includes('run("cargo"'), "brief must never exec cargo");
  assert.ok(src.includes('run("git"'), "brief reads state via git only");
});

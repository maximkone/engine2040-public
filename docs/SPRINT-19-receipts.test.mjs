// Run: node --test docs/SPRINT-19-receipts.test.mjs
// Speed fix f01: receipts hand-type golden hash plus proof text each landing; append via land+proof template.
// Fails on revert: removing the template comment or the golden canonical line fails the first two tests.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const SHARD = join(dirname(fileURLToPath(import.meta.url)), "SPRINT-19-receipts.md");

function shard() {
  return readFileSync(SHARD, "utf8").replace(/\r\n/g, "\n");
}

test("receipts carry the land+proof append template (remove it and this goes red)", () => {
  const text = shard();
  assert.match(text, /node tools\/land\.mjs <packet> --dry/, "template names the land dry-run");
  assert.match(text, /node tools\/proof\.mjs --skip-build/, "template names the proof skip-build");
  assert.match(text, /do not hand-type golden\/proof lines/, "template bans hand-typed hashes");
});

test("receipts name the canonical golden once up top (remove it and this goes red)", () => {
  const text = shard();
  assert.match(text, /Golden `d57b5c8ae3096614` holds for every landing below/, "canonical golden line stays");
});

test("template prunes nothing: receipt blocks stay", () => {
  const text = shard();
  assert.match(text, /## Receipts/, "receipts heading stays");
  for (const head of [
    "### 2026-10-10, EG9 linear HDR frame",
    "### 2026-10-10, fix4 render plus scene hygiene",
  ]) {
    assert.ok(text.includes(head), `${head} stays (no over-prune)`);
  }
  const heads = text.split("\n").filter((l) => l.startsWith("### ")).length;
  assert.ok(heads >= 89, `expected at least 89 receipt headings, found ${heads}`);
});

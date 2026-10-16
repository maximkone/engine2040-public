// Run: node --test docs/SPRINT-21-look-feel.test.mjs
// Speed fix speed-202610120259-f03: RL46 gates four look rows (RL48-51 carry a
// single-line "needs: RL46."); RL47's needs line wraps, so the single-line
// count is 4. Split: new RL46a interface row lands first, RL46 implements it,
// RL48-51 gate on RL46a, RL47 stays on the full RL46. Fails on revert:
// restoring the four "needs: RL46." lines (or deleting RL46a) goes red.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const SHARD = join(dirname(fileURLToPath(import.meta.url)), "SPRINT-21-look-feel.md");

function shard() {
  return readFileSync(SHARD, "utf8").replace(/\r\n/g, "\n");
}

function rowBlock(text, id) {
  const start = text.indexOf(`- [ ] ${id} `);
  assert.ok(start >= 0, `${id} row is present`);
  const next = text.indexOf("\n- [ ] ", start + 1);
  return next < 0 ? text.slice(start) : text.slice(start, next);
}

test("RL46a interface row exists with F2P, P2P and no gate", () => {
  const text = shard();
  const row = rowBlock(text, "RL46a");
  assert.match(row, /F2P:/, "RL46a carries F2P (revert this and the test goes red)");
  assert.match(row, /P2P:/, "RL46a carries P2P (revert this and the test goes red)");
  assert.match(row, /needs: none\./, "RL46a is gated on nothing so it lands first");
});

test("four look rows gate on the RL46a interface, not the full RL46", () => {
  const text = shard();
  for (const id of ["RL48", "RL49", "RL50", "RL51"]) {
    const row = rowBlock(text, id);
    assert.match(row, /needs: RL46a\./, `${id} gates on RL46a (restore needs: RL46. and this goes red)`);
    assert.doesNotMatch(row, /needs: RL46\./, `${id} no longer waits on the full RL46`);
  }
});

test("exact single-line full-RL46 gate count drops 4 -> 0 (RL47's wrap stays)", () => {
  const text = shard();
  const n = (text.match(/needs: RL46\./g) || []).length;
  assert.equal(n, 0, `expected 0 single-line needs: RL46. lines, found ${n}`);
  const rl47 = rowBlock(text, "RL47");
  assert.match(rl47, /RL46\./, "RL47 still gates on the full RL46 implementation");
  assert.doesNotMatch(rl47, /needs: RL46a\./, "RL47 stays on the full build, not the interface");
});

test("RL46 implements the interface (no cycle, resolves)", () => {
  const text = shard();
  const row = rowBlock(text, "RL46");
  assert.match(row, /needs: RL46a\./, "RL46 needs RL46a (revert this and the test goes red)");
});

test("no over-prune: RL46-RL51 rows all still present", () => {
  const text = shard();
  for (const id of ["RL46 ", "RL46a", "RL47", "RL48", "RL49", "RL50", "RL51"]) {
    assert.ok(text.includes(`- [ ] ${id}`), `${id} row is still present, not deleted`);
  }
});

// node --test tools/motion/mixamo.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { clipName, parseArgs, provenanceRow, wanted } from "./mixamo.mjs";

test("clip names drop the pack words and keep the take number", () => {
  assert.equal(clipName("sword and shield slash (2).fbx", "ss"), "ss_slash_2");
  assert.equal(clipName("sword and shield slash.fbx", "ss"), "ss_slash");
  assert.equal(clipName("sword and shield 180 turn (2).fbx", "ss"), "ss_180_turn_2");
  assert.equal(clipName("draw sword 1.fbx", "ss"), "ss_draw_sword_1");
  assert.equal(clipName("great sword slash (3).fbx", "gs"), "gs_slash_3");
});

test("args: --pack-dir is required, --only splits, unknown args refuse", () => {
  assert.throws(() => parseArgs([]), /--pack-dir is required/);
  const a = parseArgs(["--pack-dir", "x", "--only", "slash, block", "--prefix", "ss", "--install"]);
  assert.deepEqual(a.only, ["slash", "block"]);
  assert.equal(a.prefix, "ss");
  assert.equal(a.install, true);
  assert.throws(() => parseArgs(["--pack-dir", "x", "--nope"]), /unknown argument/);
  assert.equal(parseArgs(["--help"]).help, true);
});

test("--only picks files by word", () => {
  assert.equal(wanted("sword and shield slash.fbx", ["slash"]), true);
  assert.equal(wanted("sword and shield block.fbx", ["slash"]), false);
  assert.equal(wanted("anything.fbx", []), true);
});

test("the provenance row carries the source, ticks and slip", () => {
  const row = provenanceRow({ name: "ss_slash", source: "sword and shield slash.fbx", ticks: 91, slip: [0.115834, 0.108523] });
  assert.equal(row, "| `ss_slash.am5p` | Mixamo `sword and shield slash.fbx` | 91 | 0.115834 / 0.108523 |");
});

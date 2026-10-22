import { test } from "node:test";
import assert from "node:assert/strict";
import { findLeftovers, groupOf, parseStatus } from "./leftovers.mjs";

test("porcelain rows parse, renames keep the new path", () => {
  const raw = " M crates/game/src/lib.rs\0?? content/generated/props/\0R  new.rs\0old.rs\0";
  assert.deepEqual(parseStatus(raw), [
    { status: "M", path: "crates/game/src/lib.rs" },
    { status: "??", path: "content/generated/props/" },
    { status: "R", path: "new.rs" },
  ]);
});

test("a path groups by crate, tool folder or first two folders", () => {
  assert.equal(groupOf("crates/combat/src/fight.rs"), "crates/combat");
  assert.equal(groupOf("content/generated/concepts/a.jpg"), "content/generated");
  assert.equal(groupOf("content/generated/props/"), "content/generated");
  assert.equal(groupOf("tools/leftovers.mjs"), "tools");
  assert.equal(groupOf("AGENTS.md"), "(root)");
});

test("only paths idle past the bar are left over, oldest group first", () => {
  const hour = 3_600_000;
  const now = 100 * hour;
  const written = {
    "crates/game/src/lib.rs": now - 0.5 * hour,
    "content/generated/a.jpg": now - 12 * hour,
    "tools/x.mjs": now - 4 * hour,
    "gone.rs": null,
  };
  const rows = Object.keys(written).map((path) => ({ status: "M", path }));
  const groups = findLeftovers(rows, { hours: 3, now, stat: (p) => written[p] });
  assert.deepEqual(groups.map((g) => g.group), ["content/generated", "tools"]);
  assert.equal(groups[0].idleHours, 12);
});

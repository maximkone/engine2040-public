// Run: node --test tools/beat-lint.test.mjs
// Ladder beat gate: the real ladder.rows passes, and a synthetic ladder
// with a clumped rung (seed 2040, fixed strings, no randomness) fails.
// Bad tiers, a boss off the last rung, rank drift from ladder.rs, a minute
// or a new-element total off the compiled chart, and float cells fail too,
// with a line number so a bad row cannot slip silent. The arena a rung runs
// is gated too: an id no placement file backs fails, and a new-arena count
// that does not follow the placement swap fails. The first-bout launch
// cap is not checked here: ladder.rs owns it (FIRST_BOUT_START_SECS 45,
// FIRST_BOUT_START_LIMIT_SECS 60) and its Rust test proves it, so no row
// ever states a bout length as a launch time.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  ARENA_VARIANTS,
  checkAll,
  parseLadder,
  rankOf,
  rewardForRung,
  RUNG_MINUTES,
  RUNG_NEW_ELEMENTS,
} from "./beat-lint.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const LADDER = join(ROOT, "content", "design", "ladder.rows");

function row(id, patch = {}) {
  const n = Number(id.slice(5));
  const base = {
    tier: n < 2 ? 1 : n < 5 ? 2 : 3,
    new_moves: 1,
    new_rules: RUNG_NEW_ELEMENTS[n] - 1,
    new_arena: 0,
    arena: "base",
    reward: rewardForRung(n),
    penalty: 25,
    minutes: RUNG_MINUTES[n],
    rank: [0, 0, 1, 1, 1, 2, 2, 3][n],
    boss: n === 7 ? 1 : 0,
  };
  const got = { ...base, ...patch };
  return (
    id +
    " tier=" + got.tier +
    " new_moves=" + got.new_moves +
    " new_rules=" + got.new_rules +
    " new_arena=" + got.new_arena +
    " arena=" + got.arena +
    " reward=" + got.reward +
    " penalty=" + got.penalty +
    " minutes=" + got.minutes +
    " rank=" + got.rank +
    " boss=" + got.boss
  );
}

function ladderText(patches = {}) {
  const lines = ["table ladder version 1"];
  for (let i = 0; i < 8; i += 1) lines.push(row("rung_" + i, patches["rung_" + i] || {}));
  return lines.join("\n") + "\n";
}

// Seeded clumped ladder (seed 2040): rung_3 teaches 4 new elements at once.
const CLUMPED_SEED2040 = ladderText({ rung_3: { new_moves: 2, new_rules: 1, new_arena: 1 } });

// Compiled chart of crates/game/src/ladder.rs:41,43, the same numbers
// `Ladder::beat()` and `rung_beat()` return. Copied here on purpose: the JS
// mirror and the rows file must both follow a compiled chart change, never
// fork it silently.
const COMPILED_MINUTES = [2, 2, 3, 3, 4, 4, 5, 6];
const COMPILED_NEW_ELEMENTS = [2, 1, 2, 1, 2, 1, 2, 1];

test("rewards and ranks match ladder.rs math", () => {
  assert.equal(rewardForRung(0), 100);
  assert.equal(rewardForRung(7), 275);
  assert.equal(rankOf(225), 0);
  assert.equal(rankOf(375), 1);
  assert.equal(rankOf(2000), 3);
});

test("the chart mirror equals the compiled chart in ladder.rs", () => {
  assert.deepEqual(RUNG_MINUTES, COMPILED_MINUTES);
  assert.deepEqual(RUNG_NEW_ELEMENTS, COMPILED_NEW_ELEMENTS);
});

test("live ladder.rows passes with 8 rungs", () => {
  const r = checkAll({ ladderText: readFileSync(LADDER, "utf8") });
  assert.equal(r.rows.length, 8);
  assert.equal(r.cumulative, 1500);
  assert.deepEqual(r.rows.map((x) => x.minutes), COMPILED_MINUTES, "live row minutes equal ladder.rs RUNG_MINUTES");
  assert.deepEqual(
    r.rows.map((x) => x.new_moves + x.new_rules + x.new_arena),
    COMPILED_NEW_ELEMENTS,
    "live row new totals equal ladder.rs RUNG_NEW_ELEMENTS",
  );
  assert.deepEqual(r.failures, [], "live ladder has no rule failures, got " + r.failures.join("; "));
});

test("seeded clumped rung must fail (seed 2040: 4 new on rung_3)", () => {
  const r = checkAll({ ladderText: CLUMPED_SEED2040 });
  assert.ok(r.failures.some((f) => f.includes("rung_3") && f.includes("4 new")), "fails clump, got " + r.failures.join("; "));
});

test("falling tier must fail", () => {
  const r = checkAll({ ladderText: ladderText({ rung_4: { tier: 1 } }) });
  assert.ok(r.failures.some((f) => f.includes("falls")), "fails tier fall, got " + r.failures.join("; "));
});

test("boss off the last rung must fail", () => {
  const early = checkAll({ ladderText: ladderText({ rung_2: { boss: 1 }, rung_7: { boss: 1 } }) });
  assert.ok(early.failures.some((f) => f.includes("before the last")), "fails early boss, got " + early.failures.join("; "));
  const missing = checkAll({ ladderText: ladderText({ rung_7: { boss: 0 } }) });
  assert.ok(missing.failures.some((f) => f.includes("must carry boss=1")), "fails missing boss, got " + missing.failures.join("; "));
});

test("rank drift from ladder.rs must fail", () => {
  const r = checkAll({ ladderText: ladderText({ rung_7: { rank: 4 } }) });
  assert.ok(r.failures.some((f) => f.includes("cumulative 2000")), "fails rank drift, got " + r.failures.join("; "));
});

test("reward drift from ladder.rs must fail", () => {
  const r = checkAll({ ladderText: ladderText({ rung_0: { reward: 150, rank: 0 } }) });
  assert.ok(r.failures.some((f) => f.includes("ladder.rs pays 100")), "fails reward drift, got " + r.failures.join("; "));
});

test("a minute off the compiled chart must fail naming the rung", () => {
  const r = checkAll({ ladderText: ladderText({ rung_2: { minutes: 5 } }) });
  assert.ok(
    r.failures.some((f) => f.includes("rung_2") && f.includes("minutes 5") && f.includes("ladder.rs expects 3")),
    "fails minute drift, got " + r.failures.join("; "),
  );
});

test("a new-element total off the compiled chart must fail naming the rung", () => {
  const r = checkAll({ ladderText: ladderText({ rung_5: { new_moves: 1, new_rules: 1, new_arena: 0 } }) });
  assert.ok(
    r.failures.some((f) => f.includes("rung_5") && f.includes("2 new elements") && f.includes("ladder.rs teaches 1")),
    "fails new-total drift, got " + r.failures.join("; "),
  );
});

test("rows refuse floats with the line number", () => {
  const bad = "table ladder version 1\nrung_0 tier=1 new_moves=1 new_rules=1 new_arena=0 reward=100.5 penalty=25 minutes=1 rank=0 boss=0\n";
  assert.throws(() => parseLadder(bad), /line 2.*whole number/);
});

test("rows refuse a missing header", () => {
  assert.throws(() => parseLadder(row("rung_0") + "\n"), /first row must be/);
});

test("the live rows name the arena each rung runs", () => {
  const r = checkAll({ ladderText: readFileSync(LADDER, "utf8") });
  assert.deepEqual(
    r.rows.map((x) => x.arena),
    ["base", "base", "base", "base", "ludus", "ludus", "siege", "siege"],
    "the live chart names the placement per rung",
  );
  // The count follows the swap: 1 on the rung that changes the placement.
  assert.deepEqual(
    r.rows.map((x) => x.new_arena),
    [0, 0, 0, 0, 1, 0, 1, 0],
    "the live count follows the placement swap",
  );
});

test("every arena variant names a placement file that exists", () => {
  for (const variant of ARENA_VARIANTS) {
    assert.ok(existsSync(variant.file), "placement file missing: " + variant.file);
  }
});

test("an arena variant no placement file backs must fail", () => {
  const r = checkAll({ ladderText: ladderText({ rung_4: { arena: "nowhere" } }) });
  assert.ok(
    r.failures.some((f) => f.includes("rung_4") && f.includes("unknown arena variant")),
    "fails unknown variant, got " + r.failures.join("; "),
  );
});

test("an arena count that ignores the swap must fail", () => {
  const r = checkAll({ ladderText: ladderText({ rung_4: { arena: "ludus", new_arena: 0 } }) });
  assert.ok(
    r.failures.some((f) => f.includes("rung_4") && f.includes("new_arena 0")),
    "fails a lying count, got " + r.failures.join("; "),
  );
});

// Run: node --test tools/level-check.test.mjs
// Level layout gate: the hand-placed arena plus zones plus LEVELS.md print
// walk time, cover and entrance share, and a bad layout must fail.
// A seeded bad layout (seed 2040, fixed strings, no randomness) proves the
// three fail rules: no holdable dominance, open floor with no cover, an
// actor with no use. Float cells and a missing header fail with a line
// number so a bad row cannot slip silent. Advantage claims are seeded the same
// way: one off the duel floor, one on the floor edge, one floor with none.
// Height claims are seeded the same way: a crate_a row claiming 400 cm over the
// shipped 100 cm placement box.
// Risk claims are seeded the same way: the live 60 m row off the duel floor and
// the floor edge at exactly cx 1000, plus the two live rows the live test names.
// Cover reach is seeded too: the shipped one-sided pair leaves spawn A 1030 cm
// from cover. Cover form is seeded the same way: the shipped pair reaches both
// spawns at 510 cm but hides spawn B behind a 150 cm mound while spawn A gets a
// 100 cm cube, so the form rule fails; a pair inside HEIGHT_TOLERANCE_CM passes
// and one cm past fails.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  advantageOffFloor,
  checkAll,
  computeCover,
  computeEntranceShare,
  computeNoUse,
  computePhysicalUnread,
  computePossibleDeath,
  computeWalkS,
  COVER_REACH_CM,
  coverFormFailures,
  coverReachFailures,
  formatWalkS,
  HEIGHT_TOLERANCE_CM,
  nearestCoverCm,
  nearestCoverRow,
  parsePlacement,
  parseZones,
  riskOffFloor,
  SPAWN_A_CM,
  SPAWN_B_CM,
} from "./level-check.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);
const PLACEMENT = join(ROOT, "content", "arena", "placement.json");
const ZONES = join(ROOT, "content", "design", "arena-zones.rows");
const LEVELS = join(ROOT, "docs", "design", "LEVELS.md");
const ARENA_KIT = join(ROOT, "crates", "game", "src", "arena_kit.rs");

// Seeded bad layout (seed 2040): one dominance-free, cover-free zones text
// over two actors plus a levels text with no use lines and no scale.
const BAD_ZONES_SEED2040 = [
  "table arena-zones version 1",
  "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0",
  "crate_a class=0 cx_cm=2900 cz_cm=0 radius_cm=100 height_cm=100 advantage=0 dominance=0 risk=0",
].join("\n");
const BAD_LEVELS_SEED2040 = [
  "# empty levels: no scale, no actors",
  "Nothing here names a use.",
].join("\n");
const GOOD_LEVELS_MIN = [
  "Fighter height 1800mm, sword reach 2000mm, dodge distance 2m.",
  "sand_floor: open sand use. Past event: blood spilled and the crowd roared, so sand was raked. Goal: learn.",
  "wall_north: north bound use. Past event: a charge dented the boards, so iron strips were bolted. Goal: learn.",
  "crate_a: only cover use. Past event: a recruit hid and lived, so veterans kick it first. Goal: explore.",
  // crate_b mirrors crate_a (-5,-5) and debris_field names the second bumper.
  "crate_b: mirrored west cover use. Past event: the twin crate was dragged in, so the ring is fair. Goal: learn.",
  "debris_field: ring-band cover use. Past event: sweepers left half the stones. Goal: moral choice.",
].join("\n");

function tinyPlacement() {
  return JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 29, y: 0.5, sy: 1.0, z: 0, physical: true },
    ],
  });
}

test("walk_s is 8 m at 170 cm/s (4.7 s)", () => {
  assert.equal(formatWalkS(computeWalkS()), "4.7");
  assert.ok(computeWalkS() > 3.0 && computeWalkS() < 6.0, "walk inside 3.0..6.0 s band");
});

test("live layout passes: cover, entrance, use, physical, dominance", () => {
  const placementText = readFileSync(PLACEMENT, "utf8");
  const zonesText = readFileSync(ZONES, "utf8");
  const levelsText = readFileSync(LEVELS, "utf8");
  const arenaKitText = readFileSync(ARENA_KIT, "utf8");
  const r = checkAll({ placementText, zonesText, levelsText, arenaKitText });
  assert.equal(r.walkOne, "4.7");
  assert.equal(r.cover, 2);
  // Band, not a point: the entrance must stay a place a fighter crosses, so a
  // later move cannot silently return it to a single-digit floor-edge strip.
  assert.ok(
    r.entranceShare >= 25 && r.entranceShare <= 60,
    "entrance_share " + r.entranceShare + " outside band 25..60",
  );
  assert.equal(r.noUse, 0);
  assert.equal(r.physicalUnread, 0);
  assert.ok(r.dominance >= 1, "one holdable dominance point");
  assert.equal(r.possibleDeath, 234, "possible_death pinned 234, got " + r.possibleDeath);
  // The live risk rows are `gate_tunnel` (its mouth stands on the 10 m floor
  // edge, |cx| 1000) and `debris_field`; no other row carries the flag. Both
  // stand on the floor, so the risk rule is quiet on the shipped layout.
  assert.deepEqual(
    r.rows.filter((x) => x.risk === 1).map((x) => x.id),
    ["gate_tunnel", "debris_field"],
    "only gate_tunnel and debris_field carry risk=1",
  );
  assert.deepEqual(riskOffFloor(r.rows), [], "every live risk row stands on the duel floor");
  assert.deepEqual(r.failures, [], "live layout has no rule failures, got " + r.failures.join("; "));
});

test("seeded bad layout must fail (seed 2040: dominance, cover, use, scale)", () => {
  const r = checkAll({
    placementText: tinyPlacement(),
    zonesText: BAD_ZONES_SEED2040,
    levelsText: BAD_LEVELS_SEED2040,
    arenaKitText: "has physical reader",
  });
  assert.equal(r.cover, 0);
  assert.equal(r.dominance, 0);
  assert.ok(r.noUse > 0, "bad levels miss use lines");
  assert.ok(r.failures.some((f) => f.includes("dominance")), "fails dominance, got " + r.failures.join("; "));
  assert.ok(r.failures.some((f) => f.includes("no cover")), "fails cover, got " + r.failures.join("; "));
  assert.ok(r.failures.some((f) => f.includes("no use")), "fails use, got " + r.failures.join("; "));
  assert.ok(r.failures.some((f) => f.includes("scale")), "fails scale, got " + r.failures.join("; "));
});

test("zones refuse floats with the line number", () => {
  const bad = "table arena-zones version 1\nsand_floor class=0 cx_cm=1.5 cz_cm=0 radius_cm=100 height_cm=20 advantage=0 dominance=0 risk=0\n";
  assert.throws(() => parseZones(bad), /line 2.*whole number/);
});

test("zones refuse a missing header", () => {
  assert.throws(() => parseZones("sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=100 height_cm=20 advantage=0 dominance=0 risk=0\n"), /first row/);
});

test("zones refuse unknown class", () => {
  const bad = "table arena-zones version 1\nsand_floor class=9 cx_cm=0 cz_cm=0 radius_cm=100 height_cm=20 advantage=0 dominance=0 risk=0\n";
  assert.throws(() => parseZones(bad), /class/);
});

test("placement actors without a zone row fail", () => {
  const zonesOne = "table arena-zones version 1\nsand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=1 risk=0\n";
  const r = checkAll({
    placementText: tinyPlacement(),
    zonesText: zonesOne,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  assert.ok(r.failures.some((f) => f.includes("zones miss")), "fails missing zone, got " + r.failures.join("; "));
});

test("physical_unread counts actors when the reader is gone", () => {
  const actors = parsePlacement(tinyPlacement());
  assert.equal(computePhysicalUnread(actors, "has physical word"), 0);
  assert.equal(computePhysicalUnread(actors, "no flag here"), 2);
});

test("a moved actor with a stale zone row fails (position drift)", () => {
  // crate_a sits at (29,0) in the live placement; a move to (6,-6) with the
  // old centre in the zones file must fail, not pass silent.
  const moved = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 6, y: 0.5, sy: 1.0, z: -6, physical: true },
    ],
  });
  const stale = "table arena-zones version 1\nsand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=1 risk=0\ncrate_a class=4 cx_cm=2900 cz_cm=0 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n";
  const r = checkAll({
    placementText: moved,
    zonesText: stale,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  assert.ok(
    r.failures.some((f) => f.includes("crate_a") && f.includes("disagrees")),
    "fails stale zone centre, got " + r.failures.join("; "),
  );
});

test("matching centres pass the position rule", () => {
  // The dominance flag rides a holdable row (centre r 250 cm) since the hold
  // rule landed: a 15 m sand_floor row carrying the flag is not holdable alone.
  // The cover pair is the live mirrored one: crate_a 510 cm from spawn B and
  // crate_b 510 cm from spawn A, so the reach rule reads both spawns covered.
  // Both rows are 100 cm tall, so the cover-form rule has nothing to report
  // here either: this fixture stays green with no edit.
  const inPlace =
    "table arena-zones version 1\n" +
    "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
    "crate_a class=4 cx_cm=500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n" +
    "crate_b class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=0 dominance=0 risk=0\n" +
    "centre class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=1 dominance=1 risk=0\n";
  const moved = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 5, y: 0.5, sy: 1.0, z: -5, physical: true },
      { id: 11, name: "crate_b", x: -5, y: 0.5, sy: 1.0, z: -5, physical: true },
    ],
  });
  const r = checkAll({
    placementText: moved,
    zonesText: inPlace,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  assert.deepEqual(r.failures, [], "matching centres keep the layout green, got " + r.failures.join("; "));
});

test("cover off the duel floor, an unholdable dominance point and an off-floor advantage flag must fail", () => {
  // Seeded bad layout 2 (seed 2040, no randomness): crate_a keeps the old
  // debris_field pose (34 m, off the 10 m floor) as the only bumper, and the
  // 30 m sand_floor row carries dominance=1, so no single fighter holds it.
  // That off-floor row also carries advantage=1, so the advantage rule fires
  // on it too (an unreachable point is dead space).
  // Before these rules all of it passed exit 0 while printing `cover 1 bumpers`.
  const off = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 34, y: 0.5, sy: 1.0, z: 0, physical: true },
    ],
  });
  const bad =
    "table arena-zones version 1\n" +
    "sand_floor class=2 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=1 risk=0\n" +
    "crate_a class=4 cx_cm=3400 cz_cm=0 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n";
  const r = checkAll({
    placementText: off,
    zonesText: bad,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  assert.equal(r.cover, 1, "the off-floor bumper is still counted as cover");
  assert.equal(r.dominance, 1, "the flag is still set");
  assert.equal(r.failures.length, 6, "cover, both uncovered spawns, the off-floor advantage flag, no floor advantage point and dominance, got " + r.failures.join("; "));
  assert.ok(
    r.failures.some((f) => f.includes("crate_a") && f.includes("outside the duel floor")),
    "fails off-floor cover, got " + r.failures.join("; "),
  );
  // An off-floor bumper is not cover either, so both spawns read uncovered.
  assert.ok(
    r.failures.some(
      (f) => f === "cover spawn A at (-400,0) has no bumper within 600 cm (nearest no floor bumper): one side starts the bout uncovered",
    ) && r.failures.some((f) => f.includes("cover spawn B at (400,0)") && f.includes("(nearest no floor bumper)")),
    "fails both uncovered spawns, got " + r.failures.join("; "),
  );
  assert.ok(
    r.failures.some((f) => f.includes("sand_floor") && f.includes("not holdable alone")),
    "fails unholdable dominance, got " + r.failures.join("; "),
  );
  assert.ok(
    r.failures.some((f) => f.includes("advantage crate_a") && f.includes("outside the duel floor")),
    "fails off-floor advantage, got " + r.failures.join("; "),
  );
  assert.ok(
    r.failures.some((f) => f === "no advantage point inside the duel floor"),
    "no reachable advantage point left, got " + r.failures.join("; "),
  );
});

test("cover and dominance on the floor at the radius cap pass", () => {
  // Boundary: a dominance point of exactly the hold cap inside the floor, plus
  // one bumper inside the floor, must stay green (cap is not off by one). The
  // mirrored pair keeps both spawns inside the cover reach, and both bumpers
  // are 100 cm rows, so the cover-form rule stays quiet too.
  const zones =
    "table arena-zones version 1\n" +
    "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
    "crate_a class=4 cx_cm=500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n" +
    "crate_b class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=0 dominance=0 risk=0\n" +
    "centre class=1 cx_cm=0 cz_cm=0 radius_cm=300 height_cm=0 advantage=1 dominance=1 risk=0\n";
  const placed = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 5, y: 0.5, sy: 1.0, z: -5, physical: true },
      { id: 11, name: "crate_b", x: -5, y: 0.5, sy: 1.0, z: -5, physical: true },
    ],
  });
  const r = checkAll({
    placementText: placed,
    zonesText: zones,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  assert.equal(r.failures.length, 0, "at the cap the layout stays green, got " + r.failures.join("; "));
});

// The shipped one-sided cover pair (before this packet): both bumpers at x +5 m,
// so spawn A was 1030 cm (6.1 s) from cover while spawn B stood 510 cm from it.
const ONESIDED_PLACEMENT = JSON.stringify({
  actors: [
    { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
    { id: 10, name: "crate_a", x: 5, y: 0.5, sy: 1.0, z: -5, physical: true },
    { id: 11, name: "debris_field", x: 5, y: 0.75, sy: 1.5, z: 5, physical: true },
  ],
});
const ONESIDED_ZONES =
  "table arena-zones version 1\n" +
  "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
  "crate_a class=4 cx_cm=500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n" +
  "debris_field class=4 cx_cm=500 cz_cm=500 radius_cm=125 height_cm=150 advantage=1 dominance=0 risk=1\n" +
  "centre class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=1 dominance=1 risk=0\n";

test("the shipped one-sided cover pair leaves spawn A uncovered", () => {
  // Nothing read a spawn but walk_s before the reach rule, so this layout
  // printed exit 0 while LEVELS.md claimed the covers favoured no spawn.
  // Both bumpers stand 510 cm from spawn B (an exact tie), and a tie takes the
  // first row in file order, `crate_a` -- so both spawns' nearest cover is the
  // 100 cm crate here and the form rule stays quiet (the shipped unequal pair
  // is seeded below as SHIPPED_FORM_PAIR, where the tie is broken by the move).
  const r = checkAll({
    placementText: ONESIDED_PLACEMENT,
    zonesText: ONESIDED_ZONES,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  assert.equal(nearestCoverCm(r.rows, SPAWN_A_CM), 1030);
  assert.equal(nearestCoverCm(r.rows, SPAWN_B_CM), 510);
  assert.deepEqual(coverFormFailures(r.rows), [], "the 510/510 tie reads crate_a for both spawns");
  assert.deepEqual(r.failures, [
    "cover spawn A at (-400,0) has no bumper within 600 cm (nearest 1030 cm): one side starts the bout uncovered",
  ]);
});

test("a bumper at exactly the cover reach passes and one cm past fails", () => {
  const at = [{ id: "crate_a", class: 4, cx_cm: 200, cz_cm: 0 }];
  assert.equal(nearestCoverCm(at, SPAWN_A_CM), COVER_REACH_CM);
  assert.equal(nearestCoverCm(at, SPAWN_B_CM), 200);
  assert.deepEqual(coverReachFailures(at), [], "exactly 600 cm from spawn A stays green");
  const past = [{ id: "crate_a", class: 4, cx_cm: 201, cz_cm: 0 }];
  assert.equal(nearestCoverCm(past, SPAWN_A_CM), COVER_REACH_CM + 1);
  assert.deepEqual(coverReachFailures(past), [
    "cover spawn A at (-400,0) has no bumper within 600 cm (nearest 601 cm): one side starts the bout uncovered",
  ]);
});

test("an off-floor bumper is not cover for the reach rule", () => {
  // The off-floor cover rule fails such a row too; the reach rule must not
  // count it as cover either (the fight never reaches it).
  const off = [{ id: "crate_a", class: 4, cx_cm: 500, cz_cm: 1500 }];
  assert.equal(nearestCoverCm(off, SPAWN_B_CM), null);
  assert.deepEqual(coverReachFailures(off), [
    "cover spawn A at (-400,0) has no bumper within 600 cm (nearest no floor bumper): one side starts the bout uncovered",
    "cover spawn B at (400,0) has no bumper within 600 cm (nearest no floor bumper): one side starts the bout uncovered",
  ]);
});

test("the moved cover pair reaches both spawns at 510 cm", () => {
  // The shipped move: crate_a (-5,-5) west for spawn A, debris_field (5,5) east
  // for spawn B, so the bout starts fair from either side.
  const r = checkAll({
    placementText: readFileSync(PLACEMENT, "utf8"),
    zonesText: readFileSync(ZONES, "utf8"),
    levelsText: readFileSync(LEVELS, "utf8"),
    arenaKitText: readFileSync(ARENA_KIT, "utf8"),
  });
  assert.equal(nearestCoverCm(r.rows, SPAWN_A_CM), 510);
  assert.equal(nearestCoverCm(r.rows, SPAWN_B_CM), 510);
  assert.deepEqual(coverReachFailures(r.rows), [], "both spawns reach cover in 3.0 s");
});

// Advantage fixtures (seed 2040, no randomness): wall_north at `wallM` metres
// carries the seeded flag, crate_a holds the legal advantage point at (5,-5)
// inside the floor, crate_b mirrors it at (-5,-5) so both spawns reach cover
// (510 cm each), centre the dominance point. Only the seeded flags move: both
// bumpers are 100 cm rows, so the cover-form rule stays quiet here.
function advantageCase(wallM, wallAdvantage, crateAdvantage) {
  const placed = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 2, name: "wall_north", x: wallM, y: 0.3, sy: 3, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 5, y: 0.5, sy: 1.0, z: -5, physical: true },
      { id: 11, name: "crate_b", x: -5, y: 0.5, sy: 1.0, z: -5, physical: true },
    ],
  });
  const zones =
    "table arena-zones version 1\n" +
    "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
    "wall_north class=0 cx_cm=" + Math.round(wallM * 100) + " cz_cm=0 radius_cm=400 height_cm=300 advantage=" + wallAdvantage + " dominance=0 risk=0\n" +
    "crate_a class=4 cx_cm=500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=" + crateAdvantage + " dominance=0 risk=0\n" +
    "crate_b class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=0 dominance=0 risk=0\n" +
    "centre class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=0 dominance=1 risk=0\n";
  return checkAll({ placementText: placed, zonesText: zones, levelsText: GOOD_LEVELS_MIN, arenaKitText: "physical" });
}

test("an advantage flag off the duel floor fails (the fight never reaches it)", () => {
  // wall_north carries advantage=1 at 12 m, 2 m outside the 10 m floor, while
  // crate_a holds a legal point inside it. Nothing read the flag before the
  // rule, so the off-floor claim passed exit 0.
  const r = advantageCase(12, 1, 1);
  assert.deepEqual(r.failures, [
    "advantage wall_north stands outside the duel floor at (1200,0): the fight never reaches it",
  ]);
  assert.deepEqual(advantageOffFloor(r.rows).map((x) => x.id), ["wall_north"]);
});

test("an advantage point on the floor edge at cx 1000 passes", () => {
  // Boundary: exactly the floor half-extent is on the floor, not off it.
  const r = advantageCase(10, 1, 1);
  assert.equal(r.failures.length, 0, "the floor edge stays green, got " + r.failures.join("; "));
  assert.deepEqual(advantageOffFloor(r.rows), []);
});

test("a layout with no advantage point on the floor fails (flat bowl)", () => {
  // Every advantage flag is 0, so the ring offers no reachable advantage.
  const r = advantageCase(12, 0, 0);
  assert.deepEqual(r.failures, ["no advantage point inside the duel floor"]);
});

// Risk seed (seed 2040, no randomness): the live `sand_floor` row -- 15 m
// radius, centre 60 m out at (-6000,0), so off the 10 m duel floor -- carries
// risk=1, with the ring's holdable dominance point, an on-floor advantage point
// and the mirrored cover pair, so the risk rule is the only one that can speak.
// Nothing read the flag before the rule: this layout printed the seven lines and
// exit 0 (Worlds II.6: a risk point the fight cannot reach is dead space).
const RISK_SEED2040_PLACEMENT = JSON.stringify({
  actors: [
    { id: 1, name: "sand_floor", x: -60, y: 0.1, sy: 0.2, z: 0, physical: true },
    { id: 10, name: "crate_a", x: -5, y: 0.5, sy: 1.0, z: -5, physical: true },
    { id: 11, name: "crate_b", x: 5, y: 0.5, sy: 1.0, z: 5, physical: true },
  ],
});
const RISK_SEED2040_ZONES =
  "table arena-zones version 1\n" +
  "sand_floor class=0 cx_cm=-6000 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=1\n" +
  "crate_a class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n" +
  "crate_b class=4 cx_cm=500 cz_cm=500 radius_cm=100 height_cm=100 advantage=0 dominance=0 risk=0\n" +
  "centre class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=0 dominance=1 risk=0\n";

test("a risk flag off the duel floor fails (the fight never reaches it)", () => {
  const r = checkAll({
    placementText: RISK_SEED2040_PLACEMENT,
    zonesText: RISK_SEED2040_ZONES,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  assert.deepEqual(r.failures, [
    "risk sand_floor stands outside the duel floor at (-6000,0): the fight never reaches it",
  ]);
  assert.deepEqual(riskOffFloor(r.rows).map((x) => x.id), ["sand_floor"]);
});

// Edge fixtures (seed 2040, no randomness): the risk flag rides `centre` (a row
// with no placement actor, so the position rule cannot interfere) while a
// separate `holds` row keeps the ring's holdable dominance point and crate_a
// holds the on-floor advantage point, so only the risk rule can fire.
function riskEdgeCase(centreX) {
  const placed = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: -5, y: 0.5, sy: 1.0, z: -5, physical: true },
      { id: 11, name: "crate_b", x: 5, y: 0.5, sy: 1.0, z: 5, physical: true },
    ],
  });
  const zones =
    "table arena-zones version 1\n" +
    "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
    "crate_a class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n" +
    "crate_b class=4 cx_cm=500 cz_cm=500 radius_cm=100 height_cm=100 advantage=0 dominance=0 risk=0\n" +
    "centre class=1 cx_cm=" + centreX + " cz_cm=0 radius_cm=250 height_cm=0 advantage=0 dominance=0 risk=1\n" +
    "holds class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=0 dominance=1 risk=0\n";
  return checkAll({ placementText: placed, zonesText: zones, levelsText: GOOD_LEVELS_MIN, arenaKitText: "physical" });
}

test("a risk point on the floor edge at cx 1000 passes and one cm past fails", () => {
  // Boundary: exactly the floor half-extent is on the floor, not off it -- the
  // inclusive edge the live `gate_tunnel` row stands on.
  const at = riskEdgeCase(1000);
  assert.deepEqual(at.failures, [], "the floor edge stays green, got " + at.failures.join("; "));
  assert.deepEqual(riskOffFloor(at.rows), []);
  const past = riskEdgeCase(1001);
  assert.deepEqual(past.failures, [
    "risk centre stands outside the duel floor at (1001,0): the fight never reaches it",
  ]);
  assert.deepEqual(riskOffFloor(past.rows).map((x) => x.id), ["centre"]);
});

// Height fixtures (seed 2040, no randomness): crate_a keeps the shipped 100 cm
// placement box (`sy` 1.0) at (5,-5) on the duel floor with crate_b mirroring it;
// only the row's height_cm moves, so a row that claims a height the box never
// had fails.
function heightCase(crateHeightCm) {
  const placed = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 5, y: 1.0, sy: 1.0, z: -5, physical: true },
      { id: 11, name: "crate_b", x: -5, y: 0.5, sy: 1.0, z: -5, physical: true },
    ],
  });
  const zones =
    "table arena-zones version 1\n" +
    "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
    "crate_a class=4 cx_cm=500 cz_cm=-500 radius_cm=100 height_cm=" + crateHeightCm + " advantage=1 dominance=0 risk=0\n" +
    "crate_b class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=0 dominance=0 risk=0\n" +
    "centre class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=1 dominance=1 risk=0\n";
  return checkAll({ placementText: placed, zonesText: zones, levelsText: GOOD_LEVELS_MIN, arenaKitText: "physical" });
}

test("a zone row height that disagrees with its placement box fails", () => {
  // crate_a's row claims 400 cm over the shipped 100 cm box. Nothing read
  // height_cm before the rule, so the claim passed exit 0. The tolerance edge
  // is green: exactly HEIGHT_TOLERANCE_CM off the box passes, one cm past fails.
  const r = heightCase(400);
  // Two rules read this fixture: the row over its own box (400 over the 100 cm
  // box) and the form rule (spawn A's nearest cover is the 100 cm crate_b while
  // spawn B's is the crate_a row claiming 400 cm, both 510 cm away).
  assert.deepEqual(r.failures, [
    "zone row crate_a height_cm 400 disagrees with placement box height 100",
    "cover spawn A starts beside crate_b (100 cm) while spawn B starts beside crate_a (400 cm): one side starts the bout with better cover",
  ]);
  assert.deepEqual(heightCase(100 + HEIGHT_TOLERANCE_CM).failures, []);
  assert.ok(
    heightCase(100 + HEIGHT_TOLERANCE_CM + 1).failures.some((f) => f.includes("height_cm 106 disagrees")),
    "one cm past the tolerance must fail",
  );
});

// The shipped unequal pair frozen as text (before this packet): both spawns
// reach cover at 510 cm, but crate_a is a 100 cm cube for spawn A while
// debris_field was a 150 cm mound for spawn B. Each row matches its own box
// (`sy` 1.0 and 1.5), so the form rule is the only one that can speak here.
const SHIPPED_FORM_PAIR_PLACEMENT = JSON.stringify({
  actors: [
    { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
    { id: 10, name: "crate_a", x: -5, y: 0.5, sy: 1.0, z: -5, physical: true },
    { id: 11, name: "debris_field", x: 5, y: 0.75, sy: 1.5, z: 5, physical: true },
  ],
});
const SHIPPED_FORM_PAIR_ZONES =
  "table arena-zones version 1\n" +
  "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
  "crate_a class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n" +
  "debris_field class=4 cx_cm=500 cz_cm=500 radius_cm=125 height_cm=150 advantage=1 dominance=0 risk=1\n" +
  "centre class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=1 dominance=1 risk=0\n";

test("cover the same reach but a different height fails (the shipped unequal pair)", () => {
  // Before the form rule this pair printed exit 0 while LEVELS.md claimed both
  // spawns started fair: the reach rule reads the distance, not the silhouette.
  const r = checkAll({
    placementText: SHIPPED_FORM_PAIR_PLACEMENT,
    zonesText: SHIPPED_FORM_PAIR_ZONES,
    levelsText: GOOD_LEVELS_MIN,
    arenaKitText: "physical",
  });
  const a = nearestCoverRow(r.rows, SPAWN_A_CM);
  const b = nearestCoverRow(r.rows, SPAWN_B_CM);
  assert.equal(a.id + " " + a.distCm + " " + a.height_cm, "crate_a 510 100");
  assert.equal(b.id + " " + b.distCm + " " + b.height_cm, "debris_field 510 150");
  assert.deepEqual(coverReachFailures(r.rows), [], "both spawns reach cover in 3.0 s");
  assert.deepEqual(r.failures, [
    "cover spawn A starts beside crate_a (100 cm) while spawn B starts beside debris_field (150 cm): one side starts the bout with better cover",
  ]);
});

// Form fixtures (seed 2040, no randomness): each spawn's own bumper sits 510 cm
// away and each row's height matches its own placement box, so the height pair is
// the only cell the form rule can fire on.
function formCase(aHeightCm, bHeightCm) {
  const placed = JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, sy: 0.2, z: 0, physical: true },
      { id: 10, name: "crate_a", x: -5, y: aHeightCm / 200, sy: aHeightCm / 100, z: -5, physical: true },
      { id: 11, name: "crate_b", x: 5, y: bHeightCm / 200, sy: bHeightCm / 100, z: 5, physical: true },
    ],
  });
  const zones =
    "table arena-zones version 1\n" +
    "sand_floor class=0 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n" +
    "crate_a class=4 cx_cm=-500 cz_cm=-500 radius_cm=100 height_cm=" + aHeightCm + " advantage=1 dominance=0 risk=0\n" +
    "crate_b class=4 cx_cm=500 cz_cm=500 radius_cm=100 height_cm=" + bHeightCm + " advantage=1 dominance=0 risk=1\n" +
    "centre class=1 cx_cm=0 cz_cm=0 radius_cm=250 height_cm=0 advantage=1 dominance=1 risk=0\n";
  return checkAll({ placementText: placed, zonesText: zones, levelsText: GOOD_LEVELS_MIN, arenaKitText: "physical" });
}

test("cover form passes at equal heights and at HEIGHT_TOLERANCE_CM, and fails one cm past", () => {
  assert.deepEqual(formCase(100, 100).failures, [], "a same-height pair is fair from either spawn");
  assert.deepEqual(coverFormFailures(formCase(100, 100).rows), []);
  assert.deepEqual(
    coverFormFailures(formCase(100 + HEIGHT_TOLERANCE_CM, 100).rows),
    [],
    "exactly HEIGHT_TOLERANCE_CM apart passes",
  );
  assert.deepEqual(coverFormFailures(formCase(100 + HEIGHT_TOLERANCE_CM + 1, 100).rows), [
    "cover spawn A starts beside crate_a (106 cm) while spawn B starts beside crate_b (100 cm): one side starts the bout with better cover",
  ]);
});

test("a spawn with no floor bumper adds no form line", () => {
  // The off-floor fixture of the cover/dominance test (its count stays 6 there):
  // the only bumper stands at 34 m, so no spawn has a floor bumper and the reach
  // rule owns that verdict. A one-sided pair is not a form mismatch.
  const rows = parseZones(
    "table arena-zones version 1\n" +
      "sand_floor class=2 cx_cm=0 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=1 risk=0\n" +
      "crate_a class=4 cx_cm=3400 cz_cm=0 radius_cm=100 height_cm=100 advantage=1 dominance=0 risk=0\n",
  ).rows;
  assert.equal(nearestCoverRow(rows, SPAWN_A_CM), null);
  assert.equal(nearestCoverRow(rows, SPAWN_B_CM), null);
  assert.deepEqual(coverFormFailures(rows), [], "no floor bumper on either side adds no form line");
});

test("a placement actor with no numeric sy fails loud", () => {
  const noSy = JSON.stringify({ actors: [{ id: 1, name: "sand_floor", x: 0, y: 0.1, z: 0, physical: true }] });
  assert.throws(() => parsePlacement(noSy), /placement actor sand_floor misses sy/);
});

test("entrance share and possible_death stay in range", () => {
  const zones = parseZones(BAD_ZONES_SEED2040);
  const share = computeEntranceShare(zones.rows);
  assert.ok(share.permille >= 0 && share.permille <= 1000, "permille range");
  const deaths = computePossibleDeath(zones.rows);
  assert.ok(deaths >= 0, "possible_death count");
});

test("cover counts bumpers only", () => {
  const zones = parseZones(BAD_ZONES_SEED2040);
  assert.equal(computeCover(zones.rows), 0);
  const live = parseZones(readFileSync(ZONES, "utf8"));
  assert.equal(computeCover(live.rows), 2);
});

test("no_use pins every placement actor", () => {
  const actors = parsePlacement(tinyPlacement());
  assert.equal(computeNoUse(actors, GOOD_LEVELS_MIN).count, 0);
  assert.equal(computeNoUse(actors, BAD_LEVELS_SEED2040).count, 2);
});

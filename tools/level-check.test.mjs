// Run: node --test tools/level-check.test.mjs
// Level layout gate: the hand-placed arena plus zones plus LEVELS.md print
// walk time, cover and entrance share, and a bad layout must fail.
// A seeded bad layout (seed 2040, fixed strings, no randomness) proves the
// three fail rules: no holdable dominance, open floor with no cover, an
// actor with no use. Float cells and a missing header fail with a line
// number so a bad row cannot slip silent.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
  checkAll,
  computeCover,
  computeEntranceShare,
  computeNoUse,
  computePhysicalUnread,
  computePossibleDeath,
  computeWalkS,
  formatWalkS,
  parsePlacement,
  parseZones,
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
  "crate_a: only cover use. Past event: a recruit hid and lived, so veterans kick it first. Goal: explore.",
].join("\n");

function tinyPlacement() {
  return JSON.stringify({
    actors: [
      { id: 1, name: "sand_floor", x: 0, y: 0.1, z: 0, physical: true },
      { id: 10, name: "crate_a", x: 29, y: 0.5, z: 0, physical: true },
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
  assert.equal(r.entranceShare, 8, "entrance_share pinned 8, got " + r.entranceShare);
  assert.equal(r.noUse, 0);
  assert.equal(r.physicalUnread, 0);
  assert.ok(r.dominance >= 1, "one holdable dominance point");
  assert.ok(r.possibleDeath > 0, "possible_death above 0");
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

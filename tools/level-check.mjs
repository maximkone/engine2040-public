// level-check: hand-placed Rome arena layout gate.
//
// Job: read the parade placement plus the hand-made zones file and print
// walk time, cover count and entrance share, so a layout change can fail.
// The arena is hand-built (10 hand-made levels beat a generator for Rome);
// procgen is not this seat. Placement is generated
// (content/arena/tools/gen.mjs): this tool never edits it.
//
// Scale comes first: a new layout states fighter height 1800mm, sword reach
// 2000mm (rome-spec range bound, sweep.rs HURT_RADIUS) and dodge distance
// 2m before any geometry. Spawns (-4,0,0)/(+4,0,0) 8 m apart at 170 cm/s
// march (docs/plan/rome-spec.md:17,31) is 800/170 = 4.7 s. Band 3.0..6.0 s
// pins greybox walk time (Rogers L9 reachability: a duel must be walkable
// but not instant); the director SLICE.md holds no walk band today, so this
// tool owns the band until the director names one.
//
// Zones file format (content/design/arena-zones.rows):
//   first non-blank non-# line: `table arena-zones version 1`
//   rows: `<actor_or_zone> class=<0..4> cx_cm=<int> cz_cm=<int>
//     radius_cm=<int> height_cm=<int> advantage=<0|1> dominance=<0|1>
//     risk=<0|1>`; whole numbers only (optional leading -), `#` comments.
//   class: 0 none, 1 magnet, 2 curtain, 3 generator, 4 bumper.
//   A row named after an actor must carry that actor's placement centre
//   (within 50 cm) and that actor's placement box height (`sy` metres, within
//   5 cm), so a moved or reshaped actor cannot keep a stale row.
//
// Prints one line each (first five are the proof):
//   walk_s <spawn to spawn, 1 decimal>
//   cover <n> bumpers
//   entrance_share <permille of duel floor within 3 m of gate_tunnel>
//   no_use <n actors without a use line in LEVELS.md>
//   physical_unread <n>
// plus (tournament tapes exist only later):
//   deaths_per_zone <per-zone kills or none (no tapes)>
//   possible_death <floor cells within 2000mm of a zone centre>
// Exit 1 when a rule fails: no holdable dominance point, a dominance flag that
// no single fighter can hold (radius above the hold cap or centre off the
// floor), open floor with no cover, a bumper standing off the duel floor (the
// fight never reaches it, so it is dressing, not cover), an advantage point
// standing off the duel floor (the fight never reaches it, so it is dead
// space) or no advantage point on the floor at all, a risk point standing off
// the duel floor (the fight never reaches it, so the flag is a note, not data;
// no minimum risk-point count is required, see the rule comment), an actor with
// no use, a zone row whose centre drifted off its actor's placement pose or
// whose `height_cm` disagrees with the actor's placement box height (a moved or
// reshaped actor with a stale row else passes silent), scale missing, walk out
// of band, bad rows.
// A spawn with no floor bumper inside `COVER_REACH_CM` starts the bout uncovered.
// Two spawns whose nearest cover differs in `height_cm` by more than
// `HEIGHT_TOLERANCE_CM` fail too: one side starts the bout with better cover.
// Exit 2 on bad CLI, missing file or unparsable rows (with line number).
//
// Usage:
//   node tools/level-check.mjs [--placement P] [--zones Z] [--levels L]
//     [--arena-kit K] [--help]
// Defaults are the repo files; all paths resolve against the repo root.
//
// Owner: design level seat. Units: cm in zones, mm for reach, s for walk,
// permille 0..1000. Deterministic: same inputs give same lines (integer grid,
// no floats in files, no wall clock, no HashMap order).
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = dirname(HERE);

export const SPAWN_A_CM = [-400, 0];
export const SPAWN_B_CM = [400, 0];
export const SPEED_CM_S = 170;
export const FLOOR_HALF_CM = 1000;
export const ENTRANCE_RADIUS_CM = 300;
/**
 * Cover reach: each spawn needs a floor bumper within this many cm, so one side
 * never starts the bout in the open (600 cm is 3.5 s at the 170 cm/s march,
 * inside the walk band this tool owns; the live mirrored pair reads 510 cm).
 */
export const COVER_REACH_CM = 600;
export const REACH_MM = 2000;
export const FIGHTER_HEIGHT_MM = 1800;
export const DODGE_CM = 200;
export const DEATH_RADIUS_CM = 200;
/** A zone row's centre must sit this close to its actor's placement pose. */
export const ZONE_POS_TOLERANCE_CM = 50;
/**
 * A zone row named after a placement actor must carry that actor's box height
 * (`sy` in cm) within this many cm: `height_cm` is the design claim (the
 * silhouette the fight reads) and the placement box is what the kit builds.
 * Units cm; the tool owns the tolerance the same way it owns the walk band.
 */
export const HEIGHT_TOLERANCE_CM = 5;
export const WALK_MIN_S = 3.0;
export const WALK_MAX_S = 6.0;
export const GRID_STEP_CM = 50;
/**
 * One fighter holds a dominance point alone while its radius stays inside the
 * hold circle: sword reach 200 cm (2000mm) plus one 100 cm body step. Above it
 * a second fighter stands outside reach on the far side of the circle. The
 * director names no hold cap yet (SLICE.md holds none), so this tool owns the
 * constant the same way it owns the walk band; the 3 m centre circle (r 250 cm)
 * fits, a 15 m sand_floor row does not. Change only with a measured layout.
 */
export const DOMINANCE_HOLD_RADIUS_CM = 300;

export const DEFAULT_PLACEMENT = join(ROOT, "content", "arena", "placement.json");
export const DEFAULT_ZONES = join(ROOT, "content", "design", "arena-zones.rows");
export const DEFAULT_LEVELS = join(ROOT, "docs", "design", "LEVELS.md");
export const DEFAULT_ARENA_KIT = join(ROOT, "crates", "game", "src", "arena_kit.rs");

const INT_RE = /^-?\d+$/;

function fail(msg) {
  throw new Error(msg);
}

/** Spawn-to-spawn walk seconds (8 m at 170 cm/s is 4.7 s). Pure. */
export function computeWalkS(a = SPAWN_A_CM, b = SPAWN_B_CM, speed = SPEED_CM_S) {
  const dx = b[0] - a[0];
  const dz = b[1] - a[1];
  const distCm = Math.hypot(dx, dz);
  return distCm / speed;
}

export function formatWalkS(v) {
  return (Math.round(v * 10) / 10).toFixed(1);
}

/** Parse placement JSON text. Returns [{ id, name, xCm, zCm, sizeYCm, physical }]. */
export function parsePlacement(text, label = "placement") {
  let data;
  try {
    data = JSON.parse(text);
  } catch (err) {
    fail(label + ": unparsable JSON (" + (err && err.message ? err.message : err) + ")");
  }
  if (!data || !Array.isArray(data.actors)) fail(label + ": misses actors array");
  const out = [];
  const seen = new Set();
  for (const a of data.actors) {
    if (typeof a.name !== "string" || a.name.length === 0) fail(label + ": actor misses name");
    if (seen.has(a.name)) fail(label + ": duplicate actor " + a.name);
    seen.add(a.name);
    const x = Number(a.x);
    const z = Number(a.z);
    if (!Number.isFinite(x) || !Number.isFinite(z)) fail(label + ": actor " + a.name + " has non-finite pose");
    // The box height is a placement fact; a row's height_cm is the design
    // claim, so the claim needs this number to be checked against.
    const sy = Number(a.sy);
    if (!Number.isFinite(sy)) fail(label + ": placement actor " + a.name + " misses sy");
    // Placement units are meters (content/arena/README.md); zones use cm.
    out.push({
      id: a.id,
      name: a.name,
      xCm: Math.round(x * 100),
      zCm: Math.round(z * 100),
      sizeYCm: Math.round(sy * 100),
      physical: a.physical === true,
      physicalPresent: typeof a.physical === "boolean",
    });
  }
  if (out.length === 0) fail(label + ": no actors");
  return out;
}

/** Parse zones rows text. Throws with line number on any bad row. */
export function parseZones(text, label = "zones") {
  const lines = text.split(/\r?\n/);
  let headerLine = 0;
  let version = 0;
  const rows = [];
  const seen = new Set();
  for (let i = 0; i < lines.length; i += 1) {
    const n = i + 1;
    let line = lines[i];
    const hash = line.indexOf("#");
    if (hash >= 0) line = line.slice(0, hash);
    line = line.trim();
    if (line === "") continue;
    if (headerLine === 0) {
      const m = line.match(/^table\s+arena-zones\s+version\s+(\d+)$/);
      if (!m) fail(label + " line " + n + ": first row must be `table arena-zones version 1`");
      version = Number(m[1]);
      if (!Number.isInteger(version) || version <= 0) fail(label + " line " + n + ": bad version");
      headerLine = n;
      continue;
    }
    const parts = line.split(/\s+/);
    const id = parts[0];
    if (!id || id.includes("=")) fail(label + " line " + n + ": misses actor id");
    if (seen.has(id)) fail(label + " line " + n + ": duplicate row " + id);
    seen.add(id);
    const want = ["class", "cx_cm", "cz_cm", "radius_cm", "height_cm", "advantage", "dominance", "risk"];
    const got = {};
    for (const p of parts.slice(1)) {
      const eq = p.indexOf("=");
      if (eq < 0) fail(label + " line " + n + ": bad cell `" + p + "`");
      const k = p.slice(0, eq);
      const v = p.slice(eq + 1);
      if (!want.includes(k)) fail(label + " line " + n + ": unknown key " + k);
      if (got[k] !== undefined) fail(label + " line " + n + ": duplicate key " + k);
      if (!INT_RE.test(v)) fail(label + " line " + n + ": " + k + " must be a whole number, got `" + v + "`");
      got[k] = Number(v);
    }
    for (const k of want) {
      if (got[k] === undefined) fail(label + " line " + n + ": missing key " + k);
    }
    if (got.class < 0 || got.class > 4) fail(label + " line " + n + ": class must be 0..4");
    if (got.radius_cm <= 0) fail(label + " line " + n + ": radius_cm must be above 0");
    for (const k of ["advantage", "dominance", "risk"]) {
      if (got[k] !== 0 && got[k] !== 1) fail(label + " line " + n + ": " + k + " must be 0|1");
    }
    rows.push({ id, ...got, line: n });
  }
  if (headerLine === 0) fail(label + ": misses `table arena-zones version 1` header");
  if (rows.length === 0) fail(label + ": no zone rows");
  return { version, rows };
}

/** True when LEVELS.md states scale before geometry. Pure. */
export function checkScale(levelsText) {
  const t = levelsText.toLowerCase();
  const hasHeight = t.includes("1800") || t.includes("1.8 m") || t.includes("1.8m");
  const hasReach = t.includes("2000");
  const hasDodge = t.includes("dodge");
  return hasHeight && hasReach && hasDodge;
}

/** Map of actor use lines: lines starting `<name>:` (exact name match). */
export function parseUseLines(levelsText) {
  const map = new Map();
  for (const raw of levelsText.split(/\r?\n/)) {
    const m = raw.match(/^\s*([A-Za-z0-9_]+)\s*:\s*(.+)$/);
    if (m) map.set(m[1], m[2]);
  }
  return map;
}

/** Count placement actors with no use line in LEVELS.md. Pure. */
export function computeNoUse(actors, levelsText) {
  const uses = parseUseLines(levelsText);
  let n = 0;
  const missing = [];
  for (const a of actors) {
    if (!uses.has(a.name)) {
      n += 1;
      missing.push(a.name);
    }
  }
  return { count: n, missing };
}

/** Count bumpers (class 4). Pure. */
export function computeCover(zoneRows) {
  return zoneRows.filter((r) => r.class === 4).length;
}

/**
 * True when a zone centre stands on the duel floor, the 10 m half-extent fight
 * ring (rome-spec `halfExtentCm 1000`). Off it, the fight never reaches the
 * zone, so its flags mean nothing. Pure.
 */
export function onDuelFloor(row) {
  return Math.abs(row.cx_cm) <= FLOOR_HALF_CM && Math.abs(row.cz_cm) <= FLOOR_HALF_CM;
}

/** Bumper rows standing off the duel floor: dressing, not cover. Pure. */
export function coverOffFloor(zoneRows) {
  return zoneRows.filter((r) => r.class === 4 && !onDuelFloor(r));
}

/**
 * The nearest floor bumper (class 4 and `onDuelFloor`) as a row plus its
 * distance in cm, or null when none exists: an off-floor bumper is not cover,
 * the fight never reaches it. Ties take the first row in file order, so a
 * verdict never depends on how a caller ordered equal rows. Pure.
 */
export function nearestCoverRow(zoneRows, spawnCm = SPAWN_A_CM) {
  let best = null;
  for (const r of zoneRows) {
    if (r.class !== 4 || !onDuelFloor(r)) continue;
    const distCm = Math.round(Math.hypot(r.cx_cm - spawnCm[0], r.cz_cm - spawnCm[1]));
    if (best === null || distCm < best.distCm) best = { ...r, distCm };
  }
  return best;
}

/**
 * Distance in cm from a spawn to the nearest floor bumper, or null when none
 * exists. Pure: the `nearestCoverRow` distance, so both readers agree.
 */
export function nearestCoverCm(zoneRows, spawnCm = SPAWN_A_CM) {
  const best = nearestCoverRow(zoneRows, spawnCm);
  return best === null ? null : best.distCm;
}

function formatNearestCover(distCm) {
  return distCm === null ? "no floor bumper" : distCm + " cm";
}

/** Spawns with no floor bumper inside COVER_REACH_CM: one side starts uncovered. */
export function coverReachFailures(zoneRows) {
  const out = [];
  for (const [label, cm] of [["A", SPAWN_A_CM], ["B", SPAWN_B_CM]]) {
    const d = nearestCoverCm(zoneRows, cm);
    if (d === null || d > COVER_REACH_CM) {
      out.push(
        "cover spawn " + label + " at (" + cm[0] + "," + cm[1] + ") has no bumper within " +
          COVER_REACH_CM + " cm (nearest " + formatNearestCover(d) + "): one side starts the bout uncovered",
      );
    }
  }
  return out;
}

/**
 * Both spawns' nearest floor bumper standing at a different height: the taller
 * one hides its fighter behind more silhouette, so that side wins the first
 * exchange (Worlds II.6 advantage points). A spawn with no floor bumper adds no
 * line here - the reach rule already fails that spawn. Pure.
 */
export function coverFormFailures(zoneRows) {
  const a = nearestCoverRow(zoneRows, SPAWN_A_CM);
  const b = nearestCoverRow(zoneRows, SPAWN_B_CM);
  if (a === null || b === null) return [];
  if (Math.abs(a.height_cm - b.height_cm) <= HEIGHT_TOLERANCE_CM) return [];
  return [
    "cover spawn A starts beside " + a.id + " (" + a.height_cm + " cm) while spawn B starts beside " +
      b.id + " (" + b.height_cm + " cm): one side starts the bout with better cover",
  ];
}

/** Advantage rows standing off the duel floor: dead space, not an advantage. Pure. */
export function advantageOffFloor(zoneRows) {
  return zoneRows.filter((r) => r.advantage === 1 && !onDuelFloor(r));
}

/** Risk rows standing off the duel floor: dead space, not a risk. Pure. */
export function riskOffFloor(zoneRows) {
  return zoneRows.filter((r) => r.risk === 1 && !onDuelFloor(r));
}

/** Dominance rows no single fighter can hold alone. Pure. */
export function dominanceUnheld(zoneRows) {
  return zoneRows.filter(
    (r) => r.dominance === 1 && (r.radius_cm > DOMINANCE_HOLD_RADIUS_CM || !onDuelFloor(r)),
  );
}

/** Floor grid: centres at -975..975 step 50 (40x40 = 1600 cells). */
export function floorCells() {
  const cells = [];
  for (let x = -FLOOR_HALF_CM + GRID_STEP_CM / 2; x < FLOOR_HALF_CM; x += GRID_STEP_CM) {
    for (let z = -FLOOR_HALF_CM + GRID_STEP_CM / 2; z < FLOOR_HALF_CM; z += GRID_STEP_CM) {
      cells.push([x, z]);
    }
  }
  return cells;
}

/** Permille of duel floor cells within 3 m of the gate_tunnel zone. Pure. */
export function computeEntranceShare(zoneRows) {
  const gate = zoneRows.find((r) => r.id === "gate_tunnel");
  if (!gate) return { permille: 0, inside: 0, total: 1600 };
  let inside = 0;
  let total = 0;
  for (const [x, z] of floorCells()) {
    total += 1;
    const dx = x - gate.cx_cm;
    const dz = z - gate.cz_cm;
    if (dx * dx + dz * dz <= ENTRANCE_RADIUS_CM * ENTRANCE_RADIUS_CM) inside += 1;
  }
  return { permille: Math.round((1000 * inside) / total), inside, total };
}

/** No-play possible_death: floor cells within 2000mm of any zone centre. */
export function computePossibleDeath(zoneRows) {
  let n = 0;
  for (const [x, z] of floorCells()) {
    for (const r of zoneRows) {
      const dx = x - r.cx_cm;
      const dz = z - r.cz_cm;
      if (dx * dx + dz * dz <= DEATH_RADIUS_CM * DEATH_RADIUS_CM) {
        n += 1;
        break;
      }
    }
  }
  return n;
}

/**
 * physical_unread: 0 when crates/game/src/arena_kit.rs reads the flag
 * (contains the word `physical`, the rg definition); else the count of
 * physical actors. Pure.
 */
export function computePhysicalUnread(actors, arenaKitText) {
  if (typeof arenaKitText === "string" && arenaKitText.includes("physical")) return 0;
  return actors.filter((a) => a.physical).length;
}

/** Full check. Returns the numbers plus failures (exit 1 when non-empty). */
export function checkAll({ placementText, zonesText, levelsText, arenaKitText }) {
  const failures = [];
  const actors = parsePlacement(placementText);
  const zones = parseZones(zonesText);
  const rows = zones.rows;

  // Every placement actor needs one zone row (centre extra allowed), and the
  // row must agree with the placement: its centre with the actor's pose and
  // its height_cm with the actor's box height. A moved or reshaped actor with
  // a stale row passes every other rule, so these rules are what make a layout
  // change fail.
  const zoneIds = new Set(rows.map((r) => r.id));
  const missingZones = actors.filter((a) => !zoneIds.has(a.name)).map((a) => a.name);
  if (missingZones.length > 0) failures.push("zones miss actors: " + missingZones.join(", "));
  for (const a of actors) {
    const row = rows.find((r) => r.id === a.name);
    if (!row) continue;
    if (Math.abs(row.cx_cm - a.xCm) > ZONE_POS_TOLERANCE_CM || Math.abs(row.cz_cm - a.zCm) > ZONE_POS_TOLERANCE_CM) {
      failures.push(
        "zone row " + a.name + " centre (" + row.cx_cm + "," + row.cz_cm + ") disagrees with placement (" + a.xCm + "," + a.zCm + ")",
      );
    }
    if (Math.abs(row.height_cm - a.sizeYCm) > HEIGHT_TOLERANCE_CM) {
      failures.push(
        "zone row " + a.name + " height_cm " + row.height_cm + " disagrees with placement box height " + a.sizeYCm,
      );
    }
  }

  const walkS = computeWalkS();
  const walkOne = formatWalkS(walkS);
  if (!(walkS >= WALK_MIN_S && walkS <= WALK_MAX_S)) {
    failures.push("walk_s " + walkOne + " outside band " + WALK_MIN_S.toFixed(1) + ".." + WALK_MAX_S.toFixed(1));
  }

  const cover = computeCover(rows);
  const entrance = computeEntranceShare(rows);
  const noUse = computeNoUse(actors, levelsText);
  const physicalUnread = computePhysicalUnread(actors, arenaKitText);
  const possibleDeath = computePossibleDeath(rows);

  if (!checkScale(levelsText)) {
    failures.push("scale missing: LEVELS.md must state fighter height 1800mm, reach 2000mm and dodge distance first");
  }
  const dominance = rows.filter((r) => r.dominance === 1).length;
  if (dominance === 0) failures.push("no dominance point one fighter can hold alone");
  if (cover === 0) failures.push("open floor with no cover");
  // A flag is a claim; these rules are what make the claim hold. The seed
  // cases came from the real history: debris_field sat at 34 m (off the floor)
  // for three days as `cover 2 bumpers`, a dominance flag on the 30 m
  // sand_floor row passed while no fighter can hold it, and the stands carried
  // advantage=1 from 20 m out, where the fight never reaches it.
  for (const r of coverOffFloor(rows)) {
    failures.push(
      "cover " + r.id + " stands outside the duel floor at (" + r.cx_cm + "," + r.cz_cm + "): the fight never reaches it",
    );
  }
  for (const f of coverReachFailures(rows)) failures.push(f);
  // The two spawns' cover must be the same form too: a taller crate on one side
  // is a first-exchange advantage the reach rule cannot see.
  for (const f of coverFormFailures(rows)) failures.push(f);
  for (const r of advantageOffFloor(rows)) {
    failures.push(
      "advantage " + r.id + " stands outside the duel floor at (" + r.cx_cm + "," + r.cz_cm + "): the fight never reaches it",
    );
  }
  if (!rows.some((r) => r.advantage === 1 && onDuelFloor(r))) {
    failures.push("no advantage point inside the duel floor");
  }
  // A risk point off the floor is dead space (Worlds II.6): the fight never
  // reaches it, so the flag is a note, not data. No "at least one risk point"
  // rule: the director's bar list names no risk-point count and the entrance
  // risk point is already named in LEVELS.md (Zone circle), so a minimum would
  // fail a legal flat layout.
  for (const r of riskOffFloor(rows)) {
    failures.push(
      "risk " + r.id + " stands outside the duel floor at (" + r.cx_cm + "," + r.cz_cm + "): the fight never reaches it",
    );
  }
  for (const r of dominanceUnheld(rows)) {
    failures.push(
      "dominance " + r.id + " is not holdable alone (radius_cm " + r.radius_cm + " above cap " + DOMINANCE_HOLD_RADIUS_CM + " or centre off the duel floor)",
    );
  }
  if (noUse.count > 0) failures.push("actor with no use: " + noUse.missing.join(", "));
  if (entrance.permille < 0 || entrance.permille > 1000) failures.push("entrance_share out of range");
  if (possibleDeath === 0) failures.push("dead space: no floor cell within 2000mm of a zone centre");

  return {
    actors,
    rows,
    walkS,
    walkOne,
    cover,
    entranceShare: entrance.permille,
    entranceInside: entrance.inside,
    entranceTotal: entrance.total,
    noUse: noUse.count,
    noUseMissing: noUse.missing,
    physicalUnread,
    possibleDeath,
    dominance,
    failures,
  };
}

export function formatLines(r) {
  return [
    "walk_s " + r.walkOne,
    "cover " + r.cover + " bumpers",
    "entrance_share " + r.entranceShare,
    "no_use " + r.noUse,
    "physical_unread " + r.physicalUnread,
    "deaths_per_zone none (no tapes)",
    "possible_death " + r.possibleDeath,
  ];
}

export function parseArgs(argv) {
  const out = {
    placement: DEFAULT_PLACEMENT,
    zones: DEFAULT_ZONES,
    levels: DEFAULT_LEVELS,
    arenaKit: DEFAULT_ARENA_KIT,
    help: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === "--placement") {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith("--")) return { bad: true };
      out.placement = resolve(v);
      i += 1;
    } else if (a === "--zones") {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith("--")) return { bad: true };
      out.zones = resolve(v);
      i += 1;
    } else if (a === "--levels") {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith("--")) return { bad: true };
      out.levels = resolve(v);
      i += 1;
    } else if (a === "--arena-kit") {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith("--")) return { bad: true };
      out.arenaKit = resolve(v);
      i += 1;
    } else if (a === "--help" || a === "-h") {
      out.help = true;
    } else {
      return { bad: true };
    }
  }
  return out;
}

export function helpText() {
  return [
    "Usage: node tools/level-check.mjs [--placement P] [--zones Z] [--levels L] [--arena-kit K] [--help]",
    "  Check the hand-placed Rome arena layout: walk time, cover, entrance share.",
    "  Prints walk_s, cover, entrance_share, no_use, physical_unread,",
    "  deaths_per_zone and possible_death, one per line. Exit 1 when a rule fails.",
    "Defaults: content/arena/placement.json, content/design/arena-zones.rows,",
    "  docs/design/LEVELS.md, crates/game/src/arena_kit.rs.",
  ].join("\n");
}

export function main(argv = []) {
  const opts = parseArgs(argv);
  if (opts.bad) {
    process.stderr.write(helpText() + "\n");
    return 2;
  }
  if (opts.help) {
    process.stdout.write(helpText() + "\n");
    return 0;
  }
  let placementText;
  let zonesText;
  let levelsText;
  let arenaKitText = "";
  try {
    placementText = readFileSync(opts.placement, "utf8");
  } catch (err) {
    process.stderr.write(String(opts.placement) + ": cannot read placement (" + (err && err.message ? err.message : err) + ")\n");
    return 2;
  }
  try {
    zonesText = readFileSync(opts.zones, "utf8");
  } catch (err) {
    process.stderr.write(String(opts.zones) + ": cannot read zones (" + (err && err.message ? err.message : err) + ")\n");
    return 2;
  }
  try {
    levelsText = readFileSync(opts.levels, "utf8");
  } catch (err) {
    process.stderr.write(String(opts.levels) + ": cannot read levels (" + (err && err.message ? err.message : err) + ")\n");
    return 2;
  }
  try {
    arenaKitText = readFileSync(opts.arenaKit, "utf8");
  } catch {
    arenaKitText = "";
  }
  let r;
  try {
    r = checkAll({ placementText, zonesText, levelsText, arenaKitText });
  } catch (err) {
    process.stderr.write(String(err && err.message ? err.message : err) + "\n");
    return 2;
  }
  process.stdout.write(formatLines(r).join("\n") + "\n");
  if (r.failures.length > 0) {
    for (const f of r.failures) process.stderr.write("rule: " + f + "\n");
    return 1;
  }
  return 0;
}

if (import.meta.main) {
  process.exit(main(process.argv.slice(2)));
}

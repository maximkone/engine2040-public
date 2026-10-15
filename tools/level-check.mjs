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
// Exit 1 when a rule fails: no holdable dominance point, open floor with no
// cover, an actor with no use, scale missing, walk out of band, bad rows.
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
export const REACH_MM = 2000;
export const FIGHTER_HEIGHT_MM = 1800;
export const DODGE_CM = 200;
export const DEATH_RADIUS_CM = 200;
export const WALK_MIN_S = 3.0;
export const WALK_MAX_S = 6.0;
export const GRID_STEP_CM = 50;

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

/** Parse placement JSON text. Returns [{ id, name, xCm, zCm, physical }]. */
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
    // Placement units are meters (content/arena/README.md); zones use cm.
    out.push({
      id: a.id,
      name: a.name,
      xCm: Math.round(x * 100),
      zCm: Math.round(z * 100),
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

  // Every placement actor needs one zone row (centre extra allowed).
  const zoneIds = new Set(rows.map((r) => r.id));
  const missingZones = actors.filter((a) => !zoneIds.has(a.name)).map((a) => a.name);
  if (missingZones.length > 0) failures.push("zones miss actors: " + missingZones.join(", "));

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

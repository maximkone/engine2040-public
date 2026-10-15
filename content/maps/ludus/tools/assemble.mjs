// WD2 ludus-hub assembler — deterministic, dependency-free (node stdlib only).
// Owns `content/maps/ludus/`: assembles the playable Rome loop hub from the
// CT9 kit (no new geometry bytes, no donor bytes) and validates every gameplay
// claim the proof crate re-proves in Rust: the 8 actors are the CT9
// `placement_ludus.json` validity probe verbatim, zones reference live actors,
// spawns stand on free ground, the nav rect covers every route endpoint, and
// the stream block is a seamless WD1 2x2. Emits map.json, cells.json,
// tools/map_data.rs and manifest.json — byte-identical on every run.
import { writeFileSync, mkdirSync, readFileSync, copyFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
const LUDUS = join(HERE, '..');
const MAPS = join(LUDUS, '..');

const fail = (msg) => { console.error(`LUDUS-ASSEMBLE FAIL: ${msg}`); process.exit(1); };

// ---------------------------------------------------------------------------
// Hub definition. Actors are not authored here: they must deep-equal the CT9
// validity probe, so the kit consumer check stays the map's spatial core.
// ---------------------------------------------------------------------------
const SEED = 20260925;
const MAP_VERSION = 1;
const KIT_MANIFEST_SHA = '9A1AF61993D7A4167688B47CFF0F0F398003D79B5DAA521F90D26D09059FDF1A';
const LUDUS_ASSETS = ['yard_floor', 'barracks', 'blacksmith', 'notice_board', 'training_post', 'weapon_rack', 'street_wall'];

const ZONES = [
  {
    name: 'practice_yard', rect: { x0: -62, x1: -2, z0: -7, z1: 7 },
    members: ['yard', 'pell_west', 'pell_east', 'rack'],
    note: 'the whole training ground: drill floor, both pells and the rack',
  },
  {
    name: 'blacksmith', rect: { x0: -24, x1: -15.6, z0: -5, z1: 5 },
    members: ['blacksmith'],
    note: 'workshop corner inside the grounds: hut, forge, stump and anvil',
    interact: { name: 'forge', x: -16.2, z: 1.0 },
  },
  {
    name: 'party_board', rect: { x0: -14.6, x1: -10, z0: -3, z1: 3 },
    members: ['party_board'],
    note: 'board corner inside the grounds: posts, panel and three slips',
    interact: { name: 'board', x: -13, z: 1.2 },
  },
];

const SPAWNS = [
  { name: 'player', x: -48, z: 0, yaw_deg: 90, note: 'on the drill floor, facing the pells' },
  { name: 'drill_a', x: -20, z: -4, yaw_deg: 45, note: 'south drill mark' },
  { name: 'drill_b', x: -20, z: 4, yaw_deg: 135, note: 'north drill mark' },
  { name: 'drill_c', x: -38, z: 0, yaw_deg: 90, note: 'west drill mark between floor and hall' },
];

const NAV = { x0: -62, x1: -2, z0: -7, z1: 7, cell_m: 1.0 };
const NAV_ROUTES = [
  { from: 'player', to: 'pell_west_stand', to_x: -8, to_z: -2 },
  { from: 'player', to: 'pell_east_stand', to_x: -8, to_z: 2 },
  { from: 'player', to: 'rack_stand', to_x: -5, to_z: 1.2 },
];

const STREAM = { size: 16, cells: [{ x: 0, z: 0 }, { x: 1, z: 0 }, { x: 0, z: 1 }, { x: 1, z: 1 }] };
const CELL_COST = 23 + 256 + 32; // WD1 16-texel baked cost: header + heights + hole bits.

// ---------------------------------------------------------------------------
// Load + validate.
// ---------------------------------------------------------------------------
const probe = JSON.parse(readFileSync(join(MAPS, 'placement_ludus.json'), 'utf8'));
const actors = probe.actors;
const byName = Object.fromEntries(actors.map((a) => [a.name, a]));
const foot = (a) => ({ x0: a.x - a.sx / 2, x1: a.x + a.sx / 2, z0: a.z - a.sz / 2, z1: a.z + a.sz / 2 });
const overlaps = (r, s) => r.x0 < s.x1 && s.x0 < r.x1 && r.z0 < s.z1 && s.z0 < r.z1;
const inside = (r, s) => s.x0 <= r.x0 && r.x1 <= s.x1 && s.z0 <= r.z0 && r.z1 <= s.z1;

// 1. The map consumes the CT9 probe verbatim: 8 actors, stable ids and names.
if (actors.length !== 8) fail(`actor count ${actors.length}, want 8`);
{
  const ids = actors.map((a) => a.id).sort((a, b) => a - b);
  if (ids.join(',') !== '1,2,3,4,5,6,7,8') fail(`actor ids ${ids}, want 1..8`);
  const names = new Set(actors.map((a) => a.name));
  if (names.size !== 8) fail('actor names are not unique');
  for (const a of actors) {
    if (!LUDUS_ASSETS.includes(a.asset)) fail(`actor ${a.name} uses non-ludus asset ${a.asset}`);
    if (a.parent !== null || a.physical !== true || a.vx !== 0 || a.vz !== 0 || a.yaw !== 0) {
      fail(`actor ${a.name} broke the probe contract (parent/physical/velocity/yaw)`);
    }
    // Resting: every footprint sits on y = 0 exactly.
    if (a.y - a.sy / 2 !== 0) fail(`actor ${a.name} floats or sinks: y=${a.y} sy=${a.sy}`);
    const f = foot(a);
    if (Math.max(Math.abs(f.x0), Math.abs(f.x1), Math.abs(f.z0), Math.abs(f.z1)) > 99) {
      fail(`actor ${a.name} leaves the +-99 m arena`);
    }
  }
  // Disjoint XZ footprints, pairwise.
  for (let i = 0; i < actors.length; i++) {
    for (let j = i + 1; j < actors.length; j++) {
      if (overlaps(foot(actors[i]), foot(actors[j]))) {
        fail(`footprints overlap: ${actors[i].name} vs ${actors[j].name}`);
      }
    }
  }
}

// 2. Zones reference live actors, contain their members, and hold their
// interact points on free ground. Zones nest inside the grounds by design:
// the workshop and board corners sit at the yard edge, so only the two
// corners must be disjoint from each other.
for (const z of ZONES) {
  for (const m of z.members) {
    if (!byName[m]) fail(`zone ${z.name} names missing actor ${m}`);
    if (!inside(foot(byName[m]), z.rect)) fail(`zone ${z.name} does not contain ${m}`);
  }
  if (z.rect.x0 >= z.rect.x1 || z.rect.z0 >= z.rect.z1) fail(`zone ${z.name} is degenerate`);
  if (z.interact) {
    const p = z.interact;
    if (p.x < z.rect.x0 || p.x > z.rect.x1 || p.z < z.rect.z0 || p.z > z.rect.z1) {
      fail(`interact ${p.name} stands outside zone ${z.name}`);
    }
    for (const a of actors) {
      if (a.name === 'yard') continue; // the floor is ground, not an obstacle.
      const f = foot(a);
      if (p.x > f.x0 && p.x < f.x1 && p.z > f.z0 && p.z < f.z1) {
        fail(`interact ${p.name} stands inside ${a.name}`);
      }
    }
  }
}
{
  const smith = ZONES[1].rect, board = ZONES[2].rect;
  if (overlaps(smith, board)) fail('workshop and board corners must not overlap');
}

// 3. Spawns stand on free ground (the drill floor excepted: it is ground),
// inside the nav rect, with sane facings.
for (const s of SPAWNS) {
  for (const a of actors) {
    if (a.name === 'yard') continue;
    const f = foot(a);
    if (s.x > f.x0 && s.x < f.x1 && s.z > f.z0 && s.z < f.z1) {
      fail(`spawn ${s.name} stands inside ${a.name}`);
    }
  }
  if (s.x < NAV.x0 || s.x > NAV.x1 || s.z < NAV.z0 || s.z > NAV.z1) {
    fail(`spawn ${s.name} stands outside the walkable rect`);
  }
  if (!Number.isInteger(s.yaw_deg) || s.yaw_deg < 0 || s.yaw_deg >= 360) {
    fail(`spawn ${s.name} has a bad facing`);
  }
}

// 4. Every nav route endpoint is walkable ground: the stand points sit beside
// their prop, never inside it, and every route starts at a live spawn.
const spawnByName = Object.fromEntries(SPAWNS.map((s) => [s.name, s]));
for (const r of NAV_ROUTES) {
  if (!spawnByName[r.from]) fail(`route starts at missing spawn ${r.from}`);
  for (const a of actors) {
    if (a.name === 'yard') continue;
    const f = foot(a);
    if (r.to_x > f.x0 && r.to_x < f.x1 && r.to_z > f.z0 && r.to_z < f.z1) {
      fail(`route stand ${r.to} sits inside ${a.name}`);
    }
  }
  if (r.to_x < NAV.x0 || r.to_x > NAV.x1 || r.to_z < NAV.z0 || r.to_z > NAV.z1) {
    fail(`route stand ${r.to} sits outside the walkable rect`);
  }
}

// 5. The stream block is a seamless WD1 2x2: valid coords and size, hub first.
if (STREAM.size !== 16) fail(`stream size ${STREAM.size}, want 16`);
if (STREAM.cells.length !== 4) fail('stream block must be exactly 2x2');
for (const c of STREAM.cells) {
  if (!Number.isInteger(c.x) || !Number.isInteger(c.z)) fail('stream coords must be integers');
  if (Math.abs(c.x) > 512 || Math.abs(c.z) > 512) fail('stream coord past MAX_COORD');
}
{
  const key = STREAM.cells.map((c) => `${c.x},${c.z}`).sort().join('|');
  if (key !== '0,0|0,1|1,0|1,1') fail(`stream block ${key} is not the hub 2x2`);
}
const priority = (c) => 9 - (Math.abs(c.x) + Math.abs(c.z));

// ---------------------------------------------------------------------------
// Emit: map.json, cells.json, tools/map_data.rs, manifest.json.
// ---------------------------------------------------------------------------
mkdirSync(join(LUDUS, 'tools'), { recursive: true });
const emit = (rel, buf) => {
  writeFileSync(join(LUDUS, rel), buf);
  return buf;
};
const sha = (buf) => createHash('sha256').update(buf).digest('hex');

const map = {
  map: 'ludus',
  version: MAP_VERSION,
  seed: SEED,
  sky: '../hdri/maps_day.hdr',
  kit_manifest_sha256: KIT_MANIFEST_SHA,
  actors,
  zones: ZONES,
  spawns: SPAWNS,
  nav: { ...NAV, routes: NAV_ROUTES },
  stream: { size: STREAM.size, frame_bytes: CELL_COST },
};
const mapBuf = Buffer.from(JSON.stringify(map, null, 2) + '\n');
emit('map.json', mapBuf);

const cells = {
  map: 'ludus',
  seed: SEED,
  size: STREAM.size,
  frame_bytes: CELL_COST,
  cells: STREAM.cells.map((c) => ({ x: c.x, z: c.z, priority: priority(c), cost_bytes: CELL_COST })),
};
const cellsBuf = Buffer.from(JSON.stringify(cells, null, 2) + '\n');
emit('cells.json', cellsBuf);

const f32 = (v) => (Number.isInteger(v) ? `${v}.0` : `${v}`);
const rs = `// Generated by tools/assemble.mjs — do not hand-edit.
pub const MAP_NAME: &str = "ludus";
pub const MAP_VERSION: u32 = ${MAP_VERSION};
pub const SEED: u64 = ${SEED};
pub const CELL_SIZE: u16 = ${STREAM.size};
pub const CELL_COST: u64 = ${CELL_COST};
pub struct Placed { pub id: u32, pub name: &'static str, pub asset: &'static str, pub x: f32, pub y: f32, pub z: f32, pub sx: f32, pub sy: f32, pub sz: f32 }
pub const ACTORS: &[Placed] = &[
${actors.map((a) => `  Placed { id: ${a.id}, name: "${a.name}", asset: "${a.asset}", x: ${f32(a.x)}, y: ${f32(a.y)}, z: ${f32(a.z)}, sx: ${f32(a.sx)}, sy: ${f32(a.sy)}, sz: ${f32(a.sz)} },`).join('\n')}
];
pub struct Zone { pub name: &'static str, pub x0: f32, pub x1: f32, pub z0: f32, pub z1: f32 }
pub const ZONES: &[Zone] = &[
${ZONES.map((z) => `  Zone { name: "${z.name}", x0: ${f32(z.rect.x0)}, x1: ${f32(z.rect.x1)}, z0: ${f32(z.rect.z0)}, z1: ${f32(z.rect.z1)} },`).join('\n')}
];
pub struct Spawn { pub name: &'static str, pub x: f32, pub z: f32, pub yaw_deg: u32 }
pub const SPAWNS: &[Spawn] = &[
${SPAWNS.map((s) => `  Spawn { name: "${s.name}", x: ${f32(s.x)}, z: ${f32(s.z)}, yaw_deg: ${s.yaw_deg} },`).join('\n')}
];
pub struct Route { pub from: &'static str, pub to: &'static str, pub to_x: f32, pub to_z: f32 }
pub const ROUTES: &[Route] = &[
${NAV_ROUTES.map((r) => `  Route { from: "${r.from}", to: "${r.to}", to_x: ${f32(r.to_x)}, to_z: ${f32(r.to_z)} },`).join('\n')}
];
pub const NAV_RECT: [f32; 4] = [${f32(NAV.x0)}, ${f32(NAV.x1)}, ${f32(NAV.z0)}, ${f32(NAV.z1)}];
pub const NAV_CELL_M: f32 = ${f32(NAV.cell_m)};
pub struct StreamCell { pub x: i32, pub z: i32, pub priority: u8 }
pub const STREAM: &[StreamCell] = &[
${STREAM.cells.map((c) => `  StreamCell { x: ${c.x}, z: ${c.z}, priority: ${priority(c)} },`).join('\n')}
];
pub const LUDUS_ASSETS: &[&str] = &[${LUDUS_ASSETS.map((a) => `"${a}"`).join(', ')}];
`;
const rsBuf = Buffer.from(rs);
emit('tools/map_data.rs', rsBuf);

const manifest = { files: {} };
for (const [rel, buf] of [['map.json', mapBuf], ['cells.json', cellsBuf], ['tools/map_data.rs', rsBuf]]) {
  manifest.files[rel] = { sha256: sha(buf), bytes: buf.length };
}
const manBuf = Buffer.from(JSON.stringify(manifest, null, 2) + '\n');
emit('manifest.json', manBuf);

console.log(`LUDUS-ASSEMBLE GREEN actors=${actors.length} zones=${ZONES.length} spawns=${SPAWNS.length} routes=${NAV_ROUTES.length} cells=${STREAM.cells.length}`);

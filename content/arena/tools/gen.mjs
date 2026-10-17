// CT8 arena kit generator — deterministic, dependency-free (node stdlib only).
// Original authored geometry/textures/HDRI, CC0-1.0, engine2040 CT8 lane.
// Units: meters, +Y up. UVs kept in 0..1 (the A1 import lane clamps to the
// cooked contract, so tiling >1 would smear: per-face 0..1 mapping instead).
// No randomness: integer-lattice value noise with fixed constants only.
// CT53 loader wiring: after writing `placement.json` this generator runs the
// one real loader (`merge_dressing.mjs:loadMergedArena`, placement 1..11 +
// dressing 101+ concatenated in id order) as a post-write check when
// `dressing.json` exists. NO-CLOBBER GUARD: this script never writes,
// deletes, or reformats `dressing.json` — CC0 drops only ever rewrite the
// generated parade + manifest, and a dressing overlap/id collision fails
// the regen loud instead of overwriting the staging file.
import { writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadMergedArena } from './merge_dressing.mjs';
// Shared deterministic kit helpers (CT8+CT9): noise, mesh accumulator,
// subdivided box, heightfield, PNG/RGBE writers, glTF push, manifest track.
import { emitGltf, hash2, fbm, newAcc, box, heightPlane, rgbe, pushInto, trackInto, makeMk } from '../../kit-common.mjs';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SRC = join(ROOT, 'src');
const TEX = join(ROOT, 'textures');
const HDRI = join(ROOT, 'hdri');
const TOOLS = join(ROOT, 'tools');
for (const d of [SRC, TEX, HDRI, TOOLS]) mkdirSync(d, { recursive: true });

// Deterministic value noise + mesh accumulator live in ../../kit-common.mjs
// (shared with the CT9 map kit); kit-specific builders stay below.
// Quad corners a,b,c,d (CCW from outside) with face normal n and uv rect.
function quad(acc, a, b, c, d, n, uvScale = 1) {
  const base = acc.pos.length;
  for (const p of [a, b, c, d]) acc.pos.push(p);
  for (let i = 0; i < 4; i++) acc.nrm.push(n);
  const uvs = [[0, 0], [uvScale, 0], [uvScale, uvScale], [0, uvScale]];
  for (const t of uvs) acc.uv.push([Math.min(1, t[0]), Math.min(1, t[1])]);
  // Tangent: +X unless the face looks along X, then +Z (never parallel).
  const t = Math.abs(n[0]) > 0.5 ? [0, 0, 1, 1] : [1, 0, 0, 1];
  for (let i = 0; i < 4; i++) acc.tan.push(t);
  acc.idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
}
// Subdivided box + heightfield plane live in ../../kit-common.mjs (shared).
// Wavy cloth plane in the XY plane at fixed z (banner): grid, z displacement.
function clothPlane(acc, cx, cy, cz, w, h, nx, ny, waveFn) {
  const base = acc.pos.length;
  for (let iy = 0; iy <= ny; iy++) {
    for (let ix = 0; ix <= nx; ix++) {
      const x = cx - w / 2 + (w * ix) / nx;
      const y = cy + h / 2 - (h * iy) / ny;
      acc.pos.push([x, y, cz + waveFn(ix / nx, iy / ny)]);
      acc.nrm.push([0, 0, 1]);
      acc.uv.push([ix / nx, iy / ny]);
      acc.tan.push([1, 0, 0, 1]);
    }
  }
  for (let iy = 0; iy < ny; iy++) {
    for (let ix = 0; ix < nx; ix++) {
      const a = base + iy * (nx + 1) + ix;
      const b = a + 1, c = a + nx + 1, e = c + 1;
      acc.idx.push(a, b, e, a, e, c);
    }
  }
}

// ---------------------------------------------------------------------------
// Kit builders: each returns { meshes: [{name, acc}], material, upright }.
// All geometry pre-transformed so the piece rests on y=0 (min_y = 0).
// ---------------------------------------------------------------------------
const sandH = (x, z) =>
  0.12 * Math.sin(x * 0.9) * Math.cos(z * 0.7) + 0.05 * Math.sin(x * 2.3 + z * 1.7);
// One mesh per solid part (never merge disjoint boxes): the CT2 quadric
// simplifier collapses each mesh toward its LOD targets, and merged soup
// degenerates to empty LODs while separate boxes hold their shape.
function part(name, fn) {
  const acc = newAcc();
  fn(acc);
  return { name, acc };
}
const KIT = {
  sand_floor() {
    const acc = newAcc();
    heightPlane(acc, 0, 0, 0, 30, 30, 16, 16, sandH);
    return {
      meshes: [{ name: 'Body', acc }],
      material: { base: [0.76, 0.62, 0.42, 1], metallic: 0.0, roughness: 0.95 },
      tex: 'sand_albedo',
    };
  },
  stands() {
    return {
      meshes: [
        part('stands_base', (a) => box(a, 0, 0.5, 0, 12, 1, 6)),
        ...[0, 1, 2].map((s) =>
          part(`stands_step_${s}`, (a) => box(a, 0, 1.25 + s * 0.5, -2.2 + s * 1.1, 12, 0.5, 1.2))),
        part('stands_back', (a) => box(a, 0, 2.0, -3.0, 12, 2.0, 0.4)),
        part('stands_cheek_w', (a) => box(a, -6.2, 1.0, 0, 0.4, 2.0, 6)),
        part('stands_cheek_e', (a) => box(a, 6.2, 1.0, 0, 0.4, 2.0, 6)),
      ],
      material: { base: [0.62, 0.6, 0.57, 1], metallic: 0.0, roughness: 0.9 },
      tex: 'stone_albedo',
    };
  },
  walls() {
    return {
      meshes: [
        part('walls_wall', (a) => box(a, 0, 1.5, 0, 8, 3, 0.6)),
        ...[0, 1, 2, 3, 4].map((i) =>
          part(`walls_merlon_${i}`, (a) => box(a, -3.2 + i * 1.6, 3.35, 0, 0.8, 0.7, 0.6))),
        part('COL_walls', (a) => box(a, 0, 1.5, 0, 8, 3, 0.6)),
      ],
      material: { base: [0.58, 0.55, 0.5, 1], metallic: 0.0, roughness: 0.9 },
      tex: 'stone_albedo',
    };
  },
  gate_tunnel() {
    return {
      meshes: [
        part('gate_pylon_w', (a) => box(a, -2.1, 2.0, 0, 1.2, 4, 1.4)),
        part('gate_pylon_e', (a) => box(a, 2.1, 2.0, 0, 1.2, 4, 1.4)),
        part('gate_lintel', (a) => box(a, 0, 4.5, 0, 5.4, 1.0, 1.6)),
        part('gate_cap', (a) => box(a, 0, 5.15, 0, 5.8, 0.3, 1.8)),
        part('COL_gate', (a) => box(a, 0, 2.5, 0, 5.4, 5.0, 1.6)),
      ],
      material: { base: [0.55, 0.52, 0.47, 1], metallic: 0.0, roughness: 0.9 },
      tex: 'stone_albedo',
    };
  },
  torch() {
    return {
      meshes: [
        part('torch_pole', (a) => box(a, 0, 0.8, 0, 0.12, 1.6, 0.12)),
        part('torch_cup', (a) => box(a, 0, 1.7, 0, 0.3, 0.25, 0.3)),
      ],
      sockets: [{ name: 'SOCKET_flame', at: [0, 1.95, 0] }],
      material: { base: [0.3, 0.2, 0.12, 1], metallic: 0.6, roughness: 0.5 },
      tex: 'wood_albedo',
    };
  },
  banner() {
    const cloth = newAcc();
    clothPlane(cloth, 0, 1.95, 0.03, 1.2, 1.8, 4, 6,
      (u, v) => 0.05 * Math.sin(u * 6.28) * (0.3 + v)); // hang y 1.05..2.85
    return {
      meshes: [
        part('banner_pole', (a) => box(a, 0, 1.5, 0, 0.1, 3.0, 0.1)),
        part('banner_bar', (a) => box(a, 0, 2.9, 0, 1.4, 0.08, 0.08)),
        { name: 'banner_cloth', acc: cloth },
      ],
      material: { base: [0.62, 0.1, 0.12, 1], metallic: 0.0, roughness: 0.85 },
      tex: 'cloth_albedo',
    };
  },
  crate() {
    const defs = [
      ['FRAC_panel_top', 0, 0.97, 0, 1.0, 0.06, 1.0],
      ['FRAC_panel_bottom', 0, 0.03, 0, 1.0, 0.06, 1.0],
      ['FRAC_panel_north', 0, 0.5, -0.47, 1.0, 1.0, 0.06],
      ['FRAC_panel_south', 0, 0.5, 0.47, 1.0, 1.0, 0.06],
      ['FRAC_panel_east', 0.47, 0.5, 0, 0.06, 1.0, 1.0],
      ['FRAC_panel_west', -0.47, 0.5, 0, 0.06, 1.0, 1.0],
    ];
    return {
      meshes: [
        part('Body', (a) => box(a, 0, 0.5, 0, 0.9, 0.9, 0.9)),
        ...defs.map(([name, cx, cy, cz, sx, sy, sz]) => part(name, (a) => box(a, cx, cy, cz, sx, sy, sz))),
        part('COL_crate', (a) => box(a, 0, 0.5, 0, 1.0, 1.0, 1.0)),
      ],
      material: { base: [0.5, 0.35, 0.2, 1], metallic: 0.0, roughness: 0.8 },
      tex: 'wood_albedo',
    };
  },
  debris() {
    // Five jittered rubble chunks in a 2.5 m patch. Jitter is a function of
    // the quantized corner position, so coincident face verts move together.
    const seeds = [[-0.7, -0.5, 0.22, 'debris_a'], [0.4, -0.6, 0.16, 'debris_b'], [0.8, 0.5, 0.26, 'debris_c'], [-0.3, 0.7, 0.14, 'debris_d'], [0.0, 0.0, 0.3, 'debris_e']];
    return {
      meshes: seeds.map(([ox, oz, s, name]) => part(name, (acc) => {
        box(acc, ox, s * 0.32, oz, s, s * 0.64, s);
        for (const p of acc.pos) {
          const q = p.map((v) => Math.round(v * 97));
          const jx = (hash2(q[0] + 11, q[1] * 3 + q[2]) - 0.5) * 0.5 * s;
          const jy = (hash2(q[1] + 37, q[2] * 5 + q[0]) - 0.5) * 0.3 * s;
          const jz = (hash2(q[2] + 53, q[0] * 7 + q[1]) - 0.5) * 0.5 * s;
          p[0] += jx; p[1] = Math.max(0, p[1] + jy); p[2] += jz;
        }
        // Recompute flat face normals after the jitter (6 faces, n=2 grid).
        for (let f = 0; f < 6; f++) {
          const v0 = acc.pos[f * 9], v1 = acc.pos[f * 9 + 2], v2 = acc.pos[f * 9 + 6];
          const e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
          const e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
          let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
          const l = Math.hypot(...n) || 1;
          n = [n[0] / l, n[1] / l, n[2] / l];
          for (let k = 0; k < 9; k++) acc.nrm[f * 9 + k] = n;
        }
      })),
      material: { base: [0.5, 0.47, 0.42, 1], metallic: 0.0, roughness: 0.95 },
      tex: 'stone_albedo',
    };
  },
};

// glTF 2.0 writer: shared emitGltf in kit-common (generator tag passed per kit).
// Emitted bytes are unchanged from the removed local copy.

// PNG writer + texture-slot painter live in ../../kit-common.mjs (shared).

// PBR texture set (all power-of-two, <= 2k budget):
// albedo slots are sRGB bytes (linearized by the A2 cook), data slots linear.
function paintTextures() {
  const out = {};
  const mk = makeMk(TEX, out);
  mk('sand_albedo', 1024, (u, v) => {
    const n = fbm(u * 24, v * 24, 4), g = fbm(u * 90 + 7, v * 90 + 3, 2);
    const m = 0.82 + 0.18 * n;
    return [0.78 * m + 0.06 * g, 0.63 * m + 0.04 * g, 0.43 * m, 1];
  });
  mk('stone_albedo', 1024, (u, v) => {
    const row = Math.floor(v * 8), off = (row % 2) * 0.0625;
    const bx = (u + off) % 0.25, mortar = (bx < 0.008 || (v * 8) % 1 < 0.03) ? 0.55 : 1;
    const n = fbm(u * 40, v * 40, 4);
    const m = (0.85 + 0.15 * n) * mortar;
    return [0.66 * m, 0.63 * m, 0.58 * m, 1];
  });
  mk('wood_albedo', 1024, (u, v) => {
    const plank = Math.floor(u * 6);
    const gap = (u * 6) % 1 < 0.02 ? 0.5 : 1;
    const ring = 0.85 + 0.15 * Math.sin(v * 160 + 6 * fbm(u * 8, v * 8, 3) + plank * 2.1);
    const m = ring * gap;
    return [0.55 * m, 0.38 * m, 0.22 * m, 1];
  });
  mk('cloth_albedo', 1024, (u, v) => {
    const border = (u < 0.06 || u > 0.94 || v < 0.06 || v > 0.94) ? [0.85, 0.65, 0.2] : [0.62, 0.1, 0.12];
    const weave = 0.94 + 0.06 * Math.sin(u * 500) * Math.sin(v * 500);
    return [border[0] * weave, border[1] * weave, border[2] * weave, 1];
  });
  mk('metal_roughness', 256, (u, v) => {
    const n = fbm(u * 12, v * 12, 3);
    return [0.0, 0.35 + 0.1 * n, 0.9, 1]; // G roughness ~0.37, B metallic 0.9
  });
  mk('flat_normal', 256, () => [0.50196, 0.50196, 1, 1]); // (128,128,255)
  return out;
}

// ---------------------------------------------------------------------------
// Radiance RGBE HDRI writer (flat scanlines), one time of day each.
// ---------------------------------------------------------------------------
// RGBE helper lives in ../../kit-common.mjs (shared); sky headers stay local.
function writeHdr(path, w, h, skyFn) {
  const head = Buffer.from(`#?RADIANCE\n# engine2040 CT8 original CC0 sky\nFORMAT=32-bit_rle_rgbe\n\n-Y ${h} +X ${w}\n`, 'ascii');
  const px = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const u = (x + 0.5) / w * 2 - 1; // azimuth -1..1
      const v = 1 - (y + 0.5) / h * 2; // -1 ground .. +1 zenith
      px.set(skyFn(u, v), (y * w + x) * 4);
    }
  writeFileSync(path, Buffer.concat([head, px]));
}
function sun(u, v, au, av, size) {
  const d = Math.hypot(u - au, v - av);
  return Math.max(0, 1 - d / size);
}
function paintHdri() {
  const W = 128, H = 64;
  const days = {
    arena_day: (u, v) => {
      if (v < 0) return rgbe(0.32, 0.26, 0.18);
      const t = Math.min(1, v * 1.6);
      const sky = [0.95 - 0.5 * t, 0.75 - 0.15 * t, 0.6 + 0.35 * t].map((c) => c * (1.1 + 0.9 * (1 - t)));
      const s = sun(u, v, 0.3, 0.62, 0.09);
      return rgbe(sky[0] + s * 20, sky[1] + s * 18, sky[2] + s * 15);
    },
    arena_sunset: (u, v) => {
      if (v < 0) return rgbe(0.22, 0.13, 0.1);
      const t = Math.min(1, Math.max(0, v));
      const sky = [1.3 * (1 - t) + 0.25 * t, 0.55 * (1 - t) + 0.2 * t, 0.25 * (1 - t) + 0.35 * t];
      const s = sun(u, v, -0.45, 0.1, 0.12);
      return rgbe(sky[0] + s * 14, sky[1] + s * 6, sky[2] + s * 2.5);
    },
    arena_night: (u, v) => {
      if (v < 0) return rgbe(0.012, 0.014, 0.02);
      const t = Math.min(1, Math.max(0, v));
      const sky = [0.03 + 0.05 * t, 0.04 + 0.06 * t, 0.07 + 0.1 * t];
      const m = sun(u, v, 0.5, 0.55, 0.05);
      return rgbe(sky[0] + m * 3, sky[1] + m * 3.2, sky[2] + m * 3.6);
    },
  };
  const out = {};
  for (const [n, fn] of Object.entries(days)) {
    const p = join(HDRI, n + '.hdr');
    writeHdr(p, W, H, fn);
    out[n] = p;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Parade placement: every footprint disjoint in XZ, all resting (min_y = 0),
// inside +-99 m, grammar-clean unique names. The engine spatial run is judge.
// `physical` is the collision flag the match reads (the one data owner is
// placement.json); walls, the stands, the gate and the two cover boxes bump,
// the sand floor and the torches/banners are dressing the view still draws.
// Wall z tracks rome-spec half_extent_m (10 m) and the zone rows, so a regen
// never moves a wall inside the duel floor.
// ---------------------------------------------------------------------------
const PLACEMENT = [
  { id: 1, name: 'sand_floor', asset: 'sand_floor', x: -60, y: 0.1, z: 0, sx: 30, sy: 0.2, sz: 30, physical: false },
  { id: 2, name: 'stands', asset: 'stands', x: -20, y: 1.5, z: 0, sx: 12, sy: 3, sz: 6, physical: true },
  { id: 3, name: 'wall_north', asset: 'walls', x: -2, y: 1.5, z: -10, sx: 8, sy: 3, sz: 1, physical: true },
  { id: 4, name: 'wall_south', asset: 'walls', x: -2, y: 1.5, z: 10, sx: 8, sy: 3, sz: 1, physical: true },
  { id: 5, name: 'gate_tunnel', asset: 'gate_tunnel', x: 10, y: 2.5, z: 0, sx: 6, sy: 5, sz: 3, physical: true },
  { id: 6, name: 'torch_west', asset: 'torch', x: 20, y: 1.0, z: -3, sx: 0.5, sy: 2, sz: 0.5, physical: false },
  { id: 7, name: 'torch_east', asset: 'torch', x: 20, y: 1.0, z: 3, sx: 0.5, sy: 2, sz: 0.5, physical: false },
  { id: 8, name: 'banner_west', asset: 'banner', x: 24, y: 1.5, z: -3, sx: 1.6, sy: 3, sz: 0.6, physical: false },
  { id: 9, name: 'banner_east', asset: 'banner', x: 24, y: 1.5, z: 3, sx: 1.6, sy: 3, sz: 0.6, physical: false },
  { id: 10, name: 'crate_a', asset: 'crate', x: -5, y: 0.5, z: -5, sx: 1, sy: 1, sz: 1, physical: true },
  { id: 11, name: 'debris_field', asset: 'debris', x: 5, y: 0.5, z: 5, sx: 2.5, sy: 1, sz: 2.5, physical: true },
];

// ---------------------------------------------------------------------------
// Main: emit glTF + textures + HDRI + placement + manifest + proof bindings.
// ---------------------------------------------------------------------------
const manifest = { files: {} };
const track = (p, buf) => trackInto(manifest, p, buf);
const factors = {};
const colMesh = {};
// CT26 CLOBBER GATE: CC0 PBR drops (2k maps, authored ORMs, rewired embeds)
// live alongside the generated files. Re-running the generator would wipe
// them back to flat fallbacks, so refuse loud unless --allow-clobber.
if (!new Set(process.argv.slice(2)).has('--allow-clobber')) {
  const drops = readdirSync(TEX).filter((f) => f.endsWith('_2k.jpg') || f.endsWith('_orm.png'));
  if (drops.length > 0) {
    console.error(`REFUSE: CC0 drops present (${drops.length}: ${drops.slice(0, 4).join(', ')}...). Re-running gen.mjs would wipe them. Re-run with --allow-clobber to overwrite.`);
    process.exit(1);
  }
}
for (const [name, build] of Object.entries(KIT)) {
  const built = build();
  const buf = emitGltf(name, built, "engine2040-ct8-gen (CC0-1.0 original)");
  const p = join(SRC, name + '.gltf');
  writeFileSync(p, buf);
  track(`src/${name}.gltf`, buf);
  factors[name] = built.material;
  colMesh[name] = built.meshes.map((m, i) => (m.name.startsWith('COL_') ? i : -1)).filter((i) => i >= 0);
  const tris = built.meshes.reduce((n, m) => n + m.acc.idx.length / 3, 0);
  const verts = built.meshes.reduce((n, m) => n + m.acc.pos.length, 0);
  console.log(`mesh ${name}: ${built.meshes.length} mesh(es), ${verts} verts, ${tris} tris LOD0`);
}
for (const [name, buf] of Object.entries(paintTextures())) track(`textures/${name}.png`, buf);
paintHdri();
import { readFileSync } from 'node:fs';
for (const n of ['arena_day', 'arena_sunset', 'arena_night'])
  track(`hdri/${n}.hdr`, readFileSync(join(HDRI, n + '.hdr')));
// Field order matches the shipped placement.json (the parse reads by key, but
// a byte-stable regen keeps `git diff` honest): `physical` rides each row.
const placementJson = Buffer.from(
  JSON.stringify(
    {
      actors: PLACEMENT.map(({ physical, ...row }) => ({
        ...row,
        parent: null,
        physical,
        vx: 0,
        vz: 0,
        yaw: 0,
      })),
    },
    null,
    2,
  ),
);
writeFileSync(join(ROOT, 'placement.json'), placementJson);
track('placement.json', placementJson);
// CT53: the one real loader rides the regen. dressing.json is staged, never
// generated — this guard only reads it, and a breach fails loud.
if (existsSync(join(ROOT, 'dressing.json'))) {
  const { merged, sha } = loadMergedArena(ROOT);
  console.log(`MERGED actors=${merged.length} fails=NONE sha=${sha}`);
} else {
  console.log('MERGED skipped: no dressing.json (parade-only fallback)');
}
// Rust bindings for the proof crate (no JSON parsing in std-only proof).
const f32 = (v) => Number.isInteger(v) ? `${v}.0` : `${v}`;
const rs = `// Generated by tools/gen.mjs — do not hand-edit.
pub struct Placed { pub id: u32, pub name: &'static str, pub x: f32, pub y: f32, pub z: f32, pub sx: f32, pub sy: f32, pub sz: f32 }
pub const PLACEMENT: &[Placed] = &[
${PLACEMENT.map((a) => `  Placed { id: ${a.id}, name: "${a.name}", x: ${a.x}_f32, y: ${a.y}_f32, z: ${a.z}_f32, sx: ${a.sx}_f32, sy: ${a.sy}_f32, sz: ${a.sz}_f32 },`).join('\n')}
];
pub struct Factors { pub base: [f32; 4], pub metallic: f32, pub roughness: f32 }
pub fn factors(asset: &str) -> Factors {
  match asset {
${Object.entries(factors).map(([n, f]) => `    "${n}" => Factors { base: [${f.base.map(f32).join(', ')}], metallic: ${f32(f.metallic)}, roughness: ${f32(f.roughness)} },`).join('\n')}
    _ => Factors { base: [1.0, 1.0, 1.0, 1.0], metallic: 0.0, roughness: 1.0 },
  }
}
pub const ASSETS: &[&str] = &[${Object.keys(KIT).map((n) => `"${n}"`).join(', ')}];
pub fn col_meshes(asset: &str) -> &'static [usize] {
  match asset {
${Object.entries(colMesh).map(([n, idx]) => `    "${n}" => &[${idx.join(', ')}],`).join('\n')}
    _ => &[],
  }
}
pub const TEXTURES: &[(&str, &str)] = &[("sand_albedo", "BaseColor"), ("stone_albedo", "BaseColor"), ("wood_albedo", "BaseColor"), ("cloth_albedo", "BaseColor"), ("metal_roughness", "MetallicRoughness"), ("flat_normal", "Normal")];
pub const ASSET_TEX: &[(&str, &str)] = &[${Object.keys(KIT).map((n) => `("${n}", "${KIT[n]().tex}")`).join(', ')}];
`;
writeFileSync(join(TOOLS, 'placement_data.rs'), rs);
const manBuf = Buffer.from(JSON.stringify(manifest, null, 2));
writeFileSync(join(ROOT, 'manifest.json'), manBuf);
console.log('wrote manifest.json with', Object.keys(manifest.files).length, 'files');

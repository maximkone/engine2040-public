// CT9 map-kit generator — deterministic, dependency-free (node stdlib only).
// Original authored geometry/textures/HDRI, CC0-1.0, engine2040 CT9 lane.
// Modular buildings, walls, gates, market + camp props for the ludus (WD2),
// the Rome district (WD3) and the siege field (WD4). Same conventions as the
// CT8 arena kit (tools/gen.mjs there): meters, +Y up, UVs in 0..1 (the A1
// import lane clamps to the cooked contract, so no tiling), one mesh per
// solid part (merged disjoint boxes degenerate to empty LODs in the greedy
// CT2 pass — CT8 repair lesson applied proactively), COL_ collision proxies,
// FRAC_ chunks for DX1, SOCKET_ anchors. Albedos capped at 512 (CT8's 1024
// set cooked to a 93 MB float32 bundle; this kit notes the AS3 need instead
// of repeating it). No randomness: integer-lattice value noise only.
import { writeFileSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
// Shared deterministic kit helpers (CT8+CT9): noise, mesh accumulator,
// subdivided box, heightfield, PNG/RGBE writers, glTF push, manifest track.
import { emitGltf, fbm, newAcc, box, heightPlane, rgbe, pushInto, trackInto, makeMk } from '../../kit-common.mjs';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const SRC = join(ROOT, 'src');
const TEX = join(ROOT, 'textures');
const HDRI = join(ROOT, 'hdri');
const TOOLS = join(ROOT, 'tools');
for (const d of [SRC, TEX, HDRI, TOOLS]) mkdirSync(d, { recursive: true });

// Deterministic value noise + mesh accumulator live in ../../kit-common.mjs
// (shared with the CT8 arena kit); kit-specific builders stay below.
// Subdivided box + heightfield plane: shared (see kit-common.mjs).
// Sloped awning cloth in XZ with a fall along +z (high edge at the back
// posts, low edge at the front posts) plus a ripple.
function awning(acc, cx, cy, cz, w, d, nx, nz, fall, waveFn) {
  const base = acc.pos.length;
  const inv = 1 / Math.hypot(d, fall);
  for (let iz = 0; iz <= nz; iz++) {
    for (let ix = 0; ix <= nx; ix++) {
      const u = ix / nx, v = iz / nz;
      acc.pos.push([cx - w / 2 + w * u, cy - fall * v + waveFn(u, v), cz - d / 2 + d * v]);
      acc.nrm.push([0, d * inv, fall * inv]);
      acc.uv.push([u, v]);
      acc.tan.push([1, 0, 0, 1]);
    }
  }
  for (let iz = 0; iz < nz; iz++) {
    for (let ix = 0; ix < nx; ix++) {
      const a = base + iz * (nx + 1) + ix;
      const b = a + 1, c = a + nx + 1, e = c + 1;
      acc.idx.push(a, b, e, a, e, c);
    }
  }
}

// ---------------------------------------------------------------------------
// Kit builders: each returns { meshes: [{name, acc}], material, tex }.
// All geometry pre-transformed so the piece rests on y=0 (min_y = 0).
// ---------------------------------------------------------------------------
function part(name, fn) {
  const acc = newAcc();
  fn(acc, 4); // always n=4: n=2 slivers collapse to empty LODs in the greedy
  return { name, acc }; // CT2 pass (CT9 proof run 1: yard curb LOD2 empty).
}
const B = (acc, n) => (cx, cy, cz, sx, sy, sz) => box(acc, cx, cy, cz, sx, sy, sz, n);
const yardH = (x, z) =>
  0.06 * Math.sin(x * 0.8) * Math.cos(z * 0.6) + 0.02 * Math.sin(x * 2.1 + z * 1.9);

const KIT = {
  // -- Ludus (WD2: practice yard, barracks, blacksmith, party board) --------
  yard_floor() {
    const acc = newAcc();
    heightPlane(acc, 0, 0, 0, 20, 14, 12, 10, yardH);
    return {
      meshes: [
        { name: 'Body', acc },
        part('yard_curb_n', (a, n) => B(a, n)(0, 0.15, -7.1, 20.4, 0.3, 0.4), 2),
        part('yard_curb_s', (a, n) => B(a, n)(0, 0.15, 7.1, 20.4, 0.3, 0.4), 2),
        part('yard_curb_w', (a, n) => B(a, n)(-10.1, 0.15, 0, 0.4, 0.3, 14.6), 2),
        part('yard_curb_e', (a, n) => B(a, n)(10.1, 0.15, 0, 0.4, 0.3, 14.6), 2),
      ],
      material: { base: [0.72, 0.6, 0.44, 1], metallic: 0.0, roughness: 0.96 },
      tex: 'ground_albedo',
    };
  },
  barracks() {
    return {
      meshes: [
        part('barracks_hall', (a, n) => B(a, n)(0, 1.7, 0, 10, 3.4, 4.5)),
        part('barracks_roof', (a, n) => B(a, n)(0, 3.55, 0, 10.6, 0.3, 5.1)),
        part('barracks_parapet_n', (a, n) => B(a, n)(0, 4.05, -2.4, 10.6, 0.7, 0.3), 2),
        part('barracks_parapet_s', (a, n) => B(a, n)(0, 4.05, 2.4, 10.6, 0.7, 0.3), 2),
        part('barracks_door', (a, n) => B(a, n)(-2.5, 1.1, 2.28, 1.4, 2.2, 0.1), 2),
        part('barracks_win_a', (a, n) => B(a, n)(0.5, 2.2, 2.28, 0.9, 0.9, 0.1), 2),
        part('barracks_win_b', (a, n) => B(a, n)(3.0, 2.2, 2.28, 0.9, 0.9, 0.1), 2),
        part('barracks_beam_0', (a, n) => B(a, n)(-3.75, 3.85, 0, 0.25, 0.25, 5.1), 2),
        part('barracks_beam_1', (a, n) => B(a, n)(0, 3.85, 0, 0.25, 0.25, 5.1), 2),
        part('barracks_beam_2', (a, n) => B(a, n)(3.75, 3.85, 0, 0.25, 0.25, 5.1), 2),
        part('COL_barracks', (a, n) => B(a, n)(0, 1.7, 0, 10, 3.4, 4.5), 2),
      ],
      material: { base: [0.8, 0.74, 0.64, 1], metallic: 0.0, roughness: 0.92 },
      tex: 'plaster_albedo',
    };
  },
  blacksmith() {
    return {
      meshes: [
        part('smith_hut', (a, n) => B(a, n)(0, 1.4, 0, 4, 2.8, 3.5)),
        part('smith_roof', (a, n) => B(a, n)(0, 2.95, 0, 4.5, 0.3, 4.0)),
        part('smith_chimney', (a, n) => B(a, n)(1.2, 3.9, -0.8, 0.7, 2.2, 0.7), 2),
        part('smith_forge', (a, n) => B(a, n)(-1.1, 0.5, 0.9, 1.4, 1.0, 1.0), 2),
        part('smith_stump', (a, n) => B(a, n)(0.8, 0.3, 1.0, 0.5, 0.6, 0.5), 2),
        part('smith_anvil', (a, n) => B(a, n)(0.8, 0.75, 1.0, 0.7, 0.3, 0.35), 2),
        part('COL_smith', (a, n) => B(a, n)(0, 1.4, 0, 4, 2.8, 3.5), 2),
      ],
      material: { base: [0.6, 0.57, 0.52, 1], metallic: 0.0, roughness: 0.9 },
      tex: 'stone_albedo',
    };
  },
  notice_board() {
    return {
      meshes: [
        part('board_post_w', (a, n) => B(a, n)(-0.8, 1.3, 0, 0.16, 2.6, 0.16), 2),
        part('board_post_e', (a, n) => B(a, n)(0.8, 1.3, 0, 0.16, 2.6, 0.16), 2),
        part('board_panel', (a, n) => B(a, n)(0, 1.6, 0, 1.9, 1.1, 0.1), 2),
        part('board_cap', (a, n) => B(a, n)(0, 2.7, 0, 2.1, 0.12, 0.4), 2),
        part('board_slip_a', (a, n) => B(a, n)(-0.5, 1.6, 0.07, 0.4, 0.6, 0.03), 2),
        part('board_slip_b', (a, n) => B(a, n)(0.1, 1.65, 0.07, 0.4, 0.5, 0.03), 2),
        part('board_slip_c', (a, n) => B(a, n)(0.6, 1.55, 0.07, 0.35, 0.65, 0.03), 2),
      ],
      material: { base: [0.52, 0.37, 0.22, 1], metallic: 0.0, roughness: 0.82 },
      tex: 'timber_albedo',
    };
  },
  training_post() {
    return {
      meshes: [
        part('pell_post', (a, n) => B(a, n)(0, 1.1, 0, 0.26, 2.2, 0.26), 2),
        part('pell_wrap', (a, n) => B(a, n)(0, 1.3, 0, 0.4, 0.9, 0.4), 2),
        part('pell_arm', (a, n) => B(a, n)(0, 1.7, 0, 1.2, 0.14, 0.14), 2),
      ],
      material: { base: [0.55, 0.4, 0.24, 1], metallic: 0.0, roughness: 0.85 },
      tex: 'timber_albedo',
    };
  },
  weapon_rack() {
    return {
      meshes: [
        part('rack_leg_w', (a, n) => B(a, n)(-1.1, 0.8, 0, 0.14, 1.6, 0.5), 2),
        part('rack_leg_e', (a, n) => B(a, n)(1.1, 0.8, 0, 0.14, 1.6, 0.5), 2),
        part('rack_bar', (a, n) => B(a, n)(0, 1.55, 0, 2.4, 0.12, 0.12), 2),
        ...[0, 1, 2, 3, 4].map((i) =>
          part(`rack_spear_${i}`, (a, n) => B(a, n)(-0.9 + i * 0.45, 0.9, 0.1, 0.07, 1.8, 0.07), 2)),
        part('rack_shield_a', (a, n) => B(a, n)(-0.45, 0.7, -0.15, 0.5, 0.7, 0.06), 2),
        part('rack_shield_b', (a, n) => B(a, n)(0.45, 0.7, -0.15, 0.5, 0.7, 0.06), 2),
      ],
      material: { base: [0.5, 0.36, 0.22, 1], metallic: 0.1, roughness: 0.8 },
      tex: 'timber_albedo',
    };
  },
  // -- District (WD3: streets, rooftops, market, traversal edges) ------------
  insula() {
    return {
      meshes: [
        part('insula_ground', (a, n) => B(a, n)(0, 1.6, 0, 6, 3.2, 5)),
        part('insula_upper', (a, n) => B(a, n)(0, 4.6, 0, 6, 2.8, 5)),
        part('insula_roof', (a, n) => B(a, n)(0, 6.15, 0, 6.4, 0.3, 5.4)),
        part('insula_parapet_n', (a, n) => B(a, n)(0, 6.8, -2.55, 6.4, 1.0, 0.3), 2),
        part('insula_parapet_s', (a, n) => B(a, n)(0, 6.8, 2.55, 6.4, 1.0, 0.3), 2),
        part('insula_balcony', (a, n) => B(a, n)(0, 3.25, 3.0, 6, 0.25, 1.2), 2),
        part('insula_rail', (a, n) => B(a, n)(0, 3.85, 3.55, 6, 0.5, 0.1), 2),
        part('insula_door', (a, n) => B(a, n)(-1.5, 1.2, 2.53, 1.3, 2.4, 0.1), 2),
        part('insula_win_a', (a, n) => B(a, n)(1.2, 4.8, 2.53, 0.9, 0.9, 0.1), 2),
        part('insula_win_b', (a, n) => B(a, n)(-1.2, 4.8, 2.53, 0.9, 0.9, 0.1), 2),
        part('COL_insula', (a, n) => B(a, n)(0, 3.0, 0, 6, 6.0, 5), 2),
      ],
      material: { base: [0.82, 0.72, 0.58, 1], metallic: 0.0, roughness: 0.93 },
      tex: 'plaster_albedo',
    };
  },
  market_stall() {
    const cloth = newAcc();
    awning(cloth, 0, 2.5, 0, 3.0, 2.2, 12, 8, 0.5, (u, v) => 0.04 * Math.sin(u * 6.28) * (0.4 + v));
    return {
      meshes: [
        part('stall_post_nw', (a, n) => B(a, n)(-1.3, 1.25, -0.9, 0.14, 2.5, 0.14), 2),
        part('stall_post_ne', (a, n) => B(a, n)(1.3, 1.25, -0.9, 0.14, 2.5, 0.14), 2),
        part('stall_post_sw', (a, n) => B(a, n)(-1.3, 1.0, 0.9, 0.14, 2.0, 0.14), 2),
        part('stall_post_se', (a, n) => B(a, n)(1.3, 1.0, 0.9, 0.14, 2.0, 0.14), 2),
        part('stall_counter', (a, n) => B(a, n)(0, 0.5, 0.3, 2.8, 0.9, 1.2), 2),
        part('stall_crate_a', (a, n) => B(a, n)(-0.7, 1.2, 0.3, 0.6, 0.4, 0.6), 2),
        part('stall_crate_b', (a, n) => B(a, n)(0.6, 1.15, 0.3, 0.5, 0.3, 0.5), 2),
        { name: 'stall_awning', acc: cloth },
        part('COL_stall', (a, n) => B(a, n)(0, 0.5, 0.3, 2.8, 1.0, 1.2), 2),
      ],
      material: { base: [0.78, 0.66, 0.5, 1], metallic: 0.0, roughness: 0.88 },
      tex: 'canvas_albedo',
    };
  },
  arch_gate() {
    return {
      meshes: [
        part('arch_pylon_w', (a, n) => B(a, n)(-2.3, 2.5, 0, 1.6, 5, 2.0)),
        part('arch_pylon_e', (a, n) => B(a, n)(2.3, 2.5, 0, 1.6, 5, 2.0)),
        part('arch_lintel', (a, n) => B(a, n)(0, 5.5, 0, 6.2, 1.0, 2.2)),
        part('arch_cap', (a, n) => B(a, n)(0, 6.15, 0, 6.6, 0.3, 2.4)),
        ...[0, 1, 2, 3].map((i) =>
          part(`arch_merlon_${i}`, (a, n) => B(a, n)(-2.4 + i * 1.6, 6.65, 0, 0.8, 0.7, 0.5), 2)),
        part('COL_arch_w', (a, n) => B(a, n)(-2.3, 2.5, 0, 1.6, 5, 2.0), 2),
        part('COL_arch_e', (a, n) => B(a, n)(2.3, 2.5, 0, 1.6, 5, 2.0), 2),
      ],
      material: { base: [0.62, 0.59, 0.54, 1], metallic: 0.0, roughness: 0.9 },
      tex: 'stone_albedo',
    };
  },
  street_wall() {
    return {
      meshes: [
        part('street_wall', (a, n) => B(a, n)(0, 1.6, 0, 10, 3.2, 0.6)),
        ...[0, 1, 2, 3, 4, 5].map((i) =>
          part(`street_merlon_${i}`, (a, n) => B(a, n)(-4.15 + i * 1.66, 3.55, 0, 0.8, 0.7, 0.6), 2)),
        part('COL_street', (a, n) => B(a, n)(0, 1.6, 0, 10, 3.2, 0.6), 2),
      ],
      material: { base: [0.6, 0.57, 0.52, 1], metallic: 0.0, roughness: 0.9 },
      tex: 'stone_albedo',
    };
  },
  fountain() {
    return {
      meshes: [
        part('fountain_base', (a, n) => B(a, n)(0, 0.15, 0, 2.4, 0.3, 2.4), 2),
        part('fountain_rim_n', (a, n) => B(a, n)(0, 0.6, -1.05, 2.4, 0.7, 0.3), 2),
        part('fountain_rim_s', (a, n) => B(a, n)(0, 0.6, 1.05, 2.4, 0.7, 0.3), 2),
        part('fountain_rim_w', (a, n) => B(a, n)(-1.05, 0.6, 0, 0.3, 0.7, 1.8), 2),
        part('fountain_rim_e', (a, n) => B(a, n)(1.05, 0.6, 0, 0.3, 0.7, 1.8), 2),
        part('fountain_column', (a, n) => B(a, n)(0, 1.2, 0, 0.5, 1.6, 0.5), 2),
        part('fountain_bowl', (a, n) => B(a, n)(0, 2.05, 0, 1.2, 0.18, 1.2), 2),
        part('COL_fountain', (a, n) => B(a, n)(0, 0.5, 0, 2.4, 1.0, 2.4), 2),
      ],
      material: { base: [0.58, 0.6, 0.62, 1], metallic: 0.0, roughness: 0.85 },
      tex: 'stone_albedo',
    };
  },
  // -- Siege field (WD4: walls through DX1-DX3, camp) -------------------------
  siege_wall() {
    // Six FRAC blocks tile the wall face (2 rows x 3 cols); DX1 breaks them.
    const fracs = [];
    for (let r = 0; r < 2; r++)
      for (let c = 0; c < 3; c++)
        fracs.push([`FRAC_block_r${r}c${c}`, -2.67 + c * 2.67, 1.5 + r * 3.0]);
    return {
      meshes: [
        part('siege_wall', (a, n) => B(a, n)(0, 3.0, 0, 8, 6, 1.2)),
        part('siege_walk', (a, n) => B(a, n)(0, 6.15, 0.1, 8, 0.3, 1.6), 2),
        part('siege_parapet', (a, n) => B(a, n)(0, 6.8, -0.5, 8, 1.0, 0.4), 2),
        ...fracs.map(([name, cx, cy]) => part(name, (a, n) => B(a, n)(cx, cy, 0, 2.67, 3.0, 1.24), 2)),
        part('COL_siege', (a, n) => B(a, n)(0, 3.0, 0, 8, 6, 1.2), 2),
      ],
      material: { base: [0.55, 0.52, 0.47, 1], metallic: 0.0, roughness: 0.92 },
      tex: 'stone_albedo',
    };
  },
  palisade() {
    return {
      meshes: [
        ...[0, 1, 2, 3, 4, 5, 6].map((i) =>
          part(`palisade_post_${i}`, (a, n) => B(a, n)(-3.0 + i * 1.0, 1.4, 0, 0.3, 2.8, 0.3), 2)),
        ...[0, 1, 2, 3, 4, 5, 6].map((i) =>
          part(`palisade_tip_${i}`, (a, n) => B(a, n)(-3.0 + i * 1.0, 2.9, 0, 0.18, 0.3, 0.18), 2)),
        part('palisade_binder_a', (a, n) => B(a, n)(0, 1.9, 0.2, 6.6, 0.14, 0.1), 2),
        part('palisade_binder_b', (a, n) => B(a, n)(0, 0.9, 0.2, 6.6, 0.14, 0.1), 2),
        part('COL_palisade', (a, n) => B(a, n)(0, 1.4, 0, 6.6, 2.8, 0.4), 2),
      ],
      material: { base: [0.48, 0.35, 0.2, 1], metallic: 0.0, roughness: 0.85 },
      tex: 'timber_albedo',
    };
  },
  tent() {
    return {
      meshes: [
        part('tent_tier_low', (a, n) => B(a, n)(0, 0.6, 0, 3.2, 1.2, 2.6)),
        part('tent_tier_mid', (a, n) => B(a, n)(0, 1.6, 0, 2.4, 0.9, 1.9)),
        part('tent_tier_top', (a, n) => B(a, n)(0, 2.35, 0, 1.5, 0.7, 1.2)),
        part('tent_pole', (a, n) => B(a, n)(0, 3.2, 0, 0.12, 1.2, 0.12), 2),
        part('tent_peg_a', (a, n) => B(a, n)(-2.0, 0.15, 1.5, 0.2, 0.3, 0.2), 2),
        part('tent_peg_b', (a, n) => B(a, n)(2.0, 0.15, 1.5, 0.2, 0.3, 0.2), 2),
        part('COL_tent', (a, n) => B(a, n)(0, 1.2, 0, 3.2, 2.4, 2.6), 2),
      ],
      material: { base: [0.74, 0.62, 0.46, 1], metallic: 0.0, roughness: 0.9 },
      tex: 'canvas_albedo',
    };
  },
  ballista() {
    return {
      meshes: [
        part('ballista_rail_w', (a, n) => B(a, n)(-0.4, 0.75, 0, 0.18, 0.3, 2.4), 2),
        part('ballista_rail_e', (a, n) => B(a, n)(0.4, 0.75, 0, 0.18, 0.3, 2.4), 2),
        part('ballista_cross_a', (a, n) => B(a, n)(0, 0.75, -0.8, 1.0, 0.25, 0.2), 2),
        part('ballista_cross_b', (a, n) => B(a, n)(0, 0.75, 0.8, 1.0, 0.25, 0.2), 2),
        part('ballista_arms', (a, n) => B(a, n)(0, 1.0, -0.9, 2.4, 0.18, 0.18), 2),
        part('ballista_slider', (a, n) => B(a, n)(0, 1.0, 0.1, 0.3, 0.15, 2.0), 2),
        part('ballista_winch', (a, n) => B(a, n)(0, 0.55, 0.9, 0.6, 0.4, 0.4), 2),
        part('ballista_wheel_fl', (a, n) => B(a, n)(-0.55, 0.25, -0.7, 0.18, 0.5, 0.5), 2),
        part('ballista_wheel_fr', (a, n) => B(a, n)(0.55, 0.25, -0.7, 0.18, 0.5, 0.5), 2),
        part('ballista_wheel_rl', (a, n) => B(a, n)(-0.55, 0.25, 0.7, 0.18, 0.5, 0.5), 2),
        part('ballista_wheel_rr', (a, n) => B(a, n)(0.55, 0.25, 0.7, 0.18, 0.5, 0.5), 2),
        part('COL_ballista', (a, n) => B(a, n)(0, 0.6, 0, 1.4, 1.2, 2.6), 2),
      ],
      sockets: [{ name: 'SOCKET_muzzle', at: [0, 1.1, -1.2] }],
      material: { base: [0.5, 0.36, 0.22, 1], metallic: 0.3, roughness: 0.65 },
      tex: 'timber_albedo',
    };
  },
};

// glTF 2.0 writer: shared emitGltf in kit-common (generator tag passed per kit).
// Emitted bytes are unchanged from the removed local copy.

// PNG writer + texture-slot painter live in ../../kit-common.mjs (shared).

// PBR texture set. Albedos capped at 512 (CT8's 1024 set cooked to a 93 MB
// float32 bundle; the AS3 BC-encoder row is the real fix — noted in REPORT).
// Albedo slots are sRGB bytes (linearized by the A2 cook), data slots linear.
function paintTextures() {
  const out = {};
  const mk = makeMk(TEX, out);
  mk('plaster_albedo', 512, (u, v) => {
    const n = fbm(u * 18, v * 18, 4), stain = fbm(u * 5 + 3, v * 5 + 9, 2);
    const m = (0.88 + 0.12 * n) * (stain > 0.62 ? 0.9 : 1);
    return [0.84 * m, 0.76 * m, 0.64 * m, 1];
  });
  mk('timber_albedo', 512, (u, v) => {
    const plank = Math.floor(u * 6);
    const gap = (u * 6) % 1 < 0.025 ? 0.45 : 1;
    const ring = 0.85 + 0.15 * Math.sin(v * 120 + 6 * fbm(u * 8, v * 8, 3) + plank * 2.1);
    return [0.58 * ring * gap, 0.41 * ring * gap, 0.24 * ring * gap, 1];
  });
  mk('stone_albedo', 512, (u, v) => {
    const row = Math.floor(v * 6), off = (row % 2) * 0.0833;
    const bx = (u + off) % 0.1667, mortar = (bx < 0.008 || (v * 6) % 1 < 0.04) ? 0.55 : 1;
    const n = fbm(u * 30, v * 30, 4);
    const m = (0.85 + 0.15 * n) * mortar;
    return [0.66 * m, 0.63 * m, 0.58 * m, 1];
  });
  mk('ground_albedo', 512, (u, v) => {
    const n = fbm(u * 20, v * 20, 4), g = fbm(u * 70 + 7, v * 70 + 3, 2);
    const cobble = g > 0.72 ? 1.12 : 1;
    const m = (0.8 + 0.2 * n) * cobble;
    return [0.7 * m, 0.57 * m, 0.4 * m, 1];
  });
  mk('canvas_albedo', 512, (u, v) => {
    const stripe = (u < 0.1 || u > 0.9) ? [0.62, 0.14, 0.12] : [0.76, 0.66, 0.5];
    const weave = 0.93 + 0.07 * Math.sin(u * 300) * Math.sin(v * 300);
    const seam = (v * 8) % 1 < 0.015 ? 0.9 : 1;
    return [stripe[0] * weave * seam, stripe[1] * weave * seam, stripe[2] * weave * seam, 1];
  });
  mk('metal_roughness', 256, (u, v) => {
    const n = fbm(u * 12, v * 12, 3);
    return [0.0, 0.35 + 0.1 * n, 0.9, 1];
  });
  mk('flat_normal', 256, () => [0.50196, 0.50196, 1, 1]);
  return out;
}

// ---------------------------------------------------------------------------
// Radiance RGBE HDRI writer (flat scanlines). One shared day sky for maps.
// ---------------------------------------------------------------------------
// RGBE helper lives in ../../kit-common.mjs (shared). One shared day sky.
function writeHdr(path, w, h, skyFn) {
  const head = Buffer.from(`#?RADIANCE\n# engine2040 CT9 original CC0 sky\nFORMAT=32-bit_rle_rgbe\n\n-Y ${h} +X ${w}\n`, 'ascii');
  const px = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const u = (x + 0.5) / w * 2 - 1;
      const v = 1 - (y + 0.5) / h * 2;
      px.set(skyFn(u, v), (y * w + x) * 4);
    }
  writeFileSync(path, Buffer.concat([head, px]));
}
function paintHdri() {
  const W = 128, H = 64;
  const s = (u, v, au, av, size) => Math.max(0, 1 - Math.hypot(u - au, v - av) / size);
  writeHdr(join(HDRI, 'maps_day.hdr'), W, H, (u, v) => {
    if (v < 0) return rgbe(0.35, 0.29, 0.2);
    const t = Math.min(1, v * 1.6);
    const sky = [0.95 - 0.5 * t, 0.75 - 0.15 * t, 0.6 + 0.35 * t].map((c) => c * (1.1 + 0.9 * (1 - t)));
    const sun = s(u, v, 0.3, 0.62, 0.09);
    return rgbe(sky[0] + sun * 20, sky[1] + sun * 18, sky[2] + sun * 15);
  });
}

// ---------------------------------------------------------------------------
// Validity probes: one parade placement per map. Every footprint disjoint in
// XZ, all resting (min_y = 0), inside +-99 m, grammar-clean unique names.
// The ludus probe is the WD2 consumer check: practice yard + barracks +
// blacksmith + party board + pells + rack + boundary wall must load, cook
// and place clean. The engine spatial run is judge.
// ---------------------------------------------------------------------------
const PLACEMENTS = {
  ludus: [
    { id: 1, name: 'yard', asset: 'yard_floor', x: -52, y: 0.15, z: 0, sx: 20, sy: 0.3, sz: 14 },
    { id: 2, name: 'barracks', asset: 'barracks', x: -30, y: 2.2, z: 0, sx: 10.6, sy: 4.4, sz: 5.1 },
    { id: 3, name: 'blacksmith', asset: 'blacksmith', x: -19, y: 2.5, z: 0, sx: 4.5, sy: 5.0, sz: 4.0 },
    { id: 4, name: 'party_board', asset: 'notice_board', x: -13, y: 1.4, z: 0, sx: 2.1, sy: 2.8, sz: 0.6 },
    { id: 5, name: 'pell_west', asset: 'training_post', x: -9, y: 1.1, z: -2, sx: 1.2, sy: 2.2, sz: 1.2 },
    { id: 6, name: 'pell_east', asset: 'training_post', x: -9, y: 1.1, z: 2, sx: 1.2, sy: 2.2, sz: 1.2 },
    { id: 7, name: 'rack', asset: 'weapon_rack', x: -5, y: 0.9, z: 0, sx: 2.4, sy: 1.8, sz: 0.6 },
    { id: 8, name: 'ludus_wall', asset: 'street_wall', x: 4, y: 1.95, z: 0, sx: 10, sy: 3.9, sz: 0.6 },
  ],
  district: [
    { id: 1, name: 'insula_a', asset: 'insula', x: -44, y: 3.65, z: 0, sx: 6.4, sy: 7.3, sz: 6.6 },
    { id: 2, name: 'insula_b', asset: 'insula', x: -34, y: 3.65, z: 0, sx: 6.4, sy: 7.3, sz: 6.6 },
    { id: 3, name: 'stall_fish', asset: 'market_stall', x: -26, y: 1.6, z: -2, sx: 3.0, sy: 3.2, sz: 2.2 },
    { id: 4, name: 'stall_bread', asset: 'market_stall', x: -26, y: 1.6, z: 2, sx: 3.0, sy: 3.2, sz: 2.2 },
    { id: 5, name: 'city_gate', asset: 'arch_gate', x: -14, y: 3.5, z: 0, sx: 6.6, sy: 7.0, sz: 2.4 },
    { id: 6, name: 'city_wall', asset: 'street_wall', x: -2, y: 1.95, z: 0, sx: 10, sy: 3.9, sz: 0.6 },
    { id: 7, name: 'fountain', asset: 'fountain', x: 8, y: 1.1, z: 0, sx: 2.4, sy: 2.2, sz: 2.4 },
  ],
  siege: [
    { id: 1, name: 'wall_north', asset: 'siege_wall', x: -34, y: 3.65, z: 0, sx: 8, sy: 7.3, sz: 1.8 },
    { id: 2, name: 'wall_south', asset: 'siege_wall', x: -22, y: 3.65, z: 0, sx: 8, sy: 7.3, sz: 1.8 },
    { id: 3, name: 'fort_gate', asset: 'arch_gate', x: -10, y: 3.5, z: 0, sx: 6.6, sy: 7.0, sz: 2.4 },
    { id: 4, name: 'camp_palisade', asset: 'palisade', x: 0, y: 1.55, z: 0, sx: 6.6, sy: 3.1, sz: 0.5 },
    { id: 5, name: 'tent_alpha', asset: 'tent', x: 8, y: 1.9, z: -2, sx: 3.2, sy: 3.8, sz: 2.6 },
    { id: 6, name: 'tent_beta', asset: 'tent', x: 8, y: 1.9, z: 2, sx: 3.2, sy: 3.8, sz: 2.6 },
    { id: 7, name: 'ballista_one', asset: 'ballista', x: 16, y: 0.7, z: 0, sx: 2.4, sy: 1.4, sz: 2.6 },
  ],
};

// Traversal hints for the WD9 district bake (TV1 actions on kit edges).
// Authored markers, not engine code: heights in meters, action by TV1 bounds
// (vault <= ~1.2 m, mantle <= ~2.4 m, climb above). WD9 turns these into
// sidecar edge markers baked into NV2 link registries.
const TRAVERSAL_HINTS = [
  { asset: 'insula', edge: 'parapet_s', height_m: 1.0, action: 'vault', note: 'roof parapet, both faces' },
  { asset: 'insula', edge: 'balcony', height_m: 3.25, action: 'climb', note: 'balcony slab from street; rail 0.5 above slab is vault' },
  { asset: 'insula', edge: 'roof_slab', height_m: 6.3, action: 'climb', note: 'from balcony via upper windows ledge (WD9 measures)' },
  { asset: 'street_wall', edge: 'crest', height_m: 3.2, action: 'climb', note: 'merlons 0.7 above crest are vault once up' },
  { asset: 'siege_wall', edge: 'walk', height_m: 6.3, action: 'climb', note: 'ladders in WD4; freesolo refused (over TV1 climb bound)' },
  { asset: 'arch_gate', edge: 'cap', height_m: 6.3, action: 'none', note: 'no route; pylon faces are shear (WD9 must not link)' },
  { asset: 'fountain', edge: 'rim', height_m: 0.95, action: 'vault', note: 'basin rim, low training vault' },
  { asset: 'market_stall', edge: 'counter', height_m: 0.95, action: 'vault', note: 'counter top, low vault' },
  { asset: 'barracks', edge: 'parapet', height_m: 4.4, action: 'climb', note: 'ludus roof access for WD2 vertical drills' },
];

// ---------------------------------------------------------------------------
// Main: emit glTF + textures + HDRI + placements + hints + manifest + proof
// bindings.
// ---------------------------------------------------------------------------
const manifest = { files: {} };
const track = (p, buf) => trackInto(manifest, p, buf);
const factors = {};
const colMesh = {};
for (const [name, build] of Object.entries(KIT)) {
  const built = build();
  const buf = emitGltf(name, built, "engine2040-ct9-gen (CC0-1.0 original)");
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
track('hdri/maps_day.hdr', readFileSync(join(HDRI, 'maps_day.hdr')));
const placed = (list) => list.map((a) => ({ ...a, parent: null, physical: true, vx: 0, vz: 0, yaw: 0 }));
for (const [map, list] of Object.entries(PLACEMENTS)) {
  const buf = Buffer.from(JSON.stringify({ actors: placed(list) }, null, 2));
  writeFileSync(join(ROOT, `placement_${map}.json`), buf);
  track(`placement_${map}.json`, buf);
}
const hintsBuf = Buffer.from(JSON.stringify({ hints: TRAVERSAL_HINTS }, null, 2));
writeFileSync(join(ROOT, 'traversal_hints.json'), hintsBuf);
track('traversal_hints.json', hintsBuf);
// Rust bindings for the proof crate (no JSON parsing in std-only proof).
const f32 = (v) => Number.isInteger(v) ? `${v}.0` : `${v}`;
const placedRs = (list) =>
  list.map((a) => `  Placed { id: ${a.id}, name: "${a.name}", x: ${a.x}_f32, y: ${a.y}_f32, z: ${a.z}_f32, sx: ${a.sx}_f32, sy: ${a.sy}_f32, sz: ${a.sz}_f32 },`).join('\n');
const rs = `// Generated by tools/gen.mjs — do not hand-edit.
pub struct Placed { pub id: u32, pub name: &'static str, pub x: f32, pub y: f32, pub z: f32, pub sx: f32, pub sy: f32, pub sz: f32 }
pub const PLACEMENT_LUDUS: &[Placed] = &[
${placedRs(PLACEMENTS.ludus)}
];
pub const PLACEMENT_DISTRICT: &[Placed] = &[
${placedRs(PLACEMENTS.district)}
];
pub const PLACEMENT_SIEGE: &[Placed] = &[
${placedRs(PLACEMENTS.siege)}
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
pub const TEXTURES: &[(&str, &str)] = &[("plaster_albedo", "BaseColor"), ("timber_albedo", "BaseColor"), ("stone_albedo", "BaseColor"), ("ground_albedo", "BaseColor"), ("canvas_albedo", "BaseColor"), ("metal_roughness", "MetallicRoughness"), ("flat_normal", "Normal")];
pub const ASSET_TEX: &[(&str, &str)] = &[${Object.keys(KIT).map((n) => `("${n}", "${KIT[n]().tex}")`).join(', ')}];
`;
writeFileSync(join(TOOLS, 'placement_data.rs'), rs);
const manBuf = Buffer.from(JSON.stringify(manifest, null, 2));
writeFileSync(join(ROOT, 'manifest.json'), manBuf);
console.log('wrote manifest.json with', Object.keys(manifest.files).length, 'files');

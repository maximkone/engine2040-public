//! soft_cape (PX44c): the capes' physfx soft cloths, the wiring the physfx cloth layer
//! (`crates/physfx/src/soft.rs`) was missing.
//!
//! With `E2040_SOFT_CAPE=1` in the environment when the look loads, every seen fighter's
//! cape is a Rapier soft-body sheet (`ClothDesc::CAPE`) hung in one `PhysFx` world and
//! stepped once per accepted tick by `PhysFx::step`. The draw reads the cloth's particle
//! grid, placed so its pinned row sits on this tick's shoulder row; with the switch off the
//! RZ7 `CapePose` Verlet chain draws exactly as before.
//!
//! Presentation only: the sim never reads a particle and nothing here is hashed, so the
//! golden hash cannot move. Units: meters and seconds.

#[cfg(test)]
use engine_arena::cloth::{Cloth, ClothGrid};
use engine_core::vec3::cross3;
use engine_physfx::{ClothDesc, PhysFx, SoftCloth, Tier};
use engine_render::v1::MeshData;

use super::{cape_row_into, dot3, small_f32, sub3, CapePose, MAX_LOOK_FIGHTERS};

/// The switch that draws each cape from a physfx soft cloth instead of the Verlet sheet:
/// `E2040_SOFT_CAPE=1` in the environment when the look loads. Off by default, and the
/// switch-off path is the RZ7 chain unchanged.
const SOFT_CAPE_ENV: &str = "E2040_SOFT_CAPE";
/// Columns of the soft cape: the pinned row of [`ClothDesc::CAPE`].
pub(super) const SOFT_CAPE_COLS: usize = ClothDesc::CAPE.cols;
/// Rows of the soft cape: the hang of [`ClothDesc::CAPE`].
pub(super) const SOFT_CAPE_ROWS: usize = ClothDesc::CAPE.rows;
/// How far behind the shoulder joints the cloth's pinned row is laid, meters: the torso
/// capsule's radius plus a hand's width, so the sheet hangs on the backplate instead of a
/// quarter meter in the air behind it. The chain's top row starts further out and its own
/// colliders pull it in; a soft cloth only has its pins.
pub(super) const SOFT_CAPE_BACK_M: f32 = 0.20;
/// How far behind the shoulder line the pinned row is drawn, meters: enough that the
/// attachment clears the armour shell.
pub(super) const SOFT_CAPE_PIN_CLEARANCE_M: f32 = 0.01;
/// How far behind the shoulder line the hem is drawn, meters: the sheet leans back as it
/// hangs, so the body cannot occlude it.
pub(super) const SOFT_CAPE_HEM_CLEARANCE_M: f32 = 0.05;
/// Quiet movement under which a cloth row counts as settled, meters per tick.
#[cfg(test)]
const SOFT_CAPE_QUIET_M: f32 = 0.001;
/// Consecutive quiet ticks that count a cloth as settled.
#[cfg(test)]
const SOFT_CAPE_QUIET_TICKS: usize = 10;

/// Whether the soft-cape switch is on for a look built now.
pub(super) fn soft_cape_switch() -> bool {
    std::env::var(SOFT_CAPE_ENV).as_deref() == Ok("1")
}

/// PX44c: the capes' physfx soft cloths, one [`ClothDesc::CAPE`] sheet per seen slot.
///
/// The cloth hangs in its own particle frame pinned at the shoulder line; each tick the
/// pinned row is taken from the slot's pose, so a draw places the sheet behind the body.
/// Stepped once per accepted tick by the same `PhysFx::step` the break and ragdoll layers
/// use. Presentation only: the sim never reads a particle and nothing here is hashed.
pub(super) struct SoftCape {
    /// The float world that holds the cloths.
    fx: PhysFx,
    /// Per slot: the cloth's slot in `fx`, `None` before the slot is first seen.
    slots: [Option<usize>; MAX_LOOK_FIGHTERS],
    /// Per slot: this tick's pinned top row, world meters, one point per column.
    pins: [[[f32; 3]; SOFT_CAPE_COLS]; MAX_LOOK_FIGHTERS],
    /// Per slot: the body's backward direction, the sheet's out-of-plane axis.
    back: [[f32; 3]; MAX_LOOK_FIGHTERS],
    /// Slots the last tick held a cloth for.
    stepped: usize,
    /// Top-row scratch, one pin point per column: no allocation per tick.
    row: Vec<[f32; 3]>,
}

impl SoftCape {
    /// A cold cloth world: no cloth until a slot is seen.
    pub(super) fn new() -> Self {
        Self {
            fx: PhysFx::new(Tier::Comfort),
            slots: [None; MAX_LOOK_FIGHTERS],
            pins: [[[0.0; 3]; SOFT_CAPE_COLS]; MAX_LOOK_FIGHTERS],
            back: [[0.0; 3]; MAX_LOOK_FIGHTERS],
            stepped: 0,
            row: Vec::with_capacity(SOFT_CAPE_COLS),
        }
    }

    /// Spawn a cloth for every seen slot on first sight, pin its row to this tick's
    /// shoulders and step the world one fixed tick. A refused spawn leaves the slot
    /// without a draw instead of failing the frame.
    pub(super) fn tick(&mut self, poses: &[Option<CapePose>; MAX_LOOK_FIGHTERS]) {
        self.stepped = 0;
        for (slot, pose) in poses.iter().enumerate() {
            let Some(pose) = pose else {
                continue;
            };
            if self.slots[slot].is_none() {
                match self.fx.spawn_cloth(ClothDesc::CAPE) {
                    Ok(handle) => self.slots[slot] = Some(handle),
                    Err(_) => continue,
                }
            }
            let [left, right] = pose.shoulders;
            let (s, c) = pose.yaw_sc;
            let mut row = std::mem::take(&mut self.row);
            cape_row_into(
                &mut row,
                left,
                right,
                [-s * SOFT_CAPE_BACK_M, 0.0, -c * SOFT_CAPE_BACK_M],
                SOFT_CAPE_COLS,
            );
            for (col, point) in row.iter().take(SOFT_CAPE_COLS).enumerate() {
                self.pins[slot][col] = *point;
            }
            self.row = row;
            self.back[slot] = [-s, 0.0, -c];
            self.stepped += 1;
        }
        if self.stepped > 0 {
            self.fx.step();
        }
    }

    /// The drawn side of `slot`'s cloth, the body's backward direction.
    pub(super) fn outward(&self, slot: usize) -> [f32; 3] {
        self.back[slot]
    }

    /// The cloth of `slot`, `None` before the slot is first seen or past the cloth cap.
    pub(super) fn cloth(&self, slot: usize) -> Option<&SoftCloth> {
        self.fx.cloth(self.slots.get(slot).copied().flatten()?)
    }

    /// `slot`'s drawn vertex positions: the cloth's own particle grid, placed so its pinned
    /// row sits on this tick's shoulder row, row by row pushed back off the body (see
    /// [`clearance_m`]). `None` when the slot holds no cloth.
    pub(super) fn positions(&self, slot: usize) -> Option<Vec<[f32; 3]>> {
        let cloth = self.cloth(slot)?;
        let desc = cloth.desc();
        let cols = desc.cols.min(SOFT_CAPE_COLS);
        if cols < 2 {
            return None;
        }
        let pins = &self.pins[slot];
        let back = self.back[slot];
        let span = sub3(pins[cols - 1], pins[0]);
        let len = dot3(span, span).sqrt();
        let right = if len > 1e-6 {
            [span[0] / len, span[1] / len, span[2] / len]
        } else {
            [1.0, 0.0, 0.0]
        };
        let mut out = Vec::with_capacity(cloth.nodes());
        for (i, p) in cloth.particles().iter().enumerate() {
            let col = i % desc.cols;
            if col >= cols {
                // A grid wider than a shoulder row is clamped to the last pin column.
                out.push(pins[cols - 1]);
                continue;
            }
            let row = i / desc.cols;
            let across = (p[0] - small_f32(col) * desc.spacing_m) * splay_m(row, desc.rows);
            let clear = p[2] + clearance_m(row, desc.rows) + fold_m(col, row, cols, desc.rows);
            out.push([
                pins[col][0] + right[0] * across + back[0] * clear,
                pins[col][1] + right[1] * across + p[1] + back[1] * clear,
                pins[col][2] + right[2] * across + back[2] * clear,
            ]);
        }
        Some(out)
    }
}

/// How far the sheet widens toward the hem, as a fraction of its shoulder width: a cloak
/// cut wider than the shoulders, so its silhouette shows past the body from the front.
pub(super) const SOFT_CAPE_SPLAY: f32 = 0.45;

/// The width factor of drawn row `row` of `rows`: 1 at the shoulders, 1 + splay at the hem.
fn splay_m(row: usize, rows: usize) -> f32 {
    if rows < 2 {
        return 1.0;
    }
    let down = small_f32(row.min(rows - 1)) / small_f32(rows - 1);
    1.0 + SOFT_CAPE_SPLAY * down
}

/// How far behind the shoulder line row `row` of `rows` is drawn, meters: the sheet leaves
/// the shoulders close to the body and leans back as it hangs, so the cloth stays off the
/// backplate surface it is pinned behind instead of z-fighting it.
fn clearance_m(row: usize, rows: usize) -> f32 {
    if rows < 2 {
        return SOFT_CAPE_PIN_CLEARANCE_M;
    }
    let t = small_f32(row.min(rows - 1)) / small_f32(rows - 1);
    SOFT_CAPE_PIN_CLEARANCE_M + (SOFT_CAPE_HEM_CLEARANCE_M - SOFT_CAPE_PIN_CLEARANCE_M) * t
}

/// Out-of-plane depth of the drape folds at the hem, meters. A sheet that settles dead
/// flat gives every vertex the same normal, so the cloth shades as one solid card: the
/// drawn sheet carries a shallow standing fold down its length instead, which the mesh
/// builder turns into per-vertex normals and the light into a drape.
pub(super) const SOFT_CAPE_FOLD_M: f32 = 0.035;
/// Full folds across the sheet's width: two half-waves, zero at both shoulders.
const SOFT_CAPE_FOLDS: f32 = 2.0;

/// How far the fold stands off the sheet at `col` of `cols` and `row` of `rows`, meters:
/// zero on the pinned row and at both side edges, deepest at the hem.
fn fold_m(col: usize, row: usize, cols: usize, rows: usize) -> f32 {
    if cols < 2 || rows < 2 {
        return 0.0;
    }
    let across = small_f32(col) / small_f32(cols - 1);
    let down = small_f32(row.min(rows - 1)) / small_f32(rows - 1);
    SOFT_CAPE_FOLD_M * down * (core::f32::consts::PI * SOFT_CAPE_FOLDS * across).sin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fighter_look::fighter_look_cape_body::{push_off_shells, shell_distance, CapeShell};
    use crate::fighter_look::tests::{cape_views, content_dir};
    use crate::fighter_look::{sub3, FighterLook, CAPE_TIER};

    /// PX44c: a look with the soft cape switch forced on or off, whatever the
    /// environment says.
    fn px44c_look(soft: bool) -> FighterLook {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.capes.soft = soft.then(SoftCape::new);
        look
    }

    /// A 64-bit digest of every particle of `slot`'s soft cloth.
    fn soft_digest(look: &FighterLook, slot: usize) -> Option<u64> {
        let cloth = look.capes.soft.as_ref()?.cloth(slot)?;
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for p in cloth.particles() {
            for v in p {
                hash ^= u64::from(v.to_bits());
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        Some(hash)
    }

    #[test]
    fn px44c_the_drawn_cape_comes_from_the_soft_cloth_when_the_switch_is_on() {
        // PX44c accept: with `E2040_SOFT_CAPE=1` (forced on here) the cape draw is
        // the physfx soft cloth's particle grid placed on the shoulder row; with the
        // switch off it is the RZ7 `CapePose` sheet, and the two differ.
        let hp = [1000u16; 2];
        let run = |soft: bool| {
            let mut look = px44c_look(soft);
            for tick in 0..90u32 {
                look.observe_tick(tick, &cape_views(0.0), &hp, &[]);
            }
            look
        };

        let off = run(false);
        let (off_positions, off_cols, off_rows, _) = off.cape_draw(0).expect("the Verlet cape");
        let grid = CAPE_TIER.knobs().cape;
        assert_eq!((off_cols, off_rows), (grid.cols, grid.rows));
        let sheet = off.cape_positions(0).expect("slot 0 cape");
        assert_eq!(
            off_positions.as_slice(),
            sheet,
            "switch off draws the CapePose chain"
        );

        let on = run(true);
        let (cloth_positions, cols, rows, _) = on.cape_draw(0).expect("the cloth cape");
        assert_eq!((cols, rows), (SOFT_CAPE_COLS, SOFT_CAPE_ROWS));
        assert_eq!(cloth_positions.len(), ClothDesc::CAPE.nodes());
        let soft = on.capes.soft.as_ref().expect("the soft capes are on");
        let cloth = soft.cloth(0).expect("slot 0 cloth");
        let pins = &soft.pins[0];
        // The drawn pinned row sits on the shoulder row the cloth was pinned to, pushed
        // back off the body by the pin clearance.
        let out = soft.outward(0);
        for (col, pin) in pins.iter().enumerate() {
            let d = sub3(cloth_positions[col], *pin);
            let along = dot3(d, out);
            assert!(
                (along - SOFT_CAPE_PIN_CLEARANCE_M).abs() < 1e-5 && d[1].abs() < 1e-5,
                "drawn pin {col} sits {d:?} ({along:.4} m outward), want \
                 {SOFT_CAPE_PIN_CLEARANCE_M} m outward and level"
            );
        }
        // The drawn hem is the cloth's own bottom row, one hang below the pins.
        let hem = (SOFT_CAPE_ROWS - 1) * SOFT_CAPE_COLS;
        let cloth_drop = -cloth.particles()[hem][1];
        let drawn_drop = pins[0][1] - cloth_positions[hem][1];
        assert!(
            (drawn_drop - cloth_drop).abs() < 0.05,
            "the hem drop comes from the cloth: drawn {drawn_drop:.3} m, cloth {cloth_drop:.3} m"
        );

        let off_hem = off_positions[off_positions.len() - 1];
        let on_hem = cloth_positions[cloth_positions.len() - 1];
        let differ = sub3(on_hem, off_hem);
        let differ_m =
            (differ[0] * differ[0] + differ[1] * differ[1] + differ[2] * differ[2]).sqrt();
        println!(
            "px44c: hem off {off_hem:?} on {on_hem:?}, differ {differ_m:.3} m, cloth sag {:.2} cm",
            cloth.sag_cm()
        );
        assert!(
            differ_m > 0.03,
            "the cloth hem differs from the CapePose hem: {differ_m:.3} m"
        );
    }

    #[test]
    fn px44c_two_seeded_runs_give_the_same_cloth_digest() {
        // Presentation only, but the cloth must still be a pure fold: the same seeded
        // ticks give the same particles twice.
        let hp = [1000u16; 2];
        let run = || {
            let mut look = px44c_look(true);
            for tick in 0..120u32 {
                look.observe_tick(tick, &cape_views(0.0), &hp, &[]);
            }
            soft_digest(&look, 0).expect("slot 0 cloth")
        };
        let a = run();
        let b = run();
        println!("px44c: cloth digest a={a:016x} b={b:016x}");
        assert_eq!(a, b, "two seeded cloth runs diverged");
    }

    #[test]
    fn px44c_a_still_cape_settles_and_stays_inside_its_bound() {
        // Floor: a cape on a body that never moves goes quiet under 1 mm a tick for
        // ten ticks in a row inside 120 ticks, and its hem hangs 70..95 cm below the
        // pin row, never past 101 cm. Measured 2026-10-24: settle tick 68, sag 94.89 cm,
        // worst 100.33 cm, hem difference 0.057 m, digest 02f9c1af7afcd61b.
        let hp = [1000u16; 2];
        let mut look = px44c_look(true);
        let mut prev: Vec<[f32; 3]> = Vec::new();
        let mut quiet = 0usize;
        let mut settle = None;
        let mut worst_sag_cm = 0.0f32;
        for tick in 0..600u32 {
            look.observe_tick(tick, &cape_views(0.0), &hp, &[]);
            let soft = look.capes.soft.as_ref().expect("the soft capes are on");
            let cloth = soft.cloth(0).expect("slot 0 cloth");
            let now = cloth.particles().to_vec();
            if !prev.is_empty() {
                let moved = now
                    .iter()
                    .zip(&prev)
                    .map(|(a, b)| {
                        let d = sub3(*a, *b);
                        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                    })
                    .fold(0.0_f32, f32::max);
                quiet = if moved < SOFT_CAPE_QUIET_M {
                    quiet + 1
                } else {
                    0
                };
                if quiet >= SOFT_CAPE_QUIET_TICKS && settle.is_none() {
                    settle = Some(usize::try_from(tick).unwrap_or(usize::MAX) + 1);
                }
            }
            worst_sag_cm = worst_sag_cm.max(cloth.sag_cm());
            prev = now;
        }
        let settle = settle.expect("the still cape settles");
        let soft = look.capes.soft.as_ref().expect("the soft capes are on");
        let sag_cm = soft.cloth(0).expect("slot 0 cloth").sag_cm();
        println!(
            "px44c: settle tick {settle} (quiet under {SOFT_CAPE_QUIET_M} m/tick for \
             {SOFT_CAPE_QUIET_TICKS} ticks), sag {sag_cm:.2} cm, worst {worst_sag_cm:.2} cm"
        );
        assert!(settle <= 120, "the still cape settles by tick {settle}");
        assert!(
            (70.0..=95.0).contains(&sag_cm),
            "the settled cape hangs {sag_cm:.2} cm"
        );
        assert!(
            worst_sag_cm <= 101.0,
            "the cape never hangs past its bound: {worst_sag_cm:.2} cm"
        );
    }

    #[test]
    fn px44c_the_drawn_cape_leans_back_off_the_body() {
        // The body occluded every pixel of a sheet drawn in its own plane: the same-tick
        // capture pair of 2026-10-24 read 0 cape pixels against 9862 for the RZ7 chain.
        // The draw now pushes each row back off the shoulder line, so every row clears the
        // body and the sheet leans back as it hangs.
        let hp = [1000u16; 2];
        let mut look = px44c_look(true);
        for tick in 0..120u32 {
            look.observe_tick(tick, &cape_views(0.0), &hp, &[]);
        }
        let soft = look.capes.soft.as_ref().expect("soft capes are on");
        let out = soft.outward(0);
        let pins = soft.pins[0];
        let (positions, cols, rows, _) = look.cape_draw(0).expect("the cloth cape");
        let clear = |row: usize| dot3(sub3(positions[row * cols], pins[0]), out);
        let pin_clear = clear(0);
        let hem_clear = clear(rows - 1);
        let mut worst = f32::INFINITY;
        for row in 0..rows {
            worst = worst.min(clear(row));
        }
        println!(
            "px44c: outward clearance pin {pin_clear:.3} m, hem {hem_clear:.3} m, \
             least {worst:.3} m over {rows} rows"
        );
        assert!(
            (pin_clear - SOFT_CAPE_PIN_CLEARANCE_M).abs() < 0.01,
            "the pin row sits {pin_clear:.3} m behind the shoulders"
        );
        assert!(
            hem_clear >= SOFT_CAPE_HEM_CLEARANCE_M - 0.03,
            "the hem clears the body by {hem_clear:.3} m"
        );
        assert!(
            worst >= SOFT_CAPE_PIN_CLEARANCE_M - 0.02,
            "every row clears the body, least {worst:.3} m"
        );
    }

    #[test]
    fn px44c_the_drawn_cape_carries_a_drape_fold() {
        // A sheet that settles dead flat gives every vertex one normal and shades as a
        // solid card, so the draw stands a shallow fold off the cloth: zero on the pin row
        // and at both edges, SOFT_CAPE_FOLD_M deep at the hem's mid-quarter. The mesh
        // builder must turn that into varying normals, or the light cannot read a drape.
        let hp = [1000u16; 2];
        let mut look = px44c_look(true);
        for tick in 0..120u32 {
            look.observe_tick(tick, &cape_views(0.0), &hp, &[]);
        }
        let soft = look.capes.soft.as_ref().expect("soft capes are on");
        let out = soft.outward(0);
        let pins = soft.pins[0];
        let (positions, cols, rows, outward) = look.cape_draw(0).expect("the cloth cape");
        let cloth = soft.cloth(0).expect("slot 0 cloth");
        let hem = rows - 1;
        let depth = |col: usize| {
            let clear = dot3(sub3(positions[hem * cols + col], pins[col]), out);
            clear - clearance_m(hem, rows) - cloth.particles()[hem * cols + col][2]
        };
        let deepest = (0..cols).map(depth).fold(f32::NEG_INFINITY, f32::max);
        let pin_depth = (0..cols)
            .map(|col| dot3(sub3(positions[col], pins[col]), out) - clearance_m(0, rows))
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (pin_depth - 0.0).abs() < 1e-5,
            "the pin row draws flat on the shoulders, deepest {pin_depth:.4} m"
        );
        assert!(
            (deepest - SOFT_CAPE_FOLD_M).abs() < 0.005,
            "the hem's fold stands {deepest:.4} m off the sheet"
        );
        let mesh = cape_mesh_at(positions, cols, rows, outward);
        let spread = mesh
            .normals
            .iter()
            .map(|n| {
                dot3(*n, mesh.normals[0])
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees()
            })
            .fold(0.0_f32, f32::max);
        println!("px44c: hem fold {deepest:.3} m, normal spread {spread:.1} deg");
        assert!(
            spread > 2.0,
            "the fold must bend the normals for the light to read it, spread {spread:.1} deg"
        );
    }

    #[test]
    fn px44c_the_soft_cape_mesh_draws_both_of_its_sides() {
        // The render pipelines cull back faces, so a one-sided cloth is invisible from its
        // own back side: the soft cape's mesh carries every face twice, the sheet then the
        // same face wound the other way with flipped normals.
        let hp = [1000u16; 2];
        let mut look = px44c_look(true);
        for tick in 0..90u32 {
            look.observe_tick(tick, &cape_views(0.0), &hp, &[]);
        }
        let (positions, cols, rows, outward) = look.cape_draw(0).expect("the cloth cape");
        let one = cape_mesh_at(positions.clone(), cols, rows, outward);
        let both = cape_mesh_two_sided(positions, cols, rows, outward);
        let verts = one.positions.len();
        assert_eq!(
            both.positions.len(),
            verts * 2,
            "one mirrored copy of every vertex"
        );
        assert_eq!(
            both.indices.len(),
            one.indices.len() * 2,
            "and of every face"
        );
        for i in 0..verts {
            assert_eq!(
                both.positions[i], one.positions[i],
                "the sheet keeps its own vertices"
            );
            assert_eq!(
                both.positions[verts + i],
                one.positions[i],
                "the copy sits on the sheet"
            );
            assert_eq!(
                both.uvs[verts + i],
                one.uvs[i],
                "and samples the same texel"
            );
            let n = one.normals[i];
            let m = both.normals[verts + i];
            assert!(
                (dot3(m, n) + 1.0).abs() < 1e-5,
                "the copy at {i} faces the other way: {n:?} vs {m:?}"
            );
        }
        for (a, b) in one
            .indices
            .chunks_exact(3)
            .zip(both.indices[one.indices.len()..].chunks_exact(3))
        {
            let want = [
                a[0] + u32::try_from(verts).unwrap_or(0),
                a[2] + u32::try_from(verts).unwrap_or(0),
                a[1] + u32::try_from(verts).unwrap_or(0),
            ];
            assert_eq!(*b, want, "the copy of face {a:?} is wound the other way");
        }
        println!(
            "px44c: cloth mesh {} verts, {} faces, both sides",
            both.positions.len(),
            both.indices.len() / 3
        );
    }

    #[test]
    fn px44c_the_cape_draw_keeps_the_cloth_off_the_body() {
        // The cloth hangs in its own frame and only its pins follow the body, so a lean, a
        // twist or a stronger wind that deforms the sheet into the body would draw it
        // inside. The draw pushes every vertex out of the body's own capsules. Measured
        // 2026-10-24 on the arena rig: a sheet hung 0.20 m behind the shoulders never enters
        // a capsule by itself (0 vertex-shell pairs over 150 ticks standing and then
        // running), so this is a guard for the shapes a collider or a wind hook will bring,
        // and the capsule arithmetic below is what proves it works.
        let shell = CapeShell {
            a: [0.0, 0.0, 0.0],
            b: [0.0, 2.0, 0.0],
            radius: 0.10,
        };
        let mut inside = [[0.0, 1.0, 0.05]];
        assert!(
            (shell_distance(shell, inside[0]) + 0.05).abs() < 1e-6,
            "a point 5 cm off the axis is 5 cm inside the capsule"
        );
        assert_eq!(push_off_shells(&mut inside, &[shell], 0.02), 1);
        assert!(
            (shell_distance(shell, inside[0]) - 0.02).abs() < 1e-5,
            "pushed to the surface plus the skin, got {:?}",
            inside[0]
        );
        let mut outside = [[0.0, 1.0, 0.5]];
        assert_eq!(push_off_shells(&mut outside, &[shell], 0.02), 0);
        assert_eq!(outside[0], [0.0, 1.0, 0.5], "a vertex outside stays put");
        // A degenerate capsule is a ball, and a vertex on its axis has no radial direction:
        // it must leave along +y rather than stay inside or panic.
        let ball = CapeShell {
            a: [1.0, 1.0, 1.0],
            b: [1.0, 1.0, 1.0],
            radius: 0.10,
        };
        let mut on_axis = [[1.0, 1.0, 1.0]];
        assert_eq!(push_off_shells(&mut on_axis, &[ball], 0.0), 1);
        assert!(shell_distance(ball, on_axis[0]).abs() < 1e-5);
        assert_eq!(on_axis[0], [1.0, 1.1, 1.0]);

        // The fixture: the drawn sheet of a standing and then running fighter never lies
        // inside one of its own capsules.
        let hp = [1000u16; 2];
        let mut look = px44c_look(true);
        let mut raw_inside = 0usize;
        let mut drawn_inside = 0usize;
        for tick in 0..150u32 {
            let speed = if tick < 75 { 0.0 } else { 3.0 };
            look.observe_tick(tick, &cape_views(speed), &hp, &[]);
            let Some(pose) = look.capes.poses[0] else {
                continue;
            };
            let shells = look.capes.shells[pose.shells.0..pose.shells.1].to_vec();
            let raw = look
                .capes
                .soft
                .as_ref()
                .expect("soft capes are on")
                .positions(0)
                .expect("slot 0 cloth");
            for p in &raw {
                raw_inside += shells
                    .iter()
                    .filter(|shell| shell_distance(**shell, *p) < 0.0)
                    .count();
            }
            let (drawn, ..) = look.cape_draw(0).expect("the cloth cape");
            for p in &drawn {
                drawn_inside += shells
                    .iter()
                    .filter(|shell| shell_distance(**shell, *p) < 0.0)
                    .count();
            }
        }
        println!(
            "px44c: drawn cape inside the body on {drawn_inside} vertex-shell pairs \
             (raw sheet {raw_inside})"
        );
        assert_eq!(drawn_inside, 0, "the drawn cape passes through the body");
    }

    #[test]
    fn px44c_a_running_cape_stays_on_the_body() {
        // The arena's fighters run. The pin row is placed from the shoulders every
        // tick, so the drawn sheet must hang from them at 3 m/s too: finite, inside
        // 70..101 cm below the pin row, and on the same side of the body. This is
        // the case a still-body test cannot see.
        let hp = [1000u16; 2];
        let mut look = px44c_look(true);
        let mut worst_low = 0.0f32;
        let mut worst_high = 0.0f32;
        let mut bad = 0usize;
        for tick in 0..120u32 {
            // 5 cm a tick at 60 Hz is 3 m/s: a running fighter.
            let z = -0.05 * small_f32(usize::try_from(tick).unwrap_or(0));
            look.observe_tick(tick, &cape_views(z), &hp, &[]);
            let Some((positions, cols, rows, _)) = look.cape_draw(0) else {
                bad += 1;
                continue;
            };
            let top = positions[0][1];
            let hem = (rows - 1) * cols;
            let drop_cm = (top - positions[hem][1]) * 100.0;
            worst_low = worst_low.max(drop_cm.min(0.0));
            worst_high = worst_high.max(drop_cm.max(0.0));
            if !positions.iter().flatten().all(|v| v.is_finite()) {
                bad += 1;
            }
        }
        println!(
            "px44c running: hem below the pin row {worst_high:.2} cm at most, \
             {worst_low:.2} cm at least (negative = above), bad frames {bad}"
        );
        assert_eq!(bad, 0, "every running frame draws a finite soft cape");
        assert!(
            (70.0..=101.0).contains(&worst_high),
            "the running cape hangs {worst_high:.2} cm below its pin row"
        );
        assert!(
            worst_low >= -1.0,
            "the running cape never rises above its pin row: {worst_low:.2} cm"
        );
    }
}

/// One unit normal of the cloth at vertex `(c, r)`, turned onto `outward`'s
/// side so the cape's face points away from the body it hangs behind.
fn cape_normal(
    at: impl Fn(usize, usize) -> [f32; 3],
    cols: usize,
    rows: usize,
    c: usize,
    r: usize,
    outward: [f32; 3],
) -> [f32; 3] {
    let right = sub3(at((c + 1).min(cols - 1), r), at(c.saturating_sub(1), r));
    let down = sub3(at(c, (r + 1).min(rows - 1)), at(c, r.saturating_sub(1)));
    let mut n = cross3(right, down);
    let len2 = dot3(n, n);
    if len2.is_finite() && len2 > 1e-12 {
        let inv = 1.0 / len2.sqrt();
        n = [n[0] * inv, n[1] * inv, n[2] * inv];
    } else {
        n = [0.0, 1.0, 0.0];
    }
    if dot3(n, outward) < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    n
}

/// The cape's triangle mesh at its current positions: unit uvs over the grid,
/// normals on `outward`'s side. Pure; the sheet is a `cols` by `rows` grid hung
/// from its pinned top row.
#[cfg(test)]
pub(super) fn cape_mesh(cloth: &Cloth, grid: ClothGrid, outward: [f32; 3]) -> MeshData {
    cape_mesh_at(cloth.positions().to_vec(), grid.cols, grid.rows, outward)
}

/// The same mesh over a flat vertex grid of `cols` by `rows` points: the one builder
/// the Verlet sheet and the physfx soft cloth both draw through. Takes the positions
/// by value, so a frame adds no copy past the one the caller already built.
pub(super) fn cape_mesh_at(
    positions: Vec<[f32; 3]>,
    cols: usize,
    rows: usize,
    outward: [f32; 3],
) -> MeshData {
    let at = |c: usize, r: usize| positions[r * cols + c];
    let mut normals = Vec::with_capacity(positions.len());
    let mut uvs = Vec::with_capacity(positions.len());
    for r in 0..rows {
        for c in 0..cols {
            normals.push(cape_normal(at, cols, rows, c, r, outward));
            uvs.push([
                small_f32(c) / small_f32(cols - 1),
                small_f32(r) / small_f32(rows - 1),
            ]);
        }
    }
    let mut indices = Vec::with_capacity((cols - 1) * (rows - 1) * 6);
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let a = u32::try_from(r * cols + c).unwrap_or(u32::MAX);
            let b = a + 1;
            let d = a + u32::try_from(cols).unwrap_or(u32::MAX);
            let e = d + 1;
            indices.extend_from_slice(&[a, b, d, b, e, d]);
        }
    }
    MeshData {
        positions,
        normals,
        uvs,
        indices,
        tangents: Vec::new(),
    }
}

/// The soft cloth's draw mesh: [`cape_mesh_at`]'s sheet plus a second copy of every face
/// with reversed winding and its normals flipped, so the cloth is visible from its own back
/// side too. The render pipelines cull back faces and a cloth is a sheet, not a solid:
/// one-sided, the cape vanished the moment the camera crossed to its inner side (a
/// same-tick capture pair of 2026-10-24 drew 39 653 px of cloth from the front and 0 px
/// from behind).
pub(super) fn cape_mesh_two_sided(
    positions: Vec<[f32; 3]>,
    cols: usize,
    rows: usize,
    outward: [f32; 3],
) -> MeshData {
    let mut mesh = cape_mesh_at(positions, cols, rows, outward);
    let base = u32::try_from(mesh.positions.len()).unwrap_or(u32::MAX);
    let verts = mesh.positions.len();
    mesh.positions.extend_from_within(..verts);
    mesh.uvs.extend_from_within(..verts);
    mesh.normals.extend_from_within(..verts);
    for n in mesh.normals.iter_mut().skip(verts) {
        *n = [-n[0], -n[1], -n[2]];
    }
    let front = std::mem::take(&mut mesh.indices);
    mesh.indices.reserve(front.len() * 2);
    mesh.indices.extend_from_slice(&front);
    for face in front.chunks_exact(3) {
        mesh.indices
            .extend_from_slice(&[face[0] + base, face[2] + base, face[1] + base]);
    }
    mesh
}

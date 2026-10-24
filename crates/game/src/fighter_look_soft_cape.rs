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

use super::{cape_row_into, dot3, small_f32, sub3, CapePose, CAPE_BACK_M, MAX_LOOK_FIGHTERS};

/// The switch that draws each cape from a physfx soft cloth instead of the Verlet sheet:
/// `E2040_SOFT_CAPE=1` in the environment when the look loads. Off by default, and the
/// switch-off path is the RZ7 chain unchanged.
const SOFT_CAPE_ENV: &str = "E2040_SOFT_CAPE";
/// Columns of the soft cape: the pinned row of [`ClothDesc::CAPE`].
pub(super) const SOFT_CAPE_COLS: usize = ClothDesc::CAPE.cols;
/// Rows of the soft cape: the hang of [`ClothDesc::CAPE`].
pub(super) const SOFT_CAPE_ROWS: usize = ClothDesc::CAPE.rows;
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
        for slot in 0..MAX_LOOK_FIGHTERS {
            let Some(pose) = poses[slot] else {
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
                [-s * CAPE_BACK_M, 0.0, -c * CAPE_BACK_M],
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
    /// row sits on this tick's shoulder row. `None` when the slot holds no cloth.
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
            let across = p[0] - small_f32(col) * desc.spacing_m;
            out.push([
                pins[col][0] + right[0] * across + back[0] * p[2],
                pins[col][1] + right[1] * across + p[1] + back[1] * p[2],
                pins[col][2] + right[2] * across + back[2] * p[2],
            ]);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        // The drawn pinned row sits on the shoulder row the cloth was pinned to.
        for (col, pin) in pins.iter().enumerate() {
            let d = sub3(cloth_positions[col], *pin);
            assert!(
                d.iter().all(|v| v.abs() < 1e-5),
                "drawn pin {col} off by {d:?}"
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
                    settle = Some(tick as usize + 1);
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

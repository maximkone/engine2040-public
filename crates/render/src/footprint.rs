//! footprint: a planted foot's heel-and-toe print, and the sand dent it leaves.
//!
//! A print is transient, unlike blood or a scuff: it holds a dent in the sand
//! for [`PRINT_LIFETIME_TICKS`] and then is gone, leaving no mass behind. So
//! the map a caller reads is not a deposit at all — it is the raster of the
//! prints that are still live, rebuilt from that list, which is why the map is
//! exactly zero the tick the last print expires. The tests pin both ends: one
//! planted step writes texels, and every one of them reads zero at tick 600.
//!
//! Geometry: the heel and the toe are two discs on the sand plane, one print
//! length apart along the foot's heading. Units are metres inside the 20 m pit
//! and texels on the [`MAP_DIM`]-square map the floor pass samples. Determinism:
//! integer fade and integer texel centres, discs rasterised in a fixed order
//! (row by row, then column) with a max blend, so the same print bytes give the
//! same map bytes. Std-only, no GPU, no clock, not in a simulation hash.
//!
//! Caller: [`crate::ground_state::GroundState`], which owns the live print list
//! and the map. The plant signal itself is the fighter look's (game
//! `fighter_look_feet::Plant`), which is private to that crate: this module
//! takes plain metres and a heading, never the look's types.

use crate::ground_state::{MAP_DIM, PIT_METERS};

/// Ticks a print stays in the sand: 10 s at 60 Hz.
pub const PRINT_LIFETIME_TICKS: u32 = 600;
/// Heel disc radius in metres.
pub const PRINT_HEEL_RADIUS_M: f32 = 0.042;
/// Toe disc radius in metres: the ball of the foot is wider than the heel.
pub const PRINT_TOE_RADIUS_M: f32 = 0.055;
/// Heel centre to toe centre along the heading, in metres.
pub const PRINT_LENGTH_M: f32 = 0.19;
/// Deepest dent a fresh print leaves, in the map's 0..=255 units.
pub const PRINT_FULL_DEPTH: u8 = 255;
/// Extra heel drop of the oldest print: the dent never fades to nothing while
/// a print is live, so a print is visible for its whole 600 ticks.
pub const PRINT_FLOOR_DEPTH: u8 = 1;

/// One planted foot on the sand plane. World `x`/`y` are the pit's metres and
/// `heading` is the foot's forward direction in radians (`atan2(dy, dx)`), so
/// the toe sits `PRINT_LENGTH_M` ahead of the heel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Print {
    /// Heel centre, metres inside the pit.
    pub x_m: f32,
    /// Heel centre, metres inside the pit.
    pub y_m: f32,
    /// Forward direction of the foot in radians.
    pub heading: f32,
    /// Tick the foot planted.
    pub born_tick: u32,
}

impl Print {
    /// Ticks this print has been in the sand at `tick`.
    pub fn age(&self, tick: u32) -> u32 {
        tick.saturating_sub(self.born_tick)
    }

    /// True once the print has served its lifetime: the map drops it.
    pub fn expired(&self, tick: u32) -> bool {
        self.age(tick) >= PRINT_LIFETIME_TICKS
    }

    /// Dent depth at `tick`, in the map's 0..=255 units: full on the planting
    /// tick, the floor from the last tick before the lifetime on.
    pub fn depth(&self, tick: u32) -> u8 {
        depth_at(self.age(tick))
    }

    /// The heel and toe centres in metres, heel first.
    pub fn lobes(&self) -> ([f32; 2], [f32; 2]) {
        let heel = [self.x_m, self.y_m];
        let (sin, cos) = self.heading.sin_cos();
        let toe = [
            self.x_m + cos * PRINT_LENGTH_M,
            self.y_m + sin * PRINT_LENGTH_M,
        ];
        (heel, toe)
    }
}

/// Dent depth of a print aged `age` ticks: strictly decreasing from
/// [`PRINT_FULL_DEPTH`] to [`PRINT_FLOOR_DEPTH`], and 0 from the lifetime on.
pub fn depth_at(age: u32) -> u8 {
    if age >= PRINT_LIFETIME_TICKS {
        return 0;
    }
    let span = u32::from(PRINT_FULL_DEPTH - PRINT_FLOOR_DEPTH);
    let left = PRINT_LIFETIME_TICKS - age;
    let step = span * left / PRINT_LIFETIME_TICKS;
    PRINT_FLOOR_DEPTH + u8::try_from(step).unwrap_or(PRINT_FULL_DEPTH - PRINT_FLOOR_DEPTH)
}

/// Rasterise one print into `map` (`MAP_DIM * MAP_DIM` bytes, row major) at
/// `depth`, keeping the deeper value where prints overlap.
pub fn stamp(map: &mut [u8], print: &Print, depth: u8) {
    if depth == 0 || map.len() < MAP_DIM * MAP_DIM {
        return;
    }
    let (heel, toe) = print.lobes();
    for (centre, radius_m) in [(heel, PRINT_HEEL_RADIUS_M), (toe, PRINT_TOE_RADIUS_M)] {
        disc(
            map,
            texel_of(centre[0]),
            texel_of(centre[1]),
            radius_texels(radius_m),
            depth,
        );
    }
}

/// A filled disc of `depth` on the map, keeping the deeper value on overlap.
pub fn disc(map: &mut [u8], cx: u32, cy: u32, radius: u32, depth: u8) {
    let r = i64::from(radius);
    let (cx, cy) = (i64::from(cx), i64::from(cy));
    let last = i64::try_from(MAP_DIM).unwrap_or(i64::MAX) - 1;
    let r2 = r * r;
    for y in (cy - r).max(0)..=(cy + r).min(last) {
        for x in (cx - r).max(0)..=(cx + r).min(last) {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy > r2 {
                continue;
            }
            let i = usize::try_from(y).unwrap_or(0) * MAP_DIM + usize::try_from(x).unwrap_or(0);
            if let Some(slot) = map.get_mut(i) {
                *slot = (*slot).max(depth);
            }
        }
    }
}

/// Texel column or row of a coordinate inside the pit. The one float step here:
/// the caller accepts only `0 ..= PIT_METERS`, so the quotient is inside
/// `0 ..= MAP_DIM` and the rounded value fits `u32`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller accepts only 0 ..= PIT_METERS, so the quotient is inside 0 ..= MAP_DIM and the rounded value fits u32"
)]
pub fn texel_of(meters: f32) -> u32 {
    let scaled = (meters.clamp(0.0, PIT_METERS) / PIT_METERS * MAP_DIM as f32).round();
    (scaled as u32).min(MAP_DIM as u32 - 1)
}

/// Disc radius in texels, at least one texel so a small foot still prints.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a lobe radius is at most 2 m, so the quotient is inside 1 ..= 103 texels"
)]
fn radius_texels(radius_m: f32) -> u32 {
    let scaled = (radius_m / PIT_METERS * MAP_DIM as f32).round();
    (scaled as u32).max(1)
}

#[cfg(test)]
#[path = "footprint_tests.rs"]
mod tests;

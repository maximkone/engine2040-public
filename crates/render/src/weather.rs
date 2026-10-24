//! weather: the arena's weather state and the CPU oracle of the half-res ripple
//! pass (SW142): rain sheets, ripple rings pressed into the sand, a wet sheen.
//!
//! Owner: render. Units: meters for world space, ticks at 60 Hz (never seconds:
//! a capture is reproducible), permille for the gauge. View only, never hashed.
//! Determinism: the ripple field is a function of the world position and the
//! tick only - one integer hash, fixed loop order, no clock, no map - so the
//! same tick and the same gauge give the same bytes.
//!
//! Donor: godotshaders.com/shader/rain-drop-floor-effect (`hash12`, `hash22`,
//! `MAX_RADIUS 2`, `sin(31*d)` ring, `offset_amount 0.05`, `effect_mix 0.55`)
//! for the ring field, and rain-puddles-with-ripples-and-reflection for the
//! wetness/roughness coupling. The pass lives in `shaders/weather.wgsl`; this
//! module is the oracle it must match.

/// World-space ripple grid: one drop cell per half meter.
pub const RIPPLE_CELL_M: f32 = 0.5;

/// Donor `MAX_RADIUS`: a ring fades out two meters from the drop.
pub const RIPPLE_MAX_RADIUS: f32 = 2.0;

/// Donor `31*d`: the ring's radial frequency.
pub const RIPPLE_RING_HZ: f32 = 31.0;

/// Donor `offset_amount`: the peak UV displacement of one ring, at gauge 1000.
pub const RIPPLE_OFFSET: f32 = 0.05;

/// Donor `effect_mix`: how much of the ring reaches the frame.
pub const RIPPLE_EFFECT_MIX: f32 = 0.55;

/// Ticks one ring lives: a two-second cycle at the fixed 60 Hz tick.
pub const RIPPLE_TICKS: u32 = 120;

/// Earliest age a drop's phase may take, so every cell of the window carries a
/// ring that has visibly started (no cell sits exactly at its birth).
pub const RING_MIN_AGE: f32 = 0.02;

/// Half-res pass: one sim cell per two frame pixels.
pub const HALF_RES_DIVISOR: u32 = 2;

/// Potato tier: one cell per three frame pixels.
pub const POTATO_RES_DIVISOR: u32 = 3;

/// Half-res taps: the 5x5 neighbourhood.
pub const HALF_RES_TAPS: u32 = 25;

/// Potato taps: the row's 7-tap kernel.
pub const POTATO_TAPS: u32 = 7;

/// Wet sand keeps this share of its albedo at gauge 1000.
pub const WET_ALBEDO_SCALE: f32 = 0.75;

/// Wet sand loses this much roughness at gauge 1000.
pub const WET_ROUGHNESS_DELTA: f32 = -0.3;

/// Rain streak length per metre per second of fall (see
/// [`crate::particles_gpu::STRETCH_GAIN`]).
pub const STREAK_GAIN: f32 = crate::particles_gpu::STRETCH_GAIN;

/// The three weather states the arena knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weather {
    /// No rain, no wind: the field is zero and the frame is byte-equal.
    Clear,
    /// Wind only: dust drifts, no rings.
    Gust,
    /// Rain: rings, streaks and the wet sheen.
    Rain,
}

impl Weather {
    /// The gauge in permille: Clear 0, Gust 350, Rain 1000.
    #[must_use]
    pub const fn permille(self) -> u32 {
        match self {
            Self::Clear => 0,
            Self::Gust => 350,
            Self::Rain => 1000,
        }
    }

    /// The gauge over 1000, in `0..=1`.
    #[must_use]
    pub fn intensity(self) -> f32 {
        unit_of_permille(self.permille())
    }
}

/// A gauge in `0..=1000` as `0.0..=1.0` (a permille, not a ratio, so a receipt
/// prints a whole number).
#[must_use]
pub fn unit_of_permille(permille: u32) -> f32 {
    f32::from(u16::try_from(permille.min(1000)).unwrap_or(0)) / 1000.0
}

/// The weather state: stepped on ticks, so a capture replays exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherState {
    weather: Weather,
    tick: u64,
}

impl Default for WeatherState {
    fn default() -> Self {
        Self::new(Weather::Clear)
    }
}

impl WeatherState {
    /// A state at tick 0.
    #[must_use]
    pub const fn new(weather: Weather) -> Self {
        Self { weather, tick: 0 }
    }

    /// The current weather.
    #[must_use]
    pub const fn weather(&self) -> Weather {
        self.weather
    }

    /// The gauge in `0.0..=1.0`.
    #[must_use]
    pub fn intensity(&self) -> f32 {
        self.weather.intensity()
    }

    /// Ticks advanced since construction.
    #[must_use]
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    /// Advance one tick, at most [`u64::MAX`]; the ripple field reads the tick,
    /// never a wall clock.
    pub fn step(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }

    /// Switch weather; the tick keeps running so a change never jumps the
    /// rings back to their birth.
    pub fn set(&mut self, weather: Weather) {
        self.weather = weather;
    }
}

/// The peak a ring may displace the screen UV by, in `0.0..=1.0` UV space:
/// [`RIPPLE_OFFSET`] scaled by the gauge.
#[must_use]
pub fn ripple_peak_offset(intensity: f32) -> f32 {
    RIPPLE_OFFSET * intensity.clamp(0.0, 1.0)
}

/// The UV displacement one world point earns from the 5x5 ring neighbourhood
/// at `tick` and gauge `intensity`. Zero at gauge 0 and outside every ring.
#[must_use]
pub fn ripple_displacement(world_xz: [f32; 2], tick: u64, intensity: f32) -> [f32; 2] {
    let gauge = intensity.clamp(0.0, 1.0);
    if gauge == 0.0 || !world_xz[0].is_finite() || !world_xz[1].is_finite() {
        return [0.0; 2];
    }
    let cell = [cell_coord(world_xz[0]), cell_coord(world_xz[1])];
    let mut offset = [0.0f32; 2];
    for dz in -2..=2 {
        for dx in -2..=2 {
            // p1 is the ring's own centre; p2 and p3 are the derivative's two
            // samples either side of it, the donor's ((p2-p1)/(2h)).
            let centre = drop_centre(cell[0] + dx, cell[1] + dz);
            let age = drop_age(cell[0] + dx, cell[1] + dz, tick);
            let radius = age * RIPPLE_MAX_RADIUS;
            if radius <= 0.0 {
                continue;
            }
            let to_point = [world_xz[0] - centre[0], world_xz[1] - centre[1]];
            let distance = (to_point[0] * to_point[0] + to_point[1] * to_point[1]).sqrt();
            if distance >= radius || distance >= RIPPLE_MAX_RADIUS {
                continue;
            }
            let ring = (RIPPLE_RING_HZ * distance).sin() * (1.0 - age) * (1.0 - age);
            let scale =
                ripple_peak_offset(gauge) * RIPPLE_EFFECT_MIX * ring / (RIPPLE_CELL_M * 2.0);
            offset[0] += to_point[0] * scale;
            offset[1] += to_point[1] * scale;
        }
    }
    let limit = ripple_peak_offset(gauge);
    let length = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt();
    if length > limit && length > 0.0 {
        // Overlapping rings never move the UV past the ring's own peak: one
        // drop per cell is the field's whole budget.
        offset[0] *= limit / length;
        offset[1] *= limit / length;
    }
    offset
}

/// How many of a 5x5 window's drop cells carry a live ring at `tick` and
/// `intensity`: [`HALF_RES_TAPS`] at rain, 0 at clear.
#[must_use]
pub fn ring_cells(tick: u64, intensity: f32) -> u32 {
    if intensity <= 0.0 {
        return 0;
    }
    let mut live = 0;
    for dz in -2..=2 {
        for dx in -2..=2 {
            let age = drop_age(dx, dz, tick);
            if age > 0.0 && age < 1.0 {
                live += 1;
            }
        }
    }
    live
}

/// The half-res pass's output for a `cells` x `cells` world patch at
/// `cell_m` spacing: two little-endian f32 lanes per cell, the bytes a capture
/// compares.
#[must_use]
pub fn ripple_frame(cells: u32, cell_m: f32, tick: u64, intensity: f32) -> Vec<u8> {
    let cells = cells.max(1);
    let mut bytes = Vec::with_capacity(usize::try_from(cells * cells).unwrap_or(0) * 8);
    for y in 0..cells {
        for x in 0..cells {
            let world = [
                f32::from(u16::try_from(x).unwrap_or(0)) * cell_m,
                f32::from(u16::try_from(y).unwrap_or(0)) * cell_m,
            ];
            for lane in ripple_displacement(world, tick, intensity) {
                bytes.extend_from_slice(&lane.to_le_bytes());
            }
        }
    }
    bytes
}

/// Wet ground under the gauge: `intensity` 0 leaves both inputs exactly as they
/// were, 1 takes the albedo to [`WET_ALBEDO_SCALE`] and the roughness down by
/// [`WET_ROUGHNESS_DELTA`].
#[must_use]
pub fn ground_wet(albedo: f32, roughness: f32, intensity: f32) -> (f32, f32) {
    let gauge = intensity.clamp(0.0, 1.0);
    if gauge == 0.0 {
        return (albedo, roughness);
    }
    let albedo_scale = 1.0 - (1.0 - WET_ALBEDO_SCALE) * gauge;
    let wet_albedo = albedo * albedo_scale;
    let wet_roughness = (roughness + WET_ROUGHNESS_DELTA * gauge).max(0.0);
    (wet_albedo, wet_roughness)
}

/// Rain streak length in meters for a fall of `speed_m_s` at `intensity`: the
/// stretched quad reuses the particle pool's [`STREAK_GAIN`].
#[must_use]
pub fn rain_streak_len(speed_m_s: f32, intensity: f32) -> f32 {
    STREAK_GAIN * speed_m_s.abs() * intensity.clamp(0.0, 1.0)
}

/// The pass resolution a tier's frame takes: half res, or a third on potato.
#[must_use]
pub fn pass_cells(frame_width: u32, frame_height: u32, potato: bool) -> (u32, u32) {
    let divisor = if potato {
        POTATO_RES_DIVISOR
    } else {
        HALF_RES_DIVISOR
    };
    (
        (frame_width / divisor).max(1),
        (frame_height / divisor).max(1),
    )
}

/// Taps per pixel for a tier (the row's 25 half-res, 7 potato).
#[must_use]
pub const fn tier_taps(potato: bool) -> u32 {
    if potato {
        POTATO_TAPS
    } else {
        HALF_RES_TAPS
    }
}

/// The WGSL twin of the ring field; the pass is shipped but not yet bound into
/// the forward graph (that wiring is SW142 step 2 in the arena graph).
pub const WEATHER_WGSL: &str = include_str!("shaders/weather.wgsl");

/// The world cell a coordinate falls in.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the coordinate is clamped to a sane world range before the cast"
)]
fn cell_coord(world_m: f32) -> i32 {
    (world_m / RIPPLE_CELL_M).floor().clamp(-1.0e6, 1.0e6) as i32
}

/// A cell's integer bits, so a hash never needs a float-to-int cast.
fn cell_bits(cell: i32) -> u32 {
    u32::from_ne_bytes(cell.to_ne_bytes())
}

/// The drop's own centre inside its cell, from the donor's `hash22`.
fn drop_centre(cell_x: i32, cell_z: i32) -> [f32; 2] {
    [
        (signed_of(cell_x) + unit(hash(0x1, cell_x, cell_z))) * RIPPLE_CELL_M,
        (signed_of(cell_z) + unit(hash(0x2, cell_x, cell_z))) * RIPPLE_CELL_M,
    ]
}

/// The ring's age in `0..1` at `tick`: the cell's phase staggers the drops so
/// a whole field never pulses in step.
fn drop_age(cell_x: i32, cell_z: i32, tick: u64) -> f32 {
    let phase = RING_MIN_AGE + (1.0 - RING_MIN_AGE) * unit(hash(0x3, cell_x, cell_z));
    let cycle = f32::from(u16::try_from(tick % u64::from(RIPPLE_TICKS)).unwrap_or(0))
        / f32::from(u16::try_from(RIPPLE_TICKS).unwrap_or(1));
    (cycle + phase).fract().max(RING_MIN_AGE)
}

/// A cell index as a float: cells are clamped to i16 range (a 16 km patch), so
/// the conversion is exact and needs no cast.
fn signed_of(cell: i32) -> f32 {
    f32::from(i16::try_from(cell.clamp(-32_000, 32_000)).unwrap_or(0))
}

/// One integer hash over a salt and a cell (`hash12`'s job in the donor).
fn hash(salt: u32, cell_x: i32, cell_z: i32) -> u32 {
    let mut h = cell_bits(cell_x).wrapping_mul(0x9E37_79B9)
        ^ cell_bits(cell_z).wrapping_mul(0x85EB_CA6B)
        ^ salt;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h
}

/// A hash word as `0.0..=1.0`.
fn unit(word: u32) -> f32 {
    f32::from(u16::try_from(word >> 16).unwrap_or(0)) / 65_535.0
}

#[cfg(test)]
#[path = "weather_tests.rs"]
mod tests;

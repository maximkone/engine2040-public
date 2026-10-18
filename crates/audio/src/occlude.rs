//! occlude: wall-muffle gains and lowpass cutoffs per voice per tick.
//!
//! Owner: audio lane; consumer: the game arena mix via wall segments (the
//! occluder set is per-tick input data). Units: integer
//! cm for positions, Q16 (ONE = 65536) for gains, Hz for cutoffs, ticks at
//! 60 Hz for time. A wall passes less level and darkens the sound: the cutoff
//! is the lowest of the crossed walls' material cutoffs and the air cutoff
//! of the path length ([`Muffle`]); the mixer applies it as a one-pole
//! lowpass ([`crate::Mixer::set_voice_lowpass`]).
//! Determinism: integer-only query plus slew; same inputs give the same gain
//! curve (presentation only, never hashed; golden d57b5c8ae3096614 stands).
//! Errors: typed OccludeError; the per-tick path clamps, setters refuse.
//! Budget: 0.1 ms per voice per tick (see BUDGET_MS_PER_VOICE).

use std::fmt::{Display, Formatter, Result as FmtResult};

/// Ticks per second the game thread runs the query (matches the fixed tick).
/// Reads the single owner's checked narrow form (`engine_core::TICK_HZ_U32`):
/// a bare `as u32` on the `u64` rate would truncate silently.
pub const TICK_HZ: u32 = engine_core::TICK_HZ_U32;
/// One tick in microseconds, rounded (1_000_000 / 60).
pub const TICK_US: u32 = 16_667;
/// Slew down (ducking behind a wall) time in ms: muffles without pumping.
pub const ATTACK_MS: u32 = 120;
/// Slew up (emerging from cover) time in ms: slower, so tails never jump.
pub const RELEASE_MS: u32 = 300;
/// Q16 full gain: every gain in this module is an i32 in 0..=GAIN_ONE.
pub const GAIN_ONE: i32 = 65_536;
/// Gain fall per tick: full swing in ceil(65536 / 9102) = 8 ticks (~120 ms).
pub const ATTACK_STEP: i32 = GAIN_ONE * (TICK_US as i32) / (ATTACK_MS as i32 * 1000);
/// Gain rise per tick: full swing in ceil(65536 / 3640) = 19 ticks (~300 ms).
pub const RELEASE_STEP: i32 = GAIN_ONE * (TICK_US as i32) / (RELEASE_MS as i32 * 1000);
/// Cutoff of open air at the ears, Hz: at or above it the mixer filter is
/// bypassed (the top of the audible band).
pub const OPEN_CUTOFF_HZ: u32 = 20_000;
/// Air absorption: cutoff lost per meter of path, Hz.
pub const AIR_LOSS_HZ_PER_M: u32 = 150;
/// Lowest cutoff air alone leaves, Hz (a path of about 107 m).
pub const AIR_FLOOR_HZ: u32 = 4_000;
/// Cutoff fall per tick: a full swing in 8 ticks, same as the gain attack.
pub const CUTOFF_FALL_STEP_HZ: u32 = OPEN_CUTOFF_HZ * TICK_US / (ATTACK_MS * 1000);
/// Cutoff rise per tick: a full swing in 19 ticks, same as the gain release.
pub const CUTOFF_RISE_STEP_HZ: u32 = OPEN_CUTOFF_HZ * TICK_US / (RELEASE_MS * 1000);
/// Per-voice per-tick budget in ms at 60 Hz, game thread, release profile;
/// the timing probe runs under `--perf`, never by default.
pub const BUDGET_MS_PER_VOICE: f32 = 0.1;
/// Largest occluder set one query walks; bounds the per-tick cost.
pub const MAX_WALLS_PER_QUERY: usize = 256;

/// Sand absorption in Q16 (0.2): a sand wall passes 0.8 (-1.94 dB).
pub const SAND_ABSORB_Q16: i32 = 13_107;
/// Wood absorption in Q16 (0.4): a wood wall passes 0.6 (-4.44 dB).
pub const WOOD_ABSORB_Q16: i32 = 26_214;
/// Stone absorption in Q16 (0.6): a stone wall passes 0.4 (-7.96 dB).
pub const STONE_ABSORB_Q16: i32 = 39_322;
/// Cloth absorption in Q16 (0.8): a cloth wall passes 0.2 (-13.98 dB).
pub const CLOTH_ABSORB_Q16: i32 = 52_429;
/// Voice-bus floor in Q16 (-12 dB): taunts stay intelligible behind cloth,
/// which would otherwise duck them to -13.98 dB.
pub const VOICE_FLOOR_Q16: i32 = 16_462;

/// Arena wall material. Absorption values are the plan's per-material table
/// (sand 0.2, wood 0.4, stone 0.6, cloth 0.8); each crossed wall multiplies
/// the running gain by what the material passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// Packed sand: passes 0.8.
    Sand,
    /// Timber boards: passes 0.6.
    Wood,
    /// Arena stone: passes 0.4.
    Stone,
    /// Heavy drapes: passes 0.2.
    Cloth,
}

/// One occluder: a floor-plan segment in integer cm plus its material.
/// The caller (game-side wall wiring) passes the live set per tick; this
/// module never queries a spatial index itself and holds no dependency on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wall {
    /// Segment start, east offset in cm.
    pub ax_cm: i32,
    /// Segment start, south offset in cm.
    pub az_cm: i32,
    /// Segment end, east offset in cm.
    pub bx_cm: i32,
    /// Segment end, south offset in cm.
    pub bz_cm: i32,
    /// What the wall is made of.
    pub material: Material,
}

impl Wall {
    /// Build one occluder from integer-cm endpoints and a material.
    /// Any i32 endpoints are accepted; a zero-length wall (a point)
    /// contributes nothing (see [`target_gain_q16`]).
    pub fn new(ax_cm: i32, az_cm: i32, bx_cm: i32, bz_cm: i32, material: Material) -> Self {
        Self {
            ax_cm,
            az_cm,
            bx_cm,
            bz_cm,
            material,
        }
    }
}

/// Typed occlusion failure. The per-tick query returns these; it never
/// panics on caller data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccludeError {
    /// Gain outside `0..=GAIN_ONE`.
    BadGain {
        /// Rejected Q16 value.
        got: i32,
    },
    /// Occluder set larger than [`MAX_WALLS_PER_QUERY`].
    TooManyWalls {
        /// Rejected wall count.
        got: usize,
    },
}

impl Display for OccludeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::BadGain { got } => write!(f, "occlude: bad gain {got}"),
            Self::TooManyWalls { got } => write!(f, "occlude: too many walls {got}"),
        }
    }
}

impl std::error::Error for OccludeError {}

/// Q16 absorption of a material (0 = open air, GAIN_ONE = silence).
pub fn absorption_q16(material: Material) -> i32 {
    match material {
        Material::Sand => SAND_ABSORB_Q16,
        Material::Wood => WOOD_ABSORB_Q16,
        Material::Stone => STONE_ABSORB_Q16,
        Material::Cloth => CLOTH_ABSORB_Q16,
    }
}

/// Lowpass cutoff in Hz a wall of `material` leaves: hard walls keep less of
/// the high band than soft ones keep of the low band (stone 1.5 kHz, cloth
/// 700 Hz; with the gain chart a stone wall drops 4 kHz by about 20 dB).
pub fn cutoff_hz(material: Material) -> u32 {
    match material {
        Material::Sand => 8_000,
        Material::Wood => 4_000,
        Material::Stone => 1_500,
        Material::Cloth => 700,
    }
}

/// Cutoff in Hz that air alone leaves over a path of `distance_cm`.
pub fn air_cutoff_hz(distance_cm: u64) -> u32 {
    let loss = (distance_cm / 100).saturating_mul(u64::from(AIR_LOSS_HZ_PER_M));
    match u32::try_from(loss) {
        Ok(loss) => OPEN_CUTOFF_HZ.saturating_sub(loss).max(AIR_FLOOR_HZ),
        Err(_) => AIR_FLOOR_HZ,
    }
}

/// Q16 share a material passes: `GAIN_ONE - absorption`.
pub fn transmission_q16(material: Material) -> i32 {
    GAIN_ONE - absorption_q16(material)
}

/// Push `gain_q16` through one wall of `material` in fixed point:
/// `(gain * pass) >> 16`. Inputs clamp into range; no float math.
///
/// Wall-muffle chart (Q16 gain, level): open 65536 at 0.00 dB, sand 52429
/// at -1.94 dB, wood 39322 at -4.44 dB, stone 26214 at -7.96 dB, cloth
/// 13107 at -13.98 dB, two stone walls 10485 at -15.92 dB.
pub fn apply_wall(gain_q16: i32, material: Material) -> i32 {
    let gain = i64::from(gain_q16.clamp(0, GAIN_ONE));
    let pass = i64::from(transmission_q16(material));
    ((gain * pass) >> 16) as i32
}

/// Keep `gain_q16` at or above `floor_q16` (both clamp into range).
/// The voice bus passes [`VOICE_FLOOR_Q16`]; sfx passes 0.
pub fn apply_floor(gain_q16: i32, floor_q16: i32) -> i32 {
    gain_q16
        .clamp(0, GAIN_ONE)
        .max(floor_q16.clamp(0, GAIN_ONE))
}

/// Gain as a float in `0.0..=1.0` for the mixer call. Boundary helper only:
/// the query and the slew stay in Q16. Out-of-range input clamps.
pub fn q16_to_gain(gain_q16: i32) -> f32 {
    gain_q16.clamp(0, GAIN_ONE) as f32 / GAIN_ONE as f32
}

fn orient(a: [i128; 2], b: [i128; 2], c: [i128; 2]) -> i128 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn on_segment(a: [i128; 2], b: [i128; 2], c: [i128; 2]) -> bool {
    c[0] >= a[0].min(b[0])
        && c[0] <= a[0].max(b[0])
        && c[1] >= a[1].min(b[1])
        && c[1] <= a[1].max(b[1])
}

fn point(cm: [i32; 2]) -> [i128; 2] {
    [i128::from(cm[0]), i128::from(cm[1])]
}

/// True when segment `p->q` touches segment `r->s`, endpoints included.
/// Integer orientation in i128 (never overflows on i32 input); touching or
/// collinear-overlapping counts as blocked, parallel-disjoint does not.
pub fn segments_cross(p: [i32; 2], q: [i32; 2], r: [i32; 2], s: [i32; 2]) -> bool {
    let (p, q, r, s) = (point(p), point(q), point(r), point(s));
    let (d1, d2) = (orient(r, s, p), orient(r, s, q));
    let (d3, d4) = (orient(p, q, r), orient(p, q, s));
    if ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0)) {
        return true;
    }
    if d1 == 0 && on_segment(r, s, p) {
        return true;
    }
    if d2 == 0 && on_segment(r, s, q) {
        return true;
    }
    if d3 == 0 && on_segment(p, q, r) {
        return true;
    }
    if d4 == 0 && on_segment(p, q, s) {
        return true;
    }
    false
}

/// Muffled target for a voice at `voice_cm` heard at `listener_cm`
/// (integer-cm floor plane, `[x east, z south]`) against `walls`.
/// Each crossed wall multiplies the running gain (slice order cannot change
/// the product). Zero-length walls contribute nothing; a voice at the
/// listener's ears is unoccluded. Same inputs give the same gain on every
/// call, so replays chart the same curve.
///
/// Errors: [`OccludeError::TooManyWalls`] when the set is larger than
/// [`MAX_WALLS_PER_QUERY`].
pub fn target_gain_q16(
    voice_cm: [i32; 2],
    listener_cm: [i32; 2],
    walls: &[Wall],
) -> Result<i32, OccludeError> {
    Ok(target_muffle(voice_cm, listener_cm, walls)?.gain_q16)
}

/// What the path from a voice to the listener leaves of it: level and
/// brightness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Muffle {
    /// Level in Q16 (`GAIN_ONE` open).
    pub gain_q16: i32,
    /// Lowpass cutoff in Hz ([`OPEN_CUTOFF_HZ`] open).
    pub cutoff_hz: u32,
}

impl Muffle {
    /// Nothing between voice and ears.
    pub const OPEN: Self = Self {
        gain_q16: GAIN_ONE,
        cutoff_hz: OPEN_CUTOFF_HZ,
    };
}

/// [`target_gain_q16`] plus the cutoff: the gain is the same product, the
/// cutoff the lowest of the air cutoff for the path length and the cutoff of
/// every crossed wall (the darkest wall wins; walls do not stack their
/// filters). Same errors and the same degenerate cases as the gain query.
///
/// Errors: [`OccludeError::TooManyWalls`] when the set is larger than
/// [`MAX_WALLS_PER_QUERY`].
pub fn target_muffle(
    voice_cm: [i32; 2],
    listener_cm: [i32; 2],
    walls: &[Wall],
) -> Result<Muffle, OccludeError> {
    if walls.len() > MAX_WALLS_PER_QUERY {
        return Err(OccludeError::TooManyWalls { got: walls.len() });
    }
    if voice_cm == listener_cm {
        return Ok(Muffle::OPEN);
    }
    let dx = i64::from(voice_cm[0]) - i64::from(listener_cm[0]);
    let dz = i64::from(voice_cm[1]) - i64::from(listener_cm[1]);
    // Saturating: a path past the i64 square (billions of cm) is far beyond
    // the air floor anyway.
    let squared = dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz));
    let distance_cm = crate::isqrt(squared.unsigned_abs());
    let mut cutoff = air_cutoff_hz(distance_cm);
    let mut gain = GAIN_ONE;
    for wall in walls {
        if wall.ax_cm == wall.bx_cm && wall.az_cm == wall.bz_cm {
            continue;
        }
        if segments_cross(
            voice_cm,
            listener_cm,
            [wall.ax_cm, wall.az_cm],
            [wall.bx_cm, wall.bz_cm],
        ) {
            gain = apply_wall(gain, wall.material);
            cutoff = cutoff.min(cutoff_hz(wall.material));
            if gain == 0 {
                break;
            }
        }
    }
    Ok(Muffle {
        gain_q16: gain,
        cutoff_hz: cutoff,
    })
}

/// Per-voice muffle state: the query target plus the smoothed gain and
/// cutoff the mixer reads. Build one per voice on the game thread, call
/// [`update_muffle`](Self::update_muffle) once per tick, hand the result to
/// the mixer. Borrows the wall slice and keeps scalars only, so the
/// steady-state tick uses no heap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceOcclusion {
    current_q16: i32,
    cutoff_hz: u32,
}

impl VoiceOcclusion {
    /// New voice, fully open.
    pub fn new_open() -> Self {
        Self {
            current_q16: GAIN_ONE,
            cutoff_hz: OPEN_CUTOFF_HZ,
        }
    }

    /// Smoothed lowpass cutoff in Hz.
    pub fn current_cutoff_hz(&self) -> u32 {
        self.cutoff_hz
    }

    /// Smoothed gain in Q16.
    pub fn current_q16(&self) -> i32 {
        self.current_q16
    }

    /// Smoothed gain as a float in `0.0..=1.0` for the mixer call.
    pub fn current_gain(&self) -> f32 {
        q16_to_gain(self.current_q16)
    }

    /// Set the smoothed gain directly (seek, zone teleport). Refuses
    /// out-of-range values loudly instead of clamping.
    ///
    /// Errors: [`OccludeError::BadGain`] when outside `0..=GAIN_ONE`.
    pub fn set_q16(&mut self, gain_q16: i32) -> Result<(), OccludeError> {
        if !(0..=GAIN_ONE).contains(&gain_q16) {
            return Err(OccludeError::BadGain { got: gain_q16 });
        }
        self.current_q16 = gain_q16;
        Ok(())
    }

    /// Back to fully open without touching the heap.
    pub fn reset_open(&mut self) {
        self.current_q16 = GAIN_ONE;
        self.cutoff_hz = OPEN_CUTOFF_HZ;
    }

    /// Slew toward `target_q16` one tick: falling uses [`ATTACK_STEP`]
    /// (120 ms), rising uses [`RELEASE_STEP`] (300 ms), arrival clamps
    /// exactly with no overshoot. Out-of-range targets clamp; the tick
    /// never refuses.
    pub fn tick(&mut self, target_q16: i32) -> i32 {
        let target = target_q16.clamp(0, GAIN_ONE);
        if target < self.current_q16 {
            self.current_q16 = (self.current_q16 - ATTACK_STEP).max(target);
        } else if target > self.current_q16 {
            self.current_q16 = (self.current_q16 + RELEASE_STEP).min(target);
        }
        self.current_q16
    }

    /// Whole per-tick step: query the walls, then slew. Returns the
    /// smoothed Q16 gain. Borrows only; uses no heap.
    ///
    /// Errors: [`OccludeError::TooManyWalls`] from the query; the slew
    /// state is untouched when the query refuses.
    pub fn update(
        &mut self,
        voice_cm: [i32; 2],
        listener_cm: [i32; 2],
        walls: &[Wall],
    ) -> Result<i32, OccludeError> {
        Ok(self.update_muffle(voice_cm, listener_cm, walls)?.gain_q16)
    }

    /// [`update`](Self::update) with the cutoff: the gain slews as there, the
    /// cutoff falls by [`CUTOFF_FALL_STEP_HZ`] and rises by
    /// [`CUTOFF_RISE_STEP_HZ`] per tick and arrives exactly. Returns the
    /// smoothed pair. Borrows only; uses no heap.
    ///
    /// Errors: [`OccludeError::TooManyWalls`] from the query; the state is
    /// untouched when the query refuses.
    pub fn update_muffle(
        &mut self,
        voice_cm: [i32; 2],
        listener_cm: [i32; 2],
        walls: &[Wall],
    ) -> Result<Muffle, OccludeError> {
        let target = target_muffle(voice_cm, listener_cm, walls)?;
        let gain_q16 = self.tick(target.gain_q16);
        self.cutoff_hz = if target.cutoff_hz < self.cutoff_hz {
            self.cutoff_hz
                .saturating_sub(CUTOFF_FALL_STEP_HZ)
                .max(target.cutoff_hz)
        } else {
            self.cutoff_hz
                .saturating_add(CUTOFF_RISE_STEP_HZ)
                .min(target.cutoff_hz)
        };
        Ok(Muffle {
            gain_q16,
            cutoff_hz: self.cutoff_hz,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The const reads the checked narrow owner and widens back to the one
    /// `u64` rate: 60 Hz both ways, with no `as u32` at this call site.
    #[test]
    fn tick_hz_keeps_the_core_owner_u64() {
        let rate: u64 = u64::from(TICK_HZ);
        assert_eq!(rate, engine_core::TICK_HZ);
    }

    fn stone_cross_wall() -> Wall {
        // Voice at (500, 0), listener at origin: this stone wall at x = 250
        // spans z -500..500, so the straight path crosses it dead center.
        Wall::new(250, -500, 250, 500, Material::Stone)
    }

    #[test]
    fn open_gain_is_one_without_walls() {
        let walls: &[Wall] = &[];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], walls).expect("empty set queries"),
            GAIN_ONE
        );
    }

    #[test]
    fn stone_wall_muffles_to_charted_gain() {
        // Charted fixture: one stone wall passes 0.4 -> 26214 at -7.96 dB.
        let walls = [stone_cross_wall()];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], &walls).expect("one wall queries"),
            26_214
        );
    }

    #[test]
    fn material_chart_orders_cloth_to_sand() {
        // Wall A/B pairs per arena material: same geometry, only the
        // material changes. Open stays loudest, cloth muffles hardest.
        let shape = |material| Wall::new(250, -500, 250, 500, material);
        let gain = |material| {
            target_gain_q16([500, 0], [0, 0], &[shape(material)]).expect("one wall queries")
        };
        let (sand, wood, stone, cloth) = (
            gain(Material::Sand),
            gain(Material::Wood),
            gain(Material::Stone),
            gain(Material::Cloth),
        );
        assert_eq!(sand, 52_429, "sand passes 0.8 (-1.94 dB)");
        assert_eq!(wood, 39_322, "wood passes 0.6 (-4.44 dB)");
        assert_eq!(stone, 26_214, "stone passes 0.4 (-7.96 dB)");
        assert_eq!(cloth, 13_107, "cloth passes 0.2 (-13.98 dB)");
        assert!(cloth < stone && stone < wood && wood < sand && sand < GAIN_ONE);
    }

    #[test]
    fn misses_and_degenerate_shapes_stay_open() {
        // Wall far south of the path: parallel, never crossed.
        let south = [Wall::new(250, 500, 750, 600, Material::Stone)];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], &south).expect("miss queries"),
            GAIN_ONE
        );
        // Wall behind the voice: the segment ends before it starts.
        let behind = [Wall::new(750, -500, 750, 500, Material::Stone)];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], &behind).expect("behind queries"),
            GAIN_ONE
        );
        // Zero-length wall exactly on the path: a point blocks nothing.
        let point = [Wall::new(250, 0, 250, 0, Material::Stone)];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], &point).expect("point queries"),
            GAIN_ONE
        );
        // Voice at the listener's ears: unoccluded even walled in.
        let boxed = [
            Wall::new(-100, -100, 100, -100, Material::Stone),
            Wall::new(-100, 100, 100, 100, Material::Stone),
        ];
        assert_eq!(
            target_gain_q16([0, 0], [0, 0], &boxed).expect("coincident queries"),
            GAIN_ONE
        );
    }

    #[test]
    fn touching_endpoint_counts_as_blocked() {
        // Wall corner exactly on the path: inclusive touch still muffles.
        let touch = [Wall::new(250, 0, 250, 500, Material::Stone)];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], &touch).expect("touch queries"),
            26_214
        );
    }

    #[test]
    fn stacked_walls_multiply() {
        // Two stone walls in a row: 0.4 x 0.4 = 0.16 -> 10485 (-15.92 dB).
        let walls = [
            Wall::new(150, -500, 150, 500, Material::Stone),
            Wall::new(350, -500, 350, 500, Material::Stone),
        ];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], &walls).expect("two walls query"),
            10_485
        );
    }

    #[test]
    fn segments_cross_basics() {
        assert!(segments_cross([0, 0], [4, 4], [0, 4], [4, 0]));
        assert!(!segments_cross([0, 0], [4, 0], [0, 2], [4, 2]));
        assert!(segments_cross([0, 0], [4, 0], [2, 0], [6, 0]));
        assert!(!segments_cross([0, 0], [2, 0], [4, 0], [6, 0]));
        assert!(!segments_cross([0, 0], [1, 0], [5, 5], [6, 6]));
    }

    #[test]
    fn voice_floor_keeps_cloth_intelligible() {
        let cloth = target_gain_q16(
            [500, 0],
            [0, 0],
            &[Wall::new(250, -500, 250, 500, Material::Cloth)],
        )
        .expect("cloth queries");
        assert_eq!(cloth, 13_107);
        assert_eq!(
            apply_floor(cloth, VOICE_FLOOR_Q16),
            VOICE_FLOOR_Q16,
            "voice bus never ducks past -12 dB"
        );
        assert_eq!(VOICE_FLOOR_Q16, 16_462);
        let stone =
            target_gain_q16([500, 0], [0, 0], &[stone_cross_wall()]).expect("stone queries");
        assert_eq!(
            apply_floor(stone, VOICE_FLOOR_Q16),
            stone,
            "gains above the floor pass through"
        );
        assert_eq!(
            apply_floor(cloth, 0),
            cloth,
            "sfx floor 0 keeps the raw gain"
        );
    }

    #[test]
    fn smoothing_attack_is_faster_than_release() {
        // Ducking behind stone: first tick falls by ATTACK_STEP, arrival in
        // at most 8 ticks, every step falls or holds. These pins catch a
        // swapped attack/release (the pumping bug) rather than restating
        // the constants: direction first, counts second.
        let mut occ = VoiceOcclusion::new_open();
        let first = occ.tick(0);
        assert_eq!(first, GAIN_ONE - ATTACK_STEP);
        let mut prev = first;
        let mut attack_ticks = 1u32;
        while occ.current_q16() > 0 {
            let now = occ.tick(0);
            assert!(now <= prev, "slew down never rises");
            assert!(now >= 0, "slew down never overshoots");
            prev = now;
            attack_ticks += 1;
            assert!(attack_ticks <= 64, "attack must converge");
        }
        assert!(attack_ticks <= 8, "120 ms at 60 Hz lands in <= 8 ticks");
        // Emerging: first tick rises by RELEASE_STEP, arrival in <= 19.
        let first_up = occ.tick(GAIN_ONE);
        assert_eq!(first_up, RELEASE_STEP);
        let mut prev_up = first_up;
        let mut release_ticks = 1u32;
        while occ.current_q16() < GAIN_ONE {
            let now = occ.tick(GAIN_ONE);
            assert!(now >= prev_up, "slew up never falls");
            assert!(now <= GAIN_ONE, "slew up never overshoots");
            prev_up = now;
            release_ticks += 1;
            assert!(release_ticks <= 64, "release must converge");
        }
        assert!(release_ticks <= 19, "300 ms at 60 Hz lands in <= 19 ticks");
        assert!(
            attack_ticks < release_ticks,
            "ducking ({attack_ticks}) beats emerging ({release_ticks})"
        );
    }

    #[test]
    fn replay_gives_identical_gain_curve() {
        // Scripted scene: open, stone wall ticks 30..90, open again. Same
        // events plus same geometry give the same gain curve twice.
        fn run_scene() -> Vec<i32> {
            let open: &[Wall] = &[];
            let walled = [stone_cross_wall()];
            let mut occ = VoiceOcclusion::new_open();
            let mut curve = Vec::with_capacity(120);
            for tick in 0..120 {
                let walls: &[Wall] = if (30..90).contains(&tick) {
                    &walled
                } else {
                    open
                };
                curve.push(
                    occ.update([500, 0], [0, 0], walls)
                        .expect("scene stays under the wall cap"),
                );
            }
            curve
        }
        let (first, second) = (run_scene(), run_scene());
        assert_eq!(first, second, "same script gives the same curve");
        assert_eq!(first[0], GAIN_ONE, "scene starts open");
        assert_eq!(first[60], 26_214, "fully muffled mid-wall");
        assert_eq!(first[119], GAIN_ONE, "release recovers by the end");
        assert!(
            first[30] < GAIN_ONE && first[30] > 26_214,
            "ducking slews, never snaps"
        );
        // Fresh struct, same script: identical again (no hidden state).
        assert_eq!(run_scene(), first, "no hidden state across voices");
    }

    #[test]
    fn invalid_input_refuses() {
        let mut occ = VoiceOcclusion::new_open();
        assert_eq!(occ.set_q16(-1), Err(OccludeError::BadGain { got: -1 }));
        assert_eq!(
            occ.set_q16(GAIN_ONE + 1),
            Err(OccludeError::BadGain { got: GAIN_ONE + 1 })
        );
        assert!(occ.set_q16(0).is_ok());
        assert!(occ.set_q16(GAIN_ONE).is_ok());
        let too_many = vec![stone_cross_wall(); MAX_WALLS_PER_QUERY + 1];
        assert_eq!(
            target_gain_q16([500, 0], [0, 0], &too_many),
            Err(OccludeError::TooManyWalls {
                got: MAX_WALLS_PER_QUERY + 1
            })
        );
        let capped = vec![stone_cross_wall(); MAX_WALLS_PER_QUERY];
        assert!(
            target_gain_q16([500, 0], [0, 0], &capped).is_ok(),
            "exactly the cap still queries"
        );
        assert_eq!(
            format!("{}", OccludeError::TooManyWalls { got: 999 }),
            "occlude: too many walls 999"
        );
    }

    #[test]
    fn gain_boundary_clamps() {
        assert_eq!(q16_to_gain(0), 0.0);
        assert_eq!(q16_to_gain(GAIN_ONE), 1.0);
        assert_eq!(q16_to_gain(-100), 0.0);
        assert_eq!(q16_to_gain(GAIN_ONE + 100), 1.0);
        let mut occ = VoiceOcclusion::new_open();
        assert_eq!(occ.current_gain(), 1.0);
        occ.tick(-50);
        assert_eq!(occ.current_q16(), GAIN_ONE - ATTACK_STEP);
        occ.reset_open();
        assert_eq!(occ.current_q16(), GAIN_ONE);
    }

    #[test]
    fn hot_path_names_no_heap_types() {
        // Structural guard beside the allocator window below: the query
        // plus slew above the cfg(test) marker must not name heap or
        // failure-injection APIs, so a per-voice-per-tick container cannot
        // land unnoticed even if the window ever goes quiet.
        const SRC: &str = include_str!("occlude.rs");
        let head = SRC
            .split("#[cfg(test)]")
            .next()
            .expect("occlude.rs keeps its cfg(test) marker");
        for token in [
            "Vec",
            "Box<",
            "Box::",
            "String",
            "vec!",
            "format!",
            "format(",
            "collect",
            "to_vec",
            "HashMap",
            "BTreeMap",
            "println",
            "print!",
            "eprintln",
            "dbg!",
            "Rc<",
            "Arc<",
            "alloc",
            "Cell<",
            "RefCell",
            "Mutex",
            "RwLock",
            "unreachable!",
            "unimplemented!",
            "todo!",
            "panic!",
            "unwrap",
            "expect(",
        ] {
            assert!(
                !head.contains(token),
                "hot path must not name heap/panic API: {token}"
            );
        }
    }

    #[test]
    fn steady_state_update_allocates_nothing() {
        use engine_core::alloc_probe::measure;
        // 32-wall street scene (same shape as the timing probe): one voice,
        // 600 ticks through query plus slew inside the counting window.
        let mut walls = [Wall::new(0, 0, 0, 0, Material::Stone); 32];
        for (i, wall) in walls.iter_mut().enumerate() {
            let x = 100 + (i as i32) * 25;
            *wall = Wall::new(
                x,
                -600,
                x,
                600,
                match i % 4 {
                    0 => Material::Sand,
                    1 => Material::Wood,
                    2 => Material::Stone,
                    _ => Material::Cloth,
                },
            );
        }
        let mut occ = VoiceOcclusion::new_open();
        for _ in 0..16 {
            occ.update([900, 40], [0, 0], &walls)
                .expect("warm-up queries outside the window");
        }
        let (folded, heap) = measure(|| {
            let mut folded = 0i32;
            for _ in 0..600 {
                folded ^= occ
                    .update([900, 40], [0, 0], &walls)
                    .expect("tick queries inside the window");
            }
            folded
        });
        std::hint::black_box(folded);
        assert_eq!(heap.calls, 0, "steady-state occlusion must not allocate");
        assert!(
            occ.current_q16() < GAIN_ONE,
            "sanity: the stone walls muffled the voice during the window"
        );
    }

    #[test]
    #[ignore = "timing: 0.1 ms/voice budget probe, run under --perf in release"]
    fn budget_probe_per_voice_tick() {
        // 32-wall street scene, one voice, 4000 ticks: mean update time
        // must stay under BUDGET_MS_PER_VOICE. Wall clock lives only in
        // this ignored probe, never in the query path.
        let mut walls = [Wall::new(0, 0, 0, 0, Material::Stone); 32];
        for (i, wall) in walls.iter_mut().enumerate() {
            let x = 100 + (i as i32) * 25;
            *wall = Wall::new(
                x,
                -600,
                x,
                600,
                match i % 4 {
                    0 => Material::Sand,
                    1 => Material::Wood,
                    2 => Material::Stone,
                    _ => Material::Cloth,
                },
            );
        }
        let mut occ = VoiceOcclusion::new_open();
        occ.update([900, 40], [0, 0], &walls)
            .expect("32 walls stay under the cap");
        let start = std::time::Instant::now();
        let rounds = 4000u32;
        let mut folded = 0i32;
        for _ in 0..rounds {
            let gain = occ
                .update([900, 40], [0, 0], &walls)
                .expect("32 walls stay under the cap");
            folded ^= gain;
        }
        std::hint::black_box(folded);
        let mean_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
        assert!(
            mean_ms < BUDGET_MS_PER_VOICE as f64,
            "mean {mean_ms} ms per voice tick over budget"
        );
    }

    /// RMS (second half, after the filter settles) of one second of a sine at
    /// `hz` played at the muffle's level and cutoff, plus the heap calls the
    /// render made. `None` plays it open: no gain change, no filter.
    fn band_rms(hz: f32, muffle: Option<Muffle>) -> (f64, u64) {
        const RATE: u32 = 48_000;
        let frames = usize::try_from(RATE).expect("rate fits");
        let samples: Vec<f32> = (0..RATE)
            .map(|i| {
                (std::f32::consts::TAU * hz * crate::f32_of(i) / crate::f32_of(RATE)).sin() * 0.5
            })
            .collect();
        let mut mixer = crate::Mixer::new(RATE);
        let gain = muffle.map_or(1.0, |m| q16_to_gain(m.gain_q16));
        let id = mixer.play(&crate::Pcm::new(RATE, 1, samples), gain);
        if let Some(m) = muffle {
            assert!(
                mixer.set_voice_lowpass(id, m.cutoff_hz),
                "the voice is live"
            );
        }
        let mut out = vec![0.0f32; frames];
        let ((), heap) = engine_core::alloc_probe::measure(|| mixer.render_into(&mut out));
        let tail = &out[frames / 2..];
        let sum: f64 = tail.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        (
            (sum / f64::from(u32::try_from(tail.len()).expect("one second"))).sqrt(),
            heap.calls,
        )
    }

    fn drop_db(hz: f32, muffle: Muffle) -> f64 {
        let (open, _) = band_rms(hz, None);
        let (muffled, heap_calls) = band_rms(hz, Some(muffle));
        assert_eq!(heap_calls, 0, "the filtered render allocates nothing");
        20.0 * (open / muffled).log10()
    }

    #[test]
    fn muffle_band_behind_stone_drops_the_highs_and_keeps_the_lows() {
        let stone = target_muffle([500, 0], [0, 0], &[stone_cross_wall()]).expect("one wall");
        assert_eq!(stone.gain_q16, 26_214);
        assert_eq!(stone.cutoff_hz, cutoff_hz(Material::Stone));
        let high = drop_db(6_000.0, stone);
        let low = drop_db(100.0, stone);
        assert!(
            high >= 15.0,
            "high band drops {high:.1} dB, want at least 15"
        );
        assert!(low <= 8.0, "low band drops {low:.1} dB, want at most 8");
    }

    #[test]
    fn cutoff_is_the_darkest_wall_and_air_never_goes_below_the_floor() {
        let open = target_muffle([500, 0], [0, 0], &[]).expect("empty set");
        assert_eq!(open.gain_q16, GAIN_ONE);
        assert_eq!(open.cutoff_hz, OPEN_CUTOFF_HZ - 5 * AIR_LOSS_HZ_PER_M);
        let behind_both = [
            Wall::new(150, -500, 150, 500, Material::Cloth),
            Wall::new(250, -500, 250, 500, Material::Stone),
        ];
        let both = target_muffle([500, 0], [0, 0], &behind_both).expect("two walls");
        assert_eq!(
            both.cutoff_hz,
            cutoff_hz(Material::Cloth),
            "cloth is darker"
        );
        assert_eq!(
            both.gain_q16,
            target_gain_q16([500, 0], [0, 0], &behind_both).unwrap()
        );
        let far = target_muffle([60_000, 0], [0, 0], &[]).expect("empty set");
        assert_eq!(far.cutoff_hz, AIR_FLOOR_HZ);
        assert_eq!(air_cutoff_hz(0), OPEN_CUTOFF_HZ);
        assert_eq!(
            target_muffle([0, 0], [0, 0], &behind_both).unwrap(),
            Muffle::OPEN
        );
    }

    #[test]
    fn cutoff_slews_down_and_up_and_arrives_exactly() {
        let walls = [stone_cross_wall()];
        let mut occ = VoiceOcclusion::new_open();
        let first = occ.update_muffle([500, 0], [0, 0], &walls).expect("query");
        assert_eq!(first.cutoff_hz, OPEN_CUTOFF_HZ - CUTOFF_FALL_STEP_HZ);
        for _ in 0..16 {
            occ.update_muffle([500, 0], [0, 0], &walls).expect("query");
        }
        assert_eq!(occ.current_cutoff_hz(), cutoff_hz(Material::Stone));
        let up = occ.update_muffle([500, 0], [0, 0], &[]).expect("query");
        assert_eq!(
            up.cutoff_hz,
            cutoff_hz(Material::Stone) + CUTOFF_RISE_STEP_HZ
        );
        for _ in 0..40 {
            occ.update_muffle([500, 0], [0, 0], &[]).expect("query");
        }
        assert_eq!(occ.current_cutoff_hz(), air_cutoff_hz(500));
        occ.reset_open();
        assert_eq!(occ.current_cutoff_hz(), OPEN_CUTOFF_HZ);
    }
}

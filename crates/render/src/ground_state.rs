//! ground_state: the planar map under the pit that remembers the fight.
//!
//! A blood or scuff mark is a circle of sand that keeps a fixed amount of
//! mass. The mass is quantised once, in whole fixed-point units, as the mark
//! enters the pool; from then on it only moves (pool slot -> map plane), so
//! no step of this module can lose it. That is the invariant the tests pin:
//! `stamped_units == pool_units + map_units` for every mark kind, at every
//! tick, whether the pool was full, the map edge clipped the disc, or a texel
//! saturated.
//!
//! The pool is a bounded ring of live marks: a mark lives its kind's lifetime
//! in ticks, fades from full to [`FADE_FLOOR_BP`] while it lives, and on its
//! last tick its mass is deposited into the map and the slot is freed. The
//! map is permanent (blood on sand dries, it does not vanish), so a long
//! fight leaves a stain that outlives the pool. When the pool is full a new
//! mark is merged into its nearest live slot instead of being dropped: mass
//! adds, the slot keeps its own birth tick, and `merged` counts the folds.
//!
//! Units: metres for positions in the 20 m pit, texels for the map, ticks at
//! 60 Hz, fixed-point mass units ([`MASS_UNITS_PER_MARK`] is one full mark),
//! basis points for intensity. Determinism: integer map, integer fade curve,
//! integer deposit order (rows top to bottom, then columns); the only float
//! step is the one-shot metre -> texel quantisation at stamp time, and the
//! same input bytes give the same map bytes. Std-only, no GPU, no clock, not
//! in a simulation hash. Caller: the arena records gore and footfall stamps
//! here; the forward floor pass will upload [`GroundState::texels`].

use std::fmt;

// ---------------------------------------------------------------------------
// Bounds. The map budget is fixed by the row: 1024 x 1024 texels x 4 bytes,
// doubled for the front/back pair the floor pass composites between.
// ---------------------------------------------------------------------------

/// Side of the square pit in metres: stamps outside it are refused.
pub const PIT_METERS: f32 = 20.0;
/// Side of the ground map in texels.
pub const MAP_DIM: usize = 1024;
/// Bytes per texel: one `u16` blood plane and one `u16` scuff plane.
pub const TEXEL_BYTES: usize = 4;
/// One map plane set, 1024 x 1024 x 4 = 4 194 304 B.
pub const MAP_BYTES: usize = MAP_DIM * MAP_DIM * TEXEL_BYTES;
/// The front and back maps the floor pass composites between (bytes x2).
pub const GROUND_BYTES: usize = MAP_BYTES * 2;
/// Fixed-point units in one full mark (one splash of blood, one scuff).
pub const MASS_UNITS_PER_MARK: u32 = 10_000;
/// Live mark slots; a full pool merges rather than drops.
pub const POOL_SLOTS: usize = 128;
/// Marks one tick may add to the pool.
pub const MAX_STAMPS_PER_TICK: u32 = 16;
/// Blood stays live in the pool for 600 ticks (10 s at 60 Hz).
pub const BLOOD_LIFETIME_TICKS: u32 = 600;
/// Sand scuffs outlive blood: 1200 ticks, then their mass settles too.
pub const SCUFF_LIFETIME_TICKS: u32 = 1_200;
/// Intensity a mark keeps at the end of its life, in basis points.
pub const FADE_FLOOR_BP: u32 = 3_500;
/// Full intensity in basis points.
pub const FULL_BP: u32 = 10_000;
/// Largest accepted stamp radius in metres (arena-scale guard).
pub const MAX_STAMP_RADIUS_M: f32 = 2.0;

// ---------------------------------------------------------------------------
// Types.
// ---------------------------------------------------------------------------

/// Which plane a mark lands in. The two kinds keep separate mass ledgers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Blood: dark, wet, 600-tick pool life.
    Blood,
    /// Scuff: a boot or a body dragged over sand, 1200-tick pool life.
    Scuff,
}

impl Mark {
    /// Plane index in the interleaved texel, and the ledger index.
    fn plane(self) -> usize {
        match self {
            Self::Blood => 0,
            Self::Scuff => 1,
        }
    }

    /// Ticks the mark stays live in the pool before its mass is deposited.
    pub fn lifetime_ticks(self) -> u32 {
        match self {
            Self::Blood => BLOOD_LIFETIME_TICKS,
            Self::Scuff => SCUFF_LIFETIME_TICKS,
        }
    }
}

/// One live mark in the pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    /// Blood or scuff.
    pub mark: Mark,
    /// Texel column of the disc centre.
    pub x: u32,
    /// Texel row of the disc centre.
    pub y: u32,
    /// Disc radius in texels, at least 1.
    pub radius: u32,
    /// Mass still held by this slot, in fixed-point units.
    pub units: u32,
    /// Tick the mark (or the first mark it absorbed) entered the pool.
    pub born_tick: u32,
    /// How many later marks were folded in when the pool was full.
    pub merged: u32,
}

impl Stamp {
    /// Ticks this slot has been live at `tick`.
    pub fn age(&self, tick: u32) -> u32 {
        tick.saturating_sub(self.born_tick)
    }

    /// Display intensity of a live slot at `tick`, in basis points.
    pub fn intensity_bp(&self, tick: u32) -> u32 {
        fade_bp(self.mark, self.age(tick))
    }

    /// True when the mark has served its lifetime and its mass settles.
    pub fn expired(&self, tick: u32) -> bool {
        self.age(tick) >= self.mark.lifetime_ticks()
    }
}

/// Intensity of a mark aged `age` ticks, in basis points: full at 0, the
/// fade floor from the lifetime on, strictly decreasing in between.
pub fn fade_bp(mark: Mark, age: u32) -> u32 {
    let life = mark.lifetime_ticks();
    if age >= life {
        return FADE_FLOOR_BP;
    }
    let span = u64::from(FULL_BP - FADE_FLOOR_BP);
    let left = u64::from(life - age);
    FADE_FLOOR_BP + u32::try_from(span * left / u64::from(life)).unwrap_or(0)
}

/// Why a stamp was refused. A refused stamp changed nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum GroundError {
    /// The centre is outside the pit.
    OutsidePit { x_m: f32, y_m: f32 },
    /// The radius is zero, negative or above [`MAX_STAMP_RADIUS_M`].
    InvalidRadius { radius_m: f32 },
    /// The mark carries no mass.
    ZeroMass,
    /// The tick already added [`MAX_STAMPS_PER_TICK`] marks.
    TickBudget,
}

impl fmt::Display for GroundError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutsidePit { x_m, y_m } => {
                write!(
                    f,
                    "stamp at ({x_m}, {y_m}) m is outside the {} m pit",
                    PIT_METERS
                )
            }
            Self::InvalidRadius { radius_m } => write!(
                f,
                "stamp radius {radius_m} m is outside (0, {MAX_STAMP_RADIUS_M}] m"
            ),
            Self::ZeroMass => write!(f, "a stamp must carry mass"),
            Self::TickBudget => write!(f, "the tick already added {MAX_STAMPS_PER_TICK} stamps"),
        }
    }
}

impl std::error::Error for GroundError {}

/// The ground-state map and its live mark pool.
#[derive(Debug)]
pub struct GroundState {
    /// `MAP_DIM * MAP_DIM * 2` `u16` texels: index `2 * i` is blood mass,
    /// `2 * i + 1` is scuff mass.
    texels: Vec<u16>,
    pool: Vec<Stamp>,
    tick: u32,
    stamps_this_tick: u32,
    /// Mass ever stamped, per plane.
    stamped: [u64; 2],
    /// Deposited mass the map had no room for, per plane; the next deposit
    /// of that plane spends it first, so mass is deferred, never dropped.
    pending: [u64; 2],
}

impl Default for GroundState {
    fn default() -> Self {
        Self::new()
    }
}

impl GroundState {
    /// An empty map at tick 0, with the pool preallocated.
    pub fn new() -> Self {
        Self {
            texels: vec![0; MAP_DIM * MAP_DIM * 2],
            pool: Vec::with_capacity(POOL_SLOTS),
            tick: 0,
            stamps_this_tick: 0,
            stamped: [0; 2],
            pending: [0; 2],
        }
    }

    /// Current tick.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Bytes the map holds; always [`MAP_BYTES`].
    pub fn bytes(&self) -> usize {
        self.texels.len() * std::mem::size_of::<u16>()
    }

    /// The interleaved texels the floor pass uploads (blood, scuff per texel).
    pub fn texels(&self) -> &[u16] {
        &self.texels
    }

    /// `(blood, scuff)` mass units of one texel, or `(0, 0)` off the map.
    pub fn texel(&self, x: usize, y: usize) -> (u16, u16) {
        if x >= MAP_DIM || y >= MAP_DIM {
            return (0, 0);
        }
        let i = (y * MAP_DIM + x) * 2;
        (self.texels[i], self.texels[i + 1])
    }

    /// Live mark slots.
    pub fn pool_len(&self) -> usize {
        self.pool.len()
    }

    /// The live slots in pool order (oldest first unless a merge reordered).
    pub fn pool(&self) -> &[Stamp] {
        &self.pool
    }

    /// Marks the tick has stamped so far.
    pub fn stamps_this_tick(&self) -> u32 {
        self.stamps_this_tick
    }

    /// Mass ever stamped for `mark`.
    pub fn stamped_units(&self, mark: Mark) -> u64 {
        self.stamped[mark.plane()]
    }

    /// Mass held by live slots for `mark`.
    pub fn pool_units(&self, mark: Mark) -> u64 {
        let plane = mark.plane();
        self.pool
            .iter()
            .filter(|stamp| stamp.mark.plane() == plane)
            .map(|stamp| u64::from(stamp.units))
            .sum()
    }

    /// Mass in the map for `mark`: the plane sum plus the deferred mass the
    /// next deposit will place.
    pub fn map_units(&self, mark: Mark) -> u64 {
        self.plane_units(mark) + self.pending[mark.plane()]
    }

    /// Mass the map plane itself holds for `mark`.
    pub fn plane_units(&self, mark: Mark) -> u64 {
        let plane = mark.plane();
        self.texels
            .iter()
            .skip(plane)
            .step_by(2)
            .map(|units| u64::from(*units))
            .sum()
    }

    /// Stamp one mark of `mass_units` fixed-point units at `(x_m, y_m)` m
    /// with radius `radius_m` m. Refused (with nothing changed) outside the
    /// pit, at a bad radius, at zero mass, or past the tick budget.
    pub fn stamp(
        &mut self,
        mark: Mark,
        x_m: f32,
        y_m: f32,
        radius_m: f32,
        mass_units: u32,
    ) -> Result<(), GroundError> {
        if !(0.0..=PIT_METERS).contains(&x_m) || !(0.0..=PIT_METERS).contains(&y_m) {
            return Err(GroundError::OutsidePit { x_m, y_m });
        }
        if !(radius_m > 0.0 && radius_m <= MAX_STAMP_RADIUS_M) {
            return Err(GroundError::InvalidRadius { radius_m });
        }
        if mass_units == 0 {
            return Err(GroundError::ZeroMass);
        }
        if self.stamps_this_tick >= MAX_STAMPS_PER_TICK {
            return Err(GroundError::TickBudget);
        }
        self.stamps_this_tick += 1;
        self.stamped[mark.plane()] += u64::from(mass_units);

        let fresh = Stamp {
            mark,
            x: texel_of(x_m),
            y: texel_of(y_m),
            radius: radius_texels(radius_m),
            units: mass_units,
            born_tick: self.tick,
            merged: 0,
        };
        self.place(fresh);
        Ok(())
    }

    /// Age the map by `ticks`: expired marks deposit their mass and free
    /// their slot, and every tick opens a fresh stamp budget.
    pub fn advance(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.tick = self.tick.saturating_add(1);
            self.stamps_this_tick = 0;
            self.settle();
        }
    }

    /// Put a live mark into the pool, merging into its nearest slot when the
    /// pool is full so the mass stays accounted. A pool full of the other
    /// mark kind settles its oldest slot early instead: mass reaches the map,
    /// the slot is freed, and no mark ever changes plane.
    fn place(&mut self, stamp: Stamp) {
        if self.pool.len() < POOL_SLOTS {
            self.pool.push(stamp);
            return;
        }
        // Nearest slot of the same kind wins; a tie goes to the lower index,
        // so the choice is fixed by pool order and not by iteration luck.
        let mut best: Option<(u64, usize)> = None;
        for (i, slot) in self.pool.iter().enumerate() {
            if slot.mark != stamp.mark {
                continue;
            }
            let dx = i64::from(slot.x) - i64::from(stamp.x);
            let dy = i64::from(slot.y) - i64::from(stamp.y);
            let d2 = u64::try_from(dx * dx + dy * dy).unwrap_or(u64::MAX);
            if best.is_none_or(|(was, _)| d2 < was) {
                best = Some((d2, i));
            }
        }
        if let Some((_, index)) = best {
            let slot = &mut self.pool[index];
            slot.units = slot.units.saturating_add(stamp.units);
            slot.merged = slot.merged.saturating_add(1);
            return;
        }
        // The pool holds only the other kind: settle its oldest slot early.
        let oldest = self
            .pool
            .iter()
            .enumerate()
            .min_by_key(|(index, slot)| (slot.born_tick, *index))
            .map(|(index, _)| index)
            .unwrap_or(0);
        let settled = self.pool.remove(oldest);
        self.deposit(settled);
        self.pool.push(stamp);
    }

    /// Deposit every expired slot into the map and remove it from the pool.
    fn settle(&mut self) {
        let tick = self.tick;
        let mut i = 0;
        while i < self.pool.len() {
            if self.pool[i].expired(tick) {
                let stamp = self.pool.remove(i);
                self.deposit(stamp);
            } else {
                i += 1;
            }
        }
    }

    /// Move a whole slot's mass into its plane.
    fn deposit(&mut self, stamp: Stamp) {
        let plane = stamp.mark.plane();
        let settled = self.pending[plane].saturating_add(u64::from(stamp.units));
        let units = u32::try_from(settled).unwrap_or(u32::MAX);
        self.pending[plane] = settled.saturating_sub(u64::from(units));
        let leftover = self.deposit_disc(plane, stamp.x, stamp.y, stamp.radius, units);
        self.pending[plane] = self.pending[plane].saturating_add(u64::from(leftover));
    }

    /// Spread `units` over the in-map texels of the disc, row by row. The
    /// remainder units go to the first texels of that order. Returns the
    /// units no texel could take (a saturated disc); the caller defers them.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "both bounds checks above each cast prove the row and column are inside 0 .. MAP_DIM, so the index is a valid usize"
    )]
    fn deposit_disc(&mut self, plane: usize, cx: u32, cy: u32, radius: u32, units: u32) -> u32 {
        let dim = MAP_DIM as i64;
        let r = i64::from(radius);
        let rr = r * r;
        let mut count = 0u32;
        for dy in -r..=r {
            let y = i64::from(cy) + dy;
            if y < 0 || y >= dim {
                continue;
            }
            for dx in -r..=r {
                let x = i64::from(cx) + dx;
                if x < 0 || x >= dim {
                    continue;
                }
                if dx * dx + dy * dy <= rr {
                    count += 1;
                }
            }
        }
        if count == 0 {
            return units;
        }
        let base = units / count;
        let extra = units % count;
        let mut leftover = 0u32;
        let mut seen = 0u32;
        for dy in -r..=r {
            let y = i64::from(cy) + dy;
            if y < 0 || y >= dim {
                continue;
            }
            for dx in -r..=r {
                let x = i64::from(cx) + dx;
                if x < 0 || x >= dim {
                    continue;
                }
                if dx * dx + dy * dy > rr {
                    continue;
                }
                let share = base + u32::from(seen < extra);
                seen += 1;
                if share == 0 {
                    continue;
                }
                let i = (y as usize * MAP_DIM + x as usize) * 2 + plane;
                let room = u16::MAX - self.texels[i];
                let taken = share.min(u32::from(room));
                self.texels[i] += u16::try_from(taken).unwrap_or(0);
                leftover = leftover.saturating_add(share - taken);
            }
        }
        leftover
    }
}

/// Texel column or row of a coordinate inside the pit. The one float step in
/// this module: the input is validated to `0 ..= PIT_METERS` by the caller.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller accepts only 0 ..= PIT_METERS, so the quotient is inside 0 ..= MAP_DIM and the rounded value fits u32"
)]
fn texel_of(meters: f32) -> u32 {
    let scaled = (meters / PIT_METERS * MAP_DIM as f32).round();
    (scaled as u32).min(MAP_DIM as u32 - 1)
}

/// Disc radius in texels, at least one texel so every mark leaves a footprint.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the caller accepts only (0, MAX_STAMP_RADIUS_M], so the quotient is inside 1 ..= 103 texels"
)]
fn radius_texels(radius_m: f32) -> u32 {
    let scaled = (radius_m / PIT_METERS * MAP_DIM as f32).round();
    (scaled as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: u32 = MASS_UNITS_PER_MARK;

    /// Stamp one mark and age it one tick, the arena's shape: a stamp lands
    /// during a tick and the tick then advances.
    fn stamp_and_tick(state: &mut GroundState, mark: Mark, x: f32, y: f32) {
        state.stamp(mark, x, y, 0.25, ONE).expect("stamp");
        state.advance(1);
    }

    #[test]
    fn ground_state_map_budget_is_1024_square_rgba_plus_its_composite_pair() {
        assert_eq!(MAP_DIM, 1024);
        assert_eq!(TEXEL_BYTES, 4);
        assert_eq!(MAP_BYTES, 4_194_304);
        assert_eq!(GROUND_BYTES, 2 * MAP_BYTES);
        let state = GroundState::new();
        assert_eq!(state.bytes(), MAP_BYTES);
        assert_eq!(state.texels().len() * 2, MAP_BYTES);
        assert_eq!(state.pool_len(), 0);
        assert_eq!(state.tick(), 0);
    }

    #[test]
    fn ground_state_stamp_lands_in_the_pool_and_in_the_right_plane() {
        let mut state = GroundState::new();
        state
            .stamp(Mark::Blood, 10.0, 10.0, 0.25, ONE)
            .expect("centre stamp");
        assert_eq!(state.pool_len(), 1);
        assert_eq!(state.stamps_this_tick(), 1);
        assert_eq!(state.stamped_units(Mark::Blood), u64::from(ONE));
        assert_eq!(state.pool_units(Mark::Blood), u64::from(ONE));
        assert_eq!(state.map_units(Mark::Blood), 0);
        assert_eq!(state.map_units(Mark::Scuff), 0);
        let stamp = state.pool()[0];
        assert_eq!(stamp.mark, Mark::Blood);
        assert_eq!((stamp.x, stamp.y), (512, 512));
        assert_eq!(stamp.radius, 13);
        assert_eq!(stamp.born_tick, 0);

        // Nothing is written to the map while the mark is live.
        assert_eq!(state.texel(512, 512), (0, 0));
        assert_eq!(state.plane_units(Mark::Blood), 0);

        // The mass lands in the blood channel only.
        state.advance(BLOOD_LIFETIME_TICKS);
        assert_eq!(state.pool_len(), 0);
        let (blood, scuff) = state.texel(512, 512);
        assert!(blood > 0, "the centre texel holds blood");
        assert_eq!(scuff, 0);
        assert_eq!(state.map_units(Mark::Blood), u64::from(ONE));
        assert_eq!(state.map_units(Mark::Scuff), 0);
    }

    #[test]
    fn ground_state_refuses_bad_stamps_without_changing_anything() {
        let mut state = GroundState::new();
        assert!(matches!(
            state.stamp(Mark::Blood, -0.5, 5.0, 0.25, ONE),
            Err(GroundError::OutsidePit { .. })
        ));
        assert!(matches!(
            state.stamp(Mark::Blood, 5.0, 20.5, 0.25, ONE),
            Err(GroundError::OutsidePit { .. })
        ));
        assert!(matches!(
            state.stamp(Mark::Blood, 5.0, 5.0, 0.0, ONE),
            Err(GroundError::InvalidRadius { .. })
        ));
        assert!(matches!(
            state.stamp(Mark::Blood, 5.0, 5.0, MAX_STAMP_RADIUS_M + 0.1, ONE),
            Err(GroundError::InvalidRadius { .. })
        ));
        assert_eq!(
            state.stamp(Mark::Blood, 5.0, 5.0, 0.25, 0),
            Err(GroundError::ZeroMass)
        );
        assert_eq!(state.pool_len(), 0);
        assert_eq!(state.stamped_units(Mark::Blood), 0);
        assert_eq!(state.stamps_this_tick(), 0);
    }

    #[test]
    fn ground_state_fade_curve_runs_from_full_to_the_floor_at_the_lifetime() {
        assert_eq!(fade_bp(Mark::Blood, 0), FULL_BP);
        assert_eq!(fade_bp(Mark::Blood, BLOOD_LIFETIME_TICKS), FADE_FLOOR_BP);
        assert_eq!(fade_bp(Mark::Blood, 100_000), FADE_FLOOR_BP);

        // Strictly decreasing over the life, and never below the floor.
        let mut previous = FULL_BP + 1;
        for age in 0..BLOOD_LIFETIME_TICKS {
            let now = fade_bp(Mark::Blood, age);
            assert!(
                now < previous,
                "fade must fall at age {age}: {now} >= {previous}"
            );
            assert!(now >= FADE_FLOOR_BP);
            previous = now;
        }

        // A scuff walks the same curve over twice the life: at the blood
        // lifetime it is still above the floor, and it reaches it later.
        assert!(fade_bp(Mark::Scuff, BLOOD_LIFETIME_TICKS) > FADE_FLOOR_BP);
        assert_eq!(fade_bp(Mark::Scuff, SCUFF_LIFETIME_TICKS), FADE_FLOOR_BP);
        assert!(
            fade_bp(Mark::Scuff, 300) > fade_bp(Mark::Blood, 300),
            "the slower fade is brighter at the same age"
        );
    }

    #[test]
    fn ground_state_live_slot_reports_its_own_fade() {
        let mut state = GroundState::new();
        state
            .stamp(Mark::Blood, 4.0, 4.0, 0.25, ONE)
            .expect("stamp");
        assert_eq!(state.pool()[0].intensity_bp(0), FULL_BP);
        state.advance(300);
        let half = state.pool()[0].intensity_bp(state.tick());
        assert!(half < FULL_BP && half > FADE_FLOOR_BP);
        assert!(!state.pool()[0].expired(state.tick()));
        state.advance(299);
        assert_eq!(state.tick(), 599);
        assert!(!state.pool()[0].expired(state.tick()), "one tick short");
        state.advance(1);
        assert!(state.pool().is_empty(), "600 ticks retires blood");
    }

    #[test]
    fn ground_state_pool_is_empty_after_ten_seconds_without_a_fight() {
        // The row's first proof: 10 s of quiet leaves no live decal.
        let mut state = GroundState::new();
        for i in 0..4 {
            let x = 5.0 + 3.0 * i as f32;
            state.stamp(Mark::Blood, x, 10.0, 0.3, ONE).expect("stamp");
        }
        assert_eq!(state.pool_len(), 4);
        state.advance(BLOOD_LIFETIME_TICKS - 1);
        assert_eq!(state.pool_len(), 4, "one tick short of the lifetime");
        state.advance(1);
        assert_eq!(state.pool_len(), 0, "the pool holds 0 decals after 10 s");
        assert_eq!(state.pool_units(Mark::Blood), 0);
        assert_eq!(
            state.map_units(Mark::Blood),
            4 * u64::from(ONE),
            "the stain outlives the pool"
        );
    }

    #[test]
    fn ground_state_pool_caps_and_merges_instead_of_dropping_mass() {
        let mut state = GroundState::new();
        for i in 0..(POOL_SLOTS as u32 + 72) {
            let x = 0.5 + 19.0 * (i % 20) as f32 / 20.0;
            let y = 0.5 + 19.0 * (i / 20) as f32 / 20.0;
            stamp_and_tick(&mut state, Mark::Blood, x, y);
            assert!(state.pool_len() <= POOL_SLOTS);
        }
        assert_eq!(state.pool_len(), POOL_SLOTS);
        let merges: u64 = state.pool().iter().map(|s| u64::from(s.merged)).sum();
        assert!(merges > 0, "72 extra marks were folded in");
        assert_eq!(
            state.stamped_units(Mark::Blood),
            u64::from(ONE) * u64::from(POOL_SLOTS as u32 + 72)
        );
        assert_eq!(
            state.map_units(Mark::Blood) + state.pool_units(Mark::Blood),
            state.stamped_units(Mark::Blood),
            "a full pool loses no mass"
        );
    }

    #[test]
    fn ground_state_blood_mass_survives_3600_ticks() {
        // The row's accept test: 40 stamps spread over 3600 ticks, then at
        // least 95 percent of the blood mass is still in the map or pool.
        let mut state = GroundState::new();
        let mut total = 0u64;
        for i in 0..40u32 {
            let x = 1.0 + 18.0 * (i % 8) as f32 / 8.0;
            let y = 1.0 + 18.0 * (i / 8) as f32 / 5.0;
            state.stamp(Mark::Blood, x, y, 0.35, ONE).expect("stamp");
            total += u64::from(ONE);
            state.advance(90);
        }
        // 40 * 90 = 3600 ticks have passed since the first mark.
        assert_eq!(state.tick(), 3_600);
        assert_eq!(state.stamped_units(Mark::Blood), total);
        let kept = state.map_units(Mark::Blood) + state.pool_units(Mark::Blood);
        assert_eq!(kept, total, "mass is conserved exactly, not approximately");
        assert!(
            kept * 100 >= total * 95,
            "kept {kept} of {total} blood units ({} percent)",
            kept * 100 / total
        );
        // The stain is real pixels, not only a ledger.
        assert!(state.plane_units(Mark::Blood) > 0);
        assert!(state.pool_len() > 0, "the last few marks are still live");
    }

    #[test]
    fn ground_state_allows_sixteen_stamps_a_tick_and_no_seventeenth() {
        let mut state = GroundState::new();
        for i in 0..MAX_STAMPS_PER_TICK {
            let x = 1.0 + i as f32;
            state
                .stamp(Mark::Blood, x, 10.0, 0.2, ONE)
                .expect("inside the budget");
        }
        assert_eq!(state.stamps_this_tick(), MAX_STAMPS_PER_TICK);
        assert_eq!(
            state.stamp(Mark::Scuff, 10.0, 10.0, 0.2, ONE),
            Err(GroundError::TickBudget)
        );
        state.advance(1);
        assert_eq!(state.stamps_this_tick(), 0);
        assert!(
            state.stamp(Mark::Scuff, 10.0, 10.0, 0.2, ONE).is_ok(),
            "the next tick opens a fresh budget"
        );
    }

    #[test]
    fn ground_state_same_stamp_stream_leaves_the_same_map_bytes() {
        let build = || {
            let mut state = GroundState::new();
            for i in 0..32u32 {
                let x = 0.4 + 19.0 * (i % 7) as f32 / 7.0;
                let y = 0.4 + 19.0 * (i / 7) as f32 / 4.0;
                let mark = if i % 3 == 0 { Mark::Scuff } else { Mark::Blood };
                state.stamp(mark, x, y, 0.5, ONE).expect("stamp");
                state.advance(37);
            }
            state
        };
        let a = build();
        let b = build();
        assert_eq!(a.tick(), b.tick());
        assert_eq!(a.texels(), b.texels(), "the map is byte-for-byte the same");
        assert_eq!(a.pool(), b.pool());
        assert_eq!(
            a.map_units(Mark::Blood) + a.pool_units(Mark::Blood),
            a.stamped_units(Mark::Blood)
        );
        assert_eq!(
            a.map_units(Mark::Scuff) + a.pool_units(Mark::Scuff),
            a.stamped_units(Mark::Scuff)
        );
    }

    #[test]
    fn ground_state_edge_stamp_keeps_its_mass_inside_the_map() {
        // A disc at the corner is clipped by the map; the clipped texels must
        // not take the mass with them.
        let mut state = GroundState::new();
        state
            .stamp(Mark::Blood, 0.0, 0.0, 0.5, ONE)
            .expect("corner stamp");
        state.advance(BLOOD_LIFETIME_TICKS);
        assert_eq!(state.pool_len(), 0);
        assert_eq!(state.map_units(Mark::Blood), u64::from(ONE));
        assert!(state.texel(0, 0).0 > 0);
        assert_eq!(state.map_units(Mark::Scuff), 0);
    }
}

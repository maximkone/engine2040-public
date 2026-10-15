//! exert: exertion/gait warp over locomotion weight slices.
//!
//! Consumer: arena locomotion warps its per-lane weights and transition
//! times from per-fighter fatigue before search. Data owner: `engine-anim`;
//! this module takes plain slices and owns no locomotion constant or type.
//! Units: ticks at 60 Hz; gains in Q8 (256 reads 1x).
//!
//! Determinism: integer-only math in fixed lane order with fighter-id-rotated
//! tie-breaks, so the same table plus the same script digests identically
//! twice. Errors are [`ExertError`].
//!
//! Budget: one multiply per lane (cap [`MAX_LANES`]); single pick scan; no
//! heap allocation per tick (caller-owned buffers only).

/// Largest warpable weight lane count; bounds per-tick multiply work.
pub const MAX_LANES: usize = 64;
/// Largest accepted per-lane weight; warped output clamps here.
pub const WEIGHT_MAX: i32 = 4096;
/// Q8 gain pair, single-sourced from [`crate::layers`] (one owner per
/// constant; this re-export keeps existing paths working with one value).
pub use super::layers::{GAIN_Q8_MAX, GAIN_Q8_ONE};
/// Full-effort scale for [`ExertState::advance`]: 0 rests, 256 works fully.
pub const EFFORT_MAX: u16 = 256;
/// Fatigue clamp in ticks (60 s at 60 Hz).
pub const MAX_FATIGUE_TICKS: u32 = 3600;
/// Widest accepted base transition in ticks.
pub const MAX_BASE_TRANSITION_TICKS: u32 = 120;
/// Widest added transition in ticks.
pub const MAX_TRANSITION_ADD_TICKS: u16 = 60;
/// Largest candidate count for [`select_best`].
pub const MAX_CANDIDATES: usize = 4096;
/// Work/recover rate ceiling per tick.
pub const MAX_RATE_PER_TICK: u16 = 60;

/// What rejected an exertion call. Refusals change nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExertError {
    /// No weight lanes were given.
    EmptyWeights,
    /// Base and output slices differ in length; lengths carried.
    MismatchedLen {
        /// Length of the base slice.
        base: usize,
        /// Length of the output slice.
        out: usize,
    },
    /// More lanes than [`MAX_LANES`]; length carried.
    TooManyLanes {
        /// Requested lane count.
        len: usize,
    },
    /// The pose/trajectory split lies past the slice end; both carried.
    BadPoseSplit {
        /// Requested pose lane count.
        pose_lanes: usize,
        /// Total lane count.
        len: usize,
    },
    /// A base weight sits outside `0..=WEIGHT_MAX`; lane index carried.
    BadWeight {
        /// Offending lane index.
        lane: usize,
    },
    /// A tuning lane value sits outside its legal range; nothing was written.
    LaneRange {
        /// Tuning lane name.
        lane: &'static str,
        /// The refused value.
        value: u32,
        /// Legal minimum, inclusive.
        min: u32,
        /// Legal maximum, inclusive.
        max: u32,
    },
    /// Thresholds break the `winded < tired < spent` order; current values
    /// carried, nothing was written.
    BadOrder {
        /// Fatigue ticks reading winded.
        winded_at_ticks: u32,
        /// Fatigue ticks reading tired.
        tired_at_ticks: u32,
        /// Fatigue ticks reading spent.
        spent_at_ticks: u32,
    },
    /// Effort sits outside `0..=EFFORT_MAX`; state untouched.
    BadEffort {
        /// The refused effort.
        effort_q8: u16,
    },
    /// Base transition ticks exceed [`MAX_BASE_TRANSITION_TICKS`].
    BadBaseTicks {
        /// The refused tick count.
        ticks: u32,
    },
    /// No candidate costs were given.
    EmptyCosts,
    /// More candidates than [`MAX_CANDIDATES`]; length carried.
    TooManyCandidates {
        /// Requested candidate count.
        len: usize,
    },
}

impl core::fmt::Display for ExertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ExertError::EmptyWeights => write!(f, "exert: no weight lanes"),
            ExertError::MismatchedLen { base, out } => {
                write!(f, "exert: base len {base} differs from out len {out}")
            }
            ExertError::TooManyLanes { len } => {
                write!(f, "exert: {len} lanes past {MAX_LANES}")
            }
            ExertError::BadPoseSplit { pose_lanes, len } => {
                write!(f, "exert: pose split {pose_lanes} past {len} lanes")
            }
            ExertError::BadWeight { lane } => {
                write!(f, "exert: weight lane {lane} outside 0..={WEIGHT_MAX}")
            }
            ExertError::LaneRange {
                lane,
                value,
                min,
                max,
            } => {
                write!(f, "exert: {lane} value {value} outside {min}..={max}")
            }
            ExertError::BadOrder {
                winded_at_ticks,
                tired_at_ticks,
                spent_at_ticks,
            } => write!(
                f,
                "exert: thresholds {winded_at_ticks}/{tired_at_ticks}/{spent_at_ticks} break winded < tired < spent"
            ),
            ExertError::BadEffort { effort_q8 } => {
                write!(f, "exert: effort {effort_q8} past {EFFORT_MAX}")
            }
            ExertError::BadBaseTicks { ticks } => {
                write!(f, "exert: base {ticks} ticks past {MAX_BASE_TRANSITION_TICKS}")
            }
            ExertError::EmptyCosts => write!(f, "exert: no candidate costs"),
            ExertError::TooManyCandidates { len } => {
                write!(f, "exert: {len} candidates past {MAX_CANDIDATES}")
            }
        }
    }
}

impl std::error::Error for ExertError {}

/// One tunable exertion scalar, in table order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TuneLane {
    /// Fatigue ticks reading winded.
    WindedAtTicks,
    /// Fatigue ticks reading tired.
    TiredAtTicks,
    /// Fatigue ticks reading spent.
    SpentAtTicks,
    /// Trajectory-lane gain while winded, Q8.
    TrajGainWindedQ8,
    /// Trajectory-lane gain while tired, Q8.
    TrajGainTiredQ8,
    /// Trajectory-lane gain while spent, Q8.
    TrajGainSpentQ8,
    /// Pose-lane gain while winded, Q8.
    PoseGainWindedQ8,
    /// Pose-lane gain while tired, Q8.
    PoseGainTiredQ8,
    /// Pose-lane gain while spent, Q8.
    PoseGainSpentQ8,
    /// Added transition ticks while winded.
    TransitionAddWindedTicks,
    /// Added transition ticks while tired.
    TransitionAddTiredTicks,
    /// Added transition ticks while spent.
    TransitionAddSpentTicks,
    /// Fatigue added per full-effort tick.
    WorkPerTick,
    /// Fatigue removed per full-rest tick.
    RecoverPerTick,
}

impl TuneLane {
    /// Every tunable lane, in table order.
    pub fn all() -> [Self; 14] {
        [
            Self::WindedAtTicks,
            Self::TiredAtTicks,
            Self::SpentAtTicks,
            Self::TrajGainWindedQ8,
            Self::TrajGainTiredQ8,
            Self::TrajGainSpentQ8,
            Self::PoseGainWindedQ8,
            Self::PoseGainTiredQ8,
            Self::PoseGainSpentQ8,
            Self::TransitionAddWindedTicks,
            Self::TransitionAddTiredTicks,
            Self::TransitionAddSpentTicks,
            Self::WorkPerTick,
            Self::RecoverPerTick,
        ]
    }

    /// Lane name for diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            Self::WindedAtTicks => "winded_at_ticks",
            Self::TiredAtTicks => "tired_at_ticks",
            Self::SpentAtTicks => "spent_at_ticks",
            Self::TrajGainWindedQ8 => "traj_gain_winded_q8",
            Self::TrajGainTiredQ8 => "traj_gain_tired_q8",
            Self::TrajGainSpentQ8 => "traj_gain_spent_q8",
            Self::PoseGainWindedQ8 => "pose_gain_winded_q8",
            Self::PoseGainTiredQ8 => "pose_gain_tired_q8",
            Self::PoseGainSpentQ8 => "pose_gain_spent_q8",
            Self::TransitionAddWindedTicks => "transition_add_winded_ticks",
            Self::TransitionAddTiredTicks => "transition_add_tired_ticks",
            Self::TransitionAddSpentTicks => "transition_add_spent_ticks",
            Self::WorkPerTick => "work_per_tick",
            Self::RecoverPerTick => "recover_per_tick",
        }
    }

    /// Legal inclusive range for this lane.
    pub fn range(self) -> (u32, u32) {
        match self {
            Self::WindedAtTicks | Self::TiredAtTicks | Self::SpentAtTicks => (1, MAX_FATIGUE_TICKS),
            Self::TrajGainWindedQ8
            | Self::TrajGainTiredQ8
            | Self::TrajGainSpentQ8
            | Self::PoseGainWindedQ8
            | Self::PoseGainTiredQ8
            | Self::PoseGainSpentQ8 => (0, u32::from(GAIN_Q8_MAX)),
            Self::TransitionAddWindedTicks
            | Self::TransitionAddTiredTicks
            | Self::TransitionAddSpentTicks => (0, u32::from(MAX_TRANSITION_ADD_TICKS)),
            Self::WorkPerTick | Self::RecoverPerTick => (0, u32::from(MAX_RATE_PER_TICK)),
        }
    }
}

/// Tunable exertion scalars. The tuner drives one lane at a time through
/// [`set`](Self::set); every entry point re-validates, so a tuned fight
/// replays exactly. Gains are Q8 (256 reads 1x); thresholds, adds, work and
/// recover are in ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExertTable {
    /// Fatigue ticks reading winded; below this the gait reads fresh.
    pub winded_at_ticks: u32,
    /// Fatigue ticks reading tired; strictly above winded.
    pub tired_at_ticks: u32,
    /// Fatigue ticks reading spent; strictly above tired.
    pub spent_at_ticks: u32,
    /// Trajectory-lane gain while winded, Q8.
    pub traj_gain_winded_q8: u16,
    /// Trajectory-lane gain while tired, Q8.
    pub traj_gain_tired_q8: u16,
    /// Trajectory-lane gain while spent, Q8.
    pub traj_gain_spent_q8: u16,
    /// Pose-lane gain while winded, Q8.
    pub pose_gain_winded_q8: u16,
    /// Pose-lane gain while tired, Q8.
    pub pose_gain_tired_q8: u16,
    /// Pose-lane gain while spent, Q8.
    pub pose_gain_spent_q8: u16,
    /// Added transition ticks while winded.
    pub transition_add_winded_ticks: u16,
    /// Added transition ticks while tired.
    pub transition_add_tired_ticks: u16,
    /// Added transition ticks while spent.
    pub transition_add_spent_ticks: u16,
    /// Fatigue added per full-effort tick.
    pub work_per_tick: u16,
    /// Fatigue removed per full-rest tick.
    pub recover_per_tick: u16,
}

impl Default for ExertTable {
    /// Tuned defaults: winded at 10 s, tired at 25 s, spent at 45 s; tired
    /// trajectory lanes gain ~1.3x while pose lanes damp to ~0.9x and
    /// transitions lengthen, so tired locomotion reads tired.
    fn default() -> Self {
        Self {
            winded_at_ticks: 600,
            tired_at_ticks: 1500,
            spent_at_ticks: 2700,
            traj_gain_winded_q8: 288,
            traj_gain_tired_q8: 336,
            traj_gain_spent_q8: 384,
            pose_gain_winded_q8: 240,
            pose_gain_tired_q8: 224,
            pose_gain_spent_q8: 192,
            transition_add_winded_ticks: 2,
            transition_add_tired_ticks: 4,
            transition_add_spent_ticks: 8,
            work_per_tick: 4,
            recover_per_tick: 2,
        }
    }
}

impl ExertTable {
    /// Range plus order check: every lane inside its range and
    /// `winded < tired < spent`.
    pub fn validate(&self) -> Result<(), ExertError> {
        for lane in TuneLane::all() {
            let (min, max) = lane.range();
            let value = self.get(lane);
            if value < min || value > max {
                return Err(ExertError::LaneRange {
                    lane: lane.name(),
                    value,
                    min,
                    max,
                });
            }
        }
        if self.winded_at_ticks >= self.tired_at_ticks || self.tired_at_ticks >= self.spent_at_ticks
        {
            return Err(ExertError::BadOrder {
                winded_at_ticks: self.winded_at_ticks,
                tired_at_ticks: self.tired_at_ticks,
                spent_at_ticks: self.spent_at_ticks,
            });
        }
        Ok(())
    }

    /// Current value of one tuning lane.
    pub fn get(&self, lane: TuneLane) -> u32 {
        match lane {
            TuneLane::WindedAtTicks => self.winded_at_ticks,
            TuneLane::TiredAtTicks => self.tired_at_ticks,
            TuneLane::SpentAtTicks => self.spent_at_ticks,
            TuneLane::TrajGainWindedQ8 => u32::from(self.traj_gain_winded_q8),
            TuneLane::TrajGainTiredQ8 => u32::from(self.traj_gain_tired_q8),
            TuneLane::TrajGainSpentQ8 => u32::from(self.traj_gain_spent_q8),
            TuneLane::PoseGainWindedQ8 => u32::from(self.pose_gain_winded_q8),
            TuneLane::PoseGainTiredQ8 => u32::from(self.pose_gain_tired_q8),
            TuneLane::PoseGainSpentQ8 => u32::from(self.pose_gain_spent_q8),
            TuneLane::TransitionAddWindedTicks => u32::from(self.transition_add_winded_ticks),
            TuneLane::TransitionAddTiredTicks => u32::from(self.transition_add_tired_ticks),
            TuneLane::TransitionAddSpentTicks => u32::from(self.transition_add_spent_ticks),
            TuneLane::WorkPerTick => u32::from(self.work_per_tick),
            TuneLane::RecoverPerTick => u32::from(self.recover_per_tick),
        }
    }

    /// Write one tuning lane: range-checked first, then order-checked, so a
    /// refusal leaves the table untouched.
    pub fn set(&mut self, lane: TuneLane, value: u32) -> Result<(), ExertError> {
        let (min, max) = lane.range();
        if value < min || value > max {
            return Err(ExertError::LaneRange {
                lane: lane.name(),
                value,
                min,
                max,
            });
        }
        let mut next = *self;
        match lane {
            TuneLane::WindedAtTicks => next.winded_at_ticks = value,
            TuneLane::TiredAtTicks => next.tired_at_ticks = value,
            TuneLane::SpentAtTicks => next.spent_at_ticks = value,
            TuneLane::TrajGainWindedQ8 => next.traj_gain_winded_q8 = value as u16,
            TuneLane::TrajGainTiredQ8 => next.traj_gain_tired_q8 = value as u16,
            TuneLane::TrajGainSpentQ8 => next.traj_gain_spent_q8 = value as u16,
            TuneLane::PoseGainWindedQ8 => next.pose_gain_winded_q8 = value as u16,
            TuneLane::PoseGainTiredQ8 => next.pose_gain_tired_q8 = value as u16,
            TuneLane::PoseGainSpentQ8 => next.pose_gain_spent_q8 = value as u16,
            TuneLane::TransitionAddWindedTicks => {
                next.transition_add_winded_ticks = value as u16;
            }
            TuneLane::TransitionAddTiredTicks => {
                next.transition_add_tired_ticks = value as u16;
            }
            TuneLane::TransitionAddSpentTicks => {
                next.transition_add_spent_ticks = value as u16;
            }
            TuneLane::WorkPerTick => next.work_per_tick = value as u16,
            TuneLane::RecoverPerTick => next.recover_per_tick = value as u16,
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

/// Gait band read from fatigue: the blind-review read-out for tired
/// locomotion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gait {
    /// Below the winded threshold: warp is the identity.
    Fresh,
    /// Steady but breathing: small warp.
    Winded,
    /// Heavy: trajectory lanes dominate, transitions lengthen.
    Tired,
    /// Empty: strongest warp.
    Spent,
}

impl Gait {
    /// Numeric band level: 0 fresh, 1 winded, 2 tired, 3 spent.
    pub fn level(self) -> u8 {
        match self {
            Self::Fresh => 0,
            Self::Winded => 1,
            Self::Tired => 2,
            Self::Spent => 3,
        }
    }
}

/// Per-fighter exertion: stable id plus fatigue in ticks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExertState {
    /// Stable fighter id; seeds tie-break rotation in [`select_best`].
    pub fighter_id: u64,
    /// Accumulated fatigue in ticks, clamped to [`MAX_FATIGUE_TICKS`].
    pub fatigue_ticks: u32,
}

impl ExertState {
    /// Rested state for one fighter.
    pub fn new(fighter_id: u64) -> Self {
        Self {
            fighter_id,
            fatigue_ticks: 0,
        }
    }

    /// Gait band for the current fatigue under this table.
    pub fn gait(&self, table: &ExertTable) -> Result<Gait, ExertError> {
        table.validate()?;
        if self.fatigue_ticks < table.winded_at_ticks {
            Ok(Gait::Fresh)
        } else if self.fatigue_ticks < table.tired_at_ticks {
            Ok(Gait::Winded)
        } else if self.fatigue_ticks < table.spent_at_ticks {
            Ok(Gait::Tired)
        } else {
            Ok(Gait::Spent)
        }
    }

    /// Charge work and bleed recovery over `ticks` at `effort_q8` (0 rests,
    /// 256 works fully): fatigue moves by
    /// `effort * work * ticks / 256` up and
    /// `(256 - effort) * recover * ticks / 256` down, clamped to
    /// `0..=MAX_FATIGUE_TICKS`. Integer-only, so the same script replays
    /// exactly.
    pub fn advance(
        &mut self,
        table: &ExertTable,
        effort_q8: u16,
        ticks: u32,
    ) -> Result<(), ExertError> {
        table.validate()?;
        if effort_q8 > EFFORT_MAX {
            return Err(ExertError::BadEffort { effort_q8 });
        }
        let work = u64::from(effort_q8) * u64::from(table.work_per_tick) * u64::from(ticks)
            / u64::from(EFFORT_MAX);
        let rest = u64::from(EFFORT_MAX - effort_q8)
            * u64::from(table.recover_per_tick)
            * u64::from(ticks)
            / u64::from(EFFORT_MAX);
        let raised = u64::from(self.fatigue_ticks).saturating_add(work);
        let lowered = raised.saturating_sub(rest);
        let clamped = lowered.min(u64::from(MAX_FATIGUE_TICKS));
        self.fatigue_ticks = clamped as u32;
        Ok(())
    }
}

/// Pose/trajectory Q8 gains for one gait band: fresh reads the identity.
fn gains_for(gait: Gait, table: &ExertTable) -> (u16, u16) {
    match gait {
        Gait::Fresh => (GAIN_Q8_ONE, GAIN_Q8_ONE),
        Gait::Winded => (table.pose_gain_winded_q8, table.traj_gain_winded_q8),
        Gait::Tired => (table.pose_gain_tired_q8, table.traj_gain_tired_q8),
        Gait::Spent => (table.pose_gain_spent_q8, table.traj_gain_spent_q8),
    }
}

/// Warp per-lane weights by exertion: the first `pose_lanes` lanes scale
/// by the pose gain, the rest by the trajectory gain, each as
/// `base * gain / 256` truncated, clamped to `0..=WEIGHT_MAX`. Fresh is an
/// exact copy. Lane order is fixed and the math is integer-only, so the same
/// base plus the same state warps identically twice. Writes into the
/// caller-owned `out` slice: no allocation per tick.
pub fn warp_weights(
    base: &[i32],
    out: &mut [i32],
    pose_lanes: usize,
    state: &ExertState,
    table: &ExertTable,
) -> Result<(), ExertError> {
    table.validate()?;
    if base.is_empty() {
        return Err(ExertError::EmptyWeights);
    }
    if base.len() != out.len() {
        return Err(ExertError::MismatchedLen {
            base: base.len(),
            out: out.len(),
        });
    }
    if base.len() > MAX_LANES {
        return Err(ExertError::TooManyLanes { len: base.len() });
    }
    if pose_lanes > base.len() {
        return Err(ExertError::BadPoseSplit {
            pose_lanes,
            len: base.len(),
        });
    }
    for (lane, weight) in base.iter().enumerate() {
        if *weight < 0 || *weight > WEIGHT_MAX {
            return Err(ExertError::BadWeight { lane });
        }
    }
    let gait = state.gait(table)?;
    let (pose_gain, traj_gain) = gains_for(gait, table);
    for (lane, (from, to)) in base.iter().zip(out.iter_mut()).enumerate() {
        let gain = if lane < pose_lanes {
            pose_gain
        } else {
            traj_gain
        };
        let warped = i64::from(*from) * i64::from(gain) / i64::from(GAIN_Q8_ONE);
        *to = warped.clamp(0, i64::from(WEIGHT_MAX)) as i32;
    }
    Ok(())
}

/// Warp a transition length by exertion: base ticks plus the gait band add.
/// Tired fighters turn over more slowly, and the longer blend replays exactly
/// because the add is an integer from the validated table.
pub fn transition_ticks(
    base_ticks: u32,
    state: &ExertState,
    table: &ExertTable,
) -> Result<u32, ExertError> {
    table.validate()?;
    if base_ticks > MAX_BASE_TRANSITION_TICKS {
        return Err(ExertError::BadBaseTicks { ticks: base_ticks });
    }
    let add = match state.gait(table)? {
        Gait::Fresh => 0,
        Gait::Winded => table.transition_add_winded_ticks,
        Gait::Tired => table.transition_add_tired_ticks,
        Gait::Spent => table.transition_add_spent_ticks,
    };
    Ok(base_ticks + u32::from(add))
}

/// Deterministic pick over candidate costs: lowest cost wins; the scan starts
/// at `fighter_id % len` with strict `<` replacement, so ties break in
/// fighter-id-rotated order and the same costs pick the same index twice for
/// the same fighter.
pub fn select_best(costs: &[i64], fighter_id: u64) -> Result<usize, ExertError> {
    if costs.is_empty() {
        return Err(ExertError::EmptyCosts);
    }
    if costs.len() > MAX_CANDIDATES {
        return Err(ExertError::TooManyCandidates { len: costs.len() });
    }
    let start = (fighter_id % costs.len() as u64) as usize;
    let mut best_index = start;
    let mut best = costs[start];
    for step in 1..costs.len() {
        let index = (start + step) % costs.len();
        if costs[index] < best {
            best = costs[index];
            best_index = index;
        }
    }
    Ok(best_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Fnv64;

    fn tired_state() -> ExertState {
        let mut state = ExertState::new(7);
        state
            .advance(&ExertTable::default(), EFFORT_MAX, 400)
            .expect("script effort in range");
        assert_eq!(state.fatigue_ticks, 1600);
        state
    }

    #[test]
    fn q8_pair_is_owned_by_layers() {
        // layers owns the engine-anim Q8 pair; this re-export must
        // read one value, so a local redefinition fails loudly here.
        assert_eq!(GAIN_Q8_ONE, crate::layers::GAIN_Q8_ONE);
        assert_eq!(GAIN_Q8_MAX, crate::layers::GAIN_Q8_MAX);
    }

    #[test]
    fn fresh_warp_is_identity() {
        let table = ExertTable::default();
        let state = ExertState::new(3);
        assert_eq!(
            state.gait(&table).expect("default table valid"),
            Gait::Fresh
        );
        let base = [1000, 800, 200, 150, 300, 250];
        let mut out = [0i32; 6];
        warp_weights(&base, &mut out, 2, &state, &table).expect("valid warp");
        assert_eq!(out, base);
    }

    #[test]
    fn tired_warp_applies_integer_gains() {
        let table = ExertTable::default();
        let state = tired_state();
        assert_eq!(
            state.gait(&table).expect("default table valid"),
            Gait::Tired
        );
        // Pose lanes damp 1000 * 224 / 256 = 875; traj lanes boost
        // 1000 * 336 / 256 = 1312 (remainder 128 truncates).
        let base = [1000, 1000, 1000, 1000];
        let mut out = [0i32; 4];
        warp_weights(&base, &mut out, 2, &state, &table).expect("valid warp");
        assert_eq!(out, [875, 875, 1312, 1312]);
    }

    #[test]
    fn warp_output_clamps_at_max() {
        let table = ExertTable::default();
        let mut state = ExertState::new(1);
        state
            .advance(&ExertTable::default(), EFFORT_MAX, 700)
            .expect("script effort in range");
        assert_eq!(
            state.gait(&table).expect("default table valid"),
            Gait::Spent
        );
        // Pose 4096 * 192 / 256 = 3072; traj 4096 * 384 / 256 = 6144,
        // clamped back to 4096.
        let base = [WEIGHT_MAX; 4];
        let mut out = [0i32; 4];
        warp_weights(&base, &mut out, 1, &state, &table).expect("valid warp");
        assert_eq!(out, [3072, WEIGHT_MAX, WEIGHT_MAX, WEIGHT_MAX]);
    }

    #[test]
    fn warp_rejects_bad_input() {
        let table = ExertTable::default();
        let state = ExertState::new(0);
        let mut out = [0i32; 2];
        assert_eq!(
            warp_weights(&[], &mut [], 0, &state, &table),
            Err(ExertError::EmptyWeights)
        );
        let base = [1, 2, 3];
        assert_eq!(
            warp_weights(&base, &mut out, 2, &state, &table),
            Err(ExertError::MismatchedLen { base: 3, out: 2 })
        );
        let base2 = [1, 2];
        assert_eq!(
            warp_weights(&base2, &mut out, 3, &state, &table),
            Err(ExertError::BadPoseSplit {
                pose_lanes: 3,
                len: 2
            })
        );
        let bad = [0, -5];
        assert_eq!(
            warp_weights(&bad, &mut out, 1, &state, &table),
            Err(ExertError::BadWeight { lane: 1 })
        );
        let heavy = [0, WEIGHT_MAX + 1];
        assert_eq!(
            warp_weights(&heavy, &mut out, 1, &state, &table),
            Err(ExertError::BadWeight { lane: 1 })
        );
        let roomy = vec![1i32; MAX_LANES + 1];
        let mut roomy_out = vec![0i32; MAX_LANES + 1];
        assert_eq!(
            warp_weights(&roomy, &mut roomy_out, 1, &state, &table),
            Err(ExertError::TooManyLanes { len: MAX_LANES + 1 })
        );
    }

    #[test]
    fn table_ranges_and_order_reject() {
        let mut table = ExertTable::default();
        table.validate().expect("default table valid");
        assert_eq!(
            table.set(TuneLane::WindedAtTicks, 0),
            Err(ExertError::LaneRange {
                lane: "winded_at_ticks",
                value: 0,
                min: 1,
                max: MAX_FATIGUE_TICKS,
            })
        );
        assert_eq!(
            table.set(TuneLane::TrajGainWindedQ8, u32::from(GAIN_Q8_MAX) + 1),
            Err(ExertError::LaneRange {
                lane: "traj_gain_winded_q8",
                value: u32::from(GAIN_Q8_MAX) + 1,
                min: 0,
                max: u32::from(GAIN_Q8_MAX),
            })
        );
        // Ordering: tired must stay strictly above winded (600).
        assert_eq!(
            table.set(TuneLane::TiredAtTicks, 600),
            Err(ExertError::BadOrder {
                winded_at_ticks: 600,
                tired_at_ticks: 600,
                spent_at_ticks: 2700,
            })
        );
        // Refusals write nothing.
        assert_eq!(table, ExertTable::default());
        table
            .set(TuneLane::TrajGainTiredQ8, 400)
            .expect("gain inside range");
        assert_eq!(table.get(TuneLane::TrajGainTiredQ8), 400);
    }

    #[test]
    fn transition_grows_with_fatigue() {
        let table = ExertTable::default();
        let bands = [0, 600, 1500, 2700];
        let mut got = [0u32; 4];
        for (slot, fatigue) in bands.iter().enumerate() {
            let state = ExertState {
                fighter_id: 9,
                fatigue_ticks: *fatigue,
            };
            got[slot] = transition_ticks(10, &state, &table).expect("valid transition");
        }
        assert_eq!(got, [10, 12, 14, 18]);
        let fresh = ExertState::new(9);
        assert_eq!(
            transition_ticks(MAX_BASE_TRANSITION_TICKS + 1, &fresh, &table),
            Err(ExertError::BadBaseTicks {
                ticks: MAX_BASE_TRANSITION_TICKS + 1
            })
        );
    }

    #[test]
    fn advance_accumulates_recovers_and_clamps() {
        let table = ExertTable::default();
        let mut state = ExertState::new(5);
        state
            .advance(&table, EFFORT_MAX, 100)
            .expect("script effort in range");
        assert_eq!(state.fatigue_ticks, 400);
        state.advance(&table, 0, 100).expect("rest effort valid");
        assert_eq!(state.fatigue_ticks, 200);
        // Recovery saturates at zero instead of wrapping.
        state.advance(&table, 0, 200).expect("rest effort valid");
        assert_eq!(state.fatigue_ticks, 0);
        // Work saturates at the clamp.
        state
            .advance(&table, EFFORT_MAX, 10_000)
            .expect("script effort in range");
        assert_eq!(state.fatigue_ticks, MAX_FATIGUE_TICKS);
        // Half effort nets +1 per tick: work 2, rest 1.
        let mut half = ExertState::new(6);
        half.advance(&table, 128, 10).expect("script effort valid");
        assert_eq!(half.fatigue_ticks, 10);
        // Bad effort refuses and leaves state untouched.
        assert_eq!(
            half.advance(&table, EFFORT_MAX + 1, 1),
            Err(ExertError::BadEffort {
                effort_q8: EFFORT_MAX + 1
            })
        );
        assert_eq!(half.fatigue_ticks, 10);
    }

    #[test]
    fn gait_boundaries_match_thresholds() {
        let table = ExertTable::default();
        let probe = |fatigue: u32| {
            ExertState {
                fighter_id: 2,
                fatigue_ticks: fatigue,
            }
            .gait(&table)
            .expect("default table valid")
        };
        assert_eq!(probe(0), Gait::Fresh);
        assert_eq!(probe(599), Gait::Fresh);
        assert_eq!(probe(600), Gait::Winded);
        assert_eq!(probe(1499), Gait::Winded);
        assert_eq!(probe(1500), Gait::Tired);
        assert_eq!(probe(2699), Gait::Tired);
        assert_eq!(probe(2700), Gait::Spent);
        assert_eq!(probe(MAX_FATIGUE_TICKS), Gait::Spent);
        assert_eq!(Gait::Spent.level(), 3);
        assert_eq!(Gait::Fresh.level(), 0);
    }

    #[test]
    fn select_best_breaks_ties_by_fighter() {
        // Tied 5s: fighter 0 scans from 0 and keeps index 0, fighter 1
        // scans from 1 and keeps index 1.
        assert_eq!(select_best(&[5, 5, 9], 0).expect("costs valid"), 0);
        assert_eq!(select_best(&[5, 5, 9], 1).expect("costs valid"), 1);
        // Fighter 5 starts at 5 % 3 = 2, sees 9, then 5 at index 0 wins.
        assert_eq!(select_best(&[5, 5, 9], 5).expect("costs valid"), 0);
        // A unique minimum wins whatever the fighter id is.
        assert_eq!(select_best(&[9, 3, 7], 0).expect("costs valid"), 1);
        assert_eq!(select_best(&[9, 3, 7], 2).expect("costs valid"), 1);
        assert_eq!(select_best(&[], 0), Err(ExertError::EmptyCosts));
        let many = vec![1i64; MAX_CANDIDATES + 1];
        assert_eq!(
            select_best(&many, 0),
            Err(ExertError::TooManyCandidates {
                len: MAX_CANDIDATES + 1
            })
        );
    }

    /// Scripted tuned fight: three fighters work a fixed effort script while
    /// warping weights, lengthening transitions and picking from derived
    /// costs. Returns the transcript digest plus closing fatigue per fighter.
    fn run_fight(table: &ExertTable) -> (u64, Vec<(u64, u32)>) {
        let ids: [u64; 3] = [7, 42, 99];
        let base: [i32; 6] = [1000, 800, 200, 150, 300, 250];
        let mut digest = Fnv64::new();
        let mut ends = Vec::new();
        for id in ids {
            let mut state = ExertState::new(id);
            let mut warped = [0i32; 6];
            let mut costs = [0i64; 4];
            for tick in 0..600u32 {
                let phase = tick + (id % 8) as u32 * 37;
                let effort: u16 = if phase.is_multiple_of(3) { 256 } else { 128 };
                state
                    .advance(table, effort, 1)
                    .expect("script effort in range");
                warp_weights(&base, &mut warped, 2, &state, table).expect("script lanes valid");
                let trans = transition_ticks(10, &state, table).expect("script base ticks valid");
                for (c, slot) in costs.iter_mut().enumerate() {
                    let lane = warped[c % warped.len()];
                    *slot = i64::from(lane) + i64::from(trans) * (c as i64 + 1);
                }
                let pick = select_best(&costs, id).expect("script costs valid");
                digest.write_u64(id);
                digest.write_u32(state.fatigue_ticks);
                for weight in warped {
                    digest.write_i32(weight);
                }
                digest.write_u32(trans);
                digest.write_u32(pick as u32);
            }
            ends.push((id, state.fatigue_ticks));
        }
        (digest.finish(), ends)
    }

    #[test]
    fn tuned_fight_replays_exactly() {
        let mut table = ExertTable::default();
        table
            .set(TuneLane::WindedAtTicks, 200)
            .expect("threshold in range");
        table
            .set(TuneLane::TiredAtTicks, 500)
            .expect("threshold in range");
        table
            .set(TuneLane::SpentAtTicks, 900)
            .expect("threshold in range");
        let (first, first_ends) = run_fight(&table);
        let (second, second_ends) = run_fight(&table);
        assert_eq!(first, second);
        assert_eq!(first_ends, second_ends);
        // The script works every fighter into the spent band, so the warp
        // bands (not just fresh copies) shape the transcript.
        for (_, fatigue) in &first_ends {
            assert!(*fatigue >= 900, "fighter rests too much: {fatigue}");
        }
        // Retuning one lane moves the transcript: tuning matters.
        let mut retuned = table;
        retuned
            .set(TuneLane::TrajGainTiredQ8, 400)
            .expect("gain inside range");
        let (third, _) = run_fight(&retuned);
        assert_ne!(first, third);
    }
}

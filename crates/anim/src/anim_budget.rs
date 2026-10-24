//! anim_budget: one per-frame animation CPU budget in milliseconds, spent
//! through the LOD tier ladder, least significant characters first.
//!
//! Owner: `engine-anim`; status: pure allocator beside [`crate::lod`], the
//! game wiring is plan row AM25. Idea: Unreal's Animation Budget Allocator
//! (`docs/research/plugins-2026-10-24/05-unreal-deep.md` entry 1). Each actor
//! carries a significance (screen size, distance, in a fight); when the
//! measured pose cost passes the budget the least significant drop down the
//! existing [`Tier`] ladder (full rate, every 2nd tick, every 4th, held far
//! pose; a [`LodSlot`](crate::lod::LodSlot) shows each coarse tick without a
//! pop). The player and their current opponent are pinned to [`Tier::Full`].
//!
//! Units: milliseconds per frame, meters, screen fraction `0..=1`, frames.
//! Determinism: `total_cmp` plus index order only, no clock, no hash map;
//! the measured cost arrives as a number, so one tape gives one answer twice.
//! Presentation only: tiers gate view-side pose sampling, never the sim.
//! Errors: [`BudgetError`]. Budget: the allocator owns lanes sized to the
//! actor count, grown on the first frame or a count change and reused after,
//! so a steady frame allocates nothing.
//!
//! Cost model: one full-rate eval of a weight-1 actor costs `unit_ms`. A
//! tier at period `p` costs `unit_ms / p` amortized (the slot stagger spreads
//! the evals over the frames). `unit_ms` is learned each frame from the
//! measured cost and the load the previous assignment put on it, smoothed.
//!
//! Hysteresis: a rung change waits [`BudgetConfig::hold_frames`] since the
//! actor's last change. Going coarser needs the plan over budget, going finer
//! needs it to fit under `budget * headroom`, so the dead band between the two
//! never flips a rung. A measured cost past `emergency` times the budget
//! skips the hold for going coarser (Unreal's emergency reduced work).

use super::lod::{Tier, BIG_SCREEN, FAR_DIST_M};

/// Most actors one allocator handles; matches the crowd tape cap.
pub const MAX_BUDGET_ACTORS: usize = 1024;
/// Significance of a pinned actor (player or current opponent); above any
/// score [`significance`] can give an unpinned one.
pub const SIG_PINNED: f32 = 100.0;
/// Significance added while an actor is in a fight.
pub const FIGHT_BONUS: f32 = 1.0;
/// Rungs on the ladder: full, half, quarter, far.
const RUNGS: usize = 4;

/// Rung index of a tier: 0 is full rate, 3 is the held far pose.
fn rung_of(tier: Tier) -> u8 {
    match tier {
        Tier::Full => 0,
        Tier::Half => 1,
        Tier::Quarter => 2,
        Tier::Far => 3,
    }
}

/// Tier of a rung index; past the last rung reads as far.
fn tier_of(rung: u8) -> Tier {
    match rung {
        0 => Tier::Full,
        1 => Tier::Half,
        2 => Tier::Quarter,
        _ => Tier::Far,
    }
}

/// Amortized share of a full eval one rung costs: `1 / period`.
fn rate_of(rung: u8) -> f32 {
    match rung {
        0 => 1.0,
        1 => 0.5,
        2 => 0.25,
        _ => 0.125,
    }
}

/// Bounded allocator failures: typed, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BudgetError {
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Distance below zero; value carried.
    NegativeDistance(f32),
    /// Screen fraction outside `0..=1`; value carried.
    BadScreen(f32),
    /// Cost weight at or below zero; value carried.
    BadWeight(f32),
    /// Measured cost below zero; value carried.
    NegativeMeasured(f32),
    /// Output lane length differs from the actor count; expected plus got.
    LengthMismatch { expected: usize, got: usize },
    /// Actor count past [`MAX_BUDGET_ACTORS`]; value carried.
    TooManyActors(usize),
    /// A config field out of range; the field name is carried.
    BadConfig(&'static str),
}

impl core::fmt::Display for BudgetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BudgetError::NonFinite(lane) => write!(f, "anim budget: non-finite float in {lane}"),
            BudgetError::NegativeDistance(v) => {
                write!(f, "anim budget: distance {v} below zero")
            }
            BudgetError::BadScreen(v) => write!(f, "anim budget: screen {v} outside 0..=1"),
            BudgetError::BadWeight(v) => write!(f, "anim budget: weight {v} not above zero"),
            BudgetError::NegativeMeasured(v) => {
                write!(f, "anim budget: measured {v} ms below zero")
            }
            BudgetError::LengthMismatch { expected, got } => {
                write!(f, "anim budget: lane count {got} differs from {expected}")
            }
            BudgetError::TooManyActors(n) => {
                write!(f, "anim budget: {n} actors past {MAX_BUDGET_ACTORS}")
            }
            BudgetError::BadConfig(field) => write!(f, "anim budget: config {field} out of range"),
        }
    }
}

impl std::error::Error for BudgetError {}

/// Allocator knobs. Defaults fit the potato tier; a stronger device raises
/// `budget_ms` through its scalability setting.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BudgetConfig {
    /// Frame budget for all animation sampling, in milliseconds.
    pub budget_ms: f32,
    /// Frames an actor keeps a rung before it may change again.
    pub hold_frames: u32,
    /// Going finer needs the plan under `budget_ms * headroom`, in `(0, 1]`.
    pub headroom: f32,
    /// Measured cost over `budget_ms * emergency` skips the hold for going
    /// coarser; at least 1.
    pub emergency: f32,
    /// Weight of the newest measurement in the learned unit cost, `(0, 1]`.
    pub smoothing: f32,
    /// Unit cost used until the first measurement is learned, in ms.
    pub initial_unit_ms: f32,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            budget_ms: 1.0,
            hold_frames: 30,
            headroom: 0.8,
            emergency: 2.5,
            smoothing: 0.25,
            initial_unit_ms: 0.05,
        }
    }
}

impl BudgetConfig {
    /// Refuse a config whose fields would break the allocator.
    pub fn validate(&self) -> Result<(), BudgetError> {
        if !(self.budget_ms.is_finite() && self.budget_ms > 0.0) {
            return Err(BudgetError::BadConfig("budget_ms"));
        }
        if !(self.headroom.is_finite() && self.headroom > 0.0 && self.headroom <= 1.0) {
            return Err(BudgetError::BadConfig("headroom"));
        }
        if !(self.emergency.is_finite() && self.emergency >= 1.0) {
            return Err(BudgetError::BadConfig("emergency"));
        }
        if !(self.smoothing.is_finite() && self.smoothing > 0.0 && self.smoothing <= 1.0) {
            return Err(BudgetError::BadConfig("smoothing"));
        }
        if !(self.initial_unit_ms.is_finite() && self.initial_unit_ms > 0.0) {
            return Err(BudgetError::BadConfig("initial_unit_ms"));
        }
        Ok(())
    }
}

/// One animated character as the allocator sees it this frame. The caller
/// keeps the same index for the same character across frames.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AnimActor {
    /// Finest tier geometry allows (from [`assign_tiers`](crate::lod::assign_tiers)
    /// or [`tier_for`](crate::lod::tier_for)); the budget only goes coarser.
    pub base: Tier,
    /// Distance to the camera in meters.
    pub distance_m: f32,
    /// Share of the screen the character covers, `0..=1`.
    pub screen_frac: f32,
    /// Relative cost of one full eval; `1.0` is the reference rig.
    pub weight: f32,
    /// The player's own character: never throttled.
    pub player: bool,
    /// The player's current opponent: never throttled.
    pub opponent: bool,
    /// In a fight right now: scores higher.
    pub fighting: bool,
}

impl AnimActor {
    /// A weight-1 background actor at full rate geometry.
    pub fn new(distance_m: f32, screen_frac: f32) -> Self {
        Self {
            base: Tier::Full,
            distance_m,
            screen_frac,
            weight: 1.0,
            player: false,
            opponent: false,
            fighting: false,
        }
    }

    /// True for the player and the current opponent: held at [`Tier::Full`].
    pub fn pinned(&self) -> bool {
        self.player || self.opponent
    }
}

/// Significance of one actor: higher keeps its rate longer. Screen size and
/// nearness each add up to 1, a fight adds [`FIGHT_BONUS`], a pinned actor
/// reads [`SIG_PINNED`]. Pure arithmetic, so equal inputs score equal.
pub fn significance(actor: &AnimActor) -> f32 {
    if actor.pinned() {
        return SIG_PINNED;
    }
    let screen = (actor.screen_frac / BIG_SCREEN).clamp(0.0, 1.0);
    let near = (1.0 - actor.distance_m / FAR_DIST_M).clamp(0.0, 1.0);
    let fight = if actor.fighting { FIGHT_BONUS } else { 0.0 };
    screen + near + fight
}

/// What one [`AnimBudget::step`] decided, for the frame-split line.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BudgetReport {
    /// Learned cost of one full eval of a weight-1 actor, in ms.
    pub unit_ms: f32,
    /// Predicted pose cost of the assignment just made, in ms.
    pub predicted_ms: f32,
    /// True when the prediction is still over budget (pinned actors or the
    /// hysteresis hold keep it there).
    pub over_budget: bool,
    /// True when the emergency path ran this frame.
    pub emergency: bool,
    /// Actors per rung after this frame: full, half, quarter, far.
    pub per_rung: [u32; RUNGS],
    /// Actors that changed rung this frame.
    pub changed: u32,
}

/// The allocator: per-actor rung and hold state plus the learned unit cost.
#[derive(Clone, Debug)]
pub struct AnimBudget {
    cfg: BudgetConfig,
    unit_ms: f32,
    learned: bool,
    last_load: f32,
    frame: u64,
    rungs: Vec<u8>,
    held: Vec<u32>,
    sig: Vec<f32>,
    order: Vec<usize>,
    floor: Vec<u8>,
    want_over: Vec<u8>,
    want_fit: Vec<u8>,
}

/// Greedy plan under `target_ms`: start every actor at its floor rung and
/// walk the ladder one rung per pass, least significant first, until the
/// predicted cost fits. Pinned actors never step. `out` ends holding the plan.
fn plan(
    actors: &[AnimActor],
    order: &[usize],
    floor: &[u8],
    unit_ms: f32,
    target_ms: f32,
    out: &mut [u8],
) {
    out.copy_from_slice(floor);
    let mut load = 0.0f32;
    for (actor, &rung) in actors.iter().zip(out.iter()) {
        load += actor.weight * rate_of(rung);
    }
    for level in 0..(RUNGS as u8 - 1) {
        if unit_ms * load <= target_ms {
            return;
        }
        for &i in order {
            if unit_ms * load <= target_ms {
                return;
            }
            if actors[i].pinned() || out[i] != level {
                continue;
            }
            out[i] = level + 1;
            load -= actors[i].weight * (rate_of(level) - rate_of(level + 1));
        }
    }
}

impl AnimBudget {
    /// New allocator; refuses a config with an out-of-range field.
    pub fn new(cfg: BudgetConfig) -> Result<Self, BudgetError> {
        cfg.validate()?;
        Ok(Self {
            unit_ms: cfg.initial_unit_ms,
            cfg,
            learned: false,
            last_load: 0.0,
            frame: 0,
            rungs: Vec::new(),
            held: Vec::new(),
            sig: Vec::new(),
            order: Vec::new(),
            floor: Vec::new(),
            want_over: Vec::new(),
            want_fit: Vec::new(),
        })
    }

    /// The config in force.
    pub fn config(&self) -> &BudgetConfig {
        &self.cfg
    }

    /// Change the budget (a device tier switch); other knobs stay.
    pub fn set_budget_ms(&mut self, budget_ms: f32) -> Result<(), BudgetError> {
        let next = BudgetConfig {
            budget_ms,
            ..self.cfg
        };
        next.validate()?;
        self.cfg = next;
        Ok(())
    }

    /// Learned cost of one full weight-1 eval, in ms.
    pub fn unit_ms(&self) -> f32 {
        self.unit_ms
    }

    /// Frames stepped so far; the tick a caller gates eval with.
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// True when actor `index` re-evaluates this frame under `tiers[index]`:
    /// the slot stagger of [`Tier::updates_this_tick`], so coarse actors do
    /// not all land on one frame.
    pub fn updates_this_frame(&self, tiers: &[Tier], index: usize) -> bool {
        tiers
            .get(index)
            .is_some_and(|tier| tier.updates_this_tick(self.frame.saturating_sub(1), index))
    }

    fn check(&self, actors: &[AnimActor], measured_ms: f32) -> Result<(), BudgetError> {
        if actors.len() > MAX_BUDGET_ACTORS {
            return Err(BudgetError::TooManyActors(actors.len()));
        }
        if !measured_ms.is_finite() {
            return Err(BudgetError::NonFinite("measured_ms"));
        }
        if measured_ms < 0.0 {
            return Err(BudgetError::NegativeMeasured(measured_ms));
        }
        for actor in actors {
            if !actor.distance_m.is_finite() {
                return Err(BudgetError::NonFinite("distance_m"));
            }
            if !actor.screen_frac.is_finite() {
                return Err(BudgetError::NonFinite("screen_frac"));
            }
            if !actor.weight.is_finite() {
                return Err(BudgetError::NonFinite("weight"));
            }
            if actor.distance_m < 0.0 {
                return Err(BudgetError::NegativeDistance(actor.distance_m));
            }
            if !(0.0..=1.0).contains(&actor.screen_frac) {
                return Err(BudgetError::BadScreen(actor.screen_frac));
            }
            if actor.weight <= 0.0 {
                return Err(BudgetError::BadWeight(actor.weight));
            }
        }
        Ok(())
    }

    /// Size the lanes to `n` actors; a new actor starts at its base rung and
    /// free to change, a count change forgets the load the cost was read on.
    fn fit_lanes(&mut self, actors: &[AnimActor]) {
        let n = actors.len();
        if self.rungs.len() == n {
            return;
        }
        let old = self.rungs.len();
        self.rungs.truncate(n);
        self.held.truncate(n);
        for actor in actors.iter().skip(old) {
            self.rungs.push(if actor.pinned() { 0 } else { rung_of(actor.base) });
            self.held.push(self.cfg.hold_frames);
        }
        self.sig.resize(n, 0.0);
        self.floor.resize(n, 0);
        self.want_over.resize(n, 0);
        self.want_fit.resize(n, 0);
        self.order.clear();
        self.order.extend(0..n);
        self.last_load = 0.0;
    }

    /// One frame: learn the unit cost from `measured_ms` (the pose cost of
    /// the previous frame's assignment), plan, apply hysteresis, and write
    /// one tier per actor into `out`.
    pub fn step(
        &mut self,
        actors: &[AnimActor],
        measured_ms: f32,
        out: &mut [Tier],
    ) -> Result<BudgetReport, BudgetError> {
        self.check(actors, measured_ms)?;
        if out.len() != actors.len() {
            return Err(BudgetError::LengthMismatch {
                expected: actors.len(),
                got: out.len(),
            });
        }
        self.fit_lanes(actors);

        if measured_ms > 0.0 && self.last_load > 0.0 {
            let sample = measured_ms / self.last_load;
            self.unit_ms = if self.learned {
                self.unit_ms + self.cfg.smoothing * (sample - self.unit_ms)
            } else {
                sample
            };
            self.learned = true;
        }

        for (i, actor) in actors.iter().enumerate() {
            self.sig[i] = significance(actor);
            self.floor[i] = if actor.pinned() { 0 } else { rung_of(actor.base) };
        }
        // Least significant first; ties by index so a tape sorts one way.
        let sig = &self.sig;
        self.order.sort_unstable_by(|&a, &b| {
            sig[a].total_cmp(&sig[b]).then_with(|| a.cmp(&b))
        });
        plan(
            actors,
            &self.order,
            &self.floor,
            self.unit_ms,
            self.cfg.budget_ms,
            &mut self.want_over,
        );
        plan(
            actors,
            &self.order,
            &self.floor,
            self.unit_ms,
            self.cfg.budget_ms * self.cfg.headroom,
            &mut self.want_fit,
        );

        let emergency = measured_ms > self.cfg.budget_ms * self.cfg.emergency;
        let mut changed = 0u32;
        for (i, actor) in actors.iter().enumerate() {
            self.held[i] = self.held[i].saturating_add(1);
            let cur = self.rungs[i];
            let next = if actor.pinned() {
                0
            } else if self.want_over[i] > cur
                && (emergency || self.held[i] >= self.cfg.hold_frames)
            {
                self.want_over[i]
            } else if self.want_fit[i] < cur && self.held[i] >= self.cfg.hold_frames {
                self.want_fit[i]
            } else {
                cur
            };
            if next != cur {
                self.rungs[i] = next;
                self.held[i] = 0;
                changed += 1;
            }
        }

        let mut load = 0.0f32;
        let mut per_rung = [0u32; RUNGS];
        for (i, actor) in actors.iter().enumerate() {
            let rung = self.rungs[i];
            out[i] = tier_of(rung);
            load += actor.weight * rate_of(rung);
            per_rung[usize::from(rung).min(RUNGS - 1)] += 1;
        }
        self.last_load = load;
        self.frame = self.frame.wrapping_add(1);
        let predicted_ms = self.unit_ms * load;
        Ok(BudgetReport {
            unit_ms: self.unit_ms,
            predicted_ms,
            over_budget: predicted_ms > self.cfg.budget_ms,
            emergency,
            per_rung,
            changed,
        })
    }
}

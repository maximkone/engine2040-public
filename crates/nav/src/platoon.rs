//! platoon: platoon-led legion march. One nav plan per leader per tick,
//! followers hold a formation slot in O(1) with no nav query each.
//!
//! Owner: nav crate. Consumer: the legion brain in `engine-ai` (squad orders)
//! and the arena legion bind. Each builds one [`Legion`] per army and calls
//! [`Legion::step`] once per tick, handing it the crate's own [`TiledPlanner`]
//! (or a recorder that counts plans). A 500-soldier legion that queries nav
//! once per soldier costs 500 plans a tick; ten platoons cost ten.
//!
//! Units: millimetres on the XZ plane (component 1 is the lateral Z axis),
//! millimetres per tick at [`TICKS_PER_SECOND`]. Spacing and steps are
//! positive millimetres.
//! Determinism: integer math only, platoon then follower order, no clock, no
//! RNG, no hash order. Two ticks of the same input give the same positions,
//! and a full script replays to the same [`Legion::digest`].
//! Errors: [`PlatoonError`] for bad orders and configs, [`PlanRefusal`] when a
//! plan source cannot answer. No caller data panics.
//! Facing: a platoon's [`Heading`] comes from its order and does not follow
//! the leader's motion, so a turn is a new order and never swings slots under
//! moving followers.
//! Near-field separation is not done here: run [`crate::orca`] after a step
//! when soldiers of different platoons share ground.

use std::fmt;

use engine_core::fixed::isqrt;
use engine_core::Fnv64;

use crate::tiles::{TileError, TiledNav};

/// Followers one platoon holds, beside its leader (a 50-soldier platoon).
pub const MAX_FOLLOWERS: usize = 49;
/// Platoons one legion holds.
pub const MAX_PLATOONS: usize = 64;
/// Smallest spacing between formation neighbours in mm.
pub const MIN_SPACING_MM: i32 = 400;
/// Largest spacing between formation neighbours in mm.
pub const MAX_SPACING_MM: i32 = 8_000;
/// Largest absolute coordinate a unit may hold in mm; keeps step math in
/// `i64` without overflow.
pub const MAX_COORD_MM: i32 = 1 << 20;
/// Ticks per second the march is written for. Single owner of the rate is
/// `engine_core::TICK_HZ`.
pub const TICKS_PER_SECOND: u32 = engine_core::TICK_HZ_U32;

/// The direction a platoon faces. Slots stand behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Heading {
    /// Faces `+X`; ranks stand at lower X.
    #[default]
    East,
    /// Faces `+Y` (the lateral Z axis); ranks stand at lower Y.
    North,
    /// Faces `-X`.
    West,
    /// Faces `-Y`.
    South,
}

/// One platoon to seat: where its leader stands and walks, how many followers
/// trail it, and which way it faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatoonOrder {
    /// Leader start in mm.
    pub leader_mm: [i32; 2],
    /// Goal the leader plans toward, in mm.
    pub goal_mm: [i32; 2],
    /// Followers beside the leader, `0..=MAX_FOLLOWERS`.
    pub followers: usize,
    /// Facing of the slot block; it does not follow the leader's motion.
    pub heading: Heading,
}

/// March tuning for a whole legion: slot spacing, files per rank, and the
/// per-tick speed caps. All lengths are positive millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarchConfig {
    /// Spacing between neighbours in a rank, and between ranks.
    pub spacing_mm: i32,
    /// Followers per rank, `1..=MAX_FOLLOWERS`.
    pub files: usize,
    /// Most a leader may move in one tick.
    pub leader_step_mm: i32,
    /// Most a follower may move in one tick; at least `leader_step_mm`, so a
    /// trailing rank can close a gap instead of falling behind forever.
    pub follower_step_mm: i32,
}

impl MarchConfig {
    /// Arena legion: 800 mm slots, 7 files, leader 100 mm/tick (6 m/s at
    /// 60 Hz), follower 200 mm/tick.
    pub const ARENA: Self = Self {
        spacing_mm: 800,
        files: 7,
        leader_step_mm: 100,
        follower_step_mm: 200,
    };

    /// Refuses spacing that cannot hold a gap and steps that cannot march.
    /// Errors: [`PlatoonError::BadConfig`].
    fn validate(&self) -> Result<(), PlatoonError> {
        check_range(
            "spacing_mm",
            self.spacing_mm,
            MIN_SPACING_MM,
            MAX_SPACING_MM,
        )?;
        let files = i64::try_from(self.files).unwrap_or(i64::MAX);
        if self.files == 0 || self.files > MAX_FOLLOWERS {
            return Err(PlatoonError::BadConfig {
                field: "files",
                got: files,
                min: 1,
                max: i64::try_from(MAX_FOLLOWERS).unwrap_or(i64::MAX),
            });
        }
        check_range("leader_step_mm", self.leader_step_mm, 1, self.spacing_mm)?;
        check_range(
            "follower_step_mm",
            self.follower_step_mm,
            self.leader_step_mm.max(2),
            self.spacing_mm,
        )?;
        Ok(())
    }
}

/// Why a [`PlanSource`] could not answer one leader request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanRefusal {
    /// A point lies outside the source's walkable area.
    Outside,
    /// The source holds no field for the goal, so it cannot steer there.
    NoField,
    /// Any other refusal, named.
    Refused {
        /// Short reason, for a log or a door message.
        why: &'static str,
    },
}

impl fmt::Display for PlanRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outside => write!(f, "platoon: point outside the walkable area"),
            Self::NoField => write!(f, "platoon: goal has no baked field"),
            Self::Refused { why } => write!(f, "platoon: plan refused ({why})"),
        }
    }
}

impl std::error::Error for PlanRefusal {}

/// Typed platoon failures; bad orders seat nothing and bad steps move nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatoonError {
    /// The order list held no platoon, so there is no leader to plan for.
    NoLeaders,
    /// More platoons than one legion accepts.
    TooManyPlatoons {
        /// Platoons offered.
        got: usize,
        /// Legion bound refused past.
        max: usize,
    },
    /// One platoon asked for more followers than it accepts.
    TooManyFollowers {
        /// Platoon index in the order list.
        platoon: usize,
        /// Followers offered.
        got: usize,
        /// Platoon bound refused past.
        max: usize,
    },
    /// A config value is outside its accepted range.
    BadConfig {
        /// Field name.
        field: &'static str,
        /// Value offered.
        got: i64,
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },
    /// A unit stands past [`MAX_COORD_MM`], where step math stops being exact.
    BadCoord {
        /// Platoon index in the order list.
        platoon: usize,
        /// Coordinate offered, in mm.
        coord: [i32; 2],
    },
    /// A plan source refused a leader request; no unit moved this tick.
    Plan {
        /// Platoon whose leader was refused.
        platoon: usize,
        /// What the source said.
        refusal: PlanRefusal,
    },
}

impl fmt::Display for PlatoonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoLeaders => write!(f, "platoon: no leaders in the legion"),
            Self::TooManyPlatoons { got, max } => {
                write!(f, "platoon: {got} platoons over bound {max}")
            }
            Self::TooManyFollowers { platoon, got, max } => {
                write!(
                    f,
                    "platoon: platoon {platoon} has {got} followers over bound {max}"
                )
            }
            Self::BadConfig {
                field,
                got,
                min,
                max,
            } => write!(f, "platoon: {field} {got} outside {min}..={max}"),
            Self::BadCoord { platoon, coord } => {
                write!(
                    f,
                    "platoon: platoon {platoon} at ({}, {}) mm",
                    coord[0], coord[1]
                )
            }
            Self::Plan { platoon, refusal } => {
                write!(f, "platoon: leader {platoon} refused: {refusal}")
            }
        }
    }
}

impl std::error::Error for PlatoonError {}

/// One leader step from a plan source: the next point in mm, `None` when the
/// leader is at its goal (or has no step left), or a typed refusal.
pub type PlanStep = Result<Option<[i32; 2]>, PlanRefusal>;

/// The one nav query a platoon leader makes each tick.
///
/// A [`Legion`] calls this once per platoon per tick and never once per
/// follower. Implement it over the real nav runtime (see [`TiledPlanner`]) or
/// wrap one to count plans in a test.
pub trait PlanSource {
    /// Plans one leader step from `from_mm` toward `goal_mm`.
    ///
    /// `Ok(Some(next))` is the next point to head for, `Ok(None)` means
    /// "no step" (arrived or sealed), and `Err` is a typed refusal. The
    /// legion clamps the step to its own per-tick cap, so a source may return
    /// a far waypoint.
    fn plan(&mut self, from_mm: [i32; 2], goal_mm: [i32; 2]) -> PlanStep;
}

/// [`PlanSource`] over the tiled nav runtime: one [`TiledNav::direction`] read
/// per leader per tick.
///
/// It never bakes a path: the caller bakes a flow field for each goal
/// ([`TiledNav::bake_goal`]) once, and every platoon leader then reads it. The
/// planner converts millimetres to cells with `cell_mm` and answers with the
/// next cell centre in millimetres.
pub struct TiledPlanner<'a> {
    nav: &'a TiledNav,
    cell_mm: i32,
}

impl<'a> TiledPlanner<'a> {
    /// Wraps `nav`, converting millimetres with `cell_mm` (positive, at most
    /// [`MAX_SPACING_MM`]).
    /// Errors: [`PlatoonError::BadConfig`] for a cell outside that range.
    pub fn new(nav: &'a TiledNav, cell_mm: i32) -> Result<Self, PlatoonError> {
        check_range("cell_mm", cell_mm, 1, MAX_SPACING_MM)?;
        Ok(Self { nav, cell_mm })
    }

    /// Cell holding a millimetre point, or a refusal off the grid.
    fn cell_of(&self, mm: [i32; 2]) -> Result<(usize, usize), PlanRefusal> {
        let x = mm[0].div_euclid(self.cell_mm);
        let y = mm[1].div_euclid(self.cell_mm);
        if x < 0 || y < 0 {
            return Err(PlanRefusal::Outside);
        }
        let x = usize::try_from(x).map_err(|_| PlanRefusal::Outside)?;
        let y = usize::try_from(y).map_err(|_| PlanRefusal::Outside)?;
        Ok((x, y))
    }

    /// Centre of a cell in millimetres.
    fn center_mm(&self, cell: (usize, usize)) -> [i32; 2] {
        let half = i64::from(self.cell_mm) / 2;
        let x = i64::try_from(cell.0).unwrap_or(i64::MAX);
        let y = i64::try_from(cell.1).unwrap_or(i64::MAX);
        [
            saturate_i32(x * i64::from(self.cell_mm) + half),
            saturate_i32(y * i64::from(self.cell_mm) + half),
        ]
    }
}

impl PlanSource for TiledPlanner<'_> {
    fn plan(&mut self, from_mm: [i32; 2], goal_mm: [i32; 2]) -> PlanStep {
        let from = self.cell_of(from_mm)?;
        let goal = self.cell_of(goal_mm)?;
        match self.nav.direction(goal, from) {
            Ok(Some(next)) => Ok(Some(self.center_mm(next))),
            // The field is spent once the leader stands in the goal cell, but
            // that cell is fully walkable: finish the walk to the exact goal
            // point rather than halting up to a cell short.
            Ok(None) if from == goal => Ok((from_mm != goal_mm).then_some(goal_mm)),
            Ok(None) => Ok(None),
            Err(TileError::NoField) => Err(PlanRefusal::NoField),
            Err(TileError::OutOfBounds) => Err(PlanRefusal::Outside),
            Err(_) => Err(PlanRefusal::Refused {
                why: "tiled nav refused the steer query",
            }),
        }
    }
}

/// One army in platoon order: a leader per platoon plans, its followers hold
/// slot offsets from it.
///
/// [`Legion::step`] asks each leader's plan once, moves each leader by at most
/// the config's leader step, then moves every follower toward
/// `leader + rotate(slot)` by at most the follower step. Followers never query
/// nav, and the cost per follower is one add plus one clamped step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legion {
    platoons: Vec<Platoon>,
    config: MarchConfig,
    /// Scratch lane for this tick's planned leader targets; keeps capacity.
    planned: Vec<[i32; 2]>,
    ticks: u32,
}

/// One platoon: leader, goal, facing, precomputed slot offsets, and followers.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Platoon {
    leader: [i32; 2],
    goal: [i32; 2],
    slots: Vec<[i32; 2]>,
    followers: Vec<[i32; 2]>,
}

impl Legion {
    /// Seats a legion from platoon orders. Followers muster on their leader
    /// and walk to their slots over the first ticks.
    ///
    /// Errors: [`PlatoonError::NoLeaders`],
    /// [`PlatoonError::TooManyPlatoons`], [`PlatoonError::TooManyFollowers`],
    /// [`PlatoonError::BadConfig`], [`PlatoonError::BadCoord`]. A refused
    /// order list seats nothing.
    ///
    /// ```
    /// use engine_nav::platoon::{Heading, Legion, MarchConfig, PlatoonOrder};
    ///
    /// let orders = [PlatoonOrder {
    ///     leader_mm: [0, 0],
    ///     goal_mm: [10_000, 0],
    ///     followers: 9,
    ///     heading: Heading::East,
    /// }];
    /// let legion = Legion::new(&orders, MarchConfig::ARENA).expect("one platoon seats");
    /// assert_eq!(legion.platoon_count(), 1);
    /// assert_eq!(legion.soldier_count(), 10);
    /// ```
    pub fn new(orders: &[PlatoonOrder], config: MarchConfig) -> Result<Self, PlatoonError> {
        if orders.is_empty() {
            return Err(PlatoonError::NoLeaders);
        }
        if orders.len() > MAX_PLATOONS {
            return Err(PlatoonError::TooManyPlatoons {
                got: orders.len(),
                max: MAX_PLATOONS,
            });
        }
        config.validate()?;
        let mut platoons = Vec::with_capacity(orders.len());
        for (index, order) in orders.iter().enumerate() {
            if order.followers > MAX_FOLLOWERS {
                return Err(PlatoonError::TooManyFollowers {
                    platoon: index,
                    got: order.followers,
                    max: MAX_FOLLOWERS,
                });
            }
            check_coord(index, order.leader_mm)?;
            check_coord(index, order.goal_mm)?;
            let mut slots = Vec::with_capacity(order.followers);
            for follower in 0..order.followers {
                slots.push(rotate(
                    slot_offset(follower, config.files, config.spacing_mm),
                    order.heading,
                ));
            }
            platoons.push(Platoon {
                leader: order.leader_mm,
                goal: order.goal_mm,
                slots,
                followers: vec![order.leader_mm; order.followers],
            });
        }
        let planned = Vec::with_capacity(platoons.len());
        Ok(Self {
            platoons,
            config,
            planned,
            ticks: 0,
        })
    }

    /// Steps the legion one tick: one plan per leader, then one bounded move
    /// per soldier.
    ///
    /// Every leader plans before any unit moves, so a refusal returns
    /// [`PlatoonError::Plan`] with no unit moved (earlier leaders' plan calls
    /// already happened; the source owns its own side effects).
    pub fn step(&mut self, source: &mut impl PlanSource) -> Result<(), PlatoonError> {
        self.planned.clear();
        for (index, platoon) in self.platoons.iter().enumerate() {
            let next = source
                .plan(platoon.leader, platoon.goal)
                .map_err(|refusal| PlatoonError::Plan {
                    platoon: index,
                    refusal,
                })?;
            self.planned.push(next.unwrap_or(platoon.leader));
        }
        for (platoon, target) in self.platoons.iter_mut().zip(self.planned.iter()) {
            platoon.leader = step_toward(platoon.leader, *target, self.config.leader_step_mm);
            let leader = platoon.leader;
            for (follower, slot) in platoon.followers.iter_mut().zip(platoon.slots.iter()) {
                let want = [leader[0] + slot[0], leader[1] + slot[1]];
                *follower = step_toward(*follower, want, self.config.follower_step_mm);
            }
        }
        self.ticks = self.ticks.saturating_add(1);
        Ok(())
    }

    /// Ticks this legion has stepped.
    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    /// Platoon count.
    pub fn platoon_count(&self) -> usize {
        self.platoons.len()
    }

    /// Soldiers in the legion, leaders included.
    pub fn soldier_count(&self) -> usize {
        self.platoons
            .iter()
            .map(|platoon| platoon.followers.len() + 1)
            .sum()
    }

    /// Leader position of `platoon`, in mm; `None` past the last platoon.
    pub fn leader(&self, platoon: usize) -> Option<[i32; 2]> {
        self.platoons.get(platoon).map(|p| p.leader)
    }

    /// Follower position in mm; `None` past the last platoon or follower.
    pub fn follower(&self, platoon: usize, index: usize) -> Option<[i32; 2]> {
        self.platoons
            .get(platoon)
            .and_then(|p| p.followers.get(index))
            .copied()
    }

    /// The slot a follower holds now: leader plus its rotated offset, in mm.
    pub fn slot(&self, platoon: usize, index: usize) -> Option<[i32; 2]> {
        let p = self.platoons.get(platoon)?;
        let slot = p.slots.get(index)?;
        Some([p.leader[0] + slot[0], p.leader[1] + slot[1]])
    }

    /// Goal of `platoon`, in mm; `None` past the last platoon.
    pub fn goal(&self, platoon: usize) -> Option<[i32; 2]> {
        self.platoons.get(platoon).map(|p| p.goal)
    }

    /// Largest distance in mm between a follower and the slot it holds; `0`
    /// when every follower is seated. The accordion measure: a platoon that
    /// stretches and contracts without bound grows this.
    pub fn max_slot_error_mm(&self) -> i32 {
        let mut worst = 0;
        for platoon in &self.platoons {
            for (follower, slot) in platoon.followers.iter().zip(platoon.slots.iter()) {
                let want = [platoon.leader[0] + slot[0], platoon.leader[1] + slot[1]];
                worst = worst.max(dist_mm(*follower, want));
            }
        }
        worst
    }

    /// Canonical digest of the tick count and every position in platoon then
    /// follower order. Same script and same inputs give the same digest.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u32(self.ticks);
        hasher.write_len32(self.platoons.len());
        for platoon in &self.platoons {
            hasher.write_i32(platoon.leader[0]);
            hasher.write_i32(platoon.leader[1]);
            hasher.write_len32(platoon.followers.len());
            for follower in &platoon.followers {
                hasher.write_i32(follower[0]);
                hasher.write_i32(follower[1]);
            }
        }
        hasher.finish()
    }
}

/// Refuses a config value outside `min..=max`.
/// Errors: [`PlatoonError::BadConfig`].
fn check_range(field: &'static str, got: i32, min: i32, max: i32) -> Result<(), PlatoonError> {
    if got < min || got > max {
        return Err(PlatoonError::BadConfig {
            field,
            got: i64::from(got),
            min: i64::from(min),
            max: i64::from(max),
        });
    }
    Ok(())
}

/// Refuses a point past [`MAX_COORD_MM`].
/// Errors: [`PlatoonError::BadCoord`].
fn check_coord(platoon: usize, coord: [i32; 2]) -> Result<(), PlatoonError> {
    for axis in coord {
        if axis.abs() > MAX_COORD_MM {
            return Err(PlatoonError::BadCoord { platoon, coord });
        }
    }
    Ok(())
}

/// Local slot offset for follower `index`: rank `index / files` behind the
/// leader, files spread across the lateral axis. Landed once per follower, so
/// a tick only adds it to the leader.
fn slot_offset(index: usize, files: usize, spacing_mm: i32) -> [i32; 2] {
    let files = files.max(1);
    let file = index % files;
    let rank = index / files;
    let lateral = centered_step(file, files) * spacing_mm / 2;
    let depth = i32::try_from(rank)
        .unwrap_or(i32::MAX)
        .saturating_add(1)
        .saturating_mul(spacing_mm);
    [-depth, lateral]
}

/// Centered step for file `index` of `count`: `2*i+1-n`, so file 0 of 1 sits
/// on the axis and wider ranks spread symmetrically. Total on any input.
fn centered_step(index: usize, count: usize) -> i32 {
    let index = i64::try_from(index).unwrap_or(i64::MAX);
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    saturate_i32(
        index
            .saturating_mul(2)
            .saturating_add(1)
            .saturating_sub(count),
    )
}

/// Rotates a local offset into world axes for `heading`. Exact for quarter
/// turns, so no slot drifts.
fn rotate(offset: [i32; 2], heading: Heading) -> [i32; 2] {
    let [x, y] = offset;
    match heading {
        Heading::East => [x, y],
        Heading::North => [-y, x],
        Heading::West => [-x, -y],
        Heading::South => [y, -x],
    }
}

/// Moves `from` toward `to` by at most `step_mm`, landing exactly on `to` when
/// it is within reach. Truncating integer division never overshoots, so a
/// follower cannot oscillate around its slot. Callers keep `step_mm >= 2`.
fn step_toward(from: [i32; 2], to: [i32; 2], step_mm: i32) -> [i32; 2] {
    let dx = i64::from(to[0]) - i64::from(from[0]);
    let dy = i64::from(to[1]) - i64::from(from[1]);
    if dx == 0 && dy == 0 {
        return to;
    }
    let dist = i64::try_from(isqrt(square(dx, dy))).unwrap_or(i64::MAX);
    if dist <= i64::from(step_mm) {
        return to;
    }
    let step = i64::from(step_mm);
    [
        saturate_i32(i64::from(from[0]) + dx * step / dist),
        saturate_i32(i64::from(from[1]) + dy * step / dist),
    ]
}

/// `dx * dx + dy * dy` as `u128`; coordinated points stay under `2^21` mm per
/// axis, so the `i64` square cannot overflow.
fn square(dx: i64, dy: i64) -> u128 {
    u128::try_from(dx * dx + dy * dy).unwrap_or(u128::MAX)
}

/// Distance in mm between two points, integer floor, no float.
fn dist_mm(a: [i32; 2], b: [i32; 2]) -> i32 {
    let dx = i64::from(a[0]) - i64::from(b[0]);
    let dy = i64::from(a[1]) - i64::from(b[1]);
    saturate_i32(i64::try_from(isqrt(square(dx, dy))).unwrap_or(i64::MAX))
}

/// Narrows an `i64` to `i32`, holding the bound instead of wrapping.
fn saturate_i32(value: i64) -> i32 {
    match i32::try_from(value) {
        Ok(fits) => fits,
        Err(_) if value < 0 => i32::MIN,
        Err(_) => i32::MAX,
    }
}

#[cfg(test)]
mod tests {
    //! Measured platoon economics: plans per tick, slot error over 600 ticks
    //! at 60 Hz, and a byte-identical replay. The counting seam wraps the real
    //! [`TiledPlanner`], so the plan counts are counts of real nav queries.

    use super::*;

    /// Cell side of the test field in mm.
    const CELL_MM: i32 = 250;
    /// The stated accordion bar: max follower distance from its slot in mm,
    /// measured after warmup. A platoon that stretches past this has an
    /// accordion.
    const BAR_SLOT_ERROR_MM: i32 = 250;
    /// Grid the march runs over (cells).
    const GRID: (usize, usize) = (96, 32);
    /// Shared goal cell every test leader plans toward.
    const GOAL_CELL: (usize, usize) = (90, 8);

    /// Counting seam: increments before delegating to the real planner.
    struct Counting<'a> {
        planner: TiledPlanner<'a>,
        calls: u64,
    }

    impl<'a> Counting<'a> {
        fn new(nav: &'a TiledNav) -> Self {
            Self {
                planner: TiledPlanner::new(nav, CELL_MM).expect("test cell side is valid"),
                calls: 0,
            }
        }
    }

    impl PlanSource for Counting<'_> {
        fn plan(&mut self, from_mm: [i32; 2], goal_mm: [i32; 2]) -> PlanStep {
            self.calls += 1;
            self.planner.plan(from_mm, goal_mm)
        }
    }

    /// Open grid with one baked goal field.
    fn test_nav() -> TiledNav {
        let blocked = vec![false; GRID.0 * GRID.1];
        let mut nav = TiledNav::bake(&blocked, GRID.0, GRID.1, 8).expect("open grid bakes");
        nav.bake_goal(GOAL_CELL)
            .expect("goal bakes on an open grid");
        nav
    }

    /// Centre of a cell in test millimetres.
    fn center_mm(cell: (usize, usize)) -> [i32; 2] {
        let half = CELL_MM / 2;
        [
            saturate_i32(
                i64::try_from(cell.0).unwrap_or(i64::MAX) * i64::from(CELL_MM) + i64::from(half),
            ),
            saturate_i32(
                i64::try_from(cell.1).unwrap_or(i64::MAX) * i64::from(CELL_MM) + i64::from(half),
            ),
        ]
    }

    /// One platoon order starting on `row`, all marching to the shared goal.
    fn order(row: usize) -> PlatoonOrder {
        PlatoonOrder {
            leader_mm: center_mm((2, row)),
            goal_mm: center_mm(GOAL_CELL),
            followers: MAX_FOLLOWERS,
            heading: Heading::East,
        }
    }

    /// Ten platoons of fifty: the 500-soldier legion.
    fn legion_orders() -> Vec<PlatoonOrder> {
        (0..10).map(|index| order(8 + index * 2)).collect()
    }

    #[test]
    fn platoon_issues_fifty_times_fewer_plans_than_independent_soldiers() {
        let nav = test_nav();
        let mut counting = Counting::new(&nav);
        let mut legion =
            Legion::new(&legion_orders(), MarchConfig::ARENA).expect("500 soldiers seat");
        assert_eq!(legion.soldier_count(), 500, "the legion is 500 soldiers");
        assert_eq!(legion.platoon_count(), 10, "ten platoons of fifty");

        const TICKS: u64 = 60;
        for _ in 0..TICKS {
            legion.step(&mut counting).expect("every leader plans");
        }
        let platoon_plans = counting.calls;
        assert_eq!(
            platoon_plans,
            TICKS * 10,
            "one plan per leader per tick: {TICKS} ticks x 10 leaders"
        );

        // The naive army: each of the 500 soldiers asks nav for itself.
        let goal = center_mm(GOAL_CELL);
        let mut naive_plans = 0;
        for tick in 0..TICKS {
            for soldier in 0..500 {
                let column = 2 + (soldier % 80);
                let row = 8 + (soldier / 80) * 3;
                let from = center_mm((column, row + usize::try_from(tick % 3).unwrap_or(0)));
                let _ = counting.plan(from, goal);
                naive_plans += 1;
            }
        }
        let ratio = naive_plans / platoon_plans;
        println!("platoon: naive_plans={naive_plans} platoon_plans={platoon_plans} ratio={ratio}x");
        assert_eq!(naive_plans, 30_000, "500 soldiers x 60 ticks");
        assert!(
            ratio >= 5,
            "accept bar is 5x: naive={naive_plans} platoon={platoon_plans} ratio={ratio}x"
        );
    }

    #[test]
    fn platoon_holds_slot_spacing_for_six_hundred_ticks() {
        let nav = test_nav();
        let mut counting = Counting::new(&nav);
        let mut legion = Legion::new(&[order(8)], MarchConfig::ARENA).expect("platoon seats");
        let initial = legion.max_slot_error_mm();
        assert!(
            initial > BAR_SLOT_ERROR_MM,
            "the platoon musters on the leader, so it starts outside the {BAR_SLOT_ERROR_MM} mm bar: {initial} mm"
        );

        const TICKS: usize = 600;
        let mut after_warmup = 0;
        let mut settled = 0;
        let mut windows = [0i32; 3];
        for tick in 0..TICKS {
            legion.step(&mut counting).expect("leader plans");
            let error = legion.max_slot_error_mm();
            if tick >= 300 {
                after_warmup = after_warmup.max(error);
            }
            if tick >= 480 {
                settled = settled.max(error);
            }
            if tick >= 300 {
                let window = (tick - 300) / 100;
                if let Some(slot) = windows.get_mut(window) {
                    *slot = (*slot).max(error);
                }
            }
        }
        assert_eq!(legion.ticks(), 600, "sixty ticks a second for ten seconds");
        assert_eq!(
            legion.leader(0),
            Some(center_mm(GOAL_CELL)),
            "the leader stops on the goal cell centre"
        );
        // Independent oracle: the accessor pair agrees with the batch measure.
        let mut widest = 0;
        for index in 0..MAX_FOLLOWERS {
            let at = legion.follower(0, index).expect("follower exists");
            let slot = legion.slot(0, index).expect("slot exists");
            widest = widest.max(dist_mm(at, slot));
        }
        assert_eq!(
            widest,
            legion.max_slot_error_mm(),
            "the per-follower measurement matches the batch one"
        );
        println!(
            "platoon: initial={initial}mm after_warmup={after_warmup}mm settled={settled}mm bar={BAR_SLOT_ERROR_MM}mm windows={windows:?}"
        );
        assert!(
            after_warmup <= BAR_SLOT_ERROR_MM,
            "bar {BAR_SLOT_ERROR_MM} mm: measured {after_warmup} mm after 300 ticks"
        );
        assert!(
            settled <= after_warmup,
            "no growth: settled {settled} mm must not exceed {after_warmup} mm"
        );
        assert!(
            windows[0] >= windows[1] && windows[1] >= windows[2],
            "no oscillation: window maxima {windows:?} must not rise"
        );
        assert_eq!(
            legion.max_slot_error_mm(),
            0,
            "a halted leader lets every follower land exactly on its slot"
        );
        assert_eq!(
            counting.calls, 600,
            "only the leader planned: 600 ticks, one plan each"
        );
    }

    #[test]
    fn replay_twice_gives_the_same_digest() {
        let nav = test_nav();
        let config = MarchConfig::ARENA;
        let orders = legion_orders();

        let run = || {
            let mut counting = Counting::new(&nav);
            let mut legion = Legion::new(&orders, config).expect("legion seats");
            let mut digests = Vec::with_capacity(600);
            for _ in 0..600 {
                legion.step(&mut counting).expect("leader plans");
                digests.push(legion.digest());
            }
            (digests, counting.calls, legion)
        };
        let (first, first_plans, first_legion) = run();
        let (second, second_plans, second_legion) = run();
        assert_eq!(
            first, second,
            "the same script gives the same digest each tick"
        );
        assert_eq!(
            first_legion, second_legion,
            "the same script gives the same positions"
        );
        assert_eq!(
            first_plans, second_plans,
            "the same script asks the same plans"
        );
        assert_ne!(first[0], first[599], "the march actually changes state");
        println!(
            "platoon: digest0={:016x} digest599={:016x} plans={first_plans}",
            first[0], first[599]
        );
    }

    #[test]
    fn bad_orders_return_typed_errors() {
        let config = MarchConfig::ARENA;
        assert_eq!(Legion::new(&[], config), Err(PlatoonError::NoLeaders));

        let too_many = [PlatoonOrder {
            followers: MAX_FOLLOWERS + 1,
            ..order(8)
        }];
        assert_eq!(
            Legion::new(&too_many, config),
            Err(PlatoonError::TooManyFollowers {
                platoon: 0,
                got: MAX_FOLLOWERS + 1,
                max: MAX_FOLLOWERS,
            })
        );

        let crowd = vec![order(8); MAX_PLATOONS + 1];
        assert_eq!(
            Legion::new(&crowd, config),
            Err(PlatoonError::TooManyPlatoons {
                got: MAX_PLATOONS + 1,
                max: MAX_PLATOONS,
            })
        );

        let zero_spacing = MarchConfig {
            spacing_mm: 0,
            ..config
        };
        assert!(matches!(
            Legion::new(&[order(8)], zero_spacing),
            Err(PlatoonError::BadConfig {
                field: "spacing_mm",
                ..
            })
        ));

        let slow_followers = MarchConfig {
            follower_step_mm: config.leader_step_mm - 1,
            ..config
        };
        assert!(matches!(
            Legion::new(&[order(8)], slow_followers),
            Err(PlatoonError::BadConfig {
                field: "follower_step_mm",
                ..
            })
        ));

        let off_map = [PlatoonOrder {
            leader_mm: [MAX_COORD_MM + 1, 0],
            ..order(8)
        }];
        assert!(matches!(
            Legion::new(&off_map, config),
            Err(PlatoonError::BadCoord { platoon: 0, .. })
        ));

        // A refused plan is typed and moves nothing.
        let blocked = [false; 16];
        let nav = TiledNav::bake(&blocked, 4, 4, 4).expect("tiny grid bakes");
        let mut legion = Legion::new(
            &[PlatoonOrder {
                leader_mm: [0, 0],
                goal_mm: [0, 0],
                followers: 1,
                heading: Heading::East,
            }],
            config,
        )
        .expect("one platoon seats");
        let before = legion.digest();
        let mut cold = TiledPlanner::new(&nav, CELL_MM).expect("planner cell is valid");
        assert_eq!(
            legion.step(&mut cold),
            Err(PlatoonError::Plan {
                platoon: 0,
                refusal: PlanRefusal::NoField,
            })
        );
        assert_eq!(legion.digest(), before, "a refused plan moves nothing");
    }
}

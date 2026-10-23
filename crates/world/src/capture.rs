//! capture: the territory state machine - owner, contest, progress, score (RY4
//! of `docs/plan/ryl1.md`).
//!
//! Consumer: none yet; the faction match loop (RY5) hands one occupancy row per
//! point per tick to [`CaptureSet::step`] and reads the score. Data owner:
//! `engine-world`; the point table comes from `content/maps/` (the three
//! `kind: "statue"` nodes of `contested_middle.json`), this module only holds
//! the state. Units: integer centimetres for position and radius, ticks for
//! hold and progress, standing bodies as `u16` counts. Determinism: integer
//! math only, points keep their authored order and the hash walks that same
//! order, so one occupancy script gives the same score and the same hash on
//! every PC and on wasm.
//!
//! A point flips when one faction stands in its ring unchallenged for
//! `hold_ticks` ticks. Two factions in one ring contest it and progress
//! freezes. An empty ring decays progress back toward the owner, so walking
//! away from a half-taken point gives it back. The score tick reads the
//! points, never the bodies: one owned point is one score per tick, and the
//! hash carries the owners, the progress and the score, never a body id.
//!
//! Errors: typed [`CaptureError`]; a refused set keeps nothing. Budget: one
//! pass over the points per tick, no scan of bodies, no allocation.

use engine_core::Fnv64;
use std::fmt;

/// Most points one set holds; the faction map has three statues and two forts.
pub const MAX_POINTS: usize = 64;

/// The two asymmetric factions. Not a mirror: the numbers differ in the class
/// sheets, the silhouettes differ in `content/design/blockout/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Faction {
    /// Shield, line, slow.
    Wardens,
    /// Reach, burst, fragile.
    Ashkin,
}

impl Faction {
    /// Both factions, in the index order every table in this module uses.
    pub const ALL: [Faction; 2] = [Faction::Wardens, Faction::Ashkin];

    /// Index into the per-faction tables (`score`, an occupancy row).
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Faction::Wardens => 0,
            Faction::Ashkin => 1,
        }
    }

    /// Faction at `index`, or `None` past [`Faction::ALL`].
    #[must_use]
    pub fn from_index(index: usize) -> Option<Faction> {
        match index {
            0 => Some(Faction::Wardens),
            1 => Some(Faction::Ashkin),
            _ => None,
        }
    }
}

impl fmt::Display for Faction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Faction::Wardens => write!(f, "Wardens"),
            Faction::Ashkin => write!(f, "Ashkin"),
        }
    }
}

/// One capturable point: where it stands, how wide its ring is, how long an
/// unchallenged faction needs to take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapturePoint {
    /// Map id, unique in the set; quoted by every refusal.
    pub id: u32,
    /// Centre of the ring, integer centimetres from the map's metres.
    pub x: i32,
    /// Centre of the ring, integer centimetres from the map's metres.
    pub z: i32,
    /// Ring radius in centimetres; a body inside it is present.
    pub radius: i32,
    /// Ticks of unchallenged presence that flip the point.
    pub hold_ticks: u32,
}

/// One point's live state, borrowed from the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointView<'a> {
    /// The authored point this view reads.
    pub point: &'a CapturePoint,
    /// Faction that owns the point, `None` while it is neutral.
    pub owner: Option<Faction>,
    /// Faction currently taking it, `None` while it is held or idle.
    pub challenger: Option<Faction>,
    /// Ticks the challenger has banked, `0` when it flips back.
    pub progress: u32,
    /// Both factions stood in the ring on the last tick.
    pub contested: bool,
}

/// Why a set or a step was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// No points at all; a set needs at least one.
    Empty,
    /// More than [`MAX_POINTS`] points.
    TooManyPoints {
        /// Points handed in.
        count: usize,
        /// The cap.
        max: usize,
    },
    /// Two points share an id.
    DuplicateId(u32),
    /// A ring radius of zero or less.
    BadRadius {
        /// The offending point.
        id: u32,
        /// The offending radius.
        radius: i32,
    },
    /// A hold of zero ticks would flip on the first tick.
    NoHold {
        /// The offending point.
        id: u32,
    },
    /// The occupancy rows do not match the point count.
    OccupancyMismatch {
        /// Points in the set.
        want: usize,
        /// Rows handed in.
        got: usize,
    },
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CaptureError::Empty => write!(f, "a capture set needs at least one point"),
            CaptureError::TooManyPoints { count, max } => {
                write!(f, "{count} points exceeds the {max} point cap")
            }
            CaptureError::DuplicateId(id) => write!(f, "duplicate capture point id {id}"),
            CaptureError::BadRadius { id, radius } => {
                write!(f, "point {id} has ring radius {radius} cm")
            }
            CaptureError::NoHold { id } => {
                write!(
                    f,
                    "point {id} holds zero ticks, it would flip on the first tick"
                )
            }
            CaptureError::OccupancyMismatch { want, got } => {
                write!(f, "occupancy has {got} rows, the set holds {want} points")
            }
        }
    }
}

impl std::error::Error for CaptureError {}

/// Every capturable point and its state. Points keep their authored order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSet {
    points: Vec<CapturePoint>,
    owner: Vec<Option<Faction>>,
    challenger: Vec<Option<Faction>>,
    progress: Vec<u32>,
    contested: Vec<bool>,
    score: [u32; 2],
    ticks: u64,
}

impl CaptureSet {
    /// A set of neutral points, or the first reason the table is refused.
    ///
    /// # Errors
    /// [`CaptureError`] on an empty, duplicated, oversized or malformed table.
    pub fn new(points: Vec<CapturePoint>) -> Result<Self, CaptureError> {
        if points.is_empty() {
            return Err(CaptureError::Empty);
        }
        if points.len() > MAX_POINTS {
            return Err(CaptureError::TooManyPoints {
                count: points.len(),
                max: MAX_POINTS,
            });
        }
        for (index, point) in points.iter().enumerate() {
            if point.radius <= 0 {
                return Err(CaptureError::BadRadius {
                    id: point.id,
                    radius: point.radius,
                });
            }
            if point.hold_ticks == 0 {
                return Err(CaptureError::NoHold { id: point.id });
            }
            if points[..index].iter().any(|other| other.id == point.id) {
                return Err(CaptureError::DuplicateId(point.id));
            }
        }
        let count = points.len();
        Ok(Self {
            points,
            owner: vec![None; count],
            challenger: vec![None; count],
            progress: vec![0; count],
            contested: vec![false; count],
            score: [0; 2],
            ticks: 0,
        })
    }

    /// The authored points, in order.
    #[must_use]
    pub fn points(&self) -> &[CapturePoint] {
        &self.points
    }

    /// One point's live state, or `None` past the set.
    #[must_use]
    pub fn view(&self, index: usize) -> Option<PointView<'_>> {
        let point = self.points.get(index)?;
        Some(PointView {
            point,
            owner: self.owner[index],
            challenger: self.challenger[index],
            progress: self.progress[index],
            contested: self.contested[index],
        })
    }

    /// Faction that owns `index`, `None` while it is neutral.
    #[must_use]
    pub fn owner(&self, index: usize) -> Option<Faction> {
        self.owner.get(index).copied().flatten()
    }

    /// Faction currently taking `index`, `None` while it is held or idle.
    #[must_use]
    pub fn challenger(&self, index: usize) -> Option<Faction> {
        self.challenger.get(index).copied().flatten()
    }

    /// Ticks the challenger at `index` has banked.
    #[must_use]
    pub fn progress(&self, index: usize) -> u32 {
        self.progress.get(index).copied().unwrap_or(0)
    }

    /// Score of one faction: ticks it has held a point, summed over points.
    #[must_use]
    pub fn score(&self, faction: Faction) -> u32 {
        self.score[faction.index()]
    }

    /// Points one faction owns right now.
    #[must_use]
    pub fn points_owned(&self, faction: Faction) -> usize {
        self.owner
            .iter()
            .filter(|owner| **owner == Some(faction))
            .count()
    }

    /// Ticks stepped so far.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// One tick: `present[index][faction.index()]` is how many of that
    /// faction's bodies stand in the ring. The caller counts bodies (a spatial
    /// read), this module never scans them.
    ///
    /// # Errors
    /// [`CaptureError::OccupancyMismatch`] when the rows do not match the set.
    pub fn step(&mut self, present: &[[u16; 2]]) -> Result<(), CaptureError> {
        if present.len() != self.points.len() {
            return Err(CaptureError::OccupancyMismatch {
                want: self.points.len(),
                got: present.len(),
            });
        }
        self.ticks = self.ticks.saturating_add(1);
        for (index, point) in self.points.iter().enumerate() {
            let wardens = present[index][Faction::Wardens.index()];
            let ashkin = present[index][Faction::Ashkin.index()];
            self.contested[index] = wardens > 0 && ashkin > 0;
            if self.contested[index] {
                // two factions in one ring: nobody takes it
                continue;
            }
            let alone = if wardens > 0 {
                Some(Faction::Wardens)
            } else if ashkin > 0 {
                Some(Faction::Ashkin)
            } else {
                None
            };
            match alone {
                Some(faction) => {
                    if self.owner[index] == Some(faction) {
                        // its own faction stands in it: the challenge is over
                        self.challenger[index] = None;
                        self.progress[index] = 0;
                    } else {
                        if self.challenger[index] != Some(faction) {
                            // a swap restarts the capture, it never resumes the
                            // other side's banked ticks
                            self.challenger[index] = Some(faction);
                            self.progress[index] = 0;
                        }
                        self.progress[index] = self.progress[index].saturating_add(1);
                        if self.progress[index] >= point.hold_ticks {
                            self.owner[index] = Some(faction);
                            self.challenger[index] = None;
                            self.progress[index] = 0;
                        }
                    }
                }
                None => {
                    // an empty ring drifts back toward the owner
                    self.progress[index] = self.progress[index].saturating_sub(1);
                    if self.progress[index] == 0 {
                        self.challenger[index] = None;
                    }
                }
            }
        }
        // the score tick reads points, never bodies
        for owner in &self.owner {
            if let Some(faction) = *owner {
                self.score[faction.index()] = self.score[faction.index()].saturating_add(1);
            }
        }
        Ok(())
    }

    /// Digest of the whole state: tick, both scores, then every point's
    /// authored numbers and live state in order.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u64(self.ticks);
        for faction in Faction::ALL {
            hasher.write_u32(self.score[faction.index()]);
        }
        hasher.write_len(self.points.len());
        for index in 0..self.points.len() {
            let point = &self.points[index];
            hasher.write_u32(point.id);
            hasher.write_i32(point.x);
            hasher.write_i32(point.z);
            hasher.write_i32(point.radius);
            hasher.write_u32(point.hold_ticks);
            hasher.write_u8(match self.owner[index] {
                None => 0,
                Some(Faction::Wardens) => 1,
                Some(Faction::Ashkin) => 2,
            });
            hasher.write_u8(match self.challenger[index] {
                None => 0,
                Some(Faction::Wardens) => 1,
                Some(Faction::Ashkin) => 2,
            });
            hasher.write_u32(self.progress[index]);
            hasher.write_bool(self.contested[index]);
        }
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The side statues of `contested_middle.json` at 1 m rings, one second to
    /// take at the 60 Hz tick.
    fn side_statue(id: u32) -> CapturePoint {
        CapturePoint {
            id,
            x: 0,
            z: 0,
            radius: 100,
            hold_ticks: 60,
        }
    }

    fn row(wardens: u16, ashkin: u16) -> [u16; 2] {
        [wardens, ashkin]
    }

    #[test]
    fn a_neutral_point_flips_after_the_hold_and_scores_from_the_flip_tick() {
        let mut set = CaptureSet::new(vec![side_statue(1)]).unwrap();
        for tick in 0..100 {
            set.step(&[row(3, 0)]).unwrap();
            // the point stays neutral until the 60th tick of presence
            if tick < 59 {
                assert_eq!(set.owner(0), None, "flipped early at tick {tick}");
            } else {
                assert_eq!(
                    set.owner(0),
                    Some(Faction::Wardens),
                    "not flipped at tick {tick}"
                );
            }
        }
        // the flip tick scores too: ticks 59..=99
        assert_eq!(set.score(Faction::Wardens), 100 - 59);
        assert_eq!(set.score(Faction::Ashkin), 0);
        assert_eq!(set.progress(0), 0);
        assert_eq!(set.challenger(0), None);
    }

    #[test]
    fn a_contested_ring_freezes_progress() {
        let mut set = CaptureSet::new(vec![side_statue(1)]).unwrap();
        for _ in 0..30 {
            set.step(&[row(2, 1)]).unwrap();
        }
        assert_eq!(set.progress(0), 0);
        assert_eq!(set.owner(0), None);
        assert!(set.view(0).unwrap().contested);
        for _ in 0..200 {
            set.step(&[row(1, 1)]).unwrap();
        }
        assert_eq!(set.progress(0), 0);
        assert_eq!(set.owner(0), None);
        assert_eq!(set.score(Faction::Wardens), 0);
        assert_eq!(set.score(Faction::Ashkin), 0);
    }

    #[test]
    fn an_empty_ring_decays_progress_back_to_the_owner() {
        let mut set = CaptureSet::new(vec![side_statue(1)]).unwrap();
        for _ in 0..20 {
            set.step(&[row(0, 1)]).unwrap();
        }
        assert_eq!(set.progress(0), 20);
        assert_eq!(set.owner(0), None);
        for tick in 0..20 {
            set.step(&[row(0, 0)]).unwrap();
            // each empty tick takes one off: after the first step it is 19
            let left = 19 - tick;
            assert_eq!(set.progress(0), left);
        }
        assert_eq!(set.progress(0), 0);
        assert_eq!(set.challenger(0), None);
        // the point stayed neutral through the decay
        assert_eq!(set.owner(0), None);
    }

    #[test]
    fn a_taken_point_flips_to_the_other_faction() {
        let mut set = CaptureSet::new(vec![side_statue(1)]).unwrap();
        for _ in 0..60 {
            set.step(&[row(1, 0)]).unwrap();
        }
        assert_eq!(set.owner(0), Some(Faction::Wardens));
        for _ in 0..59 {
            set.step(&[row(0, 1)]).unwrap();
            assert_eq!(set.owner(0), Some(Faction::Wardens));
        }
        set.step(&[row(0, 1)]).unwrap();
        assert_eq!(set.owner(0), Some(Faction::Ashkin));
        // its own faction standing in it clears the challenge
        set.step(&[row(0, 4)]).unwrap();
        assert_eq!(set.view(0).unwrap().challenger, None);
        assert_eq!(set.progress(0), 0);
    }

    #[test]
    fn a_challenger_swap_restarts_the_capture() {
        let mut set = CaptureSet::new(vec![side_statue(1)]).unwrap();
        for _ in 0..5 {
            set.step(&[row(0, 2)]).unwrap();
        }
        let before = set.view(0).unwrap();
        assert_eq!(before.challenger, Some(Faction::Ashkin));
        assert_eq!(before.progress, 5);
        set.step(&[row(3, 0)]).unwrap();
        let after = set.view(0).unwrap();
        assert_eq!(after.challenger, Some(Faction::Wardens));
        assert_eq!(after.progress, 1);
        // a contest between them freezes the Wardens' fresh capture
        set.step(&[row(3, 1)]).unwrap();
        assert_eq!(set.progress(0), 1);
    }

    #[test]
    fn the_score_reads_points_never_bodies() {
        let mut set =
            CaptureSet::new(vec![side_statue(1), side_statue(2), side_statue(3)]).unwrap();
        for tick in 0..300 {
            let wardens = if tick < 60 { 7 } else { 0 };
            set.step(&[row(wardens, 0), row(0, 0), row(0, 3)]).unwrap();
        }
        // point 1 flips to the Wardens at tick 59: 300-59, nothing else is theirs
        assert_eq!(set.score(Faction::Wardens), 300 - 59);
        // point 3 flips to the Ashkin at tick 59: 300-59
        assert_eq!(set.score(Faction::Ashkin), 300 - 59);
        // point 2 was never entered
        assert_eq!(set.owner(1), None);
        // many bodies in one ring score the same as one
        let mut thin = CaptureSet::new(vec![side_statue(9)]).unwrap();
        for _ in 0..120 {
            thin.step(&[row(1, 0)]).unwrap();
        }
        let mut thick = CaptureSet::new(vec![side_statue(9)]).unwrap();
        for _ in 0..120 {
            thick.step(&[row(40, 0)]).unwrap();
        }
        assert_eq!(thin.state_hash(), thick.state_hash());
    }

    #[test]
    fn a_ten_minute_scripted_match_scores_and_hashes_twice() {
        // RY4's accept: a headless 10-minute match with scripted occupancy
        // ends with the expected score, and the same hash twice.
        // 600 s at the 60 Hz tick, two points, one second to take each.
        const TICKS: u64 = 36_000;
        let scripted = |tick: u64| -> [[u16; 2]; 2] {
            if tick < 100 {
                // the west statue is taken and held from the first tick
                [row(2, 0), row(0, 0)]
            } else if tick < 160 {
                // the Ashkin walk into the middle statue
                [row(2, 0), row(0, 2)]
            } else if tick < 500 {
                // the Ashkin hold it
                [row(2, 0), row(0, 2)]
            } else if tick < 560 {
                // the Wardens take the middle back
                [row(2, 0), row(2, 0)]
            } else {
                // the Wardens hold both, nobody walks the middle again
                [row(2, 0), row(2, 0)]
            }
        };
        let run = || -> (CaptureSet, u64) {
            let mut set = CaptureSet::new(vec![side_statue(11), side_statue(22)]).unwrap();
            for tick in 0..TICKS {
                let present = scripted(tick);
                set.step(&present).unwrap();
            }
            let hash = set.state_hash();
            (set, hash)
        };
        let (first, first_hash) = run();
        let (second, second_hash) = run();
        assert_eq!(first, second);
        assert_eq!(first_hash, second_hash);
        // expected score, counted by hand from the script:
        // west: Wardens flip at tick 59 -> 36000 - 59
        // middle: Ashkin flip at tick 159, the Wardens flip it back at 559
        //         -> Ashkin holds ticks 159..=558, which is 400
        //         -> Wardens hold 36000 - 559
        assert_eq!(first.score(Faction::Wardens), (36000 - 59) + (36000 - 559));
        assert_eq!(first.score(Faction::Ashkin), 400);
        assert_eq!(first.owner(0), Some(Faction::Wardens));
        assert_eq!(first.owner(1), Some(Faction::Wardens));
    }

    #[test]
    fn the_state_hash_sees_every_field() {
        let mut set = CaptureSet::new(vec![side_statue(1)]).unwrap();
        set.step(&[row(1, 0)]).unwrap();
        let base = set.state_hash();
        let mut again = CaptureSet::new(vec![side_statue(1)]).unwrap();
        again.step(&[row(1, 0)]).unwrap();
        assert_eq!(base, again.state_hash());
        // a different point table hashes differently
        let mut other = CaptureSet::new(vec![side_statue(2)]).unwrap();
        other.step(&[row(1, 0)]).unwrap();
        assert_ne!(base, other.state_hash());
        // a different faction in the ring hashes differently
        let mut flipped = CaptureSet::new(vec![side_statue(1)]).unwrap();
        flipped.step(&[row(0, 1)]).unwrap();
        assert_ne!(base, flipped.state_hash());
    }

    #[test]
    fn a_bad_table_is_refused_with_the_reason() {
        assert_eq!(CaptureSet::new(vec![]).unwrap_err(), CaptureError::Empty);
        let many: Vec<CapturePoint> = (0..MAX_POINTS + 1).map(|i| side_statue(i as u32)).collect();
        assert_eq!(
            CaptureSet::new(many).unwrap_err(),
            CaptureError::TooManyPoints {
                count: MAX_POINTS + 1,
                max: MAX_POINTS
            }
        );
        assert_eq!(
            CaptureSet::new(vec![side_statue(4), side_statue(4)]).unwrap_err(),
            CaptureError::DuplicateId(4)
        );
        let flat = CapturePoint {
            radius: 0,
            ..side_statue(5)
        };
        assert_eq!(
            CaptureSet::new(vec![flat]).unwrap_err(),
            CaptureError::BadRadius { id: 5, radius: 0 }
        );
        let instant = CapturePoint {
            hold_ticks: 0,
            ..side_statue(6)
        };
        assert_eq!(
            CaptureSet::new(vec![instant]).unwrap_err(),
            CaptureError::NoHold { id: 6 }
        );
    }

    #[test]
    fn a_step_refuses_an_occupancy_mismatch_and_changes_nothing() {
        let mut set = CaptureSet::new(vec![side_statue(1), side_statue(2)]).unwrap();
        let before = set.clone();
        assert_eq!(
            set.step(&[row(1, 0)]).unwrap_err(),
            CaptureError::OccupancyMismatch { want: 2, got: 1 }
        );
        assert_eq!(set, before);
    }
}

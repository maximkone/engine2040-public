//! roster: seeded spawn rosters over tagged spawn points (UW19).
//!
//! Consumer: none yet. The mission maps (UW30) and the headless composer (UW26) hand a roster
//! and the map's spawn points to [`place`] and spawn what comes back. Data owner:
//! `engine-world`; the group kinds (archetype, weapon, trap, evidence, objective ids) belong to
//! the tables that name them, this module only carries the ids. Units: ids and tag bits, no
//! meters. Determinism: integer math and `CounterRng` streams, one stream per group, so the same
//! `(roster, points, seed)` gives the same placements on every PC and a change to one group never
//! reshuffles another group's draw. Bake time only, never the tick.
//!
//! A placement is legal when its point carries the group's tag, no point holds two placements,
//! and every group lands between `min` and `max`. The minimums are met by augmenting-path
//! matching, so a greedy pick that blocks a later group is undone instead of refused: a roster
//! is refused only when no assignment of points meets all the minimums, and the error names the
//! first group (in roster order) that could not be completed. Errors: [`RosterError`].
//! Budget: `O(minimums * points)` at worst, no work per tick.

use engine_core::{CounterCursor, CounterRng};
use std::collections::BTreeSet;
use std::fmt;

/// Most spawn points one map hands in. Keeps the matching recursion shallow.
pub const MAX_POINTS: usize = 1024;
/// Most groups one roster holds.
pub const MAX_GROUPS: usize = 64;
/// Tags are bit positions `0..=MAX_TAG` of a point's tag word.
pub const MAX_TAG: u8 = 31;

/// What a group puts on its points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    /// An actor of one archetype.
    Archetype,
    /// A weapon lying on the point.
    Weapon,
    /// A trap.
    Trap,
    /// A piece of evidence.
    Evidence,
    /// A mission objective.
    Objective,
}

/// One place that can host a placement. Ids are the map's own and must differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnPoint {
    /// Map id of the point, reported back in [`Placement::point`].
    pub id: u32,
    /// Tag bits: bit `t` set means the point carries tag `t`.
    pub tags: u32,
}

/// One roster line: `min..=max` placements of one kind on points that carry one tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    /// Unique name; refusals quote it.
    pub name: String,
    /// What the group places.
    pub slot: Slot,
    /// Row id in the table for `slot` (archetype id, weapon id, ...).
    pub kind: u16,
    /// Tag a point must carry, `0..=MAX_TAG`.
    pub tag: u8,
    /// Placements the group must get.
    pub min: u16,
    /// Placements the group may get at most.
    pub max: u16,
}

/// A roster: the groups to place, in priority order (earlier groups draw extras first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    /// Groups, at most [`MAX_GROUPS`].
    pub groups: Vec<Group>,
}

/// One placed item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Placement {
    /// Index of the group in the roster.
    pub group: u16,
    /// What the group places.
    pub slot: Slot,
    /// Table row id, copied from the group.
    pub kind: u16,
    /// [`SpawnPoint::id`] it sits on.
    pub point: u32,
}

/// Why a roster was refused. Nothing is placed on refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RosterError {
    /// More than [`MAX_POINTS`] spawn points.
    TooManyPoints,
    /// More than [`MAX_GROUPS`] groups.
    TooManyGroups,
    /// Two spawn points share an id.
    DuplicatePoint(u32),
    /// Two groups share a name.
    DuplicateGroup(String),
    /// A group's tag is past [`MAX_TAG`].
    BadTag {
        /// Group name.
        group: String,
        /// The offending tag.
        tag: u8,
    },
    /// A group's `min` is above its `max`.
    MinOverMax {
        /// Group name.
        group: String,
        /// Requested minimum.
        min: u16,
        /// Requested maximum.
        max: u16,
    },
    /// No assignment of points meets the group's minimum.
    Short {
        /// Group name.
        group: String,
        /// Placements the group needs.
        need: u16,
        /// Points that carry the group's tag at all.
        tagged: usize,
    },
}

impl fmt::Display for RosterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyPoints => write!(f, "roster: more than {MAX_POINTS} spawn points"),
            Self::TooManyGroups => write!(f, "roster: more than {MAX_GROUPS} groups"),
            Self::DuplicatePoint(id) => write!(f, "roster: two spawn points share id {id}"),
            Self::DuplicateGroup(name) => write!(f, "roster: two groups are named {name}"),
            Self::BadTag { group, tag } => {
                write!(f, "roster: group {group} wants tag {tag}, past {MAX_TAG}")
            }
            Self::MinOverMax { group, min, max } => {
                write!(f, "roster: group {group} has min {min} over max {max}")
            }
            Self::Short {
                group,
                need,
                tagged,
            } => write!(
                f,
                "roster: group {group} needs {need} points but only {tagged} carry its tag and the rest are taken by other groups"
            ),
        }
    }
}

impl std::error::Error for RosterError {}

/// A count or index as the `u64` the stream wants.
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn validate(roster: &Roster, points: &[SpawnPoint]) -> Result<(), RosterError> {
    if points.len() > MAX_POINTS {
        return Err(RosterError::TooManyPoints);
    }
    if roster.groups.len() > MAX_GROUPS {
        return Err(RosterError::TooManyGroups);
    }
    let mut ids = BTreeSet::new();
    for p in points {
        if !ids.insert(p.id) {
            return Err(RosterError::DuplicatePoint(p.id));
        }
    }
    let mut names = BTreeSet::new();
    for g in &roster.groups {
        if !names.insert(g.name.as_str()) {
            return Err(RosterError::DuplicateGroup(g.name.clone()));
        }
        if g.tag > MAX_TAG {
            return Err(RosterError::BadTag {
                group: g.name.clone(),
                tag: g.tag,
            });
        }
        if g.min > g.max {
            return Err(RosterError::MinOverMax {
                group: g.name.clone(),
                min: g.min,
                max: g.max,
            });
        }
    }
    Ok(())
}

/// Indices of the points that carry `tag`, in a seeded order that belongs to the group alone.
fn eligible(points: &[SpawnPoint], tag: u8, rng: &mut CounterCursor) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..points.len())
        .filter(|i| (points[*i].tags >> tag) & 1 == 1)
        .collect();
    for i in (1..idx.len()).rev() {
        let j = usize::try_from(rng.next_range(0, count(i + 1))).unwrap_or(0);
        idx.swap(i, j);
    }
    idx
}

/// Kuhn's augmenting path, per group: gives `group` one more point. A free candidate is taken
/// first; otherwise a candidate's holder is asked to take another point so this one frees up, and
/// so on down the chain. `holder[p]` is the group that owns point `p`; `seen` marks the points
/// this search already tried.
fn assign(
    group: usize,
    orders: &[Vec<usize>],
    holder: &mut [Option<usize>],
    seen: &mut [bool],
) -> bool {
    let free = orders[group].iter().copied().find(|&p| holder[p].is_none());
    if let Some(p) = free {
        holder[p] = Some(group);
        return true;
    }
    for &p in &orders[group] {
        if seen[p] {
            continue;
        }
        seen[p] = true;
        if let Some(other) = holder[p] {
            if assign(other, orders, holder, seen) {
                holder[p] = Some(group);
                return true;
            }
        }
    }
    false
}

/// Places `roster` on `points` with the stream of `seed`.
///
/// Every group first gets its `min` placements (matching, roster order decides who is refused
/// when the points run out), then, in roster order, a seeded number of extras up to `max` from
/// the points that are still free and carry its tag; a group that finds fewer free points than
/// it drew keeps what it found. The result is sorted by `(group, point id)`.
///
/// Errors: any [`RosterError`]; [`RosterError::Short`] names the group whose minimum failed.
pub fn place(
    roster: &Roster,
    points: &[SpawnPoint],
    seed: u64,
) -> Result<Vec<Placement>, RosterError> {
    validate(roster, points)?;
    let root = CounterRng::new(seed);
    let mut streams: Vec<CounterCursor> = (0..roster.groups.len())
        .map(|g| CounterCursor::from_rng(root.fork(count(g)), 0))
        .collect();
    let orders: Vec<Vec<usize>> = roster
        .groups
        .iter()
        .zip(&mut streams)
        .map(|(g, rng)| eligible(points, g.tag, rng))
        .collect();
    let mut holder: Vec<Option<usize>> = vec![None; points.len()];
    // One augmenting search per placement of a minimum, groups in roster order.
    for (g, group) in roster.groups.iter().enumerate() {
        for _ in 0..group.min {
            let mut seen = vec![false; points.len()];
            if !assign(g, &orders, &mut holder, &mut seen) {
                return Err(RosterError::Short {
                    group: group.name.clone(),
                    need: group.min,
                    tagged: orders[g].len(),
                });
            }
        }
    }
    for (g, group) in roster.groups.iter().enumerate() {
        let spare = u64::from(group.max - group.min);
        let mut want = streams[g].next_range(0, spare + 1);
        for &p in &orders[g] {
            if want == 0 {
                break;
            }
            if holder[p].is_none() {
                holder[p] = Some(g);
                want -= 1;
            }
        }
    }
    let mut out: Vec<Placement> = holder
        .iter()
        .enumerate()
        .filter_map(|(p, h)| {
            let g = (*h)?;
            let group = &roster.groups[g];
            Some(Placement {
                group: u16::try_from(g).unwrap_or(u16::MAX),
                slot: group.slot,
                kind: group.kind,
                point: points[p].id,
            })
        })
        .collect();
    out.sort_by_key(|pl| (pl.group, pl.point));
    Ok(out)
}

/// What [`verify`] found wrong with a placement list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// A placement names a group index past the roster.
    NoSuchGroup(u16),
    /// A placement sits on a point id the map does not have.
    NoSuchPoint {
        /// Group name.
        group: String,
        /// The unknown point id.
        point: u32,
    },
    /// A placement sits on a point that lacks the group's tag.
    MissingTag {
        /// Group name.
        group: String,
        /// The point id.
        point: u32,
        /// The tag the group needs.
        tag: u8,
    },
    /// Two placements share a point.
    SharedPoint(u32),
    /// A group's placement count is outside `min..=max`.
    Count {
        /// Group name.
        group: String,
        /// Placements found.
        found: usize,
        /// Least allowed.
        min: u16,
        /// Most allowed.
        max: u16,
    },
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchGroup(g) => write!(f, "placement names group {g} past the roster"),
            Self::NoSuchPoint { group, point } => {
                write!(f, "{group}: point {point} does not exist")
            }
            Self::MissingTag { group, point, tag } => {
                write!(f, "{group}: point {point} lacks tag {tag}")
            }
            Self::SharedPoint(p) => write!(f, "point {p} holds two placements"),
            Self::Count {
                group,
                found,
                min,
                max,
            } => write!(f, "{group}: {found} placements outside {min}..={max}"),
        }
    }
}

impl std::error::Error for Fault {}

/// Checks a placement list against the roster and points it claims to come from: every
/// placement sits on a known point that carries its group's tag, no point is used twice, and
/// every group lands between `min` and `max`. The roster itself is assumed valid.
///
/// Errors: the first [`Fault`] found.
pub fn verify(roster: &Roster, points: &[SpawnPoint], placed: &[Placement]) -> Result<(), Fault> {
    let mut used = BTreeSet::new();
    let mut count = vec![0usize; roster.groups.len()];
    for pl in placed {
        let g = usize::from(pl.group);
        let group = roster.groups.get(g).ok_or(Fault::NoSuchGroup(pl.group))?;
        let point = points
            .iter()
            .find(|p| p.id == pl.point)
            .ok_or_else(|| Fault::NoSuchPoint {
                group: group.name.clone(),
                point: pl.point,
            })?;
        if (point.tags >> group.tag) & 1 == 0 {
            return Err(Fault::MissingTag {
                group: group.name.clone(),
                point: pl.point,
                tag: group.tag,
            });
        }
        if !used.insert(pl.point) {
            return Err(Fault::SharedPoint(pl.point));
        }
        count[g] += 1;
    }
    for (g, group) in roster.groups.iter().enumerate() {
        if count[g] < usize::from(group.min) || count[g] > usize::from(group.max) {
            return Err(Fault::Count {
                group: group.name.clone(),
                found: count[g],
                min: group.min,
                max: group.max,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eighty points: tag 0 on all, then door, window, roof, basement and cache tags on
    /// thinner and thinner slices.
    fn points() -> Vec<SpawnPoint> {
        (0..80u32)
            .map(|i| {
                let mut tags = 1;
                for (bit, every) in [(1, 5), (2, 7), (3, 9), (4, 11), (5, 13)] {
                    if i % every == 0 {
                        tags |= 1 << bit;
                    }
                }
                SpawnPoint { id: i, tags }
            })
            .collect()
    }

    fn group(name: &str, slot: Slot, tag: u8, min: u16, max: u16) -> Group {
        Group {
            name: name.to_string(),
            slot,
            kind: 7,
            tag,
            min,
            max,
        }
    }

    fn mission() -> Roster {
        Roster {
            groups: vec![
                group("guards", Slot::Archetype, 0, 4, 8),
                group("lookouts", Slot::Archetype, 3, 1, 3),
                group("rifles", Slot::Weapon, 0, 2, 4),
                group("snares", Slot::Trap, 1, 1, 3),
                group("ledger", Slot::Evidence, 5, 1, 1),
                group("vault_key", Slot::Objective, 4, 1, 1),
            ],
        }
    }

    #[test]
    fn a_hundred_seeds_place_legally_and_replay_exactly() {
        let (roster, pts) = (mission(), points());
        let mut distinct = BTreeSet::new();
        for seed in 0..100 {
            let first = place(&roster, &pts, seed).expect("places");
            verify(&roster, &pts, &first).expect("legal");
            assert_eq!(first, place(&roster, &pts, seed).expect("places again"));
            distinct.insert(first);
        }
        assert!(distinct.len() > 90, "seeds must spread: {}", distinct.len());
    }

    #[test]
    fn extras_stay_between_min_and_max_and_both_ends_are_drawn() {
        let (roster, pts) = (mission(), points());
        let mut guards = BTreeSet::new();
        for seed in 0..100 {
            let placed = place(&roster, &pts, seed).expect("places");
            guards.insert(placed.iter().filter(|p| p.group == 0).count());
        }
        assert!(guards.iter().all(|n| (4..=8).contains(n)), "{guards:?}");
        assert!(guards.contains(&4) && guards.len() > 2, "{guards:?}");
    }

    #[test]
    fn a_group_that_took_a_scarce_point_gives_it_up() {
        // Point 1 carries both tags, point 2 only tag 0. A greedy "wide" that takes point 1
        // would strand "narrow"; matching moves it to point 2.
        let pts = [
            SpawnPoint { id: 1, tags: 0b11 },
            SpawnPoint { id: 2, tags: 0b01 },
        ];
        let roster = Roster {
            groups: vec![
                group("wide", Slot::Weapon, 0, 1, 1),
                group("narrow", Slot::Trap, 1, 1, 1),
            ],
        };
        for seed in 0..100 {
            let placed = place(&roster, &pts, seed).expect("places");
            verify(&roster, &pts, &placed).expect("legal");
            let at = |g: u16| placed.iter().find(|p| p.group == g).map(|p| p.point);
            assert_eq!((at(0), at(1)), (Some(2), Some(1)), "seed {seed}");
        }
    }

    #[test]
    fn an_impossible_roster_names_the_group() {
        let pts = points();
        // Nine points carry tag 3 and the group wants twenty.
        let roster = Roster {
            groups: vec![
                group("guards", Slot::Archetype, 0, 4, 4),
                group("roof_snipers", Slot::Archetype, 3, 20, 20),
            ],
        };
        assert_eq!(
            place(&roster, &pts, 1),
            Err(RosterError::Short {
                group: "roof_snipers".into(),
                need: 20,
                tagged: 9,
            })
        );
        // Two groups, one tagged point: the later group is the one that is refused.
        let one = [SpawnPoint { id: 9, tags: 0b10 }];
        let rivals = Roster {
            groups: vec![
                group("first", Slot::Trap, 1, 1, 1),
                group("second", Slot::Evidence, 1, 1, 1),
            ],
        };
        let err = place(&rivals, &one, 5).expect_err("one point for two groups");
        assert!(err.to_string().contains("second"), "{err}");
    }

    #[test]
    fn bad_rosters_and_maps_are_refused_before_any_draw() {
        let pts = points();
        let dup_group = Roster {
            groups: vec![
                group("a", Slot::Trap, 0, 1, 1),
                group("a", Slot::Weapon, 0, 1, 1),
            ],
        };
        assert_eq!(
            place(&dup_group, &pts, 0),
            Err(RosterError::DuplicateGroup("a".into()))
        );
        let bad_tag = Roster {
            groups: vec![group("t", Slot::Trap, 32, 1, 1)],
        };
        assert!(matches!(
            place(&bad_tag, &pts, 0),
            Err(RosterError::BadTag { tag: 32, .. })
        ));
        let backwards = Roster {
            groups: vec![group("m", Slot::Trap, 0, 3, 2)],
        };
        assert!(matches!(
            place(&backwards, &pts, 0),
            Err(RosterError::MinOverMax { min: 3, max: 2, .. })
        ));
        let twice = [SpawnPoint { id: 4, tags: 1 }, SpawnPoint { id: 4, tags: 1 }];
        assert_eq!(
            place(&Roster::default(), &twice, 0),
            Err(RosterError::DuplicatePoint(4))
        );
        let many: Vec<SpawnPoint> = (0..=u32::try_from(MAX_POINTS).expect("fits"))
            .map(|id| SpawnPoint { id, tags: 1 })
            .collect();
        assert_eq!(
            place(&Roster::default(), &many, 0),
            Err(RosterError::TooManyPoints)
        );
        let crowd = Roster {
            groups: (0..=MAX_GROUPS)
                .map(|i| group(&format!("g{i}"), Slot::Trap, 0, 0, 1))
                .collect(),
        };
        assert_eq!(place(&crowd, &pts, 0), Err(RosterError::TooManyGroups));
        assert_eq!(place(&Roster::default(), &pts, 0), Ok(vec![]));
    }

    #[test]
    fn a_group_keeps_its_draw_when_a_later_group_is_added() {
        let pts = points();
        let base = Roster {
            groups: vec![group("guards", Slot::Archetype, 0, 3, 3)],
        };
        let mut longer = base.clone();
        longer.groups.push(group("ledger", Slot::Evidence, 5, 1, 1));
        for seed in 0..20 {
            let alone = place(&base, &pts, seed).expect("alone");
            let with = place(&longer, &pts, seed).expect("with ledger");
            let guards: Vec<Placement> = with.into_iter().filter(|p| p.group == 0).collect();
            // The ledger finds a free cache point, so nobody is displaced; and the guards draw
            // from their own stream, so adding a group leaves their points alone.
            assert_eq!(guards, alone, "seed {seed}");
        }
    }

    #[test]
    fn verify_catches_each_kind_of_fault() {
        let (roster, pts) = (mission(), points());
        let good = place(&roster, &pts, 3).expect("places");
        let mut twice = good.clone();
        twice.push(good[0]);
        assert!(verify(&roster, &pts, &twice).is_err());
        let mut wrong_tag = good.clone();
        let ledger = wrong_tag.iter_mut().find(|p| p.group == 4).expect("ledger");
        ledger.point = 1; // point 1 carries only tag 0
        let fault = verify(&roster, &pts, &wrong_tag).unwrap_err().to_string();
        assert!(fault.contains("lacks tag"), "{fault}");
        let short: Vec<Placement> = good.iter().filter(|p| p.group != 5).copied().collect();
        let fault = verify(&roster, &pts, &short).unwrap_err().to_string();
        assert!(fault.contains("vault_key"), "{fault}");
        let ghost = [Placement {
            point: 9999,
            ..good[0]
        }];
        let fault = verify(&roster, &pts, &ghost).unwrap_err().to_string();
        assert!(fault.contains("does not exist"), "{fault}");
    }
}

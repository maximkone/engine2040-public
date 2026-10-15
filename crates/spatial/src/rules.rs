//! Rule-derived affordances over the relation and sight vocabularies.
//!
//! Split from the base tag layer: at most 30 Datalog-style rules (8 assert, 13 refuse)
//! whose bodies read the relation tags and the sight tags beside the base
//! geometry. Assert rules re-derive the tags; every failed assert on an
//! authored tag becomes a refusal that carries its proof tree. [`RuleState`]
//! keeps the fact base and re-derives only the affected closure on a delta.
//! Units are integer centimeters inside; determinism comes from id-sorted
//! scans and integer math only. Behavior matches the base layer on the gold
//! room. Errors: degenerate rows read empty and refuse everything authored
//! on them; conversions saturate and unknown refuse rows skip, never panic.

use super::*;
use crate::index::{pos_known, to_cm};
use crate::relation::{between_holds, blocking_holds, holds, Frame, Rel};
use crate::sight::{los_actors, SightWorld};
use crate::{Actor, NavView};
use engine_core::Fnv64;
use std::collections::{BTreeMap, BTreeSet};

// ======================================================================
// Rule-derived affordances over the relation/sight vocabularies.
//
// At most 30 Datalog-style rules (8 assert, 13 refuse) whose bodies read the
// relation tags ([`holds`], [`between_holds`], [`blocking_holds`]) and the
// sight tags ([`los_actors`]) beside the base geometry. Assert rules
// re-derive the tags; every failed assert on an authored tag becomes a refusal
// that carries its proof tree, so 100% of refusals name the rule, the failed
// premise and the witness ids. [`RuleState`] keeps the fact base and
// re-derives only the affected closure on a delta. Units are integer
// centimeters inside; determinism comes from id-sorted scans and LE integer
// bytes only. The base layer above is untouched: rule tags must equal
// [`derive_for_agent`] tags on the gold room.

/// Ceiling on the rule table: the rule table holds at most 30 Datalog-style rules.
pub const MAX_RULES: usize = 30;
/// Witness id when a passing premise needed no witness (nav fallback, open
/// field) or a failing premise has none (band checks).
pub const NO_WITNESS: u32 = u32::MAX;
/// Open-field radius for the occlusion fallback, in cm: with no other actor
/// inside it, a wall needs no blocking witness.
const OPEN2_CM2: i64 = 160_000;
/// Nearest-neighbor window kept per actor for witness search.
const WITNESS_K: usize = 12;
/// Pair window for the ternary relation witnesses.
const TERNARY_K: usize = 8;

/// One rule-body literal. Geometry and nav literals mirror the base
/// predicates premise-for-premise; `NoAbove`, `LosClearOrIsolated`,
/// `OccludesOrOpen` and `BetweenOrBlockingOrNav` read the relation/sight tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Premise {
    /// Seat height and footprint band (mirrors the sittable band).
    SeatBand,
    /// Step height and tread band (mirrors the climbable band).
    StepBand,
    /// Tall, long wall shape (mirrors the cover shape).
    WallShape,
    /// Wide deck shape (mirrors the walkable shape).
    DeckShape,
    /// Low flat pad shape (mirrors the spawn shape).
    PadShape,
    /// Solid actor (mirrors the physical gate).
    Physical,
    /// Underside at ground level (mirrors the grounded gate).
    Grounded,
    /// Rests on the ground or a supporter top.
    Supported,
    /// Knee room above a seat top.
    ClearSit,
    /// Headroom above a climbed top.
    ClearClimb,
    /// Headroom above a walked top.
    ClearWalk,
    /// Free capsule ring around a pad (without the tile check).
    RingFree,
    /// Solid, known, not a monolith (mirrors the breakable gate).
    BreakGate,
    /// Solid control at touch height (mirrors the interactable gate).
    TouchGate,
    /// Bake tile under a pad reads walkable; vacuous without a bake view.
    TileWalk,
    /// A bake view is present (choke needs it).
    NavPresent,
    /// Baked door leaf reads as a single-file choke.
    NavDoorChoke,
    /// The schema authored this tag on this actor.
    Authored,
    /// The querying agent owns a sit clip.
    SitCap,
    /// The querying agent owns climb clips.
    ClimbCap,
    /// The querying agent owns a locomotion set.
    WalkCap,
    /// Relation: no other actor reads `Above` onto this one.
    NoAbove,
    /// Sight: the line of sight to the nearest neighbor is clear, or no
    /// neighbor is indexed.
    LosClearOrIsolated,
    /// Relation/sight: this wall blocks a near ray (witnessed), or the field is
    /// open (no other actor within 400 cm).
    OccludesOrOpen,
    /// Relation: this door sits between or blocking a near pair, or the baked
    /// door leaf already reads as a choke.
    BetweenOrBlockingOrNav,
}

impl Premise {
    /// True for the animation-gate literals, which evaluate before geometry
    /// on degenerate rows (mirroring `degenerate_reason`).
    const fn is_cap(self) -> bool {
        matches!(self, Premise::SitCap | Premise::ClimbCap | Premise::WalkCap)
    }

    /// Brief-stable literal name for proof trees.
    pub const fn name(self) -> &'static str {
        match self {
            Premise::SeatBand => "seat-band",
            Premise::StepBand => "step-band",
            Premise::WallShape => "wall-shape",
            Premise::DeckShape => "deck-shape",
            Premise::PadShape => "pad-shape",
            Premise::Physical => "physical",
            Premise::Grounded => "grounded",
            Premise::Supported => "supported",
            Premise::ClearSit => "clear-sit",
            Premise::ClearClimb => "clear-climb",
            Premise::ClearWalk => "clear-walk",
            Premise::RingFree => "ring-free",
            Premise::BreakGate => "break-gate",
            Premise::TouchGate => "touch-gate",
            Premise::TileWalk => "tile-walk",
            Premise::NavPresent => "nav-present",
            Premise::NavDoorChoke => "nav-door-choke",
            Premise::Authored => "authored",
            Premise::SitCap => "sit-cap",
            Premise::ClimbCap => "climb-cap",
            Premise::WalkCap => "walk-cap",
            Premise::NoAbove => "no-above",
            Premise::LosClearOrIsolated => "los-clear-or-isolated",
            Premise::OccludesOrOpen => "occludes-or-open",
            Premise::BetweenOrBlockingOrNav => "between-or-blocking-or-nav",
        }
    }
}

/// One assert rule: all premises true derives `tag` on the actor.
#[derive(Debug, Clone, Copy)]
pub struct AssertRule {
    /// Rule id, also the index into [`ASSERT_RULES`].
    pub id: u8,
    /// Brief-stable rule name.
    pub name: &'static str,
    /// Derived tag when every premise passes.
    pub tag: Afford,
    /// Body literals in evaluation order (caps last; degenerate rows
    /// evaluate caps first).
    pub premises: &'static [Premise],
}

/// One refuse rule: watches one assert rule for one authored tag and names
/// the refusal reason.
#[derive(Debug, Clone, Copy)]
pub struct RefuseRule {
    /// Rule id (assert count plus the index into [`REFUSE_RULES`]).
    pub id: u8,
    /// Brief-stable rule name.
    pub name: &'static str,
    /// Authored tag this rule refuses.
    pub tag: Afford,
    /// Machine-readable reason, shared with [`Mistag`].
    pub reason: &'static str,
    /// Watched assert id, or [`u8::MAX`] for the unknown-actor rule.
    pub watches: u8,
}

const SIT_P: [Premise; 6] = [
    Premise::SeatBand,
    Premise::Supported,
    Premise::ClearSit,
    Premise::NoAbove,
    Premise::LosClearOrIsolated,
    Premise::SitCap,
];
const CLIMB_P: [Premise; 6] = [
    Premise::StepBand,
    Premise::Supported,
    Premise::ClearClimb,
    Premise::NoAbove,
    Premise::LosClearOrIsolated,
    Premise::ClimbCap,
];
const COVER_P: [Premise; 5] = [
    Premise::Physical,
    Premise::WallShape,
    Premise::Supported,
    Premise::NoAbove,
    Premise::OccludesOrOpen,
];
const WALK_P: [Premise; 6] = [
    Premise::DeckShape,
    Premise::Supported,
    Premise::ClearWalk,
    Premise::NoAbove,
    Premise::LosClearOrIsolated,
    Premise::WalkCap,
];
const SPAWN_P: [Premise; 6] = [
    Premise::Physical,
    Premise::PadShape,
    Premise::Grounded,
    Premise::RingFree,
    Premise::TileWalk,
    Premise::LosClearOrIsolated,
];
const CHOKE_P: [Premise; 5] = [
    Premise::NavPresent,
    Premise::Physical,
    Premise::Grounded,
    Premise::NavDoorChoke,
    Premise::BetweenOrBlockingOrNav,
];
const BREAK_P: [Premise; 3] = [
    Premise::Authored,
    Premise::BreakGate,
    Premise::LosClearOrIsolated,
];
const CONTROL_P: [Premise; 3] = [
    Premise::Authored,
    Premise::TouchGate,
    Premise::LosClearOrIsolated,
];

/// The 8 assert rules: one per affordance, relation/sight literals included.
pub const ASSERT_RULES: [AssertRule; 8] = [
    AssertRule {
        id: 0,
        name: "sit",
        tag: Afford::Sittable,
        premises: &SIT_P,
    },
    AssertRule {
        id: 1,
        name: "climb",
        tag: Afford::Climbable,
        premises: &CLIMB_P,
    },
    AssertRule {
        id: 2,
        name: "cover",
        tag: Afford::Cover,
        premises: &COVER_P,
    },
    AssertRule {
        id: 3,
        name: "walk",
        tag: Afford::Walkable,
        premises: &WALK_P,
    },
    AssertRule {
        id: 4,
        name: "spawn",
        tag: Afford::Spawn,
        premises: &SPAWN_P,
    },
    AssertRule {
        id: 5,
        name: "choke",
        tag: Afford::Choke,
        premises: &CHOKE_P,
    },
    AssertRule {
        id: 6,
        name: "break",
        tag: Afford::Breakable,
        premises: &BREAK_P,
    },
    AssertRule {
        id: 7,
        name: "control",
        tag: Afford::Interactable,
        premises: &CONTROL_P,
    },
];

/// The 13 refuse rules: every [`Mistag`] reason in the base layer.
/// 8 assert + 13 refuse = 21 rules, under [`MAX_RULES`].
pub const REFUSE_RULES: [RefuseRule; 13] = [
    RefuseRule {
        id: 8,
        name: "refuse-seat",
        tag: Afford::Sittable,
        reason: "bad-seat",
        watches: 0,
    },
    RefuseRule {
        id: 9,
        name: "refuse-sit-anim",
        tag: Afford::Sittable,
        reason: "no-sit-anim",
        watches: 0,
    },
    RefuseRule {
        id: 10,
        name: "refuse-step",
        tag: Afford::Climbable,
        reason: "bad-step",
        watches: 1,
    },
    RefuseRule {
        id: 11,
        name: "refuse-climb-anim",
        tag: Afford::Climbable,
        reason: "no-climb-anim",
        watches: 1,
    },
    RefuseRule {
        id: 12,
        name: "refuse-cover",
        tag: Afford::Cover,
        reason: "bad-cover",
        watches: 2,
    },
    RefuseRule {
        id: 13,
        name: "refuse-walk",
        tag: Afford::Walkable,
        reason: "bad-walk",
        watches: 3,
    },
    RefuseRule {
        id: 14,
        name: "refuse-walk-anim",
        tag: Afford::Walkable,
        reason: "no-walk-anim",
        watches: 3,
    },
    RefuseRule {
        id: 15,
        name: "refuse-breakable",
        tag: Afford::Breakable,
        reason: "bad-breakable",
        watches: 6,
    },
    RefuseRule {
        id: 16,
        name: "refuse-control",
        tag: Afford::Interactable,
        reason: "bad-control",
        watches: 7,
    },
    RefuseRule {
        id: 17,
        name: "refuse-spawn",
        tag: Afford::Spawn,
        reason: "bad-spawn",
        watches: 4,
    },
    RefuseRule {
        id: 18,
        name: "refuse-choke",
        tag: Afford::Choke,
        reason: "bad-choke",
        watches: 5,
    },
    RefuseRule {
        id: 19,
        name: "refuse-no-nav",
        tag: Afford::Choke,
        reason: "needs-nav-view",
        watches: 5,
    },
    RefuseRule {
        id: 20,
        name: "refuse-unknown",
        tag: Afford::Sittable,
        reason: "unknown-actor",
        watches: u8::MAX,
    },
];

/// Total rule count: 8 assert + 13 refuse.
pub const RULE_COUNT: usize = 21;
/// Compile-time pin: the table stays exactly 21 rules, under 30.
const _: [(); RULE_COUNT] = [(); ASSERT_RULES.len() + REFUSE_RULES.len()];
/// Compile-time pin: the table never breaches the thirty-rule ceiling.
const _: () = assert!(RULE_COUNT <= MAX_RULES);

/// Assert rule index per affordance, matching [`ASSERT_RULES`] order.
pub(crate) const fn assert_for(tag: Afford) -> usize {
    match tag {
        Afford::Sittable => 0,
        Afford::Climbable => 1,
        Afford::Cover => 2,
        Afford::Walkable => 3,
        Afford::Spawn => 4,
        Afford::Choke => 5,
        Afford::Breakable => 6,
        Afford::Interactable => 7,
    }
}

/// Refusal reason for a failed premise, mirroring `verify_tag` and
/// `degenerate_reason`: animation gates refuse with `no-*-anim`, a missing
/// bake view refuses choke with `needs-nav-view`, everything else refuses
/// with the tag's `bad-*` code.
fn reason_for(tag: Afford, failed: Premise) -> &'static str {
    match failed {
        Premise::SitCap => "no-sit-anim",
        Premise::ClimbCap => "no-climb-anim",
        Premise::WalkCap => "no-walk-anim",
        Premise::NavPresent => "needs-nav-view",
        Premise::SeatBand
        | Premise::Supported
        | Premise::ClearSit
        | Premise::NoAbove
        | Premise::LosClearOrIsolated
        | Premise::StepBand
        | Premise::ClearClimb
        | Premise::Physical
        | Premise::WallShape
        | Premise::OccludesOrOpen
        | Premise::DeckShape
        | Premise::ClearWalk
        | Premise::PadShape
        | Premise::Grounded
        | Premise::RingFree
        | Premise::TileWalk
        | Premise::NavDoorChoke
        | Premise::BetweenOrBlockingOrNav
        | Premise::Authored
        | Premise::BreakGate
        | Premise::TouchGate => match tag {
            Afford::Sittable => "bad-seat",
            Afford::Climbable => "bad-step",
            Afford::Cover => "bad-cover",
            Afford::Breakable => "bad-breakable",
            Afford::Walkable => "bad-walk",
            Afford::Interactable => "bad-control",
            Afford::Spawn => "bad-spawn",
            Afford::Choke => "bad-choke",
        },
    }
}

/// Per-actor fact row: every premise input precomputed once, id-sorted and
/// order-invariant. Witness ids are [`NO_WITNESS`] when the premise needs
/// none.
#[derive(Debug, Clone, Copy)]
struct ActorFacts {
    valid: bool,
    physical: bool,
    sit_cap: bool,
    climb_cap: bool,
    walk_cap: bool,
    seat_band: bool,
    step_band: bool,
    wall_shape: bool,
    deck_shape: bool,
    pad_shape: bool,
    grounded: bool,
    supported: bool,
    clear_sit: bool,
    clear_climb: bool,
    clear_walk: bool,
    ring_nonav: bool,
    break_gate: bool,
    touch_gate: bool,
    tile_walk: bool,
    nav_present: bool,
    nav_door_choke: bool,
    no_above: bool,
    above_by: u32,
    los_clear: bool,
    los_near: u32,
    open400: bool,
    occludes: [u32; 2],
    between: [u32; 2],
    blocking: [u32; 2],
    near12: [u32; 12],
    near_len: u8,
}

impl Default for ActorFacts {
    fn default() -> Self {
        ActorFacts {
            valid: false,
            physical: false,
            sit_cap: false,
            climb_cap: false,
            walk_cap: false,
            seat_band: false,
            step_band: false,
            wall_shape: false,
            deck_shape: false,
            pad_shape: false,
            grounded: false,
            supported: false,
            clear_sit: false,
            clear_climb: false,
            clear_walk: false,
            ring_nonav: false,
            break_gate: false,
            touch_gate: false,
            tile_walk: false,
            nav_present: false,
            nav_door_choke: false,
            no_above: false,
            above_by: NO_WITNESS,
            los_clear: false,
            los_near: NO_WITNESS,
            open400: false,
            occludes: [NO_WITNESS; 2],
            between: [NO_WITNESS; 2],
            blocking: [NO_WITNESS; 2],
            near12: [NO_WITNESS; 12],
            near_len: 0,
        }
    }
}

/// Relation/sight fact fill for one valid actor: neighbor row from the
/// pass cache, the above scan over XZ-overlap candidates, one sight ray
/// and the ternary witnesses over the near window.
fn fill_sp_facts(
    id: u32,
    mine: &Dims,
    facts: &mut ActorFacts,
    fresh: &Fresh<'_>,
    nav: Option<&NavView>,
    world: &SightWorld,
) {
    match fresh.near.get(&id) {
        None => {
            facts.near_len = 0;
            facts.open400 = true;
        }
        Some(row) => {
            facts.near12 = row.near12;
            facts.near_len = row.near_len;
            facts.open400 = row.open400;
        }
    }
    // Above needs strict XZ overlap over the same integer boxes the
    // vocabulary builds, so a disjoint pair skips its call and reads false.
    facts.no_above = true;
    facts.above_by = NO_WITNESS;
    for other in fresh.actors {
        if other.id == id {
            continue;
        }
        if let Some((other_dims, _)) = fresh.boxes.get(&other.id) {
            if !xz_overlap(mine, other_dims) {
                continue;
            }
        }
        if holds(fresh.actors, other.id, Rel::Above, id, Frame::World) {
            facts.no_above = false;
            facts.above_by = other.id;
            break;
        }
    }
    if facts.near_len == 0 {
        facts.los_clear = true;
        facts.los_near = NO_WITNESS;
    } else {
        let nearest = facts.near12[0];
        facts.los_near = nearest;
        facts.los_clear = los_actors(world, id, nearest).unwrap_or(false);
    }
    let needs_occlude = facts.wall_shape;
    let needs_ternary = facts.wall_shape || nav.is_some_and(|view| view.doors.contains_key(&id));
    let window_len = TERNARY_K.min(usize::from(facts.near_len));
    let window = &facts.near12[..window_len];
    if needs_occlude && !facts.open400 {
        'outer: for (i, a) in window.iter().enumerate() {
            for b in window.iter().skip(i + 1) {
                if blocking_holds(fresh.actors, id, *a, *b) {
                    facts.occludes = [*a, *b];
                    break 'outer;
                }
            }
        }
    }
    if needs_ternary {
        'pairs: for (i, a) in window.iter().enumerate() {
            for b in window.iter().skip(i + 1) {
                if facts.between[0] == NO_WITNESS && between_holds(fresh.actors, id, *a, *b) {
                    facts.between = [*a, *b];
                }
                if facts.blocking[0] == NO_WITNESS && blocking_holds(fresh.actors, id, *a, *b) {
                    facts.blocking = [*a, *b];
                }
                if facts.between[0] != NO_WITNESS && facts.blocking[0] != NO_WITNESS {
                    break 'pairs;
                }
            }
        }
    }
}

/// One actor's fact row. Degenerate rows read all-false, like the base
/// layer: they derive nothing and refuse everything authored on them.
fn facts_for(
    id: u32,
    fresh: &Fresh<'_>,
    nav: Option<&NavView>,
    world: &SightWorld,
    caps: AnimCaps,
) -> ActorFacts {
    let mut facts = ActorFacts {
        sit_cap: caps.has_sit(),
        climb_cap: caps.has_climb(),
        walk_cap: caps.has_walk(),
        nav_present: nav.is_some(),
        ..Default::default()
    };
    let Some((mine, physical)) = fresh.boxes.get(&id) else {
        return facts;
    };
    facts.valid = true;
    facts.physical = *physical;
    facts.seat_band = mine.dy >= SEAT_MIN_CM
        && mine.dy <= SEAT_MAX_CM
        && mine.dx >= SEAT_SIDE_MIN_CM
        && mine.dx <= SEAT_SIDE_MAX_CM
        && mine.dz >= SEAT_SIDE_MIN_CM
        && mine.dz <= SEAT_SIDE_MAX_CM;
    facts.step_band =
        mine.dy >= STEP_MIN_CM && mine.dy <= STEP_MAX_CM && mine.dx.min(mine.dz) >= STEP_TREAD_CM;
    facts.wall_shape = mine.dy >= COVER_HEIGHT_CM && long_side(mine) >= COVER_LENGTH_CM;
    facts.deck_shape = mine.dx.min(mine.dz) >= WALK_MIN_CM && mine.max_y <= WALK_TOP_MAX_CM;
    facts.pad_shape = mine.dy <= SPAWN_HEIGHT_MAX_CM
        && mine.dx >= SPAWN_SIDE_MIN_CM
        && mine.dx <= SPAWN_SIDE_MAX_CM
        && mine.dz >= SPAWN_SIDE_MIN_CM
        && mine.dz <= SPAWN_SIDE_MAX_CM;
    facts.grounded = mine.min_y <= GROUND_CM;
    facts.supported = supported(&fresh.boxes, id, mine);
    facts.clear_sit = clear_above(&fresh.boxes, id, mine, SIT_CLEAR_CM);
    facts.clear_climb = clear_above(&fresh.boxes, id, mine, CLIMB_CLEAR_CM);
    facts.clear_walk = clear_above(&fresh.boxes, id, mine, WALK_CLEAR_CM);
    facts.ring_nonav = spawn_geom(&fresh.boxes, &fresh.by_id, id, mine, *physical, None);
    facts.break_gate = breakable_gate(mine, *physical);
    facts.touch_gate = interact_gate(mine, *physical);
    match nav {
        None => {
            facts.tile_walk = true;
            facts.nav_door_choke = false;
        }
        Some(view) => {
            facts.tile_walk = match fresh.by_id.get(&id) {
                None => false,
                Some(actor) => {
                    let tile = (
                        to_cm(actor.x).div_euclid(NAV_TILE_CM),
                        to_cm(actor.z).div_euclid(NAV_TILE_CM),
                    );
                    spawn_from_tile(view, tile).unwrap_or(false)
                }
            };
            facts.nav_door_choke = match view.doors.get(&id) {
                None => false,
                Some(door) => choke_from_width(door.width_cm).unwrap_or(false),
            };
        }
    }
    fill_sp_facts(id, mine, &mut facts, fresh, nav, world);
    facts
}

/// One premise verdict plus its witness ids.
fn premise_pass(premise: Premise, facts: &ActorFacts, authored: bool) -> (bool, u32, u32) {
    match premise {
        Premise::SeatBand => (facts.seat_band, NO_WITNESS, NO_WITNESS),
        Premise::StepBand => (facts.step_band, NO_WITNESS, NO_WITNESS),
        Premise::WallShape => (facts.wall_shape, NO_WITNESS, NO_WITNESS),
        Premise::DeckShape => (facts.deck_shape, NO_WITNESS, NO_WITNESS),
        Premise::PadShape => (facts.pad_shape, NO_WITNESS, NO_WITNESS),
        Premise::Physical => (facts.physical, NO_WITNESS, NO_WITNESS),
        Premise::Grounded => (facts.grounded, NO_WITNESS, NO_WITNESS),
        Premise::Supported => (facts.supported, NO_WITNESS, NO_WITNESS),
        Premise::ClearSit => (facts.clear_sit, NO_WITNESS, NO_WITNESS),
        Premise::ClearClimb => (facts.clear_climb, NO_WITNESS, NO_WITNESS),
        Premise::ClearWalk => (facts.clear_walk, NO_WITNESS, NO_WITNESS),
        Premise::RingFree => (facts.ring_nonav, NO_WITNESS, NO_WITNESS),
        Premise::BreakGate => (facts.break_gate, NO_WITNESS, NO_WITNESS),
        Premise::TouchGate => (facts.touch_gate, NO_WITNESS, NO_WITNESS),
        Premise::TileWalk => (facts.tile_walk, NO_WITNESS, NO_WITNESS),
        Premise::NavPresent => (facts.nav_present, NO_WITNESS, NO_WITNESS),
        Premise::NavDoorChoke => (facts.nav_door_choke, NO_WITNESS, NO_WITNESS),
        Premise::Authored => (authored, NO_WITNESS, NO_WITNESS),
        Premise::SitCap => (facts.sit_cap, NO_WITNESS, NO_WITNESS),
        Premise::ClimbCap => (facts.climb_cap, NO_WITNESS, NO_WITNESS),
        Premise::WalkCap => (facts.walk_cap, NO_WITNESS, NO_WITNESS),
        Premise::NoAbove => (facts.no_above, facts.above_by, NO_WITNESS),
        Premise::LosClearOrIsolated => (facts.los_clear, facts.los_near, NO_WITNESS),
        Premise::OccludesOrOpen => (
            facts.occludes[0] != NO_WITNESS || facts.open400,
            facts.occludes[0],
            facts.occludes[1],
        ),
        Premise::BetweenOrBlockingOrNav => {
            if facts.between[0] != NO_WITNESS {
                (true, facts.between[0], facts.between[1])
            } else if facts.blocking[0] != NO_WITNESS {
                (true, facts.blocking[0], facts.blocking[1])
            } else {
                (facts.nav_door_choke, NO_WITNESS, NO_WITNESS)
            }
        }
    }
}

/// Evaluation order: listed order, except degenerate rows evaluate the
/// animation gates first (mirroring `degenerate_reason`).
fn ordered(premises: &[Premise], caps_first: bool) -> [usize; 6] {
    let mut order = [0usize; 6];
    let mut slot = 0usize;
    if caps_first {
        for (i, premise) in premises.iter().enumerate() {
            if premise.is_cap() && slot < order.len() {
                order[slot] = i;
                slot += 1;
            }
        }
    }
    for (i, premise) in premises.iter().enumerate() {
        if (!caps_first || !premise.is_cap()) && slot < order.len() {
            order[slot] = i;
            slot += 1;
        }
    }
    order
}

/// One evaluated proof step: the premise, its verdict and its witnesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofStep {
    /// Evaluated literal.
    pub premise: Premise,
    /// True when the literal held.
    pub passed: bool,
    /// First witness id, or [`NO_WITNESS`].
    pub wa: u32,
    /// Second witness id, or [`NO_WITNESS`].
    pub wb: u32,
}

impl ProofStep {
    /// Blank step for proof padding.
    const fn blank() -> Self {
        ProofStep {
            premise: Premise::Authored,
            passed: false,
            wa: NO_WITNESS,
            wb: NO_WITNESS,
        }
    }
}

/// Proof tree of one assert-rule evaluation: the rule, every step in
/// evaluation order and the first failed premise, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proof {
    /// Fired rule id into [`ASSERT_RULES`].
    pub rule: u8,
    /// Fired rule name.
    pub rule_name: &'static str,
    /// True when every premise passed.
    pub passed: bool,
    /// First failed premise, or `None` when the rule fired.
    pub failed: Option<Premise>,
    /// First witness of the failed step, or [`NO_WITNESS`].
    pub fwa: u32,
    /// Second witness of the failed step, or [`NO_WITNESS`].
    pub fwb: u32,
    /// Evaluated steps in order.
    pub steps: [ProofStep; 6],
    /// Count of valid entries in [`Proof::steps`].
    pub len: usize,
}

impl Proof {
    /// Blank proof before evaluation.
    const fn blank() -> Self {
        Proof {
            rule: u8::MAX,
            rule_name: "blank",
            passed: false,
            failed: None,
            fwa: NO_WITNESS,
            fwb: NO_WITNESS,
            steps: [ProofStep::blank(); 6],
            len: 0,
        }
    }
}

/// Evaluate one assert rule against a fact row. `authored` says whether the
/// schema authored the head tag on this actor (only the break/control rules
/// read it).
fn eval_assert(rule: &AssertRule, facts: &ActorFacts, authored: bool) -> Proof {
    let order = ordered(rule.premises, !facts.valid);
    let mut proof = Proof {
        rule: rule.id,
        rule_name: rule.name,
        passed: true,
        failed: None,
        fwa: NO_WITNESS,
        fwb: NO_WITNESS,
        steps: [ProofStep::blank(); 6],
        len: 0,
    };
    for index in order.iter().take(rule.premises.len()) {
        let premise = rule.premises[*index];
        let (passed, wa, wb) = premise_pass(premise, facts, authored);
        proof.steps[proof.len] = ProofStep {
            premise,
            passed,
            wa,
            wb,
        };
        proof.len += 1;
        if !passed {
            proof.passed = false;
            proof.failed = Some(premise);
            proof.fwa = wa;
            proof.fwb = wb;
            return proof;
        }
    }
    proof
}

/// One refused authored tag with its derivation: the fired refuse rule, the
/// watched assert and the failed premise plus witnesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRefusal {
    /// Actor id (`u32::MAX` is never used: unknown ids keep their own id).
    pub id: u32,
    /// Refused tag.
    pub afford: Afford,
    /// Machine-readable reason, shared with [`Mistag`].
    pub reason: &'static str,
    /// Fired refuse rule id into [`REFUSE_RULES`].
    pub rule: u8,
    /// Fired refuse rule name.
    pub rule_name: &'static str,
    /// Failed premise of the watched assert, or `None` for unknown actors.
    pub failed: Option<Premise>,
    /// First witness id, or [`NO_WITNESS`].
    pub wa: u32,
    /// Second witness id, or [`NO_WITNESS`].
    pub wb: u32,
}

impl RuleRefusal {
    /// The plain base mistag this refusal carries the derivation for.
    pub fn mistag(&self) -> Mistag {
        Mistag {
            id: self.id,
            afford: self.afford,
            reason: self.reason,
        }
    }

    /// Brief-stable name of the failed premise, or `none`.
    pub fn failed_name(&self) -> &'static str {
        match self.failed {
            None => "none",
            Some(premise) => premise.name(),
        }
    }
}

/// Rule-built tags plus proof-carrying refusals for one scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOutcome {
    /// Derived tags in id order, like [`derive_all`].
    pub tags: Vec<ActorTags>,
    /// One entry per refused authored tag, each carrying its derivation.
    pub refusals: Vec<RuleRefusal>,
}

impl RuleOutcome {
    /// Count of derived actor rows.
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// True when no actor derived tags.
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Plain base mistags in `(id, afford)` order, for byte comparison.
    pub fn mistags(&self) -> Vec<Mistag> {
        self.refusals.iter().map(RuleRefusal::mistag).collect()
    }
}

/// Order-invariant digest over a bake view: sorted tiles, doors and flow
/// plus the spawn root, under the afford tag. Lets [`RuleState`] notice a
/// changed bake without comparing views field by field.
fn nav_digest(nav: Option<&NavView>) -> u64 {
    let mut hash = Fnv64::new();
    hash.write(AFFORD_TAG);
    match nav {
        None => hash.write(b"rule-nav-none"),
        Some(view) => {
            hash.write(b"rule-nav-some");
            for ((tx, tz), tile) in &view.tiles {
                hash.write_i32(*tx);
                hash.write_i32(*tz);
                hash.write_bool(tile.walk);
            }
            for (id, door) in &view.doors {
                hash.write_u32(*id);
                hash.write_i32(door.width_cm);
                hash.write_bool(door.two_way);
            }
            for ((fx, fz), (dx, dz)) in &view.flow {
                hash.write_i32(*fx);
                hash.write_i32(*fz);
                hash.write(&dx.to_le_bytes());
                hash.write(&dz.to_le_bytes());
            }
            hash.write_u32(view.spawn_id);
        }
    }
    hash.finish()
}

/// Evaluate all 8 assert rules for one fact row. Returns the proofs, the
/// derived set and the evaluation count. Rules 6 and 7 (break/control) read
/// `authored_here`; geometry rules derive without authored tags.
fn actor_proofs(facts: &ActorFacts, authored_here: AffordSet) -> ([Proof; 8], AffordSet, u64) {
    let mut proofs = [Proof::blank(); 8];
    let mut set = AffordSet::empty();
    let mut evals = 0u64;
    for (i, rule) in ASSERT_RULES.iter().enumerate() {
        let authored = match rule.tag {
            Afford::Breakable => authored_here.contains(Afford::Breakable),
            Afford::Interactable => authored_here.contains(Afford::Interactable),
            _ => true,
        };
        let proof = eval_assert(rule, facts, authored);
        if proof.passed {
            set.insert(rule.tag);
        }
        proofs[i] = proof;
        evals += 1;
    }
    (proofs, set, evals)
}

/// Authored rows folded by actor id: one tag set per actor over
/// [`Afford::ALL`], mirroring the merge in [`RuleState::refusals`].
fn authored_index(authored: &[Authored]) -> BTreeMap<u32, AffordSet> {
    let mut map: BTreeMap<u32, AffordSet> = BTreeMap::new();
    for row in authored {
        let entry = map.entry(row.id).or_default();
        for tag in Afford::ALL {
            if row.tags.contains(tag) {
                entry.insert(tag);
            }
        }
    }
    map
}

/// Cached neighbor row: the nearest ids plus the open-field flag, computed
/// once per pass so the closure and the fact fill share one distance scan.
#[derive(Debug, Clone, Copy)]
struct NearRow {
    near12: [u32; 12],
    near_len: u8,
    open400: bool,
}

impl Default for NearRow {
    /// Empty neighborhood over an open field, mirroring the per-actor scan.
    fn default() -> Self {
        NearRow {
            near12: [NO_WITNESS; 12],
            near_len: 0,
            open400: true,
        }
    }
}

/// Fresh id index over the incoming scene, shared by the re-derive steps:
/// bounds, neighbor rows and the actor slice behind them.
struct Fresh<'a> {
    actors: &'a [Actor],
    by_id: BTreeMap<u32, &'a Actor>,
    boxes: BTreeMap<u32, (Dims, bool)>,
    near: BTreeMap<u32, NearRow>,
}

impl<'a> Fresh<'a> {
    /// Index every actor with known bounds plus one distance scan for all
    /// neighbor rows, mirroring [`RuleState::build`].
    fn index(actors: &'a [Actor]) -> Self {
        let by_id: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
        let mut boxes: BTreeMap<u32, (Dims, bool)> = BTreeMap::new();
        for (id, actor) in &by_id {
            if let Some(dims) = dims_of(actor) {
                boxes.insert(*id, (dims, actor.physical));
            }
        }
        Fresh {
            near: neighbor_rows(actors, &by_id),
            actors,
            by_id,
            boxes,
        }
    }
}

/// One distance scan per actor over known positions: nearest ids ascending
/// (ties by id) plus whether the 400 cm field reads open. Matches the
/// per-actor scans premise for premise: the first position wins on
/// duplicate ids, unknown rows read empty and open.
fn neighbor_rows(actors: &[Actor], by_id: &BTreeMap<u32, &Actor>) -> BTreeMap<u32, NearRow> {
    let mut pos: BTreeMap<u32, (i32, i32)> = BTreeMap::new();
    for actor in actors {
        if pos_known(actor) {
            pos.entry(actor.id)
                .or_insert_with(|| (to_cm(actor.x), to_cm(actor.z)));
        }
    }
    let mut rows = BTreeMap::new();
    for id in by_id.keys() {
        let mut row = NearRow::default();
        let Some((px, pz)) = pos.get(id) else {
            rows.insert(*id, row);
            continue;
        };
        let mut scored: Vec<(i64, u32)> = Vec::new();
        for (other, (ox, oz)) in &pos {
            if other == id {
                continue;
            }
            let dx = i64::from(*px) - i64::from(*ox);
            let dz = i64::from(*pz) - i64::from(*oz);
            let d2 = dx * dx + dz * dz;
            if d2 <= OPEN2_CM2 {
                row.open400 = false;
            }
            scored.push((d2, *other));
        }
        scored.sort_unstable();
        let mut count = 0usize;
        for (_, other) in scored.into_iter().take(WITNESS_K) {
            if count < row.near12.len() {
                row.near12[count] = other;
                count += 1;
            }
        }
        // The window holds at most WITNESS_K (12) ids, so the conversion is
        // infallible; saturation is unreachable but never panics.
        row.near_len = u8::try_from(count).unwrap_or(u8::MAX);
        rows.insert(*id, row);
    }
    rows
}

/// Affected closure for a delta: changed ids plus overlap, witness and
/// neighbor touches. Id-sorted through [`BTreeSet`]; rows the closure misses
/// are covered by the missing-row pass in [`RuleState::rederive`].
fn affected_closure(
    changed: &BTreeSet<u32>,
    fresh: &Fresh<'_>,
    facts: &BTreeMap<u32, ActorFacts>,
) -> BTreeSet<u32> {
    let mut affected: BTreeSet<u32> = BTreeSet::new();
    for id in changed {
        if fresh.by_id.contains_key(id) {
            affected.insert(*id);
        }
    }
    for (id, (dims, _)) in &fresh.boxes {
        if affected.contains(id) {
            continue;
        }
        for other in changed {
            if let Some((other_dims, _)) = fresh.boxes.get(other) {
                if xz_overlap(dims, other_dims) {
                    affected.insert(*id);
                    break;
                }
            }
        }
    }
    for (id, row) in facts {
        if affected.contains(id) {
            continue;
        }
        let mut touches = false;
        for witness in [row.above_by, row.los_near] {
            if changed.contains(&witness) {
                touches = true;
                break;
            }
        }
        for pair in [row.occludes, row.between, row.blocking] {
            if changed.contains(&pair[0]) || changed.contains(&pair[1]) {
                touches = true;
                break;
            }
        }
        for slot in 0..usize::from(row.near_len) {
            if changed.contains(&row.near12[slot]) {
                touches = true;
                break;
            }
        }
        if touches {
            affected.insert(*id);
        }
    }
    for (id, row) in &fresh.near {
        if affected.contains(id) {
            continue;
        }
        for slot in 0..usize::from(row.near_len) {
            if changed.contains(&row.near12[slot]) {
                affected.insert(*id);
                break;
            }
        }
    }
    affected
}

/// Incremental rule state: the cached fact base plus per-actor proofs, so a
/// delta re-derives only the affected closure. Same contract as
/// [`derive_for_agent`] for tags and mistags; proofs ride beside them.
#[derive(Debug, Clone)]
pub struct RuleState {
    actors: Vec<Actor>,
    authored: Vec<Authored>,
    nav: Option<NavView>,
    digest: u64,
    caps: AnimCaps,
    facts: BTreeMap<u32, ActorFacts>,
    tags: BTreeMap<u32, AffordSet>,
    proofs: BTreeMap<u32, [Proof; 8]>,
    last_evals: u64,
    total_evals: u64,
}

impl RuleState {
    /// Full build over a scene: facts plus proofs for every actor.
    /// Deterministic: id-sorted scans, integer math only; any input order
    /// gives the same state.
    pub fn build(
        actors: &[Actor],
        authored: &[Authored],
        nav: Option<&NavView>,
        caps: AnimCaps,
    ) -> Self {
        let fresh = Fresh::index(actors);
        let world = SightWorld::build(actors);
        let authored_map = authored_index(authored);
        let mut state = RuleState {
            actors: actors.to_vec(),
            authored: authored.to_vec(),
            nav: nav.cloned(),
            digest: nav_digest(nav),
            caps,
            facts: BTreeMap::new(),
            tags: BTreeMap::new(),
            proofs: BTreeMap::new(),
            last_evals: 0,
            total_evals: 0,
        };
        let mut evals = 0u64;
        for id in fresh.by_id.keys() {
            let facts = facts_for(*id, &fresh, nav, &world, caps);
            let here = authored_map.get(id).copied().unwrap_or_default();
            let (proofs, set, n) = actor_proofs(&facts, here);
            state.facts.insert(*id, facts);
            state.tags.insert(*id, set);
            state.proofs.insert(*id, proofs);
            evals += n;
        }
        state.last_evals = evals;
        state.total_evals = evals;
        state
    }

    /// Derived tags in id order.
    pub fn tags(&self) -> Vec<ActorTags> {
        self.tags
            .iter()
            .map(|(id, tags)| ActorTags {
                id: *id,
                tags: *tags,
            })
            .collect()
    }

    /// Cover witness for one actor: `(occludes[0], open400)` from its fact
    /// row, or `None` when the actor has no fact row. Test hook for the
    /// cover-witness test; production readers use [`RuleState::tags`].
    #[cfg(test)]
    pub(crate) fn cover_witness(&self, id: u32) -> Option<(u32, bool)> {
        self.facts
            .get(&id)
            .map(|facts| (facts.occludes[0], facts.open400))
    }

    /// Proof-carrying refusals in `(id, afford)` order.
    pub fn refusals(&self) -> Vec<RuleRefusal> {
        let merged = authored_index(&self.authored);
        let by_id: BTreeMap<u32, &Actor> = self.actors.iter().map(|a| (a.id, a)).collect();
        let mut out = Vec::new();
        for (id, tags) in &merged {
            if !by_id.contains_key(id) {
                let rule = &REFUSE_RULES[12];
                out.push(RuleRefusal {
                    id: *id,
                    afford: Afford::Sittable,
                    reason: rule.reason,
                    rule: rule.id,
                    rule_name: rule.name,
                    failed: None,
                    wa: *id,
                    wb: NO_WITNESS,
                });
                for tag in Afford::ALL {
                    if tag != Afford::Sittable && tags.contains(tag) {
                        out.push(RuleRefusal {
                            id: *id,
                            afford: tag,
                            reason: rule.reason,
                            rule: rule.id,
                            rule_name: rule.name,
                            failed: None,
                            wa: *id,
                            wb: NO_WITNESS,
                        });
                    }
                }
                continue;
            }
            for tag in Afford::ALL {
                if !tags.contains(tag) {
                    continue;
                }
                let watched = assert_for(tag);
                let proof = self
                    .proofs
                    .get(id)
                    .map(|row| row[watched])
                    .unwrap_or(Proof::blank());
                if proof.passed {
                    continue;
                }
                let failed = proof.failed.unwrap_or(Premise::Authored);
                let reason = reason_for(tag, failed);
                // The refuse table covers every tag and reason pair the base layer
                // can refuse; a missing row skips instead of panicking.
                let Some(found) = REFUSE_RULES
                    .iter()
                    .find(|rule| rule.tag == tag && rule.reason == reason)
                else {
                    debug_assert!(false, "refuse table covers every tag and reason");
                    continue;
                };
                out.push(RuleRefusal {
                    id: *id,
                    afford: tag,
                    reason,
                    rule: found.id,
                    rule_name: found.name,
                    failed: proof.failed,
                    wa: proof.fwa,
                    wb: proof.fwb,
                });
            }
        }
        out.sort_by_key(|r| (r.id, r.afford));
        out
    }

    /// Full outcome: tags plus proof-carrying refusals.
    pub fn outcome(&self) -> RuleOutcome {
        RuleOutcome {
            tags: self.tags(),
            refusals: self.refusals(),
        }
    }

    /// Rule evaluations in the last build or re-derive.
    pub fn last_evals(&self) -> u64 {
        self.last_evals
    }

    /// Rule evaluations since construction.
    pub fn total_evals(&self) -> u64 {
        self.total_evals
    }

    /// True when no actor derived tags.
    pub fn is_empty(&self) -> bool {
        self.tags.values().all(|set| set.is_empty())
    }

    /// Re-derive after a delta: only the affected closure re-evaluates.
    ///
    /// `changed` names moved, added or removed actor ids; a bake-view edit
    /// names the door or tile's actor. `u32::MAX` forces a full rebuild, as
    /// do changed caps, authored rows or bake bytes. Returns the affected
    /// actor count. The outcome equals a fresh [`RuleState::build`]; only
    /// `last_evals` is smaller.
    pub fn rederive(
        &mut self,
        changed: &[u32],
        actors: &[Actor],
        authored: &[Authored],
        nav: Option<&NavView>,
        caps: AnimCaps,
    ) -> usize {
        if changed.contains(&NO_WITNESS)
            || caps != self.caps
            || authored != self.authored.as_slice()
            || nav_digest(nav) != self.digest
        {
            *self = Self::build(actors, authored, nav, caps);
            return self.tags.len();
        }
        let fresh = Fresh::index(actors);
        let changed_set: BTreeSet<u32> = changed.iter().copied().collect();
        let mut affected = affected_closure(&changed_set, &fresh, &self.facts);
        let world = SightWorld::build(actors);
        let authored_map = authored_index(authored);
        let mut evals = 0u64;
        for id in &affected {
            let facts = facts_for(*id, &fresh, nav, &world, caps);
            let here = authored_map.get(id).copied().unwrap_or_default();
            let (proofs, set, n) = actor_proofs(&facts, here);
            self.facts.insert(*id, facts);
            self.tags.insert(*id, set);
            self.proofs.insert(*id, proofs);
            evals += n;
        }
        for id in fresh.by_id.keys() {
            if !self.tags.contains_key(id) {
                let facts = facts_for(*id, &fresh, nav, &world, caps);
                let here = authored_map.get(id).copied().unwrap_or_default();
                let (proofs, set, n) = actor_proofs(&facts, here);
                self.facts.insert(*id, facts);
                self.tags.insert(*id, set);
                self.proofs.insert(*id, proofs);
                evals += n;
                affected.insert(*id);
            }
        }
        let stale: Vec<u32> = self
            .tags
            .keys()
            .filter(|id| !fresh.by_id.contains_key(id))
            .copied()
            .collect();
        for id in &stale {
            self.facts.remove(id);
            self.tags.remove(id);
            self.proofs.remove(id);
        }
        self.actors = actors.to_vec();
        self.authored = authored.to_vec();
        self.nav = nav.cloned();
        self.last_evals = evals;
        self.total_evals += evals;
        affected.len()
    }
}

/// Derive tags through the rule table: the rule-table twin of [`derive_for_agent`]
/// with [`AnimCaps::ALL`]. Tags equal [`derive_all`] on the gold room;
/// stacked or occluded scenes may refine them (documented in-test).
pub fn derive_with_rules(
    actors: &[Actor],
    authored: &[Authored],
    nav: Option<&NavView>,
) -> RuleOutcome {
    RuleState::build(actors, authored, nav, AnimCaps::ALL).outcome()
}

/// Derive tags through the rule table for one agent's clip set: the rule-table
/// twin of [`derive_for_agent`].
pub fn derive_rules_for_agent(
    actors: &[Actor],
    authored: &[Authored],
    nav: Option<&NavView>,
    caps: AnimCaps,
) -> RuleOutcome {
    RuleState::build(actors, authored, nav, caps).outcome()
}

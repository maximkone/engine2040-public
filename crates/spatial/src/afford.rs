//! Affordance tags from geometry plus authored schema tags, gated by the
//! querying agent's anim-set capabilities.
//!
//! Owner: spatial crate. Consumer: `scene.brief` afford rows (orchestrator
//! adds `pub mod afford;` plus the row hook; the shape is proven in-test
//! below against the real [`crate::brief`]). Units are meters in, integer
//! centimeters inside. Determinism: id-sorted scans, integer math only, no
//! wall clock; same actors plus same authored tags plus same caps in any
//! input order give the same tags and digest. Errors: typed [`AffordError`];
//! degenerate rows read empty, never panic. Budget: pairwise
//! support/clearance scans are O(n^2) integer ops for room-scale briefs,
//! never the fixed tick; one query over the 50-object gold stays under
//! 0.2 ms per object (see the timing probe test).
//!
//! Geometry wins: derived tags come from measurements; authored schema
//! tags only add [`Afford::Breakable`] and [`Afford::Interactable`], and any
//! authored tag whose geometric precondition fails is returned as a
//! [`Mistag`] instead of being applied. Animation gates the animated tags:
//! sittable needs a sit clip, climbable a climb clip, walkable a locomotion
//! set; an agent without the clip never sees the tag, and a matching
//! authored tag is refused with a `no-*-anim` reason.
//!
//! Rule layer (appended below): the same tags derived as at most 30
//! Datalog-style rules over the relation and sight vocabularies.
//! Assert rules re-derive the tags; refuse rules turn each failed assert into
//! a [`Mistag`] that carries its proof tree ([`RuleRefusal`]). [`RuleState`]
//! re-derives only the affected closure on a delta. The tag functions above
//! are frozen: the rule layer reproduces their bytes on the gold room.

use super::index::{pos_known, size_known, to_cm};
use super::navcheck::NavView;
use super::Actor;
use engine_core::Fnv64;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Digest tag so affordance bytes never alias another domain hash.
const AFFORD_TAG: &[u8] = b"e2afford:1";
/// Grounded when the underside sits this high at most, in cm.
const GROUND_CM: i32 = 2;
/// Resting contact window for a supporter top below an underside, in cm.
const REST_GAP_CM: i32 = 5;
/// Seat height band for sittable objects, in cm.
const SEAT_MIN_CM: i32 = 35;
/// Seat height band for sittable objects, in cm.
const SEAT_MAX_CM: i32 = 60;
/// Seat footprint band (width and depth), in cm.
const SEAT_SIDE_MIN_CM: i32 = 30;
/// Seat footprint band (width and depth), in cm.
const SEAT_SIDE_MAX_CM: i32 = 90;
/// Knee room above a seat top, in cm.
const SIT_CLEAR_CM: i32 = 40;
/// Step height band for climbable objects, in cm.
const STEP_MIN_CM: i32 = 25;
/// Step height band for climbable objects, in cm.
const STEP_MAX_CM: i32 = 130;
/// Narrowest climbable tread, in cm.
const STEP_TREAD_CM: i32 = 25;
/// Headroom above a climbed top, in cm.
const CLIMB_CLEAR_CM: i32 = 80;
/// Lowest wall that gives cover, in cm.
const COVER_HEIGHT_CM: i32 = 70;
/// Shortest wall face that gives cover, in cm.
const COVER_LENGTH_CM: i32 = 60;
/// Narrowest standable top, in cm.
const WALK_MIN_CM: i32 = 80;
/// Headroom above a walked top, in cm.
const WALK_CLEAR_CM: i32 = 150;
/// Highest walked top, in cm.
const WALK_TOP_MAX_CM: i32 = 400;
/// Spawn pad footprint band (width and depth), in cm.
const SPAWN_SIDE_MIN_CM: i32 = 60;
/// Spawn pad footprint band (width and depth), in cm.
const SPAWN_SIDE_MAX_CM: i32 = 200;
/// Thickest spawn pad, in cm.
const SPAWN_HEIGHT_MAX_CM: i32 = 30;
/// Standing capsule the spawn ring must fit, in cm.
const SPAWN_CAPSULE_CM: i32 = 180;
/// Free ring around a spawn pad, in cm.
const SPAWN_RING_CM: i32 = 100;
/// Largest breakable object (diagonal of material sense), in cm.
const BREAKABLE_MAX_CM: i32 = 300;
/// Reachable control height band for interactables, in cm.
const TOUCH_MIN_CM: i32 = 20;
/// Reachable control height band for interactables, in cm.
const TOUCH_MAX_CM: i32 = 220;
/// Largest interactable control, in cm.
const TOUCH_SIZE_MAX_CM: i32 = 300;
/// Nav tile pitch, in cm (mirrors `navcheck::TILE_CM`).
const NAV_TILE_CM: i32 = 100;
/// Frame the bake eats on both sides of a door, in cm (mirrors navcheck).
const DOOR_FRAME_CM: i32 = 20;
/// Narrowest clear choke passage, in cm.
const CHOKE_MIN_CM: i32 = 60;
/// Widest clear choke passage, in cm.
const CHOKE_MAX_CM: i32 = 160;

/// One derived affordance. Discriminants are frozen: the digest feeds the
/// bit index raw, so variants are append-only, never reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Afford {
    Sittable = 0,
    Climbable = 1,
    Cover = 2,
    Breakable = 3,
    Walkable = 4,
    Interactable = 5,
    Spawn = 6,
    Choke = 7,
}

impl Afford {
    /// Every affordance in digest order.
    pub const ALL: [Afford; 8] = [
        Afford::Sittable,
        Afford::Climbable,
        Afford::Cover,
        Afford::Breakable,
        Afford::Walkable,
        Afford::Interactable,
        Afford::Spawn,
        Afford::Choke,
    ];

    /// Brief-stable tag name.
    pub const fn name(self) -> &'static str {
        match self {
            Afford::Sittable => "sittable",
            Afford::Climbable => "climbable",
            Afford::Cover => "cover",
            Afford::Breakable => "breakable",
            Afford::Walkable => "walkable",
            Afford::Interactable => "interactable",
            Afford::Spawn => "spawn",
            Afford::Choke => "choke",
        }
    }

    /// Bit index in [`AffordSet`], without numeric casts.
    const fn bit(self) -> u32 {
        match self {
            Afford::Sittable => 0,
            Afford::Climbable => 1,
            Afford::Cover => 2,
            Afford::Breakable => 3,
            Afford::Walkable => 4,
            Afford::Interactable => 5,
            Afford::Spawn => 6,
            Afford::Choke => 7,
        }
    }
}

/// What stopped an affordance lookup. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AffordError {
    /// No derived actor carries this id.
    UnknownActor(u32),
}

impl fmt::Display for AffordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AffordError::UnknownActor(id) => {
                write!(f, "afford: unknown actor {id} (derive it first)")
            }
        }
    }
}

impl std::error::Error for AffordError {}

/// What stopped a nav-derived tag lookup. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavTagError {
    /// Door width in cm was negative; widths are 0 or more.
    BadDoorWidth(i32),
    /// No baked door carries this actor id.
    UnknownDoor(u32),
    /// No baked tile covers this tile coordinate.
    UnknownTile((i32, i32)),
    /// The query needs a bake view and got none.
    MissingNavView,
}

impl fmt::Display for NavTagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NavTagError::BadDoorWidth(w) => {
                write!(f, "nav tag: bad door width {w} cm (want 0 or more)")
            }
            NavTagError::UnknownDoor(id) => {
                write!(f, "nav tag: unknown door {id} (bake it first)")
            }
            NavTagError::UnknownTile((tx, tz)) => {
                write!(f, "nav tag: unknown tile ({tx},{tz}) (bake it first)")
            }
            NavTagError::MissingNavView => {
                write!(f, "nav tag: needs a bake view (pass Some)")
            }
        }
    }
}

impl std::error::Error for NavTagError {}

/// Clear passage of a baked door leaf, in cm: the width minus the frame the
/// bake eats on both sides together, floored at zero. Integer math only;
/// negative widths refuse with [`NavTagError::BadDoorWidth`] instead of
/// wrapping, so unseen geometry never panics.
pub fn door_clear_cm(width_cm: i32) -> Result<i32, NavTagError> {
    if width_cm < 0 {
        return Err(NavTagError::BadDoorWidth(width_cm));
    }
    Ok((width_cm - DOOR_FRAME_CM).max(0))
}

/// True when a baked door width reads as a single-file choke: the clear
/// passage fits one agent but is not a hall. This is the same band the
/// [`Afford::Choke`] derivation uses, so tile queries and tags never disagree.
pub fn choke_from_width(width_cm: i32) -> Result<bool, NavTagError> {
    Ok((CHOKE_MIN_CM..=CHOKE_MAX_CM).contains(&door_clear_cm(width_cm)?))
}

/// True when one baked tile reads as spawnable ground. Missing tiles refuse
/// with [`NavTagError::UnknownTile`]; the tag derivation maps that to false,
/// so unbaked ground simply reads as no spawn.
pub fn spawn_from_tile(nav: &NavView, tile: (i32, i32)) -> Result<bool, NavTagError> {
    match nav.tiles.get(&tile) {
        None => Err(NavTagError::UnknownTile(tile)),
        Some(t) => Ok(t.walk),
    }
}

/// Walkable tiles an agent would stage on beside cover: walkable bake tiles
/// 4-adjacent to a tile a cover-geometry actor overlaps. Pure function of the
/// actor boxes plus the bake tiles; integer math only, id-sorted scans, so
/// any input order gives the same tile list in the same order. Degenerate
/// actors index nothing and never panic. Needs a bake view: `None` refuses
/// with [`NavTagError::MissingNavView`]. The tag derivation never moves on
/// this (tags stay byte-identical); brief and placement readers call it
/// beside [`afford_lines`].
pub fn cover_approach_tiles(
    actors: &[Actor],
    nav: Option<&NavView>,
) -> Result<Vec<(i32, i32)>, NavTagError> {
    let Some(view) = nav else {
        return Err(NavTagError::MissingNavView);
    };
    let by_id: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
    let mut boxes: BTreeMap<u32, (Dims, bool)> = BTreeMap::new();
    for (id, actor) in &by_id {
        if let Some(dims) = dims_of(actor) {
            boxes.insert(*id, (dims, actor.physical));
        }
    }
    let mut cover_tiles: BTreeSet<(i32, i32)> = BTreeSet::new();
    for (id, (dims, physical)) in &boxes {
        if !cover_geom(&boxes, *id, dims, *physical) {
            continue;
        }
        for tx in dims.min_x.div_euclid(NAV_TILE_CM)..=dims.max_x.div_euclid(NAV_TILE_CM) {
            for tz in dims.min_z.div_euclid(NAV_TILE_CM)..=dims.max_z.div_euclid(NAV_TILE_CM) {
                cover_tiles.insert((tx, tz));
            }
        }
    }
    let mut out: BTreeSet<(i32, i32)> = BTreeSet::new();
    for tile in &cover_tiles {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let next = (tile.0.saturating_add(dx), tile.1.saturating_add(dz));
            if cover_tiles.contains(&next) {
                continue;
            }
            if view.tiles.get(&next).map(|t| t.walk).unwrap_or(false) {
                out.insert(next);
            }
        }
    }
    Ok(out.into_iter().collect())
}

/// Eight affordances as one byte. Iteration order is [`Afford::ALL`];
/// rendering is id-sorted by the caller, so sets compare and hash stably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AffordSet(u8);

impl AffordSet {
    /// No affordances.
    pub const NONE: AffordSet = AffordSet(0);

    /// Empty set.
    pub const fn empty() -> Self {
        AffordSet(0)
    }

    /// Build from a list; duplicates collapse.
    pub fn from_list(list: &[Afford]) -> Self {
        let mut out = AffordSet::empty();
        for a in list {
            out.insert(*a);
        }
        out
    }

    /// True when the tag is present.
    pub fn contains(self, tag: Afford) -> bool {
        self.0 & (1u8 << tag.bit()) != 0
    }

    /// Add a tag.
    pub fn insert(&mut self, tag: Afford) {
        self.0 |= 1u8 << tag.bit();
    }

    /// Union of two sets.
    pub const fn union(self, other: AffordSet) -> AffordSet {
        AffordSet(self.0 | other.0)
    }

    /// Tag count.
    pub fn len(self) -> usize {
        let mut n = 0;
        for tag in Afford::ALL {
            if self.contains(tag) {
                n += 1;
            }
        }
        n
    }

    /// True when no tag is present.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Raw bits, for the digest only.
    fn bits(self) -> u8 {
        self.0
    }

    /// Comma names in [`Afford::ALL`] order, or `none`.
    pub fn csv(self) -> String {
        let mut out = String::new();
        for tag in Afford::ALL {
            if self.contains(tag) {
                if !out.is_empty() {
                    out.push(',');
                }
                out.push_str(tag.name());
            }
        }
        if out.is_empty() {
            out.push_str("none");
        }
        out
    }
}

/// Anim-set capabilities of the querying agent: which actions its clip set
/// can play. Pure input beside the collider (`Actor`) and nav (`NavView`)
/// rows: cover, spawn and choke need no animation, while sittable needs a
/// sit clip, climbable a climb clip and walkable a locomotion set. Same caps
/// over any actor order give the same tags; [`AnimCaps::ALL`] reproduces the
/// ungated [`derive_all`] bytes exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AnimCaps(u8);

impl AnimCaps {
    /// No animation capability: only cover, spawn and choke derive.
    pub const NONE: AnimCaps = AnimCaps(0);
    /// Full clip set: every geometric tag derives.
    pub const ALL: AnimCaps = AnimCaps(0b111);
    /// Sit-idle clip present.
    pub const SIT: AnimCaps = AnimCaps(0b001);
    /// Climb and vault clips present.
    pub const CLIMB: AnimCaps = AnimCaps(0b010);
    /// Locomotion set present.
    pub const WALK: AnimCaps = AnimCaps(0b100);

    /// No capabilities.
    pub const fn empty() -> Self {
        AnimCaps::NONE
    }

    /// Full clip set.
    pub const fn all() -> Self {
        AnimCaps::ALL
    }

    /// Union of two capability sets.
    pub const fn union(self, other: AnimCaps) -> AnimCaps {
        AnimCaps(self.0 | other.0)
    }

    /// True when no capability is present.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when a sit clip is present.
    pub const fn has_sit(self) -> bool {
        self.0 & AnimCaps::SIT.0 != 0
    }

    /// True when a climb clip is present.
    pub const fn has_climb(self) -> bool {
        self.0 & AnimCaps::CLIMB.0 != 0
    }

    /// True when a locomotion set is present.
    pub const fn has_walk(self) -> bool {
        self.0 & AnimCaps::WALK.0 != 0
    }
}

/// One schema-authored tag row: component tags by actor id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Authored {
    /// Actor the tags were authored on.
    pub id: u32,
    /// Authored tags.
    pub tags: AffordSet,
}

impl Authored {
    /// One authored row.
    pub const fn new(id: u32, tags: AffordSet) -> Self {
        Authored { id, tags }
    }
}

/// Derived tags for one actor: geometry wins, schema adds breakable and
/// interactable only when their gates pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorTags {
    /// Actor id.
    pub id: u32,
    /// Final tags.
    pub tags: AffordSet,
}

/// One refused authored tag: the schema says it, the geometry denies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mistag {
    /// Actor id (`u32::MAX` is never used: unknown ids keep their own id).
    pub id: u32,
    /// Refused tag.
    pub afford: Afford,
    /// Machine-readable reason (`needs-nav-view`, `unknown-actor`, a
    /// `no-*-anim` clip refusal, or a `bad-*` geometry code).
    pub reason: &'static str,
}

/// Integer-cm box of one valid actor.
#[derive(Debug, Clone, Copy)]
struct Dims {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    min_z: i32,
    max_z: i32,
    dx: i32,
    dy: i32,
    dz: i32,
    cy: i32,
}

/// Box of one actor, or `None` for unknown, non-finite or non-positive rows.
/// Degenerate rows index nothing and read empty, never an error.
fn dims_of(actor: &Actor) -> Option<Dims> {
    if !(pos_known(actor) && size_known(actor)) {
        return None;
    }
    if !(actor.sx > 0.0 && actor.sy > 0.0 && actor.sz > 0.0) {
        return None;
    }
    let dx = to_cm(actor.sx);
    let dy = to_cm(actor.sy);
    let dz = to_cm(actor.sz);
    if dx <= 0 || dy <= 0 || dz <= 0 {
        return None;
    }
    let (cx, cy, cz) = (to_cm(actor.x), to_cm(actor.y), to_cm(actor.z));
    Some(Dims {
        min_x: cx - dx / 2,
        max_x: cx + dx / 2,
        min_y: cy - dy / 2,
        max_y: cy + dy / 2,
        min_z: cz - dz / 2,
        max_z: cz + dz / 2,
        dx,
        dy,
        dz,
        cy,
    })
}

/// Strict XZ interior overlap (touching edges are not overlap).
fn xz_overlap(a: &Dims, b: &Dims) -> bool {
    a.min_x < b.max_x && b.min_x < a.max_x && a.min_z < b.max_z && b.min_z < a.max_z
}

/// True when the box rests on the ground or on a physical supporter top.
fn supported(all: &BTreeMap<u32, (Dims, bool)>, id: u32, mine: &Dims) -> bool {
    if mine.min_y <= GROUND_CM {
        return true;
    }
    for (other_id, (top, physical)) in all {
        if *other_id == id || !physical {
            continue;
        }
        if !xz_overlap(mine, top) {
            continue;
        }
        let gap = mine.min_y - top.max_y;
        if (0..=REST_GAP_CM).contains(&gap) {
            return true;
        }
    }
    false
}

/// True when no other box starts within `cm` above the top over XZ overlap.
fn clear_above(all: &BTreeMap<u32, (Dims, bool)>, id: u32, mine: &Dims, cm: i32) -> bool {
    for (other_id, (top, _)) in all {
        if *other_id == id {
            continue;
        }
        if !xz_overlap(mine, top) {
            continue;
        }
        if top.min_y >= mine.max_y && top.min_y < mine.max_y + cm {
            return false;
        }
    }
    true
}

/// Largest horizontal extent, in cm.
fn long_side(mine: &Dims) -> i32 {
    mine.dx.max(mine.dz)
}

/// Seat geometry: chair-height, chair-size, stable, knee room above.
fn sittable_geom(all: &BTreeMap<u32, (Dims, bool)>, id: u32, mine: &Dims, physical: bool) -> bool {
    physical
        && mine.dy >= SEAT_MIN_CM
        && mine.dy <= SEAT_MAX_CM
        && mine.dx >= SEAT_SIDE_MIN_CM
        && mine.dx <= SEAT_SIDE_MAX_CM
        && mine.dz >= SEAT_SIDE_MIN_CM
        && mine.dz <= SEAT_SIDE_MAX_CM
        && supported(all, id, mine)
        && clear_above(all, id, mine, SIT_CLEAR_CM)
}

/// Step geometry: low, tread-wide, stable, headroom above the landing.
fn climbable_geom(all: &BTreeMap<u32, (Dims, bool)>, id: u32, mine: &Dims, physical: bool) -> bool {
    physical
        && mine.dy >= STEP_MIN_CM
        && mine.dy <= STEP_MAX_CM
        && mine.dx.min(mine.dz) >= STEP_TREAD_CM
        && supported(all, id, mine)
        && clear_above(all, id, mine, CLIMB_CLEAR_CM)
}

/// Wall geometry: tall enough to hide behind, long enough to matter.
fn cover_geom(all: &BTreeMap<u32, (Dims, bool)>, id: u32, mine: &Dims, physical: bool) -> bool {
    physical
        && mine.dy >= COVER_HEIGHT_CM
        && long_side(mine) >= COVER_LENGTH_CM
        && supported(all, id, mine)
}

/// Deck geometry: wide stable top with headroom to stand under.
fn walkable_geom(all: &BTreeMap<u32, (Dims, bool)>, id: u32, mine: &Dims, physical: bool) -> bool {
    physical
        && mine.dx.min(mine.dz) >= WALK_MIN_CM
        && mine.max_y <= WALK_TOP_MAX_CM
        && supported(all, id, mine)
        && clear_above(all, id, mine, WALK_CLEAR_CM)
}

/// Pad geometry: low flat grounded pad with a free capsule ring around it.
/// With a bake view the tile must also read walkable.
fn spawn_geom(
    all: &BTreeMap<u32, (Dims, bool)>,
    actors: &BTreeMap<u32, &Actor>,
    id: u32,
    mine: &Dims,
    physical: bool,
    nav: Option<&NavView>,
) -> bool {
    if !(physical
        && mine.dy <= SPAWN_HEIGHT_MAX_CM
        && mine.dx >= SPAWN_SIDE_MIN_CM
        && mine.dx <= SPAWN_SIDE_MAX_CM
        && mine.dz >= SPAWN_SIDE_MIN_CM
        && mine.dz <= SPAWN_SIDE_MAX_CM
        && mine.min_y <= GROUND_CM)
    {
        return false;
    }
    for (other_id, (top, other_physical)) in all {
        if *other_id == id || !other_physical {
            continue;
        }
        if top.max_y <= mine.min_y || top.min_y >= mine.min_y + SPAWN_CAPSULE_CM {
            continue;
        }
        let gap_x = (top.min_x - mine.max_x).max(mine.min_x - top.max_x);
        let gap_z = (top.min_z - mine.max_z).max(mine.min_z - top.max_z);
        if gap_x < SPAWN_RING_CM && gap_z < SPAWN_RING_CM {
            return false;
        }
    }
    match nav {
        None => true,
        Some(view) => match actors.get(&id) {
            None => false,
            Some(actor) => {
                let tile = (
                    to_cm(actor.x).div_euclid(NAV_TILE_CM),
                    to_cm(actor.z).div_euclid(NAV_TILE_CM),
                );
                // Unbaked tiles refuse above and read as no spawn here.
                spawn_from_tile(view, tile).unwrap_or(false)
            }
        },
    }
}

/// Door geometry: a baked door leaf whose clear passage is single-file but
/// not a hall. Choke needs the bake view; geometry alone reads none.
fn choke_geom(mine: &Dims, physical: bool, id: u32, nav: Option<&NavView>) -> bool {
    if !physical || mine.min_y > GROUND_CM {
        return false;
    }
    match nav {
        None => false,
        Some(view) => match view.doors.get(&id) {
            None => false,
            // A refused width (negative bake entry) reads as no choke here.
            Some(door) => choke_from_width(door.width_cm).unwrap_or(false),
        },
    }
}

/// Schema-only gate for breakable: solid, known, not a monolith.
fn breakable_gate(mine: &Dims, physical: bool) -> bool {
    physical && long_side(mine).max(mine.dy) <= BREAKABLE_MAX_CM
}

/// Schema-only gate for interactable: solid control at touch height.
fn interact_gate(mine: &Dims, physical: bool) -> bool {
    physical
        && mine.cy >= TOUCH_MIN_CM
        && mine.cy <= TOUCH_MAX_CM
        && long_side(mine).max(mine.dy) <= TOUCH_SIZE_MAX_CM
}

/// Inputs to [`verify_tag`]: the indexed scene plus the agent's clip set.
struct VerifyCtx<'a> {
    /// Integer-cm boxes by id with their solidity.
    all: &'a BTreeMap<u32, (Dims, bool)>,
    /// Actors by id for the spawn tile lookup.
    actors: &'a BTreeMap<u32, &'a Actor>,
    /// Actor the authored tag was written on.
    id: u32,
    /// That actor's box.
    mine: &'a Dims,
    /// That actor's solidity.
    physical: bool,
    /// Bake view for choke doors and spawn tiles.
    nav: Option<&'a NavView>,
    /// Querying agent's clip set.
    caps: AnimCaps,
}

/// Geometry verdict for one authored tag: `None` accepts, `Some` refuses.
/// Geometry is checked first, then the agent's clip set for the animated
/// tags, so a chair is `bad-seat` when too tall and `no-sit-anim` when the
/// agent has no sit clip.
fn verify_tag(tag: Afford, ctx: &VerifyCtx<'_>) -> Option<&'static str> {
    let geom_ok = match tag {
        Afford::Sittable => sittable_geom(ctx.all, ctx.id, ctx.mine, ctx.physical),
        Afford::Climbable => climbable_geom(ctx.all, ctx.id, ctx.mine, ctx.physical),
        Afford::Cover => cover_geom(ctx.all, ctx.id, ctx.mine, ctx.physical),
        Afford::Breakable => breakable_gate(ctx.mine, ctx.physical),
        Afford::Walkable => walkable_geom(ctx.all, ctx.id, ctx.mine, ctx.physical),
        Afford::Interactable => interact_gate(ctx.mine, ctx.physical),
        Afford::Spawn => spawn_geom(ctx.all, ctx.actors, ctx.id, ctx.mine, ctx.physical, ctx.nav),
        Afford::Choke => {
            if ctx.nav.is_none() {
                return Some("needs-nav-view");
            }
            choke_geom(ctx.mine, ctx.physical, ctx.id, ctx.nav)
        }
    };
    if !geom_ok {
        return Some(match tag {
            Afford::Sittable => "bad-seat",
            Afford::Climbable => "bad-step",
            Afford::Cover => "bad-cover",
            Afford::Breakable => "bad-breakable",
            Afford::Walkable => "bad-walk",
            Afford::Interactable => "bad-control",
            Afford::Spawn => "bad-spawn",
            Afford::Choke => "bad-choke",
        });
    }
    match tag {
        Afford::Sittable if !ctx.caps.has_sit() => Some("no-sit-anim"),
        Afford::Climbable if !ctx.caps.has_climb() => Some("no-climb-anim"),
        Afford::Walkable if !ctx.caps.has_walk() => Some("no-walk-anim"),
        _ => None,
    }
}

/// Derive every actor's tags and flag wrong authored tags.
///
/// Geometry derives sittable, climbable, cover, walkable, spawn and choke
/// (choke and the spawn tile check need `nav`); authored rows only add
/// breakable and interactable when their gates pass. Every other authored
/// tag is verified: agreement is a no-op (geometry already says it) and
/// disagreement is a [`Mistag`]. Authored ids missing from the scene are
/// mistags too (`unknown-actor`); degenerate actors read empty tags.
/// Output is id-sorted, so any input order gives the same vectors.
/// Runs with [`AnimCaps::ALL`]; use [`derive_for_agent`] for an agent whose
/// clip set lacks sit, climb or locomotion.
pub fn derive_all(
    actors: &[Actor],
    authored: &[Authored],
    nav: Option<&NavView>,
) -> (Vec<ActorTags>, Vec<Mistag>) {
    derive_for_agent(actors, authored, nav, AnimCaps::ALL)
}

/// Derive every actor's tags for one agent's clip set.
///
/// Same contract as [`derive_all`], except the animated tags additionally
/// need the matching [`AnimCaps`]: sittable needs a sit clip, climbable a
/// climb clip, walkable a locomotion set. Cover, spawn, choke, breakable and
/// interactable derive exactly as in [`derive_all`]; a verified authored tag
/// whose geometry passes but whose clip is missing is refused with a
/// `no-*-anim` reason instead of being applied.
pub fn derive_for_agent(
    actors: &[Actor],
    authored: &[Authored],
    nav: Option<&NavView>,
    caps: AnimCaps,
) -> (Vec<ActorTags>, Vec<Mistag>) {
    let by_id: BTreeMap<u32, &Actor> = actors.iter().map(|a| (a.id, a)).collect();
    let mut boxes: BTreeMap<u32, (Dims, bool)> = BTreeMap::new();
    for (id, actor) in &by_id {
        if let Some(dims) = dims_of(actor) {
            boxes.insert(*id, (dims, actor.physical));
        }
    }
    let mut wrote: BTreeMap<u32, AffordSet> = BTreeMap::new();
    for id in by_id.keys() {
        let Some((mine, physical)) = boxes.get(id) else {
            wrote.insert(*id, AffordSet::empty());
            continue;
        };
        let mut set = AffordSet::empty();
        if caps.has_sit() && sittable_geom(&boxes, *id, mine, *physical) {
            set.insert(Afford::Sittable);
        }
        if caps.has_climb() && climbable_geom(&boxes, *id, mine, *physical) {
            set.insert(Afford::Climbable);
        }
        if cover_geom(&boxes, *id, mine, *physical) {
            set.insert(Afford::Cover);
        }
        if caps.has_walk() && walkable_geom(&boxes, *id, mine, *physical) {
            set.insert(Afford::Walkable);
        }
        if spawn_geom(&boxes, &by_id, *id, mine, *physical, nav) {
            set.insert(Afford::Spawn);
        }
        if choke_geom(mine, *physical, *id, nav) {
            set.insert(Afford::Choke);
        }
        wrote.insert(*id, set);
    }
    let mut mistags: Vec<Mistag> = Vec::new();
    apply_authored(
        &mut wrote,
        &mut mistags,
        &boxes,
        &by_id,
        authored,
        nav,
        caps,
    );
    let mut out: Vec<ActorTags> = wrote
        .into_iter()
        .map(|(id, tags)| ActorTags { id, tags })
        .collect();
    out.sort_by_key(|r| r.id);
    mistags.sort_by_key(|m| (m.id, m.afford));
    (out, mistags)
}

/// Refusal code for an authored tag on a degenerate (unindexed) actor.
/// The nav-view check comes first, then the clip set, then geometry.
fn degenerate_reason(tag: Afford, nav: Option<&NavView>, caps: AnimCaps) -> &'static str {
    match tag {
        Afford::Choke if nav.is_none() => "needs-nav-view",
        Afford::Sittable if !caps.has_sit() => "no-sit-anim",
        Afford::Climbable if !caps.has_climb() => "no-climb-anim",
        Afford::Walkable if !caps.has_walk() => "no-walk-anim",
        Afford::Sittable => "bad-seat",
        Afford::Climbable => "bad-step",
        Afford::Cover => "bad-cover",
        Afford::Breakable => "bad-breakable",
        Afford::Walkable => "bad-walk",
        Afford::Interactable => "bad-control",
        Afford::Spawn => "bad-spawn",
        Afford::Choke => "bad-choke",
    }
}

/// Verify authored rows against geometry: admit the schema-owned breakable
/// and interactable tags whose gates pass, refuse everything else that the
/// measurements deny. Merges duplicate rows for one id first.
fn apply_authored(
    wrote: &mut BTreeMap<u32, AffordSet>,
    mistags: &mut Vec<Mistag>,
    boxes: &BTreeMap<u32, (Dims, bool)>,
    by_id: &BTreeMap<u32, &Actor>,
    authored: &[Authored],
    nav: Option<&NavView>,
    caps: AnimCaps,
) {
    let mut seen: BTreeMap<u32, AffordSet> = BTreeMap::new();
    for row in authored {
        let entry = seen.entry(row.id).or_default();
        for tag in Afford::ALL {
            if row.tags.contains(tag) {
                entry.insert(tag);
            }
        }
    }
    for (id, tags) in &seen {
        if !by_id.contains_key(id) {
            refuse_all(mistags, *id, *tags, "unknown-actor");
            continue;
        }
        let Some((mine, physical)) = boxes.get(id) else {
            for tag in Afford::ALL {
                if tags.contains(tag) {
                    mistags.push(Mistag {
                        id: *id,
                        afford: tag,
                        reason: degenerate_reason(tag, nav, caps),
                    });
                }
            }
            continue;
        };
        for tag in Afford::ALL {
            if !tags.contains(tag) {
                continue;
            }
            let ctx = VerifyCtx {
                all: boxes,
                actors: by_id,
                id: *id,
                mine,
                physical: *physical,
                nav,
                caps,
            };
            if let Some(reason) = verify_tag(tag, &ctx) {
                mistags.push(Mistag {
                    id: *id,
                    afford: tag,
                    reason,
                });
            } else if matches!(tag, Afford::Breakable | Afford::Interactable) {
                if let Some(final_tags) = wrote.get_mut(id) {
                    final_tags.insert(tag);
                }
            }
        }
    }
}

/// Refuse every tag in a set with one shared reason.
fn refuse_all(mistags: &mut Vec<Mistag>, id: u32, tags: AffordSet, reason: &'static str) {
    for tag in Afford::ALL {
        if tags.contains(tag) {
            mistags.push(Mistag {
                id,
                afford: tag,
                reason,
            });
        }
    }
}

/// Look up one derived actor's tags. Unknown ids refuse loudly.
pub fn tags_of(tags: &[ActorTags], id: u32) -> Result<AffordSet, AffordError> {
    for row in tags {
        if row.id == id {
            return Ok(row.tags);
        }
    }
    Err(AffordError::UnknownActor(id))
}

/// Order-invariant digest over `(id, tags)`: sorted by id, then tag plus
/// LE integer bytes. Same tags in any order give the same digest; any
/// flipped tag flips it. No float ever reaches the digest.
pub fn digest_tags(tags: &[ActorTags]) -> u64 {
    let mut sorted = tags.to_vec();
    sorted.sort_by_key(|r| r.id);
    let mut hash = Fnv64::new();
    hash.write(AFFORD_TAG);
    for row in &sorted {
        hash.write_u32(row.id);
        hash.write_u8(row.tags.bits());
    }
    hash.finish()
}

/// Brief rows for derived tags: `afford <id> <csv>` in id order.
pub fn afford_lines(tags: &[ActorTags]) -> Vec<String> {
    let mut sorted = tags.to_vec();
    sorted.sort_by_key(|r| r.id);
    sorted
        .iter()
        .map(|row| format!("afford {} {}\n", row.id, row.tags.csv()))
        .collect()
}

/// Stable rendering of [`afford_lines`] for golden tests.
pub fn afford_text(tags: &[ActorTags]) -> String {
    afford_lines(tags).concat()
}

/// Headcount per affordance in [`Afford::ALL`] order, for brief summaries.
pub fn count_by_afford(tags: &[ActorTags]) -> [usize; 8] {
    let mut counts = [0usize; 8];
    for row in tags {
        for (i, tag) in Afford::ALL.iter().enumerate() {
            if row.tags.contains(*tag) {
                counts[i] += 1;
            }
        }
    }
    counts
}

// ======================================================================
// Rule-derived affordances over the relation/sight vocabularies.
//
// The rule table lives in [`rules`]; this module re-exports its public
// surface so `afford::ASSERT_RULES` paths keep working. Behavior is
// unchanged: rule tags equal [`derive_for_agent`] tags on the gold room.
#[path = "rules.rs"]
pub mod rules;
#[cfg(test)]
pub(crate) use rules::assert_for;
pub use rules::{
    derive_rules_for_agent, derive_with_rules, AssertRule, Premise, Proof, ProofStep, RefuseRule,
    RuleOutcome, RuleRefusal, RuleState, ASSERT_RULES, MAX_RULES, NO_WITNESS, REFUSE_RULES,
    RULE_COUNT,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: format!("prop_{id:02}"),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn ghost(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            physical: false,
            ..prop(id, x, y, z, sx, sy, sz)
        }
    }

    fn set(list: &[Afford]) -> AffordSet {
        AffordSet::from_list(list)
    }

    use Afford::{Breakable, Choke, Climbable, Cover, Interactable, Sittable, Spawn, Walkable};

    /// The 50-object gold room: one 6 m lane, every object isolated so only
    /// its own geometry decides (doors refine through the bake view).
    /// Specs are `(count, y, sx, sy, sz)`; ids and lane positions follow.
    fn gold_room() -> Vec<Actor> {
        let specs: [(usize, f32, f32, f32, f32); 16] = [
            (6, 0.225, 0.45, 0.45, 0.45), // chairs 0-5
            (4, 0.15, 0.4, 0.3, 0.4),     // stools 6-9
            (4, 0.5, 1.0, 1.0, 1.0),      // crates 10-13
            (4, 1.5, 0.5, 3.0, 4.0),      // walls 14-17
            (4, 0.45, 0.5, 0.9, 3.0),     // low walls 18-21
            (3, 0.1, 3.0, 0.2, 3.0),      // floors 22-24
            (3, 0.05, 1.2, 0.1, 1.2),     // spawn pads 25-27
            (3, 1.2, 0.2, 0.2, 0.2),      // buttons 28-30
            (3, 0.75, 1.0, 1.5, 0.05),    // glass panes 31-33
            (3, 2.0, 0.4, 4.0, 0.4),      // pillars 34-36
            (3, 0.05, 0.1, 0.1, 0.1),     // pebbles 37-39
            (3, 1.1, 1.0, 2.2, 0.1),      // doors 40-42
            (2, 1.1, 1.0, 2.2, 0.1),      // wide doors 43-44
            (1, 1.1, 1.0, 2.2, 0.1),      // narrow door 45
            (2, 3.0, 1.0, 1.0, 1.0),      // floating crates 46-47
            (2, 0.375, 1.5, 0.75, 0.9),   // tables 48-49
        ];
        let mut actors: Vec<Actor> = Vec::new();
        let mut id = 0u32;
        let mut x = 0.0f32;
        for (n, y, sx, sy, sz) in specs {
            for _ in 0..n {
                actors.push(prop(id, x, y, 0.0, sx, sy, sz));
                id += 1;
                x += 6.0;
            }
        }
        actors
    }

    /// Bake view for the gold room: choke doors plus walkable pad tiles.
    fn gold_nav(actors: &[Actor]) -> NavView {
        use std::collections::BTreeMap;
        let mut doors = BTreeMap::new();
        for id in [40u32, 41, 42] {
            doors.insert(
                id,
                crate::Door {
                    width_cm: 100,
                    two_way: false,
                },
            );
        }
        for id in [43u32, 44] {
            doors.insert(
                id,
                crate::Door {
                    width_cm: 300,
                    two_way: false,
                },
            );
        }
        doors.insert(
            45u32,
            crate::Door {
                width_cm: 50,
                two_way: false,
            },
        );
        let mut tiles = BTreeMap::new();
        for actor in actors {
            tiles.insert(
                (
                    to_cm(actor.x).div_euclid(NAV_TILE_CM),
                    to_cm(actor.z).div_euclid(NAV_TILE_CM),
                ),
                crate::Tile { walk: true },
            );
        }
        NavView {
            tiles,
            links: Vec::new(),
            flow: BTreeMap::new(),
            doors,
            spawn_id: 25,
        }
    }

    /// Schema tags for the gold room: breakable glass, touchable buttons.
    fn gold_authored() -> Vec<Authored> {
        let mut out = Vec::new();
        for id in [31u32, 32, 33] {
            out.push(Authored::new(id, set(&[Breakable])));
        }
        for id in [28u32, 29, 30] {
            out.push(Authored::new(id, set(&[Interactable])));
        }
        out
    }

    /// Expected tags per gold id, in id order.
    fn gold_expected() -> Vec<(u32, AffordSet)> {
        let sit_climb = set(&[Sittable, Climbable]);
        let climb = set(&[Climbable]);
        let crate_set = set(&[Climbable, Cover, Walkable]);
        let wall = set(&[Cover]);
        let low_wall = set(&[Climbable, Cover]);
        let floor = set(&[Walkable]);
        let pad = set(&[Walkable, Spawn]);
        let button = set(&[Interactable]);
        let glass = set(&[Cover, Breakable]);
        let none = AffordSet::empty();
        let door = set(&[Cover, Choke]);
        let table = set(&[Climbable, Cover, Walkable]);
        let mut want = Vec::new();
        for id in 0..6 {
            want.push((id, sit_climb));
        }
        for id in 6..10 {
            want.push((id, climb));
        }
        for id in 10..14 {
            want.push((id, crate_set));
        }
        for id in 14..18 {
            want.push((id, wall));
        }
        for id in 18..22 {
            want.push((id, low_wall));
        }
        for id in 22..25 {
            want.push((id, floor));
        }
        for id in 25..28 {
            want.push((id, pad));
        }
        for id in 28..31 {
            want.push((id, button));
        }
        for id in 31..34 {
            want.push((id, glass));
        }
        for id in 34..40 {
            want.push((id, none));
        }
        for id in 40..43 {
            want.push((id, door));
        }
        for id in 43..46 {
            want.push((id, wall));
        }
        for id in 46..48 {
            want.push((id, none));
        }
        for id in 48..50 {
            want.push((id, table));
        }
        want
    }

    #[test]
    fn gold_fifty_objects_rate() {
        let actors = gold_room();
        assert_eq!(actors.len(), 50);
        let nav = gold_nav(&actors);
        let (tags, mistags) = derive_all(&actors, &gold_authored(), Some(&nav));
        assert!(mistags.is_empty(), "gold room mistags: {mistags:?}");
        let want = gold_expected();
        let mut failures: Vec<String> = Vec::new();
        for (id, want_set) in &want {
            let got = tags_of(&tags, *id).expect("gold id derived");
            if got != *want_set {
                failures.push(format!(
                    "id {id}: got {} want {}",
                    got.csv(),
                    want_set.csv()
                ));
            }
        }
        let pass = want.len() - failures.len();
        assert!(
            pass * 100 >= 90 * want.len(),
            "afford gold {pass}/{} < 90%: {failures:?}",
            want.len()
        );
        assert!(failures.is_empty(), "gold failures: {failures:?}");
    }

    #[test]
    fn seeded_mistags_all_flagged() {
        let mut actors = gold_room();
        actors.push(ghost(60, 300.0, 1.0, 0.0, 1.0, 2.0, 1.0));
        let nav = gold_nav(&actors);
        let authored = vec![
            Authored::new(14, set(&[Sittable])),     // wall tagged seat
            Authored::new(37, set(&[Cover])),        // pebble tagged cover
            Authored::new(28, set(&[Sittable])),     // button tagged seat
            Authored::new(40, set(&[Spawn])),        // door leaf tagged spawn
            Authored::new(43, set(&[Choke])),        // wide gate tagged choke
            Authored::new(60, set(&[Breakable])),    // ghost tagged breakable
            Authored::new(60, set(&[Interactable])), // ghost tagged control
            Authored::new(999, set(&[Sittable])),    // schema drift: no such actor
            Authored::new(0, set(&[Sittable])),      // agreement: silent
            Authored::new(31, set(&[Breakable])),    // agreement: admitted
        ];
        let (tags, mistags) = derive_all(&actors, &authored, Some(&nav));
        let reasons: Vec<(u32, Afford, &str)> =
            mistags.iter().map(|m| (m.id, m.afford, m.reason)).collect();
        for seeded in [
            (14u32, Sittable, "bad-seat"),
            (37, Cover, "bad-cover"),
            (28, Sittable, "bad-seat"),
            (40, Spawn, "bad-spawn"),
            (43, Choke, "bad-choke"),
            (60, Breakable, "bad-breakable"),
            (60, Interactable, "bad-control"),
            (999, Sittable, "unknown-actor"),
        ] {
            assert!(
                reasons.contains(&seeded),
                "missing {seeded:?} in {reasons:?}"
            );
        }
        assert_eq!(mistags.len(), 8, "agreements stay silent: {reasons:?}");
        // Geometry wins: refused tags never land on the actor.
        assert!(!tags_of(&tags, 14).expect("wall derived").contains(Sittable));
        assert!(!tags_of(&tags, 37).expect("pebble derived").contains(Cover));
        // Admitted schema tags do land.
        assert!(tags_of(&tags, 31)
            .expect("glass derived")
            .contains(Breakable));
    }

    #[test]
    fn agent_caps_all_matches_derive_all() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let (base_tags, base_mistags) = derive_all(&actors, &gold_authored(), Some(&nav));
        let (agent_tags, agent_mistags) =
            derive_for_agent(&actors, &gold_authored(), Some(&nav), AnimCaps::ALL);
        assert_eq!(agent_tags, base_tags, "ALL caps reproduce derive_all");
        assert_eq!(agent_mistags, base_mistags, "ALL caps mistags match");
        assert_eq!(
            digest_tags(&agent_tags),
            digest_tags(&base_tags),
            "ALL caps digest matches"
        );
    }

    #[test]
    fn agent_caps_none_drops_animated_tags() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let (tags, mistags) =
            derive_for_agent(&actors, &gold_authored(), Some(&nav), AnimCaps::NONE);
        assert!(
            mistags.is_empty(),
            "gold authored needs no anim: {mistags:?}"
        );
        let chair = tags_of(&tags, 0).expect("chair derived");
        assert!(
            chair.is_empty(),
            "chair needs sit+climb clips: {}",
            chair.csv()
        );
        let wall = tags_of(&tags, 14).expect("wall derived");
        assert_eq!(wall, set(&[Cover]));
        let low_wall = tags_of(&tags, 18).expect("low wall derived");
        assert_eq!(low_wall, set(&[Cover]));
        let floor = tags_of(&tags, 22).expect("floor derived");
        assert!(floor.is_empty(), "deck needs locomotion: {}", floor.csv());
        let pad = tags_of(&tags, 25).expect("pad derived");
        assert_eq!(pad, set(&[Spawn]));
        let button = tags_of(&tags, 28).expect("button derived");
        assert_eq!(button, set(&[Interactable]));
        let glass = tags_of(&tags, 31).expect("glass derived");
        assert_eq!(glass, set(&[Cover, Breakable]));
        let door = tags_of(&tags, 40).expect("door derived");
        assert_eq!(door, set(&[Cover, Choke]));
        let table = tags_of(&tags, 48).expect("table derived");
        assert_eq!(table, set(&[Cover]));
        // Headcount in ALL order: every animated tag drops, every static
        // tag holds (cover 23, breakable 3, interactable 3, spawn 3, choke 3).
        let counts = count_by_afford(&tags);
        assert_eq!(counts, [0, 0, 23, 3, 0, 3, 3, 3], "counts: {counts:?}");
    }

    #[test]
    fn agent_caps_partial_gates_each_action() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let caps = AnimCaps::SIT.union(AnimCaps::WALK);
        let (tags, _) = derive_for_agent(&actors, &gold_authored(), Some(&nav), caps);
        let chair = tags_of(&tags, 0).expect("chair derived");
        assert_eq!(chair, set(&[Sittable]), "sit clip only: {}", chair.csv());
        let crate_tags = tags_of(&tags, 10).expect("crate derived");
        assert_eq!(
            crate_tags,
            set(&[Cover, Walkable]),
            "no climb clip: {}",
            crate_tags.csv()
        );
        assert!(AnimCaps::empty().is_empty());
        assert!(!AnimCaps::all().is_empty());
    }

    #[test]
    fn authored_refused_without_matching_clip() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let authored = vec![
            Authored::new(0, set(&[Sittable])), // geometry agrees, clip missing
            Authored::new(31, set(&[Breakable])), // schema tag, needs no clip
            Authored::new(10, set(&[Climbable])), // geometry agrees, clip missing
        ];
        let (tags, mistags) = derive_for_agent(&actors, &authored, Some(&nav), AnimCaps::WALK);
        let reasons: Vec<(u32, Afford, &str)> =
            mistags.iter().map(|m| (m.id, m.afford, m.reason)).collect();
        assert!(
            reasons.contains(&(0u32, Sittable, "no-sit-anim")),
            "missing sit refusal: {reasons:?}"
        );
        assert!(
            reasons.contains(&(10u32, Climbable, "no-climb-anim")),
            "missing climb refusal: {reasons:?}"
        );
        assert_eq!(mistags.len(), 2, "breakable admits silently: {reasons:?}");
        assert!(!tags_of(&tags, 0).expect("chair derived").contains(Sittable));
        assert!(tags_of(&tags, 31)
            .expect("glass derived")
            .contains(Breakable));
    }

    /// Acceptance probe: one afford query over the 50-object gold stays under
    /// 0.2 ms per object. Minimum-of-11, so scheduler noise can only slow the
    /// reading, never flatter it; integer nanoseconds throughout, no float.
    #[test]
    #[ignore = "timing: afford budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn afford_query_under_point_two_ms_per_object() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let authored = gold_authored();
        // Warm caches and the allocator once, outside the measurement.
        let _ = derive_all(&actors, &authored, Some(&nav));
        let mut best_nanos: u128 = u128::MAX;
        for _ in 0..11 {
            let start = std::time::Instant::now();
            let (tags, _) = derive_all(&actors, &authored, Some(&nav));
            let elapsed_nanos = start.elapsed().as_nanos();
            assert_eq!(tags.len(), 50, "gold room stays 50 objects");
            if elapsed_nanos < best_nanos {
                best_nanos = elapsed_nanos;
            }
        }
        let per_object_nanos = best_nanos / 50;
        assert!(
            per_object_nanos <= 200_000,
            "afford query {per_object_nanos} ns/object over budget 200_000 ns/object"
        );
    }

    #[test]
    fn choke_needs_bake_view() {
        let actors = gold_room();
        let (tags, mistags) = derive_all(&actors, &gold_authored(), None);
        // Geometry alone reads no choke anywhere.
        for row in &tags {
            assert!(
                !row.tags.contains(Choke),
                "id {} choked without bake",
                row.id
            );
        }
        // Spawn still derives from geometry alone.
        assert!(tags_of(&tags, 25).expect("pad derived").contains(Spawn));
        // An authored choke with no bake view refuses loudly.
        let (_, mistags2) = derive_all(&actors, &[Authored::new(40, set(&[Choke]))], None);
        assert_eq!(
            mistags2,
            vec![Mistag {
                id: 40,
                afford: Choke,
                reason: "needs-nav-view"
            }]
        );
        let _ = mistags;
    }

    #[test]
    fn nav_refines_spawn_tiles() {
        let actors = gold_room();
        let mut nav = gold_nav(&actors);
        // Pad 25 sits on a blocked tile now: spawn drops, walkable stays.
        let pad = actors.iter().find(|a| a.id == 25).expect("pad 25");
        nav.tiles.insert(
            (
                to_cm(pad.x).div_euclid(NAV_TILE_CM),
                to_cm(pad.z).div_euclid(NAV_TILE_CM),
            ),
            crate::Tile { walk: false },
        );
        let (tags, _) = derive_all(&actors, &gold_authored(), Some(&nav));
        let pad_tags = tags_of(&tags, 25).expect("pad derived");
        assert!(!pad_tags.contains(Spawn));
        assert!(pad_tags.contains(Walkable));
        let other = tags_of(&tags, 26).expect("pad 26 derived");
        assert!(other.contains(Spawn));
    }

    #[test]
    fn nav_tag_fns_agree_with_hand_tags() {
        // Hand tags: (door width cm, want choke) from the single-file band.
        let widths: [(i32, bool); 10] = [
            (80, true),
            (90, true),
            (100, true),
            (120, true),
            (150, true),
            (180, true),
            (200, false),
            (300, false),
            (50, false),
            (20, false),
        ];
        let mut pass = 0usize;
        for (width, want) in widths {
            let got = choke_from_width(width).expect("hand width derives");
            if got == want {
                pass += 1;
            }
        }
        assert!(
            pass * 100 >= 90 * widths.len(),
            "nav choke {pass}/{} < 90%",
            widths.len()
        );
        assert_eq!(pass, widths.len(), "every hand choke tag agrees");
        // Tile hand tags: walkable reads spawnable, blocked does not.
        let mut tiles = std::collections::BTreeMap::new();
        tiles.insert((0, 0), crate::Tile { walk: true });
        tiles.insert((1, 0), crate::Tile { walk: false });
        let nav = NavView {
            tiles,
            links: Vec::new(),
            flow: std::collections::BTreeMap::new(),
            doors: std::collections::BTreeMap::new(),
            spawn_id: 0,
        };
        assert_eq!(spawn_from_tile(&nav, (0, 0)), Ok(true));
        assert_eq!(spawn_from_tile(&nav, (1, 0)), Ok(false));
        // Failure cases refuse loudly instead of guessing.
        assert_eq!(choke_from_width(-5), Err(NavTagError::BadDoorWidth(-5)));
        assert_eq!(
            spawn_from_tile(&nav, (9, 9)),
            Err(NavTagError::UnknownTile((9, 9)))
        );
        assert_eq!(
            cover_approach_tiles(&[], None),
            Err(NavTagError::MissingNavView)
        );
        assert_eq!(
            format!("{}", NavTagError::MissingNavView),
            "nav tag: needs a bake view (pass Some)"
        );
    }

    #[test]
    fn seeded_narrow_doors_all_flagged() {
        // Seeded narrow leaves: clear 60..=160 cm, every one must flag.
        let mut doors = std::collections::BTreeMap::new();
        for (id, width) in [80, 100, 120, 150, 180].iter().enumerate() {
            doors.insert(
                id as u32,
                crate::Door {
                    width_cm: *width,
                    two_way: false,
                },
            );
        }
        doors.insert(
            9u32,
            crate::Door {
                width_cm: 300,
                two_way: false,
            },
        );
        let nav = NavView {
            tiles: std::collections::BTreeMap::new(),
            links: Vec::new(),
            flow: std::collections::BTreeMap::new(),
            doors,
            spawn_id: 0,
        };
        let mut flagged = 0usize;
        for id in 0..5u32 {
            let width = nav.doors.get(&id).expect("seeded door baked").width_cm;
            assert_eq!(choke_from_width(width), Ok(true), "narrow door {id} flags");
            flagged += 1;
        }
        assert_eq!(flagged, 5, "100% seeded narrow-door flags");
        let wide = nav.doors.get(&9).expect("wide door baked").width_cm;
        assert_eq!(choke_from_width(wide), Ok(false));
    }

    #[test]
    fn cover_approach_beside_cover_only() {
        // One cover wall at the origin: tiles x -1..=0, z -2..=2.
        let actors = vec![prop(0, 0.0, 1.5, 0.0, 0.5, 3.0, 4.0)];
        let mut tiles = std::collections::BTreeMap::new();
        for tx in -2..=1 {
            for tz in -3..=3 {
                tiles.insert((tx, tz), crate::Tile { walk: true });
            }
        }
        // One blocked neighbor never stages.
        tiles.insert((1, 0), crate::Tile { walk: false });
        let nav = NavView {
            tiles,
            links: Vec::new(),
            flow: std::collections::BTreeMap::new(),
            doors: std::collections::BTreeMap::new(),
            spawn_id: 0,
        };
        let got = cover_approach_tiles(&actors, Some(&nav)).expect("wall approaches");
        let want = vec![
            (-2, -2),
            (-2, -1),
            (-2, 0),
            (-2, 1),
            (-2, 2),
            (-1, -3),
            (-1, 3),
            (0, -3),
            (0, 3),
            (1, -2),
            (1, -1),
            (1, 1),
            (1, 2),
        ];
        assert_eq!(got, want, "walkable 4-neighbors of cover, sorted");
        // Degenerate actors index nothing and never panic.
        let lost = prop(1, f32::NAN, 0.5, 0.0, 1.0, 1.0, 1.0);
        let got2 = cover_approach_tiles(&[lost], Some(&nav)).expect("degenerate reads");
        assert!(got2.is_empty(), "no cover means no approaches");
    }

    #[test]
    fn flat_room_golden_identical_with_nav_queries() {
        // The small flat golden never moves: pure tile queries ride beside
        // the tags, and the tag text plus digest stay byte-identical.
        let actors = vec![
            prop(0, 0.0, 0.225, 0.0, 0.45, 0.45, 0.45),
            prop(1, 6.0, 1.5, 0.0, 0.5, 3.0, 4.0),
            prop(2, 12.0, 0.05, 0.0, 0.1, 0.1, 0.1),
        ];
        let (tags, mistags) = derive_all(&actors, &[], None);
        assert!(mistags.is_empty());
        assert_eq!(
            afford_text(&tags),
            "afford 0 sittable,climbable\n\
             afford 1 cover\n\
             afford 2 none\n"
        );
        let digest = digest_tags(&tags);
        let mut tiles = std::collections::BTreeMap::new();
        for actor in &actors {
            tiles.insert(
                (
                    to_cm(actor.x).div_euclid(NAV_TILE_CM),
                    to_cm(actor.z).div_euclid(NAV_TILE_CM),
                ),
                crate::Tile { walk: true },
            );
        }
        // Bake the walkable patch around the wall so approaches can stage.
        for tx in 4..=8 {
            for tz in -3..=3 {
                tiles.insert((tx, tz), crate::Tile { walk: true });
            }
        }
        let nav = NavView {
            tiles,
            links: Vec::new(),
            flow: std::collections::BTreeMap::new(),
            doors: std::collections::BTreeMap::new(),
            spawn_id: 0,
        };
        let (tags_nav, _) = derive_all(&actors, &[], Some(&nav));
        assert_eq!(afford_text(&tags_nav), afford_text(&tags));
        assert_eq!(digest_tags(&tags_nav), digest);
        // The wall stages approaches on its walkable neighbors.
        let approaches = cover_approach_tiles(&actors, Some(&nav)).expect("flat approaches");
        assert!(!approaches.is_empty(), "cover stages approaches");
        assert!(
            approaches.windows(2).all(|w| w[0] < w[1]),
            "approach tiles stay sorted: {approaches:?}"
        );
    }

    #[test]
    fn digest_stable_twice_and_order_invariant() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let (tags, _) = derive_all(&actors, &gold_authored(), Some(&nav));
        let d1 = digest_tags(&tags);
        let d2 = digest_tags(&tags);
        assert_eq!(d1, d2, "digest stable across runs");
        let mut rev = actors.clone();
        rev.reverse();
        let (tags_rev, _) = derive_all(&rev, &gold_authored(), Some(&nav));
        assert_eq!(digest_tags(&tags_rev), d1, "digest order-invariant");
        let mut flipped = tags.clone();
        let chair = flipped.iter_mut().find(|r| r.id == 0).expect("chair 0");
        chair.tags = set(&[Sittable]);
        assert_ne!(
            digest_tags(&flipped),
            d1,
            "one flipped tag flips the digest"
        );
    }

    #[test]
    fn golden_afford_text_small_scene() {
        let actors = vec![
            prop(0, 0.0, 0.225, 0.0, 0.45, 0.45, 0.45),
            prop(1, 6.0, 1.5, 0.0, 0.5, 3.0, 4.0),
            prop(2, 12.0, 0.05, 0.0, 0.1, 0.1, 0.1),
        ];
        let (tags, mistags) = derive_all(&actors, &[], None);
        assert!(mistags.is_empty());
        assert_eq!(
            afford_text(&tags),
            "afford 0 sittable,climbable\n\
             afford 1 cover\n\
             afford 2 none\n"
        );
    }

    #[test]
    fn brief_consumer_shape_afford_rows_append() {
        // The `scene.brief` wiring this module feeds: digest groups plus one
        // afford row per actor, all stable under input reorder.
        let actors = vec![
            prop(0, 0.0, 0.225, 0.0, 0.45, 0.45, 0.45),
            prop(1, 6.0, 1.5, 0.0, 0.5, 3.0, 4.0),
        ];
        let graph = crate::relation::build(&actors);
        let report = crate::run(&actors);
        let brief = crate::brief(&actors, &graph, &report, usize::MAX);
        let (tags, _) = derive_all(&actors, &[], None);
        let text = format!("{}{}", crate::brief_text(&brief), afford_text(&tags));
        assert!(text.starts_with("digest "), "digest line stays mandatory");
        assert!(text.contains("afford 0 sittable,climbable\n"));
        assert!(text.contains("afford 1 cover\n"));
        let mut rev = actors.clone();
        rev.reverse();
        let graph2 = crate::relation::build(&rev);
        let report2 = crate::run(&rev);
        let brief2 = crate::brief(&rev, &graph2, &report2, usize::MAX);
        let (tags2, _) = derive_all(&rev, &[], None);
        let text2 = format!("{}{}", crate::brief_text(&brief2), afford_text(&tags2));
        assert_eq!(text, text2, "brief plus affords order-invariant");
    }

    #[test]
    fn degenerate_rows_read_empty() {
        let mut flat = prop(0, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0);
        flat.sy = 0.0;
        let mut lost = prop(1, 0.0, 0.5, 6.0, 1.0, 1.0, 1.0);
        lost.x = f32::NAN;
        let (tags, _) = derive_all(&[flat, lost], &[], None);
        assert_eq!(tags_of(&tags, 0).expect("flat derived"), AffordSet::empty());
        assert_eq!(tags_of(&tags, 1).expect("lost derived"), AffordSet::empty());
        // Authored tags on degenerate rows refuse instead of guessing.
        let (_, mistags) = derive_all(
            &[prop(7, 0.0, f32::NAN, 0.0, 1.0, 1.0, 1.0)],
            &[Authored::new(7, set(&[Walkable]))],
            None,
        );
        assert_eq!(
            mistags,
            vec![Mistag {
                id: 7,
                afford: Walkable,
                reason: "bad-walk"
            }]
        );
    }

    #[test]
    fn errors_name_their_cause() {
        let (tags, _) = derive_all(&[], &[], None);
        assert!(tags.is_empty());
        assert_eq!(tags_of(&tags, 7), Err(AffordError::UnknownActor(7)));
        assert_eq!(
            format!("{}", AffordError::UnknownActor(7)),
            "afford: unknown actor 7 (derive it first)"
        );
        assert_eq!(Afford::Cover.name(), "cover");
        assert_eq!(AffordSet::empty().csv(), "none");
        assert_eq!(set(&[Cover, Sittable]).csv(), "sittable,cover");
        assert_eq!(set(&[Cover, Sittable]).len(), 2);
        assert!(AffordSet::empty().is_empty());
        let counts = count_by_afford(&[
            ActorTags {
                id: 0,
                tags: set(&[Sittable, Cover]),
            },
            ActorTags {
                id: 1,
                tags: set(&[Cover]),
            },
        ]);
        // ALL order is sittable, climbable, cover, breakable, walkable,
        // interactable, spawn, choke.
        assert_eq!(counts, [1, 0, 2, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn rules_table_capped_at_thirty() {
        assert_eq!(ASSERT_RULES.len(), 8, "one assert per affordance");
        assert_eq!(REFUSE_RULES.len(), 13, "one refuse per mistag reason");
        let mut seen = std::collections::BTreeSet::new();
        for rule in &ASSERT_RULES {
            assert_eq!(
                usize::from(rule.id),
                assert_for(rule.tag),
                "assert id matches tag"
            );
            assert!(seen.insert(rule.id), "duplicate rule id {}", rule.id);
        }
        let assert_len = u8::try_from(ASSERT_RULES.len()).expect("8 asserts fit in u8");
        for rule in &REFUSE_RULES {
            assert!(
                rule.watches < assert_len || rule.watches == u8::MAX,
                "refuse {} watches a live assert",
                rule.name
            );
            assert!(seen.insert(rule.id), "duplicate rule id {}", rule.id);
        }
        assert_eq!(seen.len(), RULE_COUNT, "rule ids stay dense");
        for tag in Afford::ALL {
            assert!(
                ASSERT_RULES.iter().any(|rule| rule.tag == tag),
                "no assert derives {}",
                tag.name()
            );
        }
    }

    #[test]
    fn rules_match_gold_full_agreement() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let authored = gold_authored();
        let (base_tags, base_mistags) = derive_all(&actors, &authored, Some(&nav));
        let outcome = derive_with_rules(&actors, &authored, Some(&nav));
        assert_eq!(
            outcome.tags, base_tags,
            "rule tags equal the base tags on the gold"
        );
        assert_eq!(
            outcome.mistags(),
            base_mistags,
            "rule mistags match the base mistags on the gold"
        );
        let (base_free, mistags_free) = derive_all(&actors, &authored, None);
        let free = derive_with_rules(&actors, &authored, None);
        assert_eq!(free.tags, base_free, "nav-free rule tags match");
        assert_eq!(free.mistags(), mistags_free, "nav-free mistags match");
    }

    #[test]
    fn rules_agent_caps_match_derive_for_agent_on_gold() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let authored = gold_authored();
        for caps in [
            AnimCaps::ALL,
            AnimCaps::NONE,
            AnimCaps::SIT.union(AnimCaps::WALK),
        ] {
            let (base_tags, base_mistags) = derive_for_agent(&actors, &authored, Some(&nav), caps);
            let outcome = derive_rules_for_agent(&actors, &authored, Some(&nav), caps);
            assert_eq!(outcome.tags, base_tags, "rule tags match for {caps:?}");
            assert_eq!(
                outcome.mistags(),
                base_mistags,
                "mistags match for {caps:?}"
            );
        }
    }

    #[test]
    fn rules_refusals_all_carry_derivations() {
        let mut actors = gold_room();
        actors.push(ghost(60, 300.0, 1.0, 0.0, 1.0, 2.0, 1.0));
        let nav = gold_nav(&actors);
        let authored = vec![
            Authored::new(14, set(&[Sittable])),
            Authored::new(37, set(&[Cover])),
            Authored::new(28, set(&[Sittable])),
            Authored::new(40, set(&[Spawn])),
            Authored::new(43, set(&[Choke])),
            Authored::new(60, set(&[Breakable])),
            Authored::new(60, set(&[Interactable])),
            Authored::new(999, set(&[Sittable])),
            Authored::new(0, set(&[Sittable])),
            Authored::new(31, set(&[Breakable])),
        ];
        let outcome = derive_with_rules(&actors, &authored, Some(&nav));
        let (_, base_mistags) = derive_all(&actors, &authored, Some(&nav));
        assert_eq!(
            outcome.mistags(),
            base_mistags,
            "refusals match the base mistags"
        );
        assert_eq!(outcome.refusals.len(), 8, "agreements stay silent");
        for refusal in &outcome.refusals {
            assert!(
                usize::from(refusal.rule) < RULE_COUNT,
                "refusal rule {} out of the table",
                refusal.rule
            );
            let table = REFUSE_RULES
                .iter()
                .find(|rule| rule.id == refusal.rule)
                .expect("refusal names a tabled rule");
            assert_eq!(table.tag, refusal.afford, "rule and tag agree");
            assert_eq!(table.reason, refusal.reason, "rule and reason agree");
            assert!(
                base_mistags.contains(&refusal.mistag()),
                "refusal carries a real mistag"
            );
        }
        let wall = outcome
            .refusals
            .iter()
            .find(|r| r.id == 14 && r.afford == Sittable)
            .expect("wall seat refusal");
        assert_eq!(wall.reason, "bad-seat");
        assert_eq!(wall.failed, Some(Premise::SeatBand));
        assert_eq!(wall.rule_name, "refuse-seat");
    }

    #[test]
    fn rules_incremental_matches_full_with_delta_counts() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let authored = gold_authored();
        let full = RuleState::build(&actors, &authored, Some(&nav), AnimCaps::ALL);
        let room = u64::try_from(actors.len()).expect("room-scale count fits");
        assert_eq!(
            full.last_evals(),
            8 * room,
            "full build evaluates every rule"
        );
        let mut moved = actors.clone();
        if let Some(crate_actor) = moved.iter_mut().find(|a| a.id == 10) {
            crate_actor.x += 0.5;
        }
        let fresh = RuleState::build(&moved, &authored, Some(&nav), AnimCaps::ALL);
        let mut delta = RuleState::build(&actors, &authored, Some(&nav), AnimCaps::ALL);
        let affected = delta.rederive(&[10], &moved, &authored, Some(&nav), AnimCaps::ALL);
        assert_eq!(
            delta.outcome(),
            fresh.outcome(),
            "delta equals a fresh build"
        );
        assert!(
            delta.last_evals() < full.last_evals(),
            "delta re-evaluates less: {} vs {}",
            delta.last_evals(),
            full.last_evals()
        );
        assert!(
            affected < actors.len(),
            "closure stays partial: {affected} of {}",
            actors.len()
        );
        let idle = delta.rederive(&[], &moved, &authored, Some(&nav), AnimCaps::ALL);
        assert_eq!(idle, 0, "empty delta re-evaluates nothing");
        assert_eq!(delta.last_evals(), 0, "empty delta counts no evals");
        assert_eq!(
            delta.outcome(),
            fresh.outcome(),
            "empty delta keeps the outcome"
        );
    }

    #[test]
    fn rules_stacked_chair_refines_with_no_above_proof() {
        // The upper chair floats 52.5 cm over the lower one: the base tags still seat
        // the lower chair (clear above), while the relation `Above` edge refines it
        // away in the rule layer with a witnessed proof.
        let lower = prop(0, 0.0, 0.225, 0.0, 0.45, 0.45, 0.45);
        let upper = prop(1, 0.0, 1.2, 0.0, 0.45, 0.45, 0.45);
        let actors = vec![lower, upper];
        let (base, _) = derive_all(&actors, &[], None);
        assert!(
            tags_of(&base, 0).expect("lower derived").contains(Sittable),
            "the base tags seat the lower chair"
        );
        let outcome = derive_with_rules(&actors, &[], None);
        assert!(
            !tags_of(&outcome.tags, 0)
                .expect("lower derived")
                .contains(Sittable),
            "rules refine the roofed seat away"
        );
        assert_eq!(
            tags_of(&outcome.tags, 1).expect("upper derived"),
            tags_of(&base, 1).expect("upper derived"),
            "the upper chair agrees"
        );
        let authored = vec![Authored::new(0, set(&[Sittable]))];
        let refused = derive_with_rules(&actors, &authored, None);
        assert_eq!(refused.refusals.len(), 1, "one witnessed refusal");
        let refusal = &refused.refusals[0];
        assert_eq!(refusal.reason, "bad-seat");
        assert_eq!(refusal.failed, Some(Premise::NoAbove));
        assert_eq!(refusal.failed_name(), "no-above");
        assert_eq!(refusal.wa, 1, "the proof names the roof");
    }

    #[test]
    fn rules_cover_witnesses_fire_on_gold() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let state = RuleState::build(&actors, &gold_authored(), Some(&nav), AnimCaps::ALL);
        for row in state.tags() {
            if row.tags.contains(Cover) {
                let (occludes0, open400) = state.cover_witness(row.id).expect("fact row cached");
                assert!(
                    occludes0 != NO_WITNESS || open400,
                    "cover {} carries a sight witness or stands open",
                    row.id
                );
            }
        }
    }

    #[test]
    fn rules_bytes_identical_twice_and_feed_brief() {
        let actors = vec![
            prop(0, 0.0, 0.225, 0.0, 0.45, 0.45, 0.45),
            prop(1, 6.0, 1.5, 0.0, 0.5, 3.0, 4.0),
        ];
        let first = derive_with_rules(&actors, &[], None);
        let second = derive_with_rules(&actors, &[], None);
        assert_eq!(first, second, "rule outcome stable across runs");
        assert_eq!(
            digest_tags(&first.tags),
            digest_tags(&second.tags),
            "rule digest stable"
        );
        let (base, _) = derive_all(&actors, &[], None);
        assert_eq!(
            afford_text(&first.tags),
            afford_text(&base),
            "rule rows feed scene.brief byte-identical"
        );
        let graph = crate::relation::build(&actors);
        let report = crate::run(&actors);
        let brief = crate::brief(&actors, &graph, &report, usize::MAX);
        let text = format!("{}{}", crate::brief_text(&brief), afford_text(&first.tags));
        assert!(text.contains("afford 0 sittable,climbable\n"));
        assert!(text.contains("afford 1 cover\n"));
    }

    /// Acceptance probe: one re-derive over the 50-object gold after a
    /// single-actor delta stays under 1 ms. Minimum-of-11, so scheduler noise
    /// can only slow the reading, never flatter it.
    #[test]
    #[ignore = "timing: re-derive budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn rules_rederive_p95_under_1ms() {
        let actors = gold_room();
        let nav = gold_nav(&actors);
        let authored = gold_authored();
        let mut moved = actors.clone();
        if let Some(crate_actor) = moved.iter_mut().find(|a| a.id == 10) {
            crate_actor.x += 0.5;
        }
        let mut state = RuleState::build(&actors, &authored, Some(&nav), AnimCaps::ALL);
        let _ = state.rederive(&[10], &moved, &authored, Some(&nav), AnimCaps::ALL);
        let mut best_nanos: u128 = u128::MAX;
        for _ in 0..11 {
            let start = std::time::Instant::now();
            let affected = state.rederive(&[10], &moved, &authored, Some(&nav), AnimCaps::ALL);
            let elapsed_nanos = start.elapsed().as_nanos();
            assert!(
                affected > 0 && affected < actors.len(),
                "closure stays partial: {affected}"
            );
            if elapsed_nanos < best_nanos {
                best_nanos = elapsed_nanos;
            }
        }
        assert!(
            best_nanos <= 1_000_000,
            "re-derive {best_nanos} ns over budget 1_000_000 ns"
        );
    }
}

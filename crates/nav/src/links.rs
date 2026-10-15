//! links: traversal links over walkable edges for the animation traversal moves.
//! Owner: nav crate, consumer arena locomotion. Units: meters in, integer
//! millimeters inside, ticks at 60 Hz out. Determinism: probes freeze to
//! integers once, the chooser lanes match the animation chooser exactly, link
//! ties break by id.
//! Errors: typed [`LinkError`], refusals never guess.

use std::fmt;

use engine_core::Fnv64;

use crate::tiles::{BreakSet, TileId};

/// Millimeters per meter inside the authority path.
pub const MM_PER_M: f32 = 1000.0;
/// Warp acceptance: the warped root lands within 1 cm of the edge.
pub const WARP_TOL_MM: f32 = 10.0;
/// Widest clearable gap in millimeters.
pub const MAX_GAP_MM: i32 = 2500;
/// Rises below this step through with no link.
pub const STEP_MM: i32 = 200;
/// Tallest top cleared as a hurdle in millimeters.
pub const HURDLE_MAX_MM: i32 = 750;
/// Tallest top cleared as a vault in millimeters.
pub const VAULT_MAX_MM: i32 = 1250;
/// Tallest top cleared as a mantle in millimeters.
pub const MANTLE_MAX_MM: i32 = 1700;
/// Tallest top cleared as a climb in millimeters (needs a hold).
pub const CLIMB_MAX_MM: i32 = 2600;
/// Widest block hurdled in millimeters; wider low blocks mantle.
pub const HURDLE_DEPTH_MM: i32 = 800;
/// Widest block vaulted in millimeters; wider chest blocks mantle.
pub const VAULT_DEPTH_MM: i32 = 1200;
/// Slow approaches over hurdle-high rails mantle instead of hurdling.
pub const HURDLE_MIN_SPEED_MM_S: i32 = 1000;
/// Drops start past this depth below the feet in millimeters.
pub const DROP_EDGE_MM: i32 = -300;
/// Deeper drops than this refuse in millimeters.
pub const DROP_MAX_MM: i32 = -2500;
/// Longest warpable root path; bounds per-plan allocation.
pub const MAX_PATH: usize = 512;
/// Largest link registry; bounds memory up front.
pub const MAX_LINKS: usize = 4096;
/// Probe height lane range in meters.
pub const PROBE_HEIGHT_RANGE_M: f32 = 5.0;
/// Probe depth/gap lane range in meters.
pub const PROBE_SPAN_RANGE_M: f32 = 5.0;
/// Fastest accepted approach speed in meters per second.
pub const PROBE_SPEED_RANGE_M_S: f32 = 12.0;
/// Edge anchor range in meters per lane.
pub const EDGE_RANGE_M: f32 = 1000.0;

/// Stable link id; ties break by this, never by thread or hash order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkId(pub u32);

/// Traversal move a link carries; matches the animation chooser one to one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkKind {
    /// Low rail taken at speed.
    Hurdle,
    /// Waist-high block vaulted across.
    Vault,
    /// Chest-high ledge mantled onto.
    Mantle,
    /// High wall climbed with a hold.
    Climb,
    /// Ledge dropped from.
    Drop,
}

/// Chooser outcome including flat ground, which needs no link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraverseAction {
    /// Flat or step ground: walk, no link.
    None,
    /// Low rail taken at speed.
    Hurdle,
    /// Waist-high block vaulted across.
    Vault,
    /// Chest-high ledge mantled onto.
    Mantle,
    /// High wall climbed with a hold.
    Climb,
    /// Ledge dropped from.
    Drop,
}

/// Obstacle probe: top above the feet, thickness, gap, speed, hold flag.
/// Units: meters and meters per second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Probe {
    /// Obstacle top in meters above the feet (drops go negative).
    pub height_m: f32,
    /// Block thickness in meters along travel.
    pub depth_m: f32,
    /// Horizontal gap in meters to the far edge.
    pub gap_m: f32,
    /// Approach speed in meters per second.
    pub speed_m_s: f32,
    /// True when a climb hold is available.
    pub has_hold: bool,
}

/// Probe frozen to integer millimeters; the only form compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeFix {
    /// Obstacle top in millimeters above the feet.
    pub height_mm: i32,
    /// Block thickness in millimeters.
    pub depth_mm: i32,
    /// Gap in millimeters.
    pub gap_mm: i32,
    /// Approach speed in millimeters per second.
    pub speed_mm_s: i32,
    /// True when a climb hold is available.
    pub has_hold: bool,
}

/// Authored traversal edge marker baked by the cooker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraversalLink {
    /// Stable id for deterministic ties.
    pub id: LinkId,
    /// Move this edge carries.
    pub kind: LinkKind,
    /// Edge anchor in meters `[x, z]`.
    pub edge_m: [f32; 2],
    /// Nav tile this edge sits on; a break set over this tile rebakes
    /// exactly this link, so every edge names its rebake record.
    pub tile: TileId,
}

/// Committed plan: the move, its tick budget, the edge and the probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkPlan {
    /// Chosen move.
    pub action: TraverseAction,
    /// Tick budget at 60 Hz.
    pub duration_ticks: u32,
    /// Edge anchor in millimeters `[x, z]`.
    pub edge_mm: [i32; 2],
    /// Link that carries the move.
    pub link: LinkId,
    /// Fixed probe this plan was chosen from.
    pub probe: ProbeFix,
}

/// Typed link failures; bad input refuses, never guesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// A probe, edge or path lane is out of bounds; the lane names it.
    BadProbe(&'static str),
    /// A climb-high top with no hold; refused, never faked.
    NoHold,
    /// The gap exceeds what any move clears.
    GapTooWide {
        /// Probe gap in millimeters.
        gap_mm: i32,
        /// Widest clearable gap in millimeters.
        max_mm: i32,
    },
    /// The probe needs no link (flat ground) or no link matches.
    NoLink,
    /// Too many links registered.
    TooManyLinks {
        /// Link count offered.
        got: usize,
        /// Registry bound refused past.
        max: usize,
    },
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadProbe(lane) => write!(f, "link: bad probe ({lane})"),
            Self::NoHold => write!(f, "link: climb needs a hold"),
            Self::GapTooWide { gap_mm, max_mm } => {
                write!(f, "link: gap {gap_mm} mm past {max_mm} mm")
            }
            Self::NoLink => write!(f, "link: no traversal link"),
            Self::TooManyLinks { got, max } => write!(f, "link: {got} links over bound {max}"),
        }
    }
}

impl std::error::Error for LinkError {}

/// Fixed ticks per move at 60 Hz; matches the animation duration table.
pub fn duration_ticks(action: TraverseAction) -> u32 {
    match action {
        TraverseAction::None => 0,
        TraverseAction::Hurdle => 20,
        TraverseAction::Vault => 30,
        TraverseAction::Mantle => 40,
        TraverseAction::Climb => 60,
        TraverseAction::Drop => 25,
    }
}

/// Move a link kind carries as the chooser action.
pub fn action_of(kind: LinkKind) -> TraverseAction {
    match kind {
        LinkKind::Hurdle => TraverseAction::Hurdle,
        LinkKind::Vault => TraverseAction::Vault,
        LinkKind::Mantle => TraverseAction::Mantle,
        LinkKind::Climb => TraverseAction::Climb,
        LinkKind::Drop => TraverseAction::Drop,
    }
}

/// Validate a probe and freeze it to integer millimeters.
pub fn fix_probe(probe: &Probe) -> Result<ProbeFix, LinkError> {
    if !probe.height_m.is_finite() {
        return Err(LinkError::BadProbe("height"));
    }
    if !probe.depth_m.is_finite() {
        return Err(LinkError::BadProbe("depth"));
    }
    if !probe.gap_m.is_finite() {
        return Err(LinkError::BadProbe("gap"));
    }
    if !probe.speed_m_s.is_finite() {
        return Err(LinkError::BadProbe("speed"));
    }
    if probe.height_m < -PROBE_HEIGHT_RANGE_M || probe.height_m > PROBE_HEIGHT_RANGE_M {
        return Err(LinkError::BadProbe("height"));
    }
    if !(0.0..=PROBE_SPAN_RANGE_M).contains(&probe.depth_m) {
        return Err(LinkError::BadProbe("depth"));
    }
    if !(0.0..=PROBE_SPAN_RANGE_M).contains(&probe.gap_m) {
        return Err(LinkError::BadProbe("gap"));
    }
    if !(0.0..=PROBE_SPEED_RANGE_M_S).contains(&probe.speed_m_s) {
        return Err(LinkError::BadProbe("speed"));
    }
    Ok(ProbeFix {
        height_mm: to_mm(probe.height_m),
        depth_mm: to_mm(probe.depth_m),
        gap_mm: to_mm(probe.gap_m),
        speed_mm_s: to_mm(probe.speed_m_s),
        has_hold: probe.has_hold,
    })
}

/// Choose the move for a fixed probe; lanes match the animation chooser exactly.
fn choose_fix(fixed: &ProbeFix) -> Result<TraverseAction, LinkError> {
    if fixed.gap_mm > MAX_GAP_MM {
        return Err(LinkError::GapTooWide {
            gap_mm: fixed.gap_mm,
            max_mm: MAX_GAP_MM,
        });
    }
    let h = fixed.height_mm;
    if h <= DROP_EDGE_MM {
        if h < DROP_MAX_MM {
            return Err(LinkError::BadProbe("drop"));
        }
        return Ok(TraverseAction::Drop);
    }
    if h < STEP_MM {
        return Ok(TraverseAction::None);
    }
    if h <= HURDLE_MAX_MM {
        if fixed.depth_mm <= HURDLE_DEPTH_MM && fixed.speed_mm_s >= HURDLE_MIN_SPEED_MM_S {
            return Ok(TraverseAction::Hurdle);
        }
        return Ok(TraverseAction::Mantle);
    }
    if h <= VAULT_MAX_MM {
        if fixed.depth_mm <= VAULT_DEPTH_MM {
            return Ok(TraverseAction::Vault);
        }
        return Ok(TraverseAction::Mantle);
    }
    if h <= MANTLE_MAX_MM {
        return Ok(TraverseAction::Mantle);
    }
    if h <= CLIMB_MAX_MM {
        if fixed.has_hold {
            return Ok(TraverseAction::Climb);
        }
        return Err(LinkError::NoHold);
    }
    Err(LinkError::BadProbe("height"))
}

/// Choose the move for an obstacle probe; matches the animation chooser fixtures.
pub fn choose(probe: &Probe) -> Result<TraverseAction, LinkError> {
    choose_fix(&fix_probe(probe)?)
}

/// Pick the link a probe traverses: classify, then lowest matching id.
///
/// Flat ground (`None`) returns [`LinkError::NoLink`] because walking
/// needs no edge; a classified move with no registered link also
/// returns `NoLink` instead of guessing a neighbor kind.
pub fn select_link(probe: &Probe, links: &[TraversalLink]) -> Result<TraversalLink, LinkError> {
    if links.len() > MAX_LINKS {
        return Err(LinkError::TooManyLinks {
            got: links.len(),
            max: MAX_LINKS,
        });
    }
    let fixed = fix_probe(probe)?;
    let action = choose_fix(&fixed)?;
    if action == TraverseAction::None {
        return Err(LinkError::NoLink);
    }
    let mut best: Option<TraversalLink> = None;
    for link in links {
        check_link(link)?;
        if action_of(link.kind) != action {
            continue;
        }
        if best.map(|b| link.id < b.id).unwrap_or(true) {
            best = Some(*link);
        }
    }
    best.ok_or(LinkError::NoLink)
}

/// Commit a plan for a probe over its selected link.
pub fn plan(probe: &Probe, link: &TraversalLink) -> Result<LinkPlan, LinkError> {
    let fixed = fix_probe(probe)?;
    let action = choose_fix(&fixed)?;
    if action == TraverseAction::None || action_of(link.kind) != action {
        return Err(LinkError::NoLink);
    }
    Ok(LinkPlan {
        duration_ticks: duration_ticks(action),
        action,
        edge_mm: fix_edge(link.edge_m)?,
        link: link.id,
        probe: fixed,
    })
}

/// Classify plus commit in one call over a link registry.
pub fn traverse(probe: &Probe, links: &[TraversalLink]) -> Result<LinkPlan, LinkError> {
    let link = select_link(probe, links)?;
    plan(probe, &link)
}

/// Names the links a dirty tile set must rebake: every link sitting on a
/// dirty tile, ascending by id, nothing else.
///
/// Cook-time use only (the cooker hands the break-set tiles plus the link
/// registry and recooks exactly the links named back); it allocates one id
/// per dirty link, so it never runs on the tick. Pure: the same registry
/// plus the same tiles name the same ids in the same order, twice alike.
/// Errors: [`LinkError::TooManyLinks`] past [`MAX_LINKS`];
/// [`LinkError::BadProbe`] names a bad edge lane, the same lane
/// [`select_link`] would refuse.
pub fn rebake_links(links: &[TraversalLink], dirty: &[TileId]) -> Result<Vec<LinkId>, LinkError> {
    if links.len() > MAX_LINKS {
        return Err(LinkError::TooManyLinks {
            got: links.len(),
            max: MAX_LINKS,
        });
    }
    for link in links {
        check_link(link)?;
    }
    let mut out = Vec::new();
    for link in links {
        if dirty.contains(&link.tile) {
            out.push(link.id);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Cooker entry over a break set: the district cooker hands the break
/// batch it already built for tiles and recooks exactly the links named
/// back, ascending by id.
///
/// Same ids in the same order with the same refusals as
/// [`rebake_links`], still cook-time only and never on the tick. Pure:
/// the same registry plus the same set names the same ids twice alike.
pub fn rebake_for_set(links: &[TraversalLink], set: &BreakSet) -> Result<Vec<LinkId>, LinkError> {
    rebake_links(links, &set.tiles)
}

/// FNV-1a64 over the plan lanes; identical inputs hash identically.
pub fn plan_hash(plan: &LinkPlan) -> u64 {
    let mut hash = Fnv64::new();
    let disc: u8 = match plan.action {
        TraverseAction::None => 0,
        TraverseAction::Hurdle => 1,
        TraverseAction::Vault => 2,
        TraverseAction::Mantle => 3,
        TraverseAction::Climb => 4,
        TraverseAction::Drop => 5,
    };
    hash.write_u8(disc);
    hash.write_u32(plan.duration_ticks);
    hash.write_u32(plan.link.0);
    for lane in plan.edge_mm {
        hash.write_i32(lane);
    }
    for lane in [
        plan.probe.height_mm,
        plan.probe.depth_mm,
        plan.probe.gap_mm,
        plan.probe.speed_mm_s,
    ] {
        hash.write_i32(lane);
    }
    hash.write_bool(plan.probe.has_hold);
    hash.finish()
}

/// Warp a raw planar root path onto the plan edge by exact translation,
/// preserving shape so the warped end lands on the edge.
pub fn apply_warp(plan: &LinkPlan, raw: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, LinkError> {
    if raw.is_empty() {
        return Err(LinkError::BadProbe("path empty"));
    }
    if raw.len() > MAX_PATH {
        return Err(LinkError::BadProbe("path too long"));
    }
    if raw.iter().flatten().any(|v| !v.is_finite()) {
        return Err(LinkError::BadProbe("path"));
    }
    let mut path_mm = Vec::with_capacity(raw.len());
    for point in raw {
        path_mm.push([to_mm(point[0]), to_mm(point[1])]);
    }
    let end = match path_mm.last() {
        Some(end) => *end,
        None => return Err(LinkError::BadProbe("path empty")),
    };
    let offset = [
        plan.edge_mm[0].saturating_sub(end[0]),
        plan.edge_mm[1].saturating_sub(end[1]),
    ];
    Ok(path_mm
        .iter()
        .map(|point| {
            [
                point[0].saturating_add(offset[0]) as f32 / MM_PER_M,
                point[1].saturating_add(offset[1]) as f32 / MM_PER_M,
            ]
        })
        .collect())
}

/// Residual miss in millimeters between warped end and edge (gate: 1 cm).
pub fn warp_error_mm(warped_end_m: [f32; 2], edge_m: [f32; 2]) -> f32 {
    let dx = (warped_end_m[0] - edge_m[0]) * MM_PER_M;
    let dy = (warped_end_m[1] - edge_m[1]) * MM_PER_M;
    (dx * dx + dy * dy).sqrt()
}

/// Round meters to integer millimeters; callers validate first.
fn to_mm(value: f32) -> i32 {
    (value * MM_PER_M).round() as i32
}

/// Validate an authored edge anchor in meters.
fn fix_edge(edge_m: [f32; 2]) -> Result<[i32; 2], LinkError> {
    if edge_m.iter().any(|v| !v.is_finite()) {
        return Err(LinkError::BadProbe("edge"));
    }
    if edge_m
        .iter()
        .any(|v| *v < -EDGE_RANGE_M || *v > EDGE_RANGE_M)
    {
        return Err(LinkError::BadProbe("edge"));
    }
    Ok([to_mm(edge_m[0]), to_mm(edge_m[1])])
}

/// Validate one authored link.
fn check_link(link: &TraversalLink) -> Result<(), LinkError> {
    fix_edge(link.edge_m)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(height: f32, depth: f32, gap: f32, speed: f32, hold: bool) -> Probe {
        Probe {
            height_m: height,
            depth_m: depth,
            gap_m: gap,
            speed_m_s: speed,
            has_hold: hold,
        }
    }

    fn link(id: u32, kind: LinkKind) -> TraversalLink {
        TraversalLink {
            id: LinkId(id),
            kind,
            edge_m: [4.0, 1.0],
            tile: TileId(0),
        }
    }

    fn registry() -> Vec<TraversalLink> {
        vec![
            link(7, LinkKind::Hurdle),
            link(3, LinkKind::Vault),
            link(5, LinkKind::Mantle),
            link(9, LinkKind::Climb),
            link(11, LinkKind::Drop),
        ]
    }

    #[test]
    fn fixtures_pick_tv1_expected_actions() {
        assert_eq!(
            choose(&probe(0.0, 0.0, 0.0, 1.5, false)),
            Ok(TraverseAction::None)
        );
        assert_eq!(
            choose(&probe(0.5, 0.3, 0.4, 3.0, false)),
            Ok(TraverseAction::Hurdle)
        );
        assert_eq!(
            choose(&probe(1.0, 0.6, 0.5, 2.0, false)),
            Ok(TraverseAction::Vault)
        );
        assert_eq!(
            choose(&probe(1.5, 2.0, 0.2, 1.0, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(2.0, 0.4, 0.3, 1.0, true)),
            Ok(TraverseAction::Climb)
        );
        assert_eq!(
            choose(&probe(-1.0, 0.5, 0.4, 1.5, false)),
            Ok(TraverseAction::Drop)
        );
    }

    #[test]
    fn boundary_heights_pick_the_lower_move() {
        assert_eq!(
            choose(&probe(0.75, 0.3, 0.2, 3.0, false)),
            Ok(TraverseAction::Hurdle)
        );
        assert_eq!(
            choose(&probe(1.25, 0.6, 0.2, 2.0, false)),
            Ok(TraverseAction::Vault)
        );
        assert_eq!(
            choose(&probe(1.7, 0.6, 0.2, 2.0, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(2.6, 0.4, 0.2, 1.0, true)),
            Ok(TraverseAction::Climb)
        );
        assert_eq!(
            choose(&probe(-0.3, 0.4, 0.2, 1.0, false)),
            Ok(TraverseAction::Drop)
        );
    }

    #[test]
    fn slow_or_wide_low_blocks_become_mantles() {
        assert_eq!(
            choose(&probe(0.5, 0.3, 0.2, 0.5, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(0.5, 2.0, 0.2, 3.0, false)),
            Ok(TraverseAction::Mantle)
        );
        assert_eq!(
            choose(&probe(1.0, 2.5, 0.2, 2.0, false)),
            Ok(TraverseAction::Mantle)
        );
    }

    #[test]
    fn gap_and_hold_refusals_match_tv1() {
        assert_eq!(
            choose(&probe(1.0, 0.6, 2.6, 2.0, false)),
            Err(LinkError::GapTooWide {
                gap_mm: 2600,
                max_mm: MAX_GAP_MM
            })
        );
        assert!(choose(&probe(1.0, 0.6, 2.5, 2.0, false)).is_ok());
        assert_eq!(
            choose(&probe(2.0, 0.4, 0.3, 1.0, false)),
            Err(LinkError::NoHold)
        );
    }

    #[test]
    fn select_breaks_ties_by_lowest_id() {
        let links = vec![
            link(9, LinkKind::Vault),
            link(2, LinkKind::Vault),
            link(5, LinkKind::Vault),
        ];
        let picked = select_link(&probe(1.0, 0.6, 0.5, 2.0, false), &links).expect("vault link");
        assert_eq!(picked.id, LinkId(2));
        // Shuffled registry order still picks the lowest id.
        let mut shuffled = links.clone();
        shuffled.reverse();
        let again = select_link(&probe(1.0, 0.6, 0.5, 2.0, false), &shuffled).expect("vault link");
        assert_eq!(again, picked);
    }

    #[test]
    fn flat_ground_needs_no_link() {
        assert_eq!(
            select_link(&probe(0.0, 0.0, 0.0, 1.5, false), &registry()),
            Err(LinkError::NoLink)
        );
        assert_eq!(
            traverse(&probe(0.0, 0.0, 0.0, 1.5, false), &registry()),
            Err(LinkError::NoLink)
        );
        // Classified vault with only hurdle links registered guesses nothing.
        assert_eq!(
            select_link(
                &probe(1.0, 0.6, 0.5, 2.0, false),
                &[link(1, LinkKind::Hurdle)]
            ),
            Err(LinkError::NoLink)
        );
    }

    #[test]
    fn traverse_commits_tv1_durations() {
        let cases = [
            (probe(0.5, 0.3, 0.4, 3.0, false), TraverseAction::Hurdle, 20),
            (probe(1.0, 0.6, 0.5, 2.0, false), TraverseAction::Vault, 30),
            (probe(1.5, 2.0, 0.2, 1.0, false), TraverseAction::Mantle, 40),
            (probe(2.0, 0.4, 0.3, 1.0, true), TraverseAction::Climb, 60),
            (probe(-1.0, 0.5, 0.4, 1.5, false), TraverseAction::Drop, 25),
        ];
        for (probe, action, ticks) in cases {
            let committed = traverse(&probe, &registry()).expect("fixture traverses");
            assert_eq!(committed.action, action);
            assert_eq!(committed.duration_ticks, ticks);
            assert_eq!(committed.duration_ticks, duration_ticks(action));
        }
    }

    #[test]
    fn warp_lands_on_edge_within_1cm() {
        let cases = [
            probe(0.5, 0.3, 0.4, 3.0, false),
            probe(1.0, 0.6, 0.5, 2.0, false),
            probe(1.5, 2.0, 0.2, 1.0, false),
            probe(2.0, 0.4, 0.3, 1.0, true),
            probe(-1.0, 0.5, 0.4, 1.5, false),
        ];
        for probe in cases {
            let committed = traverse(&probe, &registry()).expect("fixture traverses");
            let raw = [[0.0, 0.0], [1.0, 0.25], [2.0, 0.5]];
            let warped = apply_warp(&committed, &raw).expect("fixture warps");
            let end = *warped.last().expect("warped nonempty");
            let err = warp_error_mm(end, [4.0, 1.0]);
            assert!(err <= WARP_TOL_MM, "miss {err} mm past 1 cm");
            assert_eq!(err, 0.0, "integer warp lands exactly");
        }
    }

    #[test]
    fn plan_hash_is_fnv1a_over_plan_lanes() {
        let plan = LinkPlan {
            action: TraverseAction::Vault,
            duration_ticks: 30,
            edge_mm: [4000, -1000],
            link: LinkId(7),
            probe: ProbeFix {
                height_mm: 1000,
                depth_mm: 600,
                gap_mm: -500,
                speed_mm_s: 2000,
                has_hold: true,
            },
        };
        // Oracle: FNV-1a 64 over action byte, little-endian lanes and the
        // hold byte, computed outside this crate.
        assert_eq!(plan_hash(&plan), 0xb5df_c543_e4eb_8e88);
    }

    #[test]
    fn traverse_replays_exactly() {
        let fixture = probe(1.0, 0.6, 0.5, 2.0, false);
        let first = traverse(&fixture, &registry()).expect("traverses");
        let second = traverse(&fixture, &registry()).expect("traverses");
        assert_eq!(first, second);
        assert_eq!(plan_hash(&first), plan_hash(&second));
        let raw = [[0.0, 0.0], [1.0, 0.25], [2.0, 0.5]];
        assert_eq!(
            apply_warp(&first, &raw).expect("warps"),
            apply_warp(&second, &raw).expect("warps")
        );
    }

    #[test]
    fn bad_probes_and_paths_name_their_lane() {
        assert_eq!(
            choose(&probe(f32::NAN, 0.5, 0.2, 1.0, false)),
            Err(LinkError::BadProbe("height"))
        );
        assert_eq!(
            choose(&probe(1.0, -0.5, 0.2, 1.0, false)),
            Err(LinkError::BadProbe("depth"))
        );
        assert_eq!(
            choose(&probe(3.5, 0.5, 0.2, 1.0, true)),
            Err(LinkError::BadProbe("height"))
        );
        assert_eq!(
            choose(&probe(-3.0, 0.5, 0.2, 1.0, false)),
            Err(LinkError::BadProbe("drop"))
        );
        let committed =
            traverse(&probe(1.0, 0.6, 0.5, 2.0, false), &registry()).expect("traverses");
        assert_eq!(
            apply_warp(&committed, &[]),
            Err(LinkError::BadProbe("path empty"))
        );
        assert_eq!(
            apply_warp(&committed, &[[f32::NAN, 0.0]]),
            Err(LinkError::BadProbe("path"))
        );
        let long = vec![[0.0, 0.0]; MAX_PATH + 1];
        assert_eq!(
            apply_warp(&committed, &long),
            Err(LinkError::BadProbe("path too long"))
        );
        let bad_edge = TraversalLink {
            id: LinkId(1),
            kind: LinkKind::Vault,
            edge_m: [f32::INFINITY, 0.0],
            tile: TileId(0),
        };
        assert_eq!(
            plan(&probe(1.0, 0.6, 0.5, 2.0, false), &bad_edge),
            Err(LinkError::BadProbe("edge"))
        );
        // Wrong-kind link never commits: a vault probe over a hurdle edge.
        assert_eq!(
            plan(
                &probe(1.0, 0.6, 0.5, 2.0, false),
                &link(1, LinkKind::Hurdle)
            ),
            Err(LinkError::NoLink)
        );
    }
}

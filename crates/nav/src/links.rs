//! links: traversal link registry over walkable edges plus rebake for break sets.
//! Owner: nav crate; live consumer: destruction break-set recooks. The
//! chooser plus warp authority lives in anim traversal alone (no mirror
//! here). Units: meters in, integer
//! millimeters inside, ticks at 60 Hz out. Determinism: link
//! ties break by id.
//! Errors: typed [`LinkError`], refusals never guess.

use std::fmt;

use crate::tiles::{BreakSet, TileId};

/// Millimeters per meter inside the authority path.
pub const MM_PER_M: f32 = 1000.0;
/// Largest link registry; bounds memory up front.
pub const MAX_LINKS: usize = 4096;
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

/// Names the links a dirty tile set must rebake: every link sitting on a
/// dirty tile, ascending by id, nothing else.
///
/// `dirty` must arrive in ascending id order ([`BreakSet`] tiles already
/// are: sorted and deduped on build), so membership is a binary search,
/// not a scan, per link.
///
/// Cook-time use only (the cooker hands the break-set tiles plus the link
/// registry and recooks exactly the links named back); it allocates one id
/// per dirty link, so it never runs on the tick. Pure: the same registry
/// plus the same tiles name the same ids in the same order, twice alike.
/// Errors: [`LinkError::TooManyLinks`] past [`MAX_LINKS`];
/// [`LinkError::BadProbe`] names a bad edge lane, the same lane
/// [`check_link`] would refuse.
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
        if dirty.binary_search(&link.tile).is_ok() {
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

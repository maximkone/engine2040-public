//! Asset browser: bundle search plus drag-to-place with preview.
//!
//! Owner: the editor session; reads bundle listings, never owns them.
//! Units: bundle entries, search hits, meters in, integer centimeters
//! inside the solver. Budget: full-bundle search <= 50 ms, panel logic
//! is UI-thread only and linear in entries. Determinism: same bundle
//! plus same query gives the same hits; same intent over the same
//! actors in any order gives the same placement or conflict.
//! Errors: [`BrowserError`]; refusals stage nothing and mutate nothing.
//! TEMPORARY std-only mirrors (no dependency edge yet; the orchestrator
//! wires the real edges and deletes these): `AssetKind` mirrors the assets
//! `bundle::EntryKind` discriminants 1-5, `Relation`/`Wall`/`Target`/
//! `Want` mirror the spatial `place` vocabulary, `DropPreview` follows the
//! endpoint `PreviewReceipt` discipline (dry-run effects plus resulting
//! revision, conflicts write nothing).

use std::fmt;

use crate::commands::bound_to;

/// Longest asset name in chars.
/// TEMPORARY mirror of the assets `bundle::MAX_NAME_CHARS`; same budget, no edge.
pub const MAX_NAME_CHARS: usize = 128;
/// Longest tag in chars.
pub const MAX_TAG_CHARS: usize = 64;
/// Most tags kept on one entry.
pub const MAX_TAGS_PER_ENTRY: usize = 8;
/// Most entries accepted in one bundle view.
/// TEMPORARY mirror of the assets `bundle::MAX_ENTRIES`; same budget, no edge.
pub const MAX_ENTRIES: usize = 4096;
/// Longest search text or tag filter in chars.
pub const MAX_QUERY_CHARS: usize = 128;
/// Most hits returned by one search; the full bundle fits exactly.
pub const MAX_RESULTS: usize = 4096;
/// Most relation wants accepted on one drop.
pub const MAX_WANTS_PER_DROP: usize = 8;
/// Most scene actors accepted around one drop.
pub const MAX_ACTORS_PER_SCENE: usize = 1024;
/// Bound for echoed text carried in errors.
/// TEMPORARY mirror of the assets `bundle::MAX_DETAIL_CHARS`; same budget, no edge.
pub const MAX_DETAIL_CHARS: usize = 128;
/// Solver grid pitch in centimeters.
/// TEMPORARY mirror of spatial `place::GRID_CM`; same step, no edge.
pub const GRID_CM: i32 = 10;
/// Yaw candidates per position (`k * PI/4`).
/// TEMPORARY mirror of spatial `place::YAW_STEPS`; same count, no edge.
pub const YAW_STEPS: i32 = 8;
/// Side-by-side edge-gap limit in cm.
/// TEMPORARY mirror of spatial `relation::NEXT_TO_CM`; same limit, no edge.
pub const NEXT_TO_CM: i32 = 100;
/// Resting-contact tolerance in cm.
/// TEMPORARY mirror of spatial `relation::SUPPORT_GAP_CM`; same gap, no edge.
pub const SUPPORT_GAP_CM: i32 = 5;
/// Forward half-plane range in cm.
/// TEMPORARY mirror of spatial `relation::FACING_RANGE_CM`; same range, no edge.
pub const FACING_RANGE_CM: i32 = 2000;
/// Default wall edge-gap range `(min, max)` in cm for `NextTo` + wall.
/// TEMPORARY mirror of spatial `DEFAULT_WALL_GAP_CM`; same range, no edge.
pub const DEFAULT_WALL_GAP_CM: (i32, i32) = (0, 50);
/// Default point distance range `(min, max)` in cm for `NextTo` + point.
/// TEMPORARY mirror of spatial `DEFAULT_POINT_DIST_CM`; same range, no edge.
pub const DEFAULT_POINT_DIST_CM: (i32, i32) = (0, 50);
/// Largest point coordinate accepted in cm (+-1 km).
pub const MAX_POINT_CM: i32 = 100_000;

/// Payload kind of one bundle entry; one byte on the bundle wire.
/// TEMPORARY mirror of the assets `bundle::EntryKind`: same variants plus
/// discriminants so the consumer maps 1:1 at the IO boundary. Do not
/// renumber; replace with the documented `bundle` edge when registered.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[repr(u8)]
pub enum AssetKind {
    /// Cooked mesh bytes.
    Mesh = 1,
    /// Cooked material bytes.
    Material = 2,
    /// Cooked texture bytes with mips.
    Texture = 3,
    /// Skin bytes.
    Skin = 4,
    /// Imported clip bytes.
    Clip = 5,
}

impl AssetKind {
    /// Every kind, in wire order.
    pub const fn all() -> [Self; 5] {
        [
            Self::Mesh,
            Self::Material,
            Self::Texture,
            Self::Skin,
            Self::Clip,
        ]
    }

    /// Decode a wire byte; `None` means the bundle row is corrupt.
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(Self::Mesh),
            2 => Some(Self::Material),
            3 => Some(Self::Texture),
            4 => Some(Self::Skin),
            5 => Some(Self::Clip),
            _ => None,
        }
    }

    /// Lowercase kind name for rows and diagnostics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Material => "material",
            Self::Texture => "texture",
            Self::Skin => "skin",
            Self::Clip => "clip",
        }
    }
}

impl fmt::Display for AssetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One directed relation. TEMPORARY mirror of the spatial relation vocabulary
/// the place solver evaluates; same names, same order, no edge yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Relation {
    Supports,
    OnTopOf,
    Inside,
    NextTo,
    Facing,
}

impl Relation {
    /// Every relation, in solver order.
    pub const fn all() -> [Self; 5] {
        [
            Self::Supports,
            Self::OnTopOf,
            Self::Inside,
            Self::NextTo,
            Self::Facing,
        ]
    }

    /// Short relation name quoted in refusals.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::OnTopOf => "on_top_of",
            Self::Inside => "inside",
            Self::NextTo => "next_to",
            Self::Facing => "facing",
        }
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Room wall. TEMPORARY mirror of the spatial wall convention: North is
/// the min-z bound, South max-z, West min-x, East max-x.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Wall {
    North,
    South,
    East,
    West,
}

impl Wall {
    /// Every wall, in solver order.
    pub const fn all() -> [Self; 4] {
        [Self::North, Self::South, Self::East, Self::West]
    }

    /// Short wall name quoted in refusals.
    pub const fn name(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::South => "south",
            Self::East => "east",
            Self::West => "west",
        }
    }
}

impl fmt::Display for Wall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a want is measured against: a room wall, an actor id, or an
/// explicit XZ point in integer centimeters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Wall(Wall),
    Object(u32),
    Point { x_cm: i32, z_cm: i32 },
}

impl Target {
    /// One-line source text for conflict sets, in intent order.
    pub fn describe(self) -> String {
        match self {
            Self::Wall(w) => format!("wall:{}", w.name()),
            Self::Object(id) => format!("object:{id}"),
            Self::Point { x_cm, z_cm } => format!("point:{x_cm},{z_cm}"),
        }
    }
}

/// One relation constraint. `distance_cm` is an inclusive `(min, max)`
/// in cm for `NextTo` and `Facing`; `None` selects the per-target
/// default. Distance is ignored for `Inside`, `OnTopOf`, `Supports`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub relation: Relation,
    pub target: Target,
    pub distance_cm: Option<(i32, i32)>,
}

impl Want {
    /// One-line source text for conflict sets, in intent order.
    pub fn describe(self) -> String {
        let range = match self.distance_cm {
            Some((lo, hi)) => format!("{lo}..={hi}"),
            None => "default".to_string(),
        };
        format!(
            "{} {} dist {range}",
            self.relation.name(),
            self.target.describe()
        )
    }
}

/// Which actor a drop moves: an id, or a name matching exactly one actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectRef {
    Id(u32),
    Name(String),
}

impl ObjectRef {
    /// Label quoted in refusals (bounded, never raw unbounded input).
    pub fn label(&self) -> String {
        match self {
            Self::Id(id) => format!("id {id}"),
            Self::Name(name) => format!("name '{}'", bound_detail(name)),
        }
    }
}

/// One placement intent: move `object` so every want holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub object: ObjectRef,
    pub wants: Vec<Want>,
}

/// One scene actor snapshot. Plain data subset of the spatial actor:
/// id, grammar-clean name, meters, extents, yaw radians.
#[derive(Clone, Debug, PartialEq)]
pub struct BrowserActor {
    pub id: u32,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
    pub yaw: f32,
}

impl BrowserActor {
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    /// Build one actor; [`validate_actor`] gates solver entry.
    pub fn new(
        id: u32,
        name: &str,
        x: f32,
        y: f32,
        z: f32,
        sx: f32,
        sy: f32,
        sz: f32,
        yaw: f32,
    ) -> Self {
        Self {
            id,
            name: name.to_string(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            yaw,
        }
    }

    /// Grounded rest height for these extents in meters.
    pub fn rest_y(self) -> f32 {
        self.sy * 0.5
    }
}

/// One bundle row: an asset kind plus its searchable name and tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleEntry {
    pub kind: AssetKind,
    pub name: String,
    pub tags: Vec<String>,
}

impl BundleEntry {
    /// Build one row without validation; [`validate_entry`] gates entry.
    pub fn new(kind: AssetKind, name: &str, tags: &[&str]) -> Self {
        Self {
            kind,
            name: name.to_string(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
        }
    }
}

/// Plain-data bundle sketch: the searchable listing the panel reads.
/// Real bundle bytes bind in the consumer; this view owns names, kinds
/// and tags only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BundleView {
    entries: Vec<BundleEntry>,
}

impl BundleView {
    /// Empty view.
    pub fn new() -> Self {
        Self::default()
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry is listed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in listing order.
    pub fn entries(&self) -> &[BundleEntry] {
        &self.entries
    }

    /// Push one validated row; duplicates and over-capacity refuse.
    pub fn push(&mut self, entry: BundleEntry) -> Result<(), BrowserError> {
        validate_entry(&entry)?;
        if self.entries.len() >= MAX_ENTRIES {
            return Err(BrowserError::TooManyEntries {
                count: self.entries.len() + 1,
                max: MAX_ENTRIES,
            });
        }
        if self.entries.iter().any(|known| known.name == entry.name) {
            return Err(BrowserError::DuplicateName { name: entry.name });
        }
        self.entries.push(entry);
        Ok(())
    }

    /// True when an asset name is listed.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|entry| entry.name == name)
    }

    /// One listed row by asset name.
    pub fn find(&self, name: &str) -> Option<&BundleEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }
}

/// One search query: substring over names plus optional kind and tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    /// Substring over the asset name; empty matches every name.
    pub text: String,
    /// Kind filter; `None` accepts every kind.
    pub kind: Option<AssetKind>,
    /// Tag filter; `None` accepts every tag set.
    pub tag: Option<String>,
}

impl SearchQuery {
    /// Build one query without validation; [`validate_query`] gates search.
    pub fn new(text: &str, kind: Option<AssetKind>, tag: Option<&str>) -> Self {
        Self {
            text: text.to_string(),
            kind,
            tag: tag.map(str::to_string),
        }
    }
}

/// One search hit: the kind plus the asset name, in result order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub kind: AssetKind,
    pub name: String,
}

impl SearchHit {
    /// Row text for the panel: `kind name`.
    pub fn row(&self) -> String {
        format!("{} {}", self.kind.label(), self.name)
    }
}

/// Dry-run outcome for a drop: one line per op plus the revision an
/// apply would publish. TEMPORARY mirror of the endpoint `PreviewReceipt`
/// discipline (effects plus resulting revision, conflicts write
/// nothing); the orchestrator binds the real receipt on wiring.
#[derive(Clone, Debug, PartialEq)]
pub struct DropPreview {
    /// Asset the drop places.
    pub asset: String,
    /// Scene actor the drop moves.
    pub id: u32,
    /// Solved center in meters plus yaw radians.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    /// One line per op, in batch order.
    pub effects: Vec<String>,
    /// `base_revision + 1`; the revision an apply would publish.
    pub resulting_revision: u64,
}

/// Shaped placement intent for the command service: the solved drop
/// plus the batch identity an apply commits atomically.
#[derive(Clone, Debug, PartialEq)]
pub struct DropIntent {
    /// Asset the drop places.
    pub asset: String,
    /// Scene actor the drop moves.
    pub id: u32,
    /// Solved center in meters plus yaw radians.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    /// Document revision the drop was staged against.
    pub base_revision: u64,
    /// Retry identity for the batch.
    pub idempotency_key: u64,
    /// One line per op, in batch order.
    pub effects: Vec<String>,
}

/// What rejected a browser action. Refusals stage nothing. Every bad
/// drop names its blocking relation in [`BrowserError::Conflict`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserError {
    /// The asset name is empty or over the character cap.
    BadName {
        /// The refused name (bounded).
        name: String,
        /// Why it failed (`empty` or `over-128-characters`).
        reason: &'static str,
    },
    /// The asset name is already listed; bundle names never repeat.
    DuplicateName {
        /// The refused name.
        name: String,
    },
    /// The bundle view is fuller than the entry bound.
    TooManyEntries {
        /// Entry count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// A tag is empty or over the character cap.
    BadTag {
        /// The refused tag (bounded).
        tag: String,
        /// Why it failed (`empty` or `over-64-characters`).
        reason: &'static str,
    },
    /// An entry carries more tags than the per-entry bound.
    TooManyTags {
        /// Tag count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// The search text or tag filter is over the query bound.
    BadQuery {
        /// Static reason (never echoes unbounded input).
        reason: &'static str,
    },
    /// The result set is fuller than the result bound.
    TooManyResults {
        /// Hit count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// No listed asset carries this name; the bundle may be missing.
    UnknownAsset {
        /// The requested name (bounded).
        name: String,
    },
    /// No scene actor matches the drop target.
    UnknownObject {
        /// Bounded `id N` / `name '...'` label.
        label: String,
    },
    /// A drop name matches several actors; rename before placing.
    AmbiguousObject {
        /// Bounded `name '...'` label.
        label: String,
        /// Match count.
        count: usize,
    },
    /// A want pairs a relation with a target it never accepts; the
    /// message names the legal targets.
    BadTarget {
        /// Blocking relation name.
        relation: &'static str,
        /// Why the pair never holds, with legal values embedded.
        reason: &'static str,
    },
    /// A distance range is inverted or negative; both ends carried.
    BadDistance {
        /// Blocking relation name.
        relation: &'static str,
        /// Refused minimum.
        min: i32,
        /// Refused maximum.
        max: i32,
    },
    /// A scene actor carries a non-finite lane or a bad extent.
    BadActor {
        /// Actor id refused.
        id: u32,
        /// Static reason.
        reason: &'static str,
    },
    /// The drop cannot land: minimal conflict set attached. `relation`
    /// names the blocking relation; nothing was staged.
    Conflict {
        /// Blocking relation name (`next_to`, `overlap`, ...).
        relation: &'static str,
        /// Why the drop fails.
        reason: String,
        /// Unmet want texts in intent order.
        conflicts: Vec<String>,
    },
    /// The batch base does not match the document revision; nothing ran.
    StaleRevision {
        /// Current document revision.
        expected: u64,
        /// Revision stamped on the batch.
        got: u64,
    },
    /// The drop carries more wants than the per-drop bound.
    TooManyWants {
        /// Want count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// No valid actor anchors the room, so walls have no bounds.
    NoBounds,
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName { name, reason } => {
                write!(f, "asset name `{name}` refused ({reason})")
            }
            Self::DuplicateName { name } => {
                write!(f, "asset name `{name}` is already listed")
            }
            Self::TooManyEntries { count, max } => {
                write!(f, "bundle holds {count} entries over the {max} bound")
            }
            Self::BadTag { tag, reason } => {
                write!(f, "asset tag `{tag}` refused ({reason})")
            }
            Self::TooManyTags { count, max } => {
                write!(f, "entry carries {count} tags over the {max} bound")
            }
            Self::BadQuery { reason } => write!(f, "search refused ({reason})"),
            Self::TooManyResults { count, max } => {
                write!(f, "search found {count} hits over the {max} bound")
            }
            Self::UnknownAsset { name } => {
                write!(f, "unknown asset `{name}`: bundle is missing it")
            }
            Self::UnknownObject { label } => {
                write!(f, "unknown-object {label}")
            }
            Self::AmbiguousObject { label, count } => {
                write!(f, "ambiguous-object {label}: {count} matches")
            }
            Self::BadTarget { relation, reason } => {
                write!(f, "relation `{relation}` refused target: {reason}")
            }
            Self::BadDistance { relation, min, max } => {
                write!(
                    f,
                    "relation `{relation}` refused distance {min}..={max}: \
                     use min >= 0 with max >= min"
                )
            }
            Self::BadActor { id, reason } => {
                write!(f, "actor {id} refused ({reason})")
            }
            Self::Conflict {
                relation,
                reason,
                conflicts,
            } => {
                write!(
                    f,
                    "drop blocked by `{relation}`: {reason}; conflicts: [{}]",
                    conflicts.join(", ")
                )
            }
            Self::StaleRevision { expected, got } => write!(
                f,
                "stale drop: document is at {expected}, drop expects {got}"
            ),
            Self::TooManyWants { count, max } => {
                write!(f, "drop carries {count} wants over the {max} bound")
            }
            Self::NoBounds => f.write_str("no-bounds: no valid actor anchors the room"),
        }
    }
}

impl std::error::Error for BrowserError {}

/// Bound echoed text to [`MAX_DETAIL_CHARS`] on a char boundary.
fn bound_detail(text: &str) -> String {
    bound_to(text, MAX_DETAIL_CHARS)
}

/// Validate one bundle row ahead of listing.
fn validate_entry(entry: &BundleEntry) -> Result<(), BrowserError> {
    if entry.name.is_empty() {
        return Err(BrowserError::BadName {
            name: String::new(),
            reason: "empty",
        });
    }
    if entry.name.chars().count() > MAX_NAME_CHARS {
        return Err(BrowserError::BadName {
            name: bound_detail(&entry.name),
            reason: "over-128-characters",
        });
    }
    if entry.tags.len() > MAX_TAGS_PER_ENTRY {
        return Err(BrowserError::TooManyTags {
            count: entry.tags.len(),
            max: MAX_TAGS_PER_ENTRY,
        });
    }
    for tag in &entry.tags {
        if tag.is_empty() {
            return Err(BrowserError::BadTag {
                tag: String::new(),
                reason: "empty",
            });
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(BrowserError::BadTag {
                tag: bound_detail(tag),
                reason: "over-64-characters",
            });
        }
    }
    Ok(())
}

/// Validate one search query ahead of scanning.
fn validate_query(query: &SearchQuery) -> Result<(), BrowserError> {
    if query.text.chars().count() > MAX_QUERY_CHARS {
        return Err(BrowserError::BadQuery {
            reason: "search text exceeds the 128-character bound",
        });
    }
    if query
        .tag
        .as_ref()
        .is_some_and(|tag| tag.chars().count() > MAX_QUERY_CHARS)
    {
        return Err(BrowserError::BadQuery {
            reason: "tag filter exceeds the 128-character bound",
        });
    }
    Ok(())
}

/// Validate one scene actor ahead of solving.
fn validate_actor(actor: &BrowserActor) -> Result<(), BrowserError> {
    for lane in [
        actor.x, actor.y, actor.z, actor.sx, actor.sy, actor.sz, actor.yaw,
    ] {
        if !lane.is_finite() {
            return Err(BrowserError::BadActor {
                id: actor.id,
                reason: "actor lane must be finite",
            });
        }
    }
    if actor.sx <= 0.0 || actor.sy <= 0.0 || actor.sz <= 0.0 {
        return Err(BrowserError::BadActor {
            id: actor.id,
            reason: "actor extents must be positive",
        });
    }
    Ok(())
}

/// Bundle listing hierarchy: one group per [`AssetKind`] in wire order,
/// each holding its entries with names sorted. The panel renders one
/// section per group; [`search`] scans the groups so hits already come
/// out in kind-then-name order. Pure: same bundle, same groups.
pub fn hierarchy(bundle: &BundleView) -> Vec<(AssetKind, Vec<&BundleEntry>)> {
    let mut groups: Vec<(AssetKind, Vec<&BundleEntry>)> = AssetKind::all()
        .into_iter()
        .map(|kind| (kind, Vec::new()))
        .collect();
    for entry in bundle.entries() {
        if let Some(group) = groups.iter_mut().find(|(kind, _)| *kind == entry.kind) {
            group.1.push(entry);
        }
    }
    for (_, rows) in &mut groups {
        rows.sort_by(|left, right| left.name.cmp(&right.name));
    }
    groups
}

/// Search the whole bundle: substring over names plus optional kind
/// and tag filters. Hits follow [`hierarchy`] order (kind discriminant,
/// then name), so the same bundle plus the same query always yields the
/// same rows.
pub fn search(bundle: &BundleView, query: &SearchQuery) -> Result<Vec<SearchHit>, BrowserError> {
    validate_query(query)?;
    let needle = query.text.to_lowercase();
    let tag_needle = query.tag.as_ref().map(|tag| tag.to_lowercase());
    let mut hits: Vec<SearchHit> = Vec::new();
    for (kind, rows) in hierarchy(bundle) {
        if query.kind.is_some_and(|filter| filter != kind) {
            continue;
        }
        for entry in rows {
            if !needle.is_empty() && !entry.name.to_lowercase().contains(&needle) {
                continue;
            }
            if let Some(ref tag) = tag_needle {
                if !entry.tags.iter().any(|known| known.to_lowercase() == *tag) {
                    continue;
                }
            }
            hits.push(SearchHit {
                kind: entry.kind,
                name: entry.name.clone(),
            });
            if hits.len() > MAX_RESULTS {
                return Err(BrowserError::TooManyResults {
                    count: hits.len(),
                    max: MAX_RESULTS,
                });
            }
        }
    }
    Ok(hits)
}

/// Validate one want shape: relation/target pairs that never hold
/// refuse with the legal targets; bad ranges refuse with both ends.
pub fn validate_want(want: &Want) -> Result<(), BrowserError> {
    match (want.relation, want.target) {
        (Relation::Inside | Relation::OnTopOf | Relation::Supports, Target::Object(_)) => {}
        (Relation::Inside, _) => {
            return Err(BrowserError::BadTarget {
                relation: want.relation.name(),
                reason: "inside needs an object target holding the placed volume",
            });
        }
        (Relation::OnTopOf, _) => {
            return Err(BrowserError::BadTarget {
                relation: want.relation.name(),
                reason: "on_top_of needs an object target to rest on",
            });
        }
        (Relation::Supports, _) => {
            return Err(BrowserError::BadTarget {
                relation: want.relation.name(),
                reason: "supports needs an object target held up above",
            });
        }
        (Relation::NextTo | Relation::Facing, _) => {}
    }
    if let Target::Point { x_cm, z_cm } = want.target {
        if x_cm.abs() > MAX_POINT_CM || z_cm.abs() > MAX_POINT_CM {
            return Err(BrowserError::BadTarget {
                relation: want.relation.name(),
                reason: "point target exceeds the +-1 km scene bound",
            });
        }
    }
    if let Some((min, max)) = want.distance_cm {
        if min < 0 || max < min {
            return Err(BrowserError::BadDistance {
                relation: want.relation.name(),
                min,
                max,
            });
        }
    }
    Ok(())
}

/// Validate one intent shape: bounded wants, each pair legal.
fn validate_intent(intent: &Intent) -> Result<(), BrowserError> {
    if intent.wants.len() > MAX_WANTS_PER_DROP {
        return Err(BrowserError::TooManyWants {
            count: intent.wants.len(),
            max: MAX_WANTS_PER_DROP,
        });
    }
    for want in &intent.wants {
        validate_want(want)?;
    }
    Ok(())
}

/// Meters to integer centimeters (banker-free round, matches the spatial solver).
fn to_cm(meters: f32) -> i32 {
    (f64::from(meters) * 100.0).round() as i32
}

/// Octant of a yaw step: `0` faces +X like the spatial relations; `k * PI/4` after.
fn octant_of(yaw_idx: i32) -> (i32, i32) {
    match yaw_idx.rem_euclid(YAW_STEPS) {
        0 => (1, 0),
        1 => (1, 1),
        2 => (0, 1),
        3 => (-1, 1),
        4 => (-1, 0),
        5 => (-1, -1),
        6 => (0, -1),
        _ => (1, -1),
    }
}

/// Yaw radians of one yaw step.
fn yaw_of(yaw_idx: i32) -> f32 {
    yaw_idx as f32 * core::f32::consts::FRAC_PI_4
}

/// Union XZ bounds over valid actors, in integer centimeters.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

fn room_bounds(actors: &[BrowserActor]) -> Option<Bounds> {
    let mut bounds = Bounds {
        min_x: i32::MAX,
        max_x: i32::MIN,
        min_z: i32::MAX,
        max_z: i32::MIN,
    };
    let mut any = false;
    for actor in actors {
        if actor.validate_silent().is_err() {
            continue;
        }
        let hx = to_cm(actor.sx).abs() / 2;
        let hz = to_cm(actor.sz).abs() / 2;
        let (cx, cz) = (to_cm(actor.x), to_cm(actor.z));
        bounds.min_x = bounds.min_x.min(cx - hx);
        bounds.max_x = bounds.max_x.max(cx + hx);
        bounds.min_z = bounds.min_z.min(cz - hz);
        bounds.max_z = bounds.max_z.max(cz + hz);
        any = true;
    }
    if any {
        Some(bounds)
    } else {
        None
    }
}

impl BrowserActor {
    /// Silent validity for bound scans (solver reports loudly elsewhere).
    fn validate_silent(&self) -> Result<(), ()> {
        if ![self.x, self.y, self.z, self.sx, self.sy, self.sz, self.yaw]
            .iter()
            .all(|lane| lane.is_finite())
        {
            return Err(());
        }
        if self.sx <= 0.0 || self.sy <= 0.0 || self.sz <= 0.0 {
            return Err(());
        }
        Ok(())
    }
}

/// Resolved drop geometry plus its grounded rest height in meters.
struct Placed {
    id: u32,
    sx: f32,
    sz: f32,
    y_m: f32,
}

fn resolve_object(intent: &Intent, actors: &[BrowserActor]) -> Result<Placed, BrowserError> {
    let matches: Vec<&BrowserActor> = match &intent.object {
        ObjectRef::Id(id) => actors.iter().filter(|actor| actor.id == *id).collect(),
        ObjectRef::Name(name) => actors.iter().filter(|actor| &actor.name == name).collect(),
    };
    if matches.is_empty() {
        return Err(BrowserError::UnknownObject {
            label: intent.object.label(),
        });
    }
    if matches.len() > 1 {
        let label = match &intent.object {
            ObjectRef::Id(id) => format!("id {id}"),
            ObjectRef::Name(name) => format!("name '{}'", bound_detail(name)),
        };
        return Err(BrowserError::AmbiguousObject {
            label,
            count: matches.len(),
        });
    }
    let actor = matches[0];
    validate_actor(actor)?;
    Ok(Placed {
        id: actor.id,
        sx: actor.sx,
        sz: actor.sz,
        y_m: actor.sy * 0.5,
    })
}

/// Solver working set: frozen maps plus bounds, all order-invariant.
struct Ctx<'a> {
    bounds: Bounds,
    boxes: std::collections::BTreeMap<u32, (i32, i32, i32, i32)>,
    by_id: std::collections::BTreeMap<u32, &'a BrowserActor>,
}

/// XZ box `(min_x, max_x, min_z, max_z)` in centimeters.
fn box_of(actor: &BrowserActor) -> (i32, i32, i32, i32) {
    let hx = to_cm(actor.sx).abs() / 2;
    let hz = to_cm(actor.sz).abs() / 2;
    let (cx, cz) = (to_cm(actor.x), to_cm(actor.z));
    (cx - hx, cx + hx, cz - hz, cz + hz)
}

fn context<'a>(actors: &'a [BrowserActor], bounds: Bounds) -> Ctx<'a> {
    let mut boxes = std::collections::BTreeMap::new();
    for actor in actors {
        if actor.validate_silent().is_ok() {
            boxes.insert(actor.id, box_of(actor));
        }
    }
    let by_id: std::collections::BTreeMap<u32, &'a BrowserActor> =
        actors.iter().map(|actor| (actor.id, actor)).collect();
    Ctx {
        bounds,
        boxes,
        by_id,
    }
}

fn valid_range(range: Option<(i32, i32)>) -> Option<(i64, i64)> {
    range
        .map(|(lo, hi)| (i64::from(lo), i64::from(hi)))
        .filter(|(lo, hi)| *lo >= 0 && *hi >= *lo)
}

fn valid_range_or(want: Option<(i32, i32)>, fallback: (i32, i32)) -> Option<(i64, i64)> {
    match want {
        Some(_) => valid_range(want),
        None => Some((i64::from(fallback.0), i64::from(fallback.1))),
    }
}

/// Inclusive range check over a squared center distance.
fn dist_in(center2: i64, range: (i64, i64)) -> bool {
    range.0 * range.0 <= center2 && center2 <= range.1 * range.1
}

fn center2(cx: i32, cz: i32, tx: i32, tz: i32) -> i64 {
    let dx = i64::from(cx) - i64::from(tx);
    let dz = i64::from(cz) - i64::from(tz);
    dx * dx + dz * dz
}

fn edge_gap2(min_x: i32, max_x: i32, min_z: i32, max_z: i32, other: &(i32, i32, i32, i32)) -> i64 {
    let gx = (i64::from(min_x.max(other.0)) - i64::from(max_x.min(other.1))).max(0);
    let gz = (i64::from(min_z.max(other.2)) - i64::from(max_z.min(other.3))).max(0);
    gx * gx + gz * gz
}

/// Spatial facing: strictly ahead in the yaw-snapped half-plane and in range.
fn face_holds(cx: i32, cz: i32, yaw_idx: i32, tx: i32, tz: i32) -> bool {
    if cx == tx && cz == tz {
        return false;
    }
    let (fx, fz) = octant_of(yaw_idx);
    let dx = i64::from(tx) - i64::from(cx);
    let dz = i64::from(tz) - i64::from(cz);
    if i64::from(fx) * dx + i64::from(fz) * dz <= 0 {
        return false;
    }
    let range = i64::from(FACING_RANGE_CM);
    dx * dx + dz * dz <= range * range
}

/// Optional center-distance refinement; `None` always holds.
fn dist_holds(cx: i32, cz: i32, tx: i32, tz: i32, distance_cm: Option<(i32, i32)>) -> bool {
    match distance_cm {
        None => true,
        Some(_) => match valid_range(distance_cm) {
            Some(range) => dist_in(center2(cx, cz, tx, tz), range),
            None => false,
        },
    }
}

/// Gap in cm between the probe footprint edge and the named room wall.
fn wall_gap(wall: Wall, bounds: &Bounds, min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> i64 {
    match wall {
        Wall::North => i64::from(min_z) - i64::from(bounds.min_z),
        Wall::South => i64::from(bounds.max_z) - i64::from(max_z),
        Wall::West => i64::from(min_x) - i64::from(bounds.min_x),
        Wall::East => i64::from(bounds.max_x) - i64::from(max_x),
    }
}

/// True when the probe at `(cx, cz)` with `yaw_idx` satisfies `want`.
/// All geometry runs on integer centimeters.
fn satisfies(want: &Want, cx: i32, cz: i32, yaw_idx: i32, obj: &Placed, ctx: &Ctx<'_>) -> bool {
    let hx = to_cm(obj.sx).abs() / 2;
    let hz = to_cm(obj.sz).abs() / 2;
    let (min_x, max_x) = (cx - hx, cx + hx);
    let (min_z, max_z) = (cz - hz, cz + hz);
    let bounds = &ctx.bounds;
    match (want.relation, want.target) {
        (Relation::NextTo, Target::Wall(wall)) => {
            let Some(range) = valid_range_or(want.distance_cm, DEFAULT_WALL_GAP_CM) else {
                return false;
            };
            let gap = wall_gap(wall, bounds, min_x, max_x, min_z, max_z);
            range.0 <= gap && gap <= range.1
        }
        (Relation::NextTo, Target::Object(id)) => {
            let Some(range) = valid_range_or(want.distance_cm, (0, NEXT_TO_CM)) else {
                return false;
            };
            let Some(other) = ctx.boxes.get(&id) else {
                return false;
            };
            if id == obj.id {
                return false;
            }
            dist_in(edge_gap2(min_x, max_x, min_z, max_z, other), range)
        }
        (Relation::NextTo, Target::Point { x_cm, z_cm }) => {
            let Some(range) = valid_range_or(want.distance_cm, DEFAULT_POINT_DIST_CM) else {
                return false;
            };
            dist_in(center2(cx, cz, x_cm, z_cm), range)
        }
        (Relation::Facing, Target::Wall(wall)) => {
            let (fx, fz) = octant_of(yaw_idx);
            let toward = match wall {
                Wall::North => fz < 0,
                Wall::South => fz > 0,
                Wall::West => fx < 0,
                Wall::East => fx > 0,
            };
            if !toward {
                return false;
            }
            match want.distance_cm {
                None => true,
                Some(_) => {
                    let Some(range) = valid_range(want.distance_cm) else {
                        return false;
                    };
                    let gap = wall_gap(wall, bounds, min_x, max_x, min_z, max_z);
                    range.0 <= gap && gap <= range.1
                }
            }
        }
        (Relation::Facing, Target::Object(id)) => {
            let Some(target) = ctx.by_id.get(&id) else {
                return false;
            };
            if target.id == obj.id || target.validate_silent().is_err() {
                return false;
            }
            face_holds(cx, cz, yaw_idx, to_cm(target.x), to_cm(target.z))
                && dist_holds(cx, cz, to_cm(target.x), to_cm(target.z), want.distance_cm)
        }
        (Relation::Facing, Target::Point { x_cm, z_cm }) => {
            face_holds(cx, cz, yaw_idx, x_cm, z_cm)
                && dist_holds(cx, cz, x_cm, z_cm, want.distance_cm)
        }
        (Relation::Inside, Target::Object(id)) => {
            let Some(other) = ctx.boxes.get(&id) else {
                return false;
            };
            if id == obj.id {
                return false;
            }
            other.0 <= min_x && max_x <= other.1 && other.2 <= min_z && max_z <= other.3
        }
        (Relation::Inside, _) => false,
        (Relation::OnTopOf | Relation::Supports, Target::Object(id)) => {
            let Some(other) = ctx.boxes.get(&id) else {
                return false;
            };
            if id == obj.id {
                return false;
            }
            // XZ overlap with the support footprint; height is grounded
            // by construction, so the check is the footprint hit.
            min_x < other.1 && other.0 < max_x && min_z < other.3 && other.2 < max_z
        }
        (Relation::OnTopOf | Relation::Supports, _) => false,
    }
}

struct Candidate {
    cx: i32,
    cz: i32,
    yaw_idx: i32,
}

/// Frozen candidate order: z ascending, x ascending, yaw ascending.
fn candidates(obj: &Placed, bounds: &Bounds) -> Vec<Candidate> {
    let hx = to_cm(obj.sx).abs() / 2;
    let hz = to_cm(obj.sz).abs() / 2;
    let (x_lo, x_hi) = (bounds.min_x + hx, bounds.max_x - hx);
    let (z_lo, z_hi) = (bounds.min_z + hz, bounds.max_z - hz);
    if x_lo > x_hi || z_lo > z_hi {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cz = z_lo;
    while cz <= z_hi {
        let mut cx = x_lo;
        while cx <= x_hi {
            for yaw_idx in 0..YAW_STEPS {
                out.push(Candidate { cx, cz, yaw_idx });
            }
            cx += GRID_CM;
        }
        cz += GRID_CM;
    }
    out
}

fn wants_hold(wants: &[Want], cx: i32, cz: i32, yaw_idx: i32, obj: &Placed, ctx: &Ctx<'_>) -> bool {
    wants
        .iter()
        .all(|want| satisfies(want, cx, cz, yaw_idx, obj, ctx))
}

/// Spatial-style self-verify: no new XZ `overlap` with another valid actor.
/// Placements rest grounded by construction, so `floating` never fires.
fn verifies(actors: &[BrowserActor], obj: &Placed, cx: i32, cz: i32) -> bool {
    let hx = to_cm(obj.sx).abs() / 2;
    let hz = to_cm(obj.sz).abs() / 2;
    let (min_x, max_x) = (cx - hx, cx + hx);
    let (min_z, max_z) = (cz - hz, cz + hz);
    for actor in actors {
        if actor.id == obj.id || actor.validate_silent().is_err() {
            continue;
        }
        let (ox0, ox1, oz0, oz1) = box_of(actor);
        if min_x < ox1 && ox0 < max_x && min_z < oz1 && oz0 < max_z {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Solved {
    cx: i32,
    cz: i32,
    yaw_idx: i32,
}

fn search_wants(
    wants: &[Want],
    obj: &Placed,
    ctx: &Ctx<'_>,
    actors: &[BrowserActor],
    grid: &[Candidate],
) -> Option<Solved> {
    for candidate in grid {
        if !wants_hold(
            wants,
            candidate.cx,
            candidate.cz,
            candidate.yaw_idx,
            obj,
            ctx,
        ) {
            continue;
        }
        if verifies(actors, obj, candidate.cx, candidate.cz) {
            return Some(Solved {
                cx: candidate.cx,
                cz: candidate.cz,
                yaw_idx: candidate.yaw_idx,
            });
        }
    }
    None
}

/// Blocking relation name for a conflict: the first unmet want's
/// relation, or `overlap` when every want holds but no cell verifies.
fn blocking_relation(unmet: &[Want]) -> &'static str {
    unmet
        .first()
        .map(|want| want.relation.name())
        .unwrap_or("overlap")
}

/// Solve one placement intent over an actor snapshot.
///
/// Returns the first quantized placement (10 cm grid, 8 yaws, z/x/yaw
/// order) satisfying every want and passing the overlap self-check,
/// or the minimal conflicting subset on failure. Pure: reads only.
fn solve_drop(
    intent: &Intent,
    actors: &[BrowserActor],
) -> Result<(Placed, Solved, Bounds), BrowserError> {
    validate_intent(intent)?;
    if actors.len() > MAX_ACTORS_PER_SCENE {
        return Err(BrowserError::BadActor {
            id: u32::MAX,
            reason: "scene exceeds the actor bound",
        });
    }
    // An empty scene has no walls to measure against; refuse before
    // resolving so the panel names the missing room, not the object.
    if actors.is_empty() {
        return Err(BrowserError::NoBounds);
    }
    for actor in actors {
        validate_actor(actor).map_err(|_| BrowserError::BadActor {
            id: actor.id,
            reason: "actor lane must be finite with positive extents",
        })?;
    }
    let obj = resolve_object(intent, actors).map_err(|error| match error {
        BrowserError::UnknownObject { label } => BrowserError::Conflict {
            relation: "unknown-object",
            reason: format!("unknown-object {label}"),
            conflicts: intent.wants.iter().map(|want| want.describe()).collect(),
        },
        BrowserError::AmbiguousObject { label, .. } => BrowserError::Conflict {
            relation: "ambiguous-object",
            reason: format!("ambiguous-object {label}"),
            conflicts: intent.wants.iter().map(|want| want.describe()).collect(),
        },
        BrowserError::BadActor { id, .. } => BrowserError::Conflict {
            relation: "unknown-size",
            reason: format!("unknown-size id {id}"),
            conflicts: intent.wants.iter().map(|want| want.describe()).collect(),
        },
        other => other,
    })?;
    let Some(bounds) = room_bounds(actors) else {
        return Err(BrowserError::NoBounds);
    };
    let ctx = context(actors, bounds);
    let grid = candidates(&obj, &bounds);
    if let Some(solved) = search_wants(&intent.wants, &obj, &ctx, actors, &grid) {
        return Ok((obj, solved, bounds));
    }
    // Minimal conflict: shrink by removal in intent order. Removing
    // want `i` sticks only while the remainder stays unsatisfiable.
    let mut conflict: Vec<usize> = (0..intent.wants.len()).collect();
    for i in 0..intent.wants.len() {
        if !conflict.contains(&i) {
            continue;
        }
        let trial: Vec<Want> = conflict
            .iter()
            .filter(|&&j| j != i)
            .map(|&j| intent.wants[j])
            .collect();
        if search_wants(&trial, &obj, &ctx, actors, &grid).is_none() {
            conflict.retain(|&j| j != i);
        }
    }
    let unmet: Vec<Want> = conflict.iter().map(|&j| intent.wants[j]).collect();
    let relation = blocking_relation(&unmet);
    let conflicts: Vec<String> = unmet.iter().map(|want| want.describe()).collect();
    Err(BrowserError::Conflict {
        relation,
        reason: format!(
            "no placement on {GRID_CM} cm grid x {YAW_STEPS} yaws satisfies {} want(s); \
             minimal conflict covers {} want(s)",
            intent.wants.len(),
            unmet.len(),
        ),
        conflicts,
    })
}

/// Effect line for one solved drop, in batch order.
fn effect_line(asset: &str, id: u32, x: f32, y: f32, z: f32, yaw: f32) -> String {
    format!("place {asset} as actor {id} at ({x:.2}, {y:.2}, {z:.2}) yaw {yaw:.2}")
}

/// Preview one drop without side effects: the asset must be listed,
/// the intent must validate and solve; the returned preview carries
/// one effect line plus the revision an apply would publish.
pub fn preview_drop(
    bundle: &BundleView,
    asset: &str,
    intent: &Intent,
    actors: &[BrowserActor],
    base_revision: u64,
) -> Result<DropPreview, BrowserError> {
    let entry = bundle
        .find(asset)
        .ok_or_else(|| BrowserError::UnknownAsset {
            name: bound_detail(asset),
        })?;
    let (obj, solved, _) = solve_drop(intent, actors)?;
    let (x, z) = (solved.cx as f32 / 100.0, solved.cz as f32 / 100.0);
    let (y, yaw) = (obj.y_m, yaw_of(solved.yaw_idx));
    Ok(DropPreview {
        asset: entry.name.clone(),
        id: obj.id,
        x,
        y,
        z,
        yaw,
        effects: vec![effect_line(&entry.name, obj.id, x, y, z, yaw)],
        resulting_revision: base_revision + 1,
    })
}

/// Shape one drop as a placement intent for the command service.
/// Validates, previews, then shapes: any failure returns the error
/// and shapes nothing. Never mutates any document.
pub fn emit_drop(
    bundle: &BundleView,
    asset: &str,
    intent: &Intent,
    actors: &[BrowserActor],
    base_revision: u64,
    idempotency_key: u64,
    live_revision: u64,
) -> Result<DropIntent, BrowserError> {
    if live_revision != base_revision {
        return Err(BrowserError::StaleRevision {
            expected: live_revision,
            got: base_revision,
        });
    }
    let preview = preview_drop(bundle, asset, intent, actors, base_revision)?;
    Ok(DropIntent {
        asset: preview.asset,
        id: preview.id,
        x: preview.x,
        y: preview.y,
        z: preview.z,
        yaw: preview.yaw,
        base_revision,
        idempotency_key,
        effects: preview.effects,
    })
}

/// Staged drop inside the panel: the asset name plus its intent.
#[derive(Clone, Debug, PartialEq, Eq)]
struct StagedDrop {
    asset: String,
    intent: Intent,
}

/// Asset browser panel: bundle search plus one staged drag-to-place.
///
/// The panel owns no scene state: [`search`] renders hits from caller
/// snapshots, `stage` validates one drop, `preview` solves it without
/// side effects, and `emit` shapes it for the command service. Undo,
/// provenance and capability checks stay with the command service
/// that applies the shaped intent.
#[derive(Clone, Debug, Default)]
pub struct BrowserPanel {
    staged: Option<StagedDrop>,
    base_revision: u64,
}

impl BrowserPanel {
    /// Empty panel: no staged drop, revision zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Revision staged drops belong to.
    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    /// True when no drop is staged.
    pub fn is_empty(&self) -> bool {
        self.staged.is_none()
    }

    /// Pin the revision staged drops belong to (call on rebuild).
    pub fn rebase(&mut self, revision: u64) {
        self.base_revision = revision;
    }

    /// Drop the staged drop. Returns true when one was staged.
    pub fn clear(&mut self) -> bool {
        self.staged.take().is_some()
    }

    /// Search the bundle from caller snapshots (pure panel helper).
    pub fn search(
        &self,
        bundle: &BundleView,
        query: &SearchQuery,
    ) -> Result<Vec<SearchHit>, BrowserError> {
        search(bundle, query)
    }

    /// Stage one drop after validating the asset and the intent shape.
    /// Solving waits for [`preview`](Self::preview).
    pub fn stage(
        &mut self,
        bundle: &BundleView,
        asset: &str,
        intent: Intent,
    ) -> Result<(), BrowserError> {
        if bundle.find(asset).is_none() {
            return Err(BrowserError::UnknownAsset {
                name: bound_detail(asset),
            });
        }
        validate_intent(&intent)?;
        self.staged = Some(StagedDrop {
            asset: asset.to_string(),
            intent,
        });
        Ok(())
    }

    /// Preview the staged drop without side effects.
    pub fn preview(
        &self,
        bundle: &BundleView,
        actors: &[BrowserActor],
    ) -> Result<DropPreview, BrowserError> {
        let Some(staged) = &self.staged else {
            return Err(BrowserError::Conflict {
                relation: "no-drop",
                reason: "no drop staged; stage one first".to_string(),
                conflicts: Vec::new(),
            });
        };
        preview_drop(
            bundle,
            &staged.asset,
            &staged.intent,
            actors,
            self.base_revision,
        )
    }

    /// Emit the staged drop for the command service (validate, shape).
    pub fn emit(
        &self,
        bundle: &BundleView,
        actors: &[BrowserActor],
        idempotency_key: u64,
        live_revision: u64,
    ) -> Result<DropIntent, BrowserError> {
        let Some(staged) = &self.staged else {
            return Err(BrowserError::Conflict {
                relation: "no-drop",
                reason: "no drop staged; stage one first".to_string(),
                conflicts: Vec::new(),
            });
        };
        emit_drop(
            bundle,
            &staged.asset,
            &staged.intent,
            actors,
            self.base_revision,
            idempotency_key,
            live_revision,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn entry(kind: AssetKind, name: &str, tags: &[&str]) -> BundleEntry {
        BundleEntry::new(kind, name, tags)
    }

    fn small_bundle() -> BundleView {
        let mut bundle = BundleView::new();
        bundle
            .push(entry(AssetKind::Mesh, "hero", &["fighter", "hero"]))
            .unwrap();
        bundle
            .push(entry(AssetKind::Material, "hero-mat", &["fighter"]))
            .unwrap();
        bundle
            .push(entry(
                AssetKind::Texture,
                "hero-albedo",
                &["fighter", "albedo"],
            ))
            .unwrap();
        bundle
            .push(entry(AssetKind::Skin, "hero-skin", &["fighter"]))
            .unwrap();
        bundle
            .push(entry(AssetKind::Clip, "hero-step", &["locomotion"]))
            .unwrap();
        bundle
            .push(entry(AssetKind::Mesh, "crate", &["prop"]))
            .unwrap();
        bundle
            .push(entry(AssetKind::Mesh, "brazier", &["prop", "torch"]))
            .unwrap();
        bundle
    }

    fn resting(id: u32, name: &str, x: f32, z: f32) -> BrowserActor {
        BrowserActor::new(id, name, x, 0.5, z, 1.0, 1.0, 1.0, 0.0)
    }

    fn wall(id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32) -> BrowserActor {
        BrowserActor::new(id, name, x, 1.0, z, sx, 2.0, sz, 0.0)
    }

    /// Walled room plus a gate slab and a brazier starter: all pairs
    /// XZ-disjoint, everything grounded and finite.
    fn room() -> Vec<BrowserActor> {
        vec![
            wall(0, "wall_north", 0.0, -5.0, 9.8, 0.2),
            wall(1, "wall_south", 0.0, 5.0, 9.8, 0.2),
            wall(2, "wall_west", -5.0, 0.0, 0.2, 10.0),
            wall(3, "wall_east", 5.0, 0.0, 0.2, 10.0),
            resting(4, "gate", 3.0, -4.5),
            resting(5, "brazier", 0.0, 2.0),
        ]
    }

    fn board_intent() -> Intent {
        Intent {
            object: ObjectRef::Name("brazier".to_string()),
            wants: vec![
                Want {
                    relation: Relation::NextTo,
                    target: Target::Wall(Wall::North),
                    distance_cm: Some((0, 60)),
                },
                Want {
                    relation: Relation::NextTo,
                    target: Target::Object(4),
                    distance_cm: Some((150, 250)),
                },
            ],
        }
    }

    #[test]
    fn kind_wire_bytes_match_as1_entrykind() {
        assert_eq!(AssetKind::Mesh as u8, 1);
        assert_eq!(AssetKind::Material as u8, 2);
        assert_eq!(AssetKind::Texture as u8, 3);
        assert_eq!(AssetKind::Skin as u8, 4);
        assert_eq!(AssetKind::Clip as u8, 5);
        for (byte, kind) in [
            (1, AssetKind::Mesh),
            (2, AssetKind::Material),
            (3, AssetKind::Texture),
            (4, AssetKind::Skin),
            (5, AssetKind::Clip),
        ] {
            assert_eq!(AssetKind::from_byte(byte), Some(kind));
        }
        assert_eq!(AssetKind::from_byte(0), None);
        assert_eq!(AssetKind::from_byte(6), None);
        assert_eq!(AssetKind::all().len(), 5);
    }

    #[test]
    fn search_filters_by_text_kind_and_tag() {
        let bundle = small_bundle();
        let hits = search(&bundle, &SearchQuery::new("hero", None, None)).unwrap();
        assert_eq!(hits.len(), 5);
        // Kind order first, then name: mesh, material, texture, skin, clip.
        assert_eq!(hits[0].kind, AssetKind::Mesh);
        assert_eq!(hits[4].kind, AssetKind::Clip);

        let meshes = search(&bundle, &SearchQuery::new("", Some(AssetKind::Mesh), None)).unwrap();
        assert!(meshes.iter().all(|hit| hit.kind == AssetKind::Mesh));
        assert_eq!(meshes.len(), 3);

        let props = search(&bundle, &SearchQuery::new("", None, Some("prop"))).unwrap();
        assert_eq!(props.len(), 2);

        let combo = search(
            &bundle,
            &SearchQuery::new("braz", Some(AssetKind::Mesh), Some("torch")),
        )
        .unwrap();
        assert_eq!(combo.len(), 1);
        assert_eq!(combo[0].name, "brazier");
        assert_eq!(combo[0].row(), "mesh brazier");

        // Empty text matches the whole bundle; tag match is exact.
        let all = search(&bundle, &SearchQuery::new("", None, None)).unwrap();
        assert_eq!(all.len(), 7);
        let partial = search(&bundle, &SearchQuery::new("", None, Some("prop-"))).unwrap();
        assert!(partial.is_empty());
    }

    #[test]
    fn search_order_is_deterministic() {
        let bundle = small_bundle();
        let query = SearchQuery::new("", None, None);
        let first = search(&bundle, &query).unwrap();
        let second = search(&bundle, &query).unwrap();
        assert_eq!(first, second);
        let keys: Vec<(u8, &str)> = first
            .iter()
            .map(|hit| (hit.kind as u8, hit.name.as_str()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn hierarchy_groups_bundle_in_wire_order() {
        let bundle = small_bundle();
        let groups = hierarchy(&bundle);
        assert_eq!(groups.len(), AssetKind::all().len());
        let kinds: Vec<AssetKind> = groups.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, AssetKind::all());
        // Every listed entry appears exactly once, under its own kind,
        // with names sorted inside each kind group.
        let mut total = 0;
        for (kind, rows) in &groups {
            assert!(rows.iter().all(|entry| entry.kind == *kind));
            let names: Vec<&str> = rows.iter().map(|entry| entry.name.as_str()).collect();
            let mut sorted = names.clone();
            sorted.sort_unstable();
            assert_eq!(names, sorted);
            total += rows.len();
        }
        assert_eq!(total, bundle.len());
    }

    #[test]
    fn search_all_matches_flattened_hierarchy() {
        let bundle = small_bundle();
        let hits = search(&bundle, &SearchQuery::new("", None, None)).unwrap();
        let flat: Vec<SearchHit> = hierarchy(&bundle)
            .into_iter()
            .flat_map(|(kind, rows)| {
                rows.into_iter().map(move |entry| SearchHit {
                    kind,
                    name: entry.name.clone(),
                })
            })
            .collect();
        assert_eq!(hits, flat);
    }

    #[test]
    fn full_bundle_search_stays_under_50ms() {
        let mut bundle = BundleView::new();
        for index in 0..MAX_ENTRIES {
            let kind = AssetKind::all()[index % AssetKind::all().len()];
            let name = format!("asset_{index:04}");
            let tag: &str = if index % 2 == 0 { "even" } else { "odd" };
            bundle.push(BundleEntry::new(kind, &name, &[tag])).unwrap();
        }
        assert_eq!(bundle.len(), MAX_ENTRIES);
        let query = SearchQuery::new("asset_3", None, Some("odd"));
        let start = Instant::now();
        let hits = search(&bundle, &query).unwrap();
        let elapsed_ms = start.elapsed().as_millis();
        eprintln!("full-bundle search: {} hits in {elapsed_ms} ms", hits.len());
        assert_eq!(
            hits.len(),
            500,
            "asset_3 selects 1000 names, the odd tag keeps 500"
        );
        assert!(
            u128::from(hits.len() as u32) <= u128::from(MAX_RESULTS as u32),
            "hits fit the result bound"
        );
        // Same query twice gives the same rows.
        assert_eq!(hits, search(&bundle, &query).unwrap());
    }

    #[test]
    fn duplicate_names_and_bad_rows_refuse() {
        let mut bundle = BundleView::new();
        bundle
            .push(entry(AssetKind::Mesh, "crate", &["prop"]))
            .unwrap();
        assert_eq!(
            bundle
                .push(entry(AssetKind::Mesh, "crate", &["prop"]))
                .unwrap_err(),
            BrowserError::DuplicateName {
                name: "crate".to_string()
            }
        );
        assert!(matches!(
            validate_entry(&BundleEntry::new(AssetKind::Mesh, "", &[])),
            Err(BrowserError::BadName { .. })
        ));
        let long = "n".repeat(MAX_NAME_CHARS + 1);
        assert!(matches!(
            validate_entry(&BundleEntry::new(AssetKind::Mesh, &long, &[])),
            Err(BrowserError::BadName { .. })
        ));
        let many: Vec<&str> = vec!["t"; MAX_TAGS_PER_ENTRY + 1];
        assert_eq!(
            validate_entry(&BundleEntry::new(AssetKind::Mesh, "ok", &many)).unwrap_err(),
            BrowserError::TooManyTags {
                count: MAX_TAGS_PER_ENTRY + 1,
                max: MAX_TAGS_PER_ENTRY
            }
        );
        assert!(matches!(
            validate_entry(&BundleEntry::new(AssetKind::Mesh, "ok", &[""])),
            Err(BrowserError::BadTag { .. })
        ));
        let long_query = "q".repeat(MAX_QUERY_CHARS + 1);
        assert!(matches!(
            search(&bundle, &SearchQuery::new(&long_query, None, None)).unwrap_err(),
            BrowserError::BadQuery { .. }
        ));
        assert_eq!(bundle.len(), 1);
        assert!(bundle.contains("crate"));
        assert!(!bundle.contains("missing"));
    }

    #[test]
    fn good_drop_solves_with_preview_discipline() {
        let bundle = small_bundle();
        let actors = room();
        let snapshot = actors.clone();
        let intent = board_intent();
        let preview = preview_drop(&bundle, "brazier", &intent, &actors, 7).unwrap();
        assert_eq!(preview.asset, "brazier");
        assert_eq!(preview.id, 5);
        assert_eq!(preview.y, 0.5);
        assert_eq!(preview.effects.len(), 1);
        assert!(preview.effects[0].contains("brazier"));
        assert!(preview.effects[0].contains('5'));
        assert_eq!(preview.resulting_revision, 8);
        // Preview writes nothing: caller snapshots are untouched.
        assert_eq!(actors, snapshot);
        // Same intent twice solves identically.
        let again = preview_drop(&bundle, "brazier", &intent, &actors, 7).unwrap();
        assert_eq!(preview, again);
        // Input order never moves the answer.
        let mut shuffled = actors.clone();
        shuffled.reverse();
        let moved = preview_drop(&bundle, "brazier", &intent, &shuffled, 7).unwrap();
        assert_eq!(
            (preview.x, preview.z, preview.yaw),
            (moved.x, moved.z, moved.yaw)
        );
    }

    #[test]
    fn bad_drop_names_its_blocking_relation() {
        let bundle = small_bundle();
        let actors = room();
        let against = |wall: Wall| Want {
            relation: Relation::NextTo,
            target: Target::Wall(wall),
            distance_cm: Some((0, 50)),
        };
        let intent = Intent {
            object: ObjectRef::Id(5),
            wants: vec![against(Wall::North), against(Wall::South)],
        };
        let snapshot = actors.clone();
        let error = preview_drop(&bundle, "brazier", &intent, &actors, 0).unwrap_err();
        let BrowserError::Conflict {
            relation,
            conflicts,
            reason,
        } = &error
        else {
            panic!("both walls must conflict, got {error}");
        };
        assert_eq!(*relation, "next_to");
        assert_eq!(conflicts.len(), 2);
        assert!(reason.contains("minimal conflict"));
        assert!(error.to_string().contains("next_to"));
        assert_eq!(actors, snapshot, "conflicts write nothing");
        // Each want alone solves: the conflict is minimal.
        for want in &intent.wants {
            let single = Intent {
                object: ObjectRef::Id(5),
                wants: vec![*want],
            };
            preview_drop(&bundle, "brazier", &single, &actors, 0).expect("single wall must solve");
        }
    }

    #[test]
    fn missing_asset_and_object_fail_loudly() {
        let bundle = small_bundle();
        let actors = room();
        assert_eq!(
            preview_drop(&bundle, "missing", &board_intent(), &actors, 0).unwrap_err(),
            BrowserError::UnknownAsset {
                name: "missing".to_string()
            }
        );
        let missing = Intent {
            object: ObjectRef::Name("missing_prop".to_string()),
            wants: vec![],
        };
        let error = preview_drop(&bundle, "crate", &missing, &actors, 0).unwrap_err();
        assert!(matches!(error, BrowserError::Conflict { .. }));
        assert!(error.to_string().contains("unknown-object"));
        let mut dupes = room();
        dupes.push(resting(6, "brazier", -2.0, 2.0));
        let dupe = Intent {
            object: ObjectRef::Name("brazier".to_string()),
            wants: vec![],
        };
        let error = preview_drop(&bundle, "crate", &dupe, &dupes, 0).unwrap_err();
        assert!(error.to_string().contains("ambiguous-object"));
    }

    #[test]
    fn want_shapes_refuse_with_legal_values() {
        let inside_wall = Want {
            relation: Relation::Inside,
            target: Target::Wall(Wall::North),
            distance_cm: None,
        };
        assert_eq!(
            validate_want(&inside_wall).unwrap_err(),
            BrowserError::BadTarget {
                relation: "inside",
                reason: "inside needs an object target holding the placed volume",
            }
        );
        let top_wall = Want {
            relation: Relation::OnTopOf,
            target: Target::Point { x_cm: 0, z_cm: 0 },
            distance_cm: None,
        };
        assert!(matches!(
            validate_want(&top_wall).unwrap_err(),
            BrowserError::BadTarget {
                relation: "on_top_of",
                ..
            }
        ));
        let supports_wall = Want {
            relation: Relation::Supports,
            target: Target::Wall(Wall::East),
            distance_cm: None,
        };
        assert!(matches!(
            validate_want(&supports_wall).unwrap_err(),
            BrowserError::BadTarget {
                relation: "supports",
                ..
            }
        ));
        let inverted = Want {
            relation: Relation::NextTo,
            target: Target::Wall(Wall::North),
            distance_cm: Some((60, 10)),
        };
        assert_eq!(
            validate_want(&inverted).unwrap_err(),
            BrowserError::BadDistance {
                relation: "next_to",
                min: 60,
                max: 10
            }
        );
        let negative = Want {
            relation: Relation::Facing,
            target: Target::Object(4),
            distance_cm: Some((-5, 10)),
        };
        assert!(matches!(
            validate_want(&negative).unwrap_err(),
            BrowserError::BadDistance { .. }
        ));
        let far = Want {
            relation: Relation::NextTo,
            target: Target::Point {
                x_cm: MAX_POINT_CM + 1,
                z_cm: 0,
            },
            distance_cm: None,
        };
        assert!(matches!(
            validate_want(&far).unwrap_err(),
            BrowserError::BadTarget { .. }
        ));
        // Unknown target objects are conflicts, not shape errors.
        let bundle = small_bundle();
        let actors = room();
        let ghost = Intent {
            object: ObjectRef::Id(5),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Object(999),
                distance_cm: None,
            }],
        };
        let error = preview_drop(&bundle, "brazier", &ghost, &actors, 0).unwrap_err();
        assert!(matches!(error, BrowserError::Conflict { .. }));
    }

    #[test]
    fn panel_stages_previews_and_emits_without_side_effects() {
        let bundle = small_bundle();
        let actors = room();
        let snapshot = actors.clone();
        let mut panel = BrowserPanel::new();
        assert!(panel.is_empty());
        panel.rebase(4);
        assert_eq!(panel.base_revision(), 4);
        panel.stage(&bundle, "brazier", board_intent()).unwrap();
        let preview = panel.preview(&bundle, &actors).unwrap();
        assert_eq!(preview.resulting_revision, 5);
        assert_eq!(preview.effects.len(), 1);
        let intent = panel.emit(&bundle, &actors, 41, 4).unwrap();
        assert_eq!(intent.base_revision, 4);
        assert_eq!(intent.idempotency_key, 41);
        assert_eq!(intent.effects, preview.effects);
        assert_eq!(intent.id, preview.id);
        assert_eq!(actors, snapshot, "panel preview and emit touch nothing");
        // Same staged drop re-emits the same intent shape.
        assert_eq!(panel.emit(&bundle, &actors, 41, 4).unwrap(), intent);
        assert!(panel.clear());
        assert!(panel.is_empty());
    }

    #[test]
    fn stale_drops_and_empty_panels_refuse() {
        let bundle = small_bundle();
        let actors = room();
        let mut panel = BrowserPanel::new();
        panel.stage(&bundle, "brazier", board_intent()).unwrap();
        assert_eq!(
            panel.emit(&bundle, &actors, 1, 2).unwrap_err(),
            BrowserError::StaleRevision {
                expected: 2,
                got: 0
            }
        );
        assert_eq!(
            emit_drop(&bundle, "brazier", &board_intent(), &actors, 0, 1, 2).unwrap_err(),
            BrowserError::StaleRevision {
                expected: 2,
                got: 0
            }
        );
        let empty = BrowserPanel::new();
        let error = empty.preview(&bundle, &actors).unwrap_err();
        assert!(matches!(error, BrowserError::Conflict { .. }));
        assert!(empty.emit(&bundle, &actors, 1, 0).is_err());
        assert_eq!(
            panel.stage(&bundle, "missing", board_intent()).unwrap_err(),
            BrowserError::UnknownAsset {
                name: "missing".to_string()
            }
        );
        let mut too_many = board_intent();
        too_many.wants = vec![
            Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: None
            };
            MAX_WANTS_PER_DROP + 1
        ];
        assert_eq!(
            panel.stage(&bundle, "brazier", too_many).unwrap_err(),
            BrowserError::TooManyWants {
                count: MAX_WANTS_PER_DROP + 1,
                max: MAX_WANTS_PER_DROP
            }
        );
    }

    #[test]
    fn non_finite_actors_and_empty_rooms_refuse() {
        let bundle = small_bundle();
        let mut actors = room();
        actors[5] = BrowserActor::new(5, "brazier", f32::NAN, 0.5, 2.0, 1.0, 1.0, 1.0, 0.0);
        let error = preview_drop(&bundle, "brazier", &board_intent(), &actors, 0).unwrap_err();
        assert!(matches!(error, BrowserError::BadActor { .. }));
        assert!(error.to_string().contains('5'));
        assert_eq!(
            preview_drop(&bundle, "brazier", &board_intent(), &[], 0).unwrap_err(),
            BrowserError::NoBounds
        );
        assert_eq!(
            BrowserError::NoBounds.to_string(),
            "no-bounds: no valid actor anchors the room"
        );
    }

    #[test]
    fn error_text_quotes_names_relations_and_bounds() {
        let taken = BrowserError::DuplicateName {
            name: "crate".to_string(),
        };
        assert!(taken.to_string().contains("crate"));
        let unknown = BrowserError::UnknownAsset {
            name: "hero".to_string(),
        };
        assert!(unknown.to_string().contains("missing it"));
        let conflict = BrowserError::Conflict {
            relation: "next_to",
            reason: "no placement".to_string(),
            conflicts: vec!["next_to wall:north dist 0..=50".to_string()],
        };
        assert!(conflict.to_string().contains("next_to"));
        let stale = BrowserError::StaleRevision {
            expected: 2,
            got: 0,
        };
        assert!(stale.to_string().contains('2'));
        let wants = BrowserError::TooManyWants { count: 9, max: 8 };
        assert!(wants.to_string().contains('9'));
    }
}

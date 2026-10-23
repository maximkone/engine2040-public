//! Asset browser: bundle search plus drag-to-place with preview.
//!
//! Owner: the editor session; reads bundle listings, never owns them.
//! Units: bundle entries, search hits, meters in, integer centimeters
//! inside the solver. Budget: full-bundle search <= 50 ms, panel logic
//! is UI-thread only and linear in entries. Determinism: same bundle
//! plus same query gives the same hits; same intent over the same
//! actors in any order gives the same placement or conflict.
//! Errors: [`BrowserError`]; refusals stage nothing and mutate nothing.
//! Vocabulary is the real thing, not a copy: `AssetKind` is the assets
//! archive/bundle entry kind (`engine_assets::ArchiveKind`, which AS1
//! `bundle::EntryKind` re-exports), and `Relation`/`Wall`/`Target`/`Want`/
//! `Intent`/`ObjectRef` are the `engine-spatial` placement vocabulary. The
//! drop solver is `engine_spatial::place::solve`; a refused drop carries its
//! `ConflictSet` as typed text. `DropPreview` follows the endpoint
//! `PreviewReceipt` discipline (dry-run effects plus resulting revision,
//! conflicts write nothing).

use std::fmt;

use engine_assets::archive::{
    ArchivedBundle, MAX_ARCHIVE_DETAIL_CHARS, MAX_ARCHIVE_ENTRIES, MAX_ARCHIVE_NAME_CHARS,
};
use engine_spatial::place::{solve, ConflictSet, TransformOut};
use engine_spatial::Actor;

use crate::commands::bound_to;

pub use engine_assets::ArchiveKind as AssetKind;
pub use engine_spatial::{Intent, ObjectRef, Relation, Target, Wall, Want};

/// Longest asset name in chars: the archive's own bound.
pub const MAX_NAME_CHARS: usize = MAX_ARCHIVE_NAME_CHARS;
/// Longest tag in chars.
pub const MAX_TAG_CHARS: usize = 64;
/// Most tags kept on one entry.
pub const MAX_TAGS_PER_ENTRY: usize = 8;
/// Most entries accepted in one bundle view: the archive's own bound.
pub const MAX_ENTRIES: usize = MAX_ARCHIVE_ENTRIES;
/// Longest search text or tag filter in chars.
pub const MAX_QUERY_CHARS: usize = 128;
/// Most hits returned by one search; the full bundle fits exactly.
pub const MAX_RESULTS: usize = MAX_ENTRIES;
/// Most relation wants accepted on one drop.
pub const MAX_WANTS_PER_DROP: usize = 8;
/// Most scene actors accepted around one drop.
pub const MAX_ACTORS_PER_SCENE: usize = 1024;
/// Bound for echoed text carried in errors: the archive's own bound.
pub const MAX_DETAIL_CHARS: usize = MAX_ARCHIVE_DETAIL_CHARS;
/// Largest point coordinate accepted in cm (+-1 km).
pub const MAX_POINT_CM: i32 = 100_000;

/// Every asset kind in wire order (the archive discriminants 1 to 5).
pub const ASSET_KINDS: [AssetKind; 5] = [
    AssetKind::Mesh,
    AssetKind::Material,
    AssetKind::Texture,
    AssetKind::Skin,
    AssetKind::Clip,
];

/// One-line source text for a want, in intent order (conflict sets, logs).
fn want_text(want: &Want) -> String {
    let target = match want.target {
        Target::Wall(wall) => format!("wall:{}", wall_name(wall)),
        Target::Object(id) => format!("object:{id}"),
        Target::Point { x_cm, z_cm } => format!("point:{x_cm},{z_cm}"),
    };
    let range = match want.distance_cm {
        Some((lo, hi)) => format!("{lo}..={hi}"),
        None => "default".to_string(),
    };
    format!("{} {target} dist {range}", want.relation.name())
}

/// Short wall name quoted in conflicts.
const fn wall_name(wall: Wall) -> &'static str {
    match wall {
        Wall::North => "north",
        Wall::South => "south",
        Wall::East => "east",
        Wall::West => "west",
    }
}

/// One scene actor snapshot the caller hands the drop: id, grammar-clean
/// name, meters, extents, yaw radians. The solver reads it as a spatial
/// actor ([`BrowserActor::to_actor`]).
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

    /// The solver's view of this actor: physical, still, no parent.
    fn to_actor(&self) -> Actor {
        Actor {
            id: self.id,
            name: self.name.clone(),
            x: self.x,
            y: self.y,
            z: self.z,
            sx: self.sx,
            sy: self.sy,
            sz: self.sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: self.yaw,
        }
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

    /// List every entry of a real archive (the AS1 bundle wrapped 1:1). The
    /// archive format already keeps names unique and bounded, so rows go in
    /// without the duplicate scan; it carries no tags, so rows list untagged.
    pub fn from_archive(archive: &ArchivedBundle) -> Result<Self, BrowserError> {
        let mut entries = Vec::with_capacity(archive.len());
        for item in archive.iter() {
            let entry = BundleEntry::new(item.kind, item.name, &[]);
            validate_entry(&entry)?;
            entries.push(entry);
        }
        Ok(Self { entries })
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

/// Placeholder a browser row shows when the cache holds no pixels for it.
pub const MISSING_THUMB: &str = "—";

/// Thumbnail state of one browser row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowThumb {
    /// The cache holds pixels for this asset; the panel draws them.
    Ready {
        /// Content digest of the pixels.
        digest: u64,
    },
    /// No pixels yet: the panel draws [`MISSING_THUMB`], never a fake image.
    Missing,
}

impl RowThumb {
    /// Text a text-only surface draws: the digest in hex, or the placeholder.
    pub fn text(self) -> String {
        match self {
            Self::Ready { digest } => format!("{digest:016x}"),
            Self::Missing => MISSING_THUMB.to_string(),
        }
    }
}

/// One browser panel row: the listed entry plus its thumbnail state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserRow {
    /// The listed asset.
    pub entry: BundleEntry,
    /// Thumbnail the cache holds for it at row build time.
    pub thumb: RowThumb,
}

/// Build the browser panel rows, one per listed entry in listing order.
///
/// Each row asks the thumbnail cache for the entry name, so a name with no
/// pixels is recorded in [`ThumbnailCache::missing`] and drawn as
/// [`MISSING_THUMB`]. Nothing is rendered here: the caller supplies the pixels.
pub fn rows_with_thumbs(
    view: &BundleView,
    cache: &mut crate::thumbnail::ThumbnailCache,
    now: u64,
) -> Vec<BrowserRow> {
    view.entries()
        .iter()
        .map(|entry| {
            let thumb = match cache.get(&entry.name, now) {
                Some(row) => RowThumb::Ready { digest: row.digest },
                None => RowThumb::Missing,
            };
            BrowserRow {
                entry: entry.clone(),
                thumb,
            }
        })
        .collect()
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
/// apply would publish. Follows the endpoint `PreviewReceipt`
/// discipline (effects plus resulting revision, conflicts write
/// nothing).
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
    let mut groups: Vec<(AssetKind, Vec<&BundleEntry>)> = ASSET_KINDS
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

/// Solve one drop with the real spatial solver. Shapes and scene are checked
/// first (loud, typed); an impossible drop returns the solver's minimal
/// conflict set as [`BrowserError::Conflict`]. Pure: reads only.
fn solve_drop(intent: &Intent, actors: &[BrowserActor]) -> Result<TransformOut, BrowserError> {
    validate_intent(intent)?;
    if actors.len() > MAX_ACTORS_PER_SCENE {
        return Err(BrowserError::BadActor {
            id: u32::MAX,
            reason: "scene exceeds the actor bound",
        });
    }
    // An empty scene has no walls to measure against; refuse before
    // solving so the panel names the missing room, not the object.
    if actors.is_empty() {
        return Err(BrowserError::NoBounds);
    }
    for actor in actors {
        validate_actor(actor).map_err(|_| BrowserError::BadActor {
            id: actor.id,
            reason: "actor lane must be finite with positive extents",
        })?;
    }
    let scene: Vec<Actor> = actors.iter().map(BrowserActor::to_actor).collect();
    match solve(intent, &scene) {
        Ok(mut placed) if !placed.is_empty() => Ok(placed.swap_remove(0)),
        Ok(_) => Err(BrowserError::NoBounds),
        Err(conflict) => Err(conflict_error(&conflict)),
    }
}

/// Map the solver's conflict set onto the panel's typed refusal. Lookup
/// failures keep their own name as the blocking relation; a search failure
/// names the first unmet want's relation.
fn conflict_error(conflict: &ConflictSet) -> BrowserError {
    if conflict.reason.starts_with("no-bounds") {
        return BrowserError::NoBounds;
    }
    let lookup = ["unknown-object", "ambiguous-object", "unknown-size"]
        .into_iter()
        .find(|name| conflict.reason.starts_with(name));
    let relation = match (lookup, conflict.unmet.first()) {
        (Some(name), _) => name,
        (None, Some(want)) => want.relation.name(),
        (None, None) => "overlap",
    };
    BrowserError::Conflict {
        relation,
        // The solver echoes object names verbatim; bound them like every
        // other echoed text.
        reason: bound_detail(&conflict.reason),
        conflicts: conflict.unmet.iter().map(want_text).collect(),
    }
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
    let placed = solve_drop(intent, actors)?;
    let (id, x, y, z, yaw) = (placed.id, placed.x, placed.y, placed.z, placed.yaw);
    Ok(DropPreview {
        asset: entry.name.clone(),
        id,
        x,
        y,
        z,
        yaw,
        effects: vec![effect_line(&entry.name, id, x, y, z, yaw)],
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
    use engine_assets::archive::ArchiveWriter;
    use std::time::Instant;

    fn entry(kind: AssetKind, name: &str, tags: &[&str]) -> BundleEntry {
        BundleEntry::new(kind, name, tags)
    }

    #[test]
    fn rows_carry_a_thumbnail_or_the_placeholder() {
        let mut bundle = BundleView::new();
        bundle
            .push(entry(AssetKind::Mesh, "crate", &["prop"]))
            .unwrap();
        bundle
            .push(entry(AssetKind::Mesh, "wall", &["prop"]))
            .unwrap();
        let mut cache = crate::thumbnail::ThumbnailCache::default();
        let pixels = vec![7u8; crate::thumbnail::THUMB_BYTES];
        let digest = cache.insert("crate", &pixels, 1).unwrap();

        let rows = rows_with_thumbs(&bundle, &mut cache, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entry.name, "crate");
        assert_eq!(rows[0].thumb, RowThumb::Ready { digest });
        assert_eq!(rows[0].thumb.text(), format!("{digest:016x}"));
        assert_eq!(rows[1].thumb, RowThumb::Missing);
        assert_eq!(rows[1].thumb.text(), MISSING_THUMB);
        assert_eq!(cache.missing(), vec!["wall"], "the miss is recorded once");
        // Two runs are identical: the row builder reads, never invents.
        assert_eq!(rows_with_thumbs(&bundle, &mut cache, 3), rows);
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
    fn kind_wire_bytes_match_the_real_archive() {
        // One entry per kind through the real writer and opener: the kind
        // byte survives the wire in order, and `ASSET_KINDS` is that order.
        let mut writer = ArchiveWriter::new();
        for (index, kind) in ASSET_KINDS.into_iter().enumerate() {
            assert_eq!(kind as usize, index + 1, "discriminants run 1 to 5");
            writer.stage(kind, kind.label(), &[index as u8]).unwrap();
        }
        let bytes = writer.finish();
        let archive = ArchivedBundle::open_slice(&bytes).unwrap();
        let wire: Vec<AssetKind> = archive.iter().map(|item| item.kind).collect();
        assert_eq!(wire, ASSET_KINDS);
        let view = BundleView::from_archive(&archive).unwrap();
        for kind in ASSET_KINDS {
            assert_eq!(view.find(kind.label()).map(|row| row.kind), Some(kind));
        }
    }

    #[test]
    fn real_archive_search_over_a_full_bundle_stays_under_50ms() {
        let mut writer = ArchiveWriter::new();
        for index in 0..MAX_ENTRIES {
            let kind = ASSET_KINDS[index % ASSET_KINDS.len()];
            writer
                .stage(kind, &format!("asset_{index:04}"), &[index as u8])
                .unwrap();
        }
        let bytes = writer.finish();
        let archive = ArchivedBundle::open_slice(&bytes).unwrap();
        let view = BundleView::from_archive(&archive).unwrap();
        assert_eq!(view.len(), MAX_ENTRIES);
        let mut worst = std::time::Duration::ZERO;
        for text in ["", "asset_3", "ASSET_40", "no such asset"] {
            let start = Instant::now();
            let hits = search(&view, &SearchQuery::new(text, None, None)).unwrap();
            worst = worst.max(start.elapsed());
            match text {
                "" => assert_eq!(hits.len(), MAX_ENTRIES),
                "asset_3" => assert_eq!(hits.len(), 1000),
                "ASSET_40" => assert_eq!(hits.len(), 96),
                _ => assert!(hits.is_empty()),
            }
        }
        eprintln!("ED14 real archive: {MAX_ENTRIES} entries, worst search {worst:?}");
        assert!(worst < std::time::Duration::from_millis(50), "{worst:?}");
        // Every kind lists under its own filter, in wire order.
        let meshes = search(&view, &SearchQuery::new("", Some(AssetKind::Mesh), None)).unwrap();
        assert_eq!(meshes.len(), MAX_ENTRIES.div_ceil(ASSET_KINDS.len()));
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
        assert_eq!(groups.len(), ASSET_KINDS.len());
        let kinds: Vec<AssetKind> = groups.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, ASSET_KINDS);
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
            let kind = ASSET_KINDS[index % ASSET_KINDS.len()];
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

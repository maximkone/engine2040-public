//! Multi-select set plus the object clipboard: copy, paste and duplicate.
//!
//! Owner: the editor session. The gizmo reads the set through
//! [`drag_set`](crate::gizmo::drag_set); the viewport marquee and the outliner
//! write it. This is a second selection shape beside
//! [`inspector::Selection`](crate::inspector::Selection): that one is a
//! display-ordered `u64` row list for the inspector grid, while this one is an
//! id-ordered [`ObjectId`] set, which is what a group drag and a copy need.
//! Units: meters/radians, engine-scene poses.
//! Determinism: the set is a [`BTreeSet`], so every walk is id-sorted; paste
//! takes its fresh ids from the caller, so no clock and no RNG is read here.
//! Errors: [`SelectError`], [`CopyError`], [`PasteError`]; a refusal stages
//! nothing and mutates nothing.

use std::collections::BTreeSet;

use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::inspector::MAX_SELECTION;

/// Suffix one pasted copy carries: `crate` becomes `crate copy`.
pub const COPY_SUFFIX: &str = " copy";

/// Selected scene objects: ordered by id, never a duplicate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    ids: BTreeSet<ObjectId>,
}

impl Selection {
    /// Empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of selected objects.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// True when `id` is selected.
    pub fn contains(&self, id: ObjectId) -> bool {
        self.ids.contains(&id)
    }

    /// Selected ids in ascending id order.
    pub fn ids(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.ids.iter().copied()
    }

    /// The selected ids as one ascending vector.
    pub fn to_vec(&self) -> Vec<ObjectId> {
        self.ids.iter().copied().collect()
    }

    /// Replace the selection with exactly `id` (a plain click).
    ///
    /// Recovers from nothing: a nil id is refused and the set stays as it was.
    pub fn select(&mut self, id: ObjectId) -> Result<(), SelectError> {
        if id.is_nil() {
            return Err(SelectError::NilId);
        }
        self.ids.clear();
        self.ids.insert(id);
        Ok(())
    }

    /// Add one object for a shift-click. True when it joined, false when it
    /// was already selected. A nil id and a set at [`MAX_SELECTION`] refuse
    /// without change.
    pub fn add(&mut self, id: ObjectId) -> Result<bool, SelectError> {
        if id.is_nil() {
            return Err(SelectError::NilId);
        }
        if self.ids.contains(&id) {
            return Ok(false);
        }
        if self.ids.len() >= MAX_SELECTION {
            return Err(SelectError::Full {
                len: self.ids.len(),
                max: MAX_SELECTION,
            });
        }
        self.ids.insert(id);
        Ok(true)
    }

    /// Remove one object. False when it was not selected.
    pub fn remove(&mut self, id: ObjectId) -> bool {
        self.ids.remove(&id)
    }

    /// Toggle one object: a selected id leaves, an absent id joins. True when
    /// it is selected afterwards.
    pub fn toggle(&mut self, id: ObjectId) -> Result<bool, SelectError> {
        if self.ids.contains(&id) {
            self.ids.remove(&id);
            return Ok(false);
        }
        self.add(id)?;
        Ok(true)
    }

    /// Drop every selected object.
    pub fn clear(&mut self) {
        self.ids.clear();
    }

    /// Replace the whole set. Duplicates collapse, the order is the id order.
    /// A nil id or more than [`MAX_SELECTION`] ids refuse the whole call, so
    /// the previous set survives an error.
    pub fn replace(&mut self, ids: impl IntoIterator<Item = ObjectId>) -> Result<(), SelectError> {
        let mut next = BTreeSet::new();
        for id in ids {
            if id.is_nil() {
                return Err(SelectError::NilId);
            }
            next.insert(id);
        }
        if next.len() > MAX_SELECTION {
            return Err(SelectError::Full {
                len: next.len(),
                max: MAX_SELECTION,
            });
        }
        self.ids = next;
        Ok(())
    }
}

/// What refused a selection change. Nothing changed on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectError {
    /// The nil id is not a selectable object.
    NilId,
    /// The selection already holds [`MAX_SELECTION`] objects.
    Full {
        /// Ids held now.
        len: usize,
        /// The bound.
        max: usize,
    },
}

impl std::fmt::Display for SelectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NilId => f.write_str("nil id is not a scene object; select a live scene id"),
            Self::Full { len, max } => write!(
                f,
                "selection holds {len} of {max} objects; remove one or press Escape to clear it"
            ),
        }
    }
}

impl std::error::Error for SelectError {}

/// One captured object: its source id, pose and display name at copy time.
///
/// The caller builds these rows from the live document; the clipboard never
/// holds scene handles.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEntry {
    /// Source object id.
    pub id: ObjectId,
    /// Source local pose (parent frame) in meters/radians.
    pub local: Transform,
    /// Source display name, when the caller has one.
    pub name: Option<String>,
}

impl ClipEntry {
    /// One capture row without validation.
    pub fn new(id: ObjectId, local: Transform, name: Option<&str>) -> Self {
        Self {
            id,
            local,
            name: name.map(str::to_string),
        }
    }
}

/// Captured objects in id order, ready to paste.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clipboard {
    entries: Vec<ClipEntry>,
}

impl Clipboard {
    /// Empty clipboard.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of captured objects.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is captured.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Captured rows in id order.
    pub fn entries(&self) -> &[ClipEntry] {
        &self.entries
    }

    /// Capture every selected object out of a caller-supplied row list.
    ///
    /// Rows land in id order whatever the input order was. A selected id with
    /// no row refuses the whole copy ([`CopyError::NoCapture`]).
    pub fn copy(selection: &Selection, captured: &[ClipEntry]) -> Result<Self, CopyError> {
        let mut entries = Vec::with_capacity(selection.len());
        for id in selection.ids() {
            let row = captured
                .iter()
                .find(|row| row.id == id)
                .ok_or(CopyError::NoCapture { id })?;
            entries.push(row.clone());
        }
        Ok(Self { entries })
    }

    /// Paste every captured object, shifted by `offset` meters, under the
    /// caller-minted fresh ids.
    ///
    /// `fresh_ids` holds one id per captured row in the same id order; the
    /// caller mints them (the viewport uses [`ObjectId::new`]) so this module
    /// reads no RNG. Pasted names go through [`copy_name`]. An empty clipboard
    /// is a typed refusal ([`PasteError::Empty`]), never a silent no-op.
    pub fn paste(
        &self,
        offset: [f32; 3],
        fresh_ids: &[ObjectId],
    ) -> Result<Vec<Pasted>, PasteError> {
        if self.entries.is_empty() {
            return Err(PasteError::Empty);
        }
        if !offset.iter().all(|axis| axis.is_finite()) {
            return Err(PasteError::NonFinite);
        }
        if fresh_ids.len() != self.entries.len() {
            return Err(PasteError::IdCount {
                expected: self.entries.len(),
                given: fresh_ids.len(),
            });
        }
        let mut seen = BTreeSet::new();
        for id in fresh_ids {
            if id.is_nil() {
                return Err(PasteError::NilId);
            }
            if !seen.insert(*id) {
                return Err(PasteError::DuplicateId { id: *id });
            }
        }
        let mut pasted = Vec::with_capacity(self.entries.len());
        for (row, id) in self.entries.iter().zip(fresh_ids.iter()) {
            let local = Transform::new(
                [
                    row.local.position[0] + offset[0],
                    row.local.position[1] + offset[1],
                    row.local.position[2] + offset[2],
                ],
                row.local.rotation,
                row.local.scale,
            );
            if !local.is_finite() {
                return Err(PasteError::NonFinite);
            }
            pasted.push(Pasted {
                source: row.id,
                id: *id,
                local,
                name: row.name.as_deref().map(copy_name),
            });
        }
        Ok(pasted)
    }
}

/// One pasted object: the source it came from plus its fresh id.
#[derive(Clone, Debug, PartialEq)]
pub struct Pasted {
    /// Object the copy came from.
    pub source: ObjectId,
    /// Fresh id the caller minted for the copy.
    pub id: ObjectId,
    /// Pose after the paste offset (meters/radians).
    pub local: Transform,
    /// Pasted display name under the [`copy_name`] suffix rule.
    pub name: Option<String>,
}

/// Duplicate a selection in one step: capture then paste.
///
/// Same rules as [`Clipboard::copy`] then [`Clipboard::paste`], including the
/// empty-selection refusal and the caller-minted ids.
pub fn duplicate(
    selection: &Selection,
    captured: &[ClipEntry],
    offset: [f32; 3],
    fresh_ids: &[ObjectId],
) -> Result<Vec<Pasted>, PasteError> {
    let clipboard = Clipboard::copy(selection, captured)
        .map_err(|CopyError::NoCapture { id }| PasteError::NoCapture { id })?;
    clipboard.paste(offset, fresh_ids)
}

/// Name a pasted copy: `crate` -> `crate copy`, `crate copy` ->
/// `crate copy 2`, `crate copy 2` -> `crate copy 3`.
///
/// An empty (or blank) name becomes `copy`, so a paste always carries a name.
pub fn copy_name(name: &str) -> String {
    let trimmed = name.trim_end();
    if trimmed.is_empty() {
        return "copy".to_string();
    }
    if let Some((base, generation)) = split_generation(trimmed) {
        return format!("{base}{COPY_SUFFIX} {}", generation.saturating_add(1));
    }
    if let Some(base) = trimmed.strip_suffix(COPY_SUFFIX) {
        let base = base.trim_end();
        if base.is_empty() {
            return "copy 2".to_string();
        }
        return format!("{base}{COPY_SUFFIX} 2");
    }
    format!("{trimmed}{COPY_SUFFIX}")
}

/// Split `name copy N` (N >= 2) into (`name`, N).
fn split_generation(name: &str) -> Option<(&str, u32)> {
    let (head, number) = name.rsplit_once(' ')?;
    let generation: u32 = number.parse().ok()?;
    if generation < 2 {
        return None;
    }
    let (base, word) = head.rsplit_once(' ')?;
    if word != COPY_SUFFIX.trim() {
        return None;
    }
    Some((base, generation))
}

/// What refused a copy. Nothing was captured on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyError {
    /// A selected id had no capture row.
    NoCapture {
        /// The selected id.
        id: ObjectId,
    },
}

impl std::fmt::Display for CopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCapture { id } => write!(
                f,
                "selected object {id} has no capture row; refresh the capture list from the live scene"
            ),
        }
    }
}

impl std::error::Error for CopyError {}

/// What refused a paste. Nothing was staged on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteError {
    /// The clipboard holds no object.
    Empty,
    /// The fresh id list does not match the clipboard length.
    IdCount {
        /// Clipboard rows.
        expected: usize,
        /// Ids given.
        given: usize,
    },
    /// A fresh id was nil.
    NilId,
    /// Two fresh ids repeated.
    DuplicateId {
        /// The repeated id.
        id: ObjectId,
    },
    /// The offset held NaN or infinity.
    NonFinite,
    /// A selected id had no capture row (`duplicate` only).
    NoCapture {
        /// The selected id.
        id: ObjectId,
    },
}

impl std::fmt::Display for PasteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str(
                "clipboard is empty; select an object and copy it (Ctrl+C) before pasting",
            ),
            Self::IdCount { expected, given } => write!(
                f,
                "paste needs {expected} fresh ids, got {given}; mint one id per clipboard row"
            ),
            Self::NilId => f.write_str("fresh paste id is nil; mint a real scene id for each copy"),
            Self::DuplicateId { id } => {
                write!(f, "fresh paste id {id} repeats; mint one unused id per copy")
            }
            Self::NonFinite => f.write_str("paste offset is not finite; pass finite meters"),
            Self::NoCapture { id } => write!(
                f,
                "selected object {id} has no capture row; refresh the capture list from the live scene"
            ),
        }
    }
}

impl std::error::Error for PasteError {}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::transform::Transform;

    fn id(n: u16) -> ObjectId {
        ObjectId::parse(&format!("00000000-0000-0000-0000-{n:012x}")).expect("valid hex id")
    }

    fn at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    #[test]
    fn selection_is_ordered_unique_and_reversible() {
        let mut set = Selection::new();
        assert!(set.is_empty());
        assert!(set.add(id(3)).unwrap());
        assert!(set.add(id(1)).unwrap());
        assert!(!set.add(id(3)).unwrap(), "a repeat add is a no-op");
        assert_eq!(set.to_vec(), vec![id(1), id(3)]);
        assert!(set.contains(id(1)));
        assert!(!set.toggle(id(3)).unwrap(), "a selected id leaves");
        assert_eq!(set.to_vec(), vec![id(1)]);
        assert!(set.toggle(id(3)).unwrap(), "an absent id joins");
        assert_eq!(set.to_vec(), vec![id(1), id(3)]);
        assert!(set.remove(id(3)));
        assert!(set.remove(id(1)));
        assert!(!set.remove(id(1)));
        assert!(set.is_empty());

        set.replace([id(7), id(2), id(7)]).unwrap();
        assert_eq!(set.to_vec(), vec![id(2), id(7)]);
        set.select(id(5)).unwrap();
        assert_eq!(set.to_vec(), vec![id(5)]);
        set.clear();
        assert!(set.is_empty());

        assert_eq!(set.select(ObjectId::NIL), Err(SelectError::NilId));
        assert_eq!(set.add(ObjectId::NIL), Err(SelectError::NilId));
        assert_eq!(set.replace([id(1), ObjectId::NIL]), Err(SelectError::NilId));
        assert!(set.is_empty(), "a refused replace leaves the set alone");
    }

    #[test]
    fn selection_refuses_past_the_bound() {
        let mut set = Selection::new();
        let ids: Vec<ObjectId> = (1..=MAX_SELECTION)
            .map(|n| id(u16::try_from(n).unwrap()))
            .collect();
        set.replace(ids).unwrap();
        assert_eq!(set.len(), MAX_SELECTION);
        assert_eq!(
            set.add(id(4100)),
            Err(SelectError::Full {
                len: MAX_SELECTION,
                max: MAX_SELECTION
            })
        );
        assert!(!set.contains(id(4100)), "a refused add changes nothing");

        let too_many: Vec<ObjectId> = (1..=MAX_SELECTION + 1)
            .map(|n| id(u16::try_from(n).unwrap()))
            .collect();
        assert_eq!(
            set.replace(too_many),
            Err(SelectError::Full {
                len: MAX_SELECTION + 1,
                max: MAX_SELECTION
            })
        );
        assert_eq!(set.len(), MAX_SELECTION, "a refused replace keeps the set");
    }

    #[test]
    fn copy_paste_twice_gives_distinct_ids_and_offset_poses() {
        let mut set = Selection::new();
        set.replace([id(1), id(2)]).unwrap();
        let captured = vec![
            ClipEntry::new(id(2), at(20.0), Some("crate")),
            ClipEntry::new(id(1), at(10.0), Some("brazier")),
        ];
        let clipboard = Clipboard::copy(&set, &captured).unwrap();
        // Rows land in id order whatever the capture order was.
        assert_eq!(clipboard.entries()[0].id, id(1));
        assert_eq!(clipboard.entries()[1].id, id(2));

        let offset = [0.5, 0.0, -1.0];
        let first = clipboard.paste(offset, &[id(11), id(12)]).unwrap();
        let second = clipboard.paste(offset, &[id(21), id(22)]).unwrap();
        assert_eq!(first[0].local.position, [10.5, 0.0, -1.0]);
        assert_eq!(second[0].local.position, [10.5, 0.0, -1.0]);
        assert_eq!(first[1].local.position, [20.5, 0.0, -1.0]);
        assert_eq!(
            first[0].local.rotation, captured[1].local.rotation,
            "the local rotation is carried, not defaulted"
        );
        let ids: Vec<ObjectId> = first
            .iter()
            .chain(second.iter())
            .map(|row| row.id)
            .collect();
        assert_eq!(ids, vec![id(11), id(12), id(21), id(22)]);
        assert_eq!(first[0].name.as_deref(), Some("brazier copy"));
        assert_eq!(first[0].source, id(1));
    }

    #[test]
    fn copy_name_bumps_one_generation() {
        assert_eq!(copy_name("crate"), "crate copy");
        assert_eq!(copy_name("crate copy"), "crate copy 2");
        assert_eq!(copy_name("crate copy 2"), "crate copy 3");
        assert_eq!(copy_name("crate copy 9"), "crate copy 10");
        assert_eq!(copy_name("   "), "copy");
        assert_eq!(copy_name("wall 2"), "wall 2 copy");
        assert_eq!(copy_name("crate copy copy"), "crate copy copy 2");
    }

    #[test]
    fn empty_clipboard_paste_is_a_typed_error_with_recovery() {
        let empty = Clipboard::new();
        assert!(empty.is_empty());
        let error = empty.paste([1.0, 0.0, 0.0], &[]).unwrap_err();
        assert_eq!(error, PasteError::Empty);
        let text = error.to_string();
        assert!(text.contains("Ctrl+C"), "{text}");
        assert_eq!(
            empty.paste([1.0, 0.0, 0.0], &[id(9)]),
            Err(PasteError::Empty)
        );
    }

    #[test]
    fn paste_refuses_bad_ids_and_offsets() {
        let mut set = Selection::new();
        set.add(id(1)).unwrap();
        let captured = [ClipEntry::new(id(1), at(1.0), None)];
        let clipboard = Clipboard::copy(&set, &captured).unwrap();
        assert_eq!(
            clipboard.paste([0.0; 3], &[]),
            Err(PasteError::IdCount {
                expected: 1,
                given: 0
            })
        );
        assert_eq!(
            clipboard.paste([0.0; 3], &[ObjectId::NIL]),
            Err(PasteError::NilId)
        );
        assert_eq!(
            clipboard.paste([f32::NAN, 0.0, 0.0], &[id(2)]),
            Err(PasteError::NonFinite)
        );
        // A nameless capture stays nameless; no suffix is invented.
        assert_eq!(clipboard.paste([0.0; 3], &[id(2)]).unwrap()[0].name, None);
    }

    #[test]
    fn duplicate_is_copy_then_paste_and_names_the_costs() {
        let mut set = Selection::new();
        set.replace([id(4), id(6)]).unwrap();
        let captured = [
            ClipEntry::new(id(4), at(4.0), Some("wall")),
            ClipEntry::new(id(6), at(6.0), Some("gate")),
        ];
        let rows = duplicate(&set, &captured, [0.0, 0.0, 2.0], &[id(40), id(60)]).unwrap();
        assert_eq!(rows[0].local.position, [4.0, 0.0, 2.0]);
        assert_eq!(rows[1].name.as_deref(), Some("gate copy"));

        // A stale selection names the id it could not capture.
        let stale = [ClipEntry::new(id(4), at(4.0), None)];
        assert_eq!(
            duplicate(&set, &stale, [0.0; 3], &[id(40), id(60)]),
            Err(PasteError::NoCapture { id: id(6) })
        );
        let none = Selection::new();
        assert_eq!(
            duplicate(&none, &captured, [0.0; 3], &[]),
            Err(PasteError::Empty)
        );
    }
}

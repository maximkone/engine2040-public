//! Painted bond authoring: the per-bond strength table, its ops and the bake
//! that hands it to the destruction owner.
//!
//! The editor never owns chunk geometry. A [`BondTable`] records only intent:
//! one [`BondStrength`] per node and canonical chunk pair (`a < b`). The
//! destruction crate bakes the chunks and bonds; [`bake_bonds`] scales each
//! named bond's `breaking_strain` by the painted permille. `1000` keeps the
//! baked strain exactly and lower values weaken the bond: no paint ever
//! strengthens a bond past what its bake chose.
//!
//! Keys are node ids, so a remove drops the node's paint with its bindings and
//! components, and undo restores it exactly.

use std::collections::BTreeMap;

use engine_destruction::bonded::{BondedDesc, MAX_BONDED_CHUNKS};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Hierarchy;

use super::{BatchError, Op};

/// Weakest accepted bond strength, in permille of the baked strain.
pub const MIN_BOND_PERMILLE: u16 = 1;
/// Strongest accepted bond strength: the baked strain itself, never more.
pub const MAX_BOND_PERMILLE: u16 = 1000;

/// Painted bond strength: permille of the baked breaking strain, `1..=1000`.
///
/// A value below `1000` weakens the bond, so a strike that holds a baked bond
/// can snap a painted one. Out-of-range input is refused by [`BondStrength::new`]
/// and never constructed, so every op, journal frame and save entry carries a
/// strength the bake can apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BondStrength(u16);

impl BondStrength {
    /// New strength from permille text or wire value; `1..=1000` only.
    pub fn new(permille: u32) -> Result<Self, BondError> {
        let value = u16::try_from(permille)
            .ok()
            .filter(|value| (MIN_BOND_PERMILLE..=MAX_BOND_PERMILLE).contains(value))
            .ok_or(BondError::StrengthOutOfRange { got: permille })?;
        Ok(Self(value))
    }

    /// The strength as written: permille of the baked strain.
    pub fn permille(self) -> u16 {
        self.0
    }

    /// Baked strain under this strength. `1000` returns the baked value
    /// bit-exactly; every other value scales it down by an IEEE division, so
    /// one strength bakes the same bytes on every PC.
    pub fn scale(self, baked: f32) -> f32 {
        if self.0 == MAX_BOND_PERMILLE {
            baked
        } else {
            baked * (f32::from(self.0) / f32::from(MAX_BOND_PERMILLE))
        }
    }
}

/// Canonical chunk pair: `a < b`, two different chunks, both under
/// [`MAX_BONDED_CHUNKS`]. One pair names one bond on every platform.
pub type BondPair = (u32, u32);

/// Painted bonds: per node, per canonical chunk pair, its strength.
///
/// `BTreeMap` keeps both levels in id then pair order, so the save bytes and
/// the digest are stable without a sort at the call site.
pub type BondTable = BTreeMap<ObjectId, BTreeMap<BondPair, BondStrength>>;

/// Why a bond paint or a bake was refused. Every variant names the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BondError {
    /// Strength outside `1..=1000` permille.
    StrengthOutOfRange {
        /// The refused value, in permille.
        got: u32,
    },
    /// A pair named the same chunk twice.
    SameChunk {
        /// The repeated chunk index.
        chunk: u32,
    },
    /// A chunk index at or past [`MAX_BONDED_CHUNKS`].
    ChunkOutOfRange {
        /// The refused chunk index.
        chunk: u32,
    },
    /// The bake holds no bond for a painted pair, so the paint would vanish.
    NoSuchBond {
        /// Lower chunk index of the painted pair.
        a: u32,
        /// Higher chunk index of the painted pair.
        b: u32,
    },
    /// Save bytes or a journal frame held a bond entry the codec refuses.
    BadBytes,
}

impl core::fmt::Display for BondError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::StrengthOutOfRange { got } => write!(
                f,
                "bond strength {got} is outside {MIN_BOND_PERMILLE}..={MAX_BOND_PERMILLE} permille"
            ),
            Self::SameChunk { chunk } => {
                write!(f, "bond needs two different chunks, got {chunk} twice")
            }
            Self::ChunkOutOfRange { chunk } => write!(
                f,
                "bond chunk {chunk} is past the last baked chunk ({MAX_BONDED_CHUNKS})"
            ),
            Self::NoSuchBond { a, b } => {
                write!(f, "the bake holds no bond {a}-{b} to paint")
            }
            Self::BadBytes => f.write_str("bond table bytes are not valid"),
        }
    }
}

impl std::error::Error for BondError {}

/// Canonicalize one chunk pair for a paint.
pub fn bond_pair(a: u32, b: u32) -> Result<BondPair, BondError> {
    let limit = u32::try_from(MAX_BONDED_CHUNKS).unwrap_or(u32::MAX);
    for chunk in [a, b] {
        if chunk >= limit {
            return Err(BondError::ChunkOutOfRange { chunk });
        }
    }
    if a == b {
        return Err(BondError::SameChunk { chunk: a });
    }
    Ok((a.min(b), a.max(b)))
}

/// Stage one bond paint: store `strength` for the node's pair, or clear it.
///
/// The inverse is the same op with the previous value (or `None`), so undo
/// writes back exactly the entry the paint replaced. Refused paints leave the
/// table untouched.
pub(crate) fn stage_set_bond(
    tree: &Hierarchy,
    table: &mut BondTable,
    id: ObjectId,
    a: u32,
    b: u32,
    strength: Option<BondStrength>,
) -> Result<(Vec<Op>, String), BatchError> {
    if id.is_nil() {
        return Err(BatchError::NilId);
    }
    if !tree.contains(id) {
        return Err(BatchError::UnknownNode { id });
    }
    let pair = bond_pair(a, b).map_err(|error| BatchError::BadBond { id, error })?;
    let previous = match strength {
        Some(value) => table.entry(id).or_default().insert(pair, value),
        None => {
            let old = table.get_mut(&id).and_then(|own| own.remove(&pair));
            if table.get(&id).is_some_and(|own| own.is_empty()) {
                table.remove(&id);
            }
            old
        }
    };
    let (a, b) = pair;
    let line = match (previous, strength) {
        (Some(from), Some(to)) => format!(
            "set-bond {id} {a}-{b} from {} to {} permille",
            from.permille(),
            to.permille()
        ),
        (None, Some(to)) => format!("set-bond {id} {a}-{b} to {} permille", to.permille()),
        (Some(from), None) => format!(
            "set-bond {id} {a}-{b} cleared (was {} permille)",
            from.permille()
        ),
        (None, None) => format!("set-bond {id} {a}-{b} already clear"),
    };
    let inverse = Op::SetBond {
        id,
        a,
        b,
        strength: previous,
    };
    Ok((vec![inverse], line))
}

/// Drop a removed node's paint and return the ops that restore it, in pair
/// order, for the remove inverse.
pub(crate) fn take_bonds(table: &mut BondTable, id: ObjectId) -> Vec<Op> {
    let Some(own) = table.remove(&id) else {
        return Vec::new();
    };
    own.into_iter()
        .map(|((a, b), strength)| Op::SetBond {
            id,
            a,
            b,
            strength: Some(strength),
        })
        .collect()
}

/// Bake one node's paint onto a destruction descriptor.
///
/// Each painted pair scales the baked `breaking_strain` of that bond; a
/// strength of `1000` keeps it exactly. A painted pair the descriptor does
/// not hold is refused, so a stale paint is reported instead of disappearing
/// from the bake.
pub fn bake_bonds(
    table: &BondTable,
    node: ObjectId,
    desc: &BondedDesc,
) -> Result<BondedDesc, BondError> {
    let mut baked = desc.clone();
    let Some(own) = table.get(&node) else {
        return Ok(baked);
    };
    for bond in &mut baked.bonds {
        let pair = (bond.a.min(bond.b), bond.a.max(bond.b));
        if let Some(strength) = own.get(&pair) {
            bond.breaking_strain = strength.scale(bond.breaking_strain);
        }
    }
    let held: Vec<BondPair> = baked
        .bonds
        .iter()
        .map(|bond| (bond.a.min(bond.b), bond.a.max(bond.b)))
        .collect();
    for &(a, b) in own.keys() {
        if !held.contains(&(a, b)) {
            return Err(BondError::NoSuchBond { a, b });
        }
    }
    Ok(baked)
}

/// Append the canonical bond section: total entry count, then per node id
/// (36-byte text, the save's id form) its pairs in `(a, b)` order as `u16`
/// chunk indices and `u16` permille.
///
/// A hand-built pair that cannot ride the `u16` field or is not canonical is
/// skipped like a broken node, so the written bytes always reopen.
pub fn push_bond_table(out: &mut Vec<u8>, table: &BondTable) {
    let entries: Vec<(ObjectId, u16, u16, u16)> = table
        .iter()
        .flat_map(|(id, own)| {
            own.iter()
                .map(move |(&pair, &strength)| (*id, pair, strength))
        })
        .filter_map(|(id, (a, b), strength)| {
            if a >= b {
                return None;
            }
            let a = u16::try_from(a).ok()?;
            let b = u16::try_from(b).ok()?;
            Some((id, a, b, strength.permille()))
        })
        .collect();
    let count = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for (id, a, b, permille) in entries {
        // The node section writes ids in the same canonical text.
        out.extend_from_slice(id.to_string().as_bytes());
        out.extend_from_slice(&a.to_le_bytes());
        out.extend_from_slice(&b.to_le_bytes());
        out.extend_from_slice(&permille.to_le_bytes());
    }
}

/// Read one bond entry's fields at `pos`: canonical pair and strength.
///
/// Every field is checked here, so a decoded [`BondTable`] holds only pairs a
/// bake could accept. The node id rides before these bytes and the save codec
/// reads it.
pub(crate) fn read_bond_fields(
    bytes: &[u8],
    pos: &mut usize,
) -> Result<(BondPair, BondStrength), BondError> {
    let a = read_u16(bytes, pos)?;
    let b = read_u16(bytes, pos)?;
    let strength = BondStrength::new(u32::from(read_u16(bytes, pos)?))?;
    let pair = bond_pair(u32::from(a), u32::from(b))?;
    Ok((pair, strength))
}

/// One little-endian `u16` at `pos`, advancing it.
fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, BondError> {
    let end = pos.checked_add(2).ok_or(BondError::BadBytes)?;
    let raw = bytes.get(*pos..end).ok_or(BondError::BadBytes)?;
    *pos = end;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

/// Bytes the pair and strength take in the save section (the node id rides
/// before them).
pub(crate) const BOND_FIELDS_BYTES: usize = 2 + 2 + 2;

#[cfg(test)]
mod tests {
    use engine_destruction::bonded::{Bond, BondedCollection, BondedDesc, Chunk3};
    use engine_scene::transform::Transform;

    use super::*;
    use crate::commands::{apply, undo, Batch, Document, Journal};
    use crate::save::{load_from_bytes, save_to_bytes};

    fn insert(doc: &mut Document, journal: &mut Journal, key: u64) -> ObjectId {
        let node = ObjectId::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id: node,
                parent: ObjectId::NIL,
                local: Transform::IDENTITY,
            }],
            base_revision: doc.revision,
            idempotency_key: key,
            provenance: None,
        };
        apply(doc, journal, &batch).expect("insert applies");
        node
    }

    fn paint(doc: &mut Document, journal: &mut Journal, node: ObjectId, key: u64) {
        let batch = Batch {
            ops: vec![Op::SetBond {
                id: node,
                a: 1,
                b: 0,
                strength: Some(BondStrength::new(500).unwrap()),
            }],
            base_revision: doc.revision,
            idempotency_key: key,
            provenance: None,
        };
        apply(doc, journal, &batch).expect("bond paint applies");
    }

    #[test]
    fn bond_strength_and_pair_refuse_by_name() {
        for refused in [0u32, 1001, u32::MAX] {
            let error = BondStrength::new(refused).expect_err("out of range");
            let text = error.to_string();
            assert!(text.contains("bond strength"), "{text}");
            assert!(text.contains("1..=1000 permille"), "{text}");
        }
        assert_eq!(BondStrength::new(1).unwrap().permille(), 1);
        assert_eq!(BondStrength::new(1000).unwrap().permille(), 1000);

        assert_eq!(bond_pair(4, 2), Ok((2, 4)));
        assert_eq!(bond_pair(7, 7), Err(BondError::SameChunk { chunk: 7 }));
        assert_eq!(
            bond_pair(1024, 0),
            Err(BondError::ChunkOutOfRange { chunk: 1024 })
        );
    }

    #[test]
    fn paint_undo_restores_the_exact_bond_table_bytes() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let node = insert(&mut doc, &mut journal, 1);
        let mut before = Vec::new();
        push_bond_table(&mut before, &doc.bonds);

        paint(&mut doc, &mut journal, node, 2);
        assert_eq!(
            doc.bonds.get(&node).and_then(|own| own.get(&(0, 1))),
            Some(&BondStrength::new(500).unwrap())
        );

        undo(&mut doc, &mut journal).expect("undo restores the paint");
        let mut after = Vec::new();
        push_bond_table(&mut after, &doc.bonds);
        assert_eq!(before, after, "undo must restore the table bytes");

        // Redo writes the paint (the pair arrived canonical already).
        crate::commands::redo(&mut doc, &mut journal).expect("redo");
        assert_eq!(
            doc.bonds.get(&node).and_then(|own| own.get(&(0, 1))),
            Some(&BondStrength::new(500).unwrap())
        );
    }

    #[test]
    fn bad_pair_is_refused_and_writes_nothing() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let node = insert(&mut doc, &mut journal, 1);
        let rev = doc.revision;
        let error = apply(
            &mut doc,
            &mut journal,
            &Batch {
                ops: vec![Op::SetBond {
                    id: node,
                    a: 3,
                    b: 3,
                    strength: Some(BondStrength::new(500).unwrap()),
                }],
                base_revision: rev,
                idempotency_key: 2,
                provenance: None,
            },
        )
        .expect_err("same chunk twice refuses");
        assert!(matches!(
            error,
            crate::commands::BatchError::BadBond {
                id,
                error: BondError::SameChunk { chunk: 3 },
            } if id == node
        ));
        assert_eq!(doc.revision, rev);
        assert!(doc.bonds.is_empty());
    }

    #[test]
    fn save_reload_bakes_the_same_bytes_twice() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let node = insert(&mut doc, &mut journal, 1);
        paint(&mut doc, &mut journal, node, 2);

        let reopened = load_from_bytes(&save_to_bytes(&doc)).expect("v5 save opens");
        assert_eq!(reopened.bonds, doc.bonds);

        let chunk = |x: f32| Chunk3 {
            x,
            y: 2.0,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        };
        let desc = BondedDesc {
            chunks: vec![chunk(0.0), chunk(1.0)],
            bonds: vec![Bond {
                a: 0,
                b: 1,
                breaking_strain: 1.0,
            }],
            anchors: vec![0],
            break_threshold: 1.0,
        };
        let baked = bake_bonds(&reopened.bonds, node, &desc).expect("paint bakes");
        assert_eq!(baked.bonds[0].breaking_strain, 0.5);
        let again = bake_bonds(&reopened.bonds, node, &desc).expect("paint bakes twice");
        assert_eq!(
            BondedCollection::new(&baked).unwrap().state_hash(),
            BondedCollection::new(&again).unwrap().state_hash()
        );
    }

    #[test]
    fn a_painted_pair_the_bake_lacks_is_refused() {
        let node = ObjectId::new();
        let mut table = BondTable::new();
        table
            .entry(node)
            .or_default()
            .insert((5, 9), BondStrength::new(500).unwrap());
        let desc = BondedDesc {
            chunks: vec![
                Chunk3 {
                    x: 0.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 1.0,
                },
                Chunk3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 1.0,
                },
            ],
            bonds: vec![Bond {
                a: 0,
                b: 1,
                breaking_strain: 1.0,
            }],
            anchors: vec![0],
            break_threshold: 1.0,
        };
        let error = bake_bonds(&table, node, &desc).expect_err("no such bond");
        assert_eq!(error, BondError::NoSuchBond { a: 5, b: 9 });
        assert!(error.to_string().contains("no bond 5-9"));
    }
}

//! DX7 fracture authoring: paint one bond, preview its strength and replay a
//! break, all over the command service so the edit undoes like any other.
//!
//! Consumer: the inspector fracture panel and AI batches; both call [`paint`]
//! (one `Op::SetBond` through [`apply`]) and read [`preview`] without touching
//! the document. Data owner is the document's [`crate::commands::BondTable`];
//! the destruction crate owns the bake and the replay. Units: permille of the baked strain for
//! paint, meters and fixed 60 Hz ticks for the replay. Errors: [`FractureError`]
//! names the refused field. Determinism: the preview bakes through
//! [`bake_bonds`] and steps the destruction owner's `BondedCollection`, so the
//! same table and descriptor replay the same events and state hash twice.

use engine_destruction::bonded::{BondedCollection, BondedDesc, BreakError, BreakEvent};
use engine_scene::ids::ObjectId;

use crate::commands::{
    apply, bake_bonds, bond_pair, Applied, Batch, BatchError, BondError, BondStrength, Document,
    Journal, Op, ProvenanceStamp,
};

/// Fixed replay step: one frame at 60 Hz, the editor's preview rate.
pub const PREVIEW_DT: f32 = 1.0 / 60.0;
/// Most ticks one break preview may step; longer replays are refused.
pub const MAX_PREVIEW_TICKS: u32 = 600;

/// One bond paint: the node, the chunk pair and the new strength (`None`
/// clears the paint). The pair is canonicalized before it reaches the batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BondPaint {
    /// Node carrying the painted bond.
    pub node: ObjectId,
    /// One chunk of the bond.
    pub a: u32,
    /// The other chunk of the bond.
    pub b: u32,
    /// New strength in permille, or `None` to clear the paint.
    pub permille: Option<u32>,
}

/// One hit for the break replay: chunk index, impulse and tick budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreviewHit {
    /// Struck chunk index in the descriptor.
    pub chunk: u32,
    /// Impulse in shard units; finite and non-negative.
    pub impulse: f32,
    /// Fixed steps to step after the hit, at most [`MAX_PREVIEW_TICKS`].
    pub ticks: u32,
}

/// One bond row of the strength preview.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrengthRow {
    /// Lower chunk index of the canonical pair.
    pub a: u32,
    /// Higher chunk index of the canonical pair.
    pub b: u32,
    /// True when the table paints this pair.
    pub painted: bool,
    /// Strain the bake chose before paint.
    pub baked_strain: f32,
    /// Strain the bake carries with the paint applied.
    pub preview_strain: f32,
}

/// One read-only break replay: the steps, the events and the final hash.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakReplay {
    /// Fixed steps stepped after the hit.
    pub ticks: u32,
    /// Events the replay emitted, in the destruction owner's order.
    pub events: Vec<BreakEvent>,
    /// `BondedCollection::state_hash` after the last step.
    pub state_hash: u64,
}

/// The whole read-only preview `preview` returns.
#[derive(Clone, Debug, PartialEq)]
pub struct FracturePreview {
    /// Node the preview baked.
    pub node: ObjectId,
    /// Every bond with its baked and painted strain, in descriptor order.
    pub strength: Vec<StrengthRow>,
    /// The break replay for the requested hit.
    pub replay: BreakReplay,
}

/// What refused a paint or a preview. Nothing is written on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FractureError {
    /// The paint aimed at a node the document does not carry.
    UnknownNode(ObjectId),
    /// The bond paint itself was refused (strength or pair).
    Bond {
        /// Node the paint aimed at.
        node: ObjectId,
        /// Why the paint was refused.
        error: BondError,
    },
    /// The command service refused the batch; nothing ran.
    Batch(BatchError),
    /// The destruction owner refused the bake or the replay.
    Break(BreakError),
    /// The replay asked for more than [`MAX_PREVIEW_TICKS`] steps.
    TooManyTicks {
        /// The refused tick count.
        got: u32,
    },
}

impl core::fmt::Display for FractureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(f, "fracture bond node {id} is not in the document"),
            Self::Bond { node, error } => write!(f, "bond paint on {node} refused: {error}"),
            Self::Batch(error) => write!(f, "bond paint batch refused: {error}"),
            Self::Break(error) => write!(f, "break preview refused: {error}"),
            Self::TooManyTicks { got } => write!(
                f,
                "break preview {got} ticks is past the {MAX_PREVIEW_TICKS} tick cap"
            ),
        }
    }
}

impl std::error::Error for FractureError {}

/// The batch one [`paint`] would apply, so a caller can preview or gate it
/// first (the AI path validates the same bytes it later applies).
///
/// The pair is canonicalized here and the strength is validated at the
/// boundary, so the returned batch holds only values the bake can apply.
pub fn paint_batch(
    doc: &Document,
    paint: BondPaint,
    idempotency_key: u64,
    stamp: ProvenanceStamp,
) -> Result<Batch, FractureError> {
    if !doc.hierarchy.contains(paint.node) {
        return Err(FractureError::UnknownNode(paint.node));
    }
    let strength = match paint.permille {
        Some(value) => Some(
            BondStrength::new(value).map_err(|error| FractureError::Bond {
                node: paint.node,
                error,
            })?,
        ),
        None => None,
    };
    let (a, b) = bond_pair(paint.a, paint.b).map_err(|error| FractureError::Bond {
        node: paint.node,
        error,
    })?;
    Ok(Batch {
        ops: vec![Op::SetBond {
            id: paint.node,
            a,
            b,
            strength,
        }],
        base_revision: doc.revision,
        idempotency_key,
        provenance: Some(stamp),
    })
}

/// Paint one bond through the command service: one batch, one revision, one
/// journal entry, so [`crate::commands::undo`] restores the exact table bytes.
pub fn paint(
    doc: &mut Document,
    journal: &mut Journal,
    paint: BondPaint,
    idempotency_key: u64,
    stamp: ProvenanceStamp,
) -> Result<Applied, FractureError> {
    let batch = paint_batch(doc, paint, idempotency_key, stamp)?;
    apply(doc, journal, &batch).map_err(FractureError::Batch)
}

/// Bake `node`'s paint onto `desc` and replay one hit without touching the
/// document: the strength rows, the events and the final state hash.
///
/// A painted pair the descriptor lacks is refused through [`bake_bonds`], so a
/// stale paint is reported instead of quietly leaving the preview.
pub fn preview(
    doc: &Document,
    node: ObjectId,
    desc: &BondedDesc,
    hit: PreviewHit,
) -> Result<FracturePreview, FractureError> {
    if !doc.hierarchy.contains(node) {
        return Err(FractureError::UnknownNode(node));
    }
    if hit.ticks > MAX_PREVIEW_TICKS {
        return Err(FractureError::TooManyTicks { got: hit.ticks });
    }
    if !hit.impulse.is_finite() || hit.impulse < 0.0 {
        return Err(FractureError::Break(BreakError::BadImpulse));
    }
    let baked =
        bake_bonds(&doc.bonds, node, desc).map_err(|error| FractureError::Bond { node, error })?;
    let strength = strength_rows(doc, node, desc, &baked);
    let mut set = BondedCollection::new(&baked).map_err(FractureError::Break)?;
    let chunk = usize::try_from(hit.chunk).unwrap_or(usize::MAX);
    set.apply_impact(chunk, hit.impulse, 0)
        .map_err(FractureError::Break)?;
    for _ in 0..hit.ticks {
        set.step(PREVIEW_DT);
    }
    let mut events = Vec::new();
    set.drain_into(&mut events);
    Ok(FracturePreview {
        node,
        strength,
        replay: BreakReplay {
            ticks: hit.ticks,
            events,
            state_hash: set.state_hash(),
        },
    })
}

/// Pair the unbaked descriptor with the baked clone row by row; both keep the
/// descriptor's order and length, so one zip is the whole table.
fn strength_rows(
    doc: &Document,
    node: ObjectId,
    desc: &BondedDesc,
    baked: &BondedDesc,
) -> Vec<StrengthRow> {
    let own = doc.bonds.get(&node);
    desc.bonds
        .iter()
        .zip(&baked.bonds)
        .map(|(raw, scaled)| {
            let pair = (raw.a.min(raw.b), raw.a.max(raw.b));
            StrengthRow {
                a: pair.0,
                b: pair.1,
                painted: own.is_some_and(|held| held.contains_key(&pair)),
                baked_strain: raw.breaking_strain,
                preview_strain: scaled.breaking_strain,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use engine_destruction::bonded::{Bond, Chunk3};
    use engine_scene::provenance::Source;
    use engine_scene::transform::Transform;

    use super::*;
    use crate::commands::{document_digest, undo};
    use crate::save::{load_from_bytes, save_to_bytes};

    fn fixed_node() -> ObjectId {
        ObjectId::parse("00000000-0000-4000-8000-0000000000aa").expect("fixed id parses")
    }

    fn fixture() -> (Document, Journal) {
        let node = fixed_node();
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let batch = Batch {
            ops: vec![Op::Insert {
                id: node,
                parent: ObjectId::NIL,
                local: Transform::IDENTITY,
            }],
            base_revision: 0,
            idempotency_key: 1,
            provenance: None,
        };
        apply(&mut doc, &mut journal, &batch).expect("insert applies");
        (doc, journal)
    }

    fn chunk(x: f32) -> Chunk3 {
        Chunk3 {
            x,
            y: 2.0,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        }
    }

    /// Two touching chunks, chunk 0 anchored; the gate sits below the baked
    /// strain, so a `1.0` impulse breaks a weakened bond and not a baked one.
    fn desc() -> BondedDesc {
        BondedDesc {
            chunks: vec![chunk(0.0), chunk(1.0)],
            bonds: vec![Bond {
                a: 0,
                b: 1,
                breaking_strain: 1.0,
            }],
            anchors: vec![0],
            break_threshold: 0.5,
        }
    }

    fn bond_bytes(doc: &Document) -> Vec<u8> {
        let mut out = Vec::new();
        crate::commands::fracture::push_bond_table(&mut out, &doc.bonds);
        out
    }

    fn model() -> ProvenanceStamp {
        ProvenanceStamp::model("mock", "fracture", [7u8; 32])
    }

    #[test]
    fn an_ai_paint_previews_without_moving_the_world_and_undoes_cleanly() {
        let (mut doc, mut journal) = fixture();
        let node = fixed_node();
        let before_bonds = bond_bytes(&doc);
        let before_save = save_to_bytes(&doc);
        let before_digest = document_digest(&doc);
        let hit = PreviewHit {
            chunk: 1,
            impulse: 1.0,
            ticks: 30,
        };

        // The unbaked descriptor never breaks at this impulse.
        let plain = preview(&doc, node, &desc(), hit).expect("plain preview");
        assert!(plain.replay.events.is_empty());
        assert_eq!(plain.strength[0].preview_strain, 1.0);

        let applied = paint(
            &mut doc,
            &mut journal,
            BondPaint {
                node,
                a: 1,
                b: 0,
                permille: Some(500),
            },
            10,
            model(),
        )
        .expect("AI paint applies");
        assert_eq!(applied.revision, 2);
        assert!(matches!(applied.provenance.source, Source::Model { .. }));

        let seen = preview(&doc, node, &desc(), hit).expect("painted preview");
        assert_eq!(seen.strength[0].preview_strain, 0.5);
        assert!(seen.strength[0].painted);
        assert!(seen
            .replay
            .events
            .iter()
            .any(|event| matches!(event, BreakEvent::BondBroken { bond: 0, .. })));

        // The preview is a pure read: both the bond bytes and the save bytes
        // are the same before and after it.
        let painted_bonds = bond_bytes(&doc);
        let painted_save = save_to_bytes(&doc);
        assert!(preview(&doc, node, &desc(), hit).is_ok());
        assert_eq!(bond_bytes(&doc), painted_bonds);
        assert_eq!(save_to_bytes(&doc), painted_save);
        assert_ne!(painted_bonds, before_bonds);
        assert_ne!(painted_save, before_save);

        undo(&mut doc, &mut journal).expect("undo");
        assert_eq!(bond_bytes(&doc), before_bonds, "undo restores the bytes");
        assert_eq!(
            document_digest(&doc),
            before_digest,
            "undo restores content"
        );
    }

    #[test]
    fn the_same_paint_sequence_bakes_identical_bytes_twice() {
        let mut hashes = Vec::new();
        for _ in 0..2 {
            let (mut doc, mut journal) = fixture();
            let node = fixed_node();
            for (key, permille) in [
                (10u64, Some(900u32)),
                (11, Some(400)),
                (12, None),
                (13, Some(650)),
            ] {
                paint(
                    &mut doc,
                    &mut journal,
                    BondPaint {
                        node,
                        a: 0,
                        b: 1,
                        permille,
                    },
                    key,
                    model(),
                )
                .expect("paint applies");
            }
            let reopened = load_from_bytes(&save_to_bytes(&doc)).expect("v5 save opens");
            let baked = bake_bonds(&reopened.bonds, node, &desc()).expect("bake");
            assert_eq!(baked.bonds[0].breaking_strain, 0.65);
            hashes.push(
                BondedCollection::new(&baked)
                    .expect("collection")
                    .state_hash(),
            );
            hashes.push(bond_bytes(&reopened).len() as u64);
        }
        assert_eq!(hashes[0], hashes[2]);
        assert_eq!(hashes[1], hashes[3]);
    }

    #[test]
    fn an_out_of_range_paint_is_refused_by_name() {
        let (mut doc, mut journal) = fixture();
        let rev = doc.revision;
        for refused in [0u32, 1001u32] {
            let error = paint(
                &mut doc,
                &mut journal,
                BondPaint {
                    node: fixed_node(),
                    a: 0,
                    b: 1,
                    permille: Some(refused),
                },
                10,
                model(),
            )
            .expect_err("out of range refuses");
            let text = error.to_string();
            assert!(text.contains("bond strength"), "{text}");
            assert!(text.contains("1..=1000 permille"), "{text}");
        }
        // A bad pair is refused by its own name too.
        let error = paint(
            &mut doc,
            &mut journal,
            BondPaint {
                node: fixed_node(),
                a: 3,
                b: 3,
                permille: Some(500),
            },
            10,
            model(),
        )
        .expect_err("same chunk refuses");
        assert!(error.to_string().contains("different chunks"));
        assert_eq!(doc.revision, rev);
        assert!(doc.bonds.is_empty());
    }

    #[test]
    fn a_replay_past_the_cap_is_refused() {
        let (doc, _journal) = fixture();
        let error = preview(
            &doc,
            fixed_node(),
            &desc(),
            PreviewHit {
                chunk: 0,
                impulse: 0.0,
                ticks: MAX_PREVIEW_TICKS + 1,
            },
        )
        .expect_err("tick cap refuses");
        assert!(error.to_string().contains("tick cap"));
    }
}

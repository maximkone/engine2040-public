//! nav_dirty: per-event dirty-set records for the nav tile handoff.
//! Owner: destruction crate; the arena maps freed chunks to nav tiles.
//! Units: chunk ids in, sorted records out; ticks at 60 Hz.
//! Determinism: index-order folds, ascending deduped ids, integer math
//! only; shuffled event order records identically after the sort.
//! Errors: typed DirtySetError; bad batches refuse, never half-record.
//! Budgets: building allocates O(freed chunks) and runs only on ticks
//! that free chunks; quiet ticks record nothing and allocate nothing.

use crate::bonded::{BreakEvent, MAX_BONDED_CHUNKS};

/// Typed dirty-set failures; bad batches refuse, never half-record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirtySetError {
    /// No chunk freed in this batch, so no tile dirties.
    EmptySet,
    /// More chunk ids than one record accepts.
    TooManyChunks {
        /// Chunk count offered.
        got: usize,
        /// Record bound refused past.
        max: usize,
    },
    /// Chunk id past the collection.
    BadChunk {
        /// Chunk id offered.
        chunk: u32,
    },
}

impl core::fmt::Display for DirtySetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DirtySetError::EmptySet => write!(f, "dirty set names no chunks"),
            DirtySetError::TooManyChunks { got, max } => {
                write!(f, "dirty set holds {got} chunks over bound {max}")
            }
            DirtySetError::BadChunk { chunk } => {
                write!(f, "dirty set chunk {chunk} past the collection")
            }
        }
    }
}

impl std::error::Error for DirtySetError {}

/// Canonical per-event dirty set: the chunks one break batch freed, in
/// ascending order without duplicates.
///
/// The arena drains [`BreakEvent`] rows, folds them here, then maps each
/// chunk through its own chunk-to-tile table into the nav break set and
/// invalidates exactly those tiles. Bond snaps ride along for the server
/// log but never dirty tiles on their own: only freed chunks map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirtySetRecord {
    tick: u32,
    chunks: Vec<u32>,
}

impl DirtySetRecord {
    /// Builds a record from raw freed-chunk ids in any order.
    ///
    /// Sorts and dedups so shuffled drains record identically. The caller
    /// passes the collection length as `chunk_count`.
    /// Errors: [`DirtySetError::EmptySet`],
    /// [`DirtySetError::TooManyChunks`], [`DirtySetError::BadChunk`].
    pub fn from_chunks(
        tick: u32,
        chunks: &[u32],
        chunk_count: usize,
    ) -> Result<Self, DirtySetError> {
        if chunks.is_empty() {
            return Err(DirtySetError::EmptySet);
        }
        if chunks.len() > MAX_BONDED_CHUNKS {
            return Err(DirtySetError::TooManyChunks {
                got: chunks.len(),
                max: MAX_BONDED_CHUNKS,
            });
        }
        for chunk in chunks {
            if (*chunk as usize) >= chunk_count {
                return Err(DirtySetError::BadChunk { chunk: *chunk });
            }
        }
        let mut sorted = chunks.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        Ok(Self {
            tick,
            chunks: sorted,
        })
    }

    /// Folds one drained event batch into a record.
    ///
    /// Collects every [`BreakEvent::ChunkFreed`] chunk in emission order,
    /// then records canonically like [`DirtySetRecord::from_chunks`].
    /// [`BreakEvent::BondBroken`] rows never dirty tiles and are skipped.
    /// A batch that frees nothing (or an empty batch) refuses with
    /// [`DirtySetError::EmptySet`]; the caller then skips invalidation.
    /// Errors: [`DirtySetError::EmptySet`],
    /// [`DirtySetError::TooManyChunks`], [`DirtySetError::BadChunk`].
    pub fn from_events(
        tick: u32,
        events: &[BreakEvent],
        chunk_count: usize,
    ) -> Result<Self, DirtySetError> {
        let mut chunks = Vec::new();
        for event in events {
            if let BreakEvent::ChunkFreed { chunk, .. } = event {
                chunks.push(*chunk);
            }
        }
        Self::from_chunks(tick, &chunks, chunk_count)
    }

    /// Match tick the batch drained on.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Freed chunk ids in ascending order without duplicates.
    pub fn chunks(&self) -> &[u32] {
        &self.chunks
    }

    /// Freed chunk count.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// True when the record names no chunks; always false by
    /// construction, since empty batches refuse in `from_chunks`.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Canonical bytes: tick plus count plus ids, all little-endian.
    /// Allocates the blob.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 4 * self.chunks.len());
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&(self.chunks.len() as u32).to_le_bytes());
        for chunk in &self.chunks {
            out.extend_from_slice(&chunk.to_le_bytes());
        }
        out
    }

    /// Stable `u64` over the canonical bytes (replay gate).
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(&self.to_bytes())
    }
}

/// Typed link-recook failures; bad batches refuse, never half-name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecookError {
    /// The chunk-to-tile map refused (empty set, bad tile, too many).
    Tiles(engine_nav::tiles::TileError),
    /// The link registry refused (too many links, bad edge lane).
    Links(engine_nav::links::LinkError),
}

impl core::fmt::Display for RecookError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecookError::Tiles(_) => write!(f, "recook names no tiles"),
            RecookError::Links(_) => write!(f, "recook names no links"),
        }
    }
}

impl std::error::Error for RecookError {}

/// Names the traversal links one break batch must recook.
///
/// District-cooker glue: the freed-chunk record lands here with the
/// arena's chunk-to-tile map plus the district link registry, and the nav
/// cooker entry names back exactly the links sitting on dirty tiles,
/// ascending by id. Cook-time only, never on the tick. Pure: the same
/// record plus the same registry names the same ids twice alike.
/// Errors: [`RecookError::Tiles`], [`RecookError::Links`].
pub fn links_to_recook(
    record: &DirtySetRecord,
    tile_count: usize,
    tile_of: impl FnMut(u32) -> Option<engine_nav::tiles::TileId>,
    links: &[engine_nav::links::TraversalLink],
) -> Result<Vec<engine_nav::links::LinkId>, RecookError> {
    let set = engine_nav::tiles::BreakSet::from_chunks(
        record.tick(),
        record.chunks(),
        tile_count,
        tile_of,
    )
    .map_err(RecookError::Tiles)?;
    engine_nav::links::rebake_for_set(links, &set).map_err(RecookError::Links)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bonded::{Bond, BondedCollection, BondedDesc, Chunk3};

    fn sword_batch() -> (BondedCollection, Vec<BreakEvent>) {
        let mut field =
            BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, 7).expect("valid crate prop");
        let mut top = 0;
        for (i, c) in field.chunks().iter().enumerate() {
            if c.y > field.chunks()[top].y {
                top = i;
            }
        }
        let impulse =
            field.chunk_mass(top).expect("top chunk mass") * field.break_threshold() + 1.0;
        field.apply_impact(top, impulse, 3).expect("valid impact");
        for _ in 0..30 {
            field.step(1.0 / 60.0);
        }
        let events = field.drain_events();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, BreakEvent::ChunkFreed { .. })),
            "a sword strike must free chunks: {events:?}"
        );
        (field, events)
    }

    fn redundant_desc() -> BondedDesc {
        // Three chunks with a redundant bond: snapping one bond frees
        // nothing because every chunk keeps an anchor path.
        let chunks = vec![
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
            Chunk3 {
                x: 2.0,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 2.0,
            },
        ];
        BondedDesc {
            chunks,
            bonds: vec![
                Bond {
                    a: 0,
                    b: 1,
                    breaking_strain: 1.0,
                },
                Bond {
                    a: 1,
                    b: 2,
                    breaking_strain: 1.0,
                },
                Bond {
                    a: 0,
                    b: 2,
                    breaking_strain: 1.0,
                },
            ],
            anchors: vec![0],
            break_threshold: crate::bake::BREAKING_THRESHOLD,
        }
    }

    #[test]
    fn contract_break_batch_names_exactly_the_freed_chunks() {
        let (field, events) = sword_batch();
        let record = DirtySetRecord::from_events(3, &events, field.len()).expect("batch records");
        let mut want = field.freed_chunks();
        want.sort_unstable();
        want.dedup();
        assert_eq!(record.tick(), 3);
        assert_eq!(record.chunks(), &*want);
        assert_eq!(record.len(), want.len());
        // Anchored chunks never free, so they never name tiles.
        for anchor in field.anchors() {
            assert!(
                !record.chunks().contains(&anchor),
                "anchored chunk {anchor} must not dirty tiles"
            );
        }
        // Bond snaps alone name nothing: the record holds freed chunks only.
        let freed = events
            .iter()
            .filter(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
            .count();
        assert_eq!(record.len(), freed, "one entry per freed chunk");
    }

    #[test]
    fn contract_records_replay_identical_bytes_twice() {
        let (first_field, first_events) = sword_batch();
        let (second_field, second_events) = sword_batch();
        let first =
            DirtySetRecord::from_events(3, &first_events, first_field.len()).expect("records");
        let second =
            DirtySetRecord::from_events(3, &second_events, second_field.len()).expect("records");
        assert_eq!(first, second);
        assert_eq!(first.to_bytes(), second.to_bytes(), "bytes x2");
        assert_eq!(first.digest(), second.digest());
        // Shuffled event order records identically after the sort.
        let mut shuffled = first_events.clone();
        shuffled.reverse();
        let folded = DirtySetRecord::from_events(3, &shuffled, first_field.len()).expect("records");
        assert_eq!(folded, first, "order never leaks into the record");
        assert_eq!(folded.to_bytes(), first.to_bytes());
    }

    #[test]
    fn contract_recook_names_exactly_the_links_on_dirty_tiles() {
        use engine_nav::links::{LinkId, LinkKind, TraversalLink};
        use engine_nav::tiles::TileId;
        let record = DirtySetRecord::from_chunks(3, &[1, 0], 4).expect("records");
        let links = vec![
            TraversalLink {
                id: LinkId(9),
                kind: LinkKind::Vault,
                edge_m: [1.0, 0.0],
                tile: TileId(3),
            },
            TraversalLink {
                id: LinkId(2),
                kind: LinkKind::Hurdle,
                edge_m: [0.0, 0.0],
                tile: TileId(0),
            },
            TraversalLink {
                id: LinkId(5),
                kind: LinkKind::Mantle,
                edge_m: [2.0, 1.0],
                tile: TileId(1),
            },
        ];
        // Chunks map straight onto tiles, so freed chunks 0 and 1 dirty
        // tiles 0 and 1; the link on clean tile 3 recooks never.
        let named =
            links_to_recook(&record, 4, |chunk| Some(TileId(chunk)), &links).expect("recooks");
        assert_eq!(named, vec![LinkId(2), LinkId(5)]);
        // Twice alike: the same batch plus the same registry replays exactly.
        let again =
            links_to_recook(&record, 4, |chunk| Some(TileId(chunk)), &links).expect("recooks");
        assert_eq!(again, named);
    }

    #[test]
    fn contract_raw_chunks_sort_and_dedup() {
        let record = DirtySetRecord::from_chunks(9, &[5, 2, 5, 1], 8).expect("records");
        assert_eq!(record.chunks(), &[1, 2, 5]);
        assert_eq!(record.tick(), 9);
        assert_eq!(record.len(), 3);
    }

    #[test]
    fn contract_bond_snaps_alone_dirty_nothing() {
        let mut field = BondedCollection::new(&redundant_desc()).expect("valid desc");
        field.add_strain(0, 1.5, 9).expect("valid strain");
        assert!(field.is_bond_broken(0));
        assert!(
            field.freed_chunks().is_empty(),
            "redundant path holds every chunk"
        );
        let events = field.drain_events();
        assert_eq!(events.len(), 1, "only the snap logs: {events:?}");
        assert_eq!(
            DirtySetRecord::from_events(9, &events, field.len()),
            Err(DirtySetError::EmptySet),
            "a batch that frees nothing names no tiles"
        );
    }

    #[test]
    fn contract_bad_batches_refuse_without_a_record() {
        let (field, _) = sword_batch();
        assert_eq!(
            DirtySetRecord::from_events(3, &[], field.len()),
            Err(DirtySetError::EmptySet)
        );
        assert_eq!(
            DirtySetRecord::from_chunks(3, &[], field.len()),
            Err(DirtySetError::EmptySet)
        );
        assert_eq!(
            DirtySetRecord::from_chunks(3, &[field.len() as u32], field.len()),
            Err(DirtySetError::BadChunk {
                chunk: field.len() as u32
            })
        );
        let many = vec![0u32; MAX_BONDED_CHUNKS + 1];
        assert_eq!(
            DirtySetRecord::from_chunks(3, &many, MAX_BONDED_CHUNKS),
            Err(DirtySetError::TooManyChunks {
                got: MAX_BONDED_CHUNKS + 1,
                max: MAX_BONDED_CHUNKS
            })
        );
    }
}

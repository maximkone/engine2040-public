//! merkle: the net desync locator.
//! Owner: net crate. Units: entity indices (usize), leaf hashes (u64),
//! comparisons (usize). Determinism: integer-only, entity order, no clock
//! and no float; the same two states always give the same report. Errors:
//! none; two states that agree answer `None`. Budget: the root compare is
//! one comparison and the walk compares one subtree root per level, so a
//! desync is located in log2(n) + 1 comparisons instead of n entity
//! compares; the walk rebuilds the branch it descends into, so its work is
//! O(n) and it allocates one tree per level. Consumer: the desync report an
//! operator reads when two peers disagree about one tick. Diagnostic only:
//! the tree runs beside the net tick hashes, it never feeds one and it never
//! gates anything.
//!
//! The tree is the core digest ([`MerkleDigest`]), so the net crate owns no
//! second Merkle tree: the leaves are hashed through the core byte hash
//! ([`hash_bytes`]) and every subtree root the walk compares comes from the
//! core digest itself. The root is a different value on purpose: it never
//! stands in for a tick hash.

use engine_core::digest::MerkleDigest;
use engine_core::hash_bytes;

use crate::delta::{BodySnap, FullSnap};
use crate::session::SNAPSHOT_SLOT_LEN;

/// The leaf a padding slot carries: no entity, so no bytes.
const PAD_LEAF: u64 = 0;

/// The body wire bytes are one slot byte plus six words.
const _: () = assert!(SNAPSHOT_SLOT_LEN == 19);

/// Where two entity states first differ: the entity, and the comparisons the
/// walk took to find it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesyncReport {
    /// The first entity at which the two states differ, counted in the order
    /// the entities were given.
    pub entity: usize,
    /// The comparisons the tree walk took to find it.
    pub comparisons: usize,
}

/// The entity state of one net snapshot: one leaf per entity, plus the
/// Merkle digest over them.
///
/// The state is a diagnostic, never a gate: it runs beside the net tick
/// hashes and never feeds one, so nothing in it can move the golden hash.
/// The leaves are padded to the tree width, so every subtree range the walk
/// takes stays in bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityState {
    entities: usize,
    leaves: Vec<u64>,
    tree: MerkleDigest,
}

impl Default for EntityState {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityState {
    /// An empty state: no entities and one padding leaf.
    #[must_use]
    pub fn new() -> Self {
        Self::build(Vec::new())
    }

    /// Build from one leaf hash per entity, in entity order: the caller
    /// hashed the entity states itself.
    #[must_use]
    pub fn from_leaves(leaves: &[u64]) -> Self {
        Self::build(leaves.to_vec())
    }

    /// Build from one byte state per entity, in entity order: every entity
    /// state is hashed through the core byte hash.
    #[must_use]
    pub fn from_entities<I, B>(entities: I) -> Self
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
    {
        let leaves: Vec<u64> = entities
            .into_iter()
            .map(|entity| hash_bytes(entity.as_ref()))
            .collect();
        Self::build(leaves)
    }

    /// Build from a net snapshot: one leaf per body, in slot order. The leaf
    /// bytes are the body's wire bytes, so the locator sees exactly what the
    /// snapshot codec carries.
    #[must_use]
    pub fn from_snapshot(snap: &FullSnap) -> Self {
        Self::from_entities(snap.fighters.iter().map(body_bytes))
    }

    /// The number of entities the state carries.
    #[must_use]
    pub fn entities(&self) -> usize {
        self.entities
    }

    /// The summary of every entity: the root of the tree.
    #[must_use]
    pub fn root(&self) -> u64 {
        self.tree.root()
    }

    /// The leaf hash of one entity; `None` past the entity count.
    #[must_use]
    pub fn leaf(&self, entity: usize) -> Option<u64> {
        (entity < self.entities).then(|| self.leaves[entity])
    }

    /// Locate the first entity at which `peer` differs from this state.
    ///
    /// `None` when the two agree: the root compare settles that in one
    /// comparison. Otherwise the walk descends the tree, one comparison per
    /// level, and names the first entity that differs. The report carries
    /// the entity and the comparisons the walk took.
    #[must_use]
    pub fn locate(&self, peer: &Self) -> Option<DesyncReport> {
        let mut comparisons = 0usize;
        // The root first: one comparison says whether the states agree.
        comparisons += 1;
        let roots_match = self.tree.root() == peer.tree.root();
        if self.entities != peer.entities {
            // Different entity counts: the first difference is the first
            // entity only one side carries.
            return Some(DesyncReport {
                entity: self.entities.min(peer.entities),
                comparisons,
            });
        }
        if roots_match {
            return None;
        }
        let entity = self.descend(peer, &mut comparisons);
        Some(DesyncReport {
            entity,
            comparisons,
        })
    }

    /// The tree over the leaves, padded to a power of two.
    fn build(leaves: Vec<u64>) -> Self {
        let entities = leaves.len();
        let width = entities.max(1).next_power_of_two();
        let mut padded = leaves;
        padded.resize(width, PAD_LEAF);
        let tree = MerkleDigest::from_leaves(&padded);
        Self {
            entities,
            leaves: padded,
            tree,
        }
    }

    /// Walk down the tree to the first differing leaf: one comparison per
    /// level. Every range is a power of two, so the subtree root the walk
    /// compares is the tree's own node.
    fn descend(&self, peer: &Self, comparisons: &mut usize) -> usize {
        let mut lo = 0usize;
        let mut hi = self.tree.width();
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            let mine = subtree_root(&self.leaves[lo..mid]);
            let theirs = subtree_root(&peer.leaves[lo..mid]);
            *comparisons += 1;
            if mine == theirs {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

/// The root of an aligned subtree: the core digest over that leaf range.
fn subtree_root(leaves: &[u64]) -> u64 {
    MerkleDigest::from_leaves(leaves).root()
}

/// The wire bytes of one body: the same fields in the same order the
/// snapshot codec puts on the wire, so the leaf covers what the peer
/// received.
fn body_bytes(body: &BodySnap) -> [u8; SNAPSHOT_SLOT_LEN] {
    let mut bytes = [0u8; SNAPSHOT_SLOT_LEN];
    let mut at = 0usize;
    put_word(&mut bytes, &mut at, &[body.slot]);
    put_word(&mut bytes, &mut at, &body.x_mm.to_le_bytes());
    put_word(&mut bytes, &mut at, &body.y_mm.to_le_bytes());
    put_word(&mut bytes, &mut at, &body.z_mm.to_le_bytes());
    put_word(&mut bytes, &mut at, &body.yaw_ddeg.to_le_bytes());
    put_word(&mut bytes, &mut at, &body.hp.to_le_bytes());
    put_word(&mut bytes, &mut at, &body.stamina.to_le_bytes());
    bytes
}

/// Write one wire word into the body bytes and move the cursor.
fn put_word(bytes: &mut [u8; SNAPSHOT_SLOT_LEN], at: &mut usize, word: &[u8]) {
    let end = *at + word.len();
    bytes[*at..end].copy_from_slice(word);
    *at = end;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rollback::FrameLog;
    use crate::session::{encode_snapshot, SNAPSHOT_HEADER_LEN};
    use engine_core::diverge::first_divergence;

    /// A state of `count` entities, each with its own bytes.
    fn state_of(count: usize) -> EntityState {
        EntityState::from_entities((0..count).map(|index| index.to_le_bytes().to_vec()))
    }

    /// A state of `count` entities plus the entity leaves behind it: the
    /// tree pads them, so a padding slot is not an entity.
    fn state_with_leaves(count: usize) -> (EntityState, Vec<u64>) {
        let leaves: Vec<u64> = (0..count)
            .map(|index| hash_bytes(&index.to_le_bytes()))
            .collect();
        (EntityState::from_leaves(&leaves), leaves)
    }

    /// A snapshot of `count` bodies; every one of them sits `nudge` further
    /// out than the slot before it.
    fn snapshot(tick: u32, count: u8, nudge: i32) -> FullSnap {
        let bodies = (0..count)
            .map(|slot| BodySnap {
                slot,
                x_mm: i32::from(slot) * 100 + nudge,
                y_mm: 0,
                z_mm: 0,
                yaw_ddeg: 0,
                hp: 1_000,
                stamina: 100,
            })
            .collect();
        FullSnap::new(tick, bodies).expect("snapshot is valid")
    }

    #[test]
    fn two_states_that_differ_in_one_entity_are_located_at_that_entity_in_at_most_log2_n_comparisons(
    ) {
        const N: usize = 1024;
        let (local, local_leaves) = state_with_leaves(N);
        let mut moved = local_leaves.clone();
        moved[777] = moved[777].wrapping_add(1);
        let remote = EntityState::from_leaves(&moved);

        let report = local.locate(&remote).expect("the states differ");
        assert_eq!(report.entity, 777, "the differing entity is named");
        let bound = usize::try_from(N.ilog2()).expect("the depth fits") + 1;
        assert!(
            report.comparisons <= bound,
            "the walk took {} comparisons, over the log2(n) + 1 bound",
            report.comparisons
        );
        assert_eq!(
            report.comparisons, 11,
            "one comparison per level plus the root"
        );
        // The full compare finds the same entity: the walk agrees with it.
        assert_eq!(first_divergence(&local_leaves, &moved), Some(777));
        // The core digest walks to the same leaf.
        assert_eq!(local.tree.first_divergence(&remote.tree), Some(777));
        // The other side names the same entity.
        assert_eq!(
            remote.locate(&local).map(|report| report.entity),
            Some(777),
            "the walk is symmetric"
        );
    }

    #[test]
    fn two_equal_states_compare_equal_at_the_root() {
        let local = state_of(64);
        let remote = state_of(64);
        assert_eq!(local.root(), remote.root(), "equal states share the root");
        assert_eq!(local.locate(&remote), None, "equal states never diverge");
        assert_eq!(local.tree.first_divergence(&remote.tree), None);
        assert_eq!(local.entities(), remote.entities());
        assert_eq!(local.leaf(3), remote.leaf(3));
    }

    #[test]
    fn the_tree_never_enters_the_tick_hash() {
        let snap = snapshot(9, 4, 0);
        let mut peer = snapshot(9, 4, 0);
        peer.fighters[2].x_mm += 1;

        // The net tick's own digests: the snapshot codec digest and the
        // frame hash ring a peer compares.
        let codec = hash_bytes(&encode_snapshot(&snap).expect("snapshot encodes"));
        let mut log = FrameLog::new(8);
        log.save(codec, 0b1011);
        let tick_hash = log.fold();

        // The locator runs over the two snapshots.
        let local = EntityState::from_snapshot(&snap);
        let remote = EntityState::from_snapshot(&peer);
        let report = local.locate(&remote).expect("the snapshots differ");
        assert_eq!(report.entity, 2, "the moved body is named");

        // The tick hash is the same before and after: the tree never enters it.
        assert_eq!(log.fold(), tick_hash, "the tick hash did not move");
        assert_eq!(
            hash_bytes(&encode_snapshot(&snap).expect("snapshot encodes")),
            codec,
            "the snapshot codec digest did not move"
        );
        // The tree's root is a different value on purpose: it never stands
        // in for the tick hash.
        assert_ne!(local.root(), tick_hash);
        assert_ne!(local.root(), codec);
    }

    #[test]
    fn the_locator_is_diagnostic_only_and_changes_nothing() {
        let (local, local_leaves) = state_with_leaves(32);
        let mut moved = local_leaves.clone();
        moved[5] = moved[5].wrapping_add(1);
        let remote = EntityState::from_leaves(&moved);

        let local_before = local.clone();
        let remote_before = remote.clone();
        let report = local.locate(&remote).expect("the states differ");
        assert_eq!(report.entity, 5);
        // The inspected states are untouched: the locator only reads.
        assert_eq!(local, local_before, "the local state did not move");
        assert_eq!(remote, remote_before, "the peer state did not move");
        assert_eq!(local.leaves, local_before.leaves);
        assert_eq!(local.root(), local_before.root());
    }

    #[test]
    fn the_locator_agrees_with_a_full_compare() {
        for count in [1usize, 2, 3, 5, 8, 17, 64, 100] {
            let (local, local_leaves) = state_with_leaves(count);
            for at in [0, count / 2, count - 1] {
                let mut moved = local_leaves.clone();
                moved[at] = moved[at].wrapping_add(1);
                let remote = EntityState::from_leaves(&moved);
                let report = local.locate(&remote).expect("the states differ");
                assert_eq!(report.entity, at, "count {count}, entity {at}");
                assert_eq!(
                    first_divergence(&local_leaves, &moved),
                    Some(u32::try_from(at).expect("entity index fits")),
                    "count {count}, entity {at}: the full compare agrees"
                );
            }
        }
    }

    #[test]
    fn the_walk_cost_grows_with_the_tree_depth_not_the_entity_count() {
        // 64 entities: the walk takes log2(64) + 1 comparisons, not 64.
        let (local, local_leaves) = state_with_leaves(64);
        let mut moved = local_leaves.clone();
        moved[63] = moved[63].wrapping_add(1);
        let remote = EntityState::from_leaves(&moved);
        let report = local.locate(&remote).expect("the states differ");
        assert_eq!(report.entity, 63);
        assert_eq!(report.comparisons, 7, "log2(64) + 1 comparisons");
    }

    #[test]
    fn states_with_different_entity_counts_diverge_at_the_shared_end() {
        let local = state_of(10);
        let remote = state_of(12);
        let report = local.locate(&remote).expect("the counts differ");
        assert_eq!(report.entity, 10, "the first entity only one side carries");
        let other = remote.locate(&local).expect("the counts differ");
        assert_eq!(other.entity, 10, "the other side names the same entity");
    }

    #[test]
    fn an_empty_state_has_no_entities_and_one_leaf() {
        let empty = EntityState::new();
        assert_eq!(empty.entities(), 0);
        assert_eq!(empty.leaf(0), None);
        assert_eq!(empty.locate(&empty), None);
        assert_eq!(empty.tree.width(), 1);
        assert_eq!(EntityState::default().entities(), 0);
    }

    #[test]
    fn the_snapshot_leaf_bytes_are_the_wire_bytes() {
        let snap = snapshot(3, 2, 0);
        let wire = encode_snapshot(&snap).expect("snapshot encodes");
        let chunk = &wire[SNAPSHOT_HEADER_LEN..];
        for (index, body) in snap.fighters.iter().enumerate() {
            let at = index * SNAPSHOT_SLOT_LEN;
            assert_eq!(
                &body_bytes(body)[..],
                &chunk[at..at + SNAPSHOT_SLOT_LEN],
                "body {index}: the leaf bytes are the wire bytes"
            );
        }
    }
}

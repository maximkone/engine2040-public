//! snapshot: versioned canonical bytes + hash, restore. Slot order, FNV-1a64. Count is u32 LE.

use std::hash::Hasher;

use engine_core::digest::MerkleDigest;
pub use engine_core::hash_bytes;
use engine_core::Fnv64;

use crate::{Position, Velocity, World};

/// Snapshot format version. Bump only with hash-law proof.
pub const SNAPSHOT_VERSION: u16 = 1;

/// Canonical bytes: deterministic LE encoding every sim type provides.
pub trait CanonicalBytes {
    /// Append deterministic bytes to `out`.
    fn append_canonical(&self, out: &mut Vec<u8>);
    /// Feed the bytes `append_canonical` would emit into `state` without
    /// materializing them (steady state: no heap allocation). Foreign impls
    /// keep the materializing default; in-crate impls write through directly.
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        let mut out = Vec::new();
        self.append_canonical(&mut out);
        state.write(&out);
    }
    /// FNV-1a64 over canonical bytes, streamed with no heap allocation.
    /// Budget: O(slots) time, O(1) extra memory.
    fn canonical_hash(&self) -> u64 {
        let mut state = CanonicalHasher::new();
        self.append_hash(&mut state);
        state.finish()
    }
    /// FNV-1a64 over canonical bytes reusing a caller scratch buffer
    /// (no allocation once its capacity suffices). For hot loops that
    /// already hold one.
    fn canonical_hash_buf(&self, buf: &mut Vec<u8>) -> u64 {
        buf.clear();
        self.append_canonical(buf);
        hash_bytes(buf)
    }
}

/// Streaming FNV-1a64 sink: `Hasher` write-through over the canonical byte
/// stream, so [`CanonicalBytes::canonical_hash`] never builds a `Vec`.
/// No derived `Default`: the zero state is not the FNV offset basis.
#[derive(Clone, Debug)]
pub struct CanonicalHasher(Fnv64);

impl Default for CanonicalHasher {
    /// Defaults to the FNV offset basis, same as [`CanonicalHasher::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl CanonicalHasher {
    /// Fresh hasher at the FNV offset basis.
    pub fn new() -> Self {
        Self(Fnv64::new())
    }
}

impl Hasher for CanonicalHasher {
    fn write(&mut self, bytes: &[u8]) {
        self.0.write(bytes);
    }
    fn finish(&self) -> u64 {
        self.0.finish()
    }
}

/// Push `u16` LE.
fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Push `u32` LE.
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Push `f32` via raw bits LE (strict-IEEE, no fast-math).
fn push_f32(out: &mut Vec<u8>, v: f32) {
    push_u32(out, v.to_bits());
}

/// Stream an `(x, y)` pair of `f32` as raw bits LE: the layout `Position` and
/// `Velocity` share, matching `push_f32` byte for byte.
fn write_xy<H: Hasher>(state: &mut H, x: f32, y: f32) {
    state.write(&x.to_bits().to_le_bytes());
    state.write(&y.to_bits().to_le_bytes());
}

impl CanonicalBytes for Position {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        push_f32(out, self.x);
        push_f32(out, self.y);
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        write_xy(state, self.x, self.y);
    }
}

impl CanonicalBytes for Velocity {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        push_f32(out, self.x);
        push_f32(out, self.y);
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        write_xy(state, self.x, self.y);
    }
}

impl CanonicalBytes for World {
    fn append_canonical(&self, out: &mut Vec<u8>) {
        push_u16(out, SNAPSHOT_VERSION);
        push_u32(out, self.alive.len() as u32);
        for i in 0..self.alive.len() {
            if self.alive[i] {
                out.push(1);
                self.pos[i].append_canonical(out);
                self.vel[i].append_canonical(out);
            } else {
                out.push(0);
            }
        }
    }
    fn append_hash<H: Hasher>(&self, state: &mut H) {
        state.write(&SNAPSHOT_VERSION.to_le_bytes());
        state.write(&(self.alive.len() as u32).to_le_bytes());
        for i in 0..self.alive.len() {
            if self.alive[i] {
                state.write(&[1]);
                self.pos[i].append_hash(state);
                self.vel[i].append_hash(state);
            } else {
                state.write(&[0]);
            }
        }
    }
}

/// Merkle digest of a [`World`] that runs beside the full rehash (RC9
/// experiment); it never replaces [`CanonicalBytes::canonical_hash`] or
/// `World::state_hash`, and its root is a different value on purpose.
///
/// Leaf `2 * slot` hashes the alive flag and the position, leaf `2 * slot + 1`
/// the alive flag and the velocity, so a divergence names its slot and
/// component. A caller that changes a slot calls [`WorldDigest::update_slot`]:
/// O(log slots) instead of rehashing every slot. A spawn past the slot count
/// needs a new digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldDigest {
    tree: MerkleDigest,
    slots: usize,
}

impl WorldDigest {
    /// Hash every slot of `world`, O(slots).
    pub fn new(world: &World) -> Self {
        let slots = world.alive.len();
        let leaves: Vec<u64> = (0..slots * 2)
            .map(|leaf| Self::leaf_hash(world, leaf))
            .collect();
        Self {
            tree: MerkleDigest::from_leaves(&leaves),
            slots,
        }
    }

    /// The summary of every slot.
    pub fn root(&self) -> u64 {
        self.tree.root()
    }

    /// Rehash `slot` after its alive flag or components changed. `false`
    /// (nothing done) when `slot` is past the slots this digest was built for.
    pub fn update_slot(&mut self, world: &World, slot: usize) -> bool {
        if slot >= self.slots || slot >= world.alive.len() {
            return false;
        }
        for leaf in [slot * 2, slot * 2 + 1] {
            self.tree.set_leaf(leaf, Self::leaf_hash(world, leaf));
        }
        true
    }

    /// First `(slot, component)` (0 position, 1 velocity) where two digests
    /// differ, `None` when equal.
    pub fn first_divergence(&self, other: &Self) -> Option<(usize, u8)> {
        let leaf = self.tree.first_divergence(&other.tree)?;
        Some((leaf / 2, u8::from(leaf % 2 == 1)))
    }

    fn leaf_hash(world: &World, leaf: usize) -> u64 {
        let slot = leaf / 2;
        let mut state = CanonicalHasher::new();
        let alive = world.alive[slot];
        state.write(&[u8::from(alive)]);
        if alive && leaf.is_multiple_of(2) {
            world.pos[slot].append_hash(&mut state);
        } else if alive {
            world.vel[slot].append_hash(&mut state);
        }
        state.finish()
    }
}

/// Versioned bytes + hash returned by [`snapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Format version (always [`SNAPSHOT_VERSION`]).
    pub version: u16,
    /// Canonical versioned bytes (exact [`restore`] input).
    pub bytes: Vec<u8>,
    /// FNV-1a64 over [`Snapshot::bytes`].
    pub hash: u64,
}

/// Snapshot decode failure. Loud `Err`, never panics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// Fewer bytes than the header or a field needs.
    TooShort,
    /// Unknown version (holds the found version).
    BadVersion(u16),
    /// Alive flag was not 0 or 1 (holds the found flag).
    BadFlag(u8),
    /// Bytes left over after a full decode (holds the count).
    Trailing(usize),
}

impl core::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooShort => write!(f, "snapshot too short"),
            Self::BadVersion(v) => write!(f, "snapshot bad version {v}"),
            Self::BadFlag(b) => write!(f, "snapshot bad alive flag {b}"),
            Self::Trailing(n) => write!(f, "snapshot trailing {n} bytes"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Read one byte, bounds-checked.
fn read_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, SnapshotError> {
    let b = bytes.get(*pos).ok_or(SnapshotError::TooShort)?;
    *pos += 1;
    Ok(*b)
}

/// Read `u16` LE, bounds-checked.
fn read_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, SnapshotError> {
    let end = pos.checked_add(2).ok_or(SnapshotError::TooShort)?;
    let s = bytes.get(*pos..end).ok_or(SnapshotError::TooShort)?;
    *pos = end;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

/// Read `u32` LE, bounds-checked.
fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, SnapshotError> {
    let end = pos.checked_add(4).ok_or(SnapshotError::TooShort)?;
    let s = bytes.get(*pos..end).ok_or(SnapshotError::TooShort)?;
    *pos = end;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read `f32` from raw-bits LE, bounds-checked.
fn read_f32(bytes: &[u8], pos: &mut usize) -> Result<f32, SnapshotError> {
    Ok(f32::from_bits(read_u32(bytes, pos)?))
}

/// Capture `world` as versioned bytes + hash (slot order).
pub fn snapshot(world: &World) -> Snapshot {
    let mut bytes = Vec::new();
    world.append_canonical(&mut bytes);
    let hash = hash_bytes(&bytes);
    Snapshot {
        version: SNAPSHOT_VERSION,
        bytes,
        hash,
    }
}

/// First divergent slot/component (0 pos, 1 other); equal lengths, None iff equal.
#[cfg(test)]
pub(crate) fn first_div(a: &World, b: &World, oa: &[u32], ob: &[u32]) -> Option<(usize, u8)> {
    let pos = a
        .alive
        .iter()
        .zip(b.alive.iter())
        .zip(a.pos.iter().zip(b.pos.iter()))
        .position(|((aa, bb), (ap, bp))| {
            aa != bb
                || (*aa && (ap.x.to_bits() != bp.x.to_bits() || ap.y.to_bits() != bp.y.to_bits()))
        });
    if let Some(i) = pos {
        return Some((i, 0));
    }
    let other = oa.iter().zip(ob.iter()).position(|(x, y)| x != y);
    if let Some(idx) = other {
        return Some((idx / 4, 1));
    }
    None
}

/// Restore the world [`snapshot`] captured (slot count is u32 LE).
pub fn restore(bytes: &[u8]) -> Result<World, SnapshotError> {
    let mut pos = 0;
    let version = read_u16(bytes, &mut pos)?;
    if version != SNAPSHOT_VERSION {
        return Err(SnapshotError::BadVersion(version));
    }
    let slots = read_u32(bytes, &mut pos)? as usize;
    let mut world = World::new();
    for _ in 0..slots {
        let flag = read_u8(bytes, &mut pos)?;
        if flag == 1 {
            let px = read_f32(bytes, &mut pos)?;
            let py = read_f32(bytes, &mut pos)?;
            let vx = read_f32(bytes, &mut pos)?;
            let vy = read_f32(bytes, &mut pos)?;
            world.spawn(Position { x: px, y: py }, Velocity { x: vx, y: vy });
        } else if flag == 0 {
            world.pos.push(Position::default());
            world.vel.push(Velocity::default());
            world.alive.push(false);
        } else {
            return Err(SnapshotError::BadFlag(flag));
        }
    }
    if pos != bytes.len() {
        return Err(SnapshotError::Trailing(bytes.len() - pos));
    }
    Ok(world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn seeded_world() -> World {
        let mut w = World::new();
        w.spawn(Position { x: 3.0, y: 4.0 }, Velocity { x: 1.0, y: -0.5 });
        w.spawn(Position { x: -1.0, y: 2.0 }, Velocity { x: 0.0, y: 0.0 });
        w.kill(0);
        w.spawn(Position { x: 7.0, y: 8.0 }, Velocity { x: 0.25, y: 0.5 });
        w.integrate(0.016);
        w
    }

    #[test]
    fn roundtrip_hash_equal() {
        let w = seeded_world();
        let snap = snapshot(&w);
        assert_eq!(snap.version, SNAPSHOT_VERSION);
        assert_eq!(snap.hash, hash_bytes(&snap.bytes));
        let back = match restore(&snap.bytes) {
            Ok(w) => w,
            Err(e) => panic!("roundtrip decodes: {e}"),
        };
        assert_eq!(back.state_hash(), w.state_hash());
        assert_eq!(back.count(), w.count());
        assert_eq!(back.alive, w.alive);
        let re = snapshot(&back);
        assert_eq!(re.bytes, snap.bytes);
        assert_eq!(re.hash, snap.hash);
    }

    #[test]
    fn empty_roundtrip() {
        let w = World::new();
        let snap = snapshot(&w);
        let back = match restore(&snap.bytes) {
            Ok(w) => w,
            Err(e) => panic!("empty decodes: {e}"),
        };
        assert_eq!(back.count(), 0);
        assert_eq!(back.state_hash(), w.state_hash());
    }

    #[test]
    fn seeded_kill_order_moves_zero_snapshot_hashes() {
        // Snapshot bytes are slot order (index-ordered Vec), so the
        // order staged kills are issued in never reaches bytes or hashes.
        fn xorshift(mut x: u64) -> u64 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        }
        fn build(kill_order: &[usize]) -> Snapshot {
            let mut w = World::new();
            for i in 0..8 {
                w.spawn(
                    Position {
                        x: i as f32,
                        y: -(i as f32),
                    },
                    Velocity { x: 0.5, y: 0.25 },
                );
            }
            for k in kill_order {
                assert!(w.kill(*k));
            }
            w.integrate(0.016);
            snapshot(&w)
        }
        let forward = [0, 2, 4, 6];
        let mut backward = forward;
        backward.reverse();
        let mut shuffled = forward;
        let mut rng = 0xC10C_7EED_00D1_5EEDu64 | 1;
        for i in (1..shuffled.len()).rev() {
            rng = xorshift(rng);
            shuffled.swap(i, (rng as usize) % (i + 1));
        }
        assert_ne!(shuffled, forward, "shuffle must perturb the test");
        let (a, b, c) = (build(&forward), build(&backward), build(&shuffled));
        assert_eq!(a.bytes, b.bytes, "kill order moved snapshot bytes");
        assert_eq!(a.bytes, c.bytes, "seeded kill order moved snapshot bytes");
        assert_eq!(a.hash, b.hash, "kill order moved the snapshot hash");
        assert_eq!(a.hash, c.hash, "seeded kill order moved the hash");
        // A real change still moves the hash, so the test is not vacuous.
        let mut moved = World::new();
        for i in 0..8 {
            moved.spawn(
                Position {
                    x: i as f32 + 1.0,
                    y: -(i as f32),
                },
                Velocity { x: 0.5, y: 0.25 },
            );
        }
        for k in forward {
            assert!(moved.kill(k));
        }
        moved.integrate(0.016);
        assert_ne!(snapshot(&moved).hash, a.hash);
    }
    #[test]
    fn bad_bytes_rejected() {
        assert_eq!(restore(&[]).err(), Some(SnapshotError::TooShort));
        let mut bad = snapshot(&seeded_world()).bytes;
        bad[0] = 0xff;
        bad[1] = 0xff;
        assert_eq!(restore(&bad).err(), Some(SnapshotError::BadVersion(0xffff)));
        let mut trailing = snapshot(&World::new()).bytes;
        trailing.push(0);
        assert_eq!(restore(&trailing).err(), Some(SnapshotError::Trailing(1)));
    }

    fn wide_world(slots: usize) -> World {
        let mut w = World::new();
        for i in 0..slots {
            w.spawn(
                Position {
                    x: i as f32 * 0.25,
                    y: -(i as f32),
                },
                Velocity { x: 0.5, y: -0.25 },
            );
            if i % 3 == 0 {
                w.kill(i);
            }
        }
        w.integrate(0.016);
        w
    }

    #[test]
    fn streamed_hash_matches_materialized_bytes() {
        // The streaming path must hash exactly the bytes append_canonical emits.
        for w in [World::new(), seeded_world(), wide_world(512)] {
            let mut bytes = Vec::new();
            w.append_canonical(&mut bytes);
            assert_eq!(w.canonical_hash(), hash_bytes(&bytes));
            assert_eq!(w.canonical_hash(), snapshot(&w).hash);
            let mut scratch = Vec::new();
            assert_eq!(w.canonical_hash_buf(&mut scratch), hash_bytes(&bytes));
            assert_eq!(scratch, bytes);
        }
        let p = Position { x: 1.5, y: -2.0 };
        let v = Velocity { x: 0.25, y: 8.0 };
        let mut pb = Vec::new();
        p.append_canonical(&mut pb);
        assert_eq!(p.canonical_hash(), hash_bytes(&pb));
        let mut vb = Vec::new();
        v.append_canonical(&mut vb);
        assert_eq!(v.canonical_hash(), hash_bytes(&vb));
        // Not vacuous: a one-bit state change moves the streamed hash.
        let mut moved = seeded_world();
        moved.pos[1].x += 0.125;
        assert_ne!(moved.canonical_hash(), seeded_world().canonical_hash());
    }

    #[test]
    fn canonical_hash_zero_alloc() {
        // CODELAW 5 gate: the hash path performs no heap allocation.
        // Calibration first: the probe must observe a real allocation,
        // otherwise a zero reading would prove nothing.
        let (_, seen) = measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.calls >= 1, "alloc probe is blind");
        let worlds = [World::new(), seeded_world(), wide_world(512)];
        let p = Position { x: 1.5, y: -2.0 };
        let v = Velocity { x: 0.25, y: 8.0 };
        for w in &worlds {
            let ((), heap) = measure(|| {
                std::hint::black_box(w.canonical_hash());
            });
            assert_eq!(heap.calls, 0, "canonical_hash allocated on the hash path");
        }
        for (_, heap) in [
            measure(|| std::hint::black_box(p.canonical_hash())),
            measure(|| std::hint::black_box(v.canonical_hash())),
        ] {
            assert_eq!(
                heap.calls, 0,
                "leaf canonical_hash allocated on the hash path"
            );
        }
        // A reused scratch buffer also hashes with no allocation once sized.
        let mut scratch = Vec::with_capacity(1 << 16);
        for w in &worlds {
            let ((), heap) = measure(|| {
                std::hint::black_box(w.canonical_hash_buf(&mut scratch));
            });
            assert_eq!(
                heap.calls, 0,
                "canonical_hash_buf reallocated a sized buffer"
            );
        }
    }

    #[test]
    fn first_div_names_first_difference_on_seeded_11k() {
        use engine_core::Rng;
        let mut world = World::new();
        let mut rng = Rng::new(2040);
        let mut other = Vec::with_capacity(44_000);
        for _ in 0..11_000 {
            world.spawn(
                Position {
                    x: f32::from_bits(rng.next_u32()),
                    y: f32::from_bits(rng.next_u32()),
                },
                Velocity::default(),
            );
            for _ in 0..4 {
                other.push(rng.next_u32());
            }
        }
        let mut moved_other = other.clone();
        moved_other[9_999] = moved_other[9_999].wrapping_add(1);
        assert_eq!(super::first_div(&world, &world, &other, &other), None);
        assert_eq!(
            super::first_div(&world, &world, &other, &moved_other),
            Some((2499, 1))
        );
        let mut ax = World::new();
        ax.spawn(Position { x: 1.0, y: 2.0 }, Velocity::default());
        let mut bx = World::new();
        bx.spawn(Position { x: 1.0, y: 3.0 }, Velocity::default());
        assert_eq!(super::first_div(&ax, &bx, &[], &[]), Some((0, 0)));
    }

    /// A seeded world of `slots` fighters: some dead, all with their own state.
    fn seeded_slots(seed: u64, slots: usize) -> World {
        use engine_core::Rng;
        let mut rng = Rng::new(seed);
        let mut w = World::new();
        for _ in 0..slots {
            let (px, py) = (rng.next_u32(), rng.next_u32());
            let (vx, vy) = (rng.next_u32(), rng.next_u32());
            let id = w.spawn(
                Position {
                    x: f32::from_bits(px),
                    y: f32::from_bits(py),
                },
                Velocity {
                    x: f32::from_bits(vx),
                    y: f32::from_bits(vy),
                },
            );
            if rng.next_u32().is_multiple_of(5) {
                w.kill(id);
            }
        }
        w
    }

    /// The full-rehash localization: first slot whose alive flag or position
    /// differs (0), else whose velocity differs (1), reading raw bits.
    fn linear_first_div(a: &World, b: &World) -> Option<(usize, u8)> {
        (0..a.alive.len()).find_map(|i| {
            let (la, lb) = (a.alive[i], b.alive[i]);
            let pos = |w: &World| (w.pos[i].x.to_bits(), w.pos[i].y.to_bits());
            let vel = |w: &World| (w.vel[i].x.to_bits(), w.vel[i].y.to_bits());
            if la != lb || (la && pos(a) != pos(b)) {
                Some((i, 0))
            } else if la && vel(a) != vel(b) {
                Some((i, 1))
            } else {
                None
            }
        })
    }

    #[test]
    fn world_digest_matches_the_full_rehash_on_20_of_20_divergences() {
        use engine_core::Rng;
        let mut rng = Rng::new(2040);
        for seed in 0..20u64 {
            let slots = 8 + usize::try_from(seed * 13 % 90).unwrap_or(0);
            let base = seeded_slots(seed, slots);
            let mut moved = seeded_slots(seed, slots);
            let slot = usize::try_from(rng.next_u32()).unwrap_or(0) % slots;
            match seed % 4 {
                0 => moved.pos[slot].x = f32::from_bits(moved.pos[slot].x.to_bits() ^ 1),
                1 => moved.vel[slot].y = f32::from_bits(moved.vel[slot].y.to_bits() ^ 0x8000_0000),
                2 => moved.alive[slot] = !moved.alive[slot],
                _ => {
                    moved.pos[slot].y += 1.0;
                    moved.vel[slot].x += 1.0;
                }
            }
            let (da, db) = (WorldDigest::new(&base), WorldDigest::new(&moved));
            let full_differs = base.canonical_hash() != moved.canonical_hash();
            let want = linear_first_div(&base, &moved);
            assert_eq!(
                want.is_some(),
                full_differs,
                "seed {seed}: oracle vs rehash"
            );
            assert_eq!(
                da.root() != db.root(),
                full_differs,
                "seed {seed}: same verdict"
            );
            assert_eq!(
                da.first_divergence(&db),
                want,
                "seed {seed}: same slot and component"
            );
            let twin = WorldDigest::new(&seeded_slots(seed, slots));
            assert_eq!((da.root(), da.first_divergence(&twin)), (twin.root(), None));
        }
    }

    #[test]
    fn world_digest_updates_match_a_fresh_build_and_never_allocate() {
        let mut w = seeded_slots(7, 64);
        let mut digest = WorldDigest::new(&w);
        let (mut changed, mut heap_calls) = (0, 0);
        for tick in 0..50usize {
            // Two slots change per tick: one is hit, one drifts.
            let (hit, drift) = (tick * 7 % 64, tick * 11 % 64);
            w.pos[hit].x += 0.5;
            w.vel[drift].y -= 0.25;
            if tick % 9 == 0 {
                w.kill(hit);
            }
            let ((), heap) = measure(|| {
                assert!(digest.update_slot(&w, hit));
                assert!(digest.update_slot(&w, drift));
                std::hint::black_box(digest.root());
            });
            heap_calls += heap.calls;
            changed += 2;
            assert_eq!(digest, WorldDigest::new(&w), "tick {tick}");
        }
        assert_eq!(
            (changed, heap_calls),
            (100, 0),
            "update_slot is O(log n), no heap"
        );
        assert!(!digest.update_slot(&w, 64), "past the slots does nothing");
    }

    /// RC9 timing: full streamed rehash against the Merkle update, both
    /// over the same world. Run: cargo test -p engine-ecs --lib --release
    /// world_digest_timing -- --ignored --nocapture
    #[test]
    #[ignore = "timing experiment: prints ns per tick, asserts nothing about the clock"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "the wall clock times the experiment, never the tick"
    )]
    fn world_digest_timing() {
        use std::time::Instant;
        for (slots, changed) in [
            (8usize, 8usize),
            (8, 1),
            (64, 2),
            (1_024, 10),
            (11_000, 110),
        ] {
            let mut w = seeded_slots(3, slots);
            let mut digest = WorldDigest::new(&w);
            let reps = 2_000_000 / slots.max(8);
            let mut sink = 0u64;
            let t = Instant::now();
            for r in 0..reps {
                for k in 0..changed {
                    w.pos[(r + k) % slots].x += 1.0;
                }
                sink ^= w.canonical_hash();
            }
            let full = t.elapsed();
            let t = Instant::now();
            for r in 0..reps {
                for k in 0..changed {
                    let slot = (r + k) % slots;
                    w.pos[slot].x += 1.0;
                    digest.update_slot(&w, slot);
                }
                sink ^= digest.root();
            }
            let merkle = t.elapsed();
            let per = |d: std::time::Duration| d.as_nanos() as f64 / reps as f64;
            println!(
                "slots {slots:>6} changed {changed:>4}: full {:>9.0} ns/tick  merkle {:>9.0} ns/tick  ({:.2}x)  sink {sink:x}",
                per(full),
                per(merkle),
                per(full) / per(merkle),
            );
        }
    }
}

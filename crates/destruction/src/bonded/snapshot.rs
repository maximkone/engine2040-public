//! snapshot: canonical bytes of a bonded collection's moving state, and the
//! restore that puts a live collection back to those bytes exactly.
//!
//! Consumer: net rollback and the replay proof: a match saves the collection
//! at a checkpoint and restores it to resim the same inputs; the world
//! snapshot (`snapshot.rs`) folds these bytes with the debris ledger's.
//! Data owner: `engine-destruction`. Units: bit-exact `f32` bits, ticks.
//! Determinism scope: the bytes are a pure function of the collection's
//! state in index order, so a restored collection hashes (`state_hash`)
//! and steps exactly like the original on every PC; the pending break log
//! is part of the state, because events not yet drained must survive a
//! rollback. Errors: [`BreakError::BadSnapshot`] for truncated, versioned
//! wrong, foreign-shape, non-finite or trailing bytes; restore never
//! panics and never half-applies (the live state is replaced only after
//! the whole payload parsed).

use super::{slot, wire, BondedCollection, BreakError, BreakEvent};
use crate::cursor::Cursor;
use engine_core::Fnv64;

/// Wire version of the snapshot bytes.
pub const BONDED_SNAPSHOT_V1: u32 = 1;
/// Bytes of one encoded break event.
const EVENT_LEN: usize = 13;

impl BondedCollection {
    /// Digest of what never changes after construction: chunk sizes and
    /// masses, the bond graph with its limits, the anchors and the
    /// threshold. A snapshot restores only into a collection of the same
    /// shape.
    #[must_use]
    pub fn shape_digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(wire(self.chunks.len()));
        for c in &self.chunks {
            h.write_u32(c.hx.to_bits());
            h.write_u32(c.hy.to_bits());
            h.write_u32(c.hz.to_bits());
            h.write_u32(c.mass.to_bits());
        }
        h.write_u32(wire(self.bonds.len()));
        for b in &self.bonds {
            h.write_u32(b.a);
            h.write_u32(b.b);
            h.write_u32(b.breaking_strain.to_bits());
        }
        for anchored in &self.anchored {
            h.write_u8(u8::from(*anchored));
        }
        h.write_u32(self.threshold.to_bits());
        h.finish()
    }

    /// Canonical bytes of the moving state: version, shape digest, tick,
    /// load capacity, chunk poses and velocities with their dynamic and
    /// freed flags, bond strains with snap flags, then the pending break
    /// log. Fixed layout in index order.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            32 + self.chunks.len() * 26 + self.bonds.len() * 5 + self.log.len() * EVENT_LEN,
        );
        out.extend_from_slice(&BONDED_SNAPSHOT_V1.to_le_bytes());
        out.extend_from_slice(&self.shape_digest().to_le_bytes());
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.capacity_q16.to_le_bytes());
        out.extend_from_slice(&wire(self.chunks.len()).to_le_bytes());
        for i in 0..self.chunks.len() {
            let c = &self.chunks[i];
            for f in [
                c.x,
                c.y,
                c.z,
                self.vel[i][0],
                self.vel[i][1],
                self.vel[i][2],
            ] {
                out.extend_from_slice(&f.to_bits().to_le_bytes());
            }
            out.push(u8::from(self.dynamic[i]));
            out.push(u8::from(self.freed[i]));
        }
        out.extend_from_slice(&wire(self.bonds.len()).to_le_bytes());
        for i in 0..self.bonds.len() {
            out.extend_from_slice(&self.strain[i].to_bits().to_le_bytes());
            out.push(u8::from(self.bond_broken[i]));
        }
        out.extend_from_slice(&wire(self.log.len()).to_le_bytes());
        for event in &self.log {
            out.extend_from_slice(&super::event_bytes(event));
        }
        out
    }

    /// FNV-1a over [`Self::snapshot_bytes`]: the checkpoint receipt.
    #[must_use]
    pub fn snapshot_digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write(&self.snapshot_bytes());
        h.finish()
    }

    /// Puts this collection back to snapshot bytes taken from a collection
    /// of the same shape. Supports and loads are recomputed from the snap
    /// flags (they are a function of them). On any error nothing changed.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), BreakError> {
        let mut r = Cursor::new(bytes, BreakError::BadSnapshot);
        if r.read_u32()? != BONDED_SNAPSHOT_V1 || r.read_u64()? != self.shape_digest() {
            return Err(BreakError::BadSnapshot);
        }
        let tick = r.read_u32()?;
        let capacity_q16 = r.read_u64()?;
        if slot(r.read_u32()?) != self.chunks.len() {
            return Err(BreakError::BadSnapshot);
        }
        let n = self.chunks.len();
        let mut pose = Vec::with_capacity(n);
        let mut dynamic = Vec::with_capacity(n);
        let mut freed = Vec::with_capacity(n);
        for _ in 0..n {
            let p = [r.read_finite()?, r.read_finite()?, r.read_finite()?];
            let v = [r.read_finite()?, r.read_finite()?, r.read_finite()?];
            pose.push((p, v));
            dynamic.push(r.read_flag()?);
            freed.push(r.read_flag()?);
        }
        if slot(r.read_u32()?) != self.bonds.len() {
            return Err(BreakError::BadSnapshot);
        }
        let mut strain = Vec::with_capacity(self.bonds.len());
        let mut broken = Vec::with_capacity(self.bonds.len());
        for _ in 0..self.bonds.len() {
            strain.push(r.read_finite()?);
            broken.push(r.read_flag()?);
        }
        let log_len = slot(r.read_u32()?);
        if log_len > n + self.bonds.len() {
            return Err(BreakError::BadSnapshot);
        }
        let mut log = Vec::with_capacity(log_len);
        for _ in 0..log_len {
            let e = r.take(EVENT_LEN)?;
            let tick_of = u32::from_le_bytes([e[1], e[2], e[3], e[4]]);
            let id = u32::from_le_bytes([e[5], e[6], e[7], e[8]]);
            let payload = u32::from_le_bytes([e[9], e[10], e[11], e[12]]);
            let event = match e[0] {
                0 if slot(id) < self.bonds.len() => BreakEvent::BondBroken {
                    tick: tick_of,
                    bond: id,
                    strain: f32::from_bits(payload),
                },
                1 if slot(id) < n && payload == 0 => BreakEvent::ChunkFreed {
                    tick: tick_of,
                    chunk: id,
                },
                _ => return Err(BreakError::BadSnapshot),
            };
            log.push(event);
        }
        if !r.is_done() {
            return Err(BreakError::BadSnapshot);
        }
        // Whole payload parsed: apply.
        self.tick = tick;
        self.capacity_q16 = capacity_q16;
        for (i, (p, v)) in pose.iter().enumerate() {
            self.chunks[i].x = p[0];
            self.chunks[i].y = p[1];
            self.chunks[i].z = p[2];
            self.vel[i] = *v;
            self.dynamic[i] = dynamic[i];
            self.freed[i] = freed[i];
        }
        self.strain = strain;
        self.bond_broken = broken;
        self.log = log;
        self.flood_loads();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bonded::{Bond, BondedDesc, Chunk3, DEFAULT_BOND_STRAIN};

    fn chain() -> BondedDesc {
        let chunk = |y: f32, mass: f32| Chunk3 {
            x: 0.0,
            y,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass,
        };
        BondedDesc {
            chunks: vec![
                chunk(0.5, 1.0),
                chunk(1.5, 1.0),
                chunk(2.5, 2.0),
                chunk(3.5, 1.0),
            ],
            bonds: (0..3)
                .map(|i| Bond {
                    a: i,
                    b: i + 1,
                    breaking_strain: DEFAULT_BOND_STRAIN,
                })
                .collect(),
            anchors: vec![0],
            break_threshold: 3.0,
        }
    }

    #[test]
    fn snapshot_restore_snapshot_is_byte_equal_and_resim_matches() {
        let mut live = BondedCollection::new(&chain()).expect("valid chain");
        live.step(1.0 / 60.0);
        live.sever_chunk(1, 2).expect("in range");
        let bytes = live.snapshot_bytes();
        assert_eq!(bytes, live.snapshot_bytes(), "bytes x2");
        // Run on, then roll back and run again: same hash and events.
        let mut first = Vec::new();
        for _ in 0..20 {
            live.step(1.0 / 60.0);
            live.drain_into(&mut first);
        }
        let end_hash = live.state_hash();
        let mut other = BondedCollection::new(&chain()).expect("valid chain");
        other.restore_bytes(&bytes).expect("restores");
        assert_eq!(other.snapshot_bytes(), bytes, "restore then snapshot");
        let mut second = Vec::new();
        for _ in 0..20 {
            other.step(1.0 / 60.0);
            other.drain_into(&mut second);
        }
        assert_eq!(other.state_hash(), end_hash);
        // The log held at the snapshot is delivered once, in both runs.
        assert!(!first.is_empty());
        assert_eq!(first, second);
        assert_eq!(other.load_digest(), live.load_digest());
    }

    #[test]
    fn restore_refuses_foreign_shapes_and_hostile_bytes() {
        let live = BondedCollection::new(&chain()).expect("valid chain");
        let bytes = live.snapshot_bytes();
        let mut shorter = chain();
        shorter.chunks.pop();
        shorter.bonds.pop();
        let mut foreign = BondedCollection::new(&shorter).expect("valid chain");
        let before = foreign.snapshot_bytes();
        assert_eq!(foreign.restore_bytes(&bytes), Err(BreakError::BadSnapshot));
        assert_eq!(foreign.snapshot_bytes(), before, "nothing half-applied");
        let mut target = BondedCollection::new(&chain()).expect("valid chain");
        let before = target.snapshot_bytes();
        for cut in 0..bytes.len() {
            assert!(target.restore_bytes(&bytes[..cut]).is_err(), "cut {cut}");
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(target.restore_bytes(&trailing).is_err());
        let mut bad_version = bytes.clone();
        bad_version[0] = 9;
        assert!(target.restore_bytes(&bad_version).is_err());
        // A NaN pose is refused.
        let mut nan = bytes.clone();
        let pose_at = 4 + 8 + 4 + 8 + 4;
        nan[pose_at..pose_at + 4].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        assert!(target.restore_bytes(&nan).is_err());
        assert_eq!(
            target.snapshot_bytes(),
            before,
            "every refusal left it intact"
        );
        assert!(target.restore_bytes(&bytes).is_ok());
    }

    #[test]
    fn the_load_rule_and_pending_log_survive_a_rollback() {
        let mut live = BondedCollection::new(&chain()).expect("valid chain");
        let capacity = live.max_load_ratio_q16() * 5 / 4;
        live.set_capacity_q16(capacity);
        live.sever_chunk(1, 1).expect("in range");
        assert!(!live.drain_events().is_empty());
        live.sever_chunk(2, 1).expect("in range");
        let pending = live.snapshot_bytes();
        let mut copy = BondedCollection::new(&chain()).expect("valid chain");
        copy.restore_bytes(&pending).expect("restores");
        assert_eq!(copy.capacity_q16(), live.capacity_q16());
        assert_eq!(copy.drain_events(), live.drain_events());
        assert_eq!(copy.state_hash(), live.state_hash());
    }
}

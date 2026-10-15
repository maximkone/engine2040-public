//! shards3d: deterministic 3D runtime wake-set over the [`Chunk3`] bake.
//! Mirrors `shards` (2D): bitset in index order, impulse gate wakes the hit
//! chunk plus AABB-contact neighbors. Units are meters and seconds.

use crate::bake::{BREAKING_THRESHOLD, CONTACT_MARGIN};
use crate::bake3d::Chunk3;
use crate::shards::{cascade_contact, WakeSet};
use engine_core::Fnv64;

/// 3D AABB overlap plus margin decides neighborhood.
pub fn in_contact(a: &Chunk3, b: &Chunk3) -> bool {
    (a.x - b.x).abs() < a.hx + b.hx + CONTACT_MARGIN
        && (a.y - b.y).abs() < a.hy + b.hy + CONTACT_MARGIN
        && (a.z - b.z).abs() < a.hz + b.hz + CONTACT_MARGIN
}

/// Bitset in chunk index order: the one `shards::WakeSet`, shared by both
/// runtimes so the mask layout cannot drift between 2D and 3D.
pub type WakeSet3 = WakeSet;

/// Awake-only 3D integrator over baked chunks. Sleeping chunks never move,
/// so the wake-set fully decides which state can change each tick.
#[derive(Clone, Debug)]
pub struct Runtime3 {
    pub chunks: Vec<Chunk3>,
    pub vel: Vec<[f32; 3]>,
    pub mask: WakeSet3,
}
impl Runtime3 {
    /// There is no `Bake3` edge graph; the runtime owns the baked chunks.
    pub fn from_chunks(chunks: Vec<Chunk3>) -> Self {
        Self {
            vel: vec![[0.0f32; 3]; chunks.len()],
            mask: WakeSet3::new(chunks.len()),
            chunks,
        }
    }
    pub fn len(&self) -> usize {
        self.chunks.len()
    }
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
    pub fn wake(&mut self, id: usize) {
        self.mask.wake(id);
    }
    pub fn sleep(&mut self, id: usize) {
        self.mask.sleep(id);
        if id < self.vel.len() {
            self.vel[id] = [0.0, 0.0, 0.0];
        }
    }
    pub fn is_awake(&self, id: usize) -> bool {
        self.mask.is_awake(id)
    }
    pub fn awake_count(&self) -> usize {
        self.mask.count()
    }
    pub fn awake_ids(&self) -> Vec<usize> {
        self.mask.ids()
    }
    pub fn wake_all(&mut self) {
        for i in 0..self.len() {
            self.mask.wake(i);
        }
    }
    pub fn sleep_all(&mut self) {
        self.mask.clear();
        for v in &mut self.vel {
            *v = [0.0, 0.0, 0.0];
        }
    }
    /// Wake every chunk in AABB contact with `id`, margin included.
    pub fn wake_neighbors(&mut self, id: usize) {
        if id >= self.len() {
            return;
        }
        // Index order keeps the wake-set deterministic.
        for k in 0..self.len() {
            if k != id && in_contact(&self.chunks[id], &self.chunks[k]) {
                self.mask.wake(k);
            }
        }
    }
    /// Bullet threshold as gate: returns true iff the hit fractures.
    pub fn apply_impact(&mut self, id: usize, impulse: f32) -> bool {
        if id >= self.len() {
            return false;
        }
        if impulse > self.chunks[id].mass * BREAKING_THRESHOLD {
            self.mask.wake(id);
            self.wake_neighbors(id);
            true
        } else {
            false
        }
    }
    /// Contact cascade: an awake chunk touching a sleeper wakes it.
    pub fn propagate_contact(&mut self) {
        cascade_contact(&mut self.mask, &self.chunks, in_contact);
    }
    /// Integrates awake chunks only (gravity + ground clamp mirrors `shards`).
    pub fn step(&mut self, dt: f32) {
        for i in 0..self.len() {
            if !self.mask.is_awake(i) {
                continue;
            }
            let (c, v) = (&mut self.chunks[i], &mut self.vel[i]);
            v[1] -= 9.8 * dt;
            c.x += v[0] * dt;
            c.y += v[1] * dt;
            c.z += v[2] * dt;
            if c.y < c.hy {
                c.y = c.hy;
                v[1] = 0.0;
            }
        }
    }
    pub fn sleep_slow(&mut self, vmax: f32) {
        let v2 = vmax * vmax;
        for i in 0..self.len() {
            if self.mask.is_awake(i)
                && self.vel[i][0] * self.vel[i][0]
                    + self.vel[i][1] * self.vel[i][1]
                    + self.vel[i][2] * self.vel[i][2]
                    < v2
            {
                self.sleep(i);
            }
        }
    }
    /// FNV-1a64 over length, mask words, and awake chunk plus velocity bits.
    /// Index order and bit-exact floats keep reruns identical.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(self.len() as u32);
        for w in self.mask.words() {
            h.write_u64(*w);
        }
        for i in 0..self.len() {
            if !self.mask.is_awake(i) {
                continue;
            }
            let c = &self.chunks[i];
            for w in [
                c.x.to_bits(),
                c.y.to_bits(),
                c.z.to_bits(),
                c.hx.to_bits(),
                c.hy.to_bits(),
                c.hz.to_bits(),
                c.mass.to_bits(),
                self.vel[i][0].to_bits(),
                self.vel[i][1].to_bits(),
                self.vel[i][2].to_bits(),
            ] {
                h.write_u32(w);
            }
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake3d::fracture3;
    fn baked() -> Vec<Chunk3> {
        fracture3(0.0, 5.0, 0.0, 6.0, 4.0, 3.0, 8, 11)
    }
    fn run() -> u64 {
        let mut r = Runtime3::from_chunks(baked());
        r.apply_impact(0, 1e9);
        r.step(1.0 / 60.0);
        r.state_hash()
    }
    #[test]
    fn wake_hash_x2() {
        assert_eq!(run(), run());
    }
    #[test]
    fn state_hash_is_fnv1a_over_the_canonical_word_stream() {
        let mut r = Runtime3::from_chunks(baked());
        r.apply_impact(0, 1e9);
        r.step(1.0 / 60.0);
        // Oracle: length, each mask word as low then high u32, then the
        // awake chunks' box, mass and velocity bits, as plain LE bytes.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(r.len() as u32).to_le_bytes());
        for w in r.mask.words() {
            bytes.extend_from_slice(&(*w as u32).to_le_bytes());
            bytes.extend_from_slice(&((*w >> 32) as u32).to_le_bytes());
        }
        for i in 0..r.len() {
            if !r.is_awake(i) {
                continue;
            }
            let c = &r.chunks[i];
            for w in [
                c.x.to_bits(),
                c.y.to_bits(),
                c.z.to_bits(),
                c.hx.to_bits(),
                c.hy.to_bits(),
                c.hz.to_bits(),
                c.mass.to_bits(),
                r.vel[i][0].to_bits(),
                r.vel[i][1].to_bits(),
                r.vel[i][2].to_bits(),
            ] {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        assert!(r.awake_count() > 0, "the oracle must cover awake chunks");
        assert_eq!(r.state_hash(), engine_core::hash_bytes(&bytes));
    }
    #[test]
    fn threshold_gates_wake() {
        let mut r = Runtime3::from_chunks(baked());
        assert!(!r.apply_impact(0, r.chunks[0].mass * BREAKING_THRESHOLD * 0.5) && !r.is_awake(0));
        assert!(r.apply_impact(0, r.chunks[0].mass * BREAKING_THRESHOLD + 1.0) && r.is_awake(0));
        for k in 0..r.len() {
            if k != 0 && in_contact(&r.chunks[0], &r.chunks[k]) {
                assert!(r.is_awake(k));
            }
        }
    }
    #[test]
    fn bad_id_breaks_nothing() {
        let mut r = Runtime3::from_chunks(baked());
        assert!(!r.apply_impact(r.len(), 1e9));
        assert!(!r.apply_impact(usize::MAX, 1e9));
        assert_eq!(r.awake_count(), 0);
    }
    #[test]
    fn sleep_freezes() {
        let mut r = Runtime3::from_chunks(baked());
        let x0 = r.chunks[1].x;
        let z0 = r.chunks[1].z;
        r.step(0.1);
        assert_eq!(r.chunks[1].x.to_bits(), x0.to_bits());
        r.wake(1);
        r.vel[1] = [2.0, 0.0, 1.0];
        r.step(0.5);
        assert!(r.chunks[1].x > x0 && r.chunks[1].z > z0);
        r.sleep(1);
        let (x1, z1) = (r.chunks[1].x, r.chunks[1].z);
        r.step(0.5);
        assert_eq!(r.chunks[1].x.to_bits(), x1.to_bits());
        assert_eq!(r.chunks[1].z.to_bits(), z1.to_bits());
    }
    #[test]
    fn ground_clamps_fall() {
        let mut r = Runtime3::from_chunks(vec![Chunk3 {
            x: 0.0,
            y: 2.0,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        }]);
        r.wake(0);
        r.step(10.0);
        assert_eq!(r.chunks[0].y.to_bits(), 0.5f32.to_bits());
        assert_eq!(r.vel[0][1].to_bits(), 0.0f32.to_bits());
    }
    #[test]
    fn slow_sleeps_fast_stays() {
        let mut r = Runtime3::from_chunks(baked());
        r.wake_all();
        r.vel[0] = [0.01, 0.0, 0.0];
        r.vel[1] = [5.0, 0.0, 0.0];
        r.sleep_slow(0.1);
        assert!(!r.is_awake(0) && r.is_awake(1) && r.awake_count() == 1);
    }
    #[test]
    fn contact_margin_3d() {
        let a = Chunk3 {
            x: 0.0,
            y: 0.5,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        };
        let near = Chunk3 {
            x: 1.0 + CONTACT_MARGIN * 0.5,
            y: 0.5,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        };
        let far_z = Chunk3 {
            x: 0.0,
            y: 0.5,
            z: 50.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        };
        assert!(in_contact(&a, &near) && !in_contact(&a, &far_z));
    }
    #[test]
    fn contact_wakes_and_all_sleeps() {
        // Two touching boxes: waking one cascades to the other.
        let pair = vec![
            Chunk3 {
                x: 0.0,
                y: 0.5,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            },
            Chunk3 {
                x: 1.0,
                y: 0.5,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            },
        ];
        assert!(in_contact(&pair[0], &pair[1]));
        let mut r = Runtime3::from_chunks(pair);
        r.wake(0);
        r.propagate_contact();
        assert_eq!(r.awake_ids(), vec![0, 1]);
        r.sleep_all();
        assert_eq!(r.awake_count(), 0);
        r.wake_all();
        assert_eq!(r.awake_count(), 2);
    }
    #[test]
    fn empty_runtime_hashes_stable() {
        let r = Runtime3::from_chunks(Vec::new());
        assert!(r.is_empty() && r.awake_ids().is_empty());
        assert_eq!(
            r.state_hash(),
            Runtime3::from_chunks(Vec::new()).state_hash()
        );
    }
}

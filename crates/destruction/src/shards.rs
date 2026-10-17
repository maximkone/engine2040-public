//! shards: runtime wake-set only. Impulse > 3xmass wakes; bitset in index order.
use crate::bake::{neighbors, Bake, BREAKING_THRESHOLD, CONTACT_MARGIN};
use crate::Chunk;
use engine_core::Fnv64;

/// True when two chunk boxes touch within contact slack; meters, deterministic.
pub fn in_contact(a: &Chunk, b: &Chunk) -> bool {
    (a.x - b.x).abs() < a.hx + b.hx + CONTACT_MARGIN
        && (a.y - b.y).abs() < a.hy + b.hy + CONTACT_MARGIN
}

/// Awake-shard bitset in index order; wakes apply after the scan.
/// Units: shard ids. Determinism: index order, no hash map.
#[derive(Clone, Debug, Default)]
pub struct WakeSet {
    n: usize,
    words: Vec<u64>,
}
impl WakeSet {
    pub fn new(n: usize) -> Self {
        Self {
            n,
            words: vec![0u64; n.div_ceil(64)],
        }
    }
    pub fn wake(&mut self, id: usize) {
        if id < self.n {
            self.words[id / 64] |= 1u64 << (id % 64);
        }
    }
    pub fn sleep(&mut self, id: usize) {
        if id < self.n {
            self.words[id / 64] &= !(1u64 << (id % 64));
        }
    }
    pub fn is_awake(&self, id: usize) -> bool {
        id < self.n && (self.words[id / 64] >> (id % 64) & 1) == 1
    }
    pub fn clear(&mut self) {
        for w in &mut self.words {
            *w = 0;
        }
    }
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }
    pub fn ids(&self) -> Vec<usize> {
        (0..self.n).filter(|&i| self.is_awake(i)).collect()
    }
    /// Raw mask words, lowest id in bit 0 of word 0; hashed as-is.
    pub(crate) fn words(&self) -> &[u64] {
        &self.words
    }
}

/// Contact cascade shared by the 2D and 3D runtimes: every sleeper that
/// touches an awake chunk wakes. Wakes apply after the scan, in ascending id
/// order, so the result never depends on scan order.
pub(crate) fn cascade_contact<T>(
    mask: &mut WakeSet,
    chunks: &[T],
    touching: impl Fn(&T, &T) -> bool,
) {
    let mut woken = Vec::new();
    for i in 0..chunks.len() {
        if !mask.is_awake(i) {
            continue;
        }
        for k in 0..chunks.len() {
            if k != i && !mask.is_awake(k) && touching(&chunks[i], &chunks[k]) {
                woken.push(k);
            }
        }
    }
    woken.sort_unstable();
    woken.dedup();
    for k in woken {
        mask.wake(k);
    }
}

#[derive(Clone, Debug)]
pub struct Runtime {
    pub chunks: Vec<Chunk>,
    pub vel: Vec<[f32; 2]>,
    pub mask: WakeSet,
}
impl Runtime {
    pub fn from_bake(b: &Bake) -> Self {
        Self {
            chunks: b.shards.clone(),
            vel: vec![[0.0f32; 2]; b.shards.len()],
            mask: WakeSet::new(b.shards.len()),
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
            self.vel[id] = [0.0, 0.0];
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
            *v = [0.0, 0.0];
        }
    }
    pub fn wake_neighbors(&mut self, bake: &Bake, id: usize) {
        for k in neighbors(bake, id) {
            self.mask.wake(k);
        }
    }
    /// Bullet threshold as gate: returns true iff the hit fractures.
    pub fn apply_impact(&mut self, bake: &Bake, id: usize, impulse: f32) -> bool {
        if id >= self.len() {
            return false;
        }
        if impulse > self.chunks[id].mass * BREAKING_THRESHOLD {
            self.mask.wake(id);
            self.wake_neighbors(bake, id);
            true
        } else {
            false
        }
    }
    /// Contact cascade: awake shard touching a sleeper (margin included) wakes it.
    pub fn propagate_contact(&mut self) {
        cascade_contact(&mut self.mask, &self.chunks, in_contact);
    }
    /// Integrates awake shards only (gravity + ground stub mirrors physics::Space).
    pub fn step(&mut self, dt: f32) {
        for i in 0..self.len() {
            if !self.mask.is_awake(i) {
                continue;
            }
            let (c, v) = (&mut self.chunks[i], &mut self.vel[i]);
            v[1] -= 9.8 * dt;
            c.x += v[0] * dt;
            c.y += v[1] * dt;
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
                && self.vel[i][0] * self.vel[i][0] + self.vel[i][1] * self.vel[i][1] < v2
            {
                self.sleep(i);
            }
        }
    }
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_len32(self.len());
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
                c.hx.to_bits(),
                c.hy.to_bits(),
                c.mass.to_bits(),
                self.vel[i][0].to_bits(),
                self.vel[i][1].to_bits(),
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
    use crate::bake::{bake, Mode, Params};
    fn baked() -> Bake {
        bake(&Params {
            x: 0.0,
            y: 5.0,
            hx: 6.0,
            hy: 4.0,
            pieces: 8,
            seed: 11,
            mode: Mode::Voronoi,
        })
    }
    fn run() -> u64 {
        let b = baked();
        let mut r = Runtime::from_bake(&b);
        r.apply_impact(&b, 0, 1e9);
        r.step(1.0 / 60.0);
        r.state_hash()
    }
    #[test]
    fn wake_hash_x2() {
        assert_eq!(run(), run());
    }
    #[test]
    fn state_hash_is_fnv1a_over_the_canonical_word_stream() {
        let b = baked();
        let mut r = Runtime::from_bake(&b);
        r.apply_impact(&b, 0, 1e9);
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
                c.hx.to_bits(),
                c.hy.to_bits(),
                c.mass.to_bits(),
                r.vel[i][0].to_bits(),
                r.vel[i][1].to_bits(),
            ] {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        assert!(r.awake_count() > 0, "the oracle must cover awake chunks");
        assert_eq!(r.state_hash(), engine_core::hash_bytes(&bytes));
    }
    #[test]
    fn threshold_gates_wake() {
        let b = baked();
        let mut r = Runtime::from_bake(&b);
        assert!(
            !r.apply_impact(&b, 0, r.chunks[0].mass * BREAKING_THRESHOLD * 0.5) && !r.is_awake(0)
        );
        assert!(
            r.apply_impact(&b, 0, r.chunks[0].mass * BREAKING_THRESHOLD + 1.0) && r.is_awake(0)
        );
        for k in neighbors(&b, 0) {
            assert!(r.is_awake(k));
        }
    }
    #[test]
    fn sleep_freezes() {
        let b = baked();
        let mut r = Runtime::from_bake(&b);
        let x0 = r.chunks[1].x;
        r.step(0.1);
        assert_eq!(r.chunks[1].x.to_bits(), x0.to_bits());
        r.wake(1);
        r.vel[1] = [2.0, 0.0];
        r.step(0.5);
        assert!(r.chunks[1].x > x0);
        r.sleep(1);
        let x1 = r.chunks[1].x;
        r.step(0.5);
        assert_eq!(r.chunks[1].x.to_bits(), x1.to_bits());
    }
    #[test]
    fn contact_margin() {
        let (a, f) = (
            Chunk {
                x: 0.0,
                y: 0.5,
                hx: 0.5,
                hy: 0.5,
                mass: 1.0,
            },
            Chunk {
                x: 50.0,
                y: 50.0,
                hx: 0.5,
                hy: 0.5,
                mass: 1.0,
            },
        );
        let t = Chunk {
            x: 1.0 + CONTACT_MARGIN * 0.5,
            y: 0.5,
            hx: 0.5,
            hy: 0.5,
            mass: 1.0,
        };
        assert!(in_contact(&a, &t) && !in_contact(&a, &f));
    }
}

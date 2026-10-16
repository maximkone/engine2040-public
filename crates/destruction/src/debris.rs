//! debris: baked debris and pre-broken arena props over `bake3d`/`shards3d`.
//!
//! Consumer: the arena feeds [`HitWake`] events mapped from combat hits; the
//! renderer draws the resulting chunk poses. This crate never imports arena.
//!
//! Units: chunk geometry is meters, velocities are m/s, [`HitWake::pos`] is
//! centimeters, [`HitWake::impulse_mpermille`] is permille of shard impulse
//! units (`impulse = value / 1000.0`), ticks are match ticks, time is seconds.
//!
//! Determinism: `bake` is a pure function of `(chunks, seed)`; `wake` iterates
//! chunk indices in order with IEEE-754 `f32` math (`+ - * /`, `sqrt`) only,
//! so the same bake plus the same wake/step script replays bit-identically.
//! Sleep/wake transitions, poses, velocities and the last applied tick all
//! feed [`DebrisField::state_hash`]. Invalid input returns errors or empty
//! wake lists; it never panics and never mutates the field.

use crate::bake::BREAKING_THRESHOLD;
use crate::bake3d::{bake_bytes3, Chunk3};
use crate::shards3d::{in_contact, Runtime3};
use engine_core::Fnv64;

/// Largest debris set one field holds; bounds memory up front.
pub const MAX_DEBRIS_CHUNKS: usize = 1024;
/// Hit positions beyond +-100 km are outside any arena; rejected as invalid.
pub const MAX_WAKE_CM: i32 = 10_000_000;
/// Wake speeds clamp here so absurd impulses stay finite; m/s.
pub const MAX_WAKE_SPEED: f32 = 128.0;
/// Upper bound for `step` dt; larger or non-positive steps are ignored; seconds.
pub const MAX_STEP_DT: f32 = 1.0;
/// Largest accepted chunk center coordinate; meters.
const MAX_CENTER_M: f32 = 10_000.0;
/// Accepted half-extent range; meters.
const MIN_HALF_EXTENT_M: f32 = 1e-4;
const MAX_HALF_EXTENT_M: f32 = 10_000.0;
/// Accepted chunk mass range; mass units.
const MIN_MASS: f32 = 1e-6;
const MAX_MASS: f32 = 1e12;
/// Accepted break-threshold range; impulse units per mass unit.
const MAX_BREAK_THRESHOLD: f32 = 1e6;
/// Fallback wake direction (1/sqrt(2)) when the hit lands exactly on a chunk
/// center; a shared constant keeps the wake path free of float-math calls.
const INV_SQRT2: f32 = core::f32::consts::FRAC_1_SQRT_2;

/// Typed hit event defined here; the arena maps combat hits into it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitWake {
    /// Hit point in world, centimeters, each within `+-MAX_WAKE_CM`.
    pub pos: [i32; 3],
    /// Impulse permille; applied impulse is `value / 1000.0`.
    /// Every `u32` is valid input; tiny values just stay sub-threshold.
    pub impulse_mpermille: u32,
    /// Match tick of the hit; recorded on fracture for replay hashes.
    pub tick: u64,
}

impl HitWake {
    /// Range-checks the hit position; impulse and tick accept any value.
    pub fn validate(&self) -> Result<(), DebrisError> {
        for c in self.pos {
            if !(-MAX_WAKE_CM..=MAX_WAKE_CM).contains(&c) {
                return Err(DebrisError::BadWakePosition);
            }
        }
        Ok(())
    }
}

/// One newly woken shard; index into the baked chunk list plus its wake
/// velocity in m/s for the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WokeShard {
    /// Chunk index in the baked chunk list.
    pub id: usize,
    /// Wake velocity assigned by the hit; m/s.
    pub vel: [f32; 3],
}

/// Pre-broken prop descriptor: shipped chunks plus the impulse-per-mass
/// threshold that fractures them. Fracture happens only through
/// [`DebrisField::wake`]; construction leaves every shard asleep.
#[derive(Clone, Debug)]
pub struct Prebroken {
    /// Shipped chunk boxes; 1..=`MAX_DEBRIS_CHUNKS`, finite, positive extents.
    pub chunks: Vec<Chunk3>,
    /// Impulse per unit mass above which a hit fractures; finite and positive.
    pub break_threshold: f32,
}

impl Prebroken {
    /// Checks chunk bounds and the threshold without mutating anything.
    pub fn validate(&self) -> Result<(), DebrisError> {
        check_chunks(&self.chunks)?;
        if !self.break_threshold.is_finite()
            || self.break_threshold <= 0.0
            || self.break_threshold > MAX_BREAK_THRESHOLD
        {
            return Err(DebrisError::BadThreshold);
        }
        Ok(())
    }
}

/// Offline bake output: chunk geometry as given, the bake seed, and the
/// canonical hash over both. `bake` never fails; runtime construction
/// validates.
#[derive(Clone, Debug)]
pub struct BakedDebris {
    /// Chunk boxes in input order.
    pub chunks: Vec<Chunk3>,
    /// Bake seed mixed into `hash`; same chunks with another seed differ.
    pub seed: u64,
    /// Canonical hash over `(seed, chunks)`.
    pub hash: u64,
}

/// Typed validation failures; external input maps here, never to a panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebrisError {
    /// No chunks supplied.
    EmptyChunks,
    /// More than `MAX_DEBRIS_CHUNKS` chunks supplied.
    TooManyChunks,
    /// Chunk at the given index is non-finite, misplaced or degenerate.
    BadChunk(usize),
    /// Threshold is non-finite, non-positive or absurd.
    BadThreshold,
    /// Hit position lies outside `+-MAX_WAKE_CM`.
    BadWakePosition,
}

impl core::fmt::Display for DebrisError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DebrisError::EmptyChunks => write!(f, "debris needs at least one chunk"),
            DebrisError::TooManyChunks => write!(f, "debris exceeds MAX_DEBRIS_CHUNKS"),
            DebrisError::BadChunk(i) => write!(f, "debris chunk {i} is out of bounds"),
            DebrisError::BadThreshold => write!(f, "break threshold is out of bounds"),
            DebrisError::BadWakePosition => write!(f, "wake position is out of bounds"),
        }
    }
}

impl std::error::Error for DebrisError {}

/// Runtime debris field: an asleep [`Runtime3`] plus its fracture threshold,
/// bake identity and last fracture tick. Wakes happen only via [`HitWake`];
/// there is no wake-by-id API, so pre-broken props cannot fracture any other
/// way. Sleeping shards never move, so the wake set fully decides which state
/// can change each tick.
#[derive(Clone, Debug)]
pub struct DebrisField {
    runtime: Runtime3,
    threshold: f32,
    seed: u64,
    bake_hash: u64,
    last_tick: u64,
}

impl DebrisField {
    /// Pure offline bake of already-fractured chunks: stores geometry in input
    /// order and hashes `(seed, chunks)`. Same inputs give the same output.
    pub fn bake(chunks: &[Chunk3], seed: u64) -> BakedDebris {
        let mut bytes = seed.to_le_bytes().to_vec();
        bytes.extend_from_slice(&bake_bytes3(chunks));
        BakedDebris {
            chunks: chunks.to_vec(),
            seed,
            hash: engine_core::hash_bytes(&bytes),
        }
    }

    /// Builds an asleep field from baked debris with the default
    /// `BREAKING_THRESHOLD`; validates chunks, never panics.
    pub fn from_baked(baked: &BakedDebris) -> Result<Self, DebrisError> {
        check_chunks(&baked.chunks)?;
        Ok(Self {
            runtime: Runtime3::from_chunks(baked.chunks.clone()),
            threshold: BREAKING_THRESHOLD,
            seed: baked.seed,
            bake_hash: baked.hash,
            last_tick: 0,
        })
    }

    /// Builds an asleep field from a pre-broken prop with its own threshold;
    /// validates everything, never panics.
    pub fn from_prebroken(prop: &Prebroken) -> Result<Self, DebrisError> {
        prop.validate()?;
        let baked = Self::bake(&prop.chunks, 0);
        Ok(Self {
            runtime: Runtime3::from_chunks(baked.chunks),
            threshold: prop.break_threshold,
            seed: 0,
            bake_hash: baked.hash,
            last_tick: 0,
        })
    }

    /// Applies a hit: finds the nearest chunk to the hit point in index order
    /// (first wins ties), gates on `impulse > mass * threshold`, then wakes
    /// that chunk plus its AABB-contact neighbors with hit-away velocities.
    /// Sub-threshold and invalid hits return an empty list and change nothing,
    /// so no-op wakes leave the state hash bit-identical.
    pub fn wake(&mut self, event: HitWake) -> Vec<WokeShard> {
        if event.validate().is_err() || self.runtime.is_empty() {
            return Vec::new();
        }
        let hit = [
            event.pos[0] as f32 / 100.0,
            event.pos[1] as f32 / 100.0,
            event.pos[2] as f32 / 100.0,
        ];
        let mut best = 0usize;
        let mut best_d2 = f32::MAX;
        for (i, c) in self.runtime.chunks.iter().enumerate() {
            let dx = c.x - hit[0];
            let dy = c.y - hit[1];
            let dz = c.z - hit[2];
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 < best_d2 {
                best_d2 = d2;
                best = i;
            }
        }
        let impulse = event.impulse_mpermille as f32 / 1000.0;
        if impulse <= self.runtime.chunks[best].mass * self.threshold {
            return Vec::new();
        }
        let mut newly = Vec::new();
        newly.push(best);
        for k in 0..self.runtime.len() {
            if k != best
                && !self.runtime.is_awake(k)
                && in_contact(&self.runtime.chunks[best], &self.runtime.chunks[k])
            {
                newly.push(k);
            }
        }
        newly.sort_unstable();
        newly.dedup();
        let mut out = Vec::with_capacity(newly.len());
        for id in newly {
            if self.runtime.is_awake(id) {
                continue;
            }
            let c = &self.runtime.chunks[id];
            let d = [c.x - hit[0], c.y - hit[1], c.z - hit[2]];
            let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let dir = if d2.is_finite() && d2 > 1e-12 {
                let inv = 1.0 / d2.sqrt();
                [d[0] * inv, d[1] * inv, d[2] * inv]
            } else {
                [INV_SQRT2, INV_SQRT2, 0.0]
            };
            // Validated mass is finite and positive, so speed is finite.
            let mut speed = impulse / c.mass;
            if speed > MAX_WAKE_SPEED {
                speed = MAX_WAKE_SPEED;
            }
            if id != best {
                speed *= 0.5;
            }
            let vel = [dir[0] * speed, dir[1] * speed, dir[2] * speed];
            self.runtime.wake(id);
            self.runtime.vel[id] = vel;
            out.push(WokeShard { id, vel });
        }
        if !out.is_empty() {
            self.last_tick = event.tick;
        }
        out
    }

    /// Integrates awake shards (gravity plus ground clamp); non-finite,
    /// non-positive or over-long steps are ignored without panicking.
    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 || dt > MAX_STEP_DT {
            return;
        }
        self.runtime.step(dt);
    }

    /// Sleeps one shard and zeroes its velocity; out-of-range ids do nothing.
    pub fn sleep(&mut self, id: usize) {
        self.runtime.sleep(id);
    }

    /// Sleeps awake shards slower than `vmax` m/s; invalid bounds do nothing.
    pub fn sleep_slow(&mut self, vmax: f32) {
        if !vmax.is_finite() || vmax < 0.0 {
            return;
        }
        self.runtime.sleep_slow(vmax);
    }

    /// FNV-1a64 over the runtime hash plus seed, threshold, bake hash and last
    /// fracture tick; index order and bit-exact floats keep reruns identical.
    /// Sleep/wake transitions change the runtime mask, trajectories change
    /// poses and velocities, so both move this hash.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        let inner = self.runtime.state_hash();
        h.write_u64(inner);
        h.write_u64(self.seed);
        h.write_u32(self.threshold.to_bits());
        h.write_u64(self.bake_hash);
        h.write_u64(self.last_tick);
        h.finish()
    }

    /// Chunk count.
    pub fn len(&self) -> usize {
        self.runtime.len()
    }
    /// True when the field holds no chunks.
    pub fn is_empty(&self) -> bool {
        self.runtime.is_empty()
    }
    /// Awake shard count.
    pub fn awake_count(&self) -> usize {
        self.runtime.awake_count()
    }
    /// Awake shard ids in index order.
    pub fn awake_ids(&self) -> Vec<usize> {
        self.runtime.awake_ids()
    }
    /// True when the given shard is awake; out-of-range ids read asleep.
    pub fn is_awake(&self, id: usize) -> bool {
        self.runtime.is_awake(id)
    }
    /// Baked chunk boxes in index order.
    pub fn chunks(&self) -> &[Chunk3] {
        &self.runtime.chunks
    }
    /// Per-shard velocities in index order; m/s.
    pub fn velocities(&self) -> &[[f32; 3]] {
        &self.runtime.vel
    }
    /// Impulse-per-mass fracture threshold of this field.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }
    /// Bake seed this field was built from.
    pub fn seed(&self) -> u64 {
        self.seed
    }
    /// Canonical hash of the baked debris this field was built from.
    pub fn bake_hash(&self) -> u64 {
        self.bake_hash
    }
    /// Tick of the last fracturing hit; 0 before the first wake.
    pub fn last_tick(&self) -> u64 {
        self.last_tick
    }
}

fn check_chunks(chunks: &[Chunk3]) -> Result<(), DebrisError> {
    if chunks.is_empty() {
        return Err(DebrisError::EmptyChunks);
    }
    if chunks.len() > MAX_DEBRIS_CHUNKS {
        return Err(DebrisError::TooManyChunks);
    }
    for (i, c) in chunks.iter().enumerate() {
        let finite = c.x.is_finite()
            && c.y.is_finite()
            && c.z.is_finite()
            && c.hx.is_finite()
            && c.hy.is_finite()
            && c.hz.is_finite()
            && c.mass.is_finite();
        let placed =
            c.x.abs() <= MAX_CENTER_M && c.y.abs() <= MAX_CENTER_M && c.z.abs() <= MAX_CENTER_M;
        let shaped = c.hx >= MIN_HALF_EXTENT_M
            && c.hx <= MAX_HALF_EXTENT_M
            && c.hy >= MIN_HALF_EXTENT_M
            && c.hy <= MAX_HALF_EXTENT_M
            && c.hz >= MIN_HALF_EXTENT_M
            && c.hz <= MAX_HALF_EXTENT_M;
        let massed = c.mass >= MIN_MASS && c.mass <= MAX_MASS;
        if !finite || !placed || !shaped || !massed {
            return Err(DebrisError::BadChunk(i));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake3d::fracture3;

    fn chunks() -> Vec<Chunk3> {
        fracture3(0.0, 5.0, 0.0, 6.0, 4.0, 3.0, 8, 11)
    }

    fn big_hit(field: &DebrisField, tick: u64) -> HitWake {
        let c = &field.chunks()[0];
        HitWake {
            pos: [
                (c.x * 100.0) as i32,
                (c.y * 100.0) as i32,
                (c.z * 100.0) as i32,
            ],
            impulse_mpermille: u32::MAX,
            tick,
        }
    }

    #[test]
    fn state_hash_is_fnv1a_over_the_canonical_word_stream() {
        let baked = DebrisField::bake(&chunks(), 0x0123_4567_89ab_cdef);
        let mut field = DebrisField::from_baked(&baked).expect("valid bake");
        assert!(!field.wake(big_hit(&field, 0x1_0000_0003)).is_empty());
        field.step(1.0 / 60.0);
        // Oracle: runtime hash, seed, threshold bits, bake hash and last tick,
        // each u64 as low then high u32, as plain LE bytes.
        let inner = field.runtime.state_hash();
        let mut words: Vec<u32> = vec![inner as u32, (inner >> 32) as u32];
        words.extend([field.seed as u32, (field.seed >> 32) as u32]);
        words.push(field.threshold.to_bits());
        words.extend([field.bake_hash as u32, (field.bake_hash >> 32) as u32]);
        words.extend([field.last_tick as u32, (field.last_tick >> 32) as u32]);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(field.last_tick, 0x1_0000_0003, "the high word is covered");
        assert_eq!(field.state_hash(), engine_core::hash_bytes(&bytes));
    }

    fn script() -> (u64, Vec<u32>, Vec<u32>) {
        let baked = DebrisField::bake(&chunks(), 7);
        let mut field = DebrisField::from_baked(&baked).expect("valid bake");
        let first = field.wake(big_hit(&field, 3));
        assert!(!first.is_empty());
        for _ in 0..30 {
            field.step(1.0 / 60.0);
        }
        let second = field.wake(HitWake {
            pos: [0, 500, 0],
            impulse_mpermille: u32::MAX,
            tick: 33,
        });
        assert!(!second.is_empty());
        for _ in 0..30 {
            field.step(1.0 / 60.0);
        }
        field.sleep_slow(0.05);
        let pos_bits: Vec<u32> = field
            .chunks()
            .iter()
            .flat_map(|c| [c.x.to_bits(), c.y.to_bits(), c.z.to_bits()])
            .collect();
        let vel_bits: Vec<u32> = field
            .velocities()
            .iter()
            .flat_map(|v| [v[0].to_bits(), v[1].to_bits(), v[2].to_bits()])
            .collect();
        (field.state_hash(), pos_bits, vel_bits)
    }

    #[test]
    fn bake_twice_identical_and_seed_matters() {
        let c = chunks();
        let a = DebrisField::bake(&c, 7);
        let b = DebrisField::bake(&c, 7);
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.seed, b.seed);
        assert_eq!(a.chunks.len(), b.chunks.len());
        for (x, y) in a.chunks.iter().zip(b.chunks.iter()) {
            assert_eq!(
                (
                    x.x.to_bits(),
                    x.y.to_bits(),
                    x.z.to_bits(),
                    x.hx.to_bits(),
                    x.hy.to_bits(),
                    x.hz.to_bits(),
                    x.mass.to_bits()
                ),
                (
                    y.x.to_bits(),
                    y.y.to_bits(),
                    y.z.to_bits(),
                    y.hx.to_bits(),
                    y.hy.to_bits(),
                    y.hz.to_bits(),
                    y.mass.to_bits()
                )
            );
        }
        let other = DebrisField::bake(&c, 8);
        assert_ne!(a.hash, other.hash);
    }

    #[test]
    fn wake_script_replays_identical_trajectories() {
        let (h1, p1, v1) = script();
        let (h2, p2, v2) = script();
        assert_eq!(h1, h2);
        assert_eq!(p1, p2);
        assert_eq!(v1, v2);
    }

    #[test]
    fn sub_threshold_wakes_nothing_and_hash_stable() {
        let baked = DebrisField::bake(&chunks(), 7);
        let mut field = DebrisField::from_baked(&baked).expect("valid bake");
        let before = field.state_hash();
        let woken = field.wake(HitWake {
            pos: [0, 500, 0],
            impulse_mpermille: 1,
            tick: 9,
        });
        assert!(woken.is_empty());
        assert_eq!(field.awake_count(), 0);
        assert_eq!(field.last_tick(), 0);
        assert_eq!(field.state_hash(), before);
    }

    #[test]
    fn sleep_wake_accounted_in_state_hash() {
        let baked = DebrisField::bake(&chunks(), 7);
        let mut field = DebrisField::from_baked(&baked).expect("valid bake");
        let asleep = field.state_hash();
        let woken = field.wake(big_hit(&field, 4));
        assert!(!woken.is_empty());
        let awake = field.state_hash();
        assert_ne!(asleep, awake);
        assert_eq!(field.last_tick(), 4);
        for w in &woken {
            assert!(field.is_awake(w.id));
            assert_eq!(field.velocities()[w.id], w.vel);
        }
        for id in field.awake_ids() {
            field.sleep(id);
        }
        assert_eq!(field.awake_count(), 0);
        let reslept = field.state_hash();
        assert_ne!(awake, reslept);
        assert_ne!(asleep, reslept);
    }

    #[test]
    fn invalid_wake_input_rejected_without_panic() {
        let baked = DebrisField::bake(&chunks(), 7);
        let mut field = DebrisField::from_baked(&baked).expect("valid bake");
        let before = field.state_hash();
        for pos in [
            [i32::MAX, 0, 0],
            [0, i32::MIN, 0],
            [0, 0, MAX_WAKE_CM + 1],
            [-MAX_WAKE_CM - 1, 0, 0],
        ] {
            let woken = field.wake(HitWake {
                pos,
                impulse_mpermille: u32::MAX,
                tick: u64::MAX,
            });
            assert!(woken.is_empty(), "pos {pos:?}");
        }
        assert_eq!(field.awake_count(), 0);
        assert_eq!(field.state_hash(), before);
        assert!(HitWake {
            pos: [0, 0, i32::MAX],
            impulse_mpermille: 0,
            tick: 0
        }
        .validate()
        .is_err());
    }

    #[test]
    fn invalid_construction_rejected_without_panic() {
        assert_eq!(
            DebrisField::from_baked(&DebrisField::bake(&[], 1)).unwrap_err(),
            DebrisError::EmptyChunks
        );
        assert_eq!(
            Prebroken {
                chunks: Vec::new(),
                break_threshold: 3.0
            }
            .validate()
            .unwrap_err(),
            DebrisError::EmptyChunks
        );
        for threshold in [
            0.0,
            -2.0,
            f32::NAN,
            f32::INFINITY,
            MAX_BREAK_THRESHOLD + 1.0,
        ] {
            assert_eq!(
                Prebroken {
                    chunks: chunks(),
                    break_threshold: threshold
                }
                .validate()
                .unwrap_err(),
                DebrisError::BadThreshold,
                "threshold {threshold}"
            );
        }
        let mut bad = chunks();
        bad[2].hx = f32::NAN;
        assert_eq!(
            Prebroken {
                chunks: bad,
                break_threshold: 3.0
            }
            .validate()
            .unwrap_err(),
            DebrisError::BadChunk(2)
        );
        let mut flat = chunks();
        flat[0].hx = 0.0;
        assert_eq!(
            DebrisField::from_baked(&DebrisField::bake(&flat, 1)).unwrap_err(),
            DebrisError::BadChunk(0)
        );
        let oversized = vec![chunks()[0]; MAX_DEBRIS_CHUNKS + 1];
        assert_eq!(
            DebrisField::from_baked(&DebrisField::bake(&oversized, 1)).unwrap_err(),
            DebrisError::TooManyChunks
        );
    }

    #[test]
    fn prebroken_starts_asleep_and_fractures_only_through_wake() {
        let prop = Prebroken {
            chunks: chunks(),
            break_threshold: 2.5,
        };
        let mut field = DebrisField::from_prebroken(&prop).expect("valid prop");
        assert_eq!(field.awake_count(), 0);
        assert!((field.threshold() - 2.5).abs() < f32::EPSILON);
        field.step(0.5);
        assert_eq!(field.awake_count(), 0);
        for v in field.velocities() {
            assert_eq!(*v, [0.0, 0.0, 0.0]);
        }
        let woken = field.wake(big_hit(&field, 1));
        assert!(!woken.is_empty());
    }

    #[test]
    fn bad_steps_and_sleep_bounds_do_nothing() {
        let baked = DebrisField::bake(&chunks(), 7);
        let mut field = DebrisField::from_baked(&baked).expect("valid bake");
        field.wake(big_hit(&field, 1));
        field.step(1.0 / 60.0);
        let before = field.state_hash();
        for dt in [0.0, -0.5, f32::NAN, f32::INFINITY, MAX_STEP_DT + 1.0] {
            field.step(dt);
        }
        field.sleep(usize::MAX);
        field.sleep_slow(f32::NAN);
        field.sleep_slow(-1.0);
        assert_eq!(field.state_hash(), before);
    }

    #[test]
    fn woke_velocities_finite_bounded_and_deterministic() {
        let run = || {
            let baked = DebrisField::bake(&chunks(), 7);
            let mut field = DebrisField::from_baked(&baked).expect("valid bake");
            field.wake(big_hit(&field, 2))
        };
        let (a, b) = (run(), run());
        assert_eq!(a.len(), b.len());
        assert!(!a.is_empty());
        for (x, y) in a.iter().zip(b.iter()) {
            assert_eq!(x.id, y.id);
            assert_eq!(
                (x.vel[0].to_bits(), x.vel[1].to_bits(), x.vel[2].to_bits()),
                (y.vel[0].to_bits(), y.vel[1].to_bits(), y.vel[2].to_bits())
            );
            let speed = (x.vel[0] * x.vel[0] + x.vel[1] * x.vel[1] + x.vel[2] * x.vel[2]).sqrt();
            assert!(speed.is_finite() && speed <= MAX_WAKE_SPEED + 1e-3);
        }
    }
}

//! limb: a severed limb as a body: detach at the cut tick, mass from volume
//! times density, velocity inherited from the parent bone plus the blade
//! impulse, equal-and-opposite recoil handed back for the stump servo.
//!
//! Consumer: the arena match step spawns one [`LimbSpawn`] per cut event
//! (see `engine_combat::Cut`) and steps the [`LimbPool`] once per tick; the
//! debris ledger frees the debris the spawn report asks for; the snapshot
//! bytes join the rollback snapshot. Data owner: `engine-physics`. Units:
//! positions in micrometers, velocities in mm/s, angles in micro-radians,
//! spin in milli-rad/s, mass in grams, impulse in milli-newton-seconds, time
//! in fixed 60 Hz ticks. Determinism scope: integer math only (no float, so
//! no NaN and no platform drift); spawns in a batch run in canonical
//! (swing, victim, joint) order; eviction picks by spawn id; the hash is
//! FNV-1a over the snapshot bytes. Errors: [`LimbError`] for bad specs,
//! caps and snapshot bytes; nothing panics on user data. Budget: spawn and
//! step touch at most [`MAX_LIMBS`] bodies, no allocation after
//! construction (the pool is pre-sized), so a 64-fighter siege costs a few
//! microseconds per tick.

use engine_core::{Fnv64, Rng};

/// Most limb bodies one pool holds.
pub const MAX_LIMBS: usize = 256;
/// Wire version of the snapshot bytes.
pub const LIMB_SNAPSHOT_V1: u32 = 1;
/// Fixed step, 60 Hz.
pub const TICKS_PER_SECOND: i64 = 60;
/// Gravity per tick in mm/s (about 9.8 m/s^2 at 60 Hz).
pub const GRAVITY_MMPS_PER_TICK: i32 = 163;
/// Fastest a limb may fly, mm/s (40 m/s).
pub const MAX_SPEED_MMPS: i32 = 40_000;
/// Largest accepted volume, cm^3 (a whole leg is about 12,000).
pub const MAX_VOLUME_CM3: u32 = 100_000;
/// Largest accepted density, kg/m^3.
pub const MAX_DENSITY_KG_M3: u32 = 20_000;
/// Largest accepted contact radius, mm.
pub const MAX_RADIUS_MM: u32 = 500;
/// Largest accepted position component, mm (200 m).
pub const MAX_POS_MM: i32 = 200_000;
/// Ticks a limb must lie still on the ground before it sleeps.
pub const SLEEP_TICKS: u16 = 30;
/// Speed under which a grounded limb counts as still, mm/s.
const STILL_MMPS: i32 = 40;
/// Largest spin a spawn draws, milli-rad/s (12 rad/s).
const SPIN_MAX_MRADPS: i64 = 12_000;
/// Draws per spin axis: `-SPIN_MAX_MRADPS..=SPIN_MAX_MRADPS`.
const SPIN_SPAN: u64 = 24_001;
/// Full turn in micro-radians.
const TURN_URAD: i32 = 6_283_185;

/// Why a limb request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimbError {
    /// Body cap zero or past [`MAX_LIMBS`].
    BadCap(usize),
    /// Volume or density is zero or past its bound.
    BadVolume,
    /// Position, radius or impulse is out of bounds.
    BadSpawn,
    /// Snapshot bytes are truncated, versioned wrong or out of bounds.
    BadSnapshot,
}

impl core::fmt::Display for LimbError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadCap(c) => write!(f, "limb cap {c} is out of range"),
            Self::BadVolume => write!(f, "limb volume or density is out of range"),
            Self::BadSpawn => write!(f, "limb spawn is out of range"),
            Self::BadSnapshot => write!(f, "limb snapshot bytes are not valid"),
        }
    }
}

impl std::error::Error for LimbError {}

/// One cut turned into a limb request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LimbSpawn {
    /// Swing sequence number of the cut (canonical order key, first).
    pub swing: u32,
    /// Victim slot (second order key).
    pub victim: u8,
    /// Severed joint wire value (third order key).
    pub joint: u8,
    /// Where the limb leaves the body, mm.
    pub origin_mm: [i32; 3],
    /// Velocity of the parent bone at the cut tick, mm/s.
    pub parent_vel_mmps: [i32; 3],
    /// Blade impulse into the limb, milli-newton-seconds.
    pub impulse_mns: [i32; 3],
    /// Volume of the severed chain, cm^3.
    pub volume_cm3: u32,
    /// Tissue density, kg/m^3 (water is 1000).
    pub density_kg_m3: u32,
    /// Contact radius against the ground, mm.
    pub radius_mm: u32,
}

impl LimbSpawn {
    /// Range-checks the request.
    pub fn validate(&self) -> Result<(), LimbError> {
        if self.volume_cm3 == 0
            || self.volume_cm3 > MAX_VOLUME_CM3
            || self.density_kg_m3 == 0
            || self.density_kg_m3 > MAX_DENSITY_KG_M3
        {
            return Err(LimbError::BadVolume);
        }
        let pos_ok = self.origin_mm.iter().all(|p| p.abs() <= MAX_POS_MM);
        let speed_ok = self
            .parent_vel_mmps
            .iter()
            .all(|v| v.abs() <= MAX_SPEED_MMPS);
        let impulse_ok = self.impulse_mns.iter().all(|i| i.abs() <= 1_000_000_000);
        if !pos_ok
            || !speed_ok
            || !impulse_ok
            || self.radius_mm == 0
            || self.radius_mm > MAX_RADIUS_MM
        {
            return Err(LimbError::BadSpawn);
        }
        Ok(())
    }

    /// Mass in grams: volume times density, at least one gram.
    #[must_use]
    pub fn mass_g(&self) -> u32 {
        let grams = u64::from(self.volume_cm3) * u64::from(self.density_kg_m3) / 1000;
        u32::try_from(grams.max(1)).unwrap_or(u32::MAX)
    }
}

/// The reaction the stump takes: equal and opposite to the blade impulse
/// into the limb; the stump servo applies it to the victim's ragdoll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recoil {
    /// Victim slot.
    pub victim: u8,
    /// Severed joint wire value.
    pub joint: u8,
    /// Impulse into the stump, milli-newton-seconds.
    pub impulse_mns: [i32; 3],
}

/// What one spawn did to the shared body budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpawnReport {
    /// Id of the new limb (spawn sequence number).
    pub id: u32,
    /// Stump reaction for the victim.
    pub recoil: Recoil,
    /// Older limbs evicted to make room.
    pub evicted_limbs: u32,
    /// Debris bodies the caller must free so limbs plus debris fit the cap:
    /// limbs outrank debris, so the ledger gives way, never the cut.
    pub debris_to_free: u32,
}

/// One limb body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limb {
    /// Spawn sequence number; the canonical order and eviction key.
    pub id: u32,
    /// Victim slot the limb came from.
    pub victim: u8,
    /// Severed joint wire value.
    pub joint: u8,
    /// Mass, grams.
    pub mass_g: u32,
    /// Contact radius, mm.
    pub radius_mm: u32,
    /// Position, micrometers.
    pub pos_um: [i32; 3],
    /// Velocity, mm/s.
    pub vel_mmps: [i32; 3],
    /// Orientation as three turn angles, micro-radians in `(-pi, pi]`.
    pub angle_urad: [i32; 3],
    /// Spin, milli-rad/s.
    pub spin_mradps: [i32; 3],
    /// Ticks lying still on the ground.
    pub still_ticks: u16,
    /// Asleep limbs skip integration until evicted.
    pub asleep: bool,
}

/// Fixed-size pool of limb bodies, spawn order, capped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimbPool {
    limbs: Vec<Limb>,
    cap: usize,
    next_id: u32,
    tick: u32,
}

impl LimbPool {
    /// New pool holding at most `cap` limbs together with the debris the
    /// caller reports; `1..=MAX_LIMBS`.
    pub fn new(cap: usize) -> Result<Self, LimbError> {
        if cap == 0 || cap > MAX_LIMBS {
            return Err(LimbError::BadCap(cap));
        }
        Ok(Self {
            limbs: Vec::with_capacity(cap),
            cap,
            next_id: 0,
            tick: 0,
        })
    }

    /// Limbs alive.
    #[must_use]
    pub fn len(&self) -> usize {
        self.limbs.len()
    }

    /// True when no limb is alive.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.limbs.is_empty()
    }

    /// The shared body cap (limbs plus debris).
    #[must_use]
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Ticks stepped.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Limbs in spawn order.
    #[must_use]
    pub fn limbs(&self) -> &[Limb] {
        &self.limbs
    }

    /// Spawns one limb. Evicts the oldest sleeping limb (else the oldest)
    /// until limbs plus `debris_live` fit the cap; when debris alone fills
    /// the cap the report says how many debris bodies to free.
    pub fn spawn(
        &mut self,
        spec: &LimbSpawn,
        debris_live: usize,
    ) -> Result<SpawnReport, LimbError> {
        spec.validate()?;
        let mut evicted = 0u32;
        // Room for the new limb: len + 1 + debris <= cap.
        while !self.limbs.is_empty() && self.limbs.len() + 1 + debris_live > self.cap {
            let at = self.limbs.iter().position(|l| l.asleep).unwrap_or(0);
            self.limbs.remove(at);
            evicted += 1;
        }
        let over = (self.limbs.len() + 1 + debris_live).saturating_sub(self.cap);
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mass = i64::from(spec.mass_g());
        let mut vel = [0i32; 3];
        for (axis, slot) in vel.iter_mut().enumerate() {
            let dv = i64::from(spec.impulse_mns[axis]) * 1000 / mass;
            let v = i64::from(spec.parent_vel_mmps[axis]) + dv;
            *slot = clamp_speed(v);
        }
        let mut rng = spin_rng(spec);
        let mut spin = [0i32; 3];
        for s in &mut spin {
            let draw = i64::try_from(rng.next_range(0, SPIN_SPAN)).unwrap_or(0);
            *s = i32::try_from(draw - SPIN_MAX_MRADPS).unwrap_or(0);
        }
        self.limbs.push(Limb {
            id,
            victim: spec.victim,
            joint: spec.joint,
            mass_g: spec.mass_g(),
            radius_mm: spec.radius_mm,
            pos_um: [
                spec.origin_mm[0].saturating_mul(1000),
                spec.origin_mm[1].saturating_mul(1000),
                spec.origin_mm[2].saturating_mul(1000),
            ],
            vel_mmps: vel,
            angle_urad: [0; 3],
            spin_mradps: spin,
            still_ticks: 0,
            asleep: false,
        });
        Ok(SpawnReport {
            id,
            recoil: Recoil {
                victim: spec.victim,
                joint: spec.joint,
                impulse_mns: [
                    spec.impulse_mns[0].saturating_neg(),
                    spec.impulse_mns[1].saturating_neg(),
                    spec.impulse_mns[2].saturating_neg(),
                ],
            },
            evicted_limbs: evicted,
            debris_to_free: u32::try_from(over).unwrap_or(u32::MAX),
        })
    }

    /// Spawns a batch in canonical (swing, victim, joint) order, whatever
    /// order the caller built it in. Returns one report per spawn in that
    /// order; the first refusal stops the batch (earlier spawns stay).
    pub fn spawn_batch(
        &mut self,
        batch: &mut [LimbSpawn],
        debris_live: usize,
    ) -> Result<Vec<SpawnReport>, LimbError> {
        batch.sort_by_key(|s| (s.swing, s.victim, s.joint));
        let mut reports = Vec::with_capacity(batch.len());
        for spec in batch.iter() {
            reports.push(self.spawn(spec, debris_live)?);
        }
        Ok(reports)
    }

    /// One fixed tick over every awake limb in spawn order: gravity, move,
    /// ground contact at `y = radius`, ground friction and sleep.
    pub fn step(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        for limb in &mut self.limbs {
            if limb.asleep {
                continue;
            }
            limb.vel_mmps[1] = limb.vel_mmps[1].saturating_sub(GRAVITY_MMPS_PER_TICK);
            for axis in 0..3 {
                let moved = i64::from(limb.vel_mmps[axis]) * 1000 / TICKS_PER_SECOND;
                limb.pos_um[axis] = clamp_i32(i64::from(limb.pos_um[axis]) + moved);
                let turn = i64::from(limb.spin_mradps[axis]) * 1000 / TICKS_PER_SECOND;
                limb.angle_urad[axis] = wrap_angle(i64::from(limb.angle_urad[axis]) + turn);
            }
            let floor_um = i32::try_from(limb.radius_mm)
                .unwrap_or(0)
                .saturating_mul(1000);
            let mut grounded = false;
            if limb.pos_um[1] <= floor_um {
                limb.pos_um[1] = floor_um;
                grounded = true;
                if limb.vel_mmps[1] < 0 {
                    // Quarter bounce; the bounce dies once it is tiny.
                    limb.vel_mmps[1] = -limb.vel_mmps[1] / 4;
                    if limb.vel_mmps[1] < STILL_MMPS {
                        limb.vel_mmps[1] = 0;
                    }
                }
                for axis in [0, 2] {
                    limb.vel_mmps[axis] = limb.vel_mmps[axis] * 7 / 8;
                }
                for s in &mut limb.spin_mradps {
                    *s = *s * 7 / 8;
                }
            }
            let slow = limb.vel_mmps.iter().all(|v| v.abs() <= STILL_MMPS)
                && limb.spin_mradps.iter().all(|s| s.abs() <= 400);
            if grounded && slow {
                limb.still_ticks = limb.still_ticks.saturating_add(1);
                if limb.still_ticks >= SLEEP_TICKS {
                    limb.asleep = true;
                    limb.vel_mmps = [0; 3];
                    limb.spin_mradps = [0; 3];
                }
            } else {
                limb.still_ticks = 0;
            }
        }
    }

    /// Canonical bytes of the whole pool: version, cap, next id, tick,
    /// count, then every limb in spawn order.
    #[must_use]
    pub fn snapshot(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(20 + self.limbs.len() * 68);
        out.extend_from_slice(&LIMB_SNAPSHOT_V1.to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.cap).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(&self.next_id.to_le_bytes());
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.limbs.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for l in &self.limbs {
            out.extend_from_slice(&l.id.to_le_bytes());
            out.push(l.victim);
            out.push(l.joint);
            out.push(u8::from(l.asleep));
            out.push(0);
            out.extend_from_slice(&l.mass_g.to_le_bytes());
            out.extend_from_slice(&l.radius_mm.to_le_bytes());
            for v in l
                .pos_um
                .iter()
                .chain(l.vel_mmps.iter())
                .chain(l.angle_urad.iter())
                .chain(l.spin_mradps.iter())
            {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&l.still_ticks.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        }
        out
    }

    /// Rebuilds a pool from [`Self::snapshot`] bytes; everything is
    /// bounds-checked, so hostile bytes refuse instead of panicking.
    pub fn restore(bytes: &[u8]) -> Result<Self, LimbError> {
        let mut at = 0usize;
        let version = take_u32(bytes, &mut at)?;
        if version != LIMB_SNAPSHOT_V1 {
            return Err(LimbError::BadSnapshot);
        }
        let cap = usize::try_from(take_u32(bytes, &mut at)?).map_err(|_| LimbError::BadSnapshot)?;
        let mut pool = Self::new(cap).map_err(|_| LimbError::BadSnapshot)?;
        pool.next_id = take_u32(bytes, &mut at)?;
        pool.tick = take_u32(bytes, &mut at)?;
        let count =
            usize::try_from(take_u32(bytes, &mut at)?).map_err(|_| LimbError::BadSnapshot)?;
        if count > cap {
            return Err(LimbError::BadSnapshot);
        }
        for _ in 0..count {
            let id = take_u32(bytes, &mut at)?;
            let head = take(bytes, &mut at, 4)?;
            let (victim, joint, asleep) = (head[0], head[1], head[2] != 0);
            let mass_g = take_u32(bytes, &mut at)?;
            let radius_mm = take_u32(bytes, &mut at)?;
            let mut v = [[0i32; 3]; 4];
            for group in &mut v {
                for slot in group.iter_mut() {
                    *slot = take_i32(bytes, &mut at)?;
                }
            }
            let tail = take(bytes, &mut at, 4)?;
            let still_ticks = u16::from_le_bytes([tail[0], tail[1]]);
            if mass_g == 0 || radius_mm == 0 || radius_mm > MAX_RADIUS_MM {
                return Err(LimbError::BadSnapshot);
            }
            pool.limbs.push(Limb {
                id,
                victim,
                joint,
                mass_g,
                radius_mm,
                pos_um: v[0],
                vel_mmps: v[1],
                angle_urad: v[2],
                spin_mradps: v[3],
                still_ticks,
                asleep,
            });
        }
        if at != bytes.len() {
            return Err(LimbError::BadSnapshot);
        }
        Ok(pool)
    }

    /// FNV-1a over [`Self::snapshot`]: the receipt a replay compares.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write(&self.snapshot());
        h.finish()
    }
}

/// Seeded stream for the spin of one cut: depends on the cut alone.
fn spin_rng(spec: &LimbSpawn) -> Rng {
    let mut h = Fnv64::new();
    h.write_u32(spec.swing);
    h.write_u8(spec.victim);
    h.write_u8(spec.joint);
    Rng::new(h.finish()).fork_named("limb-spin")
}

fn clamp_i32(v: i64) -> i32 {
    i32::try_from(v.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0)
}

fn clamp_speed(v: i64) -> i32 {
    i32::try_from(v.clamp(-i64::from(MAX_SPEED_MMPS), i64::from(MAX_SPEED_MMPS))).unwrap_or(0)
}

/// Angle folded into `[-TURN/2, TURN/2]`.
fn wrap_angle(a: i64) -> i32 {
    let turn = i64::from(TURN_URAD);
    let folded = (a + turn / 2).rem_euclid(turn) - turn / 2;
    i32::try_from(folded).unwrap_or(0)
}

fn take<'a>(bytes: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], LimbError> {
    let end = at.checked_add(n).ok_or(LimbError::BadSnapshot)?;
    let slice = bytes.get(*at..end).ok_or(LimbError::BadSnapshot)?;
    *at = end;
    Ok(slice)
}

fn take_u32(bytes: &[u8], at: &mut usize) -> Result<u32, LimbError> {
    let s = take(bytes, at, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn take_i32(bytes: &[u8], at: &mut usize) -> Result<i32, LimbError> {
    let s = take(bytes, at, 4)?;
    Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(swing: u32, victim: u8, joint: u8) -> LimbSpawn {
        LimbSpawn {
            swing,
            victim,
            joint,
            origin_mm: [1000, 1400, -500],
            parent_vel_mmps: [2000, 0, 0],
            impulse_mns: [40_000, 10_000, 0],
            volume_cm3: 4000,
            density_kg_m3: 1000,
            radius_mm: 60,
        }
    }

    fn seeded(seed: u64, n: u32) -> Vec<LimbSpawn> {
        let mut rng = Rng::new(seed).fork_named("limb-cuts");
        let mut next = |lo: i64, hi: i64| -> i32 {
            let span = u64::try_from(hi - lo + 1).expect("positive");
            i32::try_from(lo + i64::try_from(rng.next_range(0, span)).expect("small"))
                .expect("fits")
        };
        (0..n)
            .map(|i| LimbSpawn {
                swing: i,
                victim: u8::try_from(next(0, 63)).expect("slot"),
                joint: u8::try_from(next(1, 15)).expect("joint"),
                origin_mm: [
                    next(-20_000, 20_000),
                    next(300, 1800),
                    next(-20_000, 20_000),
                ],
                parent_vel_mmps: [next(-4000, 4000), next(-1000, 2000), next(-4000, 4000)],
                impulse_mns: [
                    next(-60_000, 60_000),
                    next(-20_000, 60_000),
                    next(-60_000, 60_000),
                ],
                volume_cm3: u32::try_from(next(300, 12_000)).expect("volume"),
                density_kg_m3: u32::try_from(next(900, 1200)).expect("density"),
                radius_mm: u32::try_from(next(30, 120)).expect("radius"),
            })
            .collect()
    }

    #[test]
    fn mass_comes_from_volume_times_density() {
        assert_eq!(arm(1, 0, 7).mass_g(), 4000);
        let dense = LimbSpawn {
            density_kg_m3: 1500,
            ..arm(1, 0, 7)
        };
        assert_eq!(dense.mass_g(), 6000);
        let tiny = LimbSpawn {
            volume_cm3: 1,
            density_kg_m3: 1,
            ..arm(1, 0, 7)
        };
        assert_eq!(tiny.mass_g(), 1, "never a massless body");
    }

    #[test]
    fn velocity_is_parent_plus_impulse_over_mass_and_recoil_opposes() {
        let mut pool = LimbPool::new(8).expect("pool");
        let report = pool.spawn(&arm(1, 3, 7), 0).expect("spawns");
        let limb = pool.limbs()[0];
        // 40_000 mN*s over 4 kg is 10_000 mm/s on top of 2_000 from the parent.
        assert_eq!(limb.vel_mmps, [12_000, 2_500, 0]);
        assert_eq!(report.recoil.impulse_mns, [-40_000, -10_000, 0]);
        assert_eq!((report.recoil.victim, report.recoil.joint), (3, 7));
        assert_eq!(report.id, 0);
        assert_eq!(limb.pos_um, [1_000_000, 1_400_000, -500_000]);
    }

    #[test]
    fn a_thrown_limb_lands_settles_and_sleeps_on_the_ground() {
        let mut pool = LimbPool::new(8).expect("pool");
        pool.spawn(&arm(1, 0, 7), 0).expect("spawns");
        for _ in 0..1200 {
            pool.step();
        }
        let limb = pool.limbs()[0];
        assert!(limb.asleep, "limb sleeps");
        assert_eq!(limb.pos_um[1], 60_000, "rests at its radius");
        assert_eq!(limb.vel_mmps, [0; 3]);
        assert!(limb.pos_um[0] > 1_000_000, "carried forward by the blow");
        let frozen = pool.state_hash();
        pool.step();
        assert_ne!(pool.state_hash(), frozen, "tick counter moves the hash");
        let after = pool.limbs()[0];
        assert_eq!(after.pos_um, limb.pos_um, "asleep limbs stay put");
    }

    #[test]
    fn a_batch_spawns_in_canonical_order_whatever_the_input_order() {
        let a: Vec<LimbSpawn> = vec![arm(5, 1, 9), arm(3, 2, 7), arm(3, 1, 12), arm(3, 1, 7)];
        let mut forward = a.clone();
        let mut reversed: Vec<LimbSpawn> = a.iter().rev().copied().collect();
        let mut p1 = LimbPool::new(16).expect("pool");
        let mut p2 = LimbPool::new(16).expect("pool");
        let r1 = p1.spawn_batch(&mut forward, 0).expect("spawns");
        let r2 = p2.spawn_batch(&mut reversed, 0).expect("spawns");
        assert_eq!(r1, r2);
        assert_eq!(p1.state_hash(), p2.state_hash());
        let order: Vec<(u8, u8)> = p1.limbs().iter().map(|l| (l.victim, l.joint)).collect();
        assert_eq!(order, vec![(1, 7), (1, 12), (2, 7), (1, 9)]);
    }

    #[test]
    fn a_thousand_seeded_cuts_hash_twice_and_survive_snapshot_restore() {
        let run = |snap_at: u32| {
            let cuts = seeded(2040, 1000);
            let mut pool = LimbPool::new(MAX_LIMBS).expect("pool");
            let mut snap = None;
            for (i, cut) in cuts.iter().enumerate() {
                pool.spawn(cut, 40).expect("spawns");
                for _ in 0..3 {
                    pool.step();
                }
                if u32::try_from(i).expect("small") == snap_at {
                    snap = Some((pool.snapshot(), i + 1));
                }
            }
            (pool, cuts, snap)
        };
        let (pool_a, cuts, snap) = run(499);
        let (pool_b, _, _) = run(499);
        assert_eq!(pool_a.state_hash(), pool_b.state_hash());
        assert_eq!(pool_a.snapshot(), pool_b.snapshot());
        // Restore the mid-run snapshot, replay the rest, land on the same hash.
        let (bytes, done) = snap.expect("snapshot taken");
        let mut restored = LimbPool::restore(&bytes).expect("restores");
        assert_eq!(restored.snapshot(), bytes, "restore then snapshot is equal");
        for cut in &cuts[done..] {
            restored.spawn(cut, 40).expect("spawns");
            for _ in 0..3 {
                restored.step();
            }
        }
        assert_eq!(restored.state_hash(), pool_a.state_hash());
        // Everything stays in range: no body leaves the world.
        for l in pool_a.limbs() {
            assert!(l.vel_mmps.iter().all(|v| v.abs() <= MAX_SPEED_MMPS));
            assert!(l.angle_urad.iter().all(|a| a.abs() <= TURN_URAD / 2));
            assert!(l.pos_um[1] >= i32::try_from(l.radius_mm).expect("small") * 1000);
        }
        println!(
            "limb pool 1000 cuts: hash={:016x} alive={}",
            pool_a.state_hash(),
            pool_a.len()
        );
    }

    #[test]
    fn the_body_cap_holds_with_debris_and_asks_the_ledger_to_give_way() {
        let cap = 64;
        let mut pool = LimbPool::new(cap).expect("pool");
        for (i, cut) in seeded(7, 1000).iter().enumerate() {
            let debris = 40;
            let report = pool.spawn(cut, debris).expect("spawns");
            assert!(
                pool.len() + debris <= cap,
                "cut {i}: {} limbs + {debris} debris over the cap",
                pool.len()
            );
            assert_eq!(report.debris_to_free, 0);
            pool.step();
        }
        // Debris alone fills the cap: the limb still appears and one debris goes.
        let mut full = LimbPool::new(cap).expect("pool");
        let report = full.spawn(&arm(1, 0, 7), cap).expect("spawns");
        assert_eq!(full.len(), 1);
        assert_eq!(report.debris_to_free, 1);
    }

    #[test]
    fn eviction_takes_the_oldest_sleeper_first() {
        let mut pool = LimbPool::new(3).expect("pool");
        for i in 0..3 {
            pool.spawn(&arm(i, 0, 7), 0).expect("spawns");
        }
        // Sleep only the middle limb by hand through a snapshot edit.
        let mut bytes = pool.snapshot();
        let header = 20;
        let stride = 68;
        bytes[header + stride + 6] = 1;
        let mut pool = LimbPool::restore(&bytes).expect("restores");
        let report = pool.spawn(&arm(9, 0, 7), 0).expect("spawns");
        assert_eq!(report.evicted_limbs, 1);
        let ids: Vec<u32> = pool.limbs().iter().map(|l| l.id).collect();
        assert_eq!(ids, vec![0, 2, 3], "the sleeper (id 1) left");
    }

    #[test]
    fn bad_input_refuses_and_hostile_snapshots_do_not_panic() {
        assert_eq!(LimbPool::new(0).unwrap_err(), LimbError::BadCap(0));
        assert_eq!(
            LimbPool::new(MAX_LIMBS + 1).unwrap_err(),
            LimbError::BadCap(MAX_LIMBS + 1)
        );
        let mut pool = LimbPool::new(4).expect("pool");
        let bad_volume = LimbSpawn {
            volume_cm3: 0,
            ..arm(1, 0, 7)
        };
        assert_eq!(
            pool.spawn(&bad_volume, 0).unwrap_err(),
            LimbError::BadVolume
        );
        let bad_pos = LimbSpawn {
            origin_mm: [MAX_POS_MM + 1, 0, 0],
            ..arm(1, 0, 7)
        };
        assert_eq!(pool.spawn(&bad_pos, 0).unwrap_err(), LimbError::BadSpawn);
        assert!(pool.is_empty(), "refused spawns leave no body");
        pool.spawn(&arm(1, 0, 7), 0).expect("spawns");
        let good = pool.snapshot();
        assert!(LimbPool::restore(&good).is_ok());
        for cut in 0..good.len() {
            assert!(LimbPool::restore(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut extra = good.clone();
        extra.push(0);
        assert!(LimbPool::restore(&extra).is_err());
        let mut wrong = good;
        wrong[0] = 9;
        assert!(LimbPool::restore(&wrong).is_err());
    }

    #[test]
    fn stepping_a_full_pool_allocates_nothing() {
        let mut pool = LimbPool::new(MAX_LIMBS).expect("pool");
        for cut in seeded(5, 200) {
            pool.spawn(&cut, 0).expect("spawns");
        }
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for _ in 0..60 {
                pool.step();
            }
        });
        assert_eq!(heap.calls, 0);
    }
}

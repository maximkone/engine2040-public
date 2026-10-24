//! foot_dust: footprints in the sand and a short dust burst on each landing
//! (QA8). Presentation only: it reads the sole heights the posed fighters
//! already have and returns requests; nothing it does reaches the sim. Std-only
//! state over the render crate's ground-splat pool. Same samples in the same
//! order give the same requests and the same pool, bit for bit.
//!
//! A plant is a sole that has risen above [`LIFT_M`] (arming its track) and now
//! drops to [`CONTACT_M`] or lower while moving down at [`PLANT_DOWN_MPS`] or
//! faster. One plant gives one print and one dust burst; the track disarms until
//! the sole lifts again. A standing sole never arms, so it never plants, however
//! it jitters below [`LIFT_M`].
//!
//! The print pool is an [`engine_render::decals::SplatField`]: a fixed number of
//! slots from the tier's decal cap ([`FootDust::for_tier`]), the oldest print
//! recycled first, faded on the same tick clock as blood on the floor.

use engine_render::decals::{DecalError, SplatField};
use engine_render::quality::{decal_pool_slots, knobs, Quality};

/// Sim tick length in seconds (60 Hz). Downward speed is measured per tick.
pub const TICK_S: f32 = 1.0 / 60.0;
/// Height (meters above the floor) at or under which a sole is on the sand.
pub const CONTACT_M: f32 = 0.03;
/// Height above which a sole is airborne enough to arm its next plant.
pub const LIFT_M: f32 = 0.08;
/// Downward speed at contact (m/s) that counts as a landing, not a rest.
pub const PLANT_DOWN_MPS: f32 = 0.25;
/// Footprint radius in meters (a sole is about 0.25 m long).
pub const PRINT_RADIUS_M: f32 = 0.09;
/// Dust particles per landing: `DUST_MIN` plus `DUST_PER_MPS` per m/s, capped.
pub const DUST_MIN: u32 = 6;
/// Most dust particles one landing throws.
pub const DUST_MAX: u32 = 16;
/// Extra dust particles per m/s of landing speed.
pub const DUST_PER_MPS: f32 = 3.0;
/// Outward speed of each dust particle, m/s.
pub const DUST_SPEED_MPS: f32 = 0.8;
/// Dust particle life in ticks (0.4 s at 60 Hz).
pub const DUST_LIFE_TICKS: u32 = 24;
/// Fighter ids the tracker holds; an id at or above it is refused.
pub const MAX_FIGHTERS: u8 = 16;

/// Which sole of a fighter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Foot {
    /// The left sole.
    Left,
    /// The right sole.
    Right,
}

impl Foot {
    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

/// One sole's world position this tick. `pos_m[1]` is its height above the floor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FootSample {
    /// Fighter id, below [`MAX_FIGHTERS`].
    pub fighter: u8,
    /// Which sole.
    pub foot: Foot,
    /// World position in meters.
    pub pos_m: [f32; 3],
}

/// What the caller draws or spawns for one landing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FootRequest {
    /// A footprint on the sand at `center_m`. The pool already holds it as `seq`.
    Print {
        /// Fighter that stepped.
        fighter: u8,
        /// Sole that landed.
        foot: Foot,
        /// Center on the floor, meters.
        center_m: [f32; 3],
        /// Print radius, meters.
        radius_m: f32,
        /// Pool sequence of the print (oldest-eviction order).
        seq: u64,
    },
    /// A short sand dust burst at `center_m`.
    Dust {
        /// Fighter that stepped.
        fighter: u8,
        /// Sole that landed.
        foot: Foot,
        /// Burst origin on the floor, meters.
        center_m: [f32; 3],
        /// Particles to throw, within `DUST_MIN..=DUST_MAX`.
        count: u32,
        /// Outward speed of each particle, m/s.
        speed_mps: f32,
        /// Particle life in ticks.
        life_ticks: u32,
    },
}

/// Refused input. Checked before any state changes, so a refused call leaves
/// the tracker exactly as it was.
#[derive(Clone, PartialEq, Debug)]
pub enum FootError {
    /// A fighter id at or above [`MAX_FIGHTERS`].
    Fighter(u8),
    /// A sole position with a non-finite component, for this fighter.
    NotFinite(u8),
    /// The print pool refused a footprint (not reachable after validation).
    Decal(DecalError),
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct FootTrack {
    /// Height last tick, `None` before the first sample.
    prev_y: Option<f32>,
    /// Risen above [`LIFT_M`] since the last plant.
    armed: bool,
}

/// Dust particles for a landing at `down_mps`: `DUST_MIN` plus one per
/// `1 / DUST_PER_MPS` m/s of landing speed, never above `DUST_MAX`. Counted by
/// threshold, so no float is cast to an integer.
fn dust_count(down_mps: f32) -> u32 {
    let step_mps = 1.0 / DUST_PER_MPS;
    let mut count = DUST_MIN;
    let mut due_mps = step_mps;
    while count < DUST_MAX && down_mps >= due_mps {
        count += 1;
        due_mps += step_mps;
    }
    count
}

/// Plant detector and fixed-size print pool for every sole.
#[derive(Clone, PartialEq, Debug)]
pub struct FootDust {
    tracks: Vec<[FootTrack; 2]>,
    prints: SplatField,
    landings: u64,
}

impl FootDust {
    /// Tracker whose print pool holds `slots` footprints (the pool clamps it to
    /// its own hard bound).
    pub fn with_capacity(slots: usize) -> Self {
        Self {
            tracks: vec![[FootTrack::default(); 2]; usize::from(MAX_FIGHTERS)],
            prints: SplatField::with_capacity(slots),
            landings: 0,
        }
    }

    /// Tracker whose pool takes the tier's decal cap (`decal_pool_slots`).
    pub fn for_tier(tier: Quality) -> Self {
        Self::with_capacity(decal_pool_slots(&knobs(tier)))
    }

    /// Print slots (the cap no print count may pass).
    pub fn capacity(&self) -> usize {
        self.prints.capacity()
    }

    /// The print pool, for the draw pass and the receipt counts.
    pub fn prints(&self) -> &SplatField {
        &self.prints
    }

    /// Plants seen over the life of the tracker.
    pub fn landings(&self) -> u64 {
        self.landings
    }

    /// Advance one sim tick with every sole sampled this tick. Appends the
    /// requests to `out` (the caller clears it). Refused input changes nothing
    /// and returns the error.
    pub fn step(
        &mut self,
        samples: &[FootSample],
        out: &mut Vec<FootRequest>,
    ) -> Result<(), FootError> {
        for s in samples {
            if s.fighter >= MAX_FIGHTERS {
                return Err(FootError::Fighter(s.fighter));
            }
            if !s.pos_m.iter().all(|v| v.is_finite()) {
                return Err(FootError::NotFinite(s.fighter));
            }
        }
        self.prints.tick();
        for s in samples {
            let y = s.pos_m[1];
            let track = &mut self.tracks[usize::from(s.fighter)][s.foot.index()];
            let down_mps = track.prev_y.map_or(0.0, |prev| (prev - y) / TICK_S);
            track.prev_y = Some(y);
            if y > LIFT_M {
                track.armed = true;
                continue;
            }
            if !track.armed || y > CONTACT_M || down_mps < PLANT_DOWN_MPS {
                continue;
            }
            track.armed = false;
            let center_m = [s.pos_m[0], y, s.pos_m[2]];
            let seq = self
                .prints
                .place(center_m, PRINT_RADIUS_M)
                .map_err(FootError::Decal)?;
            let count = dust_count(down_mps);
            out.push(FootRequest::Print {
                fighter: s.fighter,
                foot: s.foot,
                center_m,
                radius_m: PRINT_RADIUS_M,
                seq,
            });
            out.push(FootRequest::Dust {
                fighter: s.fighter,
                foot: s.foot,
                center_m,
                count,
                speed_mps: DUST_SPEED_MPS,
                life_ticks: DUST_LIFE_TICKS,
            });
            self.landings += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hop: the sole rises to `h` and lands once per `period` ticks, moving
    /// `stride_m` per period. Covers `0..=period * steps`, so it plants `steps`
    /// times.
    fn gait(
        fighter: u8,
        foot: Foot,
        period: u32,
        steps: u32,
        h: f32,
        stride_m: f32,
    ) -> Vec<FootSample> {
        (0..=period * steps)
            .map(|t| {
                let phase = std::f32::consts::PI * t as f32 / period as f32;
                FootSample {
                    fighter,
                    foot,
                    pos_m: [
                        stride_m * (t as f32 / period as f32),
                        h * phase.sin().abs(),
                        0.0,
                    ],
                }
            })
            .collect()
    }

    /// Feed one tick per entry of `ticks` and return every request in order.
    fn run(fd: &mut FootDust, ticks: &[Vec<FootSample>]) -> Vec<FootRequest> {
        let mut all = Vec::new();
        let mut out = Vec::new();
        for tick in ticks {
            out.clear();
            fd.step(tick, &mut out).expect("valid samples");
            all.extend_from_slice(&out);
        }
        all
    }

    fn one_per_tick(samples: &[FootSample]) -> Vec<Vec<FootSample>> {
        samples.iter().map(|s| vec![*s]).collect()
    }

    fn prints_of(reqs: &[FootRequest]) -> usize {
        reqs.iter()
            .filter(|r| matches!(r, FootRequest::Print { .. }))
            .count()
    }

    #[test]
    fn walking_foot_gives_one_print_per_plant() {
        let mut fd = FootDust::with_capacity(64);
        let reqs = run(
            &mut fd,
            &one_per_tick(&gait(0, Foot::Left, 30, 6, 0.2, 0.5)),
        );
        assert_eq!(prints_of(&reqs), 6);
        assert_eq!(reqs.len(), 12, "one print and one dust burst per plant");
        assert_eq!(fd.landings(), 6);
        assert_eq!(fd.prints().len(), 6);
        for r in &reqs {
            if let FootRequest::Dust { count, .. } = r {
                assert!((DUST_MIN..=DUST_MAX).contains(count));
            }
        }
    }

    #[test]
    fn standing_foot_gives_no_print() {
        let mut fd = FootDust::with_capacity(64);
        // A sole on the sand, bobbing 2 cm: it never lifts past LIFT_M, so it never arms.
        let ticks: Vec<Vec<FootSample>> = (0..600u32)
            .map(|t| {
                let y = 0.02 * (t as f32 * 0.3).sin().abs();
                vec![FootSample {
                    fighter: 0,
                    foot: Foot::Right,
                    pos_m: [1.0, y, 1.0],
                }]
            })
            .collect();
        let reqs = run(&mut fd, &ticks);
        assert!(reqs.is_empty());
        assert_eq!(fd.landings(), 0);
        assert!(fd.prints().is_empty());
    }

    #[test]
    fn two_hundred_plants_never_exceed_the_web_cap_and_recycle_the_oldest() {
        let mut fd = FootDust::for_tier(Quality::Web);
        assert_eq!(fd.capacity(), 128, "the web tier's decal cap");
        // One hop every 4 ticks: 200 plants inside 800 ticks, all younger than the fade.
        let ticks = one_per_tick(&gait(0, Foot::Left, 4, 200, 0.2, 0.1));
        let mut out = Vec::new();
        let mut seqs = Vec::new();
        for tick in &ticks {
            out.clear();
            fd.step(tick, &mut out).expect("valid samples");
            assert!(fd.prints().len() <= 128, "pool passed its cap");
            for r in &out {
                if let FootRequest::Print { seq, .. } = r {
                    seqs.push(*seq);
                }
            }
        }
        assert_eq!(seqs.len(), 200);
        assert_eq!(fd.landings(), 200);
        assert_eq!(fd.prints().placed_total(), 200);
        assert_eq!(fd.prints().len(), 128);
        assert_eq!(
            fd.prints().evicted_count(),
            72,
            "200 prints minus the 128 slots"
        );
        let resident: Vec<u64> = fd.prints().splats().iter().map(|s| s.seq).collect();
        assert_eq!(
            resident.first().copied(),
            Some(72),
            "the oldest print was recycled"
        );
        assert_eq!(
            resident.last().copied(),
            Some(199),
            "the newest print is kept"
        );
    }

    #[test]
    fn same_samples_give_the_same_requests_and_pool() {
        let left = gait(3, Foot::Left, 7, 40, 0.2, 0.3);
        let right = gait(3, Foot::Right, 7, 40, 0.2, 0.3);
        let ticks: Vec<Vec<FootSample>> =
            left.iter().zip(&right).map(|(l, r)| vec![*l, *r]).collect();
        let mut a = FootDust::with_capacity(32);
        let mut b = FootDust::with_capacity(32);
        let ra = run(&mut a, &ticks);
        let rb = run(&mut b, &ticks);
        assert_eq!(ra.len(), 160, "40 plants per sole, two requests each");
        assert_eq!(ra, rb);
        assert_eq!(a, b, "the whole tracker state matches");
    }

    #[test]
    fn refused_samples_change_nothing() {
        let mut fd = FootDust::with_capacity(8);
        let before = fd.clone();
        let mut out = Vec::new();
        let bad_id = [FootSample {
            fighter: MAX_FIGHTERS,
            foot: Foot::Left,
            pos_m: [0.0, 0.2, 0.0],
        }];
        assert_eq!(
            fd.step(&bad_id, &mut out),
            Err(FootError::Fighter(MAX_FIGHTERS))
        );
        let nan = [FootSample {
            fighter: 0,
            foot: Foot::Left,
            pos_m: [f32::NAN, 0.0, 0.0],
        }];
        assert_eq!(fd.step(&nan, &mut out), Err(FootError::NotFinite(0)));
        assert!(out.is_empty());
        assert_eq!(fd, before);
    }
}

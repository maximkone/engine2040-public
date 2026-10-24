//! motion_bake: the offline teacher bake's pure half (endpoint rule + pack).
//!
//! Owner: `engine-anim`. Consumer: the bake tool `tools/motion_bake`, which
//! reads a Kimodo NPZ teacher clip, retargets it with `soma_pairs()` through
//! `BoneMap`/`retarget_cook` and packs the result to AM5P. This module owns
//! the two rules every bake must obey and no I/O at all: the PartiGen
//! endpoint rule ([`bake_len`]) and the pack half ([`bake_pack`] /
//! [`bake_unpack`]) over the one AM5P home in [`super::pack`]. The tick never
//! calls it.
//!
//! Endpoint rule: a teacher clip ends where its last constrained frame ends,
//! never at a fixed duration and never at the teacher's own tail. A
//! constrained frame is one the teacher pinned (a foot plant, a key pose);
//! [`last_constrained`] reads the last one from a per-frame flag lane and
//! [`bake_len`] turns it into the clip length, so a trailing unconstrained
//! tail is cut. Zero-length, negative and over-cap lengths are refused
//! instead of clipped.
//!
//! Pack half: [`bake_pack`] writes the bytes and proves they decode back to
//! the same clip bits ([`qclip_hash`]), [`bake_unpack`] reads bytes the bake
//! wrote and proves a re-pack is byte-identical. One format, no twin.
//!
//! Determinism: comparisons and `+ - * /` only, no clock, no map iteration.

use super::clip::{qclip_hash, QClip};
use super::pack::{pack_qclip, unpack_qclip, PackError, PackedClip};

/// Fastest bake rate accepted, frames per second.
pub const MIN_BAKE_FPS: f32 = 1.0;
/// Slowest bake rate accepted, frames per second.
pub const MAX_BAKE_FPS: f32 = 1_000.0;
/// Longest bake accepted, in ticks: the shared 3600-frame cook cap
/// (`MAX_COOK_TICKS`, `MAX_KEYS_PER_TRACK`).
pub const MAX_BAKE_TICKS: u32 = 3_600;
/// Shortest bake accepted, in ticks: a one-frame bake carries no motion.
pub const MIN_BAKE_TICKS: u32 = 2;

/// One bake length: tick count plus the time of the last constrained tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BakeLen {
    /// Ticks the baked clip holds, the last constrained tick included.
    pub ticks: u32,
    /// Time of the last constrained tick, seconds: the clip's last key.
    pub dur: f32,
}

/// Why a bake length or pack step was refused; every variant names the fault.
#[derive(Clone, PartialEq, Debug)]
pub enum BakeError {
    /// No constrained frame at all: a negative last index means the teacher
    /// pinned nothing, so there is no length to take.
    NoConstraint,
    /// A single constrained frame (a still): refused, it carries no motion.
    TooShort {
        /// Tick count seen.
        ticks: u32,
    },
    /// The bake passes [`MAX_BAKE_TICKS`]; refused, never truncated.
    TooLong {
        /// Tick count wanted.
        ticks: u32,
    },
    /// The bake rate is not a finite rate inside
    /// [`MIN_BAKE_FPS`]..=[`MAX_BAKE_FPS`].
    BadFps(f32),
    /// The AM5P wire refused the clip; the cause is carried.
    Pack(PackError),
    /// The bytes did not decode back to the same clip bits; a bake never
    /// ships a lossy pack.
    NotRoundTrip,
    /// The rotation half did not decode back to the same motion; the joint
    /// and key name where the swing was lost.
    LostRotation {
        /// The joint whose rotation changed.
        joint: usize,
        /// The key within that joint's track.
        key: usize,
        /// The worst component gap seen at that key.
        worst: f32,
    },
}

impl core::fmt::Display for BakeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoConstraint => write!(f, "bake: no constrained frame, no length"),
            Self::TooShort { ticks } => {
                write!(f, "bake: {ticks} tick(s) carries no motion")
            }
            Self::TooLong { ticks } => write!(f, "bake: {ticks} ticks passes the bake cap"),
            Self::BadFps(fps) => write!(f, "bake: {fps} is not a bake rate"),
            Self::Pack(cause) => write!(f, "bake: {cause}"),
            Self::NotRoundTrip => write!(f, "bake: the AM5P pack did not round-trip"),
            Self::LostRotation { joint, key, worst } => write!(
                f,
                "bake: the AM5P pack lost the rotation at joint {joint} key {key} (gap {worst:e})"
            ),
        }
    }
}

impl std::error::Error for BakeError {}

impl From<PackError> for BakeError {
    fn from(cause: PackError) -> Self {
        Self::Pack(cause)
    }
}

/// Last frame the teacher constrained, or `None` when it constrained none.
/// `constrained[i]` is the flag of frame `i` at the bake rate; the last true
/// flag is the frame the clip ends on (PartiGen's end-of-sequence rule, done
/// as a scan instead of a net).
pub fn last_constrained(constrained: &[bool]) -> Option<u32> {
    constrained
        .iter()
        .rposition(|flag| *flag)
        .and_then(|index| u32::try_from(index).ok())
}

/// Length of a baked clip from the last constrained tick: the rule that
/// replaces a fixed duration. The last constrained tick is the clip's last
/// key, so the bake holds it and drops the teacher's tail.
///
/// Refused instead of clipped: a negative index ([`BakeError::NoConstraint`]),
/// a single tick ([`BakeError::TooShort`]), more than [`MAX_BAKE_TICKS`]
/// ([`BakeError::TooLong`]) and an unusable rate ([`BakeError::BadFps`]).
pub fn bake_len(last_constrained_tick: i64, fps: f32) -> Result<BakeLen, BakeError> {
    if !fps.is_finite() || !(MIN_BAKE_FPS..=MAX_BAKE_FPS).contains(&fps) {
        return Err(BakeError::BadFps(fps));
    }
    if last_constrained_tick < 0 {
        return Err(BakeError::NoConstraint);
    }
    let last = u32::try_from(last_constrained_tick).map_err(|_| BakeError::TooLong {
        ticks: MAX_BAKE_TICKS + 1,
    })?;
    let ticks = last + 1;
    if ticks < MIN_BAKE_TICKS {
        return Err(BakeError::TooShort { ticks });
    }
    if ticks > MAX_BAKE_TICKS {
        return Err(BakeError::TooLong { ticks });
    }
    // MAX_BAKE_TICKS fits u16, so the last key index converts exactly and
    // without a float cast (f32: From<u32> does not exist; u32 -> f32 is lossy).
    let last_key = u16::try_from(last).map_err(|_| BakeError::TooLong { ticks })?;
    Ok(BakeLen {
        ticks,
        dur: f32::from(last_key) / fps,
    })
}

/// Bake one clip to AM5P bytes through the single pack home and prove the
/// round trip: the bytes decode back to the same `qclip_hash`.
///
/// The wire stores `dur` but [`unpack_qclip`] reads the clip's duration from
/// its last knot, so a clip whose `dur` is not its last knot is refused
/// ([`BakeError::NotRoundTrip`]) rather than baked lossy; build bakes with
/// [`super::clip::bake_clip`], which sets exactly that duration.
pub fn bake_pack(clip: &QClip) -> Result<Vec<u8>, BakeError> {
    let bytes = pack_qclip(clip)?.to_bytes();
    let back = bake_unpack(&bytes)?;
    if qclip_hash(&back) != qclip_hash(clip) {
        return Err(BakeError::NotRoundTrip);
    }
    Ok(bytes)
}

/// Read AM5P bytes a bake wrote back to a clip, refusing a blob that does not
/// re-pack to the same bytes (the fixed point the wire guarantees).
pub fn bake_unpack(bytes: &[u8]) -> Result<QClip, BakeError> {
    let packed = PackedClip::from_bytes(bytes)?;
    if packed.to_bytes() != bytes {
        return Err(BakeError::NotRoundTrip);
    }
    Ok(unpack_qclip(&packed))
}

/// Bake one clip to AM5P bytes that also carry the joint rotations.
///
/// The position-only bake ([`bake_pack`]) writes a version-1 pack: the feet
/// move, the arms never turn. A combat clip needs the swing, so this packs
/// the rotation tracks as well and writes a version-2 pack
/// ([`super::pack::PACK_VERSION_ROT`]), one rotation track per joint, in the
/// same joint order as the position tracks.
///
/// The round trip is proved on both halves: the bytes decode back to the same
/// position clip ([`bake_unpack`]) and to the same rotations
/// ([`bake_unpack_rots`]), so a pack that lost a swing refuses to be baked
/// instead of shipping a fighter that only slides.
pub fn bake_pack_with_rots(
    clip: &QClip,
    rotations: &[super::pose::QuatTrack],
) -> Result<Vec<u8>, BakeError> {
    if rotations.len() != clip.tracks.len() {
        return Err(BakeError::Pack(PackError::BadLength {
            track: 0,
            times: rotations.len(),
            keys: clip.tracks.len(),
        }));
    }
    let mut packed = pack_qclip(clip)?;
    let mut rot_tracks = Vec::with_capacity(rotations.len());
    for (index, track) in rotations.iter().enumerate() {
        rot_tracks.push(super::pack::pack_quat_track(
            index,
            &track.times,
            &track.values,
        )?);
    }
    packed.rotations = rot_tracks;
    let bytes = packed.to_bytes();
    // Both halves must come back: the positions and the rotations.
    let back = bake_unpack(&bytes)?;
    if qclip_hash(&back) != qclip_hash(clip) {
        return Err(BakeError::NotRoundTrip);
    }
    let back_rots = bake_unpack_rots(&bytes)?;
    if back_rots.len() != rotations.len() {
        return Err(BakeError::NotRoundTrip);
    }
    for (joint, (back, raw)) in back_rots.iter().zip(rotations.iter()).enumerate() {
        // The times come back through the wire's time base: a uniform run is
        // rebuilt as `t0 + k * dt`, which is not bit for bit the knots the
        // bake wrote (`k * TICK_DT`), so the times are compared as the
        // numbers they are, not as bits.
        if back.times.len() != raw.times.len() {
            return Err(BakeError::LostRotation {
                joint,
                key: 0,
                worst: 0.0,
            });
        }
        for (at, wanted) in back.times.iter().zip(raw.times.iter()) {
            if (at - wanted).abs() > 1.0e-6 {
                return Err(BakeError::LostRotation {
                    joint,
                    key: 0,
                    worst: 0.0,
                });
            }
        }
        // The rotations come back through the wire's smallest-three lanes, so
        // they match the raw ones to the wire's own bound, not bit for bit.
        // The wire normalizes on the way in and rebuilds the largest
        // component non-negative (a rotation and its negation are one
        // rotation), so the comparison tries both signs and takes the
        // closer one: a flipped quaternion is the same turn.
        for (key, (decoded, wanted)) in back.values.iter().zip(raw.values.iter()).enumerate() {
            let unit = crate::pack::unit_quat(*wanted);
            let mut worst = 0.0f32;
            for lane in 0..4 {
                let gap = (decoded[lane] - unit[lane]).abs();
                let flipped = (decoded[lane] + unit[lane]).abs();
                worst = worst.max(gap.min(flipped));
            }
            if worst > super::pack::QUAT_BOUND {
                return Err(BakeError::LostRotation { joint, key, worst });
            }
        }
    }
    Ok(bytes)
}

/// The wire's own smallest-three error bound, named here so the bake's
/// refusal says which half of the pack lost the motion.
pub const ROT_BOUND: f32 = super::pack::QUAT_BOUND;

/// Read the rotation tracks out of an AM5P blob, one per joint, in the pack's
/// joint order. A version-1 pack (positions only) reads as no rotations.
///
/// The values come back through the wire's own decoder
/// ([`super::pack::sample_packed_quat`]), so what a caller reads is what the
/// tick will play, not the pre-pack float.
pub fn bake_unpack_rots(bytes: &[u8]) -> Result<Vec<super::pose::QuatTrack>, BakeError> {
    let packed = PackedClip::from_bytes(bytes)?;
    if packed.to_bytes() != bytes {
        return Err(BakeError::NotRoundTrip);
    }
    let mut out = Vec::with_capacity(packed.rotations.len());
    for (index, track) in packed.rotations.iter().enumerate() {
        let keys = track.key_count();
        let mut times = Vec::with_capacity(keys);
        let mut values = Vec::with_capacity(keys);
        for k in 0..keys {
            let at = track.time_at(k);
            times.push(at);
            values.push(super::pack::sample_packed_quat(
                &packed,
                index,
                at,
                &mut 0usize,
            ));
        }
        out.push(super::pose::QuatTrack { times, values });
    }
    Ok(out)
}

/// Sample rate of a baked foot-grounded weight track, Hz: OpenKCC's
/// `HumanoidFootIKEditor.BakeAnimations` samples feet at 30, not at the tick rate, so the
/// track is half the length of the clip's 60 Hz frames.
pub const FOOT_WEIGHT_HZ: u32 = 30;
/// Tick rate the planted lane is handed in, Hz: one flag pair per baked frame.
pub const FOOT_TICK_HZ: u32 = 60;

/// One clip's foot-grounded weight lane: per sample, the permille of its window each foot spent
/// planted. 1000 is a foot down for the whole sample, 0 is a foot in the air.
///
/// The IK starts from this lane ([`FootWeights::weight_at`]) instead of solving contacts every
/// tick, so a clipped or lagged contact table cannot drop a planted foot mid-slide.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FootWeights {
    /// Sample rate, Hz.
    pub hz: u32,
    /// Tick rate the flags came in at, Hz.
    pub tick_hz: u32,
    /// Samples baked: `ceil(ticks * hz / tick_hz)`.
    pub samples: u32,
    /// One `[left, right]` permille pair per sample.
    pub ik_weight: Vec<[u16; 2]>,
}

/// Why a foot-weight bake was refused; every variant names the fault.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FootWeightError {
    /// No frames: a clip with no motion has no plant to bake.
    Empty,
    /// The tick rate is not a whole multiple of [`FOOT_WEIGHT_HZ`]: refused rather than
    /// mis-sampled, because a fractional window cannot be counted in whole ticks.
    BadTickHz(u32),
    /// More frames than the shared bake cap ([`MAX_BAKE_TICKS`]).
    TooLong(u32),
}

impl core::fmt::Display for FootWeightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "foot weights: no frames, no plant"),
            Self::BadTickHz(hz) => write!(
                f,
                "foot weights: {hz} Hz is not a whole multiple of {FOOT_WEIGHT_HZ} Hz"
            ),
            Self::TooLong(ticks) => write!(f, "foot weights: {ticks} frames passes the bake cap"),
        }
    }
}

impl std::error::Error for FootWeightError {}

impl FootWeights {
    /// The baked weight of `foot` (0 left, 1 right) at `tick`, permille; the sample the tick
    /// falls in, and 0 past the lane's end. Never interpolated: a half-planted weight between
    /// two samples is a foot the IK would slide.
    pub fn weight_at(&self, tick: u32, foot: usize) -> u16 {
        if foot > 1 || self.tick_hz == 0 {
            return 0;
        }
        let sample =
            usize::try_from(u64::from(tick) * u64::from(self.hz) / u64::from(self.tick_hz))
                .unwrap_or(usize::MAX);
        self.ik_weight
            .get(sample)
            .map_or(0, |pair| pair[foot.min(1)])
    }
}

/// Bake a planted-foot lane, one `[left, right]` flag pair per tick, into 30 Hz permille
/// weights: each sample's weight is the share of its whole window the foot spent down, so a
/// window of two planted ticks reads 1000 and one of two airborne ticks reads 0.
///
/// Refused instead of clipped: no frames ([`FootWeightError::Empty`]), a tick rate that is not
/// a whole multiple of [`FOOT_WEIGHT_HZ`] ([`FootWeightError::BadTickHz`]) and more than
/// [`MAX_BAKE_TICKS`] frames ([`FootWeightError::TooLong`]).
pub fn bake_foot_weights(
    planted: &[[bool; 2]],
    tick_hz: u32,
) -> Result<FootWeights, FootWeightError> {
    if planted.is_empty() {
        return Err(FootWeightError::Empty);
    }
    if tick_hz == 0 || !tick_hz.is_multiple_of(FOOT_WEIGHT_HZ) {
        return Err(FootWeightError::BadTickHz(tick_hz));
    }
    let ticks = u32::try_from(planted.len()).map_err(|_| FootWeightError::TooLong(u32::MAX))?;
    if ticks > MAX_BAKE_TICKS {
        return Err(FootWeightError::TooLong(ticks));
    }
    let per = usize::try_from(tick_hz / FOOT_WEIGHT_HZ).unwrap_or(1);
    let samples = planted.len().div_ceil(per);
    let mut ik_weight = Vec::with_capacity(samples);
    for sample in 0..samples {
        let mut pair = [0u16; 2];
        for foot in 0..2 {
            let window = planted[sample * per..(sample * per + per).min(planted.len())]
                .iter()
                .filter(|flags| flags[foot])
                .count();
            // The last window of a clip may be short; its weight is over the ticks it has.
            let held = u32::try_from(window).unwrap_or(u32::MAX);
            let whole = u32::try_from((sample * per + per).min(planted.len()) - sample * per)
                .unwrap_or(1)
                .max(1);
            pair[foot] = u16::try_from(held * 1000 / whole).unwrap_or(1000);
        }
        ik_weight.push(pair);
    }
    Ok(FootWeights {
        hz: FOOT_WEIGHT_HZ,
        tick_hz,
        samples: u32::try_from(samples).unwrap_or(u32::MAX),
        ik_weight,
    })
}

#[cfg(test)]
mod tests {
    use super::super::clip::{avg_world_slip, bake_clip, bake_track, TICK_DT};
    use super::*;

    /// The fighter rig's leg chains, in the joint order the pack carries (the
    /// receipt lists it): hips 0, leftupleg 55, leftleg 56, leftfoot 57, and
    /// the right side at 60/61/62. Every other joint hangs off the hips; the
    /// world-slip meter walks only the chain up from the foot, so only these
    /// parents matter. `engine-assets` owns the real table and `anim` cannot
    /// depend on it (assets depends on anim), so this test mirrors the four
    /// links it reads.
    fn fighter_legs() -> crate::contract::Skeleton {
        use crate::contract::Joint;
        let mut joints: Vec<Joint> = (0..65)
            .map(|index| Joint {
                name: format!("joint{index}"),
                parent: Some(0),
            })
            .collect();
        let links = [
            (0usize, None, "hips"),
            (55, Some(0), "leftupleg"),
            (56, Some(55), "leftleg"),
            (57, Some(56), "leftfoot"),
            (60, Some(0), "rightupleg"),
            (61, Some(60), "rightleg"),
            (62, Some(61), "rightfoot"),
        ];
        for (index, parent, name) in links {
            joints[index] = Joint {
                name: name.to_string(),
                parent,
            };
        }
        crate::contract::Skeleton { joints }
    }

    /// One 60 Hz two-track clip: the fixture shape the tool writes.
    fn tick_clip(ticks: u32) -> QClip {
        let times: Vec<f32> = (0..ticks).map(|k| k as f32 * TICK_DT).collect();
        let left: Vec<[f32; 3]> = times
            .iter()
            .map(|t| [0.0, -0.42 + 0.10 * (t * 6.0).min(1.0), 0.0])
            .collect();
        let right: Vec<[f32; 3]> = times
            .iter()
            .map(|t| [0.20 - 0.20 * (t * 3.0).min(1.0), -0.42, 0.0])
            .collect();
        bake_clip(vec![
            bake_track(times.clone(), left),
            bake_track(times, right),
        ])
    }

    #[test]
    fn bake_len_follows_the_last_constrained_frame() {
        // The last constrained tick is the clip's last key, never a fixed
        // duration: 41 becomes 42 ticks (0.7 s at 60 Hz).
        let len = bake_len(41, 60.0).expect("42 ticks");
        assert_eq!(len.ticks, 42);
        assert!((len.dur - 41.0 / 60.0).abs() < 1e-7, "dur {}", len.dur);

        // A long trailing unconstrained tail is cut, not baked: 101 pinned
        // frames then 499 free ones bake to 101 ticks.
        let mut flags = vec![false; 600];
        for flag in flags.iter_mut().take(101) {
            *flag = true;
        }
        let last = last_constrained(&flags).expect("frame 100 is pinned");
        assert_eq!(last, 100);
        let len = bake_len(i64::from(last), 60.0).expect("101 ticks");
        assert_eq!(len.ticks, 101);
        assert!(len.ticks < flags.len() as u32, "the tail is cut");

        // A pinned frame zero is a still, not a clip; a negative index means
        // the teacher pinned nothing.
        assert_eq!(last_constrained(&[]), None);
        assert_eq!(last_constrained(&[false, false]), None);
        assert_eq!(last_constrained(&[true]), Some(0));
    }

    #[test]
    fn bake_len_refuses_zero_negative_and_absurd() {
        assert_eq!(
            bake_len(0, 60.0),
            Err(BakeError::TooShort { ticks: 1 }),
            "a one-frame bake carries no motion"
        );
        assert_eq!(bake_len(-1, 60.0), Err(BakeError::NoConstraint));
        assert_eq!(bake_len(i64::MIN, 60.0), Err(BakeError::NoConstraint));
        assert_eq!(
            bake_len(3_600, 60.0),
            Err(BakeError::TooLong { ticks: 3_601 })
        );
        assert_eq!(
            bake_len(4_000_000, 60.0),
            Err(BakeError::TooLong { ticks: 4_000_001 })
        );
        for fps in [0.0, -60.0, f32::NAN, f32::INFINITY, 0.5, 1_001.0] {
            match bake_len(41, fps) {
                // NaN never equals itself, so compare the carried bits.
                Err(BakeError::BadFps(got)) => {
                    assert_eq!(got.to_bits(), fps.to_bits(), "fps {fps} is carried");
                }
                other => panic!("fps {fps} must be refused, got {other:?}"),
            }
        }
        assert!(bake_len(3_599, 60.0).is_ok(), "the cap itself is legal");
    }

    #[test]
    fn pack_round_trips_bit_exact_through_the_one_home() {
        let clip = tick_clip(61);
        let bytes = bake_pack(&clip).expect("bakes");
        let back = bake_unpack(&bytes).expect("decodes");
        assert_eq!(back.tracks.len(), clip.tracks.len(), "track count");
        for (got, want) in back.tracks.iter().zip(clip.tracks.iter()) {
            assert_eq!(got.keys, want.keys, "keys differ");
            assert_eq!(got.times.len(), want.times.len(), "time counts differ");
            for (a, b) in got.times.iter().zip(want.times.iter()) {
                assert_eq!(a.to_bits(), b.to_bits(), "knot bits differ");
            }
            for lane in 0..3 {
                assert_eq!(got.min[lane].to_bits(), want.min[lane].to_bits());
                assert_eq!(got.max[lane].to_bits(), want.max[lane].to_bits());
            }
        }
        assert_eq!(back.dur.to_bits(), clip.dur.to_bits(), "dur bits");
        assert_eq!(qclip_hash(&back), qclip_hash(&clip), "qclip_hash");
        // Same clip, same bytes, every run; and the wire re-packs to itself.
        assert_eq!(bytes, bake_pack(&clip).expect("bakes again"));
        assert_eq!(
            PackedClip::from_bytes(&bytes).expect("wire").to_bytes(),
            bytes
        );
        // The AM5P magic is on the wire, so the fixture is a real pack.
        assert_eq!(&bytes[..4], b"AM5P");
    }

    #[test]
    fn pack_refuses_a_duration_its_last_knot_does_not_hold() {
        let mut clip = tick_clip(61);
        clip.dur += 0.5;
        assert_eq!(bake_pack(&clip), Err(BakeError::NotRoundTrip));
        // Truncated and foreign bytes name their fault through the wire.
        let bytes = bake_pack(&tick_clip(61)).expect("bakes");
        assert!(matches!(
            bake_unpack(&bytes[..bytes.len() - 1]),
            Err(BakeError::Pack(PackError::Truncated { .. }))
        ));
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(matches!(
            bake_unpack(&bad),
            Err(BakeError::Pack(PackError::BadMagic { .. }))
        ));
    }

    #[test]
    fn the_committed_stagger_fixture_loads_and_matches_its_receipt() {
        // The seed-7 run of `tools/motion_bake bake` wrote these bytes; the
        // receipt it printed names the same hash. Read at compile time, so
        // this gate needs no file I/O and no tool build.
        let bytes: &[u8] = include_bytes!("../../../fixtures/clips/stagger.am5p");
        assert_eq!(&bytes[..4], b"AM5P", "the committed pack is AM5P");
        let clip = bake_unpack(bytes).expect("the committed pack loads");
        assert_eq!(clip.tracks.len(), 65, "the whole fighter body");
        assert_eq!(clip.tracks[0].times.len(), 61, "one second at 60 Hz");
        assert_eq!(
            qclip_hash(&clip),
            0xf5f4_de57_dd5a_7280,
            "the seed-7 receipt's qclip_hash"
        );
        // The pack carries local offsets, so the foot's own track is its
        // offset inside the leg, not its motion over the sand. The left foot
        // is the planted one (track 57), and the slip meter wants its world
        // position: the offsets composed down the leg with the rotations, the
        // same meter the receipt's `avg_slip` line prints.
        let rots = bake_unpack_rots(bytes).expect("the rotations read back");
        let slip = avg_world_slip(&clip, &rots, &fighter_legs(), 57, 61);
        assert!(
            (slip - 0.024264).abs() < 5e-6,
            "the receipt's planted-foot slip is 0.024264 m/s, got {slip}"
        );
        assert!(slip < 0.08, "the planted foot slips {slip}");
        // The committed pack carries the rotations too, so the fixed point is
        // checked on the whole pack, not only on its positions.
        assert_eq!(rots.len(), 65, "one rotation track per joint");
        let mut rebuilt = pack_qclip(&clip).expect("packs");
        let mut rot_tracks = Vec::with_capacity(rots.len());
        for (index, track) in rots.iter().enumerate() {
            rot_tracks.push(
                crate::pack::pack_quat_track(index, &track.times, &track.values)
                    .expect("the quat track packs"),
            );
        }
        rebuilt.rotations = rot_tracks;
        assert_eq!(rebuilt.to_bytes(), bytes, "fixed point");
        println!("AM27_FIXTURE ticks=61 slip={slip:.6}");
    }

    /// One second at 60 Hz with the left foot down for the first twelve ticks (so the 30 Hz
    /// windows that hold ticks 0..10 are whole plants): the lane the IK starts from must read
    /// planted while it is down and clear once it lifts.
    fn planted_lane() -> Vec<[bool; 2]> {
        (0..60u32).map(|tick| [tick <= 11, false]).collect()
    }

    #[test]
    fn foot_weights_are_the_planted_share_of_each_window() {
        let weights = bake_foot_weights(&planted_lane(), FOOT_TICK_HZ).expect("one second bakes");
        assert_eq!(weights.hz, 30, "sampled at OpenKCC's 30 Hz");
        assert_eq!(weights.samples, 30, "half the frames");
        for tick in 0..=10u32 {
            let left = weights.weight_at(tick, 0);
            assert!(left >= 900, "left planted at tick {tick}, weight {left}");
        }
        let lifted = weights.weight_at(20, 0);
        assert!(lifted <= 100, "left lifted by tick 20, weight {lifted}");
        // The right foot never went down.
        assert!((0..60).all(|tick| weights.weight_at(tick, 1) == 0));
        // A window with one planted tick of two is half planted, not a hard 1000 or 0.
        let half =
            bake_foot_weights(&[[true, false], [false, false]], FOOT_TICK_HZ).expect("two frames");
        assert_eq!(half.weight_at(0, 0), 500, "ticks 0 and 1");
        println!(
            "foot weights: tick 0 {} , tick 10 {} , tick 20 {} permille",
            weights.weight_at(0, 0),
            weights.weight_at(10, 0),
            weights.weight_at(20, 0)
        );
    }

    #[test]
    fn foot_weights_refuse_an_empty_lane_and_a_bad_tick_rate() {
        assert_eq!(
            bake_foot_weights(&[], FOOT_TICK_HZ),
            Err(FootWeightError::Empty)
        );
        // 50 Hz does not divide into 30 Hz windows.
        assert_eq!(
            bake_foot_weights(&[[true, true]], 50),
            Err(FootWeightError::BadTickHz(50))
        );
        assert_eq!(
            bake_foot_weights(&[[true, true]; MAX_BAKE_TICKS as usize + 1], FOOT_TICK_HZ),
            Err(FootWeightError::TooLong(MAX_BAKE_TICKS + 1))
        );
        // Past the lane's end a foot is not planted.
        let weights = bake_foot_weights(&[[true, true]], FOOT_TICK_HZ).expect("one frame");
        assert_eq!(weights.weight_at(0, 0), 1000);
        assert_eq!(weights.weight_at(99, 0), 0);
    }
}

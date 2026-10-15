//! pack: quantized ACL-shape clip pack, exact decode.
//!
//! Owner: `engine-anim`; consumers: the clip sampler (`clip`) through
//! [`sample_packed`] plus [`unpack_qclip`], and the hero bake
//! (`loco_bake`) through [`pack_hero_fighter`]. Units: meters, seconds,
//! bytes; fighter budget [`BUDGET_BYTES_PER_FIGHTER`]. Determinism: fixed
//! field order, little-endian bytes, `+ - * /` and `round` only, no clock.
//! Errors: [`PackError`]; corrupt bytes name the fault, never panic.

use super::clip::{QClip, QTrack, TICK_DT, clamp01};
use super::loco_bake::{bake_hero_set, ClipSpec, HERO_SET};

/// Wire magic for every packed clip and fighter blob.
pub const PACK_MAGIC: [u8; 4] = *b"AM5P";
/// Wire version; decoders refuse anything else loudly.
pub const PACK_VERSION: u16 = 1;
/// Packed locomotion bundle per fighter must stay under 2 MB (plan §3).
pub const BUDGET_BYTES_PER_FIGHTER: usize = 2 * 1024 * 1024;
/// Widest clip accepted; wider input is refused, never truncated.
pub const MAX_TRACKS_PER_CLIP: usize = 1024;
/// Widest fighter bundle accepted.
pub const MAX_CLIPS_PER_FIGHTER: usize = 64;
/// Longest single track accepted (covers the 3600-frame bake cap).
pub const MAX_KEYS_PER_TRACK: usize = 65_536;
/// Largest single blob decoded; larger input is refused before allocating.
pub const MAX_PACKED_BYTES: usize = 16 * 1024 * 1024;
/// Uniform quantizer levels per lane (ACL-shape u16 lanes).
pub const QUANT_LEVELS: f32 = 65535.0;

/// Why a pack, unpack or decode step failed; every variant names the fault.
#[derive(Clone, PartialEq, Debug)]
pub enum PackError {
    /// Track `track` has `times` times but `keys` keys; the counts must match.
    BadLength {
        /// Track index inside its clip.
        track: usize,
        /// Time count seen.
        times: usize,
        /// Key count seen.
        keys: usize,
    },
    /// Non-finite float in the named lane (`times`, `values`, `range`, `dur`).
    NonFinite {
        /// Lane carrying NaN or infinity.
        lane: &'static str,
    },
    /// Clip duration is negative; durations are zero or positive.
    BadDuration(f32),
    /// Track `track` times step backwards at `index`; order is non-decreasing.
    TimesNotOrdered {
        /// Track index inside its clip.
        track: usize,
        /// First index that steps backwards.
        index: usize,
    },
    /// Clip carries more than [`MAX_TRACKS_PER_CLIP`] tracks.
    TooManyTracks {
        /// Track count seen.
        got: usize,
    },
    /// Track `track` carries more than [`MAX_KEYS_PER_TRACK`] keys.
    TooManyKeys {
        /// Track index inside its clip.
        track: usize,
        /// Key count seen.
        got: usize,
    },
    /// Fighter bundle carries more than [`MAX_CLIPS_PER_FIGHTER`] clips.
    TooManyClips {
        /// Clip count seen.
        got: usize,
    },
    /// Unknown track time base `got`; only 0 (explicit) and 1 (uniform).
    BadTrackKind(u8),
    /// Blob magic is not [`PACK_MAGIC`]; the bytes seen are carried.
    BadMagic {
        /// First four bytes seen.
        got: [u8; 4],
    },
    /// Blob version is not [`PACK_VERSION`].
    BadVersion {
        /// Version seen.
        got: u16,
    },
    /// Blob ends before the named field.
    Truncated {
        /// Field the reader wanted.
        want: &'static str,
    },
    /// Blob has `bytes` leftover after a full decode.
    Trailing {
        /// Leftover byte count.
        bytes: usize,
    },
    /// Blob is larger than [`MAX_PACKED_BYTES`]; refused before allocating.
    Oversize {
        /// Byte count seen.
        bytes: usize,
    },
    /// The bake backing the hero bundle failed; the cause is carried.
    Bake(String),
}

impl core::fmt::Display for PackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PackError::BadLength { track, times, keys } => {
                write!(f, "pack: track {track} has {times} times but {keys} keys")
            }
            PackError::NonFinite { lane } => {
                write!(f, "pack: non-finite float in {lane}")
            }
            PackError::BadDuration(dur) => {
                write!(f, "pack: negative duration {dur}")
            }
            PackError::TimesNotOrdered { track, index } => {
                write!(f, "pack: track {track} times step backwards at key {index}")
            }
            PackError::TooManyTracks { got } => {
                write!(f, "pack: {got} tracks exceeds the clip cap")
            }
            PackError::TooManyKeys { track, got } => {
                write!(f, "pack: track {track} has {got} keys past the cap")
            }
            PackError::TooManyClips { got } => {
                write!(f, "pack: {got} clips exceeds the fighter cap")
            }
            PackError::BadTrackKind(got) => {
                write!(f, "pack: unknown track time base {got}")
            }
            PackError::BadMagic { got } => {
                write!(f, "pack: bad magic {:02x?}", got)
            }
            PackError::BadVersion { got } => {
                write!(f, "pack: bad version {got}")
            }
            PackError::Truncated { want } => {
                write!(f, "pack: blob ends before {want}")
            }
            PackError::Trailing { bytes } => {
                write!(f, "pack: blob has {bytes} trailing bytes")
            }
            PackError::Oversize { bytes } => {
                write!(f, "pack: blob of {bytes} bytes exceeds the cap")
            }
            PackError::Bake(cause) => {
                write!(f, "pack: hero bake failed ({cause})")
            }
        }
    }
}

impl std::error::Error for PackError {}

/// Time base for one packed track: explicit knots, or an exact uniform grid.
#[derive(Clone, PartialEq, Debug)]
pub enum TimeBase {
    /// `times[k] == t0 + k * dt` bit-exact for every key.
    Uniform {
        /// First knot.
        t0: f32,
        /// Step between knots.
        dt: f32,
    },
    /// One knot per key, non-decreasing.
    Explicit {
        /// Knots, one per key.
        times: Vec<f32>,
    },
}

/// One packed joint track: ACL-shape u16 lanes plus range and time base.
#[derive(Clone, PartialEq, Debug)]
pub struct PackedTrack {
    /// Knot storage; key count always equals the knot count.
    pub time: TimeBase,
    /// Quantized keys, one per knot.
    pub keys: Vec<[u16; 3]>,
    /// Per-lane minima; decode of key 0 lands here when the span is zero.
    pub min: [f32; 3],
    /// Per-lane maxima.
    pub max: [f32; 3],
}

/// One packed clip: packed tracks plus duration.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PackedClip {
    /// Tracks in joint order.
    pub tracks: Vec<PackedTrack>,
    /// Clip duration in seconds, copied from the source clip.
    pub dur: f32,
}

/// One fighter bundle: every packed locomotion clip for the hero set.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct FighterPack {
    /// Clips in bake order.
    pub clips: Vec<PackedClip>,
}

/// One chart row: packed size plus decode error against its bound.
#[derive(Clone, PartialEq, Debug)]
pub struct ErrorRow {
    /// Clip name from the bake spec.
    pub name: &'static str,
    /// Clip index inside the fighter bundle.
    pub clip: usize,
    /// Track index inside its clip (0 = left foot, 1 = right foot).
    pub track: usize,
    /// Key count.
    pub keys: usize,
    /// Raw float32 estimate in bytes (16 per key: time plus value).
    pub raw_bytes: usize,
    /// Exact packed bytes for this track.
    pub packed_bytes: usize,
    /// Worst lane error over every key, in meters.
    pub max_abs_err: f32,
    /// Worst-case rounding bound for this track, in meters.
    pub bound: f32,
}

/// The packed hero fighter: bundle, per-track chart and the bake digest.
#[derive(Clone, PartialEq, Debug)]
pub struct HeroBundle {
    /// Packed clips in [`HERO_SET`] order.
    pub pack: FighterPack,
    /// One row per track, same order as the bundle.
    pub rows: Vec<ErrorRow>,
    /// Digest of the backing bake; replay must reproduce it exactly.
    pub ct3_digest: u64,
    /// Raw float32 estimate over the whole bundle, in bytes.
    pub raw_bytes: usize,
    /// Exact packed bytes over the whole bundle.
    pub packed_bytes: usize,
}

impl PackedClip {
    /// Track count.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// True when the clip holds no tracks.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Exact serialized byte count without allocating.
    pub fn packed_len(&self) -> usize {
        let mut bytes = 14;
        for track in &self.tracks {
            bytes += track.packed_len();
        }
        bytes
    }

    /// Deterministic little-endian bytes; same clip, same bytes, every run.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.packed_len());
        push_header(&mut out, self.dur, self.tracks.len() as u32);
        for track in &self.tracks {
            push_track(&mut out, track);
        }
        out
    }

    /// Inverse of [`PackedClip::to_bytes`]; corrupt input names its fault.
    pub fn from_bytes(buf: &[u8]) -> Result<Self, PackError> {
        if buf.len() > MAX_PACKED_BYTES {
            return Err(PackError::Oversize { bytes: buf.len() });
        }
        let mut read = Reader { buf, pos: 0 };
        let clip = read.clip_body()?;
        read.end()?;
        Ok(clip)
    }
}

impl FighterPack {
    /// Clip count.
    pub fn len(&self) -> usize {
        self.clips.len()
    }

    /// True when the bundle holds no clips.
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// Exact serialized byte count without allocating.
    pub fn packed_len(&self) -> usize {
        let mut bytes = 10;
        for clip in &self.clips {
            bytes += clip.packed_len();
        }
        bytes
    }

    /// Deterministic little-endian bytes; same bundle, same bytes, every run.
    ///
    /// Each clip is framed exactly like [`PackedClip::to_bytes`], so the
    /// fighter reader reuses the clip reader with no second format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.packed_len());
        out.extend_from_slice(&PACK_MAGIC);
        push_u16(&mut out, PACK_VERSION);
        push_u32(&mut out, self.clips.len() as u32);
        for clip in &self.clips {
            out.extend_from_slice(&clip.to_bytes());
        }
        out
    }

    /// Inverse of [`FighterPack::to_bytes`]; corrupt input names its fault.
    pub fn from_bytes(buf: &[u8]) -> Result<Self, PackError> {
        if buf.len() > MAX_PACKED_BYTES {
            return Err(PackError::Oversize { bytes: buf.len() });
        }
        let mut read = Reader { buf, pos: 0 };
        read.magic()?;
        read.version()?;
        let nclips = read.u32("clip count")?;
        if nclips as usize > MAX_CLIPS_PER_FIGHTER {
            return Err(PackError::TooManyClips {
                got: nclips as usize,
            });
        }
        let mut clips = Vec::with_capacity(nclips as usize);
        for _ in 0..nclips {
            clips.push(read.clip_body()?);
        }
        read.end()?;
        Ok(Self { clips })
    }
}

impl PackedTrack {
    /// Key count, always equal to the knot count.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Exact serialized byte count for this track without allocating.
    pub fn packed_len(&self) -> usize {
        let n = self.keys.len();
        match &self.time {
            TimeBase::Uniform { .. } => 24 + 1 + 4 + 4 + 4 + 6 * n,
            TimeBase::Explicit { .. } => 24 + 1 + 4 + 4 * n + 6 * n,
        }
    }

    /// Knot `i`; callers hold `i < key_count()`.
    fn time_at(&self, i: usize) -> f32 {
        match &self.time {
            TimeBase::Uniform { t0, dt } => *t0 + (i as f32) * *dt,
            TimeBase::Explicit { times } => times[i],
        }
    }
}

/// Dequantize one key back to meters.
fn dequantize(key: [u16; 3], min: [f32; 3], max: [f32; 3]) -> [f32; 3] {
    [
        min[0] + (max[0] - min[0]) * (f32::from(key[0]) / QUANT_LEVELS),
        min[1] + (max[1] - min[1]) * (f32::from(key[1]) / QUANT_LEVELS),
        min[2] + (max[2] - min[2]) * (f32::from(key[2]) / QUANT_LEVELS),
    ]
}

/// Quantize one value to u16 lanes; zero spans map to 0 (decode reads `min`).
fn quantize(value: [f32; 3], min: [f32; 3], max: [f32; 3]) -> [u16; 3] {
    let mut key = [0u16; 3];
    for lane in 0..3 {
        let span = max[lane] - min[lane];
        key[lane] = if span <= 0.0 {
            0
        } else {
            (clamp01((value[lane] - min[lane]) / span) * QUANT_LEVELS)
                .round()
                .clamp(0.0, QUANT_LEVELS) as u16
        };
    }
    key
}

/// Exact uniform grid for `times`, or `None` when knots are irregular.
/// The encode check uses the same expression the decoder runs, so a `Some`
/// grid always decodes back bit-exact.
fn uniform_base(times: &[f32]) -> Option<(f32, f32)> {
    match times.len() {
        0 => None,
        1 => Some((times[0], 0.0)),
        n => {
            let t0 = times[0];
            let dt = (times[n - 1] - t0) / ((n - 1) as f32);
            if !dt.is_finite() || dt <= 0.0 {
                return None;
            }
            for (k, knot) in times.iter().enumerate() {
                if knot.to_bits() != (t0 + (k as f32) * dt).to_bits() {
                    return None;
                }
            }
            Some((t0, dt))
        }
    }
}

/// Pack one raw track: validate, range, quantize, collapse uniform knots.
/// Empty input packs to an empty track; degenerate clips stay representable.
fn pack_track_at(
    track: usize,
    times: &[f32],
    values: &[[f32; 3]],
) -> Result<PackedTrack, PackError> {
    if times.len() != values.len() {
        return Err(PackError::BadLength {
            track,
            times: times.len(),
            keys: values.len(),
        });
    }
    if values.len() > MAX_KEYS_PER_TRACK {
        return Err(PackError::TooManyKeys {
            track,
            got: values.len(),
        });
    }
    for knot in times {
        if !knot.is_finite() {
            return Err(PackError::NonFinite { lane: "times" });
        }
    }
    for value in values {
        if value.iter().any(|lane| !lane.is_finite()) {
            return Err(PackError::NonFinite { lane: "values" });
        }
    }
    for (index, pair) in times.windows(2).enumerate() {
        if pair[1] < pair[0] {
            return Err(PackError::TimesNotOrdered {
                track,
                index: index + 1,
            });
        }
    }
    if values.is_empty() {
        return Ok(PackedTrack {
            time: TimeBase::Explicit { times: Vec::new() },
            keys: Vec::new(),
            min: [0.0; 3],
            max: [0.0; 3],
        });
    }
    let (mut min, mut max) = (values[0], values[0]);
    for value in &values[1..] {
        for lane in 0..3 {
            if value[lane] < min[lane] {
                min[lane] = value[lane];
            }
            if value[lane] > max[lane] {
                max[lane] = value[lane];
            }
        }
    }
    let keys = values
        .iter()
        .map(|value| quantize(*value, min, max))
        .collect();
    let time = match uniform_base(times) {
        Some((t0, dt)) => TimeBase::Uniform { t0, dt },
        None => TimeBase::Explicit {
            times: times.to_vec(),
        },
    };
    Ok(PackedTrack {
        time,
        keys,
        min,
        max,
    })
}

/// Pack a baked quantized clip; unpacking it returns the clip bit-exact.
/// Refuses mismatched lanes, non-finite ranges, unordered knots and caps.
pub fn pack_qclip(clip: &QClip) -> Result<PackedClip, PackError> {
    if clip.tracks.len() > MAX_TRACKS_PER_CLIP {
        return Err(PackError::TooManyTracks {
            got: clip.tracks.len(),
        });
    }
    if !clip.dur.is_finite() {
        return Err(PackError::NonFinite { lane: "dur" });
    }
    if clip.dur < 0.0 {
        return Err(PackError::BadDuration(clip.dur));
    }
    let mut tracks = Vec::with_capacity(clip.tracks.len());
    for (index, track) in clip.tracks.iter().enumerate() {
        if track.times.len() != track.keys.len() {
            return Err(PackError::BadLength {
                track: index,
                times: track.times.len(),
                keys: track.keys.len(),
            });
        }
        if track.keys.len() > MAX_KEYS_PER_TRACK {
            return Err(PackError::TooManyKeys {
                track: index,
                got: track.keys.len(),
            });
        }
        for lane in track.min.iter().chain(track.max.iter()) {
            if !lane.is_finite() {
                return Err(PackError::NonFinite { lane: "range" });
            }
        }
        for knot in &track.times {
            if !knot.is_finite() {
                return Err(PackError::NonFinite { lane: "times" });
            }
        }
        for (at, pair) in track.times.windows(2).enumerate() {
            if pair[1] < pair[0] {
                return Err(PackError::TimesNotOrdered {
                    track: index,
                    index: at + 1,
                });
            }
        }
        let time = match uniform_base(&track.times) {
            Some((t0, dt)) => TimeBase::Uniform { t0, dt },
            None => TimeBase::Explicit {
                times: track.times.clone(),
            },
        };
        tracks.push(PackedTrack {
            time,
            keys: track.keys.clone(),
            min: track.min,
            max: track.max,
        });
    }
    Ok(PackedClip {
        tracks,
        dur: clip.dur,
    })
}

/// Exact inverse of [`pack_qclip`]: times, keys, ranges and duration return
/// bit-exact, so `qclip_hash` is unchanged across the round trip.
pub fn unpack_qclip(packed: &PackedClip) -> QClip {
    let mut tracks = Vec::with_capacity(packed.tracks.len());
    for track in &packed.tracks {
        let n = track.keys.len();
        let mut times = Vec::with_capacity(n);
        for k in 0..n {
            times.push(track.time_at(k));
        }
        tracks.push(QTrack {
            times,
            keys: track.keys.clone(),
            min: track.min,
            max: track.max,
        });
    }
    let mut dur = 0.0f32;
    for track in &tracks {
        if let Some(knot) = track.times.last() {
            if *knot > dur {
                dur = *knot;
            }
        }
    }
    if packed.tracks.is_empty() {
        dur = if packed.dur.is_finite() && packed.dur >= 0.0 {
            packed.dur
        } else {
            0.0
        };
    }
    QClip { tracks, dur }
}

/// Worst lane error of a packed track against its raw values, plus the
/// rounding bound (half a quantum on the widest lane). Empty tracks read zero.
pub fn track_error(packed: &PackedTrack, values: &[[f32; 3]]) -> Result<(f32, f32), PackError> {
    if packed.keys.len() != values.len() {
        return Err(PackError::BadLength {
            track: 0,
            times: packed.keys.len(),
            keys: values.len(),
        });
    }
    let mut bound = 0.0f32;
    for lane in 0..3 {
        let span = packed.max[lane] - packed.min[lane];
        if span > 0.0 {
            bound = bound.max(span / (2.0 * QUANT_LEVELS));
        }
    }
    let mut worst = 0.0f32;
    for (key, value) in packed.keys.iter().zip(values.iter()) {
        let back = dequantize(*key, packed.min, packed.max);
        for lane in 0..3 {
            worst = worst.max((back[lane] - value[lane]).abs());
        }
    }
    Ok((worst, bound))
}

/// Sample one packed track like the runtime sampler: clamp `t`, walk the
/// forward `hint`, lerp dequantized endpoints. Out-of-range tracks and empty
/// tracks read zero, matching [`sample_track`]; steady-state calls allocate
/// nothing. Only `+ - * /` run here, so the tick math stays deterministic.
pub fn sample_packed(clip: &PackedClip, track: usize, t: f32, hint: &mut usize) -> [f32; 3] {
    let packed = match clip.tracks.get(track) {
        Some(packed) if packed.keys.len() == knot_count(packed) && !packed.keys.is_empty() => {
            packed
        }
        _ => return [0.0; 3],
    };
    let last = packed.keys.len() - 1;
    if t <= packed.time_at(0) {
        *hint = 0;
        return dequantize(packed.keys[0], packed.min, packed.max);
    }
    if t >= packed.time_at(last) {
        *hint = last;
        return dequantize(packed.keys[last], packed.min, packed.max);
    }
    let (lo, f) = packed_interval(packed, *hint, t);
    *hint = lo;
    let a = dequantize(packed.keys[lo], packed.min, packed.max);
    let b = dequantize(packed.keys[lo + 1], packed.min, packed.max);
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

/// Knot count behind [`sample_packed`]'s shape check.
fn knot_count(packed: &PackedTrack) -> usize {
    match &packed.time {
        TimeBase::Uniform { .. } => packed.keys.len(),
        TimeBase::Explicit { times } => times.len(),
    }
}

/// Interval walk shared with the clip sampler shape: forward scan on `hint`,
/// binary fallback on seeks, fraction clamped. NaN fractions hold the key.
fn packed_interval(packed: &PackedTrack, hint: usize, t: f32) -> (usize, f32) {
    let last = packed.keys.len() - 1;
    let mut lo = hint.min(last);
    if packed.time_at(lo) <= t {
        while lo < last && packed.time_at(lo + 1) <= t {
            lo += 1;
        }
        if lo >= last {
            return (last, 0.0);
        }
    } else {
        let (mut a, mut b) = (0usize, last);
        while b - a > 1 {
            let m = (a + b) / 2;
            if packed.time_at(m) <= t {
                a = m;
            } else {
                b = m;
            }
        }
        lo = a;
    }
    let (t0, t1) = (packed.time_at(lo), packed.time_at(lo + 1));
    let span = t1 - t0;
    (
        lo,
        if span <= 0.0 {
            0.0
        } else {
            clamp01((t - t0) / span)
        },
    )
}

/// Raw foot trajectories for one hero spec: 60 Hz knots plus the triangle
/// wave feet from the hero bake recipe (same phase ramp and swing), so the
/// packed bundle replays the baked locomotion set.
fn hero_raw_values(spec: &ClipSpec) -> (Vec<f32>, Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let n = usize::from(spec.frames);
    let span = f32::from(spec.frames);
    let mut times = Vec::with_capacity(n);
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for frame in 0..n {
        times.push(frame as f32 * TICK_DT);
        let phase = (frame as f32) / span;
        let tri = if phase < 0.5 {
            phase * 2.0
        } else {
            2.0 - phase * 2.0
        };
        let amp = spec.swing;
        left.push([tri * amp - amp * 0.5, 0.05, 0.0]);
        right.push([(1.0 - tri) * amp - amp * 0.5, 0.05, 0.0]);
    }
    (times, left, right)
}

/// Pack the whole hero set for the motion database bake lane: every clip
/// becomes one two-track (left/right foot) packed clip, with per-track error
/// rows and the bake digest this bundle must replay.
pub fn pack_hero_fighter() -> Result<HeroBundle, PackError> {
    let set = bake_hero_set().map_err(|cause| PackError::Bake(cause.to_string()))?;
    let mut clips = Vec::with_capacity(HERO_SET.len());
    let mut rows = Vec::with_capacity(HERO_SET.len() * 2);
    let mut raw_bytes = 0usize;
    for (index, spec) in HERO_SET.iter().enumerate() {
        let (times, left, right) = hero_raw_values(spec);
        let packed_left = pack_track_at(0, &times, &left)?;
        let packed_right = pack_track_at(1, &times, &right)?;
        for (lane, packed, values) in [
            (0usize, &packed_left, left.as_slice()),
            (1usize, &packed_right, right.as_slice()),
        ] {
            let (max_abs_err, bound) = track_error(packed, values)?;
            raw_bytes += values.len() * 16;
            rows.push(ErrorRow {
                name: spec.name,
                clip: index,
                track: lane,
                keys: values.len(),
                raw_bytes: values.len() * 16,
                packed_bytes: packed.packed_len(),
                max_abs_err,
                bound,
            });
        }
        let dur = times.last().copied().unwrap_or(0.0);
        clips.push(PackedClip {
            tracks: vec![packed_left, packed_right],
            dur,
        });
    }
    let pack = FighterPack { clips };
    Ok(HeroBundle {
        packed_bytes: pack.packed_len(),
        raw_bytes,
        ct3_digest: set.digest,
        pack,
        rows,
    })
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn push_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_bits().to_le_bytes());
}

fn push_header(out: &mut Vec<u8>, dur: f32, ntracks: u32) {
    out.extend_from_slice(&PACK_MAGIC);
    push_u16(out, PACK_VERSION);
    push_f32(out, dur);
    push_u32(out, ntracks);
}

fn push_track(out: &mut Vec<u8>, track: &PackedTrack) {
    for lane in track.min.iter().chain(track.max.iter()) {
        push_f32(out, *lane);
    }
    match &track.time {
        TimeBase::Uniform { t0, dt } => {
            out.push(1);
            push_f32(out, *t0);
            push_f32(out, *dt);
            push_u32(out, track.keys.len() as u32);
        }
        TimeBase::Explicit { times } => {
            out.push(0);
            push_u32(out, track.keys.len() as u32);
            for knot in times {
                push_f32(out, *knot);
            }
        }
    }
    for key in &track.keys {
        for lane in key {
            push_u16(out, *lane);
        }
    }
}

/// Bounded little-endian reader; every short read names its field.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, want: &'static str) -> Result<&'a [u8], PackError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(PackError::Truncated { want })?;
        if end > self.buf.len() {
            return Err(PackError::Truncated { want });
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self, want: &'static str) -> Result<u8, PackError> {
        Ok(self.take(1, want)?[0])
    }

    fn u16(&mut self, want: &'static str) -> Result<u16, PackError> {
        let bytes = self.take(2, want)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self, want: &'static str) -> Result<u32, PackError> {
        let bytes = self.take(4, want)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn f32(&mut self, want: &'static str) -> Result<f32, PackError> {
        let bytes = self.take(4, want)?;
        Ok(f32::from_bits(u32::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3],
        ])))
    }

    fn magic(&mut self) -> Result<(), PackError> {
        let bytes = self.take(4, "magic")?;
        let got = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if got != PACK_MAGIC {
            return Err(PackError::BadMagic { got });
        }
        Ok(())
    }

    fn version(&mut self) -> Result<(), PackError> {
        let got = self.u16("version")?;
        if got != PACK_VERSION {
            return Err(PackError::BadVersion { got });
        }
        Ok(())
    }

    fn finite_f32(&mut self, lane: &'static str) -> Result<f32, PackError> {
        let v = self.f32(lane)?;
        if !v.is_finite() {
            return Err(PackError::NonFinite { lane });
        }
        Ok(v)
    }

    fn clip_body(&mut self) -> Result<PackedClip, PackError> {
        self.magic()?;
        self.version()?;
        let dur = self.finite_f32("dur")?;
        if dur < 0.0 {
            return Err(PackError::BadDuration(dur));
        }
        let ntracks = self.u32("track count")?;
        if ntracks as usize > MAX_TRACKS_PER_CLIP {
            return Err(PackError::TooManyTracks {
                got: ntracks as usize,
            });
        }
        let mut tracks = Vec::with_capacity(ntracks as usize);
        for _ in 0..ntracks {
            tracks.push(self.track_body()?);
        }
        Ok(PackedClip { tracks, dur })
    }

    fn track_body(&mut self) -> Result<PackedTrack, PackError> {
        let mut min = [0.0f32; 3];
        let mut max = [0.0f32; 3];
        for lane in min.iter_mut() {
            *lane = self.finite_f32("range")?;
        }
        for lane in max.iter_mut() {
            *lane = self.finite_f32("range")?;
        }
        let kind = self.u8("track kind")?;
        let (time, n) = match kind {
            0 => {
                let n = self.u32("key count")?;
                if n as usize > MAX_KEYS_PER_TRACK {
                    return Err(PackError::TooManyKeys {
                        track: 0,
                        got: n as usize,
                    });
                }
                let mut times = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    times.push(self.finite_f32("times")?);
                }
                for (index, pair) in times.windows(2).enumerate() {
                    if pair[1] < pair[0] {
                        return Err(PackError::TimesNotOrdered {
                            track: 0,
                            index: index + 1,
                        });
                    }
                }
                (TimeBase::Explicit { times }, n as usize)
            }
            1 => {
                let t0 = self.finite_f32("times")?;
                let dt = self.finite_f32("times")?;
                let n = self.u32("key count")?;
                if n as usize > MAX_KEYS_PER_TRACK {
                    return Err(PackError::TooManyKeys {
                        track: 0,
                        got: n as usize,
                    });
                }
                (TimeBase::Uniform { t0, dt }, n as usize)
            }
            other => return Err(PackError::BadTrackKind(other)),
        };
        let mut keys = Vec::with_capacity(n);
        for _ in 0..n {
            let mut key = [0u16; 3];
            for lane in key.iter_mut() {
                *lane = self.u16("keys")?;
            }
            keys.push(key);
        }
        Ok(PackedTrack {
            time,
            keys,
            min,
            max,
        })
    }

    fn end(&self) -> Result<(), PackError> {
        if self.pos != self.buf.len() {
            return Err(PackError::Trailing {
                bytes: self.buf.len() - self.pos,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::clip::{bake_clip, bake_track, qclip_hash, sample_track};
    use super::*;
    use engine_core::hash_bytes;

    fn lin_qclip() -> QClip {
        bake_clip(vec![
            bake_track(vec![0.0, 1.0], vec![[0.0, 0.0, 0.0], [2.0, 4.0, 6.0]]),
            bake_track(
                vec![0.0, 0.5, 1.0],
                vec![[1.0, 0.0, 0.0], [1.5, 0.5, 0.0], [1.0, 0.0, 0.0]],
            ),
        ])
    }

    /// Bit-exact track equality (f32 `==` hides `-0.0`, so compare bits).
    fn assert_tracks_exact(got: &QTrack, want: &QTrack) {
        assert_eq!(got.keys, want.keys, "keys differ");
        assert_eq!(got.times.len(), want.times.len(), "time counts differ");
        for (a, b) in got.times.iter().zip(want.times.iter()) {
            assert_eq!(a.to_bits(), b.to_bits(), "knot bits differ");
        }
        for lane in 0..3 {
            assert_eq!(
                got.min[lane].to_bits(),
                want.min[lane].to_bits(),
                "min bits differ"
            );
            assert_eq!(
                got.max[lane].to_bits(),
                want.max[lane].to_bits(),
                "max bits differ"
            );
        }
    }

    #[test]
    fn unpack_returns_source_bit_exact() {
        for clip in [
            lin_qclip(),
            QClip::default(),
            bake_clip(vec![bake_track(vec![0.25], vec![[1.0, -2.0, 3.0]])]),
            bake_clip(vec![bake_track(
                vec![0.0, 0.5, 1.0],
                vec![[0.4, 0.4, 0.4], [0.4, 0.4, 0.4], [0.4, 0.4, 0.4]],
            )]),
        ] {
            let packed = pack_qclip(&clip).expect("packs");
            let back = unpack_qclip(&packed);
            assert_eq!(back.tracks.len(), clip.tracks.len());
            for (a, b) in back.tracks.iter().zip(clip.tracks.iter()) {
                assert_tracks_exact(a, b);
            }
            assert_eq!(back.dur.to_bits(), clip.dur.to_bits());
            assert_eq!(qclip_hash(&back), qclip_hash(&clip));
        }
    }

    #[test]
    fn wire_bytes_deterministic_and_stable() {
        let clip = lin_qclip();
        let packed = pack_qclip(&clip).expect("packs");
        assert_eq!(packed.packed_len(), packed.to_bytes().len());
        let first = packed.to_bytes();
        let second = packed.to_bytes();
        assert_eq!(first, second, "same clip must pack to same bytes");
        assert_eq!(hash_bytes(&first), hash_bytes(&second));
        let back = PackedClip::from_bytes(&first).expect("decodes");
        // Decode preserves the quantized payload bit-exact.
        let lint = unpack_qclip(&back);
        for (a, b) in lint.tracks.iter().zip(clip.tracks.iter()) {
            assert_tracks_exact(a, b);
        }
        // Re-packing decoded bytes is stable: decode/encode is a fixed point.
        assert_eq!(back.to_bytes(), first);
        // Empty clips round-trip too, with matching length accounting.
        let empty = pack_qclip(&QClip::default()).expect("empty packs");
        assert_eq!(empty.packed_len(), empty.to_bytes().len());
        assert_eq!(
            PackedClip::from_bytes(&empty.to_bytes())
                .expect("empty decodes")
                .to_bytes(),
            empty.to_bytes()
        );
    }

    #[test]
    fn uniform_grids_collapse_to_two_floats() {
        let bundle = pack_hero_fighter().expect("hero packs");
        // Every hero track rides the 60 Hz grid, so the uniform fast path
        // must engage on all of them (the ACL-shape time win).
        for clip in &bundle.pack.clips {
            for track in &clip.tracks {
                assert!(
                    matches!(track.time, TimeBase::Uniform { .. }),
                    "hero knots must stay uniform"
                );
            }
        }
        // An irregular track keeps explicit knots and still round-trips.
        let odd = bake_clip(vec![bake_track(
            vec![0.0, 0.3, 1.0],
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        )]);
        let packed = pack_qclip(&odd).expect("packs");
        assert!(matches!(packed.tracks[0].time, TimeBase::Explicit { .. }));
        let back = unpack_qclip(&packed);
        assert_tracks_exact(&back.tracks[0], &odd.tracks[0]);
    }

    #[test]
    fn budget_and_error_chart() {
        let bundle = pack_hero_fighter().expect("hero packs");
        assert_eq!(bundle.pack.clips.len(), 8, "hero set holds 8 clips");
        assert_eq!(bundle.rows.len(), 16, "two foot tracks per clip");
        assert!(
            bundle.packed_bytes <= BUDGET_BYTES_PER_FIGHTER,
            "packed {} over budget {}",
            bundle.packed_bytes,
            BUDGET_BYTES_PER_FIGHTER
        );
        assert!(
            bundle.packed_bytes < bundle.raw_bytes,
            "pack must beat raw ({} vs {})",
            bundle.packed_bytes,
            bundle.raw_bytes
        );
        println!(
            "AM5_BUDGET packed={} raw={} budget={} ratio={:.3}",
            bundle.packed_bytes,
            bundle.raw_bytes,
            BUDGET_BYTES_PER_FIGHTER,
            bundle.raw_bytes as f32 / bundle.packed_bytes as f32
        );
        for row in &bundle.rows {
            // Half a quantum on a sub-meter span is single-digit micrometers.
            assert!(
                row.bound <= 2e-5,
                "chart bound too loose on {}: {}",
                row.name,
                row.bound
            );
            assert!(
                row.max_abs_err <= row.bound + 1e-6,
                "decode error {} over bound {} on {}",
                row.max_abs_err,
                row.bound,
                row.name
            );
            assert_eq!(row.packed_bytes, {
                let clip = &bundle.pack.clips[row.clip];
                clip.tracks[row.track].packed_len()
            });
            println!(
                "AM5_CHART clip={} track={} keys={} raw={} packed={} max_err={:.9} bound={:.9}",
                row.name,
                row.track,
                row.keys,
                row.raw_bytes,
                row.packed_bytes,
                row.max_abs_err,
                row.bound
            );
        }
    }

    #[test]
    fn ct3_bake_replays_through_pack() {
        let first = pack_hero_fighter().expect("first pack");
        let second = pack_hero_fighter().expect("second pack");
        assert_eq!(first.pack.to_bytes(), second.pack.to_bytes());
        assert_eq!(first.ct3_digest, second.ct3_digest);
        assert_eq!(first.rows, second.rows);
        // The bundle digest is the live bake digest, not a restated constant.
        let live = bake_hero_set().expect("live CT3 bake");
        assert_eq!(first.ct3_digest, live.digest);
        println!(
            "AM5_CT3 clips={} digest=0x{:016x} packed={}",
            first.pack.clips.len(),
            first.ct3_digest,
            first.packed_bytes
        );
        // Fighter wire bytes round-trip and keep the digest path intact.
        let bytes = first.pack.to_bytes();
        assert_eq!(first.pack.packed_len(), bytes.len());
        let back = FighterPack::from_bytes(&bytes).expect("fighter decodes");
        assert_eq!(back.to_bytes(), bytes);
        assert_eq!(back.packed_len(), bytes.len());
    }

    #[test]
    fn packed_samples_match_runtime_sampler() {
        let bundle = pack_hero_fighter().expect("hero packs");
        for (index, clip) in bundle.pack.clips.iter().enumerate() {
            let raw = unpack_qclip(clip);
            for track in 0..clip.tracks.len() {
                // Sweep inside, outside and exactly on knots, with fresh,
                // forward and poisoned hints each time.
                let dur = clip.dur;
                let mut probe = [
                    -1.0f32,
                    0.0,
                    dur * 0.37,
                    dur * 0.73,
                    dur,
                    dur + 1.0,
                    f32::NAN,
                ];
                // Pin the recipe joints too: frame 0 holds `-swing/2`.
                probe[1] = 0.0;
                let _ = index;
                for t in probe {
                    for hint in [0usize, usize::MAX / 2, 3usize] {
                        let mut hp = hint;
                        let mut hq = hint;
                        let got = sample_packed(clip, track, t, &mut hp);
                        let want = sample_track(&raw.tracks[track], t, &mut hq);
                        assert_eq!(got, want, "clip {index} track {track} t={t}");
                        assert_eq!(hp, hq, "hints must agree");
                    }
                }
            }
        }
        // Out-of-range tracks and empty clips read zero, like the sampler.
        let mut hint = 0;
        assert_eq!(
            sample_packed(&PackedClip::default(), 0, 0.5, &mut hint),
            [0.0; 3]
        );
        assert_eq!(
            sample_packed(&pack_qclip(&lin_qclip()).expect("packs"), 9, 0.5, &mut hint),
            [0.0; 3]
        );
    }

    #[test]
    fn blend_error_stays_bounded() {
        // Two packed tracks with different spans, blended mid-way: the blend
        // error is a convex mix, so it cannot exceed the worse input error.
        let a_times = vec![0.0, 1.0];
        let a_vals = vec![[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]];
        let b_times = vec![0.0, 1.0];
        let b_vals = vec![[0.0, 0.0, 0.0], [0.5, -1.0, 0.25]];
        let a = pack_track_at(0, &a_times, &a_vals).expect("packs a");
        let b = pack_track_at(1, &b_times, &b_vals).expect("packs b");
        let ca = PackedClip {
            tracks: vec![a],
            dur: 1.0,
        };
        let cb = PackedClip {
            tracks: vec![b],
            dur: 1.0,
        };
        let t = 0.37f32;
        let w = 0.4f32;
        let mut hint = 0;
        let pa = sample_packed(&ca, 0, t, &mut hint);
        let mut hint = 0;
        let pb = sample_packed(&cb, 0, t, &mut hint);
        let blended = [
            pa[0] + (pb[0] - pa[0]) * w,
            pa[1] + (pb[1] - pa[1]) * w,
            pa[2] + (pb[2] - pa[2]) * w,
        ];
        // Raw reference: exact lerp of the source values at the same time.
        let ra = [
            a_vals[0][0] + (a_vals[1][0] - a_vals[0][0]) * t,
            a_vals[0][1] + (a_vals[1][1] - a_vals[0][1]) * t,
            a_vals[0][2] + (a_vals[1][2] - a_vals[0][2]) * t,
        ];
        let rb = [
            b_vals[0][0] + (b_vals[1][0] - b_vals[0][0]) * t,
            b_vals[0][1] + (b_vals[1][1] - b_vals[0][1]) * t,
            b_vals[0][2] + (b_vals[1][2] - b_vals[0][2]) * t,
        ];
        let want = [
            ra[0] + (rb[0] - ra[0]) * w,
            ra[1] + (rb[1] - ra[1]) * w,
            ra[2] + (rb[2] - ra[2]) * w,
        ];
        let (erra, bounda) = track_error(&ca.tracks[0], &a_vals).expect("err a");
        let (errb, boundb) = track_error(&cb.tracks[0], &b_vals).expect("err b");
        let worst = erra.max(errb) + 1e-6;
        for lane in 0..3 {
            assert!(
                (blended[lane] - want[lane]).abs() <= worst,
                "lane {lane}: {} vs {} (worst {worst})",
                blended[lane],
                want[lane]
            );
        }
        assert!(bounda < 1e-4 && boundb < 1e-4, "{bounda} {boundb}");
    }

    #[test]
    fn degenerate_clips_pack_and_sample() {
        // Single key: every sample holds the constant.
        let one = bake_clip(vec![bake_track(vec![0.0], vec![[1.0, 2.0, 3.0]])]);
        let packed = pack_qclip(&one).expect("single key packs");
        let mut hint = 0;
        for t in [-1.0, 0.0, 0.5, 99.0] {
            let got = sample_packed(&packed, 0, t, &mut hint);
            for lane in 0..3 {
                assert!((got[lane] - [1.0, 2.0, 3.0][lane]).abs() < 1e-3, "{got:?}");
            }
        }
        // Constant track: zero span decodes exactly, no quantum at all.
        let flat = bake_clip(vec![bake_track(
            vec![0.0, 0.5, 1.0],
            vec![[0.4, 0.4, 0.4], [0.4, 0.4, 0.4], [0.4, 0.4, 0.4]],
        )]);
        let packed = pack_qclip(&flat).expect("flat packs");
        assert_eq!(packed.tracks[0].keys, vec![[0, 0, 0]; 3]);
        let (err, bound) = track_error(&packed.tracks[0], &flat_raw()).expect("err");
        assert_eq!((err, bound), (0.0, 0.0));
        // Empty track inside a clip, and duplicate knots: both survive.
        let mixed = QClip {
            tracks: vec![
                QTrack {
                    times: Vec::new(),
                    keys: Vec::new(),
                    min: [0.0; 3],
                    max: [0.0; 3],
                },
                QTrack {
                    times: vec![0.0, 0.0, 1.0],
                    keys: vec![[0, 0, 0], [32768, 32768, 32768], [65535, 65535, 65535]],
                    min: [0.0; 3],
                    max: [1.0, 2.0, 3.0],
                },
            ],
            dur: 1.0,
        };
        let packed = pack_qclip(&mixed).expect("degenerate packs");
        let back = unpack_qclip(&packed);
        assert_tracks_exact(&back.tracks[0], &mixed.tracks[0]);
        assert_tracks_exact(&back.tracks[1], &mixed.tracks[1]);
        let mut hint = 0;
        let at_dup = sample_packed(&packed, 1, 0.0, &mut hint);
        assert!(at_dup.iter().all(|v| v.is_finite()), "{at_dup:?}");
        let mut hint = 0;
        assert_eq!(sample_packed(&packed, 0, 0.5, &mut hint), [0.0; 3]);
    }

    fn flat_raw() -> Vec<[f32; 3]> {
        vec![[0.4, 0.4, 0.4], [0.4, 0.4, 0.4], [0.4, 0.4, 0.4]]
    }

    #[test]
    fn bad_clips_are_refused() {
        // Mismatched lanes.
        let bad = QClip {
            tracks: vec![QTrack {
                times: vec![0.0, 1.0],
                keys: vec![[0, 0, 0]],
                min: [0.0; 3],
                max: [1.0; 3],
            }],
            dur: 1.0,
        };
        assert_eq!(
            pack_qclip(&bad),
            Err(PackError::BadLength {
                track: 0,
                times: 2,
                keys: 1
            })
        );
        // NaN value, NaN knot, NaN range.
        let nan_val = QClip {
            tracks: vec![QTrack {
                times: vec![0.0, 1.0],
                keys: vec![[0, 0, 0], [1, 1, 1]],
                min: [f32::NAN, 0.0, 0.0],
                max: [1.0, 1.0, 1.0],
            }],
            dur: 1.0,
        };
        assert_eq!(
            pack_qclip(&nan_val),
            Err(PackError::NonFinite { lane: "range" })
        );
        let nan_knot = QClip {
            tracks: vec![QTrack {
                times: vec![0.0, f32::NAN],
                keys: vec![[0, 0, 0], [1, 1, 1]],
                min: [0.0; 3],
                max: [1.0; 3],
            }],
            dur: 1.0,
        };
        assert_eq!(
            pack_qclip(&nan_knot),
            Err(PackError::NonFinite { lane: "times" })
        );
        // Backwards knots name the track and index.
        let back = QClip {
            tracks: vec![QTrack {
                times: vec![0.0, 1.0, 0.5],
                keys: vec![[0, 0, 0], [1, 1, 1], [2, 2, 2]],
                min: [0.0; 3],
                max: [1.0; 3],
            }],
            dur: 1.0,
        };
        assert_eq!(
            pack_qclip(&back),
            Err(PackError::TimesNotOrdered { track: 0, index: 2 })
        );
        // Negative duration.
        let neg = QClip {
            tracks: Vec::new(),
            dur: -1.0,
        };
        assert_eq!(pack_qclip(&neg), Err(PackError::BadDuration(-1.0)));
        // Raw pack path validates too.
        assert_eq!(
            pack_track_at(0, &[0.0], &[[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]),
            Err(PackError::BadLength {
                track: 0,
                times: 1,
                keys: 2
            })
        );
        assert_eq!(
            pack_track_at(2, &[0.0, f32::INFINITY], &[[0.0; 3], [1.0; 3]]),
            Err(PackError::NonFinite { lane: "times" })
        );
        assert_eq!(
            pack_track_at(3, &[0.0, 1.0], &[[0.0; 3], [f32::NAN, 0.0, 0.0]]),
            Err(PackError::NonFinite { lane: "values" })
        );
        // Errors display loudly.
        assert!(PackError::BadVersion { got: 9 }.to_string().contains('9'));
        assert!(PackError::Bake("gone".to_string())
            .to_string()
            .contains("gone"));
    }

    #[test]
    fn corrupt_bytes_are_refused() {
        let packed = pack_qclip(&lin_qclip()).expect("packs");
        let mut bytes = packed.to_bytes();
        // Bad magic.
        let mut bad = bytes.clone();
        bad[0] ^= 0xFF;
        assert!(matches!(
            PackedClip::from_bytes(&bad),
            Err(PackError::BadMagic { .. })
        ));
        // Bad version.
        let mut bad = bytes.clone();
        bad[4] = 0x7F;
        bad[5] = 0x7F;
        assert_eq!(
            PackedClip::from_bytes(&bad),
            Err(PackError::BadVersion { got: 0x7F7F })
        );
        // Truncated tail.
        bytes.pop();
        assert!(matches!(
            PackedClip::from_bytes(&bytes),
            Err(PackError::Truncated { .. })
        ));
        let full = packed.to_bytes();
        // Trailing byte.
        let mut trailing = full.clone();
        trailing.push(0);
        assert_eq!(
            PackedClip::from_bytes(&trailing),
            Err(PackError::Trailing { bytes: 1 })
        );
        // Empty input names its missing field.
        assert!(matches!(
            PackedClip::from_bytes(&[]),
            Err(PackError::Truncated { .. })
        ));
        // Forged counts fail before any big allocation.
        let mut forged = full.clone();
        // Track count sits at bytes 10..14 (magic 4, version 2, dur 4).
        forged[10] = 0xFF;
        forged[11] = 0xFF;
        forged[12] = 0xFF;
        forged[13] = 0xFF;
        assert!(matches!(
            PackedClip::from_bytes(&forged),
            Err(PackError::TooManyTracks { .. })
        ));
        // Unknown track kind.
        let mut kind = full.clone();
        // First track kind byte: header 14 + range 24 = offset 38.
        kind[38] = 7;
        assert_eq!(
            PackedClip::from_bytes(&kind),
            Err(PackError::BadTrackKind(7))
        );
        // Fighter-level forgeries.
        let fight = FighterPack::from_bytes(&pack_hero_fighter().expect("hero").pack.to_bytes())
            .expect("fighter decodes");
        assert_eq!(fight.len(), 8);
        assert!(!fight.is_empty());
        let mut fbytes = fight.to_bytes();
        fbytes.push(0);
        assert_eq!(
            FighterPack::from_bytes(&fbytes),
            Err(PackError::Trailing { bytes: 1 })
        );
        let mut fbad = fight.to_bytes();
        fbad[0] ^= 0xFF;
        assert!(matches!(
            FighterPack::from_bytes(&fbad),
            Err(PackError::BadMagic { .. })
        ));
        // Oversize blobs are refused before touching the reader.
        assert_eq!(
            PackedClip::from_bytes(&vec![0u8; MAX_PACKED_BYTES + 1]),
            Err(PackError::Oversize {
                bytes: MAX_PACKED_BYTES + 1
            })
        );
        // Error-row length mismatches are refused, never misread.
        assert!(matches!(
            track_error(&fight.clips[0].tracks[0], &[]),
            Err(PackError::BadLength { .. })
        ));
    }
}

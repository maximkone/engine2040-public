//! clip_rot: a quantized rotation lane that sits beside [`QTrack`] on the same
//! clip tick grid. Positions stay in `clip.rs`; this module carries the
//! quaternion half a version-1 clip never had, so an old clip and its wire
//! format are untouched.
//!
//! Quantization: one unit quaternion is four `i16`, each component mapped from
//! `[-1, 1]` to `[-32767, 32767]` by `round(v * 32767)`, and the decoded four
//! are renormalized, because rounding a unit quaternion moves its length. The
//! slerp also renormalizes its result. `q` and `-q` are the same rotation, so
//! baking flips the sign to `w >= 0` (with the first nonzero of `x, y, z` as
//! the tie-break when `w == 0`); two sign-flipped keys therefore bake to the
//! same bytes and the same [`rot_hash`].
//!
//! Sampling mirrors `clip::sample_track`: NaN time reads the first key, times
//! before the first key and after the last clamp to that key, and `hint` is the
//! caller's cursor over the track (moved forward on a sequential read, else a
//! binary search re-seats it). The difference is the blend: quaternions take
//! the shortest arc, positions lerp.

use engine_core::{hash_bytes, Fnv64};

use crate::clip::{clamp01, QClip};
use crate::contract::rotate;
use crate::inertialize::{acos_unit, sin_unit};
use crate::pose::QuatTrack;

/// Largest stored component magnitude; also the decode divisor. The struct
/// field is `i16`, so the bound is one below `i16::MAX` to stay symmetric.
const Q_MAX_F: f32 = 32767.0;
/// Below this the decode or bake input is too small to hold a direction.
const Q_EPS: f32 = 1e-8;

/// One quantized rotation lane: strictly increasing times and one `[i16; 4]`
/// unit quaternion per time, mapped from `[-1, 1]` to `[-32767, 32767]`.
/// Empty = invalid (sample = identity).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RotTrack {
    /// Seconds, same grid as the clip's [`QTrack`](crate::clip::QTrack).
    pub times: Vec<f32>,
    /// Quantized `[x, y, z, w]`, sign-canonicalized to `w >= 0` at bake.
    pub keys: Vec<[i16; 4]>,
}

/// A weapon or prop socket: the joint it rides, its local offset, and the
/// rotation lane that turns the offset. Sampling is local; the caller applies
/// the joint's parent chain.
#[derive(Clone, Debug, PartialEq)]
pub struct SocketTrack {
    /// Joint index in the skeleton the clip was baked for.
    pub joint: usize,
    /// Local offset in meters, unrotated.
    pub offset: [f32; 3],
    /// The socket's local rotation lane.
    pub rot: RotTrack,
}

/// Normalize, or identity when the input is degenerate or non-finite.
fn normalize(q: [f32; 4]) -> [f32; 4] {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if !n.is_finite() || n < Q_EPS {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let inv = 1.0 / n;
    [q[0] * inv, q[1] * inv, q[2] * inv, q[3] * inv]
}

/// Flip to `w >= 0`; when `w == 0` the first nonzero of `x, y, z` picks the
/// sign, so `q` and `-q` land on one representative even at a 180-degree turn.
fn canonical(mut q: [f32; 4]) -> [f32; 4] {
    let flip = if q[3] > 0.0 {
        false
    } else if q[3] < 0.0 {
        true
    } else {
        q[0] < 0.0 || (q[0] == 0.0 && (q[1] < 0.0 || (q[1] == 0.0 && q[2] < 0.0)))
    };
    if flip {
        for c in &mut q {
            *c = -*c;
        }
    }
    q
}

/// Round to the nearest integer and narrow to `i16` without a numeric cast.
/// The input is already clamped to the quant range. Adding `1.5 * 2^23` makes
/// the field hold an exact integer, so the low 23 bits minus `2^22` are it.
fn round_i16(x: f32) -> i16 {
    const MAGIC: f32 = 12_582_912.0; // 1.5 * 2^23
    const LOW_MASK: u32 = 0x007F_FFFF;
    const BIAS: i32 = 4_194_304; // 2^22
    let low = i32::try_from((x + MAGIC).to_bits() & LOW_MASK).unwrap_or(BIAS);
    i16::try_from(low - BIAS).unwrap_or(0)
}

/// Quantize one unit quaternion after normalizing and canonicalizing it.
fn quantize_q(q: [f32; 4]) -> [i16; 4] {
    let q = canonical(normalize(q));
    let mut out = [0i16; 4];
    for (slot, c) in out.iter_mut().zip(q) {
        *slot = round_i16(c.clamp(-1.0, 1.0) * Q_MAX_F);
    }
    out
}

/// Decode one key back to a unit quaternion.
fn decode_q(k: [i16; 4]) -> [f32; 4] {
    normalize([
        f32::from(k[0]) / Q_MAX_F,
        f32::from(k[1]) / Q_MAX_F,
        f32::from(k[2]) / Q_MAX_F,
        f32::from(k[3]) / Q_MAX_F,
    ])
}

/// Normalized lerp: the near-parallel fallback, where slerp is its limit.
fn lerp_norm(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    normalize([
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
        a[3] + (b[3] - a[3]) * f,
    ])
}

/// Shortest-arc spherical blend; near-parallel pairs take the normalized-lerp
/// branch so the division by `sin(theta)` never underflows. The angle and its
/// sine come from the crate's integer-free polynomial pair, never the platform
/// libm, so a replay on another PC reads the same bits.
fn slerp(a: [f32; 4], mut b: [f32; 4], f: f32) -> [f32; 4] {
    let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    if dot < 0.0 {
        dot = -dot;
        for c in &mut b {
            *c = -*c;
        }
    }
    let dot = dot.clamp(-1.0, 1.0);
    if dot > 0.9995 {
        return lerp_norm(a, b, f);
    }
    let theta = acos_unit(dot);
    let sin_theta = sin_unit(theta);
    if !sin_theta.is_finite() || sin_theta.abs() < 1e-6 {
        return lerp_norm(a, b, f);
    }
    let wa = sin_unit((1.0 - f) * theta) / sin_theta;
    let wb = sin_unit(f * theta) / sin_theta;
    normalize([
        a[0] * wa + b[0] * wb,
        a[1] * wa + b[1] * wb,
        a[2] * wa + b[2] * wb,
        a[3] * wa + b[3] * wb,
    ])
}

/// Interval start and fraction, the same answer `clip::sample_track` gets: a
/// cursor that still sits behind `t` walks forward, one that sits ahead is
/// re-seated by a partition point. Clamping happens before this is called, so
/// `times[0] <= t < times[last]` holds here.
fn interval(times: &[f32], hint: usize, t: f32) -> (usize, f32) {
    let last = times.len() - 1;
    let start = hint.min(last);
    let lo = if times[start] <= t {
        let ahead = times[start..]
            .iter()
            .position(|&x| x > t)
            .unwrap_or(last - start);
        start + ahead - 1
    } else {
        times[..last].partition_point(|&x| x <= t)
    };
    let (t0, t1) = (times[lo], times[lo + 1]);
    let s = t1 - t0;
    (lo, if s <= 0.0 { 0.0 } else { clamp01((t - t0) / s) })
}

/// Bake unit quaternions into a rotation lane. Mismatched lengths or an empty
/// input give an empty track, the same convention as [`bake_track`].
///
/// [`bake_track`]: crate::clip::bake_track
pub fn bake_rot_track(times: Vec<f32>, quats: Vec<[f32; 4]>) -> RotTrack {
    if times.len() != quats.len() || times.is_empty() {
        return RotTrack {
            times: Vec::new(),
            keys: Vec::new(),
        };
    }
    let mut keys = Vec::with_capacity(quats.len());
    for q in quats {
        keys.push(quantize_q(q));
    }
    RotTrack { times, keys }
}

/// Sample the lane: NaN reads the first key, both ends clamp, neighbours slerp
/// on the shortest arc and the result is renormalized. Empty or mismatched
/// track = identity.
pub fn sample_rot_track(tr: &RotTrack, t: f32, hint: &mut usize) -> [f32; 4] {
    if tr.times.len() != tr.keys.len() || tr.times.is_empty() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let t = if t.is_nan() { tr.times[0] } else { t };
    let last = tr.times.len() - 1;
    if t <= tr.times[0] {
        *hint = 0;
        return decode_q(tr.keys[0]);
    }
    if t >= tr.times[last] {
        *hint = last;
        return decode_q(tr.keys[last]);
    }
    let (lo, f) = interval(&tr.times, *hint, t);
    *hint = lo;
    slerp(decode_q(tr.keys[lo]), decode_q(tr.keys[lo + 1]), f)
}

/// Build a socket from a joint index, its local offset and a rotation lane.
pub fn socket_track(joint: usize, offset: [f32; 3], rot: RotTrack) -> SocketTrack {
    SocketTrack { joint, offset, rot }
}

/// Sample a socket: the local offset turned by the sampled rotation, plus that
/// rotation. The joint index is carried for the caller's parent chain.
pub fn sample_socket(tr: &SocketTrack, t: f32) -> ([f32; 3], [f32; 4]) {
    let mut hint = 0usize;
    let q = sample_rot_track(&tr.rot, t, &mut hint);
    (rotate(&tr.offset, &q), q)
}

/// FNV-1a64 over the lane, same shape as [`qclip_hash`]: times as `f32` bits,
/// keys as their `i16` bits.
///
/// [`qclip_hash`]: crate::clip::qclip_hash
pub fn rot_hash(tr: &RotTrack) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(tr.times.len());
    h.write_len(tr.keys.len());
    for t in &tr.times {
        h.write_f32(*t);
    }
    for k in &tr.keys {
        for x in k {
            h.write_u16(u16::from_ne_bytes(x.to_ne_bytes()));
        }
    }
    h.finish()
}

/// Sidecar magic: four ASCII bytes at the front of every rotation blob.
pub const ROT_BLOB_MAGIC: [u8; 4] = *b"R204";
/// Sidecar format version this writer emits and this parser accepts.
pub const ROT_BLOB_VERSION: u8 = 1;

/// A joint-indexed rotation sidecar: one lane per joint, weapon sockets and the
/// clip length in seconds. It is a separate blob, never a [`QClip`] field, so
/// the version-1 clip wire and every struct literal that builds one stay as is.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RotBlob {
    /// Joint count; `lanes.len()` after a bake or a parse.
    pub joints: u16,
    /// One lane per joint, in joint order; a joint with no rotation source has
    /// an empty lane and samples as identity.
    pub lanes: Vec<RotTrack>,
    /// Weapon and prop sockets, each carrying a copy of its joint's lane.
    pub sockets: Vec<SocketTrack>,
    /// Clip length in seconds, the value the position clip calls `dur`.
    pub ticks_len: f32,
}

/// Why a rotation blob was refused. Every message names the legal value and one
/// recovery step so a caller can fix the input without a second round trip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RotBlobError {
    /// The first four bytes are not [`ROT_BLOB_MAGIC`].
    BadMagic {
        /// Bytes that were found instead.
        found: [u8; 4],
    },
    /// The version byte is not [`ROT_BLOB_VERSION`].
    BadVersion {
        /// Version that was found instead.
        found: u8,
    },
    /// The blob ended before the header or a lane said it would.
    Truncated {
        /// Byte offset the read needed to reach.
        need: usize,
        /// Bytes actually present.
        got: usize,
    },
    /// A socket names a joint the header does not have.
    SocketJoint {
        /// Joint the socket named.
        joint: u16,
        /// Joint count the header declares.
        joints: u16,
    },
}

impl std::fmt::Display for RotBlobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic { found } => write!(
                f,
                "bad magic {found:?}; the legal magic is R204; recovery: re-export the blob with write_rot_blob"
            ),
            Self::BadVersion { found } => write!(
                f,
                "version {found} is not supported; the legal version is {ROT_BLOB_VERSION}; recovery: re-export the blob with the current writer"
            ),
            Self::Truncated { need, got } => write!(
                f,
                "blob is truncated at {got} of {need} bytes; recovery: re-read the whole blob, never a partial file"
            ),
            Self::SocketJoint { joint, joints } => write!(
                f,
                "socket joint {joint} is outside 0..{joints}; recovery: bake again with bake_rot_blob, which keeps sockets inside the joint count"
            ),
        }
    }
}

impl std::error::Error for RotBlobError {}

/// Cursor over blob bytes; every read is bounds checked and names the byte it
/// needed, so a truncated file always reports the same way.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], RotBlobError> {
        let end = self.at.checked_add(n).ok_or(RotBlobError::Truncated {
            need: usize::MAX,
            got: self.bytes.len(),
        })?;
        if end > self.bytes.len() {
            return Err(RotBlobError::Truncated {
                need: end,
                got: self.bytes.len(),
            });
        }
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N], RotBlobError> {
        let slice = self.take(N)?;
        <[u8; N]>::try_from(slice).map_err(|_| RotBlobError::Truncated {
            need: N,
            got: slice.len(),
        })
    }

    fn u8(&mut self) -> Result<u8, RotBlobError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, RotBlobError> {
        Ok(u16::from_le_bytes(self.arr::<2>()?))
    }

    fn u32(&mut self) -> Result<u32, RotBlobError> {
        Ok(u32::from_le_bytes(self.arr::<4>()?))
    }

    fn f32(&mut self) -> Result<f32, RotBlobError> {
        Ok(f32::from_le_bytes(self.arr::<4>()?))
    }

    fn lane(&mut self) -> Result<RotTrack, RotBlobError> {
        let count = usize::try_from(self.u32()?).unwrap_or(usize::MAX);
        let mut times = Vec::new();
        for _ in 0..count {
            times.push(self.f32()?);
        }
        let mut keys = Vec::with_capacity(times.len());
        for _ in 0..times.len() {
            let mut key = [0i16; 4];
            for slot in &mut key {
                let lo = self.u8()?;
                let hi = self.u8()?;
                *slot = i16::from_le_bytes([lo, hi]);
            }
            keys.push(key);
        }
        Ok(RotTrack { times, keys })
    }
}

fn write_lane(out: &mut Vec<u8>, lane: &RotTrack) {
    let count = u32::try_from(lane.times.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&count.to_le_bytes());
    for t in &lane.times {
        out.extend_from_slice(&t.to_le_bytes());
    }
    for key in &lane.keys {
        for c in key {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
}

/// Serialize a rotation blob: magic, version, joint count, clip length, one
/// lane per joint, then the sockets. Little-endian; the joint count is taken
/// from the lanes so the header can never disagree with the body.
pub fn write_rot_blob(blob: &RotBlob) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&ROT_BLOB_MAGIC);
    out.push(ROT_BLOB_VERSION);
    out.extend_from_slice(
        &u16::try_from(blob.lanes.len())
            .unwrap_or(u16::MAX)
            .to_le_bytes(),
    );
    out.extend_from_slice(&blob.ticks_len.to_le_bytes());
    for lane in &blob.lanes {
        write_lane(&mut out, lane);
    }
    out.extend_from_slice(
        &u16::try_from(blob.sockets.len())
            .unwrap_or(u16::MAX)
            .to_le_bytes(),
    );
    for socket in &blob.sockets {
        let joint = u16::try_from(socket.joint).unwrap_or(u16::MAX);
        out.extend_from_slice(&joint.to_le_bytes());
        for c in socket.offset {
            out.extend_from_slice(&c.to_le_bytes());
        }
        write_lane(&mut out, &socket.rot);
    }
    out
}

/// Read a rotation blob written by [`write_rot_blob`]. A bad magic, a version
/// other than 1, a short read or a socket past the joint count is a typed
/// [`RotBlobError`]; nothing is guessed.
pub fn parse_rot_blob(bytes: &[u8]) -> Result<RotBlob, RotBlobError> {
    let mut r = Reader::new(bytes);
    let magic = r.arr::<4>()?;
    if magic != ROT_BLOB_MAGIC {
        return Err(RotBlobError::BadMagic { found: magic });
    }
    let version = r.u8()?;
    if version != ROT_BLOB_VERSION {
        return Err(RotBlobError::BadVersion { found: version });
    }
    let joints = r.u16()?;
    let ticks_len = r.f32()?;
    let mut lanes = Vec::with_capacity(usize::from(joints));
    for _ in 0..joints {
        lanes.push(r.lane()?);
    }
    let socket_count = r.u16()?;
    let mut sockets = Vec::with_capacity(usize::from(socket_count));
    for _ in 0..socket_count {
        let joint = r.u16()?;
        if joint >= joints {
            return Err(RotBlobError::SocketJoint { joint, joints });
        }
        let offset = [r.f32()?, r.f32()?, r.f32()?];
        let rot = r.lane()?;
        sockets.push(SocketTrack {
            joint: usize::from(joint),
            offset,
            rot,
        });
    }
    Ok(RotBlob {
        joints,
        lanes,
        sockets,
        ticks_len,
    })
}

/// Digest of the serialized blob, so two bakes can be compared by bytes.
pub fn rot_blob_hash(blob: &RotBlob) -> u64 {
    hash_bytes(&write_rot_blob(blob))
}

/// Bake the rotation sidecar for a position clip. Each joint gets the clip
/// track's own times; a joint with no (or a mismatched) rotation track gets an
/// empty lane, which samples as identity. Socket lanes copy their joint's lane.
pub fn bake_rot_blob(
    clip: &QClip,
    rotations: &[QuatTrack],
    sockets: &[(usize, [f32; 3])],
) -> RotBlob {
    let mut lanes = Vec::with_capacity(clip.tracks.len());
    for (joint, track) in clip.tracks.iter().enumerate() {
        let lane = match rotations.get(joint) {
            Some(rot) if rot.times.len() == track.times.len() => {
                bake_rot_track(track.times.clone(), rot.values.clone())
            }
            _ => RotTrack::default(),
        };
        lanes.push(lane);
    }
    let sockets = sockets
        .iter()
        .map(|&(joint, offset)| {
            let lane = lanes.get(joint).cloned().unwrap_or_default();
            socket_track(joint, offset, lane)
        })
        .collect();
    RotBlob {
        joints: u16::try_from(lanes.len()).unwrap_or(u16::MAX),
        lanes,
        sockets,
        ticks_len: clip.dur,
    }
}

/// Sample one joint of a blob: a socket joint returns its turned offset and
/// rotation, a plain joint returns a zero offset. Clamp, NaN and cursor
/// semantics are the position sampler's. `None` for a joint past the lanes.
pub fn sample_rot_blob(
    blob: &RotBlob,
    t: f32,
    joint: usize,
    hint: &mut usize,
) -> Option<([f32; 3], [f32; 4])> {
    if let Some(socket) = blob.sockets.iter().find(|s| s.joint == joint) {
        return Some(sample_socket(socket, t));
    }
    let lane = blob.lanes.get(joint)?;
    Some(([0.0; 3], sample_rot_track(lane, t, hint)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::quat_mul;

    use crate::clip::{bake_clip, bake_track};

    fn turn_180() -> RotTrack {
        bake_rot_track(
            vec![0.0, 1.0],
            vec![[0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.0]],
        )
    }

    #[test]
    fn midpoint_of_180_is_90() {
        let tr = turn_180();
        let mut hint = 0usize;
        let q = sample_rot_track(&tr, 0.5, &mut hint);
        let s = 0.5f32.sqrt();
        let want = [0.0, s, 0.0, s];
        for i in 0..4 {
            assert!((q[i] - want[i]).abs() < 2e-3, "comp {i}: {q:?}");
        }
    }

    #[test]
    fn sign_flipped_bakes_share_a_hash() {
        let a = bake_rot_track(vec![0.0], vec![[0.1, 0.2, 0.3, 0.9]]);
        let b = bake_rot_track(vec![0.0], vec![[-0.1, -0.2, -0.3, -0.9]]);
        assert_eq!(rot_hash(&a), rot_hash(&b));
        // w == 0 has no sign to prefer, so the first nonzero component decides.
        let c = bake_rot_track(vec![0.0], vec![[0.0, 1.0, 0.0, 0.0]]);
        let d = bake_rot_track(vec![0.0], vec![[0.0, -1.0, 0.0, 0.0]]);
        assert_eq!(rot_hash(&c), rot_hash(&d));
    }

    #[test]
    fn clamps_at_both_ends() {
        let tr = turn_180();
        let mut hint = 1usize;
        let first = sample_rot_track(&tr, -10.0, &mut hint);
        assert_eq!(hint, 0);
        assert_eq!(first, decode_q(tr.keys[0]));
        let last = sample_rot_track(&tr, 10.0, &mut hint);
        assert_eq!(hint, 1);
        assert_eq!(last, decode_q(tr.keys[1]));
    }

    #[test]
    fn nan_time_reads_the_first_key() {
        let tr = turn_180();
        let (mut ha, mut hb) = (0usize, 1usize);
        assert_eq!(
            sample_rot_track(&tr, f32::NAN, &mut ha),
            sample_rot_track(&tr, 0.0, &mut hb)
        );
        assert_eq!(ha, 0);
    }

    #[test]
    fn empty_and_mismatched_bake_empty() {
        assert!(bake_rot_track(Vec::new(), Vec::new()).times.is_empty());
        assert!(bake_rot_track(vec![0.0], Vec::new()).keys.is_empty());
        let mut hint = 0usize;
        assert_eq!(
            sample_rot_track(&RotTrack::default(), 0.5, &mut hint),
            [0.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn three_joint_chain_socket_lands_on_hand_math() {
        // Joint 2 of a 3-joint chain, offset one metre along local +X.
        let sock = socket_track(2, [1.0, 0.0, 0.0], turn_180());
        let (local, q) = sample_socket(&sock, 0.5);
        // A 90-degree Y turn maps +X to -Z.
        for (i, want) in [0.0, 0.0, -1.0].iter().enumerate() {
            assert!((local[i] - want).abs() < 2e-3, "comp {i}: {local:?}");
        }
        // The chain's parent is one more 90-degree Y turn, so the world pose is
        // a half turn about Y and the hand sits at world -X.
        let parent = [0.0, 0.5f32.sqrt(), 0.0, 0.5f32.sqrt()];
        let world = rotate(&local, &parent);
        for (i, want) in [-1.0, 0.0, 0.0].iter().enumerate() {
            assert!((world[i] - want).abs() < 2e-3, "comp {i}: {world:?}");
        }
        let composed = quat_mul(parent, q);
        assert!(
            composed[3].abs() < 2e-3,
            "half turn has w = 0: {composed:?}"
        );
        assert!((composed[1].abs() - 1.0).abs() < 2e-3);
    }

    #[test]
    fn two_bakes_hash_equal() {
        let times = vec![0.0, 0.5, 1.0];
        let quats = vec![
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.382_683_43, 0.0, 0.923_879_5],
            [0.0, 1.0, 0.0, 0.0],
        ];
        let a = bake_rot_track(times.clone(), quats.clone());
        let b = bake_rot_track(times, quats);
        assert_eq!(rot_hash(&a), rot_hash(&b));
        let mut c = a.clone();
        c.times[1] = 0.25;
        assert_ne!(rot_hash(&a), rot_hash(&c));
    }

    fn small_blob() -> RotBlob {
        let lanes = vec![turn_180(), turn_180(), turn_180()];
        RotBlob {
            joints: 3,
            sockets: vec![socket_track(2, [1.0, 0.0, 0.0], lanes[2].clone())],
            lanes,
            ticks_len: 1.0,
        }
    }

    #[test]
    fn blob_round_trips_with_a_socket() {
        let blob = small_blob();
        let bytes = write_rot_blob(&blob);
        let back = parse_rot_blob(&bytes).expect("parses");
        assert_eq!(back, blob);
        assert_eq!(rot_blob_hash(&back), rot_blob_hash(&blob));
    }

    #[test]
    fn blob_errors_name_the_legal_value_and_a_recovery() {
        let mut bytes = write_rot_blob(&small_blob());

        let mut bad_magic = bytes.clone();
        bad_magic[0] = b'X';
        let error = parse_rot_blob(&bad_magic).expect_err("magic");
        assert!(matches!(error, RotBlobError::BadMagic { .. }));
        let text = error.to_string();
        assert!(text.contains("R204") && text.contains("recovery"), "{text}");

        let mut version_two = bytes.clone();
        version_two[4] = 2;
        let error = parse_rot_blob(&version_two).expect_err("version");
        assert!(matches!(error, RotBlobError::BadVersion { found: 2 }));
        let text = error.to_string();
        assert!(
            text.contains("version is 1") && text.contains("recovery"),
            "{text}"
        );

        bytes.pop();
        let error = parse_rot_blob(&bytes).expect_err("truncated");
        assert!(matches!(error, RotBlobError::Truncated { .. }));
        let text = error.to_string();
        assert!(text.contains("recovery"), "{text}");
    }

    #[test]
    fn bake_makes_one_lane_per_joint() {
        let times = vec![0.0, 1.0];
        let clip = bake_clip(vec![
            bake_track(times.clone(), vec![[0.0; 3], [1.0, 0.0, 0.0]]),
            bake_track(times.clone(), vec![[0.0; 3], [0.0, 1.0, 0.0]]),
        ]);
        let rotations = vec![
            QuatTrack {
                times: times.clone(),
                values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.0]],
            },
            QuatTrack {
                times: times.clone(),
                values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 0.0]],
            },
        ];
        let a = bake_rot_blob(&clip, &rotations, &[(1, [0.5, 0.0, 0.0])]);
        assert_eq!(a.lanes.len(), 2);
        assert_eq!(a.joints, 2);
        assert_eq!(a.sockets.len(), 1);
        assert_eq!(a.ticks_len, clip.dur);
        let b = bake_rot_blob(&clip, &rotations, &[(1, [0.5, 0.0, 0.0])]);
        assert_eq!(rot_blob_hash(&a), rot_blob_hash(&b));

        let mut three = clip.clone();
        three
            .tracks
            .push(bake_track(times, vec![[0.0; 3], [0.0, 0.0, 1.0]]));
        let c = bake_rot_blob(&three, &rotations, &[]);
        assert_eq!(c.lanes.len(), 3);
        assert!(
            c.lanes[2].times.is_empty(),
            "no rotation source, empty lane"
        );
    }

    #[test]
    fn blob_sampling_clamps_and_rejects_a_bad_joint() {
        let blob = small_blob();
        let (mut h0, mut h1, mut h2) = (0usize, 0usize, 0usize);
        let first = sample_rot_blob(&blob, -5.0, 0, &mut h0).expect("joint 0");
        let at_zero = sample_rot_blob(&blob, 0.0, 0, &mut h1).expect("joint 0");
        assert_eq!(first, at_zero);
        let after = sample_rot_blob(&blob, 9.0, 0, &mut h2).expect("joint 0");
        assert_eq!(after.1, decode_q(blob.lanes[0].keys[1]));

        let (offset, _) = sample_rot_blob(&blob, 0.5, 2, &mut 0).expect("socket joint");
        for (i, want) in [0.0, 0.0, -1.0].iter().enumerate() {
            assert!((offset[i] - want).abs() < 2e-3, "comp {i}: {offset:?}");
        }
        assert_eq!(sample_rot_blob(&blob, 0.5, 9, &mut 0), None);

        let nan = sample_rot_blob(&blob, f32::NAN, 0, &mut 0).expect("joint 0");
        assert_eq!(nan, at_zero);
    }
}

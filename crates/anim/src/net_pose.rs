//! net_pose: networked motion replication over pick index plus phase.
//!
//! Consumer: the multiplayer app renders remote fighters from replicated
//! [`NetPose`] values and re-runs them on rollback; the arena duel reads
//! [`checkpoints_for_stream`] to compare pose checkpoints across clients.
//! Data owner: `engine-anim`; this module never imports assets, arena or
//! net transport. Units: ticks at 60 Hz, gait phase in `[0, 1)`, wire bytes
//! per [`WIRE_LEN`].
//!
//! Determinism: the replicated pick is the motion-match `(clip, tick)` frame
//! plus a quantized gait phase; poses are re-derived from those three lanes,
//! never sent. Encode, decode and checkpoint use integer math only, so the
//! same stream gives the same checkpoints on every machine. Short or long
//! byte slices reject with [`NetPoseError`]; any exact [`WIRE_LEN`] slice
//! decodes, so the wire path is total.
//!
//! Bandwidth chart (60 Hz, one entry per fighter per tick):
//!
//! | payload | bytes/tick | kbps/fighter |
//! |---|---|---|
//! | pick `(clip, tick)` + phase | 6 | 2.88 |
//!
//! A derived 21-joint pose would cost `21 * 3 * 4 = 252` bytes (`120.96`
//! kbps); replication saves `42x` by sending the pick instead of the pose.

use engine_core::Fnv64;

/// Wire length of one encoded [`NetPose`]: `clip` + `tick` + `phase_q`.
pub const WIRE_LEN: usize = 6;
/// Fixed simulation rate the replication stream ticks at.
/// Single owner of the rate is `engine_core::TICK_HZ`.
pub const TICKS_PER_SEC: u32 = engine_core::TICK_HZ as u32;
/// Ticks of 150 ms round-trip at 60 Hz: the resim window the tests prove.
pub const RTT_TICKS_150MS: u32 = 9;
/// Phase quantizer: full `[0, 1)` turn maps to `u16` steps.
pub const PHASE_SCALE: f32 = 65535.0;

/// One replicated motion pick: the motion-match `(clip, tick)` frame plus
/// the quantized gait phase. The remote peer re-derives the pose from these
/// three lanes; nothing else travels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NetPose {
    /// Source clip id from the motion-match pick.
    pub clip: u16,
    /// Tick inside the clip from the motion-match pick.
    pub tick: u16,
    /// Gait phase in `[0, 1)` quantized to `u16` steps.
    pub phase_q: u16,
}

/// Bounded replication failures: bad wire length or a non-finite phase.
/// Never panics, never blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NetPoseError {
    /// Wire slice is not [`WIRE_LEN`] bytes; expected + got carried.
    BadLen {
        /// Required length ([`WIRE_LEN`]).
        expected: usize,
        /// Slice length seen.
        got: usize,
    },
    /// Phase is non-finite and cannot quantize.
    BadPhase,
}

impl core::fmt::Display for NetPoseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NetPoseError::BadLen { expected, got } => {
                write!(f, "net pose: wire length {got} differs from {expected}")
            }
            NetPoseError::BadPhase => write!(f, "net pose: non-finite phase"),
        }
    }
}

impl std::error::Error for NetPoseError {}

/// Quantize a gait phase in `[0, 1)` to `u16` steps. Finite out-of-range
/// phases clamp to the edge; non-finite phases reject.
pub fn quantize_phase(phase: f32) -> Result<u16, NetPoseError> {
    if !phase.is_finite() {
        return Err(NetPoseError::BadPhase);
    }
    // Clamped finite phases land in `u16` range; the crate's saturating
    // float-to-int lane keeps this cast-free (bit-exact with `as` here).
    let q = (phase.clamp(0.0, 1.0) * PHASE_SCALE).round();
    Ok(u16::try_from(crate::springs::f32_to_i32_sat(q)).unwrap_or(u16::MAX))
}

/// Dequantize a phase lane back to `[0, 1]`. Exact inverse of the edge
/// values (`0` maps to `0.0`, `u16::MAX` maps to `1.0`).
pub fn dequantize_phase(phase_q: u16) -> f32 {
    f32::from(phase_q) / PHASE_SCALE
}

impl NetPose {
    /// Build a replicated pick from a motion-match `(clip, tick)` frame plus
    /// the live gait phase. Rejects non-finite phases.
    pub fn from_pick(clip: u16, tick: u16, phase: f32) -> Result<Self, NetPoseError> {
        Ok(Self {
            clip,
            tick,
            phase_q: quantize_phase(phase)?,
        })
    }

    /// Gait phase in `[0, 1]` decoded from the quantized lane.
    pub fn phase(&self) -> f32 {
        dequantize_phase(self.phase_q)
    }

    /// Encode to [`WIRE_LEN`] little-endian bytes. Stack-only, total.
    pub fn encode(self) -> [u8; WIRE_LEN] {
        let mut out = [0u8; WIRE_LEN];
        out[0..2].copy_from_slice(&self.clip.to_le_bytes());
        out[2..4].copy_from_slice(&self.tick.to_le_bytes());
        out[4..6].copy_from_slice(&self.phase_q.to_le_bytes());
        out
    }

    /// Decode from a wire slice. Any exact-length slice decodes; short or
    /// long slices reject with [`NetPoseError::BadLen`].
    pub fn decode(bytes: &[u8]) -> Result<Self, NetPoseError> {
        if bytes.len() != WIRE_LEN {
            return Err(NetPoseError::BadLen {
                expected: WIRE_LEN,
                got: bytes.len(),
            });
        }
        let mut pair = [0u8; 2];
        pair.copy_from_slice(&bytes[0..2]);
        let clip = u16::from_le_bytes(pair);
        pair.copy_from_slice(&bytes[2..4]);
        let tick = u16::from_le_bytes(pair);
        pair.copy_from_slice(&bytes[4..6]);
        let phase_q = u16::from_le_bytes(pair);
        Ok(Self {
            clip,
            tick,
            phase_q,
        })
    }

    /// Pose checkpoint for one fixed tick: FNV-1a64 over the tick stamp plus
    /// the three replicated lanes. Two clients that resim the same stream
    /// record the same checkpoints bit-for-bit.
    pub fn checkpoint(&self, tick: u32) -> u64 {
        let mut h = Fnv64::new();
        h.write_u64(u64::from(tick));
        h.write_u16(self.clip);
        h.write_u16(self.tick);
        h.write_u16(self.phase_q);
        h.finish()
    }
}

/// Checkpoints for a whole stream: entry `k` covers
/// `start_tick + k as u32`. Same stream twice gives the same checkpoints.
pub fn checkpoints_for_stream(stream: &[NetPose], start_tick: u32) -> Vec<(u32, u64)> {
    stream
        .iter()
        .enumerate()
        .map(|(k, pose)| {
            // Saturating lane: streams never approach `u32::MAX` ticks.
            let tick = start_tick.wrapping_add(u32::try_from(k).unwrap_or(u32::MAX));
            (tick, pose.checkpoint(tick))
        })
        .collect()
}

/// Rollback resim replay: the client posed `seen` (delayed, dropped packets)
/// until the authoritative log arrived. Ticks before `first_uncertain` are
/// settled and kept from `seen`; the uncertain tail replays `authority`, so
/// the returned stream checkpoints exactly like the authority from
/// `first_uncertain` on. Over-long splits clamp to the shorter log; an empty
/// authority replays nothing.
pub fn resimulate(seen: &[NetPose], authority: &[NetPose], first_uncertain: usize) -> Vec<NetPose> {
    let split = first_uncertain.min(seen.len()).min(authority.len());
    let mut out = Vec::with_capacity(authority.len());
    out.extend_from_slice(&seen[..split]);
    out.extend_from_slice(&authority[split..]);
    out
}

/// Kilobits per second for one fighter at the fixed tick rate: 6 bytes per
/// tick at 60 Hz is 2.88 kbps.
pub fn kbps_per_fighter() -> f64 {
    wire_kbps(WIRE_LEN, TICKS_PER_SEC)
}

/// Kilobits per second for any fixed-size per-tick payload. Non-positive
/// rates read `0.0`.
pub fn wire_kbps(bytes_per_tick: usize, ticks_per_sec: u32) -> f64 {
    if ticks_per_sec == 0 {
        return 0.0;
    }
    // Saturating width: exact for every real payload, capped past `u32::MAX`.
    let bytes = f64::from(u32::try_from(bytes_per_tick).unwrap_or(u32::MAX));
    bytes * 8.0 * f64::from(ticks_per_sec) / 1000.0
}

#[cfg(test)]
#[path = "net_pose_w1_test.rs"]
mod net_pose_w1_test;

#[cfg(test)]
mod tests {
    use super::super::locomotion::Locomotion;
    use super::super::motion_match::{
        bake, build_query, cost_units, default_weights, make_feature, predict_linear, search,
        Entry, PoseSample, Trajectory, NO_HIST,
    };
    use super::*;

    fn pose_at(speed: f32, contact: u8) -> PoseSample {
        PoseSample {
            hip_vel: [speed, 0.0],
            foot_l: [0.11, 0.05, 0.0],
            foot_r: [-0.11, 0.05, 0.0],
            contact,
        }
    }

    fn traj_at(speed: f32) -> Trajectory {
        predict_linear([0.0, 0.0], [speed, 0.0]).expect("fixture velocity finite")
    }

    /// First index where the degraded stream diverges, if any.
    fn first_divergence_index(seen: &[NetPose], authority: &[NetPose]) -> Option<usize> {
        seen.iter().zip(authority.iter()).position(|(a, b)| a != b)
    }

    /// Authority tape: drive a real locomotion machine, pick each tick with
    /// the motion-match search, replicate `(clip, tick)` plus the live gait phase.
    fn authority_stream(ticks: u32) -> Vec<NetPose> {
        let db = bake(vec![
            Entry {
                feat: make_feature(&pose_at(0.0, 3), &traj_at(0.0)).expect("fixture valid"),
                clip: 0,
                tick: 0,
                contact: 3,
            },
            Entry {
                feat: make_feature(&pose_at(1.5, 3), &traj_at(1.5)).expect("fixture valid"),
                clip: 0,
                tick: 1,
                contact: 3,
            },
            Entry {
                feat: make_feature(&pose_at(3.0, 1), &traj_at(3.0)).expect("fixture valid"),
                clip: 1,
                tick: 0,
                contact: 1,
            },
            Entry {
                feat: make_feature(&pose_at(5.0, 1), &traj_at(5.0)).expect("fixture valid"),
                clip: 1,
                tick: 1,
                contact: 1,
            },
        ])
        .expect("fixture db bakes");
        let mut machine = Locomotion::new();
        let mut stream = Vec::with_capacity(ticks as usize);
        for t in 0..ticks {
            let speed = 1.5 + (t as f32 % 8.0) * 0.25;
            machine
                .update(speed, [0.0, 1.0], t, None)
                .expect("walk wish valid");
            let query = build_query(
                &pose_at(speed, 3),
                &traj_at(speed),
                NO_HIST,
                &default_weights(),
                cost_units(0.01),
                cost_units(0.04),
            )
            .expect("fixture query valid");
            let (picked, _) = search(&db, &query).expect("search hits");
            stream.push(
                NetPose::from_pick(picked.clip, picked.tick, machine.phase())
                    .expect("live phase finite"),
            );
        }
        stream
    }

    #[test]
    fn two_clients_match_after_resim_at_150ms_rtt() {
        let auth = authority_stream(40);
        // The wire round-trips exactly: what the authority sends is what a
        // remote decodes, lane for lane.
        for pose in &auth {
            assert_eq!(
                NetPose::decode(&pose.encode()).expect("exact length"),
                *pose
            );
        }
        let start = 1_000u32;
        let auth_points = checkpoints_for_stream(&auth, start);
        // Same stream twice checkpoints identically (replay x2).
        assert_eq!(checkpoints_for_stream(&auth, start), auth_points);

        // Client A sees the stream 150 ms late (9 ticks of pure delay).
        let mut late_a = vec![auth[0]; RTT_TICKS_150MS as usize];
        late_a.extend_from_slice(&auth[..auth.len() - RTT_TICKS_150MS as usize]);
        // Client B jitters worse: same delay plus a dropped packet every 17
        // ticks, holding the last good pose through the gap.
        let mut late_b = Vec::with_capacity(auth.len());
        let mut last = auth[0];
        for (k, pose) in auth.iter().enumerate() {
            if k >= RTT_TICKS_150MS as usize && k % 17 != 0 {
                last = auth[k - RTT_TICKS_150MS as usize];
            }
            let _ = pose;
            late_b.push(last);
        }
        // Before resim the degraded clients visibly diverge from authority.
        assert_ne!(
            checkpoints_for_stream(&late_a, start),
            auth_points,
            "pure delay must shift checkpoints before resim"
        );
        assert_ne!(
            checkpoints_for_stream(&late_b, start),
            auth_points,
            "drops must diverge checkpoints before resim"
        );
        // Rollback resim replays the authoritative log from the first
        // uncertain tick: both clients land back on the authority points.
        // The split is the first index where each degraded stream diverges,
        // so the settled prefix is genuinely kept and the tail genuinely
        // replayed; a vacuous replay of the authority log could not pass the
        // `fixed != late` repair asserts below.
        let split_a =
            first_divergence_index(&late_a, &auth).expect("pure delay must diverge somewhere");
        let split_b = first_divergence_index(&late_b, &auth).expect("drops must diverge somewhere");
        let fixed_a = resimulate(&late_a, &auth, split_a);
        let fixed_b = resimulate(&late_b, &auth, split_b);
        assert_ne!(fixed_a, late_a, "resim repairs the delayed stream");
        assert_ne!(fixed_b, late_b, "resim repairs the lossy stream");
        let resim_a = checkpoints_for_stream(&fixed_a, start);
        let resim_b = checkpoints_for_stream(&fixed_b, start);
        assert_eq!(resim_a, auth_points);
        assert_eq!(resim_b, auth_points);
        assert_eq!(resim_a, resim_b);
        // Replay twice: the same fix, bit-for-bit.
        assert_eq!(
            checkpoints_for_stream(&resimulate(&late_a, &auth, split_a), start),
            auth_points
        );

        // Bandwidth is charted: 6 bytes per tick at 60 Hz is 2.88 kbps.
        assert!(
            (kbps_per_fighter() - 2.88).abs() < 1e-9,
            "{}",
            kbps_per_fighter()
        );
        assert!((wire_kbps(WIRE_LEN, TICKS_PER_SEC) - 2.88).abs() < 1e-9);
        // A full derived pose costs 42x more, which is why the pick travels.
        let pose_bytes = 21usize * 3 * 4;
        assert!((wire_kbps(pose_bytes, TICKS_PER_SEC) / kbps_per_fighter() - 42.0).abs() < 1e-9);
    }

    #[test]
    fn wire_rejects_bad_lengths_and_bad_phases() {
        let pose = NetPose::from_pick(3, 7, 0.25).expect("phase valid");
        assert_eq!(pose.phase(), dequantize_phase(pose.phase_q));
        // Edges are exact; finite overshoot clamps instead of wrapping.
        assert_eq!(quantize_phase(0.0).expect("zero valid"), 0);
        assert_eq!(quantize_phase(1.0).expect("one valid"), u16::MAX);
        assert_eq!(quantize_phase(9.9).expect("overshoot clamps"), u16::MAX);
        assert_eq!(quantize_phase(-2.0).expect("undershoot clamps"), 0);
        assert_eq!(dequantize_phase(0), 0.0);
        assert_eq!(dequantize_phase(u16::MAX), 1.0);
        // Short, long and empty slices reject by length, nothing is skipped.
        for len in [0, 1, 5, 7, 12] {
            assert_eq!(
                NetPose::decode(&vec![0u8; len]),
                Err(NetPoseError::BadLen {
                    expected: WIRE_LEN,
                    got: len
                }),
                "len {len}"
            );
        }
        // Non-finite phases never enter the wire.
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(quantize_phase(bad), Err(NetPoseError::BadPhase));
            assert_eq!(NetPose::from_pick(0, 0, bad), Err(NetPoseError::BadPhase));
        }
        // Any exact-length slice still decodes: the wire path stays total.
        let raw = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        let decoded = NetPose::decode(&raw).expect("exact length decodes");
        assert_eq!(decoded.encode(), raw);
        // One flipped wire bit flips the checkpoint, never silently.
        let mut flipped = raw;
        flipped[0] ^= 1;
        let other = NetPose::decode(&flipped).expect("exact length decodes");
        assert_ne!(other, decoded);
        assert_ne!(other.checkpoint(42), decoded.checkpoint(42));
        // Same pose at two ticks checkpoints differently (no aliasing).
        assert_ne!(pose.checkpoint(1), pose.checkpoint(2));
        // Zero tick rate costs nothing and never divides.
        assert_eq!(wire_kbps(WIRE_LEN, 0), 0.0);
    }
}

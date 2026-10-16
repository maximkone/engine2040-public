//! cine: genre-neutral cinematic sequence player (intro/outro takes).
//! Caller: crates/game/src/flow.rs (shell aliases `HANDBACK_TICKS`).
//! Consumer: the match shell plays a take over Title/Result, then glides
//! home to the live camera (`flow::CINE_HANDBACK_TICKS`). Owner: timeline,
//! rig/lens evaluation, playback cursor. Units: ticks at 60 Hz, meters,
//! radians, degrees, mm. Determinism: same sequence, live poses and seed
//! give byte-identical poses twice; presentation only, never hashed.
//! Errors: `CineError` at every boundary. Budget: under the auto-cut rail
//! per presented tick, no heap growth per step after `begin`.
//!
//! PORT(bevy_director@0.4.0 sequence/Rig/Lens/Handback shapes):
//! reimplemented, zero donor bytes vendored.
use engine_core::splitmix64;
/// Handback glide home in ticks (1.5 s at 60 Hz); the shell aliases it.
pub const HANDBACK_TICKS: u32 = 90;
/// Shot index served while gliding home.
pub const SHOT_HANDBACK: u8 = 2;
/// Shot index served once the glide completes.
pub const SHOT_DONE: u8 = 3;
/// Cinematic failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CineError {
    /// Row outside its range, row name carried.
    BadConfig(&'static str),
    /// Rate not finite or not above zero.
    BadRate,
}
impl core::fmt::Display for CineError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadConfig(row) => write!(f, "bad cinematic config ({row})"),
            Self::BadRate => write!(f, "bad cinematic rate"),
        }
    }
}
impl std::error::Error for CineError {}
/// Presentation pose: position meters, yaw/pitch radians.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pose {
    /// Position meters `[x, y, z]`, `+y` up.
    pub pos: [f32; 3],
    /// Yaw radians about `+y`.
    pub yaw: f32,
    /// Pitch radians, positive up.
    pub pitch: f32,
}
/// Field of view: horizontal degrees, or focal mm on the filmback.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FovSpec {
    /// Horizontal degrees, `1.0..=120.0`.
    FovDeg(f32),
    /// Focal mm, `1.0..=500.0`.
    FocalMm(f32),
}
/// Film gate in mm; the default gate is Super35.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Filmback {
    /// Gate width mm, above zero.
    pub w_mm: f32,
    /// Gate height mm, above zero.
    pub h_mm: f32,
}
/// Super35 gate (`24.89 x 18.66` mm).
pub const SUPER35: Filmback = Filmback {
    w_mm: 24.89,
    h_mm: 18.66,
};
/// Lens datum: framing plus focus/aperture/exposure tracks (data only).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Lens {
    /// Framing against `filmback`.
    pub fov: FovSpec,
    /// Gate the focal length resolves against.
    pub filmback: Filmback,
    /// Focus meters, above zero.
    pub focus_m: f32,
    /// Aperture f-stop, above zero.
    pub aperture: f32,
    /// Exposure bias stops, finite.
    pub exposure: f32,
}
impl Lens {
    /// Horizontal field of view in degrees (data for the camera-fx hunk).
    #[must_use]
    pub fn fov_deg(self) -> f32 {
        match self.fov {
            FovSpec::FovDeg(deg) => deg,
            FovSpec::FocalMm(mm) => {
                let half = self.filmback.w_mm / (mm + mm);
                half.atan().to_degrees() * 2.0
            }
        }
    }
    /// True when every row is in range and finite.
    #[must_use]
    pub fn valid(self) -> bool {
        let fov_ok = match self.fov {
            FovSpec::FovDeg(d) => d.is_finite() && (1.0..=120.0).contains(&d),
            FovSpec::FocalMm(m) => m.is_finite() && (1.0..=500.0).contains(&m),
        };
        fov_ok
            && self.filmback.w_mm.is_finite()
            && self.filmback.w_mm > 0.0
            && self.filmback.h_mm.is_finite()
            && self.filmback.h_mm > 0.0
            && self.focus_m.is_finite()
            && self.focus_m > 0.0
            && self.aperture.is_finite()
            && self.aperture > 0.0
            && self.exposure.is_finite()
    }
}
/// Camera rail: eased keys, constant-speed dolly, orbit plus damped look-at.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rig {
    /// Eased dolly from pose to pose (SmoothStep progress).
    Keys(Pose, Pose),
    /// Constant-speed dolly from pose to pose (linear progress).
    Rail(Pose, Pose),
    /// Constant-speed orbit with damped look-at.
    Orbit {
        /// Ring center `[x, y, z]` meters.
        center: [f32; 3],
        /// Ring radius meters, above zero.
        radius: f32,
        /// Camera height meters above the center.
        height: f32,
        /// Start angle radians.
        a0: f32,
        /// End angle radians.
        a1: f32,
        /// Look target `[x, y, z]` meters.
        look: [f32; 3],
        /// Look damping share, `0.0..=1.0`.
        damp: f32,
    },
}
/// One take: absolute start on the timeline plus its rail and lens.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shot {
    /// Absolute start tick on the sequence timeline.
    pub start: u32,
    /// Length in ticks, at least one.
    pub len: u32,
    /// Camera rail for the take.
    pub rig: Rig,
    /// Lens datum for the take.
    pub lens: Lens,
    /// Blend window easing in from the previous pose (hard cut at 0).
    pub blend_in: u32,
}
/// Ordered takes with absolute starts, one handback glide and markers.
#[derive(Clone, PartialEq, Debug)]
pub struct Sequence {
    /// Takes in timeline order.
    pub shots: Vec<Shot>,
    /// Handback glide ticks after the last take.
    pub handback: u32,
    /// Marker ticks inside the takes; fire once, only playing forward.
    pub markers: Vec<u32>,
}
impl Sequence {
    /// Range-check every row; the culprit row name is carried.
    ///
    /// # Errors
    ///
    /// Fails with `CineError::BadConfig` naming the offending row.
    pub fn validate(&self) -> Result<(), CineError> {
        if self.shots.is_empty() {
            return Err(CineError::BadConfig("shots"));
        }
        if self.handback == 0 {
            return Err(CineError::BadConfig("handback"));
        }
        let mut prev_end = 0u32;
        for (index, shot) in self.shots.iter().enumerate() {
            if shot.len == 0 {
                return Err(CineError::BadConfig("len"));
            }
            if index > 0 && shot.start < prev_end {
                return Err(CineError::BadConfig("start"));
            }
            if shot.blend_in > shot.len {
                return Err(CineError::BadConfig("blend_in"));
            }
            if !shot.lens.valid() {
                return Err(CineError::BadConfig("lens"));
            }
            check_rig(&shot.rig)?;
            prev_end = shot.start.saturating_add(shot.len);
        }
        let mut prev_marker = 0u32;
        for (index, marker) in self.markers.iter().enumerate() {
            if *marker >= prev_end {
                return Err(CineError::BadConfig("markers"));
            }
            if index > 0 && *marker <= prev_marker {
                return Err(CineError::BadConfig("markers"));
            }
            prev_marker = *marker;
        }
        Ok(())
    }
    /// Total take ticks (end of the last take).
    #[must_use]
    pub fn total(&self) -> u32 {
        self.shots
            .iter()
            .map(|shot| shot.start.saturating_add(shot.len))
            .max()
            .unwrap_or(0)
    }
    /// Active take index at `tick`, or `None` past the takes.
    #[must_use]
    pub fn shot_at(&self, tick: u32) -> Option<usize> {
        for (index, shot) in self.shots.iter().enumerate() {
            let end = shot.start.saturating_add(shot.len);
            if (shot.start..end).contains(&tick) {
                return Some(index);
            }
        }
        None
    }
}
/// One presented frame: the pose plus take, marker and done flags.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Frame {
    /// Camera pose for this tick.
    pub pose: Pose,
    /// Take index, or `SHOT_HANDBACK`/`SHOT_DONE` past the takes.
    pub shot: u8,
    /// True when a marker was crossed on this step.
    pub marker: bool,
    /// True once the handback glide completes.
    pub done: bool,
}
/// Take player: snapshots the live pose at handoff, plays the takes, then
/// samples the live camera every frame while gliding home. Forward only.
#[derive(Clone, PartialEq, Debug)]
pub struct Player {
    seq: Sequence,
    end_pose: Pose,
    entry: Pose,
    seed: u64,
    fired: Vec<bool>,
    cursor: u32,
    frac: f32,
    rate: f32,
}
impl Player {
    /// Begin a take over `seq` from the live pose with `seed`.
    ///
    /// # Errors
    ///
    /// Fails with `CineError::BadConfig` when the sequence is out of range.
    pub fn begin(seq: Sequence, live: Pose, seed: u64) -> Result<Self, CineError> {
        if !pose_finite(live) {
            return Err(CineError::BadConfig("live"));
        }
        seq.validate()?;
        let total = seq.total();
        let last = seq.shots.len().saturating_sub(1);
        let mut end_pose = eval_take(&seq, last, total.saturating_sub(1));
        add_shake(&mut end_pose, seed, total.saturating_sub(1));
        Ok(Self {
            fired: vec![false; seq.markers.len()],
            seq,
            end_pose,
            entry: live,
            seed,
            cursor: 0,
            frac: 0.0,
            rate: 1.0,
        })
    }
    /// Playback rate multiplier; `1.0` is realtime.
    ///
    /// # Errors
    ///
    /// Fails with `CineError::BadRate` unless finite and above zero.
    pub fn set_rate(&mut self, rate: f32) -> Result<(), CineError> {
        if !rate.is_finite() || rate <= 0.0 {
            return Err(CineError::BadRate);
        }
        self.rate = rate;
        Ok(())
    }
    /// Shot index at `tick`: take, `SHOT_HANDBACK` or `SHOT_DONE`.
    #[must_use]
    pub fn shot_of(&self, tick: u32) -> u8 {
        if let Some(index) = self.seq.shot_at(tick) {
            u8::try_from(index).unwrap_or(u8::MAX)
        } else if tick < self.seq.total().saturating_add(self.seq.handback) {
            SHOT_HANDBACK
        } else {
            SHOT_DONE
        }
    }
    /// Present one tick, then advance at the set rate. Samples `live` while
    /// gliding home. No heap growth per call after `begin`.
    pub fn step(&mut self, live: Pose) -> Frame {
        let tick = self.cursor;
        let total = self.seq.total();
        let end = total.saturating_add(self.seq.handback);
        let done = tick >= end;
        let shot = self.shot_of(tick);
        self.frac += self.rate;
        while self.frac >= 1.0 {
            self.cursor = self.cursor.saturating_add(1);
            self.frac -= 1.0;
        }
        let mut marker = false;
        for (index, mark) in self.seq.markers.iter().enumerate() {
            let crossed = (tick..=self.cursor).contains(mark);
            if !self.fired[index] && crossed {
                self.fired[index] = true;
                marker = true;
            }
        }
        let mut pose = if tick < total {
            eval_blended(&self.seq, self.entry, tick)
        } else {
            let span = self.seq.handback.max(1);
            let t = smooth01(tick.saturating_sub(total), span);
            blend_pose(self.end_pose, live, t)
        };
        add_shake(&mut pose, self.seed, tick);
        if done {
            pose = live;
        }
        Frame {
            pose,
            shot,
            marker,
            done,
        }
    }
}
/// True when every pose row is finite.
fn pose_finite(pose: Pose) -> bool {
    pose.pos.iter().all(|v| v.is_finite()) && pose.yaw.is_finite() && pose.pitch.is_finite()
}
/// Range-check one rail; the culprit row name is carried.
fn check_rig(rig: &Rig) -> Result<(), CineError> {
    match rig {
        Rig::Keys(from, to) | Rig::Rail(from, to) => {
            if !pose_finite(*from) || !pose_finite(*to) {
                return Err(CineError::BadConfig("rig"));
            }
            Ok(())
        }
        Rig::Orbit {
            center,
            look,
            radius,
            height,
            a0,
            a1,
            damp,
        } => {
            let finite = center.iter().all(|v| v.is_finite())
                && look.iter().all(|v| v.is_finite())
                && radius.is_finite()
                && height.is_finite()
                && a0.is_finite()
                && a1.is_finite()
                && damp.is_finite();
            if !finite || *radius <= 0.0 || !(0.0..=1.0).contains(damp) {
                return Err(CineError::BadConfig("rig"));
            }
            Ok(())
        }
    }
}
/// Linear blend of scalars.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
/// Linear progress of `off` over `span`, `0.0..=1.0`.
fn lin01(off: u32, span: u32) -> f32 {
    let denom = span.max(1);
    let num = u16::try_from(off.min(denom)).unwrap_or(u16::MAX);
    let den = u16::try_from(denom).unwrap_or(u16::MAX);
    f32::from(num) / f32::from(den)
}
/// SmoothStep progress of `off` over `span`, `0.0..=1.0`.
fn smooth01(off: u32, span: u32) -> f32 {
    let t = lin01(off, span);
    t * t * (3.0 - t * 2.0)
}
/// Linear blend of poses.
fn blend_pose(from: Pose, to: Pose, t: f32) -> Pose {
    Pose {
        pos: [
            lerp(from.pos[0], to.pos[0], t),
            lerp(from.pos[1], to.pos[1], t),
            lerp(from.pos[2], to.pos[2], t),
        ],
        yaw: lerp(from.yaw, to.yaw, t),
        pitch: lerp(from.pitch, to.pitch, t),
    }
}
/// Rail pose at take `index` for absolute `tick` (no blend, no shake).
fn eval_take(seq: &Sequence, index: usize, tick: u32) -> Pose {
    let shot = seq.shots[index];
    let off = tick.saturating_sub(shot.start);
    match shot.rig {
        Rig::Keys(from, to) => blend_pose(from, to, smooth01(off, shot.len.max(1))),
        Rig::Rail(from, to) => blend_pose(from, to, lin01(off, shot.len.max(1))),
        Rig::Orbit {
            center,
            look,
            radius,
            height,
            a0,
            a1,
            damp,
        } => {
            let lin = lin01(off, shot.len.max(1));
            let angle = lerp(a0, a1, lin);
            let pos = [
                center[0] + angle.cos() * radius,
                center[1] + height,
                center[2] + angle.sin() * radius,
            ];
            let look_yaw = (look[0] - pos[0]).atan2(look[2] - pos[2]);
            Pose {
                pos,
                yaw: lerp(angle, look_yaw, damp),
                pitch: 0.0,
            }
        }
    }
}
/// Take pose with `blend_in` easing from the previous pose (hard cut at 0).
fn eval_blended(seq: &Sequence, entry: Pose, tick: u32) -> Pose {
    let Some(index) = seq.shot_at(tick) else {
        return entry;
    };
    let shot = seq.shots[index];
    let pose = eval_take(seq, index, tick);
    let off = tick.saturating_sub(shot.start);
    if shot.blend_in == 0 || off >= shot.blend_in {
        return pose;
    }
    let prev = if index == 0 {
        entry
    } else {
        let back = seq.shots[index.saturating_sub(1)];
        let last = back.start.saturating_add(back.len).saturating_sub(1);
        eval_take(seq, index.saturating_sub(1), last)
    };
    blend_pose(prev, pose, smooth01(off, shot.blend_in.max(1)))
}
/// Seeded deterministic shake: same seed plus same tick, same offsets.
fn add_shake(pose: &mut Pose, seed: u64, tick: u32) {
    let mixed = splitmix64(seed.wrapping_add(u64::from(tick)));
    let word = |shift: u32| {
        let slice = (mixed >> shift) & 0xFFFF;
        let q = u16::try_from(slice).unwrap_or(u16::MAX);
        (f32::from(q) / f32::from(u16::MAX) * 2.0 - 1.0) * 0.02
    };
    pose.pos[0] += word(0);
    pose.pos[1] += word(16);
    pose.pos[2] += word(32);
}
#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Fnv64;
    fn pose(x: f32) -> Pose {
        Pose {
            pos: [x, 2.0, 6.0],
            yaw: 0.0,
            pitch: 0.0,
        }
    }
    fn lens() -> Lens {
        Lens {
            fov: FovSpec::FovDeg(40.0),
            filmback: SUPER35,
            focus_m: 5.0,
            aperture: 2.8,
            exposure: 0.0,
        }
    }
    fn proof_sequence() -> Sequence {
        Sequence {
            shots: vec![
                Shot {
                    start: 0,
                    len: 240,
                    rig: Rig::Keys(pose(0.0), pose(4.0)),
                    lens: lens(),
                    blend_in: 30,
                },
                Shot {
                    start: 240,
                    len: 240,
                    rig: Rig::Rail(pose(4.0), pose(9.0)),
                    lens: Lens {
                        fov: FovSpec::FocalMm(35.0),
                        ..lens()
                    },
                    blend_in: 0,
                },
            ],
            handback: HANDBACK_TICKS,
            markers: vec![120],
        }
    }
    fn live_at(tick: u32) -> Pose {
        let step = f32::from(u16::try_from(tick).unwrap());
        Pose {
            pos: [20.0 + step * 0.01, 2.0, 6.0],
            yaw: 0.5,
            pitch: 0.0,
        }
    }
    fn run_take(seed: u64) -> (Vec<Pose>, Vec<u8>, String, u32, u32) {
        let seq = proof_sequence();
        let end = seq.total().saturating_add(seq.handback);
        let mut player = Player::begin(seq, live_at(0), seed).unwrap();
        let mut poses = Vec::new();
        let mut shots = Vec::new();
        let mut digest = Fnv64::new();
        let mut cut = 0u32;
        let mut markers = 0u32;
        let mut last_shot = player.shot_of(0);
        for _ in 0..end.saturating_add(1) {
            let tick = u32::try_from(poses.len()).unwrap();
            let frame = player.step(live_at(tick));
            if frame.marker {
                markers += 1;
            }
            if last_shot == 0 && frame.shot == 1 {
                cut = tick;
            }
            last_shot = frame.shot;
            digest.write_u32(tick);
            digest.write_u8(frame.shot);
            poses.push(frame.pose);
            shots.push(frame.shot);
        }
        let receipt = format!("cut={cut} shots={:016x} markers={markers}", digest.finish());
        (poses, shots, receipt, cut, markers)
    }
    #[test]
    fn cine_sequence_plays_and_posts_cut_timing() {
        let seq = proof_sequence();
        assert_eq!(seq.shots[0].lens.fov_deg(), 40.0, "fov carries through");
        let focal = seq.shots[1].lens.fov_deg();
        assert!(
            (39.0..39.5).contains(&focal),
            "focal resolves on Super35: {focal}"
        );
        let (poses, shots, receipt, cut, markers) = run_take(2040);
        assert_eq!(cut, 240, "hard cut lands exactly at tick 240: {receipt}");
        assert!(
            receipt.starts_with("cut=240 "),
            "receipt posts the cut: {receipt}"
        );
        assert_eq!(markers, 1, "one marker fires once: {receipt}");
        assert_eq!(shots[239], 0, "tick 239 still plays shot zero: {receipt}");
        assert_eq!(shots[240], 1, "tick 240 plays shot one: {receipt}");
        let glides = shots.iter().filter(|shot| **shot == SHOT_HANDBACK).count();
        let cap = usize::try_from(HANDBACK_TICKS).unwrap_or(usize::MAX);
        assert!(glides <= cap, "handback within 90 ticks: {receipt}");
        assert_eq!(
            shots[shots.len() - 1],
            SHOT_DONE,
            "glide completes: {receipt}"
        );
        let (poses_twice, _, receipt_twice, cut_twice, _) = run_take(2040);
        assert_eq!(receipt, receipt_twice, "same seed twice is byte-identical");
        assert_eq!(poses, poses_twice, "pose bytes match twice");
        assert_eq!(cut_twice, 240, "rerun cut matches: {receipt_twice}");
        let home = live_at(570);
        let drift = (poses[poses.len() - 1].pos[0] - home.pos[0]).abs();
        assert!(drift < 0.05, "handback glides home to the live camera");
    }
    #[test]
    fn cine_rejects_bad_takes() {
        let mut seq = proof_sequence();
        seq.shots[1].start = 100;
        assert_eq!(
            Player::begin(seq, live_at(0), 7),
            Err(CineError::BadConfig("start"))
        );
        let mut seq = proof_sequence();
        seq.shots[0].len = 0;
        assert_eq!(
            Player::begin(seq, live_at(0), 7),
            Err(CineError::BadConfig("len"))
        );
        let mut seq = proof_sequence();
        seq.shots[0].blend_in = 500;
        assert_eq!(
            Player::begin(seq, live_at(0), 7),
            Err(CineError::BadConfig("blend_in"))
        );
        let mut seq = proof_sequence();
        seq.shots[0].lens = Lens {
            fov: FovSpec::FovDeg(400.0),
            ..lens()
        };
        assert_eq!(
            Player::begin(seq, live_at(0), 7),
            Err(CineError::BadConfig("lens"))
        );
        let mut seq = proof_sequence();
        seq.markers = vec![900];
        assert_eq!(
            Player::begin(seq, live_at(0), 7),
            Err(CineError::BadConfig("markers"))
        );
        let bad_live = Pose {
            pos: [f32::NAN, 0.0, 0.0],
            yaw: 0.0,
            pitch: 0.0,
        };
        assert_eq!(
            Player::begin(proof_sequence(), bad_live, 7),
            Err(CineError::BadConfig("live"))
        );
        let mut player = Player::begin(proof_sequence(), live_at(0), 7).unwrap();
        assert_eq!(player.set_rate(0.0), Err(CineError::BadRate));
        assert_eq!(player.set_rate(f32::NAN), Err(CineError::BadRate));
        assert!(player.set_rate(2.0).is_ok());
        let orbit = Sequence {
            shots: vec![Shot {
                start: 0,
                len: 60,
                rig: Rig::Orbit {
                    center: [0.0, 0.0, 0.0],
                    radius: 5.0,
                    height: 2.0,
                    a0: 0.0,
                    a1: 1.0,
                    look: [0.0, 1.0, 0.0],
                    damp: 0.5,
                },
                lens: lens(),
                blend_in: 0,
            }],
            handback: 10,
            markers: vec![],
        };
        assert!(Player::begin(orbit, live_at(0), 7).is_ok());
    }
}

//! fighter_look_inertia: inertialized clip switches for the fighter look (LF9).
//!
//! Consumer: [`FighterLook::fold_switches`] starts the offsets after each
//! tick's act fold, and `slot_sample` reads them on every frame through
//! [`FighterLook::inertial_pose`], so the window and the headless view run
//! pose a switch the same way. Data owner: the per-slot
//! [`engine_anim::inertial::Inertial`] sets live on [`FighterLook`]; the
//! spring math is `engine_anim::inertial`, this file only maps poses onto its
//! lanes: lanes `0..n` carry each joint's rotation offset as a scaled axis
//! (parent frame), lanes `n..2n` carry its local position offset. Units:
//! meters, radians, seconds, fixed ticks at 60 Hz. Determinism: presentation
//! only, floats never reach a hash; the same ticks and alphas give the same
//! pose twice, and a held slot's age stands still with its clip clock.
//! Errors: a switch the set refuses (a non-finite pose) is a plain cut with
//! no offset; a frame read the set refuses is a loud [`LookError::Skin`].
//! Heap: the sets are built once at load; a frame spends none beyond the pose
//! it returns (the lane scratch lives on the stack).

use engine_anim::contract::LocalPose;
use engine_anim::inertial::{Inertial, InertialError, MAX_JOINTS};
use engine_core::quat_mul;

use super::{FighterLook, LookError, Rig, SlotAnim, MAX_LOOK_FIGHTERS, TICK_HZ};

/// Halflife of a switch offset in seconds: the time its error halves in. The
/// pop law allows a joint 10 cm of miss against its own speed in one 60 Hz
/// frame; a critically damped decay misses by about `(2 ln 2 / halflife / 60)^2`
/// of the offset, so 0.08 s keeps a 1 m hand offset near that line while the
/// arriving clip still owns the pose within about a third of a second (a
/// shorter halflife pops at attack and idle switches, measured in
/// `fighter_look_proofs`).
const HALFLIFE_S: f32 = 0.08;
/// Halflives after which an offset is dropped: under half a millimeter per
/// meter of offset is left, so the drop is invisible and a settled slot reads
/// the plain clip pose, bit for bit.
const SETTLE_HALFLIVES: f32 = 16.0;
/// Whole ticks after which a switch offset is dropped.
pub(super) const SETTLE_TICKS: u32 = (HALFLIFE_S * SETTLE_HALFLIVES * TICK_HZ) as u32 + 1;
/// Age marking a slot with no offset in flight (never switched, settled, or
/// refused): any age at or past [`SETTLE_TICKS`] reads the plain pose.
pub(super) const NO_OFFSET: u32 = u32::MAX;
/// How far short of the tick start the old slot is read for the last frame
/// the viewer saw, in ticks.
const SEEN_LAG_TICKS: f32 = 0.001;
/// Squared scaled-axis length under which the small-angle form is used.
const TINY_ANGLE_SQ: f32 = 1.0e-12;

/// One set per look slot, sized to that slot's rig: `2 * joints` lanes.
/// Cold path; a rig past the lane cap fails the load.
pub(super) fn slot_sets(rigs: &[Rig]) -> Result<Vec<Inertial>, LookError> {
    (0..MAX_LOOK_FIGHTERS)
        .map(|slot| {
            let rig = rigs
                .get(super::rig_index(rigs.len(), slot))
                .ok_or_else(|| LookError::Skin("inertial sets: no rig".to_string()))?;
            Inertial::new(2 * rig.skeleton.len(), HALFLIFE_S)
                .map_err(|err| LookError::Skin(err.to_string()))
        })
        .collect()
}

/// Seconds since the slot's switch for a frame at sub-tick `alpha`: whole
/// free ticks plus the frame's share. A held slot's frames all show the start
/// of the tick, like its clip clock, so its age does not move in the freeze.
fn age_s(slot: &SlotAnim, alpha: f32) -> f32 {
    let alpha = if slot.held { 0.0 } else { alpha };
    let ticks = f32::from(u16::try_from(slot.iz_age).unwrap_or(u16::MAX));
    (ticks + alpha).max(0.0) / TICK_HZ
}

/// Conjugate (inverse of a unit quaternion).
fn conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

/// Rotate `v` by the unit quaternion `q`.
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let p = quat_mul(quat_mul(q, [v[0], v[1], v[2], 0.0]), conj(q));
    [p[0], p[1], p[2]]
}

/// Scaled axis of a unit quaternion along its shortest arc: the axis times the
/// angle in radians, at most pi long.
fn scaled_axis(q: [f32; 4]) -> [f32; 3] {
    let sign = if q[3] < 0.0 { -1.0 } else { 1.0 };
    let (x, y, z, w) = (sign * q[0], sign * q[1], sign * q[2], sign * q[3]);
    let sin_half = (x * x + y * y + z * z).sqrt();
    let scale = if sin_half * sin_half < TINY_ANGLE_SQ {
        2.0
    } else {
        2.0 * sin_half.atan2(w) / sin_half
    };
    [x * scale, y * scale, z * scale]
}

/// Unit quaternion of a scaled axis: the inverse of [`scaled_axis`].
fn from_scaled_axis(r: [f32; 3]) -> [f32; 4] {
    let angle_sq = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
    if angle_sq < TINY_ANGLE_SQ {
        return [r[0] * 0.5, r[1] * 0.5, r[2] * 0.5, 1.0];
    }
    let angle = angle_sq.sqrt();
    let (sin_half, cos_half) = (angle * 0.5).sin_cos();
    let k = sin_half / angle;
    [r[0] * k, r[1] * k, r[2] * k, cos_half]
}

/// `q` scaled to unit length; a degenerate quaternion reads as identity.
fn unit(q: [f32; 4]) -> [f32; 4] {
    let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if norm.is_finite() && norm > 0.0 {
        [q[0] / norm, q[1] / norm, q[2] / norm, q[3] / norm]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Start the offsets of a switch on `set`. `shown` holds the pose the viewer
/// saw `shown_ticks` ticks before the last one and that last pose, `arriving`
/// the new clip's pose now and a tick on. Each rotation lane starts at the
/// rotation that carries the arriving joint onto the shown one, with the
/// angular speed the shown joint keeps beyond what the arriving clip brings;
/// each position lane starts at shown minus arriving with the speeds
/// likewise. The first frame therefore equals the old pose and moves on at
/// the old speed. A zero `shown_ticks` means no speed to carry.
/// Errors: the set's refusal (lane count, non-finite or out-of-bound floats)
/// or a pose set of mismatched joint counts; a refused call changes nothing.
pub(super) fn begin_switch(
    set: &mut Inertial,
    shown: [&LocalPose; 2],
    shown_ticks: f32,
    arriving: [&LocalPose; 2],
) -> Result<(), InertialError> {
    let shown_rate = if shown_ticks > 0.0 {
        TICK_HZ / shown_ticks
    } else {
        0.0
    };
    let n = shown[0].rotations.len();
    let lanes = 2 * n;
    if lanes > MAX_JOINTS {
        return Err(InertialError::TooManyJoints(lanes));
    }
    for pose in shown.iter().chain(arriving.iter()) {
        if pose.rotations.len() != n || pose.positions.len() != n {
            return Err(InertialError::LengthMismatch {
                expected: n,
                got: pose.rotations.len().min(pose.positions.len()),
            });
        }
    }
    let mut from = [[0.0f32; 3]; MAX_JOINTS];
    let mut from_v = [[0.0f32; 3]; MAX_JOINTS];
    let mut onto = [[0.0f32; 3]; MAX_JOINTS];
    let mut onto_v = [[0.0f32; 3]; MAX_JOINTS];
    for joint in 0..n {
        let turn = quat_mul(
            shown[1].rotations[joint],
            conj(arriving[0].rotations[joint]),
        );
        let shown_w = scaled_axis(quat_mul(
            shown[1].rotations[joint],
            conj(shown[0].rotations[joint]),
        ));
        let arriving_w = scaled_axis(quat_mul(
            arriving[1].rotations[joint],
            conj(arriving[0].rotations[joint]),
        ));
        // shown = turn * arriving, so the offset keeps what the arriving
        // clip's own spin does not explain, seen from the turned frame.
        let carried = rotate(turn, arriving_w);
        from[joint] = scaled_axis(turn);
        from_v[joint] = std::array::from_fn(|k| shown_w[k] * shown_rate - carried[k] * TICK_HZ);
        let lane = n + joint;
        from[lane] = shown[1].positions[joint];
        onto[lane] = arriving[0].positions[joint];
        from_v[lane] = std::array::from_fn(|k| {
            (shown[1].positions[joint][k] - shown[0].positions[joint][k]) * shown_rate
        });
        onto_v[lane] = std::array::from_fn(|k| {
            (arriving[1].positions[joint][k] - arriving[0].positions[joint][k]) * TICK_HZ
        });
    }
    set.switch(
        &from[..lanes],
        &from_v[..lanes],
        &onto[..lanes],
        &onto_v[..lanes],
    )
}

/// Add the set's offsets, `age_s` seconds after its switch, onto `pose`:
/// rotations turn by the decayed scaled axis, positions shift by the decayed
/// offset. Errors: the set's refusal (lane count, age); a refused call leaves
/// `pose` as it was.
pub(super) fn apply_offsets(
    set: &Inertial,
    pose: &mut LocalPose,
    age_s: f32,
) -> Result<(), InertialError> {
    let n = pose.rotations.len().min(pose.positions.len());
    let lanes = 2 * n;
    let mut off = [[0.0f32; 3]; MAX_JOINTS];
    let Some(read) = off.get_mut(..lanes) else {
        return Err(InertialError::TooManyJoints(lanes));
    };
    set.offsets_after(age_s, read)?;
    for joint in 0..n {
        let turn = from_scaled_axis(off[joint]);
        pose.rotations[joint] = unit(quat_mul(turn, pose.rotations[joint]));
        for (p, o) in pose.positions[joint].iter_mut().zip(off[n + joint]) {
            *p += o;
        }
    }
    Ok(())
}

impl FighterLook {
    /// Pose a slot shows at sub-tick `alpha`: the clip pose of
    /// [`slot_pose`](Self::slot_pose) plus the decaying offsets of its last
    /// switch. A slot with no offset in flight reads the clip pose untouched,
    /// bit for bit. Pure fold of the tick, the slot and `alpha`; no heap
    /// beyond the pose.
    pub(super) fn inertial_pose(
        &self,
        rig: &Rig,
        index: usize,
        slot: &SlotAnim,
        alpha: f32,
    ) -> Result<LocalPose, LookError> {
        // RZ2w: a floored slot is drawn from its ragdoll bones, not from a clip.
        if let Some(pose) = self.ragdoll_pose(index, rig, slot, alpha) {
            return Ok(pose);
        }
        let mut pose = self.slot_pose(rig, slot, alpha)?;
        if slot.iz_age < SETTLE_TICKS {
            if let Some(set) = self.inertia.get(index) {
                apply_offsets(set, &mut pose, age_s(slot, alpha))
                    .map_err(|err| LookError::Skin(err.to_string()))?;
            }
        }
        Ok(pose)
    }

    /// Start the offsets of every slot that cuts to another clip this tick:
    /// its act changed, or the clip it shows moves on to another by the end
    /// of the tick (a swing clip running out into the idle clip). `before` is
    /// each slot as the previous tick left it, copied ahead of the cue fold.
    /// The shown pose is that slot, aged to this tick, as the viewer last saw
    /// it and a tick before; the arriving pose is the new slot now and a tick
    /// on, so the offset carries what the viewer saw (offsets still decaying
    /// from an earlier switch included). A switch the set refuses is a plain
    /// cut. Presentation only; the sim hash never reads it.
    pub(super) fn fold_switches(&mut self, before: &[SlotAnim; MAX_LOOK_FIGHTERS], elapsed: u32) {
        for (index, was) in before.iter().enumerate() {
            let Some(slot) = self.slots.get(index).copied() else {
                continue;
            };
            if !slot.seen || !was.seen {
                continue;
            }
            // The previous tick's slot, read at this tick: its offset clock
            // and its run phase must have run with its clip clock.
            let mut shown = *was;
            if !shown.held {
                shown.iz_age = shown.iz_age.saturating_add(elapsed);
                shown.march_run_phase(elapsed);
            }
            if !self.cuts_clip(index, &shown, &slot) {
                continue;
            }
            let started = self.begin_slot_switch(index, &shown, &slot);
            if let Some(now) = self.slots.get_mut(index) {
                now.iz_age = if started.is_ok() { 0 } else { NO_OFFSET };
            }
        }
    }

    /// Where the viewer's last two frames of `shown` sit relative to this
    /// tick, in ticks: the last a hair short of the tick start, so a clip that
    /// ends exactly on the tick still shows its last frame, the one before a
    /// tick earlier; neither before the slot's own start.
    fn last_seen(&self, shown: &SlotAnim) -> (f32, f32) {
        let since =
            f32::from(u16::try_from(self.tick.saturating_sub(shown.act_start)).unwrap_or(u16::MAX));
        let last = (-SEEN_LAG_TICKS).max(-since);
        (last, (last - 1.0).max(-since))
    }

    /// True when `slot` shows another clip than `shown` did at the viewer's
    /// last frame, anywhere up to the end of this tick, or has changed act.
    fn cuts_clip(&self, index: usize, shown: &SlotAnim, slot: &SlotAnim) -> bool {
        if slot.act != shown.act {
            return true;
        }
        let Some(rig) = self.rigs.get(self.rig_for_slot(index)) else {
            return false;
        };
        let was = self.clip_pick(rig, shown, self.last_seen(shown).0).0;
        let end = self.clip_pick(rig, slot, 1.0 - SEEN_LAG_TICKS).0;
        was != end
    }

    /// Sample the shown and arriving poses of one switch and hand them to the
    /// slot's set. `Err` means no offset was started.
    fn begin_slot_switch(
        &mut self,
        index: usize,
        shown: &SlotAnim,
        slot: &SlotAnim,
    ) -> Result<(), LookError> {
        let rig = self
            .rigs
            .get(self.rig_for_slot(index))
            .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
        let (seen_now, seen_before) = self.last_seen(shown);
        let shown_before = self.inertial_pose(rig, index, shown, seen_before)?;
        let shown_now = self.inertial_pose(rig, index, shown, seen_now)?;
        let arriving_now = self.slot_pose(rig, slot, 0.0)?;
        let arriving_next = self.slot_pose(rig, slot, 1.0)?;
        let set = self
            .inertia
            .get_mut(index)
            .ok_or_else(|| LookError::Skin("inertial set missing".to_string()))?;
        begin_switch(
            set,
            [&shown_before, &shown_now],
            seen_now - seen_before,
            [&arriving_now, &arriving_next],
        )
        .map_err(|err| LookError::Skin(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_offsets, begin_switch, from_scaled_axis, scaled_axis, HALFLIFE_S, TICK_HZ};
    use engine_anim::contract::LocalPose;
    use engine_anim::inertial::{Inertial, InertialError};

    /// Unit quaternion for `angle` radians about z (half-angle sin and cos).
    fn about_z(angle: f32) -> [f32; 4] {
        let (s, c) = (angle * 0.5).sin_cos();
        [0.0, 0.0, s, c]
    }

    /// Two joints: joint 0 turned `angle` about z, joint 1 moved to `x`.
    fn pose(angle: f32, x: f32) -> LocalPose {
        LocalPose {
            positions: vec![[0.0, 1.0, 0.0], [x, 0.5, 0.0]],
            rotations: vec![about_z(angle), [0.0, 0.0, 0.0, 1.0]],
        }
    }

    fn set() -> Inertial {
        Inertial::new(4, HALFLIFE_S).expect("two joints make four lanes")
    }

    /// Largest per-component gap between two poses, rotations compared
    /// through their z angle (all fixtures turn about z).
    fn gap(a: &LocalPose, b: &LocalPose) -> f32 {
        let pos = a
            .positions
            .iter()
            .flatten()
            .zip(b.positions.iter().flatten())
            .map(|(p, q)| (p - q).abs());
        let rot = a
            .rotations
            .iter()
            .zip(&b.rotations)
            .map(|(p, q)| (scaled_axis(*p)[2] - scaled_axis(*q)[2]).abs());
        pos.chain(rot).fold(0.0, f32::max)
    }

    #[test]
    fn scaled_axis_round_trips_and_takes_the_short_way() {
        for angle in [0.0f32, 1.0e-4, 0.4, 1.5, 3.0, -2.2] {
            let q = about_z(angle);
            let back = from_scaled_axis(scaled_axis(q));
            let dot: f32 = q.iter().zip(back).map(|(a, b)| a * b).sum();
            assert!(
                (dot.abs() - 1.0).abs() < 1e-5,
                "angle {angle}: round trip {back:?} vs {q:?}"
            );
            // The same rotation as -q is the same axis, never the long way.
            let flipped = [-q[0], -q[1], -q[2], -q[3]];
            let (a, b) = (scaled_axis(q), scaled_axis(flipped));
            assert!(
                (0..3).all(|k| (a[k] - b[k]).abs() < 1e-5),
                "angle {angle}: q and -q gave {a:?} and {b:?}"
            );
            let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
            assert!(len <= std::f32::consts::PI + 1e-5, "angle {angle}: {len}");
        }
    }

    /// Slide speed of joint 1 (m/s) and spin speed of joint 0 (rad/s) just
    /// after the cut, from a short step of the offsets.
    fn speeds_after_cut(iz: &Inertial, arriving: &LocalPose) -> (f32, f32) {
        let eps = 1.0e-4;
        let (mut at, mut after) = (arriving.clone(), arriving.clone());
        apply_offsets(iz, &mut at, 0.0).expect("age zero");
        apply_offsets(iz, &mut after, eps).expect("a hair on");
        let slide = (after.positions[1][0] - at.positions[1][0]) / eps;
        let spin = (scaled_axis(after.rotations[0])[2] - scaled_axis(at.rotations[0])[2]) / eps;
        (slide, spin)
    }

    #[test]
    fn switch_frame_is_the_old_pose_and_the_old_speed_carries_on() {
        // Joint 0 spins at 3 rad/s and joint 1 slides at 2 m/s, then the clip
        // cuts to a still, different pose.
        let dt = 1.0 / TICK_HZ;
        let before = pose(0.5 - 3.0 * dt, 0.2 - 2.0 * dt);
        let now = pose(0.5, 0.2);
        let still = pose(-1.0, 0.9);
        let mut moving = set();
        begin_switch(&mut moving, [&before, &now], 1.0, [&still, &still]).expect("switch");
        let mut at_cut = still.clone();
        apply_offsets(&moving, &mut at_cut, 0.0).expect("age zero");
        assert!(
            gap(&at_cut, &now) < 1e-5,
            "first frame {}",
            gap(&at_cut, &now)
        );
        let (slide, spin) = speeds_after_cut(&moving, &still);
        assert!((slide - 2.0).abs() < 0.1, "slide {slide} m/s, wanted 2");
        assert!((spin - 3.0).abs() < 0.1, "spin {spin} rad/s, wanted 3");
        // The same cut from a pose that was not moving starts at rest.
        let mut resting = set();
        begin_switch(&mut resting, [&now, &now], 1.0, [&still, &still]).expect("switch");
        let (slide, spin) = speeds_after_cut(&resting, &still);
        assert!(slide.abs() < 0.5, "a resting joint slides {slide} m/s");
        assert!(spin.abs() < 0.6, "a resting joint spins {spin} rad/s");
    }

    #[test]
    fn offsets_settle_onto_the_arriving_pose() {
        let now = pose(2.0, 1.0);
        let still = pose(-1.0, 0.0);
        let mut iz = set();
        begin_switch(&mut iz, [&now, &now], 0.0, [&still, &still]).expect("switch");
        let mut late = still.clone();
        apply_offsets(&iz, &mut late, 20.0 * HALFLIFE_S).expect("settled age");
        assert!(gap(&late, &still) < 1e-3, "left {}", gap(&late, &still));
    }

    #[test]
    fn starting_and_reading_offsets_allocates_nothing() {
        use engine_core::alloc_probe::measure;
        let (now, still) = (pose(0.5, 0.2), pose(-1.0, 0.9));
        let mut iz = set();
        let mut shown = still.clone();
        let ((), heap) = measure(|| {
            begin_switch(&mut iz, [&now, &now], 1.0, [&still, &still]).expect("switch");
            for k in 0..240u16 {
                let age = f32::from(k) / 240.0;
                apply_offsets(&iz, &mut shown, age).expect("read");
                std::hint::black_box(&shown);
            }
        });
        assert_eq!(heap.calls, 0, "a switch and 240 frame reads touch no heap");
    }

    #[test]
    fn a_refused_switch_or_read_changes_nothing() {
        let (now, still) = (pose(0.5, 0.2), pose(-1.0, 0.9));
        let mut iz = set();
        begin_switch(&mut iz, [&now, &now], 0.0, [&still, &still]).expect("switch");
        let kept = iz.clone();
        // A pose with a different joint count.
        let short = LocalPose {
            positions: vec![[0.0; 3]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]],
        };
        assert!(matches!(
            begin_switch(&mut iz, [&short, &short], 1.0, [&still, &still]),
            Err(InertialError::LengthMismatch { .. })
        ));
        // A set sized for other lanes than the poses carry.
        let mut wrong = Inertial::new(2, HALFLIFE_S).expect("two lanes");
        assert!(matches!(
            begin_switch(&mut wrong, [&now, &now], 1.0, [&still, &still]),
            Err(InertialError::LengthMismatch { .. })
        ));
        // A non-finite pose.
        let mut nan = now.clone();
        nan.positions[1][0] = f32::NAN;
        assert!(matches!(
            begin_switch(&mut iz, [&nan, &nan], 1.0, [&still, &still]),
            Err(InertialError::NonFinite(_))
        ));
        assert_eq!(iz, kept, "refusals leave the set alone");
        // Reads: the wrong lane count and a bad age leave the pose alone.
        let mut pose_out = still.clone();
        assert!(apply_offsets(&wrong, &mut pose_out, 0.0).is_err());
        assert!(apply_offsets(&iz, &mut pose_out, f32::NAN).is_err());
        assert_eq!(pose_out, still, "refused reads write nothing");
    }
}

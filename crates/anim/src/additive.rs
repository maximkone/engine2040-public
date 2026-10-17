//! additive: locomotion base, combat upper body and flinch additive through
//! one inertializer, with analytic two-bone IK pinning planted feet.
//!
//! Owner: `engine-anim`; consumer: the fighter look calls [`AdditiveRig::tick`]
//! once per presented tick instead of stacking per-concern blends. It reuses
//! the masked override of `layers`, the spring of `inertial` (positions) and
//! [`solve_limb`]; the only new math is the rotation offset decay and the
//! rotation additive. Stages per tick: base, masked upper-body override,
//! masked additive flinch (positions and rotations), then ONE inertializer
//! over the composed pose (a cut at an interruption keeps position and
//! rotation continuous), then IK on every foot whose contact window is open.
//! Units: meters, unit quats `[x, y, z, w]`, seconds; Q8 gain (256 reads 1x).
//! Determinism: `+ - * /` and `sqrt` only, joint order, no clock; replays
//! bit-identically. Presentation only: floats never enter a hash. Errors:
//! [`AdditiveError`]; a refused tick writes nothing. Heap: all buffers are
//! sized in [`AdditiveRig::new`], a tick allocates nothing.

use crate::clip::TICK_DT;
use crate::contract::{LocalPose, Skeleton};
use crate::graph::nlerp;
use crate::ik::{limb_positions, solve_limb, IkError, Limb};
use crate::inertial::{Inertial, InertialError};
use crate::layers::{override_joint, MaskTable, GAIN_Q8_MAX};
use core::f32::consts::LN_2;
use engine_core::quat_mul;

const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Why a tick refused. A refused tick changes nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AdditiveError {
    /// A pose, mask or the rig disagrees on joint count.
    Width {
        /// Rig joint count.
        expected: usize,
        /// Count the caller offered.
        got: usize,
    },
    /// Contact flags do not match the limb count.
    Plants {
        /// Limbs the rig pins.
        expected: usize,
        /// Flags the caller offered.
        got: usize,
    },
    /// Flinch gain past [`GAIN_Q8_MAX`].
    BadGain(u16),
    /// The inertializer refused (non-finite or out-of-range lane).
    Inertial(InertialError),
    /// A foot solve refused.
    Ik(IkError),
}

impl core::fmt::Display for AdditiveError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Width { expected, got } => {
                write!(f, "additive: {got} joints differ from rig {expected}")
            }
            Self::Plants { expected, got } => {
                write!(f, "additive: {got} contact flags for {expected} limbs")
            }
            Self::BadGain(g) => write!(f, "additive: flinch gain {g} past {GAIN_Q8_MAX}"),
            Self::Inertial(e) => write!(f, "additive: {e}"),
            Self::Ik(e) => write!(f, "additive: foot solve refused ({e:?})"),
        }
    }
}

impl std::error::Error for AdditiveError {}

/// One tick of inputs. All poses are local, one entry per rig joint.
#[derive(Clone, Copy, Debug)]
pub struct Inputs<'a> {
    /// Locomotion pose.
    pub base: &'a LocalPose,
    /// Combat pose that overrides the base where `upper_mask` is set.
    pub upper: &'a LocalPose,
    /// Per-joint override weight for `upper`.
    pub upper_mask: &'a MaskTable,
    /// Flinch additive: position offsets and rotation deltas off identity.
    pub flinch: &'a LocalPose,
    /// Per-joint weight of the flinch.
    pub flinch_mask: &'a MaskTable,
    /// Flinch gain in Q8 (256 reads 1x).
    pub flinch_gain_q8: u16,
    /// True on a tick where a clip switched or a strike landed: the shown
    /// pose is kept and eased onto the new composition.
    pub interrupt: bool,
    /// One flag per rig limb: the foot is in a contact window.
    pub planted: &'a [bool],
}

/// The additive spine for one fighter.
#[derive(Clone, Debug)]
pub struct AdditiveRig {
    limbs: Vec<Limb>,
    anchors: Vec<Option<[f32; 3]>>,
    inertial: Inertial,
    /// Per-tick rotation offset keep factor, `1 / (1 + rate * dt)`.
    rot_keep: f32,
    rot_off: Vec<[f32; 4]>,
    composed: LocalPose,
    shown: LocalPose,
    shown_vel: Vec<[f32; 3]>,
    steady_vel: Vec<[f32; 3]>,
    prev_composed: Vec<[f32; 3]>,
    scratch: Vec<[f32; 3]>,
    started: bool,
}

impl AdditiveRig {
    /// Rig for `joints` joints pinning `limbs`, with the inertializer
    /// `halflife_s` (seconds).
    pub fn new(joints: usize, limbs: Vec<Limb>, halflife_s: f32) -> Result<Self, AdditiveError> {
        let inertial = Inertial::new(joints, halflife_s).map_err(AdditiveError::Inertial)?;
        let rate = 2.0 * LN_2 / halflife_s;
        let blank = LocalPose {
            positions: vec![[0.0; 3]; joints],
            rotations: vec![IDENTITY; joints],
        };
        Ok(Self {
            anchors: vec![None; limbs.len()],
            limbs,
            inertial,
            rot_keep: 1.0 / (1.0 + rate * TICK_DT),
            rot_off: vec![IDENTITY; joints],
            composed: blank.clone(),
            shown: blank,
            shown_vel: vec![[0.0; 3]; joints],
            steady_vel: vec![[0.0; 3]; joints],
            prev_composed: vec![[0.0; 3]; joints],
            scratch: vec![[0.0; 3]; joints],
            started: false,
        })
    }

    /// Joint count the rig was built for.
    pub fn joints(&self) -> usize {
        self.rot_off.len()
    }

    /// The pose shown by the last tick.
    pub fn shown(&self) -> &LocalPose {
        &self.shown
    }

    /// Compose one tick into `out`. The first tick shows the composition
    /// as is. Atomic on a shape error: nothing is written.
    pub fn tick(
        &mut self,
        skeleton: &Skeleton,
        inputs: &Inputs<'_>,
        out: &mut LocalPose,
    ) -> Result<(), AdditiveError> {
        let n = self.joints();
        self.check(skeleton, inputs, out)?;
        self.compose(inputs);
        let cut = inputs.interrupt && self.started;
        if cut {
            self.inertial
                .switch(
                    &self.shown.positions,
                    &self.shown_vel,
                    &self.composed.positions,
                    &self.steady_vel,
                )
                .map_err(AdditiveError::Inertial)?;
            for j in 0..n {
                self.rot_off[j] =
                    quat_mul(self.shown.rotations[j], conj(self.composed.rotations[j]));
            }
        } else if self.started {
            for j in 0..n {
                for k in 0..3 {
                    self.steady_vel[j][k] =
                        (self.composed.positions[j][k] - self.prev_composed[j][k]) / TICK_DT;
                }
            }
        }
        // A cut shows the held pose this tick (zero step); later ticks decay.
        let dt = if cut || !self.started { 0.0 } else { TICK_DT };
        self.inertial
            .step(&self.composed.positions, dt, &mut self.scratch)
            .map_err(AdditiveError::Inertial)?;
        if !cut && self.started {
            for off in &mut self.rot_off {
                *off = nlerp(IDENTITY, *off, self.rot_keep);
            }
        }
        out.positions.copy_from_slice(&self.scratch);
        for j in 0..n {
            out.rotations[j] = quat_mul(self.rot_off[j], self.composed.rotations[j]);
        }
        self.pin_feet(skeleton, inputs.planted, out)?;
        for j in 0..n {
            for k in 0..3 {
                self.shown_vel[j][k] = if self.started {
                    (out.positions[j][k] - self.shown.positions[j][k]) / TICK_DT
                } else {
                    0.0
                };
            }
        }
        self.shown.positions.copy_from_slice(&out.positions);
        self.shown.rotations.copy_from_slice(&out.rotations);
        self.prev_composed.copy_from_slice(&self.composed.positions);
        self.started = true;
        Ok(())
    }

    fn check(
        &self,
        skeleton: &Skeleton,
        inputs: &Inputs<'_>,
        out: &LocalPose,
    ) -> Result<(), AdditiveError> {
        let n = self.joints();
        let widths = [
            skeleton.len(),
            inputs.base.positions.len(),
            inputs.base.rotations.len(),
            inputs.upper.positions.len(),
            inputs.upper.rotations.len(),
            inputs.flinch.positions.len(),
            inputs.flinch.rotations.len(),
            inputs.upper_mask.len(),
            inputs.flinch_mask.len(),
            out.positions.len(),
            out.rotations.len(),
        ];
        if let Some(got) = widths.into_iter().find(|w| *w != n) {
            return Err(AdditiveError::Width { expected: n, got });
        }
        if inputs.planted.len() != self.limbs.len() {
            return Err(AdditiveError::Plants {
                expected: self.limbs.len(),
                got: inputs.planted.len(),
            });
        }
        if inputs.flinch_gain_q8 > GAIN_Q8_MAX {
            return Err(AdditiveError::BadGain(inputs.flinch_gain_q8));
        }
        Ok(())
    }

    /// Base, masked upper override, masked additive flinch, joint by joint.
    fn compose(&mut self, i: &Inputs<'_>) {
        let gain = f32::from(i.flinch_gain_q8) / 65280.0;
        for j in 0..self.joints() {
            let (mut p, mut r) = override_joint(
                i.base.positions[j],
                i.base.rotations[j],
                i.upper.positions[j],
                i.upper.rotations[j],
                i.upper_mask.weights()[j],
            );
            let s = f32::from(i.flinch_mask.weights()[j]) * gain;
            if s > 0.0 {
                let d = i.flinch.positions[j];
                p = [p[0] + d[0] * s, p[1] + d[1] * s, p[2] + d[2] * s];
                r = quat_mul(r, nlerp(IDENTITY, i.flinch.rotations[j], s.min(1.0)));
            }
            self.composed.positions[j] = p;
            self.composed.rotations[j] = r;
        }
    }

    /// Pin every planted foot to where it stood when its window opened.
    fn pin_feet(
        &mut self,
        skeleton: &Skeleton,
        planted: &[bool],
        out: &mut LocalPose,
    ) -> Result<(), AdditiveError> {
        for (i, limb) in self.limbs.iter().enumerate() {
            if !planted[i] {
                self.anchors[i] = None;
                continue;
            }
            let [_, mid, end] = limb_positions(skeleton, out, *limb).map_err(AdditiveError::Ik)?;
            let anchor = *self.anchors[i].get_or_insert(end);
            solve_limb(skeleton, out, *limb, anchor, mid).map_err(AdditiveError::Ik)?;
        }
        Ok(())
    }
}

fn conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Joint;
    use crate::feet::{SLIDE_BUDGET_MM_PIVOT, SLIDE_BUDGET_MM_STRAIGHT};
    use engine_core::alloc_probe::measure;
    use engine_core::Fnv64;

    // 0 pelvis, 1-3 left hip/knee/ankle, 4-6 right, 7 spine, 8 arm.
    const JOINTS: usize = 9;
    const LEFT: usize = 0;

    fn skeleton() -> Skeleton {
        let parents = [
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(0),
            Some(4),
            Some(5),
            Some(0),
            Some(7),
        ];
        Skeleton {
            joints: parents
                .iter()
                .enumerate()
                .map(|(i, p)| Joint {
                    name: format!("j{i}"),
                    parent: *p,
                })
                .collect(),
        }
    }

    fn limbs() -> Vec<Limb> {
        let sk = skeleton();
        vec![
            Limb::new(&sk, 1, 2, 3).expect("left leg"),
            Limb::new(&sk, 4, 5, 6).expect("right leg"),
        ]
    }

    /// Standing pose with the pelvis `(x, y, z)` and yaw `yaw` (small rad).
    fn stand(pelvis: [f32; 3], yaw: f32, arm_x: f32) -> LocalPose {
        let mut pose = LocalPose {
            positions: vec![
                pelvis,
                [-0.1, 0.0, 0.0],
                [0.0, -0.45, 0.03],
                [0.0, -0.45, 0.0],
                [0.1, 0.0, 0.0],
                [0.0, -0.45, 0.03],
                [0.0, -0.45, 0.0],
                [0.0, 0.3, 0.0],
                [arm_x, 0.2, 0.0],
            ],
            rotations: vec![IDENTITY; JOINTS],
        };
        let h = yaw * 0.5;
        let n = (h * h + 1.0).sqrt();
        pose.rotations[0] = [0.0, h / n, 0.0, 1.0 / n];
        pose
    }

    fn masks() -> (MaskTable, MaskTable) {
        let mut upper = vec![0u8; JOINTS];
        upper[7] = 255;
        upper[8] = 255;
        (
            MaskTable::from_vec(upper).expect("upper"),
            MaskTable::full(JOINTS).expect("flinch"),
        )
    }

    fn inputs<'a>(
        base: &'a LocalPose,
        upper: &'a LocalPose,
        masks: &'a (MaskTable, MaskTable),
        planted: &'a [bool],
        gain: u16,
    ) -> Inputs<'a> {
        Inputs {
            base,
            upper,
            upper_mask: &masks.0,
            flinch: base,
            flinch_mask: &masks.1,
            flinch_gain_q8: gain,
            interrupt: false,
            planted,
        }
    }

    fn rig() -> AdditiveRig {
        AdditiveRig::new(JOINTS, limbs(), 0.1).expect("rig")
    }

    fn end_of(pose: &LocalPose, limb: usize) -> [f32; 3] {
        limb_positions(&skeleton(), pose, limbs()[limb]).expect("limb")[2]
    }

    fn dist_mm(a: [f32; 3], b: [f32; 3]) -> f32 {
        let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * 1000.0
    }

    /// Run `ticks` ticks: the pelvis walks `(speed * t)` forward with `yaw_rate`,
    /// left foot planted from tick 5. Returns worst left-foot slide (mm) in the
    /// window and the final hash.
    fn walk(plant: bool, yaw_rate: f32) -> (f32, u64) {
        let sk = skeleton();
        let masks = masks();
        let mut rig = rig();
        let mut out = stand([0.0, 0.8, 0.0], 0.0, 0.0);
        let still = stand([0.0; 3], 0.0, 0.3);
        let mut anchor = None;
        let mut worst = 0.0f32;
        let mut h = Fnv64::new();
        for t in 0..40u32 {
            let base = stand([0.0, 0.8, 0.003 * t as f32], yaw_rate * t as f32, 0.0);
            let planted = [plant && t >= 5, false];
            let i = inputs(&base, &still, &masks, &planted, 0);
            rig.tick(&sk, &i, &mut out).expect("tick");
            if t >= 5 {
                let foot = end_of(&out, LEFT);
                let a = *anchor.get_or_insert(foot);
                worst = worst.max(dist_mm(foot, a));
            }
            for p in out
                .positions
                .iter()
                .flatten()
                .chain(out.rotations.iter().flatten())
            {
                h.write_f32(*p);
            }
        }
        (worst, h.finish())
    }

    #[test]
    fn planted_foot_slides_under_the_straight_budget() {
        let (pinned, _) = walk(true, 0.0);
        assert!(
            pinned <= SLIDE_BUDGET_MM_STRAIGHT as f32,
            "slid {pinned} mm"
        );
        let (free, _) = walk(false, 0.0);
        assert!(free > 60.0, "unpinned foot rides the pelvis: {free} mm");
    }

    #[test]
    fn planted_foot_slides_under_the_pivot_budget() {
        let (pinned, _) = walk(true, 0.03);
        assert!(pinned <= SLIDE_BUDGET_MM_PIVOT as f32, "slid {pinned} mm");
    }

    #[test]
    fn replay_is_exact_x2() {
        assert_eq!(walk(true, 0.03).1, walk(true, 0.03).1);
    }

    /// An interruption at tick 30 swaps the base for a pose 0.3 m away and
    /// adds a flinch; the shown pose must stay continuous, then settle.
    #[test]
    fn interruption_does_not_pop() {
        let sk = skeleton();
        let masks = masks();
        let mut rig = rig();
        let mut out = stand([0.0, 0.95, 0.0], 0.0, 0.0);
        let upper = stand([0.0; 3], 0.0, 0.3);
        let mut flinch = LocalPose {
            positions: vec![[0.0; 3]; JOINTS],
            rotations: vec![IDENTITY; JOINTS],
        };
        flinch.positions[7] = [0.0, 0.0, -0.05];
        flinch.rotations[7] = [0.0, 0.0, 0.25, 0.968_245_8];
        let mut prev = out.clone();
        let mut worst_step = 0.0f32;
        let mut cut_step = 0.0f32;
        let mut final_gap = 0.0f32;
        for t in 0..140u32 {
            let cut = t >= 30;
            let base = if cut {
                stand([0.3, 0.95, 0.2], 0.4, 0.0)
            } else {
                stand([0.0, 0.95, 0.0], 0.0, 0.0)
            };
            let i = Inputs {
                flinch: &flinch,
                interrupt: t == 30,
                ..inputs(
                    &base,
                    &upper,
                    &masks,
                    &[false, false],
                    if t == 30 { 256 } else { 0 },
                )
            };
            rig.tick(&sk, &i, &mut out).expect("tick");
            let step = (0..JOINTS)
                .map(|j| dist_mm(out.positions[j], prev.positions[j]))
                .fold(0.0, f32::max);
            if t == 30 {
                cut_step = step;
            } else if t > 30 {
                worst_step = worst_step.max(step);
            }
            final_gap = (0..JOINTS)
                .map(|j| dist_mm(out.positions[j], rig.composed.positions[j]))
                .fold(0.0, f32::max);
            prev = out.clone();
        }
        assert!(cut_step < 1.0, "cut tick moved {cut_step} mm: a pop");
        assert!(
            worst_step < 120.0,
            "recovery stepped {worst_step} mm in a tick"
        );
        assert!(final_gap < 1.0, "never settled: {final_gap} mm off");
        // The rotation offset decays too: the pelvis yaw ends on the base.
        let want = rig.composed.rotations[0];
        let dot: f32 = out.rotations[0].iter().zip(want).map(|(a, b)| a * b).sum();
        assert!(dot.abs() > 0.99999, "rotation never settled: {dot}");
    }

    #[test]
    fn shape_errors_write_nothing() {
        let sk = skeleton();
        let masks = masks();
        let mut rig = rig();
        let base = stand([0.0, 0.95, 0.0], 0.0, 0.0);
        let short = LocalPose {
            positions: vec![[0.0; 3]; 3],
            rotations: vec![IDENTITY; 3],
        };
        let mut out = base.clone();
        let wide = inputs(&base, &short, &masks, &[false, false], 0);
        assert_eq!(
            rig.tick(&sk, &wide, &mut out),
            Err(AdditiveError::Width {
                expected: JOINTS,
                got: 3
            })
        );
        let flags = inputs(&base, &base, &masks, &[true], 0);
        assert_eq!(
            rig.tick(&sk, &flags, &mut out),
            Err(AdditiveError::Plants {
                expected: 2,
                got: 1
            })
        );
        let gain = inputs(&base, &base, &masks, &[false, false], GAIN_Q8_MAX + 1);
        assert_eq!(
            rig.tick(&sk, &gain, &mut out),
            Err(AdditiveError::BadGain(GAIN_Q8_MAX + 1))
        );
        assert_eq!(out, base, "refused ticks leave out alone");
    }

    #[test]
    fn steady_ticks_allocate_nothing() {
        let sk = skeleton();
        let masks = masks();
        let mut rig = rig();
        let base = stand([0.0, 0.95, 0.0], 0.0, 0.0);
        let mut out = base.clone();
        let i = inputs(&base, &base, &masks, &[true, false], 0);
        rig.tick(&sk, &i, &mut out).expect("warm");
        let ((), heap) = measure(|| {
            for _ in 0..50 {
                rig.tick(&sk, &i, &mut out).expect("tick");
            }
        });
        assert_eq!(heap.calls, 0, "additive tick must not allocate");
    }

    /// Cost report for the 0.05 ms bar: `cargo test -p engine-anim --release
    /// --lib additive_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing report, run in release on the reference CPU"]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "wall clock in a test-only timing report, never in the tick"
    )]
    fn additive_cost_per_tick() {
        let sk = skeleton();
        let masks = masks();
        let mut rig = rig();
        let base = stand([0.0, 0.95, 0.0], 0.0, 0.0);
        let mut out = base.clone();
        let i = inputs(&base, &base, &masks, &[true, true], 256);
        let t = std::time::Instant::now();
        for _ in 0..10_000 {
            rig.tick(&sk, &i, &mut out).expect("tick");
        }
        let ns = t.elapsed().as_nanos() / 10_000;
        println!("additive: {ns} ns per tick (bar 50000)");
        assert!(ns < 50_000, "{ns} ns past the 0.05 ms bar");
    }
}

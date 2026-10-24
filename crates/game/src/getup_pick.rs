//! getup_pick: which get-up clip a lying body rises into (QR8).
//!
//! A get-up is not one clip. A body on its back, on its face, or on either side rises out of a
//! different start pose, and playing the wrong one snaps the body a meter or more on the first
//! frame. This module scores a small clip set: the chest's facing picks the front/back pair, then
//! the clip whose start frame is nearest the lying pose wins. Everything here is pure geometry on
//! positions in meters, so a test can drive it from a table of four poses without a rig, a body
//! file or a window.
//!
//! Facing comes from the chest normal, `cross(across, spine)` of the torso: its y sign says
//! whether the chest looks at the sky (face up) or at the sand (face down). A body on its side
//! has that normal level, and only the nearest start frame can tell left from right.
//! Presentation only: the sim never reads this and nothing here is hashed.

use engine_anim::contract::{LocalPose, Skeleton};

use super::length;

/// Which way a lying body's chest looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Facing {
    /// Chest to the sky.
    Up,
    /// Chest to the sand.
    Down,
    /// Chest level: the body lies on a side.
    Side,
}

/// One get-up candidate: its rig-facing side and the joint positions of its first frame.
pub(crate) struct Clip<'a> {
    /// Name of the clip file, for the receipt.
    #[allow(dead_code, reason = "the get-up tests read the clip name")]
    pub name: &'static str,
    /// Which way its first frame's chest looks.
    pub facing: Facing,
    /// Joint positions of the clip's first frame, meters, one per skeleton joint.
    pub start: &'a [[f32; 3]],
}

/// The chest normal's up share above which the body reads as face up (and below its negative,
/// face down): a body tipped more than 60 degrees off its side. The chest normal of a body on its
/// side is level, so the band only has to keep a sloppy side pose from reading as a face pose.
const FACING_COS: f32 = 0.5;

/// A get-up clip whose first frame sits further than this from the lying body, meters, is not a
/// get-up clip: the blend is refused and the pose is kept rather than flown to it.
const MISS_MAX_M: f32 = 3.0;

/// Which way a chest normal looks.
pub(crate) fn facing(chest_normal: [f32; 3]) -> Facing {
    let y = chest_normal[1];
    if !y.is_finite() {
        return Facing::Side;
    }
    if y > FACING_COS {
        Facing::Up
    } else if y < -FACING_COS {
        Facing::Down
    } else {
        Facing::Side
    }
}

/// The chest normal of a posed body: `cross(shoulder_r - shoulder_l, head - pelvis)`, unit
/// length, or `None` when the two spans are collinear or non-finite.
///
/// The order is the sign convention: on an upright body this normal points at the fighter's
/// front, so a body laid on its back gives `+1` on the up axis and one face down gives `-1`.
pub(crate) fn chest_normal(
    pelvis: [f32; 3],
    head: [f32; 3],
    shoulder_l: [f32; 3],
    shoulder_r: [f32; 3],
) -> Option<[f32; 3]> {
    let spine = [
        head[0] - pelvis[0],
        head[1] - pelvis[1],
        head[2] - pelvis[2],
    ];
    let across = [
        shoulder_r[0] - shoulder_l[0],
        shoulder_r[1] - shoulder_l[1],
        shoulder_r[2] - shoulder_l[2],
    ];
    let normal = [
        across[1] * spine[2] - across[2] * spine[1],
        across[2] * spine[0] - across[0] * spine[2],
        across[0] * spine[1] - across[1] * spine[0],
    ];
    let len = length(normal);
    if !len.is_finite() || len <= f32::EPSILON {
        return None;
    }
    let unit = [normal[0] / len, normal[1] / len, normal[2] / len];
    unit.iter().all(|c| c.is_finite()).then_some(unit)
}

/// The four skeleton joints the chest normal reads: pelvis, head and both shoulders.
pub(crate) fn stops(skeleton: &Skeleton) -> Option<[usize; 4]> {
    let mut found = [0usize; 4];
    for (slot, name) in ["pelvis", "head", "shoulder_l", "shoulder_r"]
        .iter()
        .enumerate()
    {
        found[slot] = skeleton
            .joints
            .iter()
            .position(|j| j.name.as_str() == *name)?;
    }
    Some(found)
}

/// The chest normal of a pose, from the joints [`stops`] names.
pub(crate) fn pose_chest_normal(pose: &LocalPose, indices: [usize; 4]) -> Option<[f32; 3]> {
    let at = |i: usize| pose.positions.get(i).copied();
    chest_normal(
        at(indices[0])?,
        at(indices[1])?,
        at(indices[2])?,
        at(indices[3])?,
    )
}

/// How far a clip's first frame is from the lying pose: the worst joint, meters. A pair of
/// different lengths, and any non-finite joint, score as infinite, so a broken candidate never
/// wins.
pub(crate) fn miss_m(clip_start: &[[f32; 3]], lying: &[[f32; 3]]) -> f32 {
    if clip_start.len() != lying.len() {
        return f32::INFINITY;
    }
    let mut worst = 0.0f32;
    for (a, b) in clip_start.iter().zip(lying) {
        let len = length([a[0] - b[0], a[1] - b[1], a[2] - b[2]]);
        if !len.is_finite() {
            return f32::INFINITY;
        }
        worst = worst.max(len);
    }
    worst
}

/// The get-up clip a body lying at `lying` rises into, as an index into `clips`.
///
/// A face-up or face-down body only considers clips of its own facing; a body on its side, and a
/// face body with no clip on its side, considers them all, and the nearest start frame decides.
/// Ties keep the earlier clip, so the answer does not depend on how the set was built.
pub(crate) fn pick(
    clips: &[Clip<'_>],
    lying: &[[f32; 3]],
    chest_normal: [f32; 3],
) -> Option<usize> {
    let want = facing(chest_normal);
    let nearest = |only: Option<Facing>| -> Option<(usize, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, clip) in clips.iter().enumerate() {
            if only.is_some_and(|side| clip.facing != side) {
                continue;
            }
            let miss = miss_m(clip.start, lying);
            if best.is_none_or(|(_, b)| miss < b) {
                best = Some((i, miss));
            }
        }
        best
    };
    let strict = (want != Facing::Side).then_some(want);
    nearest(strict).or_else(|| nearest(None)).map(|(i, _)| i)
}

/// What a landing does with the fall it took.
///
/// A hard landing rolls: the body spends the impact in a roll instead of a flat get-up. A roll that
/// leaves the ground hands the body to the ragdoll, so a landing never stacks a second pose on a
/// body already in the air. Presentation only: nothing here is hashed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LandAnswer {
    /// Slow fall: the feet and the squash take it.
    Absorb,
    /// Fast fall: play the landing roll.
    Roll,
    /// Fast fall whose roll leaves the ground: the ragdoll owns the body.
    Ragdoll,
}

/// Downward speed, meters per second, above which a landing rolls: the ALS
/// `RollingOnLandSpeedThreshold` default (4 m/s).
pub(crate) const LAND_ROLL_SPEED_M_S: f32 = 4.0;

/// What a landing does: a fall above [`LAND_ROLL_SPEED_M_S`] rolls, a slower one absorbs, and a
/// roll that leaves the ground (`grounded_next` false) goes to the ragdoll instead.
pub(crate) fn land_roll(fall_speed_m_s: f32, grounded_next: bool) -> LandAnswer {
    if !fall_speed_m_s.is_finite() || fall_speed_m_s <= LAND_ROLL_SPEED_M_S {
        return LandAnswer::Absorb;
    }
    if grounded_next {
        LandAnswer::Roll
    } else {
        LandAnswer::Ragdoll
    }
}

/// The distance, meters, from the lying body to the clip its get-up would rise into, or `None`
/// when that clip is not a get-up clip (more than [`MISS_MAX_M`] away, or the chest cannot be
/// read at all and the set is empty).
///
/// The rig ships one standing clip today, so the set has one row; a second one joins here and the
/// rule above needs no change.
pub(crate) fn arrival_miss(
    lying: &LocalPose,
    clip_start: &[[f32; 3]],
    stops: Option<[usize; 4]>,
) -> Option<f32> {
    let set = [Clip {
        name: "stand",
        facing: Facing::Up,
        start: clip_start,
    }];
    let normal = stops
        .and_then(|indices| pose_chest_normal(lying, indices))
        .unwrap_or(UP);
    let chosen = pick(&set, lying.positions.as_slice(), normal)?;
    let miss = miss_m(set[chosen].start, lying.positions.as_slice());
    (miss <= MISS_MAX_M).then_some(miss)
}

/// World up, meters per meter: the facing a body with no readable chest is scored as.
pub(crate) const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// The worst joint's second difference across three poses, centimeters: the view law's pop
/// measure, so both get-up paths are judged by one number.
#[cfg(test)]
pub(crate) fn seam_cm(before: &[[f32; 3]], last: &[[f32; 3]], now: &[[f32; 3]]) -> f32 {
    let mut worst = 0.0f32;
    for ((a, b), c) in before.iter().zip(last).zip(now) {
        let d = [
            c[0] - 2.0 * b[0] + a[0],
            c[1] - 2.0 * b[1] + a[1],
            c[2] - 2.0 * b[2] + a[2],
        ];
        worst = worst.max(length(d));
    }
    worst * 100.0
}

/// The frames the physics tape's 60-tick get-up shows (`ragdoll_tape::get_up`): a linear blend
/// from the lying pose to the clip's first frame, weight `1 - (g + 1) / ticks` on each tick.
#[cfg(test)]
pub(crate) fn linear_get_up(
    lying: &[[f32; 3]],
    target: &[[f32; 3]],
    ticks: usize,
) -> Vec<Vec<[f32; 3]>> {
    let window = f32::from(u16::try_from(ticks.max(1)).unwrap_or(u16::MAX));
    (0..ticks)
        .map(|g| {
            let step = f32::from(u16::try_from(g).unwrap_or(u16::MAX));
            let weight = 1.0 - (step + 1.0) / window;
            lying
                .iter()
                .zip(target)
                .map(|(from, to)| {
                    [
                        from[0] + (to[0] - from[0]) * weight,
                        from[1] + (to[1] - from[1]) * weight,
                        from[2] + (to[2] - from[2]) * weight,
                    ]
                })
                .collect()
        })
        .collect()
}

/// The worst seam of a frame list and how many frames broke the pop law (`pop_m`, meters),
/// starting one frame past `seed`.
/// The worst seam of a frame list and how many frames broke the pop law (`pop_m`, meters),
/// measured from the two frames before the list.
#[cfg(test)]
pub(crate) fn seam_report(
    prev2: &[[f32; 3]],
    prev: &[[f32; 3]],
    frames: &[Vec<[f32; 3]>],
    pop_m: f64,
) -> (f32, u32) {
    let (mut worst, mut pops) = (0.0f32, 0u32);
    let mut before = prev2;
    let mut last = prev;
    for now in frames {
        let seam = seam_cm(before, last, now);
        worst = worst.max(seam);
        if f64::from(seam) > pop_m * 100.0 {
            pops += 1;
        }
        before = last;
        last = now;
    }
    (worst, pops)
}

/// The A/B of one fall's get-up: (A worst seam cm, A pops, B worst seam cm, B pops), where A is
/// the 60-tick linear sim-to-animation blend to `target` and B is the inertial path the frames
/// after the stand-up actually took, over `window` ticks. Both are judged by [`seam_cm`].
#[cfg(test)]
pub(crate) fn ab_getup(
    prev2: &[[f32; 3]],
    prev: &[[f32; 3]],
    after: &[Vec<[f32; 3]>],
    target: &[[f32; 3]],
    window: u32,
    pop_m: f64,
) -> (f32, u32, f32, u32) {
    let (a_jump, a_pops) = seam_report(prev2, prev, &linear_get_up(prev, target, 60), pop_m);
    let end = usize::try_from(window)
        .unwrap_or(usize::MAX)
        .min(after.len());
    let (b_jump, b_pops) = seam_report(prev2, prev, &after[..end], pop_m);
    (a_jump, a_pops, b_jump, b_pops)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four joints per pose: pelvis, head, left shoulder, right shoulder. The chest normal is
    /// `cross(across, spine)`, so a body on its back reads `+1` on the up axis and one face down
    /// reads `-1`; a body on a side keeps that normal level.
    fn pose(p: [f32; 3], h: [f32; 3], l: [f32; 3], r: [f32; 3]) -> Vec<[f32; 3]> {
        vec![p, h, l, r]
    }

    /// The four lying poses of the table, each with its own slight offset so the nearest-start
    /// step has something to decide between.
    fn table() -> [(&'static str, Facing, Vec<[f32; 3]>); 4] {
        [
            // On its back: spine along +x, shoulders along z, chest normal up.
            (
                "getup_back",
                Facing::Up,
                pose(
                    [0.0, 0.15, 0.0],
                    [0.8, 0.15, 0.0],
                    [0.0, 0.15, -0.2],
                    [0.0, 0.15, 0.2],
                ),
            ),
            // On its face: the same lie with the shoulders mirrored, chest normal down.
            (
                "getup_front",
                Facing::Down,
                pose(
                    [0.1, 0.12, 0.0],
                    [0.9, 0.12, 0.0],
                    [0.1, 0.12, 0.2],
                    [0.1, 0.12, -0.2],
                ),
            ),
            // On its left side: spine along +z, the shoulder line vertical, chest level.
            (
                "getup_left",
                Facing::Side,
                pose(
                    [0.2, 0.2, 0.0],
                    [0.2, 0.2, 0.8],
                    [0.2, 0.0, 0.0],
                    [0.2, 0.4, 0.0],
                ),
            ),
            // On its right side: head the other way, chest level.
            (
                "getup_right",
                Facing::Side,
                pose(
                    [0.3, 0.2, 0.0],
                    [0.3, 0.2, -0.8],
                    [0.3, 0.0, 0.0],
                    [0.3, 0.4, 0.0],
                ),
            ),
        ]
    }

    #[test]
    fn ragdoll_look_picks_get_up_by_lying_orientation() {
        let table = table();
        let clips: Vec<Clip<'_>> = table
            .iter()
            .map(|(name, facing, start)| Clip {
                name,
                facing: *facing,
                start: start.as_slice(),
            })
            .collect();
        for (i, (name, _, lying)) in table.iter().enumerate() {
            let normal = chest_normal(lying[0], lying[1], lying[2], lying[3])
                .expect("the four stops spread the chest plane");
            let got = pick(&clips, lying, normal).expect("a clip on every side");
            println!(
                "ragdoll_look: lying {name} (chest up {:+.2}) -> {} at {:.2} m",
                normal[1],
                clips[got].name,
                miss_m(clips[got].start, lying)
            );
            assert_eq!(
                got, i,
                "lying {name} must pick {} , not {}",
                name, clips[got].name
            );
        }
    }

    #[test]
    fn ragdoll_look_get_up_pick_refuses_a_broken_chest() {
        // Both shoulders on one point: no chest plane, so no facing.
        assert!(
            chest_normal([0.0; 3], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]).is_none()
        );
        assert_eq!(facing([f32::NAN, 0.0, 0.0]), Facing::Side);
        let blocks = [Clip {
            name: "only",
            facing: Facing::Up,
            start: &[[0.0, 0.0, 0.0]],
        }];
        // No clip on the lying body's side still rises into the one there is.
        assert_eq!(pick(&blocks, &[[1.0, 0.0, 0.0]], [0.0, -1.0, 0.0]), Some(0));
        // A clip a rig away is not a get-up clip.
        let pose = LocalPose {
            positions: vec![[0.0, 0.0, 0.0]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]],
        };
        assert!(arrival_miss(&pose, &[[0.0, 0.0, 0.0]], None).is_some());
        assert!(arrival_miss(&pose, &[[9.0, 0.0, 0.0]], None).is_none());
    }

    /// A fast fall rolls instead of standing up flat.
    #[test]
    fn land_roll_a_fast_fall_rolls() {
        assert_eq!(land_roll(9.0, true), LandAnswer::Roll);
        assert_eq!(
            land_roll(LAND_ROLL_SPEED_M_S + 0.01, true),
            LandAnswer::Roll,
            "just past the threshold still rolls"
        );
    }

    /// A slow fall absorbs: the feet and the squash take it, and the gate is a
    /// strict above, so the threshold itself absorbs.
    #[test]
    fn land_roll_a_slow_fall_absorbs() {
        assert_eq!(land_roll(1.5, true), LandAnswer::Absorb);
        assert_eq!(land_roll(LAND_ROLL_SPEED_M_S, true), LandAnswer::Absorb);
        assert_eq!(land_roll(f32::NAN, true), LandAnswer::Absorb);
    }

    /// A roll that leaves the ground hands the body to the ragdoll.
    #[test]
    fn land_roll_leaving_the_ground_goes_ragdoll() {
        assert_eq!(land_roll(9.0, false), LandAnswer::Ragdoll);
        assert_eq!(land_roll(1.5, false), LandAnswer::Absorb);
    }
}

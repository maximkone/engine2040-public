//! coverage: the rig regions a retarget bone map must drive.
//!
//! Consumer: [`BoneMap::build_checked`](super::BoneMap::build_checked) refuses a
//! map that leaves a required set (root, hips, spine, arms, legs, head)
//! unmapped, and reports the target joints that would fix it. Data owner:
//! `engine-assets`.
//!
//! The name lists are read off the two rigs this crate actually cooks onto, not
//! guessed: the fighter-65 skeleton ([`crate::fighter::fighter_skeleton`],
//! SOMA-style names from [`crate::soma::soma_pairs`]) and the CT7 game body
//! ([`crate::ct7_map::CT7_TO_BODY`], Mixamo-style names). Only a name a real rig
//! carries may enter a list; the `clip_gates_coverage_names_come_from_a_real_rig`
//! test proves it. One mapped name satisfies a set: a rig that drives one arm
//! still has an arms region, it is not half a rig.
//!
//! Determinism: target joint order, string compares only, no allocation until
//! a refusal is built.

use engine_anim::contract::Skeleton;

/// Max target joint names one [`BoneCoverage`] report carries.
pub const MAX_COVERAGE_NAMES: usize = 8;

/// One region a playable rig must drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoneSet {
    /// The rig's root: the parentless joint, whatever its name.
    Root,
    /// The pelvis, which carries the lower body.
    Hips,
    /// The spine chain.
    Spine,
    /// An arm: upper arm, forearm or hand.
    Arms,
    /// A leg: upper leg, lower leg or foot.
    Legs,
    /// The neck or the head.
    Head,
}

impl BoneSet {
    /// Every required set, in report order.
    pub const ALL: [BoneSet; 6] = [
        BoneSet::Root,
        BoneSet::Hips,
        BoneSet::Spine,
        BoneSet::Arms,
        BoneSet::Legs,
        BoneSet::Head,
    ];

    /// Stable snake-case name carried by the refusal.
    pub fn name(self) -> &'static str {
        match self {
            BoneSet::Root => "root",
            BoneSet::Hips => "hips",
            BoneSet::Spine => "spine",
            BoneSet::Arms => "arms",
            BoneSet::Legs => "legs",
            BoneSet::Head => "head",
        }
    }

    /// Target joint names that satisfy this set: the fighter-65 names first,
    /// then the CT7 body names. [`BoneSet::Root`] carries none, because a rig's
    /// root is its parentless joint and its name says nothing about the region.
    pub fn joints(self) -> &'static [&'static str] {
        match self {
            BoneSet::Root => &[],
            BoneSet::Hips => &["hips", "pelvis"],
            BoneSet::Spine => &["spine", "spine1", "spine2", "torso"],
            BoneSet::Arms => &[
                "leftarm",
                "leftforearm",
                "lefthand",
                "rightarm",
                "rightforearm",
                "righthand",
                "arm_upper_l",
                "arm_lower_l",
                "palm_l",
                "arm_upper_r",
                "arm_lower_r",
                "palm_r",
            ],
            BoneSet::Legs => &[
                "leftupleg",
                "leftleg",
                "leftfoot",
                "rightupleg",
                "rightleg",
                "rightfoot",
                "leg_upper_l",
                "leg_lower_l",
                "foot_l",
                "leg_upper_r",
                "leg_lower_r",
                "foot_r",
            ],
            BoneSet::Head => &["neck", "head"],
        }
    }
}

/// What a bone map covers of the required sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoneCoverage<'a> {
    /// Required sets with no mapped joint, in [`BoneSet::ALL`] order.
    pub missing: Vec<BoneSet>,
    /// Members of those sets that the rig carries but the map leaves unmapped,
    /// in target joint order, deduped and capped at [`MAX_COVERAGE_NAMES`]:
    /// the nearest known joints a fixer would map next.
    pub unmapped: Vec<&'a str>,
}

impl BoneCoverage<'_> {
    /// True when every required set has at least one mapped joint.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Measure `target` against the required sets, where `mapped` says whether a
/// target joint carries a source joint.
pub(super) fn measure<'a>(
    target: &'a Skeleton,
    mapped: impl Fn(usize) -> bool,
) -> BoneCoverage<'a> {
    let mut missing: Vec<BoneSet> = Vec::new();
    let mut unmapped: Vec<&'a str> = Vec::new();
    // Root: the first parentless joint; its name is the nearest known fix.
    let root = target
        .joints
        .iter()
        .position(|joint| joint.parent.is_none());
    if let Some(index) = root {
        if !mapped(index) {
            missing.push(BoneSet::Root);
            push_name(&mut unmapped, &target.joints[index].name);
        }
    }
    for set in BoneSet::ALL {
        let names = set.joints();
        if names.is_empty() {
            continue;
        }
        let mut covered = false;
        let mut members: Vec<&'a str> = Vec::new();
        for (index, joint) in target.joints.iter().enumerate() {
            if !names.contains(&super::strip_rig_prefix(&joint.name)) {
                continue;
            }
            if mapped(index) {
                covered = true;
            } else {
                members.push(&joint.name);
            }
        }
        if !covered {
            missing.push(set);
            for name in members {
                push_name(&mut unmapped, name);
            }
        }
    }
    BoneCoverage { missing, unmapped }
}

/// The refusal a bake sees: the missing set names plus the nearest known
/// joints, both bounded.
pub(super) fn refusal(coverage: &BoneCoverage<'_>) -> super::RetargetError {
    super::RetargetError::BoneCoverage {
        sets: coverage
            .missing
            .iter()
            .map(|set| set.name().to_string())
            .collect(),
        joints: coverage
            .unmapped
            .iter()
            .map(|name| super::bound_bone_name(name))
            .collect(),
    }
}

/// Add one name, first occurrence only, up to [`MAX_COVERAGE_NAMES`].
fn push_name<'a>(names: &mut Vec<&'a str>, name: &'a str) {
    if names.len() >= MAX_COVERAGE_NAMES || names.contains(&name) {
        return;
    }
    names.push(name);
}

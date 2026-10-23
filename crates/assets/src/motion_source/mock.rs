//! Offline mock provider: the HumanML3D-22 layout, the authored clip library
//! and the rig helpers the library authors against. No network, no clock.

use super::*;

/// HumanML3D-22: `(joint, parent)` in parent-first order, lowercased.
pub static HML22: [(&str, Option<&str>); 22] = [
    ("pelvis", None),
    ("left_hip", Some("pelvis")),
    ("left_knee", Some("left_hip")),
    ("left_ankle", Some("left_knee")),
    ("right_hip", Some("pelvis")),
    ("right_knee", Some("right_hip")),
    ("right_ankle", Some("right_knee")),
    ("spine1", Some("pelvis")),
    ("spine2", Some("spine1")),
    ("spine3", Some("spine2")),
    ("neck", Some("spine3")),
    ("head", Some("neck")),
    ("left_shoulder", Some("spine3")),
    ("left_elbow", Some("left_shoulder")),
    ("left_wrist", Some("left_elbow")),
    ("left_hand", Some("left_wrist")),
    ("right_shoulder", Some("spine3")),
    ("right_elbow", Some("right_shoulder")),
    ("right_wrist", Some("right_elbow")),
    ("right_hand", Some("right_wrist")),
    ("left_foot", Some("left_ankle")),
    ("right_foot", Some("right_ankle")),
];

/// HumanML3D-22 parent index per row, in [`HML22`] order. Numeric so the doc
/// builder never searches or unwraps: the table is checked by the unit test
/// that imports it and compares names.
static HML22_PARENT: [Option<usize>; 22] = [
    None,
    Some(0),
    Some(1),
    Some(2),
    Some(0),
    Some(4),
    Some(5),
    Some(0),
    Some(7),
    Some(8),
    Some(9),
    Some(10),
    Some(9),
    Some(12),
    Some(13),
    Some(14),
    Some(9),
    Some(16),
    Some(17),
    Some(18),
    Some(3),
    Some(6),
];

/// HumanML3D-22 rest offsets in meters, parent-relative, in [`HML22`] order.
static HML22_REST: [(&str, [f32; 3]); 22] = [
    ("pelvis", [0.00, 0.95, 0.00]),
    ("left_hip", [0.09, -0.07, 0.00]),
    ("left_knee", [0.00, -0.40, 0.00]),
    ("left_ankle", [0.00, -0.41, 0.00]),
    ("right_hip", [-0.09, -0.07, 0.00]),
    ("right_knee", [0.00, -0.40, 0.00]),
    ("right_ankle", [0.00, -0.41, 0.00]),
    ("spine1", [0.00, 0.07, 0.00]),
    ("spine2", [0.00, 0.07, 0.00]),
    ("spine3", [0.00, 0.08, 0.00]),
    ("neck", [0.00, 0.12, 0.00]),
    ("head", [0.00, 0.12, 0.00]),
    ("left_shoulder", [0.08, 0.05, 0.00]),
    ("left_elbow", [0.30, 0.00, 0.00]),
    ("left_wrist", [0.26, 0.00, 0.00]),
    ("left_hand", [0.08, 0.00, 0.00]),
    ("right_shoulder", [-0.08, 0.05, 0.00]),
    ("right_elbow", [-0.30, 0.00, 0.00]),
    ("right_wrist", [-0.26, 0.00, 0.00]),
    ("right_hand", [-0.08, 0.00, 0.00]),
    ("left_foot", [0.00, -0.08, 0.12]),
    ("right_foot", [0.00, -0.08, 0.12]),
];

/// `(fighter joint, HumanML3D-22 joint)` pairs. The two hand tips have no
/// separate engine joint (the fighter hand is the wrist); the fighter clavicles
/// and fingers hold bind. Twenty of the source's 22 joints land.
pub static HML22_TO_RIG: [(&str, &str); 20] = [
    ("hips", "pelvis"),
    ("leftupleg", "left_hip"),
    ("leftleg", "left_knee"),
    ("leftfoot", "left_ankle"),
    ("lefttoebase", "left_foot"),
    ("rightupleg", "right_hip"),
    ("rightleg", "right_knee"),
    ("rightfoot", "right_ankle"),
    ("righttoebase", "right_foot"),
    ("spine", "spine1"),
    ("spine1", "spine2"),
    ("spine2", "spine3"),
    ("neck", "neck"),
    ("head", "head"),
    ("leftarm", "left_shoulder"),
    ("leftforearm", "left_elbow"),
    ("lefthand", "left_wrist"),
    ("rightarm", "right_shoulder"),
    ("rightforearm", "right_elbow"),
    ("righthand", "right_wrist"),
];

/// The HumanML3D-22 skeleton doc, parent-first.
pub fn hml22_skeleton_doc() -> SkeletonDoc {
    let joints = HML22
        .iter()
        .enumerate()
        .map(|(index, (name, _))| JointDoc {
            name: (*name).to_owned(),
            parent: HML22_PARENT[index],
        })
        .collect();
    SkeletonDoc { joints }
}

/// The HumanML3D-22 skeleton, validated through the skin import.
pub fn hml22_skeleton() -> Result<Skeleton, MotionError> {
    crate::skin_import::import_skeleton_doc(&hml22_skeleton_doc()).map_err(MotionError::Import)
}

/// The HumanML3D-22 bind pose: rest offsets plus identity rotations.
pub fn hml22_bind() -> BindPose {
    BindPose {
        positions: HML22_REST.iter().map(|(_, offset)| *offset).collect(),
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; HML22_REST.len()],
    }
}

/// The fighter/source name pairs the bake binds.
pub fn hml22_pairs() -> Vec<(String, String)> {
    HML22_TO_RIG
        .iter()
        .map(|(fighter, source)| ((*fighter).to_owned(), (*source).to_owned()))
        .collect()
}

/// Offline mock source: a small authored clip library, no network, no clock.
#[derive(Clone, Debug)]
pub struct MockMotionSource {
    card: ModelCard,
}

impl MockMotionSource {
    /// A commercial mock model trained on engine-authored clips (CC0-1.0).
    /// The HumanML3D-22 *layout* is public; no HumanML3D data trains this model.
    pub fn commercial() -> Self {
        Self {
            card: mock_card(false),
        }
    }

    /// A mock model trained on AMASS, which is non-commercial. The bake marks
    /// its clips non-commercial; the packaged path refuses them.
    pub fn non_commercial() -> Self {
        Self {
            card: mock_card(true),
        }
    }

    /// The card every generated clip carries.
    pub fn card(&self) -> &ModelCard {
        &self.card
    }
}

impl Default for MockMotionSource {
    fn default() -> Self {
        Self::commercial()
    }
}

impl MotionSource for MockMotionSource {
    fn generate(&self, request: &MotionRequest) -> Result<MotionClip, MotionError> {
        let (doc, tags) = match request.prompt() {
            "sword slash" => (sword_slash_doc(request.seed())?, sword_slash_tags()),
            "idle" => (idle_doc(request.seed())?, idle_tags()),
            other => {
                return Err(MotionError::NoClip {
                    prompt: other.to_owned(),
                })
            }
        };
        let source = hml22_skeleton()?;
        let clip = import_clip_doc(&doc, &source).map_err(MotionError::Import)?;
        Ok(MotionClip {
            provider: "mock".to_owned(),
            source,
            source_bind: hml22_bind(),
            pairs: hml22_pairs(),
            clip,
            tags,
            card: self.card.clone(),
        })
    }
}

fn mock_card(non_commercial: bool) -> ModelCard {
    let (id, weights_label, dataset, license, commercial) = if non_commercial {
        (
            "engine2040/mock-motion-amass",
            b"e2040-mock-motion-weights-amass-v1".as_slice(),
            "AMASS (HumanML3D pose test data)",
            "CC-BY-NC-4.0",
            false,
        )
    } else {
        (
            "engine2040/mock-motion",
            b"e2040-mock-motion-weights-v1".as_slice(),
            "engine2040 authored mock clips",
            "CC0-1.0",
            true,
        )
    };
    ModelCard {
        id: id.to_owned(),
        version: 1,
        weights: content_digest(weights_label),
        weights_license: "Apache-2.0".to_owned(),
        training: vec![TrainingData {
            dataset: dataset.to_owned(),
            license: license.to_owned(),
            commercial,
        }],
    }
}

/// Key times of the authored slash.
const SWORD_TIMES: [f32; 7] = [0.0, 0.18, 0.36, 0.52, 0.70, 0.86, 1.0];
/// Key times of the authored idle loop.
const IDLE_TIMES: [f32; 5] = [0.0, 0.5, 1.0, 1.5, 2.0];

fn sword_slash_doc(seed: u64) -> Result<ClipDoc, MotionError> {
    let mood = seed_mood(seed);
    let mut rotations = empty_rotations();
    slash_sword_arm(&mut rotations, mood)?;
    slash_guard_arm(&mut rotations)?;
    slash_torso(&mut rotations, mood)?;
    Ok(ClipDoc {
        name: "sword_slash".to_owned(),
        translations: hml22_position_tracks(1.0),
        rotations,
    })
}

fn empty_rotations() -> Vec<Vec<RotationKey>> {
    (0..HML22.len()).map(|_| identity_rotation()).collect()
}

fn slash_sword_arm(rotations: &mut [Vec<RotationKey>], mood: f32) -> Result<(), MotionError> {
    set_rotation(
        rotations,
        "right_shoulder",
        &SWORD_TIMES,
        &[
            arm(-70.0 * mood, 10.0),
            arm(-95.0 * mood, 20.0),
            arm(-25.0 * mood, 55.0),
            arm(55.0 * mood, 70.0),
            arm(70.0 * mood, 50.0),
            arm(25.0 * mood, 25.0),
            arm(-15.0 * mood, 10.0),
        ],
    )?;
    set_rotation(
        rotations,
        "right_elbow",
        &SWORD_TIMES,
        &[
            qy(-25.0),
            qy(-35.0),
            qy(0.0),
            qy(45.0 * mood),
            qy(35.0 * mood),
            qy(10.0),
            qy(0.0),
        ],
    )?;
    set_rotation(
        rotations,
        "right_wrist",
        &SWORD_TIMES,
        &[
            qz(-10.0),
            qz(15.0),
            qz(0.0),
            qz(30.0 * mood),
            qz(20.0),
            qz(5.0),
            qz(0.0),
        ],
    )
}

fn slash_guard_arm(rotations: &mut [Vec<RotationKey>]) -> Result<(), MotionError> {
    set_rotation(
        rotations,
        "left_shoulder",
        &SWORD_TIMES,
        &[
            arm(55.0, -25.0),
            arm(60.0, -30.0),
            arm(58.0, -28.0),
            arm(50.0, -20.0),
            arm(48.0, -18.0),
            arm(52.0, -22.0),
            arm(55.0, -25.0),
        ],
    )?;
    set_rotation(
        rotations,
        "left_elbow",
        &SWORD_TIMES,
        &[
            qy(-30.0),
            qy(-35.0),
            qy(-32.0),
            qy(-25.0),
            qy(-25.0),
            qy(-28.0),
            qy(-30.0),
        ],
    )
}

fn slash_torso(rotations: &mut [Vec<RotationKey>], mood: f32) -> Result<(), MotionError> {
    set_spine(
        rotations,
        "spine1",
        &SWORD_TIMES,
        &[0.0, -6.0, -8.0, 8.0, 12.0, 6.0, 0.0],
        1.0,
    )?;
    set_spine(
        rotations,
        "spine2",
        &SWORD_TIMES,
        &[0.0, -10.0, -14.0, 14.0, 20.0, 10.0, 0.0],
        mood,
    )?;
    set_spine(
        rotations,
        "spine3",
        &SWORD_TIMES,
        &[0.0, -12.0, -16.0, 16.0, 22.0, 12.0, 0.0],
        mood,
    )?;
    set_rotation(
        rotations,
        "neck",
        &SWORD_TIMES,
        &[
            qy(0.0),
            qy(8.0),
            qy(10.0),
            qy(-8.0),
            qy(-12.0),
            qy(-6.0),
            qy(0.0),
        ],
    )?;
    set_rotation(
        rotations,
        "head",
        &SWORD_TIMES,
        &[
            qy(0.0),
            qy(6.0),
            qy(8.0),
            qy(-6.0),
            qy(-10.0),
            qy(-5.0),
            qy(0.0),
        ],
    )
}

fn idle_doc(_seed: u64) -> Result<ClipDoc, MotionError> {
    let mut rotations = empty_rotations();
    set_lean(&mut rotations, "spine1", &[0.0, 1.5, 0.0, -1.5, 0.0])?;
    set_lean(&mut rotations, "spine2", &[0.0, 2.0, 0.0, -2.0, 0.0])?;
    set_lean(&mut rotations, "neck", &[0.0, -1.0, 0.0, 1.0, 0.0])?;
    set_spine(
        &mut rotations,
        "left_shoulder",
        &IDLE_TIMES,
        &[0.0, 1.0, 0.0, -1.0, 0.0],
        1.0,
    )?;
    set_spine(
        &mut rotations,
        "right_shoulder",
        &IDLE_TIMES,
        &[0.0, -1.0, 0.0, 1.0, 0.0],
        1.0,
    )?;
    Ok(ClipDoc {
        name: "guard_idle".to_owned(),
        translations: hml22_position_tracks(2.0),
        rotations,
    })
}

fn sword_slash_tags() -> ClipTags {
    ClipTags {
        clip: "sword_slash".to_owned(),
        looped: false,
        root_motion: false,
        tags: vec!["slash".to_owned(), "attack".to_owned()],
        hit_windows: vec![HitWindow {
            socket: "SOCKET_sword".to_owned(),
            start_s: 0.36,
            end_s: 0.62,
        }],
        ..ClipTags::default()
    }
}

fn idle_tags() -> ClipTags {
    ClipTags {
        clip: "guard_idle".to_owned(),
        looped: true,
        root_motion: false,
        tags: vec!["idle".to_owned()],
        ..ClipTags::default()
    }
}

/// Position tracks for every HML22 joint: the bind offset held from zero to
/// `duration`. The mock root does not travel; a sidecar says otherwise only
/// when the provider authors it.
fn hml22_position_tracks(duration: f32) -> Vec<Vec<TranslationKey>> {
    HML22_REST
        .iter()
        .map(|(_, offset)| {
            vec![
                TranslationKey {
                    time: 0.0,
                    value: *offset,
                },
                TranslationKey {
                    time: duration,
                    value: *offset,
                },
            ]
        })
        .collect()
}

fn identity_rotation() -> Vec<RotationKey> {
    vec![RotationKey {
        time: 0.0,
        value: [0.0, 0.0, 0.0, 1.0],
    }]
}

fn set_rotation(
    rotations: &mut [Vec<RotationKey>],
    joint: &str,
    times: &[f32],
    values: &[[f32; 4]],
) -> Result<(), MotionError> {
    debug_assert_eq!(times.len(), values.len(), "authored key lanes match");
    let index = hml22_index(joint).ok_or_else(|| MotionError::BadClip {
        field: "joint",
        detail: format!("authored joint {joint} is not in the HML22 layout"),
    })?;
    rotations[index] = times
        .iter()
        .copied()
        .zip(values.iter().copied())
        .map(|(time, value)| RotationKey { time, value })
        .collect();
    Ok(())
}

fn set_spine(
    rotations: &mut [Vec<RotationKey>],
    joint: &str,
    times: &[f32],
    degrees: &[f32],
    mood: f32,
) -> Result<(), MotionError> {
    let values: Vec<[f32; 4]> = degrees.iter().map(|degree| qy(*degree * mood)).collect();
    set_rotation(rotations, joint, times, &values)
}

fn set_lean(
    rotations: &mut [Vec<RotationKey>],
    joint: &str,
    degrees: &[f32],
) -> Result<(), MotionError> {
    let values: Vec<[f32; 4]> = degrees.iter().map(|degree| qx(*degree)).collect();
    set_rotation(rotations, joint, &IDLE_TIMES, &values)
}

fn hml22_index(name: &str) -> Option<usize> {
    HML22.iter().position(|(joint, _)| *joint == name)
}

fn seed_mood(seed: u64) -> f32 {
    let bucket = u16::try_from(seed % 5).unwrap_or(0);
    1.0 + (f32::from(bucket) - 2.0) * 0.01
}

fn q_axis(axis: [f32; 3], degrees: f32) -> [f32; 4] {
    let half = degrees.to_radians() * 0.5;
    let sin = half.sin();
    [axis[0] * sin, axis[1] * sin, axis[2] * sin, half.cos()]
}

fn qx(degrees: f32) -> [f32; 4] {
    q_axis([1.0, 0.0, 0.0], degrees)
}

fn qy(degrees: f32) -> [f32; 4] {
    q_axis([0.0, 1.0, 0.0], degrees)
}

fn qz(degrees: f32) -> [f32; 4] {
    q_axis([0.0, 0.0, 1.0], degrees)
}

/// Swing the whole arm: lower it about Z, then carry it forward about Y.
fn arm(z_degrees: f32, y_degrees: f32) -> [f32; 4] {
    normalize(quat_mul(qz(z_degrees), qy(y_degrees)))
}

fn normalize(quat: [f32; 4]) -> [f32; 4] {
    let norm =
        (quat[0] * quat[0] + quat[1] * quat[1] + quat[2] * quat[2] + quat[3] * quat[3]).sqrt();
    if !norm.is_finite() || norm <= 0.0 {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        [
            quat[0] / norm,
            quat[1] / norm,
            quat[2] / norm,
            quat[3] / norm,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_parents_match_the_named_layout() {
        for (index, (name, parent)) in HML22.iter().enumerate() {
            let expected =
                parent.and_then(|parent| HML22.iter().position(|(joint, _)| *joint == parent));
            assert_eq!(HML22_PARENT[index], expected, "row {index} {name}");
        }
    }
}

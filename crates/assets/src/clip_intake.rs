//! clip_intake: combat batch retarget plus digest dedupe.
//! Consumer: the clip bake cooks batch clips through this intake.
//! Data owner: engine-assets. Units: meters, seconds, digests as u64.
//! Determinism: input order decides output order, no hash order.
//! Errors: typed IntakeError, never panics.

use super::retarget::{
    retarget_cook, retarget_digest, ClipTags, RetargetError, RetargetPlan, RetargetedClip,
};
use std::fmt;

/// Combat clips covered by one intake run across three batches.
pub const COMBAT_CLIP_COUNT: usize = 48;
/// Clips per batch slice (three slices make the full set).
pub const CLIPS_PER_BATCH: usize = 16;
/// Batch slices in the full combat set.
pub const COMBAT_BATCH_COUNT: usize = 3;
/// Licence accepted by this intake: combat hand-made chain is CC0-1.0 only,
/// narrower than the release allow-list (`provenance::ALLOWED_SPDX`).
pub const EXPECTED_SPDX: &str = "CC0-1.0";
/// Largest batch accepted in one call (mirrors the clip import bound).
pub const MAX_BATCH_CLIPS: usize = 64;

/// Pinned cook digests of the 14 canonical clips every batch dedupes against.
pub const CANONICAL_DIGESTS: [u64; 14] = [
    0x185e70c7034759e2,
    0x3ea8557d0d3aec49,
    0xa2e04e8655f597bb,
    0x9d1cea58b0889137,
    0x143b54df62b7441a,
    0xf76a98cfcf0a9866,
    0x1696e44a049e5f7f,
    0x8f8ffa50539f3657,
    0x4274703f355d99c3,
    0xe4497f7576e75611,
    0x0e6beb775c1f2e22,
    0xa52bcd35da54bd52,
    0xf24f6072ccb699d2,
    0x4f322525c7970901,
];

/// Why a combat intake cook failed.
#[derive(Clone, Debug, PartialEq)]
pub enum IntakeError {
    /// Source licence is not the accepted one; the value is carried.
    License {
        /// Licence string seen on the clip.
        spdx: String,
    },
    /// Cook digest already exists; the digest and clip name are carried.
    Duplicate {
        /// Clip name that collided.
        name: String,
        /// Stable cook digest that matched.
        digest: u64,
    },
    /// Retarget cook refused the clip.
    Retarget(RetargetError),
    /// Batch is larger than the import bound; the length is carried.
    TooLarge {
        /// Offending clip count.
        count: usize,
    },
}

impl fmt::Display for IntakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::License { spdx } => write!(f, "intake bad licence {}", short_text(spdx)),
            Self::Duplicate { name, digest } => {
                write!(
                    f,
                    "intake duplicate {} digest {:016x}",
                    short_text(name),
                    digest
                )
            }
            Self::Retarget(inner) => write!(f, "intake retarget: {inner}"),
            Self::TooLarge { count } => write!(f, "intake too large: {count}"),
        }
    }
}

impl std::error::Error for IntakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Retarget(inner) => Some(inner),
            _ => None,
        }
    }
}

impl From<RetargetError> for IntakeError {
    fn from(inner: RetargetError) -> Self {
        Self::Retarget(inner)
    }
}

/// Keep carried strings short in diagnostics.
fn short_text(text: &str) -> String {
    text.chars().take(64).collect()
}

/// True when the digest matches a pinned canonical digest.
pub fn is_canonical_duplicate(digest: u64) -> bool {
    CANONICAL_DIGESTS.contains(&digest)
}

/// Refuse any licence except the accepted one.
pub fn check_license(spdx: &str) -> Result<(), IntakeError> {
    if spdx == EXPECTED_SPDX {
        Ok(())
    } else {
        Err(IntakeError::License {
            spdx: short_text(spdx),
        })
    }
}

/// Deterministic combat clip names in batch order (three slices of 16).
pub fn combat_clip_names() -> Vec<String> {
    let mut names = Vec::with_capacity(COMBAT_CLIP_COUNT);
    for batch in ["a", "b", "c"] {
        for index in 0..CLIPS_PER_BATCH {
            let mut name = String::with_capacity(16);
            name.push_str("batch_");
            name.push_str(batch);
            name.push('_');
            name.push_str(itoa_two(index));
            names.push(name);
        }
    }
    names
}

/// Two-digit decimal text for values below 100 without formatting machinery.
/// Out-of-range is a bug and panics, never a silent name.
fn itoa_two(value: usize) -> &'static str {
    const TABLE: [&str; 100] = [
        "00", "01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12", "13", "14",
        "15", "16", "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29",
        "30", "31", "32", "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44",
        "45", "46", "47", "48", "49", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59",
        "60", "61", "62", "63", "64", "65", "66", "67", "68", "69", "70", "71", "72", "73", "74",
        "75", "76", "77", "78", "79", "80", "81", "82", "83", "84", "85", "86", "87", "88", "89",
        "90", "91", "92", "93", "94", "95", "96", "97", "98", "99",
    ];
    TABLE[value]
}

/// Cook one combat clip, refuse licence gaps and digest dupes.
///
/// The digest is stable over name, keys, tables and tags. A digest matching
/// a canonical digest or an earlier clip in `seen` is refused with the digest
/// named. On success the digest is appended to `seen` in input order.
pub fn intake_cook(
    plan: &RetargetPlan<'_>,
    tags: ClipTags,
    spdx: &str,
    seen: &mut Vec<u64>,
) -> Result<(RetargetedClip, u64), IntakeError> {
    check_license(spdx)?;
    let cooked = retarget_cook(plan, tags)?;
    let digest = retarget_digest(&cooked);
    if is_canonical_duplicate(digest) || seen.contains(&digest) {
        return Err(IntakeError::Duplicate {
            name: short_text(&cooked.clip.name),
            digest,
        });
    }
    seen.push(digest);
    Ok((cooked, digest))
}

/// One batch entry: cook inputs plus the licence the clip arrived with.
#[derive(Clone, Debug)]
pub struct BatchClip<'a> {
    /// Retarget inputs for one source clip.
    pub plan: RetargetPlan<'a>,
    /// Sidecar tags attached to this cook.
    pub tags: ClipTags,
    /// Licence string for this clip.
    pub spdx: &'a str,
}

/// Cook one batch slice in order, return one digest per clip.
///
/// The same slice cooked twice yields the same digests in the same order.
/// Any licence gap, retarget failure or duplicate stops the batch with the
/// offending clip named.
pub fn cook_batch(clips: &[BatchClip<'_>], seen: &mut Vec<u64>) -> Result<Vec<u64>, IntakeError> {
    if clips.len() > MAX_BATCH_CLIPS {
        return Err(IntakeError::TooLarge { count: clips.len() });
    }
    let mut digests = Vec::with_capacity(clips.len());
    for entry in clips {
        let (_, digest) = intake_cook(&entry.plan, entry.tags.clone(), entry.spdx, seen)?;
        digests.push(digest);
    }
    Ok(digests)
}

#[cfg(test)]
mod tests {
    use super::super::retarget::{BindPose, BoneMap, ClipTags, HitWindow, RetargetPlan};
    use super::super::skin_import::{
        import_clip_doc, import_skeleton_doc, ClipDoc, JointDoc, RotationKey, SkeletonDoc,
        TranslationKey,
    };
    use super::*;
    use engine_anim::contract::Skeleton;

    const JOINTS: usize = 15;
    const ROOT: usize = 0;
    const SPINE: usize = 1;
    const FOOT_L: usize = 11;
    const FOOT_R: usize = 14;

    fn source_names() -> [&'static str; JOINTS] {
        [
            "root",
            "spine",
            "head",
            "upperarm_l",
            "lowerarm_l",
            "hand_l",
            "upperarm_r",
            "lowerarm_r",
            "hand_r",
            "thigh_l",
            "shin_l",
            "foot_l",
            "thigh_r",
            "shin_r",
            "foot_r",
        ]
    }

    fn target_names() -> [&'static str; JOINTS] {
        [
            "root",
            "spine",
            "head",
            "upperarm_l",
            "lowerarm_l",
            "palm_l",
            "upperarm_r",
            "lowerarm_r",
            "palm_r",
            "thigh_l",
            "shin_l",
            "foot_l",
            "thigh_r",
            "shin_r",
            "foot_r",
        ]
    }

    fn parents() -> [Option<usize>; JOINTS] {
        [
            None,
            Some(0),
            Some(1),
            Some(1),
            Some(3),
            Some(4),
            Some(1),
            Some(6),
            Some(7),
            Some(0),
            Some(9),
            Some(10),
            Some(0),
            Some(12),
            Some(13),
        ]
    }

    fn source_offsets() -> [[f32; 3]; JOINTS] {
        [
            [0.0, 0.96, 0.0],
            [0.0, 0.10, 0.0],
            [0.0, 0.24, 0.0],
            [0.20, 0.06, 0.0],
            [0.26, 0.0, 0.0],
            [0.24, 0.0, 0.0],
            [-0.20, 0.06, 0.0],
            [-0.26, 0.0, 0.0],
            [-0.24, 0.0, 0.0],
            [0.10, -0.06, 0.0],
            [0.0, -0.44, 0.0],
            [0.0, -0.44, 0.06],
            [-0.10, -0.06, 0.0],
            [0.0, -0.44, 0.0],
            [0.0, -0.44, 0.06],
        ]
    }

    fn target_offsets() -> [[f32; 3]; JOINTS] {
        let mut offsets = source_offsets();
        for joint in [3, 4, 5, 6, 7, 8] {
            for component in &mut offsets[joint] {
                *component *= 1.05;
            }
        }
        for joint in [9, 10, 11, 12, 13, 14] {
            for component in &mut offsets[joint] {
                *component *= 1.1;
            }
        }
        offsets
    }

    fn skeleton_fixture(names: [&str; JOINTS]) -> Skeleton {
        let order = parents();
        import_skeleton_doc(&SkeletonDoc {
            joints: names
                .iter()
                .enumerate()
                .map(|(index, name)| JointDoc {
                    name: (*name).to_string(),
                    parent: order[index],
                })
                .collect(),
        })
        .expect("synthetic skeleton must import")
    }

    fn rig() -> (Skeleton, Skeleton, BindPose, BindPose, BoneMap) {
        let source = skeleton_fixture(source_names());
        let target = skeleton_fixture(target_names());
        let source_bind = BindPose {
            positions: source_offsets().to_vec(),
            rotations: [[0.0, 0.0, 0.0, 1.0]; JOINTS].to_vec(),
        };
        let mut target_rots = [[0.0, 0.0, 0.0, 1.0]; JOINTS];
        target_rots[4] = [0.0, 0.0, 0.0872, 0.9962];
        let target_bind = BindPose {
            positions: target_offsets().to_vec(),
            rotations: target_rots.to_vec(),
        };
        let pairs: Vec<(String, String)> = target_names()
            .iter()
            .zip(source_names().iter())
            .map(|(t, s)| ((*t).to_string(), (*s).to_string()))
            .collect();
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(t, s)| (t.as_str(), s.as_str()))
            .collect();
        let map = BoneMap::build(&source, &target, &borrowed).expect("full map must build");
        (source, target, source_bind, target_bind, map)
    }

    fn combat_doc(name: &str, slot: usize) -> ClipDoc {
        let offsets = source_offsets();
        let slot_u8 = u8::try_from(slot).expect("combat slot below 256");
        let travel = 0.4 + f32::from(slot_u8) * 0.02;
        let lift = 0.10 + f32::from(slot_u8) * 0.002;
        let foot_l = offsets[FOOT_L];
        let foot_r = offsets[FOOT_R];
        let lifted_l = [foot_l[0], foot_l[1] + lift, foot_l[2] + 0.08];
        let lifted_r = [foot_r[0], foot_r[1] + lift, foot_r[2] + 0.08];
        ClipDoc {
            name: name.to_string(),
            translations: (0..JOINTS)
                .map(|joint| {
                    if joint == ROOT {
                        vec![
                            TranslationKey {
                                time: 0.0,
                                value: offsets[ROOT],
                            },
                            TranslationKey {
                                time: 0.4,
                                value: [travel, 0.96, 0.0],
                            },
                            TranslationKey {
                                time: 0.8,
                                value: [travel * 2.0, 0.96, 0.0],
                            },
                        ]
                    } else if joint == FOOT_L {
                        vec![
                            TranslationKey {
                                time: 0.0,
                                value: foot_l,
                            },
                            TranslationKey {
                                time: 0.4,
                                value: lifted_l,
                            },
                            TranslationKey {
                                time: 0.8,
                                value: foot_l,
                            },
                        ]
                    } else if joint == FOOT_R {
                        vec![
                            TranslationKey {
                                time: 0.0,
                                value: foot_r,
                            },
                            TranslationKey {
                                time: 0.4,
                                value: lifted_r,
                            },
                            TranslationKey {
                                time: 0.8,
                                value: foot_r,
                            },
                        ]
                    } else if joint == SPINE {
                        vec![
                            TranslationKey {
                                time: 0.0,
                                value: offsets[joint],
                            },
                            TranslationKey {
                                time: 0.4,
                                value: [offsets[joint][0], offsets[joint][1] + 0.01, 0.0],
                            },
                            TranslationKey {
                                time: 0.8,
                                value: offsets[joint],
                            },
                        ]
                    } else {
                        vec![TranslationKey {
                            time: 0.0,
                            value: offsets[joint],
                        }]
                    }
                })
                .collect(),
            rotations: (0..JOINTS)
                .map(|_| {
                    vec![RotationKey {
                        time: 0.0,
                        value: [0.0, 0.0, 0.0, 1.0],
                    }]
                })
                .collect(),
        }
    }

    fn combat_tags() -> ClipTags {
        ClipTags {
            clip: String::new(),
            looped: false,
            root_motion: false,
            tags: vec!["attack".to_string(), "melee".to_string()],
            ground_y: Some(0.02),
            feet: Some(("foot_l".to_string(), "foot_r".to_string())),
            hit_windows: vec![HitWindow {
                socket: "SOCKET_wep_R".to_string(),
                start_s: 0.1,
                end_s: 0.25,
            }],
            traversal: Vec::new(),
        }
    }

    fn cook_config() -> super::super::skin_import::CookConfig {
        super::super::skin_import::CookConfig {
            root_joint: ROOT,
            left_foot: FOOT_L,
            right_foot: FOOT_R,
            ground_y: -0.38,
            start_tick: 100,
        }
    }

    #[test]
    fn combat_batch_retargets_and_dedupes_by_digest() {
        let names = combat_clip_names();
        assert_eq!(names.len(), COMBAT_CLIP_COUNT);
        assert_eq!(names.len(), COMBAT_BATCH_COUNT * CLIPS_PER_BATCH);
        let mut ordered = names.clone();
        ordered.sort();
        ordered.dedup();
        assert_eq!(ordered.len(), COMBAT_CLIP_COUNT);

        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let mut seen: Vec<u64> = Vec::new();
        let mut first_digests: Vec<u64> = Vec::with_capacity(names.len());
        for (slot, name) in names.iter().enumerate() {
            let doc = combat_doc(name, slot);
            let clip = import_clip_doc(&doc, &source).expect("fixture clip must import");
            let plan = RetargetPlan {
                source_clip: &clip,
                source: &source,
                source_bind: &source_bind,
                target: &target,
                target_bind: &target_bind,
                map: &map,
                config: &config,
            };
            let (cooked, digest) = intake_cook(&plan, combat_tags(), EXPECTED_SPDX, &mut seen)
                .expect("cook must pass");
            assert_eq!(cooked.clip.name, *name);
            assert!(!is_canonical_duplicate(digest));
            first_digests.push(digest);
        }
        assert_eq!(seen.len(), COMBAT_CLIP_COUNT);
        let mut ranked = seen.clone();
        ranked.sort_unstable();
        ranked.dedup();
        assert_eq!(ranked.len(), COMBAT_CLIP_COUNT);

        let mut again: Vec<u64> = Vec::new();
        for (slot, name) in names.iter().enumerate() {
            let doc = combat_doc(name, slot);
            let clip = import_clip_doc(&doc, &source).expect("fixture clip must import");
            let plan = RetargetPlan {
                source_clip: &clip,
                source: &source,
                source_bind: &source_bind,
                target: &target,
                target_bind: &target_bind,
                map: &map,
                config: &config,
            };
            let (_, digest) = intake_cook(&plan, combat_tags(), EXPECTED_SPDX, &mut again)
                .expect("recook must pass");
            assert_eq!(digest, first_digests[slot]);
        }

        let doc = combat_doc(&names[0], 0);
        let clip = import_clip_doc(&doc, &source).expect("fixture clip must import");
        let plan = RetargetPlan {
            source_clip: &clip,
            source: &source,
            source_bind: &source_bind,
            target: &target,
            target_bind: &target_bind,
            map: &map,
            config: &config,
        };
        let err = intake_cook(&plan, combat_tags(), EXPECTED_SPDX, &mut seen)
            .expect_err("dupe must fail");
        match err {
            IntakeError::Duplicate { digest, .. } => {
                assert_eq!(digest, first_digests[0]);
                assert!(err.to_string().contains(&format!("{digest:016x}")));
            }
            other => panic!("wrong error {other:?}"),
        }

        let bad = intake_cook(&plan, combat_tags(), "MIT", &mut Vec::new());
        assert!(matches!(bad, Err(IntakeError::License { .. })));
    }

    #[test]
    fn canonical_set_holds_fourteen_pinned_digests() {
        assert_eq!(CANONICAL_DIGESTS.len(), 14);
        let mut ranked = CANONICAL_DIGESTS.to_vec();
        ranked.sort_unstable();
        ranked.dedup();
        assert_eq!(ranked.len(), 14);
        for digest in CANONICAL_DIGESTS {
            assert!(is_canonical_duplicate(digest));
        }
        assert!(!is_canonical_duplicate(0));
    }

    #[test]
    fn expected_spdx_stays_on_release_allow_list() {
        assert!(
            super::super::provenance::ALLOWED_SPDX.contains(&EXPECTED_SPDX),
            "intake licence must stay on the release allow-list"
        );
    }

    #[test]
    fn batch_cook_keeps_input_order_and_names_digest() {
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let names = combat_clip_names();
        let mut entries: Vec<BatchClip<'_>> = Vec::new();
        let mut owned: Vec<super::super::skin_import::ImportedClip> = Vec::new();
        for (slot, name) in names.iter().take(4).enumerate() {
            let clip = import_clip_doc(&combat_doc(name, slot), &source).expect("clip must import");
            owned.push(clip);
        }
        for clip in &owned {
            entries.push(BatchClip {
                plan: RetargetPlan {
                    source_clip: clip,
                    source: &source,
                    source_bind: &source_bind,
                    target: &target,
                    target_bind: &target_bind,
                    map: &map,
                    config: &config,
                },
                tags: combat_tags(),
                spdx: EXPECTED_SPDX,
            });
        }
        let mut seen = Vec::new();
        let digests = cook_batch(&entries, &mut seen).expect("batch must cook");
        assert_eq!(digests.len(), 4);
        assert_eq!(seen, digests);
        let too_many = vec![entries[0].clone(); MAX_BATCH_CLIPS + 1];
        assert!(matches!(
            cook_batch(&too_many, &mut Vec::new()),
            Err(IntakeError::TooLarge { .. })
        ));
    }
}

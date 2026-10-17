//! traversal_cook: traversal and finisher intake with edge markers.
//! Consumer: `super::retarget::bake_traversal_batch` cooks traversal clips
//! through this intake; tables ride `skin_import::cook_clip_tables` via
//! `retarget_cook`.
//! Data owner: engine-assets. Units: meters, seconds, digests as u64.
//! Determinism: input order decides output order, no hash order.
//! Errors: typed TraversalError, never panics.

use super::clip_intake::CANONICAL_DIGESTS;
use super::retarget::{
    parse_sidecar, retarget_cook_with_sidecar, retarget_digest, RetargetError, RetargetPlan,
    RetargetedClip,
};
use std::fmt;

/// Traversal and finisher clips covered by one intake run.
pub const TRAVERSAL_CLIP_COUNT: usize = 18;
/// Largest batch accepted in one call (mirrors the clip import bound).
pub const MAX_BATCH_CLIPS: usize = 64;
/// Licence accepted by this intake: hand-made traversal chain is CC0-1.0 only,
/// narrower than the release allow-list (`provenance::ALLOWED_SPDX`).
pub const EXPECTED_SPDX: &str = "CC0-1.0";

/// One sidecar audit finding: which input failed and why.
#[derive(Clone, Debug, PartialEq)]
pub struct SidecarFinding {
    /// Index into the audited sidecar slice.
    pub index: usize,
    /// Truncated parser message.
    pub detail: String,
}

/// Why a traversal intake cook failed.
#[derive(Clone, Debug, PartialEq)]
pub enum TraversalError {
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
    /// Retarget cook refused the clip; the clip name plus the inner error.
    Retarget {
        /// Clip name that failed.
        clip: String,
        /// Inner retarget failure (unknown foot names surface as MissingBone).
        inner: RetargetError,
    },
    /// Batch is larger than the import bound; the length is carried.
    TooLarge {
        /// Offending clip count.
        count: usize,
    },
    /// Batch slices disagree in length; the three counts are carried.
    LengthMismatch {
        /// Number of retarget plans.
        plans: usize,
        /// Number of sidecar docs.
        sidecars: usize,
        /// Number of licence strings.
        spdx: usize,
    },
    /// Cook carried no traversal edge marker; the clip name is carried.
    MissingMarker {
        /// Clip name without a marker.
        clip: String,
    },
}

impl fmt::Display for TraversalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::License { spdx } => write!(f, "traversal bad licence {}", short_text(spdx)),
            Self::Duplicate { name, digest } => {
                write!(
                    f,
                    "traversal duplicate {} digest {:016x}",
                    short_text(name),
                    digest
                )
            }
            Self::Retarget { clip, inner } => {
                write!(f, "traversal {} retarget: {inner}", short_text(clip))
            }
            Self::TooLarge { count } => write!(f, "traversal too large: {count}"),
            Self::LengthMismatch {
                plans,
                sidecars,
                spdx,
            } => write!(
                f,
                "traversal length mismatch: plans {plans} sidecars {sidecars} spdx {spdx}"
            ),
            Self::MissingMarker { clip } => {
                write!(f, "traversal {} missing marker", short_text(clip))
            }
        }
    }
}

impl std::error::Error for TraversalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Retarget { inner, .. } => Some(inner),
            _ => None,
        }
    }
}

/// Keep carried strings short in diagnostics.
fn short_text(text: &str) -> String {
    text.chars().take(64).collect()
}

/// Saturating slot byte without panics: values past 255 read as 255.
fn slot_byte(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(u8::MAX)
}

/// True when the digest matches a pinned canonical digest.
pub fn is_canonical_duplicate(digest: u64) -> bool {
    CANONICAL_DIGESTS.contains(&digest)
}

/// Refuse any licence except the accepted one.
pub fn check_license(spdx: &str) -> Result<(), TraversalError> {
    if spdx == EXPECTED_SPDX {
        Ok(())
    } else {
        Err(TraversalError::License {
            spdx: short_text(spdx),
        })
    }
}

/// Deterministic traversal and finisher clip names in bake order.
///
/// Three vault, three hurdle, three mantle, three climb, three drop and three
/// finisher clips. The prefix before `_` is the edge marker each clip carries.
pub fn traversal_clip_names() -> Vec<String> {
    vec![
        "vault_00".to_string(),
        "vault_01".to_string(),
        "vault_02".to_string(),
        "hurdle_00".to_string(),
        "hurdle_01".to_string(),
        "hurdle_02".to_string(),
        "mantle_00".to_string(),
        "mantle_01".to_string(),
        "mantle_02".to_string(),
        "climb_00".to_string(),
        "climb_01".to_string(),
        "climb_02".to_string(),
        "drop_00".to_string(),
        "drop_01".to_string(),
        "drop_02".to_string(),
        "finisher_00".to_string(),
        "finisher_01".to_string(),
        "finisher_02".to_string(),
    ]
}

/// Edge marker for a clip name: the prefix before the first `_`.
pub fn edge_for_clip(name: &str) -> &str {
    match name.split('_').next() {
        Some(edge) => edge,
        None => name,
    }
}

/// True when the clip is a finisher rather than a traversal edge.
pub fn is_finisher(name: &str) -> bool {
    edge_for_clip(name) == "finisher"
}

/// Build the sidecar JSON doc for one traversal clip.
///
/// Traversal edges ride `root_motion` as locomotion metadata; finishers carry
/// one combat hit window plus their closing marker. The schema matches the
/// canonical `attack_1h_1` sidecar plus one `traversal` entry.
pub fn traversal_sidecar(name: &str, slot: usize) -> String {
    let edge = edge_for_clip(name);
    let time = 0.2 + f32::from(slot_byte(slot)) * 0.01;
    let root_motion = !is_finisher(name);
    let (tags, windows) = if is_finisher(name) {
        (
            "\"finisher\", \"melee\"".to_string(),
            ", \"hit_windows\": [{\"socket\": \"SOCKET_wep_R\", \"start_s\": 0.1, \"end_s\": 0.25}]"
                .to_string(),
        )
    } else {
        (format!("\"traversal\", \"{edge}\""), String::new())
    };
    format!(
        "{{\"clip\": \"{name}\", \"loop\": false, \"root_motion\": {root_motion}, \"tags\": [{tags}], \"ground_y\": 0.02, \"feet\": {{\"left\": \"foot_l\", \"right\": \"foot_r\"}}{windows}, \"traversal\": [{{\"edge\": \"{edge}\", \"time_s\": {time:.2}}}]}}"
    )
}

/// Audit sidecar docs: one finding per doc that refuses to parse.
///
/// Clean intake sidecars audit to zero findings; the detail names the parser
/// refusal truncated for diagnostics.
pub fn audit_sidecars(sidecars: &[&str]) -> Vec<SidecarFinding> {
    let mut findings = Vec::new();
    for (index, doc) in sidecars.iter().enumerate() {
        if let Err(error) = parse_sidecar(doc) {
            findings.push(SidecarFinding {
                index,
                detail: short_text(&error.to_string()),
            });
        }
    }
    findings
}

/// Cook one traversal clip, refuse licence gaps, missing markers and dupes.
///
/// The sidecar `ground_y`/`feet` overrides resolve through
/// `retarget_cook_with_sidecar` (unknown feet fail `MissingBone` with the clip
/// named); `loop`/`root_motion` ride along. The digest is stable over name,
/// keys, tables and tags. A digest matching a canonical digest or an earlier
/// clip in `seen` is refused with the digest named. On success the digest is
/// appended to `seen` in input order.
pub fn intake_cook(
    plan: &RetargetPlan<'_>,
    sidecar: &str,
    spdx: &str,
    seen: &mut Vec<u64>,
) -> Result<(RetargetedClip, u64), TraversalError> {
    check_license(spdx)?;
    let clip_name = short_text(&plan.source_clip.name);
    let cooked =
        retarget_cook_with_sidecar(plan, sidecar).map_err(|inner| TraversalError::Retarget {
            clip: clip_name.clone(),
            inner,
        })?;
    if cooked.tags.traversal.is_empty() {
        return Err(TraversalError::MissingMarker { clip: clip_name });
    }
    let digest = retarget_digest(&cooked);
    if is_canonical_duplicate(digest) || seen.contains(&digest) {
        return Err(TraversalError::Duplicate {
            name: short_text(&cooked.clip.name),
            digest,
        });
    }
    seen.push(digest);
    Ok((cooked, digest))
}

/// Cook one traversal batch in order, return one digest per clip.
///
/// The same batch cooked twice yields the same digests in the same order.
/// Any licence gap, retarget failure, missing marker or duplicate stops the
/// batch with the offending clip named.
pub fn cook_traversal_batch(
    plans: &[RetargetPlan<'_>],
    sidecars: &[&str],
    spdx: &[&str],
    seen: &mut Vec<u64>,
) -> Result<Vec<u64>, TraversalError> {
    let widest = plans.len().max(sidecars.len()).max(spdx.len());
    if widest > MAX_BATCH_CLIPS {
        return Err(TraversalError::TooLarge { count: widest });
    }
    if plans.len() != sidecars.len() || plans.len() != spdx.len() {
        return Err(TraversalError::LengthMismatch {
            plans: plans.len(),
            sidecars: sidecars.len(),
            spdx: spdx.len(),
        });
    }
    let mut digests = Vec::with_capacity(plans.len());
    for index in 0..plans.len() {
        let (_, digest) = intake_cook(&plans[index], sidecars[index], spdx[index], seen)?;
        digests.push(digest);
    }
    Ok(digests)
}

#[cfg(test)]
mod tests {
    use super::super::retarget::{BindPose, BoneMap, RetargetError, RetargetPlan};
    use super::super::skin_import::{
        cook_clip_tables, import_clip_doc, import_skeleton_doc, ClipDoc, JointDoc, RotationKey,
        SkeletonDoc, TranslationKey,
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

    fn traversal_doc(name: &str, slot: usize) -> ClipDoc {
        let offsets = source_offsets();
        let slot_u8 = slot_byte(slot);
        let travel = 0.5 + f32::from(slot_u8) * 0.03;
        let lift = 0.08 + f32::from(slot_u8) * 0.003;
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
    fn traversal_clips_carry_markers_and_sidecars() {
        let names = traversal_clip_names();
        assert_eq!(names.len(), TRAVERSAL_CLIP_COUNT);
        let mut ordered = names.clone();
        ordered.sort();
        ordered.dedup();
        assert_eq!(ordered.len(), TRAVERSAL_CLIP_COUNT);
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let mut seen: Vec<u64> = Vec::new();
        let mut sidecars: Vec<String> = Vec::with_capacity(names.len());
        let mut first: Vec<u64> = Vec::with_capacity(names.len());
        for (slot, name) in names.iter().enumerate() {
            let doc = traversal_doc(name, slot);
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
            let sidecar = traversal_sidecar(name, slot);
            let (cooked, digest) = intake_cook(&plan, &sidecar, EXPECTED_SPDX, &mut seen)
                .expect("traversal cook must pass");
            assert!(!cooked.tags.traversal.is_empty(), "{name} carries a marker");
            assert_eq!(cooked.tags.traversal[0].edge, edge_for_clip(name));
            assert_eq!(cooked.tags.root_motion, !is_finisher(name));
            assert!(!is_canonical_duplicate(digest));
            assert!(cook_clip_tables(&cooked.clip, &target, plan.config).is_ok());
            sidecars.push(sidecar);
            first.push(digest);
        }
        assert_eq!(seen.len(), TRAVERSAL_CLIP_COUNT);
        let borrowed: Vec<&str> = sidecars.iter().map(String::as_str).collect();
        assert!(audit_sidecars(&borrowed).is_empty(), "0 sidecar findings");
        let mut again: Vec<u64> = Vec::new();
        for (slot, name) in names.iter().enumerate() {
            let doc = traversal_doc(name, slot);
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
            let (_, digest) = intake_cook(&plan, borrowed[slot], EXPECTED_SPDX, &mut again)
                .expect("recook must pass");
            assert_eq!(digest, first[slot]);
        }
        let doc = traversal_doc(&names[0], 0);
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
        let bad = intake_cook(
            &plan,
            r#"{"feet": {"left": "toe_l", "right": "foot_r"}, "traversal": [{"edge": "vault", "time_s": 0.2}]}"#,
            EXPECTED_SPDX,
            &mut Vec::new(),
        )
        .expect_err("bad foot must fail");
        match &bad {
            TraversalError::Retarget { clip, inner } => {
                assert_eq!(
                    inner,
                    &RetargetError::MissingBone {
                        name: "toe_l".to_string()
                    }
                );
                assert!(bad.to_string().contains(&names[0]), "names {clip}");
            }
            other => panic!("wrong error {other:?}"),
        }
    }

    #[test]
    fn traversal_names_hold_eighteen_edges() {
        let names = traversal_clip_names();
        assert_eq!(names.len(), 18);
        for name in &names {
            assert!(valid_edge(edge_for_clip(name)), "{name} edge is a token");
        }
        assert!(names.iter().any(|name| is_finisher(name)));
        assert!(names.iter().any(|name| edge_for_clip(name) == "vault"));
        assert!(names.iter().any(|name| edge_for_clip(name) == "climb"));
    }

    fn valid_edge(edge: &str) -> bool {
        !edge.is_empty()
            && edge.len() <= 64
            && edge
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    }

    #[test]
    fn expected_spdx_stays_on_release_allow_list() {
        assert!(
            super::super::provenance::ALLOWED_SPDX.contains(&EXPECTED_SPDX),
            "intake licence must stay on the release allow-list"
        );
    }

    #[test]
    fn missing_marker_is_refused_naming_the_clip() {
        let names = traversal_clip_names();
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let doc = traversal_doc(&names[0], 0);
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
        let err = intake_cook(&plan, "{}", EXPECTED_SPDX, &mut Vec::new())
            .expect_err("markerless cook must fail");
        assert!(matches!(err, TraversalError::MissingMarker { .. }));
        assert!(err.to_string().contains(&names[0]));
    }

    #[test]
    fn batch_cook_keeps_input_order() {
        let names = traversal_clip_names();
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let mut owned = Vec::new();
        for (slot, name) in names.iter().take(4).enumerate() {
            owned.push(import_clip_doc(&traversal_doc(name, slot), &source).expect("clip imports"));
        }
        let plans: Vec<RetargetPlan<'_>> = owned
            .iter()
            .map(|clip| RetargetPlan {
                source_clip: clip,
                source: &source,
                source_bind: &source_bind,
                target: &target,
                target_bind: &target_bind,
                map: &map,
                config: &config,
            })
            .collect();
        let sidecars: Vec<String> = names
            .iter()
            .take(4)
            .enumerate()
            .map(|(s, n)| traversal_sidecar(n, s))
            .collect();
        let borrowed: Vec<&str> = sidecars.iter().map(String::as_str).collect();
        let spdx = [EXPECTED_SPDX; 4];
        let mut seen = Vec::new();
        let digests =
            cook_traversal_batch(&plans, &borrowed, &spdx, &mut seen).expect("batch cooks");
        assert_eq!(digests.len(), 4);
        assert_eq!(seen, digests);
        let too_many = vec![plans[0]; MAX_BATCH_CLIPS + 1];
        let many_sides = vec!["{}"; MAX_BATCH_CLIPS + 1];
        let many_spdx = vec![EXPECTED_SPDX; MAX_BATCH_CLIPS + 1];
        assert!(matches!(
            cook_traversal_batch(&too_many, &many_sides, &many_spdx, &mut Vec::new()),
            Err(TraversalError::TooLarge { .. })
        ));
        let err = cook_traversal_batch(&plans[..1], &borrowed, &spdx, &mut Vec::new())
            .expect_err("mismatched slices must fail");
        assert!(
            matches!(err, TraversalError::LengthMismatch { .. }),
            "mismatch uses LengthMismatch, got {err:?}"
        );
        assert!(err.to_string().contains("length mismatch"));
    }
}

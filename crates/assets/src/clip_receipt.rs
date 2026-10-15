//! clip_receipt: 80-clip Tier A receipt check plus cook-twice gate.
//! Consumer: the CT23 intake bake reconciles canonical, combat and
//! traversal cooks through this check before locking the set.
//! Data owner: engine-assets. Units: digests as u64, seconds, keys.
//! Determinism: slice order decides which gap is named, no hash order.
//! Errors: typed ReceiptError, never panics.

use super::clip_intake::{CANONICAL_DIGESTS, COMBAT_CLIP_COUNT, EXPECTED_SPDX};
use super::provenance::{check_release, ProvenanceRecord};
use super::retarget::RetargetedClip;
use super::traversal_cook::TRAVERSAL_CLIP_COUNT;
use std::fmt;

/// Canonical clips every intake batch dedupes against.
pub const CANONICAL_CLIP_COUNT: usize = 14;
/// Full Tier A set: canonical plus combat plus traversal.
pub const TOTAL_CLIP_COUNT: usize = CANONICAL_CLIP_COUNT + COMBAT_CLIP_COUNT + TRAVERSAL_CLIP_COUNT;
/// Largest key total a receipt row may carry; the measured family tops at
/// 862 keys (death), so 1024 bounds it with headroom.
pub const MAX_CLIP_KEYS: usize = 1024;
/// Longest clip a receipt row may carry in seconds; the measured family
/// tops at 0.833 s (death), so 2.0 s bounds it with headroom.
pub const MAX_CLIP_SECS: f32 = 2.0;

/// One receipt row: the digest plus the stats the budget gate reads.
#[derive(Clone, Debug, PartialEq)]
pub struct ReceiptRow {
    /// Clip name in bake order.
    pub name: String,
    /// Stable cook digest over name, keys, tables and tags.
    pub digest: u64,
    /// Licence string the clip arrived with.
    pub spdx: String,
    /// Total key count across position and rotation tracks.
    pub keys: usize,
    /// Clip duration in seconds.
    pub dur_s: f32,
}

/// Why an 80-clip receipt was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum ReceiptError {
    /// A section holds the wrong row count; section plus counts carried.
    Count {
        /// Section that missed its count (`canonical`, `combat`, ...).
        section: String,
        /// Rows the section must hold.
        expected: usize,
        /// Rows the section held.
        seen: usize,
    },
    /// A cook digest repeats; the clip name plus the digest carried.
    Duplicate {
        /// Clip name that collided.
        name: String,
        /// Stable cook digest that matched.
        digest: u64,
    },
    /// A row carries the wrong licence; the value is carried.
    License {
        /// Licence string seen on the row.
        spdx: String,
    },
    /// A row breaks the key or duration budget; clip plus why carried.
    Budget {
        /// Clip that broke the budget.
        clip: String,
        /// Which bound broke and the value seen.
        detail: String,
    },
    /// The provenance set has a gap; the detail names it.
    Provenance {
        /// First gap named by the release gate.
        detail: String,
    },
    /// A digest disagrees with its earlier cook; CT23 names the set.
    Tampered {
        /// Which row drifted between the two cooks.
        detail: String,
    },
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Count {
                section,
                expected,
                seen,
            } => write!(
                f,
                "receipt {section} count: expected {expected}, seen {seen}"
            ),
            Self::Duplicate { name, digest } => {
                write!(
                    f,
                    "receipt duplicate {} digest {:016x}",
                    short_text(name),
                    digest
                )
            }
            Self::License { spdx } => write!(f, "receipt bad licence {}", short_text(spdx)),
            Self::Budget { clip, detail } => {
                write!(f, "receipt {} budget: {}", short_text(clip), detail)
            }
            Self::Provenance { detail } => write!(f, "receipt provenance: {}", detail),
            Self::Tampered { detail } => write!(f, "CT23 tampered: {}", detail),
        }
    }
}

impl std::error::Error for ReceiptError {}

/// Keep carried strings short in diagnostics.
fn short_text(text: &str) -> String {
    text.chars().take(64).collect()
}

/// Build a receipt row from a cooked clip, counting track keys.
pub fn row_from_cooked(name: &str, spdx: &str, cooked: &RetargetedClip, digest: u64) -> ReceiptRow {
    let mut keys: usize = 0;
    for track in &cooked.clip.positions.tracks {
        keys += track.times.len();
    }
    for track in &cooked.clip.rotations {
        keys += track.times.len();
    }
    ReceiptRow {
        name: name.to_string(),
        digest,
        spdx: spdx.to_string(),
        keys,
        dur_s: cooked.clip.duration,
    }
}

/// Section row counts must read 14 canonical, 48 combat, 18 traversal.
pub fn check_counts(
    canonical: &[ReceiptRow],
    combat: &[ReceiptRow],
    traversal: &[ReceiptRow],
) -> Result<(), ReceiptError> {
    for (section, rows, expected) in [
        ("canonical", canonical, CANONICAL_CLIP_COUNT),
        ("combat", combat, COMBAT_CLIP_COUNT),
        ("traversal", traversal, TRAVERSAL_CLIP_COUNT),
    ] {
        if rows.len() != expected {
            return Err(ReceiptError::Count {
                section: section.to_string(),
                expected,
                seen: rows.len(),
            });
        }
    }
    Ok(())
}

/// Canonical digests must match the pinned set in order; any drift is a
/// tampered CT23 set, never a silent re-pin.
pub fn check_canonical(canonical: &[ReceiptRow]) -> Result<(), ReceiptError> {
    for (index, row) in canonical.iter().enumerate() {
        let pinned = CANONICAL_DIGESTS.get(index).copied().unwrap_or(0);
        if row.digest != pinned {
            return Err(ReceiptError::Tampered {
                detail: format!("canonical row {index} drifted"),
            });
        }
    }
    if canonical.len() != CANONICAL_DIGESTS.len() {
        return Err(ReceiptError::Count {
            section: "canonical".to_string(),
            expected: CANONICAL_DIGESTS.len(),
            seen: canonical.len(),
        });
    }
    Ok(())
}

/// All 80 digests must be pairwise distinct; the repeat names its digest.
pub fn check_unique(
    canonical: &[ReceiptRow],
    combat: &[ReceiptRow],
    traversal: &[ReceiptRow],
) -> Result<(), ReceiptError> {
    let mut seen: Vec<u64> = Vec::with_capacity(TOTAL_CLIP_COUNT);
    for row in canonical
        .iter()
        .chain(combat.iter())
        .chain(traversal.iter())
    {
        if seen.contains(&row.digest) {
            return Err(ReceiptError::Duplicate {
                name: short_text(&row.name),
                digest: row.digest,
            });
        }
        seen.push(row.digest);
    }
    Ok(())
}

/// Every row must carry the accepted licence.
pub fn check_licenses(
    canonical: &[ReceiptRow],
    combat: &[ReceiptRow],
    traversal: &[ReceiptRow],
) -> Result<(), ReceiptError> {
    for row in canonical
        .iter()
        .chain(combat.iter())
        .chain(traversal.iter())
    {
        if row.spdx != EXPECTED_SPDX {
            return Err(ReceiptError::License {
                spdx: short_text(&row.spdx),
            });
        }
    }
    Ok(())
}

/// Every row must fit the key and duration budgets.
pub fn check_budgets(
    canonical: &[ReceiptRow],
    combat: &[ReceiptRow],
    traversal: &[ReceiptRow],
) -> Result<(), ReceiptError> {
    for row in canonical
        .iter()
        .chain(combat.iter())
        .chain(traversal.iter())
    {
        if row.keys == 0 || row.keys > MAX_CLIP_KEYS {
            return Err(ReceiptError::Budget {
                clip: short_text(&row.name),
                detail: format!("keys {} over {MAX_CLIP_KEYS}", row.keys),
            });
        }
        if !row.dur_s.is_finite() || row.dur_s <= 0.0 || row.dur_s > MAX_CLIP_SECS {
            return Err(ReceiptError::Budget {
                clip: short_text(&row.name),
                detail: format!("dur {} over {MAX_CLIP_SECS}s", row.dur_s),
            });
        }
    }
    Ok(())
}

/// The provenance set must be present and green; an empty set is a gap,
/// never a vacuous pass.
pub fn check_provenance(records: &[ProvenanceRecord]) -> Result<(), ReceiptError> {
    if records.is_empty() {
        return Err(ReceiptError::Provenance {
            detail: "CT23 receipt needs provenance rows".to_string(),
        });
    }
    check_release(records).map_err(|error| ReceiptError::Provenance {
        detail: short_text(&error.to_string()),
    })
}

/// Two cooks of the same 80 rows must agree digest for digest; any
/// disagreement names CT23.
pub fn verify_cook_x2(first: &[u64], second: &[u64]) -> Result<(), ReceiptError> {
    if first.len() != TOTAL_CLIP_COUNT {
        return Err(ReceiptError::Count {
            section: "cook-x2 first".to_string(),
            expected: TOTAL_CLIP_COUNT,
            seen: first.len(),
        });
    }
    if second.len() != TOTAL_CLIP_COUNT {
        return Err(ReceiptError::Count {
            section: "cook-x2 second".to_string(),
            expected: TOTAL_CLIP_COUNT,
            seen: second.len(),
        });
    }
    for (index, (a, b)) in first.iter().zip(second.iter()).enumerate() {
        if a != b {
            return Err(ReceiptError::Tampered {
                detail: format!("cook-x2 row {index} drifted"),
            });
        }
    }
    Ok(())
}

/// Full receipt gate: counts, pinned canonical set, pairwise digests,
/// licences, budgets, provenance, then the cook-twice agreement over the
/// assembled 80 digests. Returns the 80 digests in bake order.
pub fn check_receipt(
    canonical: &[ReceiptRow],
    combat: &[ReceiptRow],
    traversal: &[ReceiptRow],
    recooked: &[u64],
    records: &[ProvenanceRecord],
) -> Result<Vec<u64>, ReceiptError> {
    check_counts(canonical, combat, traversal)?;
    check_canonical(canonical)?;
    check_unique(canonical, combat, traversal)?;
    check_licenses(canonical, combat, traversal)?;
    check_budgets(canonical, combat, traversal)?;
    check_provenance(records)?;
    let mut digests = Vec::with_capacity(TOTAL_CLIP_COUNT);
    for row in canonical
        .iter()
        .chain(combat.iter())
        .chain(traversal.iter())
    {
        digests.push(row.digest);
    }
    verify_cook_x2(&digests, recooked)?;
    Ok(digests)
}

#[cfg(test)]
mod tests {
    use super::super::clip_intake::{combat_clip_names, intake_cook as combat_cook};
    use super::super::provenance::{ProvenanceRecord, ALLOWED_SPDX};
    use super::super::retarget::{BindPose, BoneMap, ClipTags, HitWindow, RetargetPlan};
    use super::super::skin_import::{
        import_clip_doc, import_skeleton_doc, ClipDoc, CookConfig, JointDoc, RotationKey,
        SkeletonDoc, TranslationKey,
    };
    use super::super::traversal_cook::{
        intake_cook as traversal_cook, traversal_clip_names, traversal_sidecar,
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

    /// Saturating slot byte without panics: values past 255 read as 255.
    fn slot_byte(slot: usize) -> u8 {
        match u8::try_from(slot) {
            Ok(value) => value,
            Err(_) => u8::MAX,
        }
    }

    fn fixture_doc(name: &str, slot: usize, travel_base: f32, lift_base: f32) -> ClipDoc {
        let offsets = source_offsets();
        let slot_f = f32::from(slot_byte(slot));
        let travel = travel_base + slot_f * 0.02;
        let lift = lift_base + slot_f * 0.002;
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

    fn cook_config() -> CookConfig {
        CookConfig {
            root_joint: ROOT,
            left_foot: FOOT_L,
            right_foot: FOOT_R,
            ground_y: -0.38,
            start_tick: 100,
        }
    }

    /// Pinned canonical rows: shorts, cook digests, key counts and durations
    /// copied from `content/fighters/DEDUPE-MANIFEST.md`, never invented.
    fn canonical_rows() -> Vec<ReceiptRow> {
        const SHORTS: [&str; CANONICAL_CLIP_COUNT] = [
            "idle",
            "run",
            "jump",
            "death",
            "idle_combat_1h",
            "idle_combat_unarmed",
            "attack_1h_1",
            "attack_1h_2",
            "attack_1h_3",
            "attack_2h_1",
            "attack_2h_2",
            "block_shield",
            "attack_dual_1",
            "taunt",
        ];
        const KEYS: [usize; CANONICAL_CLIP_COUNT] = [
            819, 819, 819, 862, 819, 819, 432, 432, 432, 432, 432, 432, 432, 432,
        ];
        const DURS: [f32; CANONICAL_CLIP_COUNT] = [
            0.792, 0.792, 0.792, 0.833, 0.792, 0.792, 0.417, 0.417, 0.417, 0.417, 0.417, 0.417,
            0.417, 0.417,
        ];
        SHORTS
            .iter()
            .zip(CANONICAL_DIGESTS.iter())
            .zip(KEYS.iter())
            .zip(DURS.iter())
            .map(|(((short, digest), keys), dur)| ReceiptRow {
                name: (*short).to_string(),
                digest: *digest,
                spdx: EXPECTED_SPDX.to_string(),
                keys: *keys,
                dur_s: *dur,
            })
            .collect()
    }

    fn cook_combat_rows(seen: &mut Vec<u64>) -> Vec<ReceiptRow> {
        let names = combat_clip_names();
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let mut rows = Vec::with_capacity(names.len());
        for (slot, name) in names.iter().enumerate() {
            let clip = import_clip_doc(&fixture_doc(name, slot, 0.4, 0.10), &source)
                .expect("fixture clip must import");
            let plan = RetargetPlan {
                source_clip: &clip,
                source: &source,
                source_bind: &source_bind,
                target: &target,
                target_bind: &target_bind,
                map: &map,
                config: &config,
            };
            let (cooked, digest) = combat_cook(&plan, combat_tags(), EXPECTED_SPDX, seen)
                .expect("combat cook must pass");
            rows.push(row_from_cooked(name, EXPECTED_SPDX, &cooked, digest));
        }
        rows
    }

    fn cook_traversal_rows(seen: &mut Vec<u64>) -> Vec<ReceiptRow> {
        let names = traversal_clip_names();
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let mut rows = Vec::with_capacity(names.len());
        for (slot, name) in names.iter().enumerate() {
            let clip = import_clip_doc(&fixture_doc(name, slot, 0.5, 0.08), &source)
                .expect("fixture clip must import");
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
            let (cooked, digest) = traversal_cook(&plan, &sidecar, EXPECTED_SPDX, seen)
                .expect("traversal cook must pass");
            rows.push(row_from_cooked(name, EXPECTED_SPDX, &cooked, digest));
        }
        rows
    }

    /// Hand-made source chain for the three receipt sections: the real
    /// upstream pack, licence, author and bytes pinned by the manifest.
    fn provenance_rows() -> Vec<ProvenanceRecord> {
        const SETS: [&str; 3] = [
            "content/fighters/canonical-14",
            "content/fighters/combat-48",
            "content/fighters/traversal-18",
        ];
        SETS.iter()
            .map(|name| ProvenanceRecord {
                name: (*name).to_string(),
                url: "https://opengameart.org/content/modular-rpg-characters".to_string(),
                spdx: EXPECTED_SPDX.to_string(),
                author: "System G6".to_string(),
                sha: "ef53003c6b29798a634fb27a3f9fc3181a2c02b217689744f2944fc5fcc0fd99".to_string(),
                generator: None,
                model: None,
                prompt_digest: None,
            })
            .collect()
    }

    fn full_set() -> (
        Vec<ReceiptRow>,
        Vec<ReceiptRow>,
        Vec<ReceiptRow>,
        Vec<u64>,
        Vec<ProvenanceRecord>,
    ) {
        let canonical = canonical_rows();
        let mut seen: Vec<u64> = canonical.iter().map(|row| row.digest).collect();
        let combat = cook_combat_rows(&mut seen);
        let traversal = cook_traversal_rows(&mut seen);
        let mut second: Vec<u64> = canonical.iter().map(|row| row.digest).collect();
        let mut fresh: Vec<u64> = canonical.iter().map(|row| row.digest).collect();
        for row in cook_combat_rows(&mut fresh) {
            second.push(row.digest);
        }
        for row in cook_traversal_rows(&mut fresh) {
            second.push(row.digest);
        }
        (canonical, combat, traversal, second, provenance_rows())
    }

    #[test]
    fn clip_80_receipt_counts_and_cooks_x2() {
        let (canonical, combat, traversal, second, records) = full_set();
        assert_eq!(canonical.len(), CANONICAL_CLIP_COUNT);
        assert_eq!(combat.len(), COMBAT_CLIP_COUNT);
        assert_eq!(traversal.len(), TRAVERSAL_CLIP_COUNT);
        let checked = check_receipt(&canonical, &combat, &traversal, &second, &records)
            .expect("receipt must pass");
        assert_eq!(checked.len(), TOTAL_CLIP_COUNT);
        assert_eq!(second.len(), TOTAL_CLIP_COUNT);
        let mut ranked = checked.clone();
        ranked.sort_unstable();
        ranked.dedup();
        assert_eq!(ranked.len(), TOTAL_CLIP_COUNT);
        for row in combat.iter().chain(traversal.iter()) {
            assert!(row.keys > 0 && row.keys <= MAX_CLIP_KEYS);
            assert!(row.dur_s > 0.0 && row.dur_s <= MAX_CLIP_SECS);
        }
    }

    #[test]
    fn totals_sum_to_eighty() {
        assert_eq!(CANONICAL_CLIP_COUNT, 14);
        assert_eq!(TOTAL_CLIP_COUNT, 80);
        assert_eq!(
            TOTAL_CLIP_COUNT,
            CANONICAL_CLIP_COUNT + COMBAT_CLIP_COUNT + TRAVERSAL_CLIP_COUNT
        );
    }

    #[test]
    fn canonical_count_matches_pinned_digests() {
        assert_eq!(CANONICAL_CLIP_COUNT, CANONICAL_DIGESTS.len());
        let mut ranked = CANONICAL_DIGESTS.to_vec();
        ranked.sort_unstable();
        ranked.dedup();
        assert_eq!(ranked.len(), CANONICAL_CLIP_COUNT);
    }

    #[test]
    fn expected_spdx_stays_on_release_allow_list() {
        assert!(
            ALLOWED_SPDX.contains(&EXPECTED_SPDX),
            "receipt licence must stay on the release allow-list"
        );
    }

    #[test]
    fn tampered_cook_x2_names_ct23() {
        let (canonical, combat, traversal, mut second, records) = full_set();
        let last = second.len() - 1;
        second[last] ^= 1;
        let err = check_receipt(&canonical, &combat, &traversal, &second, &records)
            .expect_err("tampered recook must fail");
        assert!(matches!(err, ReceiptError::Tampered { .. }));
        assert!(err.to_string().contains("CT23"));
    }

    #[test]
    fn drifted_canonical_names_ct23() {
        let (mut canonical, combat, traversal, second, records) = full_set();
        canonical[0].digest ^= 1;
        let err = check_receipt(&canonical, &combat, &traversal, &second, &records)
            .expect_err("drifted canonical must fail");
        assert!(matches!(err, ReceiptError::Tampered { .. }));
        assert!(err.to_string().contains("CT23"));
    }

    #[test]
    fn duplicate_digest_names_digest() {
        let (canonical, combat, mut traversal, second, records) = full_set();
        traversal[0].digest = combat[0].digest;
        let err = check_receipt(&canonical, &combat, &traversal, &second, &records)
            .expect_err("dupe digest must fail");
        match err {
            ReceiptError::Duplicate { digest, .. } => {
                assert_eq!(digest, combat[0].digest);
                assert!(err.to_string().contains(&format!("{digest:016x}")));
            }
            other => panic!("wrong error {other:?}"),
        }
    }

    #[test]
    fn missing_provenance_is_refused() {
        let (canonical, combat, traversal, second, _) = full_set();
        let err = check_receipt(&canonical, &combat, &traversal, &second, &[])
            .expect_err("empty provenance must fail");
        assert!(matches!(err, ReceiptError::Provenance { .. }));
    }

    #[test]
    fn bad_provenance_licence_is_refused() {
        let (canonical, combat, traversal, second, mut records) = full_set();
        records[0].spdx = "GPL-3.0-only".to_string();
        let err = check_receipt(&canonical, &combat, &traversal, &second, &records)
            .expect_err("off-list licence must fail");
        assert!(matches!(err, ReceiptError::Provenance { .. }));
    }

    #[test]
    fn short_section_is_refused() {
        let (canonical, combat, traversal, second, records) = full_set();
        let err = check_receipt(
            &canonical,
            &combat[..COMBAT_CLIP_COUNT - 1],
            &traversal,
            &second,
            &records,
        )
        .expect_err("short combat must fail");
        match err {
            ReceiptError::Count { expected, seen, .. } => {
                assert_eq!(expected, COMBAT_CLIP_COUNT);
                assert_eq!(seen, COMBAT_CLIP_COUNT - 1);
            }
            other => panic!("wrong error {other:?}"),
        }
    }

    #[test]
    fn over_budget_row_is_refused() {
        let (canonical, mut combat, traversal, second, records) = full_set();
        combat[0].keys = MAX_CLIP_KEYS + 1;
        let err = check_receipt(&canonical, &combat, &traversal, &second, &records)
            .expect_err("over-budget row must fail");
        assert!(matches!(err, ReceiptError::Budget { .. }));
    }

    #[test]
    fn wrong_row_licence_is_refused() {
        let (canonical, mut combat, traversal, second, records) = full_set();
        combat[0].spdx = "MIT".to_string();
        let err = check_receipt(&canonical, &combat, &traversal, &second, &records)
            .expect_err("wrong row licence must fail");
        assert!(matches!(err, ReceiptError::License { .. }));
    }
}

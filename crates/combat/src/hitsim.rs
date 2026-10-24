//! hitsim: what one blade does to a body over many clean hits, as a
//! distribution an author can tune against ("this blade cuts a bare forearm
//! 31 times in 100 clean hits").
//!
//! Consumer: the door's `hit.simulate` verb (break pack) and balance tests;
//! the match step never calls it. Data owner: `engine-combat`. Units: Q16
//! (`1.0 == 65536`) for energy, sharpness, wear, protection and odds; counts
//! for samples. Determinism scope: every number is pure integer math over
//! the anatomy row, the wound table and the spec; each sample reuses the
//! cut resolver's own odds (`effective_q16`, `chance_q16`) and its seeded
//! roll (`roll_q16`), so the report equals what [`cut_one`](crate::cut::cut_one)
//! would decide for a touching edge, and the same spec gives the same report
//! on every PC. "Clean" means the edge touches the joint: reach and
//! geometry are not part of the question. Errors: [`HitSimError`](crate::hitsim::HitSimError) for
//! out-of-range specs. Budget: `O(samples * 15)` integer ops, at most
//! [`MAX_SAMPLES`](crate::hitsim::MAX_SAMPLES) samples, one small allocation-free report.

use crate::anatomy::{AnatomyTable, JointId, ARMOR_ALL, SEVERABLE_JOINT_COUNT};
use crate::cut::{
    chance_q16, effective_q16, roll_q16, saturate_q16, BladeSwing, CUT_V1, MAX_ENERGY_Q16,
    MAX_SHARPNESS_Q16, MAX_WEAR_Q16,
};
use crate::limbloss::WoundTable;
use engine_core::Fnv64;

/// Most samples one simulation runs.
pub const MAX_SAMPLES: u32 = 100_000;

/// What to simulate: one blade state, one worn set, `samples` clean hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitSpec {
    /// Swing energy in Q16, `0..=MAX_ENERGY_Q16`.
    pub energy_q16: u32,
    /// Edge sharpness in Q16, `0..=MAX_SHARPNESS_Q16`.
    pub sharpness_q16: u32,
    /// Dulled fraction in Q16, `0..=MAX_WEAR_Q16`.
    pub wear_q16: u32,
    /// Worn armor bits (`ARMOR_*`); unknown bits are refused.
    pub worn: u8,
    /// Clean hits per joint, `1..=MAX_SAMPLES`.
    pub samples: u32,
    /// Seed folded into every sample's roll.
    pub seed: u64,
}

/// Why a spec was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitSimError {
    /// Energy past [`MAX_ENERGY_Q16`]; the culprit is carried.
    BadEnergy(u32),
    /// Sharpness past [`MAX_SHARPNESS_Q16`]; the culprit is carried.
    BadSharpness(u32),
    /// Wear past [`MAX_WEAR_Q16`]; the culprit is carried.
    BadWear(u32),
    /// Worn bits outside `ARMOR_ALL`; the culprit is carried.
    BadArmor(u8),
    /// Samples zero or past [`MAX_SAMPLES`]; the culprit is carried.
    BadSamples(u32),
}

impl core::fmt::Display for HitSimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadEnergy(v) => write!(f, "energy {v} is past {MAX_ENERGY_Q16}"),
            Self::BadSharpness(v) => write!(f, "sharpness {v} is past {MAX_SHARPNESS_Q16}"),
            Self::BadWear(v) => write!(f, "wear {v} is past {MAX_WEAR_Q16}"),
            Self::BadArmor(v) => write!(f, "worn armor bits {v:#x} are outside {ARMOR_ALL:#x}"),
            Self::BadSamples(v) => write!(f, "samples {v} is outside 1..={MAX_SAMPLES}"),
        }
    }
}

impl std::error::Error for HitSimError {}

impl HitSpec {
    /// Range-checks the spec.
    pub fn validate(&self) -> Result<(), HitSimError> {
        if self.energy_q16 > MAX_ENERGY_Q16 {
            return Err(HitSimError::BadEnergy(self.energy_q16));
        }
        if self.sharpness_q16 > MAX_SHARPNESS_Q16 {
            return Err(HitSimError::BadSharpness(self.sharpness_q16));
        }
        if self.wear_q16 > MAX_WEAR_Q16 {
            return Err(HitSimError::BadWear(self.wear_q16));
        }
        if self.worn & !ARMOR_ALL != 0 {
            return Err(HitSimError::BadArmor(self.worn));
        }
        if self.samples == 0 || self.samples > MAX_SAMPLES {
            return Err(HitSimError::BadSamples(self.samples));
        }
        Ok(())
    }
}

/// One severable joint's row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JointOutcome {
    /// Joint wire value.
    pub joint: u8,
    /// Resistance of the joint under the worn set, Q16.
    pub protection_q16: u64,
    /// Sever odds in Q16 (`65536` is certain).
    pub odds_q16: u32,
    /// Samples that severed.
    pub severed: u32,
    /// Ticks one open stump here needs to bleed a whole body out.
    pub bleed_out_ticks: u32,
}

/// The whole distribution: one row per severable joint in joint order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitReport {
    /// Samples run per joint.
    pub samples: u32,
    /// Rows in canonical joint order, pelvis excluded.
    pub rows: [JointOutcome; SEVERABLE_JOINT_COUNT],
    /// FNV-1a over the spec and every row: the receipt a replay compares.
    pub digest: u64,
}

/// Runs `spec.samples` clean hits against every severable joint.
pub fn simulate(
    anatomy: &AnatomyTable,
    wound: &WoundTable,
    spec: &HitSpec,
) -> Result<HitReport, HitSimError> {
    spec.validate()?;
    let mut rows = [JointOutcome {
        joint: 0,
        protection_q16: 0,
        odds_q16: 0,
        severed: 0,
        bleed_out_ticks: 0,
    }; SEVERABLE_JOINT_COUNT];
    let (attacker, victim) = slots_of(spec.seed);
    let swing_base = u32::try_from((spec.seed >> 16) & 0xFFFF_FFFF).unwrap_or(0);
    let mut at = 0usize;
    for joint in JointId::all() {
        if !joint.is_severable() {
            continue;
        }
        let blade = BladeSwing {
            attacker,
            victim,
            swing: 0,
            p0_mm: [0; 3],
            p1_mm: [0; 3],
            blade_half_mm: 0,
            energy_q16: spec.energy_q16,
            sharpness_q16: spec.sharpness_q16,
            wear_q16: spec.wear_q16,
        };
        let protection = anatomy.protection_q16(joint, spec.worn);
        let odds = chance_q16(effective_q16(&blade), protection, CUT_V1.sever_span_q16);
        let mut severed = 0u32;
        for sample in 0..spec.samples {
            let rolled = BladeSwing {
                swing: swing_base.wrapping_add(sample),
                ..blade
            };
            if roll_q16(&rolled, joint) < odds {
                severed += 1;
            }
        }
        rows[at] = JointOutcome {
            joint: joint.as_u8(),
            protection_q16: protection,
            odds_q16: odds,
            severed,
            bleed_out_ticks: wound.bleed_out_of(wound.stump_class[joint.index()]),
        };
        at += 1;
    }
    Ok(HitReport {
        samples: spec.samples,
        rows,
        digest: report_digest(spec, &rows),
    })
}

/// Distinct attacker and victim slots from the seed's low bytes; the roll
/// reads both, so different seeds draw different streams.
fn slots_of(seed: u64) -> (u8, u8) {
    let attacker = u8::try_from(seed & 0xFF).unwrap_or(0);
    let victim = u8::try_from((seed >> 8) & 0xFF).unwrap_or(1);
    if attacker == victim {
        (attacker, victim.wrapping_add(1))
    } else {
        (attacker, victim)
    }
}

fn report_digest(spec: &HitSpec, rows: &[JointOutcome; SEVERABLE_JOINT_COUNT]) -> u64 {
    let mut h = Fnv64::new();
    h.write_u32(spec.energy_q16);
    h.write_u32(spec.sharpness_q16);
    h.write_u32(spec.wear_q16);
    h.write_u8(spec.worn);
    h.write_u32(spec.samples);
    h.write_u64(spec.seed);
    for row in rows {
        h.write_u8(row.joint);
        h.write_u32(saturate_q16(row.protection_q16));
        h.write_u32(row.odds_q16);
        h.write_u32(row.severed);
        h.write_u32(row.bleed_out_ticks);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anatomy::{ARMOR_MANICA, ARMOR_SCUTUM, JOINT_TOTAL};

    fn spec(energy: u32, worn: u8, seed: u64) -> HitSpec {
        HitSpec {
            energy_q16: energy,
            sharpness_q16: 90_000,
            wear_q16: 0,
            worn,
            samples: 4_000,
            seed,
        }
    }

    fn rows(energy: u32, worn: u8) -> HitReport {
        simulate(
            &AnatomyTable::rome_kit(),
            &WoundTable::v1(),
            &spec(energy, worn, 7),
        )
        .expect("simulates")
    }

    #[test]
    fn the_distribution_follows_the_resolver_odds() {
        let report = rows(7 << 16, ARMOR_MANICA);
        assert_eq!(report.rows.len(), SEVERABLE_JOINT_COUNT);
        let mut mixed = 0;
        for row in report.rows {
            let expected = u64::from(row.odds_q16) * u64::from(report.samples) / 65_536;
            let got = u64::from(row.severed);
            // A seeded roll over 4,000 samples stays within 3 percent of the odds.
            assert!(
                got.abs_diff(expected) <= u64::from(report.samples) * 3 / 100,
                "joint {}: severed {got} vs odds-implied {expected}",
                row.joint
            );
            if row.odds_q16 > 0 && row.odds_q16 < 65_536 {
                mixed += 1;
            }
        }
        assert!(
            mixed >= 1,
            "the fixture energy lands in the odds ramp for some joint"
        );
    }

    #[test]
    fn armor_lowers_the_odds_and_energy_raises_them() {
        let bare = rows(9 << 16, 0);
        let worn = rows(9 << 16, ARMOR_MANICA | ARMOR_SCUTUM);
        let wrist_r = JointId::WristR.index() - 1;
        let elbow_l = JointId::ElbowL.index() - 1;
        assert!(worn.rows[wrist_r].odds_q16 <= bare.rows[wrist_r].odds_q16);
        assert!(worn.rows[elbow_l].odds_q16 <= bare.rows[elbow_l].odds_q16);
        assert!(worn.rows[wrist_r].protection_q16 > bare.rows[wrist_r].protection_q16);
        // A joint the worn set does not cover is unchanged.
        let hip = JointId::HipL.index() - 1;
        assert_eq!(worn.rows[hip], bare.rows[hip]);
        let weak = rows(8 << 16, 0);
        let strong = rows(26 << 16, 0);
        for k in 0..SEVERABLE_JOINT_COUNT {
            assert!(
                strong.rows[k].odds_q16 >= weak.rows[k].odds_q16,
                "joint {k}"
            );
        }
    }

    #[test]
    fn rows_carry_the_bleed_out_table_and_replay_equal() {
        let a = rows(20 << 16, 0);
        let b = rows(20 << 16, 0);
        assert_eq!(a, b);
        assert_eq!(a.digest, b.digest);
        let other = simulate(
            &AnatomyTable::rome_kit(),
            &WoundTable::v1(),
            &spec(20 << 16, 0, 8),
        )
        .expect("simulates");
        assert_ne!(a.digest, other.digest, "another seed, another stream");
        let wound = WoundTable::v1();
        let neck = a.rows[JointId::Neck.index() - 1];
        assert_eq!(neck.joint, JointId::Neck.as_u8());
        assert_eq!(neck.bleed_out_ticks, wound.bleed_out_ticks[2]);
        let elbow = a.rows[JointId::ElbowR.index() - 1];
        assert_eq!(elbow.bleed_out_ticks, wound.bleed_out_ticks[1]);
        assert_eq!(JOINT_TOTAL, SEVERABLE_JOINT_COUNT + 1);
    }

    #[test]
    fn bad_specs_refuse_with_the_culprit() {
        let anatomy = AnatomyTable::rome_kit();
        let wound = WoundTable::v1();
        let good = spec(20 << 16, 0, 1);
        let cases = [
            (
                HitSpec {
                    energy_q16: MAX_ENERGY_Q16 + 1,
                    ..good
                },
                HitSimError::BadEnergy(MAX_ENERGY_Q16 + 1),
            ),
            (
                HitSpec {
                    sharpness_q16: MAX_SHARPNESS_Q16 + 1,
                    ..good
                },
                HitSimError::BadSharpness(MAX_SHARPNESS_Q16 + 1),
            ),
            (
                HitSpec {
                    wear_q16: MAX_WEAR_Q16 + 1,
                    ..good
                },
                HitSimError::BadWear(MAX_WEAR_Q16 + 1),
            ),
            (HitSpec { worn: 0x80, ..good }, HitSimError::BadArmor(0x80)),
            (HitSpec { samples: 0, ..good }, HitSimError::BadSamples(0)),
            (
                HitSpec {
                    samples: MAX_SAMPLES + 1,
                    ..good
                },
                HitSimError::BadSamples(MAX_SAMPLES + 1),
            ),
        ];
        for (bad, want) in cases {
            assert_eq!(simulate(&anatomy, &wound, &bad).unwrap_err(), want);
            assert!(!want.to_string().is_empty());
        }
    }

    #[test]
    fn a_full_run_stays_inside_the_sample_budget() {
        let big = HitSpec {
            samples: MAX_SAMPLES,
            ..spec(20 << 16, 0, 3)
        };
        let report =
            simulate(&AnatomyTable::rome_kit(), &WoundTable::v1(), &big).expect("simulates");
        assert_eq!(report.samples, MAX_SAMPLES);
        assert!(report.rows.iter().all(|r| r.severed <= MAX_SAMPLES));
    }
}

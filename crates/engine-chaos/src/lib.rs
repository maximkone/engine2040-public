//! Thin A9 chaos facade over the proof suite.
//!
//! Owner: this crate owns nothing but the A9 row's healed/quarantined
//! counts; `engine-proof` owns fault injection (`chaos::run`), the observe
//! catalog owns heal records, the fixture owns tape truth. There is no
//! second chaos system here: every count below comes from one
//! `engine-proof` run at the recorded A9 seed. Units: counts (`usize`).
//! Determinism: same seed always gives the same counts on every machine.

use engine_observe::EventCode;

/// Recorded A9 seed (mirrors `engine-proof` `a9::A9_SEED`).
pub const A9_SEED: u64 = 9;

/// Healed fault codes at the recorded seed, in fault-table order.
#[must_use]
pub fn healed_codes() -> Vec<(EventCode, u32)> {
    engine_proof::chaos::run(A9_SEED).healed
}

/// Quarantined fault codes at the recorded seed, in fault-table order.
/// Injection failures are excluded: they are harness breaks, never
/// quarantines, and fail the suite loudly instead of counting here.
#[must_use]
pub fn quarantined_codes() -> Vec<(EventCode, u32)> {
    use engine_proof::chaos::LoudReason;
    engine_proof::chaos::run(A9_SEED)
        .loud
        .into_iter()
        .filter_map(|(code, reason)| match reason {
            LoudReason::Quarantined { attempts } => Some((code, attempts)),
            LoudReason::InjectionFailed(_) => None,
        })
        .collect()
}

/// `(healed, quarantined)` counts at the recorded seed.
#[must_use]
pub fn a9_counts() -> (usize, usize) {
    (healed_codes().len(), quarantined_codes().len())
}

/// True when every fault healed or quarantined with no harness failure.
#[must_use]
pub fn is_clean() -> bool {
    use engine_proof::chaos::LoudReason;
    let report = engine_proof::chaos::run(A9_SEED);
    let quarantined = report
        .loud
        .iter()
        .filter(|(_, reason)| matches!(reason, LoudReason::Quarantined { .. }))
        .count();
    let broken = report.loud.len() - quarantined;
    broken == 0 && report.healed.len() + quarantined == engine_proof::chaos::FAULTS.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_proof::chaos::LoudReason;

    #[test]
    fn a9_heals_three_and_quarantines_nine() {
        let (healed, quarantined) = a9_counts();
        assert_eq!(healed, 3, "healed {healed}");
        assert_eq!(quarantined, 9, "quarantined {quarantined}");
    }

    #[test]
    fn a9_partition_covers_every_fault_row() {
        let report = engine_proof::chaos::run(A9_SEED);
        assert_eq!(report.healed.len(), 3);
        assert_eq!(report.loud.len(), engine_proof::chaos::FAULTS.len() - 3);
        for (_, reason) in &report.loud {
            assert_eq!(*reason, LoudReason::Quarantined { attempts: 1 });
        }
        assert!(is_clean());
    }

    #[test]
    fn a9_healed_codes_are_net_assets_save() {
        assert_eq!(
            healed_codes(),
            vec![
                (EventCode::NET_CLIENT_DESYNC, 1),
                (EventCode::ASSETS_MISSING, 1),
                (EventCode::SAVE_CORRUPT, 1),
            ]
        );
    }

    #[test]
    fn same_seed_twice_gives_same_counts() {
        assert_eq!(a9_counts(), a9_counts());
    }
}

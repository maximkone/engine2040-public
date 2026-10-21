//! Thin A9 chaos facade over the proof suite.
//!
//! Owner: this crate owns nothing but the A9 row's healed/quarantined
//! counts; `engine-proof` owns fault injection (`chaos::run`), the observe
//! catalog owns heal records, the fixture owns tape truth. There is no
//! second chaos system here: every count below comes from one
//! `engine-proof` run at the recorded A9 seed. Units: counts (`usize`).
//! Determinism: same seed always gives the same counts on every machine.
//!
//! One run, one report: [`a9_report`] runs the suite once and tabulates
//! every fault row into exactly one list - healed, quarantined, or broken.
//! A broken row is a harness failure, never a quarantine, and it stays
//! visible: it must not vanish from both lists and leave the counts looking
//! complete while a row never ran.

use engine_observe::EventCode;

/// Recorded A9 seed (mirrors `engine-proof` `a9::A9_SEED`).
pub const A9_SEED: u64 = 9;

/// One run of the A9 suite: every fault row in exactly one list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A9Report {
    /// Faults that healed, in fault-table order: code plus attempt count.
    pub healed: Vec<(EventCode, u32)>,
    /// Faults that quarantined, in fault-table order: code plus attempts.
    pub quarantined: Vec<(EventCode, u32)>,
    /// Rows whose injection broke the harness; the row never ran.
    pub broken: Vec<EventCode>,
}

/// Run the A9 suite once and tabulate every fault row.
///
/// One run feeds every count below: the healed list, the quarantined list
/// and the broken rows come from the same report, so they always describe
/// one run of the suite.
#[must_use]
pub fn a9_report() -> A9Report {
    use engine_proof::chaos::LoudReason;
    let report = engine_proof::chaos::run(A9_SEED);
    let mut healed = Vec::with_capacity(report.healed.len());
    for (code, attempts) in report.healed {
        healed.push((code, attempts));
    }
    let mut quarantined = Vec::with_capacity(report.loud.len());
    let mut broken = Vec::new();
    for (code, reason) in report.loud {
        match reason {
            LoudReason::Quarantined { attempts } => quarantined.push((code, attempts)),
            LoudReason::InjectionFailed(_) => broken.push(code),
        }
    }
    A9Report {
        healed,
        quarantined,
        broken,
    }
}

/// Healed fault codes at the recorded seed, in fault-table order.
#[must_use]
pub fn healed_codes() -> Vec<(EventCode, u32)> {
    a9_report().healed
}

/// Quarantined fault codes at the recorded seed, in fault-table order.
/// Injection failures are excluded: they are harness breaks, never
/// quarantines. They are not hidden either - [`broken_codes`] names them.
#[must_use]
pub fn quarantined_codes() -> Vec<(EventCode, u32)> {
    a9_report().quarantined
}

/// Fault rows whose injection broke the harness, in fault-table order.
///
/// A broken row is a harness break: the fault was never injected, so the
/// row was neither healed nor quarantined. Empty at the recorded seed.
#[must_use]
pub fn broken_codes() -> Vec<EventCode> {
    a9_report().broken
}

/// `(healed, quarantined)` counts at the recorded seed, from one run.
#[must_use]
pub fn a9_counts() -> (usize, usize) {
    let report = a9_report();
    (report.healed.len(), report.quarantined.len())
}

/// True when every fault healed or quarantined with no harness failure.
#[must_use]
pub fn is_clean() -> bool {
    let report = a9_report();
    report.broken.is_empty()
        && report.healed.len() + report.quarantined.len() == engine_proof::chaos::FAULTS.len()
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

    #[test]
    fn every_fault_row_is_accounted_for_by_one_run() {
        // The suite has one row per fault. Every row lands in exactly one
        // list: healed, quarantined or broken. A broken row (the harness
        // itself failed) used to be dropped from both lists, so the counts
        // still looked complete while a row never ran.
        let report = a9_report();
        assert_eq!(
            report.healed.len() + report.quarantined.len() + report.broken.len(),
            engine_proof::chaos::FAULTS.len(),
            "every fault row is in exactly one list"
        );
        assert!(
            report.broken.is_empty(),
            "the recorded seed has no harness break: {:?}",
            report.broken
        );
        assert!(is_clean());
        // The counts of one run add up to the fault table.
        let (healed, quarantined) = a9_counts();
        assert_eq!(
            healed + quarantined,
            engine_proof::chaos::FAULTS.len(),
            "the count pair comes from one run"
        );
        assert!(broken_codes().is_empty());
    }
}

//! Sealed assert runs over recorded tapes; harness only, never in the tick.
//!
//! Consumer: the door harness that grades rule fixes. Owner: this module
//! owns the run shape; tapes stay with their recorders. Units: ticks for
//! budgets, counts for kills. Determinism: same tape plus same asserts
//! gives the same receipt twice; no clock, no threads, no allocation
//! past the caller's slices. Errors are typed [`AssertError`]: bad input
//! fails loud, an over-budget run stops with usage posted.

/// Most asserts one run may carry.
pub const MAX_ASSERTS_PER_RUN: usize = 64;
/// Most tape rows one run may walk.
pub const MAX_TAPE_ROWS: usize = 4096;
/// Most mutant outcomes one calibration may read.
pub const MAX_MUTANTS: usize = 32;
/// Kills wanted out of a 30-mutant sweep.
pub const KILL_TARGET: u32 = 27;
/// Mutants in the standard sweep.
pub const SWEEP_MUTANTS: u32 = 30;
/// Clean tapes that must all pass for a zero-false-alarm claim.
pub const CLEAN_TAPES: u32 = 20;

/// How closed the run is. Strict refuses any row that touched the net.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confinement {
    /// No network rows, bounded ticks, resume from progress only.
    Strict,
    /// Network rows allowed; ticks still bounded.
    Open,
}

/// The budget and closure for one sealed run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SealedConfig {
    /// Last tick the run may read; later rows stop the run.
    pub max_ticks: u32,
    /// Whether network rows are refused.
    pub confinement: Confinement,
}

/// One recorded row: the tick, its digest, and whether it used the net.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TapeRow {
    /// Tick stamped at record time.
    pub tick: u32,
    /// Digest of the state after the tick.
    pub digest: u64,
    /// True when the row reached the network.
    pub used_network: bool,
}

/// One per-tick check: the tick must carry this digest.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TickAssert {
    /// Tick to check.
    pub tick: u32,
    /// Digest the tape must hold there.
    pub want_digest: u64,
}

/// Resume point: where the next leg starts, and how much is checked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RunProgress {
    /// Tape index to start from.
    pub next_index: usize,
    /// Rows already checked on earlier legs.
    pub checked: u32,
}

/// What a finished leg reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RunReceipt {
    /// Rows checked on this leg.
    pub checked: u32,
    /// Fold of every checked digest, in tape order.
    pub digest: u64,
    /// True when the leg read the whole tape.
    pub completed: bool,
}

/// Why a sealed run or calibration refused.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AssertError {
    /// No rows or no asserts were handed over.
    Empty,
    /// The batch broke a bound; count carried.
    TooMany {
        /// Rows, asserts, or outcomes handed over.
        given: usize,
    },
    /// The tape ran past the tick budget.
    OverBudget {
        /// Ticks used.
        used: u32,
        /// Ticks allowed.
        allowed: u32,
    },
    /// A tick carried the wrong digest.
    Mismatch {
        /// Tick that failed.
        tick: u32,
        /// Digest wanted.
        want: u64,
        /// Digest found.
        got: u64,
    },
    /// A network row met a strict run.
    ConfinementBreach {
        /// Tick that touched the net.
        tick: u32,
    },
    /// Ticks or asserts arrived out of order.
    OutOfOrder {
        /// Tick that broke the order.
        tick: u32,
    },
}

impl core::fmt::Display for AssertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Empty => write!(f, "assert run: nothing to check"),
            Self::TooMany { given } => write!(f, "assert run: {given} rows over bound"),
            Self::OverBudget { used, allowed } => {
                write!(f, "assert run: {used} ticks past the {allowed} budget")
            }
            Self::Mismatch { tick, want, got } => {
                write!(f, "assert run: tick {tick} digest {got:#x} != {want:#x}")
            }
            Self::ConfinementBreach { tick } => {
                write!(
                    f,
                    "assert run: tick {tick} used the network under strict seal"
                )
            }
            Self::OutOfOrder { tick } => write!(f, "assert run: tick {tick} out of order"),
        }
    }
}

impl std::error::Error for AssertError {}

/// Walk the tape from `resume`, checking each assert in order.
///
/// Receipt digests fold through `engine_core::Fnv64`, in tape order.
///
/// Asserts must arrive sorted by tick, and tape ticks must rise. The run
/// stops loud at the first breach, mismatch, or budget overrun, so a
/// later leg can resume from the posted progress.
pub fn run_sealed(
    tape: &[TapeRow],
    asserts: &[TickAssert],
    cfg: &SealedConfig,
    resume: &RunProgress,
) -> Result<RunReceipt, AssertError> {
    if tape.is_empty() || asserts.is_empty() {
        return Err(AssertError::Empty);
    }
    if tape.len() > MAX_TAPE_ROWS || asserts.len() > MAX_ASSERTS_PER_RUN {
        return Err(AssertError::TooMany {
            given: tape.len().max(asserts.len()),
        });
    }
    if resume.next_index > tape.len() {
        return Err(AssertError::OutOfOrder { tick: u32::MAX });
    }
    for w in asserts.windows(2) {
        if w[0].tick >= w[1].tick {
            return Err(AssertError::OutOfOrder { tick: w[1].tick });
        }
    }
    let mut ai = 0usize;
    let resume_tick = tape.get(resume.next_index).map_or(0, |r| r.tick);
    while ai < asserts.len() && asserts[ai].tick < resume_tick {
        ai += 1;
    }
    let mut fold = engine_core::Fnv64::new();
    let mut checked: u32 = 0;
    let mut prev: Option<u32> = None;
    for row in tape.iter().skip(resume.next_index) {
        if let Some(p) = prev {
            if row.tick <= p {
                return Err(AssertError::OutOfOrder { tick: row.tick });
            }
        }
        prev = Some(row.tick);
        if row.tick > cfg.max_ticks {
            return Err(AssertError::OverBudget {
                used: row.tick,
                allowed: cfg.max_ticks,
            });
        }
        if cfg.confinement == Confinement::Strict && row.used_network {
            return Err(AssertError::ConfinementBreach { tick: row.tick });
        }
        while ai < asserts.len() && asserts[ai].tick < row.tick {
            ai += 1;
        }
        if ai < asserts.len() && asserts[ai].tick == row.tick {
            if asserts[ai].want_digest != row.digest {
                return Err(AssertError::Mismatch {
                    tick: row.tick,
                    want: asserts[ai].want_digest,
                    got: row.digest,
                });
            }
            ai += 1;
        }
        fold.write_u64(row.digest);
        checked += 1;
    }
    Ok(RunReceipt {
        checked,
        digest: fold.finish(),
        completed: true,
    })
}

/// One mutant: did the fixed rule kill it, and did the naive rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MutantOutcome {
    /// Mutant id, 0-based within the sweep.
    pub id: u32,
    /// True when the fixed rule trips on it.
    pub proper_killed: bool,
    /// True when the naive rule trips on it.
    pub naive_killed: bool,
}

/// The calibration count from one sweep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Calibration {
    /// Mutants swept.
    pub total: u32,
    /// Mutants the fixed rule killed.
    pub proper_kills: u32,
    /// Mutants the naive rule killed.
    pub naive_kills: u32,
}

impl Calibration {
    /// True when the sweep meets the bar: enough proper kills, and the
    /// naive rule kills strictly fewer, so the kills mean something.
    pub fn meets_bar(&self) -> bool {
        self.total == SWEEP_MUTANTS
            && self.proper_kills >= KILL_TARGET
            && self.naive_kills < self.proper_kills
    }
}

/// Count one sweep. Empty and over-bound sweeps fail loud.
pub fn calibrate(outcomes: &[MutantOutcome]) -> Result<Calibration, AssertError> {
    if outcomes.is_empty() {
        return Err(AssertError::Empty);
    }
    if outcomes.len() > MAX_MUTANTS {
        return Err(AssertError::TooMany {
            given: outcomes.len(),
        });
    }
    let mut proper = 0u32;
    let mut naive = 0u32;
    for (i, o) in outcomes.iter().enumerate() {
        let want = u32::try_from(i).map_err(|_| AssertError::OutOfOrder { tick: o.id })?;
        if o.id != want {
            return Err(AssertError::OutOfOrder { tick: o.id });
        }
        proper += u32::from(o.proper_killed);
        naive += u32::from(o.naive_killed);
    }
    let total = u32::try_from(outcomes.len()).map_err(|_| AssertError::TooMany {
        given: outcomes.len(),
    })?;
    Ok(Calibration {
        total,
        proper_kills: proper,
        naive_kills: naive,
    })
}

/// True when every clean tape passed, so no false alarm fired.
pub fn clean_sweep_passed(passed: u32, total: u32) -> bool {
    total == CLEAN_TAPES && passed == total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_tape(n: u32) -> Vec<TapeRow> {
        (0..n)
            .map(|t| TapeRow {
                tick: t,
                digest: 0x9E37_79B9_7F4A_7C15u64.wrapping_add(u64::from(t).wrapping_mul(97)),
                used_network: false,
            })
            .collect()
    }

    fn asserts_for(tape: &[TapeRow]) -> Vec<TickAssert> {
        tape.iter()
            .map(|r| TickAssert {
                tick: r.tick,
                want_digest: r.digest,
            })
            .collect()
    }

    #[test]
    fn sealed_run_passes_and_reproduces_x2() {
        let tape = clean_tape(10);
        let asserts = asserts_for(&tape);
        let cfg = SealedConfig {
            max_ticks: 600,
            confinement: Confinement::Strict,
        };
        let start = RunProgress {
            next_index: 0,
            checked: 0,
        };
        let a = run_sealed(&tape, &asserts, &cfg, &start).expect("clean tape passes");
        let b = run_sealed(&tape, &asserts, &cfg, &start).expect("clean tape passes twice");
        assert!(a.completed && b.completed);
        assert_eq!(a.checked, 10);
        assert_eq!(a.digest, b.digest, "same inputs give the same receipt");
    }

    #[test]
    fn sealed_run_trips_on_a_drifted_digest() {
        let mut tape = clean_tape(6);
        tape[4].digest ^= 0xFFFF;
        let asserts = asserts_for(&clean_tape(6));
        let cfg = SealedConfig {
            max_ticks: 600,
            confinement: Confinement::Strict,
        };
        let start = RunProgress {
            next_index: 0,
            checked: 0,
        };
        match run_sealed(&tape, &asserts, &cfg, &start) {
            Err(AssertError::Mismatch { tick, .. }) => assert_eq!(tick, 4),
            other => panic!("drift must mismatch, got {other:?}"),
        }
    }

    #[test]
    fn sealed_run_stops_at_the_tick_budget() {
        let tape = clean_tape(12);
        let asserts = asserts_for(&tape);
        let cfg = SealedConfig {
            max_ticks: 8,
            confinement: Confinement::Open,
        };
        let start = RunProgress {
            next_index: 0,
            checked: 0,
        };
        match run_sealed(&tape, &asserts, &cfg, &start) {
            Err(AssertError::OverBudget { used, allowed }) => {
                assert_eq!((used, allowed), (9, 8));
            }
            other => panic!("past-budget tape must stop, got {other:?}"),
        }
    }

    #[test]
    fn strict_run_refuses_network_rows() {
        let mut tape = clean_tape(4);
        tape[2].used_network = true;
        let asserts = asserts_for(&clean_tape(4));
        let cfg = SealedConfig {
            max_ticks: 600,
            confinement: Confinement::Strict,
        };
        let start = RunProgress {
            next_index: 0,
            checked: 0,
        };
        match run_sealed(&tape, &asserts, &cfg, &start) {
            Err(AssertError::ConfinementBreach { tick }) => assert_eq!(tick, 2),
            other => panic!("network row under seal must breach, got {other:?}"),
        }
    }

    #[test]
    fn sweep_kills_mutants_without_false_alarms() {
        let outcomes: Vec<MutantOutcome> = (0..SWEEP_MUTANTS)
            .map(|id| MutantOutcome {
                id,
                proper_killed: id != 7 && id != 19 && id != 28,
                naive_killed: id % 5 == 0,
            })
            .collect();
        let cal = calibrate(&outcomes).expect("sweep counts");
        assert_eq!(cal.proper_kills, KILL_TARGET);
        assert!(cal.meets_bar(), "27 of 30 with naive behind meets the bar");
        assert!(clean_sweep_passed(CLEAN_TAPES, CLEAN_TAPES));
        assert!(!clean_sweep_passed(CLEAN_TAPES - 1, CLEAN_TAPES));
    }

    #[test]
    fn bad_inputs_fail_loud() {
        let cfg = SealedConfig {
            max_ticks: 600,
            confinement: Confinement::Strict,
        };
        let start = RunProgress {
            next_index: 0,
            checked: 0,
        };
        assert_eq!(run_sealed(&[], &[], &cfg, &start), Err(AssertError::Empty));
        assert_eq!(calibrate(&[]), Err(AssertError::Empty));
    }
}

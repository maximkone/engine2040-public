//! Hour-long online soak over the siege harness (ON11).
//!
//! Consumer: the proof gate (`cargo test -p engine-proof online_soak`) and the
//! EMPIRE online outcome. Data owner: this module owns [`SoakConfig`],
//! [`SoakReport`], [`PerFaultSoak`] and [`SoakError`]; `engine-net` owns the
//! tape (`tick_budget::record_tape`, `canonical_digest`), the fault matrix
//! (`wan_soak::run_matrix` shape via `run_fault`) and every number asserted
//! below. Units: ticks at 60 Hz, minutes, slots, bytes, percent. Determinism:
//! window seeds fork from one seed with integer ops in [`ALL_FAULTS`] order;
//! wall-clock resim evidence never enters a digest. Budget: offline proof
//! tool, not per-tick code; at most 360 bounded windows, one report row each.

use std::fmt::{self, Display, Formatter};

use engine_net::tick_budget::{
    record_tape, SiegeTickWorld, TickBudgetError, MAX_TICKS, PLAYERS, SIEGE_SEED, SIEGE_TICKS,
    SOLDIERS,
};
use engine_net::wan_soak::{
    run_fault, Fault, FaultReport, WanError, ALL_FAULTS, CONVERGE_BAR_TICKS, DISCONNECT_VICTIMS,
};

/// Human slots in the soak; pinned to the ON10 siege shape, never mirrored.
pub const SLOTS: usize = PLAYERS;
/// AI soldiers per siege; pinned to the ON10 legion count, never mirrored.
pub const LEGIONS: usize = SOLDIERS;
/// Soak seed; the ON10 canonical siege seed, so window zero replays it.
pub const SOAK_SEED: u64 = SIEGE_SEED;
/// Ticks per soak window; the ON10 canonical short tape, so every window
/// carries a byte-identical checkpoint the matrix must hold.
pub const WINDOW_TICKS: u32 = SIEGE_TICKS;
/// Always-on short proof: one window per fault (10 s of siege per fault).
pub const WINDOWS_SHORT: u32 = 5;
/// Hour-long soak: 360 windows of 600 ticks at 60 Hz (216 000 ticks).
pub const WINDOWS_HOUR: u32 = 360;
/// Ticks in the full hour soak: 60 minutes at 60 Hz.
pub const HOUR_TICKS: u64 = 216_000;
/// Memory bar: end-of-soak growth over start, in percent.
pub const MEM_DRIFT_PCT: f64 = 5.0;
/// Reconnect bar in ticks (5 s at 60 Hz); the ON4 converge bar, aliased.
pub const RECONNECT_BAR_TICKS: u32 = CONVERGE_BAR_TICKS;
/// Slots the disconnect fault drops; the ON4 victim count, aliased.
pub const DISCONNECT_SLOTS: usize = DISCONNECT_VICTIMS;

/// Fork one window seed from the soak seed. SplitMix64 avalanche over
/// `(seed, window)` with integer ops only, so every window replays exactly.
#[must_use]
pub const fn chain_seed(seed: u64, window: u32) -> u64 {
    let mut mix = seed.wrapping_add((window as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    mix ^= mix >> 29;
    mix = mix.wrapping_mul(0x1656_67B1_9E37_79F9);
    mix ^= mix >> 32;
    mix = mix.wrapping_mul(0xBE5B_7B7E_ABCD_EF13);
    mix ^ (mix >> 31)
}

/// First minute holding `window`: windows are 600 ticks (10 s), so six
/// windows fill a minute. Names the red minute the acceptance demands.
#[must_use]
pub const fn minute_of(window: u32) -> u32 {
    window.saturating_mul(WINDOW_TICKS) / 3_600
}

/// Which slots and legions the soak runs, and for how long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoakConfig {
    /// Human slots; must equal [`SLOTS`].
    pub slots: usize,
    /// AI soldiers; must equal [`LEGIONS`].
    pub legions: usize,
    /// Destruction break events authoritative; the soak refuses `false`.
    pub destruction_on: bool,
    /// Windows stepped, in `1..=WINDOWS_HOUR`.
    pub windows: u32,
}

impl SoakConfig {
    /// Always-on short proof: five windows, one per fault.
    #[must_use]
    pub const fn short() -> Self {
        Self {
            slots: SLOTS,
            legions: LEGIONS,
            destruction_on: true,
            windows: WINDOWS_SHORT,
        }
    }

    /// Hour-long soak: 360 windows under the ON4 fault model.
    #[must_use]
    pub const fn hour() -> Self {
        Self {
            slots: SLOTS,
            legions: LEGIONS,
            destruction_on: true,
            windows: WINDOWS_HOUR,
        }
    }

    /// Refuse out-of-shape configs loudly, with the legal shape named.
    pub fn validate(self) -> Result<(), SoakError> {
        if self.slots != SLOTS {
            return Err(SoakError::BadSlots {
                got: self.slots,
                want: SLOTS,
            });
        }
        if self.legions != LEGIONS {
            return Err(SoakError::BadLegions {
                got: self.legions,
                want: LEGIONS,
            });
        }
        if !self.destruction_on {
            return Err(SoakError::DestructionOff);
        }
        if self.windows == 0 || self.windows > WINDOWS_HOUR {
            return Err(SoakError::BadWindows { got: self.windows });
        }
        Ok(())
    }
}

/// Soak failure: which gate refused, with both sides and the first bad minute.
#[derive(Clone, Debug, PartialEq)]
pub enum SoakError {
    /// Window count outside `1..=WINDOWS_HOUR`.
    BadWindows {
        /// Windows the caller offered.
        got: u32,
    },
    /// Slot count is not the 64-slot siege.
    BadSlots {
        /// Slots the caller offered.
        got: usize,
        /// Slots the siege needs.
        want: usize,
    },
    /// Legion count is not the 500-AI siege.
    BadLegions {
        /// Legions the caller offered.
        got: usize,
        /// Legions the siege needs.
        want: usize,
    },
    /// The soak runs destruction-on; an unarmed siege is refused.
    DestructionOff,
    /// A window checkpoint diverged from the clean ON10 tape.
    Desync {
        /// Fault whose window diverged.
        fault: Fault,
        /// First bad window.
        window: u32,
        /// First bad minute.
        minute: u32,
        /// Clean ON10 checkpoint.
        expected: u64,
        /// Soaked checkpoint.
        actual: u64,
    },
    /// A window reported a crash (a real crash aborts the run, so a green
    /// report always carries zero; the variant keeps forged rows red).
    Crash {
        /// Fault whose row claims a crash.
        fault: Fault,
        /// First bad window.
        window: u32,
        /// First bad minute.
        minute: u32,
    },
    /// A client needed longer than 5 s after the fault to converge.
    SlowReconnect {
        /// Fault whose window stayed dark.
        fault: Fault,
        /// First bad window.
        window: u32,
        /// First bad minute.
        minute: u32,
        /// Ticks the slowest client needed.
        got: u32,
        /// Ticks the bar allows.
        bar: u32,
    },
    /// A dropped slot never reclaimed its place on reconnect.
    SlotLost {
        /// Fault whose victim stayed homeless.
        fault: Fault,
        /// First bad window.
        window: u32,
        /// First bad minute.
        minute: u32,
        /// Victims reclaimed.
        got: u32,
        /// Victims the fault dropped.
        want: u32,
    },
    /// Server buffers grew past the 5% bar over the soak.
    MemoryDrift {
        /// Growth in percent.
        growth_pct: f64,
        /// Bar in percent.
        budget_pct: f64,
        /// First window whose capacities moved.
        window: u32,
        /// First bad minute.
        minute: u32,
    },
    /// One window breached a converge bar the soak does not own (rollback,
    /// resim, cadence, starve); carries the fault, window and minute.
    Wan {
        /// Fault whose window breached.
        fault: Fault,
        /// First bad window.
        window: u32,
        /// First bad minute.
        minute: u32,
        /// The refused-fault cause.
        cause: WanError,
    },
    /// The ON10 siege harness refused (budget, order or config).
    Tick(TickBudgetError),
}

impl Display for SoakError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadWindows { got } => {
                write!(f, "soak windows {got} outside 1..={WINDOWS_HOUR}")
            }
            Self::BadSlots { got, want } => {
                write!(f, "soak slots {got} is not the {want}-slot siege")
            }
            Self::BadLegions { got, want } => {
                write!(f, "soak legions {got} is not the {want}-AI siege")
            }
            Self::DestructionOff => write!(f, "soak refuses destruction-off: arm break events"),
            Self::Desync {
                fault,
                window,
                minute,
                expected,
                actual,
            } => write!(
                f,
                "soak desync fault={} window={window} first_bad_minute={minute}: \
                 expected {expected:016x}, got {actual:016x}",
                fault.label(),
            ),
            Self::Crash {
                fault,
                window,
                minute,
            } => write!(
                f,
                "soak crash fault={} window={window} first_bad_minute={minute}",
                fault.label(),
            ),
            Self::SlowReconnect {
                fault,
                window,
                minute,
                got,
                bar,
            } => write!(
                f,
                "soak reconnect fault={} window={window} first_bad_minute={minute}: \
                 {got} ticks over the {bar}-tick bar",
                fault.label(),
            ),
            Self::SlotLost {
                fault,
                window,
                minute,
                got,
                want,
            } => write!(
                f,
                "soak slot fault={} window={window} first_bad_minute={minute}: \
                 reclaimed {got} of {want} dropped slots",
                fault.label(),
            ),
            Self::MemoryDrift {
                growth_pct,
                budget_pct,
                window,
                minute,
            } => write!(
                f,
                "soak memory drift {growth_pct:.1}% over {budget_pct:.1}% \
                 at window={window} first_bad_minute={minute}",
            ),
            Self::Wan {
                fault,
                window,
                minute,
                cause,
            } => write!(
                f,
                "soak fault={} window={window} first_bad_minute={minute}: {cause}",
                fault.label(),
            ),
            Self::Tick(err) => write!(f, "soak siege harness refused ({err})"),
        }
    }
}

impl std::error::Error for SoakError {}

impl From<TickBudgetError> for SoakError {
    fn from(err: TickBudgetError) -> Self {
        Self::Tick(err)
    }
}

/// Deterministic projection of one fault row for x2 asserts: the folded
/// checkpoint plus every counter and the first bad window. No floats and no
/// wall clock ever enter.
pub type FaultDigest = (
    u64,
    u32,
    u32,
    u32,
    u32,
    u32,
    Option<u32>,
    u64,
    u64,
    u32,
    u32,
    u64,
);

/// Deterministic projection of a whole soak for x2 asserts: seed, window
/// count, destruction bit, folded checkpoint, memory flag and five rows.
pub type SoakDigest = (u64, u32, bool, u64, bool, [FaultDigest; 5]);

/// Aggregate over one fault's windows: counters plus the first bad window.
/// Timing fields never enter here; the wall-clock evidence stays in the
/// matrix. `bad_*` fields carry the first bad window's numbers (zero green).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PerFaultSoak {
    /// Fault these windows soaked.
    pub fault: Fault,
    /// Windows soaked under this fault.
    pub windows: u32,
    /// Checkpoint divergences (green runs carry zero).
    pub desyncs: u32,
    /// Crashes (a completed run always carries zero).
    pub crashes: u32,
    /// Worst reconnect ticks over this fault's windows.
    pub reconnect_max: u32,
    /// Dropped slots reclaimed on reconnect (disconnect fault only).
    pub reclaims: u32,
    /// First bad window, if any gate tripped under this fault.
    pub first_bad_window: Option<u32>,
    /// Clean checkpoint at the first bad window.
    pub bad_expected: u64,
    /// Soaked checkpoint at the first bad window.
    pub bad_actual: u64,
    /// Reconnect ticks at the first slow window.
    pub bad_reconnect: u32,
    /// Reclaims at the first slot-short window.
    pub bad_reclaims: u32,
    /// Fold over this fault's clean plus soaked checkpoints.
    pub hash: u64,
}

impl PerFaultSoak {
    /// True while no gate tripped under this fault.
    #[must_use]
    pub const fn is_green(self) -> bool {
        self.first_bad_window.is_none()
    }

    /// First bad minute, when the row is red.
    #[must_use]
    pub const fn first_bad_minute(self) -> Option<u32> {
        match self.first_bad_window {
            Some(window) => Some(minute_of(window)),
            None => None,
        }
    }

    /// Deterministic projection for x2 asserts (no floats, no wall clock).
    #[must_use]
    pub const fn digest_key(self) -> FaultDigest {
        (
            self.hash,
            self.windows,
            self.desyncs,
            self.crashes,
            self.reconnect_max,
            self.reclaims,
            self.first_bad_window,
            self.bad_expected,
            self.bad_actual,
            self.bad_reconnect,
            self.bad_reclaims,
            chain_seed(self.windows as u64, self.desyncs),
        )
    }

    /// Posted per-fault line: counters, bars and the red minute or green.
    #[must_use]
    pub fn summary(&self) -> String {
        let minute = match self.first_bad_minute() {
            Some(minute) => minute.to_string(),
            None => "none".to_string(),
        };
        format!(
            "soak fault={} windows={} desyncs={} crashes={} reconnect_max={} (bar {}) \
             reclaims={} first_bad_minute={} verdict={}",
            self.fault.label(),
            self.windows,
            self.desyncs,
            self.crashes,
            self.reconnect_max,
            RECONNECT_BAR_TICKS,
            self.reclaims,
            minute,
            if self.is_green() { "green" } else { "RED" },
        )
    }

    /// Fail the first tripped gate with the fault, window and minute.
    pub fn check(&self) -> Result<(), SoakError> {
        let Some(window) = self.first_bad_window else {
            return Ok(());
        };
        let minute = minute_of(window);
        if self.crashes > 0 {
            return Err(SoakError::Crash {
                fault: self.fault,
                window,
                minute,
            });
        }
        if self.desyncs > 0 {
            return Err(SoakError::Desync {
                fault: self.fault,
                window,
                minute,
                expected: self.bad_expected,
                actual: self.bad_actual,
            });
        }
        if self.bad_reconnect > RECONNECT_BAR_TICKS {
            return Err(SoakError::SlowReconnect {
                fault: self.fault,
                window,
                minute,
                got: self.bad_reconnect,
                bar: RECONNECT_BAR_TICKS,
            });
        }
        // The runner only marks a row bad alongside a real breach above, so
        // reaching here means a slot shortfall on the disconnect row (any
        // other shape is a forged row; it still fails loudly with its
        // fault, window and minute).
        if self.fault == Fault::Disconnect {
            return Err(SoakError::SlotLost {
                fault: self.fault,
                window,
                minute,
                got: self.bad_reclaims,
                want: DISCONNECT_SLOTS as u32,
            });
        }
        Err(SoakError::Wan {
            fault: self.fault,
            window,
            minute,
            cause: WanError::SlowConverge {
                got: self.bad_reconnect,
                bar: RECONNECT_BAR_TICKS,
            },
        })
    }
}

/// Whole-soak report: config, one row per fault in [`ALL_FAULTS`] order,
/// the folded checkpoint and the global memory ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct SoakReport {
    /// Config the soak ran.
    pub config: SoakConfig,
    /// Seed the window chain forked from.
    pub seed: u64,
    /// Fold over every window's clean plus soaked checkpoints.
    pub hash: u64,
    /// One row per fault, in [`ALL_FAULTS`] order.
    pub per_fault: [PerFaultSoak; 5],
    /// Server buffer growth over the soak, in percent (0.0 when stable).
    pub mem_growth_pct: f64,
    /// Capacities moved at all during the soak.
    pub mem_moved: bool,
    /// First window whose capacities moved, if any.
    pub mem_bad_window: Option<u32>,
}

impl SoakReport {
    /// True while every fault row is green and memory sits in budget.
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.per_fault.iter().all(|row| row.is_green())
            && !self.mem_moved
            && self.mem_growth_pct <= MEM_DRIFT_PCT
    }

    /// Deterministic projection for x2 asserts (no floats, no wall clock).
    #[must_use]
    pub fn digest_key(&self) -> SoakDigest {
        (
            self.seed,
            self.config.windows,
            self.config.destruction_on,
            self.hash,
            self.mem_moved,
            self.per_fault.map(PerFaultSoak::digest_key),
        )
    }

    /// Fail the first red row in report order, else the memory ledger.
    pub fn check(&self) -> Result<(), SoakError> {
        self.config.validate()?;
        for row in &self.per_fault {
            row.check()?;
        }
        if self.mem_moved || self.mem_growth_pct > MEM_DRIFT_PCT {
            let window = self.mem_bad_window.unwrap_or(0);
            return Err(SoakError::MemoryDrift {
                growth_pct: self.mem_growth_pct,
                budget_pct: MEM_DRIFT_PCT,
                window,
                minute: minute_of(window),
            });
        }
        Ok(())
    }

    /// Posted report: header plus one line per fault. Every red row names
    /// its first bad minute; that is the acceptance line.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = format!(
            "soak seed={} windows={} ticks={} slots={}+{}ai destruction={} hash={:016x} \
             mem_growth={:.1}% (bar {MEM_DRIFT_PCT:.1}%) verdict={}\n",
            self.seed,
            self.config.windows,
            u64::from(self.config.windows).saturating_mul(u64::from(WINDOW_TICKS)),
            self.config.slots,
            self.config.legions,
            if self.config.destruction_on {
                "on"
            } else {
                "OFF"
            },
            self.hash,
            self.mem_growth_pct,
            if self.is_green() { "green" } else { "RED" },
        );
        for row in &self.per_fault {
            out.push_str(&row.summary());
            out.push('\n');
        }
        out
    }
}

/// Fold one window's checkpoint pair into the running hash (integer ops).
const fn fold_window(hash: u64, clean: u64, soaked: u64) -> u64 {
    let mut out = hash ^ clean;
    out = out.wrapping_mul(1_099_511_628_211);
    out ^= soaked;
    out.wrapping_mul(1_099_511_628_211)
}

/// Bounded fault attempts over one window seed: only the wall-clock resim
/// sample varies run to run, so a poisoned sample retries while a
/// deterministic refusal fails at once. The seed never advances, so a green
/// sample is byte-identical on every attempt.
fn run_fault_stable(seed: u64, fault: Fault, ticks: u32) -> Result<FaultReport, WanError> {
    const ATTEMPTS: u32 = 3;
    let mut attempt = 0;
    loop {
        match run_fault(seed, fault, ticks) {
            Ok(report) => return Ok(report),
            Err(error) if attempt + 1 < ATTEMPTS && matches!(error, WanError::OverResim { .. }) => {
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Step `windows` soak windows from `seed`: each window forks its seed,
/// records the clean ON10 tape, soaks the same tape under its ON4 fault,
/// and folds the checkpoint pair. Windows rotate through [`ALL_FAULTS`]
/// in order, so window `w` always soaks `ALL_FAULTS[w % 5]`.
pub fn run_windows(seed: u64, windows: u32, destruction_on: bool) -> Result<SoakReport, SoakError> {
    // Share the chaos serial gate: this runner steps the same siege worlds
    // through the same wall-clock resim bar, so parallel suites would
    // otherwise poison each other's timing samples.
    let _serial = crate::chaos::hold_serial();
    let config = SoakConfig {
        slots: SLOTS,
        legions: LEGIONS,
        destruction_on,
        windows,
    };
    config.validate()?;
    if WINDOW_TICKS > MAX_TICKS {
        return Err(SoakError::Tick(TickBudgetError::BadTicks {
            got: WINDOW_TICKS,
        }));
    }

    let mut hash: u64 = 1_464_717_047_381_303_168 ^ seed;
    let mut rows: [PerFaultSoak; 5] = ALL_FAULTS.map(|fault| PerFaultSoak {
        fault,
        windows: 0,
        desyncs: 0,
        crashes: 0,
        reconnect_max: 0,
        reclaims: 0,
        first_bad_window: None,
        bad_expected: 0,
        bad_actual: 0,
        bad_reconnect: 0,
        bad_reclaims: 0,
        hash: 1_464_717_047_381_303_168 ^ seed,
    });
    // Memory ledger: one siege world stepped across every window. Its
    // buffers are allocated once in `new`; any capacity move is a leak.
    let mut mem_world = SiegeTickWorld::new(seed)?;
    let caps_before = mem_world.capacities();
    let mut mem_moved = false;
    let mut mem_bad_window: Option<u32> = None;

    for window in 0..windows {
        let fault = ALL_FAULTS[(window as usize) % ALL_FAULTS.len()];
        let slot = (window as usize) % rows.len();
        let wseed = chain_seed(seed, window);
        let clean = record_tape(wseed, WINDOW_TICKS)?;
        let row = &mut rows[slot];
        debug_assert_eq!(row.fault, fault);
        row.windows += 1;
        match run_fault_stable(wseed, fault, WINDOW_TICKS) {
            Ok(rep) => {
                hash = fold_window(hash, clean.hash, rep.hash);
                row.hash = fold_window(row.hash, clean.hash, rep.hash);
                if rep.hash != clean.hash {
                    row.desyncs += 1;
                    if row.first_bad_window.is_none() {
                        row.first_bad_window = Some(window);
                        row.bad_expected = clean.hash;
                        row.bad_actual = rep.hash;
                    }
                }
                if rep.converge_ticks > row.reconnect_max {
                    row.reconnect_max = rep.converge_ticks;
                }
                if rep.converge_ticks > RECONNECT_BAR_TICKS && row.first_bad_window.is_none() {
                    // Unreachable through `run_fault` (its own bar trips
                    // first); kept so the soak names the minute even if the
                    // ON4 bar ever loosens.
                    row.first_bad_window = Some(window);
                    row.bad_reconnect = rep.converge_ticks;
                }
                if fault == Fault::Disconnect {
                    if rep.resyncs >= DISCONNECT_SLOTS as u32 {
                        row.reclaims += 1;
                        if row.first_bad_window.is_none() {
                            row.bad_reclaims = rep.resyncs;
                        }
                    } else if row.first_bad_window.is_none() {
                        row.first_bad_window = Some(window);
                        row.bad_reclaims = rep.resyncs;
                    }
                }
            }
            Err(cause) => {
                row.hash = fold_window(row.hash, clean.hash, 0);
                if row.first_bad_window.is_none() {
                    row.first_bad_window = Some(window);
                    if let WanError::SlowConverge { got, .. } = cause {
                        row.bad_reconnect = got;
                    }
                }
                return Err(SoakError::Wan {
                    fault,
                    window,
                    minute: minute_of(window),
                    cause,
                });
            }
        }
        for tick in 0..WINDOW_TICKS {
            mem_world.step_tick(window.saturating_mul(WINDOW_TICKS).saturating_add(tick))?;
        }
        if mem_world.capacities() != caps_before && !mem_moved {
            mem_moved = true;
            mem_bad_window = Some(window);
        }
    }

    let mem_growth_pct = if mem_moved { 100.0 } else { 0.0 };
    let report = SoakReport {
        config,
        seed,
        hash,
        per_fault: rows,
        mem_growth_pct,
        mem_moved,
        mem_bad_window,
    };
    report.check()?;
    Ok(report)
}

/// Always-on short-matrix proof: five windows from the canonical seed, one
/// per fault. Fast enough for the default suite; asserts the full gate set.
pub fn run_short() -> Result<SoakReport, SoakError> {
    run_windows(SOAK_SEED, WINDOWS_SHORT, true)
}

/// Hour-long online soak: 360 windows from the canonical seed under the ON4
/// fault model with destruction on. Release-mode timing test only.
pub fn run_hour() -> Result<SoakReport, SoakError> {
    run_windows(SOAK_SEED, WINDOWS_HOUR, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_net::tick_budget::canonical_digest;

    const _: () = assert!(WINDOW_TICKS <= MAX_TICKS);

    #[test]
    fn contract_pins_without_mirroring() {
        // Every shared number is imported, never restated: drift in the ON9
        // or ON10 contract fails here, never silently.
        assert_eq!(SLOTS, PLAYERS);
        assert_eq!(LEGIONS, SOLDIERS);
        assert_eq!(SOAK_SEED, SIEGE_SEED);
        assert_eq!(WINDOW_TICKS, SIEGE_TICKS);
        assert_eq!(RECONNECT_BAR_TICKS, CONVERGE_BAR_TICKS);
        assert_eq!(RECONNECT_BAR_TICKS, 300);
        assert_eq!(DISCONNECT_SLOTS, DISCONNECT_VICTIMS);
        assert_eq!(DISCONNECT_SLOTS, 8);
        assert_eq!(WINDOWS_SHORT, ALL_FAULTS.len() as u32);
        assert_eq!(
            u64::from(WINDOWS_HOUR) * u64::from(WINDOW_TICKS),
            HOUR_TICKS
        );
        assert_eq!(HOUR_TICKS, 216_000);
        assert_eq!(minute_of(0), 0);
        assert_eq!(minute_of(6), 1);
        assert_eq!(minute_of(359), 59);
    }

    #[test]
    fn window_seeds_fork_deterministically() {
        assert_eq!(chain_seed(7, 0), chain_seed(7, 0));
        assert_ne!(chain_seed(7, 0), chain_seed(7, 1));
        assert_ne!(chain_seed(7, 1), chain_seed(8, 1));
    }

    #[test]
    fn short_matrix_soak_is_green() {
        let report = run_short().expect("short soak must converge");
        assert_eq!(report.config, SoakConfig::short());
        assert_eq!(report.seed, SOAK_SEED);
        assert_eq!(report.per_fault.len(), ALL_FAULTS.len());
        for (row, fault) in report.per_fault.iter().zip(ALL_FAULTS) {
            assert_eq!(row.fault, fault, "rows keep matrix order");
            assert_eq!(row.windows, 1, "{fault:?}: short soak gives one window");
            assert_eq!(row.desyncs, 0, "{fault:?}: zero desyncs");
            assert_eq!(row.crashes, 0, "{fault:?}: zero crashes");
            assert!(
                row.reconnect_max <= RECONNECT_BAR_TICKS,
                "{fault:?}: reconnect {} over 5 s",
                row.reconnect_max,
            );
            assert!(row.is_green(), "{fault:?}: row must be green");
            assert_eq!(row.first_bad_minute(), None);
        }
        let disconnect = &report.per_fault[4];
        assert_eq!(
            disconnect.reclaims, 1,
            "the disconnect window must reclaim its 8 slots",
        );
        assert!(!report.mem_moved, "buffers must not regrow");
        assert_eq!(report.mem_growth_pct, 0.0);
        assert!(report.is_green());
        report.check().expect("green report must pass");
        assert!(!report.summary().is_empty());
    }

    #[test]
    fn short_matrix_replays_identically_x2() {
        let first = run_short().expect("first soak");
        let second = run_short().expect("second soak");
        assert_eq!(
            first.digest_key(),
            second.digest_key(),
            "same seed gives the same soak digest",
        );
        assert_eq!(first.hash, second.hash);
    }

    #[test]
    fn window_zero_matches_the_canonical_digest() {
        // The ON4 latency path must hold the ON10 canonical checkpoint:
        // seed 7 over 600 ticks soaked under latency equals the canonical
        // tape. (Soak windows fork their own seeds via `chain_seed`, so the
        // cross-check runs at the raw canonical seed, not a window seed.)
        let canonical = canonical_digest().expect("canonical tape must record");
        let rep = run_fault(SOAK_SEED, Fault::Latency, WINDOW_TICKS).expect("latency converges");
        assert_eq!(rep.hash, canonical.hash);
        assert_eq!(
            record_tape(SOAK_SEED, WINDOW_TICKS).expect("tape").hash,
            canonical.hash
        );
        assert_ne!(
            chain_seed(SOAK_SEED, 0),
            SOAK_SEED,
            "window seeds fork; they never alias the raw seed"
        );
    }

    #[test]
    fn different_seeds_diverge() {
        let a = run_windows(7, 1, true).expect("seed 7");
        let b = run_windows(8, 1, true).expect("seed 8");
        assert_ne!(a.hash, b.hash, "different seeds must diverge");
        assert_ne!(a.digest_key(), b.digest_key());
    }

    /// Hand-built green row: forged tests mutate it instead of re-running
    /// the soak, so each failure case stays fast in debug builds.
    fn green_row(fault: Fault) -> PerFaultSoak {
        PerFaultSoak {
            fault,
            windows: 1,
            desyncs: 0,
            crashes: 0,
            reconnect_max: 9,
            reclaims: 0,
            first_bad_window: None,
            bad_expected: 0,
            bad_actual: 0,
            bad_reconnect: 0,
            bad_reclaims: 0,
            hash: 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// Hand-built green report over the five fault rows.
    fn green_report() -> SoakReport {
        SoakReport {
            config: SoakConfig::short(),
            seed: SOAK_SEED,
            hash: 0xBF58_476D_1CE4_E5B9,
            per_fault: ALL_FAULTS.map(green_row),
            mem_growth_pct: 0.0,
            mem_moved: false,
            mem_bad_window: None,
        }
    }

    #[test]
    fn forged_desync_is_red_and_names_its_minute() {
        let mut row = green_row(Fault::Loss);
        row.check().expect("hand-built row starts green");
        row.desyncs = 1;
        row.first_bad_window = Some(2);
        row.bad_expected = 0xAAAA;
        row.bad_actual = 0xBBBB;
        assert!(!row.is_green());
        let err = row.check().expect_err("desync must fail loudly");
        assert_eq!(
            err,
            SoakError::Desync {
                fault: Fault::Loss,
                window: 2,
                minute: minute_of(2),
                expected: 0xAAAA,
                actual: 0xBBBB,
            },
            "desync names fault, window, minute and both hashes",
        );
        let text = row.summary();
        assert!(
            text.contains("fault=loss"),
            "report names the fault: {text}"
        );
        assert!(
            text.contains("first_bad_minute=0"),
            "report names the first bad minute: {text}",
        );
        assert!(text.contains("RED"), "the row is red: {text}");
        let err_text = err.to_string();
        assert!(
            err_text.contains("loss") && err_text.contains("first_bad_minute=0"),
            "{err_text}"
        );
    }

    #[test]
    fn forged_crash_is_red() {
        let mut row = green_row(Fault::Latency);
        row.crashes = 1;
        row.first_bad_window = Some(0);
        let err = row.check().expect_err("crash must fail loudly");
        assert_eq!(
            err,
            SoakError::Crash {
                fault: Fault::Latency,
                window: 0,
                minute: 0,
            }
        );
    }

    #[test]
    fn forged_slow_reconnect_fails_with_both_sides() {
        let mut row = green_row(Fault::Jitter);
        row.first_bad_window = Some(1);
        row.bad_reconnect = RECONNECT_BAR_TICKS + 40;
        let err = row.check().expect_err("slow reconnect must fail");
        assert_eq!(
            err,
            SoakError::SlowReconnect {
                fault: Fault::Jitter,
                window: 1,
                minute: minute_of(1),
                got: RECONNECT_BAR_TICKS + 40,
                bar: RECONNECT_BAR_TICKS,
            }
        );
    }

    #[test]
    fn forged_slot_loss_fails_with_both_sides() {
        let mut row = green_row(Fault::Disconnect);
        row.first_bad_window = Some(4);
        row.bad_reclaims = 5;
        let err = row.check().expect_err("lost slots must fail");
        assert_eq!(
            err,
            SoakError::SlotLost {
                fault: Fault::Disconnect,
                window: 4,
                minute: minute_of(4),
                got: 5,
                want: DISCONNECT_SLOTS as u32,
            }
        );
    }

    #[test]
    fn forged_memory_drift_names_its_minute() {
        let mut report = green_report();
        report.check().expect("hand-built report starts green");
        report.mem_moved = true;
        report.mem_bad_window = Some(3);
        report.mem_growth_pct = MEM_DRIFT_PCT + 1.0;
        assert!(!report.is_green());
        let err = report.check().expect_err("drift must fail loudly");
        assert_eq!(
            err,
            SoakError::MemoryDrift {
                growth_pct: MEM_DRIFT_PCT + 1.0,
                budget_pct: MEM_DRIFT_PCT,
                window: 3,
                minute: minute_of(3),
            }
        );
        assert!(err.to_string().contains("first_bad_minute=0"));
    }

    #[test]
    fn bad_shapes_are_refused_loudly() {
        assert_eq!(
            run_windows(SOAK_SEED, 0, true),
            Err(SoakError::BadWindows { got: 0 }),
        );
        assert_eq!(
            run_windows(SOAK_SEED, WINDOWS_HOUR + 1, true),
            Err(SoakError::BadWindows {
                got: WINDOWS_HOUR + 1
            }),
        );
        assert_eq!(
            run_windows(SOAK_SEED, WINDOWS_SHORT, false),
            Err(SoakError::DestructionOff),
            "destruction-off is refused, never soaked",
        );
        assert_eq!(
            SoakConfig {
                slots: 8,
                ..SoakConfig::short()
            }
            .validate(),
            Err(SoakError::BadSlots {
                got: 8,
                want: SLOTS
            }),
        );
        assert_eq!(
            SoakConfig {
                legions: 0,
                ..SoakConfig::short()
            }
            .validate(),
            Err(SoakError::BadLegions {
                got: 0,
                want: LEGIONS
            }),
        );
        SoakConfig::short()
            .validate()
            .expect("short config is legal");
        SoakConfig::hour().validate().expect("hour config is legal");
    }

    #[test]
    fn error_strings_name_themselves() {
        let cause = WanError::BadTicks { got: 0 };
        let cases: &[SoakError] = &[
            SoakError::BadWindows { got: 0 },
            SoakError::BadSlots {
                got: 8,
                want: SLOTS,
            },
            SoakError::BadLegions {
                got: 0,
                want: LEGIONS,
            },
            SoakError::DestructionOff,
            SoakError::Desync {
                fault: Fault::Loss,
                window: 2,
                minute: 0,
                expected: 1,
                actual: 2,
            },
            SoakError::Crash {
                fault: Fault::Latency,
                window: 0,
                minute: 0,
            },
            SoakError::SlowReconnect {
                fault: Fault::Jitter,
                window: 1,
                minute: 0,
                got: 340,
                bar: 300,
            },
            SoakError::SlotLost {
                fault: Fault::Disconnect,
                window: 4,
                minute: 0,
                got: 5,
                want: 8,
            },
            SoakError::MemoryDrift {
                growth_pct: 6.0,
                budget_pct: MEM_DRIFT_PCT,
                window: 3,
                minute: 0,
            },
            SoakError::Wan {
                fault: Fault::Reorder,
                window: 3,
                minute: 0,
                cause,
            },
            SoakError::Tick(TickBudgetError::BadTicks { got: 0 }),
        ];
        for error in cases {
            let text = error.to_string();
            assert!(!text.is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = error;
        }
        assert!(SoakError::DestructionOff
            .to_string()
            .contains("destruction"));
    }

    #[test]
    #[ignore = "timing: hour-long online soak, run with --release -- --ignored --test-threads=1"]
    fn hour_long_online_soak_is_green() {
        let report = run_hour().expect("hour soak must converge");
        println!("{}", report.summary());
        assert_eq!(report.config, SoakConfig::hour());
        for (row, fault) in report.per_fault.iter().zip(ALL_FAULTS) {
            assert_eq!(row.fault, fault, "rows keep matrix order");
            assert!(
                row.is_green(),
                "{fault:?}: row must be green\n{}",
                row.summary()
            );
            row.check().expect("every fault row must pass its bars");
        }
        let disconnect = &report.per_fault[4];
        assert_eq!(
            disconnect.reclaims, disconnect.windows,
            "every disconnect window must reclaim its slots",
        );
        report.check().expect("hour soak must pass every gate");
        let second = run_hour().expect("hour soak must replay");
        assert_eq!(report.digest_key(), second.digest_key());
    }
}

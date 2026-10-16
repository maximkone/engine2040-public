//! Seeded chaos suite over the auto-heal table in
//! `docs/systems/observability-heal.md`.
//!
//! Consumer: the A9 gate evidence; subsystem lanes extend [`FAULTS`] as their
//! recovery lands. Data owner: this module owns [`FAULTS`], the seeded
//! injection and the [`ChaosReport`]; `engine-observe` owns the heal
//! registry, `engine-core` owns the seed stream, the proof fixture owns tape
//! truth. Units: ticks (`u64`), attempt counts (`u32`), byte counts
//! (`usize`). Determinism scope: [`run`] walks [`FAULTS`] in order with
//! per-row forked streams, so the same seed always gives the same report on
//! every machine. There is no `Result` on the API: internal failures become
//! loud [`LoudReason`] entries, never silent passes.

use std::fmt;
use std::sync::{Mutex, MutexGuard, OnceLock};

use engine_core::Rng;
use engine_net::wan_soak::{FaultReport, WanError};
use engine_observe::heal::{HealKind, Registry};
use engine_observe::{Event, EventCode, RingBuffer};

use crate::{record, replay, Config, Error, Input, Tape, BODY_COUNT};

/// Proof-fixture tape for chaos: seed 7, 24 ticks, one velocity input.
const CHAOS_SEED: u64 = 7;
/// Tick count of [`chaos_tape`]; small so the default suite stays fast.
const CHAOS_TICKS: u32 = 24;

/// How one [`FAULTS`] row injects its fault on the proof fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// Flip one seeded byte of a known-good tape encoding.
    CorruptTape,
    /// Drop a seeded trailing byte count of a known-good tape encoding.
    TruncateTape,
    /// Submit one seeded non-finite velocity input to the fixture.
    NanInput,
    /// Seeded desync plus resync: a divergent proof-fixture input names the
    /// tick, the WAN matrix holds every checkpoint, the catalog heals.
    NetResync,
    /// Seeded tape mutation proves detection and isolation; the fault then
    /// quarantines with a `heal.failed` record. `kind` is the catalog heal
    /// the owning subsystem will run once it lands; `ships_in` names the row
    /// that ships the real recovery (arena match, GPU, provider adapter,
    /// command service or child process), which this fixture cannot execute.
    Quarantine {
        /// Catalog heal kind the shipping row will run.
        kind: HealKind,
        /// Short label naming the row that ships the real recovery.
        ships_in: &'static str,
    },
}

/// One auto-heal table row mapped to a seeded injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaultRow {
    /// Stable observe fault code.
    pub code: EventCode,
    /// Owning subsystem: the first segment of the code's dotted name.
    pub subsystem: &'static str,
    /// Seeded injection driving detection plus the heal or quarantine record.
    pub inject: FaultKind,
}

/// Every auto-heal table row, in table order.
///
/// Shipped on the proof fixture: the physics NaN row (finite checks reject,
/// the tick quarantines), the assets/save tape rows (digest and version
/// checks reject, the preserved last-good bytes recover) and the net desync
/// row (AH4 resync from the authoritative snapshot, ON4 converge over the
/// WAN matrix, ON11 short-matrix faults one per variant). The remaining rows
/// prove detection plus isolation on the same tape bytes, then quarantine
/// with a `heal.failed` record: their real recovery needs an arena match
/// (brain replan), a GPU (device reconnect, pipeline last-good, quality
/// floor), a provider adapter (scripted fallbacks), the command service
/// (stale-edit brief) or a child process (journal restore), so each names
/// the row that ships its heal instead of faking one on the CPU.
pub static FAULTS: &[FaultRow] = &[
    FaultRow {
        code: EventCode::PHYSICS_BODY_NONFINITE,
        subsystem: "physics",
        inject: FaultKind::NanInput,
    },
    FaultRow {
        code: EventCode::BRAIN_FIGHTER_STUCK,
        subsystem: "brain",
        inject: FaultKind::Quarantine {
            kind: HealKind::RetryTick,
            ships_in: "arena match replan ships heal (brain)",
        },
    },
    FaultRow {
        code: EventCode::NET_CLIENT_DESYNC,
        subsystem: "net",
        inject: FaultKind::NetResync,
    },
    FaultRow {
        code: EventCode::RENDER_DEVICE_LOST,
        subsystem: "render",
        inject: FaultKind::Quarantine {
            kind: HealKind::Reconnect,
            ships_in: "gpu device reconnect ships heal (render)",
        },
    },
    FaultRow {
        code: EventCode::RENDER_PIPELINE_FAILED,
        subsystem: "render",
        inject: FaultKind::Quarantine {
            kind: HealKind::LastGood,
            ships_in: "gpu pipeline last-good ships heal (render)",
        },
    },
    FaultRow {
        code: EventCode::ASSETS_MISSING,
        subsystem: "assets",
        inject: FaultKind::TruncateTape,
    },
    FaultRow {
        code: EventCode::PROVIDER_TIMEOUT,
        subsystem: "provider",
        inject: FaultKind::Quarantine {
            kind: HealKind::Degrade,
            ships_in: "provider scripted fallback ships heal (provider)",
        },
    },
    FaultRow {
        code: EventCode::PROVIDER_MALFORMED,
        subsystem: "provider",
        inject: FaultKind::Quarantine {
            kind: HealKind::ResetBudget,
            ships_in: "provider schema fallback ships heal (provider)",
        },
    },
    FaultRow {
        code: EventCode::COMMANDS_STALE,
        subsystem: "commands",
        inject: FaultKind::Quarantine {
            kind: HealKind::Escalate,
            ships_in: "command service brief ships heal (commands)",
        },
    },
    FaultRow {
        code: EventCode::EDITOR_CRASH,
        subsystem: "editor",
        inject: FaultKind::Quarantine {
            kind: HealKind::Restore,
            ships_in: "child process journal restore ships heal (editor)",
        },
    },
    FaultRow {
        code: EventCode::SAVE_CORRUPT,
        subsystem: "save",
        inject: FaultKind::CorruptTape,
    },
    FaultRow {
        code: EventCode::RENDER_FRAME_OVER_BUDGET,
        subsystem: "render",
        inject: FaultKind::Quarantine {
            kind: HealKind::Degrade,
            ships_in: "presentation quality floor ships heal (render)",
        },
    },
];

/// Loud outcome for one fault: recovery is not claimed silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoudReason {
    /// The retry budget is spent; the object or tick quarantines.
    Quarantined { attempts: u32 },
    /// The harness itself failed; the detail names the broken step.
    InjectionFailed(&'static str),
}

impl fmt::Display for LoudReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Quarantined { attempts } => {
                write!(f, "quarantined after {attempts} attempt(s)")
            }
            Self::InjectionFailed(detail) => write!(f, "injection failed: {detail}"),
        }
    }
}

/// Seeded chaos outcome: every [`FAULTS`] row lands in exactly one list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChaosReport {
    /// Faults that healed; each entry mirrors one `heal.applied` record
    /// with the one-based attempt count.
    pub healed: Vec<(EventCode, u32)>,
    /// Faults that failed loudly; nothing passes silently.
    pub loud: Vec<(EventCode, LoudReason)>,
}

impl ChaosReport {
    /// Attempt count when `code` healed, else `None`.
    #[must_use]
    pub fn healed_attempts(&self, code: EventCode) -> Option<u32> {
        self.healed
            .iter()
            .find(|(candidate, _)| *candidate == code)
            .map(|(_, attempts)| *attempts)
    }

    /// Loud reason when `code` failed loudly, else `None`.
    #[must_use]
    pub fn loud_reason(&self, code: EventCode) -> Option<LoudReason> {
        self.loud
            .iter()
            .find(|(candidate, _)| *candidate == code)
            .map(|(_, reason)| *reason)
    }
}

/// Outcome of one executable row: at most one healed entry or one loud entry.
type RowOutcome = (Option<(EventCode, u32)>, Option<(EventCode, LoudReason)>);

/// Known-good tape the corrupt and truncate rows mutate.
fn chaos_tape() -> Result<Tape, Error> {
    record(
        Config {
            seed: CHAOS_SEED,
            ticks: CHAOS_TICKS,
        },
        &[Input {
            tick: 5,
            body: 1,
            vx: 1.0,
            vy: 2.0,
        }],
    )
}

/// Run one tape-backed row: mutate a known-good encoding with a seeded
/// cut, require detection, recover from the preserved last-good bytes and
/// record the heal. Detection or recovery failures are loud harness
/// findings, never silent passes.
fn run_tape_row(
    code: EventCode,
    kind: HealKind,
    attempts_max: u32,
    mut rng: Rng,
    truncate: bool,
) -> RowOutcome {
    let fail = |detail: &'static str| (None, Some((code, LoudReason::InjectionFailed(detail))));
    let good_bytes = match chaos_tape().and_then(|tape| tape.encode()) {
        Ok(bytes) => bytes,
        Err(_) => return fail("fixture record or encode failed"),
    };
    if good_bytes.is_empty() {
        return fail("fixture encoding is empty");
    }
    let mut bad_bytes = good_bytes.clone();
    if truncate {
        let cut = 1 + (rng.next_u64() as usize) % bad_bytes.len();
        bad_bytes.truncate(bad_bytes.len() - cut);
    } else {
        let index = (rng.next_u64() as usize) % bad_bytes.len();
        bad_bytes[index] ^= 0xFF;
    }
    // Detection: corrupt bytes must fail decode or diverge on replay.
    let detected = match Tape::decode(&bad_bytes) {
        Err(_) => true,
        Ok(tape) => replay(&tape).is_err(),
    };
    if !detected {
        return fail("mutation escaped decode and replay");
    }
    // Recovery to known-good: the preserved bytes decode and replay clean.
    if !Tape::decode(&good_bytes).is_ok_and(|tape| replay(&tape).is_ok()) {
        return fail("last-good recovery failed");
    }
    let mut registry = Registry::new();
    let mut ring = RingBuffer::new(16);
    registry.register(code, attempts_max, kind);
    match registry.on_fault(code, 0, &mut ring) {
        Ok(decision) => {
            let recorded = ring.query(Some(EventCode::HEAL_APPLIED)).len() == 1;
            if !decision.is_quarantine() && recorded {
                (Some((code, decision.attempt)), None)
            } else if decision.is_quarantine()
                && ring.query(Some(EventCode::HEAL_FAILED)).len() == 1
            {
                (
                    None,
                    Some((
                        code,
                        LoudReason::Quarantined {
                            attempts: decision.attempt,
                        },
                    )),
                )
            } else {
                fail("heal record mismatch")
            }
        }
        Err(_) => fail("fault code not registered"),
    }
}

/// Run the non-finite input row: a seeded NaN velocity must be rejected by
/// the fixture's finite checks, then the fault quarantines loudly because
/// the proof fixture keeps no authority snapshot to resimulate from.
fn run_nan_row(mut rng: Rng) -> RowOutcome {
    let code = EventCode::PHYSICS_BODY_NONFINITE;
    let fail = |detail: &'static str| (None, Some((code, LoudReason::InjectionFailed(detail))));
    let body = (rng.next_u64() % u64::from(BODY_COUNT)) as u32;
    let bad = Input {
        tick: 3,
        body,
        vx: f32::NAN,
        vy: 0.0,
    };
    if record(
        Config {
            seed: CHAOS_SEED,
            ticks: 8,
        },
        &[bad],
    )
    .is_ok()
    {
        return fail("non-finite input was accepted");
    }
    let mut registry = Registry::new();
    let mut ring = RingBuffer::new(16);
    // Zero budget: with no validated snapshot plus resimulation available,
    // the failing tick quarantines at once.
    registry.register(code, 0, HealKind::RetryTick);
    match registry.on_fault(code, u64::from(bad.tick), &mut ring) {
        Ok(decision) => {
            let recorded = ring.query(Some(EventCode::HEAL_FAILED)).len() == 1;
            if decision.is_quarantine() && recorded {
                (
                    None,
                    Some((
                        code,
                        LoudReason::Quarantined {
                            attempts: decision.attempt,
                        },
                    )),
                )
            } else {
                fail("non-finite input healed without a snapshot")
            }
        }
        Err(_) => fail("fault code not registered"),
    }
}

/// Ticks stepped per WAN fault inside the net row: scaled down from the
/// ON11 600-tick short matrix so the default chaos suite stays fast (the
/// row runs one matrix, and `same_seed_twice_gives_same_report` holds the
/// x2 bar). The canonical 600-tick short proof lives in
/// `online_soak::run_short`; this row holds the same shape (all five
/// faults, one authoritative hash).
const NET_TICKS: u32 = 36;

/// Bounded matrix attempts over one seeded stream: only the wall-clock
/// resim sample varies run to run, so a poisoned sample retries while a
/// deterministic refusal fails at once. The seed never advances, so a
/// green sample is byte-identical on every attempt.
fn run_matrix_stable(seed: u64) -> Result<[FaultReport; 5], WanError> {
    const ATTEMPTS: u32 = 3;
    let mut attempt = 0;
    loop {
        match engine_net::wan_soak::run_matrix(seed, NET_TICKS) {
            Ok(matrix) => return Ok(matrix),
            Err(error) if attempt + 1 < ATTEMPTS && matches!(error, WanError::OverResim { .. }) => {
                attempt += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Run the net desync row: a seeded divergent input must diverge from the
/// authoritative proof-fixture tape (detection), adopting the authoritative
/// checkpoints must replay clean (AH4 resync convergence), the seeded WAN
/// matrix must hold every checkpoint (ON4 converge over the ON11
/// short-matrix faults, with bounded attempts over one stream so only a
/// poisoned wall-clock resim sample retries), and the catalog row must
/// record `heal.applied`. Any broken step is a loud harness finding, never
/// a silent pass.
fn run_net_row(mut rng: Rng) -> RowOutcome {
    let code = EventCode::NET_CLIENT_DESYNC;
    let fail = |detail: &'static str| (None, Some((code, LoudReason::InjectionFailed(detail))));
    // Seeded divergent input, kept clear of the authority input at (5, 1).
    let mut diverge_tick = 3 + (rng.next_u64() % u64::from(CHAOS_TICKS - 6)) as u32;
    if diverge_tick >= CHAOS_TICKS {
        diverge_tick = CHAOS_TICKS - 1;
    }
    let mut body = (rng.next_u64() % u64::from(BODY_COUNT)) as u32;
    if diverge_tick == 5 && body == 1 {
        body = (body + 1) % BODY_COUNT;
    }
    let mut vx = (rng.next_u64() % 4001) as f32 - 2000.0;
    let vy = (rng.next_u64() % 4001) as f32 - 2000.0;
    if body == 1 && vx == 1.0 && vy == 2.0 {
        vx += 3.0;
    }
    if body != 1 && vx == 0.5 && vy == 0.0 {
        vx += 3.0;
    }
    let base = Input {
        tick: 5,
        body: 1,
        vx: 1.0,
        vy: 2.0,
    };
    let extra = Input {
        tick: diverge_tick,
        body,
        vx,
        vy,
    };
    let mut inputs = vec![base, extra];
    inputs.sort_by_key(|input| (input.tick, input.body));
    let authority = match record(
        Config {
            seed: CHAOS_SEED,
            ticks: CHAOS_TICKS,
        },
        &[base],
    ) {
        Ok(tape) => tape,
        Err(_) => return fail("authority record failed"),
    };
    let divergent = match record(
        Config {
            seed: CHAOS_SEED,
            ticks: CHAOS_TICKS,
        },
        &inputs,
    ) {
        Ok(tape) => tape,
        Err(_) => return fail("divergent record failed"),
    };
    // Detection: the trajectories must first differ at or after the
    // earliest input. Compare `legacy` only: `state` folds the input list
    // itself, so it differs from tick zero by construction.
    let first_diff = authority
        .checkpoints
        .iter()
        .zip(divergent.checkpoints.iter())
        .position(|(a, b)| a.legacy != b.legacy);
    let earliest = 5.min(diverge_tick) as usize;
    if first_diff.is_none_or(|index| index < earliest) {
        return fail("divergence escaped detection");
    }
    // AH4 convergence: resyncing to the authoritative snapshot replays clean.
    let mut resynced = divergent;
    resynced.inputs.clone_from(&authority.inputs);
    resynced.checkpoints.clone_from(&authority.checkpoints);
    if replay(&resynced).is_err() {
        return fail("authority resync failed");
    }
    if replay(&authority).is_err() {
        return fail("authority replay failed");
    }
    // ON4 converge over the ON11 short-matrix faults: all five WAN variants
    // hold the authoritative checkpoint from one seeded stream (`run_matrix`
    // already checks every per-fault bar; the x2 bar lives in
    // `same_seed_twice_gives_same_report`).
    let wan_seed = rng.next_u64();
    let matrix = match run_matrix_stable(wan_seed) {
        Ok(matrix) => matrix,
        Err(_) => return fail("wan matrix refused"),
    };
    let clean = match engine_net::tick_budget::record_tape(wan_seed, NET_TICKS) {
        Ok(tape) => tape,
        Err(_) => return fail("clean siege tape refused"),
    };
    if matrix.iter().any(|report| report.hash != clean.hash) {
        return fail("wan checkpoint diverged");
    }
    // Catalog heal: AI takeover with the shipped 4-attempt budget, deferred
    // to the tick boundary, recorded once as `heal.applied`.
    let mut registry = Registry::new();
    let mut ring = RingBuffer::new(16);
    registry.register(code, 4, HealKind::AiTakeover);
    match registry.on_fault(code, u64::from(diverge_tick), &mut ring) {
        Ok(decision) => {
            let recorded = ring.query(Some(EventCode::HEAL_APPLIED)).len() == 1;
            if !decision.is_quarantine()
                && decision.kind == HealKind::AiTakeover
                && decision.applies_at_tick
                && decision.attempt == 1
                && recorded
            {
                (Some((code, decision.attempt)), None)
            } else {
                fail("heal record mismatch")
            }
        }
        Err(_) => fail("fault code not registered"),
    }
}

/// Run one unshipped-subsystem row: prove the fault is detectable on the
/// proof fixture with a seeded tape mutation, prove the failure is isolated
/// (the preserved last-good bytes still decode and replay clean), then
/// quarantine loudly with a `heal.failed` record.
///
/// The quarantine is honest, never a CPU stand-in for subsystem work: the
/// `ships_in` label names the row that ships the real recovery (arena match,
/// GPU, provider adapter, command service or child process), which this
/// fixture cannot execute. The registry runs at zero budget with the
/// catalog heal kind, so the first occurrence quarantines at once. Any
/// broken step is a loud harness finding, never a silent pass.
fn run_quarantine_row(
    code: EventCode,
    kind: HealKind,
    ships_in: &'static str,
    mut rng: Rng,
) -> RowOutcome {
    let fail = |detail: &'static str| (None, Some((code, LoudReason::InjectionFailed(detail))));
    let good_bytes = match chaos_tape().and_then(|tape| tape.encode()) {
        Ok(bytes) => bytes,
        Err(_) => return fail("fixture record or encode failed"),
    };
    if good_bytes.is_empty() {
        return fail("fixture encoding is empty");
    }
    // Seeded detection: the stream picks the corrupt shape, so sibling rows
    // mutate differently while every seed stays reproducible.
    let mut bad_bytes = good_bytes.clone();
    if rng.next_u64().is_multiple_of(2) {
        let index = (rng.next_u64() as usize) % bad_bytes.len();
        bad_bytes[index] ^= 0xFF;
    } else {
        let cut = 1 + (rng.next_u64() as usize) % bad_bytes.len();
        bad_bytes.truncate(bad_bytes.len() - cut);
    }
    let detected = match Tape::decode(&bad_bytes) {
        Err(_) => true,
        Ok(tape) => replay(&tape).is_err(),
    };
    if !detected {
        return fail("mutation escaped decode and replay");
    }
    // Isolation: the preserved last-good bytes still decode and replay
    // clean, so the quarantine covers exactly the injected fault.
    if !Tape::decode(&good_bytes).is_ok_and(|tape| replay(&tape).is_ok()) {
        return fail("last-good isolation failed");
    }
    let tick = rng.next_u64() % u64::from(CHAOS_TICKS);
    let mut registry = Registry::new();
    let mut ring = RingBuffer::new(16);
    // Loud fault event naming the shipping row; the registry's `heal.failed`
    // below is the quarantine receipt.
    let label = code.name().unwrap_or("unknown");
    ring.push(Event::new(
        code,
        tick,
        &format!("{label} quarantined: {ships_in}"),
    ));
    // Zero budget: with the owning subsystem unlanded, the isolated fault
    // quarantines at once instead of healing into a guess.
    registry.register(code, 0, kind);
    match registry.on_fault(code, tick, &mut ring) {
        Ok(decision) => {
            let failed_once = ring.query(Some(EventCode::HEAL_FAILED)).len() == 1;
            let applied_none = ring.query(Some(EventCode::HEAL_APPLIED)).is_empty();
            if decision.is_quarantine() && failed_once && applied_none {
                (
                    None,
                    Some((
                        code,
                        LoudReason::Quarantined {
                            attempts: decision.attempt,
                        },
                    )),
                )
            } else {
                fail("quarantine record mismatch")
            }
        }
        Err(_) => fail("fault code not registered"),
    }
}

/// Serial gate over the wall-clock-sensitive net section.
///
/// The net row folds a wall-clock resim budget into its verdict, so two
/// suites stepping siege worlds side by side poison each other's samples
/// under parallel test threads. [`run`] and the soak runner each hold this
/// gate for one call, keeping a single timing sample live per process;
/// simulation state stays thread-local and deterministic.
pub(crate) fn hold_serial() -> MutexGuard<'static, ()> {
    static SERIAL: OnceLock<Mutex<()>> = OnceLock::new();
    let serial = SERIAL.get_or_init(|| Mutex::new(()));
    match serial.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Inject every [`FAULTS`] row with per-row forked streams of `seed` and
/// report what healed against the proof fixture versus what failed loudly.
///
/// Executable today: NaN input quarantines (`physics`), corrupt and
/// truncated tapes recover from preserved last-good bytes (`save` with a
/// retry, `assets` with a fallback drop), and the net desync row resyncs
/// from the authoritative snapshot while the WAN matrix holds every
/// checkpoint (`net` AI takeover). The other eight rows detect plus isolate
/// on the same tape bytes, then quarantine with a `heal.failed` record that
/// names the shipping row: no [`LoudReason`] but quarantine and harness
/// failure remains, so nothing passes silently and nothing is unsupported.
#[must_use]
pub fn run(seed: u64) -> ChaosReport {
    // One timing sample live per process: parallel callers would otherwise
    // step siege worlds side by side and poison the wall-clock resim bar.
    let _serial = hold_serial();
    let base = Rng::new(seed);
    let mut healed = Vec::with_capacity(FAULTS.len());
    let mut loud = Vec::with_capacity(FAULTS.len());
    for (position, row) in FAULTS.iter().enumerate() {
        let rng = base.fork(position as u64);
        let (healed_one, loud_one) = match row.inject {
            FaultKind::Quarantine { kind, ships_in } => {
                run_quarantine_row(row.code, kind, ships_in, rng)
            }
            // Save or tape corrupt: preserve the original, refuse the bad
            // bytes, recover the validated previous bytes.
            FaultKind::CorruptTape => run_tape_row(row.code, HealKind::RetryTick, 2, rng, false),
            // Asset missing or corrupt: digest and format checks reject,
            // the last-good bytes stay in service.
            FaultKind::TruncateTape => run_tape_row(row.code, HealKind::DropJob, 2, rng, true),
            FaultKind::NanInput => run_nan_row(rng),
            // Net desync: AH4 resync plus ON4/ON11 checkpoint convergence.
            FaultKind::NetResync => run_net_row(rng),
        };
        healed.extend(healed_one);
        loud.extend(loud_one);
    }
    ChaosReport { healed, loud }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Quarantined rows: code plus the subsystem label the loud fault event
    /// carries. The net/assets/save rows heal, so they are absent; the
    /// physics row quarantines for lack of an authority snapshot.
    const QUARANTINED_ROWS: [(EventCode, &str); 9] = [
        (EventCode::PHYSICS_BODY_NONFINITE, "physics"),
        (EventCode::BRAIN_FIGHTER_STUCK, "brain"),
        (EventCode::RENDER_DEVICE_LOST, "render"),
        (EventCode::RENDER_PIPELINE_FAILED, "render"),
        (EventCode::PROVIDER_TIMEOUT, "provider"),
        (EventCode::PROVIDER_MALFORMED, "provider"),
        (EventCode::COMMANDS_STALE, "commands"),
        (EventCode::EDITOR_CRASH, "editor"),
        (EventCode::RENDER_FRAME_OVER_BUDGET, "render"),
    ];

    #[test]
    fn faults_cover_every_auto_heal_row() {
        let codes: Vec<EventCode> = FAULTS.iter().map(|row| row.code).collect();
        assert_eq!(
            codes,
            vec![
                EventCode::PHYSICS_BODY_NONFINITE,
                EventCode::BRAIN_FIGHTER_STUCK,
                EventCode::NET_CLIENT_DESYNC,
                EventCode::RENDER_DEVICE_LOST,
                EventCode::RENDER_PIPELINE_FAILED,
                EventCode::ASSETS_MISSING,
                EventCode::PROVIDER_TIMEOUT,
                EventCode::PROVIDER_MALFORMED,
                EventCode::COMMANDS_STALE,
                EventCode::EDITOR_CRASH,
                EventCode::SAVE_CORRUPT,
                EventCode::RENDER_FRAME_OVER_BUDGET,
            ]
        );
        // Heal-protocol codes are records, never injected faults.
        for protocol in [
            EventCode::SIM_TICK,
            EventCode::HEAL_APPLIED,
            EventCode::HEAL_FAILED,
        ] {
            assert!(!codes.contains(&protocol));
        }
        // Subsystem labels match the code registry's first segment.
        for row in FAULTS {
            assert_eq!(
                Some(row.subsystem),
                row.code.name().and_then(|name| name.split('.').next())
            );
        }
    }

    #[test]
    fn corrupt_tape_heals_with_record() {
        // Per row: the net resync plus both tape rows heal on the first
        // attempt; none fails loudly. Order follows `FAULTS`.
        for seed in [0, 7, 11] {
            let report = run(seed);
            assert_eq!(
                report.healed,
                vec![
                    (EventCode::NET_CLIENT_DESYNC, 1),
                    (EventCode::ASSETS_MISSING, 1),
                    (EventCode::SAVE_CORRUPT, 1),
                ],
                "seed {seed}"
            );
            assert_eq!(report.healed_attempts(EventCode::ASSETS_MISSING), Some(1));
            assert_eq!(report.healed_attempts(EventCode::SAVE_CORRUPT), Some(1));
            assert_eq!(report.loud_reason(EventCode::ASSETS_MISSING), None);
            assert_eq!(report.loud_reason(EventCode::SAVE_CORRUPT), None);
        }
    }

    #[test]
    fn net_desync_heals_with_record_and_convergence() {
        // Shipped recovery (AH4 resync, ON4 converge, ON11 short matrix):
        // the seeded injection heals on the first attempt with a
        // `heal.applied` record and converged checkpoints, never loudly.
        for seed in [0, 7, 11] {
            let report = run(seed);
            assert_eq!(
                report.healed_attempts(EventCode::NET_CLIENT_DESYNC),
                Some(1),
                "seed {seed}"
            );
            assert_eq!(
                report.loud_reason(EventCode::NET_CLIENT_DESYNC),
                None,
                "seed {seed}"
            );
        }
    }

    #[test]
    fn nan_input_quarantines_loudly() {
        for seed in [0, 7, 11] {
            let report = run(seed);
            assert_eq!(
                report.loud_reason(EventCode::PHYSICS_BODY_NONFINITE),
                Some(LoudReason::Quarantined { attempts: 1 }),
                "seed {seed}"
            );
            assert_eq!(
                report.healed_attempts(EventCode::PHYSICS_BODY_NONFINITE),
                None,
                "seed {seed}"
            );
        }
    }

    #[test]
    fn same_seed_twice_gives_same_report() {
        for seed in [0, 1, 7, 42, 999_983] {
            assert_eq!(run(seed), run(seed), "seed {seed}");
        }
    }

    #[test]
    fn unshipped_rows_quarantine_loudly_with_record_never_silent() {
        // Detection plus isolation on the tape bytes, then a zero-budget
        // quarantine with one `heal.failed` record: the fault event names
        // the shipping row, so nothing heals into a guess and nothing is
        // unsupported. Order follows `FAULTS`.
        for seed in [0, 7, 11] {
            let report = run(seed);
            for (code, subsystem) in QUARANTINED_ROWS {
                assert_eq!(
                    report.loud_reason(code),
                    Some(LoudReason::Quarantined { attempts: 1 }),
                    "seed {seed} code {code:?}"
                );
                assert_eq!(
                    report.healed_attempts(code),
                    None,
                    "seed {seed} code {code:?}"
                );
                let name = code.name().unwrap_or("?");
                assert!(
                    name.starts_with(subsystem),
                    "seed {seed} {name} carries {subsystem}"
                );
            }
        }
    }

    #[test]
    fn no_row_reports_unsupported() {
        // Three tape/net heals plus nine quarantines partition the table on
        // every seed: no unsupported and no harness failure remains.
        for seed in [0, 7, 11] {
            let report = run(seed);
            assert_eq!(report.healed.len(), 3, "seed {seed}");
            assert_eq!(report.loud.len(), FAULTS.len() - 3, "seed {seed}");
            for (_, reason) in &report.loud {
                assert_eq!(
                    *reason,
                    LoudReason::Quarantined { attempts: 1 },
                    "seed {seed}"
                );
            }
        }
    }

    #[test]
    fn every_row_heals_or_fails_loudly_never_silent() {
        for seed in [0, 7, 11] {
            let report = run(seed);
            assert_eq!(
                report.healed.len() + report.loud.len(),
                FAULTS.len(),
                "seed {seed}"
            );
            let mut seen: Vec<EventCode> = report
                .healed
                .iter()
                .map(|(code, _)| *code)
                .chain(report.loud.iter().map(|(code, _)| *code))
                .collect();
            seen.sort();
            let mut wanted: Vec<EventCode> = FAULTS.iter().map(|row| row.code).collect();
            wanted.sort();
            assert_eq!(seen, wanted, "seed {seed}");
            // Loud reasons stay within the observe message bound.
            for (_, reason) in &report.loud {
                assert!(format!("{reason}").len() <= 256, "seed {seed}");
            }
        }
    }
}

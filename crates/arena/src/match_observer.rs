//! match_observer: per-tick match outcomes and canonical match bytes.
//!
//! Owner: engine-arena; consumer: match observers, replay tools, prefix and
//! resume suffix checks, playtest forks. These entry points drive a [`Sim`]
//! with the match track armed and hand back what the track resolved each tick.
//! Observed only: nothing here feeds the [`run`](crate::run) checkpoints, so the
//! legacy golden stays frozen. Units: fixed ticks. Determinism scope: same
//! config plus tape plus intake version gives identical rows. Error policy:
//! invalid input maps to an empty vector, never partial, never panics.

use engine_combat::exchange::ExchangeEvent;
use engine_combat::sweep::SweepEvent;
use engine_input::FighterTape;

use super::{run_sim_versioned, ArenaConfig, Sim, MATCH_WIRE_VERSION_V0, MATCH_WIRE_VERSION_V1};

/// Per-tick GP1/GP2 match outcomes for a valid run; empty on invalid
/// input. One row per tick, so the length matches [`run_events`](crate::run_events). The
/// match track resolves beside the fight path every tick over the landed
/// resolvers (exchange plus sweep); these bytes are observed here but never
/// feed [`run`](crate::run) checkpoints, so the legacy golden stays frozen until the
/// orchestrator migrates authority.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TickMatchEvents {
    /// Zero-based fixed tick these outcomes belong to.
    pub tick: u32,
    /// Parry/riposte/clash answers for the tick, in fighter-ID order.
    pub exchange: Vec<ExchangeEvent>,
    /// Sweep contacts for the tick, in ID order.
    pub sweep: Vec<SweepEvent>,
}

/// Per-tick legacy (V0) match outcomes for a valid run; empty on invalid
/// input. One [`TickMatchEvents`] per tick, so the length matches
/// [`run_events`](crate::run_events). The track resolves behind the legacy intake flag: every
/// press reaches the resolvers untouched.
///
/// Consumer: match observers, replay tools, prefix/resume suffix checks.
/// Data owner: the caller owns the returned rows (fresh `Vec`s per tick).
/// Units: fixed ticks at [`TICK_HZ`](crate::TICK_HZ). Threading: caller thread only, no
/// shared state. Determinism scope: same config plus tape twice gives
/// identical rows on the same machine. Error policy: invalid config or
/// tape maps to an empty vector, never partial, never panics. Observed
/// only: these rows never feed [`run`](crate::run) checkpoints. The phased spelling is
/// [`run_match_events_v1`].
///
/// # Example
///
/// ```rust
/// use engine_arena::{ArenaConfig, run_match_events};
/// use engine_input::FighterTape;
///
/// let config = ArenaConfig::try_new(7, 8, 2, 20.0).expect("valid config");
/// let rows = run_match_events(&config, &FighterTape::default());
/// assert_eq!(rows.len(), 8);
/// assert_eq!(rows[0].tick, 0);
/// assert_eq!(run_match_events(&config, &FighterTape::default()), rows);
/// ```
pub fn run_match_events(config: &ArenaConfig, tape: &FighterTape) -> Vec<TickMatchEvents> {
    run_match_events_versioned(config, tape, MATCH_WIRE_VERSION_V0)
}

/// Per-tick match outcomes behind an explicit intake version flag
/// ([`MATCH_WIRE_VERSION_V0`] legacy, [`MATCH_WIRE_VERSION_V1`] phased);
/// empty on invalid input or an unknown version. Same rows as
/// [`run_match_events`] for V0, byte-identical across reruns. Observed
/// only, never hashed.
///
/// Consumer: playtest forks and replay diagnostics pinning the intake path.
/// Units: fixed ticks. Determinism scope: same config plus tape plus
/// version gives identical rows.
pub fn run_match_events_versioned(
    config: &ArenaConfig,
    tape: &FighterTape,
    version: u32,
) -> Vec<TickMatchEvents> {
    run_sim_versioned(config, tape, false, true, version)
        .map(|sim| sim.match_log)
        .unwrap_or_default()
}

/// Per-tick phased (V1) match outcomes for a valid run; empty on invalid
/// input. Same shape as [`run_match_events`], but the track resolves behind
/// the phased intake flag: presses gate through the phased rows, so startup
/// and endlag ticks stay quiet where the legacy observer answers. Observed
/// only, never hashed.
///
/// Consumer, ownership, units, threading, determinism and error policy match
/// [`run_match_events`]; only the intake flag differs. Diagnostics and
/// replay only: these rows never feed [`run`](crate::run) checkpoints.
///
/// # Example
///
/// ```rust
/// use engine_arena::{ArenaConfig, run_match_events_v1};
/// use engine_input::FighterTape;
///
/// let config = ArenaConfig::try_new(7, 8, 2, 20.0).expect("valid config");
/// let rows = run_match_events_v1(&config, &FighterTape::default());
/// assert_eq!(rows.len(), 8);
/// assert_eq!(run_match_events_v1(&config, &FighterTape::default()), rows);
/// ```
pub fn run_match_events_v1(config: &ArenaConfig, tape: &FighterTape) -> Vec<TickMatchEvents> {
    run_match_events_versioned(config, tape, MATCH_WIRE_VERSION_V1)
}

/// Canonical match bytes for a valid run, one row per tick; empty on
/// invalid input. Each row holds the post-tick track bytes (the step
/// advances the clock after resolving), identical across reruns.
/// Diagnostics and replay only: these bytes never feed [`run`](crate::run)
/// checkpoints, so the legacy golden stays frozen.
///
/// Consumer, ownership, units, threading, determinism and error policy match
/// [`run_match_events`]; rows are the canonical byte form of the same ticks.
///
/// # Example
///
/// ```rust
/// use engine_arena::{ArenaConfig, run_match_bytes};
/// use engine_input::FighterTape;
///
/// let config = ArenaConfig::try_new(7, 8, 2, 20.0).expect("valid config");
/// let rows = run_match_bytes(&config, &FighterTape::default());
/// assert_eq!(rows.len(), 8);
/// assert_eq!(run_match_bytes(&config, &FighterTape::default()), rows);
/// ```
pub fn run_match_bytes(config: &ArenaConfig, tape: &FighterTape) -> Vec<Vec<u8>> {
    run_match_bytes_versioned(config, tape, MATCH_WIRE_VERSION_V0)
}

/// Canonical match bytes behind an explicit intake version flag
/// ([`MATCH_WIRE_VERSION_V0`] legacy, [`MATCH_WIRE_VERSION_V1`] phased),
/// one row per tick; empty on invalid input or an unknown version. Each row
/// holds the post-tick track bytes, identical across reruns. Diagnostics
/// and replay only: never hashed.
///
/// Consumer: replay diagnostics pinning the intake path. Units: fixed
/// ticks. Determinism scope: same config plus tape plus version gives
/// identical rows.
pub fn run_match_bytes_versioned(
    config: &ArenaConfig,
    tape: &FighterTape,
    version: u32,
) -> Vec<Vec<u8>> {
    let mut sim = match Sim::build_versioned(config, tape, false, true, version) {
        Some(sim) => sim,
        None => return Vec::new(),
    };
    match_bytes_rows(&mut sim, config, tape)
}

/// Canonical phased (V1) match bytes for a valid run, one row per tick;
/// empty on invalid input. Same rows as [`run_match_bytes`], resolved behind
/// the phased intake flag. Diagnostics and replay only: never hashed.
///
/// Consumer, ownership, units, threading, determinism and error policy match
/// [`run_match_bytes`]; only the intake flag differs.
///
/// # Example
///
/// ```rust
/// use engine_arena::{ArenaConfig, run_match_bytes_v1};
/// use engine_input::FighterTape;
///
/// let config = ArenaConfig::try_new(7, 8, 2, 20.0).expect("valid config");
/// let rows = run_match_bytes_v1(&config, &FighterTape::default());
/// assert_eq!(rows.len(), 8);
/// assert_eq!(run_match_bytes_v1(&config, &FighterTape::default()), rows);
/// ```
pub fn run_match_bytes_v1(config: &ArenaConfig, tape: &FighterTape) -> Vec<Vec<u8>> {
    run_match_bytes_versioned(config, tape, MATCH_WIRE_VERSION_V1)
}

/// Drain one built observer sim tick by tick into canonical rows; shared by
/// both match-bytes entrypoints so the flag is the only difference.
fn match_bytes_rows(sim: &mut Sim, config: &ArenaConfig, tape: &FighterTape) -> Vec<Vec<u8>> {
    let mut rows = Vec::with_capacity(config.ticks as usize);
    while sim.tick < config.ticks {
        if sim.step(tape).is_none() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        sim.match_track.append_bytes(&mut bytes);
        // Locked ripostes resolve out-of-band (dodge-bit attempts, see
        // Sim::step); tag them the way the call-site library's own tapes
        // do (40 plus slots plus damage).
        if let Some(row) = sim.match_log.last() {
            for event in &row.exchange {
                if let ExchangeEvent::Riposte {
                    attacker,
                    victim,
                    damage,
                } = *event
                {
                    bytes.extend_from_slice(&[40, attacker, victim]);
                    bytes.extend_from_slice(&damage.to_le_bytes());
                }
            }
        }
        rows.push(bytes);
    }
    rows
}

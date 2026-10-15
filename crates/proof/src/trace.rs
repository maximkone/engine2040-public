//! Retroactive logging: replay a tape with verbose tracing.
//!
//! Consumer: `BUG1` bundles and `A9` trace evidence; agents read
//! [`TraceReport`] through [`TraceReport::observe_events`] and
//! [`TraceReport::digest`]. Data owner: this module owns [`SimEvent`],
//! [`TraceReport`], [`replay_verbose`] and [`replay_summary`]; the proof
//! fixture owns tape truth (`crate::record`/`crate::replay`), and
//! `engine-observe` owns the event-code registry and digests. Units: ticks
//! (`u32`, zero-based), byte budgets (`usize`). Determinism scope: the same
//! tape bytes always give the same events and the same [`TraceReport`]
//! `summary_hash` on every machine; wall-clock timing is measured, never
//! hashed. Error type: [`Error`]; corrupt tapes fail, never panic.
//!
//! # Covered-event scope
//!
//! Regenerated here (deterministic simulation data, safe to replay):
//!
//! - `sim.tick` ([`EventCode::SIM_TICK`]): exactly one [`SimEvent`] per tick,
//!   in tick order, with `detail` holding the tick's `legacy`/`state`
//!   checkpoint hex plus the count of velocity inputs applied at that tick.
//!   Input values stay in the tape; the checkpoint hashes already fold them
//!   in, so the count keeps the detail bounded while the hash stays
//!   input-sensitive.
//!
//! Never regenerated here (out of scope by design — OS/GPU/provider timings
//! and other subsystems have no deterministic fixture data to replay):
//!
//! - `physics.body.nonfinite`, `brain.fighter.stuck`, `net.client.desync`,
//!   `render.device.lost`, `render.pipeline.failed`, `assets.missing`,
//!   `provider.timeout`, `provider.malformed`, `commands.stale`,
//!   `editor.crash`, `save.corrupt`, `render.frame.over_budget`,
//!   `heal.applied`, `heal.failed`.
//!
//! A `replay_verbose` report therefore contains only `sim.tick` events; the
//! test `covered_scope_is_sim_tick_only` locks that in.
//!
//! # Tracing overhead (measured, not zero-cost)
//!
//! Tracing is not free: [`replay_verbose`] retains one [`SimEvent`] (tick,
//! code, formatted `String` detail) per tick plus the canonical hash bytes,
//! while [`replay_summary`] (the live, non-verbose path) folds the same
//! canonical bytes and drops each formatted detail without retaining any
//! event. Both paths run the same two simulation passes (see below), so the
//! A/B ratio below isolates the tracing allocation cost.
//!
//! Measured 2026-09-23 on this Windows workstation, debug build, proof
//! fixture (seed 7, 24 ticks, 1 input), 9 samples after 1 warmup run each,
//! three runs: traced means 4.90/5.07/5.13 ms, untraced means 4.77/5.12/5.21
//! ms, ratios traced/untraced 1.03x/0.99x/0.98x. The two simulation passes
//! dominate (~5 ms total), so the per-tick event allocation does not move the
//! needle at this fixture size; the ratio sits at parity within run-to-run
//! noise and must be re-measured on larger fixtures before any performance
//! claim. No zero-cost claim is made: tracing retains one heap `String` per
//! tick plus the event `Vec`, while the live path drops each detail.
//!
//! # Implementation note
//!
//! Both entry points first call [`replay`], which owns the authoritative
//! validation, version-gate and first-divergence precedence, then call
//! [`record`] once to rebuild the checkpoints the events and the hash are
//! derived from. Two simulation passes keep error precedence identical to a
//! plain replay at the cost of one extra pass.

use engine_core::hash_bytes;
use engine_observe::{digest, Event, EventCode};

use crate::{record, replay, Checkpoint, Config, Error, Tape, FIXTURE_VERSION};

/// One regenerated deterministic simulation event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimEvent {
    /// Zero-based tick the event belongs to.
    pub tick: u32,
    /// Stable observe registry code; always [`EventCode::SIM_TICK`] today.
    pub code: EventCode,
    /// Bounded human/agent-readable detail: `legacy=<hex> state=<hex>
    /// inputs=<n>`. Well under [`engine_observe::MAX_MESSAGE_LEN`].
    pub detail: String,
}

impl SimEvent {
    /// Convert to an `engine-observe` diagnostic event for digests and bundles.
    #[must_use]
    pub fn to_observe(&self) -> Event {
        Event::new(self.code, u64::from(self.tick), &self.detail)
    }
}

/// Verbose replay outcome: every covered event plus the live-matching hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceReport {
    /// One [`EventCode::SIM_TICK`] event per tick, in tick order.
    pub events: Vec<SimEvent>,
    /// Canonical hash over fixture context plus every event; identical to
    /// what [`replay_summary`] returns for the same tape.
    pub summary_hash: u64,
}

impl TraceReport {
    /// Oldest-first `engine-observe` events for digests and bug bundles.
    #[must_use]
    pub fn observe_events(&self) -> Vec<Event> {
        self.events.iter().map(SimEvent::to_observe).collect()
    }

    /// Bounded agent digest over the traced events.
    #[must_use]
    pub fn digest(&self, budget_bytes: usize) -> String {
        digest(&self.observe_events(), budget_bytes)
    }
}

/// Detail for one tick: checkpoint hex plus the applied-input count.
///
/// Fixed shape (`legacy=<16 hex> state=<16 hex> inputs=<n>`), so details stay
/// far under the observe message bound and hash identically on every machine.
fn detail_for(checkpoint: Checkpoint, inputs: usize) -> String {
    format!(
        "legacy={:016x} state={:016x} inputs={inputs}",
        checkpoint.legacy, checkpoint.state
    )
}

/// Canonical bytes for one event: tick, code and length-prefixed detail.
fn append_canonical(out: &mut Vec<u8>, tick: u32, code: EventCode, detail: &str) {
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&code.raw().to_le_bytes());
    out.extend_from_slice(&(detail.len() as u64).to_le_bytes());
    out.extend_from_slice(detail.as_bytes());
}

/// Canonical hash: fixture context plus every tick event in tick order.
///
/// Both entry points fold these exact bytes, so the traced report and the
/// live summary agree by construction rather than by string comparison.
fn hash_for(config: Config, checkpoints: &[Checkpoint], counts: &[usize]) -> u64 {
    let mut bytes = Vec::with_capacity(16 + checkpoints.len() * 64);
    bytes.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
    bytes.extend_from_slice(&config.seed.to_le_bytes());
    bytes.extend_from_slice(&config.ticks.to_le_bytes());
    for (index, checkpoint) in checkpoints.iter().enumerate() {
        let detail = detail_for(*checkpoint, counts[index]);
        append_canonical(&mut bytes, index as u32, EventCode::SIM_TICK, &detail);
    }
    hash_bytes(&bytes)
}

/// Applied-input count per tick, in tick order.
///
/// Inputs are validated by [`replay`] before this runs, so every
/// `input.tick` is in range; out-of-range ticks are ignored defensively and
/// never panic.
fn inputs_per_tick(tape: &Tape) -> Vec<usize> {
    let mut counts = vec![0usize; tape.config.ticks as usize];
    for input in &tape.inputs {
        if let Some(slot) = counts.get_mut(input.tick as usize) {
            *slot += 1;
        }
    }
    counts
}

/// Re-run the tape through the authoritative replay gate, then rebuild the
/// checkpoints the trace is derived from.
///
/// [`replay`] owns validation, version gates and first-divergence precedence;
/// [`record`] then reproduces the identical checkpoints for event building.
/// On a verified tape the two agree tick for tick.
fn verified_checkpoints(tape: &Tape) -> Result<Vec<Checkpoint>, Error> {
    if tape.checkpoints.len() != tape.config.ticks as usize {
        return Err(Error::Invalid("checkpoint count does not match ticks"));
    }
    replay(tape)?;
    let fresh = record(tape.config, &tape.inputs)?;
    debug_assert_eq!(fresh.checkpoints, tape.checkpoints);
    Ok(fresh.checkpoints)
}

/// Replay a tape with verbose tracing, regenerating every covered event.
///
/// Verifies checkpoints exactly like [`replay`] (same [`Error`] precedence),
/// then returns one [`EventCode::SIM_TICK`] [`SimEvent`] per tick plus the
/// [`TraceReport::summary_hash`] that [`replay_summary`] reproduces without
/// retaining events.
///
/// # Errors
///
/// Returns [`Error`] for invalid data, unsupported versions, the first
/// divergent tick, or a golden mismatch — the same errors [`replay`] returns.
pub fn replay_verbose(tape: &Tape) -> Result<TraceReport, Error> {
    let checkpoints = verified_checkpoints(tape)?;
    let counts = inputs_per_tick(tape);
    let mut canonical = Vec::with_capacity(16 + checkpoints.len() * 64);
    canonical.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
    canonical.extend_from_slice(&tape.config.seed.to_le_bytes());
    canonical.extend_from_slice(&tape.config.ticks.to_le_bytes());
    let mut events = Vec::with_capacity(checkpoints.len());
    for (index, checkpoint) in checkpoints.iter().enumerate() {
        let detail = detail_for(*checkpoint, counts[index]);
        append_canonical(&mut canonical, index as u32, EventCode::SIM_TICK, &detail);
        events.push(SimEvent {
            tick: index as u32,
            code: EventCode::SIM_TICK,
            detail,
        });
    }
    Ok(TraceReport {
        events,
        summary_hash: hash_bytes(&canonical),
    })
}

/// Live non-verbose replay: verify the tape and return the same summary hash
/// [`replay_verbose`] reports, without retaining any event.
///
/// The canonical bytes folded here are identical to the traced path's, so a
/// mismatch means the trace diverged from live simulation, not that hashing
/// differs. Used for the traced-vs-untraced overhead A/B.
///
/// # Errors
///
/// Returns the same [`Error`] values [`replay`] returns.
pub fn replay_summary(tape: &Tape) -> Result<u64, Error> {
    let checkpoints = verified_checkpoints(tape)?;
    let counts = inputs_per_tick(tape);
    Ok(hash_for(tape.config, &checkpoints, &counts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Input;

    /// Proof fixture for trace tests: seed 7, 24 ticks, one velocity input.
    /// Small so the default suite stays fast.
    fn proof_tape() -> Tape {
        record(
            Config { seed: 7, ticks: 24 },
            &[Input {
                tick: 5,
                body: 1,
                vx: 1.0,
                vy: 2.0,
            }],
        )
        .unwrap()
    }

    #[test]
    fn traced_events_match_the_live_summary_hash() {
        let tape = proof_tape();
        let report = replay_verbose(&tape).unwrap();
        let live = replay_summary(&tape).unwrap();
        assert_eq!(report.summary_hash, live);
        assert_eq!(report.events.len(), tape.config.ticks as usize);
        // The default golden fixture agrees too.
        let golden = record(Config::default(), &[]).unwrap();
        assert_eq!(
            replay_verbose(&golden).unwrap().summary_hash,
            replay_summary(&golden).unwrap()
        );
    }

    #[test]
    fn same_tape_twice_gives_identical_traces() {
        for seed in [0, 7, 11] {
            let tape = record(
                Config { seed, ticks: 24 },
                &[Input {
                    tick: 5,
                    body: 1,
                    vx: 1.0,
                    vy: 2.0,
                }],
            )
            .unwrap();
            assert_eq!(replay_verbose(&tape), replay_verbose(&tape));
            assert_eq!(replay_summary(&tape), replay_summary(&tape));
        }
    }

    #[test]
    fn covered_scope_is_sim_tick_only() {
        let tape = proof_tape();
        let report = replay_verbose(&tape).unwrap();
        assert!(!report.events.is_empty());
        for (index, event) in report.events.iter().enumerate() {
            // Covered: exactly one ordered sim.tick per tick.
            assert_eq!(event.code, EventCode::SIM_TICK, "tick {index}");
            assert_eq!(event.tick, index as u32);
            // Details stay within the observe message bound.
            assert!(event.detail.len() <= engine_observe::MAX_MESSAGE_LEN);
            assert!(event.detail.starts_with("legacy="));
        }
        // The input at tick 5 is covered by its tick's annotation.
        assert!(report.events[5].detail.ends_with("inputs=1"));
        assert!(report.events[0].detail.ends_with("inputs=0"));
        // No out-of-scope subsystem code is ever regenerated.
        for code in [
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
            EventCode::HEAL_APPLIED,
            EventCode::HEAL_FAILED,
        ] {
            assert!(
                report.events.iter().all(|event| event.code != code),
                "out-of-scope code regenerated: {code:?}"
            );
        }
        // The digest path sees the same sim.tick rows agents consume.
        let digest_text = report.digest(1024);
        assert!(digest_text.contains("sim.tick x24"));
    }

    #[test]
    fn summary_hash_responds_to_inputs_and_checkpoints() {
        let config = Config { seed: 7, ticks: 24 };
        let base = record(config, &[]).unwrap();
        let changed = proof_tape();
        assert_ne!(
            replay_verbose(&base).unwrap().summary_hash,
            replay_verbose(&changed).unwrap().summary_hash
        );
        // Same tape, one flipped checkpoint bit: live and verbose both refuse.
        let mut diverged = changed.clone();
        diverged.checkpoints[7].state ^= 1;
        let error = replay_verbose(&diverged).unwrap_err();
        assert!(matches!(error, Error::Divergence { tick: 7, .. }));
        assert!(matches!(
            replay_summary(&diverged).unwrap_err(),
            Error::Divergence { tick: 7, .. }
        ));
    }

    #[test]
    fn invalid_tapes_fail_without_panicking() {
        let tape = proof_tape();
        assert!(matches!(
            replay_verbose(&Tape {
                config: Config { seed: 0, ticks: 0 },
                inputs: vec![],
                checkpoints: vec![],
            }),
            Err(Error::Invalid(_))
        ));
        let mut truncated = tape.clone();
        truncated.checkpoints.pop();
        assert!(matches!(replay_verbose(&truncated), Err(Error::Invalid(_))));
        assert!(matches!(replay_summary(&truncated), Err(Error::Invalid(_))));
        let mut bad_input = tape.clone();
        bad_input.inputs[0].vx = f32::NAN;
        assert!(replay_verbose(&bad_input).is_err());
    }

    #[test]
    fn overhead_probe_reports_traced_vs_untraced_wall_time() {
        use std::time::Instant;
        let tape = proof_tape();
        // Warmup so the measured runs are steady-state.
        let _ = replay_verbose(&tape).unwrap();
        let _ = replay_summary(&tape).unwrap();
        let mut traced_ms = 0.0;
        let mut live_ms = 0.0;
        let samples = 9;
        for _ in 0..samples {
            let start = Instant::now();
            let report = replay_verbose(&tape).unwrap();
            traced_ms += start.elapsed().as_secs_f64() * 1000.0;
            let start = Instant::now();
            let live = replay_summary(&tape).unwrap();
            live_ms += start.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(report.summary_hash, live);
        }
        traced_ms /= f64::from(samples);
        live_ms /= f64::from(samples);
        eprintln!(
            "trace overhead probe: traced={traced_ms:.3}ms live={live_ms:.3}ms ratio={:.2}x",
            traced_ms / live_ms.max(f64::MIN_POSITIVE)
        );
    }
}

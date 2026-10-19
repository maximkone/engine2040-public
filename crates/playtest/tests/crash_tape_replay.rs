//! E13 leg 2: an injected crash tape replays to the same tick every time.
//!
//! The playtester's injected fault is one overridden probe input
//! (`Policy::Divert`): it makes the probe run leave the control hash chain at
//! a known tick and files a `BugFile` — the exact bytes to replay it, plus
//! its `first_bad_tick`. This injects that fault, then replays the filed crash
//! tape ten times and asserts every replay files the same bug at the same
//! tick (10/10).

use engine_playtest::agent::{Agent, AgentConfig, BugFile, Caps, Driver, Metric, Policy};
use engine_playtest::seeded_bug::{BASE_SEED, MATCH_SLOTS, MATCH_TICKS, SEEDED_SLOT, SEEDED_TICK};

/// Tick of the injected probe input (the known failing tick).
const INJECTED_TICK: u32 = SEEDED_TICK;
/// Slot of the injected probe input.
const INJECTED_SLOT: u8 = SEEDED_SLOT;
/// Replays the crash tape must reproduce identically.
const REPLAYS: usize = 10;

/// Inject the fault and file the crash tape: the exact bytes to replay the
/// failing match plus the first tick that left the control chain.
fn inject_crash_tape() -> BugFile {
    let agent = Agent {
        config: AgentConfig {
            base_seed: BASE_SEED,
            ticks: MATCH_TICKS,
            fighters: MATCH_SLOTS,
            half_extent_m: 20.0,
        },
        driver: Driver {
            policy: Policy::Divert {
                tick: INJECTED_TICK,
                fighter: INJECTED_SLOT,
            },
            caps: Caps::mock(),
        },
    };
    agent
        .drive(1)
        .bugs
        .into_iter()
        .find(|bug| matches!(bug.metric, Metric::Desync { .. }))
        .expect("an injected probe input files a crash tape")
}

#[test]
fn injected_crash_tape_replays_the_same_tick_ten_of_ten() {
    let crash = inject_crash_tape();
    assert!(
        matches!(crash.metric, Metric::Desync { .. }),
        "crash tape is a desync: {:?}",
        crash.metric
    );
    assert_eq!(
        crash.first_bad_tick, INJECTED_TICK,
        "the injected fault fails at its known tick"
    );
    let mut ticks = Vec::with_capacity(REPLAYS);
    for replay in 1..=REPLAYS {
        let replayed = crash.replay();
        assert!(
            replayed.contains(&crash),
            "replay {replay} must reproduce the filed crash tape exactly"
        );
        let desync = replayed
            .iter()
            .find(|bug| matches!(bug.metric, Metric::Desync { .. }))
            .unwrap_or_else(|| panic!("replay {replay} files the crash"));
        ticks.push(desync.first_bad_tick);
    }
    assert_eq!(ticks.len(), REPLAYS, "ten replays ran");
    assert!(
        ticks.iter().all(|tick| *tick == INJECTED_TICK),
        "every replay fails at the same tick {INJECTED_TICK}: {ticks:?}"
    );
    println!("crash-tape-replay: {REPLAYS}/{REPLAYS} replays failed at tick {INJECTED_TICK}");
}

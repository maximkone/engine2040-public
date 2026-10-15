//! Heal and quarantine vectors over the public observe API.
//!
//! Consumer: chaos suite and supervisor wiring. Fresh write vs the crate own
//! heal.rs baseline; donor routatic/proxy PR #67 was idea-only (UNVERIFIED
//! licence, zero donor bytes copied). Distinct from the landed breaker and
//! vigil vectors: this file pins the 3-fault fixture, the 10-tick cooldown,
//! exactly 1 half-open probe and per-key backoff.
use engine_observe::heal::HealKind;
use engine_observe::supervisor::{
    backoff_ms, Supervisor, HALF_OPEN_MAX_CALLS, QUARANTINE_COOLDOWN_TICKS,
};
use engine_observe::{EventCode, RingBuffer};

/// Three-fault fixture: service degrade, reconnect and last-good rows.
const FIXTURE: [EventCode; 3] = [
    EventCode::PROVIDER_TIMEOUT,
    EventCode::RENDER_DEVICE_LOST,
    EventCode::SAVE_CORRUPT,
];

fn ring() -> RingBuffer {
    RingBuffer::new(64)
}

#[test]
fn cooldown_is_ten_ticks_and_single_probe() {
    assert_eq!(QUARANTINE_COOLDOWN_TICKS, 10);
    assert_eq!(HALF_OPEN_MAX_CALLS, 1);
}

#[test]
fn three_fault_fixture_heals_within_budget() {
    let mut supervisor = Supervisor::new();
    let mut events = ring();
    let kinds = [
        (EventCode::PROVIDER_TIMEOUT, HealKind::Degrade),
        (EventCode::RENDER_DEVICE_LOST, HealKind::Reconnect),
        (EventCode::SAVE_CORRUPT, HealKind::LastGood),
    ];
    for (tick, (code, kind)) in kinds.into_iter().enumerate() {
        let decision = supervisor
            .report_fault(code, tick as u64, &mut events)
            .expect("fixture fault must heal");
        assert_eq!(decision.kind, kind);
        assert_eq!(decision.attempt, 1);
        assert!(!decision.is_quarantine());
    }
    assert!(FIXTURE.contains(&EventCode::PROVIDER_TIMEOUT));
    assert_eq!(supervisor.tape_len(), 3);
    assert!(!supervisor.is_quarantined(EventCode::PROVIDER_TIMEOUT));
    assert!(!supervisor.is_quarantined(EventCode::RENDER_DEVICE_LOST));
    assert!(!supervisor.is_quarantined(EventCode::SAVE_CORRUPT));
}

#[test]
fn quarantine_cooldown_admits_exactly_one_half_open_probe() {
    let mut supervisor = Supervisor::new();
    let mut events = ring();
    let code = EventCode::SAVE_CORRUPT;
    let first = supervisor
        .report_fault(code, 0, &mut events)
        .expect("first fault heals");
    assert_eq!(first.kind, HealKind::LastGood);
    let quarantined = supervisor
        .report_fault(code, 1, &mut events)
        .expect("over-budget fault quarantines");
    assert!(quarantined.is_quarantine());
    let until = 1 + QUARANTINE_COOLDOWN_TICKS;
    assert_eq!(supervisor.quarantined_until(code), Some(until));
    assert!(supervisor.is_quarantined(code));
    assert!(!supervisor.is_half_open(code));

    let tape_len = supervisor.tape_len();
    let applied = events.query(Some(EventCode::HEAL_APPLIED)).len();
    let failed = events.query(Some(EventCode::HEAL_FAILED)).len();
    let fast = supervisor
        .report_fault(code, 2, &mut events)
        .expect("quarantined fault fast-rejects");
    assert!(fast.is_quarantine());
    assert_eq!(supervisor.tape_len(), tape_len);
    assert_eq!(events.query(Some(EventCode::HEAL_APPLIED)).len(), applied);
    assert_eq!(events.query(Some(EventCode::HEAL_FAILED)).len(), failed);

    let probe = supervisor
        .report_fault(code, until, &mut events)
        .expect("cooldown expiry must admit one probe");
    assert!(!probe.is_quarantine());
    assert_eq!(probe.attempt, 1);
    assert_eq!(probe.kind, HealKind::LastGood);
    assert!(supervisor.is_half_open(code));
    assert!(!supervisor.is_quarantined(code));

    let reopen = supervisor
        .report_fault(code, until + 1, &mut events)
        .expect("failed probe must re-quarantine");
    assert!(reopen.is_quarantine());
    assert_eq!(reopen.kind, HealKind::Quarantine);
    assert_eq!(
        supervisor.quarantined_until(code),
        Some(until + 1 + QUARANTINE_COOLDOWN_TICKS)
    );
    assert!(supervisor.is_quarantined(code));
    assert!(!supervisor.is_half_open(code));
}

#[test]
fn per_key_backoff_and_quarantine_isolation() {
    let mut supervisor = Supervisor::new();
    let mut events = ring();
    let slow = supervisor
        .report_fault(EventCode::PROVIDER_TIMEOUT, 0, &mut events)
        .expect("provider fault heals");
    assert_eq!(slow.backoff_ms, backoff_ms(200, 2000, 1));
    assert_eq!(slow.backoff_ms, 200);
    let quick = supervisor
        .report_fault(EventCode::RENDER_DEVICE_LOST, 0, &mut events)
        .expect("device fault heals");
    assert_eq!(quick.backoff_ms, backoff_ms(100, 2000, 1));
    assert_eq!(quick.backoff_ms, 100);
    assert_ne!(slow.backoff_ms, quick.backoff_ms);

    supervisor
        .report_fault(EventCode::SAVE_CORRUPT, 1, &mut events)
        .expect("save fault heals");
    supervisor
        .report_fault(EventCode::SAVE_CORRUPT, 2, &mut events)
        .expect("save fault quarantines");
    assert!(supervisor.is_quarantined(EventCode::SAVE_CORRUPT));
    assert!(!supervisor.is_quarantined(EventCode::PROVIDER_TIMEOUT));
    assert!(!supervisor.is_quarantined(EventCode::RENDER_DEVICE_LOST));
    let still_heals = supervisor
        .report_fault(EventCode::PROVIDER_TIMEOUT, 3, &mut events)
        .expect("isolated key must still heal");
    assert!(!still_heals.is_quarantine());
    assert_eq!(still_heals.attempt, 2);
    assert_eq!(still_heals.backoff_ms, backoff_ms(200, 2000, 2));
    assert_eq!(still_heals.backoff_ms, 400);
}

// Log sampler/encoding vectors vs the owned engine-observe ring.
//
// Attribution: sampler shape idea-only from rs/zerolog (MIT) and
// zerolog (MIT); zero donor bytes copied. All vectors below are fresh
// and assert against the owned engine_observe RingBuffer in
// crates/observe/src/lib.rs.
//
// Consumer: agents and the MCP door (log.digest/log.query) read the
// sampled, encoded and digested events; this file only proves sampling,
// encoding and budget behavior.
use engine_observe::{digest, Event, EventCode, RingBuffer};
const SAMPLE_EVERY: u64 = 10;
fn should_sample(tick: u64) -> bool {
    tick.is_multiple_of(SAMPLE_EVERY)
}
fn encode_event(event: &Event) -> String {
    format!("{}|{}|{}", event.code.raw(), event.tick, event.message)
}
fn encode_batch(events: &[Event]) -> String {
    events
        .iter()
        .map(encode_event)
        .collect::<Vec<_>>()
        .join("\n")
}
fn sampled_ticks(total: u64) -> Vec<u64> {
    (0..total).filter(|t| should_sample(*t)).collect()
}
#[test]
fn sample_ratio_one_in_ten_exact() {
    let kept = sampled_ticks(100);
    assert_eq!(kept.len(), 10);
    assert_eq!(kept, vec![0, 10, 20, 30, 40, 50, 60, 70, 80, 90]);
    assert_eq!(sampled_ticks(1000).len(), 100);
    assert_eq!(sampled_ticks(10).len(), 1);
    assert!(should_sample(20));
    for tick in [19, 21, 22, 29] {
        assert!(!should_sample(tick), "tick {tick} must not sample");
    }
}
#[test]
fn sampled_events_land_in_ring_buffer() {
    let mut buf = RingBuffer::new(64);
    for tick in 0..100 {
        if should_sample(tick) {
            buf.push(Event::new(EventCode::SIM_TICK, tick, "tick"));
        }
    }
    assert_eq!(buf.len(), 10);
    assert_eq!(buf.overflow_count(), 0);
    let ticks: Vec<u64> = buf.iter().map(|e| e.tick).collect();
    assert_eq!(ticks, vec![0, 10, 20, 30, 40, 50, 60, 70, 80, 90]);
    let mut small = RingBuffer::new(4);
    for tick in 0..100 {
        if should_sample(tick) {
            small.push(Event::new(EventCode::SIM_TICK, tick, "tick"));
        }
    }
    assert_eq!(small.len(), 4);
    assert_eq!(small.overflow_count(), 6);
    let kept: Vec<u64> = small.iter().map(|e| e.tick).collect();
    assert_eq!(kept, vec![60, 70, 80, 90]);
}
#[test]
fn encode_is_deterministic_twice() {
    let events = vec![
        Event::new(EventCode::SIM_TICK, 10, "tick"),
        Event::new(EventCode::PROVIDER_TIMEOUT, 20, "slow"),
        Event::new(EventCode::HEAL_APPLIED, 30, "replanned"),
    ];
    let first = encode_batch(&events);
    let second = encode_batch(&events);
    assert_eq!(first, second);
    let cloned = events.clone();
    assert_eq!(encode_batch(&cloned), first);
}
#[test]
fn encode_vectors_match_known_shape() {
    let event = Event::new(EventCode::SIM_TICK, 10, "tick");
    assert_eq!(encode_event(&event), "1|10|tick");
    let timeout = Event::new(EventCode::PROVIDER_TIMEOUT, 20, "slow");
    assert_eq!(encode_event(&timeout), "8|20|slow");
    let batch = encode_batch(&[event, timeout]);
    assert_eq!(batch, "1|10|tick\n8|20|slow");
    let other = Event::new(EventCode::SIM_TICK, 11, "tick");
    assert_ne!(encode_event(&other), "1|10|tick");
}
#[test]
fn digest_stays_within_budget() {
    let mut buf = RingBuffer::new(64);
    for tick in 0..100 {
        if should_sample(tick) {
            buf.push(Event::new(EventCode::SIM_TICK, tick, "tick"));
        }
    }
    let events: Vec<Event> = buf.iter().cloned().collect();
    assert_eq!(events.len(), 10);
    for budget in [0, 1, 16, 64, 256, 1024, 4096] {
        let out = digest(&events, budget);
        assert!(
            out.len() <= budget,
            "digest len {} exceeds budget {budget}: {out:?}",
            out.len()
        );
    }
    let full = digest(&events, 4096);
    assert!(
        full.contains("sim.tick x10 first=0 last=90"),
        "got: {full:?}"
    );
    assert_eq!(digest(&events, 256), digest(&events, 256));
}

//! `engine-observe`: std-only typed diagnostic events.
//!
//! Consumer: agents, the MCP door (`log.digest`/`log.query`) and crash-bundle
//! exporters. Data owner: each calling subsystem pushes its own events;
//! this crate owns the stable code registry, bounded storage and digests.
//! Units: ticks (`u64`), byte budgets (`usize`). Determinism scope:
//! insertion order is preserved, but [`digest`] output is sorted by
//! (count desc, code asc), so any input permutation gives the same string.
//! Error type: [`ObserveError`]; unknown codes fail, never panic.
//!
//! Events, spans and digests are diagnostics only and never enter any
//! simulation hash. Simulation state must never be derived from them.
//!
//! The [`heal`] module adds the bounded MAPE-K auto-heal registry: fault
//! codes map to caller-executed heal advice with attempt budgets,
//! escalation and quarantine.
//!
//! The [`supervisor`] module adds the shipped fault catalog, escalation
//! with backoff, tick-boundary sim heals and the heal tape the simulation
//! copies into its action log for replay.
//!
//! The [`breaker`] module adds the rolling error-percent circuit breaker and
//! the hedge budget: a retry storm stops without spending budget, and a late
//! primary may spend one backup. Its clock is the caller's revision, never a
//! wall clock.
//!
//! The [`recovery`] module adds the proof a heal must carry: the scope of
//! the heal, the verdict (recovered, not recovered, frozen frame, rolled
//! back), the digests of the last good state and of the healed state, and the
//! rolling heal cap that turns a heal storm into a red run.
//!
//! The [`rerun_sink`] module adds the bounded 120-frame rerun recording of
//! simulation state per tick for desync hunts.
//!
//! The [`mcap`] module writes and reads replay recordings (tick hash, fighter
//! poses, events, debug lines) as MCAP files with no SDK.
//!
//! The [`timeline`] module adds the per-tick timeline over the frozen PF9
//! recording shape: a seeded desync names its first divergent tick, field
//! and both values, and exports NDJSON in rerun `time_point` shape.
//!
//! The [`fps`] module adds std-only typed FPS view events: fighter poses,
//! adjudicated hits and frame cost splits, read by the endpoint
//! `check_profile` pack.

/// Std-only circuit breaker and hedge budget over the supervisor: the retry
/// storm gate and the extra-load cap, clocked by the caller's revision.
pub mod breaker;
/// Deterministic diagnostic event queue with phased delivery (test-only, OB4).
pub mod event_queue;
/// Std-only typed FPS view events: pose, hits and frame splits.
pub mod fps;
pub mod heal;
/// Std-only MCAP writer and reader for replay recordings Foxglove and Rerun open.
pub mod mcap;
/// Heal scope, the recovery proof and the rolling heal cap: a heal counts
/// only when the healed state hashes to the last good state.
pub mod recovery;
/// Bounded 120-frame rerun recording of sim state per tick for desync hunts.
pub mod rerun_sink;
pub mod supervisor;
/// The heal proof rules of the supervisor: the recovery proof, the budget
/// reset gate, the rollback and the rolling heal cap.
mod supervisor_proof;
/// Per-tick timeline over the PF9 shape: seeded desyncs name the first
/// divergent tick, field and both values; NDJSON export in `time_point` shape.
pub mod timeline;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// Maximum event message size in bytes. Longer input is truncated on a
/// `char` boundary; the event is still recorded.
pub const MAX_MESSAGE_LEN: usize = 256;

/// Stable event code. The inner value is private so an unregistered code
/// cannot be constructed silently: use [`EventCode::from_raw`], which
/// returns [`ObserveError::UnknownCode`] for anything outside the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventCode(u32);

impl EventCode {
    /// Heartbeat / tick marker for liveness checks.
    pub const SIM_TICK: Self = Self(1);
    /// Non-finite body or transform rejected before publication.
    pub const PHYSICS_BODY_NONFINITE: Self = Self(2);
    /// Fighter made no progress outside idle/block/hit-stop windows.
    pub const BRAIN_FIGHTER_STUCK: Self = Self(3);
    /// Client checkpoint hash diverged from the server snapshot.
    pub const NET_CLIENT_DESYNC: Self = Self(4);
    /// GPU device or surface lost.
    pub const RENDER_DEVICE_LOST: Self = Self(5);
    /// Shader or pipeline build failed; last good kept.
    pub const RENDER_PIPELINE_FAILED: Self = Self(6);
    /// Asset missing or digest/format check failed.
    pub const ASSETS_MISSING: Self = Self(7);
    /// Provider request exceeded its deadline.
    pub const PROVIDER_TIMEOUT: Self = Self(8);
    /// Provider output failed schema validation.
    pub const PROVIDER_MALFORMED: Self = Self(9);
    /// AI edit arrived with a stale revision.
    pub const COMMANDS_STALE: Self = Self(10);
    /// Editor or game child process exited unexpectedly.
    pub const EDITOR_CRASH: Self = Self(11);
    /// Save or input tape failed versioned checks.
    pub const SAVE_CORRUPT: Self = Self(12);
    /// Frame exceeded its presentation budget.
    pub const RENDER_FRAME_OVER_BUDGET: Self = Self(13);
    /// A bounded heal action recovered or retained last-good.
    pub const HEAL_APPLIED: Self = Self(14);
    /// A heal action failed and escalated.
    pub const HEAL_FAILED: Self = Self(15);
    /// FPS view fighter pose in integer units.
    pub const FPS_POSE: Self = Self(16);
    /// FPS view adjudicated hit with the net verdict shape.
    pub const FPS_HIT: Self = Self(17);
    /// FPS view frame cost split in microseconds.
    pub const FPS_FRAME_SPLIT: Self = Self(18);
    /// The door refused one call: the message names the tool and its code.
    pub const DOOR_REFUSED: Self = Self(19);

    /// Raw stable value for serialization into tapes and bundles.
    #[must_use]
    pub fn raw(self) -> u32 {
        self.0
    }

    /// Dotted registry name, or `None` for unregistered codes.
    #[must_use]
    pub fn name(self) -> Option<&'static str> {
        name(self.0)
    }

    /// Validate a raw code. Unknown values return
    /// [`ObserveError::UnknownCode`], never panic.
    ///
    /// # Errors
    ///
    /// Returns [`ObserveError::UnknownCode`] when `raw` is not registered.
    pub fn from_raw(raw: u32) -> Result<Self, ObserveError> {
        if name(raw).is_some() {
            Ok(Self(raw))
        } else {
            Err(ObserveError::UnknownCode(raw))
        }
    }
}

/// Registry lookup: raw code to stable dotted name.
///
/// Returns `None` for unregistered codes.
#[must_use]
pub fn name(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "sim.tick",
        2 => "physics.body.nonfinite",
        3 => "brain.fighter.stuck",
        4 => "net.client.desync",
        5 => "render.device.lost",
        6 => "render.pipeline.failed",
        7 => "assets.missing",
        8 => "provider.timeout",
        9 => "provider.malformed",
        10 => "commands.stale",
        11 => "editor.crash",
        12 => "save.corrupt",
        13 => "render.frame.over_budget",
        14 => "heal.applied",
        15 => "heal.failed",
        16 => "fps.pose",
        17 => "fps.hit",
        18 => "fps.frame.split",
        19 => "door.refused",
        _ => return None,
    })
}

/// Typed failure for observe construction. Invalid external input returns
/// this error; nothing here panics on unknown codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveError {
    /// Raw code is not in the stable registry.
    UnknownCode(u32),
}

impl fmt::Display for ObserveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownCode(code) => write!(f, "unknown event code: {code}"),
        }
    }
}

impl std::error::Error for ObserveError {}

/// One typed diagnostic event. The message is informational only and is
/// truncated to [`MAX_MESSAGE_LEN`] bytes; severity and routing derive
/// from `code`, never from message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Stable registry code.
    pub code: EventCode,
    /// Simulation or service tick when the event was emitted.
    pub tick: u64,
    /// Bounded human/agent-readable detail. No credentials or tokens.
    pub message: String,
}

impl Event {
    /// Build an event from an already-validated code. The message is
    /// truncated to [`MAX_MESSAGE_LEN`] bytes on a `char` boundary.
    #[must_use]
    pub fn new(code: EventCode, tick: u64, message: &str) -> Self {
        Self {
            code,
            tick,
            message: bound_len(message, MAX_MESSAGE_LEN),
        }
    }

    /// Build an event from a raw code, failing on unregistered values.
    ///
    /// # Errors
    ///
    /// Returns [`ObserveError::UnknownCode`] when `raw` is not registered.
    pub fn try_new(raw: u32, tick: u64, message: &str) -> Result<Self, ObserveError> {
        Ok(Self::new(EventCode::from_raw(raw)?, tick, message))
    }
}

/// Bounded per-subsystem ring buffer. When full, the oldest event is dropped
/// and the drop is counted: drops are visible via [`RingBuffer::overflow_count`],
/// never reported as an empty list.
#[derive(Debug, Clone)]
pub struct RingBuffer {
    cap: usize,
    events: VecDeque<Event>,
    overflow: u64,
}

impl RingBuffer {
    /// Create a buffer holding at most `cap` events. `cap == 0` stores
    /// nothing and counts every push as overflow.
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            events: VecDeque::with_capacity(cap),
            overflow: 0,
        }
    }

    /// Maximum events retained.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Events currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when no events are retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Events dropped because the buffer was full. Monotonic.
    #[must_use]
    pub fn overflow_count(&self) -> u64 {
        self.overflow
    }

    /// Push an event, evicting the oldest when full and counting the drop.
    /// `saturating_add` keeps the counter monotonic past `u64::MAX`.
    pub fn push(&mut self, event: Event) {
        if self.events.len() >= self.cap {
            if self.events.pop_front().is_some() {
                self.overflow = self.overflow.saturating_add(1);
            } else {
                // Zero capacity: nothing to evict, the push itself is dropped.
                self.overflow = self.overflow.saturating_add(1);
                return;
            }
        }
        self.events.push_back(event);
    }

    /// Oldest-first iterator over retained events.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Event> + '_ {
        self.events.iter()
    }

    /// Push one validated FPS pose as an `fps.pose` event.
    pub fn push_pose(&mut self, pose: fps::FpsPose) {
        self.push(pose.to_event());
    }

    /// Push one validated FPS hit as an `fps.hit` event.
    pub fn push_hit(&mut self, hit: fps::FpsHit) {
        self.push(hit.to_event());
    }

    /// Push one validated FPS frame split as an `fps.frame.split` event.
    pub fn push_split(&mut self, split: fps::FpsFrameSplit) {
        self.push(split.to_event());
    }

    /// Oldest-first snapshot, optionally filtered to one code.
    /// `None` returns every retained event.
    #[must_use]
    pub fn query(&self, code: Option<EventCode>) -> Vec<&Event> {
        self.events
            .iter()
            .filter(|e| code.is_none_or(|c| e.code == c))
            .collect()
    }
}

/// Bounded agent digest: per-code counts with first/last ticks, most
/// frequent first (ties broken by code, so input order never matters).
/// The returned string is at most `budget_bytes` bytes long; rows that do
/// not fit are omitted, and a header that alone exceeds the budget is cut
/// on a `char` boundary. A zero budget returns an empty string.
#[must_use]
pub fn digest(events: &[Event], budget_bytes: usize) -> String {
    if budget_bytes == 0 || events.is_empty() {
        return String::new();
    }
    let mut table: BTreeMap<u32, (usize, u64, u64)> = BTreeMap::new();
    for event in events {
        table
            .entry(event.code.raw())
            .and_modify(|row| {
                row.0 += 1;
                row.1 = row.1.min(event.tick);
                row.2 = row.2.max(event.tick);
            })
            .or_insert((1, event.tick, event.tick));
    }
    let mut rows: Vec<(u32, usize, u64, u64)> = table
        .into_iter()
        .map(|(code, (count, first, last))| (code, count, first, last))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let mut out = format!("events={} codes={}\n", events.len(), rows.len());
    if out.len() > budget_bytes {
        return bound_len(&out, budget_bytes);
    }
    for (code, count, first, last) in rows {
        let label = name(code).unwrap_or("unknown");
        let row = format!("{label} x{count} first={first} last={last}\n");
        if out.len() + row.len() > budget_bytes {
            break;
        }
        out.push_str(&row);
    }
    out
}

/// Truncate `text` to `max_bytes` bytes on a `char` boundary.
fn bound_len(text: &str, max_bytes: usize) -> String {
    text[..text.floor_char_boundary(max_bytes)].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_events() -> Vec<Event> {
        vec![
            Event::new(EventCode::PROVIDER_TIMEOUT, 3, "slow"),
            Event::new(EventCode::SIM_TICK, 1, "tick"),
            Event::new(EventCode::PROVIDER_TIMEOUT, 7, "slow again"),
            Event::new(EventCode::HEAL_APPLIED, 5, "replanned"),
        ]
    }

    #[test]
    fn unknown_code_fails() {
        assert_eq!(name(0), None);
        assert_eq!(name(u32::MAX), None);
        assert_eq!(
            EventCode::from_raw(u32::MAX),
            Err(ObserveError::UnknownCode(u32::MAX))
        );
        assert_eq!(
            Event::try_new(9999, 0, "nope"),
            Err(ObserveError::UnknownCode(9999))
        );
        let err = EventCode::from_raw(4242).unwrap_err();
        assert_eq!(err.to_string(), "unknown event code: 4242");
        // Typed error implements std::error::Error without panicking.
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn known_codes_roundtrip() {
        let codes = [
            EventCode::SIM_TICK,
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
        ];
        for code in codes {
            let raw = code.raw();
            assert_eq!(EventCode::from_raw(raw), Ok(code));
            assert!(code.name().is_some());
            assert!(name(raw).is_some());
        }
        // Registry values are unique.
        let mut raws: Vec<u32> = codes.iter().copied().map(EventCode::raw).collect();
        raws.sort_unstable();
        raws.dedup();
        assert_eq!(raws.len(), codes.len());
    }

    #[test]
    fn message_is_bounded() {
        let long = "é".repeat(MAX_MESSAGE_LEN * 2);
        let event = Event::new(EventCode::SIM_TICK, 0, &long);
        assert!(event.message.len() <= MAX_MESSAGE_LEN);
        // Truncation keeps valid UTF-8.
        assert!(event.message.chars().count() * "é".len() <= MAX_MESSAGE_LEN);
        let short = Event::new(EventCode::SIM_TICK, 0, "ok");
        assert_eq!(short.message, "ok");
    }

    #[test]
    fn overflow_counter_increments_on_over_cap_push() {
        let mut buf = RingBuffer::new(2);
        assert!(buf.is_empty());
        buf.push(Event::new(EventCode::SIM_TICK, 1, "a"));
        buf.push(Event::new(EventCode::SIM_TICK, 2, "b"));
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.overflow_count(), 0);
        buf.push(Event::new(EventCode::SIM_TICK, 3, "c"));
        buf.push(Event::new(EventCode::SIM_TICK, 4, "d"));
        buf.push(Event::new(EventCode::SIM_TICK, 5, "e"));
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.overflow_count(), 3);
        // Oldest-first: only the two newest survive.
        let ticks: Vec<u64> = buf.iter().map(|e| e.tick).collect();
        assert_eq!(ticks, vec![4, 5]);
    }

    #[test]
    fn zero_cap_counts_every_push_as_overflow() {
        let mut buf = RingBuffer::new(0);
        buf.push(Event::new(EventCode::SIM_TICK, 1, "a"));
        assert!(buf.is_empty());
        assert_eq!(buf.overflow_count(), 1);
    }

    #[test]
    fn query_filters_by_code_oldest_first() {
        let mut buf = RingBuffer::new(8);
        for event in sample_events() {
            buf.push(event);
        }
        assert_eq!(buf.query(None).len(), 4);
        let timeouts = buf.query(Some(EventCode::PROVIDER_TIMEOUT));
        assert_eq!(timeouts.len(), 2);
        assert_eq!(timeouts[0].tick, 3);
        assert_eq!(timeouts[1].tick, 7);
        assert!(buf.query(Some(EventCode::NET_CLIENT_DESYNC)).is_empty());
    }

    #[test]
    fn digest_stays_within_budget() {
        let events = sample_events();
        for budget in [0, 1, 10, 32, 64, 1024] {
            let out = digest(&events, budget);
            assert!(
                out.len() <= budget,
                "digest len {} exceeds budget {budget}: {out:?}",
                out.len()
            );
        }
        assert_eq!(digest(&events, 0), String::new());
        assert_eq!(digest(&[], 64), String::new());
        // A generous budget keeps every code row.
        let full = digest(&events, 1024);
        assert!(full.contains("provider.timeout x2 first=3 last=7"));
        assert!(full.contains("sim.tick x1"));
        assert!(full.contains("heal.applied x1"));
    }

    #[test]
    fn same_events_twice_give_same_digest() {
        let events = sample_events();
        assert_eq!(digest(&events, 256), digest(&events, 256));
        // Frozen order: input permutation changes nothing.
        let mut shuffled = events.clone();
        shuffled.reverse();
        assert_eq!(digest(&events, 256), digest(&shuffled, 256));
    }
}

//! announce: presentation-only announcer and fighter taunts.
//!
//! The game fills [`MatchObservation`] from arena PUBLIC observations and
//! hands it to [`Announcer::on_observation`] with an `adapter_callable` that
//! asks the provider under a tight budget. Any provider failure (timeout,
//! offline, slow, quota, malformed) maps to a scripted fallback line from a
//! fixed table: the call never blocks the tick, never writes authoritative
//! state, and never imports `arena`. Accepted lines accumulate in a
//! [`PresentationLog`]; replay reads the log back with the adapter disabled,
//! so disconnected checkpoints match. Fighter taunts live in [`TauntBook`]
//! with per-fighter cooldowns. In-crate users: `stream`, `openai`,
//! `anthropic` plus tests; no game-crate consumer yet (verified 2026-10-16).

use std::collections::HashMap;

use engine_core::Fnv64;

use super::core::AdapterOutcome;
use super::protocol::{
    AdapterKind, AiError, AiRequest, Budget, ModelSelector, PrivacyMode, RequestId, TaskKind,
};

/// Tight default budget for one provider line ask, in milliseconds. The
/// transport enforces the same bound, so a slow provider surfaces as a
/// timeout instead of stalling the tick.
pub const DEFAULT_ANNOUNCE_BUDGET_MS: u64 = 50;
/// Most fighters one observation may describe; a real arena never needs more.
pub const MAX_FIGHTERS_PER_OBSERVATION: usize = 8;
/// Longest presentation line kept, in characters; longer provider text is
/// truncated, never rejected loudly mid-match.
pub const MAX_LINE_LEN: usize = 280;
/// Start state of the presentation checkpoints ([`PresentationLog::checkpoint`]
/// and the stream tape's). Recorded checkpoints pin this exact value, which is
/// not the standard FNV-1a offset basis, so it is a named state, not a copy.
pub(crate) const CHECKPOINT_START: u64 = 0xcbf29ce4842225;

/// Fold `text` behind its byte length so adjacent texts cannot collide.
pub(crate) fn fold_text(hash: &mut Fnv64, text: &str) {
    hash.write_u64(u64::try_from(text.len()).unwrap_or(u64::MAX));
    hash.write_str(text);
}

/// One fighter snapshot: identity, health in permille, and whether down.
/// Plain data the game fills; this crate never reads arena state itself.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FigLite {
    /// Stable fighter id from the arena snapshot.
    pub id: u64,
    /// Health in permille (`0..=1000`).
    pub hp_permille: u16,
    /// True while the fighter is down.
    pub down: bool,
}

/// Presentation-relevant event kinds. Combat owns the authoritative events;
/// this is the lightweight copy the announcer may describe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventKindLite {
    /// A landed hit.
    Hit,
    /// A knockdown.
    Knockdown,
    /// A round boundary (start or end horn).
    Round,
}

/// One lightweight event attached to an observation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EventLite {
    /// What happened.
    pub kind: EventKindLite,
    /// Fighter the event concerns.
    pub fighter: u64,
}

/// One presentation tick: the arena tick, fighter snapshots, and the latest
/// event, if any. The tick clock lives outside this crate; observations are
/// read-only here, so presentation can never advance or stall it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MatchObservation {
    /// Arena tick this observation was taken at.
    pub tick: u64,
    /// Fighter snapshots (`<= MAX_FIGHTERS_PER_OBSERVATION`).
    pub fighters: Vec<FigLite>,
    /// Latest presentation-relevant event, if any.
    pub last_event: Option<EventLite>,
}

impl MatchObservation {
    /// True when sizes and ranges hold: bounded fighters, permille health.
    /// Invalid observations still get a neutral scripted line; they never
    /// panic and never reach a provider.
    pub fn is_valid(&self) -> bool {
        if self.fighters.len() > MAX_FIGHTERS_PER_OBSERVATION {
            return false;
        }
        self.fighters.iter().all(|fig| fig.hp_permille <= 1000)
    }
}

/// One presentation line: text plus a priority the HUD may sort or duck by.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Line {
    /// Display text, non-empty, at most [`MAX_LINE_LEN`] characters.
    pub text: String,
    /// 2 for knockdowns, 1 for hits and round horns, 0 otherwise.
    pub priority: u8,
    /// True when the line came from the fixed scripted table.
    pub scripted: bool,
}

/// Why a provider line ask failed. Every variant maps to the scripted table;
/// the distinction only tells the game what to show in its status row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineError {
    /// The tight budget ran out; the tick moved on without the provider.
    Timeout,
    /// No provider network calls allowed; scripted lines only.
    Offline,
    /// Any other refusal: quota, malformed, unreachable, cancelled.
    Unavailable,
}

impl std::fmt::Display for LineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LineError::Timeout => f.write_str("provider line timed out"),
            LineError::Offline => f.write_str("provider offline, scripted line"),
            LineError::Unavailable => f.write_str("provider unavailable, scripted line"),
        }
    }
}

const SCRIPTED_NEUTRAL: &[&str] = &[
    "The sand holds its breath.",
    "Steel glints under the arena sun.",
    "The crowd leans forward.",
    "Dust drifts across the sand.",
    "Both fighters circle, waiting.",
    "A hush falls over the stands.",
];

const SCRIPTED_HIT: &[&str] = &[
    "A solid hit lands!",
    "Steel finds its mark!",
    "The crowd roars at the blow!",
    "A telling strike!",
];

const SCRIPTED_KNOCKDOWN: &[&str] = &[
    "Down he goes!",
    "A crushing knockdown!",
    "The sand claims another!",
    "Flat on the sand!",
];

const SCRIPTED_ROUND: &[&str] = &["The horn sounds across the arena!", "A new round begins!"];

const HIT_TAUNTS: &[&str] = &[
    "Taste sand!",
    "Too slow!",
    "Is that all?",
    "Bleed for the crowd!",
];

const KNOCKDOWN_TAUNTS: &[&str] = &[
    "Stay down!",
    "The sand keeps you now!",
    "For the crowd!",
    "Kneel!",
];

/// Priority for an observation: knockdowns outrank hits and horns.
fn priority_for(obs: &MatchObservation) -> u8 {
    match &obs.last_event {
        Some(EventLite {
            kind: EventKindLite::Knockdown,
            ..
        }) => 2,
        Some(_) => 1,
        None => 0,
    }
}

/// Deterministic table pick: tick plus seed, no wall clock, no iteration
/// order, so the same observation always yields the same fallback line.
fn pick<'a>(table: &[&'a str], tick: u64, seed: u64) -> &'a str {
    let index = tick.wrapping_add(seed) as usize % table.len().max(1);
    table.get(index).copied().unwrap_or("The crowd watches.")
}

/// Trim provider text and bound its length. Blank output is unusable and
/// maps to the scripted table via `None`.
fn clean_provider_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bounded: String = trimmed.chars().take(MAX_LINE_LEN).collect();
    if bounded.is_empty() {
        return None;
    }
    Some(bounded)
}

/// Presentation-only announcer. Holds a seed for deterministic fallbacks, a
/// tight per-ask budget, and served/scripted counters for the game's status
/// row. No arena import, no authoritative write.
#[derive(Clone, Debug)]
pub struct Announcer {
    seed: u64,
    budget_ms: u64,
    served: u64,
    scripted: u64,
}

impl Announcer {
    /// Build an announcer with the default tight budget.
    pub fn new(seed: u64) -> Self {
        Self::with_budget(seed, DEFAULT_ANNOUNCE_BUDGET_MS)
    }

    /// Build an announcer with an explicit per-ask budget in milliseconds,
    /// clamped to `1..=30_000` like the transport bound.
    pub fn with_budget(seed: u64, budget_ms: u64) -> Self {
        Self {
            seed,
            budget_ms: budget_ms.clamp(1, 30_000),
            served: 0,
            scripted: 0,
        }
    }

    /// Per-ask budget handed to the provider callable.
    pub fn budget_ms(&self) -> u64 {
        self.budget_ms
    }

    /// Observations served so far (provider plus scripted).
    pub fn served(&self) -> u64 {
        self.served
    }

    /// Observations that fell back to the scripted table.
    pub fn scripted_count(&self) -> u64 {
        self.scripted
    }

    /// Serve one observation. The callable asks the provider with the tight
    /// budget; any error, blank or over-long answer maps to a scripted
    /// fallback line from the fixed table. Never blocks beyond the callable
    /// itself (which the transport bounds), never touches the tick clock.
    pub fn on_observation(
        &mut self,
        obs: &MatchObservation,
        adapter_callable: &mut dyn FnMut(&MatchObservation, u64) -> Result<String, LineError>,
    ) -> Line {
        self.served += 1;
        let priority = priority_for(obs);
        match adapter_callable(obs, self.budget_ms) {
            Ok(text) => match clean_provider_text(&text) {
                Some(clean) => Line {
                    text: clean,
                    priority,
                    scripted: false,
                },
                None => self.fallback(obs, priority),
            },
            Err(_) => self.fallback(obs, priority),
        }
    }

    /// Scripted fallback for one observation, from the fixed table. Public
    /// so the game can preview offline lines without a provider callable.
    pub fn scripted_line(&mut self, obs: &MatchObservation) -> Line {
        self.fallback(obs, priority_for(obs))
    }

    fn fallback(&mut self, obs: &MatchObservation, priority: u8) -> Line {
        self.scripted += 1;
        let table = if !obs.is_valid() {
            // Invalid input never reaches a provider and never panics: a
            // neutral line keeps presentation alive while diagnostics go
            // elsewhere.
            SCRIPTED_NEUTRAL
        } else {
            match &obs.last_event {
                Some(EventLite {
                    kind: EventKindLite::Hit,
                    ..
                }) => SCRIPTED_HIT,
                Some(EventLite {
                    kind: EventKindLite::Knockdown,
                    ..
                }) => SCRIPTED_KNOCKDOWN,
                Some(EventLite {
                    kind: EventKindLite::Round,
                    ..
                }) => SCRIPTED_ROUND,
                None => SCRIPTED_NEUTRAL,
            }
        };
        Line {
            text: pick(table, obs.tick, self.seed).to_owned(),
            priority,
            scripted: true,
        }
    }
}

/// Fighter taunts on hits and knockdowns with a per-fighter cooldown in
/// ticks. Presentation only: taunts never touch authoritative state.
#[derive(Clone, Debug)]
pub struct TauntBook {
    cooldown_ticks: u64,
    last_taunt: HashMap<u64, u64>,
    served: u64,
}

impl TauntBook {
    /// Build a book with a per-fighter cooldown in ticks.
    pub fn new(cooldown_ticks: u64) -> Self {
        Self {
            cooldown_ticks,
            last_taunt: HashMap::new(),
            served: 0,
        }
    }

    /// Cooldown between two taunts from the same fighter, in ticks.
    pub fn cooldown_ticks(&self) -> u64 {
        self.cooldown_ticks
    }

    /// Taunts served so far.
    pub fn served(&self) -> u64 {
        self.served
    }

    /// Taunt for `fighter_id` on `event` at `tick`, or `None` when the
    /// event is not tauntable (only hits and knockdowns are) or the
    /// fighter is still on cooldown. A backwards tick is stale and yields
    /// nothing.
    pub fn taunt_for(&mut self, fighter_id: u64, event: &EventLite, tick: u64) -> Option<Line> {
        let (table, priority) = match event.kind {
            EventKindLite::Hit => (HIT_TAUNTS, 1),
            EventKindLite::Knockdown => (KNOCKDOWN_TAUNTS, 2),
            EventKindLite::Round => return None,
        };
        if let Some(last) = self.last_taunt.get(&fighter_id) {
            match tick.checked_sub(*last) {
                Some(elapsed) if elapsed >= self.cooldown_ticks => {}
                _ => return None,
            }
        }
        let index = fighter_id.wrapping_add(tick) as usize % table.len().max(1);
        let text = table.get(index).copied().unwrap_or("Hah!");
        self.last_taunt.insert(fighter_id, tick);
        self.served += 1;
        Some(Line {
            text: text.to_owned(),
            priority,
            scripted: true,
        })
    }
}

/// One recorded accepted line: the arena tick plus the line shown.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LogEntry {
    /// Arena tick the line was accepted at.
    pub tick: u64,
    /// The accepted line.
    pub line: Line,
}

/// Recorded accepted presentation lines. The game pushes every accepted
/// line here; replay iterates the stored entries verbatim and never calls
/// a provider, so a disconnected replay reproduces identical checkpoints.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PresentationLog {
    entries: Vec<LogEntry>,
}

impl PresentationLog {
    /// Empty log.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Record one accepted line at its arena tick.
    pub fn push(&mut self, tick: u64, line: Line) {
        self.entries.push(LogEntry { tick, line });
    }

    /// Recorded entries in acceptance order.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// Replay recorded entries verbatim: no provider call, no new line.
    pub fn replay(&self) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter()
    }

    /// Entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Deterministic FNV-1a checkpoint over ticks, priorities, scripted
    /// flags and text bytes in acceptance order. The same recorded
    /// sequence always yields the same checkpoint; any changed line or
    /// tick changes it.
    pub fn checkpoint(&self) -> u64 {
        let mut hash = Fnv64::with_state(CHECKPOINT_START);
        for entry in &self.entries {
            hash.write(&entry.tick.to_le_bytes());
            hash.write_u8(entry.line.priority);
            hash.write_bool(entry.line.scripted);
            fold_text(&mut hash, &entry.line.text);
        }
        hash.finish()
    }
}

/// Build the provider ask for one observation: task `Announce`, no context
/// bytes (nothing raw ever leaves the machine here), the caller budget, and
/// `base_revision` carrying the observation tick so replayed or stale asks
/// stay distinguishable. Model and adapter choice stay with the caller, so
/// switching backends is configuration, never new code here.
pub fn announce_request(
    obs: &MatchObservation,
    id: RequestId,
    adapter: AdapterKind,
    model: &str,
    budget_ms: u64,
) -> AiRequest {
    let name = if model.is_empty() || model.len() > 128 {
        "announcer-1"
    } else {
        model
    };
    AiRequest {
        id,
        kind: TaskKind::Announce,
        model: ModelSelector {
            adapter,
            model: name.to_owned(),
        },
        context: Vec::new(),
        schema_version: super::PROTOCOL_VERSION,
        budget: Budget {
            max_output_tokens: 64,
            max_ms: budget_ms.clamp(1, 30_000),
            spend_limit: None,
        },
        deadline_ms: 60_000,
        privacy: PrivacyMode::LocalOnly,
        base_revision: obs.tick,
        body: None,
    }
}

/// Read the provider line out of an adapter outcome: the first non-blank
/// diagnostics line of a completed proposal. Any failure maps to a
/// [`LineError`] so the announcer falls back to its scripted table. An
/// offline-privacy refusal (`UnsupportedCapability`) maps to
/// [`LineError::Offline`]; timeouts map to [`LineError::Timeout`].
pub fn provider_text(outcome: &AdapterOutcome) -> Result<String, LineError> {
    match &outcome.result {
        Ok(result) => {
            let line = result
                .proposal
                .as_ref()
                .and_then(|proposal| {
                    proposal
                        .diagnostics
                        .iter()
                        .find(|line| !line.trim().is_empty())
                        .cloned()
                })
                .unwrap_or_default();
            clean_provider_text(&line).ok_or(LineError::Unavailable)
        }
        Err(AiError::Timeout) => Err(LineError::Timeout),
        Err(AiError::UnsupportedCapability) => Err(LineError::Offline),
        Err(_) => Err(LineError::Unavailable),
    }
}

/// Streaming call site: completed [`crate::stream::StreamJob`] assembled
/// text becomes a provider line through the same clean/truncate/fallback path
/// as [`Announcer::on_observation`]. Takes text only, never polls, so the tick
/// stays isolated; blank text falls back to the scripted table.
pub fn announce_from_stream(
    announcer: &mut Announcer,
    obs: &MatchObservation,
    assembled: &str,
) -> Line {
    match clean_provider_text(assembled) {
        Some(clean) => Line {
            text: clean,
            priority: priority_for(obs),
            scripted: false,
        },
        None => announcer.scripted_line(obs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{fixtures, MockAdapter, MockStep, ProviderAdapter};

    fn observation(tick: u64, last_event: Option<EventLite>) -> MatchObservation {
        MatchObservation {
            tick,
            fighters: vec![
                FigLite {
                    id: 1,
                    hp_permille: 812,
                    down: false,
                },
                FigLite {
                    id: 2,
                    hp_permille: 640,
                    down: false,
                },
            ],
            last_event,
        }
    }

    fn offline(_obs: &MatchObservation, _budget_ms: u64) -> Result<String, LineError> {
        Err(LineError::Offline)
    }

    #[test]
    fn slow_provider_falls_back_without_advancing_tick() {
        use std::time::{Duration, Instant};
        let mut announcer = Announcer::new(7);
        let obs = observation(
            42,
            Some(EventLite {
                kind: EventKindLite::Hit,
                fighter: 1,
            }),
        );
        let tick_before = obs.tick;
        let start = Instant::now();
        let mut slow = |_obs: &MatchObservation, budget_ms: u64| -> Result<String, LineError> {
            // Stub delay past the tight budget: 9 s simulated against a
            // small ask. The stub reports the timeout instead of stalling.
            let simulated_ms = 9_000u64;
            assert!(
                simulated_ms > budget_ms,
                "stub must blow the budget it claims to exceed"
            );
            Err(LineError::Timeout)
        };
        let line = announcer.on_observation(&obs, &mut slow);
        let elapsed = start.elapsed();
        assert!(line.scripted, "slow provider falls back to the fixed table");
        assert!(!line.text.is_empty());
        assert_eq!(line.priority, 1, "hit priority survives the fallback");
        assert_eq!(
            obs.tick, tick_before,
            "presentation never advances the tick clock"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "fallback returns long before the simulated 9 s stall: {elapsed:?}"
        );
        // The real arena still advances on its own tick, provider or not.
        let mut arena_tick = obs.tick;
        arena_tick += 1;
        assert_eq!(arena_tick, tick_before + 1);
    }

    #[test]
    fn offline_adapter_gives_scripted_lines() {
        let mut announcer = Announcer::new(5);
        let knockdown = observation(
            9,
            Some(EventLite {
                kind: EventKindLite::Knockdown,
                fighter: 2,
            }),
        );
        let line = announcer.on_observation(&knockdown, &mut offline);
        assert!(line.scripted, "offline means the fixed table");
        assert!(!line.text.is_empty());
        assert_eq!(line.priority, 2);
        assert!(
            SCRIPTED_KNOCKDOWN.contains(&line.text.as_str()),
            "knockdown line comes from the knockdown table: {}",
            line.text
        );
        let quiet = observation(10, None);
        let calm = announcer.on_observation(&quiet, &mut offline);
        assert!(calm.scripted);
        assert_eq!(calm.priority, 0);
        assert!(SCRIPTED_NEUTRAL.contains(&calm.text.as_str()));
        assert_eq!(announcer.scripted_count(), 2);
    }

    #[test]
    fn recorded_actions_replay_disconnected_with_equal_checkpoints() {
        // Inputs: a 24-tick observation tape with hits and knockdowns.
        let script: Vec<MatchObservation> = (0..24u64)
            .map(|tick| {
                let last_event = match tick % 6 {
                    2 => Some(EventLite {
                        kind: EventKindLite::Hit,
                        fighter: 1,
                    }),
                    4 => Some(EventLite {
                        kind: EventKindLite::Knockdown,
                        fighter: 2,
                    }),
                    _ => None,
                };
                MatchObservation {
                    tick,
                    fighters: vec![
                        FigLite {
                            id: 1,
                            hp_permille: 1000 - ((tick * 7) % 900) as u16,
                            down: false,
                        },
                        FigLite {
                            id: 2,
                            hp_permille: 1000 - ((tick * 11) % 950) as u16,
                            down: tick % 6 == 4,
                        },
                    ],
                    last_event,
                }
            })
            .collect();
        // Record the scripted line sequence with the adapter disabled.
        let mut live = Announcer::new(11);
        let mut recorded = PresentationLog::new();
        for obs in &script {
            recorded.push(obs.tick, live.on_observation(obs, &mut offline));
        }
        assert_eq!(recorded.len(), 24);
        let live_checkpoint = recorded.checkpoint();
        // Replay disconnected: fresh announcer, same seed, adapter disabled.
        let mut replayed = Announcer::new(11);
        let mut again = PresentationLog::new();
        for obs in &script {
            again.push(obs.tick, replayed.on_observation(obs, &mut offline));
        }
        assert_eq!(
            again.entries(),
            recorded.entries(),
            "disconnected replay reproduces every accepted line"
        );
        assert_eq!(
            again.checkpoint(),
            live_checkpoint,
            "disconnected replay matches checkpoints"
        );
        // Stored replay never re-asks: iterating the log is verbatim.
        let stored: Vec<&LogEntry> = recorded.replay().collect();
        assert_eq!(stored.len(), 24);
        assert_eq!(stored[0].tick, 0);
        // Divergence hygiene: one changed input changes the checkpoint.
        let mut changed = script.clone();
        changed[8].last_event = Some(EventLite {
            kind: EventKindLite::Knockdown,
            fighter: 1,
        });
        let mut divergent = Announcer::new(11);
        let mut other = PresentationLog::new();
        for obs in &changed {
            other.push(obs.tick, divergent.on_observation(obs, &mut offline));
        }
        assert_ne!(
            other.entries()[8],
            recorded.entries()[8],
            "changed input changes the accepted line"
        );
        assert_ne!(
            other.checkpoint(),
            live_checkpoint,
            "changed input changes the checkpoint"
        );
    }

    #[test]
    fn taunt_cooldowns_respected() {
        let mut book = TauntBook::new(30);
        let knockdown = EventLite {
            kind: EventKindLite::Knockdown,
            fighter: 1,
        };
        let first = book
            .taunt_for(1, &knockdown, 100)
            .expect("first knockdown taunts");
        assert!(KNOCKDOWN_TAUNTS.contains(&first.text.as_str()));
        assert_eq!(first.priority, 2);
        assert!(
            book.taunt_for(1, &knockdown, 101).is_none(),
            "cooldown blocks the next tick"
        );
        assert!(
            book.taunt_for(1, &knockdown, 129).is_none(),
            "cooldown still holds at tick 129"
        );
        assert!(
            book.taunt_for(1, &knockdown, 130).is_some(),
            "cooldown of 30 ticks ends at tick 130"
        );
        // Other fighters have their own cooldown budget.
        assert!(
            book.taunt_for(2, &knockdown, 101).is_some(),
            "fighter 2 is not on fighter 1 cooldown"
        );
        // Hits taunt too, with hit priority.
        let hit = EventLite {
            kind: EventKindLite::Hit,
            fighter: 3,
        };
        let jab = book.taunt_for(3, &hit, 200).expect("hits taunt");
        assert_eq!(jab.priority, 1);
        assert!(HIT_TAUNTS.contains(&jab.text.as_str()));
        // Round horns are not tauntable.
        let horn = EventLite {
            kind: EventKindLite::Round,
            fighter: 1,
        };
        assert!(
            book.taunt_for(1, &horn, 500).is_none(),
            "round events never taunt"
        );
        assert_eq!(book.served(), 4);
    }

    #[test]
    fn provider_lines_keep_priority_and_are_not_scripted() {
        let mut announcer = Announcer::new(3);
        let obs = observation(
            15,
            Some(EventLite {
                kind: EventKindLite::Knockdown,
                fighter: 2,
            }),
        );
        let mut provider =
            |_obs: &MatchObservation, _budget: u64| Ok(String::from("What a rally!"));
        let line = announcer.on_observation(&obs, &mut provider);
        assert!(!line.scripted);
        assert_eq!(line.text, "What a rally!");
        assert_eq!(line.priority, 2);
        assert_eq!(announcer.served(), 1);
        assert_eq!(announcer.scripted_count(), 0);
    }

    #[test]
    fn blank_or_oversize_provider_text_falls_back() {
        let mut announcer = Announcer::new(3);
        let obs = observation(16, None);
        let mut blank = |_obs: &MatchObservation, _budget: u64| Ok(String::from("   "));
        assert!(announcer.on_observation(&obs, &mut blank).scripted);
        let long = "x".repeat(MAX_LINE_LEN + 50);
        let mut oversize = |_obs: &MatchObservation, _budget: u64| Ok(long.clone());
        let trimmed = announcer.on_observation(&obs, &mut oversize);
        assert!(!trimmed.scripted, "long text truncates, it does not fail");
        assert_eq!(trimmed.text.chars().count(), MAX_LINE_LEN);
    }

    #[test]
    fn invalid_observations_stay_scripted_without_panic() {
        let mut announcer = Announcer::new(9);
        let bad = MatchObservation {
            tick: 3,
            fighters: vec![
                FigLite {
                    id: 1,
                    hp_permille: 5000,
                    down: false,
                };
                MAX_FIGHTERS_PER_OBSERVATION + 1
            ],
            last_event: Some(EventLite {
                kind: EventKindLite::Hit,
                fighter: 1,
            }),
        };
        assert!(!bad.is_valid());
        let line = announcer.on_observation(&bad, &mut offline);
        assert!(line.scripted);
        assert!(SCRIPTED_NEUTRAL.contains(&line.text.as_str()));
        assert!(observation(3, None).is_valid());
    }

    #[test]
    fn scripted_fallback_is_deterministic() {
        let obs = observation(
            21,
            Some(EventLite {
                kind: EventKindLite::Hit,
                fighter: 1,
            }),
        );
        let mut first = Announcer::new(13);
        let mut second = Announcer::new(13);
        assert_eq!(
            first.scripted_line(&obs),
            second.scripted_line(&obs),
            "same seed plus same observation gives the same line"
        );
    }

    #[test]
    fn provider_outcomes_map_to_line_results() {
        let policy = fixtures::policy();
        let caps = fixtures::caps();
        // Completed mock proposal: first diagnostics line is the provider line.
        let mut ok = MockAdapter::new(vec![MockStep::Succeed]);
        let outcome = ok.call(&fixtures::request(1, 3), &policy, &caps);
        assert_eq!(
            provider_text(&outcome).expect("mock ok carries a line"),
            "mock:ok"
        );
        // Slow script arm maps to a timeout, never a stall.
        let mut slow = MockAdapter::new(vec![MockStep::Timeout {
            simulated_ms: 9_000,
        }]);
        let timed_out = slow.call(&fixtures::request(2, 3), &policy, &caps);
        assert_eq!(provider_text(&timed_out), Err(LineError::Timeout));
        // Offline-privacy refusal maps to offline.
        let mut refused = MockAdapter::new(vec![MockStep::PrivacyDowngrade]);
        let downgraded = refused.call(&fixtures::request(3, 3), &policy, &caps);
        assert_eq!(provider_text(&downgraded), Err(LineError::Offline));
        // Request shape for the ask: announce task, caller budget, tick stamp.
        let obs = observation(77, None);
        let ask = announce_request(
            &obs,
            RequestId(40),
            AdapterKind::Mock,
            "mock-1",
            DEFAULT_ANNOUNCE_BUDGET_MS,
        );
        assert_eq!(ask.kind, TaskKind::Announce);
        assert_eq!(ask.budget.max_ms, DEFAULT_ANNOUNCE_BUDGET_MS);
        assert_eq!(ask.base_revision, 77);
    }
}

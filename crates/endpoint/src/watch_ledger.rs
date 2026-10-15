//! Watch resume plus the per-verb token ledger over the query surface.
//!
//! Owner: endpoint. Consumers: `ServerState::scene_query` records every
//! answered call into the door ledger through [`note_door_answer`]; the churn
//! test below drives the real [`WatchHub`][super::discover::WatchHub] beside a
//! per-test ledger. Units: chars on the wire, tokens estimated at
//! [`CHARS_PER_TOKEN`], revisions and digests as `u64`.
//!
//! Determinism: `BTreeMap` iteration plus integer math only; the same call
//! sequence gives the same table text and the same tape digest. A resumed
//! session rejoins at the coalesced latest digest, so disconnect churn never
//! forks the tape. Errors: typed [`LedgerError`]; every refusal names the
//! recovery. Budgets: history holds at most [`MAX_TAPE_HISTORY`] revisions;
//! the table renderer is a door/report path and may allocate.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use engine_core::Fnv64;

use super::scene_query::{QueryAnswer, QueryKind};

/// Query verbs on the hardened surface: the eight [`QueryKind`] words.
/// Rows are indexed by the frozen discriminant through [`verb_index`], so an
/// appended kind fails to compile here instead of aliasing a row.
pub const QUERY_VERB_COUNT: usize = 8;
/// Estimated chars per model token on the door (Blender/Unity/Godot MCP
/// comparison point: our ledger posts chars plus this estimate, never a
/// vendor tokenizer).
pub const CHARS_PER_TOKEN: u64 = 4;
/// Bounded revision history: resume past this horizon must re-subscribe.
pub const MAX_TAPE_HISTORY: usize = 256;
/// Digest tag so tape bytes never alias another domain hash.
const TAPE_TAG: &[u8] = b"e2watchtape:1";

/// Map a query kind onto its ledger row. Exhaustive on purpose: a new kind
/// must extend the table, never silently share a row.
fn verb_index(kind: QueryKind) -> usize {
    match kind {
        QueryKind::Near => 0,
        QueryKind::Between => 1,
        QueryKind::Visible => 2,
        QueryKind::Reachable => 3,
        QueryKind::CoverNear => 4,
        QueryKind::LargestOpen => 5,
        QueryKind::FreeSpace => 6,
        QueryKind::Cone => 7,
    }
}

/// Measured cost of one query verb: call count plus wire chars in both
/// directions. All counters saturate; the ledger never wraps a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerbRow {
    /// Which verb this row budgets.
    pub kind: QueryKind,
    /// Answered calls recorded.
    pub calls: u64,
    /// Request wire chars (door JSON), summed.
    pub request_chars: u64,
    /// Answer bytes rendered, summed.
    pub answer_chars: u64,
    /// Answers that dropped hits to fit the budget.
    pub truncated: u64,
}

impl VerbRow {
    /// Empty row for one verb.
    pub fn new(kind: QueryKind) -> Self {
        Self {
            kind,
            calls: 0,
            request_chars: 0,
            answer_chars: 0,
            truncated: 0,
        }
    }

    /// Mean request chars per call; 0 before the first call.
    pub fn avg_request_chars(&self) -> u64 {
        self.request_chars.checked_div(self.calls).unwrap_or(0)
    }

    /// Mean answer chars per call; 0 before the first call.
    pub fn avg_answer_chars(&self) -> u64 {
        self.answer_chars.checked_div(self.calls).unwrap_or(0)
    }

    /// Estimated tokens per call at [`CHARS_PER_TOKEN`]; 0 with no calls.
    pub fn est_tokens_per_call(&self) -> u64 {
        if self.calls == 0 {
            0
        } else {
            self.request_chars
                .saturating_add(self.answer_chars)
                .saturating_div(self.calls.saturating_mul(CHARS_PER_TOKEN).max(1))
        }
    }
}

/// A session's last confirmed position on the tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeCursor {
    /// Owning session.
    pub session: u64,
    /// Last revision the client confirmed.
    pub revision: u64,
    /// Digest the client saw at that revision.
    pub digest: u64,
    /// Revisions missed since that confirm (0 when current).
    pub missed: u64,
}

/// What rejoining after churn found: the coalesced latest plus the gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResumeReport {
    /// Session that rejoined.
    pub session: u64,
    /// Latest revision on the tape.
    pub revision: u64,
    /// Digest at the latest revision.
    pub digest: u64,
    /// Revisions published while the client was away.
    pub missed: u64,
    /// Bursts the hub collapsed to the latest digest.
    pub coalesced: u64,
    /// True when the client's digest disagrees with the tape: re-read.
    pub diverged: bool,
}

/// Typed ledger failure. `hint` always names the recovery, so every refusal
/// is actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerError {
    /// Machine code (`unknown_session`, `unknown_revision`).
    pub code: &'static str,
    /// What went wrong.
    pub message: String,
    /// Recovery for the caller.
    pub hint: String,
}

impl LedgerError {
    /// Build one refusal with code, detail and recovery.
    pub fn new(code: &'static str, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: hint.into(),
        }
    }
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} (hint: {})", self.code, self.message, self.hint)
    }
}

impl std::error::Error for LedgerError {}

/// Churn-proof resume plus the per-verb token ledger. Feed publishes through
/// [`WatchLedger::record_publish`] beside every hub publish, answers through
/// [`WatchLedger::record_query`], and session confirms through
/// [`WatchLedger::note_session`]; [`WatchLedger::resume`] then rejoins a
/// churned client at the coalesced latest with a measured gap.
#[derive(Debug, Clone)]
pub struct WatchLedger {
    verbs: [VerbRow; QUERY_VERB_COUNT],
    latest_revision: u64,
    latest_digest: u64,
    publishes: u64,
    stale_publishes: u64,
    history: BTreeMap<u64, u64>,
    cursors: BTreeMap<u64, ResumeCursor>,
}

impl WatchLedger {
    /// Empty ledger: no calls, no tape, no sessions.
    pub fn new() -> Self {
        Self {
            verbs: [
                VerbRow::new(QueryKind::Near),
                VerbRow::new(QueryKind::Between),
                VerbRow::new(QueryKind::Visible),
                VerbRow::new(QueryKind::Reachable),
                VerbRow::new(QueryKind::CoverNear),
                VerbRow::new(QueryKind::LargestOpen),
                VerbRow::new(QueryKind::FreeSpace),
                VerbRow::new(QueryKind::Cone),
            ],
            latest_revision: 0,
            latest_digest: 0,
            publishes: 0,
            stale_publishes: 0,
            history: BTreeMap::new(),
            cursors: BTreeMap::new(),
        }
    }

    /// Record one answered query: per-verb call plus wire chars in both
    /// directions. `request_chars` is the door JSON length the caller
    /// measured; `answer` carries the rendered byte size.
    pub fn record_query(&mut self, kind: QueryKind, request_chars: usize, answer: &QueryAnswer) {
        let row = &mut self.verbs[verb_index(kind)];
        row.calls = row.calls.saturating_add(1);
        row.request_chars = row
            .request_chars
            .saturating_add(u64::try_from(request_chars).unwrap_or(u64::MAX));
        row.answer_chars = row
            .answer_chars
            .saturating_add(u64::try_from(answer.bytes).unwrap_or(u64::MAX));
        if answer.truncated {
            row.truncated = row.truncated.saturating_add(1);
        }
    }

    /// Mirror one hub publish onto the tape. Strictly increasing revisions
    /// advance the latest cursor; stale or duplicate revisions count
    /// separately and never move it.
    pub fn record_publish(&mut self, revision: u64, digest: u64) {
        self.publishes = self.publishes.saturating_add(1);
        if revision > self.latest_revision {
            self.latest_revision = revision;
            self.latest_digest = digest;
            self.history.insert(revision, digest);
            while self.history.len() > MAX_TAPE_HISTORY {
                let oldest = *self.history.keys().next().unwrap_or(&revision);
                self.history.remove(&oldest);
            }
        } else {
            self.stale_publishes = self.stale_publishes.saturating_add(1);
        }
    }

    /// Confirm a session saw `(revision, digest)`: upserts its cursor with
    /// zero missed. Unknown sessions join here before their first resume.
    pub fn note_session(&mut self, session: u64, revision: u64, digest: u64) {
        self.cursors.insert(
            session,
            ResumeCursor {
                session,
                revision,
                digest,
                missed: 0,
            },
        );
    }

    /// Last confirmed cursor for a session, if it ever joined.
    pub fn cursor(&self, session: u64) -> Option<ResumeCursor> {
        self.cursors.get(&session).copied()
    }

    /// Latest tape position `(revision, digest, publishes)`.
    pub fn latest(&self) -> (u64, u64, u64) {
        (self.latest_revision, self.latest_digest, self.publishes)
    }

    /// Stale or duplicate publishes that never moved the tape.
    pub fn stale_publishes(&self) -> u64 {
        self.stale_publishes
    }

    /// Rejoin a churned session at the coalesced latest. Unknown sessions
    /// refuse with `unknown_session`; revisions past the horizon or the
    /// future refuse with `unknown_revision`. A digest that disagrees with
    /// the tape reports `diverged` so the client re-reads instead of
    /// forking. The cursor always advances to the latest on success.
    pub fn resume(
        &mut self,
        session: u64,
        revision: u64,
        digest: u64,
    ) -> Result<ResumeReport, LedgerError> {
        if !self.cursors.contains_key(&session) {
            return Err(LedgerError::new(
                "unknown_session",
                format!("session {session} never joined the tape"),
                "confirm one position with note_session first, then resume",
            ));
        }
        if revision > self.latest_revision {
            return Err(LedgerError::new(
                "unknown_revision",
                format!(
                    "revision {revision} is past the latest {}",
                    self.latest_revision
                ),
                "re-subscribe fresh and confirm the live baseline digest",
            ));
        }
        let taped = if revision == self.latest_revision {
            Some(self.latest_digest)
        } else {
            self.history.get(&revision).copied()
        };
        let Some(taped) = taped else {
            return Err(LedgerError::new(
                "unknown_revision",
                format!("revision {revision} fell past the {MAX_TAPE_HISTORY}-entry horizon"),
                "re-subscribe fresh and confirm the live baseline digest",
            ));
        };
        let diverged = taped != digest;
        let missed = self.latest_revision.saturating_sub(revision);
        let report = ResumeReport {
            session,
            revision: self.latest_revision,
            digest: self.latest_digest,
            missed,
            coalesced: missed,
            diverged,
        };
        self.cursors.insert(
            session,
            ResumeCursor {
                session,
                revision: self.latest_revision,
                digest: self.latest_digest,
                missed,
            },
        );
        Ok(report)
    }

    /// Cost row for one verb.
    pub fn verb_row(&self, kind: QueryKind) -> VerbRow {
        self.verbs[verb_index(kind)]
    }

    /// Totals across verbs: `(calls, request_chars, answer_chars)`.
    pub fn totals(&self) -> (u64, u64, u64) {
        let mut calls = 0u64;
        let mut requests = 0u64;
        let mut answers = 0u64;
        for row in &self.verbs {
            calls = calls.saturating_add(row.calls);
            requests = requests.saturating_add(row.request_chars);
            answers = answers.saturating_add(row.answer_chars);
        }
        (calls, requests, answers)
    }

    /// Post the token table: one line per verb plus totals, chars and the
    /// [`CHARS_PER_TOKEN`] estimate. Same calls give byte-identical text.
    pub fn token_table_text(&self) -> String {
        let mut text =
            String::from("verb calls req_chars ans_chars req_avg ans_avg est_tok truncated\n");
        for row in &self.verbs {
            text.push_str(&format!(
                "{} {} {} {} {} {} {} {}\n",
                row.kind.name(),
                row.calls,
                row.request_chars,
                row.answer_chars,
                row.avg_request_chars(),
                row.avg_answer_chars(),
                row.est_tokens_per_call(),
                row.truncated,
            ));
        }
        let (calls, requests, answers) = self.totals();
        let est = requests
            .saturating_add(answers)
            .saturating_div(CHARS_PER_TOKEN.max(1));
        text.push_str(&format!(
            "total {calls} {requests} {answers} est_tok {est}\n"
        ));
        text
    }

    /// Fold step digests into one session-tape digest. Order matters: the
    /// same steps in another order give another tape.
    pub fn fold_tape(digests: &[u64]) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(TAPE_TAG);
        for digest in digests {
            hasher.write(&digest.to_le_bytes());
        }
        hasher.finish()
    }
}

impl Default for WatchLedger {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide door ledger behind the tool path (the query tool takes no
/// ledger argument), mirroring the process-wide watch hub. Queries never
/// fail for ledger reasons: a poisoned lock skips recording.
static DOOR_LEDGER: OnceLock<Mutex<WatchLedger>> = OnceLock::new();

/// Record one answered door call into the process-wide ledger.
pub fn note_door_answer(kind: QueryKind, request_chars: usize, answer: &QueryAnswer) {
    if let Ok(mut ledger) = DOOR_LEDGER
        .get_or_init(|| Mutex::new(WatchLedger::new()))
        .lock()
    {
        ledger.record_query(kind, request_chars, answer);
    }
}

#[cfg(test)]
mod tests {
    use super::super::discover::{WatchConfig, WatchHub, WatchKind};
    use super::super::scene_query::{run_query, QueryArgs};
    use super::*;
    use engine_spatial::sight::ReachView;
    use engine_spatial::Actor;
    use std::collections::BTreeMap;

    /// Session under churn in the tape test.
    const SESSION: u64 = 11;
    /// Twenty steps: every verb at least twice, churn at 7 and 14.
    const STEPS: usize = 20;

    fn actor(id: u32, x: f32, z: f32) -> Actor {
        Actor {
            id,
            name: format!("crate_{id}"),
            x,
            y: 0.0,
            z,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn fixture() -> (Vec<Actor>, ReachView) {
        let actors = vec![
            actor(0, 0.0, 0.0),
            actor(1, 3.0, 0.0),
            actor(2, 0.0, 4.0),
            actor(3, -2.0, -2.0),
        ];
        let mut at = BTreeMap::new();
        for a in &actors {
            at.insert(a.id, 0u32);
        }
        let mut adj = BTreeMap::new();
        adj.insert(0u32, Vec::new());
        (actors, ReachView { adj, at })
    }

    /// Step `i` asks verb `i % QUERY_VERB_COUNT` with fixed args, so two runs agree.
    fn step_args(i: usize) -> QueryArgs {
        match i % QUERY_VERB_COUNT {
            0 => QueryArgs::near(0, 500),
            1 => QueryArgs::between(0, 1),
            2 => QueryArgs::visible([0.0, 1.6, 6.0], 0.0),
            3 => QueryArgs::reachable(0),
            4 => QueryArgs::cover_near(0, [5.0, 1.6, 5.0], false, 500),
            5 => QueryArgs::largest_open(30),
            6 => QueryArgs::free_space(30, 100, 8, 7),
            _ => QueryArgs::cone_from(0, 90, 500),
        }
    }

    /// One 20-step run: query, publish, read delivered with zero polls,
    /// churn (drop + resubscribe + resume) at steps 7 and 14. Returns the
    /// tape digest, the ledger latest and the final poll count.
    fn run_tape() -> (u64, (u64, u64, u64), u64) {
        let (actors, reach) = fixture();
        let hub = WatchHub::new();
        let mut ledger = WatchLedger::new();
        let mut sub = hub
            .subscribe(SESSION, WatchConfig::default_policy())
            .expect("subscribe opens");
        ledger.note_session(SESSION, 0, 0);
        let mut tape: Vec<u64> = Vec::new();
        let mut last_rev = 0u64;
        let mut last_digest = 0u64;
        for i in 0..STEPS {
            if i == 7 || i == 14 {
                // Disconnect churn: the handle dies, two publishes land
                // while away, then the client resubscribes and rejoins.
                drop(sub);
                hub.prune().expect("prune clears the dead slot");
                for _ in 0..2 {
                    last_rev += 1;
                    last_digest = last_digest.wrapping_add(0x9E37_79B9_7F4A_7C15);
                    hub.publish(
                        WatchKind::Scene,
                        last_rev,
                        last_digest,
                        "churn gap",
                        Some(99),
                    )
                    .expect("gap publishes fan out");
                    ledger.record_publish(last_rev, last_digest);
                }
                sub = hub
                    .subscribe(SESSION, WatchConfig::default_policy())
                    .expect("resubscribe opens");
                let report = ledger
                    .resume(SESSION, last_rev - 2, ledger.history[&(last_rev - 2)])
                    .expect("resume rejoins");
                assert_eq!(report.missed, 2, "churn gap measures two");
                assert!(!report.diverged, "gap digests stay on the tape");
            }
            let args = step_args(i);
            let answer = run_query(&actors, &reach, &args).expect("fixture answers");
            ledger.record_query(args.kind, args.kind.name().len(), &answer);
            last_rev += 1;
            hub.publish(WatchKind::Scene, last_rev, answer.digest, "step", None)
                .expect("step publishes");
            ledger.record_publish(last_rev, answer.digest);
            last_digest = answer.digest;
            let delivered = sub.delivered().expect("delivered reads free");
            let seen = delivered.last().expect("every publish lands").digest;
            assert_eq!(seen, answer.digest, "zero-poll read sees the step");
            tape.push(seen);
            ledger.note_session(SESSION, last_rev, last_digest);
        }
        let polls = sub.polls().expect("poll counter reads");
        (WatchLedger::fold_tape(&tape), ledger.latest(), polls)
    }

    #[test]
    fn churned_tape_replays_identical_with_zero_polls() {
        let (tape_a, latest_a, polls_a) = run_tape();
        let (tape_b, latest_b, polls_b) = run_tape();
        assert_eq!(tape_a, tape_b, "same 20 steps give the same tape x2");
        assert_eq!(latest_a, latest_b, "ledger latest agrees x2");
        assert_eq!(polls_a, 0, "watchers read delivered, never poll");
        assert_eq!(polls_b, 0, "replay reads delivered, never polls");
    }

    #[test]
    fn resume_refuses_unknown_session() {
        let mut ledger = WatchLedger::new();
        ledger.record_publish(1, 42);
        let error = ledger
            .resume(9, 1, 42)
            .expect_err("strangers cannot resume");
        assert_eq!(error.code, "unknown_session");
        assert!(error.hint.contains("note_session"), "hint names the join");
    }

    #[test]
    fn resume_flags_diverged_digest() {
        let mut ledger = WatchLedger::new();
        ledger.note_session(SESSION, 1, 111);
        ledger.record_publish(1, 111);
        ledger.record_publish(2, 222);
        let report = ledger.resume(SESSION, 1, 999).expect("divergence reports");
        assert!(report.diverged, "wrong digest flags, never forks");
        assert_eq!(report.missed, 1, "gap still measures");
        assert_eq!(report.digest, 222, "cursor still advances to latest");
    }

    #[test]
    fn resume_refuses_future_and_evicted_revisions() {
        let mut ledger = WatchLedger::new();
        ledger.note_session(SESSION, 0, 0);
        ledger.record_publish(3, 33);
        let future = ledger.resume(SESSION, 4, 0).expect_err("future refuses");
        assert_eq!(future.code, "unknown_revision");
        // Flood past the horizon, then ask for the evicted first entry.
        for rev in 4..=(MAX_TAPE_HISTORY as u64 + 10) {
            ledger.record_publish(rev, rev);
        }
        assert!(
            !ledger.history.contains_key(&3),
            "horizon evicts the oldest first"
        );
        let evicted = ledger.resume(SESSION, 3, 33).expect_err("evicted refuses");
        assert_eq!(evicted.code, "unknown_revision");
        assert!(evicted.hint.contains("re-subscribe"), "hint names the redo");
    }

    #[test]
    fn stale_publish_never_moves_latest() {
        let mut ledger = WatchLedger::new();
        ledger.record_publish(5, 50);
        ledger.record_publish(5, 51);
        ledger.record_publish(2, 20);
        assert_eq!(ledger.latest(), (5, 50, 3), "duplicates never move latest");
        assert_eq!(ledger.stale_publishes(), 2, "stale counts apart");
    }

    #[test]
    fn token_table_posts_all_eight_verbs() {
        let (actors, reach) = fixture();
        let mut ledger = WatchLedger::new();
        for i in 0..QUERY_VERB_COUNT {
            let args = step_args(i);
            let answer = run_query(&actors, &reach, &args).expect("fixture answers");
            ledger.record_query(args.kind, 64 + i, &answer);
        }
        let (calls, requests, _) = ledger.totals();
        assert_eq!(calls, QUERY_VERB_COUNT as u64, "one call per verb");
        assert_eq!(
            requests,
            (0..QUERY_VERB_COUNT)
                .map(|i: usize| 64u64.saturating_add(u64::try_from(i).unwrap_or(u64::MAX)))
                .sum::<u64>(),
            "requests sum exactly"
        );
        let text = ledger.token_table_text();
        for name in [
            "near",
            "between",
            "visible",
            "reachable",
            "cover_near",
            "largest_open",
            "free_space",
            "cone",
        ] {
            assert!(text.contains(name), "table posts {name}");
        }
        assert!(
            text.contains(&format!("total {QUERY_VERB_COUNT} ")),
            "table posts the total"
        );
        // Same calls give byte-identical text.
        assert_eq!(text, ledger.token_table_text(), "table text is stable");
        // Empty ledger still posts all eight rows at zero.
        let blank = WatchLedger::new().token_table_text();
        assert!(blank.contains("near 0 0 0"), "blank rows read zero");
    }

    #[test]
    fn verb_rows_average_and_estimate() {
        let mut row = VerbRow::new(QueryKind::Near);
        assert_eq!(row.avg_request_chars(), 0, "no calls, no average");
        assert_eq!(row.est_tokens_per_call(), 0, "no calls, no estimate");
        row.calls = 2;
        row.request_chars = 100;
        row.answer_chars = 300;
        assert_eq!(row.avg_request_chars(), 50, "request mean divides");
        assert_eq!(row.avg_answer_chars(), 150, "answer mean divides");
        assert_eq!(
            row.est_tokens_per_call(),
            50,
            "400 chars over 2 calls at 4 chars per token"
        );
    }

    #[test]
    fn truncated_answers_count_apart() {
        let (actors, reach) = fixture();
        let mut ledger = WatchLedger::new();
        let mut args = QueryArgs::near(0, 500);
        args.budget_bytes = 1;
        args.limit = 1;
        let answer = run_query(&actors, &reach, &args).expect("tiny budget answers");
        assert!(answer.truncated, "one-byte budget must truncate");
        ledger.record_query(args.kind, 8, &answer);
        assert_eq!(
            ledger.verb_row(QueryKind::Near).truncated,
            1,
            "truncation counts apart"
        );
    }
}

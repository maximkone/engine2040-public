//! memory: typed facts behind a policy write gate.
//!
//! Consumer: the endpoint memory pack ([`crate::memory`] callers) and any
//! agent that needs recall across sessions. A fact carries a declared type
//! (`fact`, `preference`, `decision`), a bounded value, an integer confidence
//! in `0..=1000`, its source (session id plus turn), a monotonic write tick
//! and whether it is shared with other sessions.
//!
//! A write needs a policy key: the session's own capability token plus the
//! grants that token carries. No key (`memory_no_key`), a token that does not
//! match the session's registration (`memory_wrong_key`) or one that did not
//! grant `memory.write` (`memory_not_granted`) is refused by name, and a
//! refused write leaves the store byte-identical.
//!
//! A read is scoped: a session recalls its own facts plus the facts another
//! session marked shared, and nothing else.
//!
//! Units: confidence in thousandths, ticks as write counts, bytes for bounds.
//! Determinism: the same writes in the same order always produce the same
//! store bytes; recall order is `(type, tick, id)` and never a clock or RNG.
//! Errors: [`MemoryError`], one named variant per refusal class.

use std::fmt;

/// Longest fact value kept, in bytes.
pub const MAX_VALUE_LEN: usize = 512;

/// Confidence ceiling: 1000 is certainty, 0 is a guess.
pub const MAX_CONFIDENCE: u16 = 1000;

/// The `memory.write` grant bit.
pub const MEMORY_WRITE: u16 = 1 << 0;

/// Declared fact type. The integer code is what encode and the recall order
/// use, so a new type never renumbers an existing one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FactType {
    /// A statement about the world.
    Fact,
    /// How a person wants things done.
    Preference,
    /// A choice that was made and holds.
    Decision,
}

impl FactType {
    /// Stable name on the wire and in recall rows.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Decision => "decision",
        }
    }

    /// Stable integer code used by encode and by the recall order.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            Self::Fact => 0,
            Self::Preference => 1,
            Self::Decision => 2,
        }
    }

    /// Type from its encoded code; `None` for a code this build never wrote.
    #[must_use]
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Fact),
            1 => Some(Self::Preference),
            2 => Some(Self::Decision),
            _ => None,
        }
    }

    /// Type from its wire name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "fact" => Some(Self::Fact),
            "preference" => Some(Self::Preference),
            "decision" => Some(Self::Decision),
            _ => None,
        }
    }
}

/// Where a fact came from: the session and the turn inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Source {
    /// Session that stated the fact.
    pub session: u64,
    /// Turn inside that session.
    pub turn: u32,
}

/// What a caller offers to write; the store assigns id and tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    /// Declared type.
    pub kind: FactType,
    /// The value itself, at most [`MAX_VALUE_LEN`] bytes.
    pub value: String,
    /// Confidence in `0..=1000`.
    pub confidence: u16,
    /// Session and turn that stated it.
    pub source: Source,
    /// True when other sessions may recall it.
    pub shared: bool,
}

/// One stored fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    /// Stable identity; equals [`Self::tick`], so a replayed store names the
    /// same facts.
    pub id: u64,
    /// Declared type.
    pub kind: FactType,
    /// The value itself.
    pub value: String,
    /// Confidence in `0..=1000`.
    pub confidence: u16,
    /// Session and turn that stated it.
    pub source: Source,
    /// Monotonic write tick, starting at 1.
    pub tick: u64,
    /// True when other sessions may recall it.
    pub shared: bool,
}

/// The session's own capability token plus the grants it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryKey {
    session: u64,
    token: u64,
    grants: u16,
}

impl MemoryKey {
    /// A key for `session` presenting `token` with `grants`.
    #[must_use]
    pub fn new(session: u64, token: u64, grants: u16) -> Self {
        Self {
            session,
            token,
            grants,
        }
    }

    /// A key that may write: the one entry point the harness uses.
    #[must_use]
    pub fn writer(session: u64, token: u64) -> Self {
        Self::new(session, token, MEMORY_WRITE)
    }

    /// A key that may only recall.
    #[must_use]
    pub fn reader(session: u64, token: u64) -> Self {
        Self::new(session, token, 0)
    }

    /// Session this key belongs to.
    #[must_use]
    pub fn session(self) -> u64 {
        self.session
    }
}

/// What refused a memory operation, one named variant per class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    /// The write carried no policy key at all.
    NoKey,
    /// The key's token does not match the session's registration.
    WrongKey,
    /// The key carries no `memory.write` grant.
    NotGranted,
    /// The value is over [`MAX_VALUE_LEN`]; nothing was kept.
    ValueTooLong {
        /// Byte length offered.
        len: usize,
        /// The bound that refused it.
        cap: usize,
    },
    /// Confidence is over [`MAX_CONFIDENCE`]; nothing was kept.
    BadConfidence {
        /// Confidence offered.
        value: u16,
    },
    /// No stored fact carries the id asked for.
    NoFact {
        /// The id asked for.
        id: u64,
    },
    /// The fact belongs to another session; only its writer may share it.
    NotYours {
        /// The fact's id.
        id: u64,
    },
    /// An encoded store line could not be read back.
    Malformed {
        /// 1-based line that failed.
        line: usize,
    },
    /// The tick counter is exhausted (`u64::MAX` writes).
    SequenceExhausted,
}

impl MemoryError {
    /// Stable machine name, so callers branch on the refusal, not on prose.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::NoKey => "memory_no_key",
            Self::WrongKey => "memory_wrong_key",
            Self::NotGranted => "memory_not_granted",
            Self::ValueTooLong { .. } => "memory_value_too_long",
            Self::BadConfidence { .. } => "memory_bad_confidence",
            Self::NoFact { .. } => "memory_no_fact",
            Self::NotYours { .. } => "memory_not_yours",
            Self::Malformed { .. } => "memory_malformed",
            Self::SequenceExhausted => "memory_sequence_exhausted",
        }
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoKey => f.write_str("memory write refused: no policy key presented"),
            Self::WrongKey => {
                f.write_str("memory write refused: the key's token is not this session's")
            }
            Self::NotGranted => {
                f.write_str("memory write refused: the key did not grant memory.write")
            }
            Self::ValueTooLong { len, cap } => {
                write!(f, "memory write refused: value is {len} bytes, cap {cap}")
            }
            Self::BadConfidence { value } => {
                write!(
                    f,
                    "memory write refused: confidence {value} over {MAX_CONFIDENCE}"
                )
            }
            Self::NoFact { id } => write!(f, "memory share refused: no fact has id {id}"),
            Self::NotYours { id } => {
                write!(f, "memory share refused: fact {id} belongs to another session")
            }
            Self::Malformed { line } => write!(f, "memory store line {line} is malformed"),
            Self::SequenceExhausted => f.write_str("memory store tick counter is exhausted"),
        }
    }
}

impl std::error::Error for MemoryError {}

/// Facts plus the per-session policy keys that may write them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryStore {
    /// `(session, token, grants)` registrations, in grant order.
    keys: Vec<(u64, u64, u16)>,
    /// Facts in write order.
    facts: Vec<Fact>,
    /// Writes so far; also the next fact id.
    tick: u64,
}

impl MemoryStore {
    /// An empty store with no registrations.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register (or replace) one session's key. Granting a session a new token
    /// revokes the old one: a key is the session's token, not a set.
    pub fn grant(&mut self, key: MemoryKey) {
        self.keys.retain(|(session, _, _)| *session != key.session);
        self.keys.push((key.session, key.token, key.grants));
    }

    /// Retained fact count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.facts.len()
    }

    /// True when no fact is retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    /// Writes so far (also the newest fact's tick).
    #[must_use]
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Check the key against the session's registration, in the order the
    /// refusals are named: missing, wrong token, then missing grant. Returns
    /// the session the key speaks for.
    fn check(&self, key: Option<&MemoryKey>) -> Result<u64, MemoryError> {
        let key = key.ok_or(MemoryError::NoKey)?;
        let Some((_, token, grants)) = self
            .keys
            .iter()
            .find(|(session, _, _)| *session == key.session)
        else {
            return Err(MemoryError::WrongKey);
        };
        if *token != key.token {
            return Err(MemoryError::WrongKey);
        }
        if grants & MEMORY_WRITE == 0 || key.grants & MEMORY_WRITE == 0 {
            return Err(MemoryError::NotGranted);
        }
        Ok(key.session)
    }

    /// Write one fact under `key`. Every check runs before anything changes,
    /// so a refused write leaves the store byte-identical.
    /// Errors: [`MemoryError::NoKey`], [`MemoryError::WrongKey`],
    /// [`MemoryError::NotGranted`], [`MemoryError::ValueTooLong`],
    /// [`MemoryError::BadConfidence`], [`MemoryError::SequenceExhausted`].
    pub fn write(&mut self, key: Option<&MemoryKey>, draft: Draft) -> Result<&Fact, MemoryError> {
        let session = self.check(key)?;
        if draft.value.len() > MAX_VALUE_LEN {
            return Err(MemoryError::ValueTooLong {
                len: draft.value.len(),
                cap: MAX_VALUE_LEN,
            });
        }
        if draft.confidence > MAX_CONFIDENCE {
            return Err(MemoryError::BadConfidence {
                value: draft.confidence,
            });
        }
        let tick = self
            .tick
            .checked_add(1)
            .ok_or(MemoryError::SequenceExhausted)?;
        let mut draft = draft;
        draft.source.session = session;
        self.facts.push(Fact {
            id: tick,
            kind: draft.kind,
            value: draft.value,
            confidence: draft.confidence,
            source: draft.source,
            tick,
            shared: draft.shared,
        });
        self.tick = tick;
        self.facts.last().ok_or(MemoryError::SequenceExhausted)
    }

    /// Mark one of `session`'s own facts shared, so every session recalls it.
    /// The writer is the only session that may share its fact, and the id must
    /// be one a fact carries; both refusals run before anything changes.
    /// Errors: [`MemoryError::NoFact`], [`MemoryError::NotYours`].
    pub fn share(&mut self, session: u64, id: u64) -> Result<&Fact, MemoryError> {
        let index = self
            .facts
            .iter()
            .position(|fact| fact.id == id)
            .ok_or(MemoryError::NoFact { id })?;
        let owner = self.facts.get(index).map(|fact| fact.source.session);
        if owner != Some(session) {
            return Err(MemoryError::NotYours { id });
        }
        if let Some(fact) = self.facts.get_mut(index) {
            fact.shared = true;
        }
        self.facts.get(index).ok_or(MemoryError::NoFact { id })
    }

    /// Facts `session` may recall: its own, plus the ones marked shared, in
    /// `(type, tick, id)` order. Nothing else leaks.
    #[must_use]
    pub fn recall(&self, session: u64) -> Vec<&Fact> {
        let mut rows: Vec<&Fact> = self
            .facts
            .iter()
            .filter(|fact| fact.shared || fact.source.session == session)
            .collect();
        rows.sort_by_key(|fact| (fact.kind.code(), fact.tick, fact.id));
        rows
    }

    /// Canonical store bytes, integer fields only plus the value as hex:
    /// `<kind>,<id>,<tick>,<confidence>,<session>,<turn>,<shared>,<value hex>`
    /// per line in write order. The policy keys are process-local and are not
    /// encoded; a decoded store must be granted again.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut out = String::new();
        for fact in &self.facts {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                fact.kind.code(),
                fact.id,
                fact.tick,
                fact.confidence,
                fact.source.session,
                fact.source.turn,
                u8::from(fact.shared),
                hex_encode(&fact.value),
            ));
        }
        out
    }

    /// Read [`Self::encode`] bytes back. A tampered or foreign line is refused
    /// as [`MemoryError::Malformed`] rather than guessed at.
    /// Errors: [`MemoryError::Malformed`].
    pub fn decode(text: &str) -> Result<Self, MemoryError> {
        let mut store = Self::new();
        for (index, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let malformed = MemoryError::Malformed {
                line: index.saturating_add(1),
            };
            let fields: Vec<&str> = line.split(',').collect();
            if fields.len() != 8 {
                return Err(malformed);
            }
            let code: u8 = fields[0].parse().map_err(|_| malformed)?;
            let kind = FactType::from_code(code).ok_or(malformed)?;
            let id: u64 = fields[1].parse().map_err(|_| malformed)?;
            let tick: u64 = fields[2].parse().map_err(|_| malformed)?;
            let confidence: u16 = fields[3].parse().map_err(|_| malformed)?;
            if confidence > MAX_CONFIDENCE {
                return Err(malformed);
            }
            let session: u64 = fields[4].parse().map_err(|_| malformed)?;
            let turn: u32 = fields[5].parse().map_err(|_| malformed)?;
            let shared: u8 = fields[6].parse().map_err(|_| malformed)?;
            if shared > 1 {
                return Err(malformed);
            }
            let value = hex_decode(fields[7]).ok_or(malformed)?;
            if value.len() > MAX_VALUE_LEN {
                return Err(MemoryError::ValueTooLong {
                    len: value.len(),
                    cap: MAX_VALUE_LEN,
                });
            }
            store.facts.push(Fact {
                id,
                kind,
                value,
                confidence,
                source: Source { session, turn },
                tick,
                shared: shared == 1,
            });
            store.tick = store.tick.max(tick);
        }
        Ok(store)
    }
}

/// Lowercase hex, two characters per byte.
fn hex_encode(value: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Inverse of [`hex_encode`]; `None` on an odd length or a non-hex character.
fn hex_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        out.push((high << 4) | low);
    }
    String::from_utf8(out).ok()
}

/// One hex digit value.
fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: FactType, value: &str, shared: bool) -> Draft {
        Draft {
            kind,
            value: value.to_owned(),
            confidence: 900,
            source: Source {
                session: 1,
                turn: 3,
            },
            shared,
        }
    }

    fn two_sessions() -> MemoryStore {
        let mut store = MemoryStore::new();
        store.grant(MemoryKey::writer(1, 0xa1));
        store.grant(MemoryKey::writer(2, 0xb2));
        store
    }

    #[test]
    fn share_marks_the_writers_own_fact_and_refuses_a_foreign_one_by_name() {
        let mut store = two_sessions();
        let first = MemoryKey::writer(1, 0xa1);
        let private = store
            .write(
                Some(&first),
                draft(FactType::Preference, "short matches", false),
            )
            .expect("a granted key writes");
        let id = private.id;
        assert!(store.recall(2).is_empty(), "a private fact never leaks");

        // A foreign session cannot share it, and the store does not change.
        let foreign = store
            .share(2, id)
            .expect_err("another session's fact");
        assert_eq!(foreign.code(), "memory_not_yours");
        assert!(store.recall(2).is_empty());

        // The writer shares it, and then the other session recalls it.
        let shared = store.share(1, id).expect("the writer shares its own fact");
        assert!(shared.shared);
        let rows = store.recall(2);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source.session, 1, "sharing keeps the source");

        // An id no fact carries refuses by name.
        let missing = store.share(1, 4_242).expect_err("no such fact");
        assert_eq!(missing.code(), "memory_no_fact");
    }

    #[test]
    fn one_session_writes_shared_and_the_other_recalls_it() {
        let mut store = two_sessions();
        let key = MemoryKey::writer(1, 0xa1);
        store
            .write(
                Some(&key),
                draft(FactType::Decision, "arena uses 2040 seed", true),
            )
            .expect("a granted key writes");

        let rows = store.recall(2);
        assert_eq!(rows.len(), 1, "the other session sees the shared fact");
        assert_eq!(rows[0].value, "arena uses 2040 seed");
        assert_eq!(rows[0].kind, FactType::Decision);
        assert_eq!(rows[0].source.session, 1, "the source stays the writer");
        assert_eq!(rows[0].tick, 1);
        assert_eq!(rows[0].id, 1);
    }

    #[test]
    fn a_private_fact_is_invisible_to_the_other_session() {
        let mut store = two_sessions();
        let key = MemoryKey::writer(1, 0xa1);
        store
            .write(
                Some(&key),
                draft(FactType::Fact, "the north wall is red", false),
            )
            .expect("the writer stores its own fact");
        assert_eq!(store.recall(1).len(), 1, "the writer recalls its own");
        assert!(store.recall(2).is_empty(), "a private fact never leaks");
    }

    #[test]
    fn an_unkeyed_write_is_refused_by_name_and_changes_nothing() {
        let mut store = two_sessions();
        let key = MemoryKey::writer(1, 0xa1);
        store
            .write(Some(&key), draft(FactType::Fact, "one fact", true))
            .expect("the first write lands");
        let before = store.encode();

        let refused = store
            .write(None, draft(FactType::Fact, "smuggled", true))
            .expect_err("no key is refused");
        assert_eq!(refused.code(), "memory_no_key");
        assert_eq!(store.encode(), before, "a refused write leaves no trace");

        let wrong = MemoryKey::writer(1, 0x99);
        assert_eq!(
            store
                .write(Some(&wrong), draft(FactType::Fact, "smuggled", true))
                .expect_err("a wrong token is refused")
                .code(),
            "memory_wrong_key"
        );
        let reader = MemoryKey::reader(1, 0xa1);
        assert_eq!(
            store
                .write(Some(&reader), draft(FactType::Fact, "smuggled", true))
                .expect_err("a key without the grant is refused")
                .code(),
            "memory_not_granted"
        );
        assert_eq!(store.encode(), before, "no refusal changes a byte");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn over_bound_values_and_confidence_are_refused_and_kept_out() {
        let mut store = two_sessions();
        let key = MemoryKey::writer(1, 0xa1);
        let long = "x".repeat(MAX_VALUE_LEN + 1);
        let refused = store
            .write(Some(&key), draft(FactType::Fact, &long, false))
            .expect_err("an over-long value is refused");
        assert_eq!(refused.code(), "memory_value_too_long");
        let mut high = draft(FactType::Fact, "high", false);
        high.confidence = MAX_CONFIDENCE + 1;
        let refused = store
            .write(Some(&key), high)
            .expect_err("confidence over the ceiling is refused");
        assert_eq!(refused.code(), "memory_bad_confidence");
        assert!(store.is_empty(), "nothing was kept");
    }

    #[test]
    fn the_same_writes_give_the_same_bytes_and_the_round_trip_is_exact() {
        let writes = [
            (FactType::Decision, "seed 2040", true),
            (FactType::Fact, "wall north", false),
            (FactType::Preference, "短 names", true),
        ];
        let mut first = two_sessions();
        let mut second = two_sessions();
        let key = MemoryKey::writer(1, 0xa1);
        for (kind, value, shared) in writes {
            first
                .write(Some(&key), draft(kind, value, shared))
                .expect("first store writes");
            second
                .write(Some(&key), draft(kind, value, shared))
                .expect("second store writes");
        }
        let bytes = first.encode();
        assert_eq!(bytes, second.encode(), "same writes, same bytes");
        let decoded = MemoryStore::decode(&bytes).expect("the bytes decode");
        assert_eq!(decoded.facts, first.facts, "facts round-trip exactly");
        assert_eq!(decoded.encode(), bytes, "and re-encode to the same bytes");
        assert!(decoded.keys.is_empty(), "policy keys are process-local");
    }

    #[test]
    fn recall_order_is_type_then_tick_then_id_never_time() {
        let mut store = two_sessions();
        let key = MemoryKey::writer(1, 0xa1);
        store
            .write(Some(&key), draft(FactType::Decision, "d1", false))
            .expect("write");
        store
            .write(Some(&key), draft(FactType::Fact, "f1", false))
            .expect("write");
        store
            .write(Some(&key), draft(FactType::Fact, "f2", false))
            .expect("write");
        store
            .write(Some(&key), draft(FactType::Preference, "p1", false))
            .expect("write");
        let rows = store.recall(1);
        let order: Vec<(&'static str, u64)> = rows
            .iter()
            .map(|fact| (fact.kind.name(), fact.id))
            .collect();
        assert_eq!(
            order,
            vec![("fact", 2), ("fact", 3), ("preference", 4), ("decision", 1)],
            "type code first, then write order"
        );
    }

    #[test]
    fn a_tampered_store_line_is_refused_not_guessed() {
        let mut store = two_sessions();
        let key = MemoryKey::writer(1, 0xa1);
        store
            .write(Some(&key), draft(FactType::Fact, "one", false))
            .expect("write");
        let bytes = store.encode();
        let tampered = format!("9{}", &bytes[1..]);
        let refused = MemoryStore::decode(&tampered).expect_err("a bad code is refused");
        assert_eq!(refused.code(), "memory_malformed");
        let short = bytes
            .trim_end()
            .split(',')
            .take(7)
            .collect::<Vec<&str>>()
            .join(",");
        assert!(
            MemoryStore::decode(&short).is_err(),
            "a short line is refused"
        );
    }
}

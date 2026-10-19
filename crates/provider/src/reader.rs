//! reader: the model-reader adapter behind the door (plan eyes E3).
//!
//! One way, no loops. In: a recorded capture (its [`ContextRef`], so no image
//! bytes cross into this crate) plus the brief text plus a provider label. Out:
//! a scripted read — entity ids, relations, counts — plus a receipt that
//! carries the label. The recording is the oracle: the answer is a pure
//! function of the capture and the brief, so the label never changes one byte
//! of it. A capture nobody recorded answers [`ReaderError::Unrecorded`] under
//! every label, which keeps that path open instead of inventing a read.
//!
//! Nothing here holds weights, a clock or a socket. The PNG bytes stay with the
//! capture owner (`capture_with` and its manifest); local and remote inference
//! live outside this repo, so their recordings must match the mock byte for
//! byte or the path stays open. A relation's `kind` is the graph's own name
//! text (`engine_spatial::relation::Relation::name`), so this crate never
//! declares a second relation vocabulary. Units: ids and counts are integers;
//! the brief and the read are keyed by FNV digests, keys and never security
//! hashes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use engine_core::hash_bytes;

use crate::protocol::ContextRef;

/// The label on a read: which path the host ran. A receipt field, never an
/// input to the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReaderLabel {
    /// The recorded path: the oracle, no model at all.
    Mock,
    /// A model on this machine.
    Local,
    /// A model reached over the network.
    Remote,
}

impl ReaderLabel {
    /// Every label, mock first: the order a parity sweep walks.
    pub const ALL: [Self; 3] = [Self::Mock, Self::Local, Self::Remote];

    /// The label as a receipt and the door spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

/// One relation a read named, in the graph's own words.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReaderRelation {
    /// Subject entity id.
    pub subject: u32,
    /// Object entity id.
    pub object: u32,
    /// The graph's relation name, e.g. `on_top_of`.
    pub kind: String,
}

/// A scripted read: the entity ids, relations and counts a reader answered.
///
/// Canonical by construction, so two spellings of one answer are one read and
/// every label compares byte for byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptedRead {
    entities: Vec<u32>,
    relations: Vec<ReaderRelation>,
    counts: Vec<(String, u32)>,
}

impl ScriptedRead {
    /// A read from a model's answer, canonicalised: ids ascending and deduped,
    /// relations and counts sorted and deduped.
    #[must_use]
    pub fn new(
        mut entities: Vec<u32>,
        mut relations: Vec<ReaderRelation>,
        mut counts: Vec<(String, u32)>,
    ) -> Self {
        entities.sort_unstable();
        entities.dedup();
        relations.sort();
        relations.dedup();
        counts.sort();
        counts.dedup();
        Self {
            entities,
            relations,
            counts,
        }
    }

    /// The entity ids the read named, ascending.
    #[must_use]
    pub fn entities(&self) -> &[u32] {
        &self.entities
    }

    /// The relations the read named, sorted.
    #[must_use]
    pub fn relations(&self) -> &[ReaderRelation] {
        &self.relations
    }

    /// The counts the read stated, sorted by label.
    #[must_use]
    pub fn counts(&self) -> &[(String, u32)] {
        &self.counts
    }

    /// True when the read named nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.relations.is_empty() && self.counts.is_empty()
    }

    /// The canonical bytes every label must agree on, and the preimage of
    /// [`Self::digest`]. One line, stable field order.
    #[must_use]
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        for id in &self.entities {
            let _ = write!(out, "entity:{id}|");
        }
        for rel in &self.relations {
            let _ = write!(out, "relation:{}<{}:{}|", rel.subject, rel.kind, rel.object);
        }
        for (label, count) in &self.counts {
            let _ = write!(out, "count:{label}={count}|");
        }
        out.into_bytes()
    }

    /// The read's digest, the `read` field of a receipt.
    #[must_use]
    pub fn digest(&self) -> u64 {
        hash_bytes(&self.bytes())
    }
}

/// What one read was asked with and answered: off the tick, on the receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReaderReceipt {
    /// The path the host ran.
    pub label: ReaderLabel,
    /// The capture digest the read was looked up under.
    pub capture: [u8; 32],
    /// The brief key.
    pub brief: u64,
    /// The read's canonical digest.
    pub read: u64,
}

impl ReaderReceipt {
    /// The one line a host logs or a door returns.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "reader {} capture={} brief={:016x} read={:016x}",
            self.label.name(),
            hex32(&self.capture),
            self.brief,
            self.read
        )
    }
}

/// One read plus the receipt that labels the path it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderAnswer {
    /// The read itself: the same bytes under every label.
    pub read: ScriptedRead,
    /// Which label asked, and the keys it asked with.
    pub receipt: ReaderReceipt,
}

/// Why a read came back empty-handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderError {
    /// That capture plus brief was never recorded: the path stays open.
    Unrecorded {
        /// The capture digest asked for.
        capture: [u8; 32],
        /// The brief key asked for.
        brief: u64,
    },
    /// The oracle already holds a different read for that pair: a recording
    /// cannot drift silently.
    AlreadyRecorded {
        /// The capture digest already recorded.
        capture: [u8; 32],
        /// The brief key already recorded.
        brief: u64,
    },
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unrecorded { capture, brief } => write!(
                f,
                "no reader recording for capture {} brief {brief:016x}: the path stays open",
                hex32(capture)
            ),
            Self::AlreadyRecorded { capture, brief } => write!(
                f,
                "a different read is already recorded for capture {} brief {brief:016x}",
                hex32(capture)
            ),
        }
    }
}

impl std::error::Error for ReaderError {}

/// The reader adapter: one recording table, three labels answering it.
#[derive(Clone, Debug, Default)]
pub struct ReaderAdapter {
    recordings: BTreeMap<([u8; 32], u64), ScriptedRead>,
}

impl ReaderAdapter {
    /// An adapter with an empty recording table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The key a brief is recorded and looked up under.
    #[must_use]
    pub fn brief_key(brief: &str) -> u64 {
        hash_bytes(brief.as_bytes())
    }

    /// Records the oracle answer for one capture plus brief: what the mock path
    /// answers and what local and remote must match. Re-recording the same read
    /// is a no-op; a different read for the same pair is refused, so a fixture
    /// cannot drift. Returns the brief key.
    pub fn record(
        &mut self,
        capture: ContextRef,
        brief: &str,
        read: ScriptedRead,
    ) -> Result<u64, ReaderError> {
        let key = (capture.digest, Self::brief_key(brief));
        match self.recordings.get(&key) {
            Some(held) if *held != read => {
                return Err(ReaderError::AlreadyRecorded {
                    capture: key.0,
                    brief: key.1,
                });
            }
            Some(_) => return Ok(key.1),
            None => {}
        }
        self.recordings.insert(key, read);
        Ok(key.1)
    }

    /// The read for a capture plus brief under any label. The label rides the
    /// receipt and never the answer; an unrecorded pair stays open.
    pub fn read(
        &self,
        capture: ContextRef,
        brief: &str,
        label: ReaderLabel,
    ) -> Result<ReaderAnswer, ReaderError> {
        let brief = Self::brief_key(brief);
        let read =
            self.recordings
                .get(&(capture.digest, brief))
                .ok_or(ReaderError::Unrecorded {
                    capture: capture.digest,
                    brief,
                })?;
        Ok(ReaderAnswer {
            read: read.clone(),
            receipt: ReaderReceipt {
                label,
                capture: capture.digest,
                brief,
                read: read.digest(),
            },
        })
    }

    /// Offline replay: the recorded path, no model, no socket, byte-identical
    /// to [`Self::read`] with any other label.
    pub fn replay(&self, capture: ContextRef, brief: &str) -> Result<ReaderAnswer, ReaderError> {
        self.read(capture, brief, ReaderLabel::Mock)
    }

    /// How many capture plus brief pairs are recorded.
    #[must_use]
    pub fn recorded(&self) -> usize {
        self.recordings.len()
    }

    /// The recorded captures with how many briefs each answers, ascending.
    #[must_use]
    pub fn captures(&self) -> Vec<([u8; 32], usize)> {
        let mut out: Vec<([u8; 32], usize)> = Vec::new();
        for key in self.recordings.keys() {
            match out.last_mut() {
                Some(last) if last.0 == key.0 => last.1 += 1,
                _ => out.push((key.0, 1)),
            }
        }
        out
    }
}

/// A digest as the receipt spells it: 16 hex pairs, lower case.
fn hex32(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::DataClass;

    fn capture(seed: u8) -> ContextRef {
        ContextRef {
            class: DataClass::Project,
            digest: [seed; 32],
            revision: u64::from(seed),
        }
    }

    /// A seeded answer: ids, two relations in the graph's words, two counts.
    fn seeded(seed: u64) -> ScriptedRead {
        let mut rng = seed;
        let mut entities = Vec::new();
        let mut counts = Vec::new();
        for _ in 0..3 {
            rng = engine_core::splitmix64(rng);
            entities.push(u32::try_from(rng % 64).unwrap_or(0));
        }
        rng = engine_core::splitmix64(rng);
        counts.push(("hidden".to_owned(), u32::try_from(rng % 9).unwrap_or(0)));
        rng = engine_core::splitmix64(rng);
        counts.push(("visible".to_owned(), u32::try_from(rng % 9).unwrap_or(0)));
        ScriptedRead::new(
            entities,
            vec![
                ReaderRelation {
                    subject: 3,
                    object: 9,
                    kind: "on_top_of".to_owned(),
                },
                ReaderRelation {
                    subject: 9,
                    object: 11,
                    kind: "next_to".to_owned(),
                },
            ],
            counts,
        )
    }

    fn recorded() -> ReaderAdapter {
        let mut adapter = ReaderAdapter::new();
        for seed in 1..=3u8 {
            adapter
                .record(
                    capture(seed),
                    "which props are near the gate?",
                    seeded(u64::from(seed)),
                )
                .expect("a fresh pair records");
        }
        adapter
    }

    #[test]
    fn three_path_labels_agree_byte_for_byte() {
        let adapter = recorded();
        assert_eq!(adapter.recorded(), 3);
        assert_eq!(adapter.captures().len(), 3);
        for seed in 1..=3u8 {
            let mut agree: Option<Vec<u8>> = None;
            for label in ReaderLabel::ALL {
                let answer = adapter
                    .read(capture(seed), "which props are near the gate?", label)
                    .expect("recorded");
                assert_eq!(answer.receipt.label, label, "the receipt names the path");
                assert_eq!(answer.receipt.capture, capture(seed).digest);
                assert_eq!(answer.receipt.read, answer.read.digest());
                match &agree {
                    Some(bytes) => assert_eq!(
                        &answer.read.bytes(),
                        bytes,
                        "{} differs from the first path",
                        label.name()
                    ),
                    None => agree = Some(answer.read.bytes()),
                }
                assert!(answer
                    .receipt
                    .line()
                    .starts_with(&format!("reader {}", label.name())));
            }
            assert!(agree.is_some_and(|bytes| !bytes.is_empty()));
        }
    }

    #[test]
    fn replay_with_no_model_is_byte_identical() {
        let adapter = recorded();
        for seed in 1..=3u8 {
            let a = adapter
                .replay(capture(seed), "which props are near the gate?")
                .expect("recorded");
            let b = adapter
                .replay(capture(seed), "which props are near the gate?")
                .expect("recorded");
            assert_eq!(a.read.bytes(), b.read.bytes(), "replay is a pure function");
            assert_eq!(a.receipt.read, b.receipt.read);
            assert_eq!(
                a.receipt.label,
                ReaderLabel::Mock,
                "replay is the mock path"
            );
            assert_eq!(a.read, seeded(u64::from(seed)), "the oracle's own answer");
        }
    }

    #[test]
    fn an_unrecorded_capture_keeps_every_path_open() {
        let adapter = recorded();
        let fresh = capture(99);
        for label in ReaderLabel::ALL {
            let err = adapter
                .read(fresh, "which props are near the gate?", label)
                .expect_err("never recorded");
            assert_eq!(
                err,
                ReaderError::Unrecorded {
                    capture: fresh.digest,
                    brief: ReaderAdapter::brief_key("which props are near the gate?"),
                }
            );
            assert!(err.to_string().contains("stays open"));
        }
        // The same frame under a brief nobody recorded stays open too.
        assert!(adapter
            .read(capture(1), "a different brief", ReaderLabel::Mock)
            .is_err());
    }

    #[test]
    fn the_oracle_refuses_a_second_different_read() {
        let mut adapter = ReaderAdapter::new();
        let brief = "count the fighters";
        adapter.record(capture(1), brief, seeded(7)).expect("fresh");
        // The same read again is a no-op, so a fixture may be built twice.
        adapter
            .record(capture(1), brief, seeded(7))
            .expect("same read");
        assert_eq!(adapter.recorded(), 1);
        // A different read for the same pair is refused, never silently swapped.
        let err = adapter
            .record(capture(1), brief, seeded(8))
            .expect_err("a drift is refused");
        assert_eq!(
            err,
            ReaderError::AlreadyRecorded {
                capture: capture(1).digest,
                brief: ReaderAdapter::brief_key(brief),
            }
        );
        assert_eq!(adapter.recorded(), 1, "the held read is untouched");
    }

    #[test]
    fn the_read_is_canonical_and_the_adapter_holds_no_clock_or_socket() {
        // The same answer spelled in another order is one read.
        let sorted = ScriptedRead::new(
            vec![4, 7, 9],
            vec![
                ReaderRelation {
                    subject: 3,
                    object: 9,
                    kind: "on_top_of".to_owned(),
                },
                ReaderRelation {
                    subject: 9,
                    object: 11,
                    kind: "next_to".to_owned(),
                },
            ],
            vec![("hidden".to_owned(), 2), ("visible".to_owned(), 5)],
        );
        assert_eq!(sorted.entities().len(), 3, "the fixture keeps three ids");
        let shuffled = ScriptedRead::new(
            vec![9, 4, 7],
            vec![sorted.relations()[1].clone(), sorted.relations()[0].clone()],
            vec![sorted.counts()[1].clone(), sorted.counts()[0].clone()],
        );
        assert_eq!(shuffled, sorted, "order is canonicalised away");
        assert_eq!(shuffled.bytes(), sorted.bytes());
        assert_eq!(shuffled.digest(), sorted.digest());
        assert!(ScriptedRead::default().is_empty());
        assert!(!sorted.is_empty());
        // An empty read is still a recorded answer, not a missing one.
        let mut adapter = ReaderAdapter::new();
        adapter
            .record(capture(4), "nothing there", ScriptedRead::default())
            .expect("fresh");
        let answer = adapter
            .replay(capture(4), "nothing there")
            .expect("recorded");
        assert!(answer.read.is_empty());
        // The law: this module never reaches for a clock, an env or a socket.
        let src = include_str!("reader.rs");
        let code = src
            .split("#[cfg(test)]")
            .next()
            .expect("the module has code before its tests");
        for forbidden in [
            "std::time",
            "Instant::now",
            "SystemTime",
            "std::env",
            "TcpStream",
            "reqwest",
            "ureq",
        ] {
            assert!(!code.contains(forbidden), "reader code uses {forbidden}");
        }
    }
}

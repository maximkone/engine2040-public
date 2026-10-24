//! ack: packet delivery tracking with a 32-bit ack bitfield and a loss rate.
//!
//! Purpose: tell a sender which of its packets arrived, and what share was
//! lost, from sequence numbers alone. The receiver half (`RecvWindow`)
//! folds arrivals into the pair a packet header would carry: the newest
//! sequence received plus a 32-bit field where bit `i` means sequence
//! `latest - 1 - i` arrived. The sender half ([`SentLedger`](crate::ack::SentLedger)) keeps a fixed
//! ring of sent records and resolves each one to delivered or lost when an
//! [`Ack`](crate::ack::Ack) names it. The shape follows naia's ack manager; the code is
//! integer-only and std-only.
//!
//! Rules of the ledger: a record resolves once, by the first ack that
//! vouches for it (a repeated, reordered or older ack changes nothing), or
//! as lost when [`WINDOW`](crate::ack::WINDOW) newer packets have been sent past it. A clear bit
//! is final: a packet that lands after an ack named it missing still counts
//! as lost, the cost every header scheme pays under reordering.
//!
//! Owner: net lane. Units: wrapping `u16` sequence numbers, loss in permille
//! (1/1000, rounded to nearest). Determinism: fixed arrays, no map, no clock;
//! the same calls give the same state. The result feeds receipts and resend
//! decisions only, never simulation state or a hash.
//! Errors: [`AckError`](crate::ack::AckError); a refused call changes nothing.
//! Consumer: [`input::InputDelivery`](crate::ack::input::InputDelivery), fed by the played bout's seats.

use std::fmt;

pub mod input;
mod recv;

pub use recv::{Arrival, RecvWindow};

#[cfg(test)]
mod bout_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod tests;

/// Wrapping packet sequence number: the width a header would carry.
pub type Seq = u16;

/// Sent records kept, and resolved outcomes the loss rate averages over.
pub const WINDOW: usize = 64;
/// Sequences behind the latest that a header's 32-bit field names.
pub const HEADER_BITS: u8 = 32;
/// Sequences an [`Ack::from_header`] vouches for: the latest plus the field.
pub const HEADER_SPAN: u8 = HEADER_BITS + 1;
/// Newest sequences the receiver's loss estimate leaves out: they may still
/// be in flight, and a late packet must not read as a lost one.
pub const RECV_GRACE: u32 = 3;

/// [`WINDOW`] as the width of the history words.
const WINDOW_BITS: u32 = 64;
/// [`WINDOW`] as a span.
const WINDOW_SPAN: u8 = 64;
/// Half the sequence space: a sequence is newer when it is less than this
/// far ahead.
const HALF: Seq = 0x8000;

/// True when `a` is newer than `b` in wrapping order: it is ahead by less
/// than half the sequence space.
#[must_use]
pub const fn is_newer(a: Seq, b: Seq) -> bool {
    a != b && a.wrapping_sub(b) < HALF
}

/// The sequence a tick maps to: its low 16 bits, so a tick stream wraps the
/// way a header counter does.
#[must_use]
pub fn seq_of_tick(tick: u32) -> Seq {
    let [low, high, ..] = tick.to_le_bytes();
    Seq::from_le_bytes([low, high])
}

fn slot(seq: Seq) -> usize {
    usize::from(seq) % WINDOW
}

/// `part` of `whole` in permille, rounded to nearest; zero for an empty whole.
fn permille(part: u32, whole: u32) -> u32 {
    if whole == 0 {
        return 0;
    }
    let scaled = (u64::from(part) * 1_000 + u64::from(whole) / 2) / u64::from(whole);
    u32::try_from(scaled).map_or(1_000, |share| share.min(1_000))
}

/// Mask of the low `n` bits of a history word.
fn low_bits(n: u32) -> u64 {
    if n >= WINDOW_BITS {
        u64::MAX
    } else {
        (1u64 << n) - 1
    }
}

/// Why a tracker call was refused. A refused call changes nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AckError {
    /// A send named a sequence that is not newer than the last one sent.
    NotNewer {
        /// The refused sequence.
        seq: Seq,
        /// The newest sequence sent so far.
        newest: Seq,
    },
    /// An ack named a sequence newer than anything sent: a corrupt or
    /// hostile header.
    FromTheFuture {
        /// The ack's newest sequence.
        head: Seq,
        /// The newest sequence sent so far.
        newest: Seq,
    },
    /// An ack arrived before any packet was sent.
    NothingSent,
}

impl fmt::Display for AckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotNewer { seq, newest } => {
                write!(
                    f,
                    "sequence {seq} is not newer than the last sent, {newest}"
                )
            }
            Self::FromTheFuture { head, newest } => {
                write!(f, "ack names sequence {head}, newest sent is {newest}")
            }
            Self::NothingSent => f.write_str("ack arrived before any packet was sent"),
        }
    }
}

impl std::error::Error for AckError {}

/// What a peer vouches for: bit `i` of `bits` says sequence `head - i`
/// arrived, for the `span` newest positions. Older sequences are unknown,
/// not lost.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Ack {
    head: Seq,
    bits: u64,
    span: u8,
}

impl Ack {
    /// The ack a packet header carries: `latest` arrived, and bit `i` of
    /// `ack_bits` says sequence `latest - 1 - i` did. Vouches for
    /// [`HEADER_SPAN`] sequences.
    #[must_use]
    pub fn from_header(latest: Seq, ack_bits: u32) -> Self {
        Self {
            head: latest,
            bits: 1 | (u64::from(ack_bits) << 1),
            span: HEADER_SPAN,
        }
    }

    /// An ack built from knowledge a peer already holds: `arrived` answers
    /// for each of the `span` sequences from `head` back (clamped to
    /// `1..=`[`WINDOW`]). The head itself need not have arrived.
    #[must_use]
    pub fn from_membership(head: Seq, span: u8, arrived: impl Fn(Seq) -> bool) -> Self {
        let span = span.clamp(1, WINDOW_SPAN);
        let mut bits = 0u64;
        for back in 0..span {
            if arrived(head.wrapping_sub(Seq::from(back))) {
                bits |= 1 << back;
            }
        }
        Self { head, bits, span }
    }

    /// The newest sequence this ack speaks for.
    #[must_use]
    pub const fn head(&self) -> Seq {
        self.head
    }

    /// How many sequences, counting the head, this ack vouches for.
    #[must_use]
    pub const fn span(&self) -> u8 {
        self.span
    }

    /// Whether `seq` arrived; `None` when the ack does not speak for it
    /// (newer than the head, or older than the span).
    #[must_use]
    pub fn arrived(&self, seq: Seq) -> Option<bool> {
        let back = self.head.wrapping_sub(seq);
        (back < Seq::from(self.span)).then(|| (self.bits >> back) & 1 == 1)
    }
}

/// Outcome counts of one [`SentLedger::on_ack`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Resolved {
    /// Records the ack settled as delivered.
    pub delivered: u32,
    /// Records the ack settled as lost.
    pub lost: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fate {
    Pending,
    Delivered,
    Lost,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Record {
    seq: Seq,
    fate: Fate,
}

/// Sender half: a fixed ring of the last [`WINDOW`] sent records, resolved
/// by [`Ack`]s, and the loss rate over the newest resolved ones.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SentLedger {
    ring: [Option<Record>; WINDOW],
    newest: Option<Seq>,
    /// The newest resolutions, newest at bit 0; a set bit is a lost packet.
    recent: u64,
    /// Valid bits of `recent`, at most [`WINDOW`].
    resolved: u32,
    delivered: u32,
    lost: u32,
}

impl Default for SentLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl SentLedger {
    /// An empty ledger.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            ring: [None; WINDOW],
            newest: None,
            recent: 0,
            resolved: 0,
            delivered: 0,
            lost: 0,
        }
    }

    /// Record that `seq` went out. A record still pending when the ring
    /// reaches its slot again ([`WINDOW`] packets later) ages out as lost.
    ///
    /// # Errors
    ///
    /// [`AckError::NotNewer`] unless `seq` is newer than the last one sent.
    pub fn on_send(&mut self, seq: Seq) -> Result<(), AckError> {
        if let Some(newest) = self.newest {
            if !is_newer(seq, newest) {
                return Err(AckError::NotNewer { seq, newest });
            }
            let moved = usize::from(seq.wrapping_sub(newest)).min(WINDOW);
            let mut passed = newest;
            for _ in 0..moved {
                passed = passed.wrapping_add(1);
                self.expire(passed);
            }
        }
        self.ring[slot(seq)] = Some(Record {
            seq,
            fate: Fate::Pending,
        });
        self.newest = Some(seq);
        Ok(())
    }

    /// Resolve every pending record `ack` vouches for: delivered when its
    /// bit is set, lost when it is clear. Records the ack does not speak
    /// for stay pending, and settled ones never change, so a repeated,
    /// reordered or older ack resolves nothing new.
    ///
    /// # Errors
    ///
    /// [`AckError::NothingSent`] before the first send;
    /// [`AckError::FromTheFuture`] when the ack names a sequence newer than
    /// the newest sent.
    pub fn on_ack(&mut self, ack: &Ack) -> Result<Resolved, AckError> {
        let newest = self.newest.ok_or(AckError::NothingSent)?;
        let head = ack.head();
        if is_newer(head, newest) {
            return Err(AckError::FromTheFuture { head, newest });
        }
        let mut out = Resolved::default();
        // Oldest first, so the loss window stays in sequence order.
        for back in (0..ack.span()).rev() {
            let seq = head.wrapping_sub(Seq::from(back));
            let Some(arrived) = ack.arrived(seq) else {
                continue;
            };
            let Some(record) = self.ring[slot(seq)].as_mut() else {
                continue;
            };
            if record.seq != seq || record.fate != Fate::Pending {
                continue;
            }
            record.fate = if arrived { Fate::Delivered } else { Fate::Lost };
            self.settle(arrived);
            if arrived {
                out.delivered += 1;
            } else {
                out.lost += 1;
            }
        }
        Ok(out)
    }

    /// Loss over the newest [`WINDOW`] resolved packets, in permille; zero
    /// before anything resolves.
    #[must_use]
    pub fn loss_permille(&self) -> u32 {
        let lost = (self.recent & low_bits(self.resolved)).count_ones();
        permille(lost, self.resolved)
    }

    /// Packets resolved as delivered since the start.
    #[must_use]
    pub const fn delivered(&self) -> u32 {
        self.delivered
    }

    /// Packets resolved as lost since the start.
    #[must_use]
    pub const fn lost(&self) -> u32 {
        self.lost
    }

    /// Sent records no ack has settled yet, at most [`WINDOW`]. A test
    /// probe: receipts read the settled counts, which this tail never joins.
    #[cfg(test)]
    #[must_use]
    pub fn pending(&self) -> usize {
        self.ring
            .iter()
            .flatten()
            .filter(|record| record.fate == Fate::Pending)
            .count()
    }

    /// Clear the slot of `seq`; a record still pending there is lost.
    fn expire(&mut self, seq: Seq) {
        if let Some(record) = self.ring[slot(seq)].take() {
            if record.fate == Fate::Pending {
                self.settle(false);
            }
        }
    }

    /// Fold one resolution into the loss window and the totals.
    fn settle(&mut self, arrived: bool) {
        self.recent = (self.recent << 1) | u64::from(!arrived);
        self.resolved = (self.resolved + 1).min(WINDOW_BITS);
        if arrived {
            self.delivered = self.delivered.saturating_add(1);
        } else {
            self.lost = self.lost.saturating_add(1);
        }
    }
}

//! Delivery tracking for one client's input stream, built from the bytes the
//! session already exchanges.
//!
//! The session wire (`WIRE_VERSION` 1) has no field for ack bits, and a
//! datagram of another version is refused, so no header changes here. What
//! the exchange does carry: the client sends one input packet per tick and
//! the server answers every authored tick with a state datagram whose window
//! lists the authoritative inputs of the last `cover` ticks. A tick of the
//! client's own fighter on that window reached the tape in time; a tick that
//! is missing was lost, late or dropped with the seat, because the server
//! never invents a human input. [`InputDelivery`] reads that as an [`Ack`] on
//! the sent input ticks, so the tracked unit is the input tick and "delivered"
//! means "counted by the server".
//!
//! A later wire change adds `seq: u16`, `ack: u16` and `ack_bits: u32` (8
//! bytes) to every packet header under a new `WIRE_VERSION`. Each sender then
//! calls [`SentLedger::on_send`] per packet and [`SentLedger::on_ack`] with
//! [`Ack::from_header`] per received header; each receiver feeds
//! [`RecvWindow::on_receive`] and writes [`RecvWindow::header`] into its next
//! packet. That tracks packets in both directions and resolves within one
//! round trip instead of waiting for a state window.

use super::{seq_of_tick, Ack, AckError, Arrival, RecvWindow, SentLedger};
use crate::session::{BodyId, BodyInput};

/// What the tracker measured, in plain counters for a receipt.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DeliveryStats {
    /// Input ticks the authoritative tape holds.
    pub input_delivered: u32,
    /// Input ticks that never reached the tape in time.
    pub input_lost: u32,
    /// Loss over the newest 64 resolved input ticks, in permille.
    pub input_loss_permille: u32,
    /// Loss estimate over the newest live state datagrams, in permille.
    pub state_loss_permille: u32,
    /// Newest live state sequence received (the header's `ack`); zero before
    /// the first one.
    pub state_latest: u32,
    /// The header's 32-bit field for it: bit `i` means state sequence
    /// `state_latest - 1 - i` arrived.
    pub state_bits: u32,
}

impl DeliveryStats {
    /// Counters in [`Self::to_words`] order.
    pub const WORDS: usize = 6;

    /// Nothing measured yet.
    pub const EMPTY: Self = Self::from_words([0; Self::WORDS]);

    /// The counters as words, for a cross-machine report.
    #[must_use]
    pub const fn to_words(self) -> [u32; Self::WORDS] {
        [
            self.input_delivered,
            self.input_lost,
            self.input_loss_permille,
            self.state_loss_permille,
            self.state_latest,
            self.state_bits,
        ]
    }

    /// The inverse of [`Self::to_words`].
    #[must_use]
    pub const fn from_words(words: [u32; Self::WORDS]) -> Self {
        let [input_delivered, input_lost, input_loss_permille, state_loss_permille, state_latest, state_bits] =
            words;
        Self {
            input_delivered,
            input_lost,
            input_loss_permille,
            state_loss_permille,
            state_latest,
            state_bits,
        }
    }
}

/// One client's delivery tracker: a [`SentLedger`] over its input ticks and a
/// [`RecvWindow`] over the live state datagrams it receives.
#[derive(Clone, Debug)]
pub struct InputDelivery {
    local: BodyId,
    sent: SentLedger,
    state: RecvWindow,
}

impl InputDelivery {
    /// A tracker for the client that plays fighter `local`.
    #[must_use]
    pub fn new(local: BodyId) -> Self {
        Self {
            local,
            sent: SentLedger::new(),
            state: RecvWindow::default(),
        }
    }

    /// The client sent its input for `tick`. Ticks rise by one per packet.
    ///
    /// # Errors
    ///
    /// [`AckError::NotNewer`] when `tick` does not rise.
    pub fn on_input_sent(&mut self, tick: u32) -> Result<(), AckError> {
        self.sent.on_send(seq_of_tick(tick))
    }

    /// A live state datagram arrived for `checkpoint_tick`, carrying the
    /// authoritative inputs `authoritative` of the last `cover` ticks. Only
    /// a first arrival resolves input ticks; a duplicate or a packet far
    /// behind the window changes nothing.
    ///
    /// # Errors
    ///
    /// [`AckError::FromTheFuture`] when the packet names a tick newer than
    /// any input sent (a hostile or corrupt checkpoint).
    pub fn on_state(
        &mut self,
        checkpoint_tick: u32,
        authoritative: &[BodyInput],
        cover: usize,
    ) -> Result<Arrival, AckError> {
        let head = seq_of_tick(checkpoint_tick);
        let arrival = self.state.on_receive(head);
        if arrival != Arrival::Fresh {
            return Ok(arrival);
        }
        let local = self.local;
        let span = u8::try_from(cover).unwrap_or(u8::MAX);
        let ack = Ack::from_membership(head, span, |seq| {
            authoritative
                .iter()
                .any(|input| input.fighter == local && seq_of_tick(input.tick) == seq)
        });
        self.sent.on_ack(&ack)?;
        Ok(arrival)
    }

    /// The counters so far.
    #[must_use]
    pub fn stats(&self) -> DeliveryStats {
        DeliveryStats {
            input_delivered: self.sent.delivered(),
            input_lost: self.sent.lost(),
            input_loss_permille: self.sent.loss_permille(),
            state_loss_permille: self.state.loss_permille(),
            state_latest: self.state.latest().map_or(0, u32::from),
            state_bits: self.state.header().map_or(0, |(_, bits)| bits),
        }
    }
}

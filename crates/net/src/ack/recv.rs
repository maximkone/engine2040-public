//! Receiver half of the ack tracker: which sequences arrived, as the header a
//! sender needs and as a loss estimate of the inbound stream.

use super::{is_newer, low_bits, permille, Seq, RECV_GRACE, WINDOW_BITS};

/// How [`RecvWindow::on_receive`] took one arrival.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arrival {
    /// First time this sequence arrived.
    Fresh,
    /// Already seen: changes nothing.
    Duplicate,
    /// [`super::WINDOW`] or more behind the newest: too old to track.
    Stale,
}

/// Receiver half: the last [`super::WINDOW`] sequences seen, as the pair a packet
/// header carries ([`Self::header`]) and a loss estimate of the inbound
/// stream ([`Self::loss_permille`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RecvWindow {
    latest: Option<Seq>,
    /// Bit `i` set: sequence `latest - i` arrived.
    seen: u64,
    /// Positions of the window that count, at most [`super::WINDOW`]: it grows from
    /// the first arrival, so sequences before the stream began are not lost.
    depth: u32,
}

impl RecvWindow {
    /// Take one arrival.
    pub fn on_receive(&mut self, seq: Seq) -> Arrival {
        let Some(latest) = self.latest else {
            *self = Self {
                latest: Some(seq),
                seen: 1,
                depth: 1,
            };
            return Arrival::Fresh;
        };
        if seq == latest {
            return Arrival::Duplicate;
        }
        if is_newer(seq, latest) {
            let ahead = u32::from(seq.wrapping_sub(latest));
            let older = if ahead >= WINDOW_BITS {
                0
            } else {
                self.seen << ahead
            };
            self.seen = older | 1;
            self.depth = (self.depth + ahead).min(WINDOW_BITS);
            self.latest = Some(seq);
            return Arrival::Fresh;
        }
        let back = u32::from(latest.wrapping_sub(seq));
        if back >= WINDOW_BITS {
            return Arrival::Stale;
        }
        let bit = 1u64 << back;
        if self.seen & bit != 0 {
            return Arrival::Duplicate;
        }
        self.seen |= bit;
        self.depth = self.depth.max(back + 1);
        Arrival::Fresh
    }

    /// The newest sequence seen.
    #[must_use]
    pub const fn latest(&self) -> Option<Seq> {
        self.latest
    }

    /// The pair a packet header carries: the newest sequence and a 32-bit
    /// field where bit `i` means sequence `latest - 1 - i` arrived. `None`
    /// before the first arrival.
    #[must_use]
    pub fn header(&self) -> Option<(Seq, u32)> {
        let latest = self.latest?;
        let [a, b, c, d, ..] = (self.seen >> 1).to_le_bytes();
        Some((latest, u32::from_le_bytes([a, b, c, d])))
    }

    /// Share of the window's sequences that never arrived, in permille. The
    /// newest [`RECV_GRACE`] positions are left out (still in flight) and a
    /// sequence that arrives late stops counting as missing. A sender that
    /// skips sequences on purpose reads as loss here.
    #[must_use]
    pub fn loss_permille(&self) -> u32 {
        let judged = self.depth.saturating_sub(RECV_GRACE);
        if judged == 0 {
            return 0;
        }
        let counted = low_bits(self.depth) & !low_bits(RECV_GRACE);
        let got = (self.seen & counted).count_ones();
        permille(judged - got, judged)
    }
}

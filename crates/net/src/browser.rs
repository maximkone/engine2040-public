//! browser: the web client side of the dedicated server's two browser legs.
//!
//! Purpose: a page joins the native server over WebTransport (datagrams plus
//! one reliable stream) or, when UDP is blocked, over a WebSocket, behind the
//! same [`Transport`](crate::quic::Transport) trait and the same payload
//! bytes the native transports carry. [`connect_race`](crate::connect_race)
//! starts both and keeps the first.
//!
//! Layers: `inbox` and `stream` are plain logic (they run and are tested
//! natively); `ws` and `wt` are the wasm32 glue over the page's own
//! `WebSocket` and `WebTransport`. `wt` declares the WebTransport bindings
//! itself: web-sys gates them behind an unstable cfg that would change every
//! build in the workspace.
//!
//! The browser has no sockets and no threads: everything is callback driven
//! on the page's one thread, so shared state is `Rc<RefCell<..>>`.
//!
//! Errors: [`LegError`](crate::connect_race::LegError) at connect,
//! [`NetError`](crate::session::NetError) on the trait, a closed-state reason
//! string once a session ends.
//!
//! Gates (QN7): a frame over the 1200-byte budget is refused before it touches a
//! socket (`quic::check_budget`, which every transport runs, the browser legs
//! included) and a pinned certificate longer than the minting limit is refused
//! before a key is made. Both refusals are counted for the whole process and read
//! back as a [`GateReceipt`], so a page that sent something too big says so
//! instead of losing it silently.

use std::sync::atomic::{AtomicU64, Ordering};

pub mod inbox;
pub mod stream;

#[cfg(target_arch = "wasm32")]
pub mod ws;
#[cfg(target_arch = "wasm32")]
pub mod wt;

static OVERSIZE_REFUSED: AtomicU64 = AtomicU64::new(0);
static CERT_REFUSED: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    /// The same two counts for the calling thread only, so a test can assert an exact
    /// number while other tests refuse frames in parallel.
    static HERE: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// Count one frame refused for being over the datagram budget before any write.
pub fn note_oversize() {
    OVERSIZE_REFUSED.fetch_add(1, Ordering::Relaxed);
    #[cfg(test)]
    HERE.with(|here| here.set((here.get().0 + 1, here.get().1)));
}

/// Count one certificate refused for a validity the browsers do not accept.
pub fn note_cert_refused() {
    CERT_REFUSED.fetch_add(1, Ordering::Relaxed);
    #[cfg(test)]
    HERE.with(|here| here.set((here.get().0, here.get().1 + 1)));
}

/// What the gates have refused so far, process-wide. Metrics only: the sim never reads it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GateReceipt {
    /// Frames refused for being over the 1200-byte budget, before any write.
    pub oversize_refused: u64,
    /// Certificates refused for a validity over the minting limit.
    pub cert_refused: u64,
}

impl GateReceipt {
    /// The counts now.
    #[must_use]
    pub fn read() -> Self {
        Self {
            oversize_refused: OVERSIZE_REFUSED.load(Ordering::Relaxed),
            cert_refused: CERT_REFUSED.load(Ordering::Relaxed),
        }
    }

    /// The receipt as the one line the page and the server print.
    #[must_use]
    pub fn line(self) -> String {
        format!(
            "gate: oversize-refused={} cert-refused={}",
            self.oversize_refused, self.cert_refused
        )
    }

    /// The counts of the calling thread alone (tests).
    #[cfg(test)]
    fn here() -> Self {
        let (oversize_refused, cert_refused) = HERE.with(std::cell::Cell::get);
        Self {
            oversize_refused,
            cert_refused,
        }
    }
}

/// What a browser client knows about its session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClientState {
    /// The handshake has not finished.
    Connecting,
    /// Frames flow.
    Open,
    /// The session ended; the reason names who ended it and how.
    Closed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quic::{InboundFrame, Transport, UdpTransport};
    use crate::session::{NetError, MAX_PACKET_BYTES};
    use crate::websocket::wire::{self, Class};

    /// Two loopback transports: `a` sends, `b` is whoever would receive the write.
    fn pair() -> (UdpTransport, UdpTransport) {
        (
            UdpTransport::bind_loopback().expect("bind a"),
            UdpTransport::bind_loopback().expect("bind b"),
        )
    }

    #[test]
    fn browser_gate_refuses_a_1201_byte_datagram_before_the_write_and_counts_it() {
        let (mut a, mut b) = pair();
        let to = crate::quic::PeerId(b.local_addr().expect("addr"));
        let before = GateReceipt::here();
        assert_eq!(
            a.send_unreliable(to, &vec![7u8; MAX_PACKET_BYTES + 1]),
            Err(NetError::TooLarge)
        );
        assert_eq!(
            GateReceipt::here().oversize_refused,
            before.oversize_refused + 1,
            "the refusal is counted"
        );
        // Nothing reached the socket: the 1200-byte frame after it is the first to arrive.
        a.send_unreliable(to, &vec![9u8; MAX_PACKET_BYTES])
            .expect("the budget itself passes");
        let mut got = Vec::new();
        for _ in 0..1_000 {
            got = b.recv();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(got.len(), 1, "only the in-budget frame was written");
        assert!(
            matches!(&got[0].frame, InboundFrame::Datagram(bytes) if bytes.len() == MAX_PACKET_BYTES)
        );
        assert_eq!(
            GateReceipt::here().oversize_refused,
            before.oversize_refused + 1,
            "an accepted frame is not counted"
        );
    }

    #[test]
    fn browser_gate_counts_the_websocket_message_refusal_too() {
        let before = GateReceipt::here();
        assert_eq!(
            wire::encode(Class::Datagram, &vec![0u8; MAX_PACKET_BYTES + 1]),
            Err(NetError::TooLarge)
        );
        assert!(wire::encode(Class::Datagram, &vec![0u8; MAX_PACKET_BYTES]).is_ok());
        assert_eq!(
            GateReceipt::here().oversize_refused,
            before.oversize_refused + 1
        );
    }

    #[test]
    fn browser_gate_receipt_line_shows_both_counts() {
        let receipt = GateReceipt {
            oversize_refused: 3,
            cert_refused: 2,
        };
        assert_eq!(receipt.line(), "gate: oversize-refused=3 cert-refused=2");
        let now = GateReceipt::read();
        assert!(now.oversize_refused >= GateReceipt::here().oversize_refused);
    }
}

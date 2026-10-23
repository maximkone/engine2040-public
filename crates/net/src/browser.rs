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
//!
//! Background tab (QN9): a hidden tab gets no timer callbacks, so its frames stop
//! and its prediction goes stale by however long the tab was hidden. [`TabWatch`]
//! turns that silent gap into a dropped and rejoined session, and its
//! [`TabReceipt`] names the hidden duration, which no packet on the wire carries.

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

/// The longest gap between page frames that still means "the tab was visible
/// and running". A hidden tab gets no timer callbacks and no animation frames,
/// so the first frame after a gap at or past this is a hidden stall.
pub const HIDDEN_STALL_MS: u64 = 1_000;

/// One hidden stall: the page had zero frames for `hidden_ms` of wall clock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TabGap {
    /// Wall-clock milliseconds the page's timers were silent.
    pub hidden_ms: u64,
    /// Session ticks the stall covers at the page's tick rate.
    pub ticks: u64,
}

/// Background-tab policy (QN9): a hidden tab stops the page's timers, so the
/// frames stop with them and the client's prediction goes stale by however long
/// the tab was hidden. The page feeds [`TabWatch::on_frame`] its frame clock;
/// the first frame back answers the gap, which drops the session and rejoins it
/// instead of trusting the stale prediction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TabWatch {
    last_ms: Option<u64>,
    hz: u32,
    stalls: u32,
    hidden_total_ms: u64,
    drops: u32,
    rejoins: u32,
}

impl TabWatch {
    /// A watch over a page whose session ticks `hz` times a second (60 for the
    /// fixed tick, so six silent seconds are 360 ticks).
    #[must_use]
    pub fn new(hz: u32) -> Self {
        Self {
            hz: hz.max(1),
            ..Self::default()
        }
    }

    /// Feed the page's frame clock. Answers the gap on the first frame after a
    /// hidden stall, `None` while the tab runs and on every frame after it.
    pub fn on_frame(&mut self, now_ms: u64) -> Option<TabGap> {
        let last = self.last_ms.replace(now_ms);
        let last = last?;
        let hidden_ms = now_ms.saturating_sub(last);
        if hidden_ms < HIDDEN_STALL_MS {
            return None;
        }
        self.stalls = self.stalls.saturating_add(1);
        self.hidden_total_ms = self.hidden_total_ms.saturating_add(hidden_ms);
        Some(TabGap {
            hidden_ms,
            ticks: hidden_ms.saturating_mul(u64::from(self.hz)) / 1_000,
        })
    }

    /// Count the drop a stall forced (one per stall).
    pub fn record_drop(&mut self) {
        self.drops = self.drops.saturating_add(1);
    }

    /// Count the rejoin that restored the session after the stall.
    pub fn record_rejoin(&mut self) {
        self.rejoins = self.rejoins.saturating_add(1);
    }

    /// The counts now.
    #[must_use]
    pub fn receipt(&self) -> TabReceipt {
        TabReceipt {
            hidden_total_ms: self.hidden_total_ms,
            stalls: self.stalls,
            drops: self.drops,
            rejoins: self.rejoins,
        }
    }
}

/// What the hidden-stall policy saw, in the one line the page and the server
/// print. The hidden duration is here because a stall is invisible on the wire:
/// no frame, no input, no packet says how long the tab was gone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TabReceipt {
    /// Wall-clock milliseconds the page spent with zero frames, summed.
    pub hidden_total_ms: u64,
    /// Hidden stalls seen (a gap at or past [`HIDDEN_STALL_MS`]).
    pub stalls: u32,
    /// Drops the stalls forced.
    pub drops: u32,
    /// Rejoins that restored the session.
    pub rejoins: u32,
}

impl TabReceipt {
    /// The receipt as the one line the page and the server print.
    #[must_use]
    pub fn line(self) -> String {
        format!(
            "tab: hidden-ms={} stalls={} drops={} rejoins={}",
            self.hidden_total_ms, self.stalls, self.drops, self.rejoins
        )
    }
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

    #[test]
    fn tab_watch_first_frame_and_a_short_gap_are_not_stalls() {
        let mut tab = TabWatch::new(60);
        assert_eq!(
            tab.on_frame(0),
            None,
            "the first frame has no gap to measure"
        );
        assert_eq!(tab.on_frame(16), None);
        assert_eq!(
            tab.on_frame(HIDDEN_STALL_MS + 15),
            None,
            "999 ms is a slow frame"
        );
        assert_eq!(tab.receipt().stalls, 0);
        assert_eq!(tab.receipt().hidden_total_ms, 0);
    }

    #[test]
    fn tab_watch_six_silent_seconds_is_one_stall_of_three_hundred_sixty_ticks() {
        let mut tab = TabWatch::new(60);
        tab.on_frame(0);
        tab.on_frame(16);
        // The tab hides: zero frames for six seconds, then one frame comes back.
        let gap = tab.on_frame(16 + 6_000).expect("a hidden stall");
        assert_eq!(gap.hidden_ms, 6_000);
        assert_eq!(gap.ticks, 360, "6 s at 60 Hz is 360 session ticks");
        assert_eq!(
            tab.on_frame(16 + 6_000 + 16),
            None,
            "the stall is counted once"
        );
        assert_eq!(tab.receipt().stalls, 1);
        assert_eq!(tab.receipt().hidden_total_ms, 6_000);
    }

    #[test]
    fn tab_watch_receipt_line_carries_the_hidden_duration() {
        let mut tab = TabWatch::new(60);
        tab.on_frame(0);
        assert!(tab.on_frame(6_000).is_some());
        tab.record_drop();
        tab.record_rejoin();
        let receipt = tab.receipt();
        assert_eq!(receipt.drops, 1);
        assert_eq!(receipt.rejoins, 1);
        assert_eq!(
            receipt.line(),
            "tab: hidden-ms=6000 stalls=1 drops=1 rejoins=1"
        );
        assert!(receipt.line().contains("hidden-ms=6000"));
    }

    #[test]
    fn tab_watch_a_clock_that_steps_back_counts_no_stall() {
        let mut tab = TabWatch::new(60);
        tab.on_frame(5_000);
        assert_eq!(
            tab.on_frame(1_000),
            None,
            "a backwards clock is not a stall"
        );
        assert_eq!(tab.receipt().stalls, 0);
        assert_eq!(
            TabWatch::new(0).on_frame(0),
            None,
            "zero hertz cannot stall"
        );
    }
}

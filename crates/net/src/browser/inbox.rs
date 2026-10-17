//! inbox: frames received from the server, waiting for the tick loop.
//!
//! The same shedding rule as the native servers: a datagram-class frame is
//! dropped (and counted) when it is over the budget or the inbox is full,
//! like loss; a reliable frame is never dropped.

use std::collections::VecDeque;

use crate::quic::{ControlKind, Inbound, InboundFrame, PeerId};
use crate::session::MAX_PACKET_BYTES;

/// Frames held before datagrams are dropped.
pub const INBOX_FRAMES: usize = 1024;

/// Received frames from one server.
#[derive(Debug)]
pub struct Inbox {
    from: PeerId,
    frames: VecDeque<Inbound>,
    dropped_datagrams: u64,
}

impl Inbox {
    /// An empty inbox for frames sent by `from`.
    #[must_use]
    pub fn new(from: PeerId) -> Self {
        Self {
            from,
            frames: VecDeque::new(),
            dropped_datagrams: 0,
        }
    }

    /// Queue one datagram-class frame, or shed it.
    pub fn push_datagram(&mut self, payload: &[u8]) {
        if payload.len() > MAX_PACKET_BYTES || self.frames.len() >= INBOX_FRAMES {
            self.dropped_datagrams += 1;
            return;
        }
        self.frames.push_back(Inbound {
            from: self.from,
            frame: InboundFrame::Datagram(payload.to_vec()),
        });
    }

    /// Queue one reliable frame. It always fits: the browser trusts the
    /// order the server wrote and cannot push back on a WebSocket.
    pub fn push_stream(&mut self, kind: ControlKind, payload: Vec<u8>) {
        self.frames.push_back(Inbound {
            from: self.from,
            frame: InboundFrame::Stream {
                kind,
                bytes: payload,
            },
        });
    }

    /// Everything received so far, oldest first.
    pub fn drain(&mut self) -> Vec<Inbound> {
        self.frames.drain(..).collect()
    }

    /// Datagram-class frames shed so far.
    #[must_use]
    pub fn dropped_datagrams(&self) -> u64 {
        self.dropped_datagrams
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    fn server() -> PeerId {
        PeerId(SocketAddr::from(([203, 0, 113, 7], 443)))
    }

    #[test]
    fn browser_inbox_keeps_arrival_order_across_both_classes() {
        let mut inbox = Inbox::new(server());
        inbox.push_datagram(b"d1");
        inbox.push_stream(ControlKind::Snapshot, b"s1".to_vec());
        inbox.push_datagram(b"d2");
        let got = inbox.drain();
        let bytes: Vec<Vec<u8>> = got
            .iter()
            .map(|frame| {
                assert_eq!(frame.from, server());
                match &frame.frame {
                    InboundFrame::Datagram(bytes) | InboundFrame::Stream { bytes, .. } => {
                        bytes.clone()
                    }
                }
            })
            .collect();
        assert_eq!(bytes, vec![b"d1".to_vec(), b"s1".to_vec(), b"d2".to_vec()]);
        assert!(inbox.drain().is_empty(), "drain empties the inbox");
    }

    #[test]
    fn browser_inbox_sheds_datagrams_but_never_reliable_frames() {
        let mut inbox = Inbox::new(server());
        inbox.push_datagram(&vec![0; MAX_PACKET_BYTES + 1]);
        assert_eq!(inbox.dropped_datagrams(), 1, "over budget sheds");
        for _ in 0..INBOX_FRAMES {
            inbox.push_datagram(b"x");
        }
        inbox.push_datagram(b"one too many");
        assert_eq!(inbox.dropped_datagrams(), 2, "a full inbox sheds");
        inbox.push_stream(ControlKind::Lobby, b"kept".to_vec());
        let got = inbox.drain();
        assert_eq!(got.len(), INBOX_FRAMES + 1, "the reliable frame got in");
        assert!(matches!(
            got.last().map(|frame| &frame.frame),
            Some(InboundFrame::Stream {
                kind: ControlKind::Lobby,
                ..
            })
        ));
    }
}

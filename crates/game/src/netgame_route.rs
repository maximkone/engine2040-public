//! netgame_route: where a [`NetGame`](super::NetGame) peer sends and hears
//! datagrams. Straight to the peer, or wrapped for the self-hosted relay
//! (`net_relay`, the ON2 `RelayHub`); the optional fault link sits between
//! the queue and the socket. A host pairs with the guest whose hello carries
//! its match identity; strangers and junk are counted, never applied.
//!
//! Consumer: `netgame` (the session) and `net_seat` through it. Sockets and
//! clocks never reach a hash.

use std::io::ErrorKind;
use std::net::SocketAddr;

use engine_net::link::Frame;
use engine_net::relay::{RelayConfig, RelayMsg};
use engine_net::session::{NetError, KIND_INPUT};

use super::netgame_wire::{decode_greeting, encode_greeting, peek_kind, KIND_HELLO, KIND_WELCOME};
use super::{NetApplied, NetGame};
use crate::GameError;

/// Largest datagram this peer reads: a relay frame around a full game packet.
const RECV_BYTES: usize = engine_net::relay::MAX_RELAY_BYTES + 32;

/// Where this peer's datagrams go: straight to the peer's socket, or wrapped
/// for the self-hosted relay (`net_relay`), which forwards between two peer ids.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Route {
    /// Straight to [`NetGame::peer`].
    Direct,
    /// Through the relay at [`NetGame::peer`].
    Relay {
        /// This peer's relay id (nonzero).
        me: u64,
        /// The other side's relay id: the guest reads it from the invitation,
        /// the host learns it from the first accepted hello.
        peer_id: Option<u64>,
        /// The relay confirmed `me`.
        registered: bool,
    },
}

/// One received datagram after the route: game bytes (with the relay sender
/// id), a note from the relay itself, or nothing for this peer.
enum Inbound {
    /// Game bytes, plus the relay id they came from on a relay route.
    Game(Vec<u8>, Option<u64>),
    /// The relay's own answer.
    Note(NetApplied),
    /// Not for this peer: counted, dropped.
    Stray,
}

impl NetGame {
    /// True once a peer is paired: the guest's address (or relay id) on the
    /// host, always on a guest.
    #[must_use]
    pub fn paired(&self) -> bool {
        match self.route {
            Route::Direct => self.peer.is_some(),
            Route::Relay { peer_id, .. } => peer_id.is_some(),
        }
    }

    /// Forget the paired guest (host only): the next accepted hello or input
    /// pairs again, from any address. The seat calls this once a guest went silent.
    pub fn unpair(&mut self) {
        if !self.is_host() {
            return;
        }
        match &mut self.route {
            Route::Direct => self.peer = None,
            Route::Relay { peer_id, .. } => *peer_id = None,
        }
    }

    /// True when the route is the relay and the relay has not confirmed this peer yet.
    #[must_use]
    pub fn relay_pending(&self) -> bool {
        matches!(
            self.route,
            Route::Relay {
                registered: false,
                ..
            }
        )
    }

    /// Ask the relay to register this peer under the default relay token
    /// (`net_relay`'s default). Nothing on a direct route.
    pub fn relay_register(&mut self) -> Result<(), GameError> {
        let (Route::Relay { me, .. }, Some(relay)) = (self.route, self.peer) else {
            return Ok(());
        };
        let token = RelayConfig::default().token;
        let bytes = RelayMsg::Register {
            client_id: me,
            token,
        }
        .encode()
        .map_err(|_| GameError::BadConfig("relay_id"))?;
        self.sock
            .send_to(&bytes, relay)
            .map_err(|err| GameError::Io(err.kind()))?;
        Ok(())
    }

    /// Send every queued datagram to the peer over the real socket (through
    /// the fault link first when one is set, wrapped for the relay on a relay
    /// route). Returns datagrams sent; `Ok(0)` with the queue kept while there
    /// is nobody to send to yet (the host before first contact, a relay route
    /// before registration). IO errors are the caller's.
    pub fn flush(&mut self) -> std::io::Result<usize> {
        let Some(peer) = self.peer else {
            return Ok(0);
        };
        let wrap = match self.route {
            Route::Direct => None,
            Route::Relay {
                me,
                peer_id: Some(to),
                registered: true,
            } => Some((me, to)),
            Route::Relay { .. } => return Ok(0),
        };
        let mut ready = std::mem::take(&mut self.outbound);
        if let Some(link) = self.fault.as_mut() {
            for bytes in &ready {
                link.send_datagram(bytes);
            }
            ready = link
                .due()
                .into_iter()
                .map(|frame| match frame {
                    Frame::Datagram(bytes) | Frame::Stream(bytes) => bytes,
                })
                .collect();
            link.tick();
        }
        let mut sent = 0usize;
        for bytes in ready {
            let wire = match wrap {
                None => bytes,
                Some((from, to)) => match (RelayMsg::Forward {
                    from,
                    to,
                    payload: bytes,
                })
                .encode()
                {
                    Ok(wire) => wire,
                    Err(_) => continue,
                },
            };
            self.sock.send_to(&wire, peer)?;
            sent += 1;
        }
        Ok(sent)
    }

    /// Non-blocking receive: decode plus apply every waiting datagram.
    /// A host pairs with the guest whose hello carries its match identity
    /// (or, on the predicting path, with the sender of the first valid
    /// input); a guest hears only its host. Datagrams from anyone else, and
    /// junk that paired nothing, are counted in [`NetGame::stray`] and
    /// dropped: an open port hears strangers. A bad datagram from the paired
    /// peer aborts loudly.
    pub fn poll(&mut self) -> Result<Vec<NetApplied>, GameError> {
        let mut applied = Vec::new();
        let mut buf = [0u8; RECV_BYTES];
        loop {
            let (n, src) = match self.sock.recv_from(&mut buf) {
                Ok(got) => got,
                Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                // Windows answers a datagram sent before the peer was up with
                // a reset on the next receive: nothing arrived, read on.
                Err(err) if err.kind() == ErrorKind::ConnectionReset => continue,
                Err(err) => return Err(GameError::Io(err.kind())),
            };
            if let Some(got) = self.receive(src, &buf[..n])? {
                applied.push(got);
            }
        }
        Ok(applied)
    }

    /// One received datagram: unwrap the route, pair or refuse a hello, apply
    /// the peer's packets, count everything else as stray.
    fn receive(&mut self, src: SocketAddr, bytes: &[u8]) -> Result<Option<NetApplied>, GameError> {
        let (payload, from_id) = match self.unwrap_route(src, bytes)? {
            Inbound::Game(payload, from_id) => (payload, from_id),
            Inbound::Note(note) => return Ok(Some(note)),
            Inbound::Stray => {
                self.stray += 1;
                return Ok(None);
            }
        };
        let from_peer = match self.route {
            Route::Direct => self.peer == Some(src),
            Route::Relay { peer_id, .. } => peer_id.is_some() && peer_id == from_id,
        };
        let kind = peek_kind(&payload).ok();
        if self.is_host() && kind == Some(KIND_HELLO) && (from_peer || !self.paired()) {
            return self.on_hello(src, from_id, &payload);
        }
        if from_peer {
            return self.on_net_event(&payload).map(Some);
        }
        if self.is_host() && !self.paired() && kind == Some(KIND_INPUT) {
            if let Ok(applied) = self.on_net_event(&payload) {
                self.pair(src, from_id);
                return Ok(Some(applied));
            }
        }
        self.stray += 1;
        Ok(None)
    }

    /// Strip the relay frame on a relay route: the relay's own answers become
    /// notes (or a loud refusal), forwards from the expected id become game
    /// bytes, anything else is stray. A direct route passes bytes through.
    fn unwrap_route(&mut self, src: SocketAddr, bytes: &[u8]) -> Result<Inbound, GameError> {
        let Route::Relay { me, peer_id, .. } = self.route else {
            return Ok(Inbound::Game(bytes.to_vec(), None));
        };
        if self.peer != Some(src) {
            return Ok(Inbound::Stray);
        }
        match RelayMsg::decode(bytes) {
            Ok(RelayMsg::Registered { client_id }) if client_id == me => {
                if let Route::Relay { registered, .. } = &mut self.route {
                    *registered = true;
                }
                Ok(Inbound::Note(NetApplied::RelayRegistered))
            }
            Ok(RelayMsg::Denied { code, .. }) => Err(GameError::BadConfig(match code {
                0 => "relay_token",
                1 => "relay_full",
                _ => "relay_input",
            })),
            Ok(RelayMsg::Forward { from, to, payload })
                if to == me && peer_id.is_none_or(|id| id == from) =>
            {
                Ok(Inbound::Game(payload, Some(from)))
            }
            _ => Ok(Inbound::Stray),
        }
    }

    /// A hello at the host: the same identity pairs the sender (when
    /// unpaired) and queues the welcome; another identity is answered with the
    /// host's welcome straight to the sender and pairs nothing. Junk is stray.
    fn on_hello(
        &mut self,
        src: SocketAddr,
        from_id: Option<u64>,
        payload: &[u8],
    ) -> Result<Option<NetApplied>, GameError> {
        let Ok((guest_identity, guest_code)) = decode_greeting(payload) else {
            self.stray += 1;
            return Ok(None);
        };
        let accepted = guest_identity == self.identity;
        let welcome = encode_greeting(KIND_WELCOME, self.identity, &self.match_code());
        if accepted {
            if !self.paired() {
                self.pair(src, from_id);
            }
            self.outbound.push(welcome);
        } else {
            self.send_to_source(src, from_id, &welcome)?;
        }
        Ok(Some(NetApplied::Hello {
            accepted,
            guest_identity,
            guest_code,
        }))
    }

    /// Pair the host with a sender: its address, or its relay id.
    fn pair(&mut self, src: SocketAddr, from_id: Option<u64>) {
        match &mut self.route {
            Route::Direct => self.peer = Some(src),
            Route::Relay { peer_id, .. } => *peer_id = from_id,
        }
    }

    /// Send one datagram to a sender that is not (or not yet) the peer: a
    /// refusal. Bypasses the queue and the fault link.
    fn send_to_source(
        &mut self,
        src: SocketAddr,
        from_id: Option<u64>,
        bytes: &[u8],
    ) -> Result<(), GameError> {
        let wire = match (self.route, from_id) {
            (Route::Relay { me, .. }, Some(to)) => RelayMsg::Forward {
                from: me,
                to,
                payload: bytes.to_vec(),
            }
            .encode()
            .map_err(|_| GameError::Net(NetError::TooLarge))?,
            _ => bytes.to_vec(),
        };
        self.sock
            .send_to(&wire, src)
            .map_err(|err| GameError::Io(err.kind()))?;
        Ok(())
    }
}

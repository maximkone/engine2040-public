//! Steam relay transport: lobbies and the Steam relay as a second session
//! transport beside QUIC and the invite-code relay, chosen per session.
//!
//! Owner: net lane. Units: ticks at 60 Hz, bytes on the wire capped at
//! [`MAX_RELAY_BYTES`], overlay invite codes of [`INVITE_BYTES`]. Determinism:
//! peer tables are `BTreeMap`s, invite lifetime is tick counts, sockets and
//! wall clocks never enter sim hashes; the QUIC path is untouched (no edits
//! outside this file plus its `mod` line). Errors: [`RelayError`] reused from
//! the invite-code relay so one owner names every refusal.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::relay::{
    InviteCode, InviteTicket, RelayError, INVITE_BYTES, MAX_RELAY_BYTES, MAX_RELAY_PEERS,
    MAX_RELAY_QUEUE,
};

/// Per-session transport choice: QUIC stays the default, Steam relay carries
/// overlay-invite sessions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransportKind {
    /// QUIC/UDP path (default; behavior unchanged by this module).
    Quic,
    /// Steam lobby relay path for overlay-invite sessions.
    SteamRelay,
}

impl TransportKind {
    /// True for the Steam relay leg.
    #[must_use]
    pub fn is_relay(self) -> bool {
        matches!(self, Self::SteamRelay)
    }

    /// Receipt name for manifests and logs.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Quic => "quic",
            Self::SteamRelay => "steam-relay",
        }
    }
}

/// Per-endpoint receipt: bytes moved and queue depth after the call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RelayReceipt {
    /// Bytes accepted (send) or drained (receive).
    pub bytes: usize,
    /// Queued payloads still held for the endpoint.
    pub queued: usize,
}

/// Steam relay fabric: overlay-invite joins plus bounded per-peer queues.
/// Two accounts join by overlay invite and converge through `send`/`drain`;
/// the fabric never touches a socket (the SDK datagram pump behind the
/// `steam` feature drains the same queues).
#[derive(Debug, Default)]
pub struct SteamRelay {
    /// Registered account ids.
    accounts: BTreeSet<u64>,
    /// Overlay invite each account joined with.
    joined: BTreeMap<u64, [u8; INVITE_BYTES]>,
    /// Queued inbound payloads per account, in arrival order.
    queues: BTreeMap<u64, VecDeque<Vec<u8>>>,
    /// Per-account queue bound (defaults to [`MAX_RELAY_QUEUE`]).
    queue_cap: usize,
}

impl SteamRelay {
    /// Empty fabric with the default queue bound.
    pub fn new() -> Self {
        Self {
            accounts: BTreeSet::new(),
            joined: BTreeMap::new(),
            queues: BTreeMap::new(),
            queue_cap: MAX_RELAY_QUEUE,
        }
    }

    /// Empty fabric with an explicit per-account queue bound (0 refuses).
    pub fn with_queue_cap(queue_cap: usize) -> Self {
        Self {
            accounts: BTreeSet::new(),
            joined: BTreeMap::new(),
            queues: BTreeMap::new(),
            queue_cap,
        }
    }

    /// Registered account count.
    #[must_use]
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Register one Steam account id; duplicates and full tables refuse.
    pub fn register(&mut self, account: u64) -> Result<(), RelayError> {
        if self.accounts.contains(&account) {
            return Ok(());
        }
        if self.accounts.len() >= MAX_RELAY_PEERS {
            return Err(RelayError::SlotFull);
        }
        self.accounts.insert(account);
        self.queues.insert(account, VecDeque::new());
        Ok(())
    }

    /// Join by overlay invite: the ticket must be live at `now_tick` and the
    /// code must match the ticket's code, else the join refuses loudly.
    pub fn join_by_overlay_invite(
        &mut self,
        account: u64,
        code: &InviteCode,
        ticket: &InviteTicket,
        now_tick: u32,
    ) -> Result<(), RelayError> {
        if !self.accounts.contains(&account) {
            return Err(RelayError::UnknownPeer(account));
        }
        if !ticket.is_live(now_tick) {
            return Err(RelayError::Expired);
        }
        if ticket.code != *code {
            return Err(RelayError::Unauthorized);
        }
        self.joined.insert(account, code.as_bytes());
        Ok(())
    }

    /// True once the account joined by overlay invite.
    #[must_use]
    pub fn is_joined(&self, account: u64) -> bool {
        self.joined.contains_key(&account)
    }

    /// Send one payload to a joined peer: size, membership and queue bounds
    /// refuse loudly; full queues refuse, never drop.
    pub fn send(&mut self, from: u64, to: u64, payload: &[u8]) -> Result<RelayReceipt, RelayError> {
        if payload.len() > MAX_RELAY_BYTES {
            return Err(RelayError::TooLarge);
        }
        if !self.accounts.contains(&from) {
            return Err(RelayError::UnknownPeer(from));
        }
        if !self.accounts.contains(&to) || !self.joined.contains_key(&to) {
            return Err(RelayError::UnknownPeer(to));
        }
        let Some(queue) = self.queues.get_mut(&to) else {
            return Err(RelayError::UnknownPeer(to));
        };
        if queue.len() >= self.queue_cap {
            return Err(RelayError::QueueFull);
        }
        queue.push_back(payload.to_vec());
        Ok(RelayReceipt {
            bytes: payload.len(),
            queued: queue.len(),
        })
    }

    /// Drain one account's inbound queue in arrival order.
    pub fn drain(&mut self, account: u64) -> Result<Vec<Vec<u8>>, RelayError> {
        if !self.accounts.contains(&account) {
            return Err(RelayError::UnknownPeer(account));
        }
        let Some(queue) = self.queues.get_mut(&account) else {
            return Err(RelayError::UnknownPeer(account));
        };
        Ok(queue.drain(..).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket_for(code: &InviteCode, now: u32) -> InviteTicket {
        InviteTicket::issue(*code, 1, 0xABCD, now).expect("ticket")
    }

    fn joined_pair(now: u32) -> (SteamRelay, u64, u64) {
        let mut state = 0x1234_5678_9ABC_DEF0;
        let code = InviteCode::mint(&mut state);
        let ticket = ticket_for(&code, now);
        let mut relay = SteamRelay::new();
        relay.register(1001).expect("register a");
        relay.register(1002).expect("register b");
        relay
            .join_by_overlay_invite(1001, &code, &ticket, now)
            .expect("join a");
        relay
            .join_by_overlay_invite(1002, &code, &ticket, now)
            .expect("join b");
        assert!(relay.is_joined(1001) && relay.is_joined(1002));
        (relay, 1001, 1002)
    }

    #[test]
    fn two_accounts_join_by_overlay_invite_and_converge() {
        let (mut relay, a, b) = joined_pair(500);
        let rounds: usize = 64;
        for round in 0..rounds {
            let from_a = vec![u8::try_from(round % 251).expect("byte"), 0xA5];
            let from_b = vec![0x5A, u8::try_from(round % 251).expect("byte")];
            let first = relay.send(a, b, &from_a).expect("a sends");
            assert_eq!(first.bytes, from_a.len());
            let second = relay.send(b, a, &from_b).expect("b sends");
            assert_eq!(second.bytes, from_b.len());
            assert_eq!(relay.drain(b).expect("drain b"), vec![from_a]);
            assert_eq!(relay.drain(a).expect("drain a"), vec![from_b]);
        }
        assert_eq!(relay.account_count(), 2);
        assert!(TransportKind::SteamRelay.is_relay());
        assert!(!TransportKind::Quic.is_relay());
        assert_eq!(TransportKind::SteamRelay.name(), "steam-relay");
    }

    #[test]
    fn invite_and_queue_bounds_refuse_loudly() {
        let mut state = 0x0BAD_F00D_CAFE_BABE;
        let code = InviteCode::mint(&mut state);
        let ticket = ticket_for(&code, 100);
        let mut relay = SteamRelay::new();
        assert_eq!(
            relay.join_by_overlay_invite(7, &code, &ticket, 100),
            Err(RelayError::UnknownPeer(7)),
            "unregistered account refuses"
        );
        relay.register(7).expect("register");
        assert_eq!(
            relay.join_by_overlay_invite(
                7,
                &code,
                &ticket,
                100 + crate::relay::INVITE_TTL_TICKS + 1
            ),
            Err(RelayError::Expired),
            "stale invite refuses"
        );
        let other = InviteCode::mint(&mut state);
        assert_eq!(
            relay.join_by_overlay_invite(7, &other, &ticket, 100),
            Err(RelayError::Unauthorized),
            "code mismatch refuses"
        );
        relay
            .join_by_overlay_invite(7, &code, &ticket, 100)
            .expect("join");
        assert_eq!(
            relay.send(7, 9, b"hi"),
            Err(RelayError::UnknownPeer(9)),
            "unknown peer refuses"
        );
        assert_eq!(
            relay.send(7, 7, &vec![0u8; MAX_RELAY_BYTES + 1]),
            Err(RelayError::TooLarge),
            "oversize payload refuses"
        );
        assert!(relay.drain(9).is_err(), "unknown drain refuses");
        let mut tight = SteamRelay::with_queue_cap(1);
        tight.register(1).expect("register 1");
        tight.register(2).expect("register 2");
        let code2 = InviteCode::mint(&mut state);
        let ticket2 = ticket_for(&code2, 0);
        tight
            .join_by_overlay_invite(2, &code2, &ticket2, 0)
            .expect("join 2");
        tight.send(1, 2, b"one").expect("first fits");
        assert_eq!(
            tight.send(1, 2, b"two"),
            Err(RelayError::QueueFull),
            "full queue refuses, never drops"
        );
        assert_eq!(tight.drain(2).expect("drain").len(), 1);
    }
}

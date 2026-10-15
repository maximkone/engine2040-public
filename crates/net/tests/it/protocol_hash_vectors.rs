//! Protocol-hash plus stale-connection refuse vectors.
//!
//! Purpose: refuse joins from mismatched builds and stale connection numbers
//! before they mutate match state. Owner: engine-net handshake gate. Units:
//! protocol id (u8), build and channel digests (u64), connection numbers
//! (u32), member counts (u32). Determinism: integer FNV-1a folds only, no
//! float, no wall clock, no map iteration in hashed state. Errors: typed
//! JoinRefuse, callers match on the variant.
//!
//! Attribution (MIT and Apache class, reimplemented shape, no vendor bytes):
//! PORT(RevenantX/LiteNetLib:LiteNetLib NetConstants plus NetConnectRequest
//! flow -- InternalPackets connect flow, ProtocolId 13 connect hash,
//! ConnectionNumber stale drop) and
//! PORT(cBournhonesque/lightyear:channel registry digest -- per-channel
//! kind and name folded into one handshake digest).

use engine_core::Fnv64;
use std::fmt;

/// Wire protocol id carried on every connect request.
const PROTOCOL_ID: u8 = 13;

/// Connection numbers below this are stale handshake replays and must drop.
const MIN_FRESH_CONNECTION_NUMBER: u32 = 4;

/// Channel table folded into the handshake digest.
const CHANNELS: [(&str, u8); 3] = [("input", 0), ("snapshot", 0), ("event", 1)];

/// Build tag the server was cooked with.
const SERVER_BUILD: &str = "engine2040-net/1";

/// FNV-1a connect hash over protocol id plus a length-prefixed build tag.
fn connect_hash(build: &str) -> u64 {
    let mut fold = Fnv64::new();
    fold.write_u8(PROTOCOL_ID);
    let len = u32::try_from(build.len()).expect("build tag fits u32");
    fold.write_u32(len);
    fold.write_str(build);
    fold.finish()
}

/// Channel-registry digest: each name length-prefixed with its kind byte.
fn channel_digest() -> u64 {
    let mut fold = Fnv64::new();
    for (name, kind) in CHANNELS {
        let len = u32::try_from(name.len()).expect("channel name fits u32");
        fold.write_u32(len);
        fold.write_str(name);
        fold.write_u8(kind);
    }
    fold.finish()
}

/// One inbound join attempt as seen on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ConnectRequest {
    build_hash: u64,
    channel: u64,
    conn_number: u32,
}

impl ConnectRequest {
    fn legit(conn_number: u32) -> Self {
        Self {
            build_hash: connect_hash(SERVER_BUILD),
            channel: channel_digest(),
            conn_number,
        }
    }
}

/// Why a join was refused; refused joins must not mutate server state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JoinRefuse {
    BuildMismatch { got: u64, want: u64 },
    ChannelMismatch { got: u64, want: u64 },
    StaleConn { got: u32 },
}

impl fmt::Display for JoinRefuse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            JoinRefuse::BuildMismatch { got, want } => {
                write!(f, "build mismatch: got {got:#x} want {want:#x}")
            }
            JoinRefuse::ChannelMismatch { got, want } => {
                write!(f, "channel mismatch: got {got:#x} want {want:#x}")
            }
            JoinRefuse::StaleConn { got } => write!(f, "stale connection: {got}"),
        }
    }
}

/// Minimal match gate: fixed build and channel digests, roster count plus
/// a deterministic state fold. Only accepted joins mutate it.
struct MatchGate {
    want_build: u64,
    want_channel: u64,
    members: u32,
    state: u64,
}

impl MatchGate {
    fn new() -> Self {
        Self {
            want_build: connect_hash(SERVER_BUILD),
            want_channel: channel_digest(),
            members: 0,
            state: Fnv64::new().finish(),
        }
    }

    fn snapshot(&self) -> (u32, u64) {
        (self.members, self.state)
    }

    fn try_join(&mut self, req: &ConnectRequest) -> Result<u32, JoinRefuse> {
        if req.build_hash != self.want_build {
            return Err(JoinRefuse::BuildMismatch {
                got: req.build_hash,
                want: self.want_build,
            });
        }
        if req.channel != self.want_channel {
            return Err(JoinRefuse::ChannelMismatch {
                got: req.channel,
                want: self.want_channel,
            });
        }
        if req.conn_number < MIN_FRESH_CONNECTION_NUMBER {
            return Err(JoinRefuse::StaleConn {
                got: req.conn_number,
            });
        }
        let id = self.members;
        self.members = self.members.wrapping_add(1);
        let mut fold = Fnv64::with_state(self.state);
        fold.write_u32(id);
        fold.write_u32(req.conn_number);
        fold.write_u64(req.build_hash);
        self.state = fold.finish();
        Ok(id)
    }
}

fn fresh_number(i: usize) -> u32 {
    let base = u32::try_from(i).expect("case index fits u32");
    MIN_FRESH_CONNECTION_NUMBER.wrapping_add(base)
}

#[test]
fn mismatched_build_refuses_20_of_20() {
    let mut gate = MatchGate::new();
    let before = gate.snapshot();
    let mut refused = 0_u32;
    for i in 0..20_usize {
        let foreign = std::format!("foreign-build-{i}");
        let req = ConnectRequest {
            build_hash: connect_hash(&foreign),
            channel: channel_digest(),
            conn_number: fresh_number(i),
        };
        let err = gate.try_join(&req).expect_err("foreign build must refuse");
        assert_eq!(
            err,
            JoinRefuse::BuildMismatch {
                got: connect_hash(&foreign),
                want: connect_hash(SERVER_BUILD),
            },
            "refuse reason must name the mismatched build"
        );
        refused += 1;
    }
    assert_eq!(refused, 20, "mismatched-build refuse 20 of 20");
    assert_eq!(gate.snapshot(), before, "refuse must not mutate state");
}

#[test]
fn stale_conn_refuses_20_of_20() {
    let mut gate = MatchGate::new();
    let before = gate.snapshot();
    let mut refused = 0_u32;
    for i in 0..20_usize {
        let stale = u32::try_from(i % 4).expect("stale slot fits u32");
        assert!(
            stale < MIN_FRESH_CONNECTION_NUMBER,
            "case must stay in the stale range"
        );
        let req = ConnectRequest {
            build_hash: connect_hash(SERVER_BUILD),
            channel: channel_digest(),
            conn_number: stale,
        };
        let err = gate.try_join(&req).expect_err("stale conn must refuse");
        assert_eq!(err, JoinRefuse::StaleConn { got: stale });
        refused += 1;
    }
    assert_eq!(refused, 20, "stale-conn refuse 20 of 20");
    assert_eq!(gate.snapshot(), before, "refuse must not mutate state");
}

#[test]
fn legit_join_accepts_20_of_20_and_refuse_leaves_state_unchanged() {
    let mut gate = MatchGate::new();
    let mut accepted = 0_u32;
    for i in 0..20_usize {
        let id = gate
            .try_join(&ConnectRequest::legit(fresh_number(i)))
            .expect("legit join must accept");
        assert_eq!(id, accepted, "member ids assign in order");
        accepted += 1;
    }
    assert_eq!(accepted, 20, "legit join 20 of 20");
    assert_eq!(gate.snapshot().0, 20, "roster holds 20 members");
    let mut replay = MatchGate::new();
    for i in 0..20_usize {
        replay
            .try_join(&ConnectRequest::legit(fresh_number(i)))
            .expect("replay must accept");
    }
    assert_eq!(
        replay.snapshot(),
        gate.snapshot(),
        "legit tape replays identically"
    );
    let pinned = gate.snapshot();
    let bad_build = ConnectRequest {
        build_hash: connect_hash("foreign-build/pinned"),
        channel: channel_digest(),
        conn_number: fresh_number(20),
    };
    assert!(gate.try_join(&bad_build).is_err(), "bad build refuses");
    let stale = ConnectRequest {
        build_hash: connect_hash(SERVER_BUILD),
        channel: channel_digest(),
        conn_number: 0,
    };
    assert!(gate.try_join(&stale).is_err(), "stale conn refuses");
    let bad_channel = ConnectRequest {
        build_hash: connect_hash(SERVER_BUILD),
        channel: channel_digest().wrapping_add(1),
        conn_number: fresh_number(21),
    };
    assert!(gate.try_join(&bad_channel).is_err(), "bad channel refuses");
    assert_eq!(gate.snapshot(), pinned, "state unchanged on refuse");
}

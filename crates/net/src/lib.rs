//! net: rollback ring + repeat-last predict + input-delay buffer + snapshot delta over UDP/QUIC/relay transports (UdpTransport/RelayTransport/SimQuicNet/QuinnEndpoint).
//!
//! # One session per use-case
//!
//! One trait covers a two-player sim: [`rollback::SimTick`] (`advance` plus
//! `state_hash`; re-exported under its old name as `netcode::Sim`). Pick the
//! session by the job:
//!
//! - Shipped online matches with server authority and native or browser
//!   clients: [`session::Server`] and [`session::Client`] over the snapshot
//!   delta wire. This is the production path.
//! - Two-player peer-to-peer tape with a 128-frame storage window and two
//!   input queues: [`rollback::P2pSession`] over [`rollback::SimTick`].
//! - Compact reference session with the 9-frame WAN authority window and a
//!   bounded hash log: [`netcode::Session`] over the same
//!   [`rollback::SimTick`]; the ggrs and sync-vector tests drive it.
//! - N-seat predicting client (the four-seat proof): [`rollback::SeatSession`]
//!   over [`rollback::SeatSim`].
//! - Per-frame hash ring only, no session: [`rollback::FrameLog`].
//! - Prediction and desync probes without a session: [`prediction`].

/// Packet delivery tracking: a 32-bit ack bitfield, a fixed ring of sent
/// records and a loss rate in permille. Std-only; feeds receipts, never the sim.
pub mod ack;
/// The dedicated server's sanitize rule (a spear press in a match without
/// the spear is masked) and the bounded log naming every changed input.
pub mod anticheat;
/// Browser clients: the WebSocket and WebTransport legs of a page joining the
/// dedicated server, behind the same `Transport` trait (wasm32 glue plus the
/// natively tested inbox and stream framing).
pub mod browser;
/// Tick-budgeted pre-cooked map-cell streaming (district/siege cells).
pub mod cells;
/// Connect race: WebTransport against WebSocket, first success wins, a failure
/// names the blocked leg. Pure logic over futures.
pub mod connect_race;
/// Real-UDP link between the two machines of a split bout: lockstep slot
/// exchange with resends, relay registration, STUN echo, hole punch, probes.
pub mod courier;
pub mod dedicated;
pub mod delay;
pub mod delta;
/// Input admission screen of the dedicated server: per-slot refusal
/// receipts, sanitized admission, lag receipts of attack presses.
pub mod guard;
/// The session connect handshake, modeled on the netcode.io 1.02 shape: a
/// sealed connect token, a challenge and a response, both ends ending on the
/// same session keys. Std-only; the seal is HMAC-SHA256 under `--features
/// crypto` and a documented development-only fold without it (`TAG_LEN`).
pub mod handshake;
pub mod heal;
/// Interest management: relevancy, per-connection priority, budgets.
pub mod interest;
/// Presentation snapshot interpolation: Hermite C1 positions, quaternion
/// yaw dispatch, snapshot ring plus extrapolate guard. Render-time only.
pub mod interp;
/// Lag window of the dedicated server: how long a tick stays open for a
/// remote player's input, sized from round trip, capped at 16 ticks, with a
/// receipt per attack press that replays exactly.
pub mod lagcomp;
/// Seeded fault-injection wire for played bouts: loss, jitter, reorder,
/// scripted outage and measured delay histograms. Std-only.
pub mod link;
/// Lobby with beacon browser, ready-up and AI-fill slots: two
/// machines form a fight with no manual addresses. Std-only.
pub mod lobby;
pub mod netcode;
/// Online played bout: invite join, hole-punch decision and the bout over QUIC
/// through the relay. Feature-gated with the QUIC transport.
#[cfg(feature = "quic")]
pub mod online;
/// The played bout: lobby-formed fight, dedicated server, predicting clients,
/// scripted mid-fight drop and rejoin over a [`link::Wire`].
pub mod played;
/// Input prediction plus checksum desync detection: the `InputPredictor`
/// trait, per-seat guess tracking with streaks, and a probe that names the
/// first divergent tick. Std-only, integer-only, fixed rings.
pub mod prediction;
/// The `Transport` trait, the frame types and the std-only transports (UDP,
/// relay, simulated QUIC). The quinn backend inside is feature-gated (`quic`);
/// the headless default stays std-only UDP. The module compiles for the
/// browser, so the browser clients can implement the same trait.
pub mod quic;
/// Posted evidence of the played proofs as JSON receipts. Std-only.
pub mod receipt;
/// Internet-join relay: self-hostable relay binary core, STUN/ICE-lite
/// hole punch with relay fallback, invite codes. Std-only; certs test-only.
pub mod relay;
/// STUN binding integrity, ICE candidates, pair nomination and consent
/// freshness: the conformance half of [`relay`], split to keep one owner file
/// bounded. Re-exported from `relay` unchanged.
pub mod relay_ice;
/// Replays with keyframes plus theater playback and divergence reports:
/// record any input tape, frame-step it, attribute seeded bugs.
pub mod replay;
pub mod rollback;
/// 8-16 fighter hardening: bandwidth caps, priority deltas, soak
/// proof. Std-only; loopback only.
pub mod scale;
pub mod session;
pub mod snapshot;
/// One played bout split across two machines over real UDP: the split wire,
/// the session drivers on both machines, the relay rendezvous.
pub mod split;
pub mod tick_budget;
/// Authenticated-transport audit: netcode-style token auth against TLS,
/// measured side by side with real handshake bytes and CPU, no dependency.
pub mod transport_audit;
/// The shared sorted-and-unique gate for key runs (input tapes, interest
/// rosters, snapshot slots, delta rows): one definition, caller-owned errors.
pub mod validate;
/// Datagram discipline for the WAN replication stream: one datagram per
/// class-ordered snapshot, over-cap batches refused and counted, far and idle
/// poses shed first, inputs and combat redundant and never expired.
pub mod wan_discipline;
pub mod wan_soak;
/// The scripted duel a page plays against the native dedicated server: the
/// host and client state machines over the `Transport` trait, the spec and the
/// script both ends share.
pub mod webduel;
/// WebSocket transport, the fallback for browsers on networks that drop UDP:
/// the wire layout (always) and the native server (feature `websocket`).
pub mod websocket;
/// WebTransport server transport for browser clients: datagrams carry the
/// snapshots, one bidirectional stream carries reliable events, and the
/// pinned self-signed certificate helper feeds `serverCertificateHashes`.
/// Feature-gated (`webtransport`); the headless default stays std-only.
#[cfg(feature = "webtransport")]
pub mod webtransport;

/// Test-binary allocator: lets `engine_core::alloc_probe::measure` count heap
/// calls on the measuring thread only, so parallel tests never interfere.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

pub use dedicated::{
    ms_to_ticks, ticks_to_ms, Beacon, HeadlessServer, ServerError, BEACON_LEN, BEACON_MAGIC,
    BEACON_VERSION, MAX_LATENCY_MS,
};
pub use heal::{
    DesyncPlan, NetHeal, NetHealError, QuarantineNotice, ReconnectPlan, HEAL_RING_CAP, MAX_NOTICES,
};
pub use session::{
    decode_delta, encode_delta, Client, Delta, FighterDelta, FighterSnap, FullSnap, InterpResult,
    Interpolator, NetError, Server, SnapshotDelta,
};

/// Owner-swap pins: every net fold routes through `engine-core` hashing, and
/// each digest below was recorded before the swap. A fold that drifts (byte
/// order, field order, owner divergence) fails here instead of silently
/// forking tapes, journals and mirrors.
#[cfg(test)]
mod hash_owner_equiv {
    use crate::replay::{Keyframe, ReplayTape, REPLAY_VERSION};
    use crate::rollback::FrameLog;
    use engine_arena::{ArenaConfig, Checkpoint};
    use engine_core::hash_bytes;
    use engine_input::fighter::{Buttons, FighterId, FighterInput};

    fn tape_input(tick: u32, fighter: u8, mx: i8, mz: i8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(fighter).expect("slot"),
            mx,
            mz,
            facing: 9000,
            buttons: Buttons::NONE,
        }
    }

    #[test]
    fn published_fnv_vectors_hold_through_owner() {
        assert_eq!(hash_bytes(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash_bytes(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn framelog_fold_matches_pre_swap_digest() {
        let mut log = FrameLog::new(8);
        log.save(0x0123_4567_89AB_CDEF, 0x9ABC_DEF0);
        log.save(7, 1);
        assert_eq!(log.fold(), 0xdf38_556d_e488_f973);
    }

    #[test]
    fn replay_tape_hash_matches_pre_swap_digest() {
        let config = ArenaConfig {
            seed: 7,
            ticks: 3,
            fighters: 2,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        };
        // The header also pins the world identity and the tick logic version;
        // `tape_hash` folds version, config, inputs and checkpoints only, so
        // the pre-swap digest below is unchanged by the header growth.
        let tape = ReplayTape {
            version: REPLAY_VERSION,
            config,
            spec_digest: engine_arena::MatchSpec::v1(config).identity(),
            logic_version: crate::replay::SIM_LOGIC_VERSION,
            inputs: vec![tape_input(0, 0, 100, -50), tape_input(1, 1, -100, 50)],
            checkpoints: vec![
                Checkpoint {
                    tick: 0,
                    hash: 0x1111_2222_3333_4444,
                },
                Checkpoint {
                    tick: 1,
                    hash: 0x5555_6666_7777_8888,
                },
                Checkpoint {
                    tick: 2,
                    hash: 0x9999_AAAA_BBBB_CCCC,
                },
            ],
            keyframes: vec![Keyframe {
                tick: 0,
                hash: 0x1111_2222_3333_4444,
            }],
        };
        assert_eq!(tape.tape_hash(), 0x3092_6e49_6049_04ce);
    }
}

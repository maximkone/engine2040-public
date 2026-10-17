//! net: rollback ring + repeat-last predict + input-delay buffer + snapshot delta over UDP/QUIC/relay transports (UdpTransport/RelayTransport/SimQuicNet/QuinnEndpoint).

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
pub mod heal;
/// Interest management: relevancy, per-connection priority, budgets.
pub mod interest;
/// Presentation snapshot interpolation: Hermite C1 positions, quaternion
/// yaw dispatch, snapshot ring plus extrapolate guard. Render-time only.
pub mod interp;
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

/// Bounded history of state hashes and inputs for resimulation; the oldest
/// frame drops once `cap` frames are held.
pub struct Rollback {
    states: std::collections::VecDeque<u64>,
    inputs: std::collections::VecDeque<u32>,
    cap: usize,
}

impl Rollback {
    /// Empty history keeping at most `cap` frames.
    pub fn new(cap: usize) -> Self {
        Self {
            states: std::collections::VecDeque::new(),
            inputs: std::collections::VecDeque::new(),
            cap,
        }
    }
    /// advance one frame; remote input predicted = last input (stub records only)
    pub fn advance(&mut self, state_hash: u64, input: u32) {
        self.states.push_back(state_hash);
        self.inputs.push_back(input);
        if self.states.len() > self.cap {
            self.states.pop_front();
            self.inputs.pop_front();
        }
    }
    /// re-fetch a saved frame for resimulation
    pub fn resim_from(&self, frame: usize) -> Option<u64> {
        self.states.get(frame).copied()
    }
    /// Frames currently held.
    pub fn len(&self) -> usize {
        self.states.len()
    }
    /// True when no frame is held.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_cap_respected() {
        let mut r = Rollback::new(3);
        for i in 0..5u64 {
            r.advance(i, i as u32);
        }
        assert_eq!(r.len(), 3);
        assert_eq!(
            (r.resim_from(0), r.resim_from(2), r.resim_from(3)),
            (Some(2), Some(4), None)
        );
    }

    #[test]
    fn resim_from_returns_frames() {
        let mut r = Rollback::new(8);
        assert_eq!(r.resim_from(0), None);
        r.advance(0xAAAA, 1);
        r.advance(0xBBBB, 2);
        assert_eq!(
            (r.resim_from(0), r.resim_from(1), r.resim_from(2)),
            (Some(0xAAAA), Some(0xBBBB), None)
        );
    }

    #[test]
    fn advance_deterministic() {
        let seq = [(10u64, 1u32), (20, 2), (30, 3)];
        let (mut a, mut b) = (Rollback::new(8), Rollback::new(8));
        for (h, inp) in seq {
            a.advance(h, inp);
            b.advance(h, inp);
        }
        for (i, (h, _)) in seq.iter().enumerate() {
            assert_eq!((a.resim_from(i), b.resim_from(i)), (Some(*h), Some(*h)));
        }
    }
}

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
        let tape = ReplayTape {
            version: REPLAY_VERSION,
            config: ArenaConfig {
                seed: 7,
                ticks: 3,
                fighters: 2,
                half_extent_m: 20.0,
                // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
                table_version: 1,
            },
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

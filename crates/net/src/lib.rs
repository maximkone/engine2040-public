//! net: rollback ring + repeat-last predict + input-delay buffer + snapshot delta over UDP/QUIC/relay transports (UdpTransport/RelayTransport/SimQuicNet/QuinnEndpoint).

pub mod anticheat;
/// Tick-budgeted pre-cooked map-cell streaming (district/siege cells).
pub mod cells;
/// Real-UDP link between the two machines of a split bout: lockstep slot
/// exchange with resends, relay registration, STUN echo, hole punch, probes.
pub mod courier;
pub mod dedicated;
pub mod delay;
pub mod delta;
/// Server-authority cheat screen: rate plus range checks, movement sanity,
/// lag-compensated hits only, replay-backed refusal reports.
pub mod guard;
pub mod heal;
/// Interest management: relevancy, per-connection priority, budgets.
pub mod interest;
/// Presentation snapshot interpolation: Hermite C1 positions, quaternion
/// yaw dispatch, snapshot ring plus extrapolate guard. Render-time only.
pub mod interp;
/// Melee lag compensation: bounded attacker-view window, recorded
/// per hit with window parameters in every receipt so replays stay exact.
pub mod lagcomp;
/// Seeded fault-injection wire for played bouts: loss, jitter, reorder,
/// scripted outage and measured delay histograms. Std-only.
pub mod link;
/// Lobby with beacon browser, ready-up and AI-fill slots: two
/// machines form a fight with no manual addresses. Std-only.
pub mod lobby;
/// NAT traversal slice for QUIC through the invite-code relay: invite legs,
/// the join handshake hook and the road decision. Std-only.
pub mod nat;
pub mod netcode;
/// Online played bout: invite join, hole-punch decision and the bout over QUIC
/// through the relay. Feature-gated with the QUIC transport.
#[cfg(feature = "quic")]
pub mod online;
/// The played bout: lobby-formed fight, dedicated server, predicting clients,
/// scripted mid-fight drop and rejoin over a [`link::Wire`].
pub mod played;
/// QUIC transport: quinn datagrams + streams behind Transport.
/// Feature-gated (`quic`); the headless default stays std-only UDP.
#[cfg(feature = "quic")]
pub mod quic;
/// Posted evidence of the played proofs as JSON receipts. Std-only.
pub mod receipt;
/// Internet-join relay: self-hostable relay binary core, STUN/ICE-lite
/// hole punch with relay fallback, invite codes. Std-only; certs test-only.
pub mod relay;
/// Replays with keyframes plus theater playback and divergence reports:
/// record any input tape, frame-step it, attribute seeded bugs.
pub mod replay;
pub mod replicate;
pub mod resim;
pub mod rollback;
/// 8-16 fighter hardening: bandwidth caps, priority deltas, soak
/// proof. Std-only; loopback only.
pub mod scale;
pub mod server;
pub mod session;
pub mod snapshot;
/// One played bout split across two machines over real UDP: the split wire,
/// the session drivers on both machines, the relay rendezvous.
pub mod split;
/// Steam lobbies and relay transport beside QUIC and the invite-code relay.
pub mod steam_relay;
pub mod tick_budget;
pub mod wan_soak;

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

/// Lockstep input-delay buffer: send instantly, render after D ticks.
/// Bounded: rejects out-of-window peer ticks (desync guard).
pub struct DelayBuffer {
    /// Ticks between sending an input and rendering it.
    pub delay: usize,
    buf: std::collections::VecDeque<u32>,
}

impl DelayBuffer {
    /// Empty buffer that releases inputs `delay` ticks late.
    pub fn new(delay: usize) -> Self {
        Self {
            delay,
            buf: std::collections::VecDeque::new(),
        }
    }
    /// Record this tick's local input.
    pub fn push_local(&mut self, input: u32) {
        self.buf.push_back(input);
    }
    /// readable input D ticks ago; neutral 0 during warmup.
    pub fn readable(&self) -> u32 {
        if self.buf.len() > self.delay {
            self.buf[self.buf.len() - 1 - self.delay]
        } else {
            0
        }
    }
}

/// Bounded history of state hashes and inputs for resimulation; the oldest
/// frame drops once `cap` frames are held.
pub struct Rollback {
    states: Vec<u64>,
    inputs: Vec<u32>,
    cap: usize,
}

impl Rollback {
    /// Empty history keeping at most `cap` frames.
    pub fn new(cap: usize) -> Self {
        Self {
            states: vec![],
            inputs: vec![],
            cap,
        }
    }
    /// advance one frame; remote input predicted = last input (stub records only)
    pub fn advance(&mut self, state_hash: u64, input: u32) {
        self.states.push(state_hash);
        self.inputs.push(input);
        if self.states.len() > self.cap {
            self.states.remove(0);
            self.inputs.remove(0);
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
    use crate::anticheat::{leaf_hash, merkle_root};
    use crate::lagcomp::{HitClaim, LagTape, LagWindow};
    use crate::replay::{Keyframe, ReplayTape, REPLAY_VERSION};
    use crate::replicate::{EntityId, ReplicationServer, Transform, Vitals};
    use crate::rollback::FrameLog;
    use crate::server::Server;
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
    fn server_hash_matches_pre_swap_digest() {
        let mut server = Server::with_seed(7);
        let a = server.join().expect("join");
        let b = server.join().expect("join");
        server.tick_with(&[(a, 0b0101), (b, 0b1010)]);
        server.tick_with(&[(a, 0b0001), (b, 0b1000)]);
        server.tick_with(&[(a, 0b1111), (b, 0b0000)]);
        assert_eq!(server.state_hash(), 0x8bf1_8bcf_9962_143e);
    }

    #[test]
    fn lag_tape_hash_matches_pre_swap_digest() {
        let mut tape = LagTape::new(2).expect("tape");
        for (server_tick, view_tick) in [(10, 8), (11, 10), (12, 12)] {
            tape.record(
                HitClaim {
                    attacker: 0,
                    target: 1,
                    server_tick,
                    view_tick,
                },
                LagWindow::FULL,
            )
            .expect("record");
        }
        assert_eq!(tape.hash(), 0xc027_b16d_a143_4ae0);
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

    #[test]
    fn replication_world_hash_matches_pre_swap_digest() {
        let mut world = ReplicationServer::new();
        world
            .spawn(
                EntityId(3),
                Transform {
                    x_mm: -2500,
                    y_mm: 1300,
                    yaw_ddeg: -900,
                },
                Vitals {
                    hp: 1000,
                    stamina: 100,
                },
            )
            .expect("spawn");
        world
            .spawn(
                EntityId(1),
                Transform {
                    x_mm: 40_000,
                    y_mm: -7,
                    yaw_ddeg: 1799,
                },
                Vitals::FULL,
            )
            .expect("spawn");
        assert_eq!(world.world_hash(), 0x742f_66ef_adbe_797a);
    }

    #[test]
    fn anticheat_leaf_and_merkle_match_pre_swap_digests() {
        let leaves = [
            leaf_hash(9, 0xA5A5, 0x0123_4567_89AB_CDEF),
            leaf_hash(10, 7, 1),
        ];
        assert_eq!(leaves[0], 0x6cc2_f477_8016_a75f);
        assert_eq!(merkle_root(&leaves), 0x6f27_df39_43eb_8a92);
    }
}

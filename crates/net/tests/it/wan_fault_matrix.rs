//! WAN fault matrix over the rollback session: loss/latency cells converge.
//!
//! Donor: gschup/ggrs `src/sync_layer.rs` checksum-gated save/load plus
//! sparse snapshots, MIT (licence verified), pattern only.
//! Link: https://github.com/gschup/ggrs
//! Fresh implementation over `engine_net::session` (`Server`/`Client`);
//! no donor bytes. The checksum gate is `on_server_state` refusing a
//! checkpoint/delta pair that disagrees; the sparse snapshots are the
//! every-8th authoritative checkpoints re-checked after the final resync.
//!
//! Consumer: MP6 checkpoint equality under WAN faults. Units: fixed ticks,
//! loss percent, latency ticks (3 ticks == 50 ms at 60 Hz). Determinism:
//! seeded integer draws only, no wall clock, no map order in hashed state.

use std::collections::{BTreeMap, VecDeque};

use engine_arena::{ArenaConfig, Checkpoint};
use engine_input::fighter::{Buttons, ATTACK};
use engine_input::{FighterId, FighterInput};
use engine_net::session::{Client, NetError, Server, SnapshotDelta, WIRE_VERSION};

/// Fixed ticks per matrix cell; small so the 9-cell matrix stays fast.
const TICKS: u32 = 48;
/// Loss cells in percent per datagram per direction.
const LOSSES: [u32; 3] = [0, 10, 25];
/// Latency cells in ticks per direction (3 ticks == 50 ms at 60 Hz).
const LATENCIES: [u32; 3] = [0, 3, 6];
/// Sparse-snapshot cadence in ticks (GGRS-style sparse save pattern).
const SPARSE_EVERY: u32 = 8;
/// Base seed; each cell mixes in its loss/latency so cells differ.
const CELL_SEED: u64 = 0x0066_7273_204D_5036;

/// Deterministic local wish: alternates drive, attacks every 12th tick.
fn local_input(tick: u32) -> FighterInput {
    FighterInput {
        tick,
        fighter: FighterId::new(0).expect("slot 0"),
        mx: if tick % 20 < 10 { 100 } else { -100 },
        mz: 0,
        facing: 0,
        buttons: if tick.is_multiple_of(12) {
            Buttons(ATTACK)
        } else {
            Buttons::NONE
        },
    }
}

/// Deterministic xorshift64 draw stream; `state` must be nonzero.
fn next_rand(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Per-cell delivery ledger for the printed loss/latency distributions.
struct CellReport {
    loss_pct: u32,
    latency: u32,
    up_sent: u64,
    up_dropped: u64,
    down_sent: u64,
    down_dropped: u64,
    applied: u64,
    mispredicts: u64,
}

/// Run one matrix cell through in-memory fault links in both directions.
///
/// Client predictions travel up with `latency` ticks of delay and `loss_pct`
/// drops; server states travel down the same way. The server authors a tick
/// once its inputs clear the latency floor; missing inputs stay AI-filled.
/// Ends with one reliable sparse snapshot (the final authoritative state)
/// applied checksum-gated, then a resync. Returns server checkpoints, client
/// checkpoints, the sparse samples and the ledger.
fn run_cell(
    loss_pct: u32,
    latency: u32,
    seed: u64,
) -> (
    Vec<Checkpoint>,
    Vec<Checkpoint>,
    Vec<Checkpoint>,
    CellReport,
) {
    let config = ArenaConfig {
        seed: 7,
        ticks: TICKS,
        fighters: 2,
        half_extent_m: 20.0,
        // engine_combat::tables::TABLES_VERSION_V1 (value 1); not reachable from engine-net tests.
        table_version: 1,
    };
    let mut server = Server::new(config).expect("server config valid");
    let mut client =
        Client::new(config, FighterId::new(0).expect("slot 0")).expect("client config valid");
    let mut up: VecDeque<(u32, FighterInput)> = VecDeque::new();
    let mut down: VecDeque<(Checkpoint, SnapshotDelta)> = VecDeque::new();
    let mut inbox: BTreeMap<(u32, u8), FighterInput> = BTreeMap::new();
    let mut authored: Vec<(Checkpoint, SnapshotDelta)> = Vec::new();
    let mut rng_up = (seed ^ 0xC10C) | 1;
    let mut rng_down = (seed ^ 0xD0ED) | 1;
    let mut report = CellReport {
        loss_pct,
        latency,
        up_sent: 0,
        up_dropped: 0,
        down_sent: 0,
        down_dropped: 0,
        applied: 0,
        mispredicts: 0,
    };
    let steps = TICKS + 2 * latency + 2;
    for step in 0..steps {
        if step < TICKS {
            let input = local_input(step);
            client.predict(input).expect("predictions stay in order");
            report.up_sent += 1;
            if next_rand(&mut rng_up) % 100 < u64::from(loss_pct) {
                report.up_dropped += 1;
            } else {
                up.push_back((step + latency, input));
            }
        }
        while up.front().is_some_and(|(at, _)| *at <= step) {
            let (_, input) = up.pop_front().expect("front checked");
            inbox.entry((input.tick, input.fighter.0)).or_insert(input);
        }
        while server.tick_now() < TICKS && server.tick_now() + latency <= step {
            let want = server.tick_now();
            let due: Vec<FighterInput> = inbox
                .iter()
                .filter(|(key, _)| key.0 == want)
                .map(|(_, input)| *input)
                .collect();
            let (checkpoint, delta) = server.tick(&due).expect("server authors in order");
            inbox.retain(|key, _| key.0 > want);
            authored.push((checkpoint, delta.clone()));
            report.down_sent += 1;
            if next_rand(&mut rng_down) % 100 < u64::from(loss_pct) {
                report.down_dropped += 1;
            } else {
                down.push_back((checkpoint, delta));
            }
        }
        while down
            .front()
            .is_some_and(|(cp, _)| cp.tick + latency <= step)
        {
            let (checkpoint, delta) = down.pop_front().expect("front checked");
            if client.on_server_state(checkpoint, &delta).is_ok() {
                report.applied += 1;
                report.mispredicts +=
                    u64::try_from(client.last_mispredicts().len()).expect("mispredicts fit u64");
                client.resimulate();
            }
        }
    }
    // Reliable sparse snapshot: the final authoritative state always lands
    // checksum-gated, modelling the GGRS sparse-save resync after a fault.
    let (last_cp, last_delta) = authored.last().expect("every tick authored").clone();
    let _ = client.on_server_state(last_cp, &last_delta);
    client.resimulate();
    let mut sparse = Vec::new();
    for (checkpoint, _) in &authored {
        if checkpoint.tick % SPARSE_EVERY == SPARSE_EVERY - 1 || checkpoint.tick + 1 == TICKS {
            sparse.push(*checkpoint);
        }
    }
    (
        server.checkpoints().to_vec(),
        client.checkpoints().to_vec(),
        sparse,
        report,
    )
}

/// Fresh loss/latency matrix: every cell holds full checkpoint equality.
#[test]
fn wan_fault_matrix_checkpoint_equality_per_cell() {
    let mut cells: u32 = 0;
    let mut total_dropped: u64 = 0;
    let mut total_applied: u64 = 0;
    for loss_pct in LOSSES {
        for latency in LATENCIES {
            let seed = CELL_SEED
                ^ (u64::from(loss_pct).wrapping_mul(7919))
                    .wrapping_add(u64::from(latency).wrapping_mul(104_729));
            let (server_cps, client_cps, sparse, report) = run_cell(loss_pct, latency, seed);
            assert_eq!(
                server_cps.len(),
                TICKS as usize,
                "server authors every tick"
            );
            assert_eq!(
                client_cps.len(),
                TICKS as usize,
                "client predicts every tick"
            );
            assert_eq!(
                client_cps, server_cps,
                "cell loss={loss_pct}% latency={latency} must hold full checkpoint equality"
            );
            for sample in &sparse {
                assert_eq!(
                    client_cps[sample.tick as usize], *sample,
                    "sparse snapshot tick {} must match in its cell",
                    sample.tick
                );
            }
            println!(
                "matrix cell loss={}% latency={}t up={}/{} down={}/{} applied={} mispredicts={}",
                report.loss_pct,
                report.latency,
                report.up_sent - report.up_dropped,
                report.up_sent,
                report.down_sent - report.down_dropped,
                report.down_sent,
                report.applied,
                report.mispredicts,
            );
            total_dropped += report.up_dropped + report.down_dropped;
            total_applied += report.applied;
            cells += 1;
        }
    }
    assert_eq!(cells, 9, "matrix holds 3 loss x 3 latency cells");
    assert!(total_dropped > 0, "the loss column must drop datagrams");
    assert!(total_applied > 0, "the client must apply states");
    println!(
        "wan_fault_matrix passed cells={cells} total_dropped={total_dropped} applied={total_applied}"
    );
}

/// Checksum gate: a forged checkpoint/delta or version refuses and holds.
#[test]
fn wan_fault_matrix_checksum_gate_refuses_forgery() {
    let config = ArenaConfig {
        seed: 7,
        ticks: 8,
        fighters: 2,
        half_extent_m: 20.0,
        // engine_combat::tables::TABLES_VERSION_V1 (value 1); not reachable from engine-net tests.
        table_version: 1,
    };
    let mut server = Server::new(config).expect("server config valid");
    let mut client =
        Client::new(config, FighterId::new(0).expect("slot 0")).expect("client config valid");
    let input = local_input(0);
    client.predict(input).expect("predict");
    let (truth, delta) = server.tick(&[input]).expect("server tick");
    // Forged hash: the checksum gate refuses, nothing changes.
    let forged = Checkpoint {
        tick: truth.tick,
        hash: truth.hash.wrapping_add(1),
    };
    assert_eq!(
        client.on_server_state(forged, &delta),
        Err(NetError::BadState),
        "forged checksum must refuse"
    );
    assert_eq!(client.server_tick(), None, "refusal changes nothing");
    // Foreign version refuses the same way.
    let mut future = delta.clone();
    future.version = WIRE_VERSION + 1;
    assert_eq!(
        client.on_server_state(truth, &future),
        Err(NetError::VersionMismatch {
            got: WIRE_VERSION + 1,
        }),
        "future version must refuse"
    );
    assert_eq!(
        client.server_tick(),
        None,
        "version refusal changes nothing"
    );
    // True state applies and resimulates to the checkpoint.
    let named = client
        .on_server_state(truth, &delta)
        .expect("truth applies");
    assert!(named.is_empty(), "matching wish names no mispredict");
    let converged = client.resimulate();
    assert_eq!(converged[0], truth, "resim holds the checkpoint");
    println!("wan_fault_matrix checksum gate passed forged=2 applied=1");
}

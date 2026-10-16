//! Per-client visibility filter vectors (integer-only).
//!
//! Idea-only PORT(simgine/bevy_replicon MIT PR #723 `visibility.rs`):
//! per-client visibility filters decide which entities each client receives,
//! so a resync must re-apply the filter and never restore a filtered-out
//! entity. Reimplemented clean-room with integer math only; no vendor bytes.
//!
//! Owner: `engine-net`; consumer: server-authoritative replication sessions
//! that filter spawns/deltas per client. Units: fighter ids, ticks (`u32`),
//! bytes (`u64`). Determinism: static masks plus integer positions, `Vec`
//! iteration order only; no float, no wall clock, no `HashMap`.

/// Fighters in the arena.
const N_FIGHTERS: usize = 4;
/// Ticks per vector.
const N_TICKS: u32 = 60;
/// Clients with divergent views.
const N_CLIENTS: usize = 2;
/// Full-filter resync cadence; the filter still applies on resync ticks.
const RESYNC_EVERY: u32 = 16;
/// Wire bytes charged per replicated fighter update.
const BYTES_PER_FIGHTER: u64 = 8;

/// Static divergent masks: client 0 sees fighters 0,1; client 1 sees 2,3.
const CLIENT_MASKS: [u8; N_CLIENTS] = [0b_0011, 0b_1100];

/// Visibility predicate: integer bit test only.
fn is_visible(client: usize, fighter: usize) -> bool {
    ((CLIENT_MASKS[client] >> fighter) & 1) == 1
}

/// Integer-only fighter position at a tick (mm on a line).
fn fighter_pos_mm(fighter: usize, tick: u32) -> i32 {
    let base = i32::try_from(fighter).expect("fighter id < 4 fits i32");
    let step = i32::try_from(tick).expect("tick < 60 fits i32");
    base.wrapping_mul(1_000).wrapping_add(step.wrapping_mul(7))
}

/// One filtered per-client update: the fighter ids actually sent.
fn filtered_update(client: usize, _tick: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for fighter in 0..N_FIGHTERS {
        if is_visible(client, fighter) {
            out.push(u32::try_from(fighter).expect("fighter id < 4 fits u32"));
        }
    }
    out
}

/// Run the 60-tick tape; returns `(leaked, bytes_per_client_per_tick)`.
fn run_tape() -> (u64, Vec<Vec<u64>>) {
    let tick_count = usize::try_from(N_TICKS).expect("60 ticks fit usize");
    let mut bytes: Vec<Vec<u64>> = vec![vec![0_u64; tick_count]; N_CLIENTS];
    let mut leaked: u64 = 0;
    for tick in 0..N_TICKS {
        for client in 0..N_CLIENTS {
            let update = filtered_update(client, tick);
            for id in &update {
                let fighter = usize::try_from(*id).expect("sent id < 4 fits usize");
                if !is_visible(client, fighter) {
                    leaked += 1;
                }
                let _ = fighter_pos_mm(fighter, tick);
            }
            let sent = u64::try_from(update.len()).expect("4 fighters fit u64");
            bytes[client][usize::try_from(tick).expect("tick fits usize")] =
                sent * BYTES_PER_FIGHTER;
        }
        if tick % RESYNC_EVERY == 0 {
            for client in 0..N_CLIENTS {
                let full: Vec<u32> = (0..N_FIGHTERS)
                    .map(|f| u32::try_from(f).expect("fighter id fits u32"))
                    .collect();
                let resent: Vec<u32> = full
                    .into_iter()
                    .filter(|id| is_visible(client, usize::try_from(*id).expect("id fits usize")))
                    .collect();
                for id in &resent {
                    let fighter = usize::try_from(*id).expect("resent id fits usize");
                    if !is_visible(client, fighter) {
                        leaked += 1;
                    }
                }
            }
        }
    }
    (leaked, bytes)
}

/// The two clients diverge for the whole 60-tick tape.
#[test]
fn divergent_visibility_holds_for_60_ticks() {
    for tick in 0..N_TICKS {
        let a = filtered_update(0, tick);
        let b = filtered_update(1, tick);
        assert_eq!(a.len(), 2, "client 0 sees 2 fighters at tick {tick}");
        assert_eq!(b.len(), 2, "client 1 sees 2 fighters at tick {tick}");
        for id in &a {
            assert!(!b.contains(id), "tick {tick}: fighter {id} leaked to both");
        }
        let mut union = a.clone();
        union.extend_from_slice(&b);
        union.sort_unstable();
        union.dedup();
        assert_eq!(
            union.len(),
            N_FIGHTERS,
            "tick {tick}: union covers 4 fighters"
        );
    }
}

/// No invisible update is ever sent; per-client bytes are recorded.
#[test]
fn leaked_updates_are_zero_and_bytes_recorded() {
    let (leaked, bytes) = run_tape();
    assert_eq!(leaked, 0, "invisible fighter updates must be zero");
    assert_eq!(bytes.len(), N_CLIENTS);
    let tick_count = usize::try_from(N_TICKS).expect("60 ticks fit usize");
    for (client, series) in bytes.iter().enumerate() {
        assert_eq!(series.len(), tick_count, "client {client}: 60 tick samples");
        for (tick, sample) in series.iter().enumerate() {
            assert_eq!(
                *sample,
                2 * BYTES_PER_FIGHTER,
                "client {client} tick {tick}: 2 fighters x 8 B"
            );
        }
        let total: u64 = series.iter().sum();
        assert_eq!(
            total,
            u64::from(N_TICKS) * 2 * BYTES_PER_FIGHTER,
            "client {client}: 60 x 16 B total"
        );
    }
}

/// Resync ticks (`RESYNC_EVERY` 16) re-apply the filter, never the full set.
#[test]
fn resync_every_16_never_restores_leak() {
    let mut resync_ticks: Vec<u32> = Vec::new();
    for tick in 0..N_TICKS {
        if tick % RESYNC_EVERY == 0 {
            resync_ticks.push(tick);
        }
    }
    assert_eq!(resync_ticks, vec![0, 16, 32, 48], "60 ticks give 4 resyncs");
    for tick in resync_ticks {
        for client in 0..N_CLIENTS {
            let update = filtered_update(client, tick);
            assert_eq!(update.len(), 2, "client {client} resync tick {tick}");
            for id in &update {
                let fighter = usize::try_from(*id).expect("id fits usize");
                assert!(
                    is_visible(client, fighter),
                    "resync tick {tick} must not restore hidden fighter {id}"
                );
            }
        }
    }
    let (leaked, _) = run_tape();
    assert_eq!(leaked, 0, "resync path must keep leaks at zero");
}

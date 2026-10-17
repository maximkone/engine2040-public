//! net_diverge: pose-hash rings that name the first divergent tick and fighter.
//!
//! Consumer: a netplay soak or a desync report keeps one [`PoseRings`] per
//! client, records every fighter's [`NetPose`] each tick, and calls
//! [`PoseRings::first_divergence`] against a peer. Owner: `engine-anim`,
//! beside `net_pose`; it reuses [`NetPose::checkpoint`] and adds no second
//! hash. Units: ticks at 60 Hz, one `u64` checkpoint per fighter per tick.
//! Diagnostics only: the rings read poses and never write one back, so no
//! simulation or replicated byte depends on them. Determinism: ticks and
//! fighters walk in ascending order, so the same two rings always name the
//! same first mismatch. Memory: a fixed [`RING_TICKS`] window per fighter;
//! a divergence older than the window is evicted and reads as none.

use crate::net_pose::NetPose;

/// Ticks each fighter's ring remembers (about one second at 60 Hz).
pub const RING_TICKS: usize = 64;

/// One fighter's last [`RING_TICKS`] checkpoints, slot `tick % RING_TICKS`.
#[derive(Clone, Debug)]
pub struct PoseRing {
    slots: [Option<(u32, u64)>; RING_TICKS],
}

impl PoseRing {
    fn new() -> Self {
        Self {
            slots: [None; RING_TICKS],
        }
    }

    fn slot(tick: u32) -> usize {
        // Remainder is below RING_TICKS, so it always fits `usize`.
        usize::try_from(tick).map_or(0, |t| t % RING_TICKS)
    }

    fn record(&mut self, tick: u32, hash: u64) {
        self.slots[Self::slot(tick)] = Some((tick, hash));
    }

    /// Checkpoint stored for exactly `tick`, if still in the window.
    pub fn get(&self, tick: u32) -> Option<u64> {
        match self.slots[Self::slot(tick)] {
            Some((t, hash)) if t == tick => Some(hash),
            _ => None,
        }
    }
}

/// The first tick and fighter where two clients' checkpoints differ.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Divergence {
    /// Earliest differing tick in the window.
    pub tick: u32,
    /// Lowest differing fighter index on that tick.
    pub fighter: usize,
    /// Checkpoint on the asking side.
    pub mine: u64,
    /// Checkpoint on the peer.
    pub theirs: u64,
}

/// One client's rings, one per fighter.
#[derive(Clone, Debug)]
pub struct PoseRings {
    rings: Vec<PoseRing>,
}

impl PoseRings {
    /// Rings for `fighters` fighters, all empty.
    pub fn new(fighters: usize) -> Self {
        Self {
            rings: vec![PoseRing::new(); fighters],
        }
    }

    /// Record `pose` for `fighter` at `tick`. A fighter index past the set is
    /// ignored (diagnostics never fail a tick).
    pub fn record(&mut self, fighter: usize, tick: u32, pose: &NetPose) {
        if let Some(ring) = self.rings.get_mut(fighter) {
            ring.record(tick, pose.checkpoint(tick));
        }
    }

    /// Fighter count.
    pub fn fighters(&self) -> usize {
        self.rings.len()
    }

    /// Walk the last [`RING_TICKS`] ticks up to `newest_tick` and name the
    /// first tick (then lowest fighter) where both sides hold a checkpoint
    /// and the two differ. A tick one side lacks is skipped; sets of
    /// different sizes compare over the fighters they share.
    pub fn first_divergence(&self, peer: &Self, newest_tick: u32) -> Option<Divergence> {
        let window = u32::try_from(RING_TICKS - 1).unwrap_or(0);
        let first = newest_tick.saturating_sub(window);
        let shared = self.rings.len().min(peer.rings.len());
        for tick in first..=newest_tick {
            for fighter in 0..shared {
                let (mine, theirs) =
                    match (self.rings[fighter].get(tick), peer.rings[fighter].get(tick)) {
                        (Some(a), Some(b)) => (a, b),
                        _ => continue,
                    };
                if mine != theirs {
                    return Some(Divergence {
                        tick,
                        fighter,
                        mine,
                        theirs,
                    });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::CounterCursor;

    const CLIENTS: usize = 4;
    const FIGHTERS: usize = 8;
    const TICKS: u32 = 200;

    /// Seeded authority pick for `(fighter, tick)`: stable in the pair.
    fn pick(seed: u64, fighter: usize, tick: u32) -> NetPose {
        let mut c = CounterCursor::new(seed ^ ((fighter as u64) << 32 | u64::from(tick)));
        let w = c.next_u64();
        NetPose {
            clip: (w & 0xff) as u16,
            tick: ((w >> 8) & 0xff) as u16,
            phase_q: ((w >> 16) & 0xffff) as u16,
        }
    }

    /// Run the soak: every client records the authority stream, one client
    /// flips one fighter's phase on one tick; compare each tick against
    /// client 0 and return the first report plus the injection.
    fn soak(seed: u64) -> (Option<Divergence>, (usize, u32, usize)) {
        let mut c = CounterCursor::new(seed);
        let victim = 1 + (c.next_range(0, (CLIENTS - 1) as u64) as usize);
        let fighter = c.next_range(0, FIGHTERS as u64) as usize;
        let bad_tick = c.next_range(10, u64::from(TICKS) - 10) as u32;
        let mut rings: Vec<PoseRings> = (0..CLIENTS).map(|_| PoseRings::new(FIGHTERS)).collect();
        let mut first = None;
        'run: for tick in 0..TICKS {
            for (client, ring) in rings.iter_mut().enumerate() {
                for f in 0..FIGHTERS {
                    let mut pose = pick(seed, f, tick);
                    if client == victim && f == fighter && tick == bad_tick {
                        pose.phase_q ^= 1;
                    }
                    ring.record(f, tick, &pose);
                }
            }
            for peer in 1..CLIENTS {
                if let Some(d) = rings[0].first_divergence(&rings[peer], tick) {
                    first = Some(d);
                    break 'run;
                }
            }
        }
        (first, (victim, bad_tick, fighter))
    }

    #[test]
    fn net_diverge_soak_names_first_tick_and_fighter_10_of_10() {
        for seed in 1..=10u64 {
            let (found, (_, tick, fighter)) = soak(seed);
            let d = found.unwrap_or_else(|| panic!("seed {seed}: no divergence named"));
            assert_eq!((d.tick, d.fighter), (tick, fighter), "seed {seed}");
            assert_ne!(d.mine, d.theirs);
        }
    }

    #[test]
    fn net_diverge_soak_replays_exact_x2() {
        for seed in 1..=10u64 {
            assert_eq!(soak(seed), soak(seed), "seed {seed}");
        }
    }

    #[test]
    fn net_diverge_matching_clients_report_none() {
        let (mut a, mut b) = (PoseRings::new(3), PoseRings::new(3));
        for tick in 0..100 {
            for f in 0..3 {
                let p = pick(7, f, tick);
                a.record(f, tick, &p);
                b.record(f, tick, &p);
            }
        }
        assert_eq!(a.first_divergence(&b, 99), None);
    }

    #[test]
    fn net_diverge_lowest_fighter_wins_a_tied_tick() {
        let (mut a, mut b) = (PoseRings::new(4), PoseRings::new(4));
        for f in 0..4 {
            let p = pick(3, f, 5);
            a.record(f, 5, &p);
            let mut q = p;
            q.tick ^= u16::from(f == 1 || f == 3);
            b.record(f, 5, &q);
        }
        let d = a.first_divergence(&b, 5).expect("diverges");
        assert_eq!((d.tick, d.fighter), (5, 1));
    }

    #[test]
    fn net_diverge_evicted_ticks_are_forgotten() {
        let (mut a, mut b) = (PoseRings::new(1), PoseRings::new(1));
        a.record(0, 0, &pick(1, 0, 0));
        b.record(0, 0, &pick(2, 0, 0));
        assert!(a.first_divergence(&b, 0).is_some());
        let later = u32::try_from(RING_TICKS).expect("fits") + 10;
        for t in 1..=later {
            let p = pick(1, 0, t);
            a.record(0, t, &p);
            b.record(0, t, &p);
        }
        assert_eq!(
            a.first_divergence(&b, later),
            None,
            "tick 0 left the window"
        );
    }
}

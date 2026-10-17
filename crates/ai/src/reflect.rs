//! reflect: the fixed reflection cadence and the capped-surprise importance.
//! Owner: [`Mind::reflect_due`](crate::mind::Mind::reflect_due) and
//! [`MemoryStore::reflect_due`](crate::memory::MemoryStore::reflect_due) call
//! these two functions; the game calls `reflect_due` every tick.
//! Units: ticks (fixed 60 Hz steps), importance `0..=1000`.
//! Determinism: integer math only; the cadence is a pure function of the tick
//! and "already reflected" reads the stream, so no extra state can diverge.
//! Errors: none; both functions are total.
//! Idea source: generative agents (2304.03442), reflection on importance
//! accumulation; reimplemented as a fixed cadence, no donor code.

/// Ticks between reflections: every 10 s at 60 Hz. Units: ticks.
pub const REFLECT_EVERY: u64 = 600;
/// Most importance a surprise adds to a reflection. Units: salience points.
pub const SURPRISE_CAP: u32 = 200;

/// True on ticks `600, 1200, ...`; tick 0 never reflects.
pub const fn due(tick: u64) -> bool {
    tick != 0 && tick % REFLECT_EVERY == 0
}

/// Importance of a new `Felt`: `base` plus the surprise, how far the top
/// memory's salience `top` rose over the last `Felt`'s `last_felt`, capped at
/// [`SURPRISE_CAP`] and clamped to 1000. No earlier `Felt` means no surprise.
pub fn importance(base: u32, top: u32, last_felt: Option<u32>) -> u32 {
    let surprise = last_felt.map_or(0, |last| top.saturating_sub(last).min(SURPRISE_CAP));
    base.saturating_add(surprise).min(1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryStore, MAX_STORED_PER_NPC};
    use crate::mind::{MemoryKind, Mind, NpcId, MAX_EVENTS};

    #[test]
    fn reflect_cadence_is_every_600_ticks_never_tick_zero() {
        let due_ticks: Vec<u64> = (0..=1800).filter(|&t| due(t)).collect();
        assert_eq!(due_ticks, vec![600, 1200, 1800]);
    }

    #[test]
    fn reflect_surprise_is_capped_and_clamped() {
        assert_eq!(importance(300, 900, None), 300, "first reflection: none");
        assert_eq!(importance(300, 900, Some(100)), 500, "rise 800 caps at 200");
        assert_eq!(importance(300, 150, Some(400)), 300, "a fall adds nothing");
        assert_eq!(importance(300, 350, Some(300)), 350, "small rise adds it");
        assert_eq!(importance(950, 900, Some(0)), 1000, "clamped to 1000");
    }

    fn feed(mind: &mut Mind, tick: u64) {
        let salience = u32::try_from(tick * 7 % 900).unwrap() + 50;
        mind.observe(tick, MemoryKind::Seen, &format!("saw {tick}"), salience)
            .unwrap();
    }

    #[test]
    fn reflect_due_runs_once_per_cadence_tick_and_surprises() {
        let mut mind = Mind::new(NpcId(1), 9);
        assert_eq!(mind.reflect_due(600), Ok(false), "empty stream: nothing");
        feed(&mut mind, 10);
        assert_eq!(mind.reflect_due(599), Ok(false));
        assert_eq!(mind.reflect_due(600), Ok(true));
        assert_eq!(mind.reflect_due(600), Ok(false), "once per tick");
        let first = mind.events().last().unwrap().salience;
        mind.observe(1190, MemoryKind::Seen, "ambush", 1000)
            .unwrap();
        assert_eq!(mind.reflect_due(1200), Ok(true));
        let second = mind.events().last().unwrap();
        assert_eq!(second.kind, MemoryKind::Felt);
        assert!(second.salience > first, "the ambush surprised it");
        assert!(second.salience <= 1000);
    }

    #[test]
    fn reflect_ten_thousand_tick_soak_holds_caps_and_replays() {
        let run = || {
            let mut mind = Mind::new(NpcId(3), 1);
            let mut store = MemoryStore::new();
            for tick in 1..=10_000u64 {
                feed(&mut mind, tick);
                store
                    .observe(NpcId(3), tick, MemoryKind::Seen, "x", 400)
                    .unwrap();
                mind.reflect_due(tick).unwrap();
                store.reflect_due(NpcId(3), tick).unwrap();
                assert!(mind.events().len() <= MAX_EVENTS);
                assert!(store.len(NpcId(3)) <= MAX_STORED_PER_NPC);
            }
            let felt = mind.events().iter().filter(|e| e.kind == MemoryKind::Felt);
            let texts: Vec<_> = mind.events().iter().map(|e| (e.tick, e.salience)).collect();
            (felt.count(), texts, store.memory_bytes())
        };
        let (felts, a, bytes) = run();
        assert!(felts > 0);
        assert_eq!((felts, a, bytes), run(), "bytes x2");
    }
}

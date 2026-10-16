//! Weapon trail ring: the last 12 blade base-plus-tip pairs per slot.
//!
//! Consumer: `fighter_look::start_swing` pushes blade points for attacking
//! slots; the next slice's `present/**` strip reads `pairs` with
//! `alpha_for_age`. Data owner: this module owns the per-slot ring.
//! Units: meters for base and tip, ticks for age. Determinism: presentation
//! only, never hashed; same pushes give the same pairs twice. Errors: none;
//! a bad slot or a non-finite coordinate stores nothing.

/// Pairs kept per slot: one blade base plus one blade tip each.
pub const TRAIL_LEN: usize = 12;
/// Slots served; matches `fighter_look::MAX_LOOK_FIGHTERS`.
pub const MAX_TRAIL_SLOTS: usize = 8;

/// One blade sample: base and tip in meters plus age in ticks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrailPoint {
    /// Blade base in meters.
    pub base_m: [f32; 3],
    /// Blade tip in meters.
    pub tip_m: [f32; 3],
    /// Ticks since the push, `0` newest.
    pub age_ticks: u8,
}

/// Oldest age kept: one below [`TRAIL_LEN`].
const OLDEST_AGE: u8 = 11;

const EMPTY_POINT: TrailPoint = TrailPoint {
    base_m: [0.0; 3],
    tip_m: [0.0; 3],
    age_ticks: 0,
};

/// Ring of recent blade samples per slot. Fixed arrays: no heap after
/// construction; slot order is integer `0..MAX_TRAIL_SLOTS`.
#[derive(Clone, PartialEq, Debug)]
pub struct TrailRing {
    points: [[TrailPoint; TRAIL_LEN]; MAX_TRAIL_SLOTS],
    len: [usize; MAX_TRAIL_SLOTS],
}

impl Default for TrailRing {
    fn default() -> Self {
        Self::new()
    }
}

impl TrailRing {
    /// Empty ring: every slot idle.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            points: [[EMPTY_POINT; TRAIL_LEN]; MAX_TRAIL_SLOTS],
            len: [0; MAX_TRAIL_SLOTS],
        }
    }

    /// Push one base-plus-tip pair for `slot` at age 0; past [`TRAIL_LEN`]
    /// the oldest drops. A bad slot or a non-finite coordinate stores
    /// nothing. Zero heap after construction.
    pub fn push(&mut self, slot: usize, base_m: [f32; 3], tip_m: [f32; 3]) {
        if slot >= MAX_TRAIL_SLOTS || !base_m.iter().chain(tip_m.iter()).all(|v| v.is_finite()) {
            return;
        }
        let ring = &mut self.points[slot];
        let len = &mut self.len[slot];
        if *len == TRAIL_LEN {
            ring.rotate_left(1);
            *len -= 1;
        }
        ring[*len] = TrailPoint {
            base_m,
            tip_m,
            age_ticks: 0,
        };
        *len += 1;
    }

    /// Pairs for `slot`, oldest first; empty for a bad slot. Borrows the
    /// ring, so no heap. Caller: `fighter_look` (next slice's strip draw).
    #[must_use]
    pub fn pairs(&self, slot: usize) -> &[TrailPoint] {
        match (self.points.get(slot), self.len.get(slot)) {
            (Some(ring), Some(&len)) => &ring[..len],
            _ => &[],
        }
    }

    /// Drop every pair for `slot`; idle slots stay empty. Bad slots no-op.
    pub fn clear_idle(&mut self, slot: usize) {
        if slot >= MAX_TRAIL_SLOTS {
            return;
        }
        self.len[slot] = 0;
    }

    /// Age every pair by one tick; pairs at [`TRAIL_LEN`] ticks drop.
    pub fn age_tick(&mut self) {
        for (ring, len) in self.points.iter_mut().zip(self.len.iter_mut()) {
            let mut kept = 0;
            for read in 0..*len {
                let mut point = ring[read];
                point.age_ticks = point.age_ticks.saturating_add(1);
                if usize::from(point.age_ticks) < TRAIL_LEN {
                    ring[kept] = point;
                    kept += 1;
                }
            }
            *len = kept;
        }
    }
}

/// Opacity by age: 1 at age 0 down to 0 at age 11, 0 beyond. Pure.
#[must_use]
pub fn alpha_for_age(age_ticks: u8) -> f32 {
    (1.0 - f32::from(age_ticks) / f32::from(OLDEST_AGE)).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(step: usize) -> ([f32; 3], [f32; 3]) {
        let x = step as f32;
        ([x, 1.0, 0.0], [x, 1.0, 1.0])
    }

    #[test]
    fn trail_ring_holds_12_pairs_and_drops_idle() {
        let mut ring = TrailRing::new();
        assert!(ring.pairs(1).is_empty(), "idle slot holds none");
        for step in 0..12 {
            let (base_m, tip_m) = at(step);
            ring.push(0, base_m, tip_m);
        }
        let held = ring.pairs(0);
        assert_eq!(held.len(), TRAIL_LEN, "12 pushes hold 12");
        assert_eq!(held[0].base_m, at(0).0, "oldest first");
        assert_eq!(held[TRAIL_LEN - 1].tip_m, at(11).1, "newest last");
        let (base_m, tip_m) = at(12);
        ring.push(0, base_m, tip_m);
        let rolled = ring.pairs(0);
        assert_eq!(rolled.len(), TRAIL_LEN, "13th still holds 12");
        assert_eq!(rolled[0].base_m, at(1).0, "13th drops the oldest");
        assert_eq!(rolled[TRAIL_LEN - 1].tip_m, at(12).1, "newest is last");
        assert!(ring.pairs(1).is_empty(), "idle slot still holds none");
        assert_eq!(alpha_for_age(0), 1.0, "new is opaque");
        assert_eq!(alpha_for_age(11), 0.0, "oldest is clear");
        assert!(
            alpha_for_age(0) > alpha_for_age(5) && alpha_for_age(5) > alpha_for_age(11),
            "alpha fades with age"
        );
        ring.clear_idle(0);
        assert!(ring.pairs(0).is_empty(), "cleared slot holds none");
    }
}

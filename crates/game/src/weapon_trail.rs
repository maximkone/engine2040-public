//! Weapon trail ring: the last 12 blade base-plus-tip pairs per fighter slot.
//! The ring holds view-only points; it never reads or writes the sim.
//! Each point ages one step per tick; alpha fades from 1 (new) to 0 (oldest).

/// Pairs kept per slot: each holds one blade base plus one blade tip.
pub const TRAIL_LEN: usize = 12;
/// Slots the ring serves; matches `fighter_look::MAX_LOOK_FIGHTERS`.
pub const MAX_TRAIL_SLOTS: usize = 8;

/// One blade sample: base and tip in meters plus its age in ticks.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TrailPoint {
    /// Blade base in meters.
    pub base: [f32; 3],
    /// Blade tip in meters.
    pub tip: [f32; 3],
    /// Ticks since the sample was pushed.
    pub age: u8,
}

/// Oldest age a kept sample can reach: one below [`TRAIL_LEN`].
const OLDEST_AGE: u8 = 11;

const EMPTY_POINT: TrailPoint = TrailPoint {
    base: [0.0; 3],
    tip: [0.0; 3],
    age: 0,
};

/// Ring of recent blade samples per slot. Fixed arrays: a push never
/// allocates.
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

    /// Add a sample at age 0; past [`TRAIL_LEN`] the oldest is dropped.
    /// A slot out of range or a non-finite coordinate is ignored.
    pub fn push(&mut self, slot: usize, base: [f32; 3], tip: [f32; 3]) {
        if slot >= MAX_TRAIL_SLOTS || !base.iter().chain(&tip).all(|v| v.is_finite()) {
            return;
        }
        let ring = &mut self.points[slot];
        let len = &mut self.len[slot];
        if *len == TRAIL_LEN {
            ring.rotate_left(1);
            *len -= 1;
        }
        ring[*len] = TrailPoint { base, tip, age: 0 };
        *len += 1;
    }

    /// Advance one tick: every sample ages by one; samples at
    /// [`TRAIL_LEN`] ticks or older are dropped.
    pub fn age_tick(&mut self) {
        for (ring, len) in self.points.iter_mut().zip(self.len.iter_mut()) {
            let mut kept = 0;
            for read in 0..*len {
                let mut point = ring[read];
                point.age = point.age.saturating_add(1);
                if usize::from(point.age) < TRAIL_LEN {
                    ring[kept] = point;
                    kept += 1;
                }
            }
            *len = kept;
        }
    }

    /// Samples of one slot, oldest first; empty for a slot out of range.
    #[must_use]
    pub fn points(&self, slot: usize) -> Vec<TrailPoint> {
        match (self.points.get(slot), self.len.get(slot)) {
            (Some(ring), Some(&len)) => ring[..len].to_vec(),
            _ => Vec::new(),
        }
    }
}

/// Opacity of a sample by age: 1 at age 0 down to 0 at the oldest kept age
/// (`TRAIL_LEN - 1`), 0 beyond.
#[must_use]
pub fn alpha_for_age(age: u8) -> f32 {
    (1.0 - f32::from(age) / f32::from(OLDEST_AGE)).clamp(0.0, 1.0)
}

/// True when a slot holds no samples: idle slots draw zero triangles.
#[must_use]
pub fn is_idle_empty(slot_points: &[TrailPoint]) -> bool {
    slot_points.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(step: usize) -> ([f32; 3], [f32; 3]) {
        let x = step as f32;
        ([x, 1.0, 0.0], [x, 1.0, 1.0])
    }

    #[test]
    fn trail_caps_at_twelve() {
        let mut ring = TrailRing::new();
        for step in 0..14 {
            let (base, tip) = at(step);
            ring.push(0, base, tip);
        }
        let points = ring.points(0);
        assert_eq!(points.len(), TRAIL_LEN);
        // Oldest first: pushes 0 and 1 fell off, 2 leads, 13 is newest.
        assert_eq!(points[0].base, at(2).0);
        assert_eq!(points[TRAIL_LEN - 1].tip, at(13).1);
        assert!(points.iter().all(|point| point.age == 0));
        assert!(ring.points(1).is_empty(), "other slots stay idle");
    }

    #[test]
    fn trail_ages_out() {
        let mut ring = TrailRing::new();
        for step in 0..3 {
            let (base, tip) = at(step);
            ring.push(2, base, tip);
            ring.age_tick();
        }
        // Each push was aged once more than the next: 3, 2, 1, oldest first.
        let ages: Vec<u8> = ring.points(2).iter().map(|point| point.age).collect();
        assert_eq!(ages, vec![3, 2, 1]);
        assert!(alpha_for_age(3) < alpha_for_age(1));
        assert_eq!(alpha_for_age(0), 1.0);
        assert_eq!(alpha_for_age(11), 0.0);
        assert_eq!(alpha_for_age(200), 0.0);
        for _ in 0..TRAIL_LEN {
            ring.age_tick();
        }
        assert!(ring.points(2).is_empty());
        assert!(is_idle_empty(&ring.points(2)));
    }

    #[test]
    fn trail_idle_empty() {
        assert_eq!(usize::from(OLDEST_AGE) + 1, TRAIL_LEN);
        let mut ring = TrailRing::default();
        for slot in 0..MAX_TRAIL_SLOTS {
            assert!(is_idle_empty(&ring.points(slot)));
        }
        let (base, tip) = at(0);
        ring.push(0, [f32::NAN, 0.0, 0.0], tip);
        ring.push(0, base, [0.0, f32::INFINITY, 0.0]);
        ring.push(MAX_TRAIL_SLOTS, base, tip);
        assert_eq!(ring, TrailRing::new(), "bad pushes store nothing");
        assert!(ring.points(MAX_TRAIL_SLOTS).is_empty());
        ring.push(0, base, tip);
        assert!(!is_idle_empty(&ring.points(0)));
    }

    #[test]
    fn trail_stable_x2() {
        let run = || {
            let mut ring = TrailRing::new();
            for step in 0..20 {
                let (base, tip) = at(step);
                ring.push(step % 3, base, tip);
                if step % 4 == 3 {
                    ring.age_tick();
                }
            }
            (0..3).map(|slot| ring.points(slot)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}

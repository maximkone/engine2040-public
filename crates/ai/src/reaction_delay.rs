//! reaction_delay: integer-only observation delay for AI brains.
//!
//! Consumer: the next slice feeds live ticks into [`ObsRing`] for the brain;
//! this slice only holds old observations for delayed reads. Data owner: the
//! caller owns the ring; this module keeps no globals. Units: ticks for
//! delay, permille for skill. Determinism: integer index only, no float, no
//! clock, no heap; same pushes give the same read twice. Errors: none;
//! [`ObsRing::read`] returns `None` when the delay is not ready yet.

/// Ring holding the last `N` observations for delayed reads.
///
/// Units: one [`push`](ObsRing::push) per tick, [`read`](ObsRing::read) takes
/// ticks back. Determinism: integer index only, no float, no clock, no heap;
/// same pushes give the same read twice. Errors: none; `read` returns `None`
/// when fewer than `delay + 1` samples exist or `delay` passed out of the
/// retained window.
#[derive(Clone, Copy, Debug)]
pub struct ObsRing<T: Copy, const N: usize> {
    slots: [Option<T>; N],
    count: usize,
}

impl<T: Copy, const N: usize> ObsRing<T, N> {
    /// Empty ring with no samples.
    ///
    /// Units: none. Determinism: same on every platform. Errors: none.
    pub fn new() -> Self {
        Self {
            slots: [None; N],
            count: 0,
        }
    }

    /// Push one observation for the current tick.
    ///
    /// Units: one observation per tick. Determinism: integer index only,
    /// overwrites the oldest slot when full. Errors: none; `N == 0` drops.
    pub fn push(&mut self, obs: T) {
        if N == 0 {
            return;
        }
        let idx = self.count % N;
        self.slots[idx] = Some(obs);
        self.count = self.count.wrapping_add(1);
    }

    /// Samples retained, capped at `N`.
    ///
    /// Units: samples. Determinism: pure read. Errors: none.
    pub fn len(&self) -> usize {
        self.count.min(N)
    }

    /// True when no sample is held.
    ///
    /// Units: none. Determinism: pure read. Errors: none.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the observation from `delay` ticks back.
    ///
    /// Units: ticks. Determinism: integer index only, so the same call twice
    /// gives the same value. Errors: returns `None` when fewer than
    /// `delay + 1` samples exist or `delay` left the retained window.
    pub fn read(&self, delay: usize) -> Option<T> {
        if N == 0 {
            return None;
        }
        if delay >= self.count {
            return None;
        }
        if delay >= N {
            return None;
        }
        let idx = (self.count - 1 - delay) % N;
        self.slots[idx]
    }
}

impl<T: Copy, const N: usize> Default for ObsRing<T, N> {
    /// Empty ring with no samples.
    ///
    /// Units: none. Determinism: same on every platform. Errors: none.
    fn default() -> Self {
        Self::new()
    }
}

/// Ticks of reaction delay for a skill in permille.
///
/// Units: skill permille 0..=1000, delay ticks 4..=15. Determinism: integer
/// math only, monotone non-increasing, same input gives same output.
/// Errors: none; skill above 1000 clamps to 4.
pub fn delay_ticks(skill_permille: u16) -> usize {
    let clamped = usize::from(skill_permille.min(1000));
    15 - 11 * clamped / 1000
}

/// The delayed observation when ready, else the last input.
///
/// Units: ticks for `delay`. Determinism: pure read, no clock, no heap.
/// Errors: none; a decision that is not ready repeats the last input
/// (FightingICE 15-frame rule).
pub fn ready_or_last<T: Copy>(ring: &ObsRing<T, 32>, delay: usize, last: T) -> T {
    match ring.read(delay) {
        Some(obs) => obs,
        None => last,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reaction_reads_nine_back() {
        let mut ring = ObsRing::<u32, 32>::new();
        for obs in 0..30u32 {
            ring.push(obs);
        }
        // Newest tick t carries 29, so delay 9 reads t minus 9.
        assert_eq!(ring.read(9), Some(20));
        assert_eq!(ring.read(9), ring.read(9));
    }

    #[test]
    fn reaction_zero_reads_now() {
        let mut ring = ObsRing::<u32, 32>::new();
        for obs in 0..30u32 {
            ring.push(obs);
        }
        assert_eq!(ring.read(0), Some(29));
    }

    #[test]
    fn reaction_delay_ticks_monotone() {
        assert_eq!(delay_ticks(0), 15);
        assert_eq!(delay_ticks(1000), 4);
        assert_eq!(delay_ticks(1001), 4);
        assert_eq!(delay_ticks(u16::MAX), 4);
        let mut prev = delay_ticks(0);
        for skill in 0..=1000u16 {
            let cur = delay_ticks(skill);
            assert!(cur <= prev, "delay rose at skill {skill}: {prev} -> {cur}");
            assert!((4..=15).contains(&cur));
            prev = cur;
        }
    }

    #[test]
    fn reaction_not_ready_repeats_last() {
        let ring = ObsRing::<u32, 32>::new();
        let first = ready_or_last(&ring, 9, 77);
        let second = ready_or_last(&ring, 9, 77);
        assert_eq!(first, 77);
        assert_eq!(first, second);
        let mut live = ObsRing::<u32, 32>::new();
        for obs in 0..30u32 {
            live.push(obs);
        }
        assert_eq!(ready_or_last(&live, 9, 77), 20);
    }
}

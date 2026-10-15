//! utility: utility-AI scorers. Pure decision, no NN, no rand.

use super::Brief;

/// Scores one action from a brief. Must be pure.
pub trait Scorer {
    fn score(&self, brief: &Brief) -> f32;
}

/// NPC action labels. They map to no ops: `Action::ops` returned a constant
/// kick (Brief has no positions yet) and nothing ever called it, so it was
/// deleted. Reintroduce a mapping when Brief carries positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Seek,
    Flee,
    Idle,
}

fn clean(s: f32) -> f32 {
    if s.is_nan() {
        0.0
    } else {
        s
    }
}

/// Deterministic argmax: strict > so first index wins ties. None on empty.
pub fn pick(scorers: &[&dyn Scorer], brief: &Brief) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, s) in scorers.iter().enumerate() {
        let v = clean(s.score(brief));
        match best {
            None => best = Some((i, v)),
            Some((_, b)) if v > b => best = Some((i, v)),
            _ => {}
        }
    }
    best.map(|(i, _)| i)
}

/// How [`pick_gated`] chooses among the scorers that reach the threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// The first scorer (lowest index) at or above the threshold wins.
    First,
    /// The highest scorer at or above the threshold wins; ties go to the
    /// lowest index.
    Highest,
}

/// Result of a gated pick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gated {
    /// Index of the winning scorer.
    Pick(usize),
    /// No scorer reached the threshold (or none exist): take the fallback.
    Otherwise,
}

/// Threshold-gated pick: scorers under `threshold` can never win, so jitter
/// below it changes nothing; when none reach it the answer is
/// [`Gated::Otherwise`]. Deterministic, no rand; a NaN score reads 0 and a
/// NaN threshold is never reached.
pub fn pick_gated(scorers: &[&dyn Scorer], brief: &Brief, threshold: f32, gate: Gate) -> Gated {
    let mut best: Option<(usize, f32)> = None;
    for (i, s) in scorers.iter().enumerate() {
        let v = clean(s.score(brief));
        // partial_cmp keeps the NaN rule explicit: unordered never reaches.
        if matches!(
            v.partial_cmp(&threshold),
            None | Some(std::cmp::Ordering::Less)
        ) {
            continue;
        }
        match (gate, best) {
            (Gate::First, _) => return Gated::Pick(i),
            (Gate::Highest, Some((_, b))) if v <= b => {}
            (Gate::Highest, _) => best = Some((i, v)),
        }
    }
    best.map_or(Gated::Otherwise, |(i, _)| Gated::Pick(i))
}

/// Linear clamped response curve with NaN guard.
pub fn response_curve(x: f32, slope: f32) -> f32 {
    if x.is_nan() || slope.is_nan() {
        return 0.0;
    }
    (x * slope).clamp(0.0, 1.0)
}

/// Clamp a score to 0..=1; NaN reads 0. Units: score. Errors: none. Deterministic.
/// Caller: crates/ai/src/gladiator.rs thinker path.
pub fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// Clamped sum of scores; NaN reads 0, empty reads 0. Units: scores 0..1. Errors: none. Deterministic.
/// Caller: crates/ai/src/gladiator.rs thinker path.
pub fn sum_of(scores: &[f32]) -> f32 {
    let mut acc = 0.0;
    for s in scores {
        acc += clamp01(*s);
    }
    clamp01(acc)
}

/// Clamped product of scores; NaN reads 0, empty reads 0. Units: scores 0..1. Errors: none. Deterministic.
/// Caller: crates/ai/src/gladiator.rs thinker path.
pub fn product_of(scores: &[f32]) -> f32 {
    if scores.is_empty() {
        return 0.0;
    }
    let mut acc = 1.0;
    for s in scores {
        acc *= clamp01(*s);
    }
    clamp01(acc)
}

/// AND gate: 0 when any factor reads below threshold, else clamped sum. NaN reads 0, NaN threshold never passes. Units: scores 0..1. Errors: none. Deterministic.
/// Caller: crates/ai/src/gladiator.rs thinker path.
pub fn all_or_nothing(scores: &[f32], threshold: f32) -> f32 {
    if threshold.is_nan() {
        return 0.0;
    }
    let mut acc = 0.0;
    for s in scores {
        let v = clamp01(*s);
        if v < threshold {
            return 0.0;
        }
        acc += v;
    }
    clamp01(acc)
}

/// Choice table over intents: ordered scorer rows plus an otherwise fallback, evaluated by pick_gated First/Highest gates. Scores never enter any digest. Units: scores 0..1, intent indices. Errors: none. Deterministic, cold build only.
/// Caller: crates/ai/src/gladiator.rs thinker path.
pub struct Thinker<'a> {
    scorers: Vec<&'a dyn Scorer>,
    intents: Vec<usize>,
    otherwise: usize,
}

impl<'a> Thinker<'a> {
    /// New table with otherwise fallback intent index. Units: intent index. Errors: none.
    pub fn new(otherwise: usize) -> Self {
        Self {
            scorers: Vec::new(),
            intents: Vec::new(),
            otherwise,
        }
    }

    /// Push a scorer row mapping to an intent index. Units: intent index. Errors: none.
    pub fn when(mut self, scorer: &'a dyn Scorer, intent: usize) -> Self {
        self.scorers.push(scorer);
        self.intents.push(intent);
        self
    }

    /// Pick the intent index: gated scorer choice maps to its intent, else otherwise. Units: intent index. Errors: none. Deterministic.
    pub fn decide(&self, brief: &Brief, threshold: f32, gate: Gate) -> usize {
        match pick_gated(&self.scorers, brief, threshold, gate) {
            Gated::Pick(i) => self.intents.get(i).copied().unwrap_or(self.otherwise),
            Gated::Otherwise => self.otherwise,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(f32);
    impl Scorer for Fixed {
        fn score(&self, _: &Brief) -> f32 {
            self.0
        }
    }

    fn brief() -> Brief {
        Brief {
            tick: 0,
            entities: 0,
            bodies: 0,
            hash: 0,
        }
    }

    #[test]
    fn highest_wins() {
        let (a, b, c) = (Fixed(0.1), Fixed(0.9), Fixed(0.3));
        assert_eq!(pick(&[&a, &b, &c], &brief()), Some(1));
    }

    #[test]
    fn tie_first_index() {
        let (a, b, c) = (Fixed(0.5), Fixed(0.5), Fixed(0.2));
        assert_eq!(pick(&[&a, &b, &c], &brief()), Some(0));
        let v: Vec<&dyn Scorer> = vec![];
        assert_eq!(pick(&v, &brief()), None);
    }

    #[test]
    fn curve_clamps() {
        assert_eq!(response_curve(-1.0, 1.0), 0.0);
        assert_eq!(response_curve(0.0, 5.0), 0.0);
        assert_eq!(response_curve(0.5, 1.0), 0.5);
        assert_eq!(response_curve(99.0, 1.0), 1.0);
        assert_eq!(response_curve(f32::NAN, 1.0), 0.0);
        assert_eq!(response_curve(0.5, -2.0), 0.0);
    }

    #[test]
    fn utility_threshold_gates_first_and_highest() {
        let (a, b, c) = (Fixed(0.3), Fixed(0.7), Fixed(0.9));
        let set: [&dyn Scorer; 3] = [&a, &b, &c];
        let br = brief();
        assert_eq!(pick_gated(&set, &br, 0.5, Gate::First), Gated::Pick(1));
        assert_eq!(pick_gated(&set, &br, 0.5, Gate::Highest), Gated::Pick(2));
        // A score exactly on the threshold reaches it.
        assert_eq!(pick_gated(&set, &br, 0.9, Gate::First), Gated::Pick(2));
        // Ties among the reached go to the lowest index.
        let (x, y) = (Fixed(0.8), Fixed(0.8));
        assert_eq!(
            pick_gated(&[&x, &y], &br, 0.5, Gate::Highest),
            Gated::Pick(0)
        );
    }

    #[test]
    fn utility_threshold_otherwise_and_jitter() {
        let br = brief();
        let none: Vec<&dyn Scorer> = vec![];
        assert_eq!(pick_gated(&none, &br, 0.5, Gate::Highest), Gated::Otherwise);
        // Jitter below the threshold never flips the result.
        for jitter in [0.0, 0.1, 0.2, 0.49] {
            let (a, b) = (Fixed(0.4 + jitter / 10.0), Fixed(jitter));
            assert_eq!(
                pick_gated(&[&a, &b], &br, 0.5, Gate::Highest),
                Gated::Otherwise
            );
        }
        // NaN scores read 0; a NaN threshold is never reached.
        let nan = Fixed(f32::NAN);
        assert_eq!(pick_gated(&[&nan], &br, 0.5, Gate::First), Gated::Otherwise);
        let ok = Fixed(1.0);
        assert_eq!(
            pick_gated(&[&ok], &br, f32::NAN, Gate::First),
            Gated::Otherwise
        );
        // Replay x2.
        let set: [&dyn Scorer; 1] = [&ok];
        assert_eq!(
            pick_gated(&set, &br, 0.5, Gate::Highest),
            pick_gated(&set, &br, 0.5, Gate::Highest)
        );
    }

    #[test]
    fn determinism_x2() {
        let (a, b) = (Fixed(0.4), Fixed(0.6));
        let br = brief();
        assert_eq!(pick(&[&a, &b], &br), pick(&[&a, &b], &br));
    }

    #[test]
    fn thinker_composite_weights() {
        assert_eq!(clamp01(f32::NAN), 0.0);
        assert_eq!(clamp01(1.5), 1.0);
        assert_eq!(sum_of(&[0.5, 0.25]), 0.75);
        assert_eq!(sum_of(&[]), 0.0);
        assert_eq!(product_of(&[0.5, 0.5]), 0.25);
        assert_eq!(product_of(&[]), 0.0);
        assert_eq!(all_or_nothing(&[0.8, 0.9], 0.5), 1.0);
        assert_eq!(all_or_nothing(&[0.8, 0.2], 0.5), 0.0);
        assert_eq!(all_or_nothing(&[0.8, f32::NAN], 0.5), 0.0);
        let br = brief();
        let attack = all_or_nothing(&[0.2, 0.2], 0.3);
        let advance = sum_of(&[0.5, 0.25]);
        assert_eq!(attack, 0.0);
        assert_eq!(advance, 0.75);
        let (a, b, c, d, e) = (
            Fixed(attack),
            Fixed(0.2),
            Fixed(0.1),
            Fixed(advance),
            Fixed(0.1),
        );
        let table = Thinker::new(3)
            .when(&a, 0)
            .when(&b, 1)
            .when(&c, 2)
            .when(&d, 3)
            .when(&e, 4);
        assert_eq!(table.decide(&br, 0.3, Gate::Highest), 3);
        let bold = crate::personality::Weights::bold();
        let shy = crate::personality::Weights::shy();
        let base = [0.5, 0.5, 0.1, 0.1, 0.1];
        let bb = bold.apply(base);
        let sb = shy.apply(base);
        let (ba0, ba1, ba2, ba3, ba4) = (
            Fixed(bb[0]),
            Fixed(bb[1]),
            Fixed(bb[2]),
            Fixed(bb[3]),
            Fixed(bb[4]),
        );
        let (sa0, sa1, sa2, sa3, sa4) = (
            Fixed(sb[0]),
            Fixed(sb[1]),
            Fixed(sb[2]),
            Fixed(sb[3]),
            Fixed(sb[4]),
        );
        let bt = Thinker::new(3)
            .when(&ba0, 0)
            .when(&ba1, 1)
            .when(&ba2, 2)
            .when(&ba3, 3)
            .when(&ba4, 4);
        let st = Thinker::new(3)
            .when(&sa0, 0)
            .when(&sa1, 1)
            .when(&sa2, 2)
            .when(&sa3, 3)
            .when(&sa4, 4);
        assert_eq!(bt.decide(&br, 0.3, Gate::Highest), 0);
        assert_eq!(st.decide(&br, 0.3, Gate::Highest), 1);
        for jitter in [-0.02, -0.01, 0.0, 0.01, 0.02] {
            let (ja, jb, jd) = (
                Fixed(0.2 + jitter),
                Fixed(0.3 + jitter),
                Fixed(0.9 + jitter),
            );
            let jt = Thinker::new(3).when(&ja, 0).when(&jb, 1).when(&jd, 3);
            assert_eq!(jt.decide(&br, 0.3, Gate::Highest), 3);
        }
        let empty: Thinker<'_> = Thinker::new(3);
        assert_eq!(empty.decide(&br, 0.3, Gate::Highest), 3);
        assert_eq!(
            table.decide(&br, 0.3, Gate::Highest),
            table.decide(&br, 0.3, Gate::Highest)
        );
        assert_eq!(br.hash, 0);
    }
}

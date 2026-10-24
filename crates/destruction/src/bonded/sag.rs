//! sag: staged collapse on the bonded support graph. A link whose carried
//! weight is over its limit builds stress for a few ticks (the structure
//! leans) and only then snaps; a cluster that lost its anchor path hangs for
//! a few ticks (it sags) and only then lets go and falls.
//!
//! Consumer: the arena match and the picture. Authority reads the snaps and
//! frees from the break log like any other break; `engine-physfx` reads
//! [`BondedCollection::sag_q16`] to lean or drop the mesh (never imported
//! here). Data owner: `engine-destruction`. Units: Q16 load (`LOAD_Q16_ONE`
//! is one mass unit), Q16 progress (`SAG_Q16_ONE` is "lets go now"), ticks,
//! millimeters for the lean helper. Determinism scope: integer arithmetic in
//! chunk index order only (the float `breaking_strain` of a link is read once
//! at [`BondedCollection::enable_sag`], never per tick); the same descriptor,
//! rules and hits give the same snaps on every PC.
//!
//! Opt-in and off by default: with no rules set `step` is exactly the plain
//! bonded step, so existing hashes do not move. The sag state is not part of
//! `state_hash` or the snapshot bytes (a restore resets it), so a rollback
//! across a live sag restarts the lean; `sag_digest` is its own receipt.
//!
//! Shape only from the Chaos Destruction connection graph (a link per
//! neighbor pair with a strain limit that breaks under load) and The Finals'
//! staged degradation (floors sag and walls lean before they fall); the
//! graph, loads and snaps are the existing DX1 bonded ones, never a second
//! graph. Budget: one pass over the chunks per step with a bounded scan of
//! each chunk's own adjacency; the state is sized once at `enable_sag` and
//! `step` allocates nothing after.

use super::{slot, wire, BondedCollection, BreakError, BreakEvent, NO_PARENT};
use engine_core::Fnv64;

/// Sag progress meaning "lets go now" (a link snaps, a cluster is freed).
pub const SAG_Q16_ONE: u32 = 65_536;
/// Largest tick count a stage may take; keeps the per-tick step at least one.
pub const MAX_SAG_TICKS: u32 = 65_536;
/// An overloaded link builds stress at most this many times faster than the
/// baseline rate, so a violent overload fails in a quarter of the time.
pub const MAX_OVERLOAD_RATE: u64 = 4;

/// The two staged-collapse rules; either may be off on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SagRules {
    /// Weight (Q16 mass, see `LOAD_Q16_ONE`) one link carries per 1.0 of its
    /// `breaking_strain`. Zero turns the link-load rule off.
    pub strength_q16_per_strain: u64,
    /// Ticks an overloaded link takes to snap at a barely-over load; at
    /// least 1 when the link-load rule is on.
    pub stress_ticks: u32,
    /// Ticks an unsupported cluster hangs and sags before it is freed. Zero
    /// frees it at once, like the plain bonded step.
    pub drop_ticks: u32,
}

/// Sized-once sag state; lives in the collection while the rules are on.
#[derive(Clone, Debug)]
pub(super) struct SagState {
    rules: SagRules,
    /// Per bond: the most weight it carries before it is overloaded, Q16.
    limit_q16: Vec<u64>,
    /// Per bond: stress progress, `0..SAG_Q16_ONE`.
    stress: Vec<u32>,
    /// Per chunk: ticks spent unsupported and still standing.
    lost: Vec<u32>,
    /// Per chunk: worst stress on the links between it and its anchor, as of
    /// the last step; the lean a picture applies to a standing chunk.
    lean: Vec<u32>,
}

/// A link strain in thousandths, rounded half up, read from the float's bits
/// so no numeric cast is needed. Validated strains are finite, non-negative
/// and at most `MAX_BOND_STRAIN`; a subnormal reads zero.
fn milli_strain(strain: f32) -> u64 {
    let bits = strain.to_bits();
    let exp = (bits >> 23) & 0xFF;
    if exp == 0 {
        return 0;
    }
    // value = mant * 2^(exp - 150), mant a 24-bit integer.
    let mant = u64::from((bits & 0x7F_FFFF) | 0x80_0000);
    let thousandths = mant * 1000;
    match exp.checked_sub(150) {
        Some(up) => thousandths.checked_shl(up).unwrap_or(u64::MAX),
        None => {
            let down = 150 - exp;
            if down >= 64 {
                0
            } else {
                (thousandths + (1u64 << (down - 1))) >> down
            }
        }
    }
}

impl BondedCollection {
    /// Turns the staged collapse on (or replaces the rules and clears all
    /// stress). Allocates the per-bond and per-chunk state once; nothing in
    /// `step` allocates after this. Bad rules change nothing.
    pub fn enable_sag(&mut self, rules: SagRules) -> Result<(), BreakError> {
        let stress_floor = u32::from(rules.strength_q16_per_strain != 0);
        if rules.stress_ticks < stress_floor
            || rules.stress_ticks > MAX_SAG_TICKS
            || rules.drop_ticks > MAX_SAG_TICKS
        {
            return Err(BreakError::BadSag);
        }
        let limit_q16 = self
            .bonds
            .iter()
            .map(|bond| {
                // Milli-strain is read from the float once, here; the tick
                // path is all integer.
                let milli = milli_strain(bond.breaking_strain);
                let limit = u128::from(milli) * u128::from(rules.strength_q16_per_strain) / 1000;
                u64::try_from(limit).unwrap_or(u64::MAX)
            })
            .collect();
        self.sag = Some(Box::new(SagState {
            rules,
            limit_q16,
            stress: vec![0; self.bonds.len()],
            lost: vec![0; self.chunks.len()],
            lean: vec![0; self.chunks.len()],
        }));
        Ok(())
    }

    /// Back to the plain bonded step; drops the sag state.
    pub fn disable_sag(&mut self) {
        self.sag = None;
    }

    /// The rules in force, `None` while the staged collapse is off.
    #[must_use]
    pub fn sag_rules(&self) -> Option<SagRules> {
        self.sag.as_ref().map(|s| s.rules)
    }

    /// Clears stress, lean and hang counters (after a restore the sag
    /// restarts from rest).
    pub(super) fn sag_reset(&mut self) {
        if let Some(sag) = self.sag.as_mut() {
            sag.stress.fill(0);
            sag.lost.fill(0);
            sag.lean.fill(0);
        }
    }

    /// Stress of one bond, `0..SAG_Q16_ONE`; zero when sag is off, the bond
    /// is out of range or already snapped.
    #[must_use]
    pub fn link_stress_q16(&self, bond: usize) -> u32 {
        match self.sag.as_ref() {
            Some(sag) if bond < self.bonds.len() && !self.bond_broken[bond] => sag.stress[bond],
            _ => 0,
        }
    }

    /// How far a chunk has sagged toward letting go, `0..=SAG_Q16_ONE`. An
    /// unsupported chunk still hanging reports the share of its hang time
    /// used (live); a standing chunk reports the worst link stress between
    /// it and its anchor as of the last step, so a whole arm leans as one.
    /// Zero for anchors, freed chunks, out-of-range ids and while sag is off.
    #[must_use]
    pub fn sag_q16(&self, chunk: usize) -> u32 {
        let Some(sag) = self.sag.as_ref() else {
            return 0;
        };
        if chunk >= self.chunks.len() || self.anchored[chunk] || self.freed[chunk] {
            return 0;
        }
        if self.support_scratch[chunk] {
            return sag.lean[chunk];
        }
        if sag.rules.drop_ticks == 0 {
            return 0;
        }
        let used =
            u64::from(sag.lost[chunk]) * u64::from(SAG_Q16_ONE) / u64::from(sag.rules.drop_ticks);
        u32::try_from(used.min(u64::from(SAG_Q16_ONE))).unwrap_or(SAG_Q16_ONE)
    }

    /// The lean a picture applies: `span_mm` scaled by [`Self::sag_q16`],
    /// whole millimeters rounded down.
    #[must_use]
    pub fn sag_offset_mm(&self, chunk: usize, span_mm: u32) -> u32 {
        let share = u64::from(span_mm) * u64::from(self.sag_q16(chunk)) / u64::from(SAG_Q16_ONE);
        u32::try_from(share).unwrap_or(u32::MAX)
    }

    /// How many chunks are sagging right now (sag above zero).
    #[must_use]
    pub fn sagging_count(&self) -> usize {
        (0..self.chunks.len())
            .filter(|&i| self.sag_q16(i) > 0)
            .count()
    }

    /// FNV-1a64 over the rules, every link stress and every hang counter:
    /// the receipt of the staged collapse, equal on every PC. Zero while off.
    #[must_use]
    pub fn sag_digest(&self) -> u64 {
        let Some(sag) = self.sag.as_ref() else {
            return 0;
        };
        let mut h = Fnv64::new();
        h.write_u64(sag.rules.strength_q16_per_strain);
        h.write_u32(sag.rules.stress_ticks);
        h.write_u32(sag.rules.drop_ticks);
        for s in &sag.stress {
            h.write_u32(*s);
        }
        for l in &sag.lost {
            h.write_u32(*l);
        }
        h.finish()
    }

    /// True while an unsupported chunk must hang before it is freed.
    pub(super) fn defers_free(&self) -> bool {
        self.sag.as_ref().is_some_and(|s| s.rules.drop_ticks > 0)
    }

    /// The intact bond between a supported chunk and its support-tree
    /// parent, found in the chunk's own sorted adjacency.
    fn parent_bond(&self, chunk: usize) -> Option<usize> {
        let parent = self.parent[chunk];
        if parent == NO_PARENT {
            return None;
        }
        let span = slot(self.adj_start[chunk])..slot(self.adj_start[chunk + 1]);
        self.adj[span]
            .iter()
            .find(|&&(neighbor, bond)| neighbor == parent && !self.bond_broken[slot(bond)])
            .map(|&(_, bond)| slot(bond))
    }

    /// One staged-collapse step, after the plain refresh: first the hanging
    /// clusters age and the ones out of time are freed (ascending chunk
    /// order), then every standing link is judged on the loads of this tick
    /// and the ones whose stress filled snap together, with one support
    /// refresh after. Allocation-free: the wave list is pre-sized scratch.
    pub(super) fn advance_sag(&mut self, tick: u32) {
        let Some(mut sag) = self.sag.take() else {
            return;
        };
        let n = self.chunks.len();
        if sag.rules.drop_ticks > 0 {
            for i in 0..n {
                if self.support_scratch[i] || self.freed[i] {
                    continue;
                }
                sag.lost[i] = sag.lost[i].saturating_add(1);
                if sag.lost[i] >= sag.rules.drop_ticks {
                    self.freed[i] = true;
                    self.dynamic[i] = true;
                    self.log.push(BreakEvent::ChunkFreed {
                        tick,
                        chunk: wire(i),
                    });
                }
            }
        }
        let mut snapped = 0;
        if sag.rules.strength_q16_per_strain != 0 {
            let step = SAG_Q16_ONE.div_ceil(sag.rules.stress_ticks);
            let relax = step.div_ceil(2);
            self.wave_scratch.clear();
            for i in 0..n {
                if self.anchored[i] || !self.support_scratch[i] || self.freed[i] {
                    continue;
                }
                let Some(bond) = self.parent_bond(i) else {
                    continue;
                };
                let carried = self.load_q16[i];
                let limit = sag.limit_q16[bond];
                if carried > limit {
                    let rate = carried
                        .checked_div(limit)
                        .map_or(MAX_OVERLOAD_RATE, |q| q.min(MAX_OVERLOAD_RATE));
                    let add = u32::try_from(u64::from(step) * rate).unwrap_or(u32::MAX);
                    sag.stress[bond] = sag.stress[bond].saturating_add(add);
                    if sag.stress[bond] >= SAG_Q16_ONE {
                        self.wave_scratch.push(bond);
                    }
                } else {
                    sag.stress[bond] = sag.stress[bond].saturating_sub(relax);
                }
            }
            snapped = self.wave_scratch.len();
            for k in 0..snapped {
                let bond = self.wave_scratch[k];
                self.strain[bond] = self.bonds[bond].breaking_strain + 1.0;
                self.bond_broken[bond] = true;
                sag.stress[bond] = 0;
                self.log.push(BreakEvent::BondBroken {
                    tick,
                    bond: wire(bond),
                    strain: self.strain[bond],
                });
            }
        }
        // Back in place before the refresh: it asks `defers_free`.
        self.sag = Some(sag);
        if snapped > 0 {
            self.refresh_support(tick);
        }
        self.refresh_lean();
    }

    /// Lean per standing chunk: the visit list of the last flood has every
    /// parent before its children, so one forward pass carries the worst
    /// stress down each branch.
    fn refresh_lean(&mut self) {
        let Some(mut sag) = self.sag.take() else {
            return;
        };
        sag.lean.fill(0);
        for k in 0..self.work_scratch.len() {
            let node = self.work_scratch[k];
            if self.anchored[node] {
                continue;
            }
            let Some(bond) = self.parent_bond(node) else {
                continue;
            };
            let above = sag.lean[slot(self.parent[node])];
            sag.lean[node] = above.max(sag.stress[bond]);
        }
        self.sag = Some(sag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;
    use crate::bonded::{Bond, BondedDesc, Chunk3, LOAD_Q16_ONE};
    use engine_core::alloc_probe::measure;

    const DT: f32 = 1.0 / 60.0;

    fn at(i: u32) -> f32 {
        f32::from(u16::try_from(i).expect("small"))
    }

    fn cube(x: f32, y: f32) -> Chunk3 {
        Chunk3 {
            x,
            y,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        }
    }

    fn link(a: u32, b: u32, breaking_strain: f32) -> Bond {
        Bond {
            a,
            b,
            breaking_strain,
        }
    }

    /// A column of five cubes, chunk 0 on the ground (anchored).
    fn column() -> BondedCollection {
        let chunks = (0..5).map(|i| cube(0.0, 0.5 + at(i))).collect();
        let bonds = (0..4).map(|i| link(i, i + 1, 1.0)).collect();
        BondedCollection::new(&BondedDesc {
            chunks,
            bonds,
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        })
        .expect("valid column")
    }

    /// A four-cube arm sticking out of a wall: chunk 0 is the wall, the
    /// root link (bond 0) carries the whole arm. `root` and `rest` are the
    /// link strains.
    fn arm(root: f32, rest: f32) -> BondedCollection {
        let chunks = (0..5).map(|i| cube(at(i), 3.0)).collect();
        let bonds = vec![
            link(0, 1, root),
            link(1, 2, rest),
            link(2, 3, rest),
            link(3, 4, rest),
        ];
        BondedCollection::new(&BondedDesc {
            chunks,
            bonds,
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        })
        .expect("valid arm")
    }

    /// A span between two abutments: chunks 0 and 4 anchored, 1..=3 between.
    fn span() -> BondedCollection {
        let chunks = (0..5).map(|i| cube(at(i), 3.0)).collect();
        let bonds = (0..4).map(|i| link(i, i + 1, 1.0)).collect();
        BondedCollection::new(&BondedDesc {
            chunks,
            bonds,
            anchors: vec![0, 4],
            break_threshold: BREAKING_THRESHOLD,
        })
        .expect("valid span")
    }

    /// Rules with a link strength of `half_units / 2` mass units per 1.0 of
    /// breaking strain.
    fn rules(half_units: u64, stress_ticks: u32, drop_ticks: u32) -> SagRules {
        SagRules {
            strength_q16_per_strain: half_units * LOAD_Q16_ONE / 2,
            stress_ticks,
            drop_ticks,
        }
    }

    #[test]
    fn milli_strain_rounds_like_the_float() {
        for (strain, want) in [
            (0.0f32, 0u64),
            (1.0, 1000),
            (0.5, 500),
            (2.0, 2000),
            (0.0004, 0),
            (0.0006, 1),
            (123.456, 123_456),
            (1.0e6, 1_000_000_000),
        ] {
            assert_eq!(milli_strain(strain), want, "{strain}");
        }
    }

    #[test]
    fn off_by_default_and_plain_step_frees_at_once() {
        let mut col = column();
        assert_eq!(col.sag_rules(), None);
        assert_eq!(col.sag_digest(), 0);
        col.sever_chunk(0, 0).expect("in range");
        col.step(DT);
        assert_eq!(col.freed_chunks(), vec![1, 2, 3, 4]);
        assert_eq!(col.sag_q16(2), 0);
        assert_eq!(col.sagging_count(), 0);
    }

    #[test]
    fn bad_rules_are_refused_without_change() {
        let mut col = column();
        assert_eq!(
            col.enable_sag(rules(5, 0, 1)),
            Err(BreakError::BadSag),
            "link rule needs at least one stress tick"
        );
        assert_eq!(
            col.enable_sag(rules(5, MAX_SAG_TICKS + 1, 1)),
            Err(BreakError::BadSag)
        );
        assert_eq!(
            col.enable_sag(rules(5, 4, MAX_SAG_TICKS + 1)),
            Err(BreakError::BadSag)
        );
        assert_eq!(col.sag_rules(), None);
        assert_eq!(col.enable_sag(rules(0, 0, 4)), Ok(()));
        col.disable_sag();
        assert_eq!(col.sag_rules(), None);
    }

    #[test]
    fn column_with_its_foot_knocked_out_sags_then_drops_everything_above() {
        let mut col = column();
        col.enable_sag(rules(0, 1, 4)).expect("rules");
        // Knock out the bottom link: the foot stays (anchor), the four
        // chunks above lose their only anchor path.
        assert_eq!(col.sever_chunk(0, 0), Ok(1));
        assert!(
            col.freed_chunks().is_empty(),
            "hangs, does not drop at once"
        );
        let mut seen = Vec::new();
        for _ in 0..3 {
            col.step(DT);
            assert!(col.freed_chunks().is_empty(), "still hanging");
            seen.push(col.sag_q16(2));
        }
        assert!(
            seen[0] > 0 && seen[0] < seen[1] && seen[1] < seen[2],
            "{seen:?}"
        );
        assert!(seen[2] < SAG_Q16_ONE);
        assert_eq!(col.sagging_count(), 4);
        assert_eq!(
            col.sag_offset_mm(1, 400),
            300,
            "three quarters of a 40 cm lean"
        );
        // The foot never sags; the fourth hang tick lets all four go at once.
        assert_eq!(col.sag_q16(0), 0);
        col.step(DT);
        assert_eq!(
            col.freed_chunks(),
            vec![1, 2, 3, 4],
            "all above fall together"
        );
        assert_eq!(col.sag_q16(2), 0, "freed chunks fall, they no longer sag");
        let freed_at: Vec<u32> = col
            .drain_events()
            .iter()
            .filter_map(|e| match e {
                BreakEvent::ChunkFreed { tick, .. } => Some(*tick),
                BreakEvent::BondBroken { .. } => None,
            })
            .collect();
        assert_eq!(freed_at, vec![4, 4, 4, 4]);
    }

    #[test]
    fn a_strong_link_holds_the_arm_a_weak_one_sags_then_falls() {
        // Four cubes of weight hang through the root link: 4 mass units.
        // Strong root: limit 2.0 * 2.5 = 5 units, so it holds forever.
        let mut strong = arm(2.0, 4.0);
        strong.enable_sag(rules(5, 10, 3)).expect("rules");
        for _ in 0..600 {
            strong.step(DT);
        }
        assert!(strong.freed_chunks().is_empty());
        assert!(strong.broken_bonds().is_empty());
        assert_eq!(strong.sagging_count(), 0);

        // Weak root: limit 1.0 * 2.5 = 2.5 units < 4, the rest hold.
        let mut weak = arm(1.0, 4.0);
        weak.enable_sag(rules(5, 10, 3)).expect("rules");
        let mut lean = Vec::new();
        for _ in 0..9 {
            weak.step(DT);
            lean.push(weak.sag_q16(4));
            assert!(weak.broken_bonds().is_empty(), "leans before it lets go");
        }
        assert!(
            lean.windows(2).all(|w| w[0] < w[1]) && lean[0] > 0,
            "{lean:?}"
        );
        assert_eq!(weak.sag_q16(4), weak.sag_q16(1), "the arm leans as one");
        assert_eq!(weak.sag_q16(0), 0, "the wall does not");
        // Tenth tick: the root link has used its ten ticks and snaps.
        weak.step(DT);
        assert_eq!(weak.broken_bonds(), vec![0]);
        assert!(weak.freed_chunks().is_empty(), "now it hangs");
        // Hang ticks 1 and 2: still up; the third lets the arm go, whole.
        weak.step(DT);
        weak.step(DT);
        assert!(weak.freed_chunks().is_empty());
        assert!(weak.sag_q16(3) > 0);
        weak.step(DT);
        assert_eq!(weak.freed_chunks(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn heavier_overload_fails_faster() {
        // Same arm; a link at a quarter of the load it needs fails in about
        // a quarter of the ticks of one that is barely over.
        let mut barely = arm(1.0, 4.0);
        barely.enable_sag(rules(7, 20, 0)).expect("rules"); // limit 3.5 < 4
        let mut violent = arm(1.0, 4.0);
        violent.enable_sag(rules(2, 20, 0)).expect("rules"); // limit 1 -> 4x
        let (mut t_barely, mut t_violent) = (0, 0);
        for t in 1..=40 {
            barely.step(DT);
            violent.step(DT);
            if t_barely == 0 && !barely.broken_bonds().is_empty() {
                t_barely = t;
            }
            if t_violent == 0 && !violent.broken_bonds().is_empty() {
                t_violent = t;
            }
        }
        assert!(t_violent > 0 && t_barely > 0);
        assert!(t_violent * 3 <= t_barely, "{t_violent} vs {t_barely}");
    }

    #[test]
    fn losing_one_abutment_raises_the_strain_on_the_other_side() {
        // Span of three cubes between two abutments. Intact, the tree hangs
        // chunks 1 and 2 on abutment 0 and chunk 3 on abutment 4: the link
        // 0-1 carries 2 units, under its 2.5 limit. Cutting 3-4 re-hangs
        // chunk 3 on the same side: 3 units, over the limit.
        let mut col = span();
        col.enable_sag(rules(5, 8, 2)).expect("rules");
        for _ in 0..120 {
            col.step(DT);
        }
        assert!(col.broken_bonds().is_empty(), "stands at rest");
        assert_eq!(col.link_stress_q16(0), 0);
        col.add_strain(3, 2.0, 120).expect("snap the far link");
        assert_eq!(col.broken_bonds(), vec![3]);
        assert!(col.freed_chunks().is_empty(), "still held on the near side");
        for _ in 0..7 {
            col.step(DT);
            assert_eq!(col.broken_bonds(), vec![3], "not yet");
        }
        assert!(
            col.link_stress_q16(0) > 0,
            "the near link strains after the loss"
        );
        col.step(DT);
        assert_eq!(col.broken_bonds(), vec![0, 3], "the overloaded link gives");
        col.step(DT);
        col.step(DT);
        assert_eq!(col.freed_chunks(), vec![1, 2, 3]);
    }

    #[test]
    fn stress_relaxes_when_the_load_goes_away() {
        let mut col = arm(1.0, 4.0);
        col.enable_sag(rules(5, 20, 0)).expect("rules");
        for _ in 0..6 {
            col.step(DT);
        }
        let before = col.link_stress_q16(0);
        assert!(before > 0);
        // Drop the outer two cubes: the root now carries 2 units, under its
        // 2.5 limit, so the lean eases off instead of building.
        col.sever_chunk(3, 6).expect("in range");
        assert_eq!(col.broken_bonds(), vec![2, 3]);
        for _ in 0..4 {
            col.step(DT);
        }
        assert!(col.link_stress_q16(0) < before);
        assert!(!col.is_bond_broken(0));
    }

    #[test]
    fn same_inputs_give_the_same_breaks_twice() {
        let run = || {
            let mut col = arm(1.0, 4.0);
            col.enable_sag(rules(5, 10, 3)).expect("rules");
            let mut log = Vec::new();
            for t in 0..40 {
                if t == 5 {
                    col.apply_impact(3, 5.0, t).expect("hit");
                }
                col.step(DT);
                col.drain_into(&mut log);
            }
            (log, col.state_hash(), col.sag_digest())
        };
        let (a, b) = (run(), run());
        assert!(!a.0.is_empty());
        assert_eq!(a, b);
        assert_ne!(a.2, 0);
    }

    #[test]
    fn predict_collapse_ignores_the_hang_delay() {
        let mut col = column();
        col.enable_sag(rules(0, 1, 6)).expect("rules");
        assert_eq!(col.predict_collapse(1), Ok(vec![2, 3, 4]));
    }

    #[test]
    fn restore_resets_the_lean() {
        let mut col = arm(1.0, 4.0);
        col.enable_sag(rules(5, 20, 0)).expect("rules");
        let bytes = col.snapshot_bytes();
        for _ in 0..5 {
            col.step(DT);
        }
        assert!(col.link_stress_q16(0) > 0);
        col.restore_bytes(&bytes).expect("same shape");
        assert_eq!(col.link_stress_q16(0), 0);
        assert_eq!(col.sag_q16(1), 0);
    }

    #[test]
    fn sag_steps_allocate_nothing() {
        let mut col = arm(1.0, 4.0);
        col.enable_sag(rules(5, 10, 3)).expect("rules");
        let mut out = Vec::with_capacity(32);
        // The break log grows once when the first event lands; reserve it so
        // the slice measures the stepping, not the log's first allocation.
        col.log.reserve(16);
        col.step(DT);
        let ((), heap) = measure(|| {
            for _ in 0..30 {
                col.step(DT);
                col.drain_into(&mut out);
            }
        });
        assert!(!col.broken_bonds().is_empty(), "the slice covered the snap");
        assert_eq!(heap.calls, 0, "staged collapse allocated in step");
    }
}

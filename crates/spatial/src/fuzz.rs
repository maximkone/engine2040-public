//! Deterministic fuzz harness over the relation-graph ops: seeded op streams
//! with first-divergence receipts.
//!
//! Consumer: the playtest gold harness runs [`run_suite`] beside the other
//! spatial gates and fails the build on a false positive or a missed probe;
//! solver and brief readers use receipts to bisect a regression to one op.
//! Data owner: this module owns op streams, per-step digests and receipts;
//! the relation graph and the check catalog own truth. Units: integer
//! centimeters inside every op, meters at the actor boundary. Determinism
//! scope: the same seed gives the same stream, trail and receipt on every
//! platform and build; input order never matters because every step rebuilds
//! through the id-sorted pipeline. No wall clock, thread timing or hash-map
//! order. The harness never errors: every op is total (unknown ids are
//! no-ops), so there is no failure type. Draws come from the crate's shared
//! FNV owner (selection only); step digests mix the graph digest, the actor
//! snapshot and the check report, so a change anywhere in the pipeline moves
//! the trail.

use super::index::to_cm;
use super::relation::{build, digest, rebuild_cached};
use super::{run, Actor};
use engine_core::Fnv64;
use std::collections::BTreeSet;

/// Clean op-stream pairs per suite run.
pub const FUZZ_PAIRS: usize = 100;
/// Ops per stream.
pub const FUZZ_OPS: usize = 24;
/// Injected-divergence probes per suite run.
pub const FUZZ_PROBES: usize = 10;
/// First clean-pair seed; pair `i` uses this base plus `i`.
pub const FUZZ_SEED_BASE: u64 = 5000;
/// First probe seed; probe `i` uses this base plus `i`.
pub const FUZZ_PROBE_BASE: u64 = 9000;
/// Arena shell id. Never removed, so the live set is never empty.
pub const ARENA_ID: u32 = 0;
/// Injected probe nudge, X cm. Odd on purpose: defeats grid alignment.
pub const INJECT_DX_CM: i32 = 137;
/// Injected probe nudge, Z cm.
pub const INJECT_DZ_CM: i32 = -41;

/// One graph op. All geometry in integer centimeters; ids are actor ids.
/// `Add` carries a full resting placement, `Move`/`Lift` shift it, `Resize`
/// regrows it around its center, `Rename` swaps a grammar-legal name,
/// `Remove` drops the actor. Ops on missing ids are no-ops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuzzOp {
    /// Add a resting actor with a grammar-legal slotted name.
    Add {
        /// Fresh actor id.
        id: u32,
        /// Center X in cm.
        x_cm: i32,
        /// Center Z in cm.
        z_cm: i32,
        /// Edge length in cm.
        size_cm: i32,
        /// Name slot, folded to 64 grammar-legal names.
        name_slot: u32,
    },
    /// Shift an actor on the ground plane.
    Move {
        /// Target actor id.
        id: u32,
        /// X shift in cm.
        dx_cm: i32,
        /// Z shift in cm.
        dz_cm: i32,
    },
    /// Shift an actor vertically (floaters included: the pipeline must stay
    /// total and deterministic on odd states).
    Lift {
        /// Target actor id.
        id: u32,
        /// Y shift in cm.
        dy_cm: i32,
    },
    /// Regrow an actor around its center, resting on the floor.
    Resize {
        /// Target actor id.
        id: u32,
        /// New edge length in cm.
        size_cm: i32,
    },
    /// Swap an actor's name for a grammar-legal slotted one.
    Rename {
        /// Target actor id.
        id: u32,
        /// Name slot, folded to 64 grammar-legal names.
        name_slot: u32,
    },
    /// Drop an actor; the arena shell refuses (stays live).
    Remove {
        /// Target actor id.
        id: u32,
    },
}

/// Per-pair receipt: what ran, where the probe went in, where the trails
/// parted, and the evidence digest. `diverged` is the op index both trails
/// shared before parting: `None` for agreeing pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzReceipt {
    /// Stream seed.
    pub seed: u64,
    /// Ops in the clean stream.
    pub ops: usize,
    /// Injected probe position, when this pair carries one.
    pub inject: Option<usize>,
    /// First differing op index, `None` when the trails agree.
    pub diverged: Option<usize>,
    /// True when every cached rebuild matched its full rebuild.
    pub cached_ok: bool,
    /// Evidence digest over both trails.
    pub digest: u64,
}

/// One replayed trail: per-step digests plus the incremental-rebuild parity
/// flag. Entry 0 is the base scene; entry `i + 1` follows op `i`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpTrail {
    /// Step digests, base scene first.
    pub digests: Vec<u64>,
    /// True when every cached rebuild matched its full rebuild.
    pub cached_ok: bool,
}

/// Whole-suite outcome: clean pairs plus injected probes with one evidence
/// digest. Read the bar with [`FuzzSuite::clean_false_positives`] (must be
/// 0), [`FuzzSuite::probes_located`] (must equal the probe count) and
/// [`FuzzSuite::all_cached_ok`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzSuite {
    /// Clean-pair receipts in seed order.
    pub pairs: Vec<FuzzReceipt>,
    /// Injected-probe receipts in seed order.
    pub probes: Vec<FuzzReceipt>,
    /// Evidence digest over every receipt, seed order.
    pub digest: u64,
}

impl FuzzSuite {
    /// Clean pairs whose trails diverged. Acceptance bar: 0.
    pub fn clean_false_positives(&self) -> usize {
        self.pairs.iter().filter(|r| r.diverged.is_some()).count()
    }

    /// Probes whose trails parted exactly at the injected op. Bar: equals
    /// the probe count.
    pub fn probes_located(&self) -> usize {
        self.probes
            .iter()
            .filter(|r| r.inject == r.diverged)
            .count()
    }

    /// True when every pair and probe saw cached rebuilds match full ones.
    pub fn all_cached_ok(&self) -> bool {
        self.pairs
            .iter()
            .chain(self.probes.iter())
            .all(|r| r.cached_ok)
    }

    /// Canonical report text: header, one row per receipt in seed order,
    /// then the summary. Byte-stable across runs and platforms.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "spatial fuzz gate v1: {} clean pairs, {} injected probes\n",
            self.pairs.len(),
            self.probes.len()
        ));
        for r in &self.pairs {
            out.push_str(&format!(
                "pair seed {} ops {} diverged none digest {:016x}\n",
                r.seed, r.ops, r.digest
            ));
        }
        for r in &self.probes {
            let inject = r.inject.unwrap_or(usize::MAX);
            let diverged = r.diverged.unwrap_or(usize::MAX);
            out.push_str(&format!(
                "probe seed {} inject {inject} diverged {diverged} digest {:016x}\n",
                r.seed, r.digest
            ));
        }
        out.push_str(&format!(
            "summary fps {} located {}/{} cached {} digest {:016x}\n",
            self.clean_false_positives(),
            self.probes_located(),
            self.probes.len(),
            if self.all_cached_ok() {
                "ok"
            } else {
                "MISMATCH"
            },
            self.digest
        ));
        out
    }
}

/// Draw stream for op selection. Hash-DRBG over the engine FNV owner: the
/// seed tags the state once, then each draw folds the counter in. Selection
/// only, never hashed into a digest, so modulo bias is harmless; determinism
/// is what matters.
struct Draws {
    /// Running FNV state.
    state: Fnv64,
    /// Draw counter folded into the state per draw.
    counter: u64,
}

impl Draws {
    /// Tag the shared owner with the harness domain and the seed.
    fn seed(seed: u64) -> Self {
        let mut state = Fnv64::new();
        state.write(b"spatial-fuzz-draws");
        state.write_u64(seed);
        Self { state, counter: 0 }
    }

    /// Next word; the counter advance keeps draws distinct.
    fn next_u64(&mut self) -> u64 {
        self.state.write_u64(self.counter);
        self.counter = self.counter.wrapping_add(1);
        self.state.finish()
    }

    /// A draw folded into `0..bound`; 0 when the bound is 0.
    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }
}

/// Integer centimeters to meters. Total: saturates at the i16-backed range
/// (the suite provokes |cm| < 16000, far inside); one exact step, identical
/// on every target. Exists because `f32` has no `From<i32>` and raw `as`
/// casts are banned by the quality ratchet.
fn cm_to_m(cm: i32) -> f32 {
    let clamped = i16::try_from(cm).unwrap_or(if cm < 0 { i16::MIN } else { i16::MAX });
    f32::from(clamped) / 100.0
}

/// Grammar-legal slotted name: lowercase prefix, never a leading digit.
fn slot_name(slot: u32) -> String {
    format!("fuzz_{:02}", slot % 64)
}

/// Resting cube actor from integer-cm placement. Mirror of the test helper
/// the check catalog assumes: floor contact, no velocity, zero yaw.
fn resting_actor(id: u32, name: String, x_cm: i32, z_cm: i32, size_cm: i32) -> Actor {
    let size_m = cm_to_m(size_cm);
    Actor {
        id,
        name,
        x: cm_to_m(x_cm),
        y: size_m / 2.0,
        z: cm_to_m(z_cm),
        sx: size_m,
        sy: size_m,
        sz: size_m,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

/// Fixed base scene: a 40 m arena shell with five resting props. Seeds
/// differ in their op streams, not their starting scene, so every receipt
/// is comparable op for op.
fn base_scene() -> Vec<Actor> {
    vec![
        Actor {
            id: ARENA_ID,
            name: "arena".into(),
            x: 0.0,
            y: -0.5,
            z: 0.0,
            sx: 40.0,
            sy: 1.0,
            sz: 40.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        },
        resting_actor(1, "prop_01".into(), -500, -500, 100),
        resting_actor(2, "prop_02".into(), 500, -500, 100),
        resting_actor(3, "prop_03".into(), -500, 500, 150),
        resting_actor(4, "prop_04".into(), 500, 500, 150),
        resting_actor(5, "prop_05".into(), 0, 0, 200),
    ]
}

/// One live id drawn from the set, `None` when it is empty.
fn pick_live(draws: &mut Draws, live: &BTreeSet<u32>) -> Option<u32> {
    let bound = u64::try_from(live.len()).unwrap_or(0);
    let want = usize::try_from(draws.below(bound)).unwrap_or(0);
    live.iter().nth(want).copied()
}

/// Deterministic op stream of `len` ops from `seed`. Only live ids are
/// referenced (fresh ids for adds), so every op is effective; the arena
/// shell is never removed, so the live set never empties.
pub fn stream_ops(seed: u64, len: usize) -> Vec<FuzzOp> {
    let mut draws = Draws::seed(seed);
    let mut ops = Vec::new();
    let mut live: BTreeSet<u32> = BTreeSet::new();
    for id in ARENA_ID..6 {
        live.insert(id);
    }
    let mut next_id: u32 = 6;
    for _ in 0..len {
        let pick = draws.below(6);
        if pick == 0 {
            let id = next_id;
            next_id = next_id.wrapping_add(1);
            live.insert(id);
            ops.push(FuzzOp::Add {
                id,
                x_cm: i32::try_from(draws.below(6001)).unwrap_or(0) - 3000,
                z_cm: i32::try_from(draws.below(6001)).unwrap_or(0) - 3000,
                size_cm: 50 + i32::try_from(draws.below(251)).unwrap_or(0),
                name_slot: u32::try_from(draws.below(64)).unwrap_or(0),
            });
        } else if pick == 1 {
            if let Some(id) = pick_live(&mut draws, &live) {
                ops.push(FuzzOp::Move {
                    id,
                    dx_cm: i32::try_from(draws.below(1001)).unwrap_or(0) - 500,
                    dz_cm: i32::try_from(draws.below(1001)).unwrap_or(0) - 500,
                });
            }
        } else if pick == 2 {
            if let Some(id) = pick_live(&mut draws, &live) {
                ops.push(FuzzOp::Lift {
                    id,
                    dy_cm: i32::try_from(draws.below(401)).unwrap_or(0) - 200,
                });
            }
        } else if pick == 3 {
            if let Some(id) = pick_live(&mut draws, &live) {
                ops.push(FuzzOp::Resize {
                    id,
                    size_cm: 50 + i32::try_from(draws.below(251)).unwrap_or(0),
                });
            }
        } else if pick == 4 {
            if let Some(id) = pick_live(&mut draws, &live) {
                ops.push(FuzzOp::Rename {
                    id,
                    name_slot: u32::try_from(draws.below(64)).unwrap_or(0),
                });
            }
        } else if let Some(id) = pick_live(&mut draws, &live) {
            if id == ARENA_ID {
                ops.push(FuzzOp::Move {
                    id,
                    dx_cm: i32::try_from(draws.below(1001)).unwrap_or(0) - 500,
                    dz_cm: i32::try_from(draws.below(1001)).unwrap_or(0) - 500,
                });
            } else {
                live.remove(&id);
                ops.push(FuzzOp::Remove { id });
            }
        }
    }
    ops
}

/// Apply one op. Total: unknown or duplicate ids are no-ops returning
/// `None`; otherwise returns the touched id for incremental-rebuild parity.
pub fn apply_op(actors: &mut Vec<Actor>, op: &FuzzOp) -> Option<u32> {
    match *op {
        FuzzOp::Add {
            id,
            x_cm,
            z_cm,
            size_cm,
            name_slot,
        } => {
            if actors.iter().any(|a| a.id == id) {
                return None;
            }
            actors.push(resting_actor(id, slot_name(name_slot), x_cm, z_cm, size_cm));
            Some(id)
        }
        FuzzOp::Move { id, dx_cm, dz_cm } => {
            let actor = actors.iter_mut().find(|a| a.id == id)?;
            actor.x = cm_to_m(to_cm(actor.x).saturating_add(dx_cm));
            actor.z = cm_to_m(to_cm(actor.z).saturating_add(dz_cm));
            Some(id)
        }
        FuzzOp::Lift { id, dy_cm } => {
            let actor = actors.iter_mut().find(|a| a.id == id)?;
            actor.y = cm_to_m(to_cm(actor.y).saturating_add(dy_cm));
            Some(id)
        }
        FuzzOp::Resize { id, size_cm } => {
            let actor = actors.iter_mut().find(|a| a.id == id)?;
            let size_m = cm_to_m(size_cm);
            actor.sx = size_m;
            actor.sy = size_m;
            actor.sz = size_m;
            actor.y = size_m / 2.0;
            Some(id)
        }
        FuzzOp::Rename { id, name_slot } => {
            let actor = actors.iter_mut().find(|a| a.id == id)?;
            actor.name = slot_name(name_slot);
            Some(id)
        }
        FuzzOp::Remove { id } => {
            if id == ARENA_ID || !actors.iter().any(|a| a.id == id) {
                return None;
            }
            actors.retain(|a| a.id != id);
            Some(id)
        }
    }
}

/// Snapshot digest over the actor rows the pipeline consumes: id-sorted
/// integer-cm geometry, parent link, physical flag and name bytes. Ops never
/// touch yaw or velocity, so those stay outside the fuzzed state.
fn snapshot_digest(actors: &[Actor]) -> u64 {
    let mut rows: Vec<&Actor> = actors.iter().collect();
    rows.sort_by_key(|a| a.id);
    let mut h = Fnv64::new();
    h.write(b"spatial-fuzz-snap");
    for a in rows {
        h.write_u32(a.id);
        for v in [a.x, a.y, a.z, a.sx, a.sy, a.sz] {
            h.write_i32(to_cm(v));
        }
        h.write_u32(a.parent.unwrap_or(u32::MAX));
        h.write_bool(a.physical);
        h.write_str(&a.name);
    }
    h.finish()
}

/// One step digest: the relation-graph digest, the actor snapshot and the
/// check-catalog report hash under one tag, so a change in any layer moves
/// the trail.
fn step_digest(actors: &[Actor]) -> u64 {
    let mut h = Fnv64::new();
    h.write(b"spatial-fuzz-step");
    h.write_u64(digest(&build(actors)));
    h.write_u64(snapshot_digest(actors));
    h.write_u64(run(actors).hash);
    h.finish()
}

/// Replay `ops` over the base scene, digesting every step and checking that
/// the incremental rebuild matches the full one at each op (the touched id
/// is the complete change set by construction).
pub fn replay(ops: &[FuzzOp]) -> OpTrail {
    let mut actors = base_scene();
    let mut digests = Vec::new();
    digests.push(step_digest(&actors));
    let mut prev = build(&actors);
    let mut ok = true;
    for op in ops {
        let touched = apply_op(&mut actors, op);
        let fresh = build(&actors);
        let mut changed = Vec::new();
        if let Some(id) = touched {
            changed.push(id);
        }
        ok = ok && rebuild_cached(&prev, &changed, &actors) == fresh;
        prev = fresh;
        digests.push(step_digest(&actors));
    }
    OpTrail {
        digests,
        cached_ok: ok,
    }
}

/// First index where two digest trails differ; a length mismatch diverges at
/// the shorter length. Equal trails give `None`.
///
/// ```rust
/// use engine_spatial::fuzz::first_divergence;
/// assert_eq!(first_divergence(&[1, 2, 3], &[1, 2, 3]), None);
/// assert_eq!(first_divergence(&[1, 2, 9], &[1, 2, 3]), Some(2));
/// assert_eq!(first_divergence(&[1, 2], &[1, 2, 3]), Some(2));
/// ```
pub fn first_divergence(a: &[u64], b: &[u64]) -> Option<usize> {
    let common = a.len().min(b.len());
    for (i, pair) in a.iter().zip(b.iter()).enumerate().take(common) {
        if pair.0 != pair.1 {
            return Some(i);
        }
    }
    if a.len() == b.len() {
        None
    } else {
        Some(common)
    }
}

/// Splice one probe nudge before op `at` (appended when past the end). The
/// target is the lowest live non-arena id after the shared prefix, so the
/// nudge always lands on a real actor and always moves the snapshot.
pub fn inject_probe(ops: &[FuzzOp], at: usize) -> Vec<FuzzOp> {
    let cut = at.min(ops.len());
    let mut actors = base_scene();
    for op in ops.iter().take(cut) {
        apply_op(&mut actors, op);
    }
    let mut ids: Vec<u32> = actors.iter().map(|a| a.id).collect();
    ids.sort_unstable();
    ids.retain(|id| *id != ARENA_ID);
    let target = ids.first().copied().unwrap_or(ARENA_ID);
    let mut out = Vec::new();
    out.extend_from_slice(&ops[..cut]);
    out.push(FuzzOp::Move {
        id: target,
        dx_cm: INJECT_DX_CM,
        dz_cm: INJECT_DZ_CM,
    });
    out.extend_from_slice(&ops[cut..]);
    out
}

/// Run one pair: the clean stream twice (or the probed stream for the
/// second trail when `inject` is set) and compare the trails. `diverged`
/// names the op both trails shared before parting.
pub fn run_pair(seed: u64, len: usize, inject: Option<usize>) -> FuzzReceipt {
    let base = stream_ops(seed, len);
    let trail_a = replay(&base);
    let trail_b = match inject {
        Some(at) => replay(&inject_probe(&base, at.min(base.len()))),
        None => replay(&base),
    };
    let diverged =
        first_divergence(&trail_a.digests, &trail_b.digests).and_then(|step| step.checked_sub(1));
    let mut h = Fnv64::new();
    h.write(b"spatial-fuzz-receipt");
    h.write_u64(seed);
    h.write_u64(u64::try_from(trail_a.digests.len()).unwrap_or(u64::MAX));
    for d in trail_a.digests.iter().chain(trail_b.digests.iter()) {
        h.write_u64(*d);
    }
    FuzzReceipt {
        seed,
        ops: base.len(),
        inject: inject.map(|at| at.min(base.len())),
        diverged,
        cached_ok: trail_a.cached_ok && trail_b.cached_ok,
        digest: h.finish(),
    }
}

/// Run the full suite: [`FUZZ_PAIRS`] clean pairs from [`FUZZ_SEED_BASE`]
/// plus [`FUZZ_PROBES`] injected probes from [`FUZZ_PROBE_BASE`] with spread
/// injection points. Receipts stay in seed order, so the text and digest
/// are byte-stable.
pub fn run_suite() -> FuzzSuite {
    let mut pairs = Vec::new();
    for i in 0..FUZZ_PAIRS {
        let seed = FUZZ_SEED_BASE.wrapping_add(u64::try_from(i).unwrap_or(0));
        pairs.push(run_pair(seed, FUZZ_OPS, None));
    }
    let mut probes = Vec::new();
    for i in 0..FUZZ_PROBES {
        let seed = FUZZ_PROBE_BASE.wrapping_add(u64::try_from(i).unwrap_or(0));
        let at = (i * (FUZZ_OPS + 1)) / FUZZ_PROBES;
        probes.push(run_pair(seed, FUZZ_OPS, Some(at)));
    }
    let mut h = Fnv64::new();
    h.write(b"spatial-fuzz-suite");
    for r in pairs.iter().chain(probes.iter()) {
        h.write_u64(r.digest);
    }
    FuzzSuite {
        pairs,
        probes,
        digest: h.finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cm_to_m_is_exact_and_total() {
        assert_eq!(cm_to_m(0), 0.0);
        assert_eq!(cm_to_m(100), 1.0);
        assert_eq!(cm_to_m(-250), -2.5);
        assert_eq!(cm_to_m(i32::MAX), f32::from(i16::MAX) / 100.0);
        assert_eq!(cm_to_m(i32::MIN), f32::from(i16::MIN) / 100.0);
    }

    #[test]
    fn draws_repeat_per_seed() {
        let (mut a, mut b) = (Draws::seed(77), Draws::seed(77));
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn streams_repeat_per_seed_and_vary_across_seeds() {
        assert_eq!(stream_ops(5000, 24), stream_ops(5000, 24));
        assert_ne!(stream_ops(5000, 24), stream_ops(5001, 24));
    }

    #[test]
    fn apply_ignores_missing_ids() {
        let mut actors = base_scene();
        let before = snapshot_digest(&actors);
        assert_eq!(
            apply_op(&mut actors, &FuzzOp::Remove { id: 4242 }),
            None,
            "removing a missing id is a no-op"
        );
        assert_eq!(
            apply_op(
                &mut actors,
                &FuzzOp::Move {
                    id: 4242,
                    dx_cm: 10,
                    dz_cm: 10
                }
            ),
            None,
            "moving a missing id is a no-op"
        );
        assert_eq!(snapshot_digest(&actors), before, "no-ops change nothing");
    }

    #[test]
    fn real_change_moves_the_trail() {
        let base = stream_ops(5000, 8);
        let plain = replay(&base);
        let mut moved = base.clone();
        moved.push(FuzzOp::Move {
            id: 1,
            dx_cm: 1,
            dz_cm: 0,
        });
        let after = replay(&moved);
        assert_eq!(
            first_divergence(&plain.digests, &after.digests),
            Some(base.len() + 1),
            "one appended op diverges at its own step"
        );
    }

    #[test]
    fn clean_pair_never_diverges() {
        for seed in [0, 1, 5000, 9999] {
            let receipt = run_pair(seed, 24, None);
            assert_eq!(receipt.diverged, None, "clean seed {seed} must agree");
            assert!(receipt.cached_ok, "seed {seed} keeps rebuild parity");
        }
    }

    #[test]
    fn probe_names_its_op() {
        for at in [0, 1, 7, 23, 24] {
            let receipt = run_pair(9000, 24, Some(at));
            assert_eq!(
                receipt.diverged,
                Some(at),
                "probe at {at} must part exactly there"
            );
            assert!(receipt.cached_ok, "probe at {at} keeps rebuild parity");
        }
    }

    #[test]
    fn suite_text_is_byte_stable() {
        let suite = run_suite();
        assert_eq!(suite.pairs.len(), FUZZ_PAIRS);
        assert_eq!(suite.probes.len(), FUZZ_PROBES);
        assert_eq!(suite.clean_false_positives(), 0, "no clean pair may part");
        assert_eq!(
            suite.probes_located(),
            FUZZ_PROBES,
            "every probe must name its op"
        );
        assert!(suite.all_cached_ok(), "every step keeps rebuild parity");
        let again = run_suite();
        assert_eq!(suite.digest, again.digest, "suite digest x2");
        assert_eq!(suite.to_text(), again.to_text(), "suite text x2");
    }
}

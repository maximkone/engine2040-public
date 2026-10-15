//! break_corpus: seeded replay corpus for the blade cut resolver.
//!
//! Consumer: `cargo test -p engine-proof break_corpus` (the Bodies proof in
//! `docs/FOCUS.md`) and the cross-platform digest matrix, which compares
//! [`CutReport::digest`] and its checkpoints between machines. Data owner:
//! `engine-combat` owns the resolver, the anatomy rows and the cut tables;
//! this module owns only the seeded case generator, the rest-pose table and
//! the pinned digest. Units: millimetres for geometry (victim-local, +Y up),
//! Q16 for energy, sharpness and wear. Determinism: case `i` is a pure
//! function of `(seed, i)` drawn from `CounterRng::fork(i)`, so it never
//! depends on corpus length or run order; integer math only; the resolver
//! must give one answer under a shuffled target order and must agree with the
//! per-joint solver. Errors: [`CorpusError`] names the first bad case.
//! Budget: offline proof tool, 10,000 cases in well under a second; never
//! called per tick.

use engine_combat::{
    cut_one, digest_cuts, try_resolve_swing, AnatomyTable, BladeSwing, Cut, CutError, CutMiss,
    CutOutcome, CutTables, JointId, JointTarget, ARMOR_ALL, CUT_POS_BOUND_MM, CUT_V1, JOINT_TOTAL,
    MAX_CUTS_PER_SWING, MAX_ENERGY_Q16, MAX_SHARPNESS_Q16, SEVERABLE_JOINT_COUNT,
};
use engine_core::{CounterCursor, CounterRng, Fnv64};
use std::fmt;

/// Seed of the shipped corpus. `CounterRng` expands every bit, so seeds
/// `2k` and `2k + 1` draw different cases (unlike the frozen legacy `Rng`).
pub const CORPUS_SEED: u64 = 2040;
/// Cases in the shipped cut corpus.
pub const CUT_CASES: u32 = 10_000;
/// Cases between two checkpoint digests; a platform diff names the block.
pub const CHECKPOINT_EVERY: u32 = 500;
/// Pinned digest of [`run_cuts`]`(CORPUS_SEED, CUT_CASES)`.
///
/// Receipt: Windows x86-64 (i5-13420H), debug and release builds, two runs
/// each, one value. The Linux x86-64 run is the HS2 matrix. Re-pin only with
/// an intentional change to the resolver, the anatomy rows, the cut tables or
/// [`cut_case`], and say which in the commit.
pub const GOLDEN_CUT_DIGEST: u64 = 0xb37f_c6d6_6df2_d417;
/// Version of the digest layout; bump when [`fold_digest`] changes shape.
const CORPUS_VERSION: u32 = 1;
/// Pinned break hash of [`run_cuts`]`(CORPUS_SEED, CUT_CASES)` folded by limb
/// group (trunk, arms, legs): the one hash over which limbs the corpus severs.
///
/// Receipt: pinned by the HS2 builder on Windows x86-64 from two identical
/// runs; the Linux x86-64 and arm runs of the HS2 matrix must print the same
/// value. Re-pin only with an intentional change to the resolver, the anatomy
/// rows, the cut tables, [`cut_case`] or [`limb_group`], and say which in the
/// commit. The joint-level [`GOLDEN_CUT_DIGEST`] stays frozen beside it.
pub const BREAK_GOLDEN: u64 = 0xbfd9_9831_3f75_2ade;
/// Version of the break-digest layout; bump when [`fold_break_digest`]
/// changes shape.
const BREAK_VERSION: u32 = 1;
/// Limb groups in the break hash, in canonical [`LimbGroup`] order.
pub const LIMB_GROUP_COUNT: usize = 5;

/// Stream tag of the target-order permutation, apart from the case inputs.
const ORDER_STREAM: u64 = 0x4f52_4445_5200_0001;
/// One case in 64 stands at the arena edge, where coordinates are large.
const EDGE_ONE_IN: u32 = 64;
/// One swing in 64 is a zero-length point blade (the degenerate-edge branch).
const POINT_BLADE_ONE_IN: u32 = 64;
/// Pose jitter per endpoint axis in mm (an animated body is never rest pose).
const POSE_JITTER_MM: i32 = 30;
/// Radius jitter in mm.
const RADIUS_JITTER_MM: i32 = 10;
/// Standing distance from the origin in mm for ordinary cases.
const STAND_SPAN_MM: i32 = 1_500;
/// Keep edge cases this far inside the strict position bound, in mm.
const EDGE_MARGIN_MM: i32 = 3_000;
/// Blade half-travel per axis in mm; the longest swing is under 3.2 m.
const BLADE_REACH_MM: i32 = 900;
/// Aim offset beyond the capsule radius in mm: some swings miss.
const AIM_SLACK_MM: u32 = 120;
/// Sharpness `0.25` in Q16, the dullest edge in the corpus.
const MIN_SHARPNESS_Q16: u32 = 16_384;
/// Energy `2.0` in Q16, the weakest swing in the corpus.
const MIN_ENERGY_Q16: u32 = 131_072;
/// Wear `0.9` in Q16, the most dulled edge in the corpus.
const MAX_CORPUS_WEAR_Q16: u32 = 58_982;

/// One victim limb in rest pose: spine ends and radius, victim-local mm.
#[derive(Clone, Copy)]
struct Bone {
    q0_mm: [i32; 3],
    q1_mm: [i32; 3],
    radius_mm: u32,
}

const fn bone(q0_mm: [i32; 3], q1_mm: [i32; 3], radius_mm: u32) -> Bone {
    Bone {
        q0_mm,
        q1_mm,
        radius_mm,
    }
}

/// Standing 1.8 m fighter facing +Z, one capsule per severable joint in
/// [`JointId`] order from the waist: each capsule spans the limb segment
/// below its joint, so the arms, legs, torso and head are all reachable.
const REST_POSE: [Bone; SEVERABLE_JOINT_COUNT] = [
    bone([0, 950, 0], [0, 1100, 0], 140),       // Waist
    bone([0, 1100, 0], [0, 1350, 0], 170),      // Chest
    bone([0, 1450, 0], [0, 1700, 0], 90),       // Neck and head
    bone([-200, 1400, 0], [-260, 1130, 0], 70), // ShoulderL
    bone([200, 1400, 0], [260, 1130, 0], 70),   // ShoulderR
    bone([-260, 1130, 0], [-300, 880, 0], 55),  // ElbowL
    bone([260, 1130, 0], [300, 880, 0], 55),    // ElbowR
    bone([-300, 880, 0], [-310, 780, 0], 40),   // WristL
    bone([300, 880, 0], [310, 780, 0], 40),     // WristR
    bone([-100, 950, 0], [-110, 500, 0], 95),   // HipL
    bone([100, 950, 0], [110, 500, 0], 95),     // HipR
    bone([-110, 500, 0], [-110, 80, 0], 70),    // KneeL
    bone([110, 500, 0], [110, 80, 0], 70),      // KneeR
    bone([-110, 80, 0], [-110, 0, 140], 50),    // AnkleL
    bone([110, 80, 0], [110, 0, 140], 50),      // AnkleR
];

/// One limb group in the break hash: every severable joint belongs to exactly
/// one group, so the per-group cut counts partition the cut stream.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LimbGroup {
    /// Waist, chest and neck bisects.
    Trunk,
    /// Shield-arm shoulder, elbow and wrist.
    ArmL,
    /// Sword-arm shoulder, elbow and wrist.
    ArmR,
    /// Shield-side hip, knee and ankle.
    LegL,
    /// Sword-side hip, knee and ankle.
    LegR,
}

impl LimbGroup {
    /// Canonical index: group order, the break-digest order.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Trunk => 0,
            Self::ArmL => 1,
            Self::ArmR => 2,
            Self::LegL => 3,
            Self::LegR => 4,
        }
    }

    /// Every group in canonical order, for order-proof iteration.
    #[must_use]
    pub const fn all() -> [Self; LIMB_GROUP_COUNT] {
        [Self::Trunk, Self::ArmL, Self::ArmR, Self::LegL, Self::LegR]
    }
}

/// Limb group of a joint; the pelvis root belongs to no limb.
///
/// Caller: [`run_with`] attributes every emitted cut through this mapping
/// into [`CutReport::by_limb`].
#[must_use]
pub const fn limb_group(joint: JointId) -> Option<LimbGroup> {
    match joint {
        JointId::Pelvis => None,
        JointId::Waist | JointId::Chest | JointId::Neck => Some(LimbGroup::Trunk),
        JointId::ShoulderL | JointId::ElbowL | JointId::WristL => Some(LimbGroup::ArmL),
        JointId::ShoulderR | JointId::ElbowR | JointId::WristR => Some(LimbGroup::ArmR),
        JointId::HipL | JointId::KneeL | JointId::AnkleL => Some(LimbGroup::LegL),
        JointId::HipR | JointId::KneeR | JointId::AnkleR => Some(LimbGroup::LegR),
    }
}

/// One corpus case: a blade swing, the victim's worn armor and its posed
/// joint capsules in canonical joint order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CutCase {
    /// The swing handed to the resolver; `swing` equals the case index.
    pub swing: BladeSwing,
    /// Worn armor bits (`ARMOR_*`).
    pub worn: u8,
    /// One capsule per severable joint, ascending joint order.
    pub targets: [JointTarget; SEVERABLE_JOINT_COUNT],
}

/// Why a corpus run stopped. Every variant names the first bad case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorpusError {
    /// A corpus of zero cases proves nothing.
    Empty,
    /// The strict resolver refused a generated case: a generator defect.
    Resolver {
        /// Case index.
        case: u32,
        /// The resolver's refusal.
        error: CutError,
    },
    /// The same targets in shuffled order resolved to different cuts.
    OrderDivergence {
        /// Case index.
        case: u32,
    },
    /// The swing resolver and the per-joint solver disagree.
    SolverDivergence {
        /// Case index.
        case: u32,
    },
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "break corpus has no cases"),
            Self::Resolver { case, error } => {
                write!(f, "case {case}: resolver refused the swing ({error})")
            }
            Self::OrderDivergence { case } => {
                write!(f, "case {case}: shuffled target order changed the cuts")
            }
            Self::SolverDivergence { case } => {
                write!(f, "case {case}: swing and per-joint resolvers disagree")
            }
        }
    }
}

impl std::error::Error for CorpusError {}

/// What a corpus run produced. `digest` is the value the golden pins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CutReport {
    /// Cases run.
    pub cases: u32,
    /// Cuts emitted over all cases.
    pub cuts: u32,
    /// Swings that cut at least one joint.
    pub cutting_swings: u32,
    /// Swings that cut two or more joints.
    pub multi_cut_swings: u32,
    /// Joint tests where the edge touched but the roll held.
    pub held: u32,
    /// Joint tests where the edge never reached the capsule.
    pub out_of_reach: u32,
    /// Cuts per joint wire value (the pelvis slot stays zero).
    pub by_joint: [u32; JOINT_TOTAL],
    /// Cuts per limb group in [`LimbGroup`] order; the five counts sum to
    /// `cuts`, so the break hash pins which limbs the corpus severs.
    pub by_limb: [u32; LIMB_GROUP_COUNT],
    /// `digest_cuts` over the whole cut stream in case order.
    pub stream_digest: u64,
    /// `digest_cuts` over the stream prefix after every [`CHECKPOINT_EVERY`]
    /// cases and after the last case; the first unequal entry names the
    /// block of cases where two machines diverged.
    pub checkpoints: Vec<u64>,
    /// The one corpus digest: version, cases, anatomy rows, stream, tallies.
    pub digest: u64,
}

impl fmt::Display for CutReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "break_corpus cases={} cuts={} cutting={} multi={} held={} reach={} stream={:016x} digest={:016x}",
            self.cases,
            self.cuts,
            self.cutting_swings,
            self.multi_cut_swings,
            self.held,
            self.out_of_reach,
            self.stream_digest,
            self.digest
        )
    }
}

/// Uniform `lo..=hi` from one word; a reversed range returns `lo`.
fn draw_u32(cursor: &mut CounterCursor, lo: u32, hi: u32) -> u32 {
    let span = u64::from(hi.saturating_sub(lo)) + 1;
    let offset = cursor.next_u64() % span;
    lo.saturating_add(u32::try_from(offset).unwrap_or(0))
}

/// Uniform `lo..=hi` from one word; a reversed range returns `lo`.
fn draw_i32(cursor: &mut CounterCursor, lo: i32, hi: i32) -> i32 {
    let span = u64::from(hi.saturating_sub(lo).max(0).unsigned_abs()) + 1;
    let offset = cursor.next_u64() % span;
    let value = i64::from(lo) + i64::try_from(offset).unwrap_or(0);
    i32::try_from(value).unwrap_or(lo)
}

fn draw_u8(cursor: &mut CounterCursor, lo: u8, hi: u8) -> u8 {
    u8::try_from(draw_u32(cursor, u32::from(lo), u32::from(hi))).unwrap_or(lo)
}

fn draw_point(cursor: &mut CounterCursor, around: [i32; 3], span: i32) -> [i32; 3] {
    [
        around[0] + draw_i32(cursor, -span, span),
        around[1] + draw_i32(cursor, -span, span),
        around[2] + draw_i32(cursor, -span, span),
    ]
}

fn add3(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Capsule of slot `slot` (joint `slot + 1`) posed at `origin`.
fn posed_target(cursor: &mut CounterCursor, origin: [i32; 3], slot: usize) -> JointTarget {
    let rest = REST_POSE[slot];
    let joint = JointId::all()[slot + 1];
    let radius = i64::from(rest.radius_mm)
        + i64::from(draw_i32(cursor, -RADIUS_JITTER_MM, RADIUS_JITTER_MM));
    JointTarget {
        joint,
        q0_mm: draw_point(cursor, add3(rest.q0_mm, origin), POSE_JITTER_MM),
        q1_mm: draw_point(cursor, add3(rest.q1_mm, origin), POSE_JITTER_MM),
        radius_mm: u32::try_from(radius.max(1)).unwrap_or(1),
    }
}

/// Case `index` of the corpus seeded with `seed`.
///
/// Pure function of its arguments: one `CounterRng` fork per case, integer
/// draws only. The victim stands near the origin (one case in 64 at the arena
/// edge), jittered off the rest pose; the blade is aimed at a random joint
/// with a miss margin, so a corpus mixes clean cuts, held edges, misses,
/// multi-joint sweeps, armored and bare joints, worn edges and point blades.
#[must_use]
pub fn cut_case(seed: u64, index: u32) -> CutCase {
    let mut draw = CounterCursor::from_rng(CounterRng::new(seed).fork(u64::from(index)), 0);
    let attacker = draw_u8(&mut draw, 0, 63);
    let victim = (attacker + draw_u8(&mut draw, 1, 63)) % 64;
    let span = if draw_u32(&mut draw, 0, EDGE_ONE_IN - 1) == 0 {
        CUT_POS_BOUND_MM - EDGE_MARGIN_MM
    } else {
        STAND_SPAN_MM
    };
    let origin = [
        draw_i32(&mut draw, -span, span),
        0,
        draw_i32(&mut draw, -span, span),
    ];
    let targets: [JointTarget; SEVERABLE_JOINT_COUNT] =
        core::array::from_fn(|slot| posed_target(&mut draw, origin, slot));

    let aimed = targets[usize::from(draw_u8(&mut draw, 0, 14))];
    let reach = i32::try_from(aimed.radius_mm + AIM_SLACK_MM).unwrap_or(0);
    let middle = [
        (aimed.q0_mm[0] + aimed.q1_mm[0]) / 2,
        (aimed.q0_mm[1] + aimed.q1_mm[1]) / 2,
        (aimed.q0_mm[2] + aimed.q1_mm[2]) / 2,
    ];
    let aim = draw_point(&mut draw, middle, reach);
    let half_travel = if draw_u32(&mut draw, 0, POINT_BLADE_ONE_IN - 1) == 0 {
        [0; 3]
    } else {
        draw_point(&mut draw, [0; 3], BLADE_REACH_MM)
    };
    let worn_wear = if draw_u32(&mut draw, 0, 1) == 0 {
        0
    } else {
        draw_u32(&mut draw, 0, MAX_CORPUS_WEAR_Q16)
    };
    let swing = BladeSwing {
        attacker,
        victim,
        swing: index,
        p0_mm: [
            aim[0] - half_travel[0],
            aim[1] - half_travel[1],
            aim[2] - half_travel[2],
        ],
        p1_mm: add3(aim, half_travel),
        blade_half_mm: draw_u32(&mut draw, 5, 30),
        energy_q16: draw_u32(&mut draw, MIN_ENERGY_Q16, MAX_ENERGY_Q16),
        sharpness_q16: draw_u32(&mut draw, MIN_SHARPNESS_Q16, MAX_SHARPNESS_Q16),
        wear_q16: worn_wear,
    };
    CutCase {
        swing,
        worn: draw_u8(&mut draw, 0, ARMOR_ALL),
        targets,
    }
}

/// The case's capsules in a seeded permutation (Fisher-Yates), drawn from a
/// stream apart from the case inputs so the permutation never moves a case.
fn shuffled_targets(
    seed: u64,
    index: u32,
    targets: &[JointTarget; SEVERABLE_JOINT_COUNT],
) -> [JointTarget; SEVERABLE_JOINT_COUNT] {
    let order = CounterRng::new(seed)
        .fork(u64::from(index))
        .fork(ORDER_STREAM);
    let mut draw = CounterCursor::from_rng(order, 0);
    let mut out = *targets;
    for top in (1..out.len()).rev() {
        let pick =
            usize::try_from(draw.next_u64() % (u64::try_from(top).unwrap_or(0) + 1)).unwrap_or(0);
        out.swap(top, pick);
    }
    out
}

/// One case resolved: the cuts plus the per-joint miss tallies.
struct Resolved {
    cuts: [Cut; MAX_CUTS_PER_SWING],
    count: usize,
    held: u32,
    out_of_reach: u32,
}

/// Resolve `case` three ways and require one answer: the strict swing
/// resolver over the canonical capsules, the same over a shuffled order, and
/// the per-joint solver joint by joint.
fn resolve_case(
    seed: u64,
    index: u32,
    case: &CutCase,
    anatomy: &AnatomyTable,
    tables: &CutTables,
) -> Result<Resolved, CorpusError> {
    let blank = Cut {
        swing: 0,
        attacker: 0,
        victim: 0,
        joint: 0,
        flags: 0,
        energy_q16: 0,
        protection_q16: 0,
    };
    let mut cuts = [blank; MAX_CUTS_PER_SWING];
    let resolver = |error| CorpusError::Resolver { case: index, error };
    let count = try_resolve_swing(
        &case.swing,
        &case.targets,
        anatomy,
        case.worn,
        tables,
        &mut cuts,
    )
    .map_err(resolver)?;

    let shuffled = shuffled_targets(seed, index, &case.targets);
    let mut again = [blank; MAX_CUTS_PER_SWING];
    let again_count = try_resolve_swing(
        &case.swing,
        &shuffled,
        anatomy,
        case.worn,
        tables,
        &mut again,
    )
    .map_err(resolver)?;
    if again_count != count || again[..count] != cuts[..count] {
        return Err(CorpusError::OrderDivergence { case: index });
    }

    let (mut held, mut out_of_reach, mut seen) = (0u32, 0u32, 0usize);
    for target in &case.targets {
        match cut_one(&case.swing, target, anatomy, case.worn, tables) {
            CutOutcome::Sever(hit) => {
                if seen >= count || cuts[seen] != hit {
                    return Err(CorpusError::SolverDivergence { case: index });
                }
                seen += 1;
            }
            CutOutcome::Miss(CutMiss::Held) => held += 1,
            CutOutcome::Miss(CutMiss::OutOfReach) => out_of_reach += 1,
            // The generator never builds self-swings or pelvis targets.
            CutOutcome::Miss(_) => return Err(CorpusError::SolverDivergence { case: index }),
        }
    }
    if seen != count {
        return Err(CorpusError::SolverDivergence { case: index });
    }
    Ok(Resolved {
        cuts,
        count,
        held,
        out_of_reach,
    })
}

/// Digest layout: version, case count, anatomy rows, cut stream, tallies.
fn fold_digest(report: &CutReport, anatomy_digest: u64) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(CORPUS_VERSION);
    hasher.write_u32(report.cases);
    hasher.write_u64(anatomy_digest);
    hasher.write_u64(report.stream_digest);
    hasher.write_u32(report.cutting_swings);
    hasher.write_u32(report.multi_cut_swings);
    hasher.write_u32(report.held);
    hasher.write_u32(report.out_of_reach);
    for count in report.by_joint {
        hasher.write_u32(count);
    }
    hasher.finish()
}

/// Break-digest layout: version, case count, anatomy rows, cut stream,
/// tallies, limb-group counts. The joint-level [`fold_digest`] stays frozen;
/// this second hash pins the limb-level break outcome of the same run.
fn fold_break_digest(report: &CutReport, anatomy_digest: u64) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(BREAK_VERSION);
    hasher.write_u32(report.cases);
    hasher.write_u64(anatomy_digest);
    hasher.write_u64(report.stream_digest);
    hasher.write_u32(report.cuts);
    hasher.write_u32(report.cutting_swings);
    hasher.write_u32(report.multi_cut_swings);
    hasher.write_u32(report.held);
    hasher.write_u32(report.out_of_reach);
    for count in report.by_limb {
        hasher.write_u32(count);
    }
    hasher.finish()
}

/// The one break hash of a corpus run: version, cases, anatomy rows, cut
/// stream and limb-group tallies in a single digest.
///
/// Caller: the HS2 matrix pins [`BREAK_GOLDEN`] through this function, beside
/// the frozen joint-level digest. Two calls with equal reports return equal
/// digests. Determinism: pure over the report bytes in canonical order with
/// integer ops only, so every architecture agrees, or names its divergent
/// block through [`CutReport::checkpoints`].
#[must_use]
pub fn break_digest(report: &CutReport) -> u64 {
    fold_break_digest(report, AnatomyTable::rome_kit().digest())
}

/// Run `cases` cases of the corpus seeded with `seed` through the cut
/// resolver and fold the cuts into one digest.
///
/// Errors: [`CorpusError`] for an empty corpus, a refused generated case, or
/// a resolver that depends on target order or disagrees with its own
/// per-joint solver. Two calls with equal arguments return equal reports.
pub fn run_cuts(seed: u64, cases: u32) -> Result<CutReport, CorpusError> {
    run_with(seed, cases, None)
}

/// `run_cuts` with every victim's worn armor forced to `worn` when given.
fn run_with(seed: u64, cases: u32, worn: Option<u8>) -> Result<CutReport, CorpusError> {
    if cases == 0 {
        return Err(CorpusError::Empty);
    }
    let anatomy = AnatomyTable::rome_kit();
    let tables = CUT_V1;
    let mut stream: Vec<Cut> = Vec::new();
    let mut report = CutReport {
        cases,
        cuts: 0,
        cutting_swings: 0,
        multi_cut_swings: 0,
        held: 0,
        out_of_reach: 0,
        by_joint: [0; JOINT_TOTAL],
        by_limb: [0; LIMB_GROUP_COUNT],
        stream_digest: 0,
        checkpoints: Vec::new(),
        digest: 0,
    };
    for index in 0..cases {
        let mut case = cut_case(seed, index);
        if let Some(forced) = worn {
            case.worn = forced;
        }
        let done = resolve_case(seed, index, &case, &anatomy, &tables)?;
        let made = &done.cuts[..done.count];
        stream.extend_from_slice(made);
        report.held += done.held;
        report.out_of_reach += done.out_of_reach;
        report.cutting_swings += u32::from(!made.is_empty());
        report.multi_cut_swings += u32::from(made.len() > 1);
        for hit in made {
            report.by_joint[usize::from(hit.joint) % JOINT_TOTAL] += 1;
            if let Some(joint) = JointId::from_u8(hit.joint) {
                if let Some(group) = limb_group(joint) {
                    report.by_limb[group.index()] += 1;
                }
            }
        }
        let finished = index + 1;
        if finished % CHECKPOINT_EVERY == 0 || finished == cases {
            report.checkpoints.push(digest_cuts(&stream));
        }
    }
    report.cuts = u32::try_from(stream.len()).unwrap_or(u32::MAX);
    report.stream_digest = digest_cuts(&stream);
    report.digest = fold_digest(&report, anatomy.digest());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_combat::{ARMOR_MANICA, MAX_TARGETS_PER_SWING, MAX_WEAR_Q16};

    fn full() -> CutReport {
        run_cuts(CORPUS_SEED, CUT_CASES).expect("the shipped corpus resolves")
    }

    #[test]
    fn break_corpus_golden_digest_x2() {
        let first = full();
        let second = full();
        println!("{first}");
        assert_eq!(first, second, "two runs of one corpus must be identical");
        assert_eq!(first.digest, GOLDEN_CUT_DIGEST, "corpus digest moved");
    }

    #[test]
    fn break_corpus_limb_groups_partition_every_severable_joint() {
        // Oracle first: each severable joint sits in exactly one group, the
        // pelvis root in none, and group order is canonical.
        let mut seen = [false; JOINT_TOTAL];
        for group in LimbGroup::all() {
            for joint in JointId::all() {
                if limb_group(joint) == Some(group) {
                    assert!(!seen[joint.index()], "joint {joint:?} in two groups");
                    seen[joint.index()] = true;
                }
            }
        }
        for joint in JointId::all() {
            assert_eq!(seen[joint.index()], joint.is_severable(), "{joint:?}");
        }
        assert_eq!(limb_group(JointId::Pelvis), None);
        let order: Vec<usize> = LimbGroup::all().iter().map(|group| group.index()).collect();
        assert_eq!(order, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn break_corpus_break_digest_is_one_pinned_hash_x2() {
        let first = full();
        let second = full();
        let (one, two) = (break_digest(&first), break_digest(&second));
        println!("break_digest={one:016x} limbs={:?}", first.by_limb);
        assert_eq!(one, two, "two runs of one corpus must be identical");
        assert_eq!(one, BREAK_GOLDEN, "break digest moved");
        // Limb counts partition the cut stream: every cut lands in one group,
        // and every limb bleeds well past a token count.
        assert_eq!(first.by_limb.iter().copied().sum::<u32>(), first.cuts);
        assert!(
            first.by_limb.iter().all(|count| *count > 50),
            "limb cover: {:?}",
            first.by_limb
        );
    }

    #[test]
    fn break_corpus_break_digest_sees_a_moved_limb() {
        let report = full();
        let pinned = break_digest(&report);
        assert_eq!(
            fold_break_digest(&report, AnatomyTable::rome_kit().digest()),
            pinned
        );
        // Moving one cut's tally across limb groups flips the one hash,
        // while the joint-level digest underneath stays put.
        let mut moved = report.clone();
        moved.by_limb[LimbGroup::ArmR.index()] =
            moved.by_limb[LimbGroup::ArmR.index()].saturating_sub(1);
        moved.by_limb[LimbGroup::LegL.index()] += 1;
        assert_ne!(break_digest(&moved), pinned);
        assert_eq!(moved.digest, report.digest);
        assert_ne!(
            fold_break_digest(&report, AnatomyTable::rome_kit().digest() ^ 1),
            pinned
        );
    }

    #[test]
    fn break_corpus_mixes_every_outcome_and_joint() {
        let report = full();
        assert_eq!(report.cases, CUT_CASES);
        // Every joint test is exactly one of cut, held or out of reach.
        assert_eq!(
            report.cuts + report.held + report.out_of_reach,
            CUT_CASES * u32::try_from(SEVERABLE_JOINT_COUNT).unwrap_or(0)
        );
        let tenth = CUT_CASES / 10;
        assert!(
            report.cutting_swings > tenth,
            "too few swings cut: {report}"
        );
        assert!(
            report.cutting_swings < CUT_CASES - tenth,
            "too many cut: {report}"
        );
        assert!(
            report.multi_cut_swings > CUT_CASES / 100,
            "no sweeps: {report}"
        );
        assert!(report.held > CUT_CASES / 10, "no held edges: {report}");
        assert!(report.out_of_reach > CUT_CASES, "no misses: {report}");
        assert_eq!(report.by_joint[0], 0, "the pelvis never severs");
        for joint in JointId::all().iter().filter(|joint| joint.is_severable()) {
            assert!(
                report.by_joint[joint.index()] > 20,
                "joint {} is barely covered: {:?}",
                joint.index(),
                report.by_joint
            );
        }
        let blocks = usize::try_from(CUT_CASES.div_ceil(CHECKPOINT_EVERY)).unwrap_or(0);
        assert_eq!(report.checkpoints.len(), blocks);
        assert_eq!(report.checkpoints.last(), Some(&report.stream_digest));
    }

    #[test]
    fn break_corpus_cases_do_not_depend_on_corpus_length() {
        let short = run_cuts(CORPUS_SEED, CHECKPOINT_EVERY).expect("prefix resolves");
        let long = full();
        assert_eq!(short.checkpoints[0], long.checkpoints[0]);
        assert_ne!(short.digest, long.digest, "length is part of the digest");
        for index in [0, 1, 777, CUT_CASES - 1] {
            assert_eq!(cut_case(CORPUS_SEED, index), cut_case(CORPUS_SEED, index));
            assert_eq!(cut_case(CORPUS_SEED, index).swing.swing, index);
        }
    }

    #[test]
    fn break_corpus_seeds_that_alias_in_the_legacy_rng_differ() {
        let even = run_cuts(CORPUS_SEED, CHECKPOINT_EVERY).expect("even seed");
        let odd = run_cuts(CORPUS_SEED + 1, CHECKPOINT_EVERY).expect("odd seed");
        assert_ne!(even.stream_digest, odd.stream_digest);
        assert_ne!(cut_case(CORPUS_SEED, 3), cut_case(CORPUS_SEED + 1, 3));
    }

    #[test]
    fn break_corpus_digest_sees_one_changed_cut() {
        let report = full();
        let anatomy = AnatomyTable::rome_kit().digest();
        assert_eq!(fold_digest(&report, anatomy), report.digest);
        let mut moved = report.clone();
        moved.stream_digest ^= 1;
        assert_ne!(fold_digest(&moved, anatomy), report.digest);
        let mut tallied = report.clone();
        tallied.held += 1;
        assert_ne!(fold_digest(&tallied, anatomy), report.digest);
        assert_ne!(fold_digest(&report, anatomy ^ 1), report.digest);

        let one = Cut {
            swing: 1,
            attacker: 2,
            victim: 3,
            joint: 7,
            flags: 0,
            energy_q16: 9,
            protection_q16: 4,
        };
        let two = Cut { joint: 9, ..one };
        assert_ne!(
            digest_cuts(&[one, two]),
            digest_cuts(&[two, one]),
            "order is signal"
        );
        assert_ne!(digest_cuts(&[one, two]), digest_cuts(&[one]));
    }

    #[test]
    fn break_corpus_armor_stops_cuts_the_bare_corpus_makes() {
        let bare = run_with(CORPUS_SEED, CUT_CASES, Some(0)).expect("bare");
        let armored = run_with(CORPUS_SEED, CUT_CASES, Some(ARMOR_ALL)).expect("armored");
        assert!(
            armored.cuts < bare.cuts,
            "armor stopped nothing: {armored} vs {bare}"
        );
        // The manica guards the sword arm only: legs cut as often as bare.
        let sleeve = run_with(CORPUS_SEED, CUT_CASES, Some(ARMOR_MANICA)).expect("manica");
        assert_eq!(
            sleeve.by_joint[JointId::KneeR.index()],
            bare.by_joint[JointId::KneeR.index()]
        );
        assert!(sleeve.by_joint[JointId::ElbowR.index()] < bare.by_joint[JointId::ElbowR.index()]);
        assert_ne!(bare.digest, armored.digest);
    }

    #[test]
    fn break_corpus_shuffle_is_a_real_permutation() {
        let case = cut_case(CORPUS_SEED, 12);
        let canonical: Vec<u8> = case.targets.iter().map(|t| t.joint.as_u8()).collect();
        for index in 0..32 {
            let order = shuffled_targets(CORPUS_SEED, index, &case.targets);
            assert_ne!(order, case.targets, "shuffle {index} left the order alone");
            let mut joints: Vec<u8> = order.iter().map(|t| t.joint.as_u8()).collect();
            joints.sort_unstable();
            assert_eq!(joints, canonical, "shuffle lost or repeated a joint");
        }
        assert!(case.targets.len() <= MAX_TARGETS_PER_SWING);
    }

    /// Closest distance between segments `p1q1` and `p2q2` in f64 (test
    /// oracle only; the resolver and the digest never see a float).
    fn segment_gap_mm(p1: [i32; 3], q1: [i32; 3], p2: [i32; 3], q2: [i32; 3]) -> f64 {
        let v = |a: [i32; 3]| [f64::from(a[0]), f64::from(a[1]), f64::from(a[2])];
        let sub = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let (p1, p2) = (v(p1), v(p2));
        let (d1, d2, r) = (sub(v(q1), p1), sub(v(q2), p2), sub(p1, p2));
        let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
        let (s, t) = if a <= 0.0 && e <= 0.0 {
            (0.0, 0.0)
        } else if a <= 0.0 {
            (0.0, (f / e).clamp(0.0, 1.0))
        } else {
            let c = dot(d1, r);
            if e <= 0.0 {
                ((-c / a).clamp(0.0, 1.0), 0.0)
            } else {
                let b = dot(d1, d2);
                let denom = a * e - b * b;
                let s = if denom > 0.0 {
                    ((b * f - c * e) / denom).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let t = (b * s + f) / e;
                if t < 0.0 {
                    ((-c / a).clamp(0.0, 1.0), 0.0)
                } else if t > 1.0 {
                    (((b - c) / a).clamp(0.0, 1.0), 1.0)
                } else {
                    (s, t)
                }
            }
        };
        let near =
            |p: [f64; 3], d: [f64; 3], k: f64| [p[0] + d[0] * k, p[1] + d[1] * k, p[2] + d[2] * k];
        let gap = sub(near(p1, d1, s), near(p2, d2, t));
        dot(gap, gap).sqrt()
    }

    #[test]
    fn break_corpus_geometry_matches_a_float_oracle() {
        let anatomy = AnatomyTable::rome_kit();
        let slack = f64::from(CUT_V1.contact_slack_mm);
        let (mut touched, mut grazes, mut cleared) = (0u32, 0u32, 0u32);
        for index in 0..CUT_CASES {
            let case = cut_case(CORPUS_SEED, index);
            for target in &case.targets {
                let gap = segment_gap_mm(
                    case.swing.p0_mm,
                    case.swing.p1_mm,
                    target.q0_mm,
                    target.q1_mm,
                );
                let bare = f64::from(case.swing.blade_half_mm) + f64::from(target.radius_mm);
                let hit = !matches!(
                    cut_one(&case.swing, target, &anatomy, case.worn, &CUT_V1),
                    CutOutcome::Miss(CutMiss::OutOfReach)
                );
                // A true graze is never missed; a reported touch is a real one.
                if gap <= bare - 1e-6 {
                    assert!(hit, "case {index}: graze at {gap} mm was missed");
                    grazes += 1;
                }
                if hit {
                    assert!(
                        gap <= bare + slack + 1e-6,
                        "case {index}: phantom touch at {gap} mm"
                    );
                    touched += 1;
                } else {
                    cleared += 1;
                }
            }
        }
        assert!(grazes > 5_000 && touched >= grazes && cleared > 100_000);
    }

    #[test]
    fn break_corpus_inputs_reach_every_generator_branch() {
        let cases: Vec<CutCase> = (0..CUT_CASES).map(|i| cut_case(CORPUS_SEED, i)).collect();
        let edge = |c: &&CutCase| c.targets[0].q0_mm[0].abs() > 100_000;
        assert!(
            cases.iter().filter(edge).count() > 50,
            "no arena-edge cases"
        );
        let point = |c: &&CutCase| c.swing.p0_mm == c.swing.p1_mm;
        assert!(cases.iter().filter(point).count() > 50, "no point blades");
        assert!(cases.iter().any(|c| c.swing.wear_q16 == 0));
        assert!(cases.iter().any(|c| c.swing.wear_q16 > MAX_WEAR_Q16 / 2));
        assert!(cases.iter().any(|c| c.worn == 0));
        assert!(cases.iter().any(|c| c.worn == ARMOR_ALL));
        let slots: std::collections::BTreeSet<u8> = cases.iter().map(|c| c.swing.victim).collect();
        assert_eq!(slots.len(), 64, "victim slots must cover the roster");
    }

    #[test]
    fn break_corpus_cases_stay_inside_the_resolver_contract() {
        for index in (0..CUT_CASES).step_by(7) {
            let case = cut_case(CORPUS_SEED, index);
            assert_ne!(case.swing.attacker, case.swing.victim);
            assert_eq!(case.worn & !ARMOR_ALL, 0);
            assert!(case.swing.wear_q16 <= MAX_WEAR_Q16);
            for (slot, target) in case.targets.iter().enumerate() {
                assert_eq!(target.joint, JointId::all()[slot + 1]);
            }
        }
    }

    #[test]
    fn break_corpus_refuses_an_empty_run_and_names_a_bad_case() {
        assert_eq!(run_cuts(CORPUS_SEED, 0), Err(CorpusError::Empty));
        let mut case = cut_case(CORPUS_SEED, 5);
        case.swing.victim = case.swing.attacker;
        let anatomy = AnatomyTable::rome_kit();
        let refused = resolve_case(CORPUS_SEED, 5, &case, &anatomy, &CUT_V1).err();
        assert_eq!(
            refused,
            Some(CorpusError::Resolver {
                case: 5,
                error: CutError::SelfTarget(case.swing.attacker)
            })
        );
        assert!(refused
            .map(|e| e.to_string().contains("case 5"))
            .unwrap_or(false));
    }
}

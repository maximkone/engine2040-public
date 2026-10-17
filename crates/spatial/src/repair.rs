//! Revision-bound repair descriptions: proposals, never silent moves.
//!
//! Consumer: the editor assistant's approved-repair flow (owns the real
//! command apply/undo). The client converts a [`Repair`] into a staged command
//! batch, previews it with [`apply_preview`], re-runs the staged-gate
//! postchecks and applies only on approval. Data owner is the caller: input
//! is a plain actor snapshot plus check [`Finding`](super::Finding)s, output
//! is plain proposals or a preview.
//! Units are actor meters in, integer centimeters inside (inherited from the
//! checks); [`Repair::delta_cm`] carries integer-cm position deltas and
//! [`Repair::yaw_deci_deg`] decidegrees of yaw. Determinism scope: the same
//! findings over the same actor set in any input order give the identical
//! proposal list. Failure mode: impossible findings (unknown actor, unknown
//! position, a nudge past the world bound) yield [`Proposal::Refused`] with a
//! conflict set, and [`apply_preview`] refuses stale revisions; either way the
//! caller's scene is untouched, both functions are pure.
//!
//! Std-only. Never imports scene or editor types.

use super::checks::relation_cycle_finding;
use super::index::{pos_known, size_known, to_cm};
use super::navcheck::{DOOR_DOUBLE_CM, DOOR_FRAME_CM};
use super::place::GRID_CM;
use super::relation::{cycle_residuals_cm, smallest_cycle, Graph, Relation, SUPPORT_GAP_CM};
use super::staged::{check_staged, StagedActor};
use super::{Actor, Finding, Report};
use engine_core::Fnv64;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

/// World half-extent in cm. Mirrors the `bounds` red line: centers past
/// this refuse instead of nudging further out.
const WORLD_BOUND_CM: i32 = 10_000;
/// Outer yellow line. Clamp repairs target this so one nudge clears both
/// `bounds` and `oob99`.
const OOB99_CM: i32 = 9_900;
/// Largest single dimension in cm. Mirrors the `giantObject` red line and
/// caps [`Repair::size_cm`] payloads.
const GIANT_CM: i32 = 5_000;
/// Fallback full-extent size in cm for unknown or degenerate dimensions.
const DEFAULT_SIZE_CM: i32 = 100;
/// Proposed door leaf width in cm: clears both the single-file red line and
/// the two-way yellow line once the frame bite is subtracted.
const PROPOSED_DOOR_CM: i32 = DOOR_DOUBLE_CM + DOOR_FRAME_CM;

/// What the client should stage for one finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairKind {
    /// Drop to the ground plane and snap XZ to the [`GRID_CM`] grid.
    SnapToGrid,
    /// Push along the smallest-penetration axis (overlap) or clamp back
    /// inside the world (bounds).
    NudgeOut,
    /// Replace full-extent sizes via [`Repair::size_cm`]; positions still move
    /// by [`Repair::delta_cm`].
    ResizeCollider,
    /// Widen the door leaf via [`Repair::door_width_cm`]; geometry is
    /// untouched, the client owns the door bake entry.
    WidenDoor,
    /// No geometry proposal: velocity, naming, hierarchy or nav-bake issues
    /// the client fixes by hand and rechecks. Preview is a no-op.
    RecheckOnly,
}

/// One revision-bound repair description. `finding_id` is the index of the
/// finding in the slice passed to [`suggest`]; `base_revision` is the scene
/// revision the deltas were computed against. `size_cm` is set only for
/// [`RepairKind::ResizeCollider`], `door_width_cm` only for
/// [`RepairKind::WidenDoor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repair {
    pub finding_id: u64,
    pub kind: RepairKind,
    pub target_id: u32,
    pub delta_cm: [i32; 3],
    pub yaw_deci_deg: i32,
    pub size_cm: Option<[i32; 3]>,
    pub door_width_cm: Option<i32>,
    pub base_revision: u64,
}

/// A refused proposal: human-readable code plus the sorted ids the client
/// must resolve by hand. Carries no scene delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredRefusal {
    pub reason: String,
    pub conflict_set: Vec<u32>,
}

/// Per-finding outcome of [`suggest`]: either a staged repair or a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proposal {
    Repair(Repair),
    Refused(StructuredRefusal),
}

/// Preview of one staged repair: full-fidelity staged actors, the adapter
/// rows for the client, the staged-vs-before delta and whether the postchecks
/// pass (delta empty). `Err` means refused: no staged set exists.
#[derive(Debug, Clone)]
pub struct StagedPreview {
    pub staged: Vec<Actor>,
    pub rows: Vec<StagedActor>,
    pub delta: Report,
    pub passed: bool,
}

fn refusal(reason: String, mut conflict_set: Vec<u32>) -> Proposal {
    conflict_set.sort_unstable();
    conflict_set.dedup();
    Proposal::Refused(StructuredRefusal {
        reason,
        conflict_set,
    })
}

fn repair(
    finding_id: u64,
    kind: RepairKind,
    target_id: u32,
    delta_cm: [i32; 3],
    revision: u64,
) -> Repair {
    Repair {
        finding_id,
        kind,
        target_id,
        delta_cm,
        yaw_deci_deg: 0,
        size_cm: None,
        door_width_cm: None,
        base_revision: revision,
    }
}

fn by_id(actors: &[Actor]) -> BTreeMap<u32, &Actor> {
    actors.iter().map(|a| (a.id, a)).collect()
}

/// Snap a cm coordinate to the [`GRID_CM`] grid. Float round is IEEE-exact
/// and order-free; grid math never enters a hashed state.
fn snap_grid(v: i32) -> i32 {
    (v as f32 / GRID_CM as f32).round() as i32 * GRID_CM
}

/// XZ footprint in cm. `None` when the actor has no usable box.
fn footprint(a: &Actor) -> Option<(i32, i32, i32, i32)> {
    if !pos_known(a) || !size_known(a) {
        return None;
    }
    let hx = to_cm(a.sx).abs() / 2;
    let hz = to_cm(a.sz).abs() / 2;
    let cx = to_cm(a.x);
    let cz = to_cm(a.z);
    Some((cx - hx, cx + hx, cz - hz, cz + hz))
}

fn suggest_one(
    finding: &Finding,
    finding_id: u64,
    actors: &BTreeMap<u32, &Actor>,
    revision: u64,
) -> Proposal {
    match finding.check {
        "overlap" => suggest_nudge(finding, finding_id, actors, revision),
        "floating" | "support" | "sunken" => suggest_ground(finding, finding_id, actors, revision),
        "bounds" | "oob99" => suggest_clamp(finding, finding_id, actors, revision),
        "scale" | "giantObject" => suggest_resize(finding, finding_id, actors, revision),
        "doorClear" => suggest_door(finding, finding_id, actors, revision),
        "unknown-position" => refusal(
            format!("unknown-position: id {} has no finite position", finding.a),
            vec![finding.a],
        ),
        _ if super::SPATIAL_CHECKS.contains(&finding.check) => {
            suggest_recheck(finding, finding_id, actors, revision)
        }
        _ => refusal(
            format!(
                "unknown-check: '{}' is not in the spatial catalog",
                finding.check
            ),
            vec![finding.a],
        ),
    }
}

/// Overlap: move the higher id along the smallest-penetration axis, away
/// from the other center, by penetration + 1 cm. Past the world bound the
/// nudge is impossible and refuses instead of leaving the world.
fn suggest_nudge(
    finding: &Finding,
    finding_id: u64,
    actors: &BTreeMap<u32, &Actor>,
    revision: u64,
) -> Proposal {
    let (Some(a), Some(b)) = (actors.get(&finding.a), actors.get(&finding.b)) else {
        let mut missing = Vec::new();
        if !actors.contains_key(&finding.a) {
            missing.push(finding.a);
        }
        if !actors.contains_key(&finding.b) {
            missing.push(finding.b);
        }
        return refusal(
            format!("unknown-actor: overlap pair {missing:?} missing"),
            missing,
        );
    };
    let (Some((ax0, ax1, az0, az1)), Some((bx0, bx1, bz0, bz1))) = (footprint(a), footprint(b))
    else {
        return refusal(
            format!(
                "unknown-geometry: overlap pair ({}, {}) has no usable box",
                a.id, b.id
            ),
            vec![a.id, b.id],
        );
    };
    let pen_x = ax1.min(bx1) - ax0.max(bx0);
    let pen_z = az1.min(bz1) - az0.max(bz0);
    if pen_x <= 0 || pen_z <= 0 {
        // Finding no longer holds against current geometry: nothing to move.
        return Proposal::Repair(repair(
            finding_id,
            RepairKind::RecheckOnly,
            b.id,
            [0, 0, 0],
            revision,
        ));
    }
    let bcx = to_cm(b.x);
    let bcz = to_cm(b.z);
    let delta = if pen_x <= pen_z {
        let dir = if bcx >= to_cm(a.x) { 1 } else { -1 };
        [(pen_x + 1) * dir, 0, 0]
    } else {
        let dir = if bcz >= to_cm(a.z) { 1 } else { -1 };
        [0, 0, (pen_z + 1) * dir]
    };
    if (bcx + delta[0]).abs() > WORLD_BOUND_CM || (bcz + delta[2]).abs() > WORLD_BOUND_CM {
        return refusal(
            format!("impossible-nudge: id {} would exit the world bound", b.id),
            vec![a.id, b.id],
        );
    }
    Proposal::Repair(repair(
        finding_id,
        RepairKind::NudgeOut,
        b.id,
        delta,
        revision,
    ))
}

/// Floating, support, sunken: ground the actor (`min_y = 0`) and snap XZ to
/// the grid. A zero delta means the finding is stale: recheck instead.
fn suggest_ground(
    finding: &Finding,
    finding_id: u64,
    actors: &BTreeMap<u32, &Actor>,
    revision: u64,
) -> Proposal {
    let Some(t) = actors.get(&finding.a) else {
        return refusal(
            format!("unknown-actor: id {} missing", finding.a),
            vec![finding.a],
        );
    };
    if !pos_known(t) {
        return refusal(
            format!("unknown-position: id {} has no finite position", t.id),
            vec![t.id],
        );
    }
    if !size_known(t) || t.sy <= 0.0 {
        return refusal(
            format!("unknown-geometry: id {} has no usable height", t.id),
            vec![t.id],
        );
    }
    let (cx, cy, cz) = (to_cm(t.x), to_cm(t.y), to_cm(t.z));
    let delta = [
        snap_grid(cx) - cx,
        to_cm(t.sy).abs() / 2 - cy,
        snap_grid(cz) - cz,
    ];
    let kind = if delta == [0, 0, 0] {
        RepairKind::RecheckOnly
    } else {
        RepairKind::SnapToGrid
    };
    Proposal::Repair(repair(finding_id, kind, t.id, delta, revision))
}

/// Bounds, oob99: clamp each center axis into the yellow line. Already inside
/// means a stale finding: recheck instead of emitting a no-op nudge.
fn suggest_clamp(
    finding: &Finding,
    finding_id: u64,
    actors: &BTreeMap<u32, &Actor>,
    revision: u64,
) -> Proposal {
    let Some(t) = actors.get(&finding.a) else {
        return refusal(
            format!("unknown-actor: id {} missing", finding.a),
            vec![finding.a],
        );
    };
    if !pos_known(t) {
        return refusal(
            format!("unknown-position: id {} has no finite position", t.id),
            vec![t.id],
        );
    }
    let cur = [to_cm(t.x), to_cm(t.y), to_cm(t.z)];
    let clamped = cur.map(|v| v.clamp(-OOB99_CM, OOB99_CM));
    let delta = [
        clamped[0] - cur[0],
        clamped[1] - cur[1],
        clamped[2] - cur[2],
    ];
    let kind = if delta == [0, 0, 0] {
        RepairKind::RecheckOnly
    } else {
        RepairKind::NudgeOut
    };
    Proposal::Repair(repair(finding_id, kind, t.id, delta, revision))
}

/// Scale, giantObject: clamp each offending dimension (non-finite or
/// non-positive to 1 m, over the giant line down to it), keep sane dims, and
/// ground the result. Missing actors refuse; NaN geometry is repairable here
/// because the payload carries full replacement sizes.
fn suggest_resize(
    finding: &Finding,
    finding_id: u64,
    actors: &BTreeMap<u32, &Actor>,
    revision: u64,
) -> Proposal {
    let Some(t) = actors.get(&finding.a) else {
        return refusal(
            format!("unknown-actor: id {} missing", finding.a),
            vec![finding.a],
        );
    };
    let fix = |v: f32| -> i32 {
        if !v.is_finite() || v <= 0.0 {
            DEFAULT_SIZE_CM
        } else {
            to_cm(v).abs().clamp(1, GIANT_CM)
        }
    };
    let size_cm = [fix(t.sx), fix(t.sy), fix(t.sz)];
    let base = |v: f32| -> i32 { snap_grid(if v.is_finite() { to_cm(v) } else { 0 }) };
    let cur = [
        if t.x.is_finite() { to_cm(t.x) } else { 0 },
        if t.y.is_finite() { to_cm(t.y) } else { 0 },
        if t.z.is_finite() { to_cm(t.z) } else { 0 },
    ];
    let want = [base(t.x), size_cm[1] / 2, base(t.z)];
    let delta = [want[0] - cur[0], want[1] - cur[1], want[2] - cur[2]];
    let mut out = repair(
        finding_id,
        RepairKind::ResizeCollider,
        t.id,
        delta,
        revision,
    );
    out.size_cm = Some(size_cm);
    Proposal::Repair(out)
}

/// Door clearance: the leaf width lives in the nav bake, not in actor
/// geometry, so the preview moves nothing. The client widens the bake entry
/// to [`Repair::door_width_cm`] and re-runs the nav checks.
fn suggest_door(
    finding: &Finding,
    finding_id: u64,
    actors: &BTreeMap<u32, &Actor>,
    revision: u64,
) -> Proposal {
    let Some(t) = actors.get(&finding.a) else {
        return refusal(
            format!("unknown-actor: door id {} missing", finding.a),
            vec![finding.a],
        );
    };
    if !pos_known(t) {
        return refusal(
            format!("unknown-position: door id {} has no finite position", t.id),
            vec![t.id],
        );
    }
    let mut out = repair(finding_id, RepairKind::WidenDoor, t.id, [0, 0, 0], revision);
    out.door_width_cm = Some(PROPOSED_DOOR_CM);
    Proposal::Repair(out)
}

/// Velocity, naming, hierarchy, density and nav-connectivity findings carry no
/// geometry delta: the client fixes them by hand (rename, reparent, velocity,
/// bake) and rechecks. The preview is a no-op that postchecks clean.
fn suggest_recheck(
    finding: &Finding,
    finding_id: u64,
    actors: &BTreeMap<u32, &Actor>,
    revision: u64,
) -> Proposal {
    if !actors.contains_key(&finding.a) {
        return refusal(
            format!("unknown-actor: id {} missing", finding.a),
            vec![finding.a],
        );
    }
    Proposal::Repair(repair(
        finding_id,
        RepairKind::RecheckOnly,
        finding.a,
        [0, 0, 0],
        revision,
    ))
}

/// One proposal per finding, in findings order. Each seeded finding yields a
/// valid repair or a structured refusal; the caller's actors are never
/// mutated. `revision` stamps every repair's `base_revision`.
pub fn suggest(findings: &[Finding], actors: &[Actor], revision: u64) -> Vec<Proposal> {
    let table = by_id(actors);
    findings
        .iter()
        .enumerate()
        .map(|(i, f)| suggest_one(f, i as u64, &table, revision))
        .collect()
}

/// Decidegrees of yaw to radians. Plain multiply, no libm call.
fn deci_to_rad(deci: i32) -> f32 {
    deci as f32 * core::f32::consts::PI / 1800.0
}

fn cm_to_m(v: i32) -> f32 {
    v as f32 / 100.0
}

fn stale_revision(repair: &Repair, revision: u64) -> StructuredRefusal {
    StructuredRefusal {
        reason: format!(
            "stale-revision: base {} != current {revision}",
            repair.base_revision
        ),
        conflict_set: vec![repair.target_id],
    }
}

/// Preview what the client stages for one repair: apply the deltas to a copy
/// of `actors`, then run the staged gate (`before` vs staged) as the
/// postcheck. `passed` is true when the delta is empty. Stale revisions,
/// missing targets, malformed payloads and impossible geometry refuse with no
/// staged set: the caller's scene never changes.
pub fn apply_preview(
    repair: &Repair,
    actors: &[Actor],
    revision: u64,
) -> Result<StagedPreview, StructuredRefusal> {
    if revision != repair.base_revision {
        return Err(stale_revision(repair, revision));
    }
    let Some(t) = actors.iter().find(|a| a.id == repair.target_id) else {
        return Err(StructuredRefusal {
            reason: format!("unknown-target: id {} missing", repair.target_id),
            conflict_set: vec![repair.target_id],
        });
    };
    // Only a resize carries full replacement geometry, so only it tolerates
    // a non-finite target position. Every other kind needs known geometry to
    // move relative to.
    if !pos_known(t) && repair.kind != RepairKind::ResizeCollider {
        return Err(StructuredRefusal {
            reason: format!("unknown-position: id {} has no finite position", t.id),
            conflict_set: vec![t.id],
        });
    }
    if repair.kind == RepairKind::ResizeCollider {
        match repair.size_cm {
            Some(s) if s.iter().all(|v| (1..=GIANT_CM).contains(v)) => {}
            _ => {
                return Err(StructuredRefusal {
                    reason: format!(
                        "malformed-repair: id {} resize needs sizes in 1..={GIANT_CM} cm",
                        t.id
                    ),
                    conflict_set: vec![t.id],
                });
            }
        }
    }
    if repair.kind == RepairKind::WidenDoor && repair.door_width_cm.is_none() {
        return Err(StructuredRefusal {
            reason: format!("malformed-repair: id {} widen needs a door width", t.id),
            conflict_set: vec![t.id],
        });
    }
    if repair.yaw_deci_deg != 0 && !t.yaw.is_finite() {
        return Err(StructuredRefusal {
            reason: format!("unknown-orientation: id {} yaw is not finite", t.id),
            conflict_set: vec![t.id],
        });
    }

    let base = |v: f32| -> i32 {
        if v.is_finite() {
            to_cm(v)
        } else {
            0
        }
    };
    let mut out = t.clone();
    out.x = cm_to_m(base(t.x) + repair.delta_cm[0]);
    out.y = cm_to_m(base(t.y) + repair.delta_cm[1]);
    out.z = cm_to_m(base(t.z) + repair.delta_cm[2]);
    if repair.yaw_deci_deg != 0 {
        out.yaw = t.yaw + deci_to_rad(repair.yaw_deci_deg);
    }
    if let Some(s) = repair.size_cm {
        out.sx = cm_to_m(s[0]);
        out.sy = cm_to_m(s[1]);
        out.sz = cm_to_m(s[2]);
    }

    let mut staged: Vec<Actor> = actors
        .iter()
        .map(|a| {
            if a.id == out.id {
                out.clone()
            } else {
                a.clone()
            }
        })
        .collect();
    staged.sort_by_key(|a| a.id);
    let rows: Vec<StagedActor> = staged
        .iter()
        .map(|a| StagedActor::new(a.id, &a.name, a.x, a.y, a.z, a.sx, a.sy, a.sz, a.physical))
        .collect();
    let after = super::staged::staged_to_actors(&rows);
    let delta = check_staged(actors, &after);
    let passed = delta.findings.is_empty();
    Ok(StagedPreview {
        staged,
        rows,
        delta,
        passed,
    })
}

/// True when `ids` is a non-empty strictly increasing conflict set.
///
/// Consumer: proposal auditors and the staged gate read the shape beside a
/// refusal. Units are actor ids; determinism is order-free by construction
/// (sorted input reads true, any other order reads false). No error case:
/// `false` means reshape by sorting and deduping. Never mutates the slice
/// and uses integer math only.
pub fn audit_conflict_ids(ids: &[u32]) -> bool {
    if ids.is_empty() {
        return false;
    }
    let mut i = 1;
    while i < ids.len() {
        if ids[i - 1] >= ids[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Integer-only proposal audit beside [`suggest`].
///
/// Consumer: the approved-repair flow checks one proposal per finding before
/// preview. Units are finding indices and revisions; determinism follows the
/// findings order in any actor order. Errors: `false` on any shape miss
/// (count, stamping or refusal order); the caller keeps its actors. Never
/// mutates the slices and never touches placement scalars: only counts, ids,
/// revisions and refusal bytes.
pub fn audit_proposals(expected: usize, proposals: &[Proposal], revision: u64) -> bool {
    if proposals.len() != expected {
        return false;
    }
    let mut i = 0;
    while i < proposals.len() {
        match &proposals[i] {
            Proposal::Repair(r) => {
                let Ok(want) = u64::try_from(i) else {
                    return false;
                };
                if r.finding_id != want || r.base_revision != revision {
                    return false;
                }
            }
            Proposal::Refused(e) => {
                if e.reason.is_empty() || !audit_conflict_ids(&e.conflict_set) {
                    return false;
                }
            }
        }
        i += 1;
    }
    true
}

/// Two marked proposal lanes from one brief input.
///
/// Consumer: the bounded repair loop runs both lanes and keeps the agreeing
/// form before touching geometry. The caller owns the data: input is findings
/// plus actors, output is two proposal lists marked by lane position (first
/// and second). Units are finding indices and revisions. Determinism follows
/// the findings order in any actor order: the second lane replays the same
/// inputs from a reversed actor copy, so a hidden order dependence shows as
/// a lane mismatch. Pure, integer, sorted through the proposal path, no clock.
/// Refusals pass through untouched and the caller scene never changes.
pub fn dual_propose(
    findings: &[Finding],
    actors: &[Actor],
    revision: u64,
) -> (Vec<Proposal>, Vec<Proposal>) {
    let first = suggest(findings, actors, revision);
    let mut flipped = actors.to_vec();
    flipped.reverse();
    let second = suggest(findings, &flipped, revision);
    (first, second)
}

/// Agreement gate over two proposal lanes.
///
/// Consumer: the bounded repair loop feeds both lanes here before staging.
/// Units are lane positions. Each entry is read twice per lane (two clones)
/// and kept only when both reads agree with each other and across lanes, so
/// a flaky decode drops the entry instead of repairing from it. Pure,
/// integer, order preserving, no clock. An empty lane or a length mismatch
/// keeps only the shared agreeing prefix.
pub fn self_consistent_decode(first: &[Proposal], second: &[Proposal]) -> Vec<Proposal> {
    let mut kept = Vec::new();
    let mut i = 0;
    while i < first.len() && i < second.len() {
        let read_a = first[i].clone();
        let read_b = first[i].clone();
        let read_c = second[i].clone();
        let read_d = second[i].clone();
        if read_a == read_b && read_c == read_d && read_a == read_c {
            kept.push(read_a);
        }
        i += 1;
    }
    kept
}

/// One engine repair round: stage a single proposal.
///
/// Consumer: the bounded loop calls this with a round counter. Units are actor
/// ids and revisions, with the counter capping the loop at three rounds.
/// Applies the first previewable repair from [`suggest`] and returns the fresh
/// sorted staged actors. Returns `None` when the round index reaches the cap
/// or when no repair previews cleanly (refusals only). Pure, integer, sorted
/// through the proposal path, no clock. The caller scene never changes.
pub fn repair_round(
    actors: &[Actor],
    findings: &[Finding],
    revision: u64,
    round: u32,
) -> Option<Vec<Actor>> {
    if round >= 3 {
        return None;
    }
    let proposals = suggest(findings, actors, revision);
    let mut index = 0;
    while index < proposals.len() {
        if let Proposal::Repair(repair) = &proposals[index] {
            if let Ok(preview) = apply_preview(repair, actors, revision) {
                return Some(preview.staged);
            }
        }
        index += 1;
    }
    None
}

/// One cycle fixed by [`repair_cycles`]: the smallest cycle named in support-up
/// form (smallest id first), the integer-cm residual of every measurable leg,
/// and the one leg whose claims were dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleStep {
    pub cycle: Vec<u32>,
    pub residuals_cm: Vec<(u32, u32, i32)>,
    pub dropped: (u32, u32),
}

/// Receipt of a cycle repair: the steps in order, whether the re-check of the
/// repaired graph found no cycle, and an FNV-1a digest over the repaired graph
/// plus the steps, equal for equal inputs on every platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleReceipt {
    pub steps: Vec<CycleStep>,
    pub clean: bool,
    pub digest: u64,
}

/// Refusal for a proposed graph that holds a support cycle: names the smallest
/// cycle as `a>b>c` and carries its ids as the sorted conflict set. `None` for
/// an acyclic graph.
pub fn cycle_refusal(graph: &Graph) -> Option<StructuredRefusal> {
    let cycle = smallest_cycle(graph)?;
    let named: Vec<String> = cycle.iter().map(|id| id.to_string()).collect();
    Some(StructuredRefusal {
        reason: format!("relation-cycle: {}", named.join(">")),
        conflict_set: cycle
            .into_iter()
            .collect::<BTreeSet<u32>>()
            .into_iter()
            .collect(),
    })
}

/// How far a leg's residual sits outside its healthy band, in cm; a leg the
/// actors cannot measure counts as fully ungrounded.
fn leg_badness(graph: &Graph, residuals: &[(u32, u32, i32)], from: u32, to: u32) -> i32 {
    let Some((_, _, r)) = residuals.iter().find(|(a, b, _)| *a == from && *b == to) else {
        return i32::MAX;
    };
    let inside_only = !graph.edges.contains(&(from, Relation::Supports, to))
        && !graph.edges.contains(&(to, Relation::OnTopOf, from));
    if inside_only {
        (-*r).max(0)
    } else {
        (-*r).max(*r - SUPPORT_GAP_CM).max(0)
    }
}

/// Repair every support cycle of a proposed relation graph and re-check it.
///
/// Consumer: the approved-repair flow, beside SP28 detection. While a cycle
/// remains, the smallest one is named and the leg whose measured residual is
/// farthest outside its healthy band loses its claims (ties go to the smaller
/// id pair); unmeasurable legs go first. Integer only; the same edge set in any
/// order gives the same steps and digest. The caller's graph is never changed:
/// the repaired copy comes back with the receipt, whose `clean` re-runs
/// `smallest_cycle` on that copy.
pub fn repair_cycles(actors: &[Actor], graph: &Graph) -> (Graph, CycleReceipt) {
    let mut out = graph.clone();
    let mut steps = Vec::new();
    while let Some(cycle) = smallest_cycle(&out) {
        let residuals_cm = cycle_residuals_cm(actors, &out, &cycle);
        let legs = cycle
            .iter()
            .enumerate()
            .map(|(i, from)| (*from, cycle[(i + 1) % cycle.len()]));
        let (from, to) = legs
            .max_by_key(|(f, t)| (leg_badness(&out, &residuals_cm, *f, *t), Reverse((*f, *t))))
            .unwrap_or((cycle[0], cycle[0]));
        out.edges.retain(|e| {
            *e != (from, Relation::Supports, to)
                && *e != (to, Relation::OnTopOf, from)
                && *e != (from, Relation::Inside, to)
        });
        steps.push(CycleStep {
            cycle,
            residuals_cm,
            dropped: (from, to),
        });
    }
    let mut h = Fnv64::new();
    h.write_u64(super::relation::digest(&out));
    for s in &steps {
        for id in &s.cycle {
            h.write_u32(*id);
        }
        h.write_u32(s.dropped.0);
        h.write_u32(s.dropped.1);
    }
    let clean = relation_cycle_finding(&out).is_none();
    let digest = h.finish();
    (
        out,
        CycleReceipt {
            steps,
            clean,
            digest,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resting(id: u32, name: &str, x: f32, z: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y: 0.5,
            z,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn repair_of(p: &Proposal) -> &Repair {
        match p {
            Proposal::Repair(r) => r,
            Proposal::Refused(e) => panic!("expected repair, got refusal {e:?}"),
        }
    }

    fn refused_of(p: &Proposal) -> &StructuredRefusal {
        match p {
            Proposal::Refused(e) => e,
            Proposal::Repair(r) => panic!("expected refusal, got repair {r:?}"),
        }
    }

    #[test]
    fn overlap_nudge_repairs_and_postcheck_passes() {
        let actors = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
        ];
        let report = super::super::run(&actors);
        assert_eq!(report.count("overlap"), 1);
        let proposals = suggest(&report.findings, &actors, 3);
        assert_eq!(proposals.len(), 1);
        let r = repair_of(&proposals[0]);
        assert_eq!(r.kind, RepairKind::NudgeOut);
        assert_eq!(r.target_id, 1);
        assert_eq!(r.base_revision, 3);
        assert_eq!(r.finding_id, 0);
        assert!(r.delta_cm[0] != 0 || r.delta_cm[2] != 0);
        let preview = apply_preview(r, &actors, 3).expect("preview must apply");
        assert!(
            preview.passed,
            "postcheck delta: {:?}",
            preview.delta.findings
        );
        assert_eq!(preview.rows.len(), 2);
        // The staged target moved by exactly the delta; the other actor did not.
        let moved = preview.staged.iter().find(|a| a.id == 1).unwrap();
        let still = preview.staged.iter().find(|a| a.id == 0).unwrap();
        assert_eq!(to_cm(moved.x) - to_cm(0.5), r.delta_cm[0]);
        assert_eq!((still.x, still.z), (0.0, 0.0));
    }

    #[test]
    fn floating_snap_repairs_and_passes() {
        let mut high = resting(0, "torch_01", 1.03, 2.07);
        high.y = 5.0;
        let actors = vec![high];
        let report = super::super::run(&actors);
        assert!(report.count("floating") >= 1);
        let finding = report
            .findings
            .iter()
            .find(|f| f.check == "floating")
            .unwrap()
            .clone();
        let proposals = suggest(std::slice::from_ref(&finding), &actors, 7);
        let r = repair_of(&proposals[0]);
        assert_eq!(r.kind, RepairKind::SnapToGrid);
        let preview = apply_preview(r, &actors, 7).expect("preview must apply");
        assert!(
            preview.passed,
            "postcheck delta: {:?}",
            preview.delta.findings
        );
        let moved = &preview.staged[0];
        assert_eq!(to_cm(moved.y) - to_cm(moved.sy).abs() / 2, 0);
        assert_eq!(to_cm(moved.x) % GRID_CM, 0);
    }

    #[test]
    fn bounds_clamp_repairs_and_passes() {
        let actors = vec![resting(0, "far_prop", 150.0, -150.0)];
        let report = super::super::run(&actors);
        assert_eq!(report.count("bounds"), 1);
        let finding = report
            .findings
            .iter()
            .find(|f| f.check == "bounds")
            .unwrap()
            .clone();
        let proposals = suggest(std::slice::from_ref(&finding), &actors, 1);
        let r = repair_of(&proposals[0]);
        assert_eq!(r.kind, RepairKind::NudgeOut);
        let preview = apply_preview(r, &actors, 1).expect("preview must apply");
        assert!(
            preview.passed,
            "postcheck delta: {:?}",
            preview.delta.findings
        );
        let moved = &preview.staged[0];
        assert!(to_cm(moved.x).abs() <= OOB99_CM);
        assert!(to_cm(moved.z).abs() <= OOB99_CM);
    }

    #[test]
    fn door_widen_proposes_width_without_moving() {
        let actors = vec![resting(7, "door_main", 5.0, 5.0)];
        let finding = Finding {
            check: "doorClear",
            a: 7,
            b: u32::MAX,
            pct: 0.0,
            severity: "red",
        };
        let proposals = suggest(std::slice::from_ref(&finding), &actors, 2);
        let r = repair_of(&proposals[0]);
        assert_eq!(r.kind, RepairKind::WidenDoor);
        assert_eq!(r.delta_cm, [0, 0, 0]);
        assert_eq!(r.door_width_cm, Some(PROPOSED_DOOR_CM));
        assert!(r.door_width_cm.unwrap() - DOOR_FRAME_CM >= DOOR_DOUBLE_CM);
        let preview = apply_preview(r, &actors, 2).expect("preview must apply");
        assert!(preview.passed);
        let moved = preview.staged.iter().find(|a| a.id == 7).unwrap();
        assert_eq!((moved.x, moved.y, moved.z), (5.0, 0.5, 5.0));
    }

    #[test]
    fn scale_resize_repairs_and_passes() {
        let giant = Actor {
            sx: 60.0,
            sy: 60.0,
            sz: 60.0,
            y: 30.0,
            ..resting(0, "hall_01", 0.0, 0.0)
        };
        let actors = vec![giant];
        let report = super::super::run(&actors);
        assert_eq!(report.count("giantObject"), 1);
        let finding = report
            .findings
            .iter()
            .find(|f| f.check == "giantObject")
            .unwrap()
            .clone();
        let proposals = suggest(std::slice::from_ref(&finding), &actors, 4);
        let r = repair_of(&proposals[0]);
        assert_eq!(r.kind, RepairKind::ResizeCollider);
        let sizes = r.size_cm.expect("resize carries sizes");
        assert!(sizes.iter().all(|v| (1..=GIANT_CM).contains(v)));
        let preview = apply_preview(r, &actors, 4).expect("preview must apply");
        assert!(
            preview.passed,
            "postcheck delta: {:?}",
            preview.delta.findings
        );
    }

    #[test]
    fn unknown_position_refuses_with_conflict() {
        let nan = Actor {
            x: f32::NAN,
            ..resting(0, "lost_prop", 0.0, 0.0)
        };
        let actors = vec![nan];
        let report = super::super::run(&actors);
        assert_eq!(report.count("unknown-position"), 1);
        let proposals = suggest(&report.findings, &actors, 9);
        assert_eq!(proposals.len(), 1);
        let e = refused_of(&proposals[0]);
        assert!(e.reason.contains("unknown-position"));
        assert_eq!(e.conflict_set, vec![0]);
    }

    #[test]
    fn unknown_actor_refuses() {
        let actors = vec![resting(0, "crate_a", 0.0, 0.0)];
        let finding = Finding {
            check: "overlap",
            a: 0,
            b: 999,
            pct: 50.0,
            severity: "red",
        };
        let proposals = suggest(std::slice::from_ref(&finding), &actors, 1);
        let e = refused_of(&proposals[0]);
        assert!(e.reason.contains("unknown-actor"));
        assert_eq!(e.conflict_set, vec![999]);
    }

    #[test]
    fn revision_mismatch_refuses_and_changes_nothing() {
        let actors = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
        ];
        let report = super::super::run(&actors);
        let proposals = suggest(&report.findings, &actors, 7);
        let r = repair_of(&proposals[0]).clone();
        let before = actors.clone();
        let err = apply_preview(&r, &actors, 8).expect_err("stale revision must refuse");
        assert!(err.reason.contains("stale-revision"));
        assert_eq!(err.conflict_set, vec![r.target_id]);
        assert_eq!(actors, before, "refused preview changes nothing");
    }

    #[test]
    fn impossible_nudge_at_world_edge_refuses() {
        // Pair overlapping at the +X world edge: pushing the higher id clear
        // would exit the world, so the finding refuses instead.
        let edge_a = Actor {
            x: 99.4,
            ..resting(0, "edge_a", 0.0, 0.0)
        };
        let edge_b = Actor {
            x: 99.8,
            ..resting(1, "edge_b", 0.0, 0.0)
        };
        let actors = vec![edge_a, edge_b];
        let report = super::super::run(&actors);
        assert_eq!(report.count("overlap"), 1);
        let proposals = suggest(&report.findings, &actors, 1);
        let e = refused_of(&proposals[0]);
        assert!(e.reason.contains("impossible-nudge"));
        assert_eq!(e.conflict_set, vec![0, 1]);
    }

    #[test]
    fn recheck_only_preview_passes_without_moving() {
        let actors = vec![resting(0, "Bad Name!", 0.0, 0.0)];
        let report = super::super::run(&actors);
        assert_eq!(report.count("grammar"), 1);
        let finding = report
            .findings
            .iter()
            .find(|f| f.check == "grammar")
            .unwrap()
            .clone();
        let proposals = suggest(std::slice::from_ref(&finding), &actors, 1);
        let r = repair_of(&proposals[0]);
        assert_eq!(r.kind, RepairKind::RecheckOnly);
        assert_eq!(r.delta_cm, [0, 0, 0]);
        let preview = apply_preview(r, &actors, 1).expect("preview must apply");
        assert!(preview.passed);
        assert_eq!(preview.staged, actors);
    }

    /// Every catalog check yields either a well-formed repair or a structured
    /// refusal: repairs target a known actor at the right finding index and
    /// revision, refusals carry a reason and a non-empty conflict set.
    #[test]
    fn every_check_kind_yields_repair_or_refusal() {
        let mut dupe: Vec<Actor> = (0..11)
            .map(|i| resting(i, "crate", f32::from(i as u8) * 0.3, 40.0))
            .collect();
        for a in dupe.iter_mut() {
            a.y = 0.5;
        }
        let mut dense: Vec<Actor> = (0..9)
            .map(|i| resting(100 + i, "pebble", f32::from(i as u8) * 0.1, 60.0))
            .collect();
        for a in dense.iter_mut() {
            a.y = 0.5;
        }
        let mover = Actor {
            vx: 2.0,
            ..resting(200, "mover", 0.0, 80.0)
        };
        let wall = resting(201, "wall", 1.5, 80.0);
        let seeded: Vec<(Finding, Vec<Actor>)> = vec![
            (
                Finding {
                    check: "overlap",
                    a: 0,
                    b: 1,
                    pct: 50.0,
                    severity: "red",
                },
                vec![
                    resting(0, "crate_a", 0.0, 0.0),
                    resting(1, "crate_b", 0.5, 0.0),
                ],
            ),
            (
                Finding {
                    check: "unknown-position",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![Actor {
                    x: f32::NAN,
                    ..resting(0, "lost", 0.0, 0.0)
                }],
            ),
            (
                Finding {
                    check: "floating",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![Actor {
                    y: 5.0,
                    ..resting(0, "fly", 0.0, 0.0)
                }],
            ),
            (
                Finding {
                    check: "bounds",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "red",
                },
                vec![resting(0, "far", 150.0, 150.0)],
            ),
            (
                Finding {
                    check: "sunken",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![Actor {
                    y: -5.0,
                    ..resting(0, "sunk", 0.0, 0.0)
                }],
            ),
            (
                Finding {
                    check: "scale",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "red",
                },
                vec![Actor {
                    sx: 0.0,
                    ..resting(0, "flat", 0.0, 0.0)
                }],
            ),
            (
                Finding {
                    check: "orphan",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![Actor {
                    parent: Some(999),
                    ..resting(0, "child", 0.0, 0.0)
                }],
            ),
            (
                Finding {
                    check: "giantObject",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![Actor {
                    sx: 60.0,
                    sy: 60.0,
                    sz: 60.0,
                    y: 30.0,
                    ..resting(0, "hall", 0.0, 0.0)
                }],
            ),
            (
                Finding {
                    check: "support",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![Actor {
                    y: 5.0,
                    ..resting(0, "hover", 0.0, 0.0)
                }],
            ),
            (
                Finding {
                    check: "dupeSpam",
                    a: 1,
                    b: 0,
                    pct: 0.0,
                    severity: "red",
                },
                dupe,
            ),
            (
                Finding {
                    check: "oob99",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![resting(0, "rim", 99.5, 0.0)],
            ),
            (
                Finding {
                    check: "sweep",
                    a: 200,
                    b: 201,
                    pct: 10.0,
                    severity: "yellow",
                },
                vec![mover, wall],
            ),
            (
                Finding {
                    check: "overlapDensity",
                    a: 100,
                    b: u32::MAX,
                    pct: 9.0,
                    severity: "yellow",
                },
                dense,
            ),
            (
                Finding {
                    check: "grammar",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![resting(0, "Bad Name!", 0.0, 0.0)],
            ),
            (
                Finding {
                    check: "navConnect",
                    a: 1,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![resting(0, "a", 0.0, 0.0), resting(1, "b", 2.0, 0.0)],
            ),
            (
                Finding {
                    check: "flowPath",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![resting(0, "a", 0.0, 0.0)],
            ),
            (
                Finding {
                    check: "accessFront",
                    a: 0,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "yellow",
                },
                vec![resting(0, "me", 0.0, 0.0), resting(1, "wall", 1.2, 0.0)],
            ),
            (
                Finding {
                    check: "doorClear",
                    a: 7,
                    b: u32::MAX,
                    pct: 0.0,
                    severity: "red",
                },
                vec![resting(7, "door_main", 5.0, 5.0)],
            ),
        ];
        assert_eq!(seeded.len(), super::super::SPATIAL_CHECKS.len());
        for (finding, actors) in &seeded {
            let proposals = suggest(std::slice::from_ref(finding), actors, 11);
            assert_eq!(proposals.len(), 1, "check {}", finding.check);
            match &proposals[0] {
                Proposal::Repair(r) => {
                    assert_eq!(r.finding_id, 0);
                    assert_eq!(r.base_revision, 11);
                    assert!(
                        actors.iter().any(|a| a.id == r.target_id),
                        "check {} targets missing id {}",
                        finding.check,
                        r.target_id
                    );
                    // Every repair previews without error at its own revision;
                    // refusal at a stale revision changes nothing.
                    let before = actors.clone();
                    let preview = apply_preview(r, actors, 11)
                        .unwrap_or_else(|e| panic!("check {}: {:?}", finding.check, e));
                    assert!(!preview.staged.is_empty());
                    assert!(apply_preview(r, actors, 12).is_err());
                    assert_eq!(*actors, before);
                }
                Proposal::Refused(e) => {
                    assert!(!e.reason.is_empty(), "check {}", finding.check);
                    assert!(!e.conflict_set.is_empty(), "check {}", finding.check);
                }
            }
        }
        // Spot-check the kind mapping the Fix-All flow depends on.
        let kind_of =
            |check: &str| -> Option<RepairKind> {
                seeded.iter().find(|(f, _)| f.check == check).and_then(
                    |(f, actors)| match &suggest(std::slice::from_ref(f), actors, 1)[0] {
                        Proposal::Repair(r) => Some(r.kind),
                        Proposal::Refused(_) => None,
                    },
                )
            };
        assert_eq!(kind_of("overlap"), Some(RepairKind::NudgeOut));
        assert_eq!(kind_of("floating"), Some(RepairKind::SnapToGrid));
        assert_eq!(kind_of("bounds"), Some(RepairKind::NudgeOut));
        assert_eq!(kind_of("scale"), Some(RepairKind::ResizeCollider));
        assert_eq!(kind_of("doorClear"), Some(RepairKind::WidenDoor));
        assert_eq!(kind_of("grammar"), Some(RepairKind::RecheckOnly));
        assert_eq!(kind_of("unknown-position"), None);
    }

    #[test]
    fn suggest_is_deterministic_and_order_invariant() {
        let actors = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
            Actor {
                y: 5.0,
                ..resting(2, "fly", 9.0, 9.0)
            },
        ];
        let report = super::super::run(&actors);
        assert!(report.findings.len() >= 2);
        let first = suggest(&report.findings, &actors, 5);
        let second = suggest(&report.findings, &actors, 5);
        assert_eq!(first, second);
        let mut rev = actors.clone();
        rev.reverse();
        assert_eq!(suggest(&report.findings, &rev, 5), first);
        // Previews are deterministic too.
        for p in &first {
            if let Proposal::Repair(r) = p {
                let a = apply_preview(r, &actors, 5).expect("preview");
                let b = apply_preview(r, &rev, 5).expect("preview");
                assert_eq!(a.staged, b.staged);
                assert_eq!(a.delta.findings, b.delta.findings);
            }
        }
    }

    #[test]
    fn unknown_check_refuses() {
        let actors = vec![resting(0, "crate_a", 0.0, 0.0)];
        let finding = Finding {
            check: "nope",
            a: 0,
            b: u32::MAX,
            pct: 0.0,
            severity: "yellow",
        };
        let proposals = suggest(std::slice::from_ref(&finding), &actors, 1);
        let e = refused_of(&proposals[0]);
        assert!(e.reason.contains("unknown-check"));
    }

    #[test]
    fn malformed_resize_refuses() {
        let actors = vec![resting(0, "crate_a", 0.0, 0.0)];
        let bad = Repair {
            finding_id: 0,
            kind: RepairKind::ResizeCollider,
            target_id: 0,
            delta_cm: [0, 0, 0],
            yaw_deci_deg: 0,
            size_cm: None,
            door_width_cm: None,
            base_revision: 1,
        };
        let err = apply_preview(&bad, &actors, 1).expect_err("sizeless resize must refuse");
        assert!(err.reason.contains("malformed-repair"));
        assert_eq!(actors, vec![resting(0, "crate_a", 0.0, 0.0)]);
    }

    #[test]
    fn sp38_repair_audit_shapes_hold() {
        let actors = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
        ];
        let report = super::super::run(&actors);
        assert_eq!(report.count("overlap"), 1);
        let proposals = suggest(&report.findings, &actors, 3);
        assert!(super::audit_proposals(report.findings.len(), &proposals, 3));
        // Failure: stale revision never audits.
        assert!(!super::audit_proposals(
            report.findings.len(),
            &proposals,
            4
        ));
        // Failure: wrong count never audits.
        assert!(!super::audit_proposals(
            report.findings.len() + 1,
            &proposals,
            3
        ));
        // Conflict-id order is strictly increasing.
        assert!(super::audit_conflict_ids(&[7]));
        assert!(!super::audit_conflict_ids(&[]));
        assert!(!super::audit_conflict_ids(&[2, 2]));
        assert!(!super::audit_conflict_ids(&[3, 1]));
        // Refusal path audits too.
        let missing = Finding {
            check: "overlap",
            a: 99,
            b: u32::MAX,
            pct: 0.0,
            severity: "red",
        };
        let refused = suggest(std::slice::from_ref(&missing), &actors, 9);
        assert!(super::audit_proposals(1, &refused, 9));
    }

    fn repair_loop_to_clean(start: &[Actor]) -> (Vec<Actor>, u32) {
        let mut cur = start.to_vec();
        let mut rounds: u32 = 0;
        loop {
            let rep = super::super::run(&cur);
            if rep.findings.is_empty() || rounds >= 3 {
                break;
            }
            let revision: u64 = 3;
            let (lane_a, lane_b) = dual_propose(&rep.findings, &cur, revision);
            let agreed = self_consistent_decode(&lane_a, &lane_b);
            if agreed.len() != rep.findings.len() {
                break;
            }
            match repair_round(&cur, &rep.findings, revision, rounds) {
                Some(next) => {
                    cur = next;
                    rounds = rounds.saturating_add(1);
                }
                None => {
                    break;
                }
            }
        }
        (cur, rounds)
    }

    #[test]
    fn sp43_repair_dual_agree_x2() {
        let actors = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
        ];
        let report = super::super::run(&actors);
        assert_eq!(report.count("overlap"), 1);
        let (a_first, b_first) = dual_propose(&report.findings, &actors, 3);
        let (a_second, b_second) = dual_propose(&report.findings, &actors, 3);
        assert_eq!(a_first, a_second);
        assert_eq!(b_first, b_second);
        assert_eq!(a_first, b_first);
        let decoded_first = self_consistent_decode(&a_first, &b_first);
        let decoded_second = self_consistent_decode(&a_second, &b_second);
        assert_eq!(decoded_first, decoded_second);
        assert_eq!(decoded_first.len(), report.findings.len());
        let (repaired_first, rounds_first) = repair_loop_to_clean(&actors);
        let (repaired_second, rounds_second) = repair_loop_to_clean(&actors);
        assert_eq!(repaired_first, repaired_second);
        assert_eq!(rounds_first, rounds_second);
        assert!(super::super::run(&repaired_first).findings.is_empty());
        assert_eq!(
            format!("{:?}", repaired_first),
            format!("{:?}", repaired_second)
        );
    }

    #[test]
    fn sp43_repair_pillar_needs_rounds() {
        let actors = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
            resting(2, "crate_c", 0.2, 0.0),
        ];
        let start_report = super::super::run(&actors);
        assert!(!start_report.findings.is_empty());
        let (repaired, rounds) = repair_loop_to_clean(&actors);
        assert!((1..=3).contains(&rounds));
        assert!(super::super::run(&repaired).findings.is_empty());
        let (again, rounds_again) = repair_loop_to_clean(&actors);
        assert_eq!(repaired, again);
        assert_eq!(rounds, rounds_again);
    }

    #[test]
    fn sp43_repair_round_cap_is_three() {
        let lost = Actor {
            x: f32::NAN,
            ..resting(0, "lost_prop", 0.0, 0.0)
        };
        let actors = vec![lost];
        let report = super::super::run(&actors);
        assert_eq!(report.count("unknown-position"), 1);
        let (lane_a, lane_b) = dual_propose(&report.findings, &actors, 1);
        assert_eq!(lane_a, lane_b);
        let agreed = self_consistent_decode(&lane_a, &lane_b);
        assert_eq!(agreed.len(), report.findings.len());
        let cur = actors.clone();
        let mut rounds: u32 = 0;
        while rounds < 3 {
            let rep = super::super::run(&cur);
            assert!(!rep.findings.is_empty());
            let step = repair_round(&cur, &rep.findings, 1, rounds);
            assert!(step.is_none());
            rounds = rounds.saturating_add(1);
        }
        assert_eq!(rounds, 3);
        assert!(repair_round(&cur, &report.findings, 1, 3).is_none());
        assert!(repair_round(&cur, &report.findings, 1, 4).is_none());
        assert!(!super::super::run(&cur).findings.is_empty());
    }
}

#[cfg(test)]
mod cycle_tests {
    use super::super::relation::{build, seeded_sp28_actors, with_support_ring};
    use super::*;

    /// 30 clean built graphs plus 30 one-ring graphs (ring ids slide with the
    /// seed over the floor props 3..=8).
    fn corpus() -> Vec<(Vec<Actor>, Graph, Option<[u32; 3]>)> {
        let mut out = Vec::new();
        for seed in 0..30u64 {
            let actors = seeded_sp28_actors(seed);
            let clean = build(&actors);
            let a = 3 + (seed % 4) as u32;
            out.push((actors.clone(), clean.clone(), None));
            let ring = with_support_ring(&clean, a, a + 1, a + 2);
            out.push((actors, ring, Some([a, a + 1, a + 2])));
        }
        out
    }

    #[test]
    fn sixty_graphs_refuse_name_repair_and_recheck() {
        let (mut detected, mut fp, mut named, mut clean_after) = (0, 0, 0, 0);
        for (actors, graph, ring) in corpus() {
            let refusal = cycle_refusal(&graph);
            let Some(ring) = ring else {
                fp += u32::from(refusal.is_some());
                let (same, receipt) = repair_cycles(&actors, &graph);
                assert!(receipt.steps.is_empty() && receipt.clean && same == graph);
                continue;
            };
            let refusal = refusal.expect("injected ring must refuse");
            detected += 1;
            let want: Vec<String> = ring.iter().map(|id| id.to_string()).collect();
            named += u32::from(
                refusal.reason == format!("relation-cycle: {}", want.join(">"))
                    && refusal.conflict_set == ring,
            );
            // Any edge order names and repairs the same way.
            let mut shuffled = graph.clone();
            shuffled.edges.reverse();
            assert_eq!(cycle_refusal(&shuffled), Some(refusal));
            let (fixed, receipt) = repair_cycles(&actors, &graph);
            let (fixed2, receipt2) = repair_cycles(&actors, &shuffled);
            assert_eq!(receipt, receipt2);
            assert_eq!(
                super::super::relation::digest(&fixed),
                super::super::relation::digest(&fixed2)
            );
            assert_eq!(receipt.steps.len(), 1);
            let step = &receipt.steps[0];
            assert_eq!(step.cycle, ring);
            assert_eq!(step.residuals_cm.len(), 3);
            assert!(ring.contains(&step.dropped.0) && ring.contains(&step.dropped.1));
            clean_after += u32::from(receipt.clean && cycle_refusal(&fixed).is_none());
            assert!(fixed.edges.len() < graph.edges.len());
        }
        assert_eq!((detected, fp, named, clean_after), (30, 0, 30, 30));
    }

    #[test]
    fn receipt_digest_is_stable_and_follows_the_repair() {
        let actors = seeded_sp28_actors(7);
        let ring = with_support_ring(&build(&actors), 3, 4, 5);
        let (_, first) = repair_cycles(&actors, &ring);
        let (_, second) = repair_cycles(&actors, &ring);
        assert_eq!(first.digest, second.digest);
        let other = with_support_ring(&build(&actors), 4, 5, 6);
        assert_ne!(repair_cycles(&actors, &other).1.digest, first.digest);
    }

    #[test]
    fn unmeasurable_leg_is_dropped_first() {
        let mut actors = seeded_sp28_actors(2);
        let lost = Actor {
            x: f32::NAN,
            ..actors[5].clone()
        };
        actors[5] = lost;
        let ring = with_support_ring(&build(&actors), 3, 4, 5);
        let (fixed, receipt) = repair_cycles(&actors, &ring);
        assert_eq!(receipt.steps[0].dropped, (4, 5));
        assert_eq!(receipt.steps[0].residuals_cm.len(), 1);
        assert!(receipt.clean && cycle_refusal(&fixed).is_none());
    }
}

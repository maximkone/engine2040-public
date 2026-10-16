//! compose: advisory hero-shot composition checks.
//!
//! Owner: the spatial crate; this module owns the shot model and nothing
//! else. Planned consumers (not yet wired): the auto director in the game
//! crate picks its presented shot with [`best_shot`], and the look harness
//! in the playtest crate ranks reference captures with [`rank_order`]; both
//! read the totals as advice.
//! Units: meters in, integer centimeters inside, screen pixels as integers,
//! every sub-score 0..100. Determinism: order-free sums, yaw snapped through
//! the shared octant table, integer projection; any input order scores the
//! same. Errors: [`ComposeError`] at every boundary. Budget: one score costs
//! a single pass over actors plus one ray per objective; the hot path
//! ([`score_shot`]/[`best_shot`]) allocates nothing, while [`rank_order`]
//! and [`spearman_permille`] allocate their rank buffers. Budget: the
//! 20-reference-shot panel scores within 1 ms per shot.

use super::afford::door_clear_cm;
use super::index::{to_cm, Box3};
use super::navcheck::{octant, NavView};
use super::sight::{los_point, SightWorld};
use super::Actor;

/// Widest frame the projector accepts, in pixels (captures never exceed it).
pub const MAX_FRAME_PX: u32 = 8192;
/// Subject-area band, permille of the frame: a hero fills a tenth to a quarter.
const SIZE_LO_PERMILLE: i64 = 80;
/// Upper end of the subject-area band, permille of the frame.
const SIZE_HI_PERMILLE: i64 = 250;
/// Thirds weight in the total, out of 100.
const W_THIRDS: u32 = 20;
/// Size weight in the total, out of 100.
const W_SIZE: u32 = 20;
/// Headroom weight in the total, out of 100.
const W_HEADROOM: u32 = 15;
/// Clutter weight in the total, out of 100.
const W_CLUTTER: u32 = 15;
/// Sightline weight in the total, out of 100.
const W_SIGHT: u32 = 20;
/// Symmetry weight in the total, out of 100.
const W_SYMMETRY: u32 = 10;

/// One hero-shot framing request. The eye and the subject live in the same
/// world the sight module built; objectives are scored by rays from the eye.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shot {
    /// Camera eye in meters; every coordinate must be finite.
    pub eye_m: [f32; 3],
    /// Camera yaw in radians, snapped through the shared octant table.
    pub yaw: f32,
    /// Framed subject by actor id.
    pub subject: u32,
    /// Frame width in pixels, 1..=8192.
    pub frame_w: u32,
    /// Frame height in pixels, 1..=8192.
    pub frame_h: u32,
}

/// Score breakdown for one shot; every field is 0..100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShotScore {
    /// Subject center near a third-line crossing.
    pub thirds: u32,
    /// Subject area inside the hero band of the frame.
    pub size: u32,
    /// Gap above the subject head near the ideal headroom.
    pub headroom: u32,
    /// Few distractors over the subject and the frame.
    pub clutter: u32,
    /// Share of objectives with a clear line from the eye.
    pub sightlines: u32,
    /// Background mass balanced left to right.
    pub symmetry: u32,
    /// Weighted mean of the six rows above.
    pub total: u32,
    /// False when the subject sits behind the camera; framing rows are 0.
    pub subject_visible: bool,
}

/// Composition failures: typed, no guessing.
///
/// # Determinism
///
/// Variants carry ids and dims only; messages never affect scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposeError {
    /// No indexed box carries this subject id.
    UnknownSubject(u32),
    /// No actor carries this objective id.
    UnknownObjective(u32),
    /// An objective center has a non-finite coordinate.
    NonFiniteObjective(u32),
    /// The eye has a non-finite coordinate.
    NonFiniteEye,
    /// Frame dims outside 1..=8192; carries the rejected pair.
    BadFrame { w: u32, h: u32 },
    /// Rank inputs disagree in length or hold fewer than two shots.
    BadPanel,
    /// A baked door width is negative; carries the rejected width in cm.
    BadDoorWidth(i32),
}

impl std::fmt::Display for ComposeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSubject(id) => write!(f, "compose: unknown subject {id}"),
            Self::UnknownObjective(id) => write!(f, "compose: unknown objective {id}"),
            Self::NonFiniteObjective(id) => {
                write!(f, "compose: objective {id} is not finite")
            }
            Self::NonFiniteEye => write!(f, "compose: camera eye is not finite"),
            Self::BadFrame { w, h } => write!(f, "compose: frame {w}x{h} outside 1..=8192"),
            Self::BadPanel => write!(f, "compose: panel needs two aligned shots"),
            Self::BadDoorWidth(w) => write!(f, "compose: door width {w} cm is negative"),
        }
    }
}

impl std::error::Error for ComposeError {}

/// Integer camera: eye and basis in centimeters, frame in pixels. Screen
/// right is the forward vector turned a quarter turn toward +Z when facing
/// +X; mirrored framings score identically by construction.
struct View {
    eye: [i32; 3],
    fx: i64,
    fz: i64,
    rx: i64,
    rz: i64,
    fw: i64,
    fh: i64,
}

/// Projected box in integer pixels, unclipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    cx: i64,
    cy: i64,
    w: i64,
    h: i64,
}

/// Box center on one axis. The mean of two int32 values always fits int32.
fn mid(lo: i32, hi: i32) -> i32 {
    i32::try_from((i64::from(lo) + i64::from(hi)) / 2).unwrap_or(0)
}

/// Clamp a 0..100 score computed in i64.
fn clamp_score(v: i64) -> u32 {
    u32::try_from(v.clamp(0, 100)).unwrap_or(0)
}

/// Project an actor-box center to integer pixels. The frame maps the cone
/// (|lateral| <= depth, |dy| <= depth) onto pixels, matching the sight cone.
/// Returns `None` behind the camera or outside the cone. All math is i64.
fn project(view: &View, center: [i32; 3], half: [i32; 2]) -> Option<Rect> {
    let dx = i64::from(center[0]) - i64::from(view.eye[0]);
    let dy = i64::from(center[1]) - i64::from(view.eye[1]);
    let dz = i64::from(center[2]) - i64::from(view.eye[2]);
    let depth = dx * view.fx + dz * view.fz;
    if depth <= 0 {
        return None;
    }
    let lat = dx * view.rx + dz * view.rz;
    if lat < -depth || lat > depth || dy < -depth || dy > depth {
        return None;
    }
    let cx = view.fw / 2 + lat * view.fw / (2 * depth);
    let cy = view.fh / 2 - dy * view.fh / (2 * depth);
    let w = i64::from(half[0]) * 2 * view.fw / (2 * depth);
    let h = i64::from(half[1]) * 2 * view.fh / (2 * depth);
    Some(Rect {
        cx,
        cy,
        w: w.max(1),
        h: h.max(1),
    })
}

/// True when any pixel of the rect lands inside the frame.
fn in_frame(r: Rect, fw: i64, fh: i64) -> bool {
    r.cx - r.w / 2 < fw && r.cx + r.w / 2 > 0 && r.cy - r.h / 2 < fh && r.cy + r.h / 2 > 0
}

/// True when the two rects share at least one pixel row and column.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.cx - a.w / 2 <= b.cx + b.w / 2
        && b.cx - b.w / 2 <= a.cx + a.w / 2
        && a.cy - a.h / 2 <= b.cy + b.h / 2
        && b.cy - b.h / 2 <= a.cy + a.h / 2
}

/// Third-line crossing score: 100 on a crossing, 0 past a quarter frame away.
fn thirds_score(r: Rect, fw: i64, fh: i64) -> u32 {
    let xs = [fw / 3, 2 * fw / 3];
    let ys = [fh / 3, 2 * fh / 3];
    let mut best = i64::MAX;
    for px in xs {
        for py in ys {
            let d = (r.cx - px).abs() + (r.cy - py).abs();
            if d < best {
                best = d;
            }
        }
    }
    let tol = (fw + fh) / 4;
    if tol <= 0 {
        return 0;
    }
    clamp_score(100 - best * 100 / tol)
}

/// Subject-size score: full marks inside the hero band, fading outside it.
fn size_score(r: Rect, fw: i64, fh: i64) -> u32 {
    let frame = fw * fh;
    if frame <= 0 {
        return 0;
    }
    let area = r.w.min(4 * fw) * r.h.min(4 * fh);
    let frac = area * 1000 / frame;
    match frac {
        f if f <= SIZE_LO_PERMILLE => clamp_score(f * 100 / SIZE_LO_PERMILLE),
        f if f <= SIZE_HI_PERMILLE => 100,
        f => clamp_score(100 - (f - SIZE_HI_PERMILLE) * 100 / SIZE_HI_PERMILLE),
    }
}

/// Headroom score: the gap above the subject head wants a twelfth of height.
fn headroom_score(r: Rect, fh: i64) -> u32 {
    if fh <= 0 {
        return 0;
    }
    let top = r.cy - r.h / 2;
    clamp_score(100 - (top - fh / 12).abs() * 100 / (fh / 4))
}

/// Clutter score from in-frame distractors: a box over the subject costs
/// five times one elsewhere in the frame.
fn clutter_score(over_subject: i64, in_frame_others: i64) -> u32 {
    clamp_score(100 - over_subject * 25 - in_frame_others * 5)
}

/// Sightline share: objectives with a clear ray from the eye, 0..100.
/// An empty objective list scores 100: nothing to block.
fn sightline_share(clear: usize, total: usize) -> u32 {
    if total == 0 || clear >= total {
        return 100;
    }
    let n = i64::try_from(total).unwrap_or(0);
    if n <= 0 {
        return 0;
    }
    let c = i64::try_from(clear).unwrap_or(0);
    clamp_score(c * 100 / n)
}

/// Left-right balance of background pixel mass: 100 when even, 0 at full
/// imbalance. An empty background scores 100: nothing to balance.
fn symmetry_score(left: i64, right: i64) -> u32 {
    let sum = left + right;
    if sum <= 0 {
        return 100;
    }
    clamp_score(100 - (left - right).abs() * 100 / sum)
}

/// Score one hero-shot framing over a built sight world. Framing rows come
/// from the integer projection, sightlines reuse the sight rays, and every
/// sum is order-free, so shuffled actors score identically.
///
/// # Errors
///
/// [`ComposeError::NonFiniteEye`] for a non-finite eye,
/// [`ComposeError::BadFrame`] for dims outside 1..=8192,
/// [`ComposeError::UnknownSubject`] for a subject with no indexed box,
/// [`ComposeError::UnknownObjective`] for an objective with no actor,
/// [`ComposeError::NonFiniteObjective`] for a non-finite objective center.
///
/// # Determinism
///
/// Integer math only; the same shot over the same actors scores the same in
/// any input order and on any target.
pub fn score_shot(
    world: &SightWorld,
    actors: &[Actor],
    shot: &Shot,
    objectives: &[u32],
) -> Result<ShotScore, ComposeError> {
    let view = build_view(shot)?;
    let subject_box = world
        .box_of(shot.subject)
        .ok_or(ComposeError::UnknownSubject(shot.subject))?;
    let subject_rect = project_box(&view, subject_box);
    let crowd = survey_crowd(world, actors, &view, shot.subject, subject_rect);
    let clear = count_clear(world, actors, shot.eye_m, objectives)?;
    let (thirds, size, headroom) = match subject_rect {
        Some(r) => (
            thirds_score(r, view.fw, view.fh),
            size_score(r, view.fw, view.fh),
            headroom_score(r, view.fh),
        ),
        None => (0, 0, 0),
    };
    let clutter = clutter_score(crowd.over, crowd.others);
    let sightlines = sightline_share(clear, objectives.len());
    let symmetry = symmetry_score(crowd.left, crowd.right);
    let total = (thirds * W_THIRDS
        + size * W_SIZE
        + headroom * W_HEADROOM
        + clutter * W_CLUTTER
        + sightlines * W_SIGHT
        + symmetry * W_SYMMETRY
        + 50)
        / 100;
    Ok(ShotScore {
        thirds,
        size,
        headroom,
        clutter,
        sightlines,
        symmetry,
        total,
        subject_visible: subject_rect.is_some(),
    })
}

/// Validate a shot and snap it to the integer camera.
fn build_view(shot: &Shot) -> Result<View, ComposeError> {
    if !shot.eye_m.iter().all(|v| v.is_finite()) {
        return Err(ComposeError::NonFiniteEye);
    }
    if shot.frame_w == 0
        || shot.frame_h == 0
        || shot.frame_w > MAX_FRAME_PX
        || shot.frame_h > MAX_FRAME_PX
    {
        return Err(ComposeError::BadFrame {
            w: shot.frame_w,
            h: shot.frame_h,
        });
    }
    let (fx, fz) = octant(shot.yaw);
    Ok(View {
        eye: [
            to_cm(shot.eye_m[0]),
            to_cm(shot.eye_m[1]),
            to_cm(shot.eye_m[2]),
        ],
        fx: i64::from(fx),
        fz: i64::from(fz),
        rx: i64::from(-fz),
        rz: i64::from(fx),
        fw: i64::from(shot.frame_w),
        fh: i64::from(shot.frame_h),
    })
}

/// Project one indexed box through the integer camera.
fn project_box(view: &View, b: &Box3) -> Option<Rect> {
    project(
        view,
        [
            mid(b.min_x, b.max_x),
            mid(b.min_y, b.max_y),
            mid(b.min_z, b.max_z),
        ],
        [(b.max_x - b.min_x) / 2, (b.max_y - b.min_y) / 2],
    )
}

/// Background survey: distractors over the subject, in-frame others, and
/// the left/right pixel mass beside the subject.
struct Crowd {
    over: i64,
    others: i64,
    left: i64,
    right: i64,
}

/// Sweep every non-subject actor once, accumulating order-free counts.
fn survey_crowd(
    world: &SightWorld,
    actors: &[Actor],
    view: &View,
    subject: u32,
    subject_rect: Option<Rect>,
) -> Crowd {
    let mut crowd = Crowd {
        over: 0,
        others: 0,
        left: 0,
        right: 0,
    };
    for a in actors {
        if a.id == subject {
            continue;
        }
        let Some(b) = world.box_of(a.id) else {
            continue;
        };
        let r = match project_box(view, b) {
            Some(v) => v,
            None => continue,
        };
        if !in_frame(r, view.fw, view.fh) {
            continue;
        }
        crowd.others += 1;
        if let Some(s) = subject_rect {
            if overlaps(s, r) {
                crowd.over += 1;
            }
        }
        let mass = r.w.min(4 * view.fw) * r.h.min(4 * view.fh);
        if r.cx < view.fw / 2 {
            crowd.left += mass;
        } else {
            crowd.right += mass;
        }
    }
    crowd
}

/// Count objectives with a clear ray from the eye.
fn count_clear(
    world: &SightWorld,
    actors: &[Actor],
    eye_m: [f32; 3],
    objectives: &[u32],
) -> Result<usize, ComposeError> {
    let mut clear = 0usize;
    for id in objectives {
        let mut found: Option<&Actor> = None;
        for a in actors {
            if a.id == *id {
                found = Some(a);
                break;
            }
        }
        let Some(o) = found else {
            return Err(ComposeError::UnknownObjective(*id));
        };
        let target = [o.x, o.y, o.z];
        if !target.iter().all(|v| v.is_finite()) {
            return Err(ComposeError::NonFiniteObjective(*id));
        }
        // Eye and target are proven finite above, so the ray cannot refuse.
        if los_point(world, eye_m, target).unwrap_or(false) {
            clear += 1;
        }
    }
    Ok(clear)
}

/// Rank shot totals from best to worst as shot indices. Ties break toward
/// the lower index, so the order is total and stable across targets.
pub fn rank_order(totals: &[u32]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..totals.len()).collect();
    order.sort_unstable_by(|a, b| totals[*b].cmp(&totals[*a]).then(a.cmp(b)));
    order
}

/// Position of each shot in best-first order: the inverse of [`rank_order`].
fn rank_positions(values: &[u32]) -> Vec<usize> {
    let order = rank_order(values);
    let mut pos = vec![0usize; values.len()];
    for (rank, idx) in order.iter().enumerate() {
        pos[*idx] = rank;
    }
    pos
}

/// Spearman rank correlation between panel ranks and computed totals, in
/// permille. Both slices align by shot; higher is better on each side.
/// Returns 1000 for identical orders, -1000 for reversed ones. Ties break
/// toward the lower index on both sides, deterministically.
///
/// # Errors
///
/// [`ComposeError::BadPanel`] when the slices disagree in length or hold
/// fewer than two shots.
pub fn spearman_permille(panel: &[u32], computed: &[u32]) -> Result<i32, ComposeError> {
    if panel.len() != computed.len() || panel.len() < 2 {
        return Err(ComposeError::BadPanel);
    }
    let panel_rank = rank_positions(panel);
    let computed_rank = rank_positions(computed);
    let mut sum: i128 = 0;
    for (a, b) in panel_rank.iter().zip(computed_rank.iter()) {
        let d = i128::try_from(*a).unwrap_or(0) - i128::try_from(*b).unwrap_or(0);
        sum += d * d;
    }
    let n: i128 = panel.len().try_into().unwrap_or(0);
    let denom = n * (n * n - 1);
    if denom <= 0 {
        return Err(ComposeError::BadPanel);
    }
    let rho = (denom - 6 * sum) * 1000 / denom;
    Ok(i32::try_from(rho.clamp(-1000, 1000)).unwrap_or(0))
}

/// Pick the winning shot id for the presenter: highest total, lowest id on
/// ties. Returns `None` for an empty slate. Pure and total: the same slate
/// picks the same id, so the cut log replays exactly.
pub fn best_shot(slate: &[(u32, u32)]) -> Option<u32> {
    let mut best: Option<(u32, u32)> = None;
    for (id, total) in slate {
        if best.is_none_or(|(bid, btotal)| *total > btotal || (*total == btotal && *id < bid)) {
            best = Some((*id, *total));
        }
    }
    best.map(|(id, _)| id)
}

/// Accessible-door clear width, in cm: ADA 404.2.3 asks 32 in (81.5 cm);
/// the integer centimeter floor is the pass bar.
pub const ADA_DOOR_CM: i32 = 81;
/// Accessible-route width, in cm: ADA 403.5.1 asks 36 in (91.5 cm); floored.
pub const ADA_ROUTE_CM: i32 = 91;
/// Wheelchair turning circle, in cm: ADA 304.3.1 asks 60 in (152.5 cm);
/// floored. Measured as clear surface gap around the subject staging spot.
pub const ADA_TURN_CM: i32 = 152;
/// Low end of the forward-reach band, in cm: ADA 308.2, 15 in (38 cm).
pub const ADA_REACH_LO_CM: i32 = 38;
/// High end of the forward-reach band, in cm: ADA 308.3, 48 in (122 cm).
pub const ADA_REACH_HI_CM: i32 = 122;
/// Headroom over circulation, in cm: ADA 307.2 asks 80 in (203 cm).
pub const ADA_HEAD_CM: i32 = 203;
/// VGA probe reach, in cm: each of the 8 isovist fan rays travels 5 m.
pub const VGA_PROBE_CM: i32 = 500;
/// Eye-column half width, in cm: the headroom column is a 30 cm square.
const EYE_COLUMN_HALF_CM: i32 = 15;

/// Access-grounded advisory score for one shot: six ADA threshold flags
/// plus two visibility-graph (VGA) features, all beside the six framing rules.
/// Advisory only: [`score_shot`] totals never read it, so the frozen framing
/// goldens stay byte-identical; the director ranks [`AccessScore::total`] with the
/// shared [`rank_order`] and checks agreement with [`spearman_permille`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessScore {
    /// Every baked door clears [`ADA_DOOR_CM`]; true when no doors bake.
    pub door: bool,
    /// The route to the subject clears [`ADA_ROUTE_CM`]: baked doors when
    /// any bake, else the nearest physical surface gap at the subject.
    pub route: bool,
    /// A wheelchair circle of [`ADA_TURN_CM`] fits at the subject spot.
    pub turn: bool,
    /// The subject spans up into the reach band (top at or above 38 cm).
    pub reach_lo: bool,
    /// The subject spans down into the reach band (base at or below 122 cm).
    pub reach_hi: bool,
    /// [`ADA_HEAD_CM`] above the eye holds no physical box (subject apart).
    pub headroom: bool,
    /// Actors with a clear ray from the eye (VGA connectivity), count.
    pub connect: u32,
    /// Clear 5 m fan rays from the eye, permille (VGA openness, 0..1000).
    pub openness_permille: u32,
    /// Advisory mean of the six flags plus both VGA shares, 0..100.
    pub total: u32,
}

/// Score the access-grounded advisory features for one shot over a built
/// sight world. Door widths reuse [`door_clear_cm`]; rays reuse
/// [`los_point`]; the fan basis reuses the shot's own integer camera, so
/// no table is mirrored. Every sum is order-free: shuffled actors score
/// identically. Advisory, never a gate: [`score_shot`] is untouched.
///
/// # Errors
///
/// [`ComposeError::NonFiniteEye`] for a non-finite eye,
/// [`ComposeError::BadFrame`] for dims outside 1..=8192,
/// [`ComposeError::UnknownSubject`] for a subject with no indexed box,
/// [`ComposeError::NonFiniteObjective`] for a non-finite actor center,
/// [`ComposeError::BadDoorWidth`] for a negative baked door width.
///
/// # Determinism
///
/// Integer centimeters inside; the only float touch is the shared `to_cm`
/// input rounding plus exact small-integer probe offsets. The same shot
/// over the same actors plus the same bake scores the same in any input
/// order and on any target. Budget: one pass over actors, one ray per
/// actor, eight probe rays; allocates nothing.
pub fn access_features(
    world: &SightWorld,
    actors: &[Actor],
    shot: &Shot,
    nav: Option<&NavView>,
) -> Result<AccessScore, ComposeError> {
    let view = build_view(shot)?;
    let subject_box = world
        .box_of(shot.subject)
        .ok_or(ComposeError::UnknownSubject(shot.subject))?;
    let gap = nearest_gap_cm(world, actors, shot.subject, subject_box);
    let door_min = door_min_clear(nav)?;
    let door = door_min.is_none_or(|m| m >= ADA_DOOR_CM);
    let route = match door_min {
        Some(m) => m >= ADA_ROUTE_CM,
        None => gap >= i64::from(ADA_ROUTE_CM),
    };
    let turn = gap >= i64::from(ADA_TURN_CM);
    let reach_lo = subject_box.max_y >= ADA_REACH_LO_CM;
    let reach_hi = subject_box.min_y <= ADA_REACH_HI_CM;
    let headroom = headroom_ok(world, actors, view.eye, shot.subject);
    let clear = count_visible(world, actors, shot.eye_m)?;
    let connect = u32::try_from(clear).unwrap_or(0);
    let connect_share = sightline_share(clear, actors.len());
    let openness_permille = probe_openness(world, shot.eye_m, &view)?;
    let flags: u32 = [door, route, turn, reach_lo, reach_hi, headroom]
        .iter()
        .map(|f| u32::from(*f))
        .sum();
    let total = (flags * 100 + connect_share + openness_permille / 10) / 8;
    Ok(AccessScore {
        door,
        route,
        turn,
        reach_lo,
        reach_hi,
        headroom,
        connect,
        openness_permille,
        total,
    })
}

/// Narrowest clear width over the baked doors, in cm. `None` when no bake
/// or no doors: thresholds degrade to geometry, never to errors.
fn door_min_clear(nav: Option<&NavView>) -> Result<Option<i32>, ComposeError> {
    let Some(view) = nav else {
        return Ok(None);
    };
    let mut min: Option<i32> = None;
    for door in view.doors.values() {
        let clear =
            door_clear_cm(door.width_cm).map_err(|_| ComposeError::BadDoorWidth(door.width_cm))?;
        min = Some(match min {
            None => clear,
            Some(m) => m.min(clear),
        });
    }
    Ok(min)
}

/// Horizontal surface gap between two boxes, in cm: the wider axis miss,
/// so corner adjacency reads conservatively short. Non-negative always.
fn surface_gap_xz(s: &Box3, o: &Box3) -> i64 {
    let gx = if o.min_x > s.max_x {
        i64::from(o.min_x) - i64::from(s.max_x)
    } else if s.min_x > o.max_x {
        i64::from(s.min_x) - i64::from(o.max_x)
    } else {
        0
    };
    let gz = if o.min_z > s.max_z {
        i64::from(o.min_z) - i64::from(s.max_z)
    } else if s.min_z > o.max_z {
        i64::from(s.min_z) - i64::from(o.max_z)
    } else {
        0
    };
    gx.max(gz)
}

/// Narrowest surface gap from the subject to any other physical box, in
/// cm. Order-free minimum; `i64::MAX` when nothing physical stages near.
fn nearest_gap_cm(world: &SightWorld, actors: &[Actor], subject: u32, s: &Box3) -> i64 {
    let mut gap = i64::MAX;
    for a in actors {
        if a.id == subject || !a.physical {
            continue;
        }
        let Some(b) = world.box_of(a.id) else {
            continue;
        };
        let g = surface_gap_xz(s, b);
        if g < gap {
            gap = g;
        }
    }
    gap
}

/// True when the 30 cm column over the eye holds [`ADA_HEAD_CM`] of clear
/// air: no other physical box overlaps it. Touching faces never block.
fn headroom_ok(world: &SightWorld, actors: &[Actor], eye: [i32; 3], subject: u32) -> bool {
    let x0 = eye[0].saturating_sub(EYE_COLUMN_HALF_CM);
    let x1 = eye[0].saturating_add(EYE_COLUMN_HALF_CM);
    let z0 = eye[2].saturating_sub(EYE_COLUMN_HALF_CM);
    let z1 = eye[2].saturating_add(EYE_COLUMN_HALF_CM);
    let y1 = eye[1].saturating_add(ADA_HEAD_CM);
    for a in actors {
        if a.id == subject || !a.physical {
            continue;
        }
        let Some(b) = world.box_of(a.id) else {
            continue;
        };
        if b.min_x < x1
            && x0 < b.max_x
            && b.min_y < y1
            && eye[1] < b.max_y
            && b.min_z < z1
            && z0 < b.max_z
        {
            return false;
        }
    }
    true
}

/// Actors with a clear ray from the eye. Same finite-center refusals as
/// the sightline survey; the count is order-free.
fn count_visible(
    world: &SightWorld,
    actors: &[Actor],
    eye_m: [f32; 3],
) -> Result<usize, ComposeError> {
    let mut clear = 0usize;
    for a in actors {
        let target = [a.x, a.y, a.z];
        if !target.iter().all(|v| v.is_finite()) {
            return Err(ComposeError::NonFiniteObjective(a.id));
        }
        // Eye and target are proven finite above, so the ray cannot refuse.
        if los_point(world, eye_m, target).unwrap_or(false) {
            clear += 1;
        }
    }
    Ok(clear)
}

/// VGA openness: the share of eight 5 m fan rays from the eye that travel
/// clear, in permille. The fan rides the shot's own integer basis
/// (forward, right, back, left plus diagonals) at eye height, so mirrored
/// framings probe identically by construction.
fn probe_openness(world: &SightWorld, eye_m: [f32; 3], view: &View) -> Result<u32, ComposeError> {
    let dirs: [(i64, i64); 8] = [
        (view.fx, view.fz),
        (view.rx, view.rz),
        (-view.fx, -view.fz),
        (-view.rx, -view.rz),
        (view.fx + view.rx, view.fz + view.rz),
        (view.fx - view.rx, view.fz - view.rz),
        (-view.fx + view.rx, -view.fz + view.rz),
        (-view.fx - view.rx, -view.fz - view.rz),
    ];
    let reach_m = f32::from(i16::try_from(VGA_PROBE_CM).unwrap_or(0)) / 100.0;
    let mut clear = 0u32;
    for (dx, dz) in dirs {
        let step_x = f32::from(i8::try_from(dx).unwrap_or(0));
        let step_z = f32::from(i8::try_from(dz).unwrap_or(0));
        let target = [
            eye_m[0] + step_x * reach_m,
            eye_m[1],
            eye_m[2] + step_z * reach_m,
        ];
        if !target.iter().all(|v| v.is_finite()) {
            return Err(ComposeError::NonFiniteEye);
        }
        // Proven finite above, so the ray cannot refuse.
        if los_point(world, eye_m, target).unwrap_or(false) {
            clear += 1;
        }
    }
    Ok(clear * 1000 / 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: "shot".to_owned(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn hero_at(x: f32, y: f32, z: f32) -> Actor {
        actor(1, x, y, z, 1.0, 1.8, 1.0)
    }

    fn shot_for(subject: u32) -> Shot {
        Shot {
            eye_m: [0.0, 1.6, 0.0],
            yaw: 0.0,
            subject,
            frame_w: 640,
            frame_h: 480,
        }
    }

    #[test]
    fn thirds_rewards_the_crossing_over_dead_center() {
        let actors = [
            actor(1, 4.0, 3.0, -1.34, 1.0, 1.8, 1.0),
            actor(2, 4.0, 3.0, 0.0, 1.0, 1.8, 1.0),
        ];
        let world = SightWorld::build(&actors);
        let near = score_shot(&world, &actors, &shot_for(1), &[]).expect("near crossing scores");
        let center = score_shot(&world, &actors, &shot_for(2), &[]).expect("center scores");
        assert!(near.subject_visible && center.subject_visible);
        assert!(
            near.thirds > center.thirds,
            "crossing {} beats center {}",
            near.thirds,
            center.thirds
        );
        assert!(near.total > center.total);
    }

    #[test]
    fn size_prefers_the_hero_band_over_tiny_and_giant() {
        let actors = [
            actor(1, 2.0, 3.0, -1.34, 1.0, 1.8, 1.0),
            actor(2, 14.0, 3.0, -1.34, 1.0, 1.8, 1.0),
            actor(3, 1.2, 1.6, -0.4, 1.0, 1.8, 1.0),
        ];
        let world = SightWorld::build(&actors);
        let mid = score_shot(&world, &actors, &shot_for(1), &[]).expect("mid scores");
        let far = score_shot(&world, &actors, &shot_for(2), &[]).expect("far scores");
        let near = score_shot(&world, &actors, &shot_for(3), &[]).expect("near scores");
        assert!(mid.subject_visible && far.subject_visible && near.subject_visible);
        assert!(
            mid.size > far.size,
            "hero band {} beats tiny {}",
            mid.size,
            far.size
        );
        assert!(
            mid.size > near.size,
            "hero band {} beats giant {}",
            mid.size,
            near.size
        );
    }

    #[test]
    fn headroom_prefers_a_gap_over_clipping_or_void() {
        let actors = [
            actor(1, 4.0, 3.0, -1.34, 1.0, 1.8, 1.0),
            actor(2, 4.0, 6.0, -1.34, 1.0, 1.8, 1.0),
            actor(3, 4.0, 0.4, -1.34, 1.0, 1.8, 1.0),
        ];
        let world = SightWorld::build(&actors);
        let good = score_shot(&world, &actors, &shot_for(1), &[]).expect("gap scores");
        let high = score_shot(&world, &actors, &shot_for(2), &[]).expect("void scores");
        let low = score_shot(&world, &actors, &shot_for(3), &[]).expect("low scores");
        assert!(good.headroom > high.headroom, "gap beats void");
        assert!(good.headroom > low.headroom, "gap beats sunk");
    }

    #[test]
    fn clutter_punishes_a_crowd_over_the_subject() {
        let clean = [hero_at(4.0, 3.0, -1.34)];
        let crowded = [
            hero_at(4.0, 3.0, -1.34),
            actor(2, 4.0, 3.0, -0.9, 1.0, 1.8, 1.0),
            actor(3, 4.0, 3.0, -1.8, 1.0, 1.8, 1.0),
        ];
        let clean_score = score_shot(&SightWorld::build(&clean), &clean, &shot_for(1), &[])
            .expect("clean scores");
        let crowd_score = score_shot(&SightWorld::build(&crowded), &crowded, &shot_for(1), &[])
            .expect("crowd scores");
        assert_eq!(clean_score.clutter, 100);
        assert!(
            crowd_score.clutter < clean_score.clutter,
            "crowd {} under clean {}",
            crowd_score.clutter,
            clean_score.clutter
        );
    }

    #[test]
    fn sightlines_flag_the_walled_objective() {
        let banner = actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5);
        let open = [hero_at(4.0, 3.0, -1.34), banner.clone()];
        let wall = actor(5, 2.5, 1.5, -0.7, 0.6, 3.0, 7.0);
        let closed = [hero_at(4.0, 3.0, -1.34), banner, wall];
        let open_score =
            score_shot(&SightWorld::build(&open), &open, &shot_for(1), &[9]).expect("open scores");
        let closed_score = score_shot(&SightWorld::build(&closed), &closed, &shot_for(1), &[9])
            .expect("walled scores");
        assert_eq!(open_score.sightlines, 100);
        assert_eq!(closed_score.sightlines, 0);
    }

    #[test]
    fn symmetry_rewards_balance_over_one_sided_mass() {
        let left = actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0);
        let right = actor(8, 5.0, 1.0, 4.0, 1.0, 2.0, 1.0);
        let balanced = [hero_at(4.0, 0.9, -3.0), left.clone(), right];
        let lopsided = [hero_at(4.0, 0.9, -3.0), left];
        let even = score_shot(&SightWorld::build(&balanced), &balanced, &shot_for(1), &[])
            .expect("even scores");
        let odd = score_shot(&SightWorld::build(&lopsided), &lopsided, &shot_for(1), &[])
            .expect("odd scores");
        assert!(
            even.symmetry > odd.symmetry,
            "even {} beats one-sided {}",
            even.symmetry,
            odd.symmetry
        );
    }

    #[test]
    fn subject_behind_the_camera_scores_framing_zero() {
        let actors = [
            hero_at(-5.0, 0.9, 0.0),
            actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5),
        ];
        let world = SightWorld::build(&actors);
        let score = score_shot(&world, &actors, &shot_for(1), &[9]).expect("behind scores");
        assert!(!score.subject_visible);
        assert_eq!(score.thirds, 0);
        assert_eq!(score.size, 0);
        assert_eq!(score.headroom, 0);
        assert_eq!(score.sightlines, 100);
    }

    #[test]
    fn shuffled_actors_score_identically() {
        let actors = [
            hero_at(4.0, 3.0, -1.34),
            actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0),
            actor(8, 5.0, 1.0, 4.0, 1.0, 2.0, 1.0),
            actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5),
        ];
        let world = SightWorld::build(&actors);
        let first = score_shot(&world, &actors, &shot_for(1), &[9]).expect("first scores");
        let mut flipped = actors.clone();
        flipped.reverse();
        let second = score_shot(&world, &flipped, &shot_for(1), &[9]).expect("flipped scores");
        assert_eq!(first, second);
    }

    #[test]
    fn refusals_name_the_culprit() {
        let actors = [hero_at(4.0, 3.0, -1.34)];
        let world = SightWorld::build(&actors);
        assert_eq!(
            score_shot(&world, &actors, &shot_for(77), &[]).expect_err("ghost subject"),
            ComposeError::UnknownSubject(77)
        );
        assert_eq!(
            score_shot(&world, &actors, &shot_for(1), &[77]).expect_err("ghost objective"),
            ComposeError::UnknownObjective(77)
        );
        let mut bad_eye = shot_for(1);
        bad_eye.eye_m = [f32::NAN, 1.6, 0.0];
        assert_eq!(
            score_shot(&world, &actors, &bad_eye, &[]).expect_err("blind eye"),
            ComposeError::NonFiniteEye
        );
        let mut bad_frame = shot_for(1);
        bad_frame.frame_w = 0;
        assert_eq!(
            score_shot(&world, &actors, &bad_frame, &[]).expect_err("empty frame"),
            ComposeError::BadFrame { w: 0, h: 480 }
        );
        assert_eq!(
            spearman_permille(&[1, 2], &[1]).expect_err("ragged panel"),
            ComposeError::BadPanel
        );
        assert_eq!(
            spearman_permille(&[1], &[1]).expect_err("single panel"),
            ComposeError::BadPanel
        );
    }

    #[test]
    fn rank_order_breaks_ties_toward_the_lower_index() {
        assert_eq!(rank_order(&[30, 90, 60]), vec![1, 2, 0]);
        assert_eq!(rank_order(&[50, 50, 50]), vec![0, 1, 2]);
        assert!(rank_order(&[]).is_empty());
    }

    #[test]
    fn spearman_spots_identical_and_reversed_orders() {
        let same = spearman_permille(&[1u32, 2, 3, 4], &[10u32, 20, 30, 40]).expect("same scores");
        assert_eq!(same, 1000);
        let flipped =
            spearman_permille(&[1u32, 2, 3, 4], &[40u32, 30, 20, 10]).expect("flip scores");
        assert_eq!(flipped, -1000);
    }

    #[test]
    fn director_picks_the_clear_hero_framing() {
        let banner = actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5);
        let tower = actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0);
        let wall = actor(5, 2.5, 1.5, -0.7, 0.6, 3.0, 7.0);
        let cast = [
            hero_at(4.0, 3.0, -1.34),
            actor(2, 4.0, 3.0, 2.5, 1.0, 1.8, 1.0),
            banner,
            tower,
            wall,
        ];
        let world = SightWorld::build(&cast);
        let objectives: &[u32] = &[9];
        let wide = Shot {
            eye_m: [0.0, 1.6, 0.0],
            yaw: 0.0,
            subject: 1,
            frame_w: 640,
            frame_h: 480,
        };
        let totals = [
            (
                11,
                score_shot(&world, &cast, &wide, objectives)
                    .expect("wide scores")
                    .total,
            ),
            (
                12,
                score_shot(&world, &cast, &Shot { subject: 2, ..wide }, objectives)
                    .expect("far subject scores")
                    .total,
            ),
        ];
        let slate = [(totals[0].0, totals[0].1), (totals[1].0, totals[1].1)];
        let order = rank_order(&[slate[0].1, slate[1].1]);
        assert!(
            totals[0].1 > totals[1].1,
            "crossing beats the edge: {} vs {}",
            totals[0].1,
            totals[1].1
        );
        assert_eq!(order[0], 0);
        assert_eq!(best_shot(&slate), Some(11));
        assert_eq!(best_shot(&[]), None);
        assert_eq!(best_shot(&[(3, 10), (2, 10)]), Some(2));
    }

    /// Twenty reference framings over a graded ladder: the hero drifts off
    /// the crossing and recedes, distractors pile on one side, then a wall
    /// closes the sightlines. Panel ranks run best-first with the ladder.
    fn ladder_totals() -> ([u32; 20], [u32; 20]) {
        let mut totals = [0u32; 20];
        let mut panel = [0u32; 20];
        for k in 0u32..20 {
            let i = usize::try_from(k).unwrap_or(0);
            let step = f32::from(u8::try_from(k).unwrap_or(0));
            let hero = hero_at(4.0 + 0.18 * step, 3.0, -(1.34 + 0.28 * step));
            let mut cast = vec![
                hero,
                actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5),
                actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0),
            ];
            let mut extra = 0u32;
            while extra < k / 5 {
                let n = 20u32 + extra;
                cast.push(actor(
                    n,
                    4.0 + 0.18 * step,
                    0.9,
                    -(1.34 + 0.28 * step) + 0.5,
                    1.0,
                    1.8,
                    1.0,
                ));
                extra += 1;
            }
            if k >= 17 {
                cast.push(actor(5, 2.5, 1.5, -0.7, 0.6, 3.0, 7.0));
            }
            let world = SightWorld::build(&cast);
            totals[i] = score_shot(&world, &cast, &shot_for(1), &[9])
                .expect("ladder shot scores")
                .total;
            panel[i] = 20 - k;
        }
        (totals, panel)
    }

    #[test]
    fn panel_agreement_clears_the_review_bar() {
        let (totals, panel) = ladder_totals();
        let rho = spearman_permille(&panel, &totals).expect("panel rho");
        assert!(
            rho >= 700,
            "review agreement rho={rho} under the 700 bar: {totals:?}"
        );
    }

    /// Twenty reference framings score within the millisecond budget: 20
    /// shots inside 20 ms total, i.e. at most 1 ms per shot. Wall time never
    /// enters a score; only this ignored perf gate reads the clock, because
    /// the budget itself is wall time.
    #[test]
    #[ignore = "timing: compose budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn twenty_reference_shots_score_within_the_millisecond_budget() {
        let mut scenes: Vec<(SightWorld, Vec<Actor>)> = Vec::new();
        for k in 0u32..20 {
            let step = f32::from(u8::try_from(k).unwrap_or(0));
            let cast = vec![
                hero_at(4.0 + 0.18 * step, 3.0, -(1.34 + 0.28 * step)),
                actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5),
                actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0),
            ];
            scenes.push((SightWorld::build(&cast), cast));
        }
        let shot = shot_for(1);
        // One warm pass outside the measurement, then the minimum of 5 takes
        // so scheduler noise can only slow the reading, never flatter it.
        for (world, cast) in &scenes {
            score_shot(world, cast, &shot, &[9]).expect("warm shot scores");
        }
        let mut best_ms: u128 = u128::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let mut scored = 0u32;
            for (world, cast) in &scenes {
                let score = score_shot(world, cast, &shot, &[9]).expect("budget shot scores");
                scored += score.total;
            }
            assert!(scored > 0, "ladder totals stay non-zero");
            let elapsed_ms = start.elapsed().as_millis();
            if elapsed_ms < best_ms {
                best_ms = elapsed_ms;
            }
        }
        assert!(
            best_ms <= 20,
            "20 shots took {best_ms} ms, over the 1 ms/shot budget"
        );
    }

    fn access_nav(widths: &[(u32, i32)]) -> crate::NavView {
        let mut doors = std::collections::BTreeMap::new();
        for (id, w) in widths {
            doors.insert(
                *id,
                crate::navcheck::Door {
                    width_cm: *w,
                    two_way: false,
                },
            );
        }
        crate::NavView {
            doors,
            ..Default::default()
        }
    }

    fn access_for(actors: &[Actor], shot: &Shot, nav: Option<&crate::NavView>) -> AccessScore {
        access_features(&SightWorld::build(actors), actors, shot, nav).expect("access scores")
    }

    #[test]
    fn access_door_flags_read_the_bake() {
        let actors = [actor(1, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0)];
        let shot = shot_for(1);
        let narrow = access_nav(&[(7, 90)]);
        let narrow_score = access_for(&actors, &shot, Some(&narrow));
        assert!(!narrow_score.door, "clear 70 cm under the 81 cm bar");
        assert!(!narrow_score.route, "clear 70 cm under the 91 cm bar");
        let standard = access_nav(&[(7, 101)]);
        let standard_score = access_for(&actors, &shot, Some(&standard));
        assert!(standard_score.door, "clear 81 cm meets the door bar");
        assert!(!standard_score.route, "clear 81 cm under the route bar");
        let wide = access_nav(&[(7, 111)]);
        let wide_score = access_for(&actors, &shot, Some(&wide));
        assert!(
            wide_score.door && wide_score.route,
            "clear 91 cm meets both"
        );
        let empty = access_nav(&[]);
        let open = access_for(&actors, &shot, Some(&empty));
        assert!(open.door, "no baked doors block nothing");
    }

    #[test]
    fn access_route_and_turn_read_surface_gaps() {
        let roomy = [
            actor(1, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            actor(2, 3.0, 0.5, 0.0, 1.0, 1.0, 1.0),
        ];
        let roomy_score = access_for(&roomy, &shot_for(1), None);
        assert!(roomy_score.route, "200 cm gap clears the route bar");
        assert!(roomy_score.turn, "200 cm gap fits the turning circle");
        let tight = [
            actor(1, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            actor(2, 1.2, 0.5, 0.0, 1.0, 1.0, 1.0),
        ];
        let tight_score = access_for(&tight, &shot_for(1), None);
        assert!(!tight_score.route, "20 cm gap under the route bar");
        assert!(!tight_score.turn, "20 cm gap under the turning bar");
        let ghost_crowd = [
            actor(1, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            Actor {
                physical: false,
                ..actor(2, 1.2, 0.5, 0.0, 1.0, 1.0, 1.0)
            },
        ];
        let ghost_score = access_for(&ghost_crowd, &shot_for(1), None);
        assert!(
            ghost_score.route && ghost_score.turn,
            "non-physical decor never narrows the gap"
        );
    }

    #[test]
    fn access_reach_flags_span_the_band() {
        let prop = [actor(1, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0)];
        let prop_score = access_for(&prop, &shot_for(1), None);
        assert!(
            prop_score.reach_lo && prop_score.reach_hi,
            "0..100 cm spans 38..122 cm"
        );
        let floater = [actor(1, 4.0, 3.0, -1.34, 1.0, 1.8, 1.0)];
        let floater_score = access_for(&floater, &shot_for(1), None);
        assert!(floater_score.reach_lo, "tall top reaches down past 38 cm");
        assert!(
            !floater_score.reach_hi,
            "210 cm base floats over the 122 cm bar"
        );
        let sunk = [actor(1, 0.0, -1.0, 0.0, 1.0, 0.2, 1.0)];
        let sunk_score = access_for(&sunk, &shot_for(1), None);
        assert!(!sunk_score.reach_lo, "-90 cm top never enters the band");
        assert!(sunk_score.reach_hi, "buried base sits under the band");
    }

    #[test]
    fn access_headroom_flags_the_beam_over_the_eye() {
        let open = [actor(1, 4.0, 0.5, -3.0, 1.0, 1.0, 1.0)];
        assert!(
            access_for(&open, &shot_for(1), None).headroom,
            "open sky passes"
        );
        let mut beamed = open.to_vec();
        beamed.push(actor(5, 0.0, 2.2, 0.0, 1.0, 0.3, 1.0));
        assert!(
            !access_for(&beamed, &shot_for(1), None).headroom,
            "a 205 cm soffit over a 160 cm eye fails the 203 cm bar"
        );
    }

    #[test]
    fn access_vga_counts_what_the_eye_holds() {
        let banner = actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5);
        let open = [hero_at(4.0, 3.0, -1.34), banner.clone()];
        let open_score = access_for(&open, &shot_for(1), None);
        let wall = actor(5, 2.5, 1.5, -0.7, 0.6, 3.0, 7.0);
        let closed = [hero_at(4.0, 3.0, -1.34), banner, wall];
        let closed_score = access_for(&closed, &shot_for(1), None);
        assert_eq!(open_score.connect, 2);
        assert_eq!(closed_score.connect, 1);
        assert!(
            closed_score.openness_permille < open_score.openness_permille,
            "the wall narrows the fan: {} vs {}",
            closed_score.openness_permille,
            open_score.openness_permille
        );
        let mut closet = vec![hero_at(4.0, 0.5, -3.0)];
        closet.push(actor(10, 1.0, 1.5, 0.0, 0.2, 3.0, 4.0));
        closet.push(actor(11, -1.0, 1.5, 0.0, 0.2, 3.0, 4.0));
        closet.push(actor(12, 0.0, 1.5, 1.0, 4.0, 3.0, 0.2));
        closet.push(actor(13, 0.0, 1.5, -1.0, 4.0, 3.0, 0.2));
        let closet_score = access_for(&closet, &shot_for(1), None);
        assert_eq!(
            closet_score.openness_permille, 0,
            "four close walls shut the fan"
        );
    }

    #[test]
    fn access_rank_agrees_with_the_framing_panel() {
        let mut access_totals = [0u32; 20];
        let mut panel = [0u32; 20];
        for k in 0u32..20 {
            let i = usize::try_from(k).unwrap_or(0);
            let step = f32::from(u8::try_from(k).unwrap_or(0));
            let hero = hero_at(4.0 + 0.18 * step, 3.0, -(1.34 + 0.28 * step));
            let mut cast = vec![
                hero,
                actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5),
                actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0),
            ];
            let mut extra = 0u32;
            while extra < k / 5 {
                let n = 20u32 + extra;
                cast.push(actor(
                    n,
                    4.0 + 0.18 * step,
                    0.9,
                    -(1.34 + 0.28 * step) + 0.5,
                    1.0,
                    1.8,
                    1.0,
                ));
                extra += 1;
            }
            if k >= 17 {
                cast.push(actor(5, 2.5, 1.5, -0.7, 0.6, 3.0, 7.0));
            }
            let world = SightWorld::build(&cast);
            access_totals[i] = access_features(&world, &cast, &shot_for(1), None)
                .expect("ladder access scores")
                .total;
            panel[i] = 20 - k;
        }
        let rho = spearman_permille(&panel, &access_totals).expect("access rho");
        assert!(
            rho >= 700,
            "advisory agreement rho={rho} under the 700 bar: {access_totals:?}"
        );
        let order = rank_order(&access_totals);
        assert!(
            access_totals[0] > access_totals[19],
            "the clearest staging outranks the walled one: {access_totals:?}"
        );
        assert!(order.len() == 20, "every ladder shot ranks");
    }

    #[test]
    fn access_shuffled_actors_score_identically() {
        let actors = [
            hero_at(4.0, 0.5, -3.0),
            actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0),
            actor(8, 5.0, 1.0, 4.0, 1.0, 2.0, 1.0),
            actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5),
        ];
        let first = access_for(&actors, &shot_for(1), None);
        let mut flipped = actors.clone();
        flipped.reverse();
        let second = access_for(&flipped, &shot_for(1), None);
        assert_eq!(first, second);
    }

    #[test]
    fn access_refusals_name_the_culprit() {
        let actors = [hero_at(4.0, 0.5, -3.0)];
        let world = SightWorld::build(&actors);
        assert_eq!(
            access_features(&world, &actors, &shot_for(77), None).expect_err("ghost subject"),
            ComposeError::UnknownSubject(77)
        );
        let mut bad_eye = shot_for(1);
        bad_eye.eye_m = [f32::NAN, 1.6, 0.0];
        assert_eq!(
            access_features(&world, &actors, &bad_eye, None).expect_err("blind eye"),
            ComposeError::NonFiniteEye
        );
        let mut bad_frame = shot_for(1);
        bad_frame.frame_h = 0;
        assert_eq!(
            access_features(&world, &actors, &bad_frame, None).expect_err("flat frame"),
            ComposeError::BadFrame { w: 640, h: 0 }
        );
        let bad_bake = access_nav(&[(7, -5)]);
        assert_eq!(
            access_features(&world, &actors, &shot_for(1), Some(&bad_bake))
                .expect_err("negative door"),
            ComposeError::BadDoorWidth(-5)
        );
        let mut blind_cast = actors.to_vec();
        blind_cast.push(actor(2, f32::NAN, 0.5, 0.0, 1.0, 1.0, 1.0));
        let blind_world = SightWorld::build(&blind_cast);
        assert_eq!(
            access_features(&blind_world, &blind_cast, &shot_for(1), None)
                .expect_err("blind center"),
            ComposeError::NonFiniteObjective(2)
        );
    }

    /// Twenty advisory framings score within the millisecond budget: 20
    /// shots inside 20 ms total, i.e. at most 1 ms per shot. Wall time never
    /// enters a score; only this ignored perf gate reads the clock, because
    /// the budget itself is wall time.
    #[test]
    #[ignore = "timing: access budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn twenty_access_shots_score_within_the_millisecond_budget() {
        let mut scenes: Vec<(SightWorld, Vec<Actor>)> = Vec::new();
        for k in 0u32..20 {
            let step = f32::from(u8::try_from(k).unwrap_or(0));
            let cast = vec![
                hero_at(4.0 + 0.18 * step, 0.5, -(1.34 + 0.28 * step)),
                actor(9, 5.0, 1.5, 0.0, 0.5, 1.0, 0.5),
                actor(7, 5.0, 1.0, -4.0, 1.0, 2.0, 1.0),
            ];
            scenes.push((SightWorld::build(&cast), cast));
        }
        let shot = shot_for(1);
        // One warm pass outside the measurement, then the minimum of 5 takes
        // so scheduler noise can only slow the reading, never flatter it.
        for (world, cast) in &scenes {
            access_features(world, cast, &shot, None).expect("warm access scores");
        }
        let mut best_ms: u128 = u128::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let mut scored = 0u32;
            for (world, cast) in &scenes {
                let score =
                    access_features(world, cast, &shot, None).expect("budget access scores");
                scored += score.total;
            }
            assert!(scored > 0, "access totals stay non-zero");
            let elapsed_ms = start.elapsed().as_millis();
            if elapsed_ms < best_ms {
                best_ms = elapsed_ms;
            }
        }
        assert!(
            best_ms <= 20,
            "20 access shots took {best_ms} ms, over the 1 ms/shot budget"
        );
    }
}

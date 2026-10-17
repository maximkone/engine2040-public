//! sight: line of sight, camera visibility, capsule cover, nav reach.
//! Owner: spatial crate. Consumer: the `scene.query` tool (wired in-test below).
//! Units: meters in, integer cm inside; eye points quantized once at entry.
//! Determinism: id-sorted boxes, id-order scans, FNV-1a over integer bytes.
//! Errors: typed `SightError`; off-mesh reach is `Reach::OffMesh`, never a guess.
//! Patterns (EB-6): forge packs/scene holds frustum/overdraw meters but no
//! segment-vs-proxy raycast, so LOS/cover/reach are reimplemented std-only
//! (Recast raycast + O3DE perception shapes; index3 sweep + relation bands
//! reused).
//! Budgets: ray query O(boxes) with 0 allocs, <=0.2 ms; reach BFS is one
//! bounded visited vec sized to the polygon count.

use super::index::{pos_known, size_known, to_cm, Box3};
use super::isovist::{area_cm2, WallSeg};
use super::navcheck::octant;
use super::relation::DIR_RANGE_CM;
use super::Actor;
use engine_core::Fnv64;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// Standing capsule top above the feet, in cm (head ray target).
pub const STAND_TOP_CM: i32 = 170;
/// Crouched capsule top above the feet, in cm (head ray target).
pub const CROUCH_TOP_CM: i32 = 120;
/// Vertical window for the camera cone, in cm (|dy| past this is out of view).
pub const VIEW_DY_CM: i32 = 500;
/// Digest tag so sight bytes never alias another domain hash.
const SIGHT_TAG: &[u8] = b"e2sight:1";

/// Typed sight failures. Invalid input returns `Err`, never a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SightError {
    /// No actor carries this id in the built world.
    UnknownActor(u32),
    /// A query point has a non-finite coordinate.
    NonFinitePoint,
}

impl fmt::Display for SightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownActor(id) => write!(f, "sight: unknown actor {id}"),
            Self::NonFinitePoint => write!(f, "sight: query point is not finite"),
        }
    }
}

impl std::error::Error for SightError {}

/// Defender capsule for cover: standing head at 170 cm, crouched at 120 cm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stance {
    Stand,
    Crouch,
}

/// Cover from a threat eye: full blocks the head ray, half blocks only the
/// chest ray (low wall), none leaves both rays clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    None,
    Half,
    Full,
}

/// Reachability through nav polygons: same or connected polygons reach;
/// disconnected polygons block; endpoints with no polygon are off-mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Reachable,
    Blocked,
    OffMesh,
}

/// Camera visibility: occlusion (`los`) and cone (`in_view`) are reported
/// separately so scene queries can name the reason; `visible` needs both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    pub los: bool,
    pub in_view: bool,
}

impl Visibility {
    /// True only when the target is both unoccluded and inside the cone.
    pub fn visible(&self) -> bool {
        self.los && self.in_view
    }
}

/// Query world: id-sorted proxy boxes. Only valid, positively sized actors
/// index; degenerate rows are invisible to sight. Non-physical actors (arena
/// shells, zones) never block rays but still own centers for queries.
#[derive(Debug, Clone, Default)]
pub struct SightWorld {
    solids: Vec<Box3>,
    boxes: BTreeMap<u32, Box3>,
}

impl SightWorld {
    /// Build from an actor snapshot. Input order never affects the result.
    pub fn build(actors: &[Actor]) -> Self {
        let mut ordered: Vec<&Actor> = actors.iter().collect();
        ordered.sort_by_key(|a| a.id);
        let mut solids = Vec::new();
        let mut boxes = BTreeMap::new();
        for a in ordered {
            if !(pos_known(a) && size_known(a)) {
                continue;
            }
            if !(a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0) {
                continue;
            }
            let (hx, hy, hz) = (
                to_cm(a.sx).abs() / 2,
                to_cm(a.sy).abs() / 2,
                to_cm(a.sz).abs() / 2,
            );
            let (cx, cy, cz) = (to_cm(a.x), to_cm(a.y), to_cm(a.z));
            let b = Box3 {
                id: a.id,
                min_x: cx - hx,
                max_x: cx + hx,
                min_y: cy - hy,
                max_y: cy + hy,
                min_z: cz - hz,
                max_z: cz + hz,
            };
            boxes.insert(a.id, b);
            if a.physical {
                solids.push(b);
            }
        }
        Self { solids, boxes }
    }

    /// Box by actor id, for endpoint lookups.
    pub fn box_of(&self, id: u32) -> Option<&Box3> {
        self.boxes.get(&id)
    }

    /// Physical boxes that do not contain `point_cm` and overlap the open
    /// height band `(lo_y, hi_y)`: what blocks a walker standing there.
    pub(crate) fn blockers_over(&self, point_cm: [i32; 3], lo_y: i32, hi_y: i32) -> Vec<Box3> {
        let blocks = |b: &&Box3| !contains(b, point_cm) && b.min_y < hi_y && b.max_y > lo_y;
        self.solids.iter().filter(blocks).copied().collect()
    }

    /// Actor count with indexed boxes (degenerate rows index nothing).
    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    /// True when no actor indexed a box.
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }
}

/// Nav reachability view. The caller fills it from the nav mesh: `at` maps
/// actor id to polygon (`polygon_at`), `adj` maps polygon id to its neighbor
/// ids (`neighbors`). Spatial owns no nav dependency by design (see
/// `navcheck::NavView`); the mapping is proven in-test below.
#[derive(Debug, Clone, Default)]
pub struct ReachView {
    pub adj: BTreeMap<u32, Vec<u32>>,
    pub at: BTreeMap<u32, u32>,
}

/// Question kind for digest records. Discriminants are frozen: the digest
/// feeds them raw, so variants are append-only, never reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SightKind {
    Los = 0,
    Visible = 1,
    InView = 2,
    Cover = 3,
    Reach = 4,
}

/// One answered question: `(kind, a, b)` with an integer outcome `v`
/// (bools as 0/1, `Cover` as 0/1/2 for none/half/full, `Reach` as 0/1/2
/// for reachable/blocked/off-mesh). No float ever reaches the digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SightAnswer {
    pub kind: SightKind,
    pub a: u32,
    pub b: u32,
    pub v: u32,
}

/// Order-invariant FNV-1a digest over sight answers: sorted by
/// `(kind, a, b)`, then tag plus LE integer bytes. Same questions in any
/// order give the same digest; any flipped outcome flips it.
pub fn digest_answers(answers: &[SightAnswer]) -> u64 {
    let mut sorted = answers.to_vec();
    sorted.sort_by_key(|r| (r.kind, r.a, r.b));
    let mut hash = Fnv64::new();
    hash.write(SIGHT_TAG);
    for r in &sorted {
        hash.write_u8(r.kind as u8);
        hash.write_u32(r.a);
        hash.write_u32(r.b);
        hash.write_u32(r.v);
    }
    hash.finish()
}

/// Quantize a meter point to integer cm, refusing non-finite input.
fn quantize(p: [f32; 3]) -> Result<[i32; 3], SightError> {
    if !p.iter().all(|v| v.is_finite()) {
        return Err(SightError::NonFinitePoint);
    }
    Ok([to_cm(p[0]), to_cm(p[1]), to_cm(p[2])])
}

/// Inclusive containment: endpoint volumes never block their own ray.
fn contains(b: &Box3, p: [i32; 3]) -> bool {
    b.min_x <= p[0]
        && p[0] <= b.max_x
        && b.min_y <= p[1]
        && p[1] <= b.max_y
        && b.min_z <= p[2]
        && p[2] <= b.max_z
}

/// True when some solid box blocks the segment, skipping boxes that contain
/// an endpoint (own capsule, room shell) and honoring the physical flag.
/// The segment test is integer slab math on exact rationals: touching a
/// face or edge is a zero-length touch and never blocks, matching the
/// strict `hit` semantics of the 3D index.
fn blocked(world: &SightWorld, e: [i32; 3], t: [i32; 3]) -> bool {
    let d = [
        i64::from(t[0]) - i64::from(e[0]),
        i64::from(t[1]) - i64::from(e[1]),
        i64::from(t[2]) - i64::from(e[2]),
    ];
    for b in &world.solids {
        if contains(b, e) || contains(b, t) {
            continue;
        }
        if seg_hits(*b, e, d) {
            return true;
        }
    }
    false
}

/// Integer slab test for one box: segment `e + s*d`, `s in [0,1]`, hits the
/// open interior with positive length. Zero-length face touches pass clear.
fn seg_hits(b: Box3, e: [i32; 3], d: [i64; 3]) -> bool {
    let (mut lo_n, mut lo_d) = (0i64, 1i64);
    let (mut hi_n, mut hi_d) = (1i64, 1i64);
    let slabs = [
        (
            i64::from(b.min_x),
            i64::from(b.max_x),
            i64::from(e[0]),
            d[0],
        ),
        (
            i64::from(b.min_y),
            i64::from(b.max_y),
            i64::from(e[1]),
            d[1],
        ),
        (
            i64::from(b.min_z),
            i64::from(b.max_z),
            i64::from(e[2]),
            d[2],
        ),
    ];
    for (mn, mx, pe, di) in slabs {
        if di == 0 {
            // Parallel axis: a strict miss needs the point strictly outside;
            // touching the face plane counts as contact (documented).
            if pe < mn || pe > mx {
                return false;
            }
            continue;
        }
        // Param window for this axis as rationals with positive denominator.
        let (n0, n1, den) = if di > 0 {
            (mn - pe, mx - pe, di)
        } else {
            (pe - mx, pe - mn, -di)
        };
        // Intersect: lo = max(lo, n0/den), hi = min(hi, n1/den).
        if n0 * lo_d > lo_n * den {
            lo_n = n0;
            lo_d = den;
        }
        if n1 * hi_d < hi_n * den {
            hi_n = n1;
            hi_d = den;
        }
        if lo_n * hi_d >= hi_n * lo_d {
            return false;
        }
    }
    // Positive-length overlap strictly inside (0, 1), so mere endpoint
    // touches at t = 0 or t = 1 never block.
    strictly_inside(lo_n, lo_d, hi_n, hi_d)
}

/// True when the param overlap has positive length strictly inside (0, 1):
/// `max(lo, 0) < min(hi, 1)`.
fn strictly_inside(lo_n: i64, lo_d: i64, hi_n: i64, hi_d: i64) -> bool {
    // L = max(lo, 0): L_num/L_den.
    let (l_n, l_d) = if lo_n <= 0 { (0, 1) } else { (lo_n, lo_d) };
    // U = min(hi, 1): U_num/U_den.
    let (u_n, u_d) = if hi_n >= hi_d { (1, 1) } else { (hi_n, hi_d) };
    l_n * u_d < u_n * l_d
}

/// First solid struck by a ray: the actor id, the entry point and the
/// distance from the eye, both in integer cm (rounded half away from zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RayHit {
    pub id: u32,
    pub point_cm: [i32; 3],
    pub dist_cm: i32,
}

/// Entry parameter `lo_n / lo_d` (denominator positive) where the ray
/// `e + s*d` first enters the open interior of `b`, or `None` for a miss or a
/// grazing touch. Same integer slab math as [`seg_hits`], without the segment end.
fn ray_entry(b: &Box3, e: [i32; 3], d: [i64; 3]) -> Option<(i64, i64)> {
    let (mut lo_n, mut lo_d) = (0i64, 1i64);
    let mut hi: Option<(i64, i64)> = None;
    let spans = [
        (b.min_x, b.max_x, e[0], d[0]),
        (b.min_y, b.max_y, e[1], d[1]),
        (b.min_z, b.max_z, e[2], d[2]),
    ];
    for (mn, mx, pe, di) in spans {
        if di == 0 {
            if pe <= mn || pe >= mx {
                return None;
            }
            continue;
        }
        let (n0, n1, den) = if di > 0 {
            (i64::from(mn - pe), i64::from(mx - pe), di)
        } else {
            (i64::from(pe - mx), i64::from(pe - mn), -di)
        };
        if n0 * lo_d > lo_n * den {
            (lo_n, lo_d) = (n0, den);
        }
        if hi.is_none_or(|(hn, hd)| n1 * hd < hn * den) {
            hi = Some((n1, den));
        }
    }
    let (hn, hd) = hi?;
    (lo_n * hd < hn * lo_d).then_some((lo_n, lo_d))
}

/// Round `v * n / den` half away from zero (`den > 0`).
fn mul_div_round(v: i64, n: i64, den: i64) -> i64 {
    let num = i128::from(v) * i128::from(n) * 2;
    let den2 = i128::from(den) * 2;
    let half = if num < 0 {
        -i128::from(den)
    } else {
        i128::from(den)
    };
    i64::try_from((num + half) / den2).unwrap_or(i64::MAX)
}

/// First solid hit along `eye_cm + s * dir` for `s >= 0`, within `range_cm`.
/// `dir` is any non-zero integer vector (its length scales nothing: distance
/// is real cm). Boxes containing the eye are skipped, like the LOS ray;
/// non-physical actors never block. Nearest entry wins, equal entries go to
/// the smaller id. Integer only; 0 allocs, O(solids).
pub fn ray_first_hit(
    world: &SightWorld,
    eye_cm: [i32; 3],
    dir: [i32; 3],
    range_cm: i32,
) -> Option<RayHit> {
    let d = dir.map(i64::from);
    let len_sq = d.iter().map(|v| v * v).sum::<i64>();
    if len_sq == 0 {
        return None;
    }
    let len = i64::try_from(len_sq.cast_unsigned().isqrt()).ok()?;
    let mut best: Option<(i64, i64, u32)> = None;
    for b in &world.solids {
        if contains(b, eye_cm) {
            continue;
        }
        let Some((n, den)) = ray_entry(b, eye_cm, d) else {
            continue;
        };
        if best.is_none_or(|(bn, bd, _)| n * bd < bn * den) {
            best = Some((n, den, b.id));
        }
    }
    let (n, den, id) = best?;
    let dist = mul_div_round(len, n, den);
    if dist > i64::from(range_cm) {
        return None;
    }
    let at =
        |axis: usize| eye_cm[axis] + i32::try_from(mul_div_round(d[axis], n, den)).unwrap_or(0);
    Some(RayHit {
        id,
        point_cm: [at(0), at(1), at(2)],
        dist_cm: i32::try_from(dist).unwrap_or(i32::MAX),
    })
}

/// Line of sight between two arbitrary points in meters. Pure occlusion:
/// no range limit, no cone. Endpoint boxes never block their own ray.
pub fn los_point(
    world: &SightWorld,
    eye_m: [f32; 3],
    target_m: [f32; 3],
) -> Result<bool, SightError> {
    let e = quantize(eye_m)?;
    let t = quantize(target_m)?;
    Ok(!blocked(world, e, t))
}

/// Line of sight between two actor centers. Unknown ids refuse loudly.
pub fn los_actors(world: &SightWorld, a: u32, b: u32) -> Result<bool, SightError> {
    let (ba, bb) = match (world.box_of(a), world.box_of(b)) {
        (Some(x), Some(y)) => (*x, *y),
        _ => {
            let missing = if world.box_of(a).is_none() { a } else { b };
            return Err(SightError::UnknownActor(missing));
        }
    };
    let center = |x: Box3| {
        [
            (x.min_x + x.max_x) / 2,
            (x.min_y + x.max_y) / 2,
            (x.min_z + x.max_z) / 2,
        ]
    };
    Ok(!blocked(world, center(ba), center(bb)))
}

/// Camera visibility of one actor from an eye point with a yaw in radians.
/// Cone: 90-degree forward fan (octant-snapped, libm-free) inside the relation
/// directional horizon plus a vertical window; occlusion is the LOS ray to
/// the target center. Non-finite yaw blinds the cone but never the ray.
pub fn visible(
    world: &SightWorld,
    eye_m: [f32; 3],
    yaw: f32,
    target: u32,
) -> Result<Visibility, SightError> {
    let e = quantize(eye_m)?;
    let b = *world
        .box_of(target)
        .ok_or(SightError::UnknownActor(target))?;
    let c = [
        (b.min_x + b.max_x) / 2,
        (b.min_y + b.max_y) / 2,
        (b.min_z + b.max_z) / 2,
    ];
    let dx = i64::from(c[0]) - i64::from(e[0]);
    let dy = i64::from(c[1]) - i64::from(e[1]);
    let dz = i64::from(c[2]) - i64::from(e[2]);
    let range = i64::from(DIR_RANGE_CM);
    let mut in_view = dx * dx + dz * dz <= range * range && dy.abs() <= i64::from(VIEW_DY_CM);
    if in_view {
        if !yaw.is_finite() {
            in_view = false;
        } else if dx == 0 && dz == 0 {
            // Overhead: horizontal cone is vacuous, range/window decide.
        } else {
            let (fx, fz) = octant(yaw);
            let (fx, fz) = (i64::from(fx), i64::from(fz));
            let dot = fx * dx + fz * dz;
            let cross = (fx * dz - fz * dx).abs();
            in_view = dot > 0 && cross <= dot;
        }
    }
    Ok(Visibility {
        los: !blocked(world, e, c),
        in_view,
    })
}

/// Cover of a defender standing at `feet_m` from a threat eye at
/// `threat_eye_m`, for a stance capsule (170 cm stand, 120 cm crouch).
/// Head ray blocked means full; only the chest ray blocked means half.
pub fn cover(
    world: &SightWorld,
    feet_m: [f32; 3],
    threat_eye_m: [f32; 3],
    stance: Stance,
) -> Result<Cover, SightError> {
    let f = quantize(feet_m)?;
    let t = quantize(threat_eye_m)?;
    let top = match stance {
        Stance::Stand => STAND_TOP_CM,
        Stance::Crouch => CROUCH_TOP_CM,
    };
    let head = [f[0], f[1] + top, f[2]];
    let chest = [f[0], f[1] + top / 2, f[2]];
    if blocked(world, t, head) {
        Ok(Cover::Full)
    } else if blocked(world, t, chest) {
        Ok(Cover::Half)
    } else {
        Ok(Cover::None)
    }
}

/// Reachability between two actors through nav polygons: same polygon or a
/// connected chain reaches; disconnected polygons block; an endpoint with
/// no recorded polygon is off-mesh. Deterministic BFS over sorted edges.
pub fn reachable(view: &ReachView, from: u32, to: u32) -> Reach {
    let (pa, pb) = match (view.at.get(&from), view.at.get(&to)) {
        (Some(a), Some(b)) => (*a, *b),
        _ => return Reach::OffMesh,
    };
    if pa == pb {
        return Reach::Reachable;
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = VecDeque::new();
    seen.insert(pa);
    queue.push_back(pa);
    while let Some(cur) = queue.pop_front() {
        let mut next: Vec<u32> = view.adj.get(&cur).cloned().unwrap_or_default();
        next.sort_unstable();
        next.dedup();
        for n in next {
            if n == pb {
                return Reach::Reachable;
            }
            if seen.insert(n) {
                queue.push_back(n);
            }
        }
    }
    Reach::Blocked
}

/// XZ footprint walls of the world's solids: four canonical edges per box
/// (bottom, right, top, left; each `a <= b` holds since `min <= max`),
/// emitted in the world's id order with a fixed edge order, so the wall
/// set replays identically. Non-physical shells own no footprint: like the
/// ray path, they never block.
fn footprint_walls(world: &SightWorld) -> Vec<WallSeg> {
    let mut walls = Vec::new();
    for b in &world.solids {
        let (x0, x1, z0, z1) = (b.min_x, b.max_x, b.min_z, b.max_z);
        walls.push(WallSeg {
            ax_cm: x0,
            az_cm: z0,
            bx_cm: x1,
            bz_cm: z0,
        });
        walls.push(WallSeg {
            ax_cm: x1,
            az_cm: z0,
            bx_cm: x1,
            bz_cm: z1,
        });
        walls.push(WallSeg {
            ax_cm: x0,
            az_cm: z1,
            bx_cm: x1,
            bz_cm: z1,
        });
        walls.push(WallSeg {
            ax_cm: x0,
            az_cm: z0,
            bx_cm: x0,
            bz_cm: z1,
        });
    }
    walls
}

/// Visible floor area in square centimetres from an eye point: the XZ
/// footprints of the world's solids become [`WallSeg`] walls, the integer
/// isovist sweep closes the visible ring, and
/// [`area_cm2`](super::isovist::area_cm2) reports its area. Open ground
/// (no solids) has no bounded ring, so it reports 0, and short rings
/// report 0 the same way. Integer cm throughout; the same world and eye
/// give the same area twice. Presentation geometry only: no tick input is
/// read, so replays hash identically. Consumer: the sight readability
/// gate (next slice after GS30).
/// Errors: [`SightError::NonFinitePoint`] for a non-finite eye.
pub fn visible_area_cm2(world: &SightWorld, eye_m: [f32; 3]) -> Result<i64, SightError> {
    let e = quantize(eye_m)?;
    let walls = footprint_walls(world);
    let ring = super::isovist::isovist([e[0], e[2]], &walls);
    Ok(area_cm2(&ring))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture builder takes each field positionally"
    )]
    fn mk(
        id: u32,
        name: &str,
        x: f32,
        y: f32,
        z: f32,
        sx: f32,
        sy: f32,
        sz: f32,
        yaw: f32,
        physical: bool,
    ) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical,
            vx: 0.0,
            vz: 0.0,
            yaw,
        }
    }

    fn prop(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        mk(id, &format!("p{id}"), x, y, z, sx, sy, sz, 0.0, true)
    }

    fn eye_target(id: u32, x: f32, y: f32, z: f32) -> Actor {
        mk(id, &format!("t{id}"), x, y, z, 1.0, 1.8, 1.0, 0.0, true)
    }

    /// Open field: no blockers at all.
    fn open_world() -> SightWorld {
        SightWorld::build(&[])
    }

    /// One wall at x = 5 m: box x[450,550], y[0,300], z[-500,500] (cm).
    fn wall_world() -> SightWorld {
        SightWorld::build(&[prop(1, 5.0, 1.5, 0.0, 1.0, 3.0, 10.0)])
    }

    fn wall_world_nonphysical() -> SightWorld {
        SightWorld::build(&[mk(1, "ghost", 5.0, 1.5, 0.0, 1.0, 3.0, 10.0, 0.0, false)])
    }

    fn watcher_world() -> (SightWorld, u32) {
        // Watcher at (-6, 1.6, 0) facing +X (yaw 0) behind the wall.
        let mut actors = vec![
            prop(1, 5.0, 1.5, 0.0, 1.0, 3.0, 10.0),
            mk(10, "watcher", -6.0, 1.6, 0.0, 1.0, 1.8, 1.0, 0.0, true),
            eye_target(20, 10.0, 1.6, 0.0),
            eye_target(21, -6.0, 1.6, 10.0),
            eye_target(22, 2.0, 1.6, 0.0),
            eye_target(23, -10.0, 1.6, 0.0),
            eye_target(24, 2.0, 1.6, 8.0),
            eye_target(25, -2.0, 1.6, 8.0),
        ];
        actors.sort_by_key(|a| a.id);
        (SightWorld::build(&actors), 10)
    }

    fn ring_world() -> (SightWorld, u32) {
        // Watcher at origin facing +X; ring targets at r = 8 m, y = 1.6.
        let d = 5.66f32;
        let mut actors = vec![
            mk(11, "watcher", 0.0, 1.6, 0.0, 1.0, 1.8, 1.0, 0.0, true),
            eye_target(30, 8.0, 1.6, 0.0),
            eye_target(31, d, 1.6, d),
            eye_target(32, 0.0, 1.6, 8.0),
            eye_target(33, -d, 1.6, d),
            eye_target(34, -8.0, 1.6, 0.0),
            eye_target(35, -d, 1.6, -d),
            eye_target(36, 0.0, 1.6, -8.0),
            eye_target(37, d, 1.6, -d),
        ];
        actors.sort_by_key(|a| a.id);
        (SightWorld::build(&actors), 11)
    }

    /// Horizon pair on diverging sightlines so neither occludes the other:
    /// 38 past the 20 m horizon, 39 inside it on the north-east diagonal.
    fn range_world() -> SightWorld {
        SightWorld::build(&[
            mk(11, "watcher", 0.0, 1.6, 0.0, 1.0, 1.8, 1.0, 0.0, true),
            eye_target(38, 25.0, 1.6, 0.0),
            eye_target(39, 13.4, 1.6, 13.4),
        ])
    }

    /// Vertical pair on diverging sightlines: 40 above the 5 m window,
    /// 41 on its edge, neither ray crossing the other's box.
    fn height_world() -> SightWorld {
        SightWorld::build(&[
            mk(11, "watcher", 0.0, 1.6, 0.0, 1.0, 1.8, 1.0, 0.0, true),
            eye_target(40, 5.0, 7.6, 6.0),
            eye_target(41, 8.0, 6.6, -2.0),
        ])
    }

    fn cover_world(height_m: f32) -> SightWorld {
        // One wall centered at x = 5 m, z = 0, spanning z +/- 3 m.
        SightWorld::build(&[prop(50, 5.0, height_m / 2.0, 0.0, 1.0, height_m, 6.0)])
    }

    fn duel_world() -> SightWorld {
        SightWorld::build(&[
            prop(40, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(41, 10.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(42, 5.0, 1.5, 0.0, 1.0, 3.0, 1.0),
            prop(43, 10.0, 0.5, 5.0, 1.0, 1.0, 1.0),
        ])
    }

    /// Closed 10 m room, interior x,z in [0,1000] cm: four 1 m walls
    /// outside the interior line, eye (5, 1.6, 5) at the centre.
    fn room_world() -> SightWorld {
        SightWorld::build(&[
            prop(1, 5.0, 1.5, -0.5, 12.0, 3.0, 1.0),
            prop(2, 5.0, 1.5, 10.5, 12.0, 3.0, 1.0),
            prop(3, -0.5, 1.5, 5.0, 1.0, 3.0, 12.0),
            prop(4, 10.5, 1.5, 5.0, 1.0, 3.0, 12.0),
        ])
    }

    fn reach_view() -> ReachView {
        let mut adj = BTreeMap::new();
        adj.insert(0u32, vec![1u32]);
        adj.insert(1u32, vec![0u32, 2u32]);
        adj.insert(2u32, vec![1u32]);
        adj.insert(9u32, vec![]);
        let mut at = BTreeMap::new();
        at.insert(1u32, 0u32);
        at.insert(2u32, 1u32);
        at.insert(3u32, 2u32);
        at.insert(4u32, 9u32);
        ReachView { adj, at }
    }

    fn b(v: bool) -> u8 {
        u8::from(v)
    }

    fn cover_v(c: Cover) -> u8 {
        match c {
            Cover::None => 0,
            Cover::Half => 1,
            Cover::Full => 2,
        }
    }

    fn reach_v(r: Reach) -> u8 {
        match r {
            Reach::Reachable => 0,
            Reach::Blocked => 1,
            Reach::OffMesh => 2,
        }
    }

    /// The 100-question gold set: (name, obtained outcome, expected outcome).
    /// Outcomes are integer codes so the table feeds the digest unchanged.
    fn gold() -> Vec<(&'static str, u8, u8)> {
        let mut g: Vec<(&'static str, u8, u8)> = Vec::new();
        // --- Open field radials: 8 dirs, all clear (1-8).
        let open = open_world();
        let eye = [0.0, 1.6, 0.0];
        let radials = [
            ([8.0, 1.6, 0.0], "east"),
            ([5.66, 1.6, 5.66], "north-east"),
            ([0.0, 1.6, 8.0], "north"),
            ([-5.66, 1.6, 5.66], "north-west"),
            ([-8.0, 1.6, 0.0], "west"),
            ([-5.66, 1.6, -5.66], "south-west"),
            ([0.0, 1.6, -8.0], "south"),
            ([5.66, 1.6, -5.66], "south-east"),
        ];
        for (t, name) in radials {
            g.push((name, b(los_point(&open, eye, t).unwrap()), 1));
        }
        // --- Degenerate rays (9-10).
        g.push(("eye==target", b(los_point(&open, eye, eye).unwrap()), 1));
        g.push((
            "far-open-50m",
            b(los_point(&open, eye, [50.0, 1.6, 0.0]).unwrap()),
            1,
        ));
        // --- Wall: through / over / around (11-13, 25).
        let wall = wall_world();
        g.push((
            "wall-through",
            b(los_point(&wall, eye, [10.0, 1.6, 0.0]).unwrap()),
            0,
        ));
        g.push((
            "wall-over",
            b(los_point(&wall, eye, [10.0, 5.0, 0.0]).unwrap()),
            1,
        ));
        g.push((
            "wall-around",
            b(los_point(&wall, eye, [10.0, 1.6, 12.0]).unwrap()),
            1,
        ));
        g.push((
            "wall-far-50m",
            b(los_point(&wall, eye, [50.0, 1.6, 0.0]).unwrap()),
            0,
        ));
        // --- Height sweep at x = 10 m: clear iff y(0.45) >= 300, i.e. Y >= 472.
        for h in [1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5] {
            let name = match h as i32 {
                1 => "h100-blocked",
                2 => "h200-blocked",
                3 => "h300-blocked",
                4 => "h400-blocked",
                _ => "hmid-blocked",
            };
            g.push((name, b(los_point(&wall, eye, [10.0, h, 0.0]).unwrap()), 0));
        }
        for h in [5.0, 5.5, 6.0] {
            let name = match h as i32 {
                5 => "h500-clear",
                6 => "h600-clear",
                _ => "hmid-clear",
            };
            g.push((name, b(los_point(&wall, eye, [10.0, h, 0.0]).unwrap()), 1));
        }
        // --- Bad input refuses (26).
        g.push((
            "nan-eye-refuses",
            b(los_point(&wall, [f32::NAN, 1.6, 0.0], [10.0, 1.6, 0.0]).is_err()),
            1,
        ));
        // --- Watcher behind the wall: los / view / visible split (27-44).
        let (ww, _watcher) = watcher_world();
        let we = [-6.0, 1.6, 0.0];
        for (id, los, view, vis, tag) in [
            (20u32, 0u8, 1u8, 0u8, "ahead-blocked"),
            (21, 1, 0, 0, "side-clear-unseen"),
            (22, 1, 1, 1, "near-clear-seen"),
            (23, 1, 0, 0, "behind-clear-unseen"),
            (24, 1, 1, 1, "cone-edge-seen"),
            (25, 1, 0, 0, "cone-outside-unseen"),
        ] {
            let v = visible(&ww, we, 0.0, id).unwrap();
            g.push((tag, b(v.los), los));
            g.push((tag, b(v.in_view), view));
            g.push((tag, b(v.visible()), vis));
        }
        // --- Ring + range + vertical + blind yaw (45-83).
        let (rw, _rwatcher) = ring_world();
        let re = [0.0, 1.6, 0.0];
        for (id, view, tag) in [
            (30u32, 1u8, "ring-e"),
            (31, 1, "ring-ne-edge"),
            (32, 0, "ring-n"),
            (33, 0, "ring-nw"),
            (34, 0, "ring-w"),
            (35, 0, "ring-sw"),
            (36, 0, "ring-s"),
            (37, 1, "ring-se-edge"),
        ] {
            let v = visible(&rw, re, 0.0, id).unwrap();
            g.push((tag, b(v.in_view), view));
            g.push((tag, b(v.los), 1));
            g.push((tag, b(v.visible()), view));
        }
        for (id, view, vis, tag) in [
            (38u32, 0u8, 0u8, "past-horizon"),
            (39, 1, 1, "inside-horizon"),
        ] {
            let v = visible(&range_world(), re, 0.0, id).unwrap();
            g.push((tag, b(v.in_view), view));
            g.push((tag, b(v.los), 1));
            g.push((tag, b(v.visible()), vis));
        }
        for (id, view, vis, tag) in [(40u32, 0u8, 0u8, "above-window"), (41, 1, 1, "window-edge")] {
            let v = visible(&height_world(), re, 0.0, id).unwrap();
            g.push((tag, b(v.in_view), view));
            g.push((tag, b(v.los), 1));
            g.push((tag, b(v.visible()), vis));
        }
        let blind = visible(&rw, re, f32::NAN, 30).unwrap();
        g.push(("blind-los", b(blind.los), 1));
        g.push(("blind-view", b(blind.in_view), 0));
        g.push(("blind-visible", b(blind.visible()), 0));
        // --- Cover ladder: feet origin, threat eye at (10, 1.5, 0) (84-89).
        let feet = [0.0, 0.0, 0.0];
        let threat = [10.0, 1.5, 0.0];
        for (h, stand, crouch, tag) in [
            (1.0f32, Cover::None, Cover::None, "low-wall"),
            (1.4, Cover::Half, Cover::Full, "mid-wall"),
            (2.0, Cover::Full, Cover::Full, "tall-wall"),
        ] {
            let w = cover_world(h);
            g.push((
                tag,
                cover_v(cover(&w, feet, threat, Stance::Stand).unwrap()),
                cover_v(stand),
            ));
            g.push((
                tag,
                cover_v(cover(&w, feet, threat, Stance::Crouch).unwrap()),
                cover_v(crouch),
            ));
        }
        // --- Reach through nav polygons (90-97).
        let rv = reach_view();
        for (a, bb, want, tag) in [
            (1u32, 2u32, Reach::Reachable, "adjacent"),
            (1, 3, Reach::Reachable, "chain"),
            (1, 1, Reach::Reachable, "same"),
            (1, 4, Reach::Blocked, "island"),
            (4, 1, Reach::Blocked, "island-rev"),
            (1, 5, Reach::OffMesh, "offmesh"),
            (5, 1, Reach::OffMesh, "offmesh-rev"),
            (666, 1, Reach::OffMesh, "unknown"),
        ] {
            g.push((tag, reach_v(reachable(&rv, a, bb)), reach_v(want)));
        }
        // --- Actor-center rays with a pillar occluder (98-100).
        let duel = duel_world();
        g.push(("pillar-blocks", b(los_actors(&duel, 40, 41).unwrap()), 0));
        g.push(("pillar-misses", b(los_actors(&duel, 40, 43).unwrap()), 1));
        g.push((
            "unknown-actor-refuses",
            b(los_actors(&duel, 40, 777).is_err()),
            1,
        ));
        // --- Shells and vaults never blind (101-105).
        let mut shelled = vec![
            prop(40, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(41, 10.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(42, 5.0, 1.5, 0.0, 1.0, 3.0, 1.0),
            prop(43, 10.0, 0.5, 5.0, 1.0, 1.0, 1.0),
            mk(44, "shell", 5.0, 3.0, 0.0, 30.0, 8.0, 30.0, 0.0, false),
        ];
        shelled.sort_by_key(|a| a.id);
        let shell = SightWorld::build(&shelled);
        g.push((
            "shell-keeps-clear",
            b(los_actors(&shell, 40, 43).unwrap()),
            1,
        ));
        let mut vaulted = vec![
            prop(40, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(41, 10.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(42, 5.0, 1.5, 0.0, 1.0, 3.0, 1.0),
            prop(43, 10.0, 0.5, 5.0, 1.0, 1.0, 1.0),
            prop(45, 5.0, 1.0, 2.0, 20.0, 4.0, 20.0),
        ];
        vaulted.sort_by_key(|a| a.id);
        let vault = SightWorld::build(&vaulted);
        g.push((
            "vault-keeps-blocked",
            b(los_actors(&vault, 40, 41).unwrap()),
            0,
        ));
        g.push((
            "vault-keeps-clear",
            b(los_actors(&vault, 40, 43).unwrap()),
            1,
        ));
        // --- Flags and degenerate rows (106-107).
        g.push((
            "nonphysical-never-blocks",
            b(los_point(&wall_world_nonphysical(), eye, [10.0, 1.6, 0.0]).unwrap()),
            1,
        ));
        let with_flat = SightWorld::build(&[
            prop(1, 5.0, 1.5, 0.0, 1.0, 3.0, 10.0),
            mk(61, "flat", 4.0, 1.6, 0.0, 0.0, 1.0, 1.0, 0.0, true),
        ]);
        // Flat (zero-size) rows index nothing, so the wall still decides.
        g.push((
            "flat-row-ignored",
            b(los_point(&with_flat, eye, [10.0, 1.6, 0.0]).unwrap()),
            0,
        ));
        g
    }

    #[test]
    fn gold_sight_reach_rate() {
        let g = gold();
        assert!(g.len() >= 100, "gold set holds {} questions", g.len());
        let mut failures: Vec<&str> = Vec::new();
        for (name, got, want) in &g {
            if got != want {
                failures.push(name);
            }
        }
        let pass = g.len() - failures.len();
        assert!(
            pass * 100 >= 95 * g.len(),
            "sight+reach {pass}/{} < 95%: {failures:?}",
            g.len()
        );
        assert!(failures.is_empty(), "gold failures: {failures:?}");
    }

    #[test]
    fn digest_stable_twice_and_order_invariant() {
        // Answers mirror the gold outcomes with stable (kind, a, b) keys.
        let g = gold();
        let answers: Vec<SightAnswer> = g
            .iter()
            .enumerate()
            .map(|(i, (_, got, _))| SightAnswer {
                kind: match i % 5 {
                    0 => SightKind::Los,
                    1 => SightKind::Visible,
                    2 => SightKind::InView,
                    3 => SightKind::Cover,
                    _ => SightKind::Reach,
                },
                a: i as u32,
                b: 0,
                v: u32::from(*got),
            })
            .collect();
        let d1 = digest_answers(&answers);
        let d2 = digest_answers(&answers);
        assert_eq!(d1, d2, "digest stable across runs");
        let mut rev = answers.clone();
        rev.reverse();
        assert_eq!(digest_answers(&rev), d1, "digest order-invariant");
        let mut flipped = answers.clone();
        flipped[0].v ^= 1;
        assert_ne!(
            digest_answers(&flipped),
            d1,
            "one flipped outcome flips the digest"
        );
    }

    #[test]
    fn build_order_never_moves_answers() {
        // Reversed input actors give identical worlds and identical rays.
        let actors = vec![
            prop(40, 0.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(41, 10.0, 0.5, 0.0, 1.0, 1.0, 1.0),
            prop(42, 5.0, 1.5, 0.0, 1.0, 3.0, 1.0),
            prop(43, 10.0, 0.5, 5.0, 1.0, 1.0, 1.0),
        ];
        let mut rev = actors.clone();
        rev.reverse();
        let (a, b) = (SightWorld::build(&actors), SightWorld::build(&rev));
        assert_eq!(a.len(), b.len());
        for id in [40u32, 41, 42, 43] {
            let (xa, xb) = (a.box_of(id).unwrap(), b.box_of(id).unwrap());
            for (pa, pb) in [
                (xa.min_x, xb.min_x),
                (xa.max_x, xb.max_x),
                (xa.min_y, xb.min_y),
                (xa.max_y, xb.max_y),
                (xa.min_z, xb.min_z),
                (xa.max_z, xb.max_z),
            ] {
                assert_eq!(pa, pb, "box {id} stable under input reorder");
            }
        }
        for (x, y) in [(40u32, 41u32), (40, 43), (41, 43)] {
            assert_eq!(los_actors(&a, x, y), los_actors(&b, x, y));
        }
        // Digest over both builds agrees as well.
        let ans = |w: &SightWorld| {
            vec![
                SightAnswer {
                    kind: SightKind::Los,
                    a: 40,
                    b: 41,
                    v: u32::from(los_actors(w, 40, 41).unwrap()),
                },
                SightAnswer {
                    kind: SightKind::Los,
                    a: 40,
                    b: 43,
                    v: u32::from(los_actors(w, 40, 43).unwrap()),
                },
            ]
        };
        assert_eq!(digest_answers(&ans(&a)), digest_answers(&ans(&b)));
    }

    #[test]
    fn sp10_consumer_shape_ids_with_reasons() {
        // The `scene.query` tool's shape, written against the public API only:
        // visible + reachable actors with one reason string each.
        fn query_visible(
            world: &SightWorld,
            view: &ReachView,
            eye: [f32; 3],
            yaw: f32,
            anchor: u32,
            candidates: &[u32],
        ) -> Vec<(u32, &'static str)> {
            let mut out = Vec::new();
            for id in candidates {
                let vis = match visible(world, eye, yaw, *id) {
                    Ok(v) => v,
                    Err(_) => {
                        out.push((*id, "blocked:unknown"));
                        continue;
                    }
                };
                if !vis.in_view {
                    out.push((*id, "blocked:out_of_view"));
                } else if !vis.los {
                    out.push((*id, "blocked:occluded"));
                } else if reachable(view, anchor, *id) != Reach::Reachable {
                    out.push((*id, "blocked:unreachable"));
                } else {
                    out.push((*id, "visible"));
                }
            }
            out
        }

        // Anchor 22 stands before the wall; 20 hides behind it; 21 is aside.
        let (world, _) = watcher_world();
        let mut view = reach_view();
        // Watcher-adjacent actors share polygon 0; actor 20 sits alone on 9.
        view.at.insert(10, 0);
        view.at.insert(20, 9);
        view.at.insert(21, 0);
        view.at.insert(22, 0);
        let got = query_visible(&world, &view, [-6.0, 1.6, 0.0], 0.0, 22, &[20, 21, 22, 777]);
        assert_eq!(
            got,
            vec![
                (20, "blocked:occluded"),
                (21, "blocked:out_of_view"),
                (22, "visible"),
                (777, "blocked:unknown"),
            ]
        );
        // Unreachable-but-seen: candidate 22 sits on the island polygon
        // while the anchor stays on the main one.
        view.at.insert(22, 9);
        let got2 = query_visible(&world, &view, [-6.0, 1.6, 0.0], 0.0, 21, &[22]);
        assert_eq!(got2, vec![(22, "blocked:unreachable")]);
    }

    #[test]
    fn nv1_mapping_shape_polygon_at_plus_neighbors() {
        // Documents the nav mesh -> ReachView mapping the caller owns: walkable
        // polygon per actor plus neighbor edges, both sorted and stable.
        // Mirrors `NavMesh::polygon_at` (off-grid is off-mesh) and
        // `neighbors` (ascending, never self) without importing the nav crate.
        let mut adj = BTreeMap::new();
        adj.insert(0u32, vec![1u32]);
        adj.insert(1u32, vec![2u32, 0u32]); // caller order unsorted on purpose
        let mut at = BTreeMap::new();
        at.insert(7u32, 0u32);
        at.insert(8u32, 1u32);
        let view = ReachView { adj, at };
        assert_eq!(reachable(&view, 7, 8), Reach::Reachable);
        assert_eq!(reachable(&view, 7, 9), Reach::OffMesh);
        // Self always reaches when on the mesh, even with no edges.
        assert_eq!(reachable(&view, 7, 7), Reach::Reachable);
    }

    #[test]
    fn errors_name_their_cause() {
        let world = wall_world();
        assert_eq!(world.len(), 1);
        assert!(!world.is_empty());
        assert!(open_world().is_empty());
        assert_eq!(
            los_actors(&world, 1, 999),
            Err(SightError::UnknownActor(999))
        );
        assert_eq!(
            visible(&world, [0.0, 1.6, 0.0], 0.0, 999),
            Err(SightError::UnknownActor(999))
        );
        assert_eq!(
            cover(
                &world,
                [f32::NAN, 0.0, 0.0],
                [10.0, 1.5, 0.0],
                Stance::Stand
            ),
            Err(SightError::NonFinitePoint)
        );
        assert_eq!(
            format!("{}", SightError::UnknownActor(7)),
            "sight: unknown actor 7"
        );
    }

    #[test]
    fn visible_area_wires_isovist_ring() {
        // Oracle (fixed before the run): open ground has no solids, so the
        // sweep sees no walls and the area is 0; a closed 10 m room from
        // its centre sees a positive floor; one pillar inside the room
        // strictly shrinks the visible floor; every answer replays.
        let eye = [5.0, 1.6, 5.0];
        assert_eq!(visible_area_cm2(&open_world(), eye).unwrap(), 0);
        let room = room_world();
        let whole = visible_area_cm2(&room, eye).unwrap();
        assert!(whole > 0, "closed room sees a positive floor");
        assert_eq!(visible_area_cm2(&room, eye).unwrap(), whole);
        let mut pillared = vec![
            prop(1, 5.0, 1.5, -0.5, 12.0, 3.0, 1.0),
            prop(2, 5.0, 1.5, 10.5, 12.0, 3.0, 1.0),
            prop(3, -0.5, 1.5, 5.0, 1.0, 3.0, 12.0),
            prop(4, 10.5, 1.5, 5.0, 1.0, 3.0, 12.0),
            prop(5, 7.0, 1.5, 5.0, 1.0, 3.0, 1.0),
        ];
        pillared.sort_by_key(|a| a.id);
        let hall = SightWorld::build(&pillared);
        let cut = visible_area_cm2(&hall, eye).unwrap();
        assert!(cut > 0, "pillared room still sees floor");
        assert!(cut < whole, "pillar strictly shrinks the visible floor");
        assert_eq!(visible_area_cm2(&hall, eye).unwrap(), cut);
        // The XZ footprint mapping is exact: one wall box becomes its four
        // canonical edges (bottom, right, top, left).
        assert_eq!(
            footprint_walls(&wall_world()),
            vec![
                WallSeg {
                    ax_cm: 450,
                    az_cm: -500,
                    bx_cm: 550,
                    bz_cm: -500
                },
                WallSeg {
                    ax_cm: 550,
                    az_cm: -500,
                    bx_cm: 550,
                    bz_cm: 500
                },
                WallSeg {
                    ax_cm: 450,
                    az_cm: 500,
                    bx_cm: 550,
                    bz_cm: 500
                },
                WallSeg {
                    ax_cm: 450,
                    az_cm: -500,
                    bx_cm: 450,
                    bz_cm: 500
                },
            ]
        );
        assert_eq!(
            visible_area_cm2(&room, [f32::NAN, 1.6, 5.0]).unwrap_err(),
            SightError::NonFinitePoint
        );
    }

    #[test]
    #[ignore = "timing: ray budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn ray_budget_half_ms_on_500_boxes() {
        // 500 scattered crates; every query scans all solids with 0 allocs.
        // The target floats above every roofline, so each ray pays the full
        // 500-box scan: the honest worst case, not an early-out.
        let actors: Vec<Actor> = (0..500u32)
            .map(|i| {
                let x = (i % 25) as f32 * 2.0;
                let z = (i / 25) as f32 * 2.0;
                prop(1000 + i, x, 1.0, z, 1.0, 2.0, 1.0)
            })
            .collect();
        let world = SightWorld::build(&actors);
        let eye = [0.0, 1.6, -10.0];
        let start = std::time::Instant::now();
        let mut hits = 0u32;
        let n = 2000u32;
        for _ in 0..n {
            if los_point(&world, eye, [48.0, 10.0, 38.0]).unwrap() {
                hits += 1;
            }
        }
        assert_eq!(hits, n, "all rays clear the roofs");
        let per_us = start.elapsed().as_secs_f64() / f64::from(n) * 1e6;
        println!("ray: {per_us:.3} us per query over 500 boxes, hits={hits}");
        assert!(
            per_us <= 200.0,
            "ray budget 0.2 ms, measured {per_us:.3} us"
        );
    }
}

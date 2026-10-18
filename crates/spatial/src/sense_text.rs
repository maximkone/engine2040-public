//! sense: CPU sight verbs that answer in plain text (SP69).
//!
//! Consumer: the assistant door verbs `sense.view`, `sense.lidar`, `sense.map`
//! and `scene.pixel`. No render, no vision model: every answer is cast from the
//! same integer-cm solids as [`crate::sight`]. Owner: spatial crate.
//! Units: integer cm, yaw in whole degrees (0 faces +z, 90 faces +x), depth in
//! buckets of [`BUCKET_CM`]. Determinism: integer slab casts and a 1/1024 sine
//! table, no libm, no float, no clock; the same world and camera give the same
//! text. Errors: typed [`SenseError`]; a ray that hits nothing is a miss token,
//! never an error. Budgets: one ray is O(solids) with no allocation.

use crate::sight::{ray_first_hit, RayHit, SightWorld, STAND_TOP_CM};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Longest ray, in cm.
pub const SENSE_RANGE_CM: i32 = 2400;
/// Depth bucket width in cm; buckets run `0..=7`.
pub const BUCKET_CM: i32 = 300;
/// Largest view edge in cells.
pub const MAX_VIEW: i32 = 128;
/// Rays in one lidar fan (5 degrees apart).
pub const LIDAR_RAYS: i32 = 72;
/// Degrees between lidar rays.
pub const LIDAR_STEP_DEG: i32 = 5;
/// Map cell edge in cm.
pub const MAP_CELL_CM: i32 = 50;
/// Largest map radius in cm.
pub const MAX_MAP_RADIUS_CM: i32 = 1500;
/// Highest curb a walker steps over, in cm above the map center height.
const STEP_CM: i32 = 30;
/// Sine of 0..=90 degrees, scaled by 1024 and rounded.
const SIN_1K: [i32; 91] = [
    0, 18, 36, 54, 71, 89, 107, 125, 143, 160, 178, 195, 213, 230, 248, 265, 282, 299, 316, 333,
    350, 367, 384, 400, 416, 433, 449, 465, 481, 496, 512, 527, 543, 558, 573, 587, 602, 616, 630,
    644, 658, 672, 685, 698, 711, 724, 737, 749, 761, 773, 784, 796, 807, 818, 828, 839, 849, 859,
    868, 878, 887, 896, 904, 912, 920, 928, 935, 943, 949, 956, 962, 968, 974, 979, 984, 989, 994,
    998, 1002, 1005, 1008, 1011, 1014, 1016, 1018, 1020, 1022, 1023, 1023, 1024, 1024,
];

/// Refusals of the sense verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenseError {
    /// A view edge is zero or past [`MAX_VIEW`].
    BadSize,
    /// A pixel lies outside the view.
    OutOfImage,
    /// A map radius is outside `MAP_CELL_CM..=MAX_MAP_RADIUS_CM`.
    BadRadius,
    /// A glance grid edge is outside `1..=MAX_GLANCE_GRID`.
    BadGrid,
    /// A glance field of view is outside 2..=170 degrees.
    BadFov,
    /// A coverage pitch is outside -180..=180 degrees.
    BadPitch,
    /// The coverage subject has no box.
    NoSubject,
}

impl core::fmt::Display for SenseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSize => write!(f, "sense: view size outside 1..={MAX_VIEW}"),
            Self::OutOfImage => write!(f, "sense: pixel outside the view"),
            Self::BadRadius => {
                write!(
                    f,
                    "sense: radius outside {MAP_CELL_CM}..={MAX_MAP_RADIUS_CM} cm"
                )
            }
            Self::BadGrid => write!(f, "sense: glance grid outside 1..={MAX_GLANCE_GRID}"),
            Self::BadFov => write!(f, "sense: glance fov outside 2..=170 degrees"),
            Self::BadPitch => write!(f, "sense: pitch outside -180..=180 degrees"),
            Self::NoSubject => write!(f, "sense: the subject has no box in this scene"),
        }
    }
}

impl std::error::Error for SenseError {}

/// A level pinhole camera with a 90 degree horizontal field of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Camera {
    pub eye_cm: [i32; 3],
    pub yaw_deg: i32,
}

/// `sin(deg) * 1024`, exact table lookup for any whole degree.
pub(crate) fn sin_1k(deg: i32) -> i32 {
    let d = deg.rem_euclid(360);
    let idx = |v: i32| usize::try_from(v).unwrap_or(0);
    match d {
        0..=90 => SIN_1K[idx(d)],
        91..=180 => SIN_1K[idx(180 - d)],
        181..=270 => -SIN_1K[idx(d - 180)],
        _ => -SIN_1K[idx(360 - d)],
    }
}

/// `cos(deg) * 1024`, exact table lookup for any whole degree.
pub(crate) fn cos_1k(deg: i32) -> i32 {
    sin_1k(90 - deg)
}

/// Ray direction (any scale) through the center of pixel `(x, y)` of a
/// `w` by `h` view.
fn pixel_dir(cam: &Camera, w: i32, h: i32, x: i32, y: i32) -> [i32; 3] {
    let (s, c) = (sin_1k(cam.yaw_deg), cos_1k(cam.yaw_deg));
    let right = 2 * x + 1 - w;
    [s * w + c * right, 1024 * (h - 2 * y - 1), c * w - s * right]
}

fn check_size(w: i32, h: i32) -> Result<(), SenseError> {
    if (1..=MAX_VIEW).contains(&w) && (1..=MAX_VIEW).contains(&h) {
        Ok(())
    } else {
        Err(SenseError::BadSize)
    }
}

fn cast(world: &SightWorld, eye: [i32; 3], dir: [i32; 3]) -> Option<RayHit> {
    ray_first_hit(world, eye, dir, SENSE_RANGE_CM)
}

fn bucket(dist_cm: i32) -> i32 {
    (dist_cm / BUCKET_CM).min(7)
}

/// Run-length text picture of what the camera sees: one line per row, each
/// token `id@bucket*count` (a miss is `.*count`).
///
/// Errors: [`SenseError::BadSize`] for a view edge outside `1..=MAX_VIEW`.
pub fn view(world: &SightWorld, cam: &Camera, w: i32, h: i32) -> Result<String, SenseError> {
    check_size(w, h)?;
    let mut out = String::new();
    let [ex, ey, ez] = cam.eye_cm;
    let _ = writeln!(
        out,
        "view {w}x{h} eye=({ex},{ey},{ez}) yaw={} range={SENSE_RANGE_CM} bucket={BUCKET_CM}cm",
        cam.yaw_deg
    );
    for y in 0..h {
        let _ = write!(out, "{y}:");
        let mut run: Option<(Option<(u32, i32)>, usize)> = None;
        for x in 0..=w {
            let cell = (x < w)
                .then(|| cast(world, cam.eye_cm, pixel_dir(cam, w, h, x, y)))
                .map(|hit| hit.map(|r| (r.id, bucket(r.dist_cm))));
            run = match (run, cell) {
                (Some((tok, n)), Some(c)) if tok == c => Some((tok, n + 1)),
                (prev, c) => {
                    if let Some((tok, n)) = prev {
                        write_token(&mut out, tok, n);
                    }
                    c.map(|c| (c, 1))
                }
            };
        }
        out.push('\n');
    }
    Ok(out)
}

fn write_token(out: &mut String, tok: Option<(u32, i32)>, n: usize) {
    match tok {
        Some((id, b)) => {
            let _ = write!(out, " {id}@{b}*{n}");
        }
        None => {
            let _ = write!(out, " .*{n}");
        }
    }
}

/// 72 horizontal rays from `origin_cm`, 5 degrees apart starting at `fan_deg`:
/// one `deg=dist@id` token each, `deg=-` for a miss.
pub fn lidar(world: &SightWorld, origin_cm: [i32; 3], fan_deg: i32) -> String {
    let [ox, oy, oz] = origin_cm;
    let mut out = format!(
        "lidar origin=({ox},{oy},{oz}) fan={fan_deg} step={LIDAR_STEP_DEG} range={SENSE_RANGE_CM}\n"
    );
    for i in 0..LIDAR_RAYS {
        let deg = fan_deg + LIDAR_STEP_DEG * i;
        let dir = [sin_1k(deg), 0, cos_1k(deg)];
        match cast(world, origin_cm, dir) {
            Some(r) => {
                let _ = write!(out, "{deg}={}@{} ", r.dist_cm, r.id);
            }
            None => {
                let _ = write!(out, "{deg}=- ");
            }
        }
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

/// One ray through pixel `(x, y)` of a `w` by `h` view as a single text line:
/// entity id, 3D point in cm and distance, or `miss`.
///
/// Errors: [`SenseError::BadSize`], [`SenseError::OutOfImage`].
pub fn pixel(
    world: &SightWorld,
    cam: &Camera,
    w: i32,
    h: i32,
    x: i32,
    y: i32,
) -> Result<String, SenseError> {
    check_size(w, h)?;
    if !(0..w).contains(&x) || !(0..h).contains(&y) {
        return Err(SenseError::OutOfImage);
    }
    Ok(match cast(world, cam.eye_cm, pixel_dir(cam, w, h, x, y)) {
        Some(r) => format!(
            "pixel {x},{y} id={} point=({},{},{}) dist={}cm",
            r.id, r.point_cm[0], r.point_cm[1], r.point_cm[2], r.dist_cm
        ),
        None => format!("pixel {x},{y} miss"),
    })
}

/// Top-down grid of [`MAP_CELL_CM`] cells around `center_cm`, row 0 at the
/// largest z, column 0 at the smallest x. `.` is a walkable cell (nothing solid
/// between a curb above the center height and standing head height), a letter
/// is the tallest solid blocking that cell, listed in the legend with its id
/// and top height. Solids that contain the center never block (the room).
///
/// Errors: [`SenseError::BadRadius`].
pub fn map(world: &SightWorld, center_cm: [i32; 3], radius_cm: i32) -> Result<String, SenseError> {
    if !(MAP_CELL_CM..=MAX_MAP_RADIUS_CM).contains(&radius_cm) {
        return Err(SenseError::BadRadius);
    }
    let n = 2 * radius_cm / MAP_CELL_CM;
    let cap = usize::try_from(n).unwrap_or(0);
    let [cx, cy, cz] = center_cm;
    let (lo_y, hi_y) = (cy + STEP_CM, cy + STAND_TOP_CM);
    let blockers = world.blockers_over(center_cm, lo_y, hi_y);
    let mut legend: BTreeMap<u32, (char, i32)> = BTreeMap::new();
    let mut rows = Vec::with_capacity(cap);
    for r in 0..n {
        let z = cz + radius_cm - (2 * r + 1) * MAP_CELL_CM / 2;
        let mut row = String::with_capacity(cap);
        for c in 0..n {
            let x = cx - radius_cm + (2 * c + 1) * MAP_CELL_CM / 2;
            let top = blockers
                .iter()
                .filter(|b| b.min_x < x && x < b.max_x && b.min_z < z && z < b.max_z)
                .max_by_key(|b| (b.max_y, std::cmp::Reverse(b.id)));
            row.push(match top {
                None => '.',
                Some(b) => {
                    let next = legend_glyph(legend.len());
                    legend.entry(b.id).or_insert((next, b.max_y)).0
                }
            });
        }
        rows.push(row);
    }
    let mut out = format!(
        "map center=({cx},{cy},{cz}) radius={radius_cm} cell={MAP_CELL_CM}cm rows=+z..-z cols=-x..+x\n"
    );
    for row in rows {
        out.push_str(&row);
        out.push('\n');
    }
    for (id, (glyph, top)) in &legend {
        let _ = writeln!(out, "{glyph}={id} top={top}cm");
    }
    Ok(out)
}

/// The sense camera's yaw (whole degrees, 0 faces +z) for a door yaw in
/// radians (0 faces +x, a quarter turn faces +z).
pub fn camera_yaw_deg(door_yaw_rad: f32) -> i32 {
    (90 - whole_deg(door_yaw_rad)).rem_euclid(360)
}

/// Largest glance grid edge in rays.
pub const MAX_GLANCE_GRID: i32 = 64;
/// Grid edge a glance uses when the caller names none.
pub const DEFAULT_GLANCE_GRID: i32 = 24;
/// Objects a glance lists; the rest fold into `more_pm`.
pub const GLANCE_VISIBLE_CAP: usize = 25;

/// One object a glance saw, with its share of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub id: u32,
    /// Share of the grid whose first hit is this object, permille.
    pub coverage_pm: i32,
    /// Nearest and farthest hit distance in cm.
    pub near_cm: i32,
    pub far_cm: i32,
}

/// What one camera sees, as numbers (SPX1). `coverage_pm` of every object
/// plus `more_pm` plus `background_pm` is exactly 1000.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Glance {
    pub grid: i32,
    /// Largest share first, ties to the lower id, at most [`GLANCE_VISIBLE_CAP`].
    pub visible: Vec<Seen>,
    /// Share of the objects that did not fit the cap, permille.
    pub more_pm: i32,
    /// In the frustum but behind another solid, id ascending.
    pub occluded: Vec<u32>,
    /// Share of the grid that hit nothing within range, permille.
    pub background_pm: i32,
    /// Smallest, largest and mean hit distance in cm; `None` when nothing was hit.
    pub depth_cm: Option<[i32; 3]>,
}

/// Ray direction through cell `(x, y)` of a `g` by `g` grid for a camera with
/// a horizontal and vertical field of view of `fov_deg` (halved to whole
/// degrees). Integer table math, any scale.
fn glance_dir(cam: &Camera, fov_deg: i32, g: i32, x: i32, y: i32) -> [i32; 3] {
    let (s, c) = (sin_1k(cam.yaw_deg), cos_1k(cam.yaw_deg));
    let (sn, cs) = (sin_1k(fov_deg / 2), cos_1k(fov_deg / 2));
    let (right, up) = (2 * x + 1 - g, g - 2 * y - 1);
    [
        s * g * cs + c * right * sn,
        1024 * up * sn,
        c * g * cs - s * right * sn,
    ]
}

/// Share each count of `total` rays gets in permille, summing to exactly 1000:
/// floors first, the leftover permille to the largest remainders (ties to the
/// earlier entry).
fn permille(counts: &[i64], total: i64) -> Vec<i32> {
    let mut shares: Vec<i64> = counts.iter().map(|n| n * 1000 / total).collect();
    let mut order: Vec<usize> = (0..counts.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(counts[i] * 1000 % total), i));
    let left = 1000 - shares.iter().sum::<i64>();
    for &i in order.iter().take(usize::try_from(left).unwrap_or(0)) {
        shares[i] += 1;
    }
    shares
        .into_iter()
        .map(|v| i32::try_from(v).unwrap_or(0))
        .collect()
}

/// Cast a `grid` by `grid` fan of rays through `cam` and tally what each hits:
/// per-object coverage, the objects the view holds but a solid hides, the
/// background share and the depth range. `fov_deg` is the full angle.
///
/// Errors: [`SenseError::BadGrid`] outside `1..=MAX_GLANCE_GRID`,
/// [`SenseError::BadFov`] outside `2..=170`.
pub fn glance(
    world: &SightWorld,
    cam: &Camera,
    fov_deg: i32,
    grid: i32,
) -> Result<Glance, SenseError> {
    if !(1..=MAX_GLANCE_GRID).contains(&grid) {
        return Err(SenseError::BadGrid);
    }
    if !(2..=170).contains(&fov_deg) {
        return Err(SenseError::BadFov);
    }
    // Per object: rays, nearest, farthest.
    let mut seen: BTreeMap<u32, (i64, i32, i32)> = BTreeMap::new();
    let (mut misses, mut depth_sum, mut hits) = (0i64, 0i64, 0i64);
    for y in 0..grid {
        for x in 0..grid {
            match cast(world, cam.eye_cm, glance_dir(cam, fov_deg, grid, x, y)) {
                None => misses += 1,
                Some(h) => {
                    let e = seen.entry(h.id).or_insert((0, h.dist_cm, h.dist_cm));
                    *e = (e.0 + 1, e.1.min(h.dist_cm), e.2.max(h.dist_cm));
                    depth_sum += i64::from(h.dist_cm);
                    hits += 1;
                }
            }
        }
    }
    let mut counts: Vec<i64> = seen.values().map(|e| e.0).collect();
    counts.push(misses);
    let shares = permille(&counts, i64::from(grid) * i64::from(grid));
    let mut rows: Vec<Seen> = seen
        .iter()
        .zip(&shares)
        .map(|((id, e), pm)| Seen {
            id: *id,
            coverage_pm: *pm,
            near_cm: e.1,
            far_cm: e.2,
        })
        .collect();
    let background_pm = shares[shares.len() - 1];
    // Objects the grid missed: a center ray tells hidden from merely small.
    let (mut occluded, mut tiny) = (Vec::new(), Vec::new());
    for b in world.solids() {
        let dir = [
            (b.min_x + b.max_x) / 2 - cam.eye_cm[0],
            (b.min_y + b.max_y) / 2 - cam.eye_cm[1],
            (b.min_z + b.max_z) / 2 - cam.eye_cm[2],
        ];
        if seen.contains_key(&b.id) || !in_frustum(cam, fov_deg, dir) {
            continue;
        }
        match cast(world, cam.eye_cm, dir) {
            Some(h) if h.id == b.id => tiny.push(Seen {
                id: b.id,
                coverage_pm: 0,
                near_cm: h.dist_cm,
                far_cm: h.dist_cm,
            }),
            Some(_) => occluded.push(b.id),
            None => {}
        }
    }
    rows.extend(tiny);
    rows.sort_by_key(|r| (std::cmp::Reverse(r.coverage_pm), r.id));
    let more_pm = rows
        .iter()
        .skip(GLANCE_VISIBLE_CAP)
        .map(|r| r.coverage_pm)
        .sum();
    rows.truncate(GLANCE_VISIBLE_CAP);
    Ok(Glance {
        grid,
        visible: rows,
        more_pm,
        occluded,
        background_pm,
        depth_cm: (hits > 0).then(|| {
            let dists = seen.values();
            let near = dists.clone().map(|e| e.1).min().unwrap_or(0);
            let far = dists.map(|e| e.2).max().unwrap_or(0);
            [
                near,
                far,
                i32::try_from(depth_sum / hits).unwrap_or(i32::MAX),
            ]
        }),
    })
}

/// True when the offset `d` (cm) from the eye lies inside the view cone and
/// within range: ahead of the camera and inside the tangent of the half angle
/// on both axes, the way [`glance_dir`] spreads its rays.
fn in_frustum(cam: &Camera, fov_deg: i32, d: [i32; 3]) -> bool {
    let (s, c) = (
        i64::from(sin_1k(cam.yaw_deg)),
        i64::from(cos_1k(cam.yaw_deg)),
    );
    let (sn, cs) = (
        i64::from(sin_1k(fov_deg / 2)),
        i64::from(cos_1k(fov_deg / 2)),
    );
    let d = d.map(i64::from);
    let ahead = d[0] * s + d[2] * c;
    let side = d[0] * c - d[2] * s;
    let up = d[1] * 1024;
    let range = i64::from(SENSE_RANGE_CM);
    ahead > 0
        && side.abs() * cs <= ahead * sn
        && up.abs() * cs <= ahead * sn
        && d.iter().map(|v| v * v).sum::<i64>() <= range * range
}

/// A glance as plain text: one row per visible object, then what is hidden,
/// the background, the depth range and the token cost of the text itself.
pub fn glance_text(g: &Glance, cam: &Camera, fov_deg: i32) -> String {
    let [ex, ey, ez] = cam.eye_cm;
    let mut out = String::new();
    let (grid, yaw) = (g.grid, cam.yaw_deg);
    let _ = writeln!(
        out,
        "glance {grid}x{grid} eye=({ex},{ey},{ez}) yaw={yaw} fov={fov_deg} range={SENSE_RANGE_CM}"
    );
    for s in &g.visible {
        let _ = writeln!(
            out,
            "visible {} {}pm near={}cm far={}cm",
            s.id, s.coverage_pm, s.near_cm, s.far_cm
        );
    }
    if g.more_pm > 0 {
        let _ = writeln!(out, "more {}pm", g.more_pm);
    }
    if !g.occluded.is_empty() {
        let ids: Vec<String> = g.occluded.iter().map(u32::to_string).collect();
        let _ = writeln!(out, "occluded {}", ids.join(","));
    }
    let _ = writeln!(out, "background {}pm", g.background_pm);
    match g.depth_cm {
        Some([near, far, mean]) => {
            let _ = writeln!(out, "depth min={near} max={far} mean={mean}cm");
        }
        None => out.push_str("depth none\n"),
    }
    let tokens = crate::estimate_tokens(&out);
    let _ = writeln!(out, "tokens {tokens}");
    out
}

/// Coverage grid edge: 32 x 32 rays.
pub const COVERAGE_GRID: i32 = 32;
/// Coverage under this many permille earns a next-call line.
pub const HIDDEN_PERMILLE: i32 = 5;

/// What a camera pose sees of one subject (SPF11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coverage {
    /// Share of the grid whose first hit is the subject, permille.
    pub permille: i32,
    /// The camera looks away from the subject.
    pub looks_away: bool,
    /// The camera is upside down: its pitch is past a quarter turn.
    pub flipped: bool,
    /// A next step when the share is under [`HIDDEN_PERMILLE`], else empty.
    pub next: String,
}

/// Forward, right and up of a camera in 1024 x 1024 units: yaw as in
/// [`Camera`] (0 faces +z), pitch positive looking up. Pitch past 90 degrees
/// keeps turning over the top, so `up` points down (a flipped camera).
fn aim(yaw_deg: i32, pitch_deg: i32) -> ([i64; 3], [i64; 3], [i64; 3]) {
    let (s, c) = (i64::from(sin_1k(yaw_deg)), i64::from(cos_1k(yaw_deg)));
    let (sp, cp) = (i64::from(sin_1k(pitch_deg)), i64::from(cos_1k(pitch_deg)));
    (
        [s * cp, 1024 * sp, c * cp],
        [c * 1024, 0, -s * 1024],
        [-s * sp, 1024 * cp, -c * sp],
    )
}

/// How much of the frame `subject` fills from `cam` tipped by `pitch_deg`
/// (positive looks up, -180..=180): the share of a 32 x 32 ray grid whose first
/// hit is the subject, with every other solid occluding it. An eye inside the
/// subject reads 1000; a camera that looks away from it reads 0. Rays reach
/// [`SENSE_RANGE_CM`].
///
/// Errors: [`SenseError::BadFov`], [`SenseError::BadPitch`], and
/// [`SenseError::NoSubject`] for an id that has no box.
pub fn coverage(
    world: &SightWorld,
    cam: &Camera,
    pitch_deg: i32,
    fov_deg: i32,
    subject: u32,
) -> Result<Coverage, SenseError> {
    if !(2..=170).contains(&fov_deg) {
        return Err(SenseError::BadFov);
    }
    if !(-180..=180).contains(&pitch_deg) {
        return Err(SenseError::BadPitch);
    }
    let target = world.box_of(subject).ok_or(SenseError::NoSubject)?;
    let (forward, right, up) = aim(cam.yaw_deg, pitch_deg);
    let flipped = up[1] < 0;
    let inside = [
        (target.min_x, target.max_x),
        (target.min_y, target.max_y),
        (target.min_z, target.max_z),
    ]
    .iter()
    .zip(cam.eye_cm)
    .all(|((lo, hi), e)| *lo <= e && e <= *hi);
    let to_target = [
        (target.min_x + target.max_x) / 2 - cam.eye_cm[0],
        (target.min_y + target.max_y) / 2 - cam.eye_cm[1],
        (target.min_z + target.max_z) / 2 - cam.eye_cm[2],
    ];
    let ahead: i64 = (0..3).map(|k| forward[k] * i64::from(to_target[k])).sum();
    let looks_away = !inside && ahead <= 0;
    let mut hits = 0;
    if inside {
        hits = COVERAGE_GRID * COVERAGE_GRID;
    } else if !looks_away {
        let (sn, cs) = (
            i64::from(sin_1k(fov_deg / 2)),
            i64::from(cos_1k(fov_deg / 2)),
        );
        let g = i64::from(COVERAGE_GRID);
        for y in 0..COVERAGE_GRID {
            for x in 0..COVERAGE_GRID {
                let (u, v) = (
                    i64::from(2 * x + 1 - COVERAGE_GRID),
                    i64::from(COVERAGE_GRID - 2 * y - 1),
                );
                let dir: [i32; 3] = std::array::from_fn(|k| {
                    let along = forward[k] * g * cs + (right[k] * u + up[k] * v) * sn;
                    i32::try_from(along / 1024).unwrap_or(0)
                });
                hits += i32::from(cast(world, cam.eye_cm, dir).is_some_and(|h| h.id == subject));
            }
        }
    }
    let permille = hits * 1000 / (COVERAGE_GRID * COVERAGE_GRID);
    let next = if permille < HIDDEN_PERMILLE {
        "Next call: scene.query kind=why_hidden with the same anchor and eye_m to see what hides it"
            .to_owned()
    } else {
        String::new()
    };
    Ok(Coverage {
        permille,
        looks_away,
        flipped,
        next,
    })
}

/// Whole degrees of an angle in radians (0.5 degree rounds away from zero).
pub fn whole_deg(rad: f32) -> i32 {
    // `to_cm` rounds a value times 100, so a hundredth of the degrees comes back whole.
    crate::to_cm(rad.to_degrees() / 100.0)
}

/// Legend letters in id-first-seen order: `A..Z`, `a..z`, then `#`.
fn legend_glyph(index: usize) -> char {
    match u8::try_from(index) {
        Ok(i) if i < 26 => char::from(b'A' + i),
        Ok(i) if i < 52 => char::from(b'a' + i - 26),
        _ => '#',
    }
}

#[cfg(test)]
mod sense_tests {
    use super::*;
    use crate::Actor;

    fn lcg(state: &mut u64) -> i32 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*state >> 33) % 1_000_000) as i32
    }

    /// Seeded room: 14 random crates on a 10 cm lattice in a 16 m square.
    fn scene(seed: u64) -> Vec<Actor> {
        let mut s = seed.wrapping_add(0xA5A5);
        (0..14u32)
            .map(|id| {
                let mut pick = |lo: i32, span: i32| (lo + lcg(&mut s) % span * 10) as f32 / 100.0;
                Actor {
                    id: 10 + id,
                    name: "crate".into(),
                    x: pick(-800, 160),
                    y: pick(50, 20),
                    z: pick(-800, 160),
                    sx: pick(50, 30),
                    sy: pick(50, 30),
                    sz: pick(50, 30),
                    parent: None,
                    physical: true,
                    vx: 0.0,
                    vz: 0.0,
                    yaw: 0.0,
                }
            })
            .collect()
    }

    fn camera(seed: u64) -> Camera {
        let mut s = seed.wrapping_mul(7919).wrapping_add(3);
        Camera {
            eye_cm: [
                lcg(&mut s) % 1600 - 800,
                100 + lcg(&mut s) % 100,
                lcg(&mut s) % 1600 - 800,
            ],
            yaw_deg: lcg(&mut s) % 360,
        }
    }

    /// Test boxes in cm exactly as the world builds them: (id, min, max).
    fn cm_boxes(actors: &[Actor]) -> Vec<(u32, [i64; 3], [i64; 3])> {
        let cm = |v: f32| i64::from((v * 100.0).round() as i32);
        actors
            .iter()
            .map(|a| {
                let c = [cm(a.x), cm(a.y), cm(a.z)];
                let h = [cm(a.sx).abs() / 2, cm(a.sy).abs() / 2, cm(a.sz).abs() / 2];
                (
                    a.id,
                    [c[0] - h[0], c[1] - h[1], c[2] - h[2]],
                    [c[0] + h[0], c[1] + h[1], c[2] + h[2]],
                )
            })
            .collect()
    }

    /// Independent check: cross the six face planes of every box with the
    /// ray in exact rationals and keep the nearest face whose crossing point
    /// lies inside the other two spans (a boundary counts only when the ray
    /// moves inward there). Returns the id and the distance in cm.
    fn brute(
        boxes: &[(u32, [i64; 3], [i64; 3])],
        eye: [i32; 3],
        dir: [i32; 3],
    ) -> Option<(u32, f64)> {
        let e = eye.map(i64::from);
        let d = dir.map(i64::from);
        let len = (d.iter().map(|v| v * v).sum::<i64>() as f64).sqrt();
        let mut best: Option<(u32, i64, i64)> = None;
        for (id, mn, mx) in boxes {
            if (0..3).all(|k| mn[k] <= e[k] && e[k] <= mx[k]) {
                continue;
            }
            for k in 0..3 {
                if d[k] == 0 {
                    continue;
                }
                let plane = if d[k] > 0 { mn[k] } else { mx[k] };
                let (n, den) = if d[k] > 0 {
                    (plane - e[k], d[k])
                } else {
                    (e[k] - plane, -d[k])
                };
                if n <= 0 {
                    continue;
                }
                let inside = (0..3).filter(|j| *j != k).all(|j| {
                    let p = i128::from(e[j]) * i128::from(den) + i128::from(d[j]) * i128::from(n);
                    let (lo, hi) = (
                        i128::from(mn[j]) * i128::from(den),
                        i128::from(mx[j]) * i128::from(den),
                    );
                    (lo < p && p < hi) || (p == lo && d[j] > 0) || (p == hi && d[j] < 0)
                });
                if inside && best.is_none_or(|(_, bn, bd)| n * bd < bn * den) {
                    best = Some((*id, n, den));
                }
            }
        }
        best.map(|(id, n, den)| (id, len * n as f64 / den as f64))
            .filter(|(_, dist)| *dist <= f64::from(SENSE_RANGE_CM))
    }

    /// Blocked cells of a map by direct plan scan, for the row-count check.
    fn blocked_cells(boxes: &[(u32, [i64; 3], [i64; 3])], center: [i32; 3], radius: i32) -> usize {
        let c = center.map(i64::from);
        let live: Vec<_> = boxes
            .iter()
            .filter(|(_, mn, mx)| !(0..3).all(|k| mn[k] <= c[k] && c[k] <= mx[k]))
            .filter(|(_, mn, mx)| mn[1] < c[1] + 170 && mx[1] > c[1] + 30)
            .collect();
        let n = i64::from(2 * radius / MAP_CELL_CM);
        let cell = i64::from(MAP_CELL_CM);
        let mut count = 0;
        for r in 0..n {
            for q in 0..n {
                let z = c[2] + i64::from(radius) - (2 * r + 1) * cell / 2;
                let x = c[0] - i64::from(radius) + (2 * q + 1) * cell / 2;
                count += usize::from(
                    live.iter()
                        .any(|(_, mn, mx)| mn[0] < x && x < mx[0] && mn[2] < z && z < mx[2]),
                );
            }
        }
        count
    }

    /// Fast cast against the brute force, allowing 1 cm of rounding and the
    /// range edge to fall either way.
    fn agrees(fast: Option<RayHit>, slow: Option<(u32, f64)>) -> bool {
        let edge = f64::from(SENSE_RANGE_CM) - 1.0;
        match (fast, slow) {
            (None, None) => true,
            (Some(f), Some((id, dist))) => f.id == id && (f64::from(f.dist_cm) - dist).abs() <= 1.0,
            (Some(f), None) => f64::from(f.dist_cm) >= edge,
            (None, Some((_, dist))) => dist >= edge,
        }
    }

    #[test]
    fn sense_view_lidar_map_match_brute_force_50_of_50() {
        let (mut ok, mut rays) = (0, 0u32);
        for seed in 0..50u64 {
            let actors = scene(seed);
            let boxes = cm_boxes(&actors);
            let world = SightWorld::build(&actors);
            let cam = camera(seed);
            let (w, h) = (24, 16);
            let mut dirs: Vec<[i32; 3]> = (0..w * h)
                .map(|i| pixel_dir(&cam, w, h, i % w, i / w))
                .collect();
            for i in 0..LIDAR_RAYS {
                let deg = cam.yaw_deg + LIDAR_STEP_DEG * i;
                dirs.push([sin_1k(deg), 0, cos_1k(deg)]);
            }
            let rays_ok = dirs.iter().all(|dir| {
                agrees(
                    cast(&world, cam.eye_cm, *dir),
                    brute(&boxes, cam.eye_cm, *dir),
                )
            });
            rays += dirs.len() as u32;
            let text = map(&world, cam.eye_cm, 600).expect("legal radius");
            let drawn = text
                .lines()
                .skip(1)
                .take(24)
                .flat_map(str::chars)
                .filter(|c| *c != '.')
                .count();
            ok += u32::from(rays_ok && drawn == blocked_cells(&boxes, cam.eye_cm, 600));
        }
        println!("sense: {ok}/50 cases, {rays} rays checked against brute force");
        assert_eq!(ok, 50);
    }
    #[test]
    fn sense_text_is_stable_and_shaped() {
        let actors = scene(3);
        let world = SightWorld::build(&actors);
        let cam = camera(3);
        let a = view(&world, &cam, 32, 12).expect("legal size");
        assert_eq!(a, view(&world, &cam, 32, 12).expect("legal size"));
        assert_eq!(a.lines().count(), 1 + 12);
        assert!(a.lines().nth(1).is_some_and(|l| l.starts_with("0:")));
        let l = lidar(&world, cam.eye_cm, 0);
        assert_eq!(l.lines().nth(1).map(|r| r.split(' ').count()), Some(72));
        assert_eq!(l, lidar(&world, cam.eye_cm, 0));
        let m = map(&world, [0, 0, 0], 800).expect("legal radius");
        assert_eq!(m, map(&world, [0, 0, 0], 800).expect("legal radius"));
        println!(
            "sense: view 32x12 = {} tokens, lidar = {} tokens, map 800 = {} tokens",
            crate::estimate_tokens(&a),
            crate::estimate_tokens(&l),
            crate::estimate_tokens(&m)
        );
    }

    #[test]
    fn sense_pixel_names_the_hit_and_refuses_bad_input() {
        let crate_box = Actor {
            id: 7,
            name: "wall".into(),
            x: 0.0,
            y: 1.0,
            z: 5.0,
            sx: 4.0,
            sy: 2.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        };
        let world = SightWorld::build(&[crate_box]);
        let cam = Camera {
            eye_cm: [0, 100, 0],
            yaw_deg: 0,
        };
        // Center pixel of an odd view looks straight at the near face z=450.
        assert_eq!(
            pixel(&world, &cam, 3, 3, 1, 1).as_deref(),
            Ok("pixel 1,1 id=7 point=(0,100,450) dist=450cm")
        );
        assert_eq!(pixel(&world, &cam, 3, 3, 3, 0), Err(SenseError::OutOfImage));
        assert_eq!(pixel(&world, &cam, 0, 3, 0, 0), Err(SenseError::BadSize));
        assert_eq!(view(&world, &cam, 129, 1), Err(SenseError::BadSize));
        assert_eq!(map(&world, [0, 0, 0], 10), Err(SenseError::BadRadius));
        let behind = Camera {
            yaw_deg: 180,
            ..cam
        };
        assert_eq!(
            pixel(&world, &behind, 3, 3, 1, 1).as_deref(),
            Ok("pixel 1,1 miss")
        );
        assert_eq!(sin_1k(0), 0);
        assert_eq!(sin_1k(90), 1024);
        assert_eq!(cos_1k(0), 1024);
        assert_eq!(sin_1k(270), -1024);
        assert_eq!(sin_1k(-90), -1024);
    }

    #[test]
    fn sense_map_marks_blockers_and_walkable_cells() {
        let wall = Actor {
            id: 7,
            name: "wall".into(),
            x: 2.0,
            y: 1.0,
            z: 0.0,
            sx: 1.0,
            sy: 2.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        };
        let world = SightWorld::build(&[wall]);
        let text = map(&world, [0, 0, 0], 300).expect("legal radius");
        let rows: Vec<&str> = text.lines().skip(1).take(12).collect();
        // x 150..250 cm is columns 9..=10 of 12; z -50..50 is rows 5 and 6.
        assert_eq!(rows[5], ".........AA.");
        assert_eq!(rows[6], ".........AA.");
        assert_eq!(rows[0], "............");
        assert!(text.ends_with("A=7 top=200cm\n"));
    }

    #[test]
    #[ignore = "timing: ray budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn sense_ray_budget_under_a_fifth_of_a_ms() {
        let actors = scene(9);
        let world = SightWorld::build(&actors);
        let cam = camera(9);
        let start = std::time::Instant::now();
        let text = view(&world, &cam, 128, 96).expect("legal size");
        let per_ray_us = start.elapsed().as_secs_f64() / (128.0 * 96.0) * 1e6;
        println!(
            "sense: view 128x96 {per_ray_us:.3} us per ray, {} tokens",
            crate::estimate_tokens(&text)
        );
        assert!(
            per_ray_us <= 200.0,
            "ray budget 0.2 ms, got {per_ray_us:.3} us"
        );
    }

    /// A 2 m crate 6 m ahead of the origin and, optionally, a tall thin pillar
    /// 3 m ahead that hides exactly the right half of it from the eye at x = 0.
    fn coverage_scene(pillar: bool) -> SightWorld {
        let mut actors = vec![crate_at(1, 0.0, 1.0, 6.0, [2.0, 2.0, 1.0])];
        if pillar {
            actors.push(crate_at(2, 0.3, 1.0, 3.0, [0.6, 3.0, 0.2]));
        }
        SightWorld::build(&actors)
    }

    fn eye(yaw_deg: i32) -> Camera {
        Camera {
            eye_cm: [0, 100, 0],
            yaw_deg,
        }
    }

    #[test]
    fn coverage_inside_behind_flipped_and_stable() {
        let world = coverage_scene(false);
        // Facing the crate: it fills part of the frame.
        let ahead = coverage(&world, &eye(0), 0, 30, 1).expect("legal camera");
        assert!(
            ahead.permille > 100 && !ahead.looks_away && !ahead.flipped,
            "{ahead:?}"
        );
        assert!(ahead.next.is_empty());
        // Inside the crate reads the whole frame.
        let inside = Camera {
            eye_cm: [0, 100, 600],
            yaw_deg: 0,
        };
        assert_eq!(coverage(&world, &inside, 0, 30, 1).unwrap().permille, 1000);
        // Turned around it reads 0, looks away and names the next call.
        let behind = coverage(&world, &eye(180), 0, 30, 1).unwrap();
        assert_eq!((behind.permille, behind.looks_away), (0, true));
        assert!(
            behind
                .next
                .starts_with("Next call: scene.query kind=why_hidden"),
            "{}",
            behind.next
        );
        // Pitched over the top the camera is upside down; a quarter turn is not.
        assert!(coverage(&world, &eye(0), 150, 30, 1).unwrap().flipped);
        assert!(!coverage(&world, &eye(0), 90, 30, 1).unwrap().flipped);
        // Same bytes twice; bad input refuses.
        assert_eq!(ahead, coverage(&world, &eye(0), 0, 30, 1).unwrap());
        assert_eq!(
            coverage(&world, &eye(0), 181, 30, 1),
            Err(SenseError::BadPitch)
        );
        assert_eq!(coverage(&world, &eye(0), 0, 1, 1), Err(SenseError::BadFov));
        assert_eq!(
            coverage(&world, &eye(0), 0, 30, 9),
            Err(SenseError::NoSubject)
        );
    }

    #[test]
    fn coverage_pillar_over_half_the_view_halves_the_share() {
        let open = coverage(&coverage_scene(false), &eye(0), 0, 30, 1)
            .unwrap()
            .permille;
        let blocked = coverage(&coverage_scene(true), &eye(0), 0, 30, 1)
            .unwrap()
            .permille;
        assert!(open > 100, "{open}");
        assert!(
            (2 * blocked - open).abs() <= open / 10,
            "open {open} blocked {blocked}"
        );
    }

    fn crate_at(id: u32, x: f32, y: f32, z: f32, size: [f32; 3]) -> Actor {
        Actor {
            id,
            name: "crate".into(),
            x,
            y,
            z,
            sx: size[0],
            sy: size[1],
            sz: size[2],
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    #[test]
    fn glance_matches_a_brute_force_first_hit_count_on_50_seeded_cameras() {
        let (grid, fov) = (DEFAULT_GLANCE_GRID, 90);
        let mut listed = 0;
        for seed in 0..50u64 {
            let actors = scene(seed);
            let world = SightWorld::build(&actors);
            let cam = camera(seed);
            let boxes = cm_boxes(&actors);
            // The independent tally: every ray against the six face planes.
            let mut by_id: BTreeMap<u32, i64> = BTreeMap::new();
            let mut misses = 0i64;
            for y in 0..grid {
                for x in 0..grid {
                    let dir = glance_dir(&cam, fov, grid, x, y);
                    let (fast, slow) = (
                        cast(&world, cam.eye_cm, dir),
                        brute(&boxes, cam.eye_cm, dir),
                    );
                    assert!(agrees(fast, slow), "seed {seed} ({x},{y})");
                    // A ray at the range edge may fall either way: take the fast answer.
                    let id = if fast.is_some() == slow.is_some() {
                        slow.map(|(id, _)| id)
                    } else {
                        fast.map(|h| h.id)
                    };
                    match id {
                        Some(id) => *by_id.entry(id).or_insert(0) += 1,
                        None => misses += 1,
                    }
                }
            }
            let mut counts: Vec<i64> = by_id.values().copied().collect();
            counts.push(misses);
            let want = permille(&counts, i64::from(grid * grid));
            let got = glance(&world, &cam, fov, grid).expect("legal glance");
            for (row, share) in by_id.keys().zip(&want) {
                let seen = got.visible.iter().find(|s| s.id == *row);
                assert_eq!(
                    seen.map(|s| s.coverage_pm),
                    Some(*share),
                    "seed {seed} id {row}"
                );
                listed += 1;
            }
            assert_eq!(got.background_pm, want[want.len() - 1], "seed {seed}");
            let total: i32 = got.visible.iter().map(|s| s.coverage_pm).sum::<i32>()
                + got.more_pm
                + got.background_pm;
            assert_eq!(total, 1000, "seed {seed}: coverage plus background is 1000");
            assert_eq!(
                got,
                glance(&world, &cam, fov, grid).unwrap(),
                "seed {seed} x2"
            );
        }
        assert!(listed > 100, "{listed} objects compared");
    }

    #[test]
    fn glance_puts_a_crate_behind_a_pillar_in_occluded_not_visible() {
        let actors = [
            crate_at(1, 0.0, 1.0, 3.0, [1.0, 2.0, 1.0]),
            crate_at(2, 0.0, 1.0, 6.0, [1.0, 1.0, 1.0]),
            crate_at(3, 4.0, 1.0, 6.0, [1.0, 1.0, 1.0]),
        ];
        let world = SightWorld::build(&actors);
        let cam = Camera {
            eye_cm: [0, 100, 0],
            yaw_deg: 0,
        };
        let g = glance(&world, &cam, 90, DEFAULT_GLANCE_GRID).expect("legal glance");
        let ids: Vec<u32> = g.visible.iter().map(|s| s.id).collect();
        assert!(ids.contains(&1) && ids.contains(&3), "{ids:?}");
        assert!(
            !ids.contains(&2),
            "the hidden crate is not visible: {ids:?}"
        );
        assert_eq!(g.occluded, vec![2]);
        assert!(g.background_pm > 0 && g.depth_cm.is_some());
        let text = glance_text(&g, &cam, 90);
        assert!(
            text.contains("occluded 2\n") && text.contains("\ntokens "),
            "{text}"
        );
        // Bad shapes refuse.
        assert_eq!(glance(&world, &cam, 90, 0), Err(SenseError::BadGrid));
        assert_eq!(glance(&world, &cam, 90, 65), Err(SenseError::BadGrid));
        assert_eq!(glance(&world, &cam, 1, 24), Err(SenseError::BadFov));
        assert_eq!(glance(&world, &cam, 171, 24), Err(SenseError::BadFov));
        // Door yaw 0 faces +x; the camera faces 0 at +z.
        let quarter = std::f32::consts::FRAC_PI_2;
        assert_eq!(
            [
                camera_yaw_deg(0.0),
                camera_yaw_deg(quarter),
                camera_yaw_deg(-quarter)
            ],
            [90, 0, 180]
        );
        // An empty room is all background.
        let empty = glance(&SightWorld::build(&[]), &cam, 90, 8).unwrap();
        assert_eq!((empty.background_pm, empty.depth_cm), (1000, None));
    }

    #[test]
    #[ignore = "timing: ray budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn glance_ray_budget_under_a_fifth_of_a_ms() {
        let actors = scene(9);
        let world = SightWorld::build(&actors);
        let cam = camera(9);
        let start = std::time::Instant::now();
        let g = glance(&world, &cam, 90, MAX_GLANCE_GRID).expect("legal glance");
        let per_ray_us = start.elapsed().as_secs_f64() / f64::from(MAX_GLANCE_GRID.pow(2)) * 1e6;
        println!(
            "sense: glance {MAX_GLANCE_GRID}x{MAX_GLANCE_GRID} {per_ray_us:.3} us per ray, {} tokens",
            crate::estimate_tokens(&glance_text(&g, &cam, 90))
        );
        assert!(
            per_ray_us <= 200.0,
            "ray budget 0.2 ms, got {per_ray_us:.3} us"
        );
    }
}

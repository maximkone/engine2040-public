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
fn sin_1k(deg: i32) -> i32 {
    let d = deg.rem_euclid(360);
    let idx = |v: i32| usize::try_from(v).unwrap_or(0);
    match d {
        0..=90 => SIN_1K[idx(d)],
        91..=180 => SIN_1K[idx(180 - d)],
        181..=270 => -SIN_1K[idx(d - 180)],
        _ => -SIN_1K[idx(360 - d)],
    }
}

fn cos_1k(deg: i32) -> i32 {
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
}

//! kit_zone: cooked static colliders as extra walls for the capsule controller.
//!
//! Consumer: [`MatchWorld::with_kit_zone`](super::MatchWorld::with_kit_zone) folds a
//! zone into the world every fighter is resolved against; the window game opts in by
//! building its world through that constructor, and the door's cast verbs read the same
//! colliders. Data owner: the asset cook writes the collider bytes and
//! `engine-physics` reads them ([`engine_physics::collider`]); this module owns only
//! what the fighter controller does with them. Units: meters, Y up.
//! Determinism: footprints are built once with `+ - * /`, `sqrt` and comparisons,
//! colliders keep their cooked order, and the zone digest is cached at build, so the same
//! cooked bytes always give the same footprints, pushes and digest. Errors:
//! [`KitZoneError`], never a panic. Budget: build once at load; a resolve touches one
//! bounding-box test per footprint and allocates nothing.
//!
//! The fighter is a vertical capsule on the floor plane. A collider only matters where it
//! meets the capsule's body, so each shape is cut to the band between the step height
//! ([`STEP_M`]; the controller walks over anything lower) and the fighter's height
//! ([`crate::FIGHTER_HEIGHT_M`]; anything higher is walked under), then flattened to the
//! floor plane. A convex hull becomes one solid convex footprint. A triangle mesh becomes
//! one footprint per triangle: a roof or deck slice is a solid polygon, a wall face is a
//! segment, and a fighter is pushed off either by the shortest way. Only layers that
//! block a fighter ([`engine_physics::layers::blocks`]) make footprints; debris and
//! sensor colliders stay in the zone for casts but never stop a walk.
//!
//! A mesh is treated as a surface to stay off, not a volume to be pushed out of: a
//! fighter placed deep inside a closed mesh is not lifted out, so spawns must start
//! outside kit pieces.

use core::fmt;

use engine_core::Fnv64;
use engine_physics::collider::{self, Collider, ColliderError, Shape};
use engine_physics::layers;

use crate::{canon_zero, FIGHTER_HEIGHT_M};

/// Lowest height a collider stops a fighter: the controller steps over anything under it
/// (`controller2` step height).
pub const STEP_M: f32 = 0.5;

/// Most footprints one zone holds; a piece with more triangles in the fighter band than
/// this belongs in a simpler collision mesh.
pub const MAX_FOOTPRINTS: usize = 8192;

/// Clearance added to every push so a resolved circle never touches.
const SKIN_M: f32 = 1.0e-4;

/// Twice-area (square meters) at or under which a flattened shape is a segment.
const SLIVER_AREA2_M2: f32 = 1.0e-6;

/// Why a kit zone was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KitZoneError {
    /// The cooked bytes did not decode.
    Cooked(ColliderError),
    /// More colliders than [`collider::MAX_COLLIDERS`].
    TooManyColliders(usize),
    /// A collider's layer is not exactly one known layer bit (id carried).
    BadLayer(String),
    /// A collider holds a NaN or infinite coordinate (id carried).
    NonFinite(String),
    /// The fighter band cuts more than [`MAX_FOOTPRINTS`] footprints out of the zone.
    TooManyFootprints(usize),
}

impl fmt::Display for KitZoneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cooked(error) => write!(f, "kit zone bytes: {error}"),
            Self::TooManyColliders(n) => write!(
                f,
                "kit zone holds {n} colliders, at most {}",
                collider::MAX_COLLIDERS
            ),
            Self::BadLayer(id) => write!(f, "kit collider {id} is not on exactly one layer"),
            Self::NonFinite(id) => write!(f, "kit collider {id} holds a non-finite coordinate"),
            Self::TooManyFootprints(n) => write!(
                f,
                "kit zone cuts {n} footprints in the fighter band, at most {MAX_FOOTPRINTS}"
            ),
        }
    }
}

impl std::error::Error for KitZoneError {}

/// One floor-plane footprint: a convex polygon (counter-clockwise) or a two-point segment.
#[derive(Clone, Debug, PartialEq)]
struct Footprint {
    pts: Vec<[f32; 2]>,
    lo: [f32; 2],
    hi: [f32; 2],
    solid: bool,
}

/// The cooked static colliders of one kit zone and their fighter footprints.
#[derive(Clone, Debug, PartialEq)]
pub struct KitZone {
    colliders: Vec<Collider>,
    footprints: Vec<Footprint>,
    digest: u64,
}

impl KitZone {
    /// Build a zone from cooked bytes ([`collider::decode`]), then [`KitZone::try_new`].
    pub fn from_cooked(bytes: &[u8]) -> Result<Self, KitZoneError> {
        Self::try_new(collider::decode(bytes).map_err(KitZoneError::Cooked)?)
    }

    /// Build a zone from colliders in cooked order.
    pub fn try_new(colliders: Vec<Collider>) -> Result<Self, KitZoneError> {
        if colliders.len() > collider::MAX_COLLIDERS {
            return Err(KitZoneError::TooManyColliders(colliders.len()));
        }
        let mut footprints = Vec::new();
        let mut hash = Fnv64::new();
        hash.write_u32(u32::try_from(colliders.len()).unwrap_or(u32::MAX));
        for c in &colliders {
            if !layers::is_single(c.layer) {
                return Err(KitZoneError::BadLayer(c.id.clone()));
            }
            fold_collider(&mut hash, c).map_err(|()| KitZoneError::NonFinite(c.id.clone()))?;
            if layers::blocks(layers::FIGHTER, c.layer) {
                footprints_of(&c.shape, &mut footprints);
                if footprints.len() > MAX_FOOTPRINTS {
                    return Err(KitZoneError::TooManyFootprints(footprints.len()));
                }
            }
        }
        Ok(Self {
            colliders,
            footprints,
            digest: hash.finish(),
        })
    }

    /// The colliders in cooked order.
    pub fn colliders(&self) -> &[Collider] {
        &self.colliders
    }

    /// True when the zone holds no collider at all.
    pub fn is_empty(&self) -> bool {
        self.colliders.is_empty()
    }

    /// FNV-1a over every collider's id, layer and shape bits; fixed at build.
    pub fn digest(&self) -> u64 {
        self.digest
    }

    /// Footprints that stop a fighter.
    pub fn footprint_count(&self) -> usize {
        self.footprints.len()
    }

    /// Push a circle of `radius` at `p` out of every footprint, in cooked order; `None`
    /// when nothing touches it.
    pub fn push_out(&self, p: [f32; 2], radius: f32) -> Option<[f32; 2]> {
        let mut moved = None;
        let mut at = p;
        for footprint in &self.footprints {
            if let Some(q) = footprint.push_out(at, radius) {
                at = q;
                moved = Some(q);
            }
        }
        moved
    }

    /// True when a circle of `radius` at `p` touches any footprint.
    pub fn overlaps(&self, p: [f32; 2], radius: f32) -> bool {
        self.footprints
            .iter()
            .any(|footprint| footprint.push_out(p, radius).is_some())
    }
}

/// Fold one collider into the zone hash; `Err` on a non-finite coordinate.
fn fold_collider(hash: &mut Fnv64, c: &Collider) -> Result<(), ()> {
    hash.write_str(&c.id);
    hash.write_u32(c.layer);
    let (points, flat): (&[[f32; 3]], Vec<u32>) = match &c.shape {
        Shape::Hull { points, faces } => (
            points,
            faces.iter().flatten().map(|&i| u32::from(i)).collect(),
        ),
        Shape::Mesh { vertices, indices } => (vertices, indices.clone()),
    };
    hash.write_u32(u32::try_from(points.len()).unwrap_or(u32::MAX));
    for v in points.iter().flatten() {
        if !v.is_finite() {
            return Err(());
        }
        hash.write_f32(*v);
    }
    hash.write_u32(u32::try_from(flat.len()).unwrap_or(u32::MAX));
    for i in flat {
        hash.write_u32(i);
    }
    Ok(())
}

/// Cut a convex polygon to one side of the plane `y = bound` (Sutherland-Hodgman).
fn clip_height(poly: &[[f32; 3]], bound: f32, keep_above: bool) -> Vec<[f32; 3]> {
    let inside = |p: &[f32; 3]| {
        if keep_above {
            p[1] >= bound
        } else {
            p[1] <= bound
        }
    };
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (i, cur) in poly.iter().enumerate() {
        let next = &poly[(i + 1) % poly.len()];
        let (here, there) = (inside(cur), inside(next));
        if here {
            out.push(*cur);
        }
        if here != there {
            let t = (bound - cur[1]) / (next[1] - cur[1]);
            out.push([
                cur[0] + (next[0] - cur[0]) * t,
                bound,
                cur[2] + (next[2] - cur[2]) * t,
            ]);
        }
    }
    out
}

/// The floor-plane points of a triangle's part inside the fighter band.
fn band_points(tri: &[[f32; 3]; 3]) -> Vec<[f32; 2]> {
    let above = clip_height(tri, STEP_M, true);
    if above.is_empty() {
        return Vec::new();
    }
    clip_height(&above, FIGHTER_HEIGHT_M, false)
        .iter()
        .map(|p| [p[0], p[2]])
        .collect()
}

/// Twice the signed area of the turn `o -> a -> b`.
fn turn(o: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/// One half of the monotone chain over points already sorted along the walk: keeps
/// the strictly left turns and drops the last point (the other half starts there).
fn half_hull<'a>(walk: impl Iterator<Item = &'a [f32; 2]>) -> Vec<[f32; 2]> {
    let mut chain: Vec<[f32; 2]> = Vec::new();
    for p in walk {
        while chain.len() >= 2 && turn(chain[chain.len() - 2], chain[chain.len() - 1], *p) <= 0.0 {
            chain.pop();
        }
        chain.push(*p);
    }
    chain.pop();
    chain
}

/// Convex hull of floor points, counter-clockwise (Andrew's monotone chain).
fn hull_2d(mut pts: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let mut hull = half_hull(pts.iter());
    hull.extend(half_hull(pts.iter().rev()));
    hull
}

/// A footprint from the floor points of one shape slice: a solid polygon when it has
/// area, else the segment between its two farthest points; `None` for a point or nothing.
fn footprint_from(points: Vec<[f32; 2]>) -> Option<Footprint> {
    let hull = hull_2d(points);
    if hull.len() < 2 {
        return None;
    }
    let area2: f32 = (0..hull.len())
        .map(|i| {
            let (a, b) = (hull[i], hull[(i + 1) % hull.len()]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    let pts = if hull.len() >= 3 && area2 > SLIVER_AREA2_M2 {
        hull
    } else {
        let mut best = (0.0f32, hull[0], hull[0]);
        for a in &hull {
            for b in &hull {
                let d = (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]);
                if d > best.0 {
                    best = (d, *a, *b);
                }
            }
        }
        if best.0 < 1.0e-10 {
            return None;
        }
        vec![best.1, best.2]
    };
    let solid = pts.len() >= 3;
    let mut lo = pts[0];
    let mut hi = pts[0];
    for p in &pts {
        for axis in 0..2 {
            lo[axis] = lo[axis].min(p[axis]);
            hi[axis] = hi[axis].max(p[axis]);
        }
    }
    Some(Footprint { pts, lo, hi, solid })
}

/// Append the footprints of one shape: one solid for a hull (a convex solid's band slice
/// is the hull of its faces' band points), one per triangle for a mesh.
fn footprints_of(shape: &Shape, out: &mut Vec<Footprint>) {
    match shape {
        Shape::Hull { .. } => {
            let points: Vec<[f32; 2]> = shape
                .triangles()
                .flat_map(|tri| band_points(&tri))
                .collect();
            out.extend(footprint_from(points));
        }
        Shape::Mesh { .. } => {
            out.extend(
                shape
                    .triangles()
                    .filter_map(|tri| footprint_from(band_points(&tri))),
            );
        }
    }
}

/// Closest point to `p` on the segment `a -> b`.
fn closest_on_segment(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> [f32; 2] {
    let e = [b[0] - a[0], b[1] - a[1]];
    let len2 = e[0] * e[0] + e[1] * e[1];
    if len2 <= 0.0 {
        return a;
    }
    let t = (((p[0] - a[0]) * e[0] + (p[1] - a[1]) * e[1]) / len2).clamp(0.0, 1.0);
    [a[0] + e[0] * t, a[1] + e[1] * t]
}

impl Footprint {
    /// Push a circle of `radius` at `p` clear of this footprint along the shortest way
    /// out; `None` when it does not touch. A circle with its centre inside a solid
    /// footprint leaves through the nearest edge.
    fn push_out(&self, p: [f32; 2], radius: f32) -> Option<[f32; 2]> {
        if p[0] < self.lo[0] - radius
            || p[0] > self.hi[0] + radius
            || p[1] < self.lo[1] - radius
            || p[1] > self.hi[1] + radius
        {
            return None;
        }
        let edges = if self.pts.len() == 2 {
            1
        } else {
            self.pts.len()
        };
        let mut inside = self.solid;
        let mut best = (f32::INFINITY, [0.0f32; 2], 0usize);
        for i in 0..edges {
            let (a, b) = (self.pts[i], self.pts[(i + 1) % self.pts.len()]);
            if self.solid && turn(a, b, p) < 0.0 {
                inside = false;
            }
            let q = closest_on_segment(a, b, p);
            let d2 = (p[0] - q[0]) * (p[0] - q[0]) + (p[1] - q[1]) * (p[1] - q[1]);
            if d2 < best.0 {
                best = (d2, q, i);
            }
        }
        let (d2, q, edge) = best;
        if !inside && d2 >= radius * radius {
            return None;
        }
        let reach = radius + SKIN_M;
        let dir = if inside || d2 <= 1.0e-12 {
            // On or inside the boundary: out through the nearest edge's outward side.
            let (a, b) = (self.pts[edge], self.pts[(edge + 1) % self.pts.len()]);
            let e = [b[0] - a[0], b[1] - a[1]];
            let len = (e[0] * e[0] + e[1] * e[1]).sqrt();
            [e[1] / len, -e[0] / len]
        } else {
            let d = d2.sqrt();
            [(p[0] - q[0]) / d, (p[1] - q[1]) / d]
        };
        Some([
            canon_zero(q[0] + dir[0] * reach),
            canon_zero(q[1] + dir[1] * reach),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::LiveMatch;
    use crate::world::{MatchWorld, Obstacle};
    use crate::{ArenaConfig, ArenaRules, FighterId, FighterInput, FIGHTER_RADIUS_M};
    use engine_input::fighter::Buttons;

    /// An axis-aligned box hull from `lo` to `hi`: eight points, twelve outward faces.
    fn box_hull(lo: [f32; 3], hi: [f32; 3]) -> Shape {
        let mut points = Vec::new();
        for x in [lo[0], hi[0]] {
            for y in [lo[1], hi[1]] {
                for z in [lo[2], hi[2]] {
                    points.push([x, y, z]);
                }
            }
        }
        // Point index = x * 4 + y * 2 + z (low = 0, high = 1).
        let faces = vec![
            [0, 1, 3],
            [0, 3, 2],
            [4, 6, 7],
            [4, 7, 5],
            [0, 4, 5],
            [0, 5, 1],
            [2, 3, 7],
            [2, 7, 6],
            [0, 2, 6],
            [0, 6, 4],
            [1, 5, 7],
            [1, 7, 3],
        ];
        Shape::Hull { points, faces }
    }

    /// The same box as a triangle mesh (one vertex per corner).
    fn box_mesh(lo: [f32; 3], hi: [f32; 3]) -> Shape {
        let Shape::Hull { points, faces } = box_hull(lo, hi) else {
            unreachable!("box_hull builds a hull");
        };
        Shape::Mesh {
            vertices: points,
            indices: faces.iter().flatten().map(|&i| u32::from(i)).collect(),
        }
    }

    fn collider(id: &str, layer: u32, shape: Shape) -> Collider {
        Collider {
            id: id.to_owned(),
            layer,
            shape,
        }
    }

    /// A 1 m square pillar, 3 m tall, centred on the origin.
    fn pillar() -> KitZone {
        KitZone::try_new(vec![collider(
            "COL_pillar",
            layers::WORLD,
            box_hull([-0.5, 0.0, -0.5], [0.5, 3.0, 0.5]),
        )])
        .expect("pillar zone")
    }

    /// Drive a circle straight at `-z` from `start` in centimeter steps.
    fn walk_south(world: &MatchWorld, start: [f32; 2], steps: usize) -> Vec<[f32; 2]> {
        let mut p = start;
        let mut out = Vec::new();
        for _ in 0..steps {
            p = world.resolve(p, [p[0], p[1] - 0.01], FIGHTER_RADIUS_M, 19.6);
            out.push(p);
        }
        out
    }

    #[test]
    fn kit_collision_a_walk_stops_at_the_pillar_and_the_open_world_walks_through() {
        let world = MatchWorld::empty().with_kit_zone(pillar());
        let walked = walk_south(&world, [0.0, 3.0], 500);
        for p in &walked {
            assert!(
                !world.overlaps(*p, FIGHTER_RADIUS_M),
                "inside the pillar at {p:?}"
            );
        }
        let nearest = walked.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        assert!(
            (0.5 + FIGHTER_RADIUS_M - 0.01..0.5 + FIGHTER_RADIUS_M + 0.05).contains(&nearest),
            "the walk must stop at the near face plus the radius, got {nearest}"
        );
        // The same walk without the collider goes straight through.
        let open = walk_south(&MatchWorld::empty(), [0.0, 3.0], 500);
        let through = open.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        assert!(
            through < -1.5,
            "the open arena lets the walker through, got {through}"
        );
    }

    #[test]
    fn kit_collision_cooked_bytes_become_a_blocking_prop() {
        // The cook's checked-in unit-cube hull: a 1 m prop with x and z in 0..1.
        let bytes = include_bytes!("../../../content/physics/unit_cube_hull.e2cc");
        let zone = KitZone::from_cooked(bytes).expect("cooked bytes build a zone");
        assert_eq!(zone.colliders()[0].id, "COL_box");
        assert_eq!(zone.footprint_count(), 1, "the prop layer blocks a fighter");
        let world = MatchWorld::empty().with_kit_zone(zone);
        let walked = walk_south(&world, [0.5, 3.0], 500);
        let nearest = walked.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        assert!(
            (1.0 + FIGHTER_RADIUS_M - 0.01..1.0 + FIGHTER_RADIUS_M + 0.05).contains(&nearest),
            "stopped at the cube's near face plus the radius, got {nearest}"
        );
        assert!(walked.iter().all(|p| !world.overlaps(*p, FIGHTER_RADIUS_M)));
    }

    /// Slot 0 of an idle two-fighter match walks `-z` at full speed, slot 1 stands.
    fn drive_south(config: &ArenaConfig, world: &MatchWorld) -> Vec<[f32; 3]> {
        let mut live = LiveMatch::with_rules_world(config, ArenaRules::V1, world.clone())
            .expect("match builds");
        let mut out = Vec::new();
        for tick in 0..config.ticks {
            let inputs = [0u8, 1].map(|slot| FighterInput {
                tick,
                fighter: FighterId(slot),
                mx: 0,
                mz: if slot == 0 { -100 } else { 0 },
                facing: 0,
                buttons: Buttons::NONE,
            });
            let stepped = live.step(&inputs).expect("tick steps");
            out.push(stepped.rows[0].observation.fighters[0].pos);
        }
        out
    }

    #[test]
    fn kit_collision_a_fighter_cannot_walk_through_a_kit_pillar() {
        // Find a seed whose settled slot 0 spawn leaves room to walk south, then put
        // a pillar dead ahead of it.
        let mut chosen = None;
        for seed in 0..64u64 {
            let config = ArenaConfig::try_new(seed, 600, 2, 12.0).expect("valid config");
            let mut probe = LiveMatch::new(&config).expect("probe");
            let spawn = probe.step(&[]).expect("probe tick").rows[0]
                .observation
                .fighters[0]
                .pos;
            if spawn[2] > -4.0 && spawn[2] < 6.0 {
                chosen = Some((config, spawn));
                break;
            }
        }
        let (config, spawn) = chosen.expect("a seed spawns with room ahead");
        let centre = [spawn[0], spawn[2] - 3.0];
        let zone = KitZone::try_new(vec![collider(
            "COL_pillar",
            layers::WORLD,
            box_hull(
                [centre[0] - 0.5, 0.0, centre[1] - 0.5],
                [centre[0] + 0.5, 3.0, centre[1] + 0.5],
            ),
        )])
        .expect("pillar zone");
        let world = MatchWorld::empty().with_kit_zone(zone);
        let near_face = centre[1] + 0.5;

        let blocked = drive_south(&config, &world);
        assert_eq!(blocked.len(), 600);
        for pose in &blocked {
            assert!(
                !world.overlaps([pose[0], pose[2]], FIGHTER_RADIUS_M),
                "fighter inside the pillar at {pose:?}"
            );
            assert!(
                pose[2] > near_face,
                "fighter reached the pillar face at {pose:?}"
            );
        }
        let reached = blocked.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!(
            reached < near_face + FIGHTER_RADIUS_M + 0.2,
            "the drive must reach the pillar, got {reached} (face {near_face})"
        );

        // The same drive with no collider walks straight through the spot.
        let open = drive_south(&config, &MatchWorld::empty());
        let through = open.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!(
            through < centre[1] - 0.5 - FIGHTER_RADIUS_M,
            "the open arena must let the fighter through, got {through}"
        );
    }

    #[test]
    fn kit_collision_a_mesh_wall_blocks_from_both_sides_and_a_roof_does_not() {
        let wall = box_mesh([-2.0, 0.0, -0.2], [2.0, 3.0, 0.2]);
        let zone = KitZone::try_new(vec![collider("COL_wall", layers::WORLD, wall)]).expect("zone");
        let world = MatchWorld::empty().with_kit_zone(zone);
        let from_south = walk_south(&world, [0.0, 3.0], 500);
        assert!(from_south
            .iter()
            .all(|p| !world.overlaps(*p, FIGHTER_RADIUS_M)));
        let nearest = from_south
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min);
        assert!(
            nearest > 0.2 + FIGHTER_RADIUS_M - 0.01,
            "stopped at the face, got {nearest}"
        );
        // From the far side, walking north.
        let mut p = [0.0, -3.0];
        for _ in 0..500 {
            p = world.resolve(p, [p[0], p[1] + 0.01], FIGHTER_RADIUS_M, 19.6);
            assert!(
                !world.overlaps(p, FIGHTER_RADIUS_M),
                "inside the wall at {p:?}"
            );
        }
        assert!(
            p[1] < -0.2 - FIGHTER_RADIUS_M + 0.01,
            "stopped at the back face, got {p:?}"
        );
        // A roof slab above head height is walked under; a low kerb is stepped over.
        let roof = collider(
            "COL_roof",
            layers::WORLD,
            box_mesh([-2.0, 2.0, -2.0], [2.0, 2.5, 2.0]),
        );
        let kerb = collider(
            "COL_kerb",
            layers::WORLD,
            box_hull([-2.0, 0.0, -2.0], [2.0, 0.4, 2.0]),
        );
        let walk_in =
            MatchWorld::empty().with_kit_zone(KitZone::try_new(vec![roof, kerb]).expect("zone"));
        assert_eq!(walk_in.kit_zone().map(KitZone::footprint_count), Some(0));
        let crossed = walk_south(&walk_in, [0.0, 3.0], 500);
        assert!(
            crossed.last().is_some_and(|p| p[1] < -1.5),
            "walked under and over"
        );
    }

    #[test]
    fn kit_collision_debris_and_sensors_never_stop_a_fighter_but_props_do() {
        let shape = || box_hull([-0.5, 0.0, -0.5], [0.5, 3.0, 0.5]);
        for (layer, stops) in [
            (layers::WORLD, true),
            (layers::PROP, true),
            (layers::DEBRIS, false),
            (layers::SENSOR, false),
        ] {
            let zone = KitZone::try_new(vec![collider("COL_x", layer, shape())]).expect("zone");
            assert_eq!(zone.footprint_count(), usize::from(stops), "layer {layer}");
            assert_eq!(zone.colliders().len(), 1, "casts still see it");
            let world = MatchWorld::empty().with_kit_zone(zone);
            let ended = walk_south(&world, [0.0, 3.0], 500).pop().expect("steps");
            assert_eq!(ended[1] > 0.0, stops, "layer {layer} ended at {ended:?}");
        }
    }

    #[test]
    fn kit_collision_settle_moves_a_spawn_out_of_a_pillar() {
        let world = MatchWorld::empty().with_kit_zone(pillar());
        let placed = world
            .settle([0.1, 0.1], FIGHTER_RADIUS_M, 19.6)
            .expect("a free spot");
        assert!(!world.overlaps(placed, FIGHTER_RADIUS_M));
        assert!(
            placed[0].abs() > 0.5 || placed[1].abs() > 0.5,
            "left the pillar: {placed:?}"
        );
    }

    #[test]
    fn kit_collision_the_digest_of_a_world_without_a_zone_is_unchanged() {
        let a = Obstacle::aligned([3.0, 1.0], [0.5, 0.5]);
        let boxes = MatchWorld::try_new(vec![a]).expect("valid");
        // The digest as it was before zones existed: count, then the six lanes.
        let mut old = Fnv64::new();
        old.write_u32(1);
        for lane in [3.0f32, 1.0, 0.5, 0.5, 1.0, 0.0] {
            old.write_u32(lane.to_bits());
        }
        assert_eq!(boxes.digest(), old.finish());
        assert_eq!(MatchWorld::empty().digest(), 0);
        assert_eq!(
            boxes
                .clone()
                .with_kit_zone(KitZone::try_new(Vec::new()).expect("empty")),
            boxes
        );
        // A real zone moves the digest, on empty and on boxed worlds alike.
        let zoned = boxes.clone().with_kit_zone(pillar());
        assert_ne!(zoned.digest(), boxes.digest());
        assert_ne!(MatchWorld::empty().with_kit_zone(pillar()).digest(), 0);
        assert!(!MatchWorld::empty().with_kit_zone(pillar()).is_empty());
        assert_eq!(
            zoned.digest(),
            boxes.clone().with_kit_zone(pillar()).digest()
        );
    }

    #[test]
    fn kit_collision_a_zone_in_a_live_match_moves_the_checkpoint_hash() {
        let config = ArenaConfig::try_new(5, 30, 2, 12.0).expect("valid config");
        let hash_of = |world: MatchWorld| {
            let mut live =
                LiveMatch::with_rules_world(&config, ArenaRules::V1, world).expect("builds");
            let mut last = None;
            while let Some(tick) = live.step(&[]) {
                last = Some(tick.checkpoint.hash);
            }
            last
        };
        let open = hash_of(MatchWorld::empty());
        let zoned = hash_of(MatchWorld::empty().with_kit_zone(pillar()));
        assert!(open.is_some() && zoned.is_some());
        assert_ne!(open, zoned, "two worlds never share a hash");
        assert_eq!(
            zoned,
            hash_of(MatchWorld::empty().with_kit_zone(pillar())),
            "same world, same hash"
        );
    }

    #[test]
    fn kit_collision_bad_zones_are_refused_by_name() {
        let nan = collider(
            "COL_nan",
            layers::WORLD,
            Shape::Hull {
                points: vec![
                    [f32::NAN, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [0.0, 0.0, 1.0],
                ],
                faces: vec![[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]],
            },
        );
        assert_eq!(
            KitZone::try_new(vec![nan]),
            Err(KitZoneError::NonFinite("COL_nan".to_owned()))
        );
        let two_bits = collider(
            "COL_two",
            layers::WORLD | layers::PROP,
            box_hull([0.0; 3], [1.0; 3]),
        );
        assert_eq!(
            KitZone::try_new(vec![two_bits]),
            Err(KitZoneError::BadLayer("COL_two".to_owned()))
        );
        let many = (0..=collider::MAX_COLLIDERS)
            .map(|i| {
                collider(
                    &format!("COL_{i}"),
                    layers::WORLD,
                    box_hull([0.0; 3], [1.0; 3]),
                )
            })
            .collect();
        assert!(matches!(
            KitZone::try_new(many),
            Err(KitZoneError::TooManyColliders(_))
        ));
        assert!(matches!(
            KitZone::from_cooked(b"nope"),
            Err(KitZoneError::Cooked(_))
        ));
        assert!(KitZone::try_new(Vec::new()).expect("empty").is_empty());
    }
}

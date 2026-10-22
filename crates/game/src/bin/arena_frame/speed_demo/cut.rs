//! cut (SD4, docs/SPRINT-33-speed-demo.md): the blade cuts a tower in two.
//!
//! The blade's plane splits the tower shell (`engine_destruction::csg::knife`, which leaves
//! both halves open along the cut); each half gets a cap so it reads solid from inside, the
//! lower half stays where it stood and the upper half slides down the cut plane, then falls
//! and tumbles once it has slid off its base. Presentation only, deterministic: fixed steps,
//! no clock, no RNG. Units: meters, seconds.

use super::num::{add, centroid, cross, dot, idx, len, scale, sub, unit};
use engine_destruction::csg::{knife, CutPlane, PropMesh};

/// Vertices within this of the cut plane belong to the cap loop; meters.
const ON_PLANE_M: f32 = 1e-3;
/// Gravity, m/s^2.
pub(super) const GRAVITY_M_S2: f32 = 9.81;
/// Sliding friction along the cut face (a steel-clean cut through concrete is slick).
const SLIDE_FRICTION: f32 = 0.15;
/// Tumble rate once a half has slid off, rad/s per m/s of its speed.
const TUMBLE_PER_SPEED: f32 = 0.08;

/// The two closed halves of a cut: `stays` keeps standing, `falls` slides off.
#[derive(Clone, Debug)]
pub(super) struct Cut {
    pub stays: PropMesh,
    pub falls: PropMesh,
    /// The cut plane, unit normal pointing from `stays` into `falls`.
    pub normal: [f32; 3],
    pub dist: f32,
}

/// Cut `tower` along `plane`. `None` when the plane misses the tower (or only grazes it);
/// otherwise the higher half falls and both halves are capped.
pub(super) fn cut_tower(tower: &PropMesh, plane: &CutPlane) -> Option<Cut> {
    let len = dot(plane.normal, plane.normal).sqrt();
    if !len.is_finite() || len <= 1e-9 {
        return None;
    }
    let mut normal = scale(plane.normal, 1.0 / len);
    let mut dist = plane.dist / len;
    let (front, back) = knife(tower, plane).ok()?;
    // knife's front is where normal . p - dist > 0; the falling half is the higher one.
    let (mut stays, mut falls) = (back, front);
    if centroid(&falls.positions)[1] < centroid(&stays.positions)[1] {
        std::mem::swap(&mut stays, &mut falls);
        normal = scale(normal, -1.0);
        dist = -dist;
    }
    // The cut face of `stays` looks along +normal (toward the falling half), the cut face
    // of `falls` along -normal.
    cap(&mut stays, normal, dist, normal);
    cap(&mut falls, normal, dist, scale(normal, -1.0));
    Some(Cut {
        stays,
        falls,
        normal,
        dist,
    })
}

/// Close an open half: the vertices on the cut plane form a convex loop (a tower is a
/// box), sorted around their centroid and fanned, every triangle facing `outward`.
fn cap(half: &mut PropMesh, normal: [f32; 3], dist: f32, outward: [f32; 3]) {
    let mut ring: Vec<[f32; 3]> = Vec::new();
    for &p in &half.positions {
        if (dot(normal, p) - dist).abs() <= ON_PLANE_M
            && !ring
                .iter()
                .any(|q| dot(sub(*q, p), sub(*q, p)) <= ON_PLANE_M * ON_PLANE_M)
        {
            ring.push(p);
        }
    }
    if ring.len() < 3 {
        return;
    }
    let c = centroid(&ring);
    let u = unit(sub(ring[0], c));
    let v = cross(normal, u);
    ring.sort_by(|a, b| {
        let (da, db) = (sub(*a, c), sub(*b, c));
        let ta = dot(da, v).atan2(dot(da, u));
        let tb = dot(db, v).atan2(dot(db, u));
        ta.total_cmp(&tb)
    });
    let base = u32::try_from(half.positions.len()).unwrap_or(u32::MAX);
    half.positions.push(c);
    half.positions.extend_from_slice(&ring);
    let n = u32::try_from(ring.len()).unwrap_or(0);
    for k in 0..n {
        let (a, b) = (base + 1 + k, base + 1 + (k + 1) % n);
        let (pa, pb) = (half.positions[idx(a)], half.positions[idx(b)]);
        if dot(cross(sub(pa, c), sub(pb, c)), outward) >= 0.0 {
            half.indices.extend_from_slice(&[base, a, b]);
        } else {
            half.indices.extend_from_slice(&[base, b, a]);
        }
    }
}

/// The falling half's motion: it slides down the cut face while its centre is still over
/// the face's footprint, then falls free and tumbles. `offset` and `angle` move its mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Slide {
    pub offset: [f32; 3],
    pub vel: [f32; 3],
    /// Tumble about `axis`, radians.
    pub angle: f32,
    pub axis: [f32; 3],
    normal: [f32; 3],
    /// How far it may slide before it leaves the face, meters (the face's radius).
    reach_m: f32,
    pub sliding: bool,
}

impl Slide {
    /// Start sliding on a cut; `kick` is the blade's push (m/s, any direction).
    pub(super) fn start(cut: &Cut, kick: [f32; 3]) -> Self {
        let normal = cut.normal;
        // The face radius: the largest distance of a cap vertex from the cut centroid.
        let on_face: Vec<[f32; 3]> = cut
            .falls
            .positions
            .iter()
            .copied()
            .filter(|p| (dot(normal, *p) - cut.dist).abs() <= ON_PLANE_M)
            .collect();
        let c = centroid(&on_face);
        let reach_m = on_face.iter().map(|p| len(sub(*p, c))).fold(0.0, f32::max);
        // Only the in-plane part of the kick slides it; pushing into the face does nothing.
        let vel = sub(kick, scale(normal, dot(kick, normal).min(0.0)));
        let axis = unit(cross(normal, [0.0, 1.0, 0.0]));
        Slide {
            offset: [0.0; 3],
            vel,
            angle: 0.0,
            axis,
            normal,
            reach_m,
            sliding: true,
        }
    }

    /// Advance `dt` seconds.
    pub(super) fn step(&mut self, dt: f32) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let g = [0.0, -GRAVITY_M_S2, 0.0];
        if self.sliding {
            // Gravity along the face, less friction against the normal load; the face
            // holds it up as long as the normal faces up at all.
            let load = -dot(g, self.normal);
            if load <= 0.0 {
                self.sliding = false;
            } else {
                let along = sub(g, scale(self.normal, dot(g, self.normal)));
                let speed = len(self.vel);
                let fric = if speed > 1e-6 {
                    scale(self.vel, -(SLIDE_FRICTION * load).min(speed / dt) / speed)
                } else {
                    [0.0; 3]
                };
                self.vel = add(self.vel, scale(add(along, fric), dt));
                // Never into the face.
                let into = dot(self.vel, self.normal);
                if into < 0.0 {
                    self.vel = sub(self.vel, scale(self.normal, into));
                }
                self.offset = add(self.offset, scale(self.vel, dt));
                if len(sub(
                    self.offset,
                    scale(self.normal, dot(self.offset, self.normal)),
                )) > self.reach_m
                {
                    self.sliding = false;
                }
                return;
            }
        }
        self.vel = add(self.vel, scale(g, dt));
        self.offset = add(self.offset, scale(self.vel, dt));
        self.angle += len(self.vel) * TUMBLE_PER_SPEED * dt;
    }
}

/// Enclosed volume of a closed, outward-wound shell (divergence theorem), m^3.
pub(super) fn volume(mesh: &PropMesh) -> f32 {
    let mut sum = 0.0;
    for t in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (
            mesh.positions[idx(t[0])],
            mesh.positions[idx(t[1])],
            mesh.positions[idx(t[2])],
        );
        sum += dot(a, cross(b, c));
    }
    sum / 6.0
}

/// An axis-aligned box shell, outward-wound (the tower stand-in and the test shape).
pub(super) fn box_shell(min: [f32; 3], max: [f32; 3]) -> PropMesh {
    let p = |x: usize, y: usize, z: usize| {
        [
            [min[0], max[0]][x],
            [min[1], max[1]][y],
            [min[2], max[2]][z],
        ]
    };
    let positions = vec![
        p(0, 0, 0),
        p(1, 0, 0),
        p(1, 1, 0),
        p(0, 1, 0),
        p(0, 0, 1),
        p(1, 0, 1),
        p(1, 1, 1),
        p(0, 1, 1),
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // -z
        4, 5, 6, 4, 6, 7, // +z
        0, 1, 5, 0, 5, 4, // -y
        3, 7, 6, 3, 6, 2, // +y
        0, 4, 7, 0, 7, 3, // -x
        1, 2, 6, 1, 6, 5, // +x
    ];
    PropMesh { positions, indices }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tower() -> PropMesh {
        box_shell([-5.0, 0.0, -5.0], [5.0, 40.0, 5.0])
    }

    /// A level cut halves the tower into two closed shells whose volumes sum to the tower;
    /// the upper half is the one that falls.
    #[test]
    fn a_level_cut_halves_a_tower_into_two_closed_shells() {
        let whole = volume(&tower());
        assert!(
            (whole - 4000.0).abs() < 1e-2,
            "the box is 10x40x10, got {whole}"
        );
        let cut = cut_tower(
            &tower(),
            &CutPlane {
                normal: [0.0, 1.0, 0.0],
                dist: 15.0,
            },
        )
        .expect("the plane crosses the tower");
        let (low, high) = (volume(&cut.stays), volume(&cut.falls));
        assert!(
            (low - 1500.0).abs() < 0.5,
            "below the cut: 10x15x10, got {low}"
        );
        assert!(
            (high - 2500.0).abs() < 0.5,
            "above the cut: 10x25x10, got {high}"
        );
        assert!(
            centroid(&cut.falls.positions)[1] > 15.0,
            "the upper half falls"
        );
        assert!(
            cut.normal[1] > 0.99,
            "the normal points into the falling half"
        );
    }

    /// A slanted cut still closes both halves; a plane that misses changes nothing.
    #[test]
    fn a_slanted_cut_closes_and_a_miss_does_nothing() {
        let n = unit([0.4, 1.0, 0.2]);
        let at = [0.0, 20.0, 0.0];
        let cut = cut_tower(
            &tower(),
            &CutPlane {
                normal: n,
                dist: dot(n, at),
            },
        )
        .expect("cuts");
        let sum = volume(&cut.stays) + volume(&cut.falls);
        assert!(
            (sum - 4000.0).abs() < 1.0,
            "the halves still make the tower, got {sum}"
        );
        assert!(
            volume(&cut.stays) > 0.0 && volume(&cut.falls) > 0.0,
            "both wound outward"
        );
        assert!(cut_tower(
            &tower(),
            &CutPlane {
                normal: [0.0, 1.0, 0.0],
                dist: 90.0
            }
        )
        .is_none());
    }

    /// On a slanted face the top half slides along the face, never into it, then leaves
    /// the face, falls and tumbles; on a level face with no push it stays put.
    #[test]
    fn the_top_half_slides_along_the_cut_then_falls() {
        let n = unit([0.5, 1.0, 0.0]);
        let cut = cut_tower(
            &tower(),
            &CutPlane {
                normal: n,
                dist: dot(n, [0.0, 20.0, 0.0]),
            },
        )
        .expect("cuts");
        let mut slide = Slide::start(&cut, [0.0; 3]);
        let dt = 1.0 / 60.0;
        let mut steps = 0;
        while slide.sliding && steps < 600 {
            slide.step(dt);
            assert!(
                dot(slide.offset, cut.normal).abs() < 1e-3,
                "slides along the face, not through it"
            );
            steps += 1;
        }
        assert!(!slide.sliding, "it slides off within 10 s");
        assert!(
            slide.offset[0] > 0.0,
            "downhill: a face whose normal leans +x falls away toward +x"
        );
        let y = slide.offset[1];
        for _ in 0..60 {
            slide.step(dt);
        }
        assert!(slide.offset[1] < y - 1.0, "then it falls");
        assert!(slide.angle > 0.0, "and tumbles");
        let level = cut_tower(
            &tower(),
            &CutPlane {
                normal: [0.0, 1.0, 0.0],
                dist: 20.0,
            },
        )
        .expect("cuts");
        let mut rest = Slide::start(&level, [0.0; 3]);
        for _ in 0..120 {
            rest.step(dt);
        }
        assert!(
            len(rest.offset) < 1e-4,
            "a level cut with no push stays put"
        );
    }
}

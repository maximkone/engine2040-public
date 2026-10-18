//! Pick manifest (EV21): the engine side of "point at it".
//!
//! A model that looks at a capture says "that thing, there". This module
//! turns a pixel of a capture into an exact entity id, or into a 3D point on
//! the floor when the pixel shows no entity, and back: [`Camera::project`]
//! and [`Camera::unproject`] are the pair, and a [`PickManifest`] lists the
//! frame as rectangular regions, each with one entity id or `-`, tiling the
//! frame with no gap and no overlap.
//!
//! Consumer: `scene.place` callers and the SP12 gold pointing eval
//! (`fixtures/pick/gold.txt`, 100 tasks). Data owner: the caller owns the
//! actors (the same `engine_spatial::Actor` rows the brief reads); this
//! module owns the camera maths and the manifest text. Units: integer
//! centimeters and centidegrees in the camera and the answers, f64 inside.
//! A box actor sits at its center `(x, y, z)` meters with full size
//! `(sx, sy, sz)` meters; the floor is `y = 0`. Duplicate names never pick:
//! a name that two actors share is [`NameError::Ambiguous`] and only an id
//! resolves it. Determinism scope: same-machine reproducibility, no wall
//! clock, no map order; the manifest text is byte-identical twice. Nothing
//! here feeds the simulation hash.

use engine_core::checked::{saturating_f64_to_u64, saturating_i64_to_i32};
use engine_core::hash_hex;
use engine_spatial::Actor;

/// How far a ray that meets neither an entity nor the floor reaches, in cm.
pub const FAR_CM: f64 = 10_000.0;

/// A capture camera: where it stands, where it looks and what it sees.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Camera {
    /// Eye position in centimeters, `y` up.
    pub eye_cm: [i32; 3],
    /// Turn about `y` in centidegrees; `0` looks along `+z`, positive turns
    /// toward `+x`.
    pub yaw_cdeg: i32,
    /// Tilt in centidegrees; negative looks down.
    pub pitch_cdeg: i32,
    /// Vertical field of view in centidegrees.
    pub fov_cdeg: i32,
    /// Frame size in pixels, width then height.
    pub size: [u32; 2],
}

/// What a pixel shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Picked {
    /// The nearest entity under the pixel, by actor id.
    Entity(u32),
    /// No entity: the floor point (or far point) the pixel's ray reaches, in
    /// integer centimeters.
    Point([i32; 3]),
}

/// Why a name did not resolve to one id.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NameError {
    /// No actor has the name.
    Missing,
    /// Several actors share the name; the ids that do.
    Ambiguous(Vec<u32>),
}

/// One manifest rectangle, `x1` and `y1` exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Region {
    /// Left pixel column.
    pub x0: u32,
    /// Top pixel row.
    pub y0: u32,
    /// Column after the right edge.
    pub x1: u32,
    /// Row after the bottom edge.
    pub y1: u32,
    /// Entity shown in the region, `None` for the floor.
    pub id: Option<u32>,
}

/// The frame as regions plus the camera that made them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PickManifest {
    /// Camera the regions were cast with.
    pub camera: Camera,
    /// Cell edge in pixels the frame was sampled at.
    pub cell: u32,
    /// Rectangles that tile the frame, in row-major order.
    pub regions: Vec<Region>,
}

/// Camera frame: unit right, up and forward vectors plus the focal length in
/// pixels.
struct Basis {
    right: [f64; 3],
    up: [f64; 3],
    forward: [f64; 3],
    focal: f64,
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cdeg(value: i32) -> f64 {
    (f64::from(value) / 100.0).to_radians()
}

/// Nearest integer centimeter (halves away from zero), saturating.
fn cm(value: f64) -> i32 {
    let magnitude = saturating_f64_to_u64(value.abs().round());
    let signed = i64::try_from(magnitude).unwrap_or(i64::MAX);
    saturating_i64_to_i32(if value < 0.0 { -signed } else { signed })
}

impl Camera {
    fn eye(&self) -> [f64; 3] {
        self.eye_cm.map(f64::from)
    }

    fn basis(&self) -> Basis {
        let (yaw, pitch) = (cdeg(self.yaw_cdeg), cdeg(self.pitch_cdeg));
        let forward = [
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            yaw.cos() * pitch.cos(),
        ];
        let right = [yaw.cos(), 0.0, -yaw.sin()];
        let up = [
            forward[1] * right[2] - forward[2] * right[1],
            forward[2] * right[0] - forward[0] * right[2],
            forward[0] * right[1] - forward[1] * right[0],
        ];
        let half_height = f64::from(self.size[1]) / 2.0;
        Basis {
            right,
            up,
            forward,
            focal: half_height / (cdeg(self.fov_cdeg) / 2.0).tan(),
        }
    }

    /// Frame pixel `(x, y)` of a world point in centimeters, with its depth
    /// along the view axis; `None` when the point is not in front of the eye.
    /// Pixel centers sit at half integers.
    pub fn project(&self, point_cm: [f64; 3]) -> Option<([f64; 2], f64)> {
        let b = self.basis();
        let eye = self.eye();
        let d = [
            point_cm[0] - eye[0],
            point_cm[1] - eye[1],
            point_cm[2] - eye[2],
        ];
        let depth = dot(d, b.forward);
        if depth <= 1e-9 {
            return None;
        }
        let (w, h) = (f64::from(self.size[0]), f64::from(self.size[1]));
        let x = w / 2.0 + b.focal * dot(d, b.right) / depth;
        let y = h / 2.0 - b.focal * dot(d, b.up) / depth;
        Some(([x, y], depth))
    }

    /// The world point in centimeters at `depth` along the view axis through
    /// frame pixel `pixel`: the inverse of [`Camera::project`].
    pub fn unproject(&self, pixel: [f64; 2], depth: f64) -> [f64; 3] {
        let b = self.basis();
        let eye = self.eye();
        let (w, h) = (f64::from(self.size[0]), f64::from(self.size[1]));
        let x = (pixel[0] - w / 2.0) * depth / b.focal;
        let y = -(pixel[1] - h / 2.0) * depth / b.focal;
        [0, 1, 2].map(|k| eye[k] + b.right[k] * x + b.up[k] * y + b.forward[k] * depth)
    }

    /// Unit ray direction through the center of pixel `(x, y)`.
    fn ray(&self, x: u32, y: u32) -> [f64; 3] {
        let far = self.unproject([f64::from(x) + 0.5, f64::from(y) + 0.5], 1.0);
        let eye = self.eye();
        let d = [far[0] - eye[0], far[1] - eye[1], far[2] - eye[2]];
        let len = dot(d, d).sqrt();
        d.map(|lane| lane / len)
    }

    /// What pixel `(x, y)` shows: the nearest entity box its ray enters (ties
    /// go to the lowest id), else the floor point, else the far point.
    pub fn pick(&self, actors: &[Actor], x: u32, y: u32) -> Picked {
        let (eye, dir) = (self.eye(), self.ray(x, y));
        let mut best: Option<(f64, u32)> = None;
        for actor in actors {
            if let Some(t) = hit_box(actor, eye, dir) {
                if best.is_none_or(|(bt, bid)| t < bt || (t == bt && actor.id < bid)) {
                    best = Some((t, actor.id));
                }
            }
        }
        if let Some((_, id)) = best {
            return Picked::Entity(id);
        }
        let t = if dir[1] < 0.0 && eye[1] > 0.0 {
            (-eye[1] / dir[1]).min(FAR_CM)
        } else {
            FAR_CM
        };
        Picked::Point([0, 1, 2].map(|k| cm(eye[k] + dir[k] * t)))
    }
}

/// Distance along the ray to the box of `actor`, `None` for a miss, a box
/// behind the eye or a row with no usable pose or size.
fn hit_box(actor: &Actor, eye: [f64; 3], dir: [f64; 3]) -> Option<f64> {
    let center = [actor.x, actor.y, actor.z].map(|m| f64::from(m) * 100.0);
    let half = [actor.sx, actor.sy, actor.sz].map(|m| f64::from(m) * 50.0);
    if !center.iter().chain(&half).all(|lane| lane.is_finite()) || half.iter().any(|h| *h <= 0.0) {
        return None;
    }
    let (mut near, mut far) = (0.0f64, f64::INFINITY);
    for k in 0..3 {
        let (lo, hi) = (center[k] - half[k], center[k] + half[k]);
        if dir[k].abs() < 1e-12 {
            if eye[k] < lo || eye[k] > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - eye[k]) / dir[k], (hi - eye[k]) / dir[k]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far && far > 0.0).then_some(near)
}

/// The one actor named `name`, or why no single actor is.
pub fn resolve_name(actors: &[Actor], name: &str) -> Result<u32, NameError> {
    let ids: Vec<u32> = actors
        .iter()
        .filter(|actor| actor.name == name)
        .map(|actor| actor.id)
        .collect();
    match ids.as_slice() {
        [] => Err(NameError::Missing),
        [one] => Ok(*one),
        _ => Err(NameError::Ambiguous(ids)),
    }
}

impl PickManifest {
    /// Sample the frame on a `cell`-pixel grid (cell centers) and merge equal
    /// neighbors into rectangles: runs along a row, then a run that repeats
    /// the run above it exactly grows that rectangle downward. `None` for a
    /// zero cell or an empty frame.
    pub fn build(camera: &Camera, actors: &[Actor], cell: u32) -> Option<Self> {
        let [w, h] = camera.size;
        if cell == 0 || w == 0 || h == 0 {
            return None;
        }
        let step = usize::try_from(cell).ok()?;
        let mut regions: Vec<Region> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        for y0 in (0..h).step_by(step) {
            let y1 = (y0 + cell).min(h);
            let mut runs: Vec<Region> = Vec::new();
            for x0 in (0..w).step_by(step) {
                let x1 = (x0 + cell).min(w);
                let id = match camera.pick(actors, (x0 + x1 - 1) / 2, (y0 + y1 - 1) / 2) {
                    Picked::Entity(id) => Some(id),
                    Picked::Point(_) => None,
                };
                match runs.last_mut() {
                    Some(run) if run.id == id => run.x1 = x1,
                    _ => runs.push(Region { x0, y0, x1, y1, id }),
                }
            }
            let mut next: Vec<usize> = Vec::new();
            for run in runs {
                let twin = open.iter().copied().find(|&i| {
                    let r = regions[i];
                    (r.x0, r.x1, r.id) == (run.x0, run.x1, run.id)
                });
                if let Some(i) = twin {
                    regions[i].y1 = y1;
                    next.push(i);
                } else {
                    next.push(regions.len());
                    regions.push(run);
                }
            }
            open = next;
        }
        Some(Self {
            camera: *camera,
            cell,
            regions,
        })
    }

    /// Reproducible text: the camera, one `region x0 y0 x1 y1 id|-` line per
    /// rectangle and an FNV digest of everything above it.
    pub fn text(&self) -> String {
        let c = &self.camera;
        let mut out = format!(
            "pick v1 eye_cm={},{},{} yaw_cdeg={} pitch_cdeg={} fov_cdeg={} size={}x{} cell={}\n",
            c.eye_cm[0],
            c.eye_cm[1],
            c.eye_cm[2],
            c.yaw_cdeg,
            c.pitch_cdeg,
            c.fov_cdeg,
            c.size[0],
            c.size[1],
            self.cell,
        );
        for r in &self.regions {
            let id = r.id.map_or_else(|| "-".to_owned(), |id| id.to_string());
            out.push_str(&format!(
                "region {} {} {} {} {id}\n",
                r.x0, r.y0, r.x1, r.y1
            ));
        }
        let digest = hash_hex(out.as_bytes());
        out.push_str(&format!("digest {digest}\n"));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLD: &str = include_str!("../../../fixtures/pick/gold.txt");

    fn scene() -> (Camera, Vec<Actor>) {
        let mut camera = None;
        let mut actors = Vec::new();
        for line in GOLD.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            match w.as_slice() {
                ["camera", eye, yaw, pitch, fov, size] => {
                    let e: Vec<i32> = eye.split(',').map(|v| v.parse().unwrap()).collect();
                    let (sw, sh) = size.split_once('x').unwrap();
                    camera = Some(Camera {
                        eye_cm: [e[0], e[1], e[2]],
                        yaw_cdeg: yaw.parse().unwrap(),
                        pitch_cdeg: pitch.parse().unwrap(),
                        fov_cdeg: fov.parse().unwrap(),
                        size: [sw.parse().unwrap(), sh.parse().unwrap()],
                    });
                }
                ["actor", id, name, x, y, z, sx, sy, sz] => actors.push(Actor {
                    id: id.parse().unwrap(),
                    name: (*name).to_owned(),
                    x: x.parse().unwrap(),
                    y: y.parse().unwrap(),
                    z: z.parse().unwrap(),
                    sx: sx.parse().unwrap(),
                    sy: sy.parse().unwrap(),
                    sz: sz.parse().unwrap(),
                    parent: None,
                    physical: true,
                    vx: 0.0,
                    vz: 0.0,
                    yaw: 0.0,
                }),
                _ => {}
            }
        }
        (camera.expect("camera line"), actors)
    }

    #[test]
    fn gold_pointing_eval_resolves_100_of_100() {
        let (camera, actors) = scene();
        let mut solved = 0;
        let mut entities = 0;
        for line in GOLD.lines().filter(|l| l.starts_with("task ")) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let (x, y): (u32, u32) = (w[1].parse().unwrap(), w[2].parse().unwrap());
            let want = match w[3] {
                "id" => {
                    entities += 1;
                    Picked::Entity(w[4].parse().unwrap())
                }
                _ => Picked::Point([
                    w[4].parse().unwrap(),
                    w[5].parse().unwrap(),
                    w[6].parse().unwrap(),
                ]),
            };
            let got = camera.pick(&actors, x, y);
            assert_eq!(got, want, "pixel {x},{y}");
            solved += 1;
        }
        assert_eq!(solved, 100);
        assert!(entities >= 30, "{entities} entity answers");
    }

    #[test]
    fn manifest_tiles_the_frame_and_reproduces_bytes() {
        let (camera, actors) = scene();
        let first = PickManifest::build(&camera, &actors, 16).expect("manifest");
        let second = PickManifest::build(&camera, &actors, 16).expect("manifest");
        assert_eq!(first.text(), second.text(), "bytes x2");
        let [w, h] = camera.size;
        let mut seen = vec![0u8; usize::try_from(w * h).unwrap()];
        for r in &first.regions {
            assert!(
                r.x0 < r.x1 && r.y0 < r.y1 && r.x1 <= w && r.y1 <= h,
                "{r:?}"
            );
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    seen[usize::try_from(y * w + x).unwrap()] += 1;
                }
            }
        }
        assert!(seen.iter().all(|n| *n == 1), "no gap and no overlap");
        // Each region shows what the exact pick shows at its sampled cell.
        for r in &first.regions {
            let at = camera.pick(
                &actors,
                (r.x0 + (r.x0 + 16).min(r.x1) - 1) / 2,
                (r.y0 + (r.y0 + 16).min(r.y1) - 1) / 2,
            );
            assert_eq!(
                at == Picked::Entity(r.id.unwrap_or(u32::MAX)),
                r.id.is_some(),
                "{r:?}"
            );
        }
        assert!(first.regions.iter().any(|r| r.id.is_some()));
        assert!(first.regions.iter().any(|r| r.id.is_none()));
        assert!(first.text().ends_with(&format!(
            "digest {}\n",
            hash_hex(first.text().rsplit_once("digest ").unwrap().0.as_bytes())
        )));
        assert!(PickManifest::build(&camera, &actors, 0).is_none());
    }

    #[test]
    fn duplicate_names_resolve_by_id_only() {
        let (camera, actors) = scene();
        assert_eq!(
            resolve_name(&actors, "crate"),
            Err(NameError::Ambiguous(vec![1, 3]))
        );
        assert_eq!(resolve_name(&actors, "barrel"), Ok(2));
        assert_eq!(resolve_name(&actors, "ghost"), Err(NameError::Missing));
        // The two crates are told apart by where they are pointed at.
        let seen = |id: u32| {
            let a = actors.iter().find(|a| a.id == id).unwrap();
            let center = [a.x, a.y, a.z].map(|m| f64::from(m) * 100.0);
            let ([px, py], _) = camera.project(center).expect("in front");
            (px.floor() as u32, py.floor() as u32)
        };
        for id in [1, 3] {
            let (x, y) = seen(id);
            assert_eq!(camera.pick(&actors, x, y), Picked::Entity(id));
        }
    }

    #[test]
    fn project_and_unproject_round_trip_within_a_centimeter() {
        let (camera, _) = scene();
        for step in 0..60u32 {
            let k = f64::from(step);
            let point = [-800.0 + k * 27.3, 10.0 + k * 3.1, 100.0 + k * 41.7];
            let (pixel, depth) = camera.project(point).expect("in front");
            let back = camera.unproject(pixel, depth);
            for lane in 0..3 {
                assert!(
                    (back[lane] - point[lane]).abs() < 1.0,
                    "{point:?} -> {back:?}"
                );
            }
        }
        // A point behind the eye does not project.
        assert!(camera.project([0.0, 600.0, -5000.0]).is_none());
    }
}

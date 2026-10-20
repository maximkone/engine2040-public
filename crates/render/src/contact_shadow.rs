//! contact_shadow: the screen-space contact march, CPU oracle.
//!
//! A shadow map loses the last centimetres where a body meets the ground: the
//! texel is too coarse and the depth bias eats the contact line. This march
//! reads the frame's depth field instead (the live depth prepass) and darkens
//! only the pixels whose surface meets another surface within
//! [`THICKNESS_M`] over a [`LENGTH_M`] ray toward the sun, in [`STEPS`] even
//! steps. It multiplies ambient only: the sun keeps its own shadow map.
//!
//! Owner: render lane. Consumer: the live GTAO upsample (the field is
//! multiplied into the same ambient occlusion texture) and this oracle, whose
//! parity test pins the shader constants.
//!
//! Donor shape: Bevy `bevy_pbr/src/contact_shadows.rs` (linear steps 16,
//! thickness 0.1, length 0.3; read live 2026-10-20, zero bytes vendored).
//! Determinism: pure integer-count loop over floats in a fixed order, no
//! clock, no allocation. Units: metres, UV in 0..1, factor in 0..=1.

/// March steps toward the sun.
pub const STEPS: u32 = 16;
/// Total march length in metres.
pub const LENGTH_M: f32 = 0.3;
/// A surface within this distance in front of the ray counts as an occluder.
pub const THICKNESS_M: f32 = 0.1;
/// Nearest distance that still counts: skips the pixel's own surface.
pub const BIAS_M: f32 = 0.02;
/// How much of ambient the darkest contact keeps (0.55 means 45% darker).
pub const MAX_SHADE: f32 = 0.55;

/// World point as three floats; small local helpers keep this module std-only.
pub type Point = [f32; 3];

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: Point) -> f32 {
    dot(a, a).sqrt()
}

fn normalize(a: Point) -> Option<Point> {
    let len = length(a);
    if !len.is_finite() || len < 1e-12 {
        return None;
    }
    Some([a[0] / len, a[1] / len, a[2] / len])
}

/// One frame's depth field and camera, borrowed for a whole image.
///
/// `project` maps a world point to UV in 0..1 (or `None` when it leaves the
/// picture). `world_at` maps a UV back to the surface the depth buffer holds
/// there (or `None` for sky). The live shader reads the same two functions
/// from the camera matrix and the depth texture.
pub struct ContactFrame<'a> {
    /// Camera position in world metres.
    pub eye: Point,
    /// Unit direction *toward* the sun.
    pub sun: Point,
    /// World point to screen UV in 0..1, `None` off screen.
    pub project: &'a dyn Fn(Point) -> Option<[f32; 2]>,
    /// Screen UV in 0..1 to the surface world point, `None` for sky.
    pub world_at: &'a dyn Fn(f32, f32) -> Option<Point>,
}

/// Ambient factor for one world point: 1 keeps all of it, [`MAX_SHADE`] is
/// the darkest a contact line gets. `strength` 0 turns the pass off and
/// returns exactly 1. Pure and total: a bad sun or zero strength is neutral,
/// never a panic.
pub fn contact_factor(frame: &ContactFrame<'_>, world: Point, strength: f32) -> f32 {
    if strength <= 0.0 {
        return 1.0;
    }
    let Some(sun) = normalize(frame.sun) else {
        return 1.0;
    };
    let Ok(steps) = u16::try_from(STEPS) else {
        return 1.0;
    };
    let step = LENGTH_M / f32::from(steps);
    let mut shade: f32 = 0.0;
    let mut i: u16 = 1;
    while i <= steps {
        let travelled = step * f32::from(i);
        let ray = [
            world[0] + sun[0] * travelled,
            world[1] + sun[1] * travelled,
            world[2] + sun[2] * travelled,
        ];
        let Some(uv) = (frame.project)(ray) else {
            break;
        };
        if let Some(surface) = (frame.world_at)(uv[0], uv[1]) {
            let to_eye = sub(frame.eye, ray);
            let eye_len = length(to_eye);
            if eye_len > 1e-6 {
                let towards_eye = [
                    to_eye[0] / eye_len,
                    to_eye[1] / eye_len,
                    to_eye[2] / eye_len,
                ];
                let front = dot(sub(surface, ray), towards_eye);
                if front > BIAS_M && front < THICKNESS_M {
                    // Nearer occluders read darker; the first hit ends the
                    // march, so a long wall does not stack shade.
                    shade = 1.0 - f32::from(i - 1) / f32::from(steps);
                    break;
                }
            }
        }
        i += 1;
    }
    let factor = 1.0 - shade * (1.0 - MAX_SHADE) * strength;
    factor.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic depth field: a floor at y = 0 plus one axis-aligned box
    /// from `lo` to `hi`, seen through a pinhole at the frame's eye.
    struct Field {
        eye: Point,
        lo: Point,
        hi: Point,
    }

    impl Field {
        fn project(&self, p: Point) -> Option<[f32; 2]> {
            let d = sub(p, self.eye);
            if d[2] >= -1e-4 {
                return None;
            }
            let u = 0.5 + 0.5 * (d[0] / -d[2]) * 1.2;
            let v = 0.5 - 0.5 * (d[1] / -d[2]) * 1.2;
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                return None;
            }
            Some([u, v])
        }

        fn ray(&self, u: f32, v: f32) -> Point {
            // Inverse of `project`: the 1.2 factor is the pinhole's spread.
            let x = (u - 0.5) * 2.0 / 1.2;
            let y = -(v - 0.5) * 2.0 / 1.2;
            let _ = self;
            normalize([x, y, -1.0]).unwrap_or([0.0, 0.0, -1.0])
        }

        fn world_at(&self, u: f32, v: f32) -> Option<Point> {
            let dir = self.ray(u, v);
            let mut best: Option<Point> = None;
            // Floor y = 0, camera above it.
            if dir[1] < -1e-6 {
                let t = -self.eye[1] / dir[1];
                if t > 0.0 {
                    best = Some([
                        self.eye[0] + dir[0] * t,
                        self.eye[1] + dir[1] * t,
                        self.eye[2] + dir[2] * t,
                    ]);
                }
            }
            // Slab test for the box.
            if let Some(t) = slab(self.eye, dir, self.lo, self.hi) {
                let hit = [
                    self.eye[0] + dir[0] * t,
                    self.eye[1] + dir[1] * t,
                    self.eye[2] + dir[2] * t,
                ];
                let closer = match best {
                    Some(b) => length(sub(hit, self.eye)) < length(sub(b, self.eye)),
                    None => true,
                };
                if closer {
                    best = Some(hit);
                }
            }
            best
        }
    }
    /// Nearest positive hit on an axis-aligned box, `None` when missed.
    fn slab(origin: Point, dir: Point, lo: Point, hi: Point) -> Option<f32> {
        let mut t_near = f32::NEG_INFINITY;
        let mut t_far = f32::INFINITY;
        for axis in 0..3 {
            let o = origin[axis];
            let d = dir[axis];
            if d.abs() < 1e-8 {
                if o < lo[axis] || o > hi[axis] {
                    return None;
                }
                continue;
            }
            let mut a = (lo[axis] - o) / d;
            let mut b = (hi[axis] - o) / d;
            if a > b {
                core::mem::swap(&mut a, &mut b);
            }
            t_near = t_near.max(a);
            t_far = t_far.min(b);
        }
        (t_far >= t_near && t_far > 0.0).then_some(t_near.max(0.0))
    }

    fn scene() -> Field {
        Field {
            eye: [0.0, 0.6, 2.4],
            // A thin pillar on the floor, 1 cm right of the test floor point.
            lo: [0.25, 0.0, 0.1],
            hi: [0.35, 0.3, 0.2],
        }
    }

    /// Build the frame and run the march; the closures outlive the struct here.
    fn factor(field: &Field, sun: Point, world: Point, strength: f32) -> f32 {
        let project = |p| field.project(p);
        let world_at = |u, v| field.world_at(u, v);
        let frame = ContactFrame {
            eye: field.eye,
            sun,
            project: &project,
            world_at: &world_at,
        };
        contact_factor(&frame, world, strength)
    }

    #[test]
    fn constants_are_the_donor_contact_shadow_numbers() {
        let steps = std::hint::black_box(STEPS);
        let length = std::hint::black_box(LENGTH_M);
        let thickness = std::hint::black_box(THICKNESS_M);
        let bias = std::hint::black_box(BIAS_M);
        let shade = std::hint::black_box(MAX_SHADE);
        assert_eq!(steps, 16);
        assert!((length - 0.3).abs() < 1e-6);
        assert!((thickness - 0.1).abs() < 1e-6);
        assert!((bias - 0.02).abs() < 1e-6);
        assert!(shade > 0.0 && shade < 1.0);
    }

    #[test]
    fn open_floor_keeps_all_of_its_ambient() {
        let field = scene();
        let sun = normalize([-0.6, 0.8, 0.0]).unwrap_or([0.0, 1.0, 0.0]);
        let floor = [1.2, 0.0, 1.2];
        assert_eq!(factor(&field, sun, floor, 1.0), 1.0);
    }

    #[test]
    fn floor_beside_the_box_darkens_toward_the_contact_line() {
        let field = scene();
        // Sun points along -X, so the ray from a floor point right of the box
        // enters the box's lower side.
        let sun = normalize([-1.0, 0.08, 0.0]).unwrap_or([-1.0, 0.0, 0.0]);
        let beside = [0.34, 0.0, 0.15];
        let factor = factor(&field, sun, beside, 1.0);
        assert!(factor < 0.95, "the contact line darkens, got {factor}");
        assert!(
            factor >= MAX_SHADE,
            "never below the floor shade, got {factor}"
        );
    }

    #[test]
    fn strength_zero_and_a_bad_sun_are_exactly_neutral() {
        let field = scene();
        let beside = [0.34, 0.0, 0.15];
        let sun = normalize([-1.0, 0.08, 0.0]).unwrap_or([-1.0, 0.0, 0.0]);
        assert_eq!(factor(&field, sun, beside, 0.0), 1.0);
        assert_eq!(
            factor(&field, [0.0, 0.0, 0.0], beside, 1.0),
            1.0,
            "a zero sun is neutral, never a NaN"
        );
    }

    #[test]
    fn a_surface_farther_than_the_march_length_does_not_darken() {
        let mut field = scene();
        // Move the pillar out of reach of a 0.3 m ray from the same floor point.
        field.lo = [0.25, 0.0, -1.0];
        field.hi = [0.35, 0.3, -0.9];
        let sun = normalize([-1.0, 0.08, 0.0]).unwrap_or([-1.0, 0.0, 0.0]);
        let beside = [0.34, 0.0, 0.15];
        assert_eq!(factor(&field, sun, beside, 1.0), 1.0);
    }
}

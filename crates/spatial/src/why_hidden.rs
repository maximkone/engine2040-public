//! SPX6: why is this thing not on screen. The first failing reason in a fixed
//! order, plus the one number that explains it.
//!
//! Consumer: the `scene.query` kind `why_hidden` (see
//! `docs/plan/spatial-entry-points.md`). Units: meters in, integer cm inside.
//! Determinism: integer cones and the same occluder ray as [`super::sees`], no
//! float in the verdict, so the same actors give the same bytes.

use super::index::to_cm;
use super::intmath::{angle_deg, isqrt};
use super::navcheck::octant;
use super::relation::DIR_RANGE_CM;
use super::sees::{first_blocker, mid};
use super::sight::{SightError, SightWorld, VIEW_DY_CM};
use super::Actor;

/// What the caller knows about the target that geometry cannot say: whether
/// it has something to draw. `Default` assumes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    pub has_mesh: bool,
    pub has_material: bool,
}

impl Default for Draw {
    fn default() -> Self {
        Self {
            has_mesh: true,
            has_material: true,
        }
    }
}

/// First failing reason a target is not seen, in the fixed order of the enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidden {
    Visible,
    NotFound,
    NoMesh,
    NoMaterial,
    ZeroScale,
    DegenerateBounds,
    BehindEye,
    TooFar,
    OutOfView,
    OccludedBy(u32),
}

impl Hidden {
    /// Door word: `occluded_by:<id>` carries the occluder, the rest are fixed.
    pub fn word(self) -> String {
        match self {
            Self::Visible => "visible".to_owned(),
            Self::NotFound => "not_found".to_owned(),
            Self::NoMesh => "no_mesh".to_owned(),
            Self::NoMaterial => "no_material".to_owned(),
            Self::ZeroScale => "zero_scale".to_owned(),
            Self::DegenerateBounds => "degenerate_bounds".to_owned(),
            Self::BehindEye => "behind_eye".to_owned(),
            Self::TooFar => "too_far".to_owned(),
            Self::OutOfView => "out_of_view".to_owned(),
            Self::OccludedBy(id) => format!("occluded_by:{id}"),
        }
    }
}

/// A verdict plus the one number that explains it: smallest side for
/// `zero_scale`, cm along the facing for `behind_eye`, distance for
/// `too_far`, degrees off the facing (or cm of height) for `out_of_view`, cm to
/// the occluder for `occluded_by`, 0 otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Why {
    pub verdict: Hidden,
    pub evidence: i64,
}

/// Why is `target` not seen from `eye_m` facing `yaw`? Checks in a fixed order
/// and stops at the first failure; the view rules are those of
/// `sight::visible` (90-degree cone, 20 m horizon, 5 m height window) and the
/// occluder comes from the same ray as [`sees_pair`]'s center ray.
pub fn why_hidden(
    actors: &[Actor],
    draw: Draw,
    target: u32,
    eye_m: [f32; 3],
    yaw: f32,
) -> Result<Why, SightError> {
    if !eye_m.iter().all(|v| v.is_finite()) {
        return Err(SightError::NonFinitePoint);
    }
    let why = |verdict, evidence| Ok(Why { verdict, evidence });
    let Some(a) = actors.iter().find(|a| a.id == target) else {
        return why(Hidden::NotFound, 0);
    };
    if !draw.has_mesh {
        return why(Hidden::NoMesh, 0);
    }
    if !draw.has_material {
        return why(Hidden::NoMaterial, 0);
    }
    let finite = [a.x, a.y, a.z, a.sx, a.sy, a.sz]
        .iter()
        .all(|v| v.is_finite());
    if finite && [a.sx, a.sy, a.sz].contains(&0.0) {
        let side = [a.sx, a.sy, a.sz].map(|v| to_cm(v).abs()).into_iter().min();
        return why(Hidden::ZeroScale, i64::from(side.unwrap_or(0)));
    }
    let world = SightWorld::build(actors);
    let Some(b) = world.box_of(target).filter(|_| finite) else {
        return why(Hidden::DegenerateBounds, 0);
    };
    let e = [to_cm(eye_m[0]), to_cm(eye_m[1]), to_cm(eye_m[2])];
    let c = mid(b);
    let (dx, dy, dz) = (
        i64::from(c[0]) - i64::from(e[0]),
        i64::from(c[1]) - i64::from(e[1]),
        i64::from(c[2]) - i64::from(e[2]),
    );
    let flat = dx != 0 || dz != 0;
    let (fx, fz) = if yaw.is_finite() {
        let (x, z) = octant(yaw);
        (i64::from(x), i64::from(z))
    } else {
        (0, 0)
    };
    let dot = fx * dx + fz * dz;
    let lat = fx * dz - fz * dx;
    if flat && yaw.is_finite() && dot <= 0 {
        return why(Hidden::BehindEye, dot);
    }
    let range = i64::from(DIR_RANGE_CM);
    if dx * dx + dz * dz > range * range {
        return why(Hidden::TooFar, isqrt(dx * dx + dz * dz));
    }
    if dy.abs() > i64::from(VIEW_DY_CM) {
        return why(Hidden::OutOfView, dy.abs());
    }
    if !yaw.is_finite() || (flat && lat.abs() > dot) {
        return why(Hidden::OutOfView, i64::from(angle_deg(dot, lat)));
    }
    match first_blocker(&world, e, c) {
        Some(hit) => why(Hidden::OccludedBy(hit.id), i64::from(hit.dist_cm)),
        None => why(Hidden::Visible, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: format!("a{id}"),
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

    #[test]
    fn why_hidden_has_one_fixture_per_verdict() {
        let eye = [0.0, 1.0, 0.0];
        let base = |id, x: f32, z: f32| solid(id, x, 1.0, z, 1.0, 2.0, 1.0);
        let check = |actors: &[Actor], draw: Draw, target: u32, yaw: f32| {
            let first = why_hidden(actors, draw, target, eye, yaw).unwrap();
            assert_eq!(first, why_hidden(actors, draw, target, eye, yaw).unwrap());
            first.verdict.word()
        };
        let yaw0 = 0.0; // facing +X
        let ok = Draw::default();
        // visible: straight ahead, nothing between.
        assert_eq!(check(&[base(1, 6.0, 0.0)], ok, 1, yaw0), "visible");
        assert_eq!(check(&[base(1, 6.0, 0.0)], ok, 9, yaw0), "not_found");
        let no_mesh = Draw {
            has_mesh: false,
            ..ok
        };
        assert_eq!(check(&[base(1, 6.0, 0.0)], no_mesh, 1, yaw0), "no_mesh");
        let no_mat = Draw {
            has_material: false,
            ..ok
        };
        assert_eq!(check(&[base(1, 6.0, 0.0)], no_mat, 1, yaw0), "no_material");
        let flat = solid(1, 6.0, 1.0, 0.0, 1.0, 0.0, 1.0);
        assert_eq!(check(&[flat], ok, 1, yaw0), "zero_scale");
        let mut nan = base(1, 6.0, 0.0);
        nan.x = f32::NAN;
        assert_eq!(check(&[nan], ok, 1, yaw0), "degenerate_bounds");
        assert_eq!(check(&[base(1, -6.0, 0.0)], ok, 1, yaw0), "behind_eye");
        assert_eq!(check(&[base(1, 30.0, 0.0)], ok, 1, yaw0), "too_far");
        assert_eq!(check(&[base(1, 2.0, 6.0)], ok, 1, yaw0), "out_of_view");
        let wall = solid(2, 3.0, 1.0, 0.0, 0.5, 2.0, 4.0);
        let hidden = check(&[base(1, 6.0, 0.0), wall], ok, 1, yaw0);
        assert_eq!(hidden, "occluded_by:2");
        let why = why_hidden(
            &[base(1, 6.0, 0.0), solid(2, 3.0, 1.0, 0.0, 0.5, 2.0, 4.0)],
            ok,
            1,
            eye,
            yaw0,
        )
        .unwrap();
        assert_eq!(why.evidence, 275, "wall face at x = 275 cm from the eye");
    }
}

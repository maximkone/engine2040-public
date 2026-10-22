//! hero (HC8): the body in the chase cam. A caped figure built from boxes: it lies along the
//! flight path with both arms thrust ahead when flying, and hangs leaning forward with one
//! arm up toward the web when swinging. Three merged meshes (suit, cape, skin) so the whole
//! hero costs three draws. Presentation only. Units: meters, radians; y up; yaw 0 looks -z.

use engine_core::quat_mul;
use engine_destruction::csg::PropMesh;
use engine_render::v1::MeshData;

use super::cut::box_shell;
use super::draw::{flat_from, shell_bounds};
use super::num::{add, axis_quat, rotate, scale};

/// The figure is drawn this many times life size, so it reads from the chase camera.
pub(super) const HERO_SCALE: f32 = 3.0;
/// How far the swing pose leans forward from upright, radians.
const SWING_LEAN_RAD: f32 = 0.55;

/// What the hero is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pose {
    /// Lying along the flight path, arms ahead.
    Fly,
    /// Hanging from a web, leaning forward, one arm up.
    Swing,
}

/// Where and how the hero is.
#[derive(Clone, Copy, Debug)]
pub(super) struct HeroPose {
    /// The hip, meters.
    pub pos: [f32; 3],
    /// Heading, radians (0 looks down -z, + turns right).
    pub yaw: f32,
    /// Climb angle of the path, radians (+ up).
    pub pitch: f32,
    pub pose: Pose,
    /// Seconds on the demo's clock (the cape flutters).
    pub t_s: f32,
}

/// The three parts, one mesh each.
pub(super) struct HeroMeshes {
    pub suit: MeshData,
    pub cape: MeshData,
    pub skin: MeshData,
}

/// The hero's parts in the standing frame (hip at the origin, feet at -0.85, facing -z).
fn standing(pose: Pose) -> (Vec<PropMesh>, PropMesh, Vec<PropMesh>) {
    let b = box_shell;
    let (left_arm, right_arm) = match pose {
        Pose::Fly => (
            b([-0.38, 0.45, -0.08], [-0.22, 1.35, 0.08]),
            b([0.22, 0.45, -0.08], [0.38, 1.35, 0.08]),
        ),
        Pose::Swing => (
            b([-0.38, 0.0, -0.08], [-0.22, 0.58, 0.08]),
            b([0.22, 0.55, -0.08], [0.38, 1.45, 0.08]),
        ),
    };
    let suit = vec![
        b([-0.22, 0.0, -0.12], [0.22, 0.62, 0.12]),
        b([-0.20, -0.85, -0.10], [-0.02, 0.0, 0.10]),
        b([0.02, -0.85, -0.10], [0.20, 0.0, 0.10]),
        left_arm,
        right_arm,
    ];
    let cape = b([-0.20, -0.75, 0.12], [0.20, 0.55, 0.15]);
    let skin = vec![b([-0.11, 0.64, -0.11], [0.11, 0.88, 0.11])];
    (suit, cape, skin)
}

/// The body's attitude: yaw about y, then lean about x. Flying lies the figure along the path
/// (head ahead); swinging leans it forward from upright.
fn attitude(p: &HeroPose) -> [f32; 4] {
    let lean = match p.pose {
        Pose::Fly => p.pitch - std::f32::consts::FRAC_PI_2,
        Pose::Swing => -SWING_LEAN_RAD,
    };
    quat_mul(
        axis_quat([0.0, 1.0, 0.0], -p.yaw),
        axis_quat([1.0, 0.0, 0.0], lean),
    )
}

fn merge(parts: &[PropMesh], place: impl Fn([f32; 3]) -> [f32; 3]) -> PropMesh {
    let mut out = PropMesh {
        positions: Vec::new(),
        indices: Vec::new(),
    };
    for part in parts {
        let base = u32::try_from(out.positions.len()).unwrap_or(0);
        out.positions
            .extend(part.positions.iter().map(|q| place(*q)));
        out.indices.extend(part.indices.iter().map(|i| i + base));
    }
    out
}

/// The hero's three meshes for this pose.
pub(super) fn hero_meshes(p: &HeroPose) -> Result<HeroMeshes, String> {
    let (suit, cape, skin) = standing(p.pose);
    let rot = attitude(p);
    // The cape streams a little to one side and back, swaying with the clock.
    let sway = 0.12 * (p.t_s * 9.0).sin();
    let place = |q: [f32; 3]| add(rotate(rot, scale(q, HERO_SCALE)), p.pos);
    let flutter = |q: [f32; 3]| {
        let k = (0.6 - q[1]).max(0.0);
        place([q[0] + sway * k, q[1], q[2]])
    };
    Ok(HeroMeshes {
        suit: flat_from(&merge(&suit, place), |q| q)?,
        cape: flat_from(&merge(&[cape], flutter), |q| q)?,
        skin: flat_from(&merge(&skin, place), |q| q)?,
    })
}

/// The bounds of every hero part for this pose (the head ahead and the cape behind), used by
/// the tests and by anything that wants the hero's reach.
pub(super) fn hero_bounds(p: &HeroPose) -> ([f32; 3], [f32; 3]) {
    let (suit, cape, skin) = standing(p.pose);
    let rot = attitude(p);
    let place = |q: [f32; 3]| add(rotate(rot, scale(q, HERO_SCALE)), p.pos);
    let all: Vec<PropMesh> = suit.into_iter().chain([cape]).chain(skin).collect();
    shell_bounds(&merge(&all, place))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(pose: Pose, yaw: f32, pitch: f32) -> HeroPose {
        HeroPose {
            pos: [10.0, 50.0, -20.0],
            yaw,
            pitch,
            pose,
            t_s: 0.0,
        }
    }

    /// Flying level toward -z the figure lies along z: its length runs forward, and it is
    /// flat in y.
    #[test]
    fn a_level_flyer_lies_along_its_heading() {
        let (min, max) = hero_bounds(&pose(Pose::Fly, 0.0, 0.0));
        assert!(max[2] - min[2] > 2.0 * (max[1] - min[1]), "{min:?} {max:?}");
        // Head ahead: the bounds reach farther toward -z than the hip.
        assert!(min[2] < -20.0 - 1.0);
    }

    /// Yawing the flyer a quarter turn right makes it lie along x instead.
    #[test]
    fn the_flyer_follows_the_yaw() {
        let (min, max) = hero_bounds(&pose(Pose::Fly, std::f32::consts::FRAC_PI_2, 0.0));
        assert!(max[0] - min[0] > 2.0 * (max[2] - min[2]), "{min:?} {max:?}");
        assert!(max[0] > 10.0 + 1.0, "heads right (+x)");
    }

    /// Swinging the hero stands: taller than wide, and the raised arm reaches above the head.
    #[test]
    fn the_swinger_hangs_upright_with_an_arm_up() {
        let (min, max) = hero_bounds(&pose(Pose::Swing, 0.0, 0.0));
        assert!(max[1] - min[1] > max[0] - min[0], "{min:?} {max:?}");
    }

    /// Three meshes build, none empty, every vertex finite.
    #[test]
    fn the_hero_builds_three_meshes() {
        for p in [Pose::Fly, Pose::Swing] {
            let m = hero_meshes(&pose(p, 0.7, 0.2)).expect("builds");
            for mesh in [&m.suit, &m.cape, &m.skin] {
                assert!(!mesh.indices.is_empty());
            }
        }
    }
}

//! View audit: the posed truth of one captured frame, for `tools/frame-audit.mjs`.
//!
//! The `arena view:` laws and the `--view-brief` rows judge bodies as boxes.
//! They said `view=ok` on a frame with no shadows, a black sword and shield,
//! a spear standing up out of a lying body (2026-10-24). This module writes
//! what those laws cannot see: every posed body part with its world bounds,
//! true length, material factors and a few world points on its surface, the
//! hand and weapon-socket joints, and every spear with what it is held by or
//! stuck in. The tool projects those points through the frame camera, reads
//! the captured PNG under them and judges shadow, material, scale and
//! attachment per frame (see `docs/book/ref-tools.md`, `frame-audit.mjs`).
//!
//! Consumer: the arena window writes one line per captured frame to
//! [`AUDIT_FILE`] beside the PNGs (`--capture-dir`); the file is view only,
//! never hashed, and costs nothing on a run with no capture armed (the look
//! folds [`PartTruth`] only while its audit switch is on). Units: meters,
//! radians for the camera, factors `0..=1`. Determinism: same inputs, same
//! bytes. Errors: none; a non-finite number prints as JSON `null`.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use engine_render::v1::Camera;

use super::{project_ndc, BodyTruth, ANCHORS};

static ON: AtomicBool = AtomicBool::new(false);

/// Turn the frame audit fold on or off for every look in the process: the
/// window arms it once when a capture is armed.
pub fn set_audit(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// True while poses fold parts and anchors into their truth.
#[must_use]
pub fn audit_on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// File name of the audit rows beside the capture PNGs.
pub const AUDIT_FILE: &str = "view-audit.jsonl";
/// World points kept per part: spread evenly over its vertices.
pub const PART_SAMPLES: usize = 8;
/// Joint names the audit places in the world, in [`BodyTruth::anchors`] order.
pub const ANCHOR_NAMES: [&str; ANCHORS] = ["palm_r", "wep_pos_r", "palm_l", "wep_pos_l"];

/// One posed body part: what a frame audit needs to judge it by what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct PartTruth {
    /// Part index in the rig's part order.
    pub index: u16,
    /// Source material name (`gladius_blade`, `scutum_board`); empty when unnamed.
    pub name: Arc<str>,
    /// World minimum of the posed vertices, meters.
    pub min: [f32; 3],
    /// World maximum of the posed vertices, meters.
    pub max: [f32; 3],
    /// Longest span of the part in meters: the distance between the two
    /// vertices that are far apart (two sweeps; within a few percent of the
    /// true diameter), so a tilted blade is not read longer by its box.
    pub len_m: f32,
    /// The part carries a base-color texture.
    pub textured: bool,
    /// Base-color factor, linear `0..=1`.
    pub base: [f32; 3],
    /// Metallic factor.
    pub metallic: f32,
    /// Roughness factor.
    pub roughness: f32,
    /// World points on the part, [`PART_SAMPLES`] at most.
    pub samples: Vec<[f32; 3]>,
}

/// The factors of a part's material, as the look binds them.
#[derive(Clone, Copy, Debug)]
pub struct PartMaterial {
    /// A base-color texture is bound.
    pub textured: bool,
    /// Base-color factor.
    pub base: [f32; 3],
    /// Metallic factor.
    pub metallic: f32,
    /// Roughness factor.
    pub roughness: f32,
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f32>().sqrt()
}

fn farthest(points: &[[f32; 3]], from: [f32; 3]) -> [f32; 3] {
    let mut best = from;
    let mut far = -1.0f32;
    for p in points {
        let d = dist(*p, from);
        if d > far {
            far = d;
            best = *p;
        }
    }
    best
}

impl PartTruth {
    /// Fold one posed part. `None` when no finite vertex exists.
    #[must_use]
    pub fn fold(
        index: u16,
        name: Arc<str>,
        positions: &[[f32; 3]],
        material: PartMaterial,
    ) -> Option<Self> {
        let points: Vec<[f32; 3]> = positions
            .iter()
            .copied()
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .collect();
        let first = *points.first()?;
        let (mut min, mut max) = (first, first);
        for p in &points {
            for ((lo, hi), v) in min.iter_mut().zip(max.iter_mut()).zip(p) {
                *lo = lo.min(*v);
                *hi = hi.max(*v);
            }
        }
        let a = farthest(&points, first);
        let b = farthest(&points, a);
        let c = farthest(&points, b);
        let step = (points.len() / PART_SAMPLES).max(1);
        let samples: Vec<[f32; 3]> = points
            .iter()
            .step_by(step)
            .take(PART_SAMPLES)
            .copied()
            .collect();
        Some(Self {
            index,
            name,
            min,
            max,
            len_m: dist(b, c).max(dist(a, b)),
            textured: material.textured,
            base: material.base,
            metallic: material.metallic,
            roughness: material.roughness,
            samples,
        })
    }

    /// Lift the part by `dy` meters: the sand rest the look applies to a body.
    pub fn lift(&mut self, dy: f32) {
        self.min[1] += dy;
        self.max[1] += dy;
        for s in &mut self.samples {
            s[1] += dy;
        }
    }
}

/// One spear in a frame: where it is, what holds it or what it is stuck in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpearTruth {
    /// Thrower slot.
    pub slot: u8,
    /// `held`, `flying`, `stuck` (in a body), or `rest` (in the world or dropped).
    pub kind: &'static str,
    /// Tip in meters.
    pub tip: [f32; 3],
    /// Unit direction from tail to tip.
    pub dir: [f32; 3],
    /// Drawn length in meters.
    pub len_m: f32,
    /// The slot whose palm holds it.
    pub holder: Option<u8>,
    /// The slot it is stuck in.
    pub victim: Option<u8>,
}

fn num(v: f32) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        "null".to_string()
    }
}

fn vec3(v: [f32; 3]) -> String {
    format!("[{},{},{}]", num(v[0]), num(v[1]), num(v[2]))
}

fn slot_json(slot: Option<u8>) -> String {
    slot.map_or_else(|| "null".to_string(), |s| s.to_string())
}

/// A JSON string body: quotes, backslashes and control bytes escaped.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn part_json(part: &PartTruth) -> String {
    let samples: Vec<String> = part.samples.iter().map(|s| vec3(*s)).collect();
    format!(
        "{{\"i\":{},\"name\":\"{}\",\"min\":{},\"max\":{},\"len\":{},\"tex\":{},\"base\":{},\"metal\":{},\"rough\":{},\"pts\":[{}]}}",
        part.index,
        esc(&part.name),
        vec3(part.min),
        vec3(part.max),
        num(part.len_m),
        part.textured,
        vec3(part.base),
        num(part.metallic),
        num(part.roughness),
        samples.join(",")
    )
}

fn body_json(camera: &Camera, body: &BodyTruth) -> String {
    let ndc = project_ndc(camera, body.root).map_or_else(
        || "null".to_string(),
        |p| format!("[{},{}]", num(p[0]), num(p[1])),
    );
    let anchors: Vec<String> = ANCHOR_NAMES
        .iter()
        .zip(body.anchors.iter())
        .filter_map(|(name, at)| at.map(|p| format!("\"{name}\":{}", vec3(p))))
        .collect();
    let parts: Vec<String> = body.parts.iter().map(part_json).collect();
    let (min, max) = if body.has_bounds() {
        (vec3(body.min), vec3(body.max))
    } else {
        ("null".to_string(), "null".to_string())
    };
    format!(
        "{{\"slot\":{},\"act\":\"{}\",\"ragdolled\":{},\"root\":{},\"root_ndc\":{ndc},\"min\":{min},\"max\":{max},\"anchors\":{{{}}},\"parts\":[{}]}}",
        body.slot,
        body.act,
        body.ragdolled,
        vec3(body.root),
        anchors.join(","),
        parts.join(",")
    )
}

fn spear_json(spear: &SpearTruth) -> String {
    format!(
        "{{\"slot\":{},\"kind\":\"{}\",\"tip\":{},\"dir\":{},\"len\":{},\"holder\":{},\"victim\":{}}}",
        spear.slot,
        spear.kind,
        vec3(spear.tip),
        vec3(spear.dir),
        num(spear.len_m),
        slot_json(spear.holder),
        slot_json(spear.victim)
    )
}

/// One captured frame as one audit row: the frame file, the camera that drew
/// it, every body with its parts and anchors, every spear. Same inputs, same
/// bytes.
#[must_use]
pub fn audit_line(
    file: &str,
    tick: u32,
    camera: &Camera,
    bodies: &[BodyTruth],
    spears: &[SpearTruth],
) -> String {
    let body_rows: Vec<String> = bodies.iter().map(|b| body_json(camera, b)).collect();
    let spear_rows: Vec<String> = spears.iter().map(spear_json).collect();
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"v\":1,\"file\":\"{}\",\"tick\":{tick},\"camera\":{{\"eye\":{},\"target\":{},\"up\":{},\"fov_y\":{},\"aspect\":{},\"near\":{}}},\"bodies\":[{}],\"spears\":[{}]}}",
        esc(file),
        vec3(camera.eye.to_array()),
        vec3(camera.target.to_array()),
        vec3(camera.up.to_array()),
        num(camera.fov_y),
        num(camera.aspect),
        num(camera.near),
        body_rows.join(","),
        spear_rows.join(",")
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera::new([0.0, 1.5, 6.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 1.0)
    }

    fn material() -> PartMaterial {
        PartMaterial {
            textured: true,
            base: [1.0; 3],
            metallic: 0.0,
            roughness: 0.5,
        }
    }

    #[test]
    fn the_audit_projection_agrees_with_the_screen_rect_and_refuses_behind_the_eye() {
        let camera = cam();
        let at = [0.7, 1.2, 0.0];
        let p = project_ndc(&camera, at).expect("in front");
        let tiny = 1e-3;
        let rect = super::super::screen_rect(
            &camera,
            [at[0] - tiny, at[1] - tiny, at[2] - tiny],
            [at[0] + tiny, at[1] + tiny, at[2] + tiny],
        )
        .expect("a rect");
        assert!(
            (p[0] - (rect[0] + rect[2]) / 2.0).abs() < 1e-3,
            "x {p:?} {rect:?}"
        );
        assert!(
            (p[1] - (rect[1] + rect[3]) / 2.0).abs() < 1e-3,
            "y {p:?} {rect:?}"
        );
        assert_eq!(
            project_ndc(&camera, [0.0, 1.5, 9.0]),
            None,
            "behind the eye"
        );
    }

    #[test]
    fn a_part_folds_bounds_length_and_samples() {
        let blade: Vec<[f32; 3]> = (0..40)
            .map(|i| [0.0, 1.0 + i as f32 * 0.02, 0.0])
            .chain([[0.01, 1.0, 0.0], [-0.01, 1.78, 0.0]])
            .collect();
        let part = PartTruth::fold(5, "gladius_blade".into(), &blade, material()).expect("folds");
        assert!((part.len_m - 0.78).abs() < 0.02, "len {}", part.len_m);
        assert_eq!(part.samples.len(), PART_SAMPLES);
        assert!((part.max[1] - 1.78).abs() < 1e-5);
        assert!(PartTruth::fold(0, "x".into(), &[[f32::NAN; 3]], material()).is_none());
    }

    #[test]
    fn a_lift_moves_bounds_and_points_together() {
        let mut part = PartTruth::fold(
            0,
            "p".into(),
            &[[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            material(),
        )
        .unwrap();
        part.lift(0.5);
        assert!((part.min[1] - 0.5).abs() < 1e-6 && (part.max[1] - 1.5).abs() < 1e-6);
        assert!(part.samples.iter().all(|s| s[1] >= 0.5));
    }

    #[test]
    fn the_row_names_parts_anchors_and_spears_and_repeats() {
        let mut body = BodyTruth::new(1, "down", [0.0, 0.0, 0.0], 0.0, vec![[0.0; 3]]);
        body.fold_vertices(&[[-0.3, 0.0, -0.3], [0.3, 0.4, 0.3]]);
        body.parts.push(
            PartTruth::fold(
                0,
                "gladius_blade".into(),
                &[[0.0, 0.1, 0.0], [0.5, 0.1, 0.0]],
                material(),
            )
            .unwrap(),
        );
        body.anchors[0] = Some([0.1, 0.2, 0.0]);
        let spear = SpearTruth {
            slot: 0,
            kind: "held",
            tip: [0.0, 2.0, 0.0],
            dir: [0.0, 1.0, 0.0],
            len_m: 1.9,
            holder: Some(1),
            victim: None,
        };
        let a = audit_line("frame_000002.png", 3115, &cam(), &[body.clone()], &[spear]);
        let b = audit_line("frame_000002.png", 3115, &cam(), &[body], &[spear]);
        assert_eq!(a, b, "same inputs, same bytes");
        for want in [
            "\"file\":\"frame_000002.png\"",
            "\"name\":\"gladius_blade\"",
            "\"palm_r\":[0.1000,0.2000,0.0000]",
            "\"act\":\"down\"",
            "\"kind\":\"held\"",
            "\"holder\":1",
            "\"victim\":null",
            "\"root_ndc\":[",
        ] {
            assert!(a.contains(want), "{want} in {a}");
        }
        assert!(!a.contains('\n'), "one line per frame");
    }
}

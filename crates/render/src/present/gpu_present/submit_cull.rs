//! submit_cull: the EG38 live submit order and its sphere cull (RA3).
//!
//! The forward list is built here: `rl32_submit_order` groups draws by
//! pipeline, material and mesh (stable twice), then the tested
//! `sphere_visible_cpu` rule drops every draw whose world bound is outside
//! the frame frustum. The filter is a subsequence of a sorted order, so
//! same-pipeline draws stay adjacent.
//!
//! Units: meters for sphere centres and radii, counts for draws. Determinism:
//! the same list plus the same built view-projection gives the same order
//! twice; no clock, no allocation. Presentation only, never hashed.
use super::presenter::Presenter;
use super::submit::rl32_submit_order;
use crate::present::{DrawList, MAX_DRAWS_PER_FRAME};

/// One frame's staged order plus the byte images the frame uniform takes.
/// `order` is filled to `submitted`; the tail is unused.
pub(super) struct StagedFrame {
    pub(super) cam_bytes: [u8; 80],
    pub(super) light_bytes: [u8; 32],
    pub(super) order: [usize; MAX_DRAWS_PER_FRAME],
    pub(super) submitted: usize,
}

impl StagedFrame {
    /// Submitted draw indices in the pipeline-grouped order the forward and
    /// depth passes draw.
    pub(super) fn order(&self) -> &[usize] {
        &self.order[..self.submitted]
    }
}

/// Per-draw world bounds for one frame, in list order.
pub(super) struct DrawBounds {
    /// `[x, y, z, radius]` of each draw's mesh, in list order.
    pub(super) spheres: [[f32; 4]; MAX_DRAWS_PER_FRAME],
    /// False where the static bound cannot describe the draw: a skinned body
    /// is posed by its joint palette, so the sphere test never culls it.
    pub(super) cullable: [bool; MAX_DRAWS_PER_FRAME],
}

impl Presenter {
    /// Bounds for the frame's draws in list order: the resident mesh's world
    /// sphere and whether it may be culled. A missing mesh reads a zero bound
    /// and the staging loop refuses it right after, as before.
    pub(super) fn frame_draw_bounds(&self, list: &DrawList) -> DrawBounds {
        let mut bounds = DrawBounds {
            spheres: [[0.0; 4]; MAX_DRAWS_PER_FRAME],
            cullable: [false; MAX_DRAWS_PER_FRAME],
        };
        for (slot, item) in list.items.iter().take(MAX_DRAWS_PER_FRAME).enumerate() {
            if let Some(mesh) = self.meshes.get(&item.mesh) {
                bounds.spheres[slot] = mesh.bounds;
                bounds.cullable[slot] = !self.skin_blocks.contains_key(&item.mesh);
            }
        }
        bounds
    }
}

/// Inward frustum planes from one built view-projection: clip-row extraction
/// with DirectX 0..1 depth (near is row 2, far is row 3 minus row 2), each
/// plane normalized. A degenerate or non-finite plane returns six zero planes,
/// which keep every draw: the cull never hides geometry on a bad matrix.
/// Caller: `live_submit_order` below (the frame's built matrix, never rebuilt).
pub(super) fn frustum_planes_from_view_proj(view_proj: glam::Mat4) -> [[f32; 4]; 6] {
    let c = view_proj.to_cols_array();
    let row = |i: usize| [c[i], c[4 + i], c[8 + i], c[12 + i]];
    let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
    let comb = |a: [f32; 4], b: [f32; 4], s: f32| {
        [
            a[0] + b[0] * s,
            a[1] + b[1] * s,
            a[2] + b[2] * s,
            a[3] + b[3] * s,
        ]
    };
    let raw = [
        comb(r3, r0, 1.0),
        comb(r3, r0, -1.0),
        comb(r3, r1, 1.0),
        comb(r3, r1, -1.0),
        r2,
        comb(r3, r2, -1.0),
    ];
    let mut out = [[0.0f32; 4]; 6];
    for (i, p) in raw.iter().enumerate() {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        if l < 1e-9 || !l.is_finite() {
            return [[0.0; 4]; 6];
        }
        out[i] = [p[0] / l, p[1] / l, p[2] / l, p[3] / l];
    }
    out
}

/// The live submit order: `rl32_submit_order` (pipeline groups adjacent,
/// stable twice) then the tested sphere test per draw. Returns the kept
/// indices and their count. A frame whose view-projection yields no finite
/// planes keeps every draw instead of hiding geometry.
/// Caller: `Presenter::stage_frame` and `stage_frame_with_camera`.
pub(super) fn live_submit_order(
    list: &DrawList,
    bounds: &DrawBounds,
    view_proj: glam::Mat4,
) -> ([usize; MAX_DRAWS_PER_FRAME], usize) {
    let planes = frustum_planes_from_view_proj(view_proj);
    let mut buf = [0usize; MAX_DRAWS_PER_FRAME];
    let sorted = rl32_submit_order(list, &mut buf).len();
    let mut kept = 0usize;
    for slot in 0..sorted {
        let draw = buf[slot];
        // A skinned body's static positions cannot bound its posed vertices,
        // so it is never culled by this sphere rule.
        let cullable = bounds.cullable.get(draw).copied().unwrap_or(false);
        let visible = !cullable
            || bounds.spheres.get(draw).is_some_and(|s| {
                crate::draw::sphere_visible_cpu([s[0], s[1], s[2]], s[3], &planes)
            });
        if visible {
            buf[kept] = draw;
            kept += 1;
        }
    }
    (buf, kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::present::{DrawItem, MeshKey, TextureKey};

    fn item(mesh: u64, tex: Option<u64>) -> DrawItem {
        DrawItem {
            mesh: MeshKey::new(mesh, [0x11; 32]),
            material: crate::v1::PbrMaterial {
                base_color: [0.5, 0.5, 0.5],
                metallic: 0.0,
                roughness: 0.5,
                exposure: 1.0,
            },
            base_color: tex.map(|t| TextureKey::new(t, [0x77; 32])),
            normal_map: None,
            orm: None,
            emissive: None,
        }
    }

    fn bounds(spheres: &[[f32; 4]]) -> DrawBounds {
        let mut out = DrawBounds {
            spheres: [[0.0; 4]; MAX_DRAWS_PER_FRAME],
            cullable: [true; MAX_DRAWS_PER_FRAME],
        };
        for (slot, sphere) in spheres.iter().enumerate() {
            out.spheres[slot] = *sphere;
        }
        out
    }

    #[test]
    fn draw_sort_submit_order_is_stable_and_culls_outside_sphere() {
        // Caller: `live_submit_order` (the live list built by `stage_frame`
        // and `stage_frame_with_camera`). Behavior: same order twice, same
        // pipeline adjacent, and a sphere outside the frustum is not
        // submitted (counter 0 in the pair-only case).
        let scene = crate::v1::default_scene(160, 120).expect("scene valid");
        let view_proj = scene.camera.view_proj().expect("camera builds");
        let list = DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: vec![
                item(3, Some(7)),
                item(1, None),
                item(2, Some(7)),
                item(4, Some(9)),
            ],
        };
        let fwd = (scene.camera.target - scene.camera.eye).normalize();
        let mid = (scene.camera.eye + scene.camera.target) * 0.5;
        let behind = scene.camera.eye - fwd * 5.0;
        let all = bounds(&[[mid.x, mid.y, mid.z, 0.5]; 4]);
        let (first, count) = live_submit_order(&list, &all, view_proj);
        let (second, count_again) = live_submit_order(&list, &all, view_proj);
        assert_eq!(count, 4, "every visible draw submits");
        assert_eq!(count, count_again);
        assert_eq!(first[..count], second[..count], "same order twice");
        let i0 = first[..count]
            .iter()
            .position(|&d| d == 0)
            .expect("draw 0 submits");
        let i2 = first[..count]
            .iter()
            .position(|&d| d == 2)
            .expect("draw 2 submits");
        assert_eq!(i0.abs_diff(i2), 1, "same pipeline adjacent: {first:?}");

        let pair = DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: vec![item(1, Some(7)), item(2, Some(7))],
        };
        let mixed = bounds(&[
            [mid.x, mid.y, mid.z, 0.1],
            [behind.x, behind.y, behind.z, 0.1],
        ]);
        let (kept, submitted) = live_submit_order(&pair, &mixed, view_proj);
        assert_eq!(submitted, 1, "one of the pair submits");
        assert_eq!(kept[..submitted], [0], "outside sphere is not submitted");

        let outside = bounds(&[
            [behind.x, behind.y, behind.z, 0.1],
            [behind.x, behind.y, behind.z, 0.1],
        ]);
        let (_, submitted) = live_submit_order(&pair, &outside, view_proj);
        assert_eq!(submitted, 0, "a list fully outside submits nothing");
    }

    #[test]
    fn draw_sort_never_culls_a_skinned_draw() {
        // A skinned body's world pose comes from its joint palette, so its
        // static bound cannot describe it: the flag keeps it submitted.
        let scene = crate::v1::default_scene(160, 120).expect("scene valid");
        let view_proj = scene.camera.view_proj().expect("camera builds");
        let list = DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: vec![item(1, None)],
        };
        let fwd = (scene.camera.target - scene.camera.eye).normalize();
        let behind = scene.camera.eye - fwd * 5.0;
        let mut fixed = bounds(&[[behind.x, behind.y, behind.z, 0.1]]);
        fixed.cullable[0] = false;
        let (kept, submitted) = live_submit_order(&list, &fixed, view_proj);
        assert_eq!(submitted, 1, "an uncullable draw stays submitted");
        assert_eq!(kept[..submitted], [0]);
    }

    /// The text between the first `{` after `fn <name>` and its matching `}`.
    fn fn_body<'a>(src: &'a str, name: &str) -> &'a str {
        let start = src
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("fn {name} exists"));
        let open = start + src[start..].find('{').expect("fn has a body");
        let mut depth = 0usize;
        for (i, b) in src.as_bytes()[open..].iter().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &src[open..=open + i];
                    }
                }
                _ => {}
            }
        }
        panic!("fn {name} body closes");
    }

    #[test]
    fn draw_sort_live_stage_calls_the_sphere_cull() {
        // Source pin: both live stage paths build the order through the cull,
        // and that order calls the tested sphere test.
        for (name, src, stage) in [
            (
                "submit.rs (capture stage)",
                include_str!("submit.rs"),
                "stage_frame",
            ),
            (
                "live.rs (live stage)",
                include_str!("live.rs"),
                "stage_frame_with_camera",
            ),
        ] {
            let body = fn_body(src, stage);
            assert_eq!(
                body.matches("live_submit_order(").count(),
                1,
                "{name} builds the culled order"
            );
            assert_eq!(
                body.matches("frame_draw_bounds(").count(),
                1,
                "{name} reads the mesh bounds"
            );
        }
        let cull = include_str!("submit_cull.rs");
        assert!(
            cull.contains("sphere_visible_cpu"),
            "the live order calls the tested sphere test"
        );
    }
}

//! Debug draw for the live fighters (C13): the hurt capsule the sweep rows
//! already use, plus one forward-facing line per fighter, projected into the
//! after-blit line space `crates/render/src/debug_draw.rs` draws.
//!
//! Consumer: the arena window, after the tick and after the frame camera is
//! final (`ArenaApp::push_debug_lines`); the same presenter serves the window
//! and the offscreen capture, so both see the lines.
//! Units: fighter positions and capsule size in meters, camera aspect as
//! width over height, output in the debug pass's HUD space (y up; one unit is
//! one frame height, x spans `-1..2*aspect-1`).
//! Determinism: presentation only; reads the live views and a camera, never
//! writes sim state and never enters a hash. Off at the `debug-draw` knob,
//! where the builder returns `None` before any allocation, so an off frame is
//! bit-identical to one without the pass.
//!
//! Size owner: [`engine_arena::HURT_HALF_MM`] and [`engine_arena::HURT_RADIUS_MM`]
//! are the same constants the sweep rows fill, so the wireframe cannot drift
//! from the shape that decides hits. App leg only: the camera and the batch
//! types come from the render stack, which the headless default graph omits.

use engine_render::debug_draw::{DebugLine, LineBatch, LineGroup};
use engine_render::v1::Camera;

use crate::FighterView;

/// Group name of the hurt-capsule wireframes.
pub const GROUP_HURT: &str = "hurt";
/// Group name of the facing lines.
pub const GROUP_FACING: &str = "facing";
/// Ring segments around each capsule cap; 45 degrees apart.
pub const RING_SEGMENTS: usize = 8;
/// Lines one fighter stages: two rings, four verticals and one facing line.
pub const LINES_PER_FIGHTER: usize = 2 * RING_SEGMENTS + 4 + 1;
/// Length of the forward-facing line, in meters.
pub const FACING_LINE_M: f32 = 1.0;
/// Linear RGBA of the hurt capsule wireframe (orange-red).
pub const HURT_COLOUR: [f32; 4] = [1.0, 0.35, 0.1, 1.0];
/// Linear RGBA of the facing line (green).
pub const FACING_COLOUR: [f32; 4] = [0.2, 1.0, 0.35, 1.0];

/// Unit ring directions at 45 degree steps, starting at +x.
const RING_DIRS: [[f32; 2]; RING_SEGMENTS] = [
    [1.0, 0.0],
    [
        std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    ],
    [0.0, 1.0],
    [
        -std::f32::consts::FRAC_1_SQRT_2,
        std::f32::consts::FRAC_1_SQRT_2,
    ],
    [-1.0, 0.0],
    [
        -std::f32::consts::FRAC_1_SQRT_2,
        -std::f32::consts::FRAC_1_SQRT_2,
    ],
    [0.0, -1.0],
    [
        std::f32::consts::FRAC_1_SQRT_2,
        -std::f32::consts::FRAC_1_SQRT_2,
    ],
];

/// Smallest clip `w` treated as in front of the camera; a point at or behind
/// the eye is skipped instead of mirrored across the screen.
const MIN_W: f32 = 1e-3;

/// One millimeter row to meters; a capsule value larger than `u16` can only
/// come from an owner bug, and clamping there keeps the wireframe finite.
fn mm_to_m(mm: u32) -> f32 {
    f32::from(u16::try_from(mm).unwrap_or(u16::MAX)) / 1000.0
}

/// The arena's canonical hurt capsule in meters: half-height and radius.
/// Converted from the one millimeter owner so the sizes cannot drift.
#[must_use]
pub fn capsule_m() -> (f32, f32) {
    (
        mm_to_m(engine_arena::HURT_HALF_MM),
        mm_to_m(engine_arena::HURT_RADIUS_MM),
    )
}

/// Build this frame's debug line batch, or `None` while the knob is off.
///
/// `enabled` is the presenter's `debug-draw` switch: off returns before any
/// projection or allocation. On stages two groups, [`GROUP_HURT`] with
/// [`LINES_PER_FIGHTER`] minus one line per fighter and [`GROUP_FACING`] with
/// the facing line; a point at or behind the camera is dropped, and an
/// invalid camera or a refused line yields the lines built so far.
#[must_use]
pub fn fighter_lines(views: &[FighterView], camera: &Camera, enabled: bool) -> Option<LineBatch> {
    if !enabled {
        return None;
    }
    let view_proj = camera.view_proj().ok()?.to_cols_array();
    let aspect = camera.aspect;
    let (half_m, radius_m) = capsule_m();
    let mut hurt = LineGroup::new(GROUP_HURT, 0);
    let mut facing = LineGroup::new(GROUP_FACING, 0);
    for view in views {
        push_capsule(&mut hurt, view, half_m, radius_m, &view_proj, aspect);
        push_facing(&mut facing, view, half_m, &view_proj, aspect);
    }
    LineBatch::build(&[hurt, facing]).ok()
}

/// Push the capsule wireframe for one fighter: a ring at each cap, four
/// verticals between them, every point at the arena radius around the spine.
fn push_capsule(
    group: &mut LineGroup,
    view: &FighterView,
    half_m: f32,
    radius_m: f32,
    view_proj: &[f32; 16],
    aspect: f32,
) {
    let [x, y, z] = view.pos_m;
    let ring = |dir: [f32; 2], height: f32| [x + dir[0] * radius_m, height, z + dir[1] * radius_m];
    let (bottom, top) = (y - half_m, y + half_m);
    for index in 0..RING_SEGMENTS {
        let a = RING_DIRS[index];
        let b = RING_DIRS[(index + 1) % RING_SEGMENTS];
        push_line(
            group,
            ring(a, bottom),
            ring(b, bottom),
            HURT_COLOUR,
            view_proj,
            aspect,
        );
        push_line(
            group,
            ring(a, top),
            ring(b, top),
            HURT_COLOUR,
            view_proj,
            aspect,
        );
    }
    for dir in [RING_DIRS[0], RING_DIRS[2], RING_DIRS[4], RING_DIRS[6]] {
        push_line(
            group,
            ring(dir, bottom),
            ring(dir, top),
            HURT_COLOUR,
            view_proj,
            aspect,
        );
    }
}

/// Push the forward-facing line for one fighter: from the top of the capsule
/// along the body forward `(sin yaw, 0, cos yaw)`, the same convention the
/// fighter look and first-person view use.
fn push_facing(
    group: &mut LineGroup,
    view: &FighterView,
    half_m: f32,
    view_proj: &[f32; 16],
    aspect: f32,
) {
    let [x, y, z] = view.pos_m;
    let (sin_yaw, cos_yaw) = view.yaw.sin_cos();
    let start = [x, y + half_m, z];
    let end = [
        x + sin_yaw * FACING_LINE_M,
        y + half_m,
        z + cos_yaw * FACING_LINE_M,
    ];
    push_line(group, start, end, FACING_COLOUR, view_proj, aspect);
}

/// Project and push one world-space segment; a segment with either endpoint
/// at or behind the camera stages nothing.
fn push_line(
    group: &mut LineGroup,
    a_m: [f32; 3],
    b_m: [f32; 3],
    colour_rgba: [f32; 4],
    view_proj: &[f32; 16],
    aspect: f32,
) {
    let (Some(a_m), Some(b_m)) = (
        project(view_proj, aspect, a_m),
        project(view_proj, aspect, b_m),
    ) else {
        return;
    };
    // Both endpoints are finite (the camera validated and the view is finite),
    // so the group's finite check cannot refuse here.
    let _ = group.push(DebugLine {
        a_m,
        b_m,
        colour_rgba,
    });
}

/// One world point to the after-blit line space: the camera's clip-space
/// point divided by `w`, then the overlay ortho inverted in x.
///
/// The overlay draws quad x in `0..1` through `ndc_x = 2/aspect * x - 1`
/// (`overlay_mvp`), and the line space halves that to `0..1` again, so a
/// world point's HUD x is `aspect * ndc_x + aspect - 1`; HUD y is `ndc_y`
/// (y up). `None` behind the camera.
fn project(view_proj: &[f32; 16], aspect: f32, p: [f32; 3]) -> Option<[f32; 3]> {
    let x = view_proj[0] * p[0] + view_proj[4] * p[1] + view_proj[8] * p[2] + view_proj[12];
    let y = view_proj[1] * p[0] + view_proj[5] * p[1] + view_proj[9] * p[2] + view_proj[13];
    let w = view_proj[3] * p[0] + view_proj[7] * p[1] + view_proj[11] * p[2] + view_proj[15];
    if !(w > MIN_W) {
        return None;
    }
    let inv = 1.0 / w;
    Some([aspect * x * inv + aspect - 1.0, y * inv, 0.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        let mut camera = Camera::new(
            [0.0, 1.6, -6.0],
            [0.0, 0.8, 0.0],
            [0.0, 1.0, 0.0],
            60.0f32.to_radians(),
        );
        camera.aspect = 16.0 / 9.0;
        camera
    }

    fn view(slot: u8, x: f32, yaw: f32) -> FighterView {
        FighterView {
            slot,
            pos_m: [x, 0.0, 0.0],
            yaw,
        }
    }

    fn verts(batch: &LineBatch, group: &str) -> Vec<[f32; 3]> {
        let span = batch
            .groups()
            .iter()
            .find(|row| row.name == group)
            .expect("group staged");
        batch.verts()[span.first_vert..span.first_vert + span.vert_count]
            .iter()
            .map(|(p, _)| *p)
            .collect()
    }

    #[test]
    fn debug_draw_off_builds_nothing() {
        // The knob off returns before any projection or allocation, so the
        // off frame never stages a line.
        assert!(fighter_lines(&[view(0, 0.0, 0.0)], &camera(), false).is_none());
        assert!(fighter_lines(&[], &camera(), false).is_none());
    }

    #[test]
    fn debug_draw_two_fighters_stage_42_lines_in_two_groups() {
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 3.0)];
        let batch = fighter_lines(&views, &camera(), true).expect("on stages a batch");
        assert_eq!(batch.line_count(), 2 * LINES_PER_FIGHTER);
        assert_eq!(batch.line_count(), 42);
        let names: Vec<_> = batch.groups().iter().map(|g| g.name).collect();
        assert_eq!(names, vec![GROUP_HURT, GROUP_FACING]);
        assert_eq!(batch.groups()[0].line_count(), 40);
        assert_eq!(batch.groups()[1].line_count(), 2);
        // Both fighters stand in front of the camera, so every point is
        // finite and lands inside the frame's HUD span.
        let aspect = camera().aspect;
        for (p, _) in batch.verts() {
            assert!(p[0].is_finite() && p[1].is_finite(), "finite point {p:?}");
            assert!(
                (-1.0..=2.0 * aspect - 1.0).contains(&p[0]),
                "x {} inside the line space",
                p[0]
            );
            assert!((-1.0..=1.0).contains(&p[1]), "y {} inside", p[1]);
        }
        // Same inputs, same bytes twice.
        let again = fighter_lines(&views, &camera(), true).expect("rerun");
        assert_eq!(batch, again);
    }

    #[test]
    fn debug_draw_capsule_matches_the_arena_sweep_size() {
        // The vertical at +x runs between the arena's own half-height and
        // radius, so the wireframe cannot drift from the sweep shape.
        let (half_m, radius_m) = capsule_m();
        let camera = camera();
        let cols = camera.view_proj().expect("valid camera").to_cols_array();
        let batch = fighter_lines(&[view(0, 0.0, 0.0)], &camera, true).expect("batch");
        let hurt = verts(&batch, GROUP_HURT);
        assert_eq!(hurt.len(), 2 * (LINES_PER_FIGHTER - 1));
        let bottom = project(&cols, camera.aspect, [radius_m, -half_m, 0.0]).expect("in front");
        let top = project(&cols, camera.aspect, [radius_m, half_m, 0.0]).expect("in front");
        assert!(hurt.contains(&bottom), "capsule bottom at -half-height");
        assert!(hurt.contains(&top), "capsule top at +half-height");
        // The top ring leaves and reaches the point once each and the +x
        // vertical starts there, so three verts sit exactly on the top cap.
        assert_eq!(hurt.iter().filter(|p| **p == top).count(), 3);
    }

    #[test]
    fn debug_draw_facing_line_follows_the_yaw() {
        let camera = camera();
        let (half_m, _) = capsule_m();
        let cols = camera.view_proj().expect("valid camera").to_cols_array();
        let batch = fighter_lines(&[view(0, 0.0, 0.0)], &camera, true).expect("batch");
        let facing = verts(&batch, GROUP_FACING);
        assert_eq!(facing.len(), 2, "one line per fighter");
        let start = project(&cols, camera.aspect, [0.0, half_m, 0.0]).expect("top");
        let end = project(&cols, camera.aspect, [0.0, half_m, FACING_LINE_M]).expect("end");
        assert_eq!(facing, vec![start, end], "yaw 0 faces +z");
        // A quarter turn points the line along +x instead.
        let turned = fighter_lines(&[view(0, 0.0, std::f32::consts::FRAC_PI_2)], &camera, true)
            .expect("turned batch");
        let facing = verts(&turned, GROUP_FACING);
        let end = project(&cols, camera.aspect, [FACING_LINE_M, half_m, 0.0]).expect("turned end");
        assert!(
            (facing[1][0] - end[0]).abs() < 1e-5,
            "facing turns with yaw"
        );
        assert!((facing[1][1] - end[1]).abs() < 1e-5, "same height");
    }

    #[test]
    fn debug_draw_skips_points_behind_the_camera() {
        // Eye at -6 facing further -z: the fighter at the origin is behind
        // it, so no segment stages and the batch stays empty (not a mirror).
        let mut away = Camera::new(
            [0.0, 1.6, -6.0],
            [0.0, 1.6, -12.0],
            [0.0, 1.0, 0.0],
            60.0f32.to_radians(),
        );
        away.aspect = 16.0 / 9.0;
        let batch = fighter_lines(&[view(0, 0.0, 0.0)], &away, true).expect("on");
        assert!(batch.is_empty(), "behind-camera points stage no line");
    }
}

/// GPU leg: the staged lines reach the after-blit pass of the offscreen twin
/// and of the live pixel path, and an off frame stays byte-identical. Skips
/// loudly with no adapter, so CI without a GPU stays green.
#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::windowed::{hud_draw_list, SMOKE_FRAME_SIZE};
    use engine_render::pass_truth::LookToggles;
    use engine_render::present::{DrawItem, MeshKey, PresentError, Presenter};
    use engine_render::test_gpu::{require_or_skip, NoAdapter};

    /// Frame width over height for the smoke size, without a numeric cast.
    fn frame_aspect() -> f32 {
        let px = |value: u32| f32::from(u16::try_from(value).unwrap_or(u16::MAX));
        px(SMOKE_FRAME_SIZE.0) / px(SMOKE_FRAME_SIZE.1)
    }

    /// One offscreen presenter, or a routed skip when this machine has no
    /// adapter (`E2040_REQUIRE_GPU=1` turns the skip into a failure).
    fn presenter_or_skip(test: &str) -> Option<Presenter> {
        let probe = Presenter::init_offscreen().map_err(|err| match err {
            PresentError::NoAdapter => NoAdapter("no wgpu adapter on this machine".into()),
            other => panic!("offscreen presenter: {other}"),
        });
        require_or_skip(test, probe)
    }

    #[test]
    fn debug_draw_lines_reach_both_twin_paths_in_pixels() {
        // The window paints through the same `Presenter` as the capture, so a
        // line that moves offscreen pixels moves window pixels too. Off must
        // be byte-identical across two renders, on must move pixels, and the
        // pair lands under `target/c13/` for an eyes-on read.
        let Some(mut presenter) =
            presenter_or_skip("debug_draw_lines_reach_both_twin_paths_in_pixels")
        else {
            return;
        };
        presenter
            .resize(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .expect("frame size");
        let key = MeshKey::new(1, [0xA1; 32]);
        let _ = presenter.upload_mesh(key, &engine_render::v1::cube());

        // Live fighters: the same views the arena tick exposes to the window.
        let mut app = crate::App::new(crate::GameConfig::default()).expect("session");
        app.tick_empty();
        let views = app.fighter_views().to_vec();
        assert_eq!(views.len(), 2, "two live fighters at tick 1");

        let mut list = hud_draw_list(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1).expect("hud list");
        let scene = engine_render::v1::default_scene(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .expect("scene");
        list.items.push(DrawItem {
            mesh: key,
            material: scene.material,
            base_color: None,
            normal_map: None,
            orm: None,
            emissive: None,
        });
        // Frame the near fighter from the side of the pair at four meters: its
        // capsule fills a third of the frame height, while the far fighter
        // stays in front of the camera so both batches stage.
        let (a, b) = (views[0].pos_m, views[1].pos_m);
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let span = (dx * dx + dz * dz).sqrt();
        let (px, pz) = if span > 0.5 {
            (-dz / span, dx / span)
        } else {
            (1.0, 0.0)
        };
        let mut camera = Camera::new(
            [a[0] + px * 4.0, a[1] + 1.2, a[2] + pz * 4.0],
            [a[0], a[1] + 0.8, a[2]],
            [0.0, 1.0, 0.0],
            60.0f32.to_radians(),
        );
        camera.aspect = frame_aspect();
        list.camera = camera;

        presenter.debug_line_enable(false);
        let off = presenter
            .render_offscreen(&list)
            .expect("off frame renders");
        let off_png = off.png.expect("off frame has bytes");
        let off_again = presenter
            .render_offscreen(&list)
            .expect("second off frame renders");
        assert_eq!(
            off_again.png.expect("second off has bytes"),
            off_png,
            "an off frame is byte-identical to the next one"
        );

        presenter.debug_line_enable(true);
        assert!(presenter.debug_lines_enabled());
        let batch = fighter_lines(&views, &list.camera, true).expect("knob on builds the batch");
        assert_eq!(
            batch.line_count(),
            2 * LINES_PER_FIGHTER,
            "two fighters, one capsule and facing line each"
        );
        presenter.push_debug_lines(&batch);
        let on = presenter.render_offscreen(&list).expect("on frame renders");
        let on_png = on.png.expect("on frame has bytes");
        assert_ne!(on_png, off_png, "the staged lines move the PNG bytes");

        // The pair lands before the live leg's assert, so a red still leaves
        // the pictures for an eyes-on read.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/c13");
        std::fs::create_dir_all(&dir).expect("target/c13");
        std::fs::write(dir.join("debug_lines_off.png"), &off_png).expect("off png writes");
        std::fs::write(dir.join("debug_lines_on.png"), &on_png).expect("on png writes");

        // The live pixel path draws the same batch: switch off, then on.
        presenter.debug_line_enable(false);
        let off_px = presenter
            .capture_live_pixels(&list, &LookToggles::all_on())
            .expect("off live pixels");
        assert!(
            presenter.debug_lines().line_count() > 0,
            "batch stays staged"
        );
        presenter.debug_line_enable(true);
        let on_px = presenter
            .capture_live_pixels(&list, &LookToggles::all_on())
            .expect("on live pixels");
        assert_eq!(
            (off_px.width, off_px.height),
            (on_px.width, on_px.height),
            "same frame size on both legs"
        );
        let changed = off_px
            .rgba
            .chunks_exact(4)
            .zip(on_px.rgba.chunks_exact(4))
            .filter(|(off, on)| (0..3).any(|c| off[c].abs_diff(on[c]) > 2))
            .count();
        let any = off_px
            .rgba
            .chunks_exact(4)
            .zip(on_px.rgba.chunks_exact(4))
            .filter(|(off, on)| *off != *on)
            .count();
        let lit = on_px
            .rgba
            .chunks_exact(4)
            .filter(|px| u32::from(px[0]) + u32::from(px[1]) + u32::from(px[2]) > 600)
            .count();
        eprintln!(
            "C13 debug draw: {changed} px differ on vs off, {any} any-byte, {lit} bright ({} lines, {}x{})",
            batch.line_count(),
            off_px.width,
            off_px.height
        );
        std::fs::write(dir.join("debug_lines_live_off.rgba"), &off_px.rgba).expect("off raw");
        std::fs::write(dir.join("debug_lines_live_on.rgba"), &on_px.rgba).expect("on raw");
        assert!(
            changed > 100,
            "the debug lines light {changed} pixels, want over 100"
        );
    }
}

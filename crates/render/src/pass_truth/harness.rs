//! Device leg of the EG8 live-frame arm: render a draw list through the
//! window's own frame path with every look pass on and then with each pass off,
//! read the pixels back and gate them through [`super::live_gate`].
//!
//! Units: RGBA8 pixels, one draw id per pixel. Determinism: a repeatable
//! device frame (the harness renders the all-on frame twice and refuses to
//! gate one that differs from itself), so the same list gives the same rows
//! twice. Cost: ten blocking frames and readbacks per scene (one extra for the
//! repeat check), cold path only, never in a window run.
//! Errors: [`PresentError`]; an unrepeatable frame is a loud error, never a
//! guessed verdict.

use super::live_gate::{gate_live_pair, live_gate_report, LivePair, LiveRow};
use super::{frame_diff_count, LookToggles, LOOK_PASSES};
use crate::present::{DrawList, GpuTimingReport, LivePixels, PresentError, Presenter};

/// The gated scene: one row per look pass plus the frame facts behind them.
#[derive(Debug)]
pub struct LiveGate {
    /// One row per look pass in registry order.
    pub rows: Vec<LiveRow>,
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
    /// The all-on frame diffed against a second all-on frame, in pixels.
    pub noise_px: usize,
}

impl LiveGate {
    /// The row of one look pass; `None` for an unknown name.
    #[must_use]
    pub fn row(&self, name: &str) -> Option<&LiveRow> {
        self.rows.iter().find(|row| row.verdict.row == name)
    }

    /// The dated report bytes for this gate run. Same run gives the same
    /// bytes twice.
    #[must_use]
    pub fn report(&self, date: &str, scene: &str) -> String {
        live_gate_report(
            date,
            scene,
            (self.width, self.height),
            self.noise_px,
            &self.rows,
        )
    }
}

/// Frames rendered before a cost run starts counting, so target and pipeline
/// builds never read as pass cost.
const COST_WARMUP_FRAMES: usize = 6;

/// Device cost of the live frame under one toggle set: `frames` frames are
/// rendered after a warm-up, the device is settled after each so every
/// frame's timestamps land, and the report covers those frames alone. `None`
/// when the adapter has no timestamp queries. A measurement, never a gate:
/// the numbers vary from run to run, so no report table carries them.
/// Errors: a frame fails to render or the device cannot settle.
/// Caller: the A/B cost proofs beside the gate (sky, IBL).
pub fn live_gpu_cost(
    presenter: &mut Presenter,
    list: &DrawList,
    toggles: &LookToggles,
    frames: usize,
) -> Result<Option<GpuTimingReport>, PresentError> {
    if !presenter.gpu_timing_supported() {
        return Ok(None);
    }
    for _ in 0..COST_WARMUP_FRAMES {
        presenter.render_live_offscreen_with_toggles(list, toggles)?;
        presenter.settle_gpu_timing()?;
    }
    presenter.reset_gpu_timing();
    for _ in 0..frames {
        presenter.render_live_offscreen_with_toggles(list, toggles)?;
        presenter.settle_gpu_timing()?;
    }
    Ok(presenter.gpu_timing_report())
}

/// Frame size in pixels as `usize` pair, or a typed error when it cannot fit.
fn frame_size(pixels: &LivePixels) -> Result<(usize, usize), PresentError> {
    match (
        usize::try_from(pixels.width),
        usize::try_from(pixels.height),
    ) {
        (Ok(width), Ok(height)) => Ok((width, height)),
        _ => Err(PresentError::Readback("pass truth frame size".into())),
    }
}

/// Render `list` with every look pass on (twice, to prove the frame repeats)
/// and then once per look pass with that pass off, and gate each pair on the
/// pixels the pass paints. The list is staged, drawn and read back through
/// `capture_live_pixels`, so the verdicts describe the frame the window draws.
/// Errors: the frame fails to render, or the all-on frame differs from itself.
/// Caller: the EG8 gate tests and the arena report.
pub fn gate_live_scene(
    presenter: &mut Presenter,
    list: &DrawList,
) -> Result<LiveGate, PresentError> {
    let all_on = LookToggles::all_on();
    let on = presenter.capture_live_pixels(list, &all_on)?;
    let again = presenter.capture_live_pixels(list, &all_on)?;
    let (width, height) = frame_size(&on)?;
    let noise_px = frame_diff_count(&on.rgba, &again.rgba, width, height, None, 0)
        .map_err(|e| PresentError::Readback(format!("pass truth repeat: {e}")))?;
    if noise_px != 0 || on.ids != again.ids {
        return Err(PresentError::Readback(format!(
            "pass truth: the all-on frame differs from itself in {noise_px} px"
        )));
    }
    let mut rows = Vec::with_capacity(LOOK_PASSES.len());
    for spec in LOOK_PASSES.iter() {
        let mut toggles = LookToggles::all_on();
        toggles.disable(spec.row);
        let off = presenter.capture_live_pixels(list, &toggles)?;
        if frame_size(&off)? != (width, height) {
            return Err(PresentError::Readback(
                "pass truth: the off frame changed size".into(),
            ));
        }
        rows.push(gate_live_pair(&LivePair {
            row: spec.row,
            on: &on.rgba,
            off: &off.rgba,
            ids: &on.ids,
            width,
            height,
        }));
    }
    Ok(LiveGate {
        rows,
        width,
        height,
        noise_px,
    })
}

#[cfg(test)]
/// Test support for the live-frame gate: an arena-like scene built from
/// primitives (a sand floor, a stone wall closing the pit with sky above it, a
/// bare-metal armor pillar in the open, the arena day sun) small enough to
/// gate in a unit test, plus the adapter-or-skip opener every device proof
/// shares. Test-only; the shapes mirror what the arena kit draws.
pub(crate) mod fixtures {
    use crate::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter};
    use crate::test_gpu::{require_or_skip, NoAdapter};
    use crate::v1::{self, Camera, DirectionalLight, MeshData, PbrMaterial};

    /// Frame size of the gated scene: wide enough for sky, wall and sand.
    pub(crate) const W: u32 = 320;
    pub(crate) const H: u32 = 240;

    pub(crate) fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    /// An axis-aligned box from the unit cube: same winding, scaled and moved.
    pub(crate) fn box_mesh(min: [f32; 3], max: [f32; 3]) -> MeshData {
        let cube = v1::cube();
        let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
        let mid = [
            0.5 * (min[0] + max[0]),
            0.5 * (min[1] + max[1]),
            0.5 * (min[2] + max[2]),
        ];
        let positions: Vec<[f32; 3]> = cube
            .positions
            .iter()
            .map(|p| {
                [
                    p[0] * size[0] + mid[0],
                    p[1] * size[1] + mid[1],
                    p[2] * size[2] + mid[2],
                ]
            })
            .collect();
        v1::upload_mesh(&positions, &cube.normals, &cube.uvs, &cube.indices)
            .expect("scaled cube is a valid mesh")
    }

    /// A square floor at y = 0, up-facing.
    pub(crate) fn floor_mesh(half: f32) -> MeshData {
        let positions = [
            [-half, 0.0, -half],
            [-half, 0.0, half],
            [half, 0.0, half],
            [half, 0.0, -half],
        ];
        let normals = [[0.0, 1.0, 0.0]; 4];
        let uvs = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        v1::upload_mesh(&positions, &normals, &uvs, &[0, 1, 2, 0, 2, 3])
            .expect("floor quad is a valid mesh")
    }

    /// Draw indices in the arena-like scene (the wall is draw 1): they are the
    /// draw ids the region proofs name.
    pub(crate) const FLOOR_DRAW: u32 = 0;
    pub(crate) const WALL_DRAW: u32 = 1;
    pub(crate) const ARMOR_DRAW: u32 = 2;

    /// An arena-like frame: a sand floor, a stone wall closing the pit with
    /// sky above it, and a bare-metal armor pillar in the open, under the
    /// arena day sun. The same shapes the arena kit draws, small enough to
    /// gate in a unit test.
    pub(crate) fn arena_like_scene(presenter: &mut Presenter, width: u32, height: u32) -> DrawList {
        let sun = crate::arena_day::ARENA_DAY_SUN_DIR;
        let parts: [(u64, MeshData, [f32; 3], f32, f32); 3] = [
            (0xE801, floor_mesh(30.0), [0.42, 0.32, 0.20], 0.0, 0.9),
            (
                0xE802,
                box_mesh([8.0, 0.0, -14.0], [8.6, 3.5, 14.0]),
                [0.55, 0.50, 0.45],
                0.0,
                0.8,
            ),
            (
                0xE803,
                box_mesh([-0.5, 0.0, -0.3], [0.5, 1.9, 0.3]),
                [0.78, 0.74, 0.70],
                1.0,
                0.3,
            ),
        ];
        let mut items = Vec::with_capacity(parts.len());
        for (id, mesh, base_color, metallic, roughness) in parts {
            let key = MeshKey::new(id, [0xE8; 32]);
            presenter
                .upload_mesh(key, &mesh)
                .expect("scene mesh uploads");
            items.push(DrawItem {
                mesh: key,
                material: PbrMaterial {
                    base_color,
                    metallic,
                    roughness,
                    exposure: 1.0,
                },
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            });
        }
        let mut camera = Camera::new(
            [-7.0, 1.7, 3.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        camera.aspect = width as f32 / height as f32;
        DrawList {
            camera,
            light: DirectionalLight {
                direction: glam::Vec3::from_array(sun),
                color: glam::Vec3::new(1.0, 0.96, 0.90),
                intensity: 3.0,
            },
            items,
        }
    }

    /// Draw indices of the EG11b sun fixture: the sand floor is draw 0 and
    /// the one box is draw 1 (the ids the shadow proofs name).
    pub(crate) const SUN_FLOOR_DRAW: u32 = 0;
    pub(crate) const SUN_BOX_DRAW: u32 = 1;

    /// EG11b fixture: one 1.6 m box standing on open sand under the arena
    /// day sun, seen from high enough that its cast shadow lands in plain
    /// view beside the box, so a pixel can be attributed to the shadow
    /// without guessing.
    pub(crate) fn sun_shadow_scene(presenter: &mut Presenter, width: u32, height: u32) -> DrawList {
        let sun = crate::arena_day::ARENA_DAY_SUN_DIR;
        let parts: [(u64, MeshData, [f32; 3], f32, f32); 2] = [
            (0xEB01, floor_mesh(12.0), [0.42, 0.32, 0.20], 0.0, 0.9),
            (
                0xEB02,
                box_mesh([0.0, 0.0, 0.0], [1.0, 1.6, 1.0]),
                [0.78, 0.74, 0.70],
                0.0,
                0.5,
            ),
        ];
        let mut items = Vec::with_capacity(parts.len());
        for (id, mesh, base_color, metallic, roughness) in parts {
            let key = MeshKey::new(id, [0xEB; 32]);
            presenter
                .upload_mesh(key, &mesh)
                .expect("sun fixture mesh uploads");
            items.push(DrawItem {
                mesh: key,
                material: PbrMaterial {
                    base_color,
                    metallic,
                    roughness,
                    exposure: 1.0,
                },
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            });
        }
        // The sun throws the box shadow toward +x and -z by about 1.13 and
        // 0.82 m per meter of height (`ARENA_DAY_SUN_DIR`), so the camera
        // looks down at that side of the box.
        let mut camera = Camera::new(
            [-4.5, 5.5, -5.0],
            [1.2, 0.2, -0.1],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        camera.aspect = width as f32 / height as f32;
        DrawList {
            camera,
            light: DirectionalLight {
                direction: glam::Vec3::from_array(sun),
                color: glam::Vec3::new(1.0, 0.96, 0.90),
                intensity: 3.0,
            },
            items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pass_truth::harness::fixtures::{
        arena_like_scene, init_or_skip, sun_shadow_scene, ARMOR_DRAW, FLOOR_DRAW, H, SUN_BOX_DRAW,
        SUN_FLOOR_DRAW, W, WALL_DRAW,
    };
    use crate::pass_truth::live_gate::{diff_in_draws, NO_DRAW_ID};
    use crate::pass_truth::{LookToggles, PassRegion, PASS_EPSILON, PASS_LIVE_FLOOR};

    /// The row of one look pass by name.
    fn row<'a>(gate: &'a LiveGate, name: &str) -> &'a LiveRow {
        gate.row(name).expect("every look pass has a row")
    }

    /// The repository root, where `target/reports` lives.
    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// One live frame of `list` under a toggle set: LDR pixels plus draw ids.
    fn capture(presenter: &mut Presenter, list: &DrawList, off: Option<&str>) -> LivePixels {
        let mut toggles = LookToggles::all_on();
        if let Some(name) = off {
            toggles.disable(name);
        }
        presenter
            .capture_live_pixels(list, &toggles)
            .expect("live frame captures")
    }

    /// Pixels where the draw ids say `draws` cover, differing above epsilon.
    fn surface_diff(on: &LivePixels, off: &LivePixels, draws: &[u32]) -> usize {
        diff_in_draws(
            &on.rgba,
            &off.rgba,
            &on.ids,
            draws,
            on.width as usize,
            on.height as usize,
        )
        .expect("frames line up")
    }

    #[test]
    fn the_gate_reads_pixels_for_every_look_pass_and_repeats() {
        // Caller: `gate_live_scene`. Behavior: the window's live frame is
        // rendered with each look pass on and off; a pass reads LIVE only when
        // the pixels in its own region moved, the run repeats, and the dated
        // artifact lists every pass. A switch that did nothing reads PROBE.
        let Some(mut presenter) = init_or_skip("the_gate_reads_pixels_for_every_look_pass") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let first = gate_live_scene(&mut presenter, &list).expect("scene gates");
        let second = gate_live_scene(&mut presenter, &list).expect("scene gates again");
        assert_eq!(first.rows, second.rows, "the gate run twice, same table");
        assert_eq!(first.noise_px, 0, "the all-on frame repeats exactly");
        for name in ["graph", "shadow_cluster", "ibl", "sky", "gtao"] {
            let r = row(&first, name);
            assert!(r.verdict.live, "{name} moves pixels in its region: {r:?}");
            assert!(r.verdict.diff_pixels > 0 && r.counted_px > 0);
        }
        for name in ["post", "material_layer", "particles_gpu", "decals", "spot"] {
            let r = row(&first, name);
            assert!(!r.verdict.live, "{name} paints nothing here: {r:?}");
            assert_eq!(r.verdict.diff_pixels, 0, "{name} frames are equal");
        }
        // Every pixel is sky or covered: the two regions split the frame.
        assert_eq!(
            row(&first, "sky").counted_px + row(&first, "ibl").counted_px,
            (W * H) as usize
        );
        // The sky is the open part above the wall, all of it re-drawn.
        assert_eq!(
            row(&first, "sky").verdict.diff_pixels,
            row(&first, "sky").counted_px,
            "every sky pixel moves when the sky is off"
        );
        let report = first.report("arena-like", "arena-like: sand, wall, armor");
        assert_eq!(
            report.as_bytes(),
            second
                .report("arena-like", "arena-like: sand, wall, armor")
                .as_bytes()
        );
        assert!(report.contains("| graph | LIVE |"));
        assert!(report.contains("| shadow_cluster | LIVE |"));
        assert!(report.contains("| sky | LIVE |"));
        assert!(report.contains("| gtao | LIVE |"));
        assert!(report.contains("| decals | PROBE | 0 |"));
        assert!(report.contains("10 rows, 5 LIVE, 5 PROBE"));
        // The artifact: dated by the caller (`PASS_TRUTH_DATE`), repo-rooted.
        let date = std::env::var("PASS_TRUTH_DATE").unwrap_or_else(|_| "arena-like".into());
        let path = super::super::live_gate::write_live_gate_report(&repo_root(), &date, &report)
            .expect("report writes");
        assert_eq!(
            std::fs::read(&path).expect("report reads back"),
            report.as_bytes()
        );
    }

    #[test]
    fn ibl_off_changes_the_armor_and_the_sand_and_nothing_else() {
        // Caller: the LF2 accept. Behavior: with the baked ambient cut, the
        // armor and the sand regions differ above epsilon, the capture repeats
        // exactly, and the sky (which the ambient never lights) is untouched.
        let Some(mut presenter) = init_or_skip("ibl_off_changes_the_armor_and_the_sand") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let on = capture(&mut presenter, &list, None);
        let on_again = capture(&mut presenter, &list, None);
        let off = capture(&mut presenter, &list, Some("ibl"));
        assert_eq!(on.rgba, on_again.rgba, "capture x2 equal");
        let armor_px = on.ids.iter().filter(|id| **id == ARMOR_DRAW).count();
        let sand_px = on.ids.iter().filter(|id| **id == FLOOR_DRAW).count();
        assert!(armor_px > 500 && sand_px > 500, "both surfaces are in view");
        let armor = surface_diff(&on, &off, &[ARMOR_DRAW]);
        let sand = surface_diff(&on, &off, &[FLOOR_DRAW]);
        eprintln!("ibl on vs off: armor {armor}/{armor_px} px, sand {sand}/{sand_px} px");
        assert!(
            armor * 2 > armor_px,
            "armor mostly differs: {armor}/{armor_px}"
        );
        assert!(sand * 2 > sand_px, "sand mostly differs: {sand}/{sand_px}");
        let sky = super::super::live_gate::region_mask(PassRegion::Sky, &on.ids);
        let sky_diff = frame_diff_count(
            &on.rgba,
            &off.rgba,
            on.width as usize,
            on.height as usize,
            sky.as_deref(),
            PASS_EPSILON,
        )
        .expect("frames line up");
        assert_eq!(sky_diff, 0, "the ambient never lights the sky");
        // The cut does not leak: the next all-on frame is the first one again.
        let after = capture(&mut presenter, &list, None);
        assert_eq!(on.rgba, after.rgba, "the cut word resets every frame");
    }

    #[test]
    fn sky_off_changes_the_sky_and_leaves_every_surface_alone() {
        // Caller: the LF1 accept. Behavior: the sky region differs above
        // epsilon on vs off, no covered pixel moves (the own pass tests
        // against the stored depth and writes none), and the capture repeats.
        let Some(mut presenter) = init_or_skip("sky_off_changes_the_sky_and_leaves_surfaces")
        else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let on = capture(&mut presenter, &list, None);
        let off = capture(&mut presenter, &list, Some("sky"));
        let off_again = capture(&mut presenter, &list, Some("sky"));
        assert_eq!(off.rgba, off_again.rgba, "the sky-off frame repeats");
        let (w, h) = (on.width as usize, on.height as usize);
        let sky_mask = super::super::live_gate::region_mask(PassRegion::Sky, &on.ids);
        let surface_mask = super::super::live_gate::region_mask(PassRegion::Geometry, &on.ids);
        let sky_px = on.ids.iter().filter(|id| **id == NO_DRAW_ID).count();
        let sky = frame_diff_count(&on.rgba, &off.rgba, w, h, sky_mask.as_deref(), PASS_EPSILON)
            .expect("frames line up");
        let surfaces = frame_diff_count(&on.rgba, &off.rgba, w, h, surface_mask.as_deref(), 0)
            .expect("frames line up");
        eprintln!("sky on vs off: sky {sky}/{sky_px} px, covered {surfaces} px");
        assert!(sky_px > 1000, "the pit leaves sky in view: {sky_px}");
        assert_eq!(sky, sky_px, "every sky pixel differs above epsilon");
        assert_eq!(surfaces, 0, "no covered pixel changes at all");
    }

    #[test]
    fn the_gtao_field_darkens_the_wall_base_and_leaves_the_open_sand() {
        // LF3 landing 2: the field scales the ambient at the contact, so
        // the wall base darkens while the open sand stays put: a contact
        // effect, never a global darkening.
        let Some(mut presenter) = init_or_skip("the_gtao_field_darkens_the_wall_base") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let on = capture(&mut presenter, &list, None);
        let off = capture(&mut presenter, &list, Some("gtao"));
        let wall = surface_diff(&on, &off, &[WALL_DRAW]);
        let floor = surface_diff(&on, &off, &[FLOOR_DRAW]);
        let wall_px = on.ids.iter().filter(|id| **id == WALL_DRAW).count();
        let floor_px = on.ids.iter().filter(|id| **id == FLOOR_DRAW).count();
        eprintln!("gtao on/off: wall {wall}/{wall_px} px, floor {floor}/{floor_px} px");
        assert!(wall_px > 100 && floor_px > 100, "both surfaces are in view");
        assert!(wall > 0, "the wall base darkens: {wall}/{wall_px}");
        assert!(
            (wall + floor) * 10 < wall_px + floor_px,
            "the open sand stays put: {} of {} px differ",
            wall + floor,
            wall_px + floor_px
        );
    }

    #[test]
    fn gtao_toggle_moves_the_ambient_pixels_only() {
        // LF3 landing 2: the GTAO field scales the ambient diffuse, so the
        // toggle moves the covered pixels. The control: with the baked
        // ambient cut the field has nothing to scale, so the direct sun
        // pixels stay byte-equal and the sun never darkens.
        let Some(mut presenter) = init_or_skip("gtao_toggle_moves_the_ambient_pixels_only") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let on = capture(&mut presenter, &list, None);
        let off = capture(&mut presenter, &list, Some("gtao"));
        let w = on.width as usize;
        let h = on.height as usize;
        let surface_mask = super::super::live_gate::region_mask(PassRegion::Geometry, &on.ids);
        let surfaces = frame_diff_count(
            &on.rgba,
            &off.rgba,
            w,
            h,
            surface_mask.as_deref(),
            PASS_EPSILON,
        )
        .expect("frames line up");
        eprintln!("gtao on/off: {surfaces} covered px differ");
        assert!(surfaces > 0, "the field darkens the ambient where it bites");
        // The control frame: no baked ambient, so the field scales nothing
        // and the direct sun pixels are byte-equal with the pass on or off.
        let mut sun_only = LookToggles::all_on();
        sun_only.disable("ibl");
        let sun_on = presenter
            .capture_live_pixels(&list, &sun_only)
            .expect("live frame captures");
        sun_only.disable("gtao");
        let sun_off = presenter
            .capture_live_pixels(&list, &sun_only)
            .expect("live frame captures");
        assert_eq!(
            sun_on.rgba, sun_off.rgba,
            "the direct sun never moves when the field comes on"
        );
    }

    #[test]
    fn the_sky_bake_repeats_and_is_the_texture_the_shader_samples() {
        // Caller: the LF1 accept ("bake bytes x2 equal"). Behavior: two
        // arena-day bakes give identical texels, and the presenter's bake (the
        // texels `fs_sky` loads) is that same bake.
        let Some(presenter) = init_or_skip("the_sky_bake_repeats") else {
            return;
        };
        let a = crate::arena_day::bake_arena_day().expect("day rig bakes");
        let b = crate::arena_day::bake_arena_day().expect("day rig bakes again");
        assert_eq!(a.look.texels, b.look.texels, "bake bytes x2 equal");
        assert_eq!(
            presenter.sky_look.texels, a.look.texels,
            "the shader's bake"
        );
    }

    #[test]
    fn post_reads_live_once_the_frame_holds_light_above_the_bloom_threshold() {
        // Caller: the `post` switch (the blit's bloom halo). Behavior: in the
        // plain arena-like scene nothing reaches the bloom threshold, so the
        // gate reads post PROBE; a sun twelve times brighter pushes the sand
        // and the armor over it and the same switch reads LIVE. The first
        // proves a PROBE is a measurement, the second that the switch is wired.
        let Some(mut presenter) = init_or_skip("post_reads_live_with_bright_light") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let mut list = arena_like_scene(&mut presenter, W, H);
        list.light.intensity *= 12.0;
        let gate = gate_live_scene(&mut presenter, &list).expect("bright scene gates");
        let post = row(&gate, "post");
        eprintln!("post under a bright sun: {post:?}");
        assert!(post.verdict.live, "bloom halo moves pixels: {post:?}");
        assert!(!row(&gate, "decals").verdict.live, "decals stay PROBE");
    }

    #[test]
    fn the_sky_costs_its_own_measured_pass_and_the_row_reports_it() {
        // Caller: the LF1 accept ("`arena per-pass` sky above 0 ms on live
        // frames") and the frame-cost honesty rule. Behavior: with the sky on,
        // its own timed pass reads above zero on the device clock and the
        // frame receipt's sky cell carries it; with the sky off there is no sky
        // sample and the cell is zero. The on and off totals are printed so
        // the cost of drawing the sky is a number, not a switch.
        let Some(mut presenter) = init_or_skip("the_sky_costs_its_own_measured_pass") else {
            return;
        };
        presenter.resize(1280, 720).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, 1280, 720);
        let all_on = LookToggles::all_on();
        let mut sky_off = LookToggles::all_on();
        sky_off.disable("sky");
        let Some(on) = live_gpu_cost(&mut presenter, &list, &all_on, 80).expect("cost runs") else {
            crate::test_gpu::note_skip("sky_cost", "adapter lacks TIMESTAMP_QUERY");
            return;
        };
        // The newest resolved sky sample rides the next live frame's row.
        let on_cell = presenter
            .render_live_offscreen_with_toggles(&list, &all_on)
            .expect("live frame renders")
            .per_pass_us()[0]
            .1;
        let off = live_gpu_cost(&mut presenter, &list, &sky_off, 80)
            .expect("cost runs")
            .expect("same adapter, same support");
        let off_cell = presenter
            .render_live_offscreen_with_toggles(&list, &sky_off)
            .expect("live frame renders")
            .per_pass_us()[0]
            .1;
        let mut ibl_off = LookToggles::all_on();
        ibl_off.disable("ibl");
        let no_ibl = live_gpu_cost(&mut presenter, &list, &ibl_off, 80)
            .expect("cost runs")
            .expect("same adapter, same support");
        eprintln!(
            "sky on  : total p50 {:.4} ms (forward {:.4}, sky {:.4}, blit {:.4}) over {} frames, receipt sky cell {on_cell} us",
            on.total.p50, on.forward.p50, on.sky.p50, on.blit.p50, on.frames
        );
        eprintln!(
            "sky off : total p50 {:.4} ms (forward {:.4}, sky {:.4}, blit {:.4}) over {} frames, receipt sky cell {off_cell} us",
            off.total.p50, off.forward.p50, off.sky.p50, off.blit.p50, off.frames
        );
        eprintln!(
            "ibl off : total p50 {:.4} ms (forward {:.4}, sky {:.4}, blit {:.4}) over {} frames",
            no_ibl.total.p50, no_ibl.forward.p50, no_ibl.sky.p50, no_ibl.blit.p50, no_ibl.frames
        );
        assert!(on.sky.p50 > 0.0, "the own-pass sky is timed: {on:?}");
        assert!(on_cell > 0, "the per-pass row carries the sky ms");
        assert_eq!(off.sky.p50, 0.0, "no sky pass, no sky sample: {off:?}");
        assert_eq!(off_cell, 0, "the row stays 0 when the sky is off");
        assert!(
            on.sky.p50 < on.total.p50,
            "the sky is part of the frame, not all of it"
        );
    }

    #[test]
    fn gtao_costs_its_own_measured_pass_and_the_row_reports_it() {
        // Caller: the LF3 accept ("gtao above 0 ms live") and the frame-cost
        // honesty rule. Behavior: with the row on, the GTAO compute pass reads
        // above zero on the device clock and the frame receipt's gtao cell
        // carries it; with the row off there is no sample and the cell is
        // zero. Both totals are printed, so the cost is a number.
        let Some(mut presenter) = init_or_skip("gtao_costs_its_own_measured_pass") else {
            return;
        };
        presenter.resize(1280, 720).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, 1280, 720);
        let all_on = LookToggles::all_on();
        let mut gtao_off = LookToggles::all_on();
        gtao_off.disable("gtao");
        let Some(on) = live_gpu_cost(&mut presenter, &list, &all_on, 80).expect("cost runs") else {
            crate::test_gpu::note_skip("gtao_cost", "adapter lacks TIMESTAMP_QUERY");
            return;
        };
        let on_cell = presenter
            .render_live_offscreen_with_toggles(&list, &all_on)
            .expect("live frame renders")
            .per_pass_us()[6]
            .1;
        let off = live_gpu_cost(&mut presenter, &list, &gtao_off, 80)
            .expect("cost runs")
            .expect("same adapter, same support");
        let off_cell = presenter
            .render_live_offscreen_with_toggles(&list, &gtao_off)
            .expect("live frame renders")
            .per_pass_us()[6]
            .1;
        eprintln!(
            "gtao on : total p50 {:.4} ms (forward {:.4}, sky {:.4}, gtao {:.4}, blit {:.4}) over {} frames, receipt gtao cell {on_cell} us",
            on.total.p50, on.forward.p50, on.sky.p50, on.gtao.p50, on.blit.p50, on.frames
        );
        eprintln!(
            "gtao off: total p50 {:.4} ms (forward {:.4}, sky {:.4}, gtao {:.4}, blit {:.4}) over {} frames, receipt gtao cell {off_cell} us",
            off.total.p50, off.forward.p50, off.sky.p50, off.gtao.p50, off.blit.p50, off.frames
        );
        assert!(on.gtao.p50 > 0.0, "the GTAO pass is timed: {on:?}");
        assert!(on_cell > 0, "the per-pass row carries the gtao ms");
        assert_eq!(off.gtao.p50, 0.0, "no pass, no sample: {off:?}");
        assert_eq!(off_cell, 0, "the row stays 0 when the row is off");
        assert!(
            on.gtao.p50 < crate::gtao::GTAO_BUDGET_MS as f32,
            "GTAO fits its budget at 720p: {:.4} ms",
            on.gtao.p50
        );
        assert!(on.gtao.p50 < on.total.p50, "the pass is part of the frame");
    }

    /// Relative luminance of one RGBA8 pixel, 0..=255 scale.
    fn luma(px: &[u8]) -> f32 {
        0.2126 * f32::from(px[0]) + 0.7152 * f32::from(px[1]) + 0.0722 * f32::from(px[2])
    }

    #[test]
    fn eg11b_the_sun_map_shadows_the_sand_under_the_box_and_only_there() {
        // Caller: EG11b accept, through the EG8 gate machinery (the window's
        // live frame, pass on versus off). One 1.6 m box stands on open sand
        // under the arena sun: with the sun pass on, the sand behind the box
        // darkens past the luminance floor, every open-sand pixel stays
        // within 1% (in fact byte-identical), and no pixel anywhere brightens
        // -- the old view-distance cascade dim is gone, not merely hidden.
        let Some(mut presenter) = init_or_skip("eg11b_the_sun_map_shadows_the_sand_under_the_box")
        else {
            return;
        };
        // The sun switch is one process-wide static: hold the EG11 test lock
        // so the switch test cannot flip it in the middle of this frame.
        let _held = crate::present::SUN_SWITCH_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        presenter.resize(W, H).expect("valid size applies");
        let list = sun_shadow_scene(&mut presenter, W, H);
        let on = capture(&mut presenter, &list, None);
        let again = capture(&mut presenter, &list, None);
        assert_eq!(
            on.rgba, again.rgba,
            "the shadowed frame repeats byte for byte"
        );
        assert_eq!(on.ids, again.ids, "and covers the same draws");
        let off = capture(&mut presenter, &list, Some("shadow_cluster"));
        assert_eq!(
            off.rgba.len(),
            on.rgba.len(),
            "the off frame keeps its size"
        );
        let (w, h) = (W as usize, H as usize);
        let pixels = w * h;
        let mut floor_px = 0usize;
        let mut shadowed = Vec::new();
        let mut brightened = 0usize;
        let mut darkest = 1.0f32;
        for i in 0..pixels {
            let (a, b) = (
                luma(&on.rgba[i * 4..i * 4 + 4]),
                luma(&off.rgba[i * 4..i * 4 + 4]),
            );
            if a > b + f32::from(PASS_EPSILON) {
                brightened += 1;
            }
            if on.ids[i] != SUN_FLOOR_DRAW {
                continue;
            }
            floor_px += 1;
            if a < b - f32::from(PASS_EPSILON) {
                shadowed.push(i);
                darkest = darkest.min(a / b.max(0.001));
            }
        }
        assert_eq!(brightened, 0, "the sun map only removes light");
        assert!(
            shadowed.len() > PASS_LIVE_FLOOR + 500,
            "the shadow covers a region: {} px",
            shadowed.len()
        );
        assert!(
            shadowed.len() * 20 < floor_px,
            "the shadow stays local: {} of {floor_px} sand px",
            shadowed.len()
        );
        assert!(
            darkest < 0.8,
            "the shadowed sand drops past 20%: darkest ratio {darkest:.3}"
        );
        // Open sand is every floor pixel clear of the shadow's PCF penumbra
        // (a 3x3 compare reaches three texels): it must sit within 1% of the
        // off frame, which is where the old cascade dim would have leaked.
        let mut near_shadow = vec![false; pixels];
        for i in shadowed.iter().copied() {
            let (x, y) = (i % w, i / w);
            for ny in y.saturating_sub(3)..=(y + 3).min(h - 1) {
                for nx in x.saturating_sub(3)..=(x + 3).min(w - 1) {
                    near_shadow[ny * w + nx] = true;
                }
            }
        }
        let mut open_moved = 0usize;
        let mut open_px = 0usize;
        for (i, id) in on.ids.iter().enumerate() {
            if *id != SUN_FLOOR_DRAW || near_shadow[i] {
                continue;
            }
            open_px += 1;
            let (a, b) = (
                luma(&on.rgba[i * 4..i * 4 + 4]),
                luma(&off.rgba[i * 4..i * 4 + 4]),
            );
            if (a - b).abs() > 0.01 * b.max(1.0) {
                open_moved += 1;
            }
        }
        assert!(open_px > floor_px / 2, "open sand is most of the floor");
        assert_eq!(open_moved, 0, "open sand stays within 1% of the off frame");
        // The shadow is the box's: a shadowed pixel touches the box's own
        // screen footprint (the sun throws it within one box width).
        let box_px: Vec<usize> = (0..pixels).filter(|i| on.ids[*i] == SUN_BOX_DRAW).collect();
        assert!(!box_px.is_empty(), "the fixture box is drawn");
        let touches_box = shadowed.iter().any(|i| {
            let (x, y) = (i % w, i / w);
            box_px.iter().any(|b| {
                let (bx, by) = (b % w, b / w);
                x.abs_diff(bx) <= 2 && y.abs_diff(by) <= 2
            })
        });
        assert!(touches_box, "the shadow starts at the fixture box");
    }
}

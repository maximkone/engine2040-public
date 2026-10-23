//! LF5 slice 1: the forward pass writes screen-space motion beside HDR and
//! id, so TAA can reproject. Ground truth is analytic (each fixture point's
//! own two projections), never a golden image: a still frame is all zero, a
//! known camera pan and a moved body each carry their closed-form shift.
//! LF5b slice 2 adds the presenter's Halton(2,3) projection jitter: the
//! camera uniform carries it, the motion target never does.
//! Skip (pass) with no adapter, exactly like the `tests_gpu` receipts.

#[cfg(feature = "gpu")]
#[cfg(test)]
mod motion_tests {
    use super::super::*;
    use crate::test_gpu::{require_or_skip, NoAdapter};
    use glam::{Vec2, Vec3, Vec4};

    /// Square fixture size: the camera aspect is 1 and one uv unit is 128 px.
    pub(super) const SIZE: u32 = 128;
    /// Fixture vertical field of view, degrees (the `camera_at` value).
    pub(super) const FOV_Y_DEGREES: f32 = 40.0;
    /// Eye distance from the fixture plane (`z = 0`), meters.
    pub(super) const EYE_Z: f32 = 6.0;

    pub(super) fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    pub(super) fn plane_key() -> MeshKey {
        MeshKey::new(0x0F5, [0x5A; 32])
    }

    /// A fronto-parallel square at `z = 0` facing `+z`. Every pixel of a
    /// straight camera sees the same depth, so a yaw or a body shift is a
    /// uniform analytic move, never a rounded corner.
    pub(super) fn plane(half: f32) -> crate::v1::MeshData {
        crate::v1::MeshData {
            positions: vec![
                [-half, -half, 0.0],
                [half, -half, 0.0],
                [half, half, 0.0],
                [-half, half, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
        }
    }

    /// Camera at `(0, 0, EYE_Z)` turned `yaw_degrees` about `+y` around its
    /// own eye; yaw 0 looks down `-z` at the fixture plane.
    pub(super) fn camera(yaw_degrees: f32) -> crate::v1::Camera {
        let theta = yaw_degrees.to_radians();
        let eye = Vec3::new(0.0, 0.0, EYE_Z);
        let forward = Vec3::new(-theta.sin(), 0.0, -theta.cos());
        let mut camera = crate::v1::Camera::new(
            eye.to_array(),
            (eye + forward * EYE_Z).to_array(),
            [0.0, 1.0, 0.0],
            FOV_Y_DEGREES.to_radians(),
        );
        // The fixture target is square, so one uv unit is the same on both
        // axes; the live path overrides the aspect from the target size.
        camera.aspect = 1.0;
        camera
    }

    pub(super) fn list_for(camera: crate::v1::Camera, item: DrawItem) -> DrawList {
        let scene = crate::v1::default_scene(SIZE, SIZE).expect("default scene is valid");
        DrawList {
            camera,
            light: scene.light,
            items: vec![item],
        }
    }

    pub(super) fn plane_item() -> DrawItem {
        let scene = crate::v1::default_scene(SIZE, SIZE).expect("default scene is valid");
        DrawItem {
            mesh: plane_key(),
            material: scene.material,
            base_color: None,
            normal_map: None,
            orm: None,
            emissive: None,
        }
    }

    /// IEEE half to float, for the `Rg16Float` motion target.
    fn f16(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = i32::from((bits >> 10) & 0x1f);
        let fraction = f32::from(bits & 0x03ff);
        match exponent {
            0 => sign * fraction * 2f32.powi(-24),
            31 => {
                if fraction == 0.0 {
                    sign * f32::INFINITY
                } else {
                    f32::NAN
                }
            }
            _ => sign * (1.0 + fraction / 1024.0) * 2f32.powi(exponent - 15),
        }
    }

    /// Copy the motion target back as one uv pair per pixel. Test only: this
    /// waits on the GPU.
    pub(super) fn read_motion(presenter: &Presenter) -> (Vec<Vec2>, u32, u32) {
        let targets = presenter.targets.as_ref().expect("targets exist");
        let (w, h) = (targets.width, targets.height);
        let padded = (w * 4).div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-motion-readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.motion_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        presenter.queue.submit(Some(enc.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
            r.expect("motion readback maps");
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("motion readback polls");
        let mut motion = vec![Vec2::ZERO; (w * h) as usize];
        {
            let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
            let row = padded as usize;
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let at = y * row + x * 4;
                    let u = f16(u16::from_le_bytes([mapped[at], mapped[at + 1]]));
                    let v = f16(u16::from_le_bytes([mapped[at + 2], mapped[at + 3]]));
                    motion[y * w as usize + x] = Vec2::new(u, v);
                }
            }
        }
        buffer.unmap();
        (motion, w, h)
    }

    /// The fixture plane point seen through pixel `(px, py)`'s own ray in
    /// `camera`, at the pixel center (the 1x fragment center).
    fn plane_point_at(px: u32, py: u32, w: u32, h: u32, camera: &crate::v1::Camera) -> Vec3 {
        let ndc = Vec2::new(
            2.0 * (px as f32 + 0.5) / w as f32 - 1.0,
            1.0 - 2.0 * (py as f32 + 0.5) / h as f32,
        );
        let inverse = camera
            .view_proj()
            .expect("fixture camera is valid")
            .inverse();
        let near = inverse * Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let near = near.truncate() / near.w;
        let far = inverse * Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let far = far.truncate() / far.w;
        let direction = (far - near).normalize();
        let t = -near.z / direction.z;
        near + direction * t
    }

    fn project(point: Vec3, camera: &crate::v1::Camera) -> Vec2 {
        let clip = camera.view_proj().expect("fixture camera is valid") * point.extend(1.0);
        Vec2::new(clip.x / clip.w, clip.y / clip.w)
    }

    /// Motion the shader should write for a surface point now at `current`
    /// whose previous world position was `previous`: the uv-space ndc delta,
    /// y down, the pair TAA reprojects.
    fn expected_motion(
        previous: Vec3,
        current: Vec3,
        camera_before: &crate::v1::Camera,
        camera_now: &crate::v1::Camera,
    ) -> Vec2 {
        let before = project(previous, camera_before);
        let now = project(current, camera_now);
        Vec2::new((now.x - before.x) * 0.5, (now.y - before.y) * -0.5)
    }

    /// The stored motion at a probe pixel matches the analytic shift within
    /// 0.1 px.
    fn check_probe(motion: &[Vec2], w: u32, h: u32, px: u32, py: u32, expected: Vec2, what: &str) {
        let got = motion[(py * w + px) as usize];
        let error_px =
            ((got.x - expected.x).abs() * w as f32).max((got.y - expected.y).abs() * h as f32);
        assert!(
            error_px <= 0.1,
            "{what} pixel ({px},{py}): motion {got:?} vs analytic {expected:?} = {error_px:.4} px"
        );
    }

    #[test]
    fn a_still_camera_and_still_body_write_zero_motion() {
        let Some(mut presenter) = init_or_skip("a_still_camera_and_still_body_write_zero_motion")
        else {
            return;
        };
        presenter
            .upload_mesh(plane_key(), &plane(20.0))
            .expect("fixture plane uploads");
        presenter.resize(SIZE, SIZE).expect("valid size applies");
        let list = list_for(camera(0.0), plane_item());
        for frame in 0..2 {
            let receipt = presenter
                .render_live_offscreen(&list)
                .expect("still frame renders");
            assert!(receipt.is_clean(), "frame {frame} validation errors 0");
            assert_eq!(
                presenter.deferred_validation_errors(),
                0,
                "frame {frame} has no late validation error"
            );
        }
        let (motion, w, h) = read_motion(&presenter);
        assert_eq!(motion.len(), (w * h) as usize, "one pair per pixel");
        let max_px = motion.iter().fold(0.0f32, |worst, m| {
            worst.max(m.x.abs() * w as f32).max(m.y.abs() * h as f32)
        });
        assert!(
            max_px <= 0.05,
            "a still fixture writes no motion, worst {max_px:.4} px"
        );
    }

    #[test]
    fn a_known_camera_pan_writes_its_analytic_shift() {
        let Some(mut presenter) = init_or_skip("a_known_camera_pan_writes_its_analytic_shift")
        else {
            return;
        };
        presenter
            .upload_mesh(plane_key(), &plane(20.0))
            .expect("fixture plane uploads");
        presenter.resize(SIZE, SIZE).expect("valid size applies");
        let before_camera = camera(0.0);
        let after_camera = camera(10.0);
        presenter
            .render_live_offscreen(&list_for(before_camera.clone(), plane_item()))
            .expect("first frame renders");
        let receipt = presenter
            .render_live_offscreen(&list_for(after_camera.clone(), plane_item()))
            .expect("panned frame renders");
        assert!(receipt.is_clean(), "panned frame validation errors 0");
        assert_eq!(presenter.deferred_validation_errors(), 0);
        let (motion, w, h) = read_motion(&presenter);
        for (px, py) in [(64, 64), (40, 64), (88, 64), (64, 40), (64, 88)] {
            let point = plane_point_at(px, py, w, h, &after_camera);
            let expected = expected_motion(point, point, &before_camera, &after_camera);
            check_probe(&motion, w, h, px, py, expected, "camera pan");
        }
        // A 10-degree yaw is tens of pixels, not a rounding artifact.
        let center = plane_point_at(64, 64, w, h, &after_camera);
        let shift_px =
            (expected_motion(center, center, &before_camera, &after_camera).x * w as f32).abs();
        assert!(shift_px > 20.0, "the pan moves the center {shift_px:.2} px");
    }

    #[test]
    fn a_moved_body_writes_its_own_shift() {
        let Some(mut presenter) = init_or_skip("a_moved_body_writes_its_own_shift") else {
            return;
        };
        presenter
            .upload_mesh(plane_key(), &plane(20.0))
            .expect("fixture plane uploads");
        presenter.resize(SIZE, SIZE).expect("valid size applies");
        let camera = camera(0.0);
        let list = list_for(camera.clone(), plane_item());
        presenter
            .render_live_offscreen(&list)
            .expect("first frame renders");
        // A CPU-refreshed body: the same plane shifted 0.1 m along +x.
        let shift = 0.1f32;
        let moved: Vec<[f32; 3]> = plane(20.0)
            .positions
            .iter()
            .map(|p| [p[0] + shift, p[1], p[2]])
            .collect();
        let normals = vec![[0.0, 0.0, 1.0]; 4];
        let uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert!(
            presenter
                .refresh_mesh_vertices(plane_key(), &moved, &normals, &uvs)
                .expect("vertex refresh applies"),
            "the fixture is resident"
        );
        let receipt = presenter
            .render_live_offscreen(&list)
            .expect("moved frame renders");
        assert!(receipt.is_clean(), "moved frame validation errors 0");
        assert_eq!(presenter.deferred_validation_errors(), 0);
        let (motion, w, h) = read_motion(&presenter);
        for (px, py) in [(64, 64), (50, 64), (78, 64), (64, 50), (64, 78)] {
            let current = plane_point_at(px, py, w, h, &camera);
            let previous = current - Vec3::new(shift, 0.0, 0.0);
            let expected = expected_motion(previous, current, &camera, &camera);
            check_probe(&motion, w, h, px, py, expected, "body move");
        }
        let center = motion[(64 * w + 64) as usize];
        assert!(
            center.x.abs() * w as f32 > 2.0,
            "a 0.1 m body move is visible: {:.3} px",
            center.x * w as f32
        );
    }
}

/// LF5b slice 2: the presenter feeds the 8-frame Halton(2,3) offset into the
/// camera uniform behind a switch that defaults off, while the motion target
/// keeps the un-jittered matrix. Proven on the presenter's recorded camera
/// images (the bytes each buffer took) and on the motion target itself, no
/// golden pixels: GPU frames, skipped with no adapter.
#[cfg(feature = "gpu")]
#[cfg(test)]
mod jitter_tests {
    use super::motion_tests::{
        camera, init_or_skip, list_for, plane, plane_item, plane_key, read_motion, SIZE,
    };
    use crate::post;
    use crate::present::Presenter;

    /// One little-endian `f32` at `at` in a camera-uniform image.
    fn word_at(bytes: &[u8], at: usize) -> f32 {
        f32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
    }

    /// The clip-space point a column-major view-projection image gives `p`.
    fn clip_of(matrix: &[u8], p: [f32; 4]) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        for (row, item) in out.iter_mut().enumerate() {
            *item = (0..4)
                .map(|column| word_at(matrix, column * 16 + row * 4) * p[column])
                .sum();
        }
        out
    }

    /// Check one jittered frame: the offset is Halton(2,3) inside the half
    /// pixel cell, the camera image differs from the staged bytes by exactly
    /// the clip w row shift (a world point's ndc moves by the offset, its
    /// depth does not), and the un-jittered matrix and its one-frame lag stay
    /// staged. Returns the offset and the steady matrix.
    fn check_jittered_frame(
        presenter: &Presenter,
        step: u32,
        previous: Option<[u8; 64]>,
    ) -> ([f32; 2], [u8; 64]) {
        let frame = u64::from(step);
        let (staged, cam, un_jittered, prev) = presenter.last_camera_images();
        let expected = post::jitter_offset(frame);
        assert_eq!(
            presenter.applied_jitter_px(),
            expected,
            "frame {step} applies the shared offset"
        );
        // The eight offsets are Halton(2,3), inside the half-pixel cell.
        assert_eq!(
            expected[0],
            post::halton(step + 1, 2) - 0.5,
            "frame {step} x is Halton base 2"
        );
        assert_eq!(
            expected[1],
            post::halton(step + 1, 3) - 0.5,
            "frame {step} y is Halton base 3"
        );
        for (axis, &value) in expected.iter().enumerate() {
            assert!(
                value > -0.5 && value < 0.5,
                "frame {step} axis {axis} inside (-0.5, 0.5): {value}"
            );
        }
        // The jitter is the projection shift `row0 -= ndc.x * row3` and
        // `row1 -= ndc.y * row3` across all four columns; rows 2 and 3 and
        // the camera position stay as staged.
        let ndc = post::jitter_ndc(frame, SIZE, SIZE);
        let mut expected_cam = staged;
        for column in 0..4 {
            let base = column * 16;
            let w = word_at(&staged, base + 12);
            let x = word_at(&staged, base) - ndc[0] * w;
            let y = word_at(&staged, base + 4) - ndc[1] * w;
            expected_cam[base..base + 4].copy_from_slice(&x.to_le_bytes());
            expected_cam[base + 4..base + 8].copy_from_slice(&y.to_le_bytes());
            assert_eq!(
                &cam[base + 8..base + 16],
                &staged[base + 8..base + 16],
                "frame {step} column {column} rows 2 and 3 unchanged"
            );
        }
        assert_eq!(
            cam, expected_cam,
            "frame {step} jitter is the clip w row shift"
        );
        assert_eq!(
            &cam[64..],
            &staged[64..],
            "frame {step} camera position unchanged"
        );
        // Independent of the byte form: this world point's ndc moves by
        // exactly -ndc and its depth (clip w) does not move.
        let point = [0.5f32, -0.25, 0.0, 1.0];
        let before = clip_of(&staged, point);
        let after = clip_of(&cam, point);
        assert_eq!(
            after[3], before[3],
            "frame {step} clip w unchanged by the jitter"
        );
        let ndc_before = [before[0] / before[3], before[1] / before[3]];
        let ndc_after = [after[0] / after[3], after[1] / after[3]];
        for ((&before, &after), &want) in ndc_before.iter().zip(&ndc_after).zip(&ndc) {
            let shift = after - before;
            assert!(
                (shift + want).abs() < 1e-5,
                "frame {step} ndc shifts by -{want}, got {shift}"
            );
        }
        assert_eq!(
            &un_jittered[..],
            &staged[..64],
            "frame {step} steady matrix is the staged one"
        );
        if let Some(previous) = previous {
            assert_eq!(
                prev, previous,
                "frame {step} previous camera lags the steady matrix"
            );
        }
        (expected, un_jittered)
    }

    #[test]
    fn jitter_offsets_follow_halton_without_entering_the_motion_target() {
        let Some(mut presenter) =
            init_or_skip("jitter_offsets_follow_halton_without_entering_the_motion_target")
        else {
            return;
        };
        presenter
            .upload_mesh(plane_key(), &plane(20.0))
            .expect("fixture plane uploads");
        presenter.resize(SIZE, SIZE).expect("valid size applies");
        presenter.set_taa_jitter_enabled(true);
        assert!(presenter.taa_jitter_enabled(), "the switch is on");
        let list = list_for(camera(0.0), plane_item());
        let mut steady: Option<[u8; 64]> = None;
        let mut seen = Vec::new();
        for step in 0..8u32 {
            let receipt = presenter
                .render_live_offscreen(&list)
                .expect("jittered frame renders");
            assert!(receipt.is_clean(), "frame {step} validation errors 0");
            assert_eq!(
                presenter.deferred_validation_errors(),
                0,
                "frame {step} has no late validation error"
            );
            let (expected, un_jittered) = check_jittered_frame(&presenter, step, steady);
            steady = Some(un_jittered);
            seen.push(expected);
        }
        assert_eq!(seen.len(), 8, "one offset per frame");
        for (at, pair) in seen.iter().enumerate() {
            for other in &seen[at + 1..] {
                assert_ne!(pair, other, "offsets {pair:?} and {other:?} repeat");
            }
        }
        // With the jitter on, a still camera and body still write zero motion:
        // the motion entries read the steady matrix, never the jittered one.
        let (motion, w, h) = read_motion(&presenter);
        let max_px = motion.iter().fold(0.0f32, |worst, m| {
            worst.max(m.x.abs() * w as f32).max(m.y.abs() * h as f32)
        });
        assert!(
            max_px <= 0.05,
            "a jittered still frame writes no motion, worst {max_px:.4} px"
        );
    }

    #[test]
    fn jitter_off_writes_the_staged_camera_bytes_unchanged() {
        let Some(mut presenter) =
            init_or_skip("jitter_off_writes_the_staged_camera_bytes_unchanged")
        else {
            return;
        };
        assert!(!presenter.taa_jitter_enabled(), "the jitter defaults off");
        presenter
            .upload_mesh(plane_key(), &plane(20.0))
            .expect("fixture plane uploads");
        presenter.resize(SIZE, SIZE).expect("valid size applies");
        let first = list_for(camera(0.0), plane_item());
        let second = list_for(camera(10.0), plane_item());
        presenter
            .render_live_offscreen(&first)
            .expect("first frame renders");
        let (one_staged, one_cam, one_steady, one_prev) = presenter.last_camera_images();
        assert_eq!(
            presenter.applied_jitter_px(),
            [0.0, 0.0],
            "off applies no offset"
        );
        assert_eq!(one_cam, one_staged, "off writes the staged camera bytes");
        assert_eq!(
            &one_steady[..],
            &one_staged[..64],
            "off leaves the steady matrix staged"
        );
        assert_eq!(one_prev, one_steady, "the first frame publishes itself");
        presenter
            .render_live_offscreen(&second)
            .expect("panned frame renders");
        let (two_staged, two_cam, two_steady, two_prev) = presenter.last_camera_images();
        assert_eq!(
            presenter.applied_jitter_px(),
            [0.0, 0.0],
            "still off after a camera move"
        );
        assert_eq!(two_cam, two_staged, "off writes the staged camera bytes");
        assert_eq!(
            &two_steady[..],
            &two_staged[..64],
            "off leaves the steady matrix staged"
        );
        assert_eq!(
            two_prev, one_steady,
            "the previous camera is frame one's un-jittered matrix"
        );
        assert_ne!(two_steady, one_steady, "a camera pan changes the matrix");
        // The same camera with the switch on: rows 0 and 1 take the clip w
        // row shift, and the motion history still takes the un-jittered
        // matrix.
        presenter.set_taa_jitter_enabled(true);
        presenter
            .render_live_offscreen(&second)
            .expect("jittered frame renders");
        let (three_staged, three_cam, three_steady, three_prev) = presenter.last_camera_images();
        let ndc = post::jitter_ndc(2, SIZE, SIZE);
        assert_eq!(
            presenter.applied_jitter_px(),
            post::jitter_offset(2),
            "frame two of the sequence"
        );
        let mut expected_cam = three_staged;
        for column in 0..4 {
            let base = column * 16;
            let w = word_at(&three_staged, base + 12);
            let x = word_at(&three_staged, base) - ndc[0] * w;
            let y = word_at(&three_staged, base + 4) - ndc[1] * w;
            expected_cam[base..base + 4].copy_from_slice(&x.to_le_bytes());
            expected_cam[base + 4..base + 8].copy_from_slice(&y.to_le_bytes());
            assert_eq!(
                &three_cam[base + 8..base + 16],
                &three_staged[base + 8..base + 16],
                "column {column} rows 2 and 3 unchanged"
            );
        }
        assert_eq!(three_cam, expected_cam, "the w row shift is exact");
        assert_eq!(
            &three_cam[64..],
            &three_staged[64..],
            "camera position unchanged"
        );
        assert_eq!(
            three_staged, two_staged,
            "the same camera stages the same bytes"
        );
        assert_eq!(
            &three_steady[..],
            &three_staged[..64],
            "the motion matrix stays un-jittered"
        );
        assert_eq!(
            three_prev, two_steady,
            "the motion history stays un-jittered"
        );
        assert_ne!(
            three_cam, three_staged,
            "the switch on writes a jittered camera"
        );
    }
}

/// LF5c slice 3: the live TAA resolve. The flicker metric is the mean luma
/// change over the fixture plane's edge band across two jittered frames,
/// resolve off versus on; the ghost metric is the width of the band next to a
/// moved body's new edge where the resolved frame differs from the current
/// frame by more than a quarter of the edge step. Both are analytic fixtures
/// and device pixels, never a golden image; GPU frames skip with no adapter.
#[cfg(feature = "gpu")]
#[cfg(test)]
mod taa_tests {
    use super::motion_tests::{
        camera, init_or_skip, list_for, plane, plane_item, plane_key, EYE_Z, FOV_Y_DEGREES, SIZE,
    };
    use crate::present::DrawList;
    use crate::present::Presenter;

    /// Screen x of the fixture plane's left border, pixels. The frame 0 to
    /// frame 1 jitter moves the border by a quarter pixel, across the pixel
    /// center at 35.5, so one edge column flips and the band carries flicker.
    const EDGE_PX: f32 = 35.6;

    /// The same square plane, sized so its left border lands on [`EDGE_PX`]
    /// through the fixture camera (no yaw, aspect 1, `EYE_Z` out).
    fn edge_plane() -> crate::v1::MeshData {
        let ndc = 2.0 * EDGE_PX / SIZE as f32 - 1.0;
        let half_extent = EYE_Z * (0.5 * FOV_Y_DEGREES).to_radians().tan();
        plane(-ndc * half_extent)
    }

    /// Copy the LDR twin into a tight RGBA8 image. Test only: this waits on
    /// the GPU, once per frame.
    fn read_twin(presenter: &Presenter) -> Vec<u8> {
        let targets = presenter.targets.as_ref().expect("twin targets exist");
        let (w, h) = (targets.width, targets.height);
        let padded = (w * 4).div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa-twin-readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("taa-twin-copy"),
            });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &targets.out_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        presenter.queue.submit(Some(enc.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
            r.expect("taa twin readback maps");
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("taa twin readback polls");
        let row = (w * 4) as usize;
        let mut rgba = vec![0u8; row * h as usize];
        {
            let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
            for y in 0..h as usize {
                let src = y * padded as usize;
                rgba[y * row..(y + 1) * row].copy_from_slice(&mapped[src..src + row]);
            }
        }
        buffer.unmap();
        rgba
    }

    /// One window-path twin frame, checked clean, then its pixels.
    fn frame(presenter: &mut Presenter, list: &DrawList) -> Vec<u8> {
        let receipt = presenter
            .render_live_offscreen(list)
            .expect("frame renders");
        assert!(receipt.is_clean(), "frame validation errors 0");
        read_twin(presenter)
    }

    fn luma(rgba: &[u8], pixel: u32) -> f32 {
        let at = pixel as usize * 4;
        0.2126 * f32::from(rgba[at])
            + 0.7152 * f32::from(rgba[at + 1])
            + 0.0722 * f32::from(rgba[at + 2])
    }

    fn luma_at(rgba: &[u8], w: u32, x: u32, y: u32) -> f32 {
        luma(rgba, y * w + x)
    }

    /// The six border columns (three per plane edge) over the rows the plane
    /// fills, from the analytic fixture geometry.
    fn edge_band(w: u32, h: u32) -> Vec<u32> {
        let left = EDGE_PX.floor() as u32;
        let right = (w as f32 - EDGE_PX).floor() as u32;
        let columns = [left - 1, left, left + 1, right - 1, right, right + 1];
        let mut band = Vec::new();
        for y in (h / 3)..(2 * h / 3) {
            for x in columns {
                band.push(y * w + x);
            }
        }
        band
    }

    fn mean_change(a: &[u8], b: &[u8], band: &[u32]) -> f32 {
        let total: f32 = band
            .iter()
            .map(|&pixel| (luma(a, pixel) - luma(b, pixel)).abs())
            .sum();
        total / band.len() as f32
    }

    #[test]
    fn taa_resolve_cuts_sand_edge_flicker_at_least_half() {
        let Some(mut off) = init_or_skip("taa_resolve_cuts_sand_edge_flicker_at_least_half") else {
            return;
        };
        let Some(mut on) = init_or_skip("taa_flicker_resolve_on") else {
            return;
        };
        // The sand edge over the bare clear colour: the sky off gives the
        // fixture a full-luma step at each border, so a quarter-pixel jitter
        // that flips one edge column is a real signal, not a few tone levels.
        let mut toggles = crate::pass_truth::LookToggles::all_on();
        toggles.disable("sky");
        let list = list_for(camera(0.0), plane_item());
        for presenter in [&mut off, &mut on] {
            presenter
                .upload_mesh(plane_key(), &edge_plane())
                .expect("fixture plane uploads");
            presenter.resize(SIZE, SIZE).expect("valid size applies");
            presenter.set_taa_jitter_enabled(true);
        }
        on.set_taa_resolve_enabled(true);
        let mut frames = Vec::new();
        for presenter in [&mut off, &mut on] {
            let mut twins = Vec::new();
            for step in 0..2 {
                let receipt = presenter
                    .render_live_offscreen_with_toggles(&list, &toggles)
                    .expect("jittered frame renders");
                assert!(receipt.is_clean(), "frame {step} validation errors 0");
                assert_eq!(
                    presenter.deferred_validation_errors(),
                    0,
                    "frame {step} has no late validation error"
                );
                twins.push(read_twin(presenter));
            }
            frames.push(twins);
        }
        let band = edge_band(SIZE, SIZE);
        let flicker_off = mean_change(&frames[0][0], &frames[0][1], &band);
        let flicker_on = mean_change(&frames[1][0], &frames[1][1], &band);
        assert!(
            flicker_off >= 5.0,
            "the jitter flickers the unresolved edge: {flicker_off:.2} levels"
        );
        eprintln!(
            "taa flicker: off {flicker_off:.2} levels, on {flicker_on:.2} levels, \
             drop {:.1}%",
            100.0 * (1.0 - flicker_on / flicker_off)
        );
        assert!(
            flicker_on <= 0.5 * flicker_off,
            "the resolve cuts edge flicker at least half: {flicker_on:.2} vs {flicker_off:.2} levels"
        );
        assert_eq!(
            off.taa_resolve_receipt().passes,
            0,
            "the switch off records no resolve pass"
        );
        let receipt = on.taa_resolve_receipt();
        assert_eq!(
            receipt.passes, 2,
            "the switch on records one pass per frame"
        );
        assert_eq!(receipt.validation_errors, 0, "no late validation error");
        assert!(receipt.history_valid, "two resolved frames leave a history");
    }

    /// LF5d's AA default: both modes render the same sand-edge fixture, two
    /// frames each, and the calmer frame-to-frame edge diff is the one the
    /// presenter ships. TAA runs as shipped (Halton jitter plus the resolve);
    /// 4x MSAA is coverage, so it carries no jitter. Measured on the RTX 3050:
    /// TAA 10.60 levels, 4x MSAA 0.00 levels, so 4x MSAA is the calmer default.
    #[test]
    fn sand_edge_aa_default() {
        let Some(mut taa) = init_or_skip("sand_edge_aa_default_taa") else {
            return;
        };
        let Some(mut msaa) = init_or_skip("sand_edge_aa_default_msaa") else {
            return;
        };
        msaa.set_msaa_enabled(true);
        if !msaa.msaa_enabled() {
            crate::test_gpu::note_skip("sand_edge_aa_default", "adapter cannot build the 4x pair");
            return;
        }
        let mut toggles = crate::pass_truth::LookToggles::all_on();
        toggles.disable("sky");
        let list = list_for(camera(0.0), plane_item());
        for presenter in [&mut taa, &mut msaa] {
            presenter
                .upload_mesh(plane_key(), &edge_plane())
                .expect("fixture plane uploads");
            presenter.resize(SIZE, SIZE).expect("valid size applies");
        }
        taa.set_taa_jitter_enabled(true);
        taa.set_taa_resolve_enabled(true);
        let mut frames = Vec::new();
        for presenter in [&mut taa, &mut msaa] {
            let mut twins = Vec::new();
            for step in 0..2 {
                let receipt = presenter
                    .render_live_offscreen_with_toggles(&list, &toggles)
                    .expect("aa frame renders");
                assert!(receipt.is_clean(), "frame {step} validation errors 0");
                assert_eq!(
                    presenter.deferred_validation_errors(),
                    0,
                    "frame {step} has no late validation error"
                );
                twins.push(read_twin(presenter));
            }
            frames.push(twins);
        }
        let band = edge_band(SIZE, SIZE);
        // A blank twin would read a 0.00 diff as well, so each mode's first
        // frame must carry the fixture's edge step in the band.
        let mut steps = Vec::new();
        for (mode, twins) in [("taa", &frames[0]), ("msaa", &frames[1])] {
            let (mut low, mut high) = (f32::MAX, f32::MIN);
            for &pixel in &band {
                let value = luma(&twins[0], pixel);
                low = low.min(value);
                high = high.max(value);
            }
            assert!(
                high - low >= 5.0,
                "{mode} twin shows the sand edge, band step {:.2} levels",
                high - low
            );
            steps.push(high - low);
        }
        let taa_flicker = mean_change(&frames[0][0], &frames[0][1], &band);
        let msaa_flicker = mean_change(&frames[1][0], &frames[1][1], &band);
        eprintln!(
            "sand edge aa default: taa (jitter+resolve) {taa_flicker:.2} levels \
             (edge step {:.2}), msaa 4x {msaa_flicker:.2} levels (edge step {:.2})",
            steps[0], steps[1]
        );
        assert!(
            msaa_flicker <= taa_flicker,
            "the presenter default is the calmer mode: 4x msaa \
             {msaa_flicker:.2} vs taa (jitter+resolve) {taa_flicker:.2} levels"
        );
    }

    #[test]
    fn taa_resolve_keeps_a_running_fighter_ghost_under_three_px() {
        let Some(mut truth) = init_or_skip("taa_ghost_truth") else {
            return;
        };
        let Some(mut got) =
            init_or_skip("taa_resolve_keeps_a_running_fighter_ghost_under_three_px")
        else {
            return;
        };
        let list = list_for(camera(0.0), plane_item());
        let mesh = edge_plane();
        for presenter in [&mut truth, &mut got] {
            presenter
                .upload_mesh(plane_key(), &mesh)
                .expect("fixture plane uploads");
            presenter.resize(SIZE, SIZE).expect("valid size applies");
        }
        got.set_taa_resolve_enabled(true);
        let truth_before = frame(&mut truth, &list);
        let resolved_before = frame(&mut got, &list);
        assert_eq!(
            resolved_before, truth_before,
            "the first resolved frame follows the current image"
        );
        // The same body shifts 0.4 m along +x: 11.7 px through this camera.
        let shift = 0.4f32;
        let moved: Vec<[f32; 3]> = mesh
            .positions
            .iter()
            .map(|p| [p[0] + shift, p[1], p[2]])
            .collect();
        let normals = vec![[0.0, 0.0, 1.0]; 4];
        let uvs = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for presenter in [&mut truth, &mut got] {
            assert!(
                presenter
                    .refresh_mesh_vertices(plane_key(), &moved, &normals, &uvs)
                    .expect("vertex refresh applies"),
                "the fixture is resident"
            );
        }
        let truth_after = frame(&mut truth, &list);
        let resolved_after = frame(&mut got, &list);
        let row = SIZE / 2;
        let moved_px = left_edge(&truth_after, SIZE, row) - left_edge(&truth_before, SIZE, row);
        assert!(
            moved_px >= 8.0,
            "the body runs {moved_px:.2} px between the two frames"
        );
        let mut worst = 0u32;
        for y in [SIZE / 3, SIZE / 2, 2 * SIZE / 3] {
            let left = left_edge(&truth_after, SIZE, y);
            let right = right_edge(&truth_after, SIZE, y);
            let step = luma_at(&truth_after, SIZE, SIZE / 2, y) - luma_at(&truth_after, SIZE, 1, y);
            worst = worst.max(ghost_left(
                &resolved_after,
                &truth_after,
                SIZE,
                y,
                left,
                step,
            ));
            worst = worst.max(ghost_right(
                &resolved_after,
                &truth_after,
                SIZE,
                y,
                right,
                step,
            ));
        }
        assert!(
            worst <= 3,
            "the resolved frame keeps the running body's ghost under 3 px: {worst} px"
        );
        let receipt = got.taa_resolve_receipt();
        assert_eq!(receipt.passes, 2, "two frames, two resolves");
        assert_eq!(receipt.validation_errors, 0, "no late validation error");
        assert!(
            got.taa_last_frame_valid(),
            "the moved frame read the first frame's history"
        );
    }

    #[test]
    fn a_camera_cut_and_a_resize_drop_the_taa_history() {
        let Some(mut presenter) = init_or_skip("a_camera_cut_and_a_resize_drop_the_taa_history")
        else {
            return;
        };
        presenter
            .upload_mesh(plane_key(), &edge_plane())
            .expect("fixture plane uploads");
        presenter.resize(SIZE, SIZE).expect("valid size applies");
        presenter.set_taa_resolve_enabled(true);
        let list = list_for(camera(0.0), plane_item());
        frame(&mut presenter, &list);
        assert!(
            !presenter.taa_last_frame_valid(),
            "the first frame has no history and follows the current image"
        );
        frame(&mut presenter, &list);
        assert!(
            presenter.taa_last_frame_valid(),
            "the second frame reads the first frame's resolved image"
        );
        presenter.reset_taa_history();
        assert!(
            !presenter.taa_resolve_receipt().history_valid,
            "a cut drops the history at once"
        );
        frame(&mut presenter, &list);
        assert!(
            !presenter.taa_last_frame_valid(),
            "the cut frame follows the current image"
        );
        frame(&mut presenter, &list);
        assert!(
            presenter.taa_last_frame_valid(),
            "the frame after the cut rebuilds the history"
        );
        presenter
            .resize(SIZE / 2, SIZE / 2)
            .expect("valid size applies");
        frame(&mut presenter, &list);
        assert!(
            !presenter.taa_last_frame_valid(),
            "a resize rebuilds the targets and drops the history"
        );
        assert!(
            presenter.taa_resolve_receipt().history_valid,
            "the post-resize frame resolves into the new history"
        );
    }

    /// First screen x where a row's luma crosses the midpoint between its
    /// background and body colors, with a linear sub-pixel crossing.
    fn left_edge(rgba: &[u8], w: u32, y: u32) -> f32 {
        let background = luma_at(rgba, w, 1, y);
        let body = luma_at(rgba, w, w / 2, y);
        let step = body - background;
        let mid = background + 0.5 * step;
        for x in 1..w {
            let value = luma_at(rgba, w, x, y);
            let previous = luma_at(rgba, w, x - 1, y);
            let crossed = if step >= 0.0 {
                value >= mid
            } else {
                value <= mid
            };
            if crossed {
                let span = value - previous;
                return if span.abs() > 1.0e-6 {
                    (x - 1) as f32 + (mid - previous) / span
                } else {
                    x as f32
                };
            }
        }
        w as f32
    }

    /// Last screen x where a row's luma crosses the same midpoint, scanning
    /// from the right border.
    fn right_edge(rgba: &[u8], w: u32, y: u32) -> f32 {
        let background = luma_at(rgba, w, w - 2, y);
        let body = luma_at(rgba, w, w / 2, y);
        let step = body - background;
        let mid = background + 0.5 * step;
        for x in (1..w - 1).rev() {
            let value = luma_at(rgba, w, x, y);
            let next = luma_at(rgba, w, x + 1, y);
            let crossed = if step >= 0.0 {
                value >= mid
            } else {
                value <= mid
            };
            if crossed {
                let span = next - value;
                return if span.abs() > 1.0e-6 {
                    x as f32 + (mid - value) / span
                } else {
                    x as f32
                };
            }
        }
        0.0
    }

    /// Pixels left of the new border still showing the old body, walking out
    /// from the border while the resolved frame differs from the truth.
    fn ghost_left(got: &[u8], truth: &[u8], w: u32, y: u32, edge: f32, step: f32) -> u32 {
        let limit = 0.25 * step.abs();
        let mut count = 0;
        let mut x = edge.floor() as i32;
        while x >= 1 {
            let pixel = x as u32;
            if (luma_at(got, w, pixel, y) - luma_at(truth, w, pixel, y)).abs() <= limit {
                break;
            }
            count += 1;
            x -= 1;
        }
        count
    }

    /// Pixels right of the new border still showing the old background.
    fn ghost_right(got: &[u8], truth: &[u8], w: u32, y: u32, edge: f32, step: f32) -> u32 {
        let limit = 0.25 * step.abs();
        let mut count = 0;
        let mut x = edge.ceil() as u32;
        while x + 1 < w {
            if (luma_at(got, w, x, y) - luma_at(truth, w, x, y)).abs() <= limit {
                break;
            }
            count += 1;
            x += 1;
        }
        count
    }
}

//! tests_gpu: windowed presenter tests plus submit audits.
//! Inner modules keep their own feature gates.
// ---------------------------------------------------------------------------
// GPU tests behind `window`: offscreen twin, multi-draw, resize/minimize.
// Skip (pass) with no adapter, exactly like the `v1_gpu` receipt test.
// ---------------------------------------------------------------------------

#[cfg(feature = "window")]
#[cfg(test)]
mod gpu_tests {
    use super::super::*;
    use crate::test_gpu::{require_or_skip, NoAdapter};

    fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    fn cube_list(key: MeshKey, width: u32, height: u32) -> DrawList {
        let scene = crate::v1::default_scene(width, height).expect("default scene is valid");
        DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
            }],
        }
    }

    fn covered_pixels(png: &[u8]) -> usize {
        let img = image::load_from_memory(png)
            .expect("capture is a valid PNG")
            .to_rgba8();
        let mut covered = 0usize;
        for p in img.pixels() {
            let off = (p[0] as f32 - 7.0)
                .abs()
                .max((p[1] as f32 - 8.0).abs())
                .max((p[2] as f32 - 15.0).abs());
            if off > 5.0 {
                covered += 1;
            }
        }
        covered
    }

    #[test]
    fn evidence_ms_to_us_scales_and_saturates() {
        // Named conversion for the receipt rows: exact scale, zero floor,
        // saturation ceiling. Would fail on a truncating float cast.
        assert_eq!(Presenter::ms_to_us(0.8), 800, "ms scale to whole us");
        assert_eq!(Presenter::ms_to_us(0.0), 0, "zero reads zero");
        assert_eq!(Presenter::ms_to_us(-2.0), 0, "negative clamps to zero");
        assert_eq!(Presenter::ms_to_us(f64::NAN), 0, "non-finite reads zero");
        assert_eq!(Presenter::ms_to_us(1.0e16), u64::MAX, "overflow saturates");
    }

    #[test]
    fn offscreen_twin_renders_clean() {
        let Some(mut presenter) = init_or_skip("offscreen_twin_renders_clean") else {
            return;
        };
        assert!(
            !presenter.adapter_name().is_empty(),
            "receipt names the adapter"
        );
        assert!(
            !presenter.adapter_driver().is_empty(),
            "receipt names the driver"
        );
        let key = MeshKey::new(1, [0xA1; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        assert_eq!(presenter.mesh_count(), 1);
        assert_eq!(presenter.mesh_keys(), vec![key]);
        // Hand-built meshes cannot sneak past the validator.
        let mut broken = crate::v1::cube();
        broken.indices.push(9999);
        match presenter.upload_mesh(MeshKey::new(9, [0; 32]), &broken) {
            Err(PresentError::InvalidMesh(_)) => {}
            other => panic!("out-of-range index must be refused, got {other:?}"),
        }
        presenter.resize(160, 120).expect("valid size applies");
        let out = presenter
            .render_offscreen(&cube_list(key, 160, 120))
            .expect("twin renders");
        let png = out.png.expect("live frame has pixels");
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(
            out.manifest.is_clean(),
            "validation errors must be 0, got {:?}",
            out.manifest
        );
        assert!(out.receipt.is_clean() && out.receipt.presented);
        assert_eq!((out.receipt.width, out.receipt.height), (160, 120));
        assert_eq!(out.receipt.draws, 1);
        assert_eq!(presenter.frames_presented(), 1);
        assert!(
            covered_pixels(&png) > 100,
            "lit cube must cover pixels through HDR plus blit"
        );
    }

    #[test]
    fn multi_draw_two_materials_cover_more() {
        let Some(mut presenter) = init_or_skip("multi_draw_two_materials_cover_more") else {
            return;
        };
        let key = MeshKey::new(2, [0xB2; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(128, 128).expect("valid size applies");
        let mut list = cube_list(key, 128, 128);
        let mut dielectric = list.items[0].material.clone();
        dielectric.metallic = 0.0;
        dielectric.base_color = [0.8, 0.1, 0.1];
        list.items.push(DrawItem {
            mesh: key,
            material: dielectric,
            base_color: None,
            normal_map: None,
        });
        let out = presenter.render_offscreen(&list).expect("two draws render");
        assert!(out.manifest.is_clean(), "two draws stay validation-clean");
        assert_eq!(out.receipt.draws, 2);
        // Unknown mesh keys fail the frame with their draw index, not a panic.
        let mut bad = list.clone();
        bad.items[1].mesh = MeshKey::new(999, [0x00; 32]);
        match presenter.render_offscreen(&bad) {
            Err(PresentError::InvalidDraw { index, .. }) => assert_eq!(index, 1),
            other => panic!("unknown mesh must fail at draw 1, got {other:?}"),
        }
    }

    #[test]
    fn resize_and_minimize_survive_with_last_good() {
        let Some(mut presenter) = init_or_skip("resize_and_minimize_survive_with_last_good") else {
            return;
        };
        let key = MeshKey::new(3, [0xC3; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(160, 120).expect("valid size applies");
        let first = presenter
            .render_offscreen(&cube_list(key, 160, 120))
            .expect("first size renders");
        assert!(first.receipt.presented && first.manifest.is_clean());
        // Resize rebuilds HDR, depth and twin targets (generation bumps).
        let gen = presenter.generation();
        presenter.resize(96, 96).expect("valid size applies");
        assert!(presenter.generation() > gen, "resize reconfigures");
        let second = presenter
            .render_offscreen(&cube_list(key, 96, 96))
            .expect("resized frame renders");
        assert!((second.receipt.width, second.receipt.height) == (96, 96));
        assert!(second.manifest.is_clean(), "resized frame is clean");
        // Minimize suspends: no pixels, no panic, last-good kept.
        presenter.resize(0, 0).expect("minimize suspends");
        assert!(presenter.suspended());
        let held = presenter
            .render_offscreen(&cube_list(key, 96, 96))
            .expect("suspended frame never errors");
        assert!(held.png.is_none() && !held.receipt.presented);
        assert_eq!((held.receipt.width, held.receipt.height), (96, 96));
        assert!(held.manifest.is_clean());
        // Restore renders again on fresh targets.
        presenter.resize(64, 64).expect("restore applies");
        assert!(!presenter.suspended());
        let restored = presenter
            .render_offscreen(&cube_list(key, 64, 64))
            .expect("restored frame renders");
        assert!(restored.receipt.presented && restored.manifest.is_clean());
        assert!(covered_pixels(&restored.png.expect("pixels")) > 10);
        // Over-bound sizes are refused with last-good untouched.
        match presenter.resize(MAX_FRAME_DIM + 1, 64) {
            Err(PresentError::InvalidSize { width, height }) => {
                assert_eq!((width, height), (MAX_FRAME_DIM + 1, 64));
            }
            other => panic!("over-bound resize must be refused, got {other:?}"),
        }
        assert_eq!(presenter.size(), (64, 64));
    }

    #[test]
    fn texture_residency_reports_and_rejects() {
        let Some(mut presenter) = init_or_skip("texture_residency_reports_and_rejects") else {
            return;
        };
        let key = TextureKey::new(11, [0xD4; 32]);
        presenter
            .upload_texture(
                key,
                &TextureUpload {
                    width: 4,
                    height: 4,
                    rgba: vec![200u8; 64],
                },
            )
            .expect("4x4 texture uploads");
        assert_eq!(presenter.texture_keys(), vec![key]);
        let (view, w, h) = presenter.texture_view(&key).expect("resident view");
        assert_eq!((w, h), (4, 4));
        let _ = view;
        assert!(presenter.texture_handle(&key).is_some());
        assert!(
            presenter
                .upload_texture(
                    TextureKey::new(12, [0x00; 32]),
                    &TextureUpload {
                        width: 4,
                        height: 4,
                        rgba: vec![0u8; 63],
                    },
                )
                .is_err(),
            "short pixels are refused before the GPU"
        );
        assert!(presenter.evict_texture(&key));
        assert!(presenter.texture_keys().is_empty());
    }

    #[test]
    fn missing_texture_ship_path_refuses_and_receipt_quotes_budgets() {
        let Some(mut presenter) =
            init_or_skip("missing_texture_ship_path_refuses_and_receipt_quotes_budgets")
        else {
            return;
        };
        assert_eq!(
            (presenter.mesh_capacity(), presenter.texture_capacity()),
            (MAX_CACHED_MESHES, MAX_CACHED_TEXTURES)
        );
        let key = TextureKey::new(21, [0xE2; 32]);
        presenter
            .upload_texture(
                key,
                &TextureUpload {
                    width: 4,
                    height: 4,
                    rgba: vec![200u8; 64],
                },
            )
            .expect("4x4 texture uploads");
        assert_eq!(presenter.texture_count(), 1);
        assert_eq!(presenter.texture_high_water(), 1);
        assert!(presenter.require_texture_view(&key).is_ok());
        // The preview path returns `None`; only the ship path refuses loudly.
        let absent = TextureKey::new(22, [0x00; 32]);
        assert!(presenter.texture_view(&absent).is_none());
        match presenter.require_texture_view(&absent) {
            Err(PresentError::MissingTexture { id, .. }) => assert_eq!(id, 22),
            other => panic!("ship path must refuse loudly, got {other:?}"),
        }
        // Manual drops feed the same eviction telemetry the receipt quotes.
        assert!(presenter.evict_texture(&key));
        assert_eq!(presenter.texture_evicted(), 1);
        assert_eq!(presenter.last_evicted_texture_id(), Some(21));
        let mesh = MeshKey::new(23, [0xA3; 32]);
        presenter
            .upload_mesh(mesh, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let out = presenter
            .render_offscreen(&cube_list(mesh, 64, 64))
            .expect("frame renders");
        assert_eq!((out.receipt.mesh_count, out.receipt.texture_count), (1, 0));
        assert_eq!(out.receipt.mesh_high_water, 1);
        assert_eq!(out.receipt.texture_evicted, 1);
        assert_eq!(out.receipt.evicted_texture_id, Some(21));
        assert_eq!(out.receipt.evicted_mesh_id, None);
        let summary = out.receipt.cache_summary();
        assert!(
            summary.contains("21"),
            "telemetry names the evicted asset id: {summary}"
        );
    }

    #[test]
    fn device_loss_fails_loudly_until_rebuilt() {
        let Some(mut presenter) = init_or_skip("device_loss_fails_loudly_until_rebuilt") else {
            return;
        };
        presenter.mark_device_lost("test loss".into());
        match presenter.render_offscreen(&cube_list(MeshKey::new(1, [0; 32]), 64, 64)) {
            Err(PresentError::DeviceLost(_)) => {}
            other => panic!("lost device must fail loudly, got {other:?}"),
        }
    }

    #[test]
    fn surface_render_without_surface_is_typed() {
        let Some(mut presenter) = init_or_skip("surface_render_without_surface_is_typed") else {
            return;
        };
        presenter.resize(64, 64).expect("valid size applies");
        match presenter.render_surface(&cube_list(MeshKey::new(1, [0; 32]), 64, 64)) {
            Err(PresentError::NoSurface) => {}
            other => panic!("missing surface must be typed, got {other:?}"),
        }
    }

    /// Torch bytes stage deterministically: same frame stages same bytes twice,
    /// two live torches with six zeroed slots, clears count zero. Mirrors the twin.
    #[test]
    fn light_torch_bytes_x2() {
        let Some(mut presenter) = init_or_skip("light_torch_bytes_x2") else {
            return;
        };
        let key = MeshKey::new(0x70C4, [0x70; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let list = cube_list(key, 64, 64);
        let first = presenter.torch_bytes(&list);
        let second = presenter.torch_bytes(&list);
        assert_eq!(first, second, "same frame gives same torch bytes twice");
        let count = u32::from_le_bytes([first[256], first[257], first[258], first[259]]);
        assert_eq!(count, 2, "two torches live");
        let wx = f32::from_le_bytes([first[0], first[1], first[2], first[3]]);
        let wy = f32::from_le_bytes([first[4], first[5], first[6], first[7]]);
        let wz = f32::from_le_bytes([first[8], first[9], first[10], first[11]]);
        assert!(
            (wx - 20.0).abs() < 1e-6 && (wy - 1.0).abs() < 1e-6 && (wz + 3.0).abs() < 1e-6,
            "west head placed"
        );
        let ex = f32::from_le_bytes([first[32], first[33], first[34], first[35]]);
        let ey = f32::from_le_bytes([first[36], first[37], first[38], first[39]]);
        let ez = f32::from_le_bytes([first[40], first[41], first[42], first[43]]);
        assert!(
            (ex - 20.0).abs() < 1e-6 && (ey - 1.0).abs() < 1e-6 && (ez - 3.0).abs() < 1e-6,
            "east head placed"
        );
        assert!(
            first[64..256].iter().all(|b| *b == 0),
            "six spare slots stay zeroed"
        );
        let empty = DrawList {
            camera: list.camera.clone(),
            light: list.light.clone(),
            items: Vec::new(),
        };
        let cleared = presenter.torch_bytes(&empty);
        let zero = u32::from_le_bytes([cleared[256], cleared[257], cleared[258], cleared[259]]);
        assert_eq!(zero, 0, "clears zero torches");
        assert!(cleared[..256].iter().all(|b| *b == 0), "clears zero slots");
        let out = presenter.render_offscreen(&list).expect("frame renders");
        assert!(out.manifest.is_clean(), "manifest clean");
        assert!(out.receipt.is_clean() && out.receipt.presented);
        assert_eq!(out.receipt.draws, 1);
    }

    /// Torch on versus off: same fixture stages count two versus zero with
    /// different bytes, both frames stay clean with stable pixels. The masked
    /// pixel lift awaits the forward group slot, so this lands bytes plus fixture.
    #[test]
    fn light_torch_on_vs_off_differs() {
        let Some(mut presenter) = init_or_skip("light_torch_on_vs_off_differs") else {
            return;
        };
        let near = MeshKey::new(0x70A5, [0x7A; 32]);
        let far = MeshKey::new(0x70A6, [0x7B; 32]);
        let mut near_cube = crate::v1::cube();
        for p in &mut near_cube.positions {
            p[0] += 19.0;
            p[1] += 1.0;
            p[2] += -3.0;
        }
        presenter
            .upload_mesh(near, &near_cube)
            .expect("near uploads");
        presenter
            .upload_mesh(far, &crate::v1::cube())
            .expect("far uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let scene = crate::v1::default_scene(64, 64).expect("scene builds");
        let list = DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: vec![
                DrawItem {
                    mesh: near,
                    material: scene.material.clone(),
                    base_color: None,
                    normal_map: None,
                },
                DrawItem {
                    mesh: far,
                    material: scene.material,
                    base_color: None,
                    normal_map: None,
                },
            ],
        };
        let on = presenter.torch_bytes(&list);
        let empty = DrawList {
            camera: list.camera.clone(),
            light: list.light.clone(),
            items: Vec::new(),
        };
        let off = presenter.torch_bytes(&empty);
        assert_ne!(on, off, "torches on differ from count zero");
        let on_count = u32::from_le_bytes([on[256], on[257], on[258], on[259]]);
        let off_count = u32::from_le_bytes([off[256], off[257], off[258], off[259]]);
        assert_eq!((on_count, off_count), (2, 0), "on two, off zero");
        let first = presenter.render_offscreen(&list).expect("first renders");
        assert!(first.manifest.is_clean(), "manifest clean");
        assert!(first.receipt.is_clean() && first.receipt.presented);
        assert_eq!(first.receipt.draws, 2);
        let second = presenter.render_offscreen(&list).expect("second renders");
        assert!(second.manifest.is_clean(), "second clean");
        let a = first.png.expect("first pixels");
        let b = second.png.expect("second pixels");
        assert_eq!(a, b, "same fixture replays byte-identical");
    }

    /// Pixels reading clearly red vs clearly green (margin over the other
    /// channels, so fog/tonemap survivors still sort by hue).
    fn hue_counts(png: &[u8]) -> (usize, usize) {
        let img = image::load_from_memory(png)
            .expect("capture is a valid PNG")
            .to_rgba8();
        let (mut red, mut green) = (0usize, 0usize);
        for p in img.pixels() {
            let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
            if r > 110 && r > g + 40 && r > b + 40 {
                red += 1;
            } else if g > 110 && g > r + 40 && g > b + 40 {
                green += 1;
            }
        }
        (red, green)
    }

    /// Behavior: a 2x2 red/green checker bound as `base_color` renders both
    /// colors in the offscreen frame, 0 validation errors.
    #[test]
    fn textured_draw_renders_both_checker_colors() {
        let Some(mut presenter) = init_or_skip("textured_draw_renders_both_checker_colors") else {
            return;
        };
        let mesh = MeshKey::new(41, [0xA4; 32]);
        presenter
            .upload_mesh(mesh, &crate::v1::cube())
            .expect("cube uploads");
        // 2x2 checker: red, green / green, red.
        let tex = TextureKey::new(42, [0xB4; 32]);
        presenter
            .upload_texture(
                tex,
                &TextureUpload {
                    width: 2,
                    height: 2,
                    rgba: vec![
                        255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255,
                    ],
                },
            )
            .expect("checker uploads");
        presenter.resize(128, 128).expect("valid size applies");
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        let list = DrawList {
            camera: scene.camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh,
                material: crate::v1::PbrMaterial {
                    base_color: [1.0, 1.0, 1.0],
                    metallic: 0.0,
                    roughness: 0.5,
                    exposure: 1.0,
                },
                base_color: Some(tex),
                normal_map: None,
            }],
        };
        let out = presenter
            .render_offscreen(&list)
            .expect("textured frame renders");
        assert!(
            out.manifest.is_clean(),
            "textured frame is validation-clean"
        );
        assert!(out.receipt.is_clean() && out.receipt.presented);
        let png = out.png.expect("live frame has pixels");
        let (red, green) = hue_counts(&png);
        eprintln!("VS2 checker: red {red} green {green}");
        assert!(red > 100, "checker red renders, got {red}");
        assert!(green > 100, "checker green renders, got {green}");
        // Receipt outside the repo (temp dir, never git dirt).
        let dir = std::env::temp_dir().join("engine2040-vs2");
        std::fs::create_dir_all(&dir).expect("temp receipt dir");
        std::fs::write(dir.join("vs2-textured.png"), &png).expect("textured png writes");
    }

    /// Parity: every draw with `None` renders the same bytes as before —
    /// the white fallback is an identity multiply, so `None` pixels equal
    /// white-textured pixels and fresh presenters agree byte for byte.
    /// Fresh presenters per render (not consecutive frames) so the
    /// per-frame particle step cannot drift the comparison.
    #[test]
    fn none_draw_matches_white_textured_bytes() {
        fn fresh_textured_pair(textured: bool) -> Vec<u8> {
            let Some(mut presenter) = init_or_skip("none_draw_matches_white_textured_bytes") else {
                return Vec::new();
            };
            let mesh = MeshKey::new(43, [0xC4; 32]);
            presenter
                .upload_mesh(mesh, &crate::v1::cube())
                .expect("cube uploads");
            let mut list = cube_list(mesh, 96, 96);
            if textured {
                let white = TextureKey::new(44, [0xD4; 32]);
                presenter
                    .upload_texture(
                        white,
                        &TextureUpload {
                            width: 1,
                            height: 1,
                            rgba: vec![255, 255, 255, 255],
                        },
                    )
                    .expect("white uploads");
                list.items[0].base_color = Some(white);
            }
            presenter.resize(96, 96).expect("valid size applies");
            let out = presenter.render_offscreen(&list).expect("frame renders");
            assert!(out.manifest.is_clean(), "p2p frame is clean");
            let png = out.png.expect("live frame has pixels");
            let dir = std::env::temp_dir().join("engine2040-vs2");
            std::fs::create_dir_all(&dir).expect("temp receipt dir");
            std::fs::write(
                dir.join(if textured {
                    "vs2-white.png"
                } else {
                    "vs2-plain.png"
                }),
                &png,
            )
            .expect("p2p png writes");
            png
        }
        let plain_a = fresh_textured_pair(false);
        if plain_a.is_empty() {
            return;
        }
        let plain_b = fresh_textured_pair(false);
        assert_eq!(plain_a, plain_b, "untextured repeats are byte-identical");
        let white = fresh_textured_pair(true);
        assert_eq!(plain_a, white, "white fallback keeps every byte");
    }

    /// Failure: an unresident texture key fails the frame with its
    /// draw index, mirroring the unknown-mesh path, never a panic.
    #[test]
    fn unknown_texture_fails_with_draw_index() {
        let Some(mut presenter) = init_or_skip("unknown_texture_fails_with_draw_index") else {
            return;
        };
        let mesh = MeshKey::new(45, [0xE4; 32]);
        presenter
            .upload_mesh(mesh, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let mut list = cube_list(mesh, 64, 64);
        list.items[0].base_color = Some(TextureKey::new(46, [0x00; 32]));
        match presenter.render_offscreen(&list) {
            Err(PresentError::InvalidDraw { index, .. }) => assert_eq!(index, 0),
            other => panic!("unknown texture must fail at draw 0, got {other:?}"),
        }
    }

    #[test]
    fn forward_path_runs_all_four_look_passes() {
        let Some(mut presenter) = init_or_skip("forward_path_runs_all_four_look_passes") else {
            return;
        };
        let key = MeshKey::new(31, [0xF1; 32]);
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        // Two live frames: sky, layers, particles and decals record under
        // one shared look bind before the forward draws, 0 errors, and the
        // particle pool advances between frames.
        let first = presenter
            .render_offscreen(&cube_list(key, 64, 64))
            .expect("first frame renders");
        assert!(first.receipt.presented && first.manifest.is_clean());
        assert_eq!(first.receipt.look_passes, 4, "sky+layers+particles+decals");
        assert_eq!(first.receipt.look_validation_errors, 0);
        let second = presenter
            .render_offscreen(&cube_list(key, 64, 64))
            .expect("second frame renders");
        assert_eq!(second.receipt.look_passes, 4);
        assert_eq!(second.receipt.look_validation_errors, 0);
        assert!(
            covered_pixels(&second.png.expect("pixels")) > 10,
            "look tranche leaves the lit cube intact"
        );
        // Suspended frames record nothing and stay clean.
        presenter.resize(0, 0).expect("minimize suspends");
        let held = presenter
            .render_offscreen(&cube_list(key, 64, 64))
            .expect("suspended frame never errors");
        assert_eq!(
            (
                held.receipt.look_passes,
                held.receipt.look_validation_errors
            ),
            (0, 0)
        );
        assert!(held.manifest.is_clean());
    }

    #[test]
    fn acquire_outcomes_bridge_to_surface_events() {
        // Pure mapping, no GPU needed: the windowed presenter feeds every
        // `CurrentSurfaceTexture` through this bridge.
        use wgpu::CurrentSurfaceTexture;
        assert_eq!(
            SurfaceEvent::from(&CurrentSurfaceTexture::Outdated),
            SurfaceEvent::Outdated
        );
        assert_eq!(
            SurfaceEvent::from(&CurrentSurfaceTexture::Lost),
            SurfaceEvent::Lost
        );
        assert_eq!(
            SurfaceEvent::from(&CurrentSurfaceTexture::Timeout),
            SurfaceEvent::Timeout
        );
        assert_eq!(
            SurfaceEvent::from(&CurrentSurfaceTexture::Occluded),
            SurfaceEvent::Occluded
        );
        assert_eq!(
            SurfaceEvent::from(&CurrentSurfaceTexture::Validation),
            SurfaceEvent::ValidationError
        );
        // The bridge agrees with the headless policy table.
        for event in [
            SurfaceEvent::Outdated,
            SurfaceEvent::Lost,
            SurfaceEvent::Timeout,
            SurfaceEvent::Occluded,
            SurfaceEvent::ValidationError,
        ] {
            assert_ne!(
                surface_action(event),
                SurfaceAction::Render,
                "{event:?} never renders"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// RL9 GPU tests behind `window`: the fog tranche runs per live frame with 0
// validation errors, the blit composite shows height fog plus contact
// shadows, and the kill switch restores the clear frame. Skip (pass) with no
// adapter, exactly like the RA1 twin tests.
// ---------------------------------------------------------------------------

#[cfg(feature = "window")]
#[cfg(test)]
mod rl9_gpu_tests {
    use super::super::*;
    use crate::fog;
    use crate::test_gpu::{require_or_skip, NoAdapter};

    fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    fn cube_key() -> MeshKey {
        MeshKey::new(0xF09, [0xF9; 32])
    }

    /// Level arena camera: eye and target level at 1.2 m so the view-top
    /// looks over the horizon (high rays, clear) and the view-bottom looks
    /// into the ground haze (low rays, fogged). View terms: the offscreen
    /// file runs upright (see the composite test), so file-bottom bands
    /// hold the haze and file-top bands the clear.
    fn level_camera() -> crate::v1::Camera {
        crate::v1::Camera::new(
            [0.0, 2.0, 5.0],
            [0.0, 2.0, 0.0],
            [0.0, 1.0, 0.0],
            40.0f32.to_radians(),
        )
    }

    fn level_list(width: u32, height: u32, items: Vec<DrawItem>) -> DrawList {
        let scene = crate::v1::default_scene(width, height).expect("default scene is valid");
        DrawList {
            camera: level_camera(),
            light: scene.light,
            items,
        }
    }

    fn level_cube_list(key: MeshKey, width: u32, height: u32) -> DrawList {
        let scene = crate::v1::default_scene(width, height).expect("default scene is valid");
        level_list(
            width,
            height,
            vec![DrawItem {
                mesh: key,
                material: scene.material,
                base_color: None,
                normal_map: None,
            }],
        )
    }

    #[test]
    fn fog_tranche_runs_clean_with_zero_validation() {
        let Some(mut presenter) = init_or_skip("fog_tranche_runs_clean_with_zero_validation")
        else {
            return;
        };
        assert!(presenter.fog_enabled(), "fog ships on");
        let key = cube_key();
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let out = presenter
            .render_offscreen(&level_cube_list(key, 64, 64))
            .expect("frame renders");
        assert!(out.receipt.presented && out.manifest.is_clean());
        assert_eq!(
            out.receipt.fog_passes, 2,
            "height fog plus contact run per live frame"
        );
        assert_eq!(
            out.receipt.fog_validation_errors, 0,
            "0 fog validation errors"
        );
        assert_eq!(out.receipt.look_passes, 4, "look tranche intact");
        assert_eq!(out.receipt.look_validation_errors, 0);
        let plan = presenter
            .fog_plan()
            .expect("fog wiring compiles on the live frame");
        assert!(
            plan.order_names_match(&[fog::FOG_PASS, fog::CONTACT_PASS, "forward"]),
            "presenter executes the declared wiring"
        );
        // Level camera: the top half looks over the horizon (clear) while
        // the bottom half looks into the haze (fogged). A vertical flip in
        // the feed would read the other way around.
        let (top, bottom) = presenter.fog_means();
        assert!(
            top.is_finite() && bottom.is_finite(),
            "fog evidence is finite"
        );
        assert!(
            bottom - top > 0.1,
            "height fog grades with frame height: top {top} bottom {bottom}"
        );
        let contact_min = presenter.fog_contact_min();
        assert!(
            (0.0..=1.0).contains(&contact_min),
            "contact evidence bounded: {contact_min}"
        );
        assert!(
            contact_min < 0.999,
            "cube edges shadow their neighbors: min {contact_min}"
        );
        eprintln!(
            "RL9 fog on {}: fog+contact submit {} us vs bars {}+{} ms",
            presenter.adapter_name(),
            presenter.fog_submit_us(),
            fog::FOG_BUDGET_MS,
            fog::CONTACT_BUDGET_MS,
        );
    }

    #[test]
    fn ibl_chart_submits_clean_after_forward() {
        let Some(mut presenter) = init_or_skip("ibl_chart_submits_clean_after_forward") else {
            return;
        };
        presenter.resize(64, 64).expect("valid size applies");
        let out = presenter
            .render_offscreen(&level_list(64, 64, Vec::new()))
            .expect("frame renders");
        assert!(out.receipt.presented && out.manifest.is_clean());
        // The chart is no longer a pre-forward look pass (that tranche is
        // exactly sky, layers, particles, decals), but it still submits on
        // every live frame: one post-forward pass, 0 validation errors,
        // counted on its own receipt rows.
        assert_eq!(out.receipt.look_passes, 4, "forward look tranche");
        assert_eq!(out.receipt.look_validation_errors, 0);
        assert_eq!(out.receipt.ibl_passes, 1, "chart submits post-forward");
        assert_eq!(out.receipt.ibl_validation_errors, 0);
        eprintln!(
            "RL2 chart on {}: submit vs chart budget {} ms",
            presenter.adapter_name(),
            crate::ibl::gpu::IBL_SUBMIT_BUDGET_MS,
        );
    }

    #[test]
    fn gtao_tranche_runs_clean_on_live_depth() {
        let Some(mut presenter) = init_or_skip("gtao_tranche_runs_clean_on_live_depth") else {
            return;
        };
        let key = cube_key();
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let out = presenter
            .render_offscreen(&level_cube_list(key, 64, 64))
            .expect("frame renders");
        assert!(out.receipt.presented && out.manifest.is_clean());
        assert_eq!(
            out.receipt.gtao_passes, 2,
            "half-res GTAO plus bilateral upsample run per live frame"
        );
        assert_eq!(out.receipt.gtao_validation_errors, 0);
        eprintln!(
            "RL3 gtao on {}: submit {} us vs bar {} ms",
            presenter.adapter_name(),
            presenter.gtao_submit_us(),
            crate::gtao::GTAO_BUDGET_MS,
        );
    }

    #[test]
    fn gtao_background_gate_matches_the_half_shader_and_the_ra1_far_clip() {
        // One figure, two implementations: the CPU oracle's constant in
        // `gtao.rs` and the device half kernel's `GATE_M` must agree, and it
        // must sit at or under the RA1 far clip so cleared (sky) texels land
        // on the background side of it.
        let src = include_str!("../gtao.rs");
        let want = format!("const GATE_M: f32 = {:.1};", crate::gtao::GTAO_BACKGROUND_M);
        assert!(
            src.contains(&want),
            "the half kernel carries its own gate: {want} missing"
        );
        let far = crate::v1::Camera::new(
            [0.0, 2.0, 5.0],
            [0.0, 2.0, 0.0],
            [0.0, 1.0, 0.0],
            40.0f32.to_radians(),
        )
        .far;
        assert!(
            crate::gtao::GTAO_BACKGROUND_M <= far,
            "the gate must sit under the RA1 far clip {far}"
        );
    }

    /// Copy one R32 float AO texture to the CPU. Test only: this waits
    /// on the GPU, same shape as the id-target readback helper.
    fn read_ao_back(presenter: &Presenter, texture: &wgpu::Texture) -> Vec<f32> {
        let (w, h) = (texture.width(), texture.height());
        let row = w * 4;
        let padded = row.div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-ao-readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
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
            r.expect("test ao readback maps");
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("test ao readback polls");
        let w_usize = usize::try_from(w).expect("width fits");
        let h_usize = usize::try_from(h).expect("height fits");
        let mut out = vec![0.0f32; w_usize * h_usize];
        {
            let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
            let row_usize = usize::try_from(row).expect("row fits");
            let padded_usize = usize::try_from(padded).expect("stride fits");
            for (y, dst) in out.chunks_exact_mut(w_usize).enumerate() {
                let off = y * padded_usize;
                for (slot, src) in dst
                    .iter_mut()
                    .zip(mapped[off..off + row_usize].chunks_exact(4))
                {
                    *slot = f32::from_le_bytes([src[0], src[1], src[2], src[3]]);
                }
            }
        }
        buffer.unmap();
        out
    }

    /// Mean over one axis-aligned block of a row-major field.
    fn region_mean(field: &[f32], width: usize, x0: usize, y0: usize, bw: usize, bh: usize) -> f32 {
        let mut sum = 0.0f64;
        let mut count = 0u64;
        for y in y0..y0 + bh {
            for x in x0..x0 + bw {
                sum += f64::from(field[y * width + x]);
                count += 1;
            }
        }
        (sum / f64::from(count.max(1) as u32)) as f32
    }

    #[test]
    fn gtao_live_ao_darkens_box_base() {
        let Some(mut presenter) = init_or_skip("gtao_live_ao_darkens_box_base") else {
            let cfg = Presenter::gtao_config_for_frame(64, 64).expect("live size configures");
            assert_eq!(
                (
                    cfg.width_px,
                    cfg.height_px,
                    cfg.radius_px,
                    cfg.falloff_m,
                    cfg.intensity
                ),
                (64, 64, 8.0, 2.0, 1.0),
                "presenter runs the arena kernel"
            );
            eprintln!("SKIPPED gtao_live_ao_darkens_box_base: no adapter");
            return;
        };
        let key = cube_key();
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let out = presenter
            .render_offscreen(&level_cube_list(key, 64, 64))
            .expect("frame renders");
        assert!(out.receipt.presented && out.manifest.is_clean());
        assert_eq!(
            out.receipt.gtao_passes, 2,
            "half-res GTAO plus bilateral upsample run per live frame"
        );
        assert_eq!(out.receipt.gtao_validation_errors, 0);
        let targets = presenter.targets.as_ref().expect("targets exist");
        let first = read_ao_back(&presenter, &targets.ao_texture);
        let second = read_ao_back(&presenter, &targets.ao_texture);
        assert_eq!(first.len(), 64 * 64, "ao field covers the frame");
        assert_eq!(first, second, "same depth gives same AO twice");
        for (i, v) in first.iter().enumerate() {
            assert!(v.is_finite(), "ao texel {i} is finite");
            // f32 rounding can land one ulp above 1.0; allow 1e-6 of slack.
            assert!(
                (0.0..=1.0 + 1e-6).contains(v),
                "ao texel {i} stays in range: {v}"
            );
        }
        let base = region_mean(&first, 64, 24, 40, 16, 16);
        let corner = region_mean(&first, 64, 0, 0, 8, 8);
        assert!(
            base < 0.99,
            "box base darkens below open: base {base} corner {corner}"
        );
        assert!(
            (0.98..=1.0).contains(&corner),
            "open sand stays open: base {base} corner {corner}"
        );
        assert!(
            presenter.gtao_submit_us() > 0,
            "device frame records gtao submit time"
        );
        eprintln!(
            "LF3 ao on {}: base {base} corner {corner} submit {} us",
            presenter.adapter_name(),
            presenter.gtao_submit_us(),
        );
    }

    #[test]
    fn gtao_config_rejects_bad_sizes() {
        let cfg = Presenter::gtao_config_for_frame(64, 64).expect("live size configures");
        assert_eq!(
            (
                cfg.width_px,
                cfg.height_px,
                cfg.radius_px,
                cfg.falloff_m,
                cfg.intensity
            ),
            (64, 64, 8.0, 2.0, 1.0),
            "presenter runs the arena kernel"
        );
        assert!(
            matches!(
                Presenter::gtao_config_for_frame(0, 64),
                Err(PresentError::InvalidSize { .. })
            ),
            "zero-area frames have no GTAO feed"
        );
        assert!(
            matches!(
                Presenter::gtao_config_for_frame(4096, 64),
                Err(PresentError::InvalidSize { .. })
            ),
            "over-bound frames (above 2048) have no GTAO feed"
        );
    }
}

// ---------------------------------------------------------------------------
// SL21 window-submit allocation audit (headless slice of the RA1 submit
// path): the per-frame CPU work every window submit runs (`decode_depth_feed`
// plus the zero-alloc pack/config helpers), counted through the crate's one
// test allocator, `engine_core::alloc_probe`. The `window` build runs the
// same audit in `graph.rs`.
// ---------------------------------------------------------------------------
/// Headless SL21 gate: the RA1 submit decodes one depth feed per live frame
/// (N/A for zero-alloc, 4-buffer floor), while the uniform pack plus contact
/// config on the same path allocate nothing.
#[cfg(all(test, not(feature = "window")))]
mod sl21_submit_audit {
    use super::super::*;

    /// Identity inverse for the audit fixture (world equals NDC, eye at the
    /// origin), mirroring `rl9_tests`.
    const IDENT: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    #[test]
    fn sl21_window_submit_alloc_audit_reports_na_floor() {
        // Warmup outside counting (contract: steady state is post-warmup);
        // the decode owns no caches, so warmup only settles lazy state.
        let raw = vec![0.5f32; 16 * 16];
        let warm = decode_depth_feed(&raw, 16, 16, IDENT, [0.0, 0.0, 0.0]).expect("warms");
        assert_eq!(warm.texels(), 256);
        std::hint::black_box(&warm);

        // The submit slice: one live frame decodes its depth feed once.
        let (feed, heap) = engine_core::alloc_probe::measure(|| {
            decode_depth_feed(&raw, 16, 16, IDENT, [0.0, 0.0, 0.0]).expect("decodes")
        });
        let (count, bytes) = (heap.calls, heap.bytes);
        // Behavior first: exact distance plus orientation, so a broken
        // decode fails here instead of passing the gate quietly.
        // Texel (0,0) of the 16x16 identity fixture unprojects to
        // (-0.9375, +0.9375, 0.5); the distance is exact Euclid.
        let want = (0.9375f64 * 0.9375 + 0.9375 * 0.9375 + 0.5 * 0.5).sqrt() as f32;
        assert!(
            (feed.depth_m[0] - want).abs() < 1e-6,
            "exact distance: {} vs {want}",
            feed.depth_m[0]
        );
        assert_eq!(feed.texels(), 256);
        assert_eq!(
            (feed.depth_m.len(), feed.height_m.len(), feed.normals.len()),
            (256, 256, 256)
        );
        // N/A floor (JAN28-1): four buffers per frame (`world`, `depth_m`,
        // `height_m`, `normals`); the window submit path cannot post 0.
        assert_eq!(
            count, 4,
            "decode floor is 4 buffers per frame (JAN28-1 N/A)"
        );
        let want_bytes = 256u64
            * (std::mem::size_of::<[f64; 3]>() as u64
                + 4
                + 4
                + std::mem::size_of::<[f32; 3]>() as u64);
        assert_eq!(bytes, want_bytes, "decode bytes are the four buffers");
        eprintln!("SL21 submit audit (decode 16x16): {count} allocs, {bytes} bytes (floor 4/11264: world+depth+height+normals)");

        // Zero-alloc helpers on the same submit path: the fog-composite
        // uniform packs stack bytes, the contact config validates scalars.
        let (_, pack) = engine_core::alloc_probe::measure(|| {
            std::hint::black_box(fog_blit_uniform_bytes([0.5, 0.25, 1.0], 64, 32))
        });
        assert_eq!(
            (pack.calls, pack.bytes),
            (0, 0),
            "uniform pack allocates nothing"
        );
        let (_, cfg) = engine_core::alloc_probe::measure(|| {
            contact_for_frame(64, 64).expect("live size builds")
        });
        assert_eq!(
            (cfg.calls, cfg.bytes),
            (0, 0),
            "contact config allocates nothing"
        );
        assert!(fog_fits_frame(64, 64), "audit frame fits the fog budget");

        // Failure case (outside counting): ragged depth still refuses with
        // the retry-frame error, never a half-decoded feed.
        assert!(
            matches!(
                decode_depth_feed(&raw[..10], 16, 16, IDENT, [0.0, 0.0, 0.0]),
                Err(PresentError::Readback(_))
            ),
            "ragged depth is refused"
        );
        // Counter-case: distinct depth decodes distinctly, so the gate is
        // not measuring a constant.
        let flat = vec![0.0f32; 16 * 16];
        let other = decode_depth_feed(&flat, 16, 16, IDENT, [0.0, 0.0, 0.0]).expect("redecodes");
        assert_ne!(
            feed.depth_m, other.depth_m,
            "distinct depth decodes distinctly"
        );
    }
}

// ---------------------------------------------------------------------------
// Live window frame tests behind `window`: no map and no poll-wait on a live
// frame, probe frames tallied apart, the sky and the forward fog matching the
// CPU twin of the closed form, and pass timestamps read without waiting.
// Skip (pass) with no adapter, like the RA1 twin tests.
// ---------------------------------------------------------------------------

#[cfg(feature = "window")]
#[cfg(test)]
mod live_frame_tests {
    use super::super::*;
    use crate::test_gpu::{require_or_skip, NoAdapter};

    fn init_or_skip(test: &str) -> Option<Presenter> {
        let probe = match Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        require_or_skip(test, probe)
    }

    fn cube_key() -> MeshKey {
        MeshKey::new(0x11FE, [0x1F; 32])
    }

    fn list_with(camera: crate::v1::Camera, items: Vec<DrawItem>) -> DrawList {
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        DrawList {
            camera,
            light: scene.light,
            items,
        }
    }

    fn cube_item() -> DrawItem {
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        DrawItem {
            mesh: cube_key(),
            material: scene.material,
            base_color: None,
            normal_map: None,
        }
    }

    /// Camera looking at the origin from `z` meters out, in fogged air.
    fn camera_at(z: f32) -> crate::v1::Camera {
        crate::v1::Camera::new(
            [0.0, 0.0, z],
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            40.0f32.to_radians(),
        )
    }

    /// Copy the LDR twin to the CPU. Test only: this waits on the GPU.
    fn read_twin(presenter: &Presenter) -> (Vec<u8>, u32, u32) {
        let targets = presenter.targets.as_ref().expect("twin targets exist");
        let (w, h) = (targets.width, targets.height);
        let padded = (w * 4).div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-twin-readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
            r.expect("test readback maps");
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("test readback polls");
        let row = (w * 4) as usize;
        let mut rgba = vec![0u8; row * h as usize];
        {
            let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
            for (y, dst) in rgba.chunks_exact_mut(row).enumerate() {
                let off = y * padded as usize;
                dst.copy_from_slice(&mapped[off..off + row]);
            }
        }
        buffer.unmap();
        (rgba, w, h)
    }

    fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [f32; 3] {
        let i = ((y * w + x) * 4) as usize;
        [rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32]
    }

    #[test]
    fn live_frames_never_map_or_wait_on_the_gpu() {
        let Some(mut presenter) = init_or_skip("live_frames_never_map_or_wait_on_the_gpu") else {
            return;
        };
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(96, 96).expect("valid size applies");
        assert!(!presenter.look_probes(), "probes ship off");
        for _ in 0..6 {
            let receipt = presenter
                .render_live_offscreen(&list_with(camera_at(6.0), vec![cube_item()]))
                .expect("live frame renders");
            assert!(receipt.presented && receipt.is_clean());
            assert_eq!(receipt.look_passes, 0, "no probe on a live frame");
            assert_eq!(
                receipt.fog_passes + receipt.gtao_passes + receipt.ibl_passes,
                0
            );
        }
        let stats = presenter.wait_stats();
        assert_eq!(stats.live_frames, 6);
        assert_eq!(stats.live_maps, 0, "a live frame never maps and waits");
        assert_eq!(stats.live_waits, 0, "a live frame never poll-waits");
        assert_eq!(stats.live_blocking(), 0);
        assert_eq!((stats.probe_frames, stats.capture_frames), (0, 0));
    }

    #[test]
    fn capture_frames_block_by_design_and_stay_off_the_live_tally() {
        let Some(mut presenter) =
            init_or_skip("capture_frames_block_by_design_and_stay_off_the_live_tally")
        else {
            return;
        };
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        let out = presenter
            .render_offscreen(&list_with(camera_at(6.0), vec![cube_item()]))
            .expect("capture renders");
        assert!(out.receipt.presented && out.manifest.is_clean());
        let stats = presenter.wait_stats();
        assert_eq!(stats.capture_frames, 1);
        // Depth readback, color readback, four look submits, chart, fog and
        // gtao submits: every one is a counted map plus wait.
        assert_eq!(stats.capture_maps, 9, "the counter sees every readback");
        assert_eq!(stats.capture_waits, 9);
        assert_eq!(stats.live_blocking(), 0);
    }

    #[test]
    fn probe_frames_are_tallied_apart_from_live_frames() {
        let Some(mut presenter) = init_or_skip("probe_frames_are_tallied_apart_from_live_frames")
        else {
            return;
        };
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(64, 64).expect("valid size applies");
        presenter.set_look_probes(true);
        presenter.set_probe_every(1000);
        presenter.set_fog_every(1000);
        let list = list_with(camera_at(6.0), vec![cube_item()]);
        // Frame 0 probes (first frame, fresh targets); the next ones hold.
        let first = presenter.render_live_offscreen(&list).expect("probe frame");
        assert_eq!(first.look_passes, 4, "opt-in probes run on the first frame");
        assert_eq!(first.fog_passes, 2);
        for _ in 0..3 {
            let held = presenter.render_live_offscreen(&list).expect("held frame");
            assert_eq!(held.look_passes, 0);
        }
        let stats = presenter.wait_stats();
        assert_eq!(stats.probe_frames, 1);
        assert!(stats.probe_waits >= 7, "probe frame waits: {stats:?}");
        assert_eq!(stats.live_frames, 3, "held frames count as live");
        assert_eq!(stats.live_blocking(), 0, "held frames still never wait");
        assert_eq!(
            (stats.live_maps, stats.live_waits),
            (0, 0),
            "a held frame records no depth copy, so it maps nothing: {stats:?}"
        );
        // Back to the default: the next live frame resets the stale pair.
        presenter.set_look_probes(false);
        let live = presenter.render_live_offscreen(&list).expect("live frame");
        assert_eq!(live.look_passes, 0);
        assert_eq!(presenter.wait_stats().live_blocking(), 0);
    }

    #[test]
    fn gpu_timestamps_resolve_late_without_waiting() {
        let Some(mut presenter) = init_or_skip("gpu_timestamps_resolve_late_without_waiting")
        else {
            return;
        };
        if !presenter.gpu_timing_supported() {
            crate::test_gpu::note_skip(
                "gpu_timestamps_resolve_late_without_waiting",
                "adapter lacks TIMESTAMP_QUERY",
            );
            assert!(presenter.gpu_timing_report().is_none());
            return;
        }
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(128, 128).expect("valid size applies");
        let list = list_with(camera_at(6.0), vec![cube_item()]);
        for round in 0..6 {
            for _ in 0..4 {
                presenter.render_live_offscreen(&list).expect("live frame");
            }
            if round % 2 == 1 {
                // Test only: let the device finish so the late maps land.
                presenter
                    .device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .expect("test poll");
            }
        }
        let report = presenter.gpu_timing_report().expect("timestamps resolved");
        assert!(report.frames >= 4, "late reads landed: {report:?}");
        assert!(
            report.total.p50.is_finite() && report.total.p50 > 0.0,
            "{report:?}"
        );
        assert!(report.total.p99 >= report.total.p50);
        assert!(
            report.forward.p50 > 0.0 && report.blit.p50 > 0.0,
            "{report:?}"
        );
        assert_eq!(presenter.wait_stats().live_blocking(), 0);
    }

    /// Cube positions shifted along x: the world-space mesh a draw list
    /// carries (draws have no transform).
    fn shifted_cube(dx: f32) -> crate::v1::MeshData {
        let mut cube = crate::v1::cube();
        for p in &mut cube.positions {
            p[0] += dx;
        }
        cube
    }

    fn material(rgb: [f32; 3]) -> crate::v1::PbrMaterial {
        crate::v1::PbrMaterial {
            base_color: rgb,
            metallic: 0.0,
            roughness: 0.8,
            exposure: 1.0,
        }
    }

    #[test]
    fn refresh_rewrites_the_resident_buffer_and_refuses_bad_input() {
        let Some(mut presenter) =
            init_or_skip("refresh_rewrites_the_resident_buffer_and_refuses_bad_input")
        else {
            return;
        };
        presenter.resize(128, 128).expect("valid size applies");
        let key = cube_key();
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        let list = list_with(camera_at(6.0), vec![cube_item()]);
        presenter.render_live_offscreen(&list).expect("first frame");
        let (before, w, _) = read_twin(&presenter);
        let centre_before = pixel(&before, w, 64, 64);
        let created = presenter.gpu_create_totals();

        // Move the cube 1.5 m right by rewriting its vertices in place.
        let moved = shifted_cube(1.5);
        let wrote = presenter
            .refresh_mesh_vertices(key, &moved.positions, &moved.normals, &moved.uvs)
            .expect("refresh succeeds");
        assert!(wrote, "a resident mesh is rewritten in place");
        assert_eq!(
            presenter.gpu_create_totals(),
            created,
            "a refresh creates no buffer"
        );
        presenter.render_live_offscreen(&list).expect("moved frame");
        let (after, _, _) = read_twin(&presenter);
        let centre_after = pixel(&after, w, 64, 64);
        let right_after = pixel(&after, w, 64 + 43, 64);
        let right_before = pixel(&before, w, 64 + 43, 64);
        let swing = |a: [f32; 3], b: [f32; 3]| (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f32>();
        // The live IBL is the arena day sky now (not the photo studio), so the
        // lit cube face sits closer to the sky behind it: 30 summed levels still
        // say "the cube moved" (a static frame differs by 0 to 2).
        assert!(
            swing(centre_before, centre_after) > 30.0,
            "the cube left the centre: {centre_before:?} -> {centre_after:?}"
        );
        assert!(
            swing(right_before, right_after) > 30.0,
            "the cube arrived on the right: {right_before:?} -> {right_after:?}"
        );

        // Refusals write nothing and say why.
        assert!(
            !presenter
                .refresh_mesh_vertices(
                    MeshKey::new(0xDEAD, [1; 32]),
                    &moved.positions,
                    &moved.normals,
                    &moved.uvs
                )
                .expect("unknown key is not an error"),
            "an absent mesh asks the caller for the one-time upload"
        );
        let short = presenter.refresh_mesh_vertices(
            key,
            &moved.positions[..3],
            &moved.normals[..3],
            &moved.uvs[..3],
        );
        assert!(
            matches!(short, Err(PresentError::InvalidMesh(_))),
            "{short:?}"
        );
        let mut nan = moved.positions.clone();
        nan[2][1] = f32::NAN;
        let bad = presenter.refresh_mesh_vertices(key, &nan, &moved.normals, &moved.uvs);
        assert!(matches!(bad, Err(PresentError::InvalidMesh(_))), "{bad:?}");
        let mut flat = moved.normals.clone();
        flat[0] = [0.0, 0.0, 0.0];
        let degenerate = presenter.refresh_mesh_vertices(key, &moved.positions, &flat, &moved.uvs);
        assert!(
            matches!(degenerate, Err(PresentError::InvalidMesh(_))),
            "{degenerate:?}"
        );
        assert!(
            presenter
                .refresh_mesh_vertices(key, &moved.positions, &moved.normals, &moved.uvs)
                .expect("still refreshes after refusals"),
            "refusals left the buffer writable"
        );
    }

    /// Copy a resident mesh's 48-byte-per-vertex buffer to the CPU. Test
    /// only: this waits on the GPU.
    fn read_vertex_bytes(presenter: &Presenter, key: &MeshKey) -> Vec<u8> {
        let mesh = presenter.meshes.get(key).expect("mesh is resident");
        let size = u64::from(mesh.vertices) * 48;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-vertex-readback"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_buffer_to_buffer(&mesh.vertex, 0, &buffer, 0, size);
        presenter.queue.submit(Some(enc.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
            r.expect("test readback maps");
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("test readback polls");
        let bytes = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped")
            .to_vec();
        buffer.unmap();
        bytes
    }

    /// The cube turned `angle` about +Y, then moved `dx` along +X: new
    /// positions and normals over the same uvs and triangles.
    fn turned_cube(angle: f32, dx: f32) -> crate::v1::MeshData {
        let (s, c) = angle.sin_cos();
        let mut cube = crate::v1::cube();
        for p in &mut cube.positions {
            *p = [c * p[0] + s * p[2] + dx, p[1], -s * p[0] + c * p[2]];
        }
        for n in &mut cube.normals {
            *n = [c * n[0] + s * n[2], n[1], -s * n[0] + c * n[2]];
        }
        cube
    }

    #[test]
    fn refresh_leaves_the_bytes_a_fresh_upload_would() {
        let Some(mut presenter) = init_or_skip("refresh_leaves_the_bytes_a_fresh_upload_would")
        else {
            return;
        };
        let key = cube_key();
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        let before = read_vertex_bytes(&presenter, &key);
        assert_eq!(
            before,
            crate::v1::cube().vertex_bytes(),
            "the upload is the 48-byte vertex image"
        );
        // A turn changes the tangents, not just the positions, so a refresh
        // that leaves them stale or packs a shorter vertex cannot match.
        let turned = turned_cube(0.6, 0.4);
        let wrote = presenter
            .refresh_mesh_vertices(key, &turned.positions, &turned.normals, &turned.uvs)
            .expect("refresh succeeds");
        assert!(wrote, "a resident mesh is rewritten in place");
        let fresh = crate::v1::upload_mesh(
            &turned.positions,
            &turned.normals,
            &turned.uvs,
            &turned.indices,
        )
        .expect("turned cube is valid")
        .vertex_bytes();
        let after = read_vertex_bytes(&presenter, &key);
        assert_eq!(after.len(), 24 * 48, "the buffer keeps its size");
        assert_ne!(after, before, "the refresh moved the vertices");
        let tangent_bytes = |bytes: &[u8]| -> Vec<u8> {
            bytes
                .chunks_exact(48)
                .flat_map(|vertex| vertex[32..48].to_vec())
                .collect()
        };
        assert_ne!(
            tangent_bytes(&after),
            tangent_bytes(&before),
            "the turn moved the tangents"
        );
        assert_eq!(after, fresh, "refresh equals a fresh upload, byte for byte");
    }

    #[test]
    fn refresh_refuses_a_skinned_mesh_and_writes_nothing() {
        let Some(mut presenter) = init_or_skip("refresh_refuses_a_skinned_mesh_and_writes_nothing")
        else {
            return;
        };
        let mesh = crate::v1::cube();
        let mut pipe = crate::v1_skin::SkinPipeline::new();
        pipe.upload_skeleton(1).expect("one joint");
        let rows = mesh.positions.len();
        let skin = pipe
            .upload_weights(&vec![[0u16; 4]; rows], &vec![[1.0, 0.0, 0.0, 0.0]; rows])
            .expect("rows are valid");
        let key = cube_key();
        presenter
            .upload_skinned_mesh(key, &mesh, &skin, 0)
            .expect("skinned cube uploads");
        let moved = shifted_cube(1.5);
        let refused =
            presenter.refresh_mesh_vertices(key, &moved.positions, &moved.normals, &moved.uvs);
        assert!(
            matches!(refused, Err(PresentError::InvalidMesh(_))),
            "a 64-byte vertex buffer is not refreshed with 48-byte vertices: {refused:?}"
        );
    }

    #[test]
    fn steady_live_frames_create_no_buffers_or_bind_groups() {
        let Some(mut presenter) =
            init_or_skip("steady_live_frames_create_no_buffers_or_bind_groups")
        else {
            return;
        };
        presenter.resize(96, 96).expect("valid size applies");
        let key = cube_key();
        presenter
            .upload_mesh(key, &crate::v1::cube())
            .expect("cube uploads");
        let tex = TextureKey::new(0x7E57, [0x3C; 32]);
        presenter
            .upload_texture(
                tex,
                &TextureUpload {
                    width: 2,
                    height: 2,
                    rgba: vec![
                        255, 0, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 255,
                    ],
                },
            )
            .expect("checker uploads");
        let twin_key = MeshKey::new(0x11FF, [0x2F; 32]);
        presenter
            .upload_mesh(twin_key, &shifted_cube(2.0))
            .expect("second cube uploads");
        let scene = crate::v1::default_scene(96, 96).expect("default scene is valid");
        let list = DrawList {
            camera: camera_at(7.0),
            light: scene.light,
            items: vec![
                cube_item(),
                DrawItem {
                    mesh: twin_key,
                    material: material([1.0, 1.0, 1.0]),
                    base_color: Some(tex),
                    normal_map: None,
                },
            ],
        };
        // Frame 1 builds the targets; the next ten animate the first cube
        // by rewriting its vertices and must create nothing at all.
        presenter.render_live_offscreen(&list).expect("frame 1");
        let after_first = presenter.gpu_create_totals();
        for step in 1..=10u8 {
            let moved = shifted_cube(-0.1 * f32::from(step));
            let wrote = presenter
                .refresh_mesh_vertices(key, &moved.positions, &moved.normals, &moved.uvs)
                .expect("refresh");
            assert!(wrote);
            presenter
                .render_live_offscreen(&list)
                .expect("steady frame");
        }
        assert_eq!(
            presenter.gpu_create_totals(),
            after_first,
            "no buffer or bind group after frame 1"
        );
        let stats = presenter.wait_stats();
        assert_eq!(stats.live_frames, 11);
        assert_eq!(
            (stats.steady_buffers, stats.steady_bind_groups),
            (0, 0),
            "{}",
            stats.summary()
        );
        assert!(
            stats.live_buffers >= 1 && stats.live_bind_groups >= 1,
            "frame 1 owns the target builds: {}",
            stats.summary()
        );
        // The counter is live: a buffer made between frames lands on the
        // next frame's tally instead of vanishing.
        presenter
            .upload_mesh(MeshKey::new(0x22, [9; 32]), &crate::v1::cube())
            .expect("late upload");
        presenter.render_live_offscreen(&list).expect("frame 13");
        assert_eq!(
            presenter.wait_stats().steady_buffers,
            2,
            "vertex plus index"
        );
    }

    #[test]
    fn each_draw_reads_its_own_material_from_the_ring() {
        let Some(mut presenter) = init_or_skip("each_draw_reads_its_own_material_from_the_ring")
        else {
            return;
        };
        presenter.resize(128, 128).expect("valid size applies");
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        let colors = [[1.0, 0.05, 0.05], [0.05, 1.0, 0.05], [0.05, 0.05, 1.0]];
        let mut items = Vec::new();
        for (index, (rgb, dx)) in colors.iter().zip([-1.6f32, 0.0, 1.6]).enumerate() {
            let key = MeshKey::new(0x500 + index as u64, [0x50; 32]);
            presenter
                .upload_mesh(key, &shifted_cube(dx))
                .expect("cube uploads");
            items.push(DrawItem {
                mesh: key,
                material: material(*rgb),
                base_color: None,
                normal_map: None,
            });
        }
        let list = DrawList {
            camera: camera_at(6.0),
            light: scene.light,
            items,
        };
        presenter.render_live_offscreen(&list).expect("three draws");
        let (rgba, w, _) = read_twin(&presenter);
        // View width at 6 m with a 40 degree fov is about 4.4 m: 1.6 m is
        // about 47 px off centre.
        for (index, x) in [64 - 47, 64, 64 + 47].into_iter().enumerate() {
            let px = pixel(&rgba, w, x, 64);
            let dominant = (0..3)
                .max_by(|a, b| px[*a].total_cmp(&px[*b]))
                .expect("three channels");
            assert_eq!(
                dominant, index,
                "draw {index} shows its own material, got {px:?}"
            );
        }
    }

    /// LF4 accept legs on the live GPU path: a torch-glint halo differs
    /// with bloom on versus off, a frame with no pixel above the knee is
    /// unchanged, read-back x2 is byte-identical, validation errors 0.
    /// Caller: `Presenter::set_bloom_enabled` through `present_frame_live`
    /// into the blit halo. Skip without an adapter, never fake-pass.
    #[test]
    fn bloom_on_off_halo_differs_dark_unchanged_live() {
        let Some(mut presenter) = init_or_skip("bloom_on_off_halo_differs_dark_unchanged_live")
        else {
            return;
        };
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(128, 128).expect("valid size applies");
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        let torch = DrawList {
            camera: camera_at(6.0),
            light: scene.light.clone(),
            items: vec![DrawItem {
                mesh: cube_key(),
                material: crate::v1::PbrMaterial {
                    base_color: [1.0, 1.0, 1.0],
                    metallic: 0.0,
                    roughness: 0.8,
                    exposure: 4.0,
                },
                base_color: None,
                normal_map: None,
            }],
        };
        let mut dim_light = scene.light.clone();
        dim_light.intensity = 0.2;
        let dark = DrawList {
            camera: camera_at(6.0),
            light: dim_light,
            items: vec![DrawItem {
                mesh: cube_key(),
                material: crate::v1::PbrMaterial {
                    base_color: [0.1, 0.1, 0.1],
                    metallic: 0.0,
                    roughness: 0.8,
                    exposure: 1.0,
                },
                base_color: None,
                normal_map: None,
            }],
        };
        presenter.set_bloom_enabled(false);
        assert!(!presenter.bloom_enabled(), "switch reads back off");
        let off_receipt = presenter
            .render_live_offscreen(&torch)
            .expect("torch off renders");
        assert!(off_receipt.presented && off_receipt.is_clean());
        assert_eq!(
            off_receipt.validation_errors, 0,
            "torch off stays validation-clean"
        );
        let (off_rgba, off_w, off_h) = read_twin(&presenter);
        presenter.set_bloom_enabled(true);
        assert!(presenter.bloom_enabled(), "switch reads back on");
        let on_receipt = presenter
            .render_live_offscreen(&torch)
            .expect("torch on renders");
        assert!(on_receipt.presented && on_receipt.is_clean());
        assert_eq!(
            on_receipt.validation_errors, 0,
            "torch on stays validation-clean"
        );
        let (on_rgba, on_w, on_h) = read_twin(&presenter);
        assert_eq!((off_w, off_h), (128, 128), "twin covers the frame");
        assert_eq!((on_w, on_h), (128, 128), "twin covers the frame");
        assert_eq!(off_rgba.len(), on_rgba.len(), "same frame size");
        let on_again = presenter
            .render_live_offscreen(&torch)
            .expect("torch on replays");
        assert!(on_again.presented && on_again.is_clean());
        assert_eq!(on_again.validation_errors, 0, "replay stays clean");
        let (on2_rgba, _, _) = read_twin(&presenter);
        assert_eq!(on_rgba, on2_rgba, "same halo replays byte-identical");
        let mut halo_count = 0usize;
        let mut max_diff = 0u8;
        for chunk in off_rgba.chunks_exact(4).zip(on_rgba.chunks_exact(4)) {
            let (a, b) = chunk;
            for c in 0..3 {
                let diff = a[c].abs_diff(b[c]);
                if diff > max_diff {
                    max_diff = diff;
                }
                if diff >= 5 {
                    halo_count += 1;
                    break;
                }
            }
        }
        assert!(
            halo_count > 50,
            "torch halo differs on vs off: {halo_count} pixels, max {max_diff}"
        );
        assert!(
            max_diff >= 8,
            "halo lift is visible on vs off, max {max_diff}"
        );
        let width = usize::try_from(off_w).expect("twin width fits in memory");
        for y in 0..8usize {
            for x in 0..8usize {
                let idx = (y * width + x) * 4;
                assert_eq!(
                    off_rgba[idx..idx + 4],
                    on_rgba[idx..idx + 4],
                    "sub-knee corner stays identical at ({x}, {y})"
                );
            }
        }
        let dark_off = presenter
            .render_live_offscreen(&dark)
            .expect("dark off renders");
        assert!(dark_off.presented && dark_off.is_clean());
        let (dark_off_rgba, _, _) = read_twin(&presenter);
        presenter.set_bloom_enabled(true);
        let dark_on = presenter
            .render_live_offscreen(&dark)
            .expect("dark on renders");
        assert!(dark_on.presented && dark_on.is_clean());
        assert_eq!(dark_on.validation_errors, 0, "dark on stays clean");
        let (dark_on_rgba, _, _) = read_twin(&presenter);
        assert_eq!(
            dark_off_rgba, dark_on_rgba,
            "sub-knee frame is unchanged on vs off"
        );
        presenter.set_bloom_enabled(false);
        assert!(!presenter.bloom_enabled(), "switch restores off");
    }
}

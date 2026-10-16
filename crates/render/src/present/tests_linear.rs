//! tests_linear: the linear HDR frame and the one forward shader file.
//!
//! Pure checks read the shader sources; device tests (skipped with no
//! adapter, like the RA1 twin tests) read the Rgba16Float forward target back
//! and compare the blit's tone map and the forward fog with CPU twins.

#![cfg(test)]

#[cfg(feature = "window")]
use super::*;

/// Sources that build WGSL: none of them may patch shader text.
#[cfg(feature = "window")]
const SHADER_BUILDERS: [(&str, &str); 7] = [
    ("present.rs", include_str!("../present.rs")),
    ("twin_shadow.rs", include_str!("../twin_shadow.rs")),
    ("v1_gpu.rs", include_str!("../v1_gpu.rs")),
    ("v1_skin.rs", include_str!("../v1_skin.rs")),
    ("fog_fwd.rs", include_str!("fog_fwd.rs")),
    ("gpu_present.rs", include_str!("gpu_present.rs")),
    (
        "gpu_present/presenter.rs",
        include_str!("gpu_present/presenter.rs"),
    ),
];

/// Body of the WGSL function `name`: from its signature to the closing brace
/// at column 0.
#[cfg(feature = "window")]
fn wgsl_fn_body<'a>(source: &'a str, name: &str) -> &'a str {
    let head = format!("fn {name}(");
    let start = source.find(&head).expect("function exists in the source");
    let rest = &source[start..];
    let end = rest.find("\n}\n").expect("function closes");
    &rest[..end]
}

#[cfg(feature = "window")]
mod shader_text {
    use super::*;
    use crate::v1_gpu::FORWARD_PBR_WGSL;

    #[test]
    fn one_forward_file_holds_every_entry_and_one_brdf() {
        for entry in [
            "fn vs(",
            "fn vs_skinned(",
            "fn vs_depth(",
            "fn vs_sky(",
            "fn fs(",
            "fn fs_sky(",
            "fn fs_display(",
            "fn fs_skinned(",
        ] {
            assert!(FORWARD_PBR_WGSL.contains(entry), "entry point {entry}");
        }
        // The BRDF and the display transform exist once, in the one source.
        for term in [
            "fn d_ggx(",
            "fn fresnel_term(",
            "fn aces_fitted(",
            "fn linear_to_srgb_term(",
        ] {
            assert_eq!(FORWARD_PBR_WGSL.matches(term).count(), 1, "{term} once");
        }
        let skin = include_str!("../v1_skin.rs");
        assert!(
            !skin.contains("fn d_ggx(") && !skin.contains("fn aces_fitted("),
            "the skinned pass keeps no second BRDF or tonemap copy"
        );
    }

    #[test]
    fn no_wgsl_is_built_by_string_patching() {
        for (name, source) in SHADER_BUILDERS {
            assert!(
                !source.contains(".replacen("),
                "{name} patches text with replacen"
            );
        }
        assert!(
            !include_str!("../twin_shadow.rs").contains("fn twin_mesh_forward_wgsl"),
            "the patch layer is gone"
        );
    }

    #[test]
    fn forward_fragment_writes_linear_radiance_and_never_tone_maps() {
        let fs = wgsl_fn_body(FORWARD_PBR_WGSL, "fs");
        assert!(fs.contains("mat.exposure"), "exposure is the only scale");
        for banned in ["aces_fitted", "linear_to_srgb_term", "pow("] {
            assert!(!fs.contains(banned), "forward fs must not contain {banned}");
        }
        let sky = wgsl_fn_body(FORWARD_PBR_WGSL, "fs_sky");
        assert!(!sky.contains("aces_fitted") && !sky.contains("linear_to_srgb_term"));
        // The offscreen 8-bit capture entry is the one that does map.
        let display = wgsl_fn_body(FORWARD_PBR_WGSL, "fs_display");
        assert!(display.contains("aces_fitted") && display.contains("linear_to_srgb_term"));
        assert_eq!(HDR_FORMAT, wgpu::TextureFormat::Rgba16Float);
    }

    #[test]
    fn the_blit_does_the_one_tone_map() {
        let blit = crate::present::gpu_present::BLIT_WGSL;
        let fs = wgsl_fn_body(blit, "fs");
        assert!(fs.contains("aces_fitted") && fs.contains("linear_to_srgb_term"));
        let hw = wgsl_fn_body(blit, "fs_hw_srgb");
        assert!(hw.contains("aces_fitted"), "tone map still runs");
        assert!(
            !hw.contains("linear_to_srgb_term"),
            "a *Srgb view encodes on write, the shader must not encode twice"
        );
        assert_eq!(blit.matches("fn aces_fitted(").count(), 1);
    }
}

#[cfg(feature = "window")]
mod device {
    use super::*;
    use crate::present::fog_fwd::FogFwd;

    fn init_or_skip() -> Option<Presenter> {
        match Presenter::init_offscreen() {
            Ok(p) => Some(p),
            Err(PresentError::NoAdapter) => {
                eprintln!("SKIPPED linear frame gpu test: no wgpu adapter on this machine");
                None
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        }
    }

    fn cube_key() -> MeshKey {
        MeshKey::new(0x1A7E, [0x77; 32])
    }

    fn camera_at(z: f32) -> crate::v1::Camera {
        crate::v1::Camera::new(
            [0.0, 0.0, z],
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            40.0f32.to_radians(),
        )
    }

    fn list(camera: crate::v1::Camera, exposure: f32) -> DrawList {
        let scene = crate::v1::default_scene(128, 128).expect("default scene is valid");
        DrawList {
            camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh: cube_key(),
                material: crate::v1::PbrMaterial {
                    base_color: [1.0, 1.0, 1.0],
                    metallic: 0.0,
                    roughness: 0.5,
                    exposure,
                },
                base_color: None,
                normal_map: None,
            }],
        }
    }

    /// Copy a texture to the CPU, rows unpadded. Test only: this waits.
    fn read_texture(
        presenter: &Presenter,
        texture: &wgpu::Texture,
        bytes_per_texel: u32,
    ) -> (Vec<u8>, u32, u32) {
        let (w, h) = (texture.width(), texture.height());
        let row = w * bytes_per_texel;
        let padded = row.div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-linear-readback"),
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
            r.expect("test readback maps");
        });
        presenter
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("test readback polls");
        let row = row as usize;
        let mut out = vec![0u8; row * h as usize];
        {
            let mapped = buffer.slice(..).get_mapped_range().expect("mapped");
            for (y, dst) in out.chunks_exact_mut(row).enumerate() {
                let off = y * padded as usize;
                dst.copy_from_slice(&mapped[off..off + row]);
            }
        }
        buffer.unmap();
        (out, w, h)
    }

    fn half_to_f32(h: u16) -> f32 {
        let sign = if h >> 15 == 1 { -1.0 } else { 1.0 };
        let exp = i32::from((h >> 10) & 0x1f);
        let frac = f32::from(h & 0x3ff);
        let magnitude = match exp {
            0 => frac * 2.0f32.powi(-24),
            31 if frac == 0.0 => f32::INFINITY,
            31 => f32::NAN,
            _ => (1.0 + frac / 1024.0) * 2.0f32.powi(exp - 15),
        };
        sign * magnitude
    }

    /// RGB of one texel of the linear HDR target.
    fn hdr_pixel(presenter: &Presenter, x: u32, y: u32) -> [f32; 3] {
        let targets = presenter.targets.as_ref().expect("targets exist");
        let (bytes, w, _) = read_texture(presenter, &targets.hdr_texture, 8);
        let at = ((y * w + x) * 8) as usize;
        let channel = |c: usize| {
            half_to_f32(u16::from_le_bytes([
                bytes[at + c * 2],
                bytes[at + c * 2 + 1],
            ]))
        };
        [channel(0), channel(1), channel(2)]
    }

    fn twin_pixel(presenter: &Presenter, x: u32, y: u32) -> [f32; 3] {
        let targets = presenter.targets.as_ref().expect("targets exist");
        let (bytes, w, _) = read_texture(presenter, &targets.out_texture, 4);
        let at = ((y * w + x) * 4) as usize;
        [
            f32::from(bytes[at]),
            f32::from(bytes[at + 1]),
            f32::from(bytes[at + 2]),
        ]
    }

    /// CPU twin of `aces_fitted` plus the sRGB transfer, 0..=255 per channel.
    fn display_of(linear: [f32; 3]) -> [f32; 3] {
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let fit = |v: f32| {
            (v * (v + 0.0245786) - 0.000090537) / (v * (0.983729 * v + 0.4329510) + 0.238081)
        };
        let to_rrt = [
            [0.59719, 0.35458, 0.04823],
            [0.07600, 0.90834, 0.01566],
            [0.02840, 0.13383, 0.83777],
        ];
        let from_rrt = [
            [1.60475, -0.53108, -0.07367],
            [-0.10208, 1.10813, -0.00605],
            [-0.00327, -0.07276, 1.07602],
        ];
        let r = to_rrt.map(|col| fit(dot(linear, col)));
        let mapped = from_rrt.map(|col| dot(r, col).clamp(0.0, 1.0));
        mapped.map(|x| {
            let enc = if x <= 0.0031308 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            };
            enc * 255.0
        })
    }

    #[test]
    fn forward_target_is_linear_hdr_with_exposure_as_the_only_scale() {
        let Some(mut presenter) = init_or_skip() else {
            return;
        };
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(128, 128).expect("valid size applies");
        presenter.set_fog_enabled(false);
        presenter
            .render_live_offscreen(&list(camera_at(6.0), 1.0))
            .expect("exposure 1 frame");
        let targets = presenter.targets.as_ref().expect("targets exist");
        assert_eq!(
            targets.hdr_texture.format(),
            wgpu::TextureFormat::Rgba16Float,
            "the forward target is a half-float linear target"
        );
        let base = hdr_pixel(&presenter, 64, 64);
        presenter
            .render_live_offscreen(&list(camera_at(6.0), 4.0))
            .expect("exposure 4 frame");
        let bright = hdr_pixel(&presenter, 64, 64);
        for c in 0..3 {
            assert!(base[c] > 0.05, "the cube face is lit: {base:?}");
            let ratio = bright[c] / base[c];
            assert!(
                (ratio - 4.0).abs() < 0.08,
                "channel {c}: exposure scales radiance linearly, ratio {ratio} ({base:?} -> {bright:?})"
            );
        }
        // Radiance above 1.0 survives in the target: nothing tone-maps or
        // clamps before the blit.
        presenter
            .render_live_offscreen(&list(camera_at(6.0), 16.0))
            .expect("exposure 16 frame");
        let hot = hdr_pixel(&presenter, 64, 64);
        assert!(
            hot.iter().any(|c| *c > 1.5),
            "HDR radiance above 1 is stored: {hot:?}"
        );
        // The twin after the blit is the tone map of exactly that radiance.
        let shown = twin_pixel(&presenter, 64, 64);
        let want = display_of(hot);
        for c in 0..3 {
            assert!(
                (shown[c] - want[c]).abs() <= 2.5,
                "blit = ACES + sRGB of the linear target, channel {c}: {shown:?} vs {want:?}"
            );
        }
    }

    #[test]
    fn sky_and_fog_mix_in_linear_radiance_before_the_tone_map() {
        let Some(mut presenter) = init_or_skip() else {
            return;
        };
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(128, 128).expect("valid size applies");
        let draw = list(camera_at(15.0), 1.0);
        presenter.set_fog_enabled(false);
        presenter
            .render_live_offscreen(&draw)
            .expect("unfogged frame");
        let plain = hdr_pixel(&presenter, 64, 64);
        presenter.set_fog_enabled(true);
        presenter
            .render_live_offscreen(&draw)
            .expect("fogged frame");
        let fogged = hdr_pixel(&presenter, 64, 64);
        let fog: FogFwd = presenter.fog_fwd;
        // The centre pixel looks straight at the cube's front face.
        let f = fog.factor([0.0, 0.0, 15.0], [0.0, 0.0, 0.5]);
        assert!(f > 0.15, "the test camera sits in visible fog: {f}");
        for c in 0..3 {
            let want = plain[c] + f * (fog.color[c] - plain[c]);
            assert!(
                (fogged[c] - want).abs() <= 0.02 * want.max(0.05),
                "channel {c}: linear mix {} vs {want} (plain {} f {f})",
                fogged[c],
                plain[c]
            );
        }
        // Sky pixels: eye inside the fog looking up and out. The target
        // holds the CPU twin's linear sky; the twin holds its tone map.
        let camera = crate::v1::Camera::new(
            [0.0, 1.0, 0.0],
            [0.0, 4.0, -6.0],
            [0.0, 1.0, 0.0],
            70.0f32.to_radians(),
        );
        let empty = DrawList {
            camera: camera.clone(),
            light: crate::v1::default_scene(128, 128)
                .expect("default scene is valid")
                .light,
            items: Vec::new(),
        };
        presenter.render_live_offscreen(&empty).expect("sky frame");
        let mut cam = camera;
        cam.aspect = 1.0;
        let inv = cam.view_proj().expect("camera").inverse();
        for (x, y) in [
            (10u32, 10u32),
            (64, 30),
            (118, 20),
            (64, 64),
            (20, 110),
            (100, 120),
        ] {
            let ndc = glam::Vec4::new(
                2.0 * (x as f32 + 0.5) / 128.0 - 1.0,
                1.0 - 2.0 * (y as f32 + 0.5) / 128.0,
                1.0,
                1.0,
            );
            let world = inv * ndc;
            let dir = (world.truncate() / world.w - cam.eye).normalize();
            let want = fog.sky_color(cam.eye.to_array(), dir.to_array());
            let got = hdr_pixel(&presenter, x, y);
            for c in 0..3 {
                assert!(
                    (got[c] - want[c]).abs() <= 0.01 + 0.01 * want[c],
                    "sky ({x},{y}) channel {c}: {got:?} vs {want:?}"
                );
            }
            let shown = twin_pixel(&presenter, x, y);
            let shown_want = display_of(want);
            for c in 0..3 {
                assert!(
                    (shown[c] - shown_want[c]).abs() <= 3.0,
                    "sky ({x},{y}) display channel {c}: {shown:?} vs {shown_want:?}"
                );
            }
        }
    }

    #[test]
    fn srgb_surface_blit_matches_the_shader_encoded_twin() {
        // A `*Srgb` swapchain view encodes on write: the `fs_hw_srgb` entry
        // into an Rgba8UnormSrgb target must land on the same bytes as the
        // shader-encoded `fs` entry into the Rgba8Unorm twin (the earlier
        // blit encoded twice on such a surface and washed the frame out).
        let Some(mut presenter) = init_or_skip() else {
            return;
        };
        presenter
            .upload_mesh(cube_key(), &crate::v1::cube())
            .expect("cube uploads");
        presenter.resize(128, 128).expect("valid size applies");
        presenter
            .render_live_offscreen(&list(camera_at(6.0), 2.0))
            .expect("frame renders");
        let targets = presenter.targets.as_ref().expect("targets exist");
        let size = wgpu::Extent3d {
            width: 128,
            height: 128,
            depth_or_array_layers: 1,
        };
        let srgb_target = presenter.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test-srgb-target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let layout = presenter
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&presenter.blit_layout)],
                immediate_size: 0,
            });
        let pipeline = presenter
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("test-srgb-blit"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &presenter.blit_shader,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &presenter.blit_shader,
                    entry_point: Some("fs_hw_srgb"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let view = srgb_target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &targets.blit_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        presenter.queue.submit(Some(enc.finish()));
        let (hw, _, _) = read_texture(&presenter, &srgb_target, 4);
        let (twin, _, _) = read_texture(&presenter, &targets.out_texture, 4);
        let mut worst = 0i32;
        for (a, b) in hw.chunks_exact(4).zip(twin.chunks_exact(4)) {
            for c in 0..3 {
                worst = worst.max((i32::from(a[c]) - i32::from(b[c])).abs());
            }
        }
        assert!(
            worst <= 2,
            "hardware sRGB encode equals the shader encode, worst byte diff {worst}"
        );
    }

    #[test]
    fn depth_only_entry_compiles_without_a_fragment_stage() {
        let Some(presenter) = init_or_skip() else {
            return;
        };
        let device = &presenter.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test-forward-depth"),
            source: wgpu::ShaderSource::Wgsl(crate::v1_gpu::FORWARD_PBR_WGSL.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&presenter.forward_layout)],
            immediate_size: 0,
        });
        let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test-depth-only"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_depth"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 24,
                            shader_location: 2,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let error = pollster::block_on(scope.pop());
        assert!(error.is_none(), "depth-only pipeline validates: {error:?}");
    }

    /// The capture twin composites the probe fog pair on linear radiance,
    /// before the one tone map. Level camera over an empty frame: the top
    /// rows look over the horizon (clear), the bottom rows into the haze.
    /// Fully fogged bands read the bound ramp, tone-mapped, darkened by at
    /// most the contact AO's own range. (The earlier assertion divided the
    /// ramp by the mean AO of the whole bottom half, 0.6, but the bottom
    /// rows sit where contact AO is open, about 1.0: the test was wrong, not
    /// the composite.)
    #[test]
    fn capture_twin_fog_pair_composites_linear_before_the_tone_map() {
        let Some(mut presenter) = init_or_skip() else {
            return;
        };
        presenter.resize(64, 64).expect("valid size applies");
        let camera = crate::v1::Camera::new(
            [0.0, 2.0, 5.0],
            [0.0, 2.0, 0.0],
            [0.0, 1.0, 0.0],
            40.0f32.to_radians(),
        );
        let scene = crate::v1::default_scene(64, 64).expect("default scene is valid");
        let empty = DrawList {
            camera,
            light: scene.light,
            items: Vec::new(),
        };
        let on = presenter
            .render_offscreen(&empty)
            .expect("fog frame renders");
        assert_eq!(on.receipt.fog_passes, 2);
        assert!(on.manifest.is_clean());
        let png = on.png.expect("live frame has pixels");
        let img = image::load_from_memory(&png)
            .expect("capture is a valid PNG")
            .to_rgba8();
        let mean = |y0: u32, y1: u32| {
            let mut acc = [0.0f64; 3];
            let mut n = 0.0f64;
            for y in y0..y1 {
                for x in 0..64 {
                    let p = img.get_pixel(x, y);
                    for c in 0..3 {
                        acc[c] += f64::from(p[c]);
                    }
                    n += 1.0;
                }
            }
            acc.map(|v| (v / n) as f32)
        };
        // File rows run upright: row 0 is view-top.
        let top = mean(0, 8);
        let bottom = mean(56, 64);
        let clear = [8.0f32, 9.0, 15.0];
        for c in 0..3 {
            assert!(
                (top[c] - clear[c]).abs() < 6.0,
                "over-horizon rows keep the clear: {top:?}"
            );
        }
        // The ramp the blit mixes is the presenter's own look-bake horizon
        // color after the sun-scatter tint the fog tranche applies for this
        // camera (eye to target is -z): rebuild it the same way.
        let params = presenter.look_fog_params().expect("fog params");
        let tinted = params
            .with_sun_tint(presenter.look_frame().sun_dir, [0.0, 0.0, -1.0])
            .expect("tint applies");
        let ramp = tinted.fog_color;
        let ao_min = presenter.fog_contact_min();
        assert!(
            (0.0..=1.0).contains(&ao_min),
            "contact evidence bounded: {ao_min}"
        );
        let ceiling = display_of(ramp);
        let floor = display_of(ramp.map(|v| v * ao_min));
        for c in 0..3 {
            assert!(
                bottom[c] > top[c] + 40.0,
                "the haze rows are far from the clear: {bottom:?} vs {top:?}"
            );
            assert!(
                bottom[c] <= ceiling[c] + 8.0 && bottom[c] >= floor[c] - 8.0,
                "fogged rows sit between the darkest AO ramp and the open ramp, channel {c}: {bottom:?} in [{floor:?}, {ceiling:?}]"
            );
        }
        // Kill switch: the same frame is the clear colour everywhere.
        presenter.set_fog_enabled(false);
        let off = presenter
            .render_offscreen(&empty)
            .expect("unfogged frame renders");
        assert_eq!(off.receipt.fog_passes, 0);
        let png_off = off.png.expect("pixels");
        let img_off = image::load_from_memory(&png_off)
            .expect("capture is a valid PNG")
            .to_rgba8();
        let mut worst = 0.0f32;
        for p in img_off.pixels() {
            for c in 0..3 {
                worst = worst.max((f32::from(p[c]) - clear[c]).abs());
            }
        }
        assert!(worst < 6.0, "fog off restores the clear frame: {worst}");
    }
}

/// LF4 slice 2: the live blit blooms linear HDR before the one tone map.
/// No adapter needed: reads `blit.wgsl` text. Fails before the bloom lands.
#[test]
fn the_blit_blooms_before_the_one_tone_map() {
    let blit = include_str!("../shaders/blit.wgsl");
    assert!(
        blit.contains("fn blit_bloom("),
        "live blit holds the LF4 bloom"
    );
    assert!(
        blit.contains("BLIT_BLOOM_THRESHOLD"),
        "bloom threshold is named"
    );
    assert!(
        blit.contains("blit_bloom(lit.rgb"),
        "fs blooms the composite"
    );
    assert!(
        blit.contains("display_tonemap(bloomed)"),
        "tone map reads the bloomed HDR"
    );
}

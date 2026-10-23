//! Steady sun shadow: the compare bias (2^-19 plus a slope term and a normal
//! offset) and the three stability pins: a 1 cm camera move leaves every map
//! texel byte-equal, a sun-facing sand plane reads 0 shadowed pixels, and a
//! foot standing on that plane shadows within 2 px of its base.
//!
//! The CPU oracles here mirror `sun_bias` / `sun_shadow_at` in
//! `forward.wgsl`; the GPU legs render the live map through the production
//! matrix, shader and pipeline, then read every texel back.

use super::*;
use crate::arena_day::ARENA_DAY_SUN_DIR;

/// Foot fixture: half width across the shadow, half depth along it, height.
const FOOT_HALF_W: f32 = 0.12;
const FOOT_HALF_D: f32 = 0.05;
const FOOT_H: f32 = 0.18;

fn unit(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// The light's travel direction: from the sun toward the pit.
fn light_travel() -> [f32; 3] {
    let s = unit(ARENA_DAY_SUN_DIR);
    [-s[0], -s[1], -s[2]]
}

/// The shadow direction on level ground: the light's horizontal part, unit.
fn shadow_axis() -> [f32; 3] {
    let l = light_travel();
    unit([l[0], 0.0, l[2]])
}

/// The width axis across the shadow, unit and horizontal.
fn across_axis() -> [f32; 3] {
    let p = shadow_axis();
    [p[2], 0.0, -p[0]]
}

fn idx(i: usize) -> f32 {
    f32::from(u16::try_from(i).expect("map index fits u16"))
}

/// Light-space (a, b) in meters of a map texel coordinate (`fx`, `fy`),
/// fractions allowed. `a` is the horizontal axis across the sun, `b` the
/// axis the sun's shadow runs on.
fn texel_ab(edge: u32, fx: f32, fy: f32) -> (f32, f32) {
    let e = edge_f32(edge);
    let half = SUN_SHADOW_HALF_EXTENT_M;
    ((2.0 * fx / e - 1.0) * half, (1.0 - 2.0 * fy / e) * half)
}

/// The ground point at light-space (a, b): the inverse of the sun ortho
/// projection on the `y = 0` plane.
fn ground_at_ab(a: f32, b: f32) -> [f32; 3] {
    let sun = unit(ARENA_DAY_SUN_DIR);
    let (t, u, _) = sun_light_basis(sun);
    let det = t[0] * u[2] - t[2] * u[0];
    [
        (a * u[2] - t[2] * b) / det,
        0.0,
        (t[0] * b - a * u[0]) / det,
    ]
}

/// The ground point of a map texel coordinate (`fx`, `fy`), fractions allowed.
fn ground_at(edge: u32, fx: f32, fy: f32) -> [f32; 3] {
    let (a, b) = texel_ab(edge, fx, fy);
    ground_at_ab(a, b)
}

/// The arena sand: one 30 m quad, two triangles, facing up.
fn plane_tris() -> Vec<[[f32; 3]; 3]> {
    let p = [
        [-15.0, 0.0, -15.0],
        [15.0, 0.0, -15.0],
        [15.0, 0.0, 15.0],
        [-15.0, 0.0, 15.0],
    ];
    vec![[p[0], p[1], p[2]], [p[0], p[2], p[3]]]
}

fn quad(a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) -> [[[f32; 3]; 3]; 2] {
    [[a, b, c], [a, c, d]]
}

/// The foot's back wall bottom-centre sits at light-space (a, b) = (0, 0), so
/// the contact walk starts on a whole map texel row (the pit centre).
fn foot_centre() -> [f32; 3] {
    let p = shadow_axis();
    let back = ground_at_ab(0.0, 0.0);
    [
        back[0] - p[0] * FOOT_HALF_D,
        back[1],
        back[2] - p[2] * FOOT_HALF_D,
    ]
}

/// A foot-sized box on the plane at [`foot_centre`]: 0.24 m across the
/// shadow, 0.10 m deep, 0.18 m tall, open at the bottom.
fn foot_tris() -> Vec<[[f32; 3]; 3]> {
    let c = foot_centre();
    let p = shadow_axis();
    let w = across_axis();
    let corner =
        |a: f32, d: f32, y: f32| [c[0] + w[0] * a + p[0] * d, y, c[2] + w[2] * a + p[2] * d];
    let mut tris = Vec::with_capacity(10);
    let a = FOOT_HALF_W;
    let d = FOOT_HALF_D;
    // Along the shadow: the back (shadow-side) wall and the front wall.
    tris.extend(quad(
        corner(-a, d, 0.0),
        corner(a, d, 0.0),
        corner(a, d, FOOT_H),
        corner(-a, d, FOOT_H),
    ));
    tris.extend(quad(
        corner(-a, -d, 0.0),
        corner(a, -d, 0.0),
        corner(a, -d, FOOT_H),
        corner(-a, -d, FOOT_H),
    ));
    // Across the shadow: the two sides.
    tris.extend(quad(
        corner(-a, -d, 0.0),
        corner(-a, d, 0.0),
        corner(-a, d, FOOT_H),
        corner(-a, -d, FOOT_H),
    ));
    tris.extend(quad(
        corner(a, -d, 0.0),
        corner(a, d, 0.0),
        corner(a, d, FOOT_H),
        corner(a, -d, FOOT_H),
    ));
    // The top.
    tris.extend(quad(
        corner(-a, -d, FOOT_H),
        corner(a, -d, FOOT_H),
        corner(a, d, FOOT_H),
        corner(-a, d, FOOT_H),
    ));
    tris
}

/// Analytic mirror of the map's stored depth at light-space (a, b): the
/// nearest of the ground, the foot's two walls along the shadow and its top.
fn stored_depth_at(vp: [[f32; 4]; 4], a: f32, b: f32, foot: bool) -> f32 {
    let ground = ground_at_ab(a, b);
    let Some([_, _, mut d]) = sun_uv_depth(vp, ground).expect("ground projects") else {
        return 1.0;
    };
    if !foot {
        return d;
    }
    let l = light_travel();
    let sun = unit(ARENA_DAY_SUN_DIR);
    let (_, u, _) = sun_light_basis(sun);
    let p = shadow_axis();
    let span = SUN_SHADOW_FAR_M - SUN_SHADOW_NEAR_M;
    let by_metre = u[1];
    let along = p[0] * u[0] + p[2] * u[2];
    // Both walls along the shadow: same base height range, shifted in b.
    for side in [FOOT_HALF_D, -FOOT_HALF_D] {
        let c = foot_centre();
        let base = [c[0] + p[0] * side, 0.0, c[2] + p[2] * side];
        let Some([_, _, d_base]) = sun_uv_depth(vp, base).expect("wall base") else {
            continue;
        };
        let b_base = base[0] * u[0] + base[2] * u[2];
        let b_top = b_base + FOOT_H * by_metre;
        let (lo, hi) = (b_base.min(b_top), b_base.max(b_top));
        if a.abs() <= FOOT_HALF_W && (lo..=hi).contains(&b) {
            let y = (b - b_base) / by_metre;
            d = d.min(d_base + y * l[1] / span);
        }
    }
    // The top face: horizontal, so its depth moves with the depth axis only.
    let c = foot_centre();
    let top = [c[0], FOOT_H, c[2]];
    let Some([_, _, d_top]) = sun_uv_depth(vp, top).expect("top projects") else {
        return d;
    };
    let b_top = top[0] * u[0] + top[1] * u[1] + top[2] * u[2];
    let (lo, hi) = (b_top - FOOT_HALF_D * along, b_top + FOOT_HALF_D * along);
    if a.abs() <= FOOT_HALF_W && (lo..=hi).contains(&b) {
        let depth = d_top + (b - b_top) / along * (p[0] * l[0] + p[2] * l[2]) / span;
        d = d.min(depth);
    }
    d
}

/// The compare bias for a ground fragment: 2^-19 plus the slope term with
/// `tan(theta)` from the up normal, exactly as `sun_bias` in the shader.
fn ground_bias(edge: u32) -> f32 {
    let sun = unit(ARENA_DAY_SUN_DIR);
    let no_l = sun[1].clamp(0.0, 1.0).max(0.05);
    let tan_theta = (1.0 - no_l * no_l).max(0.0).sqrt() / no_l;
    SUN_SHADOW_BIAS + sun_shadow_slope_scale_for(edge) * tan_theta
}

/// Lit taps of the 3x3 compare around a ground fragment in texel (`ix`, `iy`)
/// at fractions `frac`: the shader's PCF grid, clamped to the map edge.
fn ground_lit_taps(
    edge: u32,
    vp: [[f32; 4]; 4],
    ix: usize,
    iy: usize,
    frac: [f32; 2],
    bias: f32,
    stored: &dyn Fn(usize, usize) -> f32,
) -> u32 {
    let ground = ground_at(edge, idx(ix) + frac[0], idx(iy) + frac[1]);
    let up = sun_shadow_normal_offset_m(edge);
    let sample = [ground[0], ground[1] + up, ground[2]];
    let Some([_, _, frag]) = sun_uv_depth(vp, sample).expect("fragment projects") else {
        return 9;
    };
    let reference = frag - bias;
    let n = usize::try_from(edge).expect("edge fits usize");
    let xs = [ix.saturating_sub(1), ix, (ix + 1).min(n - 1)];
    let ys = [iy.saturating_sub(1), iy, (iy + 1).min(n - 1)];
    let mut lit = 0;
    for y in ys {
        for x in xs {
            if reference <= stored(x, y) {
                lit += 1;
            }
        }
    }
    lit
}

/// Render one sun map through the production matrix, depth shader and depth
/// pipeline, then read every texel back. `tris` are world meters.
fn render_sun_map(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    vp: [[f32; 4]; 4],
    edge: u32,
    tris: &[[[f32; 3]; 3]],
) -> Vec<f32> {
    let mut desc = sun_shadow_depth_desc_for(edge);
    desc.usage |= wgpu::TextureUsages::COPY_SRC;
    let texture = device.create_texture(&desc);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut matrix = [0u8; 64];
    for (ci, col) in vp.iter().enumerate() {
        for (ri, c) in col.iter().enumerate() {
            matrix[ci * 16 + ri * 4..ci * 16 + ri * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("steady-sun-uniform"),
        size: 64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&uniform, 0, &matrix);
    // The live depth pipeline reads the forward vertex stride and the
    // position at offset 0, so pad each corner to that stride.
    let stride = super::super::submit::FORWARD_VERTEX_STRIDE;
    let stride_usize = usize::try_from(stride).expect("stride fits");
    let mut raw = Vec::with_capacity(tris.len() * 3 * stride_usize);
    for tri in tris {
        for p in tri {
            for c in p {
                raw.extend_from_slice(&c.to_le_bytes());
            }
            raw.resize(raw.len() + stride_usize - 12, 0);
        }
    }
    let vertices = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("steady-sun-vertices"),
        size: raw.len() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&vertices, 0, &raw);
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("steady-sun-depth"),
        source: wgpu::ShaderSource::Wgsl(SUN_SHADOW_DEPTH_WGSL.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("steady-sun-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pipeline = sun_depth_pipeline(device, &layout, &shader);
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("steady-sun-bind"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("steady-sun-encode"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("steady-sun-depth-pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(
            0..u32::try_from(tris.len() * 3).expect("triangles fit"),
            0..1,
        );
    }
    let row_bytes = edge * 4;
    let padded = row_bytes.div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("steady-sun-readback"),
        size: u64::from(padded) * u64::from(edge),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(edge),
            },
        },
        wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("readback poll holds");
    rx.recv()
        .expect("readback channel holds")
        .expect("readback maps");
    let w = usize::try_from(edge).expect("edge fits");
    let mut map = vec![0.0f32; w * w];
    {
        let mapped = slice.get_mapped_range().expect("mapped range holds");
        let row = usize::try_from(padded).expect("padded row fits");
        for y in 0..w {
            for x in 0..w {
                let off = y * row + x * 4;
                map[y * w + x] = f32::from_le_bytes([
                    mapped[off],
                    mapped[off + 1],
                    mapped[off + 2],
                    mapped[off + 3],
                ]);
            }
        }
    }
    readback.unmap();
    map
}

/// The shared GX1 device, or `None` with a `SKIPPED` line when none is there.
fn gpu_or_skip(test: &str) -> Option<(&'static wgpu::Device, &'static wgpu::Queue)> {
    let (device, queue, _) = crate::test_gpu::require_or_skip(test, crate::graph::gx1_probe())?;
    Some((device, queue))
}

#[test]
fn sun_bias_is_two_to_the_minus_19_plus_slope_and_normal_offset() {
    // The row's bias: 2^-19 in normalized depth, a slope term sized in
    // texels of the surface's depth slope, and a one-texel normal offset.
    assert_eq!(
        SUN_SHADOW_BIAS,
        2f32.powi(-19),
        "constant bias is 2^-19, not the old flat 0.002"
    );
    assert_eq!(SUN_SHADOW_SLOPE_SCALE, 2.0, "slope term covers 2 texels");
    assert_eq!(SUN_SHADOW_NORMAL_OFFSET_TEXELS, 1.0, "normal offset");
    let edge = SUN_SHADOW_SIZE;
    let texel = sun_shadow_texel_m(edge);
    assert!(
        (texel - 30.0 / 2048.0).abs() < 1e-9,
        "one texel is 30/2048 m"
    );
    assert!(
        (sun_shadow_normal_offset_m(edge) - texel).abs() < 1e-9,
        "the offset is one texel in meters"
    );
    let slope = sun_shadow_slope_scale_for(edge);
    let want = 2.0 * texel / (SUN_SHADOW_FAR_M - SUN_SHADOW_NEAR_M);
    assert!(
        (slope - want).abs() < 1e-12,
        "slope scale: {slope} vs {want}"
    );
    // The compare bytes carry all five fields the shader reads.
    let sample = sun_fwd_sample_for_edge(edge);
    assert_eq!(sample.bias, SUN_SHADOW_BIAS);
    assert_eq!(sample.enabled, 1.0);
    assert!((sample.texel - 1.0 / 2048.0).abs() < 1e-12);
    assert_eq!(sample.normal_offset_m, texel);
    assert_eq!(sample.slope_scale, slope);
    let vp = sun_view_proj_for_dir(ARENA_DAY_SUN_DIR).expect("arena sun builds");
    let bytes = sun_forward_uniform_bytes(vp, sample);
    let at = |offset: usize| {
        f32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    assert_eq!(bytes.len(), 96, "matrix plus two std140 rows");
    assert_eq!(at(76), texel, "normal offset at 76");
    assert_eq!(at(80), slope, "slope scale at 80");
    // The shader reads the same three pieces.
    let wgsl = include_str!("../../../shaders/forward.wgsl");
    assert!(wgsl.contains("sun_fwd.bias + sun_fwd.slope_scale * tan_theta"));
    assert!(wgsl.contains("sun_fwd.normal_offset"));
    // A half-res map keeps the same world offset but a coarser texel.
    let half = sun_shadow_size_for_half(true);
    assert!(
        (sun_shadow_normal_offset_m(half) - 2.0 * texel).abs() < 1e-9,
        "half edge doubles the texel"
    );
}

#[test]
fn sun_map_pixels_survive_a_one_centimetre_camera_move() {
    // The live sun rig is pinned to the pit: its matrix comes from the sun
    // direction alone, so moving the frame camera cannot move a texel. A rig
    // that followed the camera would have to resnap; this pins the equal arm.
    let edge = sun_shadow_size_for_half(SUN_SHADOW_HALF_RES);
    let dir = ARENA_DAY_SUN_DIR;
    let camera = [1.0f32, 2.0, 3.0];
    let moved = [camera[0] + 0.01, camera[1], camera[2]];
    assert!(
        ((moved[0] - camera[0]) - 0.01).abs() < 1e-6,
        "the frame camera moved 1 cm"
    );
    // The source tripwire: the builder takes the light direction, no camera.
    let src = include_str!("../sun_shadow.rs");
    let at = src
        .find("pub fn sun_view_proj_for_dir")
        .expect("the sun matrix builder exists");
    let sig = &src[at..at + src[at..].find('{').expect("the body follows")];
    assert!(
        !sig.contains("camera"),
        "the sun rig takes no camera: {sig}"
    );
    let vp = sun_view_proj_for_dir(dir).expect("arena sun builds");
    let vp_again = sun_view_proj_for_dir(dir).expect("builds twice");
    assert_eq!(vp, vp_again, "the matrix ignores the frame");
    let sample = sun_fwd_sample_for_edge(edge);
    assert_eq!(
        sun_forward_uniform_bytes(vp, sample),
        sun_forward_uniform_bytes(vp_again, sample),
        "the compare bytes ignore the frame"
    );
    let tris = plane_tris();
    let Some((device, queue)) = gpu_or_skip("sun_map_pixels_survive_a_one_centimetre_camera_move")
    else {
        return;
    };
    let first = render_sun_map(device, queue, vp, edge, &tris);
    // The camera moved 1 cm; the map reads the same light, never the camera.
    assert_ne!(camera, moved, "the camera moved");
    let second = render_sun_map(device, queue, vp, edge, &tris);
    let bits = |m: &[f32]| m.iter().map(|d| d.to_bits()).collect::<Vec<u32>>();
    let a = bits(&first);
    let b = bits(&second);
    let diff = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
    assert_eq!(
        diff, 0,
        "every map texel stays byte-equal over a 1 cm camera move: {diff} differ"
    );
    assert_eq!(
        first.len(),
        usize::try_from(edge * edge).expect("edge fits")
    );
}

#[test]
fn sun_facing_sand_plane_has_zero_shadowed_pixels() {
    // The sand plane under the arena sun is the grazing case: 2^-19 alone
    // cannot cover one texel of its depth slope, so the slope term has to.
    let edge = sun_shadow_size_for_half(SUN_SHADOW_HALF_RES);
    let vp = sun_view_proj_for_dir(ARENA_DAY_SUN_DIR).expect("arena sun builds");
    let bias = ground_bias(edge);
    assert!(bias > SUN_SHADOW_BIAS, "the slope term is live");
    let n = usize::try_from(edge).expect("edge fits");
    // Every 16 texels at three sub-texel fractions, so a fragment near a
    // texel edge (the widest PCF footprint) is covered too. Points outside
    // the 30 m sand quad cannot be compared against a cleared texel.
    let sweep = |bias: f32, stored: &dyn Fn(usize, usize) -> f32| {
        let (mut checked, mut shadowed) = (0u32, 0u32);
        for iy in (8..n - 8).step_by(16) {
            for ix in (8..n - 8).step_by(16) {
                for fy in [0.0625f32, 0.5, 0.9375] {
                    for fx in [0.0625f32, 0.5, 0.9375] {
                        let g = ground_at(edge, idx(ix) + fx, idx(iy) + fy);
                        if g[0].abs() > 14.5 || g[2].abs() > 14.5 {
                            continue;
                        }
                        checked += 1;
                        if ground_lit_taps(edge, vp, ix, iy, [fx, fy], bias, stored) < 9 {
                            shadowed += 1;
                        }
                    }
                }
            }
        }
        (checked, shadowed)
    };
    let cpu_stored = |x: usize, y: usize| {
        let (a, b) = texel_ab(edge, idx(x) + 0.5, idx(y) + 0.5);
        stored_depth_at(vp, a, b, false)
    };
    let (checked, shadowed) = sweep(bias, &cpu_stored);
    assert!(checked > 10_000, "the sweep covers the plane: {checked}");
    assert_eq!(
        shadowed, 0,
        "{shadowed} of {checked} sand pixels read shadowed from the plane's own depth"
    );
    // The slope term is load bearing: the constant 2^-19 alone leaves the
    // far taps of the same sweep shadowed, which is the acne the old flat
    // 0.002 hid by lifting every contact shadow.
    let (_, constant_only) = sweep(SUN_SHADOW_BIAS, &cpu_stored);
    assert!(
        constant_only > 0,
        "2^-19 alone must leave acne, else the slope term proves nothing"
    );
    let Some((device, queue)) = gpu_or_skip("sun_facing_sand_plane_has_zero_shadowed_pixels")
    else {
        return;
    };
    let map = render_sun_map(device, queue, vp, edge, &plane_tris());
    let gpu_stored = |x: usize, y: usize| map[y * n + x];
    let (gpu_checked, gpu_shadowed) = sweep(bias, &gpu_stored);
    assert!(
        gpu_checked > 10_000,
        "the GPU sweep covers the plane: {gpu_checked}"
    );
    assert_eq!(
        gpu_shadowed, 0,
        "{gpu_shadowed} of {gpu_checked} sand pixels shadowed on the live map"
    );
    eprintln!(
        "sun-facing sand: 0 shadowed of {checked} cpu and {gpu_checked} gpu pixels \
         (constant-only leaves {constant_only}), bias {bias:.6}"
    );
}

#[test]
fn foot_contact_shadow_stays_within_two_pixels() {
    // The foot's back wall base centre is at map texel (edge/2, edge/2); the
    // shadow runs to lower rows. Walk it and find the first row the compare
    // calls shadowed: that distance is the peter-pan gap the bias buys. The
    // old flat 0.002 was 0.204 m of wall height at this sun (taller than the
    // 0.18 m foot), so the whole shadow vanished instead of starting late.
    let edge = sun_shadow_size_for_half(SUN_SHADOW_HALF_RES);
    let vp = sun_view_proj_for_dir(ARENA_DAY_SUN_DIR).expect("arena sun builds");
    let bias = ground_bias(edge);
    let n = usize::try_from(edge).expect("edge fits");
    let centre = n / 2;
    assert!(edge.is_multiple_of(2), "the base sits on a whole texel row");
    let mut tris = plane_tris();
    tris.extend(foot_tris());
    let walk = |stored: &dyn Fn(usize, usize) -> f32| {
        let mut first: Option<f32> = None;
        for k in 1..=16u32 {
            let t = f32::from(u16::try_from(k).expect("step fits")) / 8.0;
            let above = k.div_ceil(8);
            let whole = f32::from(u16::try_from(above - 1).expect("row fits"));
            let frac = 1.0 - (t - whole);
            let iy = centre - usize::try_from(above).expect("rows fit");
            let ix = centre;
            let lit = ground_lit_taps(edge, vp, ix, iy, [0.5, frac], bias, stored);
            if lit < 5 && first.is_none() {
                first = Some(t);
            }
        }
        first
    };
    // CPU oracle first: the analytic ground plus the foot box.
    let cpu_stored = |x: usize, y: usize| {
        let (a, b) = texel_ab(edge, idx(x) + 0.5, idx(y) + 0.5);
        stored_depth_at(vp, a, b, true)
    };
    let cpu_first = walk(&cpu_stored);
    let Some(cpu_gap) = cpu_first else {
        panic!("the foot casts no shadow in the cpu oracle");
    };
    assert!(
        cpu_gap <= 2.0,
        "contact shadow starts {cpu_gap} px from the foot, over the 2 px bar"
    );
    // Far from the foot the same plane must stay fully lit, so the gap is
    // the bias and not a broken compare.
    let away = ground_lit_taps(edge, vp, centre + 64, centre, [0.5, 0.5], bias, &cpu_stored);
    assert_eq!(away, 9, "the plane away from the foot stays lit");
    let Some((device, queue)) = gpu_or_skip("foot_contact_shadow_stays_within_two_pixels") else {
        return;
    };
    let map = render_sun_map(device, queue, vp, edge, &tris);
    let gpu_stored = |x: usize, y: usize| map[y * n + x];
    let Some(gpu_gap) = walk(&gpu_stored) else {
        panic!("the foot casts no shadow on the live map");
    };
    assert!(
        gpu_gap <= 2.0,
        "contact shadow starts {gpu_gap} px from the foot on the live map"
    );
    assert_eq!(
        ground_lit_taps(edge, vp, centre + 64, centre, [0.5, 0.5], bias, &gpu_stored),
        9,
        "the live plane away from the foot stays lit"
    );
    eprintln!("foot contact: cpu gap {cpu_gap} px, gpu gap {gpu_gap} px");
}

//! Tests for `impostor_gpu`: the CPU contracts run everywhere, the device
//! tests (bake, draw, orbit, receipt) skip with a printed line when this
//! machine has no adapter. The device tests are the RL64 accept: a baked
//! sphere differs between frames and is not empty, the impostor drawn at a
//! bake direction matches the mesh drawn there within 3 per cent, an orbit of
//! 64 views never pops, and the receipt prints atlas bytes, bake ms and draw
//! ms for 1000 impostors beside the adapter line.

use super::*;
use glam::Mat4;

const SPHERE_R: f32 = 0.5;

/// A UV sphere of radius [`SPHERE_R`] standing on y = 0, with a smooth
/// two-tone map so frames from different directions differ.
fn textured_sphere() -> BakeMesh {
    let (lon, lat) = (32u32, 16u32);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for j in 0..=lat {
        let v =
            f32::from(u16::try_from(j).unwrap_or(0)) / f32::from(u16::try_from(lat).unwrap_or(1));
        let phi = v * std::f32::consts::PI;
        for i in 0..=lon {
            let u = f32::from(u16::try_from(i).unwrap_or(0))
                / f32::from(u16::try_from(lon).unwrap_or(1));
            let theta = u * std::f32::consts::TAU;
            let n = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
            positions.push([n[0] * SPHERE_R, n[1] * SPHERE_R + SPHERE_R, n[2] * SPHERE_R]);
            normals.push(n);
            uvs.push([u, v]);
        }
    }
    let mut indices = Vec::new();
    for j in 0..lat {
        for i in 0..lon {
            let a = j * (lon + 1) + i;
            let b = a + lon + 1;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    let (w, h) = (32u32, 16u32);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let u =
                f32::from(u16::try_from(x).unwrap_or(0)) / f32::from(u16::try_from(w).unwrap_or(1));
            let v =
                f32::from(u16::try_from(y).unwrap_or(0)) / f32::from(u16::try_from(h).unwrap_or(1));
            let swirl = 0.5 + 0.5 * (u * std::f32::consts::TAU * 2.0).sin();
            let to_byte =
                |f: f32| u8::try_from((f.clamp(0.0, 1.0) * 255.0).round() as u32).unwrap_or(255);
            rgba.extend_from_slice(&[to_byte(u), to_byte(v), to_byte(swirl), 255]);
        }
    }
    BakeMesh {
        parts: vec![BakePart {
            positions,
            normals,
            uvs,
            indices,
            base_color: [1.0, 1.0, 1.0],
            texture: Some(BakeTexture {
                width: w,
                height: h,
                rgba,
            }),
            tunic: false,
        }],
    }
}

type Gpu = (
    &'static wgpu::Device,
    &'static wgpu::Queue,
    &'static wgpu::AdapterInfo,
);

fn gpu(test: &str) -> Option<Gpu> {
    crate::test_gpu::require_or_skip(test, crate::graph::gx1_probe())
}

/// RGBA8 bytes of a texture, rows unpadded.
fn read_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let row = (w * 4).next_multiple_of(256);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rl64-test-readback"),
        size: u64::from(row) * u64::from(h),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(enc.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |res| {
        res.expect("the readback buffer maps");
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .expect("the readback buffer is mapped");
    let mut out = Vec::new();
    for y in 0..usize::try_from(h).unwrap_or(0) {
        let at = y * usize::try_from(row).unwrap_or(0);
        out.extend_from_slice(&mapped[at..at + usize::try_from(w * 4).unwrap_or(0)]);
    }
    out
}

/// The frame buffers the draw binds, filled for one camera.
struct Rig {
    cam: wgpu::Buffer,
    light: wgpu::Buffer,
    fog: wgpu::Buffer,
    sh: wgpu::Buffer,
    prev: wgpu::Buffer,
    steady: wgpu::Buffer,
}

fn uniform(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rl64-test-uniform"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

impl Rig {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view_proj: [f32; 16],
        eye: [f32; 3],
    ) -> Self {
        let rig = Self {
            cam: uniform(device, 80),
            light: uniform(device, 32),
            fog: uniform(device, 128),
            sh: uniform(device, 144),
            prev: uniform(device, 64),
            steady: uniform(device, 64),
        };
        let mut cam = view_proj.to_vec();
        cam.extend_from_slice(&[eye[0], eye[1], eye[2], 0.0]);
        queue.write_buffer(&rig.cam, 0, &f32_bytes(&cam));
        queue.write_buffer(&rig.prev, 0, &f32_bytes(&view_proj));
        queue.write_buffer(&rig.steady, 0, &f32_bytes(&view_proj));
        // Sun from above-right, warm; the SH carries a flat grey ambient.
        queue.write_buffer(
            &rig.light,
            0,
            &f32_bytes(&[0.3, 0.8, 0.5, 0.0, 3.0, 2.8, 2.5, 1.0]),
        );
        let mut sh = vec![0.0f32; 36];
        sh[0] = 1.2;
        sh[1] = 1.3;
        sh[2] = 1.5;
        queue.write_buffer(&rig.sh, 0, &f32_bytes(&sh));
        // Fog off: zero density, zero plane.
        queue.write_buffer(&rig.fog, 0, &f32_bytes(&[0.0; 32]));
        rig
    }

    fn buffers(&self) -> FrameBuffers<'_> {
        FrameBuffers {
            cam: &self.cam,
            light: &self.light,
            fog: &self.fog,
            sh: &self.sh,
            prev_cam: &self.prev,
            steady_cam: &self.steady,
        }
    }
}

const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Draw the staged batches once into a fresh `w` by `h` target and return its
/// bytes (sRGB encoded) plus the wall milliseconds submit-to-idle took.
fn draw_once(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sub: &mut ImpostorSubmitter,
    rig: &Rig,
    (w, h): (u32, u32),
    readback: bool,
) -> (Vec<u8>, f64) {
    let shape = PassShape::Single(TARGET_FORMAT);
    sub.prepare(device, &rig.buffers(), &[shape]);
    let color = target_texture(
        device,
        "rl64-test-color",
        w.max(h),
        TARGET_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let depth = target_texture(
        device,
        "rl64-test-depth",
        w.max(h),
        DEPTH_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let (cv, dv) = (
        color.create_view(&wgpu::TextureViewDescriptor::default()),
        depth.create_view(&wgpu::TextureViewDescriptor::default()),
    );
    let started = engine_clock::now();
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rl64-test-pass"),
            color_attachments: &[clear_attachment(&cv)],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_viewport(0.0, 0.0, pad_f32(w), pad_f32(h), 0.0, 1.0);
        sub.encode_draw(&mut rp, shape);
    }
    queue.submit(Some(enc.finish()));
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    let bytes = if readback {
        let all = read_rgba8(device, queue, &color, w.max(h), w.max(h));
        // Square target: crop to w by h.
        let side = usize::try_from(w.max(h)).unwrap_or(0);
        let (cw, ch) = (
            usize::try_from(w).unwrap_or(0),
            usize::try_from(h).unwrap_or(0),
        );
        let mut out = Vec::with_capacity(cw * ch * 4);
        for y in 0..ch {
            out.extend_from_slice(&all[y * side * 4..y * side * 4 + cw * 4]);
        }
        out
    } else {
        Vec::new()
    };
    (bytes, ms)
}

/// Mean absolute RGB difference of two same-size RGBA images, in 0..=1.
fn mean_rgb_error(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let mut sum = 0u64;
    let mut count = 0u64;
    for (x, y) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for k in 0..3 {
            sum += u64::from(x[k].abs_diff(y[k]));
            count += 1;
        }
    }
    // Totals stay far below 2^53, so the f64 is exact.
    #[expect(clippy::cast_precision_loss, reason = "image sums stay below 2^53")]
    {
        sum as f64 / (count.max(1) as f64 * 255.0)
    }
}

fn covered(rgba: &[u8], from: usize, frame_px: usize, atlas_px: usize) -> usize {
    let (cx, cy) = (from % 8, from / 8);
    let mut count = 0;
    for y in 0..frame_px {
        for x in 0..frame_px {
            let at = ((cy * frame_px + y) * atlas_px + cx * frame_px + x) * 4;
            if rgba[at + 3] > 127 {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn potato_layout_is_eight_by_eight_frames_of_64px_in_two_mib() {
    let layout = potato_layout().expect("the potato budget fits");
    assert_eq!(layout.frames_per_side, 8);
    assert_eq!(layout.frame_px, 64);
    assert_eq!(layout.atlas_px, 512);
    assert_eq!(layout.bytes, 2 << 20);
}

#[test]
fn mesh_validation_names_the_bad_part() {
    assert!(matches!(
        BakeMesh::default().validate(),
        Err(ImpostorGpuError::EmptyMesh)
    ));
    let mut mesh = textured_sphere();
    assert!(mesh.validate().is_ok());
    mesh.parts[0].indices.push(1_000_000);
    mesh.parts[0].indices.push(0);
    mesh.parts[0].indices.push(0);
    assert!(matches!(mesh.validate(), Err(ImpostorGpuError::BadPart(0))));
    let mut mesh = textured_sphere();
    if let Some(tex) = mesh.parts[0].texture.as_mut() {
        tex.rgba.pop();
    }
    assert!(matches!(
        mesh.validate(),
        Err(ImpostorGpuError::BadTexture(0))
    ));
    let (centre, radius) = textured_sphere().bounding_sphere();
    assert!((centre[1] - SPHERE_R).abs() < 1e-4 && centre[0].abs() < 1e-4);
    assert!((radius - SPHERE_R).abs() < 2e-3, "radius {radius}");
}

/// A part with vertices and no triangles (a cut-out piece left empty) is left
/// out of the sphere and of the bake, never sliced as an empty buffer.
fn with_an_empty_part() -> BakeMesh {
    let mut mesh = textured_sphere();
    mesh.parts.push(BakePart {
        positions: vec![[40.0, 40.0, 40.0]; 3],
        normals: vec![[0.0, 1.0, 0.0]; 3],
        uvs: vec![[0.0, 0.0]; 3],
        indices: Vec::new(),
        base_color: [1.0; 3],
        texture: None,
        tunic: false,
    });
    mesh
}

#[test]
fn a_part_without_triangles_is_left_out_of_the_sphere() {
    let (centre, radius) = with_an_empty_part().bounding_sphere();
    assert!((centre[1] - SPHERE_R).abs() < 1e-4, "centre {centre:?}");
    assert!((radius - SPHERE_R).abs() < 2e-3, "radius {radius}");
}

#[test]
fn a_mesh_with_an_empty_part_still_bakes() {
    let Some((device, queue, _)) = gpu("a_mesh_with_an_empty_part_still_bakes") else {
        return;
    };
    let layout = potato_layout().expect("layout");
    let atlas = bake_atlas(device, queue, &with_an_empty_part(), layout)
        .expect("the empty part is skipped, not sliced");
    assert!((atlas.radius_m - SPHERE_R * SPHERE_MARGIN).abs() < 3e-3);
}

#[test]
fn the_frame_camera_fits_the_sphere_and_looks_at_its_centre() {
    let (centre, radius) = ([0.0, 0.5, 0.0], 0.5);
    for i in [0u32, 9, 19, 32, 45, 63] {
        let vp = Mat4::from_cols_array(&frame_view_proj(8, i, centre, radius).expect("frame"));
        let at = |p: Vec3| vp.project_point3(p);
        let mid = at(Vec3::from(centre));
        assert!(
            mid.x.abs() < 1e-5 && mid.y.abs() < 1e-5,
            "frame {i} centre {mid:?}"
        );
        assert!((0.0..1.0).contains(&mid.z), "frame {i} depth {mid:?}");
        // The sphere's right extreme along the sprite axis lands on the edge.
        let dir = frame_direction(8, i).expect("direction");
        let (right, up) = bake_basis(dir).expect("basis");
        let r = at(Vec3::from(centre) + Vec3::from(right) * radius);
        let u = at(Vec3::from(centre) + Vec3::from(up) * radius);
        assert!(
            (r.x - 1.0).abs() < 1e-4 && r.y.abs() < 1e-4,
            "frame {i} right {r:?}"
        );
        assert!(
            (u.y - 1.0).abs() < 1e-4 && u.x.abs() < 1e-4,
            "frame {i} up {u:?}"
        );
    }
}

#[test]
fn an_instance_packs_to_the_32_bytes_the_shader_reads() {
    let inst = ImpostorInstance {
        centre_m: [1.0, 2.0, 3.0],
        scale: 0.5,
        tint: [0.25, 0.5, 0.75],
        yaw_rad: 1.5,
    };
    let mut out = Vec::new();
    inst.pack(&mut out);
    assert_eq!(out.len(), IMPOSTOR_INSTANCE_BYTES);
    let word =
        |k: usize| f32::from_le_bytes([out[k * 4], out[k * 4 + 1], out[k * 4 + 2], out[k * 4 + 3]]);
    assert_eq!(
        (0..8).map(word).collect::<Vec<_>>(),
        vec![1.0, 2.0, 3.0, 0.5, 0.25, 0.5, 0.75, 1.5]
    );
    let mut bad = inst;
    bad.yaw_rad = f32::NAN;
    assert!(inst.finite() && !bad.finite());
}

/// The WGSL structs the packers write: offsets pinned to naga's own layout.
#[cfg(feature = "shader-validate")]
#[test]
fn the_wgsl_structs_match_the_packers() {
    let src = include_str!("../shaders/impostor.wgsl");
    let layout = |name: &str| {
        engine_material::compile::uniform_struct_layout(src, name)
            .unwrap_or_else(|| panic!("impostor.wgsl declares {name}"))
    };
    let offsets = |fields: &[(String, u32)], want: &[(&str, u32)]| {
        for (name, at) in want {
            let got = fields.iter().find(|(f, _)| f == name).map(|(_, o)| *o);
            assert_eq!(got, Some(*at), "field {name}");
        }
    };
    let (span, fields) = layout("ImpostorItem");
    assert_eq!(span, 32);
    offsets(
        &fields,
        &[
            ("radius", 0),
            ("frames_per_side", 4),
            ("frame_px", 8),
            ("cutoff", 12),
            ("lit", 16),
        ],
    );
    let (span, fields) = layout("Instance");
    assert_eq!(span, 32);
    offsets(
        &fields,
        &[("center", 0), ("scale", 12), ("tint", 16), ("yaw", 28)],
    );
}

#[test]
fn a_baked_sphere_differs_between_frames_and_is_not_empty() {
    let Some((device, queue, info)) = gpu("a_baked_sphere_differs_between_frames_and_is_not_empty")
    else {
        return;
    };
    let layout = potato_layout().expect("layout");
    let atlas = bake_atlas(device, queue, &textured_sphere(), layout).expect("the sphere bakes");
    let px = layout.atlas_px;
    let rgba = read_rgba8(device, queue, atlas.albedo_texture(), px, px);
    let (px_us, frame_us) = (
        usize::try_from(px).unwrap_or(0),
        usize::try_from(layout.frame_px).unwrap_or(0),
    );
    let last = usize::try_from(layout.frames_per_side * layout.frames_per_side / 2).unwrap_or(0);
    let (a, b) = (
        covered(&rgba, 0, frame_us, px_us),
        covered(&rgba, last, frame_us, px_us),
    );
    let whole = frame_us * frame_us;
    // The sphere fills pi/4 of its fitted frame (less the margin).
    let want = std::f64::consts::FRAC_PI_4 / (1.03f64 * 1.03);
    for (name, count) in [("frame 0", a), ("frame n*n/2", b)] {
        #[expect(clippy::cast_precision_loss, reason = "pixel counts are tiny")]
        let share = count as f64 / whole as f64;
        assert!(
            (share - want).abs() < 0.04,
            "{name} covers {share:.3}, want {want:.3}"
        );
    }
    // The two frames show different sides of the map.
    let cell = |from: usize| {
        let (cx, cy) = (from % 8, from / 8);
        let mut out = Vec::new();
        for y in 0..frame_us {
            let at = ((cy * frame_us + y) * px_us + cx * frame_us) * 4;
            out.extend_from_slice(&rgba[at..at + frame_us * 4]);
        }
        out
    };
    let err = mean_rgb_error(&cell(0), &cell(last));
    assert!(err > 0.02, "frames 0 and {last} differ by only {err:.4}");
    // The normal atlas carries the world normal (premultiplied by coverage);
    // the sphere has no tunic part, so the mask in alpha stays zero.
    let normals = read_rgba8(device, queue, atlas.normal_texture(), px, px);
    assert!(
        normals.chunks_exact(4).any(|t| t[2] > 100),
        "normals written"
    );
    assert!(
        normals.chunks_exact(4).all(|t| t[3] == 0),
        "no tunic part, no mask"
    );
    eprintln!(
        "impostor_gpu: adapter={} atlas={}x{} frame={}px coverage {:.3}/{:.3} bake_ms={:.2}",
        info.name,
        px,
        px,
        layout.frame_px,
        a as f64 / whole as f64,
        b as f64 / whole as f64,
        atlas.bake_ms
    );
}

#[test]
fn the_bake_is_cached_by_mesh_id_and_layout() {
    let Some((device, queue, _)) = gpu("the_bake_is_cached_by_mesh_id_and_layout") else {
        return;
    };
    let layout = potato_layout().expect("layout");
    let small = AtlasLayout::fit(4, TEXEL_BYTES_ALBEDO_NORMAL, 1 << 20, 512).expect("small");
    let mut sub = ImpostorSubmitter::new();
    assert_eq!(
        sub.stats().meshes,
        0,
        "no atlas is baked before the first call"
    );
    let mesh = textured_sphere();
    assert!(sub
        .bake(device, queue, 7, &mesh, layout, false)
        .expect("bake"));
    assert!(
        !sub.bake(device, queue, 7, &mesh, layout, false)
            .expect("cached"),
        "same id and layout"
    );
    assert!(
        sub.bake(device, queue, 7, &mesh, small, false)
            .expect("rebake"),
        "a new layout bakes"
    );
    assert!(sub
        .bake(device, queue, 8, &mesh, layout, false)
        .expect("second id"));
    let stats = sub.stats();
    assert_eq!(stats.meshes, 2);
    assert_eq!(stats.atlas_bytes, small.bytes + layout.bytes);
    assert!(!sub.has_draw(), "nothing staged, nothing drawn");
}

/// A view-projection that is the bake's own ortho camera on `dir`, and the
/// eye 10 m out on it, so the quad lands exactly on the target.
fn ortho_rig(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    dir: [f32; 3],
    centre: [f32; 3],
    radius: f32,
) -> Rig {
    let vp = dir_view_proj(dir, centre, radius).expect("view projection");
    let d = Vec3::from(dir).normalize();
    let eye = Vec3::from(centre) + d * 10.0;
    Rig::new(device, queue, vp, [eye.x, eye.y, eye.z])
}

fn one_instance(centre: [f32; 3]) -> [ImpostorInstance; 1] {
    [ImpostorInstance {
        centre_m: centre,
        scale: 1.0,
        tint: [1.0, 1.0, 1.0],
        yaw_rad: 0.0,
    }]
}

/// The mesh rendered at `dir` by the bake's own pipeline at 1x, 64 px.
fn mesh_reference(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mesh: &BakeMesh,
    dir: [f32; 3],
    centre: [f32; 3],
    radius: f32,
    px: u32,
) -> Vec<u8> {
    use wgpu::util::DeviceExt as _;
    let kit = BakeKit::new(device);
    let parts = kit.parts(device, queue, mesh);
    let vp = dir_view_proj(dir, centre, radius).expect("view projection");
    let mut bytes = f32_bytes(&vp);
    bytes.resize(usize::try_from(FRAME_STRIDE).unwrap_or(256), 0);
    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind = kit.frame_bind(device, &buf);
    let render_use = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
    let albedo = target_texture(device, "ref-albedo", px, ALBEDO_FORMAT, render_use);
    let normal = target_texture(device, "ref-normal", px, NORMAL_FORMAT, render_use);
    let depth = target_texture(
        device,
        "ref-depth",
        px,
        wgpu::TextureFormat::Depth32Float,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
    let (av, nv, dv) = (view(&albedo), view(&normal), view(&depth));
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rl64-reference"),
            color_attachments: &[clear_attachment(&av), clear_attachment(&nv)],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &dv,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        record_frames(&mut rp, &kit, &bind, &parts, 1, px);
    }
    queue.submit(Some(enc.finish()));
    read_rgba8(device, queue, &albedo, px, px)
}

#[test]
fn the_impostor_at_a_bake_direction_matches_the_mesh_within_three_percent() {
    let Some((device, queue, info)) =
        gpu("the_impostor_at_a_bake_direction_matches_the_mesh_within_three_percent")
    else {
        return;
    };
    let layout = potato_layout().expect("layout");
    let mesh = textured_sphere();
    let mut sub = ImpostorSubmitter::new();
    sub.bake(device, queue, 1, &mesh, layout, false)
        .expect("bake");
    let (centre, radius) = {
        let atlas = sub.atlas(1).expect("baked");
        (atlas.centre_m, atlas.radius_m)
    };
    sub.set_instances(device, queue, 1, &one_instance(centre))
        .expect("stage");
    let mut worst = 0.0f64;
    for index in [19u32, 27, 36, 44, 50] {
        let dir = frame_direction(8, index).expect("direction");
        let rig = ortho_rig(device, queue, dir, centre, radius);
        let (drawn, _) = draw_once(device, queue, &mut sub, &rig, (64, 64), true);
        let reference = mesh_reference(device, queue, &mesh, dir, centre, radius, 64);
        let err = mean_rgb_error(&drawn, &reference);
        worst = worst.max(err);
        assert!(
            err < 0.03,
            "frame {index}: impostor vs mesh {err:.4} of full scale"
        );
        assert!(
            drawn.chunks_exact(4).any(|p| p[0] > 20),
            "frame {index} drew something"
        );
    }
    eprintln!(
        "impostor_gpu: adapter={} impostor vs mesh worst mean error {:.4}",
        info.name, worst
    );
}

#[test]
fn an_orbit_of_64_views_changes_less_than_eight_percent_per_step() {
    let Some((device, queue, info)) =
        gpu("an_orbit_of_64_views_changes_less_than_eight_percent_per_step")
    else {
        return;
    };
    let layout = potato_layout().expect("layout");
    let mut sub = ImpostorSubmitter::new();
    sub.bake(device, queue, 1, &textured_sphere(), layout, false)
        .expect("bake");
    let (centre, radius) = {
        let atlas = sub.atlas(1).expect("baked");
        (atlas.centre_m, atlas.radius_m)
    };
    sub.set_instances(device, queue, 1, &one_instance(centre))
        .expect("stage");
    let elevation = 20.0f32.to_radians();
    let mut frames = Vec::new();
    for k in 0..64u32 {
        let az = std::f32::consts::TAU * f32::from(u16::try_from(k).unwrap_or(0)) / 64.0;
        let dir = [
            elevation.cos() * az.sin(),
            elevation.sin(),
            elevation.cos() * az.cos(),
        ];
        let rig = ortho_rig(device, queue, dir, centre, radius);
        frames.push(draw_once(device, queue, &mut sub, &rig, (64, 64), true).0);
    }
    let mut steps: Vec<f64> = (0..64)
        .map(|k| mean_rgb_error(&frames[k], &frames[(k + 1) % 64]))
        .collect();
    let worst = steps.iter().copied().fold(0.0, f64::max);
    steps.sort_by(f64::total_cmp);
    let median = steps[32];
    assert!(
        worst < 0.08,
        "an orbit step changed {worst:.4} of full scale"
    );
    assert!(
        worst < median * 3.0 + 0.01,
        "a step popped: worst {worst:.4} vs median {median:.4}"
    );
    eprintln!(
        "impostor_gpu: adapter={} orbit 64 views worst step {:.4} median {:.4}",
        info.name, worst, median
    );
}

#[test]
fn the_receipt_prints_atlas_bake_and_draw_cost_for_1000_impostors() {
    let Some((device, queue, info)) =
        gpu("the_receipt_prints_atlas_bake_and_draw_cost_for_1000_impostors")
    else {
        return;
    };
    let layout = potato_layout().expect("layout");
    let mut sub = ImpostorSubmitter::new();
    sub.bake(device, queue, 1, &textured_sphere(), layout, true)
        .expect("bake");
    let (centre, bake_ms) = {
        let atlas = sub.atlas(1).expect("baked");
        (atlas.centre_m, atlas.bake_ms)
    };
    // A 40 by 25 stand of spheres 25 to 70 m from a camera at the pit.
    let mut list = Vec::new();
    for row in 0..25u16 {
        for col in 0..40u16 {
            let x = (f32::from(col) - 19.5) * 1.6;
            let z = 25.0 + f32::from(row) * 1.8;
            let y = 3.0 + f32::from(row) * 0.4;
            list.push(ImpostorInstance {
                centre_m: [x + centre[0], y + centre[1], z + centre[2]],
                scale: 1.0,
                tint: [1.0, 1.0, 1.0],
                yaw_rad: 0.0,
            });
        }
    }
    assert_eq!(list.len(), 1000);
    sub.set_instances(device, queue, 1, &list)
        .expect("stage 1000");
    let eye = Vec3::new(0.0, 2.0, 0.0);
    let view = glam::camera::rh::view::look_at_mat4(eye, Vec3::new(0.0, 6.0, 40.0), Vec3::Y);
    let proj =
        glam::camera::rh::proj::directx::perspective(60f32.to_radians(), 16.0 / 9.0, 0.1, 300.0);
    let rig = Rig::new(
        device,
        queue,
        (proj * view).to_cols_array(),
        [eye.x, eye.y, eye.z],
    );
    let (image, _) = draw_once(device, queue, &mut sub, &rig, (640, 360), true);
    assert!(
        image
            .chunks_exact(4)
            .filter(|p| p[0] > 16 || p[1] > 16 || p[2] > 16)
            .count()
            > 2000,
        "the crowd reached the target"
    );
    let _warm = draw_once(device, queue, &mut sub, &rig, (1280, 720), false).1;
    let samples: Vec<f64> = (0..20)
        .map(|_| draw_once(device, queue, &mut sub, &rig, (1280, 720), false).1)
        .collect();
    let mut sorted = samples.clone();
    sorted.sort_by(f64::total_cmp);
    let stats = sub.stats();
    eprintln!(
        "impostor receipt: adapter=\"{}\" backend={:?} atlas_bytes={} frame_px={} bake_ms={:.2} draw_ms_1000_p50={:.3} draw_ms_1000_min={:.3} instances={} draws={}",
        info.name, info.backend, stats.atlas_bytes, layout.frame_px, bake_ms, sorted[10], sorted[0],
        stats.instances, stats.draws
    );
    assert_eq!(
        (stats.instances, stats.draws, stats.atlas_bytes),
        (1000, 1, 2 << 20)
    );
}

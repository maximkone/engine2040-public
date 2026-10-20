//! material_gallery: the shipped materials under three lights, drawn on a device.
//!
//! Consumer: the look review loop and the NS31h2 capture gate. [`gallery`]
//! draws one sphere and one slab per record under each light through
//! `engine_render::material_pipeline::pipeline_for`, so a record that compiles
//! but cannot draw is a capture the gallery cannot pin. [`PairReport`] carries
//! the analytic `engine_material::compare` numbers of a dry/wet pair: the
//! pixels say the look changed, the report says how.
//!
//! The stage is fixed: a sphere left and a slab right in world space, one
//! perspective camera, one directional light per capture. The generated WGSL
//! has no model matrix, so the meshes are built in world space.
//!
//! Units: meters on the stage, bytes per frame, `0..=1` for the linearized
//! mean luminance, `0..=255` for the sRGB peak channel, ms only in the test
//! receipt. Determinism: one device draws the same frame twice, and a capture
//! hash is FNV-1a 64 over that frame's bytes; no clock enters a capture.
//! Errors: no adapter gives an empty capture list (the test turns that into
//! `SKIPPED`); a record whose pipeline or readback fails is left out, so the
//! count refuses the run instead of pinning a frame the device never drew.
//!
//! [`gallery`] is behind the `window` feature (it needs a wgpu device); the
//! pair report and its tests compile in the default build.

use engine_material::compare::{compare, Delta};
use engine_material::MaterialSet;

/// Short names of the three gallery lights, in capture order.
pub const LIGHT_NAMES: [&str; 3] = ["day", "torch", "overcast"];

/// One light of the gallery: direction toward the light, linear color and
/// intensity, in the shader's units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LightPreset {
    /// Short name, carried into each [`Capture`].
    pub name: &'static str,
    /// Direction toward the light in world space; the shader normalizes it.
    pub dir_to_light: [f32; 3],
    /// Linear light color.
    pub color: [f32; 3],
    /// Light intensity.
    pub intensity: f32,
}

impl LightPreset {
    /// A high, near-white sun: the look the arena kit is authored against.
    #[must_use]
    pub fn day() -> Self {
        Self {
            name: "day",
            dir_to_light: [0.32, 0.84, 0.44],
            color: [1.0, 0.97, 0.92],
            intensity: 4.0,
        }
    }

    /// A low, warm fire in front of the stage.
    #[must_use]
    pub fn torch() -> Self {
        Self {
            name: "torch",
            dir_to_light: [0.18, 0.34, 0.92],
            color: [1.0, 0.52, 0.18],
            intensity: 6.0,
        }
    }

    /// A dim, cool, overcast sky from almost straight up.
    #[must_use]
    pub fn overcast() -> Self {
        Self {
            name: "overcast",
            dir_to_light: [0.0, 0.97, 0.24],
            color: [0.78, 0.82, 0.88],
            intensity: 1.6,
        }
    }

    /// Day, torch, overcast: the three lights every gallery run draws.
    #[must_use]
    pub fn presets() -> [Self; 3] {
        [Self::day(), Self::torch(), Self::overcast()]
    }
}

/// One frame of the gallery: a sphere and a slab of one record under one light.
///
/// The frame is `128 x 128` RGBA8. `frame` hashes its bytes; `mean_luminance`
/// and `peak` are read from the same bytes so a capture that drew nothing is
/// visible without decoding the hash.
#[derive(Clone, PartialEq, Debug)]
pub struct Capture {
    /// Record name (for example `marble`).
    pub material: String,
    /// Light name: `day`, `torch` or `overcast`.
    pub light: &'static str,
    /// FNV-1a 64 over the frame's RGBA8 bytes.
    pub frame: u64,
    /// Mean linear relative luminance over the frame, `0..=1`.
    pub mean_luminance: f32,
    /// Brightest sRGB channel in the frame, `0..=255`.
    pub peak: u8,
}

/// The analytic `engine_material::compare` numbers of one dry/wet pair.
#[derive(Clone, PartialEq, Debug)]
pub struct PairReport {
    /// `"dry -> wet"`, the two record names compared.
    pub pair: String,
    /// Every field of the pair's [`Delta`].
    pub delta: Delta,
}

impl PairReport {
    /// Compare `wet` against `dry`; `None` when either name is not in the set.
    #[must_use]
    pub fn of(records: &MaterialSet, dry: &str, wet: &str) -> Option<Self> {
        let dry_record = records.get(dry)?;
        let wet_record = records.get(wet)?;
        Some(Self {
            pair: format!("{dry} -> {wet}"),
            delta: compare(dry_record, wet_record),
        })
    }

    /// The shipped dry (`marble`) versus wet (`marble_wet`) marble pair.
    #[must_use]
    pub fn marble_pair(records: &MaterialSet) -> Option<Self> {
        Self::of(records, "marble", "marble_wet")
    }
}

impl core::fmt::Display for PairReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{}: delta_e {:.2}, luminance x{:.3}, highlight x{:.2}, structure changed {}",
            self.pair,
            self.delta.delta_e,
            self.delta.luminance_ratio,
            self.delta.specular_peak_ratio,
            self.delta.structure_changed
        )
    }
}

/// The window leg: draw the gallery on a wgpu device.
#[cfg(feature = "window")]
mod gpu {
    use super::{Capture, LightPreset, MaterialSet};
    use engine_core::Fnv64;
    use engine_material::compile::{compile, Compiled};
    use engine_render::material_pipeline::{pipeline_for, COLOR_FORMAT, DEPTH_FORMAT};
    use engine_render::test_gpu::NoAdapter;
    use wgpu::util::DeviceExt;

    /// Frame side in pixels; square, so aspect is one.
    const SIDE: u32 = 128;
    /// Bytes per readback row; 128 pixels is already 256-byte aligned.
    const ROW_BYTES: u32 = SIDE * 4;
    /// Floats per vertex: pos, normal, uv, then the 16-byte tangent pad.
    const VERTEX_FLOATS: usize = 12;
    /// Near clip in meters.
    const NEAR: f32 = 0.1;
    /// Far clip in meters.
    const FAR: f32 = 20.0;
    /// Eye in meters: the camera sits on +z looking at the origin.
    const EYE: [f32; 3] = [0.0, 0.22, 2.4];
    /// Vertical field of view in radians.
    const FOV_Y: f32 = 0.7;
    /// Sphere center; left of the slab.
    const SPHERE_CENTER: [f32; 3] = [-0.42, 0.0, 0.0];
    /// Sphere radius in meters.
    const SPHERE_RADIUS: f32 = 0.42;
    /// Sphere rings from pole to pole.
    const SPHERE_RINGS: usize = 16;
    /// Sphere segments around the axis.
    const SPHERE_SEGMENTS: usize = 24;
    /// Slab center; right of the sphere.
    const SLAB_CENTER: [f32; 3] = [0.43, -0.02, 0.0];
    /// Slab half extents in meters.
    const SLAB_HALF: [f32; 3] = [0.40, 0.20, 0.24];
    /// Work in progress receipt under the workspace target directory.
    const RECEIPT: &str = "../../target/ns31h/gallery.json";

    /// Open a device for one gallery run, or say why there is none.
    fn open_device() -> Result<(wgpu::Device, wgpu::Queue, String), NoAdapter> {
        engine_render::test_gpu::hold_gpu_slot();
        let instance = engine_render::backend::new_instance(wgpu::Backends::PRIMARY);
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        };
        let adapter = pollster::block_on(instance.request_adapter(&options))
            .map_err(|error| NoAdapter(format!("no wgpu adapter: {error}")))?;
        let name = adapter.get_info().name;
        let request = wgpu::DeviceDescriptor {
            label: Some("material-gallery"),
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&request))
            .map_err(|error| NoAdapter(format!("device request failed: {error}")))?;
        Ok((device, queue, name))
    }

    /// The adapter a gallery run would use, or `None` when the machine has none.
    #[must_use]
    pub fn adapter_name() -> Option<String> {
        open_device().ok().map(|(_, _, name)| name)
    }

    /// The gallery: one sphere plus slab frame per record and light.
    ///
    /// Records come in set order, lights in the given order, so capture
    /// `record_index * lights.len() + light_index` names its pair. An empty
    /// set or no adapter gives an empty list. A record whose pipeline or
    /// readback fails is left out, so the capture count refuses the run
    /// instead of pinning a frame the device never drew.
    #[must_use]
    pub fn gallery(records: &MaterialSet, lights: [LightPreset; 3]) -> Vec<Capture> {
        if records.is_empty() {
            return Vec::new();
        }
        let Ok((device, queue, _adapter)) = open_device() else {
            return Vec::new();
        };
        let (vertices, sphere_vertices, slab_vertices) = stage_mesh();
        let verts = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material-gallery-verts"),
            contents: &float_bytes(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let cam = uniform_buffer(&device, "material-gallery-cam", &camera_bytes());
        let light = uniform_buffer(&device, "material-gallery-light", &light_bytes(&lights[0]));
        let chan = uniform_buffer(&device, "material-gallery-chan", &[0u8; 32]);
        let white = white_texture_view(&device, &queue);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material-gallery-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let frame = Frame::new(&device);
        let mut captures = Vec::with_capacity(records.len() * lights.len());

        for record in records.records() {
            let compiled = compile(record);
            let Ok(pipeline) = pipeline_for(&device, &compiled) else {
                continue;
            };
            let material = uniform_buffer(
                &device,
                "material-gallery-material",
                &float_bytes(&compiled.uniform),
            );
            let layout0 = pipeline.get_bind_group_layout(0);
            let bind0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("material-gallery-bind0"),
                layout: &layout0,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: cam.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: light.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: material.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: chan.as_entire_binding(),
                    },
                ],
            });
            let bind1 = bind_textures(&device, &pipeline, &sampler, &white, &compiled);
            for preset in &lights {
                queue.write_buffer(&light, 0, &light_bytes(preset));
                let drawn = Draw {
                    pipeline: &pipeline,
                    bind0: &bind0,
                    bind1: bind1.as_ref(),
                    verts: &verts,
                    sphere: sphere_vertices,
                    slab: slab_vertices,
                };
                let Some((frame_hash, mean_luminance, peak)) = frame.draw(&device, &queue, &drawn)
                else {
                    continue;
                };
                captures.push(Capture {
                    material: record.name.clone(),
                    light: preset.name,
                    frame: frame_hash,
                    mean_luminance,
                    peak,
                });
            }
        }
        captures
    }

    /// Group 1 of a textured structure: the shared sampler plus the white
    /// stand-in in every bound slot. Untextured records need no group 1.
    fn bind_textures(
        device: &wgpu::Device,
        pipeline: &wgpu::RenderPipeline,
        sampler: &wgpu::Sampler,
        white: &wgpu::TextureView,
        compiled: &Compiled,
    ) -> Option<wgpu::BindGroup> {
        let bindings = compiled.structure.texture_bindings();
        if bindings.is_empty() {
            return None;
        }
        let layout = pipeline.get_bind_group_layout(1);
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Sampler(sampler),
        }];
        for &(_, binding) in &bindings {
            entries.push(wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(white),
            });
        }
        Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material-gallery-bind1"),
            layout: &layout,
            entries: &entries,
        }))
    }

    /// A one-pixel white 2D texture, the stand-in for a bound slot.
    fn white_texture_view(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("material-gallery-white"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255u8; 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// The color, depth and readback targets of one gallery frame.
    struct Frame {
        color: wgpu::Texture,
        depth: wgpu::Texture,
        readback: wgpu::Buffer,
    }

    /// One draw: the pipeline, both bind groups, the shared vertices and the
    /// two vertex ranges.
    struct Draw<'a> {
        pipeline: &'a wgpu::RenderPipeline,
        bind0: &'a wgpu::BindGroup,
        bind1: Option<&'a wgpu::BindGroup>,
        verts: &'a wgpu::Buffer,
        sphere: u32,
        slab: u32,
    }

    impl Frame {
        fn new(device: &wgpu::Device) -> Self {
            let size = wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            };
            let color = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("material-gallery-color"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let depth = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("material-gallery-depth"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("material-gallery-readback"),
                size: u64::from(ROW_BYTES) * u64::from(SIDE),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            Self {
                color,
                depth,
                readback,
            }
        }

        /// Draw both shapes and return the frame hash, its mean luminance and
        /// its peak sRGB channel, or `None` when the readback did not map.
        fn draw(
            &self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            draw: &Draw<'_>,
        ) -> Option<(u64, f32, u8)> {
            let color_view = self
                .color
                .create_view(&wgpu::TextureViewDescriptor::default());
            let depth_view = self
                .depth
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("material-gallery-frame"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("material-gallery-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
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
                pass.set_pipeline(draw.pipeline);
                pass.set_bind_group(0, draw.bind0, &[]);
                if let Some(bind1) = draw.bind1 {
                    pass.set_bind_group(1, bind1, &[]);
                }
                pass.set_vertex_buffer(0, draw.verts.slice(..));
                pass.draw(0..draw.sphere, 0..1);
                pass.draw(draw.sphere..draw.sphere + draw.slab, 0..1);
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.color,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(ROW_BYTES),
                        rows_per_image: Some(SIDE),
                    },
                },
                wgpu::Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(encoder.finish()));
            let slice = self.readback.slice(..);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
            let polled = device.poll(wgpu::PollType::wait_indefinitely()).is_ok();
            let mapped = polled
                && receiver
                    .recv()
                    .ok()
                    .and_then(|result| result.ok())
                    .is_some();
            if !mapped {
                return None;
            }
            let mut hash = Fnv64::new();
            let stats = slice.get_mapped_range().ok().map(|bytes| {
                hash.write(&bytes);
                frame_stats(&bytes)
            });
            self.readback.unmap();
            let (mean_luminance, peak) = stats?;
            Some((hash.finish(), mean_luminance, peak))
        }
    }

    /// Mean linear luminance and peak sRGB channel over RGBA8 bytes.
    fn frame_stats(bytes: &[u8]) -> (f32, u8) {
        let mut sum = 0.0f32;
        let mut peak = 0u8;
        let mut pixels = 0u32;
        for pixel in bytes.chunks_exact(4) {
            let r = srgb_to_linear(pixel[0]);
            let g = srgb_to_linear(pixel[1]);
            let b = srgb_to_linear(pixel[2]);
            sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
            peak = peak.max(pixel[0].max(pixel[1]).max(pixel[2]));
            pixels += 1;
        }
        let mean = if pixels == 0 {
            0.0
        } else {
            sum / f32::from(u16::try_from(pixels).unwrap_or(u16::MAX))
        };
        (mean, peak)
    }

    /// One sRGB-encoded channel as linear light.
    fn srgb_to_linear(value: u8) -> f32 {
        let c = f32::from(value) / 255.0;
        if c <= 0.040_45 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// The stage worth of floats: sphere vertices, then slab vertices.
    fn stage_mesh() -> (Vec<f32>, u32, u32) {
        let sphere = sphere_mesh();
        let slab = slab_mesh();
        let sphere_vertices = vertex_count(&sphere);
        let slab_vertices = vertex_count(&slab);
        let mut all = sphere;
        all.extend_from_slice(&slab);
        (all, sphere_vertices, slab_vertices)
    }

    /// Vertex count of a float image, refusing an image that is not whole
    /// vertices.
    fn vertex_count(floats: &[f32]) -> u32 {
        debug_assert_eq!(
            floats.len() % VERTEX_FLOATS,
            0,
            "a vertex image holds whole {} float vertices",
            VERTEX_FLOATS
        );
        u32::try_from(floats.len() / VERTEX_FLOATS).unwrap_or(u32::MAX)
    }

    /// `index / steps` as a float, without a numeric cast.
    fn fraction(index: usize, steps: usize) -> f32 {
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        let steps = u16::try_from(steps).unwrap_or(1).max(1);
        f32::from(index) / f32::from(steps)
    }

    /// A UV sphere in world space, expanded to triangle vertices (no index
    /// buffer), normals outward.
    fn sphere_mesh() -> Vec<f32> {
        let mut out = Vec::with_capacity(SPHERE_RINGS * SPHERE_SEGMENTS * 6 * VERTEX_FLOATS);
        for ring in 0..SPHERE_RINGS {
            let v0 = fraction(ring, SPHERE_RINGS);
            let v1 = fraction(ring + 1, SPHERE_RINGS);
            let theta0 = v0 * std::f32::consts::PI;
            let theta1 = v1 * std::f32::consts::PI;
            for segment in 0..SPHERE_SEGMENTS {
                let u0 = fraction(segment, SPHERE_SEGMENTS);
                let u1 = fraction(segment + 1, SPHERE_SEGMENTS);
                let (p00, n00) = sphere_point(theta0, u0);
                let (p10, n10) = sphere_point(theta0, u1);
                let (p01, n01) = sphere_point(theta1, u0);
                let (p11, n11) = sphere_point(theta1, u1);
                push_vertex(&mut out, p00, n00, [u0, v0]);
                push_vertex(&mut out, p10, n10, [u1, v0]);
                push_vertex(&mut out, p11, n11, [u1, v1]);
                push_vertex(&mut out, p00, n00, [u0, v0]);
                push_vertex(&mut out, p11, n11, [u1, v1]);
                push_vertex(&mut out, p01, n01, [u0, v1]);
            }
        }
        out
    }

    /// Point and outward normal of the sphere at one latitude and longitude.
    fn sphere_point(theta: f32, phi: f32) -> ([f32; 3], [f32; 3]) {
        let normal = [
            theta.sin() * phi.cos(),
            theta.cos(),
            theta.sin() * phi.sin(),
        ];
        let point = [
            SPHERE_CENTER[0] + normal[0] * SPHERE_RADIUS,
            SPHERE_CENTER[1] + normal[1] * SPHERE_RADIUS,
            SPHERE_CENTER[2] + normal[2] * SPHERE_RADIUS,
        ];
        (point, normal)
    }

    /// A box slab in world space, six flat quads, one normal per face.
    fn slab_mesh() -> Vec<f32> {
        let mut out = Vec::with_capacity(6 * 6 * VERTEX_FLOATS);
        let [hx, hy, hz] = SLAB_HALF;
        let [cx, cy, cz] = SLAB_CENTER;
        let corner = |sx: f32, sy: f32, sz: f32| [cx + sx * hx, cy + sy * hy, cz + sz * hz];
        let faces: [([[f32; 3]; 4], [f32; 3]); 6] = [
            (
                [
                    corner(-1.0, -1.0, 1.0),
                    corner(1.0, -1.0, 1.0),
                    corner(1.0, 1.0, 1.0),
                    corner(-1.0, 1.0, 1.0),
                ],
                [0.0, 0.0, 1.0],
            ),
            (
                [
                    corner(1.0, -1.0, -1.0),
                    corner(-1.0, -1.0, -1.0),
                    corner(-1.0, 1.0, -1.0),
                    corner(1.0, 1.0, -1.0),
                ],
                [0.0, 0.0, -1.0],
            ),
            (
                [
                    corner(1.0, -1.0, 1.0),
                    corner(1.0, -1.0, -1.0),
                    corner(1.0, 1.0, -1.0),
                    corner(1.0, 1.0, 1.0),
                ],
                [1.0, 0.0, 0.0],
            ),
            (
                [
                    corner(-1.0, -1.0, -1.0),
                    corner(-1.0, -1.0, 1.0),
                    corner(-1.0, 1.0, 1.0),
                    corner(-1.0, 1.0, -1.0),
                ],
                [-1.0, 0.0, 0.0],
            ),
            (
                [
                    corner(-1.0, 1.0, 1.0),
                    corner(1.0, 1.0, 1.0),
                    corner(1.0, 1.0, -1.0),
                    corner(-1.0, 1.0, -1.0),
                ],
                [0.0, 1.0, 0.0],
            ),
            (
                [
                    corner(-1.0, -1.0, -1.0),
                    corner(1.0, -1.0, -1.0),
                    corner(1.0, -1.0, 1.0),
                    corner(-1.0, -1.0, 1.0),
                ],
                [0.0, -1.0, 0.0],
            ),
        ];
        for (corners, normal) in faces {
            quad(&mut out, corners, normal);
        }
        out
    }

    /// Two triangles of one face, with UV corners.
    fn quad(out: &mut Vec<f32>, corners: [[f32; 3]; 4], normal: [f32; 3]) {
        let uv = [[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for &index in &[0usize, 1, 2, 0, 2, 3] {
            push_vertex(out, corners[index], normal, uv[index]);
        }
    }

    /// One 48-byte vertex: position, normal, uv, then the tangent pad.
    fn push_vertex(out: &mut Vec<f32>, pos: [f32; 3], normal: [f32; 3], uv: [f32; 2]) {
        out.extend_from_slice(&pos);
        out.extend_from_slice(&normal);
        out.extend_from_slice(&uv);
        out.extend_from_slice(&[0.0; 4]);
    }

    /// A uniform buffer of `bytes`, exact size so the layout's
    /// `min_binding_size` matches.
    fn uniform_buffer(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// Floats as little-endian bytes.
    fn float_bytes(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    /// The fixed camera: 16 view-projection columns, the eye, one pad float.
    fn camera_bytes() -> Vec<u8> {
        let view_proj = multiply(perspective(FOV_Y, 1.0, NEAR, FAR), look_at());
        let columns = to_columns(view_proj);
        let mut floats = Vec::with_capacity(20);
        for column in columns {
            floats.extend_from_slice(&column);
        }
        floats.extend_from_slice(&EYE);
        floats.push(0.0);
        float_bytes(&floats)
    }

    /// The light as `LightU`: direction, pad, color, intensity.
    fn light_bytes(preset: &LightPreset) -> Vec<u8> {
        let mut floats = Vec::with_capacity(8);
        floats.extend_from_slice(&preset.dir_to_light);
        floats.push(0.0);
        floats.extend_from_slice(&preset.color);
        floats.push(preset.intensity);
        float_bytes(&floats)
    }

    /// A row-major 4x4 matrix.
    type Mat4 = [[f32; 4]; 4];

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    fn normalize(a: [f32; 3]) -> [f32; 3] {
        let inverse = 1.0 / dot(a, a).sqrt();
        [a[0] * inverse, a[1] * inverse, a[2] * inverse]
    }

    /// Row-major look-at from the fixed eye to the origin.
    fn look_at() -> Mat4 {
        let forward = normalize(sub([0.0, 0.0, 0.0], EYE));
        let side = normalize(engine_core::vec3::cross3(forward, [0.0, 1.0, 0.0]));
        let up = engine_core::vec3::cross3(side, forward);
        [
            [side[0], side[1], side[2], -dot(side, EYE)],
            [up[0], up[1], up[2], -dot(up, EYE)],
            [-forward[0], -forward[1], -forward[2], dot(forward, EYE)],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }

    /// Right-handed perspective with the wgpu depth range `0..=1`.
    fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
        let f = 1.0 / (fov_y * 0.5).tan();
        [
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), near * far / (near - far)],
            [0.0, 0.0, -1.0, 0.0],
        ]
    }

    /// Row-major product of two 4x4 matrices.
    fn multiply(a: Mat4, b: Mat4) -> Mat4 {
        let mut out = [[0.0f32; 4]; 4];
        for (row, line) in out.iter_mut().enumerate() {
            for (column, cell) in line.iter_mut().enumerate() {
                *cell = (0..4).map(|k| a[row][k] * b[k][column]).sum();
            }
        }
        out
    }

    /// The same matrix as the 16 columns the shader's `mat4x4` reads.
    fn to_columns(matrix: Mat4) -> Mat4 {
        let mut columns = [[0.0f32; 4]; 4];
        for (row, line) in matrix.iter().enumerate() {
            for (column, value) in line.iter().enumerate() {
                columns[column][row] = *value;
            }
        }
        columns
    }

    /// The receipt path of this run, under the workspace target directory.
    #[must_use]
    pub fn receipt_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(RECEIPT)
    }
}

#[cfg(feature = "window")]
pub use gpu::{adapter_name, gallery, receipt_path};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_gallery_pair_report_of_dry_and_wet_marble_is_stable() {
        let records = MaterialSet::shipped();
        let first = PairReport::marble_pair(&records).expect("marble and marble_wet ship");
        let second = PairReport::marble_pair(&records).expect("marble and marble_wet ship");
        assert_eq!(first, second, "the same pair gives the same report");
        assert_eq!(
            first.to_string(),
            second.to_string(),
            "the printed report is stable twice"
        );
        assert!(
            first.delta.luminance_ratio < 0.95,
            "wet marble is darker: x{:.3}",
            first.delta.luminance_ratio
        );
        assert!(
            first.delta.specular_peak_ratio >= 2.0,
            "wet marble peaks at least twice as bright: x{:.2}",
            first.delta.specular_peak_ratio
        );
        eprintln!("NS31h2 {first}");
    }

    #[test]
    fn material_gallery_ships_nineteen_records_under_three_lights() {
        let records = MaterialSet::shipped();
        assert_eq!(records.len(), 19, "the shipped gallery set");
        let presets = LightPreset::presets();
        assert_eq!(presets.map(|light| light.name), LIGHT_NAMES);
        // The three lights must actually differ, or the captures repeat.
        assert_ne!(presets[0].dir_to_light, presets[1].dir_to_light);
        assert_ne!(presets[1].dir_to_light, presets[2].dir_to_light);
        assert_ne!(presets[0].intensity, presets[2].intensity);
    }

    #[cfg(feature = "window")]
    #[test]
    fn material_gallery_renders_nineteen_records_under_three_lights() {
        let records = MaterialSet::shipped();
        assert_eq!(records.len(), 19, "the shipped gallery set");
        let start = std::time::Instant::now();
        let captures = gallery(&records, LightPreset::presets());
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        if captures.is_empty() {
            let probe: Result<(), engine_render::test_gpu::NoAdapter> = Err(
                engine_render::test_gpu::NoAdapter("no wgpu adapter".to_string()),
            );
            assert!(
                engine_render::test_gpu::require_or_skip("material_gallery", probe).is_none(),
                "no adapter is a skip, not a pass"
            );
            return;
        }
        assert_eq!(captures.len(), 57, "19 records x 3 lights");
        assert!(
            elapsed_ms <= 120_000.0,
            "57 captures took {elapsed_ms:.0} ms, bar 120000"
        );
        // A transparent record (glass) contributes almost nothing over the
        // black clear, so only opaque records must show a lit pixel.
        let blended: Vec<&str> = records
            .records()
            .iter()
            .filter(|record| engine_material::compile::Structure::of(record).blended())
            .map(|record| record.name.as_str())
            .collect();
        for capture in &captures {
            if blended.contains(&capture.material.as_str()) {
                continue;
            }
            assert!(
                capture.peak > 0,
                "{} under {} drew no lit pixel",
                capture.material,
                capture.light
            );
        }
        let at = |name: &str, light: &str| {
            captures
                .iter()
                .find(|capture| capture.material == name && capture.light == light)
                .unwrap_or_else(|| panic!("{name} under {light} is a capture"))
                .frame
        };
        assert_ne!(
            at("marble_wet", "day"),
            at("marble", "day"),
            "wet marble reaches the frame"
        );
        assert_ne!(
            at("marble", "day"),
            at("marble", "torch"),
            "the light preset reaches the frame"
        );
        let adapter = adapter_name().unwrap_or_else(|| "none".to_string());
        let path = receipt_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("the ns31h receipt directory");
        }
        let body = format!(
            "{{\n  \"adapter\": \"{adapter}\",\n  \"records\": {},\n  \"lights\": [\"{}\", \"{}\", \"{}\"],\n  \"captures\": {},\n  \"captures_ms\": {elapsed_ms:.1},\n  \"bar_ms\": 120000,\n  \"ok\": true\n}}\n",
            records.len(),
            LIGHT_NAMES[0],
            LIGHT_NAMES[1],
            LIGHT_NAMES[2],
            captures.len(),
        );
        std::fs::write(&path, &body).expect("write the ns31h receipt");
        eprintln!("NS31h2 gallery receipt {}: {body}", path.display());
    }
}

//! material_swatch: six shipped materials as a lit sphere, gated against the
//! CPU twin.
//!
//! Every swatch is drawn at 128x128 through `pipeline_for` (one pipeline per
//! compiled structure, built from the generated WGSL), the center block of
//! pixels is read back, and the block is compared per channel to
//! `engine_material::twin::twin_lit_rgb` within `TOLERANCE` 8-bit steps. The
//! twin runs the same fragment chain on the CPU: the layer twin
//! (`engine_material::channels::twin_layers`) over the packed uniform, one
//! directional light, the ACES fit and the sRGB encode.
//!
//! Why: the material shader is generated text and the layered look is data, so
//! nothing but a rendered pixel pins what the pair computes. The furnace test
//! pins the energy of the same lobe math without a device; this pins the color
//! on one.
//!
//! The sphere is small and the eye close, so the swatch fills the frame. The
//! center block is the one place the twin can name exactly: the surface point
//! at the middle of the frame has the normal straight at the eye, and a sphere
//! of this radius gives the fragment's screen-derivative pair the curvature
//! `CURVATURE_PER_INVERSE_METER / RADIUS` at every point.
//!
//! Units: RGBA8 pixels (the fragment writes sRGB into a linear target) against
//! `0..=1` twin values. Determinism: one frame is compared to the twin, never
//! to another frame, so a device that shades differently fails here. Errors: a
//! record whose pipeline does not build panics with wgpu's own text; a missing
//! adapter prints one `SKIPPED` line through `require_or_skip` and the test
//! passes, as the other device tests do.

#![cfg(feature = "window")]

use engine_material::channels::{pack_channels, Coverage};
use engine_material::compile::compile;
use engine_material::skin::CURVATURE_PER_INVERSE_METER;
use engine_material::twin::{twin_lit_rgb, TwinCamera, TwinLight};
use engine_material::MaterialSet;
use engine_render::material_pipeline::{pipeline_for, COLOR_FORMAT, DEPTH_FORMAT};
use engine_render::test_gpu::{hold_gpu_slot, require_or_skip, NoAdapter};
use engine_render::v1::{Camera, CameraUniform, LightUniform};

/// Side of the swatch frame in pixels.
const SIDE: u32 = 128;
/// Bytes per row of a 128-pixel RGBA8 frame; already 256-byte aligned.
const ROW_BYTES: u32 = SIDE * 4;
/// Pixels per side of the center block that stands for the shaded point.
const BLOCK: u32 = 4;
/// Per-channel 8-bit distance the Accept line allows.
const TOLERANCE: u8 = 2;
/// Longitudes and latitudes of the swatch sphere, so a vertex sits on the
/// point the twin names.
const LON: u32 = 64;
/// Latitude bands.
const LAT: u32 = 32;
/// Vertices drawn: six per sphere quad.
const VERTEX_COUNT: u32 = LON * LAT * 6;
/// Sphere radius in meters.
const RADIUS: f32 = 0.5;
/// Eye height above the pit floor: the sphere fills about 85% of the frame.
const EYE_Z: f32 = 1.6;
/// Vertical field of view in radians.
const FOV_Y: f32 = 0.785_398_1;
/// Coverage every swatch is drawn at: the blood channel runs the layer code of
/// the records that carry a blood layer, and leaves the ones that do not
/// untouched. A wet layer is left out on purpose: the fragment mixes a wet
/// layer's coat ior to 1.33, a value the layer twin does not report.
const BLOOD: u8 = 200;
/// The six records of `MaterialSet::shipped()` the Accept line names.
const SWATCHES: [&str; 6] = ["steel", "marble_wet", "cloth", "skin", "ceramic", "glass"];

/// The swatch sphere in the 48-byte stride the generated vertex stage reads:
/// position, normal, uv, then the tangent pad the layout leaves unread.
fn sphere_vertices() -> Vec<u8> {
    let mut out = Vec::with_capacity(VERTEX_COUNT as usize * 48);
    let at = |lon: u32, lat: u32| -> (f32, f32) {
        (
            std::f32::consts::PI * lat as f32 / LAT as f32,
            std::f32::consts::TAU * lon as f32 / LON as f32,
        )
    };
    for lat in 0..LAT {
        for lon in 0..LON {
            let corners = [
                (lon, lat),
                (lon + 1, lat),
                (lon + 1, lat + 1),
                (lon, lat + 1),
            ];
            for corner in [[0usize, 1, 2], [0, 2, 3]] {
                for index in corner {
                    let (corner_lon, corner_lat) = corners[index];
                    let (theta, phi) = at(corner_lon, corner_lat);
                    let normal = [
                        theta.sin() * phi.cos(),
                        theta.cos(),
                        theta.sin() * phi.sin(),
                    ];
                    let position = [normal[0] * RADIUS, normal[1] * RADIUS, normal[2] * RADIUS];
                    let uv = [
                        corner_lon as f32 / LON as f32,
                        corner_lat as f32 / LAT as f32,
                    ];
                    for value in position.iter().chain(normal.iter()).chain(uv.iter()) {
                        out.extend_from_slice(&value.to_le_bytes());
                    }
                    out.extend_from_slice(&[0u8; 16]);
                }
            }
        }
    }
    out
}

/// The swatch camera: one meter and a half away on the +z axis, looking at the
/// sphere in the origin, square frame.
fn camera_uniform() -> CameraUniform {
    let mut camera = Camera::new([0.0, 0.0, EYE_Z], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], FOV_Y);
    camera.aspect = 1.0;
    CameraUniform {
        view_proj_cols: camera
            .view_proj()
            .expect("the swatch camera is valid")
            .to_cols_array_2d(),
        camera_pos: [0.0, 0.0, EYE_Z],
    }
}

/// The one directional light: white at radiance 1, mostly head-on so the
/// brushed metal's specular lobe lands inside the frame's center block.
fn light_uniform() -> LightUniform {
    LightUniform {
        dir_to_light: [0.35, 0.55, 1.0],
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
    }
}

/// The point the center block stands for: the sphere's front point, with the
/// normal toward the eye and the curvature a sphere of `RADIUS` gives.
fn twin_camera() -> TwinCamera {
    TwinCamera {
        pos: [0.0, 0.0, EYE_Z],
        point: [0.0, 0.0, RADIUS],
        normal: [0.0, 0.0, 1.0],
        curvature: CURVATURE_PER_INVERSE_METER / RADIUS,
    }
}

/// A uniform buffer holding `bytes`.
fn uniform_buffer(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// A uniform buffer holding `floats` little-endian.
fn floats_buffer(device: &wgpu::Device, label: &str, floats: &[f32]) -> wgpu::Buffer {
    let mut bytes = Vec::with_capacity(floats.len() * 4);
    for value in floats {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    uniform_buffer(device, label, &bytes)
}

/// A render target of `format` that can also be copied out.
fn target(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The device the swatches are drawn on, or a skip line when the machine has
/// no adapter.
fn device_or_skip(test: &str) -> Option<(wgpu::Device, wgpu::Queue, String)> {
    hold_gpu_slot();
    let instance = engine_render::backend::new_instance(wgpu::Backends::PRIMARY);
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    };
    let probe = pollster::block_on(instance.request_adapter(&options))
        .map_err(|error| NoAdapter(format!("no wgpu adapter: {error}")));
    let adapter = require_or_skip(test, probe)?;
    let request = wgpu::DeviceDescriptor {
        label: Some("material-swatch"),
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&request))
        .unwrap_or_else(|error| panic!("adapter present, device request failed: {error}"));
    Some((device, queue, adapter.get_info().name))
}

/// Everything one swatch frame needs: the sphere, the shared camera, light and
/// channel uniforms, one color and one depth target and a readback buffer.
struct Swatch {
    device: wgpu::Device,
    queue: wgpu::Queue,
    verts: wgpu::Buffer,
    cam: wgpu::Buffer,
    light: wgpu::Buffer,
    chan: wgpu::Buffer,
    color: wgpu::Texture,
    depth: wgpu::Texture,
    readback: wgpu::Buffer,
}

impl Swatch {
    /// The shared frame state on `device` at `coverage`.
    fn new(device: wgpu::Device, queue: wgpu::Queue, coverage: &Coverage) -> Self {
        let verts = {
            use wgpu::util::DeviceExt;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("swatch-sphere"),
                contents: &sphere_vertices(),
                usage: wgpu::BufferUsages::VERTEX,
            })
        };
        let cam = uniform_buffer(&device, "swatch-cam", &camera_uniform().to_bytes());
        let light = uniform_buffer(&device, "swatch-light", &light_uniform().to_bytes());
        let chan = floats_buffer(&device, "swatch-chan", &pack_channels(coverage));
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swatch-readback"),
            size: u64::from(ROW_BYTES) * u64::from(SIDE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Self {
            color: target(&device, "swatch-color", COLOR_FORMAT),
            depth: target(&device, "swatch-depth", DEPTH_FORMAT),
            device,
            queue,
            verts,
            cam,
            light,
            chan,
            readback,
        }
    }

    /// Draw one sphere frame with `pipeline` and its material `uniform`, clear
    /// to black, and return the RGBA8 pixels.
    fn present(&self, pipeline: &wgpu::RenderPipeline, uniform: &wgpu::Buffer) -> Vec<u8> {
        let layout = pipeline.get_bind_group_layout(0);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("swatch-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.cam.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.light.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.chan.as_entire_binding(),
                },
            ],
        });
        let color_view = self
            .color
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = self
            .depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("swatch-frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("swatch-pass"),
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, self.verts.slice(..));
            pass.draw(0..VERTEX_COUNT, 0..1);
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
        self.queue.submit(Some(encoder.finish()));
        let slice = self.readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll the swatch frame");
        receiver
            .recv()
            .expect("the map channel answered")
            .expect("the swatch readback mapped");
        let pixels = slice
            .get_mapped_range()
            .expect("the frame's bytes are readable")
            .to_vec();
        self.readback.unmap();
        pixels
    }
}

/// Mean of the center `BLOCK` by `BLOCK` pixels of one RGBA8 frame, per color
/// channel.
fn center_block(frame: &[u8]) -> [u8; 3] {
    let side = SIDE as usize;
    let block = BLOCK as usize;
    let start = side / 2 - block / 2;
    let mut sums = [0u32; 3];
    for y in start..start + block {
        for x in start..start + block {
            let at = (y * side + x) * 4;
            for (sum, byte) in sums.iter_mut().zip(&frame[at..at + 3]) {
                *sum += u32::from(*byte);
            }
        }
    }
    let count = (block * block) as u32;
    std::array::from_fn(|i| u8::try_from((sums[i] + count / 2) / count).unwrap_or(0))
}

/// Six shipped materials, each drawn as a lit sphere and compared to the twin
/// at the block that stands for the sphere's front point.
///
/// The shipped glass is fully transmissive at this thickness (weight 1,
/// attenuation white), so the blended pass premultiplies its fragment to
/// nothing over the black clear: its swatch proves the twin's alpha path, and
/// the other five prove the lit color.
#[test]
fn shipped_materials_match_the_twin_at_the_sphere_center() {
    let Some((device, queue, adapter)) = device_or_skip("material_swatch") else {
        return;
    };
    let coverage = Coverage {
        blood: BLOOD,
        ..Coverage::default()
    };
    let swatch = Swatch::new(device, queue, &coverage);
    let set = MaterialSet::shipped();
    let camera = twin_camera();
    let light = TwinLight {
        dir_to_light: light_uniform().dir_to_light,
        radiance: light_uniform().color,
    };
    let mut lit_swatches = 0;
    let mut failures: Vec<String> = Vec::new();
    for name in SWATCHES {
        let record = set.get(name).unwrap_or_else(|| panic!("shipped {name}"));
        let compiled = compile(record);
        let pipeline = pipeline_for(&swatch.device, &compiled)
            .unwrap_or_else(|error| panic!("{name} builds one pipeline: {error}"));
        let uniform = floats_buffer(&swatch.device, "swatch-material", &compiled.uniform);
        let block = center_block(&swatch.present(&pipeline, &uniform));
        let expected = twin_lit_rgb(record, &coverage, &camera, &light);
        let mut worst = 0u8;
        for (channel, value) in block.iter().enumerate() {
            let twin = (expected[channel] * 255.0).round().clamp(0.0, 255.0) as u8;
            worst = worst.max(twin.abs_diff(*value));
        }
        if block != [0, 0, 0] {
            lit_swatches += 1;
        }
        eprintln!(
            "material_swatch {name} on {adapter}: {block:?} against twin {expected:?} ({worst} steps)"
        );
        if worst > TOLERANCE {
            failures.push(format!(
                "{name}: center block {block:?} is {worst} steps from the twin {expected:?}"
            ));
        }
    }
    assert!(
        lit_swatches >= 5,
        "only {lit_swatches} of {} swatches drew a lit sphere: the frame is not the swatches",
        SWATCHES.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

//! lut_upload: the GPU form of a `.cube` look table (gap E11).
//!
//! [`LutTexture::new`] turns a parsed [`Lut3d`] into a `D3` `Rgba16Float` texture
//! plus a clamp-to-edge linear sampler, bytes straight from
//! [`Lut3d::rgba16f_table`]. [`LutSlot`] is the presenter-side holder: it
//! uploads once per table and drops the texture when the table changes, so the
//! frame loop never re-uploads. Units: the texture is `size x size x size`
//! with blue on the fastest axis and normalized coordinates `0..=1`; the CPU
//! statement of the same fetch is [`Lut3d::sample_texel`] in texel units.
//!
//! Determinism: presentation only. The upload is a pure function of the table
//! bytes, so two uploads of one table give the same texels; nothing here feeds
//! the tick hash.
//!
//! Consumers: the presenter's `set_look` seam (this module's `LutSlot`), and
//! the device test that compares the GPU fetch against the CPU oracle. The
//! forward shader's fetch is the shader owner's change; the ready-to-paste
//! WGSL sits in `post::lut`'s module doc.

use crate::post::lut::Lut3d;

/// Texture format of the look table: filterable in core WebGPU.
pub const LUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Smallest accepted table edge.
pub const LUT_SIZE_MIN: u16 = 2;
/// Largest accepted table edge (`post::lut::LUT_SIZE_MAX`).
pub const LUT_SIZE_MAX: u16 = 129;
/// Uploaded bytes per texel: four f16 channels.
pub const LUT_BYTES_PER_TEXEL: usize = 8;

/// Why a look table could not reach the GPU. Every variant names the value
/// and [`std::fmt::Display`] adds the one fix.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LutUploadError {
    /// The table edge is outside the upload band.
    Size {
        /// Edge the table carries.
        size: u16,
        /// Smallest accepted edge.
        min: u16,
        /// Largest accepted edge.
        max: u16,
    },
    /// The byte count is not `size^3 * 8`.
    Bytes {
        /// Bytes the table produced.
        got: usize,
        /// Bytes `size^3 * 8` needs.
        expected: usize,
    },
    /// The format cannot be linearly filtered on a core device.
    Filterable {
        /// Format that was refused, as a short name.
        format: &'static str,
        /// The one fix.
        fix: &'static str,
    },
}

impl std::fmt::Display for LutUploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Size { size, min, max } => write!(
                f,
                "look table edge {size} is outside {min}..={max}; cook the .cube at a size in that band"
            ),
            Self::Bytes { got, expected } => write!(
                f,
                "look table bytes are {got}, not {expected}; upload the table's Rgba16Float bytes"
            ),
            Self::Filterable { format, fix } => {
                write!(f, "look table format {format} is not filterable; {fix}")
            }
        }
    }
}

impl std::error::Error for LutUploadError {}

/// Refuse a texture format a linear sampler cannot read on a core device.
///
/// # Errors
///
/// [`LutUploadError::Filterable`] for a 32-bit float or non-float format,
/// naming the fix.
pub fn ensure_filterable(format: wgpu::TextureFormat) -> Result<(), LutUploadError> {
    match format {
        wgpu::TextureFormat::Rgba16Float => Ok(()),
        wgpu::TextureFormat::Rgba32Float
        | wgpu::TextureFormat::Rg32Float
        | wgpu::TextureFormat::R32Float => Err(LutUploadError::Filterable {
            format: "32-bit float",
            fix: "upload Rgba16Float, or open the device with FLOAT32_FILTERABLE",
        }),
        _ => Err(LutUploadError::Filterable {
            format: "non-float",
            fix: "upload Rgba16Float",
        }),
    }
}

/// Bytes one uploaded table takes: `size^3` texels of four f16 channels.
pub fn lut_bytes(size: u16) -> usize {
    let edge = usize::from(size);
    edge.saturating_mul(edge)
        .saturating_mul(edge)
        .saturating_mul(LUT_BYTES_PER_TEXEL)
}

/// A look table on the GPU: the `D3` view of its 3D texture and the linear
/// clamp-to-edge sampler the shader binds beside it.
pub struct LutTexture {
    /// `D3` view of the uploaded `size x size x size` `Rgba16Float` texture.
    pub view: wgpu::TextureView,
    /// Linear, clamp-to-edge sampler.
    pub sampler: wgpu::Sampler,
}

impl LutTexture {
    /// Upload `table` as a 3D texture, bytes from
    /// [`Lut3d::rgba16f_table`], row layout `size * 8` bytes per row and
    /// `size` rows per slice.
    ///
    /// # Errors
    ///
    /// [`LutUploadError::Size`] outside `2..=129`,
    /// [`LutUploadError::Bytes`] when the upload bytes disagree with
    /// `size^3 * 8`, [`LutUploadError::Filterable`] when the format cannot be
    /// sampled linearly.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        table: &Lut3d,
    ) -> Result<Self, LutUploadError> {
        let size = table.size();
        if !(LUT_SIZE_MIN..=LUT_SIZE_MAX).contains(&size) {
            return Err(LutUploadError::Size {
                size,
                min: LUT_SIZE_MIN,
                max: LUT_SIZE_MAX,
            });
        }
        ensure_filterable(LUT_FORMAT)?;
        let bytes = table.rgba16f_table();
        let expected = lut_bytes(size);
        if bytes.len() != expected {
            return Err(LutUploadError::Bytes {
                got: bytes.len(),
                expected,
            });
        }
        let edge = u32::from(size);
        let extent = wgpu::Extent3d {
            width: edge,
            height: edge,
            depth_or_array_layers: edge,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("look-lut-3d"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: LUT_FORMAT,
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
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(
                    edge.saturating_mul(u32::try_from(LUT_BYTES_PER_TEXEL).unwrap_or(8)),
                ),
                rows_per_image: Some(edge),
            },
            extent,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("look-lut-view"),
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("look-lut-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Ok(Self { view, sampler })
    }
}

/// The presenter's one look slot: uploads on a table change, drops on `None`,
/// and does nothing while the table digest stays. On an upload error the
/// previous table stays bound (last good), so a bad cook cannot blank a frame.
#[derive(Default)]
pub struct LutSlot {
    table: Option<LutTexture>,
    applied: Option<u64>,
}

impl LutSlot {
    /// An empty slot: no table bound, the LUT-free frame.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sync the slot with `table`: `Ok(true)` when it uploaded or dropped,
    /// `Ok(false)` when the bound table already matches. Compares
    /// [`Lut3d::digest`], so the frame loop never re-uploads.
    ///
    /// # Errors
    ///
    /// [`LutUploadError`] from [`LutTexture::new`]; the previous table stays
    /// bound.
    pub fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        table: Option<&Lut3d>,
    ) -> Result<bool, LutUploadError> {
        let wanted = table.map(Lut3d::digest);
        if wanted == self.applied {
            return Ok(false);
        }
        match table {
            Some(found) => {
                let uploaded = LutTexture::new(device, queue, found)?;
                self.table = Some(uploaded);
                self.applied = wanted;
            }
            None => {
                self.table = None;
                self.applied = None;
            }
        }
        Ok(true)
    }

    /// The bound table's view, or `None` when the chain runs LUT-free.
    pub fn view(&self) -> Option<&wgpu::TextureView> {
        self.table.as_ref().map(|held| &held.view)
    }

    /// The bound table's sampler, or `None` when the chain runs LUT-free.
    pub fn sampler(&self) -> Option<&wgpu::Sampler> {
        self.table.as_ref().map(|held| &held.sampler)
    }

    /// Digest of the bound table, or `None` when nothing is bound.
    pub fn applied(&self) -> Option<u64> {
        self.applied
    }
}

/// The look-table display pass (E11): one fullscreen triangle that reads the
/// graded LDR frame, samples the bound table with the linear clamp sampler
/// and writes the display frame.
///
/// A frame with no table bound gets `None` from [`Self::bind`] and records no
/// pass at all, so its bytes stay the blit's own. The pass owns one bind
/// layout, one cached pipeline per target format (built on the first ask, a
/// clone after that) and one cached bind group keyed by the table digest plus
/// the source size, so a steady LUT frame allocates nothing.
pub struct LutPresentPass {
    layout: wgpu::BindGroupLayout,
    pipes: std::sync::Mutex<Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>>,
    bound: std::sync::Mutex<Option<LutBinding>>,
}

/// The cached bind group of one (table, source size) pair.
struct LutBinding {
    digest: u64,
    width: u32,
    height: u32,
    bind: wgpu::BindGroup,
}

impl LutPresentPass {
    /// Build the pass's bind layout: the graded frame, the 3D table and its
    /// filtering sampler. Cold path only.
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("look-present-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        Self {
            layout,
            pipes: std::sync::Mutex::new(Vec::new()),
            bound: std::sync::Mutex::new(None),
        }
    }

    /// Build (or clone) the pipeline for `format`: `fs` for a plain byte
    /// target, `fs_hw_srgb` for a `*Srgb` one. Built on the first ask so the
    /// warm-up owns the only cold build.
    pub fn pipeline_for(
        &self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let mut kept = self
            .pipes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, pipeline)) = kept.iter().find(|(f, _)| *f == format) {
            return pipeline.clone();
        }
        let pipeline = lut_pipeline(device, &self.layout, format);
        kept.push((format, pipeline.clone()));
        pipeline
    }

    /// The bind group for this frame, or `None` when the slot holds no table
    /// (the frame then records no pass). Reused while the table digest and the
    /// source size stay.
    pub fn bind(
        &self,
        device: &wgpu::Device,
        slot: &LutSlot,
        src: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Option<wgpu::BindGroup> {
        let digest = slot.applied()?;
        let (view, sampler) = match (slot.view(), slot.sampler()) {
            (Some(view), Some(sampler)) => (view, sampler),
            _ => return None,
        };
        let mut kept = self
            .bound
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(held) = kept.as_ref() {
            if held.digest == digest && held.width == width && held.height == height {
                return Some(held.bind.clone());
            }
        }
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("look-present-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        *kept = Some(LutBinding {
            digest,
            width,
            height,
            bind: bind.clone(),
        });
        Some(bind)
    }

    /// Record the pass: one fullscreen triangle from `src` (the graded frame)
    /// to `dst` (the display target).
    pub fn encode(
        &self,
        enc: &mut wgpu::CommandEncoder,
        bind: &wgpu::BindGroup,
        pipeline: &wgpu::RenderPipeline,
        dst: &wgpu::TextureView,
    ) {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("look-present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dst,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Every texel is written by the fullscreen triangle; the
                    // clear only keeps a validation-clean load op.
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, bind, &[]);
        rp.draw(0..3, 0..1);
    }
}

/// Compile the look-table pass shader and its pipeline for `format`. `fs`
/// encodes sRGB itself (the LDR twin and any non-sRGB surface); `fs_hw_srgb`
/// leaves the encode to a `*Srgb` target. Cold path: the caller caches it.
pub fn lut_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("look-present"),
        source: wgpu::ShaderSource::Wgsl(super::LUT_PRESENT_WGSL.into()),
    });
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("look-present-layout"),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some("look-present-pipeline"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(if format.is_srgb() { "fs_hw_srgb" } else { "fs" }),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One f16 bit pattern back to f32, for reading the render target.
    fn half_to_f32(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = (bits >> 10) & 0x1f;
        let mantissa = f32::from(bits & 0x03ff);
        if exp == 0 {
            return sign * mantissa * 2f32.powi(-24);
        }
        if exp == 0x1f {
            return sign * f32::INFINITY;
        }
        sign * (1.0 + mantissa / 1024.0) * 2f32.powi(i32::from(exp) - 15)
    }

    /// `.cube` text of `size^3` rows, blue fastest, from a warm map.
    fn warm_cube(size: usize) -> String {
        let last = (size - 1) as f32;
        let mut text = format!("LUT_3D_SIZE {size}\n");
        for r in 0..size {
            for g in 0..size {
                for b in 0..size {
                    let rf = r as f32 / last;
                    let gf = g as f32 / last;
                    let bf = b as f32 / last;
                    text.push_str(&format!(
                        "{} {} {}\n",
                        rf * 0.9 + 0.08,
                        gf * 0.95 + 0.02,
                        bf * 0.8
                    ));
                }
            }
        }
        text
    }

    /// The format gate refuses what a core device cannot filter and takes the
    /// upload format.
    #[test]
    fn lut_upload_filterable_formats() {
        assert!(ensure_filterable(LUT_FORMAT).is_ok(), "Rgba16Float filters");
        let error = ensure_filterable(wgpu::TextureFormat::R32Float)
            .expect_err("32-bit float needs the filterable feature");
        let shown = error.to_string();
        assert!(
            shown.contains("not filterable") && shown.contains("Rgba16Float"),
            "the message names the fix: {shown}"
        );
        assert!(
            ensure_filterable(wgpu::TextureFormat::Rgba8Unorm).is_err(),
            "a byte format is refused"
        );
        assert_eq!(lut_bytes(2), 8 * 8, "a 2x2x2 table uploads 64 bytes");
        assert_eq!(lut_bytes(16), 16 * 16 * 16 * 8, "a 16-cube upload size");
    }

    /// The GPU fetch matches [`Lut3d::sample_texel`] on a warm 16-cube: the
    /// one line that needs a real device. Skips loudly with no adapter.
    #[test]
    fn lut_gpu_matches_cpu_oracle() {
        let Some((device, queue, _info)) = crate::test_gpu::require_or_skip(
            "lut_gpu_matches_cpu_oracle",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let table = crate::post::lut::parse_cube(&warm_cube(16)).expect("warm 16-cube parses");
        assert_eq!(table.size(), 16, "the fixture is a 16 cube");
        let lut = LutTexture::new(device, queue, &table).expect("the table uploads");

        const SIDE: u32 = 16;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lut-oracle-target"),
            size: wgpu::Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lut-oracle"),
            source: wgpu::ShaderSource::Wgsl(
                r#"
@group(0) @binding(0) var lut_tex: texture_3d<f32>;
@group(0) @binding(1) var lut_sampler: sampler;
@group(0) @binding(2) var out_tex: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(4, 4)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= 16u || id.y >= 16u) {
        return;
    }
    // Display color at this pixel: (i + 0.5) / 16 is a texel center.
    let rgb = vec3<f32>(
        (f32(id.x) + 0.5) / 16.0,
        (f32(id.y) + 0.5) / 16.0,
        0.5,
    );
    // Domain 0..=1 onto texel centers: grid = color * (size - 1), uv =
    // (grid + 0.5) / size. The upload is blue-fastest, so x is blue, z red.
    let dims = vec3<f32>(textureDimensions(lut_tex));
    let grid = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * (dims - vec3<f32>(1.0));
    let uv = (grid + vec3<f32>(0.5)) / dims;
    let sampled = textureSampleLevel(lut_tex, lut_sampler, vec3<f32>(uv.b, uv.g, uv.r), 0.0);
    textureStore(out_tex, vec2<i32>(i32(id.x), i32(id.y)), sampled);
}
"#
                .into(),
            ),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lut-oracle-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("lut-oracle-pipeline"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("lut-oracle-pipeline-layout"),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                }),
            ),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lut-oracle-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&lut.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&lut.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&target_view),
                },
            ],
        });

        // 16 x 16 target, 8 bytes per texel: rows pad to the 256-byte copy
        // alignment, so read 256 bytes per row and keep the first 128.
        const ROW: u32 = 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lut-oracle-readback"),
            size: u64::from(ROW) * u64::from(SIDE),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lut-oracle-encoder"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("lut-oracle-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(4, 4, 1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ROW),
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
        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = slice.get_mapped_range().expect("mapped range");

        let half = |offset: usize| -> f32 {
            let low = mapped[offset];
            let high = mapped[offset + 1];
            half_to_f32(u16::from(high) << 8 | u16::from(low))
        };
        let mut worst = 0.0f32;
        for y in 0..SIDE {
            for x in 0..SIDE {
                let base = usize::try_from(y * ROW + x * 8).expect("row offset");
                let gpu = [half(base), half(base + 2), half(base + 4)];
                // Same fetch on the CPU: normalized (x + 0.5, y + 0.5, 0.5)
                // is the same display color the shader built.
                let cpu = table.apply([
                    (f32::from(u16::try_from(x).unwrap_or(0)) + 0.5) / 16.0,
                    (f32::from(u16::try_from(y).unwrap_or(0)) + 0.5) / 16.0,
                    0.5,
                ]);
                for channel in 0..3 {
                    let gap = (gpu[channel] - cpu[channel]).abs();
                    assert!(
                        gap <= 2.0 / 255.0,
                        "texel ({x}, {y}) channel {channel}: GPU {} vs CPU {} (gap {gap})",
                        gpu[channel],
                        cpu[channel]
                    );
                    worst = worst.max(gap);
                }
            }
        }
        eprintln!("lut_gpu_matches_cpu_oracle: 16x16 texels, worst gap {worst:.6}");
    }

    /// One display value through the sRGB encode, matching `fs`'s byte path.
    fn srgb_encode(linear: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for (slot, value) in out.iter_mut().zip(linear.iter()) {
            let v = value.clamp(0.0, 1.0);
            *slot = if v <= 0.003_130_8 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
        }
        out
    }

    /// One byte back to the linear display value the pass starts from.
    fn srgb_decode(bytes: [u8; 3]) -> [f32; 3] {
        let mut out = [0.0f32; 3];
        for (slot, byte) in out.iter_mut().zip(bytes.iter()) {
            let v = f32::from(*byte) / 255.0;
            *slot = if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            };
        }
        out
    }

    /// A 16x16 display ramp on the device, through the look pass, matches
    /// `Lut3d::apply` within 2/255 per channel; clearing the table drops the
    /// pass so the frame keeps the source bytes. Needs a device; skips loudly
    /// with no adapter.
    #[test]
    fn lut_present_pass_matches_the_cpu_and_skips_without_a_table() {
        let Some((device, queue, _info)) = crate::test_gpu::require_or_skip(
            "lut_present_pass_matches_the_cpu_and_skips_without_a_table",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let table = crate::post::lut::parse_cube(&warm_cube(16)).expect("warm 16-cube parses");
        let mut slot = LutSlot::new();
        assert!(
            slot.sync(device, queue, Some(&table)).expect("uploads"),
            "the first sync uploads"
        );
        assert_eq!(
            slot.applied(),
            Some(table.digest()),
            "the slot names the table"
        );

        const SIDE: u32 = 16;
        let extent = wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        };
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("look-present-source"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
        // The graded display frame the blit would write: a red/green ramp at
        // one fixed blue, already sRGB-encoded.
        let mut ramp = Vec::with_capacity((SIDE * SIDE * 4) as usize);
        for y in 0..SIDE {
            for x in 0..SIDE {
                let display = [
                    (f32::from(u16::try_from(x).unwrap_or(0)) + 0.5) / 16.0,
                    (f32::from(u16::try_from(y).unwrap_or(0)) + 0.5) / 16.0,
                    0.25,
                ];
                let bytes = srgb_encode(display);
                for channel in bytes {
                    ramp.push((channel * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
                }
                ramp.push(255);
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &ramp,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIDE * 4),
                rows_per_image: Some(SIDE),
            },
            extent,
        );
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("look-present-target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let pass = LutPresentPass::new(device);
        let bind = pass
            .bind(device, &slot, &source_view, SIDE, SIDE)
            .expect("a bound table gives a bind group");
        let pipeline = pass.pipeline_for(device, wgpu::TextureFormat::Rgba8Unorm);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("look-present-encoder"),
        });
        pass.encode(&mut enc, &bind, &pipeline, &target_view);
        queue.submit(Some(enc.finish()));

        let out = read_rgba8(device, queue, &target, SIDE, SIDE);
        let mut worst = 0.0f32;
        let mut moved = 0usize;
        for y in 0..SIDE {
            for x in 0..SIDE {
                let at = ((y * SIDE + x) * 4) as usize;
                let seen = [out[at], out[at + 1], out[at + 2]];
                let input = [ramp[at], ramp[at + 1], ramp[at + 2]];
                // The CPU statement: decode the byte the pass starts from into
                // linear display values, sample the table, encode back.
                let want = srgb_encode(table.apply(srgb_decode(input)));
                if seen != input {
                    moved = moved.saturating_add(1);
                }
                for channel in 0..3 {
                    let expect = (want[channel] * 255.0 + 0.5).clamp(0.0, 255.0);
                    let gap = (f32::from(seen[channel]) - expect).abs() / 255.0;
                    assert!(
                        gap <= 2.0 / 255.0,
                        "texel ({x}, {y}) channel {channel}: pass {} vs Lut3d::apply {} (gap {gap})",
                        seen[channel],
                        expect
                    );
                    worst = worst.max(gap);
                }
            }
        }
        assert!(
            moved > 0,
            "the warm table must move the ramp, every texel came back unchanged"
        );

        // Clearing the table drops the pass: the frame keeps the source bytes.
        assert!(
            slot.sync(device, queue, None).expect("clears"),
            "clearing the table drops the upload"
        );
        assert_eq!(slot.applied(), None, "no digest is bound");
        assert!(
            pass.bind(device, &slot, &source_view, SIDE, SIDE).is_none(),
            "a frame with no table records no look pass"
        );
        let kept = read_rgba8(device, queue, &source, SIDE, SIDE);
        assert_eq!(
            kept, ramp,
            "the LUT-free frame is byte-identical to the source"
        );
        eprintln!(
            "lut_present_pass_matches_the_cpu_and_skips_without_a_table: 16x16 ramp, {moved} texels moved, worst gap {worst:.6} (2/255 = {:.6})",
            2.0 / 255.0
        );
    }

    /// Copy a 16x16 RGBA8 texture back as rows of four bytes.
    fn read_rgba8(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        const ROW: u32 = 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("look-present-readback"),
            size: u64::from(ROW) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("look-present-copy"),
        });
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
                    bytes_per_row: Some(ROW),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(enc.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = slice.get_mapped_range().expect("mapped range");
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * ROW) as usize;
            out.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        out
    }
}

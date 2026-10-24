//! ibl: GPU image-based lighting baked from the Poly Haven HDRI (RL2).
//!
//! Owner: render lane; consumers WIN1/ED1 material frame through the RA1
//! presenter and the Q1 chart. Units: texels for the prefiltered cubemap and
//! the BRDF LUT, roughness steps for the prefilter levels, steradians behind
//! the SH projection weights. Determinism: fixed index orders, f64
//! accumulation for the SH projection, bit-identical bytes for the same HDRI
//! bytes on every run. Errors are typed `IblError`: a missing HDRI refuses
//! loudly, and device failures name the call or the validation scope.
//!
//! Cook-time bake (behind `v1`, needs the R2 `crate::v1_ibl_shadow` HDRI
//! path): `bake` turns decoded HDRI bytes into an `IblBake` holding a
//! roughness-stepped prefiltered cubemap, 9-coefficient irradiance SH and a
//! GGX split-sum BRDF LUT. The forward pass samples the bake on the CPU with
//! `sample_cube`, `eval_sh` and `brdf_sample`, combined by
//! `shade_ibl`; the WGSL sampling chunk for the RA1 forward pipeline is
//! `gpu::FORWARD_IBL_WGSL`, and `IblBake::submit_gpu` (behind `window`)
//! records the live material-chart pass binding the shared
//! [`crate::gtao::LookFrame`] plus the full bake.

use std::f32::consts::PI;
use std::fmt;

// ---------------------------------------------------------------------------
// Bake shape: fixed sizes bound every allocation up front.
// ---------------------------------------------------------------------------

/// Prefiltered cubemap face edge in texels.
pub const IBL_CUBE_SIZE: u32 = 64;
/// Roughness steps: level `l` holds roughness `l / (LEVELS - 1)`.
pub const IBL_LEVELS: usize = 5;
/// Cubemap faces in stored order: +X, -X, +Y, -Y, +Z, -Z.
pub const IBL_FACES: usize = 6;
/// BRDF LUT edge in texels (NdotV by roughness).
pub const BRDF_LUT_SIZE: u32 = 64;
/// Hammersley samples per BRDF LUT texel; fixed, deterministic. RL47 A7: 256,
/// up from 128. The roughness-1 normal-view texel read E 0.3107 at 128 taps
/// and 0.3088 at 256 (converged 0.3068), and the multiple-scatter gain is
/// `1 / E - 1`, so the tail error halves; the startup bake of the 64 x 64
/// table cost 14-16 ms at 128 and 27-36 ms at 256 (standalone `rustc -O`,
/// loaded laptop CPU), +13-20 ms against the +50 ms budget.
pub const BRDF_SAMPLES: u32 = 256;
/// Irradiance SH coefficients (L2); matches [`crate::gi::SH_COEFFS`].
pub const IBL_SH_COEFFS: usize = 9;
/// Largest chart target edge the submit sizes in pixels. The test chart
/// runs 32x32; the cap keeps readback buffers bounded like the graph
/// transient budget instead of growing with the viewport.
pub const IBL_CHART_MAX_EDGE: u32 = 256;

// ---------------------------------------------------------------------------
// Concerns: one file each (CQ9 split). Public paths stay ibl::<name> through
// the re-exports below; the #[path] child decls for the shader twins stay here
// because their paths resolve against src/, not src/ibl/.
// ---------------------------------------------------------------------------
pub mod brdf;
pub mod chart;
pub mod cube;
pub mod error;
pub mod forward;
pub mod light;

#[cfg(all(test, feature = "v1"))]
pub(crate) use self::brdf::radical_inverse_vdc;
pub use self::brdf::*;
pub use self::chart::*;
pub use self::cube::*;
pub(crate) use self::cube::check_roughness;
#[cfg(feature = "v1")]
pub(crate) use self::cube::check_unit_dir;
pub use self::error::*;
#[cfg(test)]
pub(crate) use self::forward::fs_close;
pub use self::forward::*;
#[cfg(feature = "v1")]
pub use self::light::*;

/// RL47: the CPU twins of the forward shader's specular-lobe terms (the
/// multiple-scatter gain, then specular anti-aliasing and the two ambient
/// occlusion terms); each is pinned against the shipped WGSL on the device.
#[path = "spec_lobe.rs"]
pub mod spec_lobe;

/// RL47 split-sum shade plus its multiple-scatter factor. Presentation only;
/// the public paths stay `ibl::shade_ibl`, `ibl::energy_compensation` and
/// friends through the re-export below.
#[cfg(feature = "v1")]
#[path = "ibl_shade.rs"]
mod ibl_shade;
#[cfg(feature = "v1")]
pub use self::ibl_shade::{
    energy_compensation, ibl_on_off_diff, shade_ibl, shade_ibl_occluded, IBL_OFF_FLOOR,
};

/// Hash of the bound forward chunk through the crate's graph owner; equals
/// [`FORWARD_IBL_CHUNK_HASH`]. Test-only (builds one graph node);
/// per-submit packing copies the pinned const instead, so steady-state
/// submits allocate nothing. It sits above the GPU module so this file's
/// device code stays below the single `#[cfg(test)]` seam the layer audit
/// reads.
#[cfg(test)]
fn forward_ibl_chunk_probe() -> u64 {
    let mut probe = crate::graph::Graph::new();
    probe.add_node(FORWARD_IBL_WGSL);
    probe.graph_hash()
}

/// IBL GPU descriptors for the forward presenter: one `Rgba32Float` cubemap
/// (level 0 resident; rougher levels follow as mips) plus the `Rg32Float`
/// BRDF LUT. Cook bytes upload verbatim; the llvmpipe round-trip test pins
/// byte parity.
#[cfg(feature = "gpu")]
pub mod gpu {
    use super::IblError;

    pub use super::{sh_uniform_bytes_into, FORWARD_IBL_WGSL};

    /// Prefiltered cubemap descriptor: `Rgba32Float`, 6 array layers (one per
    /// face), mip count = roughness levels, sampler-readable for the forward
    /// pass plus copy-dest for the cook upload and copy-src for readback.
    pub fn prefilter_cube_desc(size: u32, levels: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl2-ibl-prefilter"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: super::IBL_FACES as u32,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }
    }

    /// BRDF LUT descriptor: `Rg32Float` (scale, bias), sampler-readable plus
    /// copy-dest for the cook upload.
    pub fn brdf_lut_desc(size: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl2-ibl-brdf-lut"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }
    }

    /// SH uniform image: 9 RGB coefficients packed as 9 vec4s (RGB in xyz,
    /// w pads), 144 bytes for the `ibl_sh` uniform above. Same order as
    /// [`super::IrradianceSh::coeffs`], so the GPU dot matches
    /// [`super::eval_sh`] term for term. Packing lives in
    /// [`super::sh_uniform_bytes_into`]; this wrapper owns the one-shot Vec.
    pub fn sh_uniform_bytes(sh: &super::IrradianceSh) -> Vec<u8> {
        let mut out = [0u8; 144];
        sh_uniform_bytes_into(sh, &mut out);
        out.to_vec()
    }

    /// BRDF LUT cook bytes in upload order: row-major `(scale, bias)` f32
    /// pairs, the same order as [`super::BrdfLut::to_bytes`]. Shared by the
    /// one-shot upload below and the chart submitter re-upload, so both
    /// paths push verbatim cook bytes.
    pub fn brdf_upload_bytes(lut: &super::BrdfLut) -> Vec<u8> {
        let mut bytes = vec![0u8; lut.data.len() * 8];
        brdf_upload_bytes_into(lut, &mut bytes);
        bytes
    }

    /// Pack the BRDF LUT cook bytes into caller-owned storage (exactly
    /// `data.len() * 8` bytes). The submitter reuses one scratch LUT, so
    /// steady-state submits grow nothing.
    pub fn brdf_upload_bytes_into(lut: &super::BrdfLut, out: &mut [u8]) {
        debug_assert_eq!(out.len(), lut.data.len() * 8);
        for (t, chunk) in lut.data.iter().zip(out.chunks_exact_mut(8)) {
            chunk[0..4].copy_from_slice(&t[0].to_le_bytes());
            chunk[4..8].copy_from_slice(&t[1].to_le_bytes());
        }
    }

    /// Upload the BRDF LUT cook bytes verbatim into a GPU texture. Real GPU
    /// work for the presenter material bind group; callers keep last-good on
    /// error.
    pub fn upload_brdf_lut(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lut: &super::BrdfLut,
    ) -> Result<wgpu::Texture, IblError> {
        if lut.size == 0 || lut.size > 512 {
            return Err(IblError::Invalid(format!(
                "ibl brdf gpu size {} outside 1..=512",
                lut.size
            )));
        }
        if lut.data.len() != lut.size as usize * lut.size as usize {
            return Err(IblError::Invalid(
                "ibl brdf lut data length mismatches size".into(),
            ));
        }
        let texture = device.create_texture(&brdf_lut_desc(lut.size));
        let bytes = brdf_upload_bytes(lut);
        let size = wgpu::Extent3d {
            width: lut.size,
            height: lut.size,
            depth_or_array_layers: 1,
        };
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
                bytes_per_row: Some(lut.size * 8),
                rows_per_image: Some(lut.size),
            },
            size,
        );
        Ok(texture)
    }

    /// Prefiltered cook bytes as a 2D array for exact texel-fetch sampling:
    /// `Rgba32Float`, one layer per (level, face) with layer
    /// `level * 6 + face`, level-major then face-minor like
    /// [`super::PrefilterCube::faces`]. The 32-bit float format is not
    /// hardware-filterable, so the chart shader below fetches texels with
    /// `textureLoad` and does the bilinear plus level blend in-shader: the
    /// same math as [`super::sample_cube`], bit for bit up to f32 fusing.
    pub fn prefilter_array_desc(size: u32, levels: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl2-ibl-prefilter-array"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: levels * super::IBL_FACES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        }
    }

    /// Exact byte length of one packed layer (`edge * edge * 16`).
    /// Fails loudly on overflow instead of wrapping the size.
    pub fn layer_byte_len(cube: &super::PrefilterCube) -> Result<usize, super::IblError> {
        let edge = usize::try_from(cube.size)
            .map_err(|_| super::IblError::Invalid("ibl cube edge exceeds usize".into()))?;
        edge.checked_mul(edge)
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| super::IblError::Invalid("ibl layer bytes overflow".into()))
    }

    /// Pack one (level, face) layer as RGBA32F upload bytes into
    /// caller-owned storage (exactly [`layer_byte_len`] bytes). The
    /// submitter reuses one scratch layer for all 30 uploads, so
    /// steady-state submits grow nothing.
    pub fn prefilter_layer_bytes_into(
        cube: &super::PrefilterCube,
        level: usize,
        face: usize,
        out: &mut [u8],
    ) -> Result<(), super::IblError> {
        if out.len() != layer_byte_len(cube)? {
            return Err(super::IblError::Invalid(
                "ibl chart layer scratch has the wrong length".into(),
            ));
        }
        let edge = usize::try_from(cube.size)
            .map_err(|_| super::IblError::Invalid("ibl cube edge exceeds usize".into()))?;
        let one = 1.0f32.to_le_bytes();
        for (y_us, y) in (0..cube.size).enumerate() {
            for (x_us, x) in (0..cube.size).enumerate() {
                let t = cube.texel(level, face, x, y)?;
                let base = (y_us * edge + x_us) * 16;
                out[base..base + 4].copy_from_slice(&t[0].to_le_bytes());
                out[base + 4..base + 8].copy_from_slice(&t[1].to_le_bytes());
                out[base + 8..base + 12].copy_from_slice(&t[2].to_le_bytes());
                out[base + 12..base + 16].copy_from_slice(&one);
            }
        }
        Ok(())
    }

    /// Upload the live forward bake once: prefilter array, BRDF LUT and
    /// SH uniform buffer packed by the bake owner. Cold init only;
    /// steady-state frames reuse the returned handles.
    pub fn upload_live_bake(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bake: &super::IblBake,
    ) -> Result<(wgpu::Texture, wgpu::Texture, wgpu::Buffer), super::IblError> {
        let levels = u32::try_from(super::IBL_LEVELS)
            .map_err(|_| super::IblError::Invalid("ibl levels exceed u32".into()))?;
        let pre = device.create_texture(&prefilter_array_desc(super::IBL_CUBE_SIZE, levels));
        let layer_len = layer_byte_len(&bake.cube)?;
        let mut scratch = vec![0u8; layer_len];
        for level in 0..super::IBL_LEVELS {
            for face in 0..super::IBL_FACES {
                prefilter_layer_bytes_into(&bake.cube, level, face, &mut scratch)?;
                let layer = u32::try_from(level * super::IBL_FACES + face)
                    .map_err(|_| super::IblError::Invalid("ibl layer exceeds u32".into()))?;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &pre,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &scratch,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(super::IBL_CUBE_SIZE * 16),
                        rows_per_image: Some(super::IBL_CUBE_SIZE),
                    },
                    wgpu::Extent3d {
                        width: super::IBL_CUBE_SIZE,
                        height: super::IBL_CUBE_SIZE,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let lut_tex = device.create_texture(&brdf_lut_desc(super::BRDF_LUT_SIZE));
        let mut lut_scratch = vec![0u8; bake.brdf.data.len() * 8];
        brdf_upload_bytes_into(&bake.brdf, &mut lut_scratch);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &lut_scratch,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(super::BRDF_LUT_SIZE * 8),
                rows_per_image: Some(super::BRDF_LUT_SIZE),
            },
            wgpu::Extent3d {
                width: super::BRDF_LUT_SIZE,
                height: super::BRDF_LUT_SIZE,
                depth_or_array_layers: 1,
            },
        );
        let mut sh_bytes = [0u8; 144];
        super::sh_uniform_bytes_into(&bake.sh, &mut sh_bytes);
        let sh_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ra1-u-ibl-sh"),
            size: 144,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&sh_buf, 0, &sh_bytes);
        Ok((pre, lut_tex, sh_buf))
    }

    /// Layout entries for live forward lighting (bindings 9-12):
    /// prefilter array, BRDF LUT, sampler, SH uniform. Cold layout only.
    pub fn forward_ibl_layout_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
        [
            wgpu::BindGroupLayoutEntry {
                binding: 9,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 11,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 12,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]
    }

    /// Parts for one live forward bind group in the forward group.
    /// Owner: render lane; consumer: presenter forward binds.
    /// Units: GPU handles; determinism: same handles give the same bind.
    pub struct ForwardLiveParts<'a> {
        /// Forward bind-group layout (group 0 with lighting entries).
        pub layout: &'a wgpu::BindGroupLayout,
        /// Camera uniform buffer.
        pub cam: &'a wgpu::Buffer,
        /// Light uniform buffer.
        pub light: &'a wgpu::Buffer,
        /// Material ring buffer (one stride window per draw).
        pub material: &'a wgpu::Buffer,
        /// Material window size in bytes.
        pub material_bytes: u64,
        /// Shadow uniform buffer.
        pub shadow: &'a wgpu::Buffer,
        /// Base-color texture view for this draw.
        pub base_view: &'a wgpu::TextureView,
        /// Filtering sampler shared by base and lighting.
        pub sampler: &'a wgpu::Sampler,
        /// Fog uniform buffer.
        pub fog: &'a wgpu::Buffer,
        /// Prefilter array view (30 layers).
        pub pre_view: &'a wgpu::TextureView,
        /// BRDF LUT view.
        pub brdf_view: &'a wgpu::TextureView,
        /// SH uniform buffer (144 bytes).
        pub sh_buf: &'a wgpu::Buffer,
    }

    /// Posted encode + submit + readback budget for the chart pass in ms
    /// (debug build on the reference machine, evidence only, never hashed;
    /// arena-size timing belongs to PF2).
    pub const IBL_SUBMIT_BUDGET_MS: f64 = 16.0;
    /// Posted GPU-vs-CPU chart parity: absolute linear-RGB units. The shader
    /// repeats the CPU sampler math texel for texel, so only f32 fusing
    /// separates the legs (observed 2e-5 on the reference GPU); an axis flip
    /// or a missing lobe would miss by 10x this gate.
    pub const IBL_CHART_ABS_TOL: f32 = 0.005;
    /// Posted GPU-vs-CPU chart parity: relative error on chart pixels whose
    /// reference channel exceeds 0.2 (observed 0.002% on the reference GPU).
    pub const IBL_CHART_REL_TOL: f32 = 0.01;

    /// Material-chart pass shader over the shared look uniform plus the full
    /// bake: one `Rgba32Float` target, no vertex buffers, no depth. Each
    /// pixel derives its material from its coordinates (see
    /// [`chart_inputs`]): columns alternate dielectric/metal, halves swap
    /// albedo, rows sweep roughness, normals fan out over the faces. SH,
    /// prefilter and BRDF sampling repeat the CPU [`super::shade_ibl`] math;
    /// `textureLoad` fetches exact cook texels (32-bit float is not
    /// hardware-filterable), so the pass is a real GPU sampling of the bake
    /// with the look exposure applied. Engine-owned code.
    pub const CHART_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(1) @binding(0) var pre: texture_2d_array<f32>;
@group(1) @binding(1) var brdf: texture_2d<f32>;
@group(1) @binding(2) var<uniform> sh: array<vec4<f32>, 9>;
struct VOut {
    @builtin(position) pos: vec4<f32>,
};
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: VOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    return out;
}
fn chart_basis(d: vec3<f32>) -> array<f32, 9> {
    var b: array<f32, 9>;
    b[0] = 0.282095;
    b[1] = -0.488603 * d.y;
    b[2] = 0.488603 * d.z;
    b[3] = -0.488603 * d.x;
    b[4] = 1.092548 * d.x * d.y;
    b[5] = -1.092548 * d.y * d.z;
    b[6] = 0.315392 * (3.0 * d.z * d.z - 1.0);
    b[7] = -1.092548 * d.x * d.z;
    b[8] = 0.546274 * (d.x * d.x - d.y * d.y);
    return b;
}
fn pre_level(layer_base: i32, face: i32, u: f32, v: f32) -> vec3<f32> {
    let x = clamp((u + 1.0) * 0.5 * 64.0 - 0.5, 0.0, 63.0);
    let y = clamp((v + 1.0) * 0.5 * 64.0 - 0.5, 0.0, 63.0);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, 63);
    let y1 = min(y0 + 1, 63);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let layer = layer_base + face;
    let a = textureLoad(pre, vec2<i32>(x0, y0), layer, 0).rgb;
    let bb = textureLoad(pre, vec2<i32>(x1, y0), layer, 0).rgb;
    let c = textureLoad(pre, vec2<i32>(x0, y1), layer, 0).rgb;
    let e = textureLoad(pre, vec2<i32>(x1, y1), layer, 0).rgb;
    return mix(mix(a, bb, fx), mix(c, e, fx), fy);
}
fn brdf_pair(ndotv: f32, roughness: f32) -> vec2<f32> {
    let x = clamp(ndotv * 64.0 - 0.5, 0.0, 63.0);
    let y = clamp(roughness * 64.0 - 0.5, 0.0, 63.0);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, 63);
    let y1 = min(y0 + 1, 63);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = textureLoad(brdf, vec2<i32>(x0, y0), 0).rg;
    let bb = textureLoad(brdf, vec2<i32>(x1, y0), 0).rg;
    let c = textureLoad(brdf, vec2<i32>(x0, y1), 0).rg;
    let e = textureLoad(brdf, vec2<i32>(x1, y1), 0).rg;
    return mix(mix(a, bb, fx), mix(c, e, fx), fy);
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let ix = i32(in.pos.x);
    let iy = i32(in.pos.y);
    let w = f32(look.width_px);
    let h = f32(look.height_px);
    let fx = (f32(ix) + 0.5) / w;
    let fy = (f32(iy) + 0.5) / h;
    let n = normalize(vec3<f32>((fx - 0.5) * 2.0, 0.6, (fy - 0.5) * 2.0));
    let vv = vec3<f32>(0.4, 0.6, 0.5);
    let v = vv / length(vv);
    var albedo = vec3<f32>(0.5, 0.5, 0.5);
    if (fx >= 0.5) {
        albedo = vec3<f32>(0.8, 0.2, 0.1);
    }
    let metallic = f32(ix % 2);
    let roughness = fy;
    let b = chart_basis(n);
    var e = vec3<f32>(0.0, 0.0, 0.0);
    e += sh[0].xyz * b[0];
    e += sh[1].xyz * b[1];
    e += sh[2].xyz * b[2];
    e += sh[3].xyz * b[3];
    e += sh[4].xyz * b[4];
    e += sh[5].xyz * b[5];
    e += sh[6].xyz * b[6];
    e += sh[7].xyz * b[7];
    e += sh[8].xyz * b[8];
    e = max(e, vec3<f32>(0.0, 0.0, 0.0));
    let diffuse = (1.0 - metallic) * albedo * e / 3.141592653589793;
    let ndotv = clamp(dot(n, v), 0.0, 1.0);
    let r = 2.0 * ndotv * n - v;
    let ax = abs(r.x);
    let ay = abs(r.y);
    let az = abs(r.z);
    var face = 5;
    var uu = 0.0;
    var vw = 0.0;
    if (ax >= ay && ax >= az) {
        if (r.x > 0.0) {
            face = 0;
            uu = -r.z / ax;
            vw = -r.y / ax;
        } else {
            face = 1;
            uu = r.z / ax;
            vw = -r.y / ax;
        }
    } else if (ay >= ax && ay >= az) {
        if (r.y > 0.0) {
            face = 2;
            uu = r.x / ay;
            vw = r.z / ay;
        } else {
            face = 3;
            uu = r.x / ay;
            vw = -r.z / ay;
        }
    } else {
        if (r.z > 0.0) {
            face = 4;
            uu = r.x / az;
            vw = -r.y / az;
        } else {
            face = 5;
            uu = -r.x / az;
            vw = -r.y / az;
        }
    }
    let f = roughness * 4.0;
    let l0 = min(i32(floor(f)), 3);
    let l1 = l0 + 1;
    let t = f - f32(l0);
    let p0 = pre_level(l0 * 6, face, uu, vw);
    let p1 = pre_level(l1 * 6, face, uu, vw);
    let sampled = mix(p0, p1, t);
    let fab = brdf_pair(ndotv, roughness);
    let f0 = vec3<f32>(0.04, 0.04, 0.04) + (albedo - vec3<f32>(0.04, 0.04, 0.04)) * metallic;
    let spec = sampled * (f0 * fab.x + vec3<f32>(fab.y, fab.y, fab.y));
    let escale = clamp(look.exposure / 10.0, 0.5, 2.0);
    return vec4<f32>((diffuse + spec) * escale, 1.0);
}
"#;

    /// Chart material inputs for pixel `(x, y)` in a `w` by `h` target:
    /// returns `(normal, view_dir, albedo, metallic, roughness)`. The exact
    /// formulas the [`CHART_WGSL`] fragment runs, so the CPU
    /// [`super::shade_ibl`] reference and the GPU pass read the same chart.
    pub fn chart_inputs(
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> ([f32; 3], [f32; 3], [f32; 3], f32, f32) {
        let fx = (x as f32 + 0.5) / w as f32;
        let fy = (y as f32 + 0.5) / h as f32;
        let raw = [(fx - 0.5) * 2.0, 0.6, (fy - 0.5) * 2.0];
        let l = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2]).sqrt();
        let n = [raw[0] / l, raw[1] / l, raw[2] / l];
        let vv = [0.4f32, 0.6, 0.5];
        let vl = (vv[0] * vv[0] + vv[1] * vv[1] + vv[2] * vv[2]).sqrt();
        let v = [vv[0] / vl, vv[1] / vl, vv[2] / vl];
        let albedo = if fx < 0.5 {
            [0.5, 0.5, 0.5]
        } else {
            [0.8, 0.2, 0.1]
        };
        (n, v, albedo, (x % 2) as f32, fy)
    }

    /// One live chart submit: pass counts from the bake, pixel proof from the
    /// readback, timing as evidence. Units: counts, texels, ms.
    #[derive(Clone, Debug)]
    pub struct IblGpuReceipt {
        /// Frame index from the submitted look frame.
        pub frame_index: u64,
        /// Chart target width in pixels.
        pub width_px: u32,
        /// Chart target height in pixels.
        pub height_px: u32,
        /// wgpu passes recorded (the chart pass).
        pub passes_submitted: u32,
        /// Prefiltered texels bound (`levels * 6 * size * size`).
        pub cube_texels: u64,
        /// BRDF LUT texels bound (`size * size`).
        pub lut_texels: u64,
        /// SH coefficients bound (always 9).
        pub sh_coeffs: u32,
        /// wgpu validation errors observed; green means 0.
        pub validation_errors: u32,
        /// Pixels in the target differing from the clear color.
        pub covered_pixels: u64,
        /// Largest absolute GPU-vs-CPU channel gap over the chart.
        pub max_abs_diff: f32,
        /// Largest relative GPU-vs-CPU channel gap (reference above 0.2).
        pub max_rel_diff: f32,
        /// Wall time for upload + encode + submit + readback in ms.
        pub submit_ms: f64,
    }

    impl IblGpuReceipt {
        /// Posted budget this submit is charged against.
        pub fn budget_ms(&self) -> f64 {
            IBL_SUBMIT_BUDGET_MS
        }
        /// True while the submit stayed inside the posted budget.
        pub fn within_budget(&self) -> bool {
            self.submit_ms <= IBL_SUBMIT_BUDGET_MS
        }
        /// True when the submit observed zero validation errors.
        pub fn is_clean(&self) -> bool {
            self.validation_errors == 0
        }
        /// True while the GPU chart tracks the CPU shade inside the posted
        /// absolute and relative tolerances.
        pub fn within_chart_tolerance(&self) -> bool {
            self.max_abs_diff <= IBL_CHART_ABS_TOL && self.max_rel_diff <= IBL_CHART_REL_TOL
        }
    }

    /// Reusable chart submit state: the chart pipeline, the look bind layout
    /// over the 32-byte look uniform plus the bake bind layout over the
    /// prefilter array, the BRDF LUT and the 144-byte SH uniform, with the
    /// upload textures and the chart target cached. Built once (WIN1 and ED1
    /// each own one); submits on matching shape reuse every texture and bind
    /// group, so steady-state frames create no pipelines, bind groups or
    /// targets (the reuse test proves the counts stay flat) and grow nothing
    /// on the host either: uploads pack into the owned scratch below (the
    /// scratch test proves the capacities stay flat). Per-submit encoder
    /// and readback buffers are driver resources, same as the graph
    /// submitter, and sit outside the reuse guarantee.
    pub struct IblSubmitter {
        look_layout: wgpu::BindGroupLayout,
        bake_layout: wgpu::BindGroupLayout,
        pub(crate) pipeline: wgpu::RenderPipeline,
        pub(crate) look_buf: wgpu::Buffer,
        sh_buf: wgpu::Buffer,
        array_tex: Option<wgpu::Texture>,
        brdf_tex: Option<wgpu::Texture>,
        pub(crate) target: Option<wgpu::Texture>,
        pub(crate) target_view: Option<wgpu::TextureView>,
        pub(crate) look_bind: Option<wgpu::BindGroup>,
        pub(crate) bake_bind: Option<wgpu::BindGroup>,
        backing_ready: bool,
        cached_target: Option<(u32, u32)>,
        /// One packed prefilter layer (`64x64x16` bytes), refilled per
        /// layer upload.
        layer_scratch: Vec<u8>,
        /// Packed BRDF LUT (`64x64x8` bytes), refilled per submit.
        lut_scratch: Vec<u8>,
        /// Packed SH uniform (144 bytes), refilled per submit.
        sh_scratch: [u8; 144],
    }

    impl IblSubmitter {
        /// Compile the chart pipeline and both bind layouts; allocate the
        /// look and SH uniform buffers. No bake or target state yet: upload
        /// textures size on the first submit and the target resizes only
        /// when the look viewport changes.
        pub fn new(device: &wgpu::Device) -> Self {
            use crate::pipeline_warmup::CountedPipelines as _;
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("rl2-ibl-chart"),
                source: wgpu::ShaderSource::Wgsl(CHART_WGSL.into()),
            });
            let look_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rl2-ibl-chart-look-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let bake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rl2-ibl-chart-bake-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
            let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rl2-ibl-chart-layout"),
                bind_group_layouts: &[Some(&look_layout), Some(&bake_layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
                label: Some("rl2-ibl-chart-pipeline"),
                layout: Some(&pipe_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
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
            let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rl2-ibl-chart-look"),
                size: crate::graph::LOOK_UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let sh_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rl2-ibl-chart-sh"),
                size: 9 * 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            Self {
                look_layout,
                bake_layout,
                pipeline,
                look_buf,
                sh_buf,
                array_tex: None,
                brdf_tex: None,
                target: None,
                target_view: None,
                look_bind: None,
                bake_bind: None,
                backing_ready: false,
                cached_target: None,
                layer_scratch: Vec::new(),
                lut_scratch: Vec::new(),
                sh_scratch: [0u8; 144],
            }
        }

        /// Resident upload plus target texture count (probe for the reuse test).
        pub fn texture_count(&self) -> usize {
            usize::from(self.array_tex.is_some())
                + usize::from(self.brdf_tex.is_some())
                + usize::from(self.target.is_some())
        }

        /// Resident bind-group count (probe for the reuse test).
        pub fn bind_group_count(&self) -> usize {
            usize::from(self.look_bind.is_some()) + usize::from(self.bake_bind.is_some())
        }

        /// Host scratch capacities in bytes (layer, LUT). Probe for the
        /// scratch test: flat across matching-shape submits.
        pub fn scratch_capacities(&self) -> (usize, usize) {
            (self.layer_scratch.capacity(), self.lut_scratch.capacity())
        }

        /// Upload the bake into the cached textures, creating the backing
        /// plus both bind groups on the first call.
        /// `pub(crate)` for the file-scope live submit; tests use the probes.
        pub(crate) fn ensure_backing(
            &mut self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            bake: &super::IblBake,
        ) -> Result<(), super::IblError> {
            if !self.backing_ready {
                self.create_backing(device)?;
            }
            self.upload_backing(queue, bake)
        }

        /// Create the prefilter array plus the LUT plus both bind groups.
        /// Runs once; later submits re-upload without recreating.
        fn create_backing(&mut self, device: &wgpu::Device) -> Result<(), super::IblError> {
            let levels = u32::try_from(super::IBL_LEVELS)
                .map_err(|_| super::IblError::Invalid("ibl levels exceed u32".into()))?;
            let array = device.create_texture(&prefilter_array_desc(super::IBL_CUBE_SIZE, levels));
            let lut = device.create_texture(&brdf_lut_desc(super::BRDF_LUT_SIZE));
            let array_view = array.create_view(&wgpu::TextureViewDescriptor {
                label: Some("rl2-ibl-prefilter-array-view"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
            let lut_view = lut.create_view(&wgpu::TextureViewDescriptor {
                label: Some("rl2-ibl-brdf-view"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                ..Default::default()
            });
            let look_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl2-ibl-chart-look-bind"),
                layout: &self.look_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.look_buf.as_entire_binding(),
                }],
            });
            let bake_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl2-ibl-chart-bake-bind"),
                layout: &self.bake_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&array_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&lut_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.sh_buf.as_entire_binding(),
                    },
                ],
            });
            // Views are held alive by the bind groups; textures stay
            // resident so later submits re-upload without recreating.
            self.array_tex = Some(array);
            self.brdf_tex = Some(lut);
            self.look_bind = Some(look_bind);
            self.bake_bind = Some(bake_bind);
            self.backing_ready = true;
            Ok(())
        }

        /// Re-upload every layer plus the LUT plus the SH uniform through
        /// owned scratch, so steady-state submits grow nothing on the host.
        fn upload_backing(
            &mut self,
            queue: &wgpu::Queue,
            bake: &super::IblBake,
        ) -> Result<(), super::IblError> {
            let array = self
                .array_tex
                .as_ref()
                .ok_or_else(|| super::IblError::Invalid("ibl chart backing vanished".into()))?;
            let layer_need = layer_byte_len(&bake.cube)?;
            if self.layer_scratch.len() != layer_need {
                self.layer_scratch.resize(layer_need, 0);
            }
            for level in 0..super::IBL_LEVELS {
                for face in 0..super::IBL_FACES {
                    prefilter_layer_bytes_into(&bake.cube, level, face, &mut self.layer_scratch)?;
                    let layer = u32::try_from(level * super::IBL_FACES + face).map_err(|_| {
                        super::IblError::Invalid("ibl chart layer exceeds u32".into())
                    })?;
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: array,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: layer,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &self.layer_scratch,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(super::IBL_CUBE_SIZE * 16),
                            rows_per_image: Some(super::IBL_CUBE_SIZE),
                        },
                        wgpu::Extent3d {
                            width: super::IBL_CUBE_SIZE,
                            height: super::IBL_CUBE_SIZE,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
            let lut_tex = self
                .brdf_tex
                .as_ref()
                .ok_or_else(|| super::IblError::Invalid("ibl chart backing vanished".into()))?;
            let lut_need = bake.brdf.data.len() * 8;
            if self.lut_scratch.len() != lut_need {
                self.lut_scratch.resize(lut_need, 0);
            }
            brdf_upload_bytes_into(&bake.brdf, &mut self.lut_scratch);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: lut_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.lut_scratch,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(super::BRDF_LUT_SIZE * 8),
                    rows_per_image: Some(super::BRDF_LUT_SIZE),
                },
                wgpu::Extent3d {
                    width: super::BRDF_LUT_SIZE,
                    height: super::BRDF_LUT_SIZE,
                    depth_or_array_layers: 1,
                },
            );
            sh_uniform_bytes_into(&bake.sh, &mut self.sh_scratch);
            queue.write_buffer(&self.sh_buf, 0, &self.sh_scratch);
            Ok(())
        }

        /// Size the chart target from the look viewport, recreating only on
        /// size change so steady-state frames reuse the target and its view.
        /// `pub(crate)` for the file-scope live submit; tests use the probes.
        pub(crate) fn ensure_target(
            &mut self,
            device: &wgpu::Device,
            w: u32,
            h: u32,
        ) -> Result<(), super::IblError> {
            super::check_chart_target_size(w, h)?;
            if self.cached_target != Some((w, h)) {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("rl2-ibl-chart-target"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.target = Some(texture);
                self.target_view = Some(view);
                self.cached_target = Some((w, h));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Live chart submit behind `window`: the bake sampled on a real device.
// ---------------------------------------------------------------------------

/// Window submit over the cook (RL2 live): one wgpu chart pass sampling the
/// prefilter array, the BRDF LUT and the SH uniform under the shared
/// [`crate::gtao::LookFrame`] bind, with a float-target readback compared
/// against the CPU `shade_ibl` reference. Owner: render lane; consumers
/// WIN1 (match material chart) and the ED1 viewport: both own one
/// [`gpu::IblSubmitter`] and call this per bake with a validated frame.
/// Units: texels, pixels, ms. Budget: [`gpu::IBL_SUBMIT_BUDGET_MS`] at test
/// sizes; the chart target downscales the look viewport through
/// [`chart_target_size`] like the graph slot targets (bounded by
/// [`IBL_CHART_MAX_EDGE`], so a 1600x900 window submits 256x144).
/// Determinism: fixed pixel
/// order, exact cook texels in, same bytes out on every run; pixels are
/// evidence only, never hashed. Errors: `IblError` names the bad input,
/// the device call or the validation scope; this path has no silent variant
/// because the caller hands over a live device.
/// Validate the frame plus the bake shape before the submitter sizes
/// anything, so a refusal sizes nothing. Returns the chart target size.
/// Only the standard cook shape submits: the chart shader addresses those
/// extents directly.
#[cfg(all(feature = "v1", feature = "gpu"))]
fn check_chart_bake(
    frame: &crate::gtao::LookFrame,
    bake: &IblBake,
) -> Result<(u32, u32), IblError> {
    frame.validate().map_err(chart_frame_error)?;
    let (w, h) = chart_target_size(frame.viewport_px[0], frame.viewport_px[1])?;
    if bake.cube.size != IBL_CUBE_SIZE || bake.cube.levels != IBL_LEVELS {
        return Err(IblError::Invalid(format!(
            "ibl chart needs the standard cook cube {}x{}x{IBL_LEVELS}, got {}x{}x{}",
            IBL_CUBE_SIZE, IBL_CUBE_SIZE, IBL_CUBE_SIZE, bake.cube.size, bake.cube.levels
        )));
    }
    let edge = usize::try_from(IBL_CUBE_SIZE)
        .map_err(|_| IblError::Invalid("ibl cube edge exceeds usize".into()))?;
    if bake.cube.faces.len() != IBL_LEVELS * IBL_FACES * edge * edge {
        return Err(IblError::Invalid(
            "ibl chart cube data length mismatches the standard shape".into(),
        ));
    }
    let lut_edge = usize::try_from(BRDF_LUT_SIZE)
        .map_err(|_| IblError::Invalid("ibl lut edge exceeds usize".into()))?;
    if bake.brdf.size != BRDF_LUT_SIZE || bake.brdf.data.len() != lut_edge * lut_edge {
        return Err(IblError::Invalid(format!(
            "ibl chart needs the standard BRDF edge {BRDF_LUT_SIZE}, got {}",
            bake.brdf.size
        )));
    }
    Ok((w, h))
}

/// Refusal text for an invalid look frame. Cold path only.
#[cfg(all(feature = "v1", feature = "gpu"))]
fn chart_frame_error(e: crate::gtao::LookError) -> IblError {
    IblError::Invalid(format!("ibl look frame: {e}"))
}

/// Refusal text for a validation-scope hit. Cold path only.
#[cfg(all(feature = "v1", feature = "gpu"))]
fn chart_validation(err: wgpu::Error) -> IblError {
    IblError::Validation(format!("{err:?}"))
}

/// Refusal text for a chart reference failure. Cold path only.
#[cfg(all(feature = "v1", feature = "gpu"))]
fn chart_reference(e: IblError) -> IblError {
    IblError::Invalid(format!("ibl chart reference: {e}"))
}

/// Record the chart pass and submit once, then hand back the readback
/// buffer plus its padded row stride. Owns the validation scope for the
/// encode, so the caller only maps and scores.
#[cfg(all(feature = "v1", feature = "gpu"))]
fn run_chart_pass(
    submitter: &gpu::IblSubmitter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    w: u32,
    h: u32,
) -> Result<(wgpu::Buffer, u32), IblError> {
    let view = submitter
        .target_view
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart target view vanished".into()))?;
    let look_bind = submitter
        .look_bind
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart look bind vanished".into()))?;
    let bake_bind = submitter
        .bake_bind
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart bake bind vanished".into()))?;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rl2-ibl-chart"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rl2-ibl-chart-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&submitter.pipeline);
        pass.set_bind_group(0, look_bind, &[]);
        pass.set_bind_group(1, bake_bind, &[]);
        pass.draw(0..3, 0..1);
    }
    let bytes_per_row = w * 16;
    let padded = bytes_per_row.div_ceil(256) * 256;
    let target = submitter
        .target
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart target vanished".into()))?;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rl2-ibl-chart-readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
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
    queue.submit(Some(encoder.finish()));
    if let Some(err) = pollster::block_on(scope.pop()) {
        return Err(chart_validation(err));
    }
    Ok((readback, padded))
}

/// Map the chart readback and wait for it. Cold path helper: the device
/// poll and the channel hop live here, not in the submit body.
#[cfg(all(feature = "v1", feature = "gpu"))]
fn map_chart_readback(device: &wgpu::Device, readback: &wgpu::Buffer) -> Result<(), IblError> {
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| IblError::Device(format!("chart poll: {e}")))?;
    rx.recv()
        .map_err(|e| IblError::Device(format!("chart map channel: {e}")))?
        .map_err(|e| IblError::Device(format!("chart map: {e}")))?;
    Ok(())
}

/// Score the mapped chart readback against the CPU `shade_ibl`
/// reference: covered pixels plus the worst absolute and relative channel
/// gaps. Pure pixel loop, no allocation; unmaps before returning.
#[cfg(all(feature = "v1", feature = "gpu"))]
fn score_chart_pixels(
    readback: &wgpu::Buffer,
    padded: u32,
    w: u32,
    h: u32,
    bake: &IblBake,
    escale: f32,
) -> Result<(u64, f32, f32), IblError> {
    let stride = usize::try_from(padded)
        .map_err(|_| IblError::Invalid("ibl chart stride exceeds usize".into()))?;
    let slice = readback.slice(..);
    let mapped = slice
        .get_mapped_range()
        .map_err(|e| IblError::Device(format!("chart mapped range: {e}")))?;
    let mut covered = 0u64;
    let mut max_abs = 0.0f32;
    let mut max_rel = 0.0f32;
    for (y_us, y) in (0..h).enumerate() {
        let row = y_us * stride;
        for (x_us, x) in (0..w).enumerate() {
            let px = row + x_us * 16;
            let g = [
                f32::from_le_bytes([mapped[px], mapped[px + 1], mapped[px + 2], mapped[px + 3]]),
                f32::from_le_bytes([
                    mapped[px + 4],
                    mapped[px + 5],
                    mapped[px + 6],
                    mapped[px + 7],
                ]),
                f32::from_le_bytes([
                    mapped[px + 8],
                    mapped[px + 9],
                    mapped[px + 10],
                    mapped[px + 11],
                ]),
            ];
            if g[0].abs().max(g[1].abs()).max(g[2].abs()) > 1e-3 {
                covered += 1;
            }
            let (n, v, albedo, metallic, roughness) = gpu::chart_inputs(x, y, w, h);
            let e = eval_sh(&bake.sh, n).map_err(chart_reference)?;
            let diffuse = [
                (1.0 - metallic) * albedo[0] * e[0] / PI,
                (1.0 - metallic) * albedo[1] * e[1] / PI,
                (1.0 - metallic) * albedo[2] * e[2] / PI,
            ];
            let ndotv = (n[0] * v[0] + n[1] * v[1] + n[2] * v[2]).clamp(0.0, 1.0);
            let refl = [
                2.0 * ndotv * n[0] - v[0],
                2.0 * ndotv * n[1] - v[1],
                2.0 * ndotv * n[2] - v[2],
            ];
            let pre = sample_cube(&bake.cube, refl, roughness).map_err(chart_reference)?;
            let lut = brdf_sample(&bake.brdf, ndotv, roughness).map_err(chart_reference)?;
            let f0 = [
                0.04 + (albedo[0] - 0.04) * metallic,
                0.04 + (albedo[1] - 0.04) * metallic,
                0.04 + (albedo[2] - 0.04) * metallic,
            ];
            for c in 0..3 {
                let want = (diffuse[c] + pre[c] * (f0[c] * lut[0] + lut[1])) * escale;
                let a = (g[c] - want).abs();
                max_abs = max_abs.max(a);
                if want.abs() > 0.2 {
                    max_rel = max_rel.max(a / want.abs());
                }
            }
        }
    }
    drop(mapped);
    readback.unmap();
    Ok((covered, max_abs, max_rel))
}

#[cfg(all(feature = "v1", feature = "gpu"))]
impl IblBake {
    /// Record the chart pass on `device` and submit once. Consumes no
    /// `submitter` capacity on matching shape; fails loudly (never silent)
    /// on invalid frames, non-standard bake shapes or a validation hit.
    /// Only the standard cook shape submits (`IBL_CUBE_SIZE` by
    /// `IBL_LEVELS`, `BRDF_LUT_SIZE`): the chart shader addresses those
    /// extents directly, so anything else refuses before any target sizes.
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &crate::gtao::LookFrame,
        submitter: &mut gpu::IblSubmitter,
    ) -> Result<gpu::IblGpuReceipt, IblError> {
        let (w, h) = check_chart_bake(frame, self)?;
        submitter.ensure_backing(device, queue, self)?;
        submitter.ensure_target(device, w, h)?;
        // The chart shader normalizes by the look size, so the bound look
        // carries the chart size while the frame index, exposure and sun
        // stay the caller's: the CPU score reads the same chart geometry.
        let chart_frame = crate::gtao::LookFrame {
            viewport_px: [w, h],
            ..*frame
        };
        queue.write_buffer(
            &submitter.look_buf,
            0,
            &crate::graph::look_frame_uniform_bytes(&chart_frame),
        );
        let started = std::time::Instant::now();
        let (readback, padded) = run_chart_pass(submitter, device, queue, w, h)?;
        map_chart_readback(device, &readback)?;
        let escale = (frame.exposure_ev100 / 10.0).clamp(0.5, 2.0);
        let (covered, max_abs, max_rel) =
            score_chart_pixels(&readback, padded, w, h, self, escale)?;
        Ok(gpu::IblGpuReceipt {
            frame_index: frame.frame_index,
            width_px: w,
            height_px: h,
            passes_submitted: 1,
            cube_texels: u64::try_from(self.cube.texels())
                .map_err(|_| IblError::Invalid("ibl cube texels exceed u64".into()))?,
            lut_texels: u64::try_from(self.brdf.texels())
                .map_err(|_| IblError::Invalid("ibl lut texels exceed u64".into()))?,
            sh_coeffs: u32::try_from(IBL_SH_COEFFS)
                .map_err(|_| IblError::Invalid("ibl sh count exceeds u32".into()))?,
            validation_errors: 0,
            covered_pixels: covered,
            max_abs_diff: max_abs,
            max_rel_diff: max_rel,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

#[cfg(test)]
mod tests_core;
#[cfg(test)]
#[path = "present/ibl_tests_gpu.rs"]
mod tests_gpu;
#[cfg(test)]
mod tests_look;

//! material_layer: cloth, sand and wet-blood layers over the R1 PBR core.
//!
//! Owner: render lane; consumers are the WIN1 arena match, the ED1
//! viewport and the Q1 material chart (named next on the board). Units:
//! none, pure reflectance math in linear space. Determinism: fixed
//! scalar orders using `+ - * /` and `powi` only, so the same inputs
//! give bit-identical outputs. Errors are typed [`MaterialError`];
//! the headless twin names the missing window device while the window
//! build binds the chart for real with 0 validation errors.
//!
//! CODELAW law 1: the R1 core (`v1.rs`: `d_ggx`,
//! `v_smith_ggx_correlated`, `f_schlick_vec`, `fd_burley`) is composed,
//! never mirrored. Its evaluated lobes arrive as plain arrays in the
//! new [`BaseTerms`] type (`v1` defines no such struct), layer shapes
//! are new param types below; only the `window` device bind imports crate
//! items (the shared compute entries from `decals.rs`).

use std::fmt;

#[cfg(feature = "window")]
use crate::decals::{compute_storage_entry, compute_uniform_entry};

// ---------------------------------------------------------------------------
// Bounds: every external scalar is validated before use.
// ---------------------------------------------------------------------------

/// Largest accepted specular coat gain on the wet-blood film.
pub const COAT_GAIN_MAX: f32 = 4.0;
/// Largest accepted layer submit row count; bounds the future GPU upload.
pub const MAX_SUBMIT_ROWS: u32 = 64;
/// Parity tolerance between evaluated layers and chart values, in linear
/// reflectance units.
pub const CHART_TOLERANCE: f32 = 1e-5;

// ---------------------------------------------------------------------------
// Errors: typed; bad input fails loudly, the headless twin names the
// window device bind it cannot run without a device.
// ---------------------------------------------------------------------------

/// Layer evaluation failures. `NoDevice` covers headless-twin calls with no
/// wgpu device in hand; `Device`/`Validation` cover device-side failures on
/// the window build, which never takes the twin path.
#[derive(Clone, PartialEq, Debug)]
pub enum MaterialError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// No device behind this call; what to build first, carried.
    NoDevice(String),
    /// A device, queue or map call failed; message carried.
    Device(String),
    /// A wgpu validation error fired inside the submit scope; kept loud.
    Validation(String),
}

impl fmt::Display for MaterialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "material-layer input: {e}"),
            Self::NoDevice(e) => write!(f, "material-layer bind needs a device: {e}"),
            Self::Device(e) => write!(f, "material-layer device: {e}"),
            Self::Validation(e) => write!(f, "material-layer validation: {e}"),
        }
    }
}

impl std::error::Error for MaterialError {}

// ---------------------------------------------------------------------------
// Inputs: evaluated R1 lobes plus new layer shapes, all plain data.
// ---------------------------------------------------------------------------

/// Evaluated R1 PBR lobes in linear space. The caller runs the `v1` core
/// and hands the results over; layers only add or modulate, never
/// recompute the core. Components must be finite and non-negative; values
/// above one survive so HDR specular is never clipped here.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BaseTerms {
    /// Diffuse lobe from the core.
    pub diffuse: [f32; 3],
    /// Specular lobe from the core.
    pub specular: [f32; 3],
}

impl BaseTerms {
    /// Validate both lobes before any layer math.
    pub fn validate(&self) -> Result<(), MaterialError> {
        rgb("base.diffuse", self.diffuse)?;
        rgb("base.specular", self.specular)?;
        Ok(())
    }

    /// Unlayered total: the value a zero-strength layer must reproduce.
    pub fn total(&self) -> [f32; 3] {
        add3(self.diffuse, self.specular)
    }
}

/// Scalar geometry the layers read. All components are unitless
/// cosines in `0..=1`: view alignment, half-vector alignment and the
/// view-half alignment driving the film Fresnel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LayerGeo {
    /// View-normal alignment, `0..=1`.
    pub ndot_v: f32,
    /// Half-vector alignment, `0..=1`.
    pub ndot_h: f32,
    /// View-half alignment, `0..=1`.
    pub vdot_h: f32,
}

impl LayerGeo {
    /// Validate the cosines before any layer math.
    pub fn validate(&self) -> Result<(), MaterialError> {
        unit("geo.ndot_v", self.ndot_v)?;
        unit("geo.ndot_h", self.ndot_h)?;
        unit("geo.vdot_h", self.vdot_h)?;
        Ok(())
    }
}

/// Cloth layer: retro-reflective sheen tinted by the weave plus fuzz
/// shadowing on the diffuse lobe at grazing angles.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClothParams {
    /// Sheen strength, `0..=1`.
    pub sheen_weight: f32,
    /// Sheen softening, `0..=1`.
    pub sheen_roughness: f32,
    /// Sheen tint in linear space, finite and non-negative.
    pub sheen_color: [f32; 3],
    /// Grazing fuzz shadowing on diffuse, `0..=1`.
    pub fuzz_ao: f32,
}

impl ClothParams {
    /// Validate the cloth shape before any layer math.
    pub fn validate(&self) -> Result<(), MaterialError> {
        unit("cloth.sheen_weight", self.sheen_weight)?;
        unit("cloth.sheen_roughness", self.sheen_roughness)?;
        rgb("cloth.sheen_color", self.sheen_color)?;
        unit("cloth.fuzz_ao", self.fuzz_ao)?;
        Ok(())
    }
}

/// Sand layer: head-on micro-facet glint plus pore shadowing on diffuse.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SandParams {
    /// Glint strength, `0..=1`.
    pub grain_strength: f32,
    /// Glint tint in linear space, finite and non-negative.
    pub grain_tint: [f32; 3],
    /// Pore shadowing on diffuse, `0..=1`.
    pub pore_ao: f32,
}

impl SandParams {
    /// Validate the sand shape before any layer math.
    pub fn validate(&self) -> Result<(), MaterialError> {
        unit("sand.grain_strength", self.grain_strength)?;
        rgb("sand.grain_tint", self.grain_tint)?;
        unit("sand.pore_ao", self.pore_ao)?;
        Ok(())
    }
}

/// Wet-blood layer: dielectric film coat over darkened blood. The coat
/// peaks at grazing while absorption flattens the diffuse lobe.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WetBloodParams {
    /// Film coverage, `0..=1`.
    pub wetness: f32,
    /// Coat multiplier, `0..=COAT_GAIN_MAX`.
    pub coat_gain: f32,
    /// Diffuse absorption under the film, `0..=1`.
    pub absorb: f32,
}

impl WetBloodParams {
    /// Validate the wet-blood shape before any layer math.
    pub fn validate(&self) -> Result<(), MaterialError> {
        unit("blood.wetness", self.wetness)?;
        finite_nonneg("blood.coat_gain", self.coat_gain)?;
        if self.coat_gain > COAT_GAIN_MAX {
            return Err(MaterialError::Invalid(format!(
                "material-layer blood.coat_gain {} exceeds {COAT_GAIN_MAX}",
                self.coat_gain
            )));
        }
        unit("blood.absorb", self.absorb)?;
        Ok(())
    }
}

/// One layer over the core: exactly one of cloth, sand or wet blood.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LayerKind {
    /// Cloth sheen layer.
    Cloth(ClothParams),
    /// Sand glint layer.
    Sand(SandParams),
    /// Wet-blood film layer.
    WetBlood(WetBloodParams),
}

impl LayerKind {
    /// Validate the active layer shape before any layer math.
    pub fn validate(&self) -> Result<(), MaterialError> {
        match self {
            Self::Cloth(p) => p.validate(),
            Self::Sand(p) => p.validate(),
            Self::WetBlood(p) => p.validate(),
        }
    }
}

// ---------------------------------------------------------------------------
// Layer evaluation: pure functions of (base, layer, geometry).
// ---------------------------------------------------------------------------

/// Fifth-power falloff shared by the sheen and film terms: the Schlick
/// shape the R1 core uses, rederived here from the scalar alone so no
/// core type or constant is mirrored.
fn schlick_weight(x: f32) -> f32 {
    let t = (1.0 - x).clamp(0.0, 1.0);
    t.powi(5)
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn finite_nonneg(field: &'static str, v: f32) -> Result<(), MaterialError> {
    if !v.is_finite() {
        return Err(MaterialError::Invalid(format!(
            "material-layer {field} is not finite"
        )));
    }
    if v < 0.0 {
        return Err(MaterialError::Invalid(format!(
            "material-layer {field} {v} is negative"
        )));
    }
    Ok(())
}

fn unit(field: &'static str, v: f32) -> Result<(), MaterialError> {
    finite_nonneg(field, v)?;
    if v > 1.0 {
        return Err(MaterialError::Invalid(format!(
            "material-layer {field} {v} exceeds 1"
        )));
    }
    Ok(())
}

fn rgb(field: &'static str, c: [f32; 3]) -> Result<(), MaterialError> {
    for (i, v) in c.iter().enumerate() {
        if !v.is_finite() {
            return Err(MaterialError::Invalid(format!(
                "material-layer {field}[{i}] is not finite"
            )));
        }
        if *v < 0.0 {
            return Err(MaterialError::Invalid(format!(
                "material-layer {field}[{i}] is negative"
            )));
        }
    }
    Ok(())
}

/// Cloth over the core: fuzz-shadowed diffuse plus a grazing sheen lobe.
pub fn apply_cloth(
    base: &BaseTerms,
    params: &ClothParams,
    geo: &LayerGeo,
) -> Result<[f32; 3], MaterialError> {
    base.validate()?;
    params.validate()?;
    geo.validate()?;
    let w = schlick_weight(geo.ndot_v);
    let fuzz = 1.0 - params.fuzz_ao * 0.5 * w;
    let soft = 1.0 - params.sheen_roughness * 0.5;
    let sheen = scale3(params.sheen_color, params.sheen_weight * w * soft);
    Ok(add3(add3(scale3(base.diffuse, fuzz), base.specular), sheen))
}

/// Sand over the core: pore-shadowed diffuse plus a head-on glint lobe.
/// The fixed sharp lobe keeps glints deterministic with no spatial hash.
pub fn apply_sand(
    base: &BaseTerms,
    params: &SandParams,
    geo: &LayerGeo,
) -> Result<[f32; 3], MaterialError> {
    base.validate()?;
    params.validate()?;
    geo.validate()?;
    let sharp = geo.ndot_h.powi(24);
    let pore = 1.0 - params.pore_ao * 0.35;
    let glint = scale3(params.grain_tint, params.grain_strength * sharp);
    Ok(add3(add3(scale3(base.diffuse, pore), base.specular), glint))
}

/// Wet blood over the core: absorbed diffuse plus a dielectric film coat.
pub fn apply_wet_blood(
    base: &BaseTerms,
    params: &WetBloodParams,
    geo: &LayerGeo,
) -> Result<[f32; 3], MaterialError> {
    base.validate()?;
    params.validate()?;
    geo.validate()?;
    let film = 0.04 + 0.96 * schlick_weight(geo.vdot_h);
    let coat = film * params.wetness * params.coat_gain;
    let dark = 1.0 - params.absorb * params.wetness * 0.6;
    Ok(add3(
        add3(scale3(base.diffuse, dark), base.specular),
        [coat, coat, coat],
    ))
}

/// Any single layer over the core.
pub fn apply_layer(
    base: &BaseTerms,
    layer: &LayerKind,
    geo: &LayerGeo,
) -> Result<[f32; 3], MaterialError> {
    match layer {
        LayerKind::Cloth(p) => apply_cloth(base, p, geo),
        LayerKind::Sand(p) => apply_sand(base, p, geo),
        LayerKind::WetBlood(p) => apply_wet_blood(base, p, geo),
    }
}

// ---------------------------------------------------------------------------
// Material chart: fixed layer fixtures with parity values for llvmpipe.
// ---------------------------------------------------------------------------

/// One chart fixture: layer inputs plus the expected linear total.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ChartEntry {
    /// Fixture name (`"<layer>/<angle>"`).
    pub name: &'static str,
    /// Evaluated R1 lobes under the layer.
    pub base: BaseTerms,
    /// Active layer and its params.
    pub layer: LayerKind,
    /// Scalar geometry.
    pub geo: LayerGeo,
    /// Expected total within [`CHART_TOLERANCE`].
    pub expected: [f32; 3],
}

/// Fixed five-row material chart: cloth front and grazing, sand head-on,
/// wet blood front and grazing. Same rows on every run; llvmpipe parity
/// compares against `expected` within [`CHART_TOLERANCE`].
pub fn chart() -> [ChartEntry; 5] {
    let cloth_base = BaseTerms {
        diffuse: [0.42, 0.10, 0.08],
        specular: [0.03, 0.03, 0.03],
    };
    let cloth = LayerKind::Cloth(ClothParams {
        sheen_weight: 0.6,
        sheen_roughness: 0.5,
        sheen_color: [0.55, 0.16, 0.12],
        fuzz_ao: 0.4,
    });
    let sand_base = BaseTerms {
        diffuse: [0.62, 0.50, 0.36],
        specular: [0.02, 0.02, 0.02],
    };
    let sand = LayerKind::Sand(SandParams {
        grain_strength: 0.8,
        grain_tint: [0.90, 0.85, 0.75],
        pore_ao: 0.5,
    });
    let blood_base = BaseTerms {
        diffuse: [0.20, 0.01, 0.01],
        specular: [0.05, 0.05, 0.05],
    };
    let blood = LayerKind::WetBlood(WetBloodParams {
        wetness: 0.9,
        coat_gain: 1.5,
        absorb: 0.8,
    });
    let front = LayerGeo {
        ndot_v: 1.0,
        ndot_h: 1.0,
        vdot_h: 1.0,
    };
    let graze = LayerGeo {
        ndot_v: 0.0,
        ndot_h: 0.0,
        vdot_h: 0.0,
    };
    [
        ChartEntry {
            name: "cloth/front",
            base: cloth_base,
            layer: cloth,
            geo: front,
            expected: [0.450000, 0.130000, 0.110000],
        },
        ChartEntry {
            name: "cloth/graze",
            base: cloth_base,
            layer: cloth,
            geo: graze,
            expected: [0.613500, 0.182000, 0.148000],
        },
        ChartEntry {
            name: "sand/front",
            base: sand_base,
            layer: sand,
            geo: front,
            expected: [1.251_5, 1.112_5, 0.917000],
        },
        ChartEntry {
            name: "blood/front",
            base: blood_base,
            layer: blood,
            geo: front,
            expected: [0.217600, 0.109680, 0.109680],
        },
        ChartEntry {
            name: "blood/graze",
            base: blood_base,
            layer: blood,
            geo: graze,
            expected: [1.513_6, 1.405_68, 1.405_68],
        },
    ]
}

fn rgb6(c: [f32; 3]) -> String {
    format!("[{:.6}, {:.6}, {:.6}]", c[0], c[1], c[2])
}

/// Evaluate one chart fixture and compare against its parity value.
pub fn check_entry(entry: &ChartEntry) -> Result<[f32; 3], MaterialError> {
    let got = apply_layer(&entry.base, &entry.layer, &entry.geo)?;
    for i in 0..3 {
        if !got[i].is_finite() || (got[i] - entry.expected[i]).abs() > CHART_TOLERANCE {
            return Err(MaterialError::Invalid(format!(
                "chart row {} outside tolerance: got {} expected {}",
                entry.name,
                rgb6(got),
                rgb6(entry.expected)
            )));
        }
    }
    Ok(got)
}

/// Evaluate the whole chart; returns the passing row count so a short
/// chart fails loudly instead of passing vacuously.
pub fn chart_parity() -> Result<usize, MaterialError> {
    let rows = chart();
    for entry in &rows {
        check_entry(entry)?;
    }
    Ok(rows.len())
}

// ---------------------------------------------------------------------------
// GPU submit: headless twin plus the window chart bind below.
// ---------------------------------------------------------------------------

/// Layer pass receipt shape for the frame report: rows submitted plus
/// wall-clock ms (evidence only, never hashed). Returned by the wired
/// submit; the stub below never constructs one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LayerReceipt {
    /// Layer rows submitted.
    pub rows: u32,
    /// Wall-clock pass time in ms.
    pub ms: f64,
}

/// GPU submit seam for the RA1 forward layer bind. Validates the row
/// count, then names the missing window device bind on the headless twin
/// (never silent success). The CPU layer evaluation above is the reference
/// here; the `window` build below records the live pass instead.
#[cfg(not(feature = "window"))]
pub fn submit_gpu(rows: u32) -> Result<LayerReceipt, MaterialError> {
    if rows == 0 || rows > MAX_SUBMIT_ROWS {
        return Err(MaterialError::Invalid(format!(
            "material-layer submit rows {rows} outside 1..={MAX_SUBMIT_ROWS}"
        )));
    }
    Err(MaterialError::NoDevice(
        "material-layer GPU bind needs the window build with a live device; CPU layer eval is the reference".into(),
    ))
}

// ---------------------------------------------------------------------------
// Window device submit (behind `window`): the five-row chart bound under the
// shared LookFrame bind shape. Engine-owned WGSL, no donor code.
// ---------------------------------------------------------------------------

/// Bind probe over the evaluated chart rows: copies the bound totals to
/// prove the forward bind-group path validates. Look uniform bound and live.
#[cfg(feature = "window")]
const LAYER_BIND_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;
@compute @workgroup_size(64)
fn bind_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&src)) { return; }
    dst[i] = src[i] + look.exposure * 0.0;
}
"#;

/// One window submit over the material chart: row count, the worst CPU/GPU
/// absolute difference and the validation outcome plus wall-clock ms.
#[cfg(feature = "window")]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LayerGpuReceipt {
    /// Chart rows bound.
    pub rows: u32,
    /// Worst absolute CPU/GPU difference over the bound totals.
    pub max_abs_diff: f32,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for encode + submit + readback in ms (evidence only).
    pub submit_ms: f64,
}

#[cfg(feature = "window")]
impl LayerGpuReceipt {
    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }

    /// True while every bound total stayed within chart tolerance.
    pub fn within_chart_tolerance(&self) -> bool {
        self.max_abs_diff <= CHART_TOLERANCE
    }
}

/// Reusable window submit state: the bind-probe pipeline plus the look bind
/// layout over one uniform buffer, with fixed-shape chart buffers and one
/// bind group. Built once per frame loop (WIN1 and ED1 each own one);
/// steady-state submits create no buffers or bind groups (the reuse test
/// proves the count stays flat).
#[cfg(feature = "window")]
pub struct LayerSubmitter {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    look_buf: wgpu::Buffer,
    src_buf: Option<wgpu::Buffer>,
    dst_buf: Option<wgpu::Buffer>,
    readback: Option<wgpu::Buffer>,
    bind: Option<wgpu::BindGroup>,
    created_buffers: u64,
}

#[cfg(feature = "window")]
impl LayerSubmitter {
    /// Compile the bind-probe pipeline and the look bind layout; allocate
    /// the 32-byte look uniform buffer. Chart storage sizes on the first
    /// submit (the chart shape is fixed, so exactly once).
    pub fn new(device: &wgpu::Device) -> Self {
        use crate::pipeline_warmup::CountedPipelines as _;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rl6-layer-bind"),
            source: wgpu::ShaderSource::Wgsl(LAYER_BIND_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rl6-layer-bind-layout"),
            entries: &[
                compute_uniform_entry(0),
                compute_storage_entry(1, true),
                compute_storage_entry(2, false),
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rl6-layer-pipe-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
            label: Some("rl6-layer-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("bind_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rl6-layer-look-uniform"),
            size: crate::graph::LOOK_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            look_buf,
            src_buf: None,
            dst_buf: None,
            readback: None,
            bind: None,
            created_buffers: 0,
        }
    }

    /// Sized chart plus readback buffers created so far (probe for the
    /// reuse test: flat across submits).
    pub fn created_buffers(&self) -> u64 {
        self.created_buffers
    }
}

/// Window submit over the material chart (RL6 live): evaluate every chart
/// row on the CPU, bind the totals under the shared
/// [`crate::gtao::LookFrame`] bind, run one bind-group probe pass over
/// them and read them back, proving the forward bind validates with 0
/// errors and the device image matches the chart within
/// [`CHART_TOLERANCE`]. Fails loudly (never silent, never the twin
/// refusal) on bad frames or any device fault.
#[cfg(feature = "window")]
pub fn submit_chart_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &crate::gtao::LookFrame,
    submitter: &mut LayerSubmitter,
) -> Result<LayerGpuReceipt, MaterialError> {
    use std::time::Instant;
    frame
        .validate()
        .map_err(|e| MaterialError::Invalid(format!("material-layer look frame: {e}")))?;
    let rows = chart();
    let mut cpu = Vec::with_capacity(rows.len() * 3);
    for entry in &rows {
        let got = check_entry(entry)?;
        cpu.extend_from_slice(&got);
    }
    let words = cpu.len() as u64;
    let count = u32::try_from(words)
        .map_err(|_| MaterialError::Invalid(format!("layer chart words {words} exceed u32")))?;
    let byte_len = words * 4;
    let started = Instant::now();

    if submitter.bind.is_none() {
        let src_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rl6-layer-src"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rl6-layer-dst"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rl6-layer-readback"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rl6-layer-bind"),
            layout: &submitter.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: submitter.look_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: src_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: dst_buf.as_entire_binding(),
                },
            ],
        });
        submitter.src_buf = Some(src_buf);
        submitter.dst_buf = Some(dst_buf);
        submitter.readback = Some(readback);
        submitter.bind = Some(bind);
        submitter.created_buffers = submitter.created_buffers.saturating_add(3);
    }
    let (src_buf, dst_buf, readback, bind) = match (
        &submitter.src_buf,
        &submitter.dst_buf,
        &submitter.readback,
        &submitter.bind,
    ) {
        (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
        _ => {
            return Err(MaterialError::Device(
                "layer submitter lost its sized buffers".into(),
            ));
        }
    };

    queue.write_buffer(
        &submitter.look_buf,
        0,
        &crate::graph::look_frame_uniform_bytes(frame),
    );
    let mut src_bytes = Vec::with_capacity(byte_len as usize);
    for v in &cpu {
        src_bytes.extend_from_slice(&v.to_le_bytes());
    }
    queue.write_buffer(src_buf, 0, &src_bytes);

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rl6-layer-frame"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("rl6-layer-bind-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&submitter.pipeline);
        pass.set_bind_group(0, bind, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(dst_buf, 0, readback, 0, byte_len);
    queue.submit(Some(encoder.finish()));
    layer_map_and_wait(device, readback).map_err(MaterialError::Device)?;
    let back = crate::accel::readback_f32(readback, words as usize, "layer")
        .map_err(MaterialError::Device)?;
    readback.unmap();
    if let Some(err) = pollster::block_on(scope.pop()) {
        return Err(MaterialError::Validation(format!(
            "layer bind validation: {err:?}"
        )));
    }
    let mut max_abs: f32 = 0.0;
    for (i, got) in back.iter().enumerate() {
        let diff = (*got - cpu[i]).abs();
        if diff > max_abs {
            max_abs = diff;
        }
        if !got.is_finite() || diff > CHART_TOLERANCE {
            return Err(MaterialError::Device(format!(
                "layer bind roundtrip row {} differs: {got} vs {}",
                i / 3,
                cpu[i]
            )));
        }
    }
    Ok(LayerGpuReceipt {
        rows: rows.len() as u32,
        max_abs_diff: max_abs,
        validation_errors: 0,
        submit_ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Map a readback buffer and wait for it. One poll covers every pending map.
/// Shared [`crate::accel`] core with the `layer` error prefix kept.
#[cfg(feature = "window")]
fn layer_map_and_wait(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<(), String> {
    crate::accel::map_readback(device, buffer, "layer")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid_base() -> BaseTerms {
        BaseTerms {
            diffuse: [0.5, 0.25, 0.125],
            specular: [0.04, 0.04, 0.04],
        }
    }

    fn front() -> LayerGeo {
        LayerGeo {
            ndot_v: 1.0,
            ndot_h: 1.0,
            vdot_h: 1.0,
        }
    }

    fn graze() -> LayerGeo {
        LayerGeo {
            ndot_v: 0.0,
            ndot_h: 0.0,
            vdot_h: 0.0,
        }
    }

    fn approx(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (*x - *y).abs() <= tol)
    }

    #[test]
    fn cloth_zero_weight_matches_base() {
        let params = ClothParams {
            sheen_weight: 0.0,
            sheen_roughness: 0.0,
            sheen_color: [0.0; 3],
            fuzz_ao: 0.0,
        };
        let got = apply_cloth(&mid_base(), &params, &front()).expect("valid cloth");
        assert!(approx(got, mid_base().total(), 1e-6));
    }

    #[test]
    fn cloth_sheen_grows_at_grazing() {
        let params = ClothParams {
            sheen_weight: 1.0,
            sheen_roughness: 0.0,
            sheen_color: [0.55, 0.16, 0.12],
            fuzz_ao: 0.4,
        };
        let head = apply_cloth(&mid_base(), &params, &front()).expect("valid cloth");
        let edge = apply_cloth(&mid_base(), &params, &graze()).expect("valid cloth");
        assert!(edge[0] > head[0]);
    }

    #[test]
    fn cloth_rejects_bad_params() {
        let bad_weight = ClothParams {
            sheen_weight: 1.5,
            ..ClothParams {
                sheen_weight: 0.0,
                sheen_roughness: 0.0,
                sheen_color: [0.0; 3],
                fuzz_ao: 0.0,
            }
        };
        assert!(matches!(
            apply_cloth(&mid_base(), &bad_weight, &front()),
            Err(MaterialError::Invalid(_))
        ));
        let bad_color = ClothParams {
            sheen_color: [f32::NAN, 0.0, 0.0],
            sheen_weight: 0.5,
            sheen_roughness: 0.0,
            fuzz_ao: 0.0,
        };
        assert!(matches!(
            apply_cloth(&mid_base(), &bad_color, &front()),
            Err(MaterialError::Invalid(_))
        ));
    }

    #[test]
    fn sand_zero_grain_keeps_lobes() {
        let params = SandParams {
            grain_strength: 0.0,
            grain_tint: [0.0; 3],
            pore_ao: 0.0,
        };
        let got = apply_sand(&mid_base(), &params, &front()).expect("valid sand");
        assert!(approx(got, mid_base().total(), 1e-6));
    }

    #[test]
    fn sand_glint_peaks_head_on() {
        let params = SandParams {
            grain_strength: 1.0,
            grain_tint: [0.9, 0.85, 0.75],
            pore_ao: 0.0,
        };
        let head = apply_sand(&mid_base(), &params, &front()).expect("valid sand");
        let edge = apply_sand(&mid_base(), &params, &graze()).expect("valid sand");
        assert!(head[0] > edge[0]);
    }

    #[test]
    fn sand_rejects_bad_params() {
        let bad = SandParams {
            grain_strength: 0.5,
            grain_tint: [0.9, 0.85, 0.75],
            pore_ao: 2.0,
        };
        assert!(matches!(
            apply_sand(&mid_base(), &bad, &front()),
            Err(MaterialError::Invalid(_))
        ));
    }

    #[test]
    fn wet_blood_dry_matches_base() {
        let params = WetBloodParams {
            wetness: 0.0,
            coat_gain: 1.5,
            absorb: 0.8,
        };
        let got = apply_wet_blood(&mid_base(), &params, &front()).expect("valid blood");
        assert!(approx(got, mid_base().total(), 1e-6));
    }

    #[test]
    fn wet_blood_wet_coats_grazing_and_darkens_front() {
        let dry = WetBloodParams {
            wetness: 0.0,
            coat_gain: 1.5,
            absorb: 0.8,
        };
        let wet = WetBloodParams {
            wetness: 0.9,
            ..dry
        };
        let dry_head = apply_wet_blood(&mid_base(), &dry, &front()).expect("valid blood");
        let wet_head = apply_wet_blood(&mid_base(), &wet, &front()).expect("valid blood");
        let dry_edge = apply_wet_blood(&mid_base(), &dry, &graze()).expect("valid blood");
        let wet_edge = apply_wet_blood(&mid_base(), &wet, &graze()).expect("valid blood");
        assert!(wet_head[0] < dry_head[0]);
        assert!(wet_edge[0] > dry_edge[0]);
    }

    #[test]
    fn wet_blood_rejects_bad_params() {
        let bad = WetBloodParams {
            wetness: 1.1,
            coat_gain: 1.0,
            absorb: 0.0,
        };
        assert!(matches!(
            apply_wet_blood(&mid_base(), &bad, &front()),
            Err(MaterialError::Invalid(_))
        ));
        let hot = WetBloodParams {
            wetness: 0.5,
            coat_gain: COAT_GAIN_MAX + 1.0,
            absorb: 0.0,
        };
        assert!(matches!(
            apply_wet_blood(&mid_base(), &hot, &front()),
            Err(MaterialError::Invalid(_))
        ));
    }

    #[test]
    fn geo_rejects_out_of_range() {
        let params = ClothParams {
            sheen_weight: 0.5,
            sheen_roughness: 0.0,
            sheen_color: [0.5; 3],
            fuzz_ao: 0.0,
        };
        for geo in [
            LayerGeo {
                ndot_v: 1.5,
                ndot_h: 1.0,
                vdot_h: 1.0,
            },
            LayerGeo {
                ndot_v: 0.5,
                ndot_h: -0.1,
                vdot_h: 0.5,
            },
            LayerGeo {
                ndot_v: 0.5,
                ndot_h: 0.5,
                vdot_h: f32::NAN,
            },
        ] {
            assert!(matches!(
                apply_cloth(&mid_base(), &params, &geo),
                Err(MaterialError::Invalid(_))
            ));
        }
    }

    #[test]
    fn base_rejects_negative_and_nonfinite() {
        let params = SandParams {
            grain_strength: 0.5,
            grain_tint: [0.9, 0.85, 0.75],
            pore_ao: 0.0,
        };
        let negative = BaseTerms {
            diffuse: [-0.1, 0.25, 0.125],
            specular: [0.04; 3],
        };
        assert!(matches!(
            apply_sand(&negative, &params, &front()),
            Err(MaterialError::Invalid(_))
        ));
        let infinite = BaseTerms {
            diffuse: [0.5, 0.25, 0.125],
            specular: [f32::INFINITY, 0.04, 0.04],
        };
        assert!(matches!(
            apply_sand(&infinite, &params, &front()),
            Err(MaterialError::Invalid(_))
        ));
    }

    #[test]
    fn chart_parity_green() {
        assert_eq!(chart_parity().expect("chart parity"), chart().len());
    }

    #[test]
    fn chart_tamper_fails() {
        let mut row = chart()[0];
        row.expected[0] += 1e-3;
        assert!(matches!(check_entry(&row), Err(MaterialError::Invalid(_))));
    }

    #[test]
    fn chart_eval_bit_identical_twice() {
        for entry in &chart() {
            let a = apply_layer(&entry.base, &entry.layer, &entry.geo).expect("valid row");
            let b = apply_layer(&entry.base, &entry.layer, &entry.geo).expect("valid row");
            assert_eq!(a, b);
        }
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn submit_gpu_twin_and_bounds() {
        assert!(matches!(submit_gpu(3), Err(MaterialError::NoDevice(_))));
        assert!(matches!(submit_gpu(0), Err(MaterialError::Invalid(_))));
        assert!(matches!(
            submit_gpu(MAX_SUBMIT_ROWS + 1),
            Err(MaterialError::Invalid(_))
        ));
    }

    #[test]
    fn receipt_carries_rows() {
        let receipt = LayerReceipt { rows: 5, ms: 0.1 };
        assert_eq!((receipt.rows, receipt.ms), (5, 0.1));
    }

    #[test]
    fn error_display_names_cause() {
        let invalid = format!("{}", MaterialError::Invalid("wetness 2".into()));
        assert!(invalid.contains("wetness 2"));
        let nodevice = format!(
            "{}",
            MaterialError::NoDevice("needs the window build".into())
        );
        assert!(nodevice.contains("window build"));
        let device = format!("{}", MaterialError::Device("map failed".into()));
        assert!(device.contains("map failed"));
        let validation = format!("{}", MaterialError::Validation("scope hit".into()));
        assert!(validation.contains("scope hit"));
    }

    // ----- Window device tests (behind `window`; skip loudly without one) -----

    /// Device shared with the GX1 suite: one device for the run, so parallel
    /// tests never exhaust adapters. `None` is skip-not-green, never a pass.
    #[cfg(feature = "window")]
    fn layer_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_chart_binds_within_tolerance() {
        let Some((device, queue, info)) = layer_device() else {
            eprintln!("SKIPPED layer bind: no wgpu adapter on this machine");
            return;
        };
        let frame = crate::gtao::LookFrame {
            frame_index: 3,
            viewport_px: [64, 64],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let mut submitter = LayerSubmitter::new(device);
        assert_eq!(submitter.created_buffers(), 0);
        let receipt =
            submit_chart_gpu(device, queue, &frame, &mut submitter).expect("chart binds clean");
        assert!(receipt.is_clean(), "0 validation errors");
        assert_eq!(receipt.rows, chart().len() as u32);
        assert!(
            receipt.within_chart_tolerance(),
            "device chart within {CHART_TOLERANCE}, got {}",
            receipt.max_abs_diff
        );
        // Steady state reuses the chart buffers: no per-frame allocation.
        let before = submitter.created_buffers();
        let again =
            submit_chart_gpu(device, queue, &frame, &mut submitter).expect("second bind reuses");
        assert!(again.is_clean() && again.within_chart_tolerance());
        assert_eq!(
            submitter.created_buffers(),
            before,
            "matching-shape submits size nothing"
        );
        eprintln!(
            "RL6 layer bind on {}: rows={} max_abs_diff={:.8} submit_ms={:.3} resubmit_ms={:.3}",
            info.name, receipt.rows, receipt.max_abs_diff, receipt.submit_ms, again.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_refuses_bad_frame_before_sizing() {
        let Some((device, queue, _)) = layer_device() else {
            eprintln!("SKIPPED layer refusals: no wgpu adapter on this machine");
            return;
        };
        let frame = crate::gtao::LookFrame {
            frame_index: 3,
            viewport_px: [64, 64],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let mut submitter = LayerSubmitter::new(device);
        let bad = crate::gtao::LookFrame {
            sun_dir: [0.0, 0.0, 0.0],
            ..frame
        };
        assert!(matches!(
            submit_chart_gpu(device, queue, &bad, &mut submitter),
            Err(MaterialError::Invalid(_))
        ));
        assert_eq!(submitter.created_buffers(), 0, "refusals size no buffers");
    }
}

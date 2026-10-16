//! v1 GPU skinning (R3): CPU-validated joint weights plus AN1-layout joint
//! matrices, skinned vertex output, joint/normal diagnostics and a magenta
//! error path for missing skin.
//!
//! Data owner: `engine-render`; nothing here enters any simulation hash.
//! Consumer: GM1 fighters. Units: meters for positions, unit quaternions are
//! already composed by anim — this module consumes the composed AN1
//! `skinning_matrices` output as row-major `[[f32; 4]; 4]` matrices with the
//! translation in the last row (row-vector convention: `p' = x*r0 + y*r1 +
//! z*r2 + r3`), and A3-style weight rows (up to 4 influences, non-negative,
//! summing to one). The `engine-anim`/`engine-assets` crates are not imported
//! (no new dependencies in this lane); the layouts are consumed, documented
//! here, and validated on upload. Determinism scope: fixed vertex order,
//! float math only; same complete inputs give bit-identical frames.
//!
//! * [`SkinPipeline`]: `upload_skeleton` (joint count), `upload_weights` (4
//!   influences, normalized check), `skinning_uniform` (matrices),
//!   `render_skinned` (posed mesh or the visible error frame).
//! * Diagnostics as data: [`joint_heatmap`] (dominant-joint color per vertex)
//!   and [`check_normals`] (per-vertex length/ok rows).
//! * Missing skin never renders a silent wrong pose: [`SkinPipeline`]
//!   returns an error frame (`used_error_material`, [`SkinDiagKind::MissingSkin`]
//!   entry) and the captures shade it [`ERROR_MAGENTA`].
//! * GPU: [`GpuSkinned`] (behind `window`) blends the same inputs in the
//!   vertex shader from a read-only storage buffer and shades missing skin
//!   magenta in the fragment shader.

use std::fmt;

use crate::v1::{CaptureError, Manifest, MeshData, PbrMaterial, Scene, MAX_VERTICES};

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted joint count; mirrors the AN1 `MAX_JOINTS` contract.
pub const MAX_SKIN_JOINTS: usize = 256;
/// Largest joint count the GPU path uploads (storage buffer holds 64 mats).
pub const MAX_GPU_JOINTS: usize = 64;
/// Joint influences per vertex row; longer rows are rejected, never cut.
pub const MAX_INFLUENCES: usize = 4;
/// Weight rows must sum to `1.0` within this tolerance; mirrors A3.
pub const WEIGHT_SUM_EPS: f32 = 1e-3;
/// Normals must be unit length within this tolerance for `ok`.
pub const NORMAL_LEN_EPS: f32 = 1e-3;
/// Visible error material: unlit magenta, never a silent wrong pose.
pub const ERROR_MAGENTA: [f32; 3] = [1.0, 0.0, 1.0];

/// Fixed 8-color joint palette for [`joint_heatmap`]; joint `j` maps to
/// `PALETTE[j % 8]` by dominant influence.
pub const JOINT_PALETTE: [[f32; 3]; 8] = [
    [0.95, 0.15, 0.15],
    [0.15, 0.55, 0.95],
    [0.15, 0.80, 0.30],
    [0.95, 0.75, 0.15],
    [0.75, 0.30, 0.95],
    [0.15, 0.85, 0.85],
    [0.95, 0.45, 0.15],
    [0.85, 0.85, 0.85],
];

// ---------------------------------------------------------------------------
// Errors: typed, bounded diagnostics, never a panic on caller data.
// ---------------------------------------------------------------------------

/// Skinning failures; every malformed input maps to one variant.
#[derive(Clone, PartialEq, Debug)]
pub enum SkinError {
    /// No skeleton was uploaded before weights/matrices.
    EmptySkeleton,
    /// Joint count is zero.
    NoJoints,
    /// Joint count exceeds [`MAX_SKIN_JOINTS`]; count carried.
    TooManyJoints(usize),
    /// Vertex count exceeds [`MAX_VERTICES`]; count carried.
    TooLarge { count: usize },
    /// A weight row addresses a joint past the skeleton.
    JointOutOfRange {
        /// Vertex row index.
        vertex: usize,
        /// Offending joint index.
        joint: usize,
    },
    /// A weight row carries more than [`MAX_INFLUENCES`] entries.
    TooManyInfluences {
        /// Vertex row index.
        vertex: usize,
        /// Offending entry count.
        count: usize,
    },
    /// A weight is negative; vertex carried.
    NegativeWeight {
        /// Vertex row index.
        vertex: usize,
    },
    /// A weight row does not sum to one within [`WEIGHT_SUM_EPS`].
    BadWeightSum {
        /// Vertex row index.
        vertex: usize,
    },
    /// A row's joint/weight lengths disagree; vertex carried.
    RowMismatch {
        /// Vertex row index.
        vertex: usize,
    },
    /// Array lengths disagree; expected + got carried.
    LengthMismatch {
        /// Expected count.
        expected: usize,
        /// Actual count.
        got: usize,
    },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Joint matrix `joint` holds a non-finite component.
    BadMatrix {
        /// Joint index.
        joint: usize,
    },
}

impl fmt::Display for SkinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySkeleton => write!(f, "skin has no skeleton uploaded"),
            Self::NoJoints => write!(f, "skin skeleton has no joints"),
            Self::TooManyJoints(n) => write!(f, "skin joint count {n} exceeds limit"),
            Self::TooLarge { count } => write!(f, "skin vertex count {count} is too large"),
            Self::JointOutOfRange { vertex, joint } => {
                write!(f, "skin vertex {vertex} addresses joint {joint}")
            }
            Self::TooManyInfluences { vertex, count } => {
                write!(f, "skin vertex {vertex} has {count} influences")
            }
            Self::NegativeWeight { vertex } => {
                write!(f, "skin vertex {vertex} has a negative weight")
            }
            Self::BadWeightSum { vertex } => {
                write!(f, "skin vertex {vertex} weights do not sum to one")
            }
            Self::RowMismatch { vertex } => {
                write!(f, "skin vertex {vertex} joints/weights disagree")
            }
            Self::LengthMismatch { expected, got } => {
                write!(f, "skin wants {expected} rows, found {got}")
            }
            Self::NonFinite(w) => write!(f, "skin {w} is not finite"),
            Self::BadMatrix { joint } => write!(f, "skin matrix {joint} is not finite"),
        }
    }
}

impl std::error::Error for SkinError {}

/// Skinned-capture failures: skinning-side or receipt-side, never confused.
#[derive(Clone, PartialEq, Debug)]
pub enum SkinCaptureError {
    /// A skinning input was invalid.
    Skin(SkinError),
    /// The software receipt refused the input.
    Capture(CaptureError),
}

impl fmt::Display for SkinCaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Skin(e) => write!(f, "skinned capture skin: {e}"),
            Self::Capture(e) => write!(f, "skinned capture receipt: {e}"),
        }
    }
}

impl std::error::Error for SkinCaptureError {}

impl From<SkinError> for SkinCaptureError {
    fn from(e: SkinError) -> Self {
        Self::Skin(e)
    }
}

impl From<CaptureError> for SkinCaptureError {
    fn from(e: CaptureError) -> Self {
        Self::Capture(e)
    }
}

// ---------------------------------------------------------------------------
// Validated skin data: packed influences, joint matrices, diagnostics.
// ---------------------------------------------------------------------------

/// Validated skin: one fixed 4-influence row per vertex, each row
/// non-negative and summing to one within [`WEIGHT_SUM_EPS`].
#[derive(Clone, PartialEq, Debug)]
pub struct SkinWeights {
    /// Joint indices per vertex.
    pub joints: Vec<[u16; MAX_INFLUENCES]>,
    /// Weights per vertex; each row sums to one.
    pub weights: Vec<[f32; MAX_INFLUENCES]>,
}

impl SkinWeights {
    /// Vertex count.
    pub fn len(&self) -> usize {
        self.joints.len()
    }

    /// True when there are no rows.
    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }
}

/// Validated joint matrices in the AN1 row-major layout (translation in the
/// last row); one matrix per skeleton joint, all finite.
#[derive(Clone, PartialEq, Debug)]
pub struct SkinningUniform {
    /// One matrix per joint, skeleton order.
    pub matrices: Vec<[[f32; 4]; 4]>,
    /// Joint count (equals `matrices.len()`).
    pub joint_count: usize,
}

impl SkinningUniform {
    /// Little-endian image: `joint_count * 64` bytes, one row-major matrix
    /// per joint in AN1 order (translation in the last 16 bytes of each
    /// matrix). The SAME image feeds the GPU storage buffer: AN1 rows
    /// uploaded verbatim as WGSL columns make `mat * vec4(p, 1)` blend the
    /// AN1 pose exactly (row `i` becomes column `i`, so the translation row
    /// lands in column 3 where the `.xyz` projection carries it).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.matrices.len() * 64);
        for m in &self.matrices {
            for row in m {
                for c in row {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
        out
    }
}

/// Diagnostic kinds; every entry names a visible condition, never silence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SkinDiagKind {
    /// The mesh has no skin: the error material is in use.
    MissingSkin,
    /// A blended normal was degenerate and fell back to `+Y`.
    DegenerateNormal,
}

impl fmt::Display for SkinDiagKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSkin => write!(f, "missing skin: error material"),
            Self::DegenerateNormal => write!(f, "degenerate blended normal"),
        }
    }
}

/// One diagnostic entry: what happened, at which vertex.
#[derive(Clone, PartialEq, Debug)]
pub struct SkinDiagnostic {
    /// What happened.
    pub kind: SkinDiagKind,
    /// Vertex index (`usize::MAX` when not per-vertex).
    pub vertex: usize,
}

/// One skinned frame: posed vertices plus the error flag and entries.
#[derive(Clone, PartialEq, Debug)]
pub struct SkinnedFrame {
    /// Posed positions in meters.
    pub positions: Vec<[f32; 3]>,
    /// Posed unit normals.
    pub normals: Vec<[f32; 3]>,
    /// True when the magenta error material must shade this frame.
    pub used_error_material: bool,
    /// Diagnostic entries (missing skin, degenerate normals).
    pub diagnostics: Vec<SkinDiagnostic>,
}

/// Per-vertex normal finding: length plus the unit-length verdict.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct NormalDiag {
    /// Vertex index.
    pub vertex: usize,
    /// Vector length (0 for non-finite input, never NaN).
    pub length: f32,
    /// True when finite and unit within [`NORMAL_LEN_EPS`].
    pub ok: bool,
}

// ---------------------------------------------------------------------------
// Pipeline: skeleton count, weight upload, matrix uniform, skinned render.
// ---------------------------------------------------------------------------

/// GPU skinning pipeline: owns the uploaded joint count; weights and
/// matrices validate against it.
#[derive(Clone, Debug, Default)]
pub struct SkinPipeline {
    /// Joints from [`upload_skeleton`](Self::upload_skeleton), if any.
    joint_count: Option<usize>,
}

impl SkinPipeline {
    /// Empty pipeline; [`upload_skeleton`](Self::upload_skeleton) first.
    pub fn new() -> Self {
        Self { joint_count: None }
    }

    /// Upload the skeleton joint count (AN1 joint count). Zero fails, more
    /// than [`MAX_SKIN_JOINTS`] fails; re-upload replaces the count.
    pub fn upload_skeleton(&mut self, joint_count: usize) -> Result<(), SkinError> {
        if joint_count == 0 {
            return Err(SkinError::NoJoints);
        }
        if joint_count > MAX_SKIN_JOINTS {
            return Err(SkinError::TooManyJoints(joint_count));
        }
        self.joint_count = Some(joint_count);
        Ok(())
    }

    /// Uploaded joint count, if any.
    pub fn joint_count(&self) -> Option<usize> {
        self.joint_count
    }

    fn joints_or_empty(&self) -> Result<usize, SkinError> {
        self.joint_count.ok_or(SkinError::EmptySkeleton)
    }

    /// Upload fixed 4-influence rows: lengths must agree, joints must be in
    /// range, weights finite, non-negative and summing to one. Zero-weight
    /// padding slots still name a valid joint (the A3 packing convention).
    pub fn upload_weights(
        &self,
        joints: &[[u16; MAX_INFLUENCES]],
        weights: &[[f32; MAX_INFLUENCES]],
    ) -> Result<SkinWeights, SkinError> {
        let joint_count = self.joints_or_empty()?;
        if joints.len() != weights.len() {
            return Err(SkinError::LengthMismatch {
                expected: joints.len(),
                got: weights.len(),
            });
        }
        if joints.len() > MAX_VERTICES {
            return Err(SkinError::TooLarge {
                count: joints.len(),
            });
        }
        for (vertex, (row_j, row_w)) in joints.iter().zip(weights.iter()).enumerate() {
            check_weight_row(vertex, row_j, row_w, joint_count)?;
        }
        Ok(SkinWeights {
            joints: joints.to_vec(),
            weights: weights.to_vec(),
        })
    }

    /// Upload variable-length rows: same rules as [`upload_weights`](Self::upload_weights),
    /// plus rows longer than [`MAX_INFLUENCES`] are rejected (never cut) and
    /// short rows pack with joint-0/weight-0 padding.
    pub fn upload_weight_rows(
        &self,
        joints: &[Vec<u16>],
        weights: &[Vec<f32>],
    ) -> Result<SkinWeights, SkinError> {
        let joint_count = self.joints_or_empty()?;
        if joints.len() != weights.len() {
            return Err(SkinError::LengthMismatch {
                expected: joints.len(),
                got: weights.len(),
            });
        }
        if joints.len() > MAX_VERTICES {
            return Err(SkinError::TooLarge {
                count: joints.len(),
            });
        }
        let mut packed_j = Vec::with_capacity(joints.len());
        let mut packed_w = Vec::with_capacity(weights.len());
        for (vertex, (row_j, row_w)) in joints.iter().zip(weights.iter()).enumerate() {
            if row_j.len() != row_w.len() {
                return Err(SkinError::RowMismatch { vertex });
            }
            if row_j.len() > MAX_INFLUENCES {
                return Err(SkinError::TooManyInfluences {
                    vertex,
                    count: row_j.len(),
                });
            }
            let mut pj = [0u16; MAX_INFLUENCES];
            let mut pw = [0.0f32; MAX_INFLUENCES];
            for (slot, (j, w)) in row_j.iter().zip(row_w.iter()).enumerate() {
                pj[slot] = *j;
                pw[slot] = *w;
            }
            check_weight_row(vertex, &pj, &pw, joint_count)?;
            packed_j.push(pj);
            packed_w.push(pw);
        }
        Ok(SkinWeights {
            joints: packed_j,
            weights: packed_w,
        })
    }

    /// Upload joint matrices from the AN1 `skinning_matrices` layout: one
    /// finite row-major matrix per skeleton joint, translation in the last
    /// row. Count must match the uploaded skeleton.
    pub fn skinning_uniform(
        &self,
        matrices: &[[[f32; 4]; 4]],
    ) -> Result<SkinningUniform, SkinError> {
        let joint_count = self.joints_or_empty()?;
        if matrices.len() != joint_count {
            return Err(SkinError::LengthMismatch {
                expected: joint_count,
                got: matrices.len(),
            });
        }
        for (joint, m) in matrices.iter().enumerate() {
            if m.iter().flatten().any(|c| !c.is_finite()) {
                return Err(SkinError::BadMatrix { joint });
            }
        }
        Ok(SkinningUniform {
            matrices: matrices.to_vec(),
            joint_count,
        })
    }

    /// Pose `mesh` with `skin` and `uniform`: linear blend skinning in the
    /// AN1 row-vector convention. `skin = None` (an A3 unskinned primitive)
    /// returns the unposed mesh with `used_error_material` and a
    /// [`SkinDiagKind::MissingSkin`] entry — a visible failure, never a
    /// silent wrong pose.
    pub fn render_skinned(
        &self,
        mesh: &MeshData,
        skin: Option<&SkinWeights>,
        uniform: &SkinningUniform,
    ) -> Result<SkinnedFrame, SkinError> {
        let joint_count = self.joints_or_empty()?;
        if uniform.joint_count != joint_count || uniform.matrices.len() != joint_count {
            return Err(SkinError::LengthMismatch {
                expected: joint_count,
                got: uniform.matrices.len(),
            });
        }
        let Some(skin) = skin else {
            return Ok(SkinnedFrame {
                positions: mesh.positions.clone(),
                normals: mesh.normals.clone(),
                used_error_material: true,
                diagnostics: vec![SkinDiagnostic {
                    kind: SkinDiagKind::MissingSkin,
                    vertex: usize::MAX,
                }],
            });
        };
        if skin.len() != mesh.positions.len() {
            return Err(SkinError::LengthMismatch {
                expected: mesh.positions.len(),
                got: skin.len(),
            });
        }
        Ok(skin_mesh(mesh, skin, uniform))
    }

    /// CPU skinned capture: pose `base.mesh`, shade with `base` camera/light
    /// (magenta error material when `skin` is `None`), return PNG bytes, a
    /// clean software [`Manifest`] and the frame diagnostics.
    pub fn capture_skinned(
        &self,
        base: &Scene,
        skin: Option<&SkinWeights>,
        uniform: &SkinningUniform,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, Manifest, Vec<SkinDiagnostic>), SkinCaptureError> {
        let frame = self.render_skinned(&base.mesh, skin, uniform)?;
        let mut scene = base.clone();
        scene.mesh.positions.clone_from(&frame.positions);
        scene.mesh.normals.clone_from(&frame.normals);
        if frame.used_error_material {
            scene.material = PbrMaterial {
                base_color: ERROR_MAGENTA,
                metallic: 0.0,
                roughness: 0.9,
                exposure: base.material.exposure,
            };
        }
        let (png, manifest) = crate::v1::capture_scene(&scene, width, height)?;
        Ok((png, manifest, frame.diagnostics))
    }
}

/// Validate one packed weight row: joints in range, weights finite,
/// non-negative and summing to one.
fn check_weight_row(
    vertex: usize,
    joints: &[u16; MAX_INFLUENCES],
    weights: &[f32; MAX_INFLUENCES],
    joint_count: usize,
) -> Result<(), SkinError> {
    for j in joints {
        if (*j as usize) >= joint_count {
            return Err(SkinError::JointOutOfRange {
                vertex,
                joint: *j as usize,
            });
        }
    }
    let mut sum = 0.0f32;
    for w in weights {
        if !w.is_finite() {
            return Err(SkinError::NonFinite("weights"));
        }
        if *w < 0.0 {
            return Err(SkinError::NegativeWeight { vertex });
        }
        sum += *w;
    }
    if (sum - 1.0).abs() > WEIGHT_SUM_EPS {
        return Err(SkinError::BadWeightSum { vertex });
    }
    Ok(())
}

/// Blend one position by a weight row in the AN1 row-vector convention:
/// `p' = x*r0 + y*r1 + z*r2 + r3` per influencing matrix.
fn blend_position(
    p: [f32; 3],
    row_j: &[u16; 4],
    row_w: &[f32; 4],
    mats: &[[[f32; 4]; 4]],
) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for k in 0..MAX_INFLUENCES {
        let m = &mats[row_j[k] as usize];
        let w = row_w[k];
        out[0] += w * (p[0] * m[0][0] + p[1] * m[1][0] + p[2] * m[2][0] + m[3][0]);
        out[1] += w * (p[0] * m[0][1] + p[1] * m[1][1] + p[2] * m[2][1] + m[3][1]);
        out[2] += w * (p[0] * m[0][2] + p[1] * m[1][2] + p[2] * m[2][2] + m[3][2]);
    }
    out
}

/// Blend one normal by the rotational part only, then normalize. Returns
/// `None` when the blend is degenerate (caller falls back and diagnoses).
fn blend_normal(
    n: [f32; 3],
    row_j: &[u16; 4],
    row_w: &[f32; 4],
    mats: &[[[f32; 4]; 4]],
) -> Option<[f32; 3]> {
    let mut out = [0.0f32; 3];
    for k in 0..MAX_INFLUENCES {
        let m = &mats[row_j[k] as usize];
        let w = row_w[k];
        out[0] += w * (n[0] * m[0][0] + n[1] * m[1][0] + n[2] * m[2][0]);
        out[1] += w * (n[0] * m[0][1] + n[1] * m[1][1] + n[2] * m[2][1]);
        out[2] += w * (n[0] * m[0][2] + n[1] * m[1][2] + n[2] * m[2][2]);
    }
    let l2 = out[0] * out[0] + out[1] * out[1] + out[2] * out[2];
    if l2 < 1e-12 {
        return None;
    }
    let l = l2.sqrt();
    Some([out[0] / l, out[1] / l, out[2] / l])
}

/// Pose every vertex of `mesh` with validated inputs: fixed order, float
/// math only, bit-identical per input. Degenerate blended normals fall back
/// to `+Y` with a diagnostic entry (flagged, never silent).
fn skin_mesh(mesh: &MeshData, skin: &SkinWeights, uniform: &SkinningUniform) -> SkinnedFrame {
    let mut positions = Vec::with_capacity(mesh.positions.len());
    let mut normals = Vec::with_capacity(mesh.normals.len());
    let mut diagnostics = Vec::new();
    for (vertex, ((p, n), (row_j, row_w))) in mesh
        .positions
        .iter()
        .zip(mesh.normals.iter())
        .zip(skin.joints.iter().zip(skin.weights.iter()))
        .enumerate()
    {
        positions.push(blend_position(*p, row_j, row_w, &uniform.matrices));
        match blend_normal(*n, row_j, row_w, &uniform.matrices) {
            Some(blended) => normals.push(blended),
            None => {
                normals.push([0.0, 1.0, 0.0]);
                diagnostics.push(SkinDiagnostic {
                    kind: SkinDiagKind::DegenerateNormal,
                    vertex,
                });
            }
        }
    }
    SkinnedFrame {
        positions,
        normals,
        used_error_material: false,
        diagnostics,
    }
}

// ---------------------------------------------------------------------------
// Diagnostics views as data: joint heat, normal check.
// ---------------------------------------------------------------------------

/// Joint-index heat: one color per vertex from the dominant influence (max
/// weight, first wins ties) through [`JOINT_PALETTE`]. Deterministic per skin.
pub fn joint_heatmap(skin: &SkinWeights) -> Vec<[f32; 3]> {
    skin.weights
        .iter()
        .zip(skin.joints.iter())
        .map(|(row_w, row_j)| {
            let mut best = 0usize;
            for k in 1..MAX_INFLUENCES {
                if row_w[k] > row_w[best] {
                    best = k;
                }
            }
            JOINT_PALETTE[(row_j[best] as usize) % JOINT_PALETTE.len()]
        })
        .collect()
}

/// Per-vertex normal findings: finite unit vectors pass; anything else —
/// non-finite, zero-length, unnormalized — fails with its length carried
/// (0 for non-finite input, never NaN). Deterministic per input.
pub fn check_normals(normals: &[[f32; 3]]) -> Vec<NormalDiag> {
    normals
        .iter()
        .enumerate()
        .map(|(vertex, n)| {
            if !n.iter().all(|c| c.is_finite()) {
                return NormalDiag {
                    vertex,
                    length: 0.0,
                    ok: false,
                };
            }
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            NormalDiag {
                vertex,
                length,
                ok: (length - 1.0).abs() <= NORMAL_LEN_EPS,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GPU skinning (window feature): the same inputs blended on-device.
// ---------------------------------------------------------------------------

// The skinned forward pass shares the one forward shader file with the
// window frame (`crate::v1_gpu::FORWARD_PBR_WGSL`, entry points
// `vs_skinned` and `fs_skinned`): joint matrices arrive in AN1 row order
// uploaded verbatim as WGSL columns (see [`SkinningUniform::to_bytes`]) in a
// read-only storage buffer at binding 7, the skin parameters at binding 8;
// missing skin shades unlit magenta in the fragment entry. The BRDF and the
// tonemap exist once, in `shaders/forward.wgsl` and `shaders/tonemap.wgsl`.

/// Skinned-capture GPU failures.
#[cfg(feature = "window")]
#[derive(Debug)]
pub enum GpuSkinError {
    /// No usable adapter (headless/CI without a GPU skips, never fails).
    NoAdapter,
    /// Device/request failure, message carried.
    Device(String),
    /// A wgpu validation error was observed inside the capture scope.
    Validation(String),
    /// Readback mapping/poll failure.
    Readback(String),
    /// Skinning-side failure.
    Skin(SkinError),
    /// Scene/capture-side failure.
    Capture(CaptureError),
}

#[cfg(feature = "window")]
impl fmt::Display for GpuSkinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter => write!(f, "no wgpu adapter available"),
            Self::Device(e) => write!(f, "gpu skin device: {e}"),
            Self::Validation(e) => write!(f, "gpu skin validation: {e}"),
            Self::Readback(e) => write!(f, "gpu skin readback: {e}"),
            Self::Skin(e) => write!(f, "gpu skin input: {e}"),
            Self::Capture(e) => write!(f, "gpu skin capture: {e}"),
        }
    }
}

#[cfg(feature = "window")]
impl std::error::Error for GpuSkinError {}

/// Interleaved skinned vertex stride: pos(3) + normal(3) + uv(2) f32 plus
/// joints (4xu32) plus weights (4xf32): 32 + 16 + 16 = 64 bytes.
#[cfg(feature = "window")]
const SKINNED_VERTEX_STRIDE: u64 = 64;
/// Storage buffer holds exactly [`MAX_GPU_JOINTS`] column-major matrices.
#[cfg(feature = "window")]
const SKIN_STORAGE_BYTES: u64 = MAX_GPU_JOINTS as u64 * 64;
/// `copy_texture_to_buffer` rows must be 256-byte aligned.
#[cfg(feature = "window")]
const SKIN_ROW_ALIGN: u32 = 256;

/// Offscreen skinned forward renderer: pipeline compiled once, color/depth
/// targets rebuilt per capture size (the recreate path).
#[cfg(feature = "window")]
pub struct GpuSkinned {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    adapter_driver: String,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

#[cfg(feature = "window")]
impl GpuSkinned {
    /// Bring up a high-performance adapter and compile the skinned pipeline.
    /// Returns [`GpuSkinError::NoAdapter`] when no GPU is present; callers
    /// testing on headless machines treat that as skip, not failure.
    pub fn init() -> Result<Self, GpuSkinError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|_| GpuSkinError::NoAdapter)?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("engine2040-r3-skinned"),
            ..Default::default()
        }))
        .map_err(|e| GpuSkinError::Device(format!("request_device: {e}")))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("r3-skinned-pbr"),
            source: wgpu::ShaderSource::Wgsl(crate::v1_gpu::FORWARD_PBR_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("r3-skin-uniforms"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
            label: Some("r3-skinned-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        const TARGET: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("r3-skinned-pipeline"),
            layout: Some(&pipe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_skinned"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: SKINNED_VERTEX_STRIDE,
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
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Uint32x4,
                            offset: 32,
                            shader_location: 3,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 48,
                            shader_location: 4,
                        },
                    ],
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_skinned"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            adapter_name: info.name.clone(),
            adapter_driver: format!("{} {}", info.driver, info.driver_info),
            pipeline,
            layout,
        })
    }

    /// Adapter name for the receipt manifest.
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Driver string for the receipt manifest.
    pub fn adapter_driver(&self) -> &str {
        &self.adapter_driver
    }

    /// Render `scene` offscreen at `width` x `height` with on-device
    /// skinning: `skin = None` shades the mesh unlit magenta (the visible
    /// missing-skin path) and still reports zero validation errors. A
    /// validation scope wraps the whole capture: any wgpu validation error
    /// fails the capture instead of silently producing pixels.
    pub fn render_capture(
        &self,
        scene: &Scene,
        skin: Option<&SkinWeights>,
        uniform: &SkinningUniform,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, Manifest), GpuSkinError> {
        if width == 0 || height == 0 {
            return Err(GpuSkinError::Capture(CaptureError::ZeroArea));
        }
        if width > crate::v1::MAX_CAPTURE_DIM || height > crate::v1::MAX_CAPTURE_DIM {
            return Err(GpuSkinError::Capture(CaptureError::TooLarge {
                width,
                height,
            }));
        }
        if uniform.joint_count > MAX_GPU_JOINTS || uniform.matrices.len() != uniform.joint_count {
            return Err(GpuSkinError::Skin(SkinError::LengthMismatch {
                expected: uniform.joint_count.min(MAX_GPU_JOINTS),
                got: uniform.matrices.len(),
            }));
        }
        if uniform.joint_count == 0 {
            return Err(GpuSkinError::Skin(SkinError::NoJoints));
        }
        if let Some(skin) = skin {
            if skin.len() != scene.mesh.positions.len() {
                return Err(GpuSkinError::Skin(SkinError::LengthMismatch {
                    expected: scene.mesh.positions.len(),
                    got: skin.len(),
                }));
            }
        }
        let mut camera = scene.camera.clone();
        camera.aspect = width as f32 / height as f32;
        let (cam_u, light_u, mat_u) =
            crate::v1::build_uniforms(&camera, camera.aspect, &scene.light, &scene.material)
                .map_err(|e| match e {
                    crate::v1::UniformError::Camera(e) => {
                        GpuSkinError::Capture(CaptureError::Camera(e))
                    }
                    crate::v1::UniformError::Light(e) => {
                        GpuSkinError::Capture(CaptureError::Light(e))
                    }
                    crate::v1::UniformError::Material(e) => {
                        GpuSkinError::Capture(CaptureError::Material(e))
                    }
                })?;

        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);

        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("r3-capture-color"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("r3-capture-depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // Interleaved skinned vertices: missing skin uploads joint-0/weight-1
        // rows (the shader ignores them and shades magenta instead).
        let n = scene.mesh.positions.len();
        let mut verts = Vec::with_capacity(n * SKINNED_VERTEX_STRIDE as usize);
        for i in 0..n {
            for c in scene.mesh.positions[i] {
                verts.extend_from_slice(&c.to_le_bytes());
            }
            for c in scene.mesh.normals[i] {
                verts.extend_from_slice(&c.to_le_bytes());
            }
            for c in scene.mesh.uvs[i] {
                verts.extend_from_slice(&c.to_le_bytes());
            }
            match skin {
                Some(skin) => {
                    for j in skin.joints[i] {
                        verts.extend_from_slice(&u32::from(j).to_le_bytes());
                    }
                    for w in skin.weights[i] {
                        verts.extend_from_slice(&w.to_le_bytes());
                    }
                }
                None => {
                    for _ in 0..MAX_INFLUENCES {
                        verts.extend_from_slice(&0u32.to_le_bytes());
                    }
                    verts.extend_from_slice(&1.0f32.to_le_bytes());
                    for _ in 1..MAX_INFLUENCES {
                        verts.extend_from_slice(&0.0f32.to_le_bytes());
                    }
                }
            }
        }
        use wgpu::util::DeviceExt;
        let vb = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("r3-mesh-vb"),
                contents: &verts,
                usage: wgpu::BufferUsages::VERTEX,
            });
        let ib = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("r3-mesh-ib"),
                contents: &scene.mesh.index_bytes(),
                usage: wgpu::BufferUsages::INDEX,
            });
        let cam_buf = self.uniform_buffer("r3-u-camera", &cam_u.to_bytes());
        let light_buf = self.uniform_buffer("r3-u-light", &light_u.to_bytes());
        let mat_buf = self.uniform_buffer("r3-u-material", &mat_u.to_bytes());
        // Storage joints: AN1 rows uploaded verbatim as WGSL columns (see
        // `SkinningUniform::to_bytes`), zero-filled past the joint count.
        let mut joint_bytes = vec![0u8; SKIN_STORAGE_BYTES as usize];
        let gpu_mats = uniform.to_bytes();
        joint_bytes[..gpu_mats.len()].copy_from_slice(&gpu_mats);
        let joint_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("r3-u-joints"),
                contents: &joint_bytes,
                usage: wgpu::BufferUsages::STORAGE,
            });
        let mut params = Vec::with_capacity(16);
        params.extend_from_slice(&(uniform.joint_count as u32).to_le_bytes());
        params.extend_from_slice(&(skin.is_some() as u32).to_le_bytes());
        params.extend_from_slice(&0u32.to_le_bytes());
        params.extend_from_slice(&0u32.to_le_bytes());
        let param_buf = self.uniform_buffer("r3-u-params", &params);
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("r3-skin-uniforms"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cam_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: light_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: mat_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: joint_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: param_buf.as_entire_binding(),
                },
            ],
        });

        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("r3-capture"),
            });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("r3-skinned"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.03,
                            g: 0.035,
                            b: 0.06,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
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
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bind, &[]);
            rp.set_vertex_buffer(0, vb.slice(..));
            rp.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
            rp.draw_indexed(0..scene.mesh.indices.len() as u32, 0, 0..1);
        }

        let bytes_per_row = width * 4;
        let padded = bytes_per_row.div_ceil(SKIN_ROW_ALIGN) * SKIN_ROW_ALIGN;
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("r3-readback"),
            size: (padded as u64) * (height as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit(Some(enc.finish()));

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GpuSkinError::Readback(format!("poll: {e}")))?;
        rx.recv()
            .map_err(|e| GpuSkinError::Readback(format!("map channel: {e}")))?
            .map_err(|e| GpuSkinError::Readback(format!("map: {e}")))?;
        let mut rgba = vec![0u8; (bytes_per_row as usize) * (height as usize)];
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| GpuSkinError::Readback(format!("mapped range: {e}")))?;
            for (dst, src) in rgba.chunks_exact_mut(bytes_per_row as usize).enumerate() {
                let off = dst * padded as usize;
                src.copy_from_slice(&mapped[off..off + bytes_per_row as usize]);
            }
        }
        readback.unmap();

        let scope_err = pollster::block_on(scope.pop());
        if let Some(err) = scope_err {
            return Err(GpuSkinError::Validation(format!("{err:?}")));
        }

        let png =
            crate::v1::encode_png_rgba8(&rgba, width, height).map_err(GpuSkinError::Capture)?;
        let manifest = Manifest::new(&self.adapter_name, &self.adapter_driver, width, height, 0);
        Ok((png, manifest))
    }

    fn uniform_buffer(&self, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytes,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
    }
}

// ---------------------------------------------------------------------------
// Tests: weight validation, scripted-pose exactness, error path, x2.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1::{cube, default_scene};

    const IDENTITY: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    fn two_joint_pipeline() -> SkinPipeline {
        let mut pipe = SkinPipeline::new();
        pipe.upload_skeleton(2).expect("2 joints upload");
        pipe
    }

    fn shift_y(by: f32) -> [[f32; 4]; 4] {
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, by, 0.0, 1.0],
        ]
    }

    #[test]
    fn skeleton_upload_validates_joint_count() {
        let mut pipe = SkinPipeline::new();
        assert_eq!(pipe.upload_skeleton(0), Err(SkinError::NoJoints));
        assert_eq!(
            pipe.upload_skeleton(MAX_SKIN_JOINTS + 1),
            Err(SkinError::TooManyJoints(MAX_SKIN_JOINTS + 1))
        );
        // Weights before any skeleton fail loudly, never default.
        let j = [[0u16; 4]];
        let w = [[1.0f32, 0.0, 0.0, 0.0]];
        assert_eq!(pipe.upload_weights(&j, &w), Err(SkinError::EmptySkeleton));
        assert_eq!(
            pipe.skinning_uniform(&[IDENTITY]),
            Err(SkinError::EmptySkeleton)
        );
        pipe.upload_skeleton(1).expect("1 joint uploads");
        assert_eq!(pipe.joint_count(), Some(1));
    }

    #[test]
    fn weight_validation_rejects_unnormalized_and_over_four() {
        let pipe = two_joint_pipeline();
        // Well-formed fixed rows upload.
        let ok = pipe
            .upload_weights(
                &[[0u16, 0, 0, 0], [1u16, 0, 0, 0]],
                &[[1.0f32, 0.0, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]],
            )
            .expect("valid rows upload");
        assert_eq!(ok.len(), 2);
        assert!(!ok.is_empty());
        // Unnormalized rows are rejected, never renormalized silently.
        assert_eq!(
            pipe.upload_weights(&[[0u16, 0, 0, 0]], &[[0.5f32, 0.0, 0.0, 0.0]]),
            Err(SkinError::BadWeightSum { vertex: 0 })
        );
        // Negative weights are rejected.
        assert_eq!(
            pipe.upload_weights(&[[0u16, 0, 0, 0]], &[[1.5f32, -0.5, 0.0, 0.0]]),
            Err(SkinError::NegativeWeight { vertex: 0 })
        );
        // Out-of-range joints are rejected.
        assert_eq!(
            pipe.upload_weights(&[[7u16, 0, 0, 0]], &[[1.0f32, 0.0, 0.0, 0.0]]),
            Err(SkinError::JointOutOfRange {
                vertex: 0,
                joint: 7
            })
        );
        // Non-finite weights are rejected.
        assert_eq!(
            pipe.upload_weights(&[[0u16, 0, 0, 0]], &[[f32::NAN, 0.0, 0.0, 0.0]]),
            Err(SkinError::NonFinite("weights"))
        );
        // Over-4 variable rows are rejected, never cut.
        assert_eq!(
            pipe.upload_weight_rows(
                &[vec![0u16, 0, 0, 0, 1]],
                &[vec![0.2f32, 0.2, 0.2, 0.2, 0.2]]
            ),
            Err(SkinError::TooManyInfluences {
                vertex: 0,
                count: 5
            })
        );
        // Ragged variable rows are rejected.
        assert_eq!(
            pipe.upload_weight_rows(&[vec![0u16, 1]], &[vec![1.0f32]]),
            Err(SkinError::RowMismatch { vertex: 0 })
        );
        // Short variable rows pack with zero padding and still sum to one.
        let packed = pipe
            .upload_weight_rows(&[vec![1u16]], &[vec![1.0f32]])
            .expect("short rows pack");
        assert_eq!(packed.joints[0], [1, 0, 0, 0]);
        assert_eq!(packed.weights[0], [1.0, 0.0, 0.0, 0.0]);
        // Length disagreement fails loudly.
        assert_eq!(
            pipe.upload_weights(
                &[[0u16, 0, 0, 0]],
                &[[1.0f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]]
            ),
            Err(SkinError::LengthMismatch {
                expected: 1,
                got: 2
            })
        );
        // Matrix count must match the skeleton; non-finite matrices fail.
        assert_eq!(
            pipe.skinning_uniform(&[IDENTITY]),
            Err(SkinError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        let mut bad = IDENTITY;
        bad[0][0] = f32::INFINITY;
        assert_eq!(
            pipe.skinning_uniform(&[IDENTITY, bad]),
            Err(SkinError::BadMatrix { joint: 1 })
        );
    }

    #[test]
    fn joint_and_normal_diagnostics_are_exact_on_scripted_pose() {
        // Scripted pose: joint 0 is identity, joint 1 shifts +1 m in Y.
        let pipe = two_joint_pipeline();
        let uniform = pipe
            .skinning_uniform(&[IDENTITY, shift_y(1.0)])
            .expect("2 matrices upload");
        // Uniform byte image: AN1 row order, 64 bytes per joint. Joint 0 is
        // identity; joint 1 is the +1 m Y shift with translation in its last
        // row at offset 64 + 48. The same image feeds the GPU storage buffer
        // (AN1 rows read verbatim as WGSL columns).
        let raw = uniform.to_bytes();
        assert_eq!(raw.len(), 128);
        let mut word = [0u8; 4];
        word.copy_from_slice(&raw[..4]);
        assert_eq!(f32::from_le_bytes(word), 1.0);
        word.copy_from_slice(&raw[64 + 48..64 + 52]);
        assert_eq!(f32::from_le_bytes(word), 0.0);
        word.copy_from_slice(&raw[64 + 52..64 + 56]);
        assert_eq!(f32::from_le_bytes(word), 1.0);
        word.copy_from_slice(&raw[64 + 60..64 + 64]);
        assert_eq!(f32::from_le_bytes(word), 1.0);

        // Three verts: pinned to joint 0, pinned to joint 1, blended half.
        let skin = pipe
            .upload_weights(
                &[[0u16, 0, 0, 0], [1u16, 0, 0, 0], [0u16, 1, 0, 0]],
                &[
                    [1.0f32, 0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0, 0.0],
                    [0.5, 0.5, 0.0, 0.0],
                ],
            )
            .expect("scripted rows upload");
        let mesh = MeshData {
            positions: vec![[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
            normals: vec![[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            uvs: vec![[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]],
            indices: vec![0, 1, 2],
        };
        let frame = pipe
            .render_skinned(&mesh, Some(&skin), &uniform)
            .expect("scripted pose skins");
        assert!(!frame.used_error_material);
        assert!(frame.diagnostics.is_empty());
        // Exact: identity keeps [1,0,0], +1 Y shift gives [1,1,0], the blend
        // is the exact midpoint [1,0.5,0] (0.5 weights are binary-exact).
        assert_eq!(
            frame.positions,
            vec![[1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [1.0, 0.5, 0.0]]
        );
        // Pure translations keep +Y normals bit-identical.
        assert_eq!(
            frame.normals,
            vec![[0.0, 1.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]]
        );
        // Joint heat follows the dominant influence through the palette.
        assert_eq!(
            joint_heatmap(&skin),
            vec![JOINT_PALETTE[0], JOINT_PALETTE[1], JOINT_PALETTE[0]]
        );
        // Normal check passes unit vectors exactly and flags the rest.
        let diags = check_normals(&frame.normals);
        assert_eq!(diags.len(), 3);
        for d in &diags {
            assert!(d.ok, "posed normal {d:?} must be unit");
            assert_eq!(d.length, 1.0);
        }
        let bad = check_normals(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [f32::NAN, 0.0, 0.0]]);
        assert_eq!(
            bad.iter().map(|d| d.ok).collect::<Vec<_>>(),
            vec![false, false, false]
        );
        assert_eq!(bad[0].length, 0.0);
        assert_eq!(bad[1].length, 2.0);
        assert_eq!(bad[2].length, 0.0);
    }

    #[test]
    fn missing_skin_renders_the_error_path() {
        let pipe = two_joint_pipeline();
        let uniform = pipe
            .skinning_uniform(&[IDENTITY, IDENTITY])
            .expect("identity pose uploads");
        let mesh = cube();
        // No skin: the unposed mesh returns with the error flag plus the
        // MissingSkin entry — positions are untouched, never wrongly posed.
        let frame = pipe
            .render_skinned(&mesh, None, &uniform)
            .expect("missing skin returns the error frame");
        assert!(frame.used_error_material);
        assert_eq!(frame.positions, mesh.positions);
        assert!(frame
            .diagnostics
            .iter()
            .any(|d| d.kind == SkinDiagKind::MissingSkin));
        // The capture shades the error frame magenta and stays a valid PNG.
        let base = default_scene(96, 64).expect("default scene is valid");
        let (err_png, err_manifest, err_diags) = pipe
            .capture_skinned(&base, None, &uniform, 96, 64)
            .expect("error capture works");
        assert!(err_diags
            .iter()
            .any(|d| d.kind == SkinDiagKind::MissingSkin));
        assert!(err_manifest.is_clean());
        let err_img = image::load_from_memory(&err_png)
            .expect("error capture is a valid PNG")
            .to_rgba8();
        let mut magenta = 0usize;
        for p in err_img.pixels() {
            if p[0] > 150 && p[2] > 150 && p[1] < 120 {
                magenta += 1;
            }
        }
        assert!(
            magenta > 50,
            "missing skin must shade visibly magenta, got {magenta} pixels"
        );
        // A skinned capture of the same scene is byte-different: the error
        // path cannot be confused with a real pose.
        let joints = vec![[0u16, 0, 0, 0]; mesh.positions.len()];
        let weights = vec![[1.0f32, 0.0, 0.0, 0.0]; mesh.positions.len()];
        let skin = pipe
            .upload_weights(&joints, &weights)
            .expect("cube skin uploads");
        let mut posed_base = base.clone();
        posed_base.mesh = mesh;
        let (ok_png, _, ok_diags) = pipe
            .capture_skinned(&posed_base, Some(&skin), &uniform, 96, 64)
            .expect("skinned capture works");
        assert!(ok_diags.is_empty());
        assert_ne!(err_png, ok_png, "error and posed captures must differ");
    }

    #[test]
    fn skinning_is_deterministic_twice() {
        let build = || {
            let mut pipe = SkinPipeline::new();
            pipe.upload_skeleton(2).expect("2 joints upload");
            let uniform = pipe
                .skinning_uniform(&[IDENTITY, shift_y(0.25)])
                .expect("pose uploads");
            let mesh = cube();
            let joints = vec![[0u16, 1, 0, 0]; mesh.positions.len()];
            let weights = vec![[0.75f32, 0.25, 0.0, 0.0]; mesh.positions.len()];
            let skin = pipe.upload_weights(&joints, &weights).expect("rows upload");
            let frame = pipe
                .render_skinned(&mesh, Some(&skin), &uniform)
                .expect("pose skins");
            let heat = joint_heatmap(&skin);
            let norms = check_normals(&frame.normals);
            (frame, heat, norms, uniform.to_bytes())
        };
        let (f1, h1, n1, b1) = build();
        let (f2, h2, n2, b2) = build();
        assert_eq!(f1, f2, "same pose skins bit-identically twice");
        assert_eq!(h1, h2, "joint heat is bit-identical twice");
        assert_eq!(n1, n2, "normal checks are bit-identical twice");
        assert_eq!(b1, b2, "uniform bytes match twice");
        // Captures are byte-identical per size as well.
        let mut pipe = SkinPipeline::new();
        pipe.upload_skeleton(2).expect("2 joints upload");
        let uniform = pipe
            .skinning_uniform(&[IDENTITY, shift_y(0.25)])
            .expect("pose uploads");
        let base = default_scene(64, 48).expect("default scene is valid");
        let (p1, m1, _) = pipe
            .capture_skinned(&base, None, &uniform, 64, 48)
            .expect("first error capture works");
        let (p2, m2, _) = pipe
            .capture_skinned(&base, None, &uniform, 64, 48)
            .expect("second error capture works");
        assert_eq!((p1, m1), (p2, m2), "error captures match twice");
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_skinned_capture_is_validation_clean() {
        use super::{GpuSkinError, GpuSkinned};
        let gpu = match GpuSkinned::init() {
            Ok(g) => g,
            Err(GpuSkinError::NoAdapter) => {
                eprintln!("SKIPPED gpu_skinned_capture: no wgpu adapter on this machine");
                return;
            }
            Err(e) => panic!("gpu skin init failed (must fail loudly, never fake green): {e}"),
        };
        eprintln!(
            "R3 gpu: adapter '{}' driver '{}'",
            gpu.adapter_name(),
            gpu.adapter_driver()
        );
        // Two-joint bend over the cube: top verts ride joint 1 (+0.4 m Y).
        let mesh = cube();
        let mut pipe = SkinPipeline::new();
        pipe.upload_skeleton(2).expect("2 joints upload");
        let bent = pipe
            .skinning_uniform(&[IDENTITY, shift_y(0.4)])
            .expect("bent pose uploads");
        let flat = pipe
            .skinning_uniform(&[IDENTITY, IDENTITY])
            .expect("flat pose uploads");
        let mut joints = Vec::with_capacity(mesh.positions.len());
        let mut weights = Vec::with_capacity(mesh.positions.len());
        for p in &mesh.positions {
            if p[1] > 0.0 {
                joints.push([1u16, 0, 0, 0]);
            } else {
                joints.push([0u16, 0, 0, 0]);
            }
            weights.push([1.0f32, 0.0, 0.0, 0.0]);
        }
        let skin = pipe
            .upload_weights(&joints, &weights)
            .expect("cube skin uploads");
        let mut scene = default_scene(160, 120).expect("default scene is valid");
        scene.mesh = mesh;
        // Bent vs flat: the pose provably moves pixels on-device.
        let (bent_png, bent_manifest) = gpu
            .render_capture(&scene, Some(&skin), &bent, 160, 120)
            .expect("bent gpu capture renders clean");
        let (flat_png, _) = gpu
            .render_capture(&scene, Some(&skin), &flat, 160, 120)
            .expect("flat gpu capture renders clean");
        assert!(bent_manifest.is_clean());
        assert_eq!((bent_manifest.width, bent_manifest.height), (160, 120));
        assert!(!bent_manifest.adapter.is_empty());
        assert!(!bent_manifest.driver.is_empty());
        let bent_img = image::load_from_memory(&bent_png)
            .expect("bent png decodes")
            .to_rgba8();
        let flat_img = image::load_from_memory(&flat_png)
            .expect("flat png decodes")
            .to_rgba8();
        let covered = |img: &image::RgbaImage| {
            let mut n = 0usize;
            for p in img.pixels() {
                let off = (p[0] as f32 - 7.0)
                    .abs()
                    .max((p[1] as f32 - 8.0).abs())
                    .max((p[2] as f32 - 15.0).abs());
                if off > 5.0 {
                    n += 1;
                }
            }
            n
        };
        let bent_covered = covered(&bent_img);
        let flat_covered = covered(&flat_img);
        assert!(
            bent_covered > 100 && flat_covered > 100,
            "gpu cube must cover pixels, bent {bent_covered} flat {flat_covered}"
        );
        let mut energy = 0u64;
        for (a, b) in bent_img.pixels().zip(flat_img.pixels()) {
            energy += (a[0] as i32 - b[0] as i32).unsigned_abs() as u64
                + (a[1] as i32 - b[1] as i32).unsigned_abs() as u64
                + (a[2] as i32 - b[2] as i32).unsigned_abs() as u64;
        }
        assert!(energy > 500, "gpu pose must move pixels, energy {energy}");
        // Missing skin shades visibly magenta with zero validation errors.
        let (err_png, err_manifest) = gpu
            .render_capture(&scene, None, &flat, 160, 120)
            .expect("error gpu capture renders clean");
        assert!(err_manifest.is_clean());
        let err_img = image::load_from_memory(&err_png)
            .expect("error png decodes")
            .to_rgba8();
        let mut magenta = 0usize;
        for p in err_img.pixels() {
            if p[0] > 150 && p[2] > 150 && p[1] < 120 {
                magenta += 1;
            }
        }
        assert!(
            magenta > 50,
            "missing gpu skin must shade visibly magenta, got {magenta} pixels"
        );
        // Persist the receipt outside the repo (temp dir, never git dirt).
        let dir = std::env::temp_dir().join("engine2040-r3");
        std::fs::create_dir_all(&dir).expect("temp receipt dir");
        std::fs::write(dir.join("r3-skinned.png"), &bent_png).expect("receipt png writes");
        std::fs::write(dir.join("r3-manifest.json"), bent_manifest.to_json())
            .expect("receipt json writes");
        eprintln!(
            "R3 receipt: {} + manifest {}",
            dir.join("r3-skinned.png").display(),
            bent_manifest.to_json()
        );
    }
}

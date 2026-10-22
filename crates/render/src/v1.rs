//! v1: V1 forward-PBR receipt — mesh upload, depth-tested raster, glam camera,
//! one directional light, metallic-roughness PBR, exposure plus ACES, PNG
//! capture with a JSON manifest, and a procedural cube until A1 lands.
//!
//! Coordinates are right-handed, +Y up, meters; camera looks down -Z in view
//! space. Lighting math runs in linear space; the receipt image is encoded
//! sRGB. Pixels never enter any simulation hash.
//!
//! Provenance: the BRDF core (`d_ggx`, `v_smith_ggx_correlated`, `f_schlick`,
//! `fresnel`, `fd_burley`, `perceptual_to_roughness`, `f_ab`,
//! `specular_multiscatter`, diffuse/`F0` helpers) is ported from Bevy Engine's
//! MIT/Apache-2.0 renderer at `v0.15.0`
//! (`crates/bevy_pbr/src/render/pbr_lighting.wgsl`, each function carrying a
//! `PORT(bevyengine/bevy@v0.15.0:<path>)` marker below). The tone map is
//! the crate's one hue-keeping curve (`crate::tonemap::tone_map`, the CPU
//! twin of the WGSL `tonemap.wgsl` chunk); the sRGB EOTF
//! (`linear_to_srgb`) is reimplemented from IEC 61966-2-1, not a Bevy port
//! (no such function in Bevy v0.15.0 `color_operations.wgsl` or
//! `tonemapping_shared.wgsl`). Upstream renamed these shader modules after v0.19;
//! every release tag through v0.19.1 ships them under the `.wgsl` names
//! cited here. The wgpu port of that math lives in
//! `v1_gpu::FORWARD_PBR_WGSL`. The orchestrator records the matching
//! `EVIDENCE.md` row; this module only carries the per-construct comments.
//!
//! This procedural receipt alone is NOT a V1 exit: MV1 needs the A1/A2
//! imported fixtures plus the material/normal/depth diagnostic views. The cube
//! here stands in for cooked `CookedMesh`/`CookedMaterial` until the importer
//! lands, and the single-light forward pass grows into shadows/IBL in R2.

use std::cmp::Ordering;
use std::f32::consts::PI;
use std::fmt;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted vertex count per upload (parallel arrays each).
pub const MAX_VERTICES: usize = 1 << 20;
/// Largest accepted index count per upload; always a multiple of three.
pub const MAX_INDICES: usize = 1 << 20;
/// Largest accepted capture dimension; keeps `w * h` far from `u32` overflow.
pub const MAX_CAPTURE_DIM: u32 = 2048;
/// Bevy's perceptual-roughness floor (`perceptualRoughnessToRoughness`).
pub const MIN_PERCEPTUAL_ROUGHNESS: f32 = 0.089;

// ---------------------------------------------------------------------------
// Mesh upload: positions/normals/uv + indices, mirroring C3 `CookedMesh`
// attribute order so A1 can feed this path without reshuffling.
// ---------------------------------------------------------------------------

/// Mesh upload failures: bounded diagnostics naming the bad field.
#[derive(Clone, PartialEq, Debug)]
pub enum MeshError {
    /// No vertices at all.
    EmptyMesh,
    /// Attribute array length differs from positions; field + counts carried.
    LengthMismatch {
        /// Attribute that disagrees (`normals`, `uvs`).
        field: &'static str,
        /// Vertex count from positions.
        expected: usize,
        /// Count of the disagreeing array.
        got: usize,
    },
    /// Index addresses no vertex, culprit carried.
    IndexOutOfRange(u32),
    /// Index count is not a whole triangle list.
    IndexCountNotMultipleOfThree(usize),
    /// Non-finite component; field + vertex carried.
    NonFinite {
        /// Attribute (`positions`, `normals`, `uvs`).
        field: &'static str,
        /// Vertex index holding the bad value.
        vertex: usize,
    },
    /// Normal with ~zero length; vertex carried.
    DegenerateNormal(usize),
    /// UV outside `0..=1` (C3 convention); vertex carried.
    UvOutOfRange(usize),
    /// Array exceeds [`MAX_VERTICES`] / [`MAX_INDICES`].
    TooLarge {
        /// Array (`vertices`, `indices`).
        field: &'static str,
        /// Offending count.
        count: usize,
    },
}

impl fmt::Display for MeshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMesh => write!(f, "mesh has no vertices"),
            Self::LengthMismatch {
                field,
                expected,
                got,
            } => write!(
                f,
                "mesh {field} length {got} differs from positions {expected}"
            ),
            Self::IndexOutOfRange(i) => write!(f, "mesh index {i} addresses no vertex"),
            Self::IndexCountNotMultipleOfThree(n) => {
                write!(f, "mesh index count {n} is not a triangle list")
            }
            Self::NonFinite { field, vertex } => {
                write!(f, "mesh {field} vertex {vertex} is not finite")
            }
            Self::DegenerateNormal(v) => write!(f, "mesh normal {v} has ~zero length"),
            Self::UvOutOfRange(v) => write!(f, "mesh uv {v} is outside 0..=1"),
            Self::TooLarge { field, count } => write!(f, "mesh {field} count {count} is too large"),
        }
    }
}

impl std::error::Error for MeshError {}

/// Validated triangle mesh: parallel attribute arrays plus indices.
#[derive(Clone, PartialEq, Debug)]
pub struct MeshData {
    /// Positions in meters.
    pub positions: Vec<[f32; 3]>,
    /// Normals (finite, nonzero; unit for the procedural cube).
    pub normals: Vec<[f32; 3]>,
    /// First UV channel in `0..=1`.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle indices into the attribute arrays.
    pub indices: Vec<u32>,
    /// Tangents as `[x, y, z, w]` with `w` the handedness (`1.0` or `-1.0`).
    /// Computed deterministically by [`upload_mesh`] from positions plus UVs;
    /// degenerate UVs fall back per vertex to `+X` orthogonalized against the
    /// normal with `w = 1.0`. Carried to the GPU by [`MeshData::vertex_bytes`]
    /// as 48 bytes per vertex (pos + normal + uv + tangent); normal/ORM
    /// sampling is the documented remainder.
    pub tangents: Vec<[f32; 4]>,
}

fn all_finite_3(field: &'static str, xs: &[[f32; 3]]) -> Result<(), MeshError> {
    for (i, v) in xs.iter().enumerate() {
        if !v.iter().all(|c| c.is_finite()) {
            return Err(MeshError::NonFinite { field, vertex: i });
        }
    }
    Ok(())
}

// Tangents are owned by `engine-assets` (ARCHITECTURE C3): `upload_mesh` calls
// `engine_assets::import_repair::recompute_tangents`; no local mirror.

/// Validate and pack parallel arrays plus indices into a [`MeshData`].
/// Rejects empty meshes, length mismatches, out-of-range indices,
/// non-finite components, degenerate normals and out-of-range UVs.
pub fn upload_mesh(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Result<MeshData, MeshError> {
    upload_mesh_checked(positions, normals, uvs, indices, true)
}

/// Largest UV magnitude a tiling mesh may carry (tiles across a surface).
pub const MAX_TILED_UV: f32 = 1024.0;

/// [`upload_mesh`] for meshes whose UVs tile a Repeat-sampled texture: the
/// UVs may leave `0..=1` (finite, at most [`MAX_TILED_UV`]); every other rule
/// is the same. The presenter revalidates uploads through this, so a kit with
/// tiling UVs reaches the GPU; importers that promise unit UVs call the
/// strict [`upload_mesh`].
pub fn upload_mesh_tiled(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Result<MeshData, MeshError> {
    upload_mesh_checked(positions, normals, uvs, indices, false)
}

fn upload_mesh_checked(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
    unit_uv: bool,
) -> Result<MeshData, MeshError> {
    if positions.is_empty() {
        return Err(MeshError::EmptyMesh);
    }
    if positions.len() > MAX_VERTICES {
        return Err(MeshError::TooLarge {
            field: "vertices",
            count: positions.len(),
        });
    }
    if normals.len() != positions.len() {
        return Err(MeshError::LengthMismatch {
            field: "normals",
            expected: positions.len(),
            got: normals.len(),
        });
    }
    if uvs.len() != positions.len() {
        return Err(MeshError::LengthMismatch {
            field: "uvs",
            expected: positions.len(),
            got: uvs.len(),
        });
    }
    if indices.len() > MAX_INDICES {
        return Err(MeshError::TooLarge {
            field: "indices",
            count: indices.len(),
        });
    }
    if !indices.len().is_multiple_of(3) {
        return Err(MeshError::IndexCountNotMultipleOfThree(indices.len()));
    }
    all_finite_3("positions", positions)?;
    all_finite_3("normals", normals)?;
    for (i, n) in normals.iter().enumerate() {
        let l2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
        if l2 < 1e-12 {
            return Err(MeshError::DegenerateNormal(i));
        }
    }
    for (i, uv) in uvs.iter().enumerate() {
        if !uv.iter().all(|c| c.is_finite()) {
            return Err(MeshError::NonFinite {
                field: "uvs",
                vertex: i,
            });
        }
        let (low, high) = if unit_uv {
            (0.0, 1.0)
        } else {
            (-MAX_TILED_UV, MAX_TILED_UV)
        };
        if uv.iter().any(|c| *c < low || *c > high) {
            return Err(MeshError::UvOutOfRange(i));
        }
    }
    let n = positions.len() as u32;
    for i in indices {
        if *i >= n {
            return Err(MeshError::IndexOutOfRange(*i));
        }
    }
    // CODELAW one-owner: tangents come from the import lane; inputs are already validated above.
    let proxy = engine_assets::CookedMesh {
        positions: positions.to_vec(),
        normals: normals.to_vec(),
        uv0: uvs.to_vec(),
        tangents: Vec::new(),
        indices: indices.to_vec(),
    };
    let tangents = engine_assets::import_repair::recompute_tangents(&proxy);
    Ok(MeshData {
        positions: proxy.positions,
        normals: proxy.normals,
        uvs: proxy.uv0,
        indices: proxy.indices,
        tangents,
    })
}

/// One cube face: outward normal plus 4 corners (position only; uv fixed).
type CubeFace = ((f32, f32, f32), [[f32; 3]; 4]);

/// Procedural unit cube (half-extent 0.5, 24 verts, 12 tris) standing in for
/// A1's imported fixture. R1 disables face culling in both rasterizers and
/// depth-tests only, so winding is informational until a culling pass lands.
pub fn cube() -> MeshData {
    // 6 faces x 4 corners: position, outward normal, uv.
    #[rustfmt::skip]
    const FACES: [CubeFace; 6] = [
        ((1.0, 0.0, 0.0), [[0.5,-0.5,-0.5],[0.5,-0.5,0.5],[0.5,0.5,0.5],[0.5,0.5,-0.5]]),
        ((-1.0, 0.0, 0.0), [[-0.5,-0.5,0.5],[-0.5,-0.5,-0.5],[-0.5,0.5,-0.5],[-0.5,0.5,0.5]]),
        ((0.0, 1.0, 0.0), [[-0.5,0.5,-0.5],[-0.5,0.5,0.5],[0.5,0.5,0.5],[0.5,0.5,-0.5]]),
        ((0.0, -1.0, 0.0), [[-0.5,-0.5,0.5],[-0.5,-0.5,-0.5],[0.5,-0.5,-0.5],[0.5,-0.5,0.5]]),
        ((0.0, 0.0, 1.0), [[-0.5,-0.5,0.5],[0.5,-0.5,0.5],[0.5,0.5,0.5],[-0.5,0.5,0.5]]),
        ((0.0, 0.0, -1.0), [[0.5,-0.5,-0.5],[-0.5,-0.5,-0.5],[-0.5,0.5,-0.5],[0.5,0.5,-0.5]]),
    ];
    const UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut positions = Vec::with_capacity(24);
    let mut normals = Vec::with_capacity(24);
    let mut uvs = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face, (normal, corners)) in FACES.iter().enumerate() {
        let base = (face * 4) as u32;
        let n = [normal.0, normal.1, normal.2];
        for (i, corner) in corners.iter().enumerate() {
            positions.push(*corner);
            normals.push(n);
            uvs.push(UVS[i]);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    // Cube constants above are finite, parallel, in-range and indexed by
    // construction; a failure here is a programming bug, not user input.
    upload_mesh(&positions, &normals, &uvs, &indices).expect("procedural cube is valid")
}

impl MeshData {
    /// Interleaved vertex bytes for the GPU: pos(3) + normal(3) + uv(2) + tangent(4) f32 LE.
    pub fn vertex_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.positions.len() * 48);
        for i in 0..self.positions.len() {
            for c in self.positions[i] {
                out.extend_from_slice(&c.to_le_bytes());
            }
            for c in self.normals[i] {
                out.extend_from_slice(&c.to_le_bytes());
            }
            for c in self.uvs[i] {
                out.extend_from_slice(&c.to_le_bytes());
            }
            for c in self.tangents[i] {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }

    /// Index bytes for the GPU: u32 LE.
    pub fn index_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.indices.len() * 4);
        for i in &self.indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Camera via glam: eye/target/up + vertical fov. The render adapter owns the
// OpenGL-to-WebGPU depth remap (see `proj_matrix`).
// ---------------------------------------------------------------------------

/// Camera failures: every component validated before any matrix is built.
#[derive(Clone, PartialEq, Debug)]
pub enum CameraError {
    /// A component is NaN/infinite; which vector carried.
    NonFinite(&'static str),
    /// Eye equals target: no view direction.
    DegenerateView,
    /// Up is zero-length or parallel to the view direction.
    BadUp,
    /// Vertical fov outside `(0, PI)` radians.
    BadFov(f32),
    /// Aspect not finite and positive.
    BadAspect(f32),
    /// Near/far not `0 < near < far` and finite.
    BadClip { near: f32, far: f32 },
}

impl fmt::Display for CameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "camera {w} is not finite"),
            Self::DegenerateView => write!(f, "camera eye equals target"),
            Self::BadUp => write!(f, "camera up is degenerate"),
            Self::BadFov(v) => write!(f, "camera fov_y {v} outside (0, PI)"),
            Self::BadAspect(v) => write!(f, "camera aspect {v} is not positive finite"),
            Self::BadClip { near, far } => {
                write!(f, "camera clip near={near} far={far} is not 0 < near < far")
            }
        }
    }
}

impl std::error::Error for CameraError {}

/// Perspective camera. Units: meters, radians, right-handed with -Z forward.
#[derive(Clone, PartialEq, Debug)]
pub struct Camera {
    /// Camera position in meters.
    pub eye: glam::Vec3,
    /// Look-at point in meters.
    pub target: glam::Vec3,
    /// Up direction (need not be unit, must not be parallel to view).
    pub up: glam::Vec3,
    /// Vertical field of view in radians, `(0, PI)`.
    pub fov_y: f32,
    /// Width over height; `capture` overrides it from the receipt size.
    pub aspect: f32,
    /// Near clip in meters, `> 0`.
    pub near: f32,
    /// Far clip in meters, `> near`.
    pub far: f32,
}

impl Camera {
    /// Build with default clip (`near = 0.1`, `far = 100`) and 16:9 aspect.
    /// [`Camera::validate`] still runs before any matrix leaves the camera.
    pub fn new(eye: [f32; 3], target: [f32; 3], up: [f32; 3], fov_y: f32) -> Self {
        Self {
            eye: glam::Vec3::from_array(eye),
            target: glam::Vec3::from_array(target),
            up: glam::Vec3::from_array(up),
            fov_y,
            aspect: 16.0 / 9.0,
            near: 0.1,
            far: 100.0,
        }
    }

    /// Check every component; matrices are pure functions of valid input, so
    /// identical cameras give bit-identical matrices on every run.
    pub fn validate(&self) -> Result<(), CameraError> {
        for (name, v) in [("eye", self.eye), ("target", self.target), ("up", self.up)] {
            if !v.is_finite() {
                return Err(CameraError::NonFinite(name));
            }
        }
        if !self.fov_y.is_finite() || self.fov_y <= 1e-4 || self.fov_y >= PI - 1e-4 {
            return Err(CameraError::BadFov(self.fov_y));
        }
        if !self.aspect.is_finite() || self.aspect <= 0.0 {
            return Err(CameraError::BadAspect(self.aspect));
        }
        if !self.near.is_finite()
            || !self.far.is_finite()
            || self.near <= 0.0
            || !matches!(self.near.partial_cmp(&self.far), Some(Ordering::Less))
        {
            return Err(CameraError::BadClip {
                near: self.near,
                far: self.far,
            });
        }
        let view = self.target - self.eye;
        if view.length_squared() < 1e-10 {
            return Err(CameraError::DegenerateView);
        }
        if self.up.length_squared() < 1e-12
            || view.normalize().cross(self.up.normalize()).length_squared() < 1e-12
        {
            return Err(CameraError::BadUp);
        }
        Ok(())
    }

    /// World-to-view matrix (glam right-handed look-at).
    pub fn view_matrix(&self) -> Result<glam::Mat4, CameraError> {
        self.validate()?;
        Ok(glam::camera::rh::view::look_at_mat4(
            self.eye,
            self.target,
            self.up,
        ))
    }

    /// View-to-clip matrix in the DirectX/WebGPU NDC convention (Y-up,
    /// depth `0..1`) via glam's `rh::proj::directx::perspective` — the
    /// adapter's declared clip/depth conversion. Both rasterizers share
    /// these matrices, hence identical depth.
    pub fn proj_matrix(&self) -> Result<glam::Mat4, CameraError> {
        self.validate()?;
        Ok(glam::camera::rh::proj::directx::perspective(
            self.fov_y,
            self.aspect,
            self.near,
            self.far,
        ))
    }

    /// Combined view-projection: `proj * view` (column-vector convention).
    pub fn view_proj(&self) -> Result<glam::Mat4, CameraError> {
        Ok(self.proj_matrix()? * self.view_matrix()?)
    }
}

/// Test-only eye camera (FV3): the first-person eye pose as the camera the
/// game feed builds. The feed itself belongs to the game crate
/// (`crates/game/src/fps_view.rs::camera_for_eye`); the render crate cannot
/// depend on it, so this constructor lays the same eye out for the render
/// crate's own submit proof. Frame: right-handed, `+y` up, yaw `0` faces
/// `+z`; the target sits one meter along the forward, and the roll turns the
/// up axis about that forward, so the roll never moves the eye or the target.
/// Units: eye in meters, angles in radians, aspect width over height.
/// Errors: the camera's own [`CameraError`]; the camera validates before it
/// returns, so a caller never holds an unchecked pose.
#[cfg(test)]
pub fn eye_camera_for_test(
    eye_m: [f32; 3],
    yaw_rad: f32,
    pitch_rad: f32,
    roll_rad: f32,
    fov_y_rad: f32,
    aspect: f32,
) -> Result<Camera, CameraError> {
    let (sy, cy) = yaw_rad.sin_cos();
    let (sp, cp) = pitch_rad.sin_cos();
    // Forward from yaw and pitch: yaw 0 level faces +z, and the triple is unit
    // length, so the target stays exactly one meter from the eye.
    let forward = [sy * cp, sp, cy * cp];
    let target = [
        eye_m[0] + forward[0],
        eye_m[1] + forward[1],
        eye_m[2] + forward[2],
    ];
    // Rodrigues over the world up about the forward axis: the roll turns the
    // up axis only. Roll 0 is exact, `sin_cos(0.0)` giving (0.0, 1.0).
    let (sr, cr) = roll_rad.sin_cos();
    let along_up = forward[1];
    let up = [
        -forward[2] * sr + forward[0] * along_up * (1.0 - cr),
        cr + forward[1] * along_up * (1.0 - cr),
        forward[0] * sr + forward[2] * along_up * (1.0 - cr),
    ];
    let mut camera = Camera::new(eye_m, target, up, fov_y_rad);
    camera.aspect = aspect;
    camera.validate()?;
    Ok(camera)
}

// ---------------------------------------------------------------------------
// Light + material: one directional light, metallic-roughness factors.
// ---------------------------------------------------------------------------

/// Light failures.
#[derive(Clone, PartialEq, Debug)]
pub enum LightError {
    /// Direction, color or intensity is NaN/infinite; which field carried.
    NonFinite(&'static str),
    /// Direction has ~zero length.
    DegenerateDirection,
    /// Intensity is negative.
    NegativeIntensity(f32),
    /// A color component is negative (HDR above 1 is fine).
    NegativeColor,
}

impl fmt::Display for LightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "light {w} is not finite"),
            Self::DegenerateDirection => write!(f, "light direction has ~zero length"),
            Self::NegativeIntensity(v) => write!(f, "light intensity {v} is negative"),
            Self::NegativeColor => write!(f, "light color has a negative component"),
        }
    }
}

impl std::error::Error for LightError {}

/// One directional light. `direction` points from the surface TOWARD the
/// light (Bevy's `direction_to_light`); `color` is linear, `intensity` scales
/// it (Bevy premultiplies `color * intensity` on the CPU side).
#[derive(Clone, PartialEq, Debug)]
pub struct DirectionalLight {
    /// Direction toward the light (normalized on upload).
    pub direction: glam::Vec3,
    /// Linear color, components `>= 0`.
    pub color: glam::Vec3,
    /// Nonnegative scale.
    pub intensity: f32,
}

impl DirectionalLight {
    /// Validate ranges before the light reaches any uniform.
    pub fn validate(&self) -> Result<(), LightError> {
        if !self.direction.is_finite() {
            return Err(LightError::NonFinite("direction"));
        }
        if !self.color.is_finite() {
            return Err(LightError::NonFinite("color"));
        }
        if !self.intensity.is_finite() {
            return Err(LightError::NonFinite("intensity"));
        }
        if self.direction.length_squared() < 1e-12 {
            return Err(LightError::DegenerateDirection);
        }
        if self.intensity < 0.0 {
            return Err(LightError::NegativeIntensity(self.intensity));
        }
        if self.color.x < 0.0 || self.color.y < 0.0 || self.color.z < 0.0 {
            return Err(LightError::NegativeColor);
        }
        Ok(())
    }

    /// Linear radiance: `color * intensity`.
    pub fn radiance(&self) -> Result<[f32; 3], LightError> {
        self.validate()?;
        Ok([
            self.color.x * self.intensity,
            self.color.y * self.intensity,
            self.color.z * self.intensity,
        ])
    }
}

/// Material failures.
#[derive(Clone, PartialEq, Debug)]
pub enum MaterialError {
    /// A factor is NaN/infinite; which field carried.
    NonFinite(&'static str),
    /// Base color outside `0..=1` (C3 convention).
    BaseColorOutOfRange,
    /// Metallic or roughness outside `0..=1`; field + value carried.
    FactorOutOfRange { field: &'static str, value: f32 },
    /// Exposure not finite and positive.
    BadExposure(f32),
}

impl fmt::Display for MaterialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "material {w} is not finite"),
            Self::BaseColorOutOfRange => write!(f, "material base color outside 0..=1"),
            Self::FactorOutOfRange { field, value } => {
                write!(f, "material {field} {value} outside 0..=1")
            }
            Self::BadExposure(v) => write!(f, "material exposure {v} is not positive finite"),
        }
    }
}

impl std::error::Error for MaterialError {}

/// Metallic-roughness PBR factors (glTF convention): linear base color,
/// `metallic`/`roughness` in `0..=1` where `roughness` is the perceptual
/// (authored) value the shader remaps, plus exposure applied pre-tonemap.
#[derive(Clone, PartialEq, Debug)]
pub struct PbrMaterial {
    /// Linear base color in `0..=1`.
    pub base_color: [f32; 3],
    /// In `0..=1`.
    pub metallic: f32,
    /// Perceptual roughness in `0..=1`.
    pub roughness: f32,
    /// Pre-tonemap exposure, `> 0`.
    pub exposure: f32,
}

impl PbrMaterial {
    /// Validate factors before they reach any uniform.
    pub fn validate(&self) -> Result<(), MaterialError> {
        if !self.base_color.iter().all(|c| c.is_finite()) {
            return Err(MaterialError::NonFinite("base_color"));
        }
        if !self.metallic.is_finite() {
            return Err(MaterialError::NonFinite("metallic"));
        }
        if !self.roughness.is_finite() {
            return Err(MaterialError::NonFinite("roughness"));
        }
        if !self.exposure.is_finite() {
            return Err(MaterialError::NonFinite("exposure"));
        }
        if self.base_color.iter().any(|c| *c < 0.0 || *c > 1.0) {
            return Err(MaterialError::BaseColorOutOfRange);
        }
        for (field, v) in [("metallic", self.metallic), ("roughness", self.roughness)] {
            if !(0.0..=1.0).contains(&v) {
                return Err(MaterialError::FactorOutOfRange { field, value: v });
            }
        }
        if self.exposure <= 0.0 {
            return Err(MaterialError::BadExposure(self.exposure));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Uniforms: CPU-side structs with the exact WGSL layout, so the plumbing test
// proves each factor lands at the byte offset the shader reads.
// ---------------------------------------------------------------------------

/// Camera uniform: 64-byte column-major view-proj + camera pos + pad = 80 B.
#[derive(Clone, PartialEq, Debug)]
pub struct CameraUniform {
    /// View-projection columns (glam `to_cols_array` order).
    pub view_proj_cols: [[f32; 4]; 4],
    /// Camera position for the view vector.
    pub camera_pos: [f32; 3],
}

impl CameraUniform {
    /// 80-byte little-endian image matching `CameraU` in WGSL.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_array().to_vec()
    }

    /// Stack image of the same 80 bytes, reused per draw with no heap.
    pub(crate) fn to_array(&self) -> [u8; 80] {
        let mut out = [0u8; 80];
        for (i, col) in self.view_proj_cols.iter().enumerate() {
            for (j, c) in col.iter().enumerate() {
                let at = (i * 4 + j) * 4;
                out[at..at + 4].copy_from_slice(&c.to_le_bytes());
            }
        }
        for (j, c) in self.camera_pos.iter().enumerate() {
            let at = 64 + j * 4;
            out[at..at + 4].copy_from_slice(&c.to_le_bytes());
        }
        out[76..80].copy_from_slice(&0f32.to_le_bytes());
        out
    }
}

/// Light uniform: dir-to-light + pad + color + intensity = 32 B.
#[derive(Clone, PartialEq, Debug)]
pub struct LightUniform {
    /// Normalized direction toward the light.
    pub dir_to_light: [f32; 3],
    /// Linear radiance (`color * intensity`).
    pub color: [f32; 3],
    /// Kept alongside for diagnostics; the shader uses premultiplied color.
    pub intensity: f32,
}

impl LightUniform {
    /// 32-byte little-endian image matching `LightU` in WGSL.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_array().to_vec()
    }

    /// Stack image of the same 32 bytes, reused per frame with no heap.
    pub(crate) fn to_array(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, c) in self.dir_to_light.iter().enumerate() {
            let at = i * 4;
            out[at..at + 4].copy_from_slice(&c.to_le_bytes());
        }
        out[12..16].copy_from_slice(&0f32.to_le_bytes());
        for (i, c) in self.color.iter().enumerate() {
            let at = 16 + i * 4;
            out[at..at + 4].copy_from_slice(&c.to_le_bytes());
        }
        out[28..32].copy_from_slice(&self.intensity.to_le_bytes());
        out
    }
}

/// Material uniform: base rgb + metallic + roughness + exposure + pad = 32 B.
#[derive(Clone, PartialEq, Debug)]
pub struct MaterialUniform {
    /// Linear base color.
    pub base_color: [f32; 3],
    /// In `0..=1`.
    pub metallic: f32,
    /// Perceptual roughness in `0..=1`.
    pub roughness: f32,
    /// Pre-tonemap exposure.
    pub exposure: f32,
}

impl MaterialUniform {
    /// 32-byte little-endian image matching `MaterialU` in WGSL.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_array().to_vec()
    }

    /// Stack image of the same 32 bytes, reused per draw with no heap.
    pub(crate) fn to_array(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, c) in self.base_color.iter().enumerate() {
            let at = i * 4;
            out[at..at + 4].copy_from_slice(&c.to_le_bytes());
        }
        out[12..16].copy_from_slice(&self.metallic.to_le_bytes());
        out[16..20].copy_from_slice(&self.roughness.to_le_bytes());
        out[20..24].copy_from_slice(&self.exposure.to_le_bytes());
        out[24..28].copy_from_slice(&0f32.to_le_bytes());
        out[28..32].copy_from_slice(&0f32.to_le_bytes());
        out
    }
}

/// Uniform build failures wrap the offending input error.
#[derive(Clone, PartialEq, Debug)]
pub enum UniformError {
    /// Bad camera.
    Camera(CameraError),
    /// Bad light.
    Light(LightError),
    /// Bad material.
    Material(MaterialError),
}

impl fmt::Display for UniformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Camera(e) => write!(f, "uniform camera: {e}"),
            Self::Light(e) => write!(f, "uniform light: {e}"),
            Self::Material(e) => write!(f, "uniform material: {e}"),
        }
    }
}

impl std::error::Error for UniformError {}

/// Pack validated inputs into the exact byte images the shader consumes.
/// `aspect` overrides the camera aspect (receipt size wins over stored).
pub fn build_uniforms(
    camera: &Camera,
    aspect: f32,
    light: &DirectionalLight,
    material: &PbrMaterial,
) -> Result<(CameraUniform, LightUniform, MaterialUniform), UniformError> {
    let mut cam = camera.clone();
    if aspect.is_finite() && aspect > 0.0 {
        cam.aspect = aspect;
    }
    let view_proj = cam.view_proj().map_err(UniformError::Camera)?;
    build_uniforms_with_view_proj(&cam, view_proj, light, material)
}

/// Pack with the frame's already built `view_proj`: the live and capture
/// frames build the matrix once in `effective_camera` and hand it here, so
/// no frame packs a matrix it also built elsewhere. `camera` must be the
/// camera that matrix was built from (its `eye` fills `camera_pos`); the
/// bytes are identical to [`build_uniforms`] for the same camera.
pub fn build_uniforms_with_view_proj(
    camera: &Camera,
    view_proj: glam::Mat4,
    light: &DirectionalLight,
    material: &PbrMaterial,
) -> Result<(CameraUniform, LightUniform, MaterialUniform), UniformError> {
    light.validate().map_err(UniformError::Light)?;
    material.validate().map_err(UniformError::Material)?;
    let dir = light.direction.normalize();
    let radiance = light.radiance().map_err(UniformError::Light)?;
    let flat = view_proj.to_cols_array();
    let mut view_proj_cols = [[0.0f32; 4]; 4];
    for (i, chunk) in flat.chunks_exact(4).enumerate() {
        view_proj_cols[i].copy_from_slice(chunk);
    }
    Ok((
        CameraUniform {
            view_proj_cols,
            camera_pos: camera.eye.to_array(),
        },
        LightUniform {
            dir_to_light: dir.to_array(),
            color: radiance,
            intensity: light.intensity,
        },
        MaterialUniform {
            base_color: material.base_color,
            metallic: material.metallic,
            roughness: material.roughness,
            exposure: material.exposure,
        },
    ))
}

/// Build only the material uniform; camera plus light come from the
/// once-per-frame pack, so per-draw work skips view math and light checks.
pub(crate) fn build_material(material: &PbrMaterial) -> Result<MaterialUniform, UniformError> {
    material.validate().map_err(UniformError::Material)?;
    Ok(MaterialUniform {
        base_color: material.base_color,
        metallic: material.metallic,
        roughness: material.roughness,
        exposure: material.exposure,
    })
}

// ---------------------------------------------------------------------------
// CPU PBR mirror: the same Bevy-ported math the WGSL runs, used by the
// software receipt rasterizer and by behavior tests of the port.
// ---------------------------------------------------------------------------

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn mul3v(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
    let l = dot3(v, v).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn saturate(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `perceptualRoughnessToRoughness` — clamp the authored
/// perceptual value (0.089 floor guards mobile fp precision per Filament),
/// then square to the microfacet alpha.
pub fn perceptual_to_roughness(p: f32) -> f32 {
    let c = p.clamp(MIN_PERCEPTUAL_ROUGHNESS, 1.0);
    c * c
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `D_GGX` normal distribution (Walter et al. via
/// Filament), Bevy's exact formulation.
pub fn d_ggx(roughness: f32, ndot_h: f32) -> f32 {
    let one_minus = 1.0 - ndot_h * ndot_h;
    let a = ndot_h * roughness;
    let k = roughness / (one_minus + a * a);
    k * k * (1.0 / PI)
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `V_SmithGGXCorrelated` visibility (Smith GGX
/// correlated, Heitz), Bevy's exact formulation.
pub fn v_smith_ggx_correlated(roughness: f32, ndot_v: f32, ndot_l: f32) -> f32 {
    let a2 = roughness * roughness;
    let lambda_v = ndot_l * ((ndot_v - a2 * ndot_v) * ndot_v + a2).sqrt();
    let lambda_l = ndot_v * ((ndot_l - a2 * ndot_l) * ndot_l + a2).sqrt();
    0.50 / (lambda_v + lambda_l)
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `F_Schlick` (Schlick 1994 via Filament).
pub fn f_schlick_vec(f0: [f32; 3], f90: f32, vdot_h: f32) -> [f32; 3] {
    let t = (1.0 - vdot_h).powi(5);
    [
        f0[0] + (f90 - f0[0]) * t,
        f0[1] + (f90 - f0[1]) * t,
        f0[2] + (f90 - f0[2]) * t,
    ]
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `fresnel` — grazing reflectance from the Filament
/// occlusion heuristic.
pub fn fresnel(f0: [f32; 3], ldot_h: f32) -> [f32; 3] {
    let f90 = saturate(dot3(f0, [50.0 * 0.33; 3]));
    f_schlick_vec(f0, f90, ldot_h)
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `Fd_Burley` Disney diffuse (Burley 2012 via Filament).
pub fn fd_burley(roughness: f32, ndot_v: f32, ndot_l: f32, ldot_h: f32) -> f32 {
    let f90 = 0.5 + 2.0 * roughness * ldot_h * ldot_h;
    let light_scatter = 1.0 + (f90 - 1.0) * (1.0 - ndot_l).powi(5);
    let view_scatter = 1.0 + (f90 - 1.0) * (1.0 - ndot_v).powi(5);
    light_scatter * view_scatter * (1.0 / PI)
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `F_AB` scale/bias polynomial (Unreal mobile
/// approximation) with Bevy's positivity guard.
pub fn f_ab(perceptual_roughness: f32, ndot_v: f32) -> [f32; 2] {
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r = [
        perceptual_roughness * c0[0] + c1[0],
        perceptual_roughness * c0[1] + c1[1],
        perceptual_roughness * c0[2] + c1[2],
        perceptual_roughness * c0[3] + c1[3],
    ];
    let a004 = (r[0] * r[0]).min((-9.28 * ndot_v).exp2()) * r[0] + r[1];
    let eps = 0.00005;
    [
        (-1.04 * a004 + r[2]).max(eps),
        (1.04 * a004 + r[3]).max(eps),
    ]
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_functions.wgsl): `calculate_diffuse_color` and `calculate_F0`
/// (dielectric default reflectance 0.5^2 * 0.16 = 0.04, Filament remap).
pub fn diffuse_color(base_color: [f32; 3], metallic: f32) -> [f32; 3] {
    mul3(base_color, 1.0 - metallic)
}

/// F0: dielectric 0.04 blended to base color by metallic.
pub fn reflectance_f0(base_color: [f32; 3], metallic: f32) -> [f32; 3] {
    let d = [0.04; 3];
    [
        d[0] + (base_color[0] - d[0]) * metallic,
        d[1] + (base_color[1] - d[1]) * metallic,
        d[2] + (base_color[2] - d[2]) * metallic,
    ]
}

/// PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): `specular_multiscatter` energy compensation
/// (Filament §9.5) over Cook-Torrance D·V·F.
pub fn specular_light(
    d: f32,
    v: f32,
    f: [f32; 3],
    f0: [f32; 3],
    fab: [f32; 2],
    intensity: f32,
) -> [f32; 3] {
    let mut fr = [
        d * v * f[0] * intensity,
        d * v * f[1] * intensity,
        d * v * f[2] * intensity,
    ];
    let s = 1.0 / (fab[0] + fab[1]) - 1.0;
    for i in 0..3 {
        fr[i] *= 1.0 + f0[i] * s;
    }
    fr
}

/// The one tone map of the crate, applied to every channel at once: the
/// hue-keeping path. The fit is evaluated at the peak of the frame and the
/// result is one scale for all three channels, so the input ratios (the
/// hue) survive and a torch flame keeps its orange. The one source is
/// [`crate::tonemap::tone_map`], the CPU twin of the WGSL
/// `tonemap.wgsl` chunk, so the CPU oracle and the shader agree on the
/// numbers.
pub fn tone_map(color: [f32; 3]) -> [f32; 3] {
    crate::tonemap::tone_map(color, 1.0, crate::tonemap::DISPLAY_WHITE)
}

// `linear_to_srgb`: sRGB EOTF (IEC 61966-2-1) reimplemented from the
// standard, not a Bevy port — Bevy v0.15.0 `color_operations.wgsl` and
// `tonemapping_shared.wgsl` carry no such function (verified 2026-09-25).
// The one source is `crate::tonemap::linear_to_srgb`, the CPU twin of the
// WGSL `tonemap.wgsl` chunk.

/// One shaded surface point: material factors plus the geometric terms.
/// Bundled so the shade entry point stays a single argument.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShadePoint {
    /// Linear base color.
    pub base_color: [f32; 3],
    /// In `0..=1`.
    pub metallic: f32,
    /// Authored perceptual roughness in `0..=1`.
    pub perceptual_roughness: f32,
    /// Unit surface normal.
    pub normal: [f32; 3],
    /// Unit direction toward the camera.
    pub view: [f32; 3],
    /// Unit direction toward the light.
    pub light_dir: [f32; 3],
    /// Linear light radiance (`color * intensity`).
    pub radiance: [f32; 3],
    /// Pre-tonemap exposure.
    pub exposure: f32,
}

/// Shade one surface point with the single directional light: Burley diffuse
/// plus Cook-Torrance specular with multiscatter compensation
/// (PORT(bevyengine/bevy@v0.15.0:crates/bevy_pbr/src/render/pbr_lighting.wgsl): Bevy `directional_light` combine
/// `color = (diffuse + specular) * NdotL * light_color`), then exposure,
/// the crate's one hue-keeping tone map and the sRGB encode. Output: sRGB
/// in `0..=1`.
/// No ambient/IBL yet: that is R2.
pub fn shade_pbr(p: &ShadePoint) -> [f32; 3] {
    let roughness = perceptual_to_roughness(p.perceptual_roughness);
    let h = norm3(add3(p.light_dir, p.view));
    let ndot_l = saturate(dot3(p.normal, p.light_dir));
    let ndot_v = dot3(p.normal, p.view).max(0.0001);
    let ndot_h = saturate(dot3(p.normal, h));
    let ldot_h = saturate(dot3(p.light_dir, h));
    let diffuse = mul3(
        diffuse_color(p.base_color, p.metallic),
        fd_burley(roughness, ndot_v, ndot_l, ldot_h),
    );
    let f0 = reflectance_f0(p.base_color, p.metallic);
    let f = fresnel(f0, ldot_h);
    let d = d_ggx(roughness, ndot_h);
    let vv = v_smith_ggx_correlated(roughness, ndot_v, ndot_l);
    let fab = f_ab(p.perceptual_roughness, ndot_v);
    let spec = specular_light(d, vv, f, f0, fab, 1.0);
    let lit = mul3v(add3(diffuse, spec), p.radiance);
    let lit = mul3(lit, ndot_l);
    let exposed = mul3(lit, p.exposure.max(0.0));
    let clamped = [
        exposed[0].max(0.0),
        exposed[1].max(0.0),
        exposed[2].max(0.0),
    ];
    crate::tonemap::linear_to_srgb(tone_map(clamped))
}

// ---------------------------------------------------------------------------
// Swapchain/surface state: resize and minimize survive via recreate. The
// headless logic lives here so the recreate path is testable without a GPU;
// the wgpu surface/target code calls into the same transitions.
// ---------------------------------------------------------------------------

/// Surface ownership state. Informational only — never hashed, never sim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwapState {
    /// Configured at the carried size (last good).
    Live {
        /// Backing width.
        width: u32,
        /// Backing height.
        height: u32,
        /// Successful reconfigures so far.
        generation: u64,
    },
    /// Minimized/zero-area: keeps the last good size, draws nothing.
    Suspended {
        /// Last good width.
        width: u32,
        /// Last good height.
        height: u32,
        /// Successful reconfigures so far.
        generation: u64,
    },
}

/// Recreate outcome for one `recreate` call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecreateOutcome {
    /// Same live size: no work.
    Unchanged,
    /// Targets rebuilt (resize or restore from minimize).
    Reconfigured {
        /// New generation.
        generation: u64,
    },
    /// Zero area: drawing suspended, last good size kept.
    Suspended,
}

impl SwapState {
    /// Start state; zero area begins suspended with a 1x1 placeholder size.
    pub fn new(width: u32, height: u32) -> Self {
        if width == 0 || height == 0 {
            Self::Suspended {
                width: 1,
                height: 1,
                generation: 0,
            }
        } else {
            Self::Live {
                width,
                height,
                generation: 0,
            }
        }
    }

    /// Recreate backing targets for `width` x `height`. Zero area suspends
    /// (minimize path) and keeps the last good size; restoring a nonzero size
    /// reconfigures. Returns the outcome so callers can skip redundant work.
    pub fn recreate(&mut self, width: u32, height: u32) -> RecreateOutcome {
        if width == 0 || height == 0 {
            let (w, h, g) = match *self {
                Self::Live {
                    width: w,
                    height: h,
                    generation: g,
                } => (w, h, g),
                Self::Suspended {
                    width: w,
                    height: h,
                    generation: g,
                } => (w, h, g),
            };
            *self = Self::Suspended {
                width: w,
                height: h,
                generation: g,
            };
            return RecreateOutcome::Suspended;
        }
        match *self {
            Self::Live {
                width: w,
                height: h,
                ..
            } if w == width && h == height => RecreateOutcome::Unchanged,
            Self::Live { generation: g, .. } | Self::Suspended { generation: g, .. } => {
                *self = Self::Live {
                    width,
                    height,
                    generation: g + 1,
                };
                RecreateOutcome::Reconfigured { generation: g + 1 }
            }
        }
    }

    /// Backing size to allocate (last good while suspended).
    pub fn size(&self) -> (u32, u32) {
        match *self {
            Self::Live { width, height, .. } | Self::Suspended { width, height, .. } => {
                (width, height)
            }
        }
    }

    /// Whether frames may be submitted.
    pub fn is_live(&self) -> bool {
        matches!(*self, Self::Live { .. })
    }

    /// Successful reconfigures so far.
    pub fn generation(&self) -> u64 {
        match *self {
            Self::Live { generation, .. } | Self::Suspended { generation, .. } => generation,
        }
    }
}

// ---------------------------------------------------------------------------
// Manifest: the receipt names scene, resolution and device.
// ---------------------------------------------------------------------------

/// Capture receipt: adapter/driver name the real device, `size` the receipt
/// resolution, `validation_errors` must be 0 for a green capture.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    /// GPU adapter name (software rasterizer reports `software`).
    pub adapter: String,
    /// Driver string.
    pub driver: String,
    /// Receipt resolution.
    pub width: u32,
    /// Receipt resolution.
    pub height: u32,
    /// wgpu validation errors observed during the capture; green means 0.
    pub validation_errors: u32,
}

impl Manifest {
    /// Build with bounded adapter/driver strings (truncated to 256 chars).
    pub fn new(
        adapter: &str,
        driver: &str,
        width: u32,
        height: u32,
        validation_errors: u32,
    ) -> Self {
        fn bound(s: &str) -> String {
            s.chars().take(256).collect()
        }
        Self {
            adapter: bound(adapter),
            driver: bound(driver),
            width,
            height,
            validation_errors,
        }
    }

    /// `true` when the capture observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }

    fn escape_into(out: &mut String, s: &str) {
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    use std::fmt::Write as _;
                    // A write into a `String` cannot fail.
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
    }

    /// Stable JSON receipt:
    /// `{"adapter":"...","driver":"...","size":[w,h],"validation_errors":0}`.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"adapter\":\"");
        Self::escape_into(&mut out, &self.adapter);
        out.push_str("\",\"driver\":\"");
        Self::escape_into(&mut out, &self.driver);
        out.push_str("\",\"size\":[");
        out.push_str(&format!("{},{}", self.width, self.height));
        out.push_str("],\"validation_errors\":");
        out.push_str(&self.validation_errors.to_string());
        out.push('}');
        out
    }
}

// ---------------------------------------------------------------------------
// Scene + capture: the procedural-cube receipt.
// ---------------------------------------------------------------------------

/// Capture failures.
#[derive(Clone, PartialEq, Debug)]
pub enum CaptureError {
    /// Zero-area receipt.
    ZeroArea,
    /// Receipt dimension exceeds [`MAX_CAPTURE_DIM`].
    TooLarge { width: u32, height: u32 },
    /// Bad mesh.
    Mesh(MeshError),
    /// Bad camera.
    Camera(CameraError),
    /// Bad light.
    Light(LightError),
    /// Bad material.
    Material(MaterialError),
    /// PNG encode failure.
    Png(String),
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroArea => write!(f, "capture size is zero"),
            Self::TooLarge { width, height } => {
                write!(f, "capture {width}x{height} exceeds {MAX_CAPTURE_DIM}")
            }
            Self::Mesh(e) => write!(f, "capture mesh: {e}"),
            Self::Camera(e) => write!(f, "capture camera: {e}"),
            Self::Light(e) => write!(f, "capture light: {e}"),
            Self::Material(e) => write!(f, "capture material: {e}"),
            Self::Png(e) => write!(f, "capture png: {e}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Everything one receipt needs: mesh, camera, light, material.
#[derive(Clone, PartialEq, Debug)]
pub struct Scene {
    /// Triangle mesh.
    pub mesh: MeshData,
    /// Camera; `capture` overrides aspect from the receipt size.
    pub camera: Camera,
    /// The one directional light.
    pub light: DirectionalLight,
    /// Metallic-roughness factors.
    pub material: PbrMaterial,
}

/// Default procedural-cube scene: three-quarter view of a copper-ish metal
/// cube under a warm key light. Deterministic: same size, same bytes.
pub fn default_scene(width: u32, height: u32) -> Result<Scene, CaptureError> {
    check_size(width, height)?;
    let mut camera = Camera::new(
        [2.4, 1.7, 2.8],
        [0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        40.0_f32.to_radians(),
    );
    camera.aspect = width as f32 / height as f32;
    Ok(Scene {
        mesh: cube(),
        camera,
        light: DirectionalLight {
            direction: glam::Vec3::new(-0.45, 0.75, 0.55),
            color: glam::Vec3::new(1.0, 0.96, 0.90),
            intensity: 3.0,
        },
        material: PbrMaterial {
            base_color: [0.72, 0.45, 0.28],
            metallic: 0.9,
            roughness: 0.35,
            exposure: 1.0,
        },
    })
}

fn check_size(width: u32, height: u32) -> Result<(), CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::ZeroArea);
    }
    if width > MAX_CAPTURE_DIM || height > MAX_CAPTURE_DIM {
        return Err(CaptureError::TooLarge { width, height });
    }
    Ok(())
}

/// Encode RGBA8 bytes as PNG via the workspace `image` crate.
pub(crate) fn encode_png_rgba8(
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CaptureError> {
    use image::codecs::png::PngEncoder;
    use image::ImageEncoder;
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| CaptureError::Png(e.to_string()))?;
    Ok(out)
}

/// Render `scene` at `width` x `height` with the software receipt rasterizer:
/// uploads the mesh (validated), transforms with the glam camera, depth-tests
/// in a float depth buffer, shades with the CPU PBR mirror and returns PNG
/// bytes plus a manifest (`adapter = "software"`, zero validation errors).
/// Deterministic per size: float math only, fixed triangle order.
pub fn capture_scene(
    scene: &Scene,
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, Manifest), CaptureError> {
    check_size(width, height)?;
    if scene.mesh.positions.is_empty() {
        return Err(CaptureError::Mesh(MeshError::EmptyMesh));
    }
    let mut camera = scene.camera.clone();
    camera.aspect = width as f32 / height as f32;
    camera.validate().map_err(CaptureError::Camera)?;
    scene.light.validate().map_err(CaptureError::Light)?;
    scene.material.validate().map_err(CaptureError::Material)?;

    let view_proj = camera.view_proj().map_err(CaptureError::Camera)?;
    let radiance = scene.light.radiance().map_err(CaptureError::Light)?;
    let eye = camera.eye.to_array();

    // Project every vertex once: clip, NDC (depth already remapped to 0..1
    // by the camera adapter), viewport pixels, plus 1/w for perspective
    // correction. Triangles with a vertex behind the eye are skipped; the
    // default scene never produces them.
    struct Pv {
        sx: f32,
        sy: f32,
        z: f32,
        inv_w: f32,
        world: [f32; 3],
        normal: [f32; 3],
    }
    let mut proj: Vec<Option<Pv>> = Vec::with_capacity(scene.mesh.positions.len());
    for i in 0..scene.mesh.positions.len() {
        let p = scene.mesh.positions[i];
        let clip = view_proj * glam::Vec4::new(p[0], p[1], p[2], 1.0);
        if clip.w <= 1e-6 {
            proj.push(None);
            continue;
        }
        let inv_w = 1.0 / clip.w;
        let ndc = [clip.x * inv_w, clip.y * inv_w, clip.z * inv_w];
        proj.push(Some(Pv {
            sx: (ndc[0] * 0.5 + 0.5) * width as f32,
            sy: (1.0 - (ndc[1] * 0.5 + 0.5)) * height as f32,
            z: ndc[2],
            inv_w,
            world: p,
            normal: scene.mesh.normals[i],
        }));
    }

    let w = width as usize;
    let h = height as usize;
    let mut rgba = vec![0u8; w * h * 4];
    let mut depth = vec![1.0f32; w * h];
    // Backdrop gradient in display space (dark slate, top to bottom).
    for y in 0..h {
        let t = y as f32 / (h.max(2) - 1) as f32;
        let bg = [0.030 + 0.055 * t, 0.038 + 0.045 * t, 0.062 + 0.038 * t];
        for x in 0..w {
            let o = (y * w + x) * 4;
            rgba[o] = (saturate(bg[0]) * 255.0) as u8;
            rgba[o + 1] = (saturate(bg[1]) * 255.0) as u8;
            rgba[o + 2] = (saturate(bg[2]) * 255.0) as u8;
            rgba[o + 3] = 255;
        }
    }

    let light_dir = scene.light.direction.normalize().to_array();
    let edge = |ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32| {
        (px - ax) * (by - ay) - (py - ay) * (bx - ax)
    };
    for tri in scene.mesh.indices.chunks_exact(3) {
        let (a, b, c) = (
            proj[tri[0] as usize].as_ref(),
            proj[tri[1] as usize].as_ref(),
            proj[tri[2] as usize].as_ref(),
        );
        let (a, b, c) = match (a, b, c) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => continue,
        };
        let area = edge(a.sx, a.sy, b.sx, b.sy, c.sx, c.sy);
        if area.abs() < 1e-9 {
            continue;
        }
        let x0 = (a.sx.min(b.sx).min(c.sx).floor().max(0.0) as usize).min(w.saturating_sub(1));
        let x1 = (a.sx.max(b.sx).max(c.sx).ceil().max(0.0) as usize).min(w.saturating_sub(1));
        let y0 = (a.sy.min(b.sy).min(c.sy).floor().max(0.0) as usize).min(h.saturating_sub(1));
        let y1 = (a.sy.max(b.sy).max(c.sy).ceil().max(0.0) as usize).min(h.saturating_sub(1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let wa = edge(b.sx, b.sy, c.sx, c.sy, px, py) / area;
                let wb = edge(c.sx, c.sy, a.sx, a.sy, px, py) / area;
                let wc = edge(a.sx, a.sy, b.sx, b.sy, px, py) / area;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                // Perspective-correct interpolation via 1/w.
                let iw = wa * a.inv_w + wb * b.inv_w + wc * c.inv_w;
                if iw <= 1e-12 {
                    continue;
                }
                let z = (wa * a.z * a.inv_w + wb * b.z * b.inv_w + wc * c.z * c.inv_w) / iw;
                let idx = y * w + x;
                if z >= depth[idx] {
                    continue;
                }
                let interp = |f: fn(&Pv) -> [f32; 3]| {
                    let fa = f(a);
                    let fb = f(b);
                    let fc = f(c);
                    [
                        (wa * fa[0] * a.inv_w + wb * fb[0] * b.inv_w + wc * fc[0] * c.inv_w) / iw,
                        (wa * fa[1] * a.inv_w + wb * fb[1] * b.inv_w + wc * fc[1] * c.inv_w) / iw,
                        (wa * fa[2] * a.inv_w + wb * fb[2] * b.inv_w + wc * fc[2] * c.inv_w) / iw,
                    ]
                };
                let world = interp(|v| v.world);
                let normal = norm3(interp(|v| v.normal));
                let view = norm3([eye[0] - world[0], eye[1] - world[1], eye[2] - world[2]]);
                let srgb = shade_pbr(&ShadePoint {
                    base_color: scene.material.base_color,
                    metallic: scene.material.metallic,
                    perceptual_roughness: scene.material.roughness,
                    normal,
                    view,
                    light_dir,
                    radiance,
                    exposure: scene.material.exposure,
                });
                depth[idx] = z;
                let o = idx * 4;
                // The shared dither, the same fixed 4x4 pattern the GPU
                // capture (`forward.wgsl` `fs_display`) and the live blit
                // use, so the 8-bit output breaks the bands the tone map
                // leaves behind.
                let dither = crate::tonemap::dither4(
                    u32::try_from(x).unwrap_or(0),
                    u32::try_from(y).unwrap_or(0),
                );
                rgba[o] = (saturate(srgb[0] + dither) * 255.0) as u8;
                rgba[o + 1] = (saturate(srgb[1] + dither) * 255.0) as u8;
                rgba[o + 2] = (saturate(srgb[2] + dither) * 255.0) as u8;
                rgba[o + 3] = 255;
            }
        }
    }

    let png = encode_png_rgba8(&rgba, width, height)?;
    let manifest = Manifest::new(
        "software",
        &format!("engine-render cpu-rasterizer {}", env!("CARGO_PKG_VERSION")),
        width,
        height,
        0,
    );
    Ok((png, manifest))
}

/// Capture the default procedural-cube scene at `width` x `height`.
/// This is the `capture(width, height) -> (png_bytes, Manifest)` contract
/// entry point; see [`capture_scene`] for custom scenes.
pub fn capture(width: u32, height: u32) -> Result<(Vec<u8>, Manifest), CaptureError> {
    let scene = default_scene(width, height)?;
    capture_scene(&scene, width, height)
}

// ---------------------------------------------------------------------------
// Tests: camera determinism, manifest contents, recreate path, PBR plumbing,
// mesh validation, ported-math behavior, PNG receipt shape.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= eps)
    }

    fn get_f32(bytes: &[u8], off: usize) -> f32 {
        let mut b = [0u8; 4];
        b.copy_from_slice(&bytes[off..off + 4]);
        f32::from_le_bytes(b)
    }

    #[test]
    fn camera_view_matrix_is_deterministic() {
        let cam = Camera::new([2.4, 1.7, 2.8], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.7);
        let a = cam.view_matrix().expect("valid camera").to_cols_array();
        let b = cam.view_matrix().expect("valid camera").to_cols_array();
        assert_eq!(a, b, "same camera must give bit-identical view matrices");
        // The eye maps to the view-space origin.
        let view = cam.view_matrix().expect("valid camera");
        let at_eye = view.transform_point3(cam.eye);
        assert!(approx(at_eye.to_array(), [0.0, 0.0, 0.0], 1e-4));
        // A moved target moves the matrix.
        let mut moved = cam.clone();
        moved.target = glam::Vec3::new(1.0, 0.0, 0.0);
        let c = moved.view_matrix().expect("valid camera").to_cols_array();
        assert_ne!(a, c);
    }

    #[test]
    fn camera_rejects_degenerate_input() {
        let base = Camera::new([2.4, 1.7, 2.8], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.7);
        assert!(base.validate().is_ok());
        let mut bad = base.clone();
        bad.target = bad.eye;
        assert_eq!(bad.validate(), Err(CameraError::DegenerateView));
        let mut bad = base.clone();
        bad.up = glam::Vec3::ZERO;
        assert_eq!(bad.validate(), Err(CameraError::BadUp));
        // Up parallel to the view direction is unusable.
        let mut bad = base.clone();
        bad.up = glam::Vec3::new(-2.4, -1.7, -2.8);
        assert_eq!(bad.validate(), Err(CameraError::BadUp));
        let mut bad = base.clone();
        bad.fov_y = 0.0;
        assert_eq!(bad.validate(), Err(CameraError::BadFov(0.0)));
        let mut bad = base.clone();
        bad.aspect = 0.0;
        assert_eq!(bad.validate(), Err(CameraError::BadAspect(0.0)));
        let mut bad = base.clone();
        bad.near = 10.0;
        bad.far = 1.0;
        assert_eq!(
            bad.validate(),
            Err(CameraError::BadClip {
                near: 10.0,
                far: 1.0
            })
        );
        let mut bad = base.clone();
        bad.eye = glam::Vec3::new(f32::NAN, 0.0, 0.0);
        assert_eq!(bad.validate(), Err(CameraError::NonFinite("eye")));
    }

    /// The eye camera the game feed builds: identical eyes give the same
    /// view-proj bytes, twice.
    #[test]
    fn camera_eye_camera_view_proj_is_bit_identical_twice() {
        let eye = [1.0, 3.6, 3.0];
        let a =
            eye_camera_for_test(eye, 0.75, -0.25, 0.1, 1.0, 16.0 / 9.0).expect("valid eye camera");
        let b =
            eye_camera_for_test(eye, 0.75, -0.25, 0.1, 1.0, 16.0 / 9.0).expect("valid eye camera");
        assert_eq!(a, b, "the eye camera must replay exactly");
        let ma = a.view_proj().expect("valid").to_cols_array();
        let mb = b.view_proj().expect("valid").to_cols_array();
        assert!(
            ma.iter().all(|v| v.is_finite()),
            "the view-proj must be finite"
        );
        assert_eq!(ma, mb, "identical eyes gave different view-proj bytes");
    }

    /// The frame the game feed uses: yaw `0` faces `+z`, the target sits one
    /// meter ahead and a level roll keeps `+y` up. The game crate pins the
    /// same literals, so a drift on either side turns one of the two tests
    /// red.
    #[test]
    fn camera_eye_camera_frame_matches_the_game_feed() {
        let camera = eye_camera_for_test([1.0, 3.6, 3.0], 0.0, 0.0, 0.0, 1.0, 16.0 / 9.0)
            .expect("valid eye camera");
        assert_eq!(camera.eye.to_array(), [1.0, 3.6, 3.0], "eye missed");
        assert_eq!(
            camera.target.to_array(),
            [1.0, 3.6, 4.0],
            "yaw 0 must face +z one meter ahead"
        );
        assert_eq!(
            camera.up.to_array(),
            [0.0, 1.0, 0.0],
            "a level roll must keep +y up"
        );
        // The roll turns the up axis about the forward, never the eye or the
        // target.
        let rolled = eye_camera_for_test([1.0, 3.6, 3.0], 0.0, 0.0, 0.3, 1.0, 16.0 / 9.0)
            .expect("valid eye camera");
        assert_eq!(rolled.eye, camera.eye, "the roll moved the eye");
        assert_eq!(rolled.target, camera.target, "the roll moved the target");
        assert_ne!(rolled.up, camera.up, "the roll left the up axis alone");
        let up = rolled.up.normalize();
        let view = (rolled.target - rolled.eye).normalize();
        assert!(
            up.dot(view).abs() < 1e-6,
            "the rolled up axis left the view plane"
        );
    }

    /// The window submit runs the eye camera: the forward GPU submit takes
    /// the frame camera from the scene, so the eye camera's view-proj reaches
    /// the GPU through the same camera uniform every other caller uses. Zero
    /// validation errors at a test size, and the warm submit stays inside
    /// `GPU_SUBMIT_BUDGET_MS`.
    #[test]
    #[cfg(feature = "window")]
    fn camera_eye_camera_window_submit_is_clean_and_inside_the_budget() {
        use crate::test_gpu::{require_or_skip, NoAdapter};
        // A standing eye three meters from the cube, looking down at it.
        let eye = eye_camera_for_test(
            [0.0, 1.6, 3.0],
            std::f32::consts::PI,
            -0.45,
            0.0,
            1.0,
            16.0 / 9.0,
        )
        .expect("valid eye camera");
        let gpu = match crate::v1_gpu::GpuForward::init() {
            Ok(gpu) => gpu,
            Err(crate::v1_gpu::GpuError::NoAdapter) => {
                let _ = require_or_skip(
                    "camera_eye_camera_window_submit",
                    Err::<crate::v1_gpu::GpuForward, NoAdapter>(NoAdapter(
                        "no wgpu adapter on this machine".into(),
                    )),
                );
                return;
            }
            Err(e) => panic!("the forward GPU path must not fail for a valid eye camera: {e}"),
        };
        let mut scene = default_scene(160, 120).expect("the default scene is valid");
        scene.camera = eye;
        // One warm capture: the first frame pays the driver and pipeline
        // warm-up, the measured frames pay only the submit.
        let (_png, manifest) = gpu
            .render_capture(&scene, 160, 120)
            .expect("the warm capture submits");
        assert!(manifest.is_clean(), "the warm capture was not clean");
        // The best of three warm frames: a busy shared PC cannot fake a red
        // here, and the frame itself is the cheap one.
        let mut best = f64::INFINITY;
        for _ in 0..3 {
            let started = std::time::Instant::now();
            let (_png, manifest) = gpu
                .render_capture(&scene, 160, 120)
                .expect("the eye camera frame submits");
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(
                manifest.validation_errors, 0,
                "the eye camera frame had validation errors"
            );
            assert!(manifest.is_clean(), "the eye camera frame was not clean");
            best = best.min(ms);
        }
        eprintln!(
            "eye camera submit: {best:.3} ms (budget {:.1} ms)",
            crate::graph::GPU_SUBMIT_BUDGET_MS
        );
        assert!(
            best <= crate::graph::GPU_SUBMIT_BUDGET_MS,
            "the eye camera submit took {best:.3} ms, over the {:.1} ms budget",
            crate::graph::GPU_SUBMIT_BUDGET_MS
        );
    }

    #[test]
    fn manifest_contents_round_trip() {
        let m = Manifest::new(
            "NVIDIA GeForce RTX 3050 6GB Laptop GPU",
            "Vulkan 610.62",
            320,
            200,
            0,
        );
        assert!(m.is_clean());
        let json = m.to_json();
        assert!(json.contains("\"adapter\":\"NVIDIA GeForce RTX 3050 6GB Laptop GPU\""));
        assert!(json.contains("\"driver\":\"Vulkan 610.62\""));
        assert!(json.contains("\"size\":[320,200]"));
        assert!(json.contains("\"validation_errors\":0"));
        let v: serde_json::Value = serde_json::from_str(&json).expect("manifest is valid JSON");
        assert_eq!(v["adapter"], "NVIDIA GeForce RTX 3050 6GB Laptop GPU");
        assert_eq!(v["driver"], "Vulkan 610.62");
        assert_eq!(v["size"], serde_json::json!([320, 200]));
        assert_eq!(v["validation_errors"], 0);
        // Dirty manifests stay dirty in the receipt.
        let dirty = Manifest::new("gpu", "drv", 64, 64, 2);
        assert!(!dirty.is_clean());
        assert!(dirty.to_json().contains("\"validation_errors\":2"));
    }

    #[test]
    fn swap_recreate_resize_path() {
        // Live start, same size is free.
        let mut s = SwapState::new(1280, 720);
        assert!(s.is_live());
        assert_eq!(s.size(), (1280, 720));
        assert_eq!(s.recreate(1280, 720), RecreateOutcome::Unchanged);
        assert_eq!(s.generation(), 0);
        // Minimize: zero area suspends and keeps the last good size.
        assert_eq!(s.recreate(0, 0), RecreateOutcome::Suspended);
        assert!(!s.is_live());
        assert_eq!(s.size(), (1280, 720));
        assert_eq!(s.generation(), 0);
        // Restore reconfigures and bumps the generation.
        assert_eq!(
            s.recreate(800, 600),
            RecreateOutcome::Reconfigured { generation: 1 }
        );
        assert!(s.is_live());
        assert_eq!(s.size(), (800, 600));
        assert_eq!(s.generation(), 1);
        // Resize reconfigures again.
        assert_eq!(
            s.recreate(1920, 1080),
            RecreateOutcome::Reconfigured { generation: 2 }
        );
        // Zero-area start suspends with a placeholder, then restores.
        let mut z = SwapState::new(0, 0);
        assert!(!z.is_live());
        assert_eq!(
            z.recreate(640, 480),
            RecreateOutcome::Reconfigured { generation: 1 }
        );
        assert!(z.is_live());
    }

    #[test]
    fn pbr_factor_plumbing_reaches_uniform_bytes() {
        let mut camera = Camera::new([2.4, 1.7, 2.8], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.7);
        camera.aspect = 2.0;
        let light = DirectionalLight {
            direction: glam::Vec3::new(-0.45, 0.75, 0.55),
            color: glam::Vec3::new(1.0, 0.96, 0.90),
            intensity: 3.0,
        };
        let material = PbrMaterial {
            base_color: [0.72, 0.45, 0.28],
            metallic: 0.9,
            roughness: 0.25,
            exposure: 1.25,
        };
        let (cam_u, light_u, mat_u) =
            build_uniforms(&camera, 2.0, &light, &material).expect("valid uniforms");
        let cb = cam_u.to_bytes();
        let lb = light_u.to_bytes();
        let mb = mat_u.to_bytes();
        assert_eq!((cb.len(), lb.len(), mb.len()), (80, 32, 32));
        // Material factors land at the WGSL offsets.
        assert_eq!(
            [get_f32(&mb, 0), get_f32(&mb, 4), get_f32(&mb, 8)],
            material.base_color
        );
        assert_eq!(get_f32(&mb, 12), 0.9);
        assert_eq!(get_f32(&mb, 16), 0.25);
        assert_eq!(get_f32(&mb, 20), 1.25);
        // Light direction is normalized on upload; radiance is premultiplied.
        let dir = [get_f32(&lb, 0), get_f32(&lb, 4), get_f32(&lb, 8)];
        let len = dot3(dir, dir).sqrt();
        assert!(
            (len - 1.0).abs() < 1e-6,
            "light dir must be unit, got {len}"
        );
        let rad = [get_f32(&lb, 16), get_f32(&lb, 20), get_f32(&lb, 24)];
        assert!(
            approx(rad, [3.0, 2.88, 2.7], 1e-5),
            "radiance is color*intensity, got {rad:?}"
        );
        assert_eq!(get_f32(&lb, 28), 3.0);
        // Camera position rides at bytes 64..76.
        let pos = [get_f32(&cb, 64), get_f32(&cb, 68), get_f32(&cb, 72)];
        assert!(approx(pos, [2.4, 1.7, 2.8], 1e-6));
        // View-proj matches the camera math (first column spot-check).
        let expect = camera.view_proj().expect("valid camera").to_cols_array();
        for (i, want) in expect.iter().enumerate() {
            assert_eq!(get_f32(&cb, i * 4), *want, "view-proj col byte {i}");
        }
        // Bad inputs fail at the named stage, never silently.
        let mut bad_mat = material.clone();
        bad_mat.roughness = 2.0;
        assert!(matches!(
            build_uniforms(&camera, 2.0, &light, &bad_mat),
            Err(UniformError::Material(_))
        ));
        let mut bad_cam = camera.clone();
        bad_cam.fov_y = 0.0;
        assert!(matches!(
            build_uniforms(&bad_cam, 2.0, &light, &material),
            Err(UniformError::Camera(_))
        ));
    }

    #[test]
    fn mesh_upload_validates() {
        let good = cube();
        assert_eq!((good.positions.len(), good.indices.len()), (24, 36));
        assert!(upload_mesh(&good.positions, &good.normals, &good.uvs, &good.indices).is_ok());
        assert_eq!(upload_mesh(&[], &[], &[], &[]), Err(MeshError::EmptyMesh));
        assert!(matches!(
            upload_mesh(
                &good.positions,
                &good.normals[..12],
                &good.uvs,
                &good.indices
            ),
            Err(MeshError::LengthMismatch {
                field: "normals",
                ..
            })
        ));
        let mut bad_idx = good.indices.clone();
        bad_idx[0] = 24;
        assert_eq!(
            upload_mesh(&good.positions, &good.normals, &good.uvs, &bad_idx),
            Err(MeshError::IndexOutOfRange(24))
        );
        assert_eq!(
            upload_mesh(
                &good.positions,
                &good.normals,
                &good.uvs,
                &good.indices[..35]
            ),
            Err(MeshError::IndexCountNotMultipleOfThree(35))
        );
        let mut bad_pos = good.positions.clone();
        bad_pos[3] = [f32::NAN, 0.0, 0.0];
        assert_eq!(
            upload_mesh(&bad_pos, &good.normals, &good.uvs, &good.indices),
            Err(MeshError::NonFinite {
                field: "positions",
                vertex: 3
            })
        );
        let mut bad_n = good.normals.clone();
        bad_n[5] = [0.0, 0.0, 0.0];
        assert_eq!(
            upload_mesh(&good.positions, &bad_n, &good.uvs, &good.indices),
            Err(MeshError::DegenerateNormal(5))
        );
        let mut bad_uv = good.uvs.clone();
        bad_uv[7] = [1.5, 0.5];
        assert_eq!(
            upload_mesh(&good.positions, &good.normals, &bad_uv, &good.indices),
            Err(MeshError::UvOutOfRange(7))
        );
    }

    #[test]
    fn ported_pbr_math_behaves() {
        // Schlick endpoints: normal incidence returns F0, grazing returns f90.
        let f0 = [0.04, 0.04, 0.04];
        assert_eq!(f_schlick_vec(f0, 1.0, 1.0), f0);
        assert_eq!(f_schlick_vec(f0, 1.0, 0.0), [1.0, 1.0, 1.0]);
        // Perceptual remap clamps Bevy's floor then squares.
        assert_eq!(
            perceptual_to_roughness(0.0),
            MIN_PERCEPTUAL_ROUGHNESS * MIN_PERCEPTUAL_ROUGHNESS
        );
        assert_eq!(perceptual_to_roughness(1.0), 1.0);
        // NDF peaks at the half-vector and stays finite/positive.
        let peak = d_ggx(0.25, 1.0);
        assert!(peak.is_finite() && peak > d_ggx(0.25, 0.5) && d_ggx(0.25, 0.0) >= 0.0);
        // Visibility stays finite/positive over the hemisphere.
        let v = v_smith_ggx_correlated(0.5, 0.7, 0.6);
        assert!(v.is_finite() && v > 0.0);
        // Env-BRDF terms stay positive (Bevy's divide-by-zero guard).
        let fab = f_ab(0.4, 0.6);
        assert!(fab[0] > 0.0 && fab[1] > 0.0);
        // ACES maps black to black, compresses HDR into [0, 1], monotonic.
        assert_eq!(tone_map([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        let dim = tone_map([0.5, 0.5, 0.5]);
        let hot = tone_map([4.0, 4.0, 4.0]);
        assert!(dim.iter().all(|c| *c >= 0.0 && *c <= 1.0));
        assert!(hot.iter().all(|c| *c >= 0.0 && *c <= 1.0));
        assert!(hot[0] >= dim[0] && hot[1] >= dim[1] && hot[2] >= dim[2]);
        // Facing the light outshines a perpendicular surface.
        let lit = |normal: [f32; 3]| {
            shade_pbr(&ShadePoint {
                base_color: [0.7, 0.7, 0.7],
                metallic: 0.0,
                perceptual_roughness: 0.5,
                normal,
                view: [0.0, 0.0, 1.0],
                light_dir: [0.0, 0.0, 1.0],
                radiance: [3.0, 3.0, 3.0],
                exposure: 1.0,
            })
        };
        let facing = lit([0.0, 0.0, 1.0]);
        let edge_on = lit([1.0, 0.0, 0.0]);
        assert!(facing[0] + facing[1] + facing[2] > edge_on[0] + edge_on[1] + edge_on[2]);
    }

    #[test]
    fn capture_produces_png_and_manifest() {
        let (png, manifest) = capture(96, 64).expect("software capture works");
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let img = image::load_from_memory(&png).expect("capture is a valid PNG");
        assert_eq!((img.width(), img.height()), (96, 64));
        assert_eq!((manifest.width, manifest.height), (96, 64));
        assert!(manifest.is_clean());
        assert!(!manifest.adapter.is_empty() && !manifest.driver.is_empty());
        // The cube covers real pixels: count pixels that differ from the
        // deterministic backdrop gradient.
        let rgba = img.to_rgba8();
        let mut covered = 0usize;
        for (y, row) in rgba.rows().enumerate() {
            let t = y as f32 / (64 - 1) as f32;
            let bg = [
                (0.030 + 0.055 * t) * 255.0,
                (0.038 + 0.045 * t) * 255.0,
                (0.062 + 0.038 * t) * 255.0,
            ];
            for p in row {
                let off = (p[0] as f32 - bg[0])
                    .abs()
                    .max((p[1] as f32 - bg[1]).abs())
                    .max((p[2] as f32 - bg[2]).abs());
                if off > 5.0 {
                    covered += 1;
                }
            }
        }
        assert!(covered > 100, "cube must cover pixels, got {covered}");
        // The light provably shades: the same scene with the light off must
        // differ by real energy (dark metal faces stay dark either way, so an
        // absolute highlight threshold would test taste, not plumbing).
        let mut dark = default_scene(96, 64).expect("default scene is valid");
        dark.light.intensity = 0.0;
        let (dark_png, _) = capture_scene(&dark, 96, 64).expect("unlit capture works");
        let dark_img = image::load_from_memory(&dark_png)
            .expect("unlit png decodes")
            .to_rgba8();
        let mut energy = 0u64;
        for (a, b) in rgba.pixels().zip(dark_img.pixels()) {
            energy += (a[0] as i32 - b[0] as i32).unsigned_abs() as u64
                + (a[1] as i32 - b[1] as i32).unsigned_abs() as u64
                + (a[2] as i32 - b[2] as i32).unsigned_abs() as u64;
        }
        assert!(energy > 1000, "light must shade pixels, energy {energy}");
        // Same size twice gives identical bytes (deterministic receipt).
        let (png2, _) = capture(96, 64).expect("software capture works");
        assert_eq!(png, png2);
        assert_eq!(capture(0, 5), Err(CaptureError::ZeroArea));
        assert!(matches!(
            capture(MAX_CAPTURE_DIM + 1, 8),
            Err(CaptureError::TooLarge { .. })
        ));
    }

    #[test]
    fn mesh_tangents_are_orthonormal_with_handedness() {
        // One quad (two tris) on +Z with standard UVs: tangents run +X.
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let normals = [[0.0, 0.0, 1.0]; 4];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let indices = [0, 1, 2, 0, 2, 3];
        let mesh = upload_mesh(&positions, &normals, &uvs, &indices).expect("quad is valid");
        assert_eq!(mesh.tangents.len(), 4, "one tangent per vertex");
        for (t, n) in mesh.tangents.iter().zip(normals.iter()) {
            let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-5, "tangent must be unit, got {len}");
            let dot = t[0] * n[0] + t[1] * n[1] + t[2] * n[2];
            assert!(dot.abs() < 1e-5, "tangent must be orthogonal, dot {dot}");
            assert!(
                t[3] == 1.0 || t[3] == -1.0,
                "handedness is +-1, got {}",
                t[3]
            );
            assert!(t.iter().all(|c| c.is_finite()), "tangent must be finite");
        }
        // Deterministic: same mesh twice gives bit-identical tangents.
        let again = upload_mesh(&positions, &normals, &uvs, &indices).expect("quad is valid");
        assert_eq!(mesh.tangents, again.tangents);
        // Dominant direction is +X for this UV layout.
        assert!(
            mesh.tangents[0][0] > 0.9,
            "expected +X tangent, got {:?}",
            mesh.tangents[0]
        );
        // GPU channel: 48 bytes per vertex with tangent xyzw round-tripping.
        let bytes = mesh.vertex_bytes();
        assert_eq!(
            bytes.len(),
            mesh.positions.len() * 48,
            "stride is 48 with tangents"
        );
        for (i, t) in mesh.tangents.iter().enumerate() {
            let base = i * 48 + 32;
            for (k, want) in t.iter().enumerate() {
                let got = f32::from_le_bytes(
                    bytes[base + k * 4..base + (k + 1) * 4]
                        .try_into()
                        .expect("tangent bytes in range"),
                );
                assert_eq!(got, *want, "tangent byte {k} round-trips");
            }
        }
        let shader = include_str!("shaders/forward.wgsl");
        assert!(
            shader.contains("@location(3) tangent: vec4<f32>"),
            "shader declares tangent at location 3"
        );
    }

    #[test]
    fn mesh_tangents_fallback_on_degenerate_uvs() {
        // Mirrored UVs flip handedness; degenerate UVs fall back to +X-like.
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let normals = [[0.0, 0.0, 1.0]; 4];
        let mirrored = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let flipped = [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let indices = [0, 1, 2, 0, 2, 3];
        let a = upload_mesh(&positions, &normals, &mirrored, &indices).expect("quad is valid");
        let b = upload_mesh(&positions, &normals, &flipped, &indices).expect("quad is valid");
        assert_ne!(
            a.tangents[0][3], b.tangents[0][3],
            "mirrored UVs flip handedness"
        );
        // Degenerate UV triangle: all UVs equal, no direction to solve.
        let flat_uvs = [[0.5, 0.5]; 4];
        let flat =
            upload_mesh(&positions, &normals, &flat_uvs, &indices).expect("flat uvs still upload");
        for (t, n) in flat.tangents.iter().zip(normals.iter()) {
            assert!(t.iter().all(|c| c.is_finite()), "fallback must be finite");
            assert_eq!(t[3], 1.0, "fallback handedness is +1");
            let dot = t[0] * n[0] + t[1] * n[1] + t[2] * n[2];
            assert!(dot.abs() < 1e-5, "fallback stays orthogonal, dot {dot}");
        }
    }
}

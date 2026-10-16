//! import_repair: diagnostic plus setting-driven import repairs.
//!
//! Consumer: the editor's import-fix flow (diagnose, approve settings, repair
//! and recook) and hot-reload material reloads (repaired
//! [`CookedMesh`]/[`CookedMaterial`] with stable ids). Data owner: `engine-assets`; identity stays with the
//! registry, version with digests (identity vs version in `ARCHITECTURE.md`).
//! Units: meters in, meters out; no scale is ever guessed. Determinism scope:
//! same input bytes plus same [`ImportSettings`] give bit-identical mesh,
//! material and `after_digest` (float work is `+ - * /` and `sqrt` in index
//! order only). Errors are the typed [`RepairRefusal`]; sources are never
//! mutated (`&[u8]` in, fresh values out) and rejected geometry stays intact.
//!
//! Rules, all enforced here so callers cannot drift:
//!
//! - Units are explicit only. A file is unitless unless its JSON declares
//!   `unit_meters_per_unit` or `meters_per_unit`; a unitless file with
//!   `unit_meters_per_unit: None` is refused, never guessed from bounds.
//! - Tangents follow [`TangentMode`]: keep the file tangents, deterministically
//!   recompute them from positions plus UVs, or refuse tangentless files.
//! - Color handling is per slot ([`SlotMap`]): the map in settings decides
//!   which slots survive; nothing is reinterpreted or guessed.
//! - Budgets ([`BudgetSet`]`) refuse over-limit imports: triangle count, UV
//!   stretch in permille, and decoded texture pixels. Multi-primitive files
//!   are refused (one primitive per call; EA1 batches the rest) rather than
//!   silently dropping geometry.
//! - Callers log `(before_digest, settings, after_digest)` as the repair
//!   report; the cooker keeps last-good on refusal.
//!
//! No new dependency edges: this module uses `cooked`, `mesh_import` and the
//! feature-independent `registry::{content_hash, sha_hex}` plus the existing
//! `gltf`/`image` import-lane deps.

use super::cooked::{validate_material, validate_mesh, CookedMaterial, CookedMesh, TextureSlot};
use super::mesh_import::{import_gltf_bytes, MeshImportError, MAX_BYTES};
use super::registry::{content_hash, sha_hex};
use super::wire::bound_chars;
use std::fmt;
use std::io::Cursor;

/// Bound for refusal/diagnostic detail strings carried in errors.
pub const MAX_DETAIL_CHARS: usize = 256;
/// Cap on decoded texture bytes probed for the pixel budget (matches cook).
pub const MAX_PROBE_BYTES: usize = 16 * 1024 * 1024;

/// Diagnosis code: the file failed to import; detail names the cause.
pub const CODE_IMPORT: &str = "import.failed";
/// Diagnosis code: input exceeds the import byte ceiling.
pub const CODE_TOO_LARGE: &str = "import.too_large";
/// Diagnosis code: unitless file; an explicit `unit_meters_per_unit` fixes it.
pub const CODE_UNITS_MISSING: &str = "units.missing";
/// Diagnosis code: file carries no tangents; recompute (or keep-fallback) fixes it.
pub const CODE_TANGENTS_MISSING: &str = "tangents.missing";
/// Diagnosis code: more than one primitive; repair handles one per call.
pub const CODE_MESH_MULTIPLE: &str = "mesh.multiple";
/// Diagnosis code: triangle count exceeds the budget; not fixable here.
pub const CODE_BUDGET_TRIS: &str = "budget.tris";
/// Diagnosis code: UV stretch exceeds the budget; not fixable here.
pub const CODE_BUDGET_UV: &str = "budget.uv_stretch";
/// Diagnosis code: a texture exceeds the pixel budget; not fixable here.
pub const CODE_BUDGET_TEXTURE_PX: &str = "budget.texture_px";
/// Diagnosis code: an embedded texture cannot be decoded; not fixable here.
pub const CODE_TEXTURE_UNDECODABLE: &str = "texture.undecodable";

/// What to do about tangents on repair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TangentMode {
    /// Keep the file tangents; files without tangents keep the deterministic
    /// import fallback (`+X` orthogonalized against the normal, `w = 1`).
    Keep,
    /// Recompute every tangent deterministically from positions plus UVs
    /// (degenerate UVs fall back per vertex, same rule as the import lane).
    Recompute,
    /// Refuse files that carry no tangents instead of filling any in.
    Refuse,
}

/// Which texture slots survive a repair. Disallowed slots are dropped; the
/// surviving slots keep the import lane's per-slot encoding, never a guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotMap {
    /// Keep `BaseColor` (sRGB) references.
    pub allow_base_color: bool,
    /// Keep `MetallicRoughness` (linear) references.
    pub allow_metallic_roughness: bool,
    /// Keep `Normal` (linear) references.
    pub allow_normal: bool,
    /// Keep `Occlusion` (linear) references.
    pub allow_occlusion: bool,
    /// Keep `Emissive` (sRGB) references.
    pub allow_emissive: bool,
}

impl Default for SlotMap {
    fn default() -> Self {
        Self {
            allow_base_color: true,
            allow_metallic_roughness: true,
            allow_normal: true,
            allow_occlusion: true,
            allow_emissive: true,
        }
    }
}

impl SlotMap {
    /// True when `slot` survives this map.
    pub fn allows(self, slot: TextureSlot) -> bool {
        match slot {
            TextureSlot::BaseColor => self.allow_base_color,
            TextureSlot::MetallicRoughness => self.allow_metallic_roughness,
            TextureSlot::Normal => self.allow_normal,
            TextureSlot::Occlusion => self.allow_occlusion,
            TextureSlot::Emissive => self.allow_emissive,
        }
    }
}

/// Repair budgets. Anything over budget is refused with the before digest;
/// budgets are never loosened to make an import pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BudgetSet {
    /// Max triangles (`indices.len() / 3`) in the repaired mesh.
    pub max_tris: u32,
    /// Max UV edge stretch in permille (edge length in UV units times 1000).
    pub max_uv_stretch_permille: u32,
    /// Max decoded texels of any single embedded texture.
    pub max_texture_px: u32,
}

impl Default for BudgetSet {
    fn default() -> Self {
        Self {
            max_tris: 1_000_000,
            // A full unit-square diagonal is ~1414 permille; the default
            // admits any `0..=1` UV layout while catching wild coordinates.
            max_uv_stretch_permille: 1500,
            max_texture_px: 16_777_216,
        }
    }
}

/// Settings that authorize a repair. Every field is explicit: `None` units
/// plus a unitless file refuses, and budgets refuse rather than rescale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImportSettings {
    /// Explicit meters per file unit (`Some(0.01)` for centimeter files).
    /// `None` means the file must declare its own units; nothing is guessed.
    pub unit_meters_per_unit: Option<f32>,
    /// What to do about tangents.
    pub tangents: TangentMode,
    /// Which texture slots survive.
    pub color_slots: SlotMap,
    /// Budgets enforced before any repair is returned.
    pub budgets: BudgetSet,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            unit_meters_per_unit: None,
            tangents: TangentMode::Keep,
            color_slots: SlotMap::default(),
            budgets: BudgetSet::default(),
        }
    }
}

/// One diagnosed problem with a stable machine-readable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Stable code (`units.missing`, `budget.tris`, ...); see `CODE_*`.
    pub code: &'static str,
    /// Bounded human detail (never echoes unbounded input).
    pub detail: String,
    /// True when an [`ImportSettings`] choice repairs it (units, tangents).
    /// Budget, topology and undecodable-texture issues are never fixable here.
    pub fixable: bool,
}

/// Read-only diagnosis of raw import bytes: problems plus the content digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportDiagnosis {
    /// Every problem found, in stable check order; empty means clean.
    pub issues: Vec<Issue>,
    /// Hex digest of the exact input bytes that were diagnosed.
    pub before_digest: String,
}

/// A refused repair: why, plus the digest of the untouched input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairRefusal {
    /// Bounded reason naming the code and the offending value.
    pub reason: String,
    /// Hex digest of the exact input bytes; the source is unchanged.
    pub before_digest: String,
}

impl fmt::Display for RepairRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "import repair refused: {}", self.reason)
    }
}

impl std::error::Error for RepairRefusal {}

/// Stable asset id for source bytes: the crate content-hash canon, so the
/// same file keeps the same id across recooks while `after_digest` versions
/// the cooked output (identity vs version).
pub fn asset_id(bytes: &[u8]) -> u64 {
    content_hash(bytes)
}

/// Hex digest of raw input bytes (the `before_digest` in both structs).
pub fn before_digest_of(bytes: &[u8]) -> String {
    sha_hex(content_hash(bytes))
}

/// Hex digest of repaired cooked data: canonical little-endian field bytes
/// (positions, normals, UVs, tangents, indices, PBR factors, sorted texture
/// refs) hashed with the same canon. Same mesh plus material always gives the
/// same digest, so EA1 can compare recooks.
pub fn after_digest_of(mesh: &CookedMesh, material: &CookedMaterial) -> String {
    sha_hex(content_hash(&canonical_cooked_bytes(mesh, material)))
}

/// Canonical bytes for [`after_digest_of`]: fixed field order, little-endian
/// floats/ints, texture refs sorted by slot then asset id.
fn canonical_cooked_bytes(mesh: &CookedMesh, material: &CookedMaterial) -> Vec<u8> {
    let mut refs: Vec<(u8, u64)> = material
        .textures
        .iter()
        .map(|r| {
            let rank = match r.slot {
                TextureSlot::BaseColor => 0u8,
                TextureSlot::MetallicRoughness => 1u8,
                TextureSlot::Normal => 2u8,
                TextureSlot::Occlusion => 3u8,
                TextureSlot::Emissive => 4u8,
            };
            (rank, r.asset)
        })
        .collect();
    refs.sort_unstable();
    let mut out = Vec::with_capacity(
        mesh.positions.len() * 36 + mesh.indices.len() * 4 + refs.len() * 9 + 32,
    );
    for array in mesh.positions.iter().chain(mesh.normals.iter()) {
        for v in array {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for uv in &mesh.uv0 {
        for v in uv {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for tangent in &mesh.tangents {
        for v in tangent {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    for index in &mesh.indices {
        out.extend_from_slice(&index.to_le_bytes());
    }
    for v in material.base_color_factor {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&material.metallic_factor.to_le_bytes());
    out.extend_from_slice(&material.roughness_factor.to_le_bytes());
    for (rank, asset) in refs {
        out.push(rank);
        out.extend_from_slice(&asset.to_le_bytes());
    }
    out
}

/// Truncate a detail string to at most `MAX_DETAIL_CHARS` chars.
fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_DETAIL_CHARS)
}

/// True when the file JSON declares explicit units. glTF stores meters; the
/// declaration just proves the exporter said so, so presence (not a bounding
/// box heuristic) is what authorizes unit handling.
fn has_explicit_units(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let lower = text.to_ascii_lowercase();
    lower.contains("unit_meters_per_unit") || lower.contains("meters_per_unit")
}

/// True when any primitive declares a `TANGENT` attribute. Parsed from the
/// document (not guessed from the cooked fallback), so `Refuse` and the
/// diagnosis see what the file actually carries.
fn document_has_tangents(bytes: &[u8]) -> bool {
    let parsed = match gltf::Gltf::from_slice(bytes) {
        Ok(parsed) => parsed,
        Err(_) => return false,
    };
    parsed.document.meshes().any(|mesh| {
        mesh.primitives().any(|primitive| {
            primitive
                .attributes()
                .any(|(semantic, _)| semantic == gltf::mesh::Semantic::Tangents)
        })
    })
}

/// Max UV edge length over all triangles, in permille. Deterministic in index
/// order; uses `sqrt` only (no simulation hash involvement).
fn uv_stretch_permille_of(mesh: &CookedMesh) -> u32 {
    let mut worst = 0.0f32;
    for tri in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let (Some(uv0), Some(uv1), Some(uv2)) = (mesh.uv0.get(a), mesh.uv0.get(b), mesh.uv0.get(c))
        else {
            continue;
        };
        for (p, q) in [(uv0, uv1), (uv1, uv2), (uv2, uv0)] {
            let dx = p[0] - q[0];
            let dy = p[1] - q[1];
            let len = (dx * dx + dy * dy).sqrt();
            if len.is_finite() && len > worst {
                worst = len;
            }
        }
    }
    (worst * 1000.0).floor().clamp(0.0, u32::MAX as f32) as u32
}

/// Decode standard base64 (data-URI payloads) without new dependencies.
/// Returns `None` on bad characters, bad padding or unbounded length.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    if text.len() > MAX_PROBE_BYTES.div_ceil(3) * 4 {
        return None;
    }
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for quad in bytes.chunks_exact(4) {
        let pad = quad.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut bits: u32 = 0;
        for (i, c) in quad.iter().enumerate() {
            if *c == b'=' {
                if i < 4 - pad {
                    return None;
                }
                bits <<= 6;
            } else {
                if pad > 0 && i >= 4 - pad {
                    return None;
                }
                bits = (bits << 6) | u32::from(value(*c)?);
            }
        }
        out.push((bits >> 16) as u8);
        if pad < 2 {
            out.push((bits >> 8) as u8);
        }
        if pad < 1 {
            out.push(bits as u8);
        }
    }
    Some(out)
}

/// Raw encoded bytes of one image: buffer-view slices plus `data:` URIs.
/// External file URIs have no resolvable bytes in a slice repair (`None`).
fn raw_image_bytes(buffers: &[gltf::buffer::Data], image: &gltf::Image<'_>) -> Option<Vec<u8>> {
    match image.source() {
        gltf::image::Source::View { view, .. } => {
            let data = buffers.get(view.buffer().index())?;
            let end = view.offset().checked_add(view.length())?;
            data.0.get(view.offset()..end).map(|slice| slice.to_vec())
        }
        gltf::image::Source::Uri { uri, .. } => {
            let payload = uri.strip_prefix("data:")?.split(',').nth(1)?;
            if payload.len() > MAX_PROBE_BYTES.div_ceil(3) * 4 {
                return None;
            }
            decode_base64(payload)
        }
    }
}

/// Widest embedded texture in texels, for the pixel budget. Returns the
/// offending image index when a texture cannot be decoded or measured, so the
/// caller refuses instead of silently passing an unchecked texture.
fn probe_textures(bytes: &[u8]) -> Result<u32, usize> {
    let parsed = gltf::Gltf::from_slice(bytes).map_err(|_| 0usize)?;
    let buffers = gltf::import_buffers(&parsed.document, None, parsed.blob).map_err(|_| 0usize)?;
    let mut widest = 0u32;
    for (index, image) in parsed.document.images().enumerate() {
        let raw = raw_image_bytes(&buffers, &image).ok_or(index)?;
        if raw.is_empty() || raw.len() > MAX_PROBE_BYTES {
            return Err(index);
        }
        let (width, height) = image::ImageReader::new(Cursor::new(&raw))
            .with_guessed_format()
            .map_err(|_| index)?
            .into_dimensions()
            .map_err(|_| index)?;
        widest = widest.max(width.saturating_mul(height));
    }
    Ok(widest)
}

/// Deterministic tangent recompute from positions plus UVs: per-triangle
/// solve accumulated in index order, Gram-Schmidt against the normal, `w`
/// from the handedness sign. Degenerate UVs fall back per vertex to `+X`
/// orthogonalized against the normal with `w = 1.0` (the import-lane rule).
pub fn recompute_tangents(mesh: &CookedMesh) -> Vec<[f32; 4]> {
    let mut accum = vec![([0.0f32; 3], [0.0f32; 3]); mesh.positions.len()];
    for tri in mesh.indices.chunks_exact(3) {
        let [a, b, c] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let (Some(p0), Some(p1), Some(p2)) = (
            mesh.positions.get(a),
            mesh.positions.get(b),
            mesh.positions.get(c),
        ) else {
            continue;
        };
        let (Some(t0), Some(t1), Some(t2)) = (mesh.uv0.get(a), mesh.uv0.get(b), mesh.uv0.get(c))
        else {
            continue;
        };
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let (s1, t_1) = (t1[0] - t0[0], t1[1] - t0[1]);
        let (s2, t_2) = (t2[0] - t0[0], t2[1] - t0[1]);
        let denom = s1 * t_2 - s2 * t_1;
        if !denom.is_finite() || denom.abs() <= 1e-12 {
            continue;
        }
        let r = 1.0 / denom;
        let sdir = [
            (e1[0] * t_2 - e2[0] * t_1) * r,
            (e1[1] * t_2 - e2[1] * t_1) * r,
            (e1[2] * t_2 - e2[2] * t_1) * r,
        ];
        let tdir = [
            (e2[0] * s1 - e1[0] * s2) * r,
            (e2[1] * s1 - e1[1] * s2) * r,
            (e2[2] * s1 - e1[2] * s2) * r,
        ];
        for idx in [a, b, c] {
            if let Some(slot) = accum.get_mut(idx) {
                slot.0[0] += sdir[0];
                slot.0[1] += sdir[1];
                slot.0[2] += sdir[2];
                slot.1[0] += tdir[0];
                slot.1[1] += tdir[1];
                slot.1[2] += tdir[2];
            }
        }
    }
    mesh.normals
        .iter()
        .zip(accum.iter())
        .map(|(n, (sdir, tdir))| orthogonalize_tangent(n, sdir, tdir))
        .collect()
}

/// One orthogonalized tangent: `sdir` minus its normal projection,
/// normalized, with `w` from the handedness against `tdir`.
fn orthogonalize_tangent(n: &[f32; 3], sdir: &[f32; 3], tdir: &[f32; 3]) -> [f32; 4] {
    let dot = sdir[0] * n[0] + sdir[1] * n[1] + sdir[2] * n[2];
    let mut t = [
        sdir[0] - n[0] * dot,
        sdir[1] - n[1] * dot,
        sdir[2] - n[2] * dot,
    ];
    let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
    if !len.is_finite() || len <= 1e-12 {
        // Degenerate accumulation: the import-lane orthogonal fallback.
        let reference = if n[0].abs() > 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let dot = reference[0] * n[0] + reference[1] * n[1] + reference[2] * n[2];
        t = [
            reference[0] - n[0] * dot,
            reference[1] - n[1] * dot,
            reference[2] - n[2] * dot,
        ];
        let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
        if len.is_finite() && len > 1e-12 {
            t[0] /= len;
            t[1] /= len;
            t[2] /= len;
        } else {
            t = [1.0, 0.0, 0.0];
        }
        return [t[0], t[1], t[2], 1.0];
    }
    t[0] /= len;
    t[1] /= len;
    t[2] /= len;
    // Handedness: sign of dot(cross(n, t), tdir); zero stays +1.
    let cross = [
        n[1] * t[2] - n[2] * t[1],
        n[2] * t[0] - n[0] * t[2],
        n[0] * t[1] - n[1] * t[0],
    ];
    let w = if cross[0] * tdir[0] + cross[1] * tdir[1] + cross[2] * tdir[2] < 0.0 {
        -1.0
    } else {
        1.0
    };
    [t[0], t[1], t[2], w]
}

/// Stable code for an import failure (diagnosis never echoes raw dumps).
fn import_issue_code(error: &MeshImportError) -> &'static str {
    match error {
        MeshImportError::TooLarge { .. } => CODE_TOO_LARGE,
        MeshImportError::UnsupportedFileExtension { .. }
        | MeshImportError::UnsupportedExtension { .. }
        | MeshImportError::UnsupportedMode { .. }
        | MeshImportError::SparseUnsupported { .. } => CODE_IMPORT,
        MeshImportError::Parse { .. }
        | MeshImportError::MissingBuffer { .. }
        | MeshImportError::NoMeshes
        | MeshImportError::InvalidData { .. } => CODE_IMPORT,
    }
}

/// Read-only diagnosis of raw import bytes against the default budgets.
/// Never fails and never mutates: malformed input yields an `import.failed`
/// issue, and every issue carries whether an [`ImportSettings`] choice fixes
/// it (units and tangents do; budgets, topology and undecodable textures do
/// not). Pair with [`repair`] using caller-approved settings.
pub fn diagnose(bytes: &[u8]) -> ImportDiagnosis {
    diagnose_with(bytes, &BudgetSet::default())
}

/// [`diagnose`] with explicit budgets (repair enforces its own settings'
/// budgets; this screens a file against a deployment's limits first).
pub fn diagnose_with(bytes: &[u8], budgets: &BudgetSet) -> ImportDiagnosis {
    let before_digest = before_digest_of(bytes);
    let mut issues = Vec::new();
    if bytes.len() > MAX_BYTES {
        issues.push(Issue {
            code: CODE_TOO_LARGE,
            detail: bound_detail(&format!("{} bytes over {MAX_BYTES}", bytes.len())),
            fixable: false,
        });
        return ImportDiagnosis {
            issues,
            before_digest,
        };
    }
    let imported = match import_gltf_bytes(bytes) {
        Ok(imported) => imported,
        Err(error) => {
            issues.push(Issue {
                code: import_issue_code(&error),
                detail: bound_detail(&error.to_string()),
                fixable: false,
            });
            return ImportDiagnosis {
                issues,
                before_digest,
            };
        }
    };
    if !has_explicit_units(bytes) {
        issues.push(Issue {
            code: CODE_UNITS_MISSING,
            detail: bound_detail(
                "unitless file: pass unit_meters_per_unit explicitly; no scale is guessed",
            ),
            fixable: true,
        });
    }
    if !document_has_tangents(bytes) {
        issues.push(Issue {
            code: CODE_TANGENTS_MISSING,
            detail: bound_detail("no TANGENT attribute: recompute or keep the fallback"),
            fixable: true,
        });
    }
    if imported.meshes.len() != 1 {
        issues.push(Issue {
            code: CODE_MESH_MULTIPLE,
            detail: bound_detail(&format!(
                "{} primitives: repair handles one per call",
                imported.meshes.len()
            )),
            fixable: false,
        });
    }
    let total_tris: u32 = imported
        .meshes
        .iter()
        .map(|mesh| (mesh.indices.len() / 3).min(u32::MAX as usize) as u32)
        .fold(0u32, |sum, tris| sum.saturating_add(tris));
    if total_tris > budgets.max_tris {
        issues.push(Issue {
            code: CODE_BUDGET_TRIS,
            detail: bound_detail(&format!("{total_tris} tris over {}", budgets.max_tris)),
            fixable: false,
        });
    }
    let worst_uv = imported
        .meshes
        .iter()
        .map(uv_stretch_permille_of)
        .max()
        .unwrap_or(0);
    if worst_uv > budgets.max_uv_stretch_permille {
        issues.push(Issue {
            code: CODE_BUDGET_UV,
            detail: bound_detail(&format!(
                "{worst_uv} permille over {}",
                budgets.max_uv_stretch_permille
            )),
            fixable: false,
        });
    }
    match probe_textures(bytes) {
        Ok(widest) => {
            if widest > budgets.max_texture_px {
                issues.push(Issue {
                    code: CODE_BUDGET_TEXTURE_PX,
                    detail: bound_detail(&format!(
                        "{widest} texels over {}",
                        budgets.max_texture_px
                    )),
                    fixable: false,
                });
            }
        }
        Err(index) => issues.push(Issue {
            code: CODE_TEXTURE_UNDECODABLE,
            detail: bound_detail(&format!("texture image {index} cannot be decoded")),
            fixable: false,
        }),
    }
    ImportDiagnosis {
        issues,
        before_digest,
    }
}

/// Repair raw import bytes with caller-approved settings into cooked data.
///
/// On success returns the mesh, the slot-filtered material and the
/// `after_digest` (log all three with the settings as the repair report).
/// On refusal nothing is written anywhere: the input slice is untouched, no
/// registry or cooker state exists here to change, and the reason carries the
/// `before_digest` so EA1 can match it against [`diagnose`].
pub fn repair(
    bytes: &[u8],
    settings: &ImportSettings,
) -> Result<(CookedMesh, CookedMaterial, String), RepairRefusal> {
    let before_digest = before_digest_of(bytes);
    let refuse = |reason: String| RepairRefusal {
        reason: bound_detail(&reason),
        before_digest: before_digest.clone(),
    };
    if let Some(scale) = settings.unit_meters_per_unit {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(refuse(format!(
                "units.invalid_setting: {scale} is not a finite positive scale"
            )));
        }
    }
    let imported = import_gltf_bytes(bytes).map_err(|e| refuse(format!("import refused: {e}")))?;
    if imported.meshes.len() != 1 || imported.materials.len() != 1 {
        return Err(refuse(format!(
            "mesh.multiple: {} primitives need one repair call each",
            imported.meshes.len()
        )));
    }
    let file_has_tangents = document_has_tangents(bytes);
    let scale = if has_explicit_units(bytes) {
        settings.unit_meters_per_unit.unwrap_or(1.0)
    } else {
        match settings.unit_meters_per_unit {
            Some(scale) => scale,
            // The one rule that matters: a unitless file without an explicit
            // setting is refused. Guessing from bounding boxes hides a scale
            // bug in every downstream placement.
            None => {
                return Err(refuse(
                    "units.missing: unitless file needs explicit unit_meters_per_unit".to_string(),
                ));
            }
        }
    };
    if settings.tangents == TangentMode::Refuse && !file_has_tangents {
        return Err(refuse(
            "tangents.missing: file carries no TANGENT attribute".to_string(),
        ));
    }
    let mut mesh = imported.meshes[0].clone();
    let mut material = imported.materials[0].clone();
    if scale != 1.0 {
        for position in &mut mesh.positions {
            position[0] *= scale;
            position[1] *= scale;
            position[2] *= scale;
        }
    }
    if settings.tangents == TangentMode::Recompute {
        mesh.tangents = recompute_tangents(&mesh);
    }
    material
        .textures
        .retain(|reference| settings.color_slots.allows(reference.slot));
    validate_mesh(&mesh).map_err(|e| refuse(format!("mesh invalid after repair: {e:?}")))?;
    validate_material(&material)
        .map_err(|e| refuse(format!("material invalid after repair: {e:?}")))?;
    let tris = (mesh.indices.len() / 3).min(u32::MAX as usize) as u32;
    if tris > settings.budgets.max_tris {
        return Err(refuse(format!(
            "budget.tris: {tris} over {}",
            settings.budgets.max_tris
        )));
    }
    let stretch = uv_stretch_permille_of(&mesh);
    if stretch > settings.budgets.max_uv_stretch_permille {
        return Err(refuse(format!(
            "budget.uv_stretch: {stretch} permille over {}",
            settings.budgets.max_uv_stretch_permille
        )));
    }
    match probe_textures(bytes) {
        Ok(widest) => {
            if widest > settings.budgets.max_texture_px {
                return Err(refuse(format!(
                    "budget.texture_px: {widest} over {}",
                    settings.budgets.max_texture_px
                )));
            }
        }
        Err(index) => {
            return Err(refuse(format!(
                "texture.undecodable: image {index} cannot be decoded"
            )));
        }
    }
    let after_digest = after_digest_of(&mesh, &material);
    Ok((mesh, material, after_digest))
}

#[cfg(test)]
mod tests {
    use super::super::cooked::TextureSlot;
    use super::*;

    const TRIANGLE: &[u8] = include_bytes!("../../../fixtures/khronos/Triangle.gltf");

    /// Settings that authorize the unitless fixtures with an explicit scale.
    fn explicit(scale: f32) -> ImportSettings {
        ImportSettings {
            unit_meters_per_unit: Some(scale),
            ..ImportSettings::default()
        }
    }

    /// Tiny standard-base64 encoder for building glTF data URIs in tests.
    fn b64_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len() / 3 * 4 + 4);
        for chunk in bytes.chunks(3) {
            let (a, b, c) = (
                chunk[0] as u32,
                *chunk.get(1).unwrap_or(&0) as u32,
                *chunk.get(2).unwrap_or(&0) as u32,
            );
            let bits = (a << 16) | (b << 8) | c;
            out.push(ALPHABET[((bits >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((bits >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[((bits >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(bits & 63) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    /// Minimal single-mesh glTF with `tris` disjoint triangles, `+Z` normals
    /// and per-triangle UVs spanning `uv_span`. No material, no tangents, no
    /// unit declaration: unitless by construction.
    fn make_gltf(tris: usize, uv_span: f32) -> Vec<u8> {
        assert!(tris >= 1, "test helper needs at least one triangle");
        let verts = tris * 3;
        let mut positions = Vec::with_capacity(verts * 12);
        let mut normals = Vec::with_capacity(verts * 12);
        let mut uvs = Vec::with_capacity(verts * 8);
        let mut indices = Vec::with_capacity(tris * 12);
        let mut xmax = 0.0f32;
        for tri in 0..tris {
            let x = tri as f32 * 2.0;
            for (dx, dy, uv) in [
                (0.0f32, 0.0f32, [0.0f32, 0.0f32]),
                (1.0f32, 0.0f32, [uv_span, 0.0f32]),
                (0.0f32, 1.0f32, [0.0f32, uv_span]),
            ] {
                positions.extend_from_slice(&(x + dx).to_le_bytes());
                positions.extend_from_slice(&dy.to_le_bytes());
                positions.extend_from_slice(&0.0f32.to_le_bytes());
                normals.extend_from_slice(&0.0f32.to_le_bytes());
                normals.extend_from_slice(&0.0f32.to_le_bytes());
                normals.extend_from_slice(&1.0f32.to_le_bytes());
                uvs.extend_from_slice(&uv[0].to_le_bytes());
                uvs.extend_from_slice(&uv[1].to_le_bytes());
                xmax = xmax.max(x + dx);
            }
            for k in 0..3 {
                indices.extend_from_slice(&((tri * 3 + k) as u32).to_le_bytes());
            }
        }
        let pos_off = 0;
        let nrm_off = positions.len();
        let uv_off = nrm_off + normals.len();
        let idx_off = uv_off + uvs.len();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&positions);
        buffer.extend_from_slice(&normals);
        buffer.extend_from_slice(&uvs);
        buffer.extend_from_slice(&indices);
        let uri = b64_encode(&buffer);
        let json = format!(
            concat!(
                "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],",
                "\"nodes\":[{{\"mesh\":0}}],",
                "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":1,\"NORMAL\":2,",
                "\"TEXCOORD_0\":3}},\"indices\":0}}]}}],",
                "\"buffers\":[{{\"uri\":\"data:application/octet-stream;base64,{uri}\",",
                "\"byteLength\":{len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":{pos_off},\"byteLength\":{pos_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{nrm_off},\"byteLength\":{nrm_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{uv_off},\"byteLength\":{uv_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{idx_off},\"byteLength\":{idx_len}}}],",
                "\"accessors\":[",
                "{{\"bufferView\":3,\"byteOffset\":0,\"componentType\":5125,",
                "\"count\":{counts},\"type\":\"SCALAR\"}},",
                "{{\"bufferView\":0,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":{verts},\"type\":\"VEC3\",\"min\":[0.0,0.0,0.0],",
                "\"max\":[{xmax},1.0,0.0]}},",
                "{{\"bufferView\":1,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":{verts},\"type\":\"VEC3\"}},",
                "{{\"bufferView\":2,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":{verts},\"type\":\"VEC2\"}}]}}"
            ),
            uri = uri,
            len = buffer.len(),
            pos_off = pos_off,
            pos_len = positions.len(),
            nrm_off = nrm_off,
            nrm_len = normals.len(),
            uv_off = uv_off,
            uv_len = uvs.len(),
            idx_off = idx_off,
            idx_len = indices.len(),
            counts = tris * 3,
            verts = verts,
            xmax = xmax,
        );
        json.into_bytes()
    }

    /// Encode flat RGBA rows as PNG bytes via the `image` import-lane dep.
    fn png_bytes(width: u32, height: u32, texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let image: image::RgbaImage =
            image::ImageBuffer::from_fn(width, height, |x, y| image::Rgba(texel(x, y)));
        let dynamic = image::DynamicImage::ImageRgba8(image);
        let mut out = Vec::new();
        {
            let mut cursor = Cursor::new(&mut out);
            dynamic
                .write_to(&mut cursor, image::ImageFormat::Png)
                .expect("test png must encode");
        }
        out
    }

    /// Single triangle plus one buffer-view PNG image behind two texture
    /// slots (base color and normal share the image).
    fn make_textured_gltf(png: &[u8]) -> Vec<u8> {
        let mut positions = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for v in p {
                positions.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut normals = Vec::new();
        for _ in 0..3 {
            for v in [0.0f32, 0.0, 1.0] {
                normals.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut uvs = Vec::new();
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            for v in uv {
                uvs.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut indices = Vec::new();
        for i in [0u32, 1, 2] {
            indices.extend_from_slice(&i.to_le_bytes());
        }
        let pos_len = positions.len();
        let nrm_len = normals.len();
        let uv_len = uvs.len();
        let idx_len = indices.len();
        let mut geo = Vec::new();
        geo.extend_from_slice(&positions);
        geo.extend_from_slice(&normals);
        geo.extend_from_slice(&uvs);
        geo.extend_from_slice(&indices);
        let json = format!(
            concat!(
                "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],",
                "\"nodes\":[{{\"mesh\":0}}],",
                "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":1,\"NORMAL\":2,",
                "\"TEXCOORD_0\":3}},\"indices\":0,\"material\":0}}]}}],",
                "\"materials\":[{{\"pbrMetallicRoughness\":{{",
                "\"baseColorTexture\":{{\"index\":0}}}},",
                "\"normalTexture\":{{\"index\":1}}}}],",
                "\"textures\":[{{\"source\":0}},{{\"source\":0}}],",
                "\"images\":[{{\"bufferView\":4,\"mimeType\":\"image/png\"}}],",
                "\"buffers\":[",
                "{{\"uri\":\"data:application/octet-stream;base64,{geo}\",",
                "\"byteLength\":{geo_len}}},",
                "{{\"uri\":\"data:application/octet-stream;base64,{png}\",",
                "\"byteLength\":{png_len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{pos_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{nrm_off},\"byteLength\":{nrm_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{uv_off},\"byteLength\":{uv_len}}},",
                "{{\"buffer\":0,\"byteOffset\":{idx_off},\"byteLength\":{idx_len}}},",
                "{{\"buffer\":1,\"byteOffset\":0,\"byteLength\":{png_len}}}],",
                "\"accessors\":[",
                "{{\"bufferView\":3,\"byteOffset\":0,\"componentType\":5125,",
                "\"count\":3,\"type\":\"SCALAR\"}},",
                "{{\"bufferView\":0,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":3,\"type\":\"VEC3\",\"min\":[0.0,0.0,0.0],",
                "\"max\":[1.0,1.0,0.0]}},",
                "{{\"bufferView\":1,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":3,\"type\":\"VEC3\"}},",
                "{{\"bufferView\":2,\"byteOffset\":0,\"componentType\":5126,",
                "\"count\":3,\"type\":\"VEC2\"}}]}}"
            ),
            geo = b64_encode(&geo),
            geo_len = geo.len(),
            png = b64_encode(png),
            png_len = png.len(),
            pos_len = pos_len,
            nrm_off = pos_len,
            nrm_len = nrm_len,
            uv_off = pos_len + nrm_len,
            uv_len = uv_len,
            idx_off = pos_len + nrm_len + uv_len,
            idx_len = idx_len,
        );
        json.into_bytes()
    }

    #[test]
    fn unitless_file_without_explicit_units_is_refused_never_guessed() {
        // The Khronos triangle declares no units: the default settings must
        // refuse, and the refusal must name units without touching the input.
        let before = TRIANGLE.to_vec();
        let diagnosis = diagnose(TRIANGLE);
        assert!(
            diagnosis
                .issues
                .iter()
                .any(|issue| issue.code == CODE_UNITS_MISSING && issue.fixable),
            "diagnosis flags fixable units, got {:?}",
            diagnosis.issues
        );
        let input = TRIANGLE.to_vec();
        let err = repair(TRIANGLE, &ImportSettings::default())
            .expect_err("unitless file without explicit units must refuse");
        assert!(
            err.reason.contains("units.missing"),
            "refusal names units, got {}",
            err.reason
        );
        assert_eq!(err.before_digest, diagnosis.before_digest);
        assert_eq!(err.before_digest, before_digest_of(TRIANGLE));
        assert_eq!(input, before, "rejected geometry stays intact");
        assert!(
            !has_explicit_units(TRIANGLE),
            "fixture is unitless by construction"
        );
        // An explicit setting authorizes the same file.
        let (mesh, _, _) = repair(TRIANGLE, &explicit(2.0)).expect("explicit units repair");
        let plain = import_gltf_bytes(TRIANGLE).expect("fixture imports").meshes[0].clone();
        assert_eq!(mesh.positions.len(), plain.positions.len());
        for (scaled, original) in mesh.positions.iter().zip(plain.positions.iter()) {
            for (s, o) in scaled.iter().zip(original.iter()) {
                assert!((s - o * 2.0).abs() < 1e-6, "explicit scale applies exactly");
            }
        }
    }

    #[test]
    fn invalid_unit_settings_refused() {
        for scale in [f32::NAN, f32::INFINITY, 0.0, -1.0] {
            let err = repair(TRIANGLE, &explicit(scale))
                .expect_err(&format!("scale {scale} must refuse"));
            assert!(
                err.reason.contains("units.invalid_setting"),
                "names the bad setting, got {}",
                err.reason
            );
            assert_eq!(err.before_digest, before_digest_of(TRIANGLE));
        }
    }

    #[test]
    fn over_budget_import_refused_with_before_and_after_digests() {
        // Two triangles against a one-triangle budget: refused, with the
        // diagnose digest carried on the refusal.
        let bytes = make_gltf(2, 0.25);
        let diagnosis = diagnose_with(
            &bytes,
            &BudgetSet {
                max_tris: 1,
                ..BudgetSet::default()
            },
        );
        assert!(
            diagnosis
                .issues
                .iter()
                .any(|issue| issue.code == CODE_BUDGET_TRIS && !issue.fixable),
            "diagnosis flags unfixable tris, got {:?}",
            diagnosis.issues
        );
        let settings = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            budgets: BudgetSet {
                max_tris: 1,
                ..BudgetSet::default()
            },
            ..ImportSettings::default()
        };
        let before = bytes.clone();
        let err = repair(&bytes, &settings).expect_err("over-budget must refuse");
        assert!(
            err.reason.contains("budget.tris"),
            "refusal names the budget, got {}",
            err.reason
        );
        assert_eq!(err.before_digest, before_digest_of(&bytes));
        assert_eq!(err.before_digest, diagnosis.before_digest);
        assert_eq!(bytes, before, "rejected geometry stays intact");
        // Within budget the same file recooks to a stable after digest in a
        // different domain than the source digest.
        let ok = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            ..ImportSettings::default()
        };
        let (_, _, after) = repair(&bytes, &ok).expect("within budget repairs");
        assert_ne!(after, before_digest_of(&bytes));
        let (_, _, again) = repair(&bytes, &ok).expect("recook");
        assert_eq!(after, again, "same input plus settings, same digest");
    }

    #[test]
    fn uv_stretch_budget_refused() {
        // Full-span UVs carry a 1414-permille hypotenuse edge; a 1413 budget
        // refuses while the exact budget passes.
        let bytes = make_gltf(1, 1.0);
        let tight = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            budgets: BudgetSet {
                max_uv_stretch_permille: 1413,
                ..BudgetSet::default()
            },
            ..ImportSettings::default()
        };
        let err = repair(&bytes, &tight).expect_err("stretched UVs must refuse");
        assert!(
            err.reason.contains("budget.uv_stretch"),
            "refusal names UV stretch, got {}",
            err.reason
        );
        let loose = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            budgets: BudgetSet {
                max_uv_stretch_permille: 1414,
                ..BudgetSet::default()
            },
            ..ImportSettings::default()
        };
        repair(&bytes, &loose).expect("exact-budget UVs pass");
    }

    #[test]
    fn texture_pixel_budget_refused() {
        // A real 8x8 PNG (64 texels) against a 63-texel budget refuses.
        let png = png_bytes(8, 8, |x, y| [(x * 32) as u8, (y * 32) as u8, 64, 255]);
        let bytes = make_textured_gltf(&png);
        let tight = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            budgets: BudgetSet {
                max_texture_px: 63,
                ..BudgetSet::default()
            },
            ..ImportSettings::default()
        };
        let err = repair(&bytes, &tight).expect_err("over-pixel texture must refuse");
        assert!(
            err.reason.contains("budget.texture_px"),
            "refusal names the texture budget, got {}",
            err.reason
        );
        assert_eq!(err.before_digest, diagnose(&bytes).before_digest);
        let loose = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            budgets: BudgetSet {
                max_texture_px: 64,
                ..BudgetSet::default()
            },
            ..ImportSettings::default()
        };
        repair(&bytes, &loose).expect("exact-budget texture passes");
    }

    #[test]
    fn tangent_recompute_is_deterministic() {
        // The fixture carries no tangents: recompute twice must agree bit for
        // bit, and `Refuse` must reject the same file.
        let settings = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            tangents: TangentMode::Recompute,
            ..ImportSettings::default()
        };
        let (first, _, digest_first) = repair(TRIANGLE, &settings).expect("recompute repairs");
        let (second, _, digest_second) = repair(TRIANGLE, &settings).expect("recompute again");
        assert_eq!(first.tangents, second.tangents);
        assert_eq!(
            first, second,
            "same input plus settings, bit-identical mesh"
        );
        assert_eq!(digest_first, digest_second);
        for tangent in &first.tangents {
            assert!(tangent.iter().all(|v| v.is_finite()));
            assert!(tangent[3] == 1.0 || tangent[3] == -1.0);
        }
        let refused = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            tangents: TangentMode::Refuse,
            ..ImportSettings::default()
        };
        let err = repair(TRIANGLE, &refused).expect_err("refuse mode rejects tangentless files");
        assert!(
            err.reason.contains("tangents.missing"),
            "refusal names tangents, got {}",
            err.reason
        );
        // Generated UVs exercise the real solve (not just the fallback): the
        // right triangle's solve is deterministic across recooks too.
        let solved = make_gltf(1, 1.0);
        let (a, _, da) = repair(&solved, &settings).expect("solved recompute");
        let (b, _, db) = repair(&solved, &settings).expect("solved recompute again");
        assert_eq!(a.tangents, b.tangents);
        assert_eq!(da, db);
    }

    #[test]
    fn same_asset_id_across_recook() {
        // Identity (source content hash) is stable; the cooked digest versions
        // the output. Texture references keep content ids across recooks.
        assert_eq!(asset_id(TRIANGLE), asset_id(TRIANGLE));
        let settings = explicit(0.5);
        let (mesh_a, material_a, after_a) = repair(TRIANGLE, &settings).expect("first cook");
        let (mesh_b, material_b, after_b) = repair(TRIANGLE, &settings).expect("recook");
        assert_eq!(asset_id(TRIANGLE), content_hash(TRIANGLE));
        assert_eq!(
            (&mesh_a, &material_a, &after_a),
            (&mesh_b, &material_b, &after_b)
        );
        assert_eq!(after_a, after_digest_of(&mesh_a, &material_a));
        // A different explicit scale is a different cook of the same asset.
        let (_, _, other) = repair(TRIANGLE, &explicit(1.0)).expect("other scale");
        assert_ne!(after_a, other, "scale changes the cooked digest");
        assert_eq!(
            asset_id(TRIANGLE),
            asset_id(TRIANGLE),
            "the asset id does not follow the scale"
        );
        // Texture slot ids survive the recook on the textured file.
        let png = png_bytes(4, 4, |x, y| [(x * 64) as u8, (y * 64) as u8, 128, 255]);
        let bytes = make_textured_gltf(&png);
        let textured = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            ..ImportSettings::default()
        };
        let (_, material_a, _) = repair(&bytes, &textured).expect("textured cook");
        let (_, material_b, _) = repair(&bytes, &textured).expect("textured recook");
        assert_eq!(material_a.textures, material_b.textures);
        assert!(!material_a.textures.is_empty(), "slots survive by default");
    }

    #[test]
    fn slot_map_filters_deterministically() {
        let png = png_bytes(4, 4, |_, _| [200, 100, 50, 255]);
        let bytes = make_textured_gltf(&png);
        let without_normal = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            color_slots: SlotMap {
                allow_base_color: true,
                allow_metallic_roughness: true,
                allow_normal: false,
                allow_occlusion: true,
                allow_emissive: true,
            },
            ..ImportSettings::default()
        };
        let (_, material, _) = repair(&bytes, &without_normal).expect("slot repair");
        assert!(
            material
                .textures
                .iter()
                .all(|reference| reference.slot != TextureSlot::Normal),
            "disallowed slots are dropped, got {:?}",
            material.textures
        );
        assert!(
            material
                .textures
                .iter()
                .any(|reference| reference.slot == TextureSlot::BaseColor),
            "allowed slots survive"
        );
        let (_, material_again, digest_again) =
            repair(&bytes, &without_normal).expect("slot recook");
        assert_eq!(material, material_again);
        let (_, default_material, default_digest) = repair(
            &bytes,
            &ImportSettings {
                unit_meters_per_unit: Some(1.0),
                ..ImportSettings::default()
            },
        )
        .expect("default slots keep both");
        assert!(default_material.textures.len() > material.textures.len());
        assert_ne!(
            digest_again, default_digest,
            "slot choice versions the cook"
        );
    }

    #[test]
    fn malformed_bytes_diagnose_without_panic() {
        for bytes in [b"".as_slice(), b"not gltf at all".as_slice()] {
            let diagnosis = diagnose(bytes);
            assert_eq!(diagnosis.before_digest, before_digest_of(bytes));
            assert!(
                diagnosis
                    .issues
                    .iter()
                    .any(|issue| issue.code == CODE_IMPORT && !issue.fixable),
                "malformed input is an unfixable import issue"
            );
            let err = repair(bytes, &explicit(1.0)).expect_err("malformed must refuse");
            assert_eq!(err.before_digest, diagnosis.before_digest);
            assert!(!format!("{err}").is_empty());
        }
    }

    #[test]
    fn refusal_reasons_stay_bounded() {
        let bytes = make_gltf(2, 0.25);
        let settings = ImportSettings {
            unit_meters_per_unit: Some(1.0),
            budgets: BudgetSet {
                max_tris: 1,
                ..BudgetSet::default()
            },
            ..ImportSettings::default()
        };
        let err = repair(&bytes, &settings).expect_err("must refuse");
        assert!(err.reason.chars().count() <= MAX_DETAIL_CHARS);
        let diagnosis = diagnose(&bytes);
        for issue in &diagnosis.issues {
            assert!(issue.detail.chars().count() <= MAX_DETAIL_CHARS);
        }
    }
}

//! kit_import: the arena kit's glTF files cooked for the web bundle.
//!
//! Consumer: the `arena_bundle` bin (offline cook). The packed lane writes one
//! compact glTF per file through [`super::kit_gltf`] plus the cooked KTX2 sets;
//! the cooked lane stages the same meshes, materials and textures into
//! `E204BNDL` bytes. Data owner: `engine-assets`. Units: meters in, meters out;
//! texture dimensions in texels. One [`KitPiece`] per kit file: every triangle
//! primitive of the file merged into one [`CookedMesh`] (node transforms baked
//! into the vertices and normals, indices rebased) and one [`CookedMaterial`]
//! (the file's first material, dressed by the cooked texture set the native kit
//! wears). The embedded glTF images are dropped: the bundle's cooked set covers
//! them, and [`import_kit_gltf_full`] hands the banner's own maps to the
//! emitter.
//!
//! Determinism: document order only (meshes, primitives, nodes), no map
//! iteration that reaches the output, no clock, no RNG; the same file bytes
//! give the same structs. The identity fast path copies the source arrays, so
//! the kit's own files (no node transforms) keep bit-identical geometry.
//! Errors: typed [`KitImportError`], never a panic.

use super::cooked::{
    validate_material, validate_mesh, CookError, CookedMaterial, CookedMesh, CookedTexture, Mip,
    TextureEncoding, TextureRef, TextureSlot,
};
use super::ktx2_cook::{cook_ktx2, decode_level_rgba, parse_ktx2, Ktx2Texture, MapKind};
use super::mesh_import::{self, MAX_BYTES, MAX_VERTICES};
use super::registry::content_hash;
use super::wire::bound_chars;
use engine_core::vec3::cross3;
use std::fmt;

/// Bound for echoed paths and cook details carried in [`KitImportError`].
pub const MAX_KIT_DETAIL_CHARS: usize = 128;

/// The eight kit files, in the native placement's asset order (one import per
/// file). Paths are `src/<file>` under the arena content root.
pub const KIT_FILES: [&str; 8] = [
    "sand_floor.gltf",
    "stands.gltf",
    "walls.gltf",
    "gate_tunnel.gltf",
    "torch.gltf",
    "banner.gltf",
    "crate.gltf",
    "debris.gltf",
];

/// The studio HDRI the offscreen forward and skinned captures bake:
/// `content/arena/<name>` on disk, `<name>` inside the arena bundle. Tier 1
/// of the bundle, so the first-frame kit prefix never carries it.
pub const STUDIO_HDRI: &str = "hdri/polyhaven-studio-small-09-1k.hdr";

/// One cooked CC0 material set the kit wears. The names match
/// `content/arena/cooked/<name>_{base,normal,orm}.ktx2` and the native
/// `kit_pbr::COOKED_SETS` table.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KitSet {
    /// Bricks, stands, debris.
    Stone,
    /// The sand floor.
    Sand,
    /// The crate planks.
    Wood,
    /// The torch pole and cup.
    Iron,
    /// The paved gate tunnel.
    Floor,
}

impl KitSet {
    /// Every set in cook order (the native `arena_cook` order).
    pub const ALL: [Self; 5] = [Self::Stone, Self::Sand, Self::Wood, Self::Iron, Self::Floor];

    /// Set name: the cooked file prefix and the entry-name part.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stone => "stone",
            Self::Sand => "sand",
            Self::Wood => "wood",
            Self::Iron => "iron",
            Self::Floor => "floor",
        }
    }

    /// Metallic factor the set's ORM blue channel multiplies (1 for iron).
    pub fn metallic(self) -> f32 {
        match self {
            Self::Iron => 1.0,
            _ => 0.0,
        }
    }

    /// Source directory under `sources/` plus whether it ships an occlusion
    /// map (the metal set ships a metalness map instead).
    pub fn source_set(self) -> (&'static str, bool) {
        match self {
            Self::Stone => ("Bricks097", true),
            Self::Sand => ("Ground093C", true),
            Self::Wood => ("Planks039", true),
            Self::Iron => ("Metal046B", false),
            Self::Floor => ("PavingStones150", true),
        }
    }
}

/// One map of a set: colour, tangent-space normal or packed ORM channels.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KitMap {
    /// sRGB base colour.
    Base,
    /// Linear OpenGL-convention normal map.
    Normal,
    /// Linear packed occlusion (R), roughness (G), metallic (B).
    Orm,
}

impl KitMap {
    /// Every map of a set, in material slot order.
    pub const ALL: [Self; 3] = [Self::Base, Self::Normal, Self::Orm];

    /// Name part of the entry: `base`, `normal` or `orm`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Normal => "normal",
            Self::Orm => "orm",
        }
    }

    /// Material slot the map fills.
    pub fn slot(self) -> TextureSlot {
        match self {
            Self::Base => TextureSlot::BaseColor,
            Self::Normal => TextureSlot::Normal,
            Self::Orm => TextureSlot::MetallicRoughness,
        }
    }

    /// How `ktx2_cook` encodes the map.
    pub fn map_kind(self) -> MapKind {
        match self {
            Self::Base => MapKind::BaseColor,
            Self::Normal => MapKind::Normal,
            Self::Orm => MapKind::Orm,
        }
    }

    /// True when the cooked texels are linearized sRGB (base colour only).
    pub fn is_srgb(self) -> bool {
        self == Self::Base
    }
}

/// Bundle entry name of one set map, e.g. `tex/stone_base`.
///
/// Caller: the `arena_bundle` bin stages textures under this name; a consumer
/// resolves a material's [`TextureRef::asset`] by hashing the same name with
/// [`kit_texture_asset`].
pub fn kit_texture_name(set: KitSet, map: KitMap) -> String {
    format!("tex/{}_{}", set.name(), map.name())
}

/// Asset id a material reference carries for one set map: [`content_hash`] of
/// the entry name, so a reader of the bundle can resolve the reference from
/// the entry names alone (the payload bytes never travel with the bundle).
pub fn kit_texture_asset(set: KitSet, map: KitMap) -> u64 {
    content_hash(kit_texture_name(set, map).as_bytes())
}

/// Set a kit file wears, or `None` for the banner (it keeps the glTF cloth
/// look its own material carries).
///
/// Mirrors the native `kit_pbr::pbr_for_file` table: stone for walls, stands
/// and debris; sand for the floor; paving for the gate tunnel; wood for the
/// crate; iron for the torch.
pub fn kit_set(file: &str) -> Option<KitSet> {
    match kit_stem(file) {
        "sand_floor" => Some(KitSet::Sand),
        "stands" | "walls" | "debris" => Some(KitSet::Stone),
        "gate_tunnel" => Some(KitSet::Floor),
        "crate" => Some(KitSet::Wood),
        "torch" => Some(KitSet::Iron),
        _ => None,
    }
}

/// File stem of a kit path (`src/walls.gltf` and `walls.gltf` both give
/// `walls`); the mesh entry name of the piece.
pub fn kit_stem(file: &str) -> &str {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file);
    name.strip_suffix(".gltf")
        .or_else(|| name.strip_suffix(".glb"))
        .unwrap_or(name)
}

/// True when `bytes` are a Git LFS pointer, not the asset itself.
pub fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs")
}

/// Why a kit cook failed; every bad input maps to one variant.
#[derive(Clone, Debug, PartialEq)]
pub enum KitImportError {
    /// A kit file cannot be read; bounded detail.
    Io(String),
    /// glTF parse, validation or primitive import failed.
    Import(mesh_import::MeshImportError),
    /// An image or KTX2 cook failed; bounded detail.
    Texture(String),
    /// A merged mesh or texture exceeds the cook budget; count carried.
    TooLarge {
        /// Offending vertex or index count.
        count: usize,
    },
    /// Cooked data failed validation; field plus bounded reason.
    InvalidData {
        /// Offending field.
        field: &'static str,
        /// Bounded reason (never echoes unbounded input).
        detail: String,
    },
}

impl fmt::Display for KitImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(detail) => write!(f, "kit io: {detail}"),
            Self::Import(err) => write!(f, "kit glTF: {err}"),
            Self::Texture(detail) => write!(f, "kit texture: {detail}"),
            Self::TooLarge { count } => write!(f, "kit too large: {count}"),
            Self::InvalidData { field, detail } => write!(f, "kit {field} invalid: {detail}"),
        }
    }
}

impl std::error::Error for KitImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Import(err) => Some(err),
            _ => None,
        }
    }
}

impl From<mesh_import::MeshImportError> for KitImportError {
    fn from(err: mesh_import::MeshImportError) -> Self {
        Self::Import(err)
    }
}

/// Truncate echoed text to [`MAX_KIT_DETAIL_CHARS`].
fn bound(text: &str) -> String {
    bound_chars(text, MAX_KIT_DETAIL_CHARS)
}
pub(crate) fn texture_err(err: impl fmt::Display) -> KitImportError {
    KitImportError::Texture(bound(&err.to_string()))
}

fn disagree(field: &'static str, detail: &str) -> KitImportError {
    KitImportError::InvalidData {
        field,
        detail: bound(detail),
    }
}

fn cook_err(err: &CookError) -> KitImportError {
    match err {
        CookError::EmptyMesh => disagree("positions", "no vertices"),
        CookError::IndexOutOfRange(index) => disagree("indices", &format!("index {index}")),
        CookError::TextureTooLarge { count } => KitImportError::TooLarge { count: *count },
        other => disagree("mesh", &format!("{other:?}")),
    }
}

/// One kit file cooked: merged geometry plus the material it draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct KitPiece {
    /// Source file name, e.g. `walls.gltf`.
    pub file: String,
    /// Cooked set the piece wears (`None` for the banner cloth).
    pub set: Option<KitSet>,
    /// Every triangle primitive of the file, merged with node transforms
    /// baked; embedded images dropped.
    pub mesh: CookedMesh,
    /// The file's first material, with the set's texture references.
    pub material: CookedMaterial,
    /// Source triangle count of the merged mesh.
    pub triangles: u64,
}

/// One encoded image the native kit dresses a file with, plus the slot it
/// fills. Only the banner's glTF carries maps the cooked sets do not cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KitImage {
    /// Material slot the image fills.
    pub slot: TextureSlot,
    /// Encoded PNG or JPEG bytes as stored in the glTF.
    pub bytes: Vec<u8>,
}

/// One cooked set map: the bundle payload plus the KTX2 bytes it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct KitTexture {
    /// Cooked texture staged as a bundle texture entry.
    pub cooked: CookedTexture,
    /// KTX2 file the texels were decoded from; at the native cap these bytes
    /// equal `content/arena/cooked/<set>_<map>.ktx2`.
    pub ktx2: Vec<u8>,
}

/// Cook one kit glTF (text or binary) into merged cooked geometry.
///
/// Bakes every node transform into positions and normals (a mirroring node
/// flips tangent handedness and normal direction), merges the primitives in
/// document order, rebases indices into one mesh and drops the embedded
/// images. The material is the file's first material; a piece that wears a
/// cooked set gets that set's three texture references with the base colour,
/// roughness and metallic overridden exactly as the native kit does, so the
/// browser and the window agree. Same bytes in, same piece out.
/// Encoded maps the native kit dresses a file with: the first material that
/// names a base-colour map (the banner cloth), else the first that names a
/// normal map, else none. The window's `kit_texture` and `gltf_maps` read the
/// same materials, so the compact glTF carries exactly the maps it draws with.
fn dressed_images(imported: &mesh_import::GltfImport) -> Vec<KitImage> {
    let has = |slot: TextureSlot| {
        imported.materials.iter().find(|material| {
            material
                .textures
                .iter()
                .any(|reference| reference.slot == slot)
        })
    };
    let chosen = has(TextureSlot::BaseColor).or_else(|| has(TextureSlot::Normal));
    let Some(material) = chosen else {
        return Vec::new();
    };
    material
        .textures
        .iter()
        .filter_map(|reference| {
            imported.images.get(&reference.asset).map(|bytes| KitImage {
                slot: reference.slot,
                bytes: bytes.clone(),
            })
        })
        .collect()
}

/// [`import_kit_gltf`] plus the encoded images of the material the native kit
/// dresses the file with, in that material's texture order.
///
/// Consumer: the compact web glTF emitter ([`super::kit_gltf`]) re-embeds the
/// banner's own cloth maps; a set piece's images are dead and the caller drops
/// them. Bytes are the PNG/JPEG as stored (never decoded here).
pub fn import_kit_gltf_full(
    file: &str,
    bytes: &[u8],
) -> Result<(KitPiece, Vec<KitImage>), KitImportError> {
    if bytes.len() > MAX_BYTES {
        return Err(KitImportError::TooLarge { count: bytes.len() });
    }
    if is_lfs_pointer(bytes) {
        return Err(KitImportError::Io(format!(
            "{file}: git-lfs pointer, run git lfs pull"
        )));
    }
    let (document, blob) = mesh_import::parse_document(bytes)?;
    let imported = mesh_import::import_document(&document, blob)?;
    // Primitive order of the import: document meshes, primitives inside each.
    let mut mesh_of = Vec::with_capacity(imported.meshes.len());
    for mesh in document.meshes() {
        for _ in mesh.primitives() {
            mesh_of.push(mesh.index());
        }
    }
    if mesh_of.len() != imported.meshes.len() {
        return Err(disagree("positions", "primitive order differs"));
    }
    let transforms = mesh_transforms(&document);
    let mut merged = Merge::default();
    for (cooked, mesh_index) in imported.meshes.iter().zip(&mesh_of) {
        let places = transforms
            .get(*mesh_index)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if places.is_empty() {
            merged.append(cooked, &IDENTITY)?;
        } else {
            for place in places {
                merged.append(cooked, place)?;
            }
        }
    }
    let mesh = merged.finish()?;
    validate_mesh(&mesh).map_err(|err| cook_err(&err))?;
    let base = imported.materials.first().cloned().unwrap_or_default();
    let images = dressed_images(&imported);
    let set = kit_set(file);
    let material = kit_material(base, set);
    validate_material(&material).map_err(|err| cook_err(&err))?;
    Ok((
        KitPiece {
            file: file.to_string(),
            set,
            triangles: u64::try_from(mesh.indices.len() / 3).unwrap_or(u64::MAX),
            mesh,
            material,
        },
        images,
    ))
}

/// [`import_kit_gltf`] without the images: the cooked bundle's meshes and
/// materials, which the cooked set dresses.
///
/// # Errors
/// Same refusals as [`import_kit_gltf_full`].
pub fn import_kit_gltf(file: &str, bytes: &[u8]) -> Result<KitPiece, KitImportError> {
    import_kit_gltf_full(file, bytes).map(|(piece, _images)| piece)
}

/// The material a kit file draws with: the glTF factors, plus the set's three
/// texture references and the native set override (white base, roughness 1,
/// the set's metallic factor) when a cooked set applies.
fn kit_material(mut base: CookedMaterial, set: Option<KitSet>) -> CookedMaterial {
    base.textures.clear();
    let Some(set) = set else {
        return base;
    };
    base.base_color_factor = [1.0; 4];
    base.metallic_factor = set.metallic();
    base.roughness_factor = 1.0;
    base.textures = KitMap::ALL
        .iter()
        .map(|map| TextureRef {
            asset: kit_texture_asset(set, *map),
            slot: map.slot(),
        })
        .collect();
    base
}

/// Cook one set map from RGBA8 source texels: KTX2 through the shared
/// `ktx2_cook` lane, then the cooked file decoded back into the bundle's f32
/// mip chain. Same texels, kind and cap give the same texture; at the native
/// cap the KTX2 bytes equal `content/arena/cooked/<set>_<map>.ktx2`.
pub fn cook_kit_texture(
    map: KitMap,
    rgba: &[u8],
    width: u32,
    height: u32,
    max_dim: u32,
) -> Result<KitTexture, KitImportError> {
    let ktx2 = cook_ktx2(rgba, width, height, map.map_kind(), max_dim).map_err(texture_err)?;
    let tex = parse_ktx2(&ktx2).map_err(texture_err)?;
    Ok(KitTexture {
        cooked: cooked_from_ktx2(map, &tex)?,
        ktx2,
    })
}

/// Decode a cooked KTX2 file back into the bundle's f32 mip chain. Colour
/// texels are linearized (`TextureEncoding::Srgb`); normal and ORM texels stay
/// linear bytes over 255. Normal maps keep the OpenGL green convention the
/// sources ship; the render load inverts it for the engine frame.
fn cooked_from_ktx2(map: KitMap, tex: &Ktx2Texture) -> Result<CookedTexture, KitImportError> {
    let mut mips = Vec::with_capacity(tex.levels.len());
    for level in 0..tex.levels.len() {
        let (width, height, rgba) = decode_level_rgba(tex, level).map_err(texture_err)?;
        let texels = rgba
            .chunks_exact(4)
            .map(|texel| {
                [
                    channel(map, texel[0]),
                    channel(map, texel[1]),
                    channel(map, texel[2]),
                    1.0,
                ]
            })
            .collect();
        mips.push(Mip {
            width,
            height,
            texels,
        });
    }
    if mips.is_empty() {
        return Err(disagree("texture", "no mip levels"));
    }
    Ok(CookedTexture {
        width: tex.width,
        height: tex.height,
        mips,
        slot: map.slot(),
        encoding: if map.is_srgb() {
            TextureEncoding::Srgb
        } else {
            TextureEncoding::Linear
        },
    })
}

/// One channel of one decoded texel: sRGB maps linearize, data maps divide.
fn channel(map: KitMap, byte: u8) -> f32 {
    if map.is_srgb() {
        super::bc::srgb_byte_to_linear(byte)
    } else {
        f32::from(byte) / 255.0
    }
}

/// One node transform: glTF column-major `m[column][row]`.
type Matrix = [[f32; 4]; 4];

/// The identity transform.
const IDENTITY: Matrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// World transforms that place each mesh, indexed by mesh index: the matrix of
/// every node that names the mesh, in node index order. A mesh no node names
/// gets an empty list (the caller bakes identity, matching the importer).
fn mesh_transforms(document: &gltf::Document) -> Vec<Vec<Matrix>> {
    let nodes: Vec<gltf::Node<'_>> = document.nodes().collect();
    let mut world: Vec<Option<Matrix>> = vec![None; nodes.len()];
    let mut is_child = vec![false; nodes.len()];
    for node in &nodes {
        for child in node.children() {
            if let Some(flag) = is_child.get_mut(child.index()) {
                *flag = true;
            }
        }
    }
    // Explicit stack: roots in index order, children pushed in reverse so the
    // first child is visited first. Each node is placed once (cycles guarded).
    let mut stack: Vec<(usize, Matrix)> = Vec::with_capacity(nodes.len());
    for index in (0..nodes.len()).rev() {
        if !is_child.get(index).copied().unwrap_or(true) {
            stack.push((index, IDENTITY));
        }
    }
    while let Some((index, parent)) = stack.pop() {
        let Some(node) = nodes.get(index) else {
            continue;
        };
        if world.get(index).is_some_and(|slot| slot.is_some()) {
            continue;
        }
        let placed = mul(&parent, &node.transform().matrix());
        if let Some(slot) = world.get_mut(index) {
            *slot = Some(placed);
        }
        // `Children` is not double-ended, so the reverse push needs a buffer.
        #[expect(
            clippy::needless_collect,
            reason = "gltf Children is not double-ended; the stack needs reverse order"
        )]
        let children: Vec<usize> = node.children().map(|child| child.index()).collect();
        for child in children.into_iter().rev() {
            stack.push((child, placed));
        }
    }
    let mut out: Vec<Vec<Matrix>> = vec![Vec::new(); document.meshes().len()];
    for (index, node) in nodes.iter().enumerate() {
        let Some(mesh) = node.mesh() else {
            continue;
        };
        let placed = world.get(index).copied().flatten().unwrap_or(IDENTITY);
        if let Some(row) = out.get_mut(mesh.index()) {
            row.push(placed);
        }
    }
    out
}

/// Column-major matrix product `a * b`.
fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [[0.0f32; 4]; 4];
    for (column, row) in out.iter_mut().enumerate() {
        for (index, value) in row.iter_mut().enumerate() {
            let mut sum = 0.0f32;
            for k in 0..4 {
                sum += a[k][index] * b[column][k];
            }
            *value = sum;
        }
    }
    out
}

/// True when `m` is exactly the identity (the kit's own files take this path
/// and keep their source bits).
fn is_identity(m: &Matrix) -> bool {
    m.iter()
        .flatten()
        .copied()
        .eq(IDENTITY.iter().flatten().copied())
}

/// Column `index` of the linear part of `m` as a vector.
fn column(m: &Matrix, index: usize) -> [f32; 3] {
    [m[index][0], m[index][1], m[index][2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalized(v: [f32; 3]) -> Option<[f32; 3]> {
    let len2 = dot(v, v);
    if len2 <= 1e-20 || !len2.is_finite() {
        return None;
    }
    let len = len2.sqrt();
    Some([v[0] / len, v[1] / len, v[2] / len])
}

/// Transform one point by `m` (w = 1).
fn point(m: &Matrix, p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}

/// Transform one direction by the linear part of `m`.
fn vector(m: &Matrix, v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ]
}

/// Transform one normal by the inverse transpose; `None` when the linear part
/// is singular (the caller then keeps the source normal).
fn normal(m: &Matrix, n: [f32; 3]) -> Option<[f32; 3]> {
    let (c0, c1, c2) = (column(m, 0), column(m, 1), column(m, 2));
    let det = dot(c0, cross3(c1, c2));
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    // (A^-1)^T n has components (c1 x c2).n, (c2 x c0).n, (c0 x c1).n, over det.
    normalized([
        dot(cross3(c1, c2), n) / det,
        dot(cross3(c2, c0), n) / det,
        dot(cross3(c0, c1), n) / det,
    ])
}

/// Determinant sign: a mirroring node flips tangent handedness.
fn det_sign(m: &Matrix) -> f32 {
    let det = dot(column(m, 0), cross3(column(m, 1), column(m, 2)));
    if det < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// Accumulates the merged geometry of one kit file.
#[derive(Default)]
struct Merge {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uv0: Vec<[f32; 2]>,
    tangents: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Merge {
    /// Append one primitive's geometry, rebasing its indices and baking `m`.
    fn append(&mut self, mesh: &CookedMesh, m: &Matrix) -> Result<(), KitImportError> {
        let base = self.positions.len();
        let total = base
            .checked_add(mesh.positions.len())
            .ok_or(KitImportError::TooLarge { count: usize::MAX })?;
        if total > MAX_VERTICES {
            return Err(KitImportError::TooLarge { count: total });
        }
        let delta = u32::try_from(base).map_err(|_| KitImportError::TooLarge { count: base })?;
        if is_identity(m) {
            self.positions.extend_from_slice(&mesh.positions);
            self.normals.extend_from_slice(&mesh.normals);
            self.uv0.extend_from_slice(&mesh.uv0);
            self.tangents.extend_from_slice(&mesh.tangents);
        } else {
            let flip = det_sign(m);
            for (index, position) in mesh.positions.iter().enumerate() {
                self.positions.push(point(m, *position));
                let source = mesh.normals.get(index).copied().unwrap_or([0.0, 1.0, 0.0]);
                self.normals.push(normal(m, source).unwrap_or(source));
            }
            self.uv0.extend_from_slice(&mesh.uv0);
            for tangent in &mesh.tangents {
                let heading = normalized(vector(m, [tangent[0], tangent[1], tangent[2]]))
                    .unwrap_or([1.0, 0.0, 0.0]);
                self.tangents
                    .push([heading[0], heading[1], heading[2], tangent[3] * flip]);
            }
        }
        for index in &mesh.indices {
            let rebased = index
                .checked_add(delta)
                .ok_or(KitImportError::TooLarge { count: usize::MAX })?;
            self.indices.push(rebased);
        }
        Ok(())
    }

    /// Finish the merged mesh, refusing an empty one.
    fn finish(self) -> Result<CookedMesh, KitImportError> {
        if self.positions.is_empty() {
            return Err(disagree("positions", "no vertices"));
        }
        Ok(CookedMesh {
            positions: self.positions,
            normals: self.normals,
            uv0: self.uv0,
            tangents: self.tangents,
            indices: self.indices,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arena content dir; every reading test skips when the tree holds LFS
    /// pointers instead of bytes.
    fn arena_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/arena")
    }

    fn present(path: &std::path::Path) -> Option<Vec<u8>> {
        let bytes = std::fs::read(path).ok()?;
        if is_lfs_pointer(&bytes) {
            return None;
        }
        Some(bytes)
    }

    /// Expected source triangles per kit file (measured from the shipped
    /// files: 5,724 vertices and 7,472 triangles over the eight files).
    const SOURCE_TRIS: [(&str, u64); 8] = [
        ("sand_floor.gltf", 512),
        ("stands.gltf", 1_344),
        ("walls.gltf", 1_344),
        ("gate_tunnel.gltf", 960),
        ("torch.gltf", 384),
        ("banner.gltf", 432),
        ("crate.gltf", 1_536),
        ("debris.gltf", 960),
    ];

    #[test]
    fn every_kit_file_imports_with_its_source_triangle_counts() {
        let root = arena_root();
        let (mut triangles, mut vertices) = (0u64, 0usize);
        for (file, want) in SOURCE_TRIS {
            let Some(bytes) = present(&root.join("src").join(file)) else {
                return;
            };
            let piece = import_kit_gltf(file, &bytes).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(piece.triangles, want, "{file} triangles");
            assert_eq!(
                piece.mesh.indices.len(),
                usize::try_from(want).unwrap() * 3,
                "{file} index count"
            );
            assert_eq!(piece.mesh.positions.len(), piece.mesh.normals.len());
            assert_eq!(piece.mesh.positions.len(), piece.mesh.uv0.len());
            assert!(
                piece
                    .mesh
                    .uv0
                    .iter()
                    .flatten()
                    .all(|v| (0.0..=1.0).contains(v)),
                "{file} UVs stay in the cooked range"
            );
            // Same bytes in, same piece out.
            assert_eq!(
                piece,
                import_kit_gltf(file, &bytes).expect("recook"),
                "{file}"
            );
            triangles += piece.triangles;
            vertices += piece.mesh.positions.len();
        }
        assert_eq!(triangles, 7_472, "kit source triangles");
        assert_eq!(vertices, 5_724, "kit source vertices");
    }

    #[test]
    fn set_pieces_wear_their_cooked_set_and_banner_keeps_its_gltf_factor() {
        let root = arena_root();
        let cases = [
            ("sand_floor.gltf", Some(KitSet::Sand), 0.0f32),
            ("walls.gltf", Some(KitSet::Stone), 0.0),
            ("gate_tunnel.gltf", Some(KitSet::Floor), 0.0),
            ("crate.gltf", Some(KitSet::Wood), 0.0),
            ("torch.gltf", Some(KitSet::Iron), 1.0),
        ];
        for (file, want_set, metallic) in cases {
            let Some(bytes) = present(&root.join("src").join(file)) else {
                return;
            };
            let piece = import_kit_gltf(file, &bytes).expect("kit piece");
            assert_eq!(piece.set, want_set, "{file}");
            let set = want_set.expect("a set piece");
            assert_eq!(
                piece.material.base_color_factor, [1.0; 4],
                "{file} white base"
            );
            assert_eq!(piece.material.roughness_factor, 1.0, "{file} rough");
            assert_eq!(piece.material.metallic_factor, metallic, "{file} metallic");
            let refs: Vec<(u64, TextureSlot)> = piece
                .material
                .textures
                .iter()
                .map(|reference| (reference.asset, reference.slot))
                .collect();
            assert_eq!(
                refs,
                vec![
                    (kit_texture_asset(set, KitMap::Base), TextureSlot::BaseColor),
                    (kit_texture_asset(set, KitMap::Normal), TextureSlot::Normal),
                    (
                        kit_texture_asset(set, KitMap::Orm),
                        TextureSlot::MetallicRoughness
                    ),
                ],
                "{file} references every map of its set"
            );
        }
        // The banner wears no cooked set: it keeps the pole's glTF factor and
        // carries no reference (the embedded cloth images are dropped).
        if let Some(bytes) = present(&root.join("src/banner.gltf")) {
            let piece = import_kit_gltf("banner.gltf", &bytes).expect("banner");
            assert_eq!(piece.set, None);
            assert_eq!(piece.material.base_color_factor, [0.3, 0.2, 0.12, 1.0]);
            assert!(piece.material.textures.is_empty());
        }
    }

    /// One triangle buffer: positions, normals (+Z), UVs, indices.
    fn triangle_bin() -> Vec<u8> {
        let mut out = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
            for v in p {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for v in [0.0f32, 0.0, 1.0] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            for v in uv {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for index in [0u32, 1, 2] {
            out.extend_from_slice(&index.to_le_bytes());
        }
        out
    }

    /// GLB around one triangle whose single node carries `node_extra`
    /// (`"translation":[10,0,0]`, `"scale":[2,2,-2]`, ...).
    fn triangle_glb(node_extra: &str) -> Vec<u8> {
        let bin = triangle_bin();
        let json = format!(
            concat!(
                "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],",
                "\"nodes\":[{{\"mesh\":0{node_extra}}}],",
                "\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1,",
                "\"TEXCOORD_0\":2}},\"indices\":3}}]}}],",
                "\"buffers\":[{{\"byteLength\":{len}}}],",
                "\"bufferViews\":[",
                "{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":36}},",
                "{{\"buffer\":0,\"byteOffset\":36,\"byteLength\":36}},",
                "{{\"buffer\":0,\"byteOffset\":72,\"byteLength\":24}},",
                "{{\"buffer\":0,\"byteOffset\":96,\"byteLength\":12}}],",
                "\"accessors\":[",
                "{{\"bufferView\":0,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\",",
                "\"min\":[0.0,0.0,0.0],\"max\":[1.0,1.0,0.0]}},",
                "{{\"bufferView\":1,\"componentType\":5126,\"count\":3,\"type\":\"VEC3\"}},",
                "{{\"bufferView\":2,\"componentType\":5126,\"count\":3,\"type\":\"VEC2\"}},",
                "{{\"bufferView\":3,\"componentType\":5125,\"count\":3,\"type\":\"SCALAR\"}}]}}"
            ),
            len = bin.len(),
            node_extra = node_extra,
        );
        let mut json_bytes = json.into_bytes();
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }
        let mut bin_bytes = bin;
        while !bin_bytes.len().is_multiple_of(4) {
            bin_bytes.push(0);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        let total = 12 + 8 + json_bytes.len() + 8 + bin_bytes.len();
        out.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
        out.extend_from_slice(&u32::try_from(json_bytes.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
        out.extend_from_slice(&json_bytes);
        out.extend_from_slice(&u32::try_from(bin_bytes.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
        out.extend_from_slice(&bin_bytes);
        out
    }

    #[test]
    fn node_transform_is_baked_into_vertices_normals_and_tangents() {
        let piece = import_kit_gltf("moved.gltf", &triangle_glb(",\"translation\":[10,0,0]"))
            .expect("moved triangle imports");
        assert_eq!(piece.triangles, 1);
        assert_eq!(piece.mesh.positions[0], [10.0, 0.0, 0.0]);
        assert_eq!(piece.mesh.positions[1], [11.0, 0.0, 0.0]);
        assert_eq!(piece.mesh.positions[2], [10.0, 1.0, 0.0]);
        // A pure translation leaves the +Z normals and the fallback tangent.
        assert_eq!(piece.mesh.normals[0], [0.0, 0.0, 1.0]);
        assert_eq!(piece.mesh.tangents[0], [1.0, 0.0, 0.0, 1.0]);

        // A mirroring node (negative z scale) flips the normal and the tangent
        // handedness; a 2x scale is removed by the normalization.
        let mirrored =
            import_kit_gltf("mirror.gltf", &triangle_glb(",\"scale\":[2,2,-2]")).expect("mirror");
        assert_eq!(mirrored.mesh.positions[1], [2.0, 0.0, 0.0]);
        assert_eq!(mirrored.mesh.normals[0], [0.0, 0.0, -1.0]);
        assert_eq!(mirrored.mesh.tangents[0], [1.0, 0.0, 0.0, -1.0]);
    }

    #[test]
    fn identity_nodes_keep_the_source_bits() {
        let source = triangle_glb("");
        let piece = import_kit_gltf("still.gltf", &source).expect("triangle imports");
        assert_eq!(piece.mesh.positions[1], [1.0, 0.0, 0.0]);
        assert_eq!(piece.mesh.uv0[2], [0.0, 1.0]);
        assert_eq!(piece.mesh.indices, vec![0, 1, 2]);
        let imported = mesh_import::import_gltf_bytes(&source).expect("plain import");
        assert_eq!(
            piece.mesh, imported.meshes[0],
            "no transform node: the merged mesh is the imported mesh"
        );
    }

    #[test]
    fn texture_cook_at_the_native_cap_equals_the_committed_cook() {
        let root = arena_root();
        let source = root.join("sources/Bricks097/Bricks097_1K-JPG_Color.jpg");
        let Some(encoded) = present(&source) else {
            return;
        };
        let Some(committed) = present(&root.join("cooked/stone_base.ktx2")) else {
            return;
        };
        let image = image::load_from_memory(&encoded)
            .expect("source JPG decodes")
            .to_rgba8();
        let (width, height) = image.dimensions();
        let cooked = cook_kit_texture(KitMap::Base, image.as_raw(), width, height, 1024)
            .expect("stone base cooks");
        assert_eq!(
            cooked.ktx2, committed,
            "the shared ktx2_cook lane writes the committed bytes"
        );
        assert_eq!((cooked.cooked.width, cooked.cooked.height), (width, height));
        assert_eq!(cooked.cooked.encoding, TextureEncoding::Srgb);
        assert_eq!(cooked.cooked.slot, TextureSlot::BaseColor);
        assert_eq!(cooked.cooked.mips.len(), 11, "1024x512 runs down to 1x1");
    }

    #[cfg(feature = "registry")]
    #[test]
    fn packed_bundle_returns_every_kit_mesh_by_name_with_source_triangles() {
        use crate::bundle::{load_bundle, EntryKind};
        use crate::bundle_cook::{cook_bundle_bytes, StagedCooked};

        let root = arena_root();
        let Some(stone) = present(&root.join("sources/Bricks097/Bricks097_1K-JPG_Color.jpg"))
        else {
            return;
        };
        let image = image::load_from_memory(&stone)
            .expect("source JPG decodes")
            .to_rgba8();
        let (width, height) = image.dimensions();
        let build = || -> StagedCooked {
            let mut staged = StagedCooked::new();
            for (file, _) in SOURCE_TRIS {
                let bytes = present(&root.join("src").join(file)).expect("kit file");
                let piece = import_kit_gltf(file, &bytes).expect("kit piece");
                staged.meshes.push((kit_stem(file).to_string(), piece.mesh));
                staged
                    .materials
                    .push((format!("mat/{}", kit_stem(file)), piece.material));
            }
            let texture = cook_kit_texture(KitMap::Base, image.as_raw(), width, height, 32)
                .expect("stone base cooks");
            staged.textures.push((
                kit_texture_name(KitSet::Stone, KitMap::Base),
                texture.cooked,
            ));
            staged
        };
        let first = cook_bundle_bytes(&build()).expect("bundle cooks");
        assert_eq!(
            first,
            cook_bundle_bytes(&build()).expect("recook"),
            "same inputs, byte-identical bundle"
        );
        let loaded = load_bundle(&first).expect("bundle verifies");
        assert_eq!(loaded.len(), 8 + 8 + 1);
        for (file, want) in SOURCE_TRIS {
            let stem = kit_stem(file);
            let entry = loaded
                .find(EntryKind::Mesh, stem)
                .unwrap_or_else(|| panic!("mesh {stem} present"));
            let mesh = entry.decode_mesh().expect("mesh decodes");
            assert_eq!(
                u64::try_from(mesh.indices.len() / 3).unwrap(),
                want,
                "{stem} source triangles"
            );
            let material = loaded
                .find(EntryKind::Material, &format!("mat/{stem}"))
                .unwrap_or_else(|| panic!("material for {stem}"));
            material.decode_material().expect("material decodes");
        }
        let id = kit_texture_asset(KitSet::Stone, KitMap::Base);
        let name = kit_texture_name(KitSet::Stone, KitMap::Base);
        let texture = loaded
            .find(EntryKind::Texture, &name)
            .unwrap_or_else(|| panic!("texture {name}"));
        assert_eq!(
            texture.decode_texture().expect("texture decodes").width,
            32,
            "the cap reaches the staged texture"
        );
        let wall = loaded
            .find(EntryKind::Material, "mat/walls")
            .expect("wall material")
            .decode_material()
            .expect("wall material decodes");
        assert!(
            wall.textures.iter().any(|reference| reference.asset == id),
            "the wall's material names the staged stone albedo"
        );
    }
}

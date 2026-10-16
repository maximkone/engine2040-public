//! gen_intake: a generated mesh becomes a game-ready prop, with provenance.
//!
//! Consumer: the `asset.intake` door verb (`engine-endpoint`, feature
//! `intake`). Data owner: `engine-assets`. Input is the GLB an offline image-to-3D
//! runner made (center's `kaggle.mjs run mesh`) plus the provenance the runner
//! recorded: model id, SPDX licence, the prompt or image digest, the seed.
//! Output is one E204BNDL bundle (the LOD chain plus a neutral material), a
//! collision hull, one provenance row, and a cache entry.
//!
//! The steps: licence gate (refuses non-commercial and unknown models before any
//! work), glTF import, repair (weld, degenerate and duplicate triangles, floating
//! specks, unreferenced vertices), pose (up axis, size in metres, base on the
//! floor), simplify to at most [`PROP_MAX_TRIS`] triangles, smooth normals, the
//! prop cook (LOD chain and hulls), the bundle. Offline cook only, never the tick:
//! plain `f32`/`f64` arithmetic with `sqrt` and no trigonometry, fixed iteration
//! orders, so the same GLB and inputs cook the same bytes twice. Units: metres.
//! Errors: typed [`IntakeError`] with a stable [`IntakeError::code`].

use super::bundle_cook::{cook_bundle_bytes, BundleCookError, StagedCooked};
use super::cooked::{CookedMaterial, CookedMesh};
use super::mesh_cook::{
    cook_mesh, lod_name, simplify, tri_count, CollisionHull, MeshClass, MeshCookError,
    MeshCookOutput, PROP_MAX_TRIS,
};
use super::mesh_import::{import_gltf_bytes, MeshImportError, MAX_BYTES};
use engine_scene::ids::content_digest;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

/// Licences a linked or shipped part may carry (the `deny.toml` list).
pub const ALLOWED_SPDX: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "Zlib",
    "ISC",
    "CC0-1.0",
    "Unlicense",
    "0BSD",
    "MIT-0",
    "Unicode-3.0",
    "Unicode-DFS-2016",
];

/// Models and tools refused whatever licence text rides with them: the hunt
/// found weights whose card says MIT but whose pipeline pulls restricted parts.
/// `(lowercase id prefix, reason)`.
pub const RESTRICTED_PREFIXES: &[(&str, &str)] = &[
    (
        "briaai/rmbg",
        "BRIA RMBG weights are non-commercial (CC BY-NC 4.0 or the BRIA licence)",
    ),
    (
        "diso",
        "diso is CC BY-NC 4.0 (differentiable marching cubes)",
    ),
    (
        "tencent-hunyuan/hunyuan3d",
        "Hunyuan3D carries the Tencent community licence (territory limits)",
    ),
    (
        "tencent/hunyuan3d",
        "Hunyuan3D carries the Tencent community licence (territory limits)",
    ),
    ("nvlabs/partpacker", "PartPacker is NVIDIA non-commercial"),
    (
        "buaacyw/meshanything",
        "MeshAnything weights are non-commercial",
    ),
    (
        "black-forest-labs/flux.1-dev",
        "FLUX.1-dev is non-commercial",
    ),
    (
        "black-forest-labs/flux.1-kontext-dev",
        "FLUX.1-Kontext-dev is non-commercial",
    ),
    (
        "tencentarc/pixal3d",
        "Pixal3D weights are gated with territory limits",
    ),
];

/// Longest asset name in chars.
pub const MAX_NAME_CHARS: usize = 64;
/// Version folded into every cache key; bump when the cook changes.
pub const INTAKE_VERSION: u32 = 1;
/// Default prop size: the longest side in millimetres.
pub const DEFAULT_SIZE_MM: u32 = 1000;
/// Smallest and largest accepted size in millimetres.
pub const SIZE_MM_RANGE: (u32, u32) = (10, 60_000);
/// Triangles kept in the largest piece's share before a floating piece is a speck (percent).
const SPECK_PERCENT: usize = 2;
/// Squared triangle area (m^2 of the raw mesh units) at or below which a triangle is degenerate.
const ZERO_AREA: f64 = 1e-24;
/// Simplify attempts; each one aims lower than the last.
const SIMPLIFY_TRIES: u32 = 6;
/// Largest provenance field in chars.
const MAX_FIELD_CHARS: usize = 256;

/// Why an intake was refused or failed.
#[derive(Clone, Debug, PartialEq)]
pub enum IntakeError {
    /// A model or component carries a licence the allow-list refuses.
    LicenseRefused {
        /// Model or component id.
        model: String,
        /// The licence text given.
        license: String,
        /// Why: non-commercial, copyleft or not on the list.
        why: String,
    },
    /// The licence is not an SPDX identifier or expression.
    LicenseNotSpdx {
        /// Model or component id.
        model: String,
        /// The licence text given.
        license: String,
    },
    /// The model or tool is on the restricted list.
    ModelRestricted {
        /// Model or component id.
        model: String,
        /// Why it is refused.
        why: String,
    },
    /// The runner declared a condition nobody accepted.
    ConditionsUnaccepted {
        /// Conditions declared and not accepted.
        missing: Vec<String>,
    },
    /// A provenance field is missing or malformed.
    Provenance {
        /// The field.
        field: &'static str,
        /// What is wrong.
        detail: String,
    },
    /// A setting is outside its range.
    Settings {
        /// The setting.
        field: &'static str,
        /// What is wrong.
        detail: String,
    },
    /// The glTF did not import.
    Import(MeshImportError),
    /// The mesh did not cook.
    Cook(MeshCookError),
    /// After repair nothing cookable is left.
    Empty {
        /// What was removed.
        detail: String,
    },
    /// Simplification could not reach the triangle budget.
    OverBudget {
        /// Triangles left.
        tris: u32,
        /// The budget.
        max: u32,
    },
    /// The bundle did not write.
    Bundle(BundleCookError),
    /// The cache directory failed.
    Cache {
        /// Bounded detail.
        detail: String,
    },
}

impl IntakeError {
    /// Stable machine code for the door.
    pub fn code(&self) -> &'static str {
        match self {
            Self::LicenseRefused { .. } => "license_refused",
            Self::LicenseNotSpdx { .. } => "license_not_spdx",
            Self::ModelRestricted { .. } => "model_restricted",
            Self::ConditionsUnaccepted { .. } => "conditions_unaccepted",
            Self::Provenance { .. } => "bad_provenance",
            Self::Settings { .. } => "bad_settings",
            Self::Import(_) => "import_failed",
            Self::Cook(_) => "cook_failed",
            Self::Empty { .. } => "nothing_left",
            Self::OverBudget { .. } => "over_budget",
            Self::Bundle(_) => "bundle_failed",
            Self::Cache { .. } => "cache_failed",
        }
    }
}

impl fmt::Display for IntakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LicenseRefused {
                model,
                license,
                why,
            } => {
                write!(f, "intake refused `{model}`: licence `{license}` {why}")
            }
            Self::LicenseNotSpdx { model, license } => write!(
                f,
                "intake refused `{model}`: licence `{license}` is not an SPDX identifier"
            ),
            Self::ModelRestricted { model, why } => {
                write!(f, "intake refused `{model}`: {why}")
            }
            Self::ConditionsUnaccepted { missing } => write!(
                f,
                "intake needs these conditions accepted in accept_conditions: {}",
                missing.join("; ")
            ),
            Self::Provenance { field, detail } => {
                write!(f, "intake provenance `{field}`: {detail}")
            }
            Self::Settings { field, detail } => write!(f, "intake setting `{field}`: {detail}"),
            Self::Import(e) => write!(f, "intake import: {e}"),
            Self::Cook(e) => write!(f, "intake cook: {e}"),
            Self::Empty { detail } => write!(f, "intake left no mesh: {detail}"),
            Self::OverBudget { tris, max } => {
                write!(
                    f,
                    "intake could not reach the budget: {tris} tris over {max}"
                )
            }
            Self::Bundle(e) => write!(f, "intake bundle: {e}"),
            Self::Cache { detail } => write!(f, "intake cache: {detail}"),
        }
    }
}

impl std::error::Error for IntakeError {}

impl From<MeshCookError> for IntakeError {
    fn from(error: MeshCookError) -> Self {
        Self::Cook(error)
    }
}

impl From<BundleCookError> for IntakeError {
    fn from(error: BundleCookError) -> Self {
        Self::Bundle(error)
    }
}

/// One model or tool the result depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRef {
    /// Hub id or tool name (`VAST-AI/TripoSG`).
    pub id: String,
    /// SPDX identifier or expression.
    pub license: String,
}

/// What produced the GLB, as the runner recorded it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The shape model.
    pub model: ModelRef,
    /// Cutout, encoder and every other part the output depends on.
    pub components: Vec<ModelRef>,
    /// Lowercase hex digest (16 to 64 digits) of the prompt text or the source image.
    pub source_digest: String,
    /// Seed the runner used.
    pub seed: u64,
    /// Conditions the runner declared (for example a territory limit).
    pub conditions: Vec<String>,
    /// Where the code came from (repository and commit), free text.
    pub code_ref: String,
}

/// Up axis of the generated mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpAxis {
    /// `+Y` is up (glTF).
    Y,
    /// `+Z` is up; the mesh turns to `+Y`.
    Z,
}

impl UpAxis {
    fn tag(self) -> &'static str {
        match self {
            Self::Y => "y",
            Self::Z => "z",
        }
    }
}

/// How the mesh is posed and budgeted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntakeSettings {
    /// Longest side after posing, millimetres.
    pub size_mm: u32,
    /// Up axis of the source mesh.
    pub up_axis: UpAxis,
    /// Triangle budget for LOD0, at most [`PROP_MAX_TRIS`].
    pub max_tris: u32,
}

impl Default for IntakeSettings {
    fn default() -> Self {
        Self {
            size_mm: DEFAULT_SIZE_MM,
            up_axis: UpAxis::Y,
            max_tris: PROP_MAX_TRIS,
        }
    }
}

/// One intake request.
#[derive(Clone, Debug)]
pub struct IntakeRequest<'a> {
    /// Asset name: lowercase letters, digits, `_` and `-`.
    pub name: &'a str,
    /// The GLB (or glTF) bytes.
    pub glb: &'a [u8],
    /// What produced it.
    pub provenance: &'a Provenance,
    /// Conditions the caller accepts.
    pub accepted_conditions: &'a [String],
    /// Posing and budget.
    pub settings: IntakeSettings,
}

/// What repair removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RepairStats {
    /// Vertices merged into an identical one.
    pub welded_vertices: u32,
    /// Triangles with a repeated corner or no area.
    pub degenerate_tris: u32,
    /// Triangles that repeat another triangle.
    pub duplicate_tris: u32,
    /// Triangles in floating specks.
    pub speck_tris: u32,
    /// Floating specks removed.
    pub specks: u32,
}

/// One finished intake.
#[derive(Clone, Debug)]
pub struct IntakeOutput {
    /// Asset name.
    pub name: String,
    /// E204BNDL bytes: `name`, `name_LOD1`.. meshes and `name_mat`.
    pub bundle: Vec<u8>,
    /// Blake3 hex of the bundle bytes.
    pub bundle_hash: String,
    /// The provenance row: one JSON object.
    pub row: String,
    /// Triangles after repair, before simplification.
    pub tris_in: u32,
    /// LOD0 triangles.
    pub tris: u32,
    /// Triangles per LOD.
    pub lod_tris: Vec<u32>,
    /// LOD0 collision hull.
    pub hull: CollisionHull,
    /// The prop cook digest.
    pub cook_digest: u64,
    /// Cache key (blake3 hex).
    pub cache_key: String,
    /// Blake3 hex of the GLB bytes.
    pub glb_digest: String,
    /// True when the cache answered.
    pub cache_hit: bool,
    /// What repair removed.
    pub repair: RepairStats,
}

// ---------------------------------------------------------------------------
// Licence gate.
// ---------------------------------------------------------------------------

fn canonical_spdx(id: &str) -> Option<&'static str> {
    ALLOWED_SPDX
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(id))
}

fn looks_spdx(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | ':'))
}

fn non_commercial_hint(license: &str) -> bool {
    let lower = license.to_ascii_lowercase();
    [
        "non-commercial",
        "noncommercial",
        "-nc",
        "research only",
        "research-only",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
}

/// Check one SPDX identifier or `AND`/`OR` expression against [`ALLOWED_SPDX`]:
/// an `OR` passes when any side passes, an `AND` only when every part passes;
/// a `WITH` exception is read past.
pub fn check_license(model: &str, license: &str) -> Result<(), IntakeError> {
    let spaced = license.replace(['(', ')'], " ");
    let mut tokens: Vec<&str> = Vec::new();
    let mut parts = spaced.split_whitespace();
    while let Some(token) = parts.next() {
        if token.eq_ignore_ascii_case("WITH") {
            parts.next();
        } else {
            tokens.push(token);
        }
    }
    if tokens.is_empty() {
        return Err(IntakeError::LicenseNotSpdx {
            model: model.to_owned(),
            license: license.to_owned(),
        });
    }
    let mut any_group_ok = false;
    let mut group_ok = true;
    for token in tokens {
        if token.eq_ignore_ascii_case("OR") {
            any_group_ok |= group_ok;
            group_ok = true;
        } else if token.eq_ignore_ascii_case("AND") {
            continue;
        } else if !looks_spdx(token) {
            return Err(IntakeError::LicenseNotSpdx {
                model: model.to_owned(),
                license: license.to_owned(),
            });
        } else {
            group_ok &= canonical_spdx(token.trim_end_matches('+')).is_some();
        }
    }
    any_group_ok |= group_ok;
    if any_group_ok {
        return Ok(());
    }
    let why = if non_commercial_hint(license) {
        "is non-commercial".to_owned()
    } else {
        format!("is not on the allow-list ({})", ALLOWED_SPDX.join(", "))
    };
    Err(IntakeError::LicenseRefused {
        model: model.to_owned(),
        license: license.to_owned(),
        why,
    })
}

/// Refuse a model or tool on the restricted list.
pub fn check_model(model: &str) -> Result<(), IntakeError> {
    let lower = model.to_ascii_lowercase();
    match RESTRICTED_PREFIXES
        .iter()
        .find(|(prefix, _)| lower.starts_with(prefix))
    {
        Some((_, why)) => Err(IntakeError::ModelRestricted {
            model: model.to_owned(),
            why: (*why).to_owned(),
        }),
        None => Ok(()),
    }
}

fn check_field(field: &'static str, value: &str) -> Result<(), IntakeError> {
    if value.trim().is_empty() || value.chars().count() > MAX_FIELD_CHARS || value.contains('\n') {
        return Err(IntakeError::Provenance {
            field,
            detail: format!("want 1 to {MAX_FIELD_CHARS} chars on one line"),
        });
    }
    Ok(())
}

/// The whole gate: provenance shape, every model and component (restricted
/// list, then licence), and the declared conditions against the accepted ones.
pub fn check_provenance(provenance: &Provenance, accepted: &[String]) -> Result<(), IntakeError> {
    check_field("model.id", &provenance.model.id)?;
    check_field("model.license", &provenance.model.license)?;
    let digest = &provenance.source_digest;
    if !(16..=64).contains(&digest.len())
        || !digest.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
    {
        return Err(IntakeError::Provenance {
            field: "source_digest",
            detail: "want 16 to 64 lowercase hex digits (digest of the prompt or the image)"
                .to_owned(),
        });
    }
    if !provenance.code_ref.is_empty() {
        check_field("code_ref", &provenance.code_ref)?;
    }
    for part in std::iter::once(&provenance.model).chain(provenance.components.iter()) {
        check_field("component.id", &part.id)?;
        check_field("component.license", &part.license)?;
        check_model(&part.id)?;
        check_license(&part.id, &part.license)?;
    }
    let missing: Vec<String> = provenance
        .conditions
        .iter()
        .filter(|c| !accepted.contains(c))
        .cloned()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(IntakeError::ConditionsUnaccepted { missing })
    }
}

/// Digest of a prompt text, for [`Provenance::source_digest`].
pub fn digest_text(text: &str) -> String {
    content_digest(text.as_bytes())
}

fn check_name(name: &str) -> Result<(), IntakeError> {
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(IntakeError::Settings {
            field: "name",
            detail: format!("want 1 to {MAX_NAME_CHARS} chars of a-z 0-9 _ -"),
        })
    }
}

fn check_settings(settings: &IntakeSettings) -> Result<(), IntakeError> {
    if !(SIZE_MM_RANGE.0..=SIZE_MM_RANGE.1).contains(&settings.size_mm) {
        return Err(IntakeError::Settings {
            field: "size_mm",
            detail: format!("want {} to {}", SIZE_MM_RANGE.0, SIZE_MM_RANGE.1),
        });
    }
    if !(100..=PROP_MAX_TRIS).contains(&settings.max_tris) {
        return Err(IntakeError::Settings {
            field: "max_tris",
            detail: format!("want 100 to {PROP_MAX_TRIS}"),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Cache.
// ---------------------------------------------------------------------------

/// The text a cache key digests: the generation inputs and the settings, not the GLB.
fn key_text(req: &IntakeRequest<'_>) -> String {
    let p = req.provenance;
    let mut parts: Vec<String> = p
        .components
        .iter()
        .map(|c| format!("{}@{}", c.id, c.license))
        .collect();
    parts.sort();
    let mut accepted: Vec<&String> = p.conditions.iter().collect();
    accepted.sort();
    format!(
        "engine2040.asset.intake.v{INTAKE_VERSION}\nname={}\nmodel={}@{}\ncomponents={}\nsource={}\nseed={}\nsize_mm={}\nup={}\nmax_tris={}\nconditions={}\n",
        req.name,
        p.model.id,
        p.model.license,
        parts.join(";"),
        p.source_digest,
        p.seed,
        req.settings.size_mm,
        req.settings.up_axis.tag(),
        req.settings.max_tris,
        accepted.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(";"),
    )
}

/// Cache key for a request: model id, SPDX licence, components, prompt or image
/// digest, seed, name and settings. The GLB bytes are checked on a hit, not keyed.
pub fn cache_key(req: &IntakeRequest<'_>) -> String {
    content_digest(key_text(req).as_bytes())
}

/// Finished intakes by key, in memory and optionally in a folder (`<key>.bundle`,
/// `<key>.row.json`, `<key>.meta`). A folder entry whose bundle does not match its
/// recorded hash is a miss.
#[derive(Clone, Debug, Default)]
pub struct IntakeCache {
    dir: Option<PathBuf>,
    mem: BTreeMap<String, IntakeOutput>,
}

fn cache_err(detail: impl fmt::Display) -> IntakeError {
    IntakeError::Cache {
        detail: detail.to_string().chars().take(160).collect(),
    }
}

fn bits(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

fn parse_bits(text: &str) -> Option<f32> {
    u32::from_str_radix(text, 16).ok().map(f32::from_bits)
}

fn triple(text: &str) -> Option<[f32; 3]> {
    let mut it = text.split(',').map(parse_bits);
    let out = [it.next()??, it.next()??, it.next()??];
    it.next().is_none().then_some(out)
}

fn join_triple(v: &[f32; 3]) -> String {
    format!("{},{},{}", bits(v[0]), bits(v[1]), bits(v[2]))
}

fn meta_text(out: &IntakeOutput) -> String {
    let mut text = String::new();
    let lods: Vec<String> = out.lod_tris.iter().map(u32::to_string).collect();
    let r = &out.repair;
    for (k, v) in [
        ("name", out.name.clone()),
        ("glb", out.glb_digest.clone()),
        ("bundle_hash", out.bundle_hash.clone()),
        ("tris_in", out.tris_in.to_string()),
        ("tris", out.tris.to_string()),
        ("lod_tris", lods.join(",")),
        ("cook_digest", format!("{:016x}", out.cook_digest)),
        (
            "repair",
            format!(
                "{},{},{},{},{}",
                r.welded_vertices, r.degenerate_tris, r.duplicate_tris, r.speck_tris, r.specks
            ),
        ),
        ("hull_min", join_triple(&out.hull.min)),
        ("hull_max", join_triple(&out.hull.max)),
        ("hull_center", join_triple(&out.hull.center)),
        ("hull_radius", bits(out.hull.radius)),
    ] {
        text.push_str(k);
        text.push('=');
        text.push_str(&v);
        text.push('\n');
    }
    for p in &out.hull.support {
        text.push_str("support=");
        text.push_str(&join_triple(p));
        text.push('\n');
    }
    text
}

fn parse_meta(meta: &str, key: &str, bundle: Vec<u8>, row: String) -> Option<IntakeOutput> {
    let mut map: BTreeMap<&str, &str> = BTreeMap::new();
    let mut support = Vec::new();
    for line in meta.lines() {
        let (k, v) = line.split_once('=')?;
        if k == "support" {
            support.push(triple(v)?);
        } else {
            map.insert(k, v);
        }
    }
    let nums: Vec<u32> = map
        .get("repair")?
        .split(',')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let &[welded, degenerate, duplicate, speck_tris, specks] = nums.as_slice() else {
        return None;
    };
    let lod_tris: Vec<u32> = map
        .get("lod_tris")?
        .split(',')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let bundle_hash = (*map.get("bundle_hash")?).to_owned();
    if content_digest(&bundle) != bundle_hash {
        return None;
    }
    Some(IntakeOutput {
        name: (*map.get("name")?).to_owned(),
        bundle,
        bundle_hash,
        row,
        tris_in: map.get("tris_in")?.parse().ok()?,
        tris: map.get("tris")?.parse().ok()?,
        lod_tris,
        hull: CollisionHull {
            min: triple(map.get("hull_min")?)?,
            max: triple(map.get("hull_max")?)?,
            center: triple(map.get("hull_center")?)?,
            radius: parse_bits(map.get("hull_radius")?)?,
            support,
        },
        cook_digest: u64::from_str_radix(map.get("cook_digest")?, 16).ok()?,
        cache_key: key.to_owned(),
        glb_digest: (*map.get("glb")?).to_owned(),
        cache_hit: true,
        repair: RepairStats {
            welded_vertices: welded,
            degenerate_tris: degenerate,
            duplicate_tris: duplicate,
            speck_tris,
            specks,
        },
    })
}

impl IntakeCache {
    /// A cache that lives only as long as this value.
    pub fn memory() -> Self {
        Self::default()
    }

    /// A cache backed by `dir` (created when missing).
    pub fn open(dir: &Path) -> Result<Self, IntakeError> {
        std::fs::create_dir_all(dir).map_err(cache_err)?;
        Ok(Self {
            dir: Some(dir.to_path_buf()),
            mem: BTreeMap::new(),
        })
    }

    /// The entry for `key` (memory first, then the folder), marked as a hit.
    pub fn get(&self, key: &str) -> Option<IntakeOutput> {
        if let Some(found) = self.mem.get(key) {
            return Some(IntakeOutput {
                cache_hit: true,
                ..found.clone()
            });
        }
        let dir = self.dir.as_ref()?;
        // One base join per `get`; the key is hex so `set_extension` appends.
        let base = dir.join(key);
        let mut meta_path = base.clone();
        meta_path.set_extension("meta");
        let meta = std::fs::read_to_string(&meta_path).ok()?;
        let mut bundle_path = base.clone();
        bundle_path.set_extension("bundle");
        let bundle = std::fs::read(&bundle_path).ok()?;
        let mut row_path = base;
        row_path.set_extension("row.json");
        let row = std::fs::read_to_string(&row_path).ok()?;
        parse_meta(&meta, key, bundle, row)
    }

    /// The finished bundle for generation inputs alone (no GLB needed): what
    /// a generate verb asks before it runs the runner again.
    pub fn lookup(&self, req: &IntakeRequest<'_>) -> Option<IntakeOutput> {
        self.get(&cache_key(req))
    }

    fn put(&mut self, out: &IntakeOutput) -> Result<(), IntakeError> {
        if let Some(dir) = &self.dir {
            // Disk holds the bundle; skip `mem` so `put` copies no bytes.
            // One base join; the key is hex so `set_extension` appends.
            let base = dir.join(&out.cache_key);
            let mut bundle_path = base.clone();
            bundle_path.set_extension("bundle");
            std::fs::write(&bundle_path, &out.bundle).map_err(cache_err)?;
            let mut row_path = base.clone();
            row_path.set_extension("row.json");
            std::fs::write(&row_path, &out.row).map_err(cache_err)?;
            let mut meta_path = base;
            meta_path.set_extension("meta");
            std::fs::write(&meta_path, meta_text(out)).map_err(cache_err)?;
            return Ok(());
        }
        self.mem.insert(out.cache_key.clone(), out.clone());
        Ok(())
    }

    /// Entries held in memory.
    pub fn len(&self) -> usize {
        self.mem.len()
    }

    /// True when nothing is held in memory.
    pub fn is_empty(&self) -> bool {
        self.mem.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Repair, pose, normals.
// ---------------------------------------------------------------------------

/// Plain triangle soup with shared vertices, before cooking.
struct Soup {
    pos: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    idx: Vec<u32>,
}

/// Index as `usize` (saturating, so a bad index fails a lookup, never wraps).
fn ix(i: u32) -> usize {
    usize::try_from(i).unwrap_or(usize::MAX)
}

fn to_index(n: usize) -> Result<u32, IntakeError> {
    u32::try_from(n).map_err(|_| IntakeError::Empty {
        detail: "more vertices than an index holds".to_owned(),
    })
}

fn merge(meshes: &[CookedMesh]) -> Result<Soup, IntakeError> {
    let mut soup = Soup {
        pos: Vec::new(),
        uv: Vec::new(),
        idx: Vec::new(),
    };
    for mesh in meshes {
        let base = to_index(soup.pos.len())?;
        soup.pos.extend_from_slice(&mesh.positions);
        soup.uv.extend_from_slice(&mesh.uv0);
        soup.idx.extend(mesh.indices.iter().map(|i| i + base));
    }
    Ok(soup)
}

fn pos_key(p: &[f32; 3]) -> [u32; 3] {
    // `-0.0` and `0.0` are the same place.
    let fix = |v: f32| if v == 0.0 { 0 } else { v.to_bits() };
    [fix(p[0]), fix(p[1]), fix(p[2])]
}

/// Merge vertices at the same position; returns how many went.
fn weld(soup: &mut Soup) -> Result<u32, IntakeError> {
    let mut seen: BTreeMap<[u32; 3], u32> = BTreeMap::new();
    let mut remap = Vec::with_capacity(soup.pos.len());
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    for (i, p) in soup.pos.iter().enumerate() {
        let next = to_index(pos.len())?;
        let slot = *seen.entry(pos_key(p)).or_insert(next);
        if slot == next {
            pos.push(*p);
            uv.push(soup.uv.get(i).copied().unwrap_or([0.0, 0.0]));
        }
        remap.push(slot);
    }
    let merged = soup.pos.len() - pos.len();
    for index in &mut soup.idx {
        *index = remap.get(ix(*index)).copied().unwrap_or(0);
    }
    soup.pos = pos;
    soup.uv = uv;
    u32::try_from(merged).map_err(|_| IntakeError::Empty {
        detail: "weld count overflow".to_owned(),
    })
}

fn tri_area_sq(pos: &[[f32; 3]], tri: &[u32]) -> f64 {
    let at = |i: u32| pos.get(ix(i)).copied().unwrap_or([0.0; 3]);
    let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
    let e1 = [0, 1, 2].map(|k| f64::from(b[k]) - f64::from(a[k]));
    let e2 = [0, 1, 2].map(|k| f64::from(c[k]) - f64::from(a[k]));
    let cross = [
        e1[1] * e2[2] - e1[2] * e2[1],
        e1[2] * e2[0] - e1[0] * e2[2],
        e1[0] * e2[1] - e1[1] * e2[0],
    ];
    cross.iter().map(|v| v * v).sum::<f64>() * 0.25
}

/// Drop degenerate and duplicate triangles; returns `(degenerate, duplicate)`.
fn clean_tris(soup: &mut Soup) -> (u32, u32) {
    let mut seen: BTreeSet<[u32; 3]> = BTreeSet::new();
    let mut kept = Vec::with_capacity(soup.idx.len());
    let (mut degenerate, mut duplicate) = (0u32, 0u32);
    for tri in soup.idx.chunks_exact(3) {
        if tri[0] == tri[1]
            || tri[1] == tri[2]
            || tri[0] == tri[2]
            || tri_area_sq(&soup.pos, tri) <= ZERO_AREA
        {
            degenerate += 1;
            continue;
        }
        let mut sorted = [tri[0], tri[1], tri[2]];
        sorted.sort_unstable();
        if !seen.insert(sorted) {
            duplicate += 1;
            continue;
        }
        kept.extend_from_slice(tri);
    }
    soup.idx = kept;
    (degenerate, duplicate)
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[ix(x)] != x {
        let up = parent[ix(parent[ix(x)])];
        parent[ix(x)] = up;
        x = up;
    }
    x
}

/// Remove connected pieces smaller than [`SPECK_PERCENT`] of the largest piece's
/// triangles; returns `(triangles removed, pieces removed)`.
fn drop_specks(soup: &mut Soup) -> (u32, u32) {
    let n = soup.pos.len();
    let mut parent: Vec<u32> = (0..n).filter_map(|i| u32::try_from(i).ok()).collect();
    for tri in soup.idx.chunks_exact(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        for (x, y) in [(a, b), (b, c)] {
            let (rx, ry) = (find(&mut parent, x), find(&mut parent, y));
            if rx != ry {
                parent[ix(rx.max(ry))] = rx.min(ry);
            }
        }
    }
    let mut count = vec![0usize; n];
    for tri in soup.idx.chunks_exact(3) {
        let root = find(&mut parent, tri[0]);
        count[ix(root)] += 1;
    }
    let largest = count.iter().copied().max().unwrap_or(0);
    let floor = (largest * SPECK_PERCENT).div_ceil(100).max(1);
    let (mut removed_tris, mut removed_pieces) = (0usize, 0usize);
    removed_pieces += count.iter().filter(|c| **c > 0 && **c < floor).count();
    let mut kept = Vec::with_capacity(soup.idx.len());
    for tri in soup.idx.chunks_exact(3) {
        let root = find(&mut parent, tri[0]);
        if count[ix(root)] < floor {
            removed_tris += 1;
        } else {
            kept.extend_from_slice(tri);
        }
    }
    soup.idx = kept;
    (
        u32::try_from(removed_tris).unwrap_or(u32::MAX),
        u32::try_from(removed_pieces).unwrap_or(u32::MAX),
    )
}

/// Keep referenced vertices only, renumbered in first-use order.
fn compact(soup: &mut Soup) -> Result<(), IntakeError> {
    let mut remap = vec![u32::MAX; soup.pos.len()];
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    for index in &mut soup.idx {
        let old = ix(*index);
        if remap[old] == u32::MAX {
            remap[old] = to_index(pos.len())?;
            pos.push(soup.pos[old]);
            uv.push(soup.uv.get(old).copied().unwrap_or([0.0, 0.0]));
        }
        *index = remap[old];
    }
    soup.pos = pos;
    soup.uv = uv;
    Ok(())
}

/// Turn to `+Y` up, scale the longest side to `size_mm`, centre on x and z,
/// stand the lowest point on `y = 0`.
fn pose(soup: &mut Soup, settings: &IntakeSettings) -> Result<(), IntakeError> {
    if settings.up_axis == UpAxis::Z {
        for p in &mut soup.pos {
            *p = [p[0], p[2], -p[1]];
        }
    }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in &soup.pos {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let extent = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f32, f32::max);
    if !extent.is_finite() || extent <= 1e-9 {
        return Err(IntakeError::Empty {
            detail: "the mesh has no size".to_owned(),
        });
    }
    let size_m = f32::from(u16::try_from(settings.size_mm).unwrap_or(u16::MAX)) / 1000.0;
    let scale = size_m / extent;
    let mid = [(lo[0] + hi[0]) * 0.5, (lo[2] + hi[2]) * 0.5];
    for p in &mut soup.pos {
        *p = [
            (p[0] - mid[0]) * scale,
            (p[1] - lo[1]) * scale,
            (p[2] - mid[1]) * scale,
        ];
    }
    Ok(())
}

/// Area-weighted smooth normals and `+X`-derived tangents for an indexed mesh.
fn finish_attributes(pos: &[[f32; 3]], uv: &[[f32; 2]], idx: &[u32]) -> CookedMesh {
    let mut acc = vec![[0.0f32; 3]; pos.len()];
    for tri in idx.chunks_exact(3) {
        let at = |i: u32| pos.get(ix(i)).copied().unwrap_or([0.0; 3]);
        let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            e1[1] * e2[2] - e1[2] * e2[1],
            e1[2] * e2[0] - e1[0] * e2[2],
            e1[0] * e2[1] - e1[1] * e2[0],
        ];
        for corner in tri {
            if let Some(slot) = acc.get_mut(ix(*corner)) {
                for (s, c) in slot.iter_mut().zip(cross) {
                    *s += c;
                }
            }
        }
    }
    let unit = |v: [f32; 3], fallback: [f32; 3]| -> [f32; 3] {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if len.is_finite() && len > 1e-20 {
            [v[0] / len, v[1] / len, v[2] / len]
        } else {
            fallback
        }
    };
    let normals: Vec<[f32; 3]> = acc.iter().map(|v| unit(*v, [0.0, 1.0, 0.0])).collect();
    let tangents = normals
        .iter()
        .map(|n| {
            let r = if n[0].abs() > 0.9 {
                [0.0f32, 0.0, 1.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let d = r[0] * n[0] + r[1] * n[1] + r[2] * n[2];
            let t = unit(
                [r[0] - n[0] * d, r[1] - n[1] * d, r[2] - n[2] * d],
                [1.0, 0.0, 0.0],
            );
            [t[0], t[1], t[2], 1.0]
        })
        .collect();
    CookedMesh {
        positions: pos.to_vec(),
        normals,
        uv0: uv
            .iter()
            .map(|t| [t[0].clamp(0.0, 1.0), t[1].clamp(0.0, 1.0)])
            .collect(),
        tangents,
        indices: idx.to_vec(),
    }
}

// ---------------------------------------------------------------------------
// The provenance row.
// ---------------------------------------------------------------------------

fn json_str(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_list(out: &mut String, items: &[String]) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_str(out, item);
    }
    out.push(']');
}

fn json_floats(out: &mut String, v: &[f32]) {
    out.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{x:?}");
    }
    out.push(']');
}

#[expect(
    clippy::too_many_arguments,
    reason = "one row writer takes each recorded fact; a struct would only rename them"
)]
fn build_row(
    req: &IntakeRequest<'_>,
    key: &str,
    glb_digest: &str,
    bundle_hash: &str,
    tris_in: u32,
    lod_tris: &[u32],
    hull: &CollisionHull,
    cook_digest: u64,
    repair: &RepairStats,
) -> String {
    let p = req.provenance;
    let mut row = String::from("{\"v\":1,\"asset\":");
    json_str(&mut row, req.name);
    row.push_str(",\"bundle\":");
    json_str(&mut row, bundle_hash);
    row.push_str(",\"model\":{\"id\":");
    json_str(&mut row, &p.model.id);
    row.push_str(",\"license\":");
    json_str(&mut row, &p.model.license);
    row.push_str("},\"components\":[");
    for (i, c) in p.components.iter().enumerate() {
        if i > 0 {
            row.push(',');
        }
        row.push_str("{\"id\":");
        json_str(&mut row, &c.id);
        row.push_str(",\"license\":");
        json_str(&mut row, &c.license);
        row.push('}');
    }
    row.push_str("],\"source_digest\":");
    json_str(&mut row, &p.source_digest);
    let _ = write!(row, ",\"seed\":{}", p.seed);
    row.push_str(",\"conditions\":");
    json_list(&mut row, &p.conditions);
    row.push_str(",\"accepted\":");
    json_list(&mut row, req.accepted_conditions);
    row.push_str(",\"code_ref\":");
    json_str(&mut row, &p.code_ref);
    row.push_str(",\"glb\":");
    json_str(&mut row, glb_digest);
    row.push_str(",\"cache_key\":");
    json_str(&mut row, key);
    let _ = write!(
        row,
        ",\"tris_in\":{tris_in},\"tris\":{},\"lod_tris\":[{}],\"cook\":\"{cook_digest:016x}\"",
        lod_tris.first().copied().unwrap_or(0),
        lod_tris
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    let _ = write!(
        row,
        ",\"settings\":{{\"size_mm\":{},\"up\":\"{}\",\"max_tris\":{}}}",
        req.settings.size_mm,
        req.settings.up_axis.tag(),
        req.settings.max_tris
    );
    let _ = write!(
        row,
        ",\"repair\":{{\"welded\":{},\"degenerate\":{},\"duplicate\":{},\"speck_tris\":{},\"specks\":{}}}",
        repair.welded_vertices,
        repair.degenerate_tris,
        repair.duplicate_tris,
        repair.speck_tris,
        repair.specks
    );
    row.push_str(",\"hull\":{\"min\":");
    json_floats(&mut row, &hull.min);
    row.push_str(",\"max\":");
    json_floats(&mut row, &hull.max);
    row.push_str(",\"center\":");
    json_floats(&mut row, &hull.center);
    let _ = write!(
        row,
        ",\"radius\":{:?},\"support\":{}",
        hull.radius,
        hull.support.len()
    );
    row.push_str("},\"textured\":false}");
    row
}

// ---------------------------------------------------------------------------
// The intake.
// ---------------------------------------------------------------------------

fn reach_budget(mesh: &CookedMesh, max_tris: u32) -> Result<CookedMesh, IntakeError> {
    let mut current = mesh.clone();
    let mut target = max_tris;
    for _ in 0..SIMPLIFY_TRIES {
        current = simplify(&current, target)?;
        if tri_count(&current) <= max_tris {
            return Ok(current);
        }
        target = (target / 10 * 9).max(1);
    }
    Err(IntakeError::OverBudget {
        tris: tri_count(&current),
        max: max_tris,
    })
}

/// Import, weld, drop degenerate and duplicate triangles and floating specks, renumber.
fn repair_glb(glb: &[u8]) -> Result<(Soup, RepairStats), IntakeError> {
    let imported = import_gltf_bytes(glb).map_err(IntakeError::Import)?;
    let mut soup = merge(&imported.meshes)?;
    let mut repair = RepairStats {
        welded_vertices: weld(&mut soup)?,
        ..RepairStats::default()
    };
    (repair.degenerate_tris, repair.duplicate_tris) = clean_tris(&mut soup);
    (repair.speck_tris, repair.specks) = drop_specks(&mut soup);
    compact(&mut soup)?;
    if soup.idx.is_empty() {
        return Err(IntakeError::Empty {
            detail: format!(
                "{} degenerate, {} duplicate, {} speck triangles removed",
                repair.degenerate_tris, repair.duplicate_tris, repair.speck_tris
            ),
        });
    }
    Ok((soup, repair))
}

/// The LOD chain and one neutral material as bundle bytes.
fn stage_bundle(name: &str, cooked: &MeshCookOutput) -> Result<Vec<u8>, IntakeError> {
    let mut staged = StagedCooked::new();
    for (level, lod) in cooked.lods.iter().enumerate() {
        staged.meshes.push((lod_name(name, level), lod.clone()));
    }
    staged.materials.push((
        format!("{}_mat", name),
        CookedMaterial {
            base_color_factor: [0.72, 0.72, 0.72, 1.0],
            metallic_factor: 0.0,
            roughness_factor: 0.8,
            textures: Vec::new(),
        },
    ));
    cook_bundle_bytes(&staged).map_err(IntakeError::from)
}

/// Cook one generated GLB into a prop bundle with provenance, or answer from the
/// cache. The licence gate runs first: a refused model costs no cook.
pub fn intake(
    req: &IntakeRequest<'_>,
    cache: &mut IntakeCache,
) -> Result<IntakeOutput, IntakeError> {
    check_name(req.name)?;
    check_settings(&req.settings)?;
    check_provenance(req.provenance, req.accepted_conditions)?;
    if req.glb.len() > MAX_BYTES {
        return Err(IntakeError::Import(MeshImportError::TooLarge {
            count: req.glb.len(),
        }));
    }
    let key = cache_key(req);
    let glb_digest = content_digest(req.glb);
    if let Some(hit) = cache.get(&key) {
        if hit.glb_digest == glb_digest {
            return Ok(hit);
        }
    }
    let (mut soup, repair) = repair_glb(req.glb)?;
    pose(&mut soup, &req.settings)?;
    let tris_in = u32::try_from(soup.idx.len() / 3).unwrap_or(u32::MAX);
    let full = finish_attributes(&soup.pos, &soup.uv, &soup.idx);
    let reduced = reach_budget(&full, req.settings.max_tris)?;
    // Collapses move vertices a little: pose again so the finished prop is exactly `size_mm` and on the floor.
    let mut settled = Soup {
        pos: reduced.positions,
        uv: reduced.uv0,
        idx: reduced.indices,
    };
    let upright = IntakeSettings {
        up_axis: UpAxis::Y,
        ..req.settings
    };
    pose(&mut settled, &upright)?;
    let lod0 = finish_attributes(&settled.pos, &settled.uv, &settled.idx);
    let cooked = cook_mesh(&lod0, MeshClass::Prop)?;
    let bundle = stage_bundle(req.name, &cooked)?;
    let bundle_hash = content_digest(&bundle);
    let lod_tris: Vec<u32> = cooked.lods.iter().map(tri_count).collect();
    let hull = cooked.hulls.first().cloned().ok_or(IntakeError::Empty {
        detail: "the cook made no hull".to_owned(),
    })?;
    let row = build_row(
        req,
        &key,
        &glb_digest,
        &bundle_hash,
        tris_in,
        &lod_tris,
        &hull,
        cooked.digest,
        &repair,
    );
    let out = IntakeOutput {
        name: req.name.to_owned(),
        tris: lod_tris.first().copied().unwrap_or(0),
        bundle,
        bundle_hash,
        row,
        tris_in,
        lod_tris,
        hull,
        cook_digest: cooked.digest,
        cache_key: key,
        glb_digest,
        cache_hit: false,
        repair,
    };
    cache.put(&out)?;
    Ok(out)
}

#[cfg(test)]
mod tests;

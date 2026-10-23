//! arena_bundle bin: cook the arena kit into one small bundle the page fetches.
//!
//! Consumer: `crates/web/src/content.rs` fetches the file once and opens it with
//! `BundleSource::open`, then the played page hands that source to
//! `engine_game::arena_kit::stage_kit_from`. The native window keeps reading
//! `content/arena` from disk at full size, so nothing here touches
//! `content/arena/cooked/`.
//!
//! Two lanes, same inputs:
//!
//! - `packed` (default): one `E2BUNDLE` file tree — `src/<file>` is a compact
//!   glTF (the merged mesh, the kit material, the banner's own maps downscaled;
//!   a set piece carries no image) and `cooked/<set>_<map>.ktx2` are the five
//!   CC0 sets cooked down to `--max-dim`, all in tier 0 (the first frame needs
//!   them). The studio HDRI is tier 1, after that prefix, for the offscreen
//!   captures. This is what `BundleSource` reads and `import_kit_from` imports,
//!   exactly like the shipped files. The 256-texel step is 1.36 MB, 128 is
//!   0.71 MB and 64 is 0.55 MB; the cook halves 1024, so `--max-dim 192` lands
//!   on 128. The HDRI adds about 1.6 MB to the total, never to the first tier.
//! - `cooked`: one `E204BNDL` cooked-asset bundle (mesh, material and texture
//!   entries) for a `BundleLoader` consumer. Its texture payload is decoded RGBA
//!   mips, four times the KTX2 bytes, so only the 64 step (1.29 MB) fits.
//!
//! ```text
//! arena_bundle [--root content/arena] [--out <file>] [--format packed|cooked]
//!              [--max-dim <n>]
//! ```
//! The receipt line names entries, bytes per kind, the first-tier prefix, the
//! total, triangles, the cap, the dimensions it produced and the budget
//! verdict. Same sources and cap give the same bytes (run it twice and diff the
//! files).

#[cfg(all(feature = "import", feature = "registry"))]
use engine_assets::asset_source::{MemorySource, PackReport, PackRules};
#[cfg(all(feature = "import", feature = "registry"))]
use engine_assets::bundle_cook::{
    cook_bundle_bytes, load_bundle_file, write_bundle_file, StagedCooked,
};
#[cfg(all(feature = "import", feature = "registry"))]
use engine_assets::kit_import::{
    cook_kit_texture, import_kit_gltf, import_kit_gltf_full, is_lfs_pointer, kit_stem,
    kit_texture_asset, kit_texture_name, KitMap, KitSet, KIT_FILES, STUDIO_HDRI,
};
#[cfg(all(feature = "import", feature = "registry"))]
use engine_assets::{compact_gltf, AssetSource, BundleSource, EntryKind};
#[cfg(all(feature = "import", feature = "registry"))]
use image::RgbaImage;

/// Byte bar the page's first fetch carries; a first-tier prefix bigger than
/// this prints `over`. Later tiers are not in that fetch.
pub const WEB_BUDGET_BYTES: usize = 2_000_000;
/// Default output: the arena content dir, next to `src/` and `cooked/`.
const DEFAULT_OUT: &str = "content/arena/kit.e2bundle";
/// Default long-edge cap for the packed lane: the KTX2 files stay small, so
/// the 256-texel step (1.36 MB) is the largest one under the bar.
const DEFAULT_PACKED_MAX_DIM: u32 = 256;
/// Default long-edge cap for the cooked lane: its texture payload is decoded
/// RGBA mips, four times the KTX2 bytes, so only 64 clears the bar.
const DEFAULT_COOKED_MAX_DIM: u32 = 64;

#[cfg(all(feature = "import", feature = "registry"))]
fn main() {
    if let Err(message) = run() {
        eprintln!("arena_bundle FAIL: {message}");
        std::process::exit(1);
    }
}

#[cfg(not(all(feature = "import", feature = "registry")))]
fn main() {
    eprintln!("arena_bundle needs --features import,registry");
    std::process::exit(2);
}

/// What the bundle holds: the page's packed file tree, or cooked entries.
#[cfg(all(feature = "import", feature = "registry"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    /// `E2BUNDLE`: compact kit glTF plus cooked KTX2 sets (the page's lane).
    Packed,
    /// `E204BNDL`: cooked mesh, material and texture entries.
    Cooked,
}

#[cfg(all(feature = "import", feature = "registry"))]
fn usage() -> String {
    [
        "usage: arena_bundle [--root content/arena] [--out <file>] [--format packed|cooked] \
[--max-dim <n>]",
        "",
        "  --root <dir>      arena content dir holding src/ and sources/ (content/arena)",
        "  --out <file>      bundle to write (content/arena/kit.e2bundle)",
        "  --format <lane>   packed: E2BUNDLE of compact glTF + KTX2 (default);",
        "                    cooked: E204BNDL of mesh, material and texture entries",
        "  --max-dim <n>     long-edge cap for the cooked textures (packed 256, cooked 64;",
        "                    the cook halves 1024, so the steps are 256, 128, 64, 32)",
    ]
    .join("\n")
}

#[cfg(all(feature = "import", feature = "registry"))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct Config {
    root: std::path::PathBuf,
    out: std::path::PathBuf,
    format: Format,
    max_dim: u32,
}

#[cfg(all(feature = "import", feature = "registry"))]
fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut root = std::path::PathBuf::from("content/arena");
    let mut out: Option<std::path::PathBuf> = None;
    let mut format = Format::Packed;
    let mut max_dim: Option<u32> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                let value = args.get(i).ok_or("--root needs a path")?;
                root = std::path::PathBuf::from(value);
            }
            "--out" => {
                i += 1;
                let value = args.get(i).ok_or("--out needs a path")?;
                out = Some(std::path::PathBuf::from(value));
            }
            "--format" => {
                i += 1;
                format = match args.get(i).map(String::as_str) {
                    Some("packed") => Format::Packed,
                    Some("cooked") => Format::Cooked,
                    Some(other) => {
                        return Err(format!("--format wants packed or cooked, got {other:?}"))
                    }
                    None => return Err("--format wants packed or cooked".to_string()),
                };
            }
            "--max-dim" => {
                i += 1;
                let value = args
                    .get(i)
                    .and_then(|value| value.parse().ok())
                    .filter(|dim| *dim > 0)
                    .ok_or("--max-dim needs a positive number")?;
                max_dim = Some(value);
            }
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("unknown argument {other:?}\n{}", usage())),
        }
        i += 1;
    }
    Ok(Config {
        root,
        out: out.unwrap_or_else(|| std::path::PathBuf::from(DEFAULT_OUT)),
        format,
        max_dim: max_dim.unwrap_or(match format {
            Format::Packed => DEFAULT_PACKED_MAX_DIM,
            Format::Cooked => DEFAULT_COOKED_MAX_DIM,
        }),
    })
}

#[cfg(all(feature = "import", feature = "registry"))]
fn run() -> Result<(), String> {
    let config = parse_args(&std::env::args().skip(1).collect::<Vec<_>>())?;
    match config.format {
        Format::Packed => run_packed(&config),
        Format::Cooked => run_cooked(&config),
    }
}

/// What the tier-0 kit contributes to the receipt.
#[cfg(all(feature = "import", feature = "registry"))]
struct PackedStats {
    dims_lo: (u32, u32),
    dims_hi: (u32, u32),
    ktx2_bytes: usize,
    gltf_bytes: usize,
    image_bytes: usize,
    images: usize,
    triangles: u64,
    texture_files: usize,
}

/// Cook every kit set and compact every kit glTF into `staged`, all tier 0:
/// the first frame needs this prefix.
#[cfg(all(feature = "import", feature = "registry"))]
fn stage_packed_kit(config: &Config) -> Result<(MemorySource, PackedStats), String> {
    let mut staged = MemorySource::new();
    let (mut dims_lo, mut dims_hi) = ((u32::MAX, u32::MAX), (0u32, 0u32));
    let mut ktx2_bytes = 0usize;
    for set in KitSet::ALL {
        let cooked = cook_set(config, set)?;
        dims_lo = (dims_lo.0.min(cooked.dims.0), dims_lo.1.min(cooked.dims.1));
        dims_hi = (dims_hi.0.max(cooked.dims.0), dims_hi.1.max(cooked.dims.1));
        for (path, texture) in cooked.files {
            ktx2_bytes += texture.len();
            staged
                .insert(&path, texture)
                .map_err(|err| err.to_string())?;
        }
    }
    let mut triangles = 0u64;
    let mut gltf_bytes = 0usize;
    let mut image_bytes = 0usize;
    let mut images = 0usize;
    for file in KIT_FILES {
        let bytes = read_kit(&config.root, file)?;
        let (piece, maps) =
            import_kit_gltf_full(file, &bytes).map_err(|err| format!("{file}: {err}"))?;
        let compact =
            compact_gltf(&piece, &maps, config.max_dim).map_err(|err| format!("{file}: {err}"))?;
        if piece.set.is_none() && compact.images == 0 {
            return Err(format!("{file}: wears no cooked set and carries no map"));
        }
        triangles += piece.triangles;
        gltf_bytes += compact.json.len();
        image_bytes += compact.image_bytes;
        images += compact.images;
        staged
            .insert(&format!("src/{file}"), compact.json.into_bytes())
            .map_err(|err| err.to_string())?;
    }
    Ok((
        staged,
        PackedStats {
            dims_lo,
            dims_hi,
            ktx2_bytes,
            gltf_bytes,
            image_bytes,
            images,
            triangles,
            texture_files: KitSet::ALL.len() * KitMap::ALL.len(),
        },
    ))
}

/// Write the packed bundle, read it back through the page's reader, check the
/// bytes and the digest, and return the report plus the tier-0 prefix length.
#[cfg(all(feature = "import", feature = "registry"))]
fn write_and_verify_packed(
    config: &Config,
    staged: &MemorySource,
    rules: &PackRules,
) -> Result<(PackReport, usize), String> {
    let mut bytes = Vec::new();
    let report = engine_assets::asset_source::pack_to(staged, rules, &mut bytes)
        .map_err(|err| err.to_string())?;
    let parent = parent_of(&config.out);
    std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    std::fs::write(&config.out, &bytes)
        .map_err(|err| format!("{}: {err}", config.out.display()))?;
    // Re-read the file from disk through the reader the page uses and check
    // every staged file comes back byte for byte (the reader hashes each blob).
    let on_disk =
        std::fs::read(&config.out).map_err(|err| format!("{}: {err}", config.out.display()))?;
    let source = BundleSource::open(on_disk).map_err(|err| err.to_string())?;
    for path in staged.list("").map_err(|err| err.to_string())? {
        let want = staged.read(&path).map_err(|err| err.to_string())?;
        let got = source.read(&path).map_err(|err| err.to_string())?;
        if want != got {
            return Err(format!("{path}: read back differs from the staged bytes"));
        }
    }
    let digest =
        engine_assets::asset_source::content_digest(&source, "").map_err(|err| err.to_string())?;
    if digest != report.digest {
        return Err(format!(
            "loaded digest {digest:016x} differs from the pack digest {:016x}",
            report.digest
        ));
    }
    // The prefix a streaming page can stage the first frame from: every tier 0
    // blob ends at `offset + len`; the header and index are in front of them.
    let first_bytes = source
        .entries_in_file_order()
        .filter(|entry| entry.priority == 0)
        .map(|entry| entry.offset + entry.len)
        .max()
        .unwrap_or(0);
    Ok((report, first_bytes))
}

/// The packed lane: compact glTF per kit file plus the cooked KTX2 sets in
/// tier 0, then the studio HDRI in tier 1, packed into one `E2BUNDLE` the page
/// opens with `BundleSource`.
#[cfg(all(feature = "import", feature = "registry"))]
fn run_packed(config: &Config) -> Result<(), String> {
    let (mut staged, kit) = stage_packed_kit(config)?;
    // QW4: the HDRI leaves the wasm and rides the fetched bundle. It is tier 1:
    // the first-frame kit prefix stays the whole tier 0.
    let hdri = read_hdri(&config.root)?;
    let hdri_bytes = hdri.len();
    staged
        .insert(STUDIO_HDRI, hdri)
        .map_err(|err| err.to_string())?;
    if staged.len() != KIT_FILES.len() + kit.texture_files + 1 {
        return Err(format!(
            "staged {} files, expected {}",
            staged.len(),
            KIT_FILES.len() + kit.texture_files + 1
        ));
    }
    let rules = PackRules::new().priority(STUDIO_HDRI, 1);
    let (report, first_bytes) = write_and_verify_packed(config, &staged, &rules)?;
    let bar = if first_bytes <= WEB_BUDGET_BYTES {
        "ok"
    } else {
        "over"
    };
    println!(
        "arena_bundle: format=packed entries={} gltf={} textures={} hdri={hdri_bytes} \
gltf_bytes={} image_bytes={} images={} ktx2_bytes={} first={first_bytes} total={} \
tris={} max_dim={} dims={}x{}..{}x{} digest={:016x} bar={WEB_BUDGET_BYTES} {bar}",
        report.entries,
        KIT_FILES.len(),
        kit.texture_files,
        kit.gltf_bytes,
        kit.image_bytes,
        kit.images,
        kit.ktx2_bytes,
        report.bytes,
        kit.triangles,
        config.max_dim,
        kit.dims_lo.0,
        kit.dims_lo.1,
        kit.dims_hi.0,
        kit.dims_hi.1,
        report.digest
    );
    if bar == "over" {
        eprintln!(
            "arena_bundle: first tier {first_bytes} B is over the {WEB_BUDGET_BYTES} B web bar; \
the cook halves 1024, so the next fitting step is --max-dim 64, then 32"
        );
    }
    Ok(())
}

/// The cooked lane: mesh, material and texture entries in one `E204BNDL`.
#[cfg(all(feature = "import", feature = "registry"))]
fn run_cooked(config: &Config) -> Result<(), String> {
    let mut staged = StagedCooked::new();
    let (mut dims_lo, mut dims_hi) = ((u32::MAX, u32::MAX), (0u32, 0u32));
    let mut texture_ktx2 = 0usize;
    for set in KitSet::ALL {
        let cooked = cook_set(config, set)?;
        texture_ktx2 += cooked.ktx2_bytes;
        dims_lo = (dims_lo.0.min(cooked.dims.0), dims_lo.1.min(cooked.dims.1));
        dims_hi = (dims_hi.0.max(cooked.dims.0), dims_hi.1.max(cooked.dims.1));
        for (name, texture) in cooked.textures {
            staged.textures.push((name, texture));
        }
    }
    let mut triangles = 0u64;
    for file in KIT_FILES {
        let bytes = read_kit(&config.root, file)?;
        let piece = import_kit_gltf(file, &bytes).map_err(|err| format!("{file}: {err}"))?;
        triangles += piece.triangles;
        staged.meshes.push((kit_stem(file).to_string(), piece.mesh));
        staged
            .materials
            .push((format!("mat/{}", kit_stem(file)), piece.material));
    }
    check_refs(&staged)?;
    let bytes = cook_bundle_bytes(&staged).map_err(|err| err.to_string())?;
    let parent = parent_of(&config.out);
    std::fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
    let receipt = write_bundle_file(&bytes, &config.out, false).map_err(|err| err.to_string())?;
    let (loaded, verified) = load_bundle_file(&config.out, false).map_err(|err| err.to_string())?;
    let mut mesh_bytes = 0usize;
    let mut material_bytes = 0usize;
    let mut texture_bytes = 0usize;
    let mut meshes = 0usize;
    let mut materials = 0usize;
    let mut textures = 0usize;
    for entry in &loaded.entries {
        match entry.kind {
            EntryKind::Mesh => {
                meshes += 1;
                mesh_bytes += entry.bytes.len();
            }
            EntryKind::Material => {
                materials += 1;
                material_bytes += entry.bytes.len();
            }
            EntryKind::Texture => {
                textures += 1;
                texture_bytes += entry.bytes.len();
            }
            _ => {}
        }
    }
    let bar = if verified.bytes <= WEB_BUDGET_BYTES {
        "ok"
    } else {
        "over"
    };
    println!(
        "arena_bundle: format=cooked entries={} meshes={meshes} mats={materials} \
textures={textures} mesh_bytes={mesh_bytes} material_bytes={material_bytes} \
texture_bytes={texture_bytes} total={} tris={triangles} max_dim={} dims={}x{}..{}x{} \
ktx2_bytes={texture_ktx2} bar={WEB_BUDGET_BYTES} {bar}",
        receipt.entries, verified.bytes, config.max_dim, dims_lo.0, dims_lo.1, dims_hi.0, dims_hi.1
    );
    if bar == "over" {
        eprintln!(
            "arena_bundle: {} B is over the {WEB_BUDGET_BYTES} B web bar; the cook halves \
1024, so the fitting steps are --max-dim 64 (1.29 MB), then 32",
            verified.bytes
        );
    }
    Ok(())
}

#[cfg(all(feature = "import", feature = "registry"))]
fn parent_of(out: &std::path::Path) -> &std::path::Path {
    out.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
}

#[cfg(all(feature = "import", feature = "registry"))]
fn read_kit(root: &std::path::Path, file: &str) -> Result<Vec<u8>, String> {
    let path = root.join("src").join(file);
    let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    if is_lfs_pointer(&bytes) {
        return Err(format!(
            "{}: git-lfs pointer, run git lfs pull",
            path.display()
        ));
    }
    Ok(bytes)
}

/// The studio HDRI, from the arena content root. A pointer or a file that is
/// not Radiance HDR is refused before it reaches the bundle.
#[cfg(all(feature = "import", feature = "registry"))]
fn read_hdri(root: &std::path::Path) -> Result<Vec<u8>, String> {
    let path = root.join(STUDIO_HDRI);
    let bytes = std::fs::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    if is_lfs_pointer(&bytes) {
        return Err(format!(
            "{}: git-lfs pointer, run git lfs pull",
            path.display()
        ));
    }
    if !bytes.starts_with(b"#?RADIANCE") {
        return Err(format!("{}: not a Radiance HDR file", path.display()));
    }
    Ok(bytes)
}

/// One set cooked: the KTX2 file per map (the packed lane's `cooked/` entries
/// and the cooked lane's decoded textures share one cook) plus the dimensions
/// the cap produced.
#[cfg(all(feature = "import", feature = "registry"))]
struct CookedSet {
    /// `cooked/<set>_<map>.ktx2` path and bytes, in `KitMap::ALL` order.
    files: Vec<(String, Vec<u8>)>,
    /// Cooked textures for the `E204BNDL` lane, under their bundle names.
    textures: Vec<(String, engine_assets::CookedTexture)>,
    /// Total KTX2 bytes.
    ktx2_bytes: usize,
    /// Dimensions of the first map's level 0.
    dims: (u32, u32),
}

/// Cook one CC0 set from `sources/<dir>` into base, normal and ORM maps. The
/// ORM assembly (R occlusion, G roughness, B metallic) matches `arena_cook`;
/// the metal set ships a metalness map where the others ship occlusion.
#[cfg(all(feature = "import", feature = "registry"))]
fn cook_set(config: &Config, set: KitSet) -> Result<CookedSet, String> {
    let (dir, has_ao) = set.source_set();
    let source = config.root.join("sources").join(dir);
    let file = |suffix: &str| source.join(format!("{dir}_1K-JPG_{suffix}.jpg"));
    let color = load_jpg(&file("Color"))?;
    let normal = load_jpg(&file("NormalGL"))?;
    let rough = load_jpg(&file("Roughness"))?;
    let (width, height) = color.dimensions();
    if normal.dimensions() != (width, height) || rough.dimensions() != (width, height) {
        return Err(format!("{dir}: map sizes disagree"));
    }
    let ao = if has_ao {
        Some(load_jpg(&file("AmbientOcclusion"))?)
    } else {
        None
    };
    let metal = if has_ao {
        None
    } else {
        Some(load_jpg(&file("Metalness"))?)
    };
    let orm = orm_texels(&rough, ao.as_ref(), metal.as_ref());
    let mut files = Vec::with_capacity(3);
    let mut textures = Vec::with_capacity(3);
    let mut ktx2_bytes = 0usize;
    let mut dims = (0u32, 0u32);
    let maps: [(KitMap, &[u8]); 3] = [
        (KitMap::Base, color.as_raw()),
        (KitMap::Normal, normal.as_raw()),
        (KitMap::Orm, &orm),
    ];
    for (map, rgba) in maps {
        let cooked = cook_kit_texture(map, rgba, width, height, config.max_dim)
            .map_err(|err| format!("{dir} {}: {err}", map.name()))?;
        if dims == (0, 0) {
            dims = (cooked.cooked.width, cooked.cooked.height);
        }
        ktx2_bytes += cooked.ktx2.len();
        files.push((kit_cooked_file(set, map), cooked.ktx2));
        textures.push((kit_texture_name(set, map), cooked.cooked));
    }
    Ok(CookedSet {
        files,
        textures,
        ktx2_bytes,
        dims,
    })
}

/// Path of one set map's KTX2 file in the source: the name
/// `kit_pbr::load_cooked_set` reads from a fetched bundle.
#[cfg(all(feature = "import", feature = "registry"))]
fn kit_cooked_file(set: KitSet, map: KitMap) -> String {
    format!("cooked/{}_{}.ktx2", set.name(), map.name())
}

/// Packed ORM texels from the roughness image plus occlusion (or metalness).
#[cfg(all(feature = "import", feature = "registry"))]
fn orm_texels(rough: &RgbaImage, ao: Option<&RgbaImage>, metal: Option<&RgbaImage>) -> Vec<u8> {
    let count = rough.len() / 4;
    let mut out = vec![255u8; rough.len()];
    for index in 0..count {
        out[index * 4] = ao.map_or(255, |map| map.as_raw()[index * 4]);
        out[index * 4 + 1] = rough.as_raw()[index * 4];
        out[index * 4 + 2] = metal.map_or(0, |map| map.as_raw()[index * 4]);
    }
    out
}

#[cfg(all(feature = "import", feature = "registry"))]
fn load_jpg(path: &std::path::Path) -> Result<RgbaImage, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
    if is_lfs_pointer(&bytes) {
        return Err(format!(
            "{}: git-lfs pointer, run git lfs pull",
            path.display()
        ));
    }
    let image =
        image::load_from_memory(&bytes).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(image.to_rgba8())
}

/// Refuse a material reference that names no staged texture: a dangling ref
/// would draw a set-coloured kit with no maps in the page.
#[cfg(all(feature = "import", feature = "registry"))]
fn check_refs(staged: &StagedCooked) -> Result<(), String> {
    let mut known = Vec::new();
    for set in KitSet::ALL {
        for map in KitMap::ALL {
            known.push(kit_texture_asset(set, map));
        }
    }
    for (name, material) in &staged.materials {
        for reference in &material.textures {
            if !known.contains(&reference.asset) {
                return Err(format!(
                    "{name}: reference {} names no set map",
                    reference.asset
                ));
            }
        }
    }
    Ok(())
}

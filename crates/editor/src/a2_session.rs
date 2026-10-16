//! Recorded editable-scene session: import, place, edit, save, reopen.
//!
//! Owner: the recorded evidence run; reads licensed content bytes, owns only
//! the session report. Units: bytes for files, meters/radians for poses. The
//! run is std-only: it drives the same [`Shell`], browser and save codec
//! paths the editor executable uses, and the executable itself invokes it
//! for evidence (passing its window-path captures in). Determinism: same
//! content bytes give the same placements, the same deterministic node ids
//! (seeded slots) and the same save/reopen byte equality; two runs write
//! byte-identical `scene.edoc` files and byte-identical fallback PNGs.
//! Errors: [`A2Error`]; any failure stops the session with its cause.

use std::fmt;
use std::path::{Path, PathBuf};

use engine_core::adler::adler32;
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::browser::{
    preview_drop, search, AssetKind, BrowserActor, BundleEntry, BundleView, Intent, ObjectRef,
    Relation, SearchQuery, Target, Want,
};
use crate::save::{load_from_bytes, save_to_bytes};
use crate::shell::Shell;

/// Content files read by the session, relative to the content root.
const HERO_MESH: &str = "fighters/glb/hero_body.glb";
const OPP_MESH: &str = "fighters/glb/opp_male_light_body.glb";
const WALLS_MESH: &str = "arena/src/walls.gltf";
const CRATE_MESH: &str = "arena/src/crate.gltf";
const SAND_TEX: &str = "arena/textures/sand_albedo.png";
const STONE_TEX: &str = "arena/textures/stone_albedo.png";
const ARENA_MANIFEST: &str = "arena/manifest.json";
const FIGHTERS_MANIFEST: &str = "fighters/manifest.json";

/// First four bytes of a binary glTF asset.
const GLB_MAGIC: [u8; 4] = *b"glTF";
/// First eight bytes of a PNG asset.
const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// What stopped the recorded session. Every variant names its file or node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum A2Error {
    /// A content or manifest file could not be read.
    Io {
        /// File that failed to read.
        path: String,
        /// OS report.
        reason: String,
    },
    /// A content file missed its format magic: not the licensed bytes.
    BadMagic {
        /// File refused.
        file: String,
        /// Magic expected (`glTF`, `PNG`, `JSON`).
        expected: &'static str,
    },
    /// A manifest does not name a file it must cover.
    Manifest {
        /// Manifest checked.
        file: String,
        /// Entry text missing from it.
        missing: String,
    },
    /// The asset browser refused a lookup or drop.
    Browser(String),
    /// The command service refused a shell edit.
    Shell(String),
    /// The schema inspector refused a component edit.
    Auto(String),
    /// The save codec refused a reopen.
    Save(String),
    /// A session invariant broke (counts, revisions, byte equality).
    Check(String),
    /// A headless viewport capture could not be encoded or written.
    Capture(String),
}

impl fmt::Display for A2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, reason } => write!(f, "cannot read {path}: {reason}"),
            Self::BadMagic { file, expected } => {
                write!(
                    f,
                    "licensed bytes check failed for {file}: expected {expected} magic"
                )
            }
            Self::Manifest { file, missing } => {
                write!(f, "manifest {file} does not cover `{missing}`")
            }
            Self::Browser(reason) => write!(f, "browser refused: {reason}"),
            Self::Shell(reason) => write!(f, "shell refused: {reason}"),
            Self::Auto(reason) => write!(f, "inspector refused: {reason}"),
            Self::Save(reason) => write!(f, "save/reopen refused: {reason}"),
            Self::Check(reason) => write!(f, "session check failed: {reason}"),
            Self::Capture(reason) => write!(f, "viewport capture failed: {reason}"),
        }
    }
}

impl std::error::Error for A2Error {}

/// One licensed source file verified by the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LicensedSource {
    /// Session label (`hero mesh`, `sand texture`, ...).
    pub name: String,
    /// Path relative to the content root.
    pub path: String,
    /// File length in bytes.
    pub bytes: usize,
    /// License tag read from the manifest row.
    pub license: String,
}

/// Full receipt of one recorded session run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionReport {
    /// Licensed sources verified, in read order.
    pub sources: Vec<LicensedSource>,
    /// Placed objects: node id plus asset plus pose in meters.
    pub placed: Vec<String>,
    /// Browser preview line for the crate drop.
    pub preview_line: String,
    /// Component edit receipt (`pose.px` old vs new on the hero).
    pub component_edit: String,
    /// Material binding receipt (command-service edit, persisted in v2 bytes).
    pub material_edit: String,
    /// Refusals exercised, each naming its cause.
    pub refusals: Vec<String>,
    /// Saved scene file name plus bytes plus receipt hash.
    pub saved: String,
    /// Reopened scene file name plus bytes plus receipt hash.
    pub reopened: String,
    /// True when reopen-then-save is byte-identical to the save.
    pub bytes_match: bool,
    /// Document revision at save time.
    pub revision: u64,
    /// Node count at save time.
    pub nodes: usize,
    /// Viewport captures written beside the scene (`file (bytes, fnv)`).
    pub captures: Vec<String>,
    /// How the capture PNGs were produced (exe window path or fallback).
    pub capture_method: String,
}

impl SessionReport {
    /// Render the evidence report text.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("A2 recorded editable-scene session\n");
        out.push_str(
            "licensed content bytes only; editor shell + browser + save codec, \
             driven by the editor executable\n\n",
        );
        out.push_str("sources (path, bytes, license):\n");
        for source in &self.sources {
            out.push_str(&format!(
                "  {}: {} ({} bytes, {})\n",
                source.name, source.path, source.bytes, source.license
            ));
        }
        out.push_str("placed objects (node id, asset, pose m):\n");
        for line in &self.placed {
            out.push_str(&format!("  {line}\n"));
        }
        out.push_str(&format!("browser preview: {}\n", self.preview_line));
        out.push_str(&format!("component edit: {}\n", self.component_edit));
        out.push_str(&format!("material edit: {}\n", self.material_edit));
        out.push_str("refusals exercised (each wrote nothing):\n");
        for line in &self.refusals {
            out.push_str(&format!("  {line}\n"));
        }
        out.push_str(&format!(
            "save: {} | reopen: {} | byte-identical: {}\n",
            self.saved, self.reopened, self.bytes_match
        ));
        out.push_str(&format!(
            "revision r{} with {} nodes; save v2 persists hierarchy plus revision \
             plus material bindings (references)\n",
            self.revision, self.nodes
        ));
        out.push_str(&format!("capture method: {}\n", self.capture_method));
        out.push_str("captures (file, bytes, fnv):\n");
        for line in &self.captures {
            out.push_str(&format!("  {line}\n"));
        }
        out
    }
}

/// FNV-1a64 over bytes, hex for the receipt (deterministic). Shared with the
/// companion recorded runs in this crate so every receipt hashes one way.
pub(crate) fn fnv_hex(bytes: &[u8]) -> String {
    format!("{:016x}", engine_core::hash_bytes(bytes))
}

/// Window-path capture supplier for the recorded runs: the editor executable
/// passes its GPU-offscreen/live-panel captures in through this hook, keyed
/// by the shell to capture. Returns the PNG bytes plus the method label for
/// the report. The library fallback applies when the hook is absent or
/// returns `None`.
pub type ExeCapture<'a> = &'a dyn Fn(&Shell) -> Option<(Vec<u8>, String)>;

/// Deterministic session node id for `slot` (1 = wall, 2 = crate, 3 = hero).
///
/// Fixed `a2e70001-0000-4000-8000-<slot>` UUIDs: version-4 and RFC4122-variant
/// shaped like minted ids, but constant across runs so `scene.edoc` bytes
/// match run to run. Within a run each slot is used once, so identity is
/// unique per node; across runs the same slot is the same node.
fn deterministic_id(slot: u32) -> Result<ObjectId, A2Error> {
    let text = format!("a2e70001-0000-4000-8000-{slot:012x}");
    ObjectId::parse(&text)
        .map_err(|_| A2Error::Check(format!("session slot {slot} id is not a UUID")))
}

// ---------------------------------------------------------------------------
// Headless viewport snapshot: std-only CPU raster plus std-only PNG writer.
//
// Mirrors the `ViewportImage` live texture (same backdrop/grid/cross colors
// and 32 px grid step) without the `ui` feature or the GPU: the renderer
// `Presenter` offscreen path in `app.rs` stays the reference-GPU receipt for
// windowed runs, while this raster gives deterministic PNG evidence on
// headless machines with no adapter. Markers map world x/z in [-10, 10] m to
// panel pixels (amber for the selection); rows behind no valid reading still
// leave the grid, so the panel never goes blank.
// ---------------------------------------------------------------------------

/// Panel width for the headless snapshot in pixels.
const SNAP_WIDTH: u32 = 320;
/// Panel height for the headless snapshot in pixels.
const SNAP_HEIGHT: u32 = 200;
/// Grid spacing in pixels for the floor backdrop (matches `ViewportImage`).
const SNAP_GRID_STEP: u32 = 32;
/// Largest stored-deflate block in the PNG writer (zlib ceiling).
const PNG_BLOCK_MAX: usize = 65535;

fn crc32_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, cell) in table.iter_mut().enumerate() {
            let mut c = index as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *cell = c;
        }
        table
    })
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for byte in data {
        crc = crc32_table()[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

fn png_write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc = crc32_update(0xFFFF_FFFF, tag);
    crc = crc32_update(crc, data);
    out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
}

/// Encode RGBA8 pixels as a PNG (8-bit, filter 0, stored deflate): the same
/// pixels always give the same bytes on every machine. Shared with the
/// editor executable so its live-panel fallback encodes identically.
pub fn encode_png_rgba8(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, A2Error> {
    if width == 0 || height == 0 || width > 1024 || height > 1024 {
        return Err(A2Error::Capture(format!(
            "capture size {width}x{height} rejected (headless cap 1024px)"
        )));
    }
    let stride = width as usize * 4;
    if pixels.len() != stride * height as usize {
        return Err(A2Error::Capture("pixel length mismatch".to_string()));
    }
    let mut raw = Vec::with_capacity(pixels.len() + height as usize);
    for row in pixels.chunks_exact(stride) {
        raw.push(0u8);
        raw.extend_from_slice(row);
    }
    let mut zlib = Vec::with_capacity(raw.len() + 16);
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut blocks = raw.chunks(PNG_BLOCK_MAX).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        zlib.push(u8::from(last));
        let len = block.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(8 + 25 + zlib.len() + 16 + 12);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    png_write_chunk(&mut out, b"IHDR", &ihdr);
    png_write_chunk(&mut out, b"IDAT", &zlib);
    png_write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Render the shell headless at `SNAP_WIDTH` x `SNAP_HEIGHT`: grid backdrop
/// plus one marker per row world position (amber for the selection).
fn render_headless_rgba(shell: &Shell) -> Vec<u8> {
    const BG: [u8; 3] = [7, 8, 15];
    const LINE: [u8; 3] = [22, 26, 44];
    const CROSS: [u8; 3] = [60, 66, 96];
    const NODE: [u8; 3] = [235, 235, 240];
    const SELECTED: [u8; 3] = [255, 180, 40];
    let (width, height) = (SNAP_WIDTH, SNAP_HEIGHT);
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let x = i as u32 % width;
        let y = i as u32 / width;
        let mut c = BG;
        if x.is_multiple_of(SNAP_GRID_STEP) || y.is_multiple_of(SNAP_GRID_STEP) {
            c = LINE;
        }
        if x == width / 2 || y == height / 2 {
            c = CROSS;
        }
        px[0..3].copy_from_slice(&c);
        px[3] = 255;
    }
    let paint_square = |rgba: &mut [u8], cx: i32, cy: i32, half: i32, color: [u8; 3]| {
        for dy in -half..=half {
            for dx in -half..=half {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let i = (y as u32 * width + x as u32) as usize * 4;
                rgba[i..i + 3].copy_from_slice(&color);
                rgba[i + 3] = 255;
            }
        }
    };
    for row in shell.rows() {
        let Some(world) = shell.document().hierarchy.world(row.id) else {
            continue;
        };
        if !world.position.iter().all(|v| v.is_finite()) {
            continue;
        }
        // Orthographic top-down map: x/z in [-10, 10] m cover the panel.
        let nx = ((f64::from(world.position[0]) + 10.0) / 20.0 * f64::from(width)).round();
        let ny = ((f64::from(world.position[2]) + 10.0) / 20.0 * f64::from(height)).round();
        if nx < 0.0 || ny < 0.0 || nx >= f64::from(width) || ny >= f64::from(height) {
            continue;
        }
        let selected = shell.selection == Some(row.id);
        let (half, color) = if selected { (5, SELECTED) } else { (3, NODE) };
        paint_square(&mut rgba, nx as i32, ny as i32, half, color);
    }
    rgba
}

/// Capture the shell headless as PNG bytes at the fixed snapshot size.
/// The editor executable prefers its window paths (GPU offscreen, then the
/// live panel) and falls back to this raster only with no adapter or panel.
pub(crate) fn capture_viewport_png(shell: &Shell) -> Result<Vec<u8>, A2Error> {
    let rgba = render_headless_rgba(shell);
    encode_png_rgba8(&rgba, SNAP_WIDTH, SNAP_HEIGHT)
}

/// Read one content file or stop with its path.
fn read_content(root: &Path, rel: &str) -> Result<Vec<u8>, A2Error> {
    let path = root.join(rel);
    std::fs::read(&path).map_err(|error| A2Error::Io {
        path: path.to_string_lossy().into_owned(),
        reason: error.to_string(),
    })
}

/// Check the manifest names the file it must cover.
fn check_manifest(manifest: &str, file: &str, entry: &str) -> Result<(), A2Error> {
    if manifest.contains(entry) {
        Ok(())
    } else {
        Err(A2Error::Manifest {
            file: file.to_string(),
            missing: entry.to_string(),
        })
    }
}

/// Yaw-only rotation quaternion (radians about +Y).
fn yaw_quat(yaw: f32) -> [f32; 4] {
    let (half_sin, half_cos) = (0.5 * yaw).sin_cos();
    [0.0, half_sin, 0.0, half_cos]
}

/// Bind one browser material to a node through the shell command service, so
/// the edit journals, undoes and persists in the save bytes. Unknown assets
/// and non-material assets refuse with the asset named; nothing is staged.
fn assign_material(
    bundle: &BundleView,
    shell: &mut Shell,
    node: ObjectId,
    asset: &str,
) -> Result<String, A2Error> {
    shell
        .bind_material(node, asset, bundle)
        .map_err(|error| A2Error::Browser(format!("unknown asset `{asset}`: {error}")))?;
    Ok(format!("node {node} bound to material `{asset}`"))
}

/// Licensed receipt rows for the six session files.
fn licensed_sources(
    hero_len: usize,
    opp_len: usize,
    walls_len: usize,
    crate_len: usize,
    sand_len: usize,
    stone_len: usize,
) -> Vec<LicensedSource> {
    vec![
        LicensedSource {
            name: "hero mesh".to_string(),
            path: HERO_MESH.to_string(),
            bytes: hero_len,
            license: "CC0-1.0 via OGA System G6 (fighters manifest)".to_string(),
        },
        LicensedSource {
            name: "opponent mesh".to_string(),
            path: OPP_MESH.to_string(),
            bytes: opp_len,
            license: "CC0-1.0 via OGA System G6 (fighters manifest)".to_string(),
        },
        LicensedSource {
            name: "walls mesh".to_string(),
            path: WALLS_MESH.to_string(),
            bytes: walls_len,
            license: "CC0-1.0 originals (arena manifest)".to_string(),
        },
        LicensedSource {
            name: "crate mesh".to_string(),
            path: CRATE_MESH.to_string(),
            bytes: crate_len,
            license: "CC0-1.0 originals (arena manifest)".to_string(),
        },
        LicensedSource {
            name: "sand texture".to_string(),
            path: SAND_TEX.to_string(),
            bytes: sand_len,
            license: "CC0-1.0 originals (arena manifest)".to_string(),
        },
        LicensedSource {
            name: "stone texture".to_string(),
            path: STONE_TEX.to_string(),
            bytes: stone_len,
            license: "CC0-1.0 originals (arena manifest)".to_string(),
        },
    ]
}

/// Prefix check for binary magics (glTF, PNG): length plus leading bytes.
fn check_prefix(
    bytes: &[u8],
    prefix: &[u8],
    file: &str,
    expected: &'static str,
) -> Result<(), A2Error> {
    if bytes.len() < prefix.len() || bytes[0..prefix.len()] != prefix[..] {
        return Err(A2Error::BadMagic {
            file: file.to_string(),
            expected,
        });
    }
    Ok(())
}

/// JSON check for text meshes: valid UTF-8 carrying an asset marker.
fn check_json_asset(bytes: &[u8], file: &str) -> Result<(), A2Error> {
    let text = core::str::from_utf8(bytes).map_err(|_| A2Error::BadMagic {
        file: file.to_string(),
        expected: "JSON",
    })?;
    if !text.contains("\"asset\"") {
        return Err(A2Error::BadMagic {
            file: file.to_string(),
            expected: "JSON",
        });
    }
    Ok(())
}

/// Licensed bytes in: six content files with format magics, each covered by
/// its manifest row. Returns the receipt rows for the report.
fn read_sources(content_root: &Path) -> Result<Vec<LicensedSource>, A2Error> {
    let hero = read_content(content_root, HERO_MESH)?;
    check_prefix(&hero, &GLB_MAGIC, HERO_MESH, "glTF")?;
    let opp = read_content(content_root, OPP_MESH)?;
    check_prefix(&opp, &GLB_MAGIC, OPP_MESH, "glTF")?;
    let walls = read_content(content_root, WALLS_MESH)?;
    check_json_asset(&walls, WALLS_MESH)?;
    let crate_mesh = read_content(content_root, CRATE_MESH)?;
    check_json_asset(&crate_mesh, CRATE_MESH)?;
    let sand = read_content(content_root, SAND_TEX)?;
    check_prefix(&sand, &PNG_MAGIC, SAND_TEX, "PNG")?;
    let stone = read_content(content_root, STONE_TEX)?;
    check_prefix(&stone, &PNG_MAGIC, STONE_TEX, "PNG")?;
    let arena_manifest = read_content(content_root, ARENA_MANIFEST)?;
    let arena_text = core::str::from_utf8(&arena_manifest).map_err(|_| A2Error::BadMagic {
        file: ARENA_MANIFEST.to_string(),
        expected: "JSON",
    })?;
    check_manifest(arena_text, ARENA_MANIFEST, "src/crate.gltf")?;
    check_manifest(arena_text, ARENA_MANIFEST, "src/walls.gltf")?;
    check_manifest(arena_text, ARENA_MANIFEST, "textures/sand_albedo.png")?;
    check_manifest(arena_text, ARENA_MANIFEST, "textures/stone_albedo.png")?;
    let fighters_manifest = read_content(content_root, FIGHTERS_MANIFEST)?;
    let fighters_text =
        core::str::from_utf8(&fighters_manifest).map_err(|_| A2Error::BadMagic {
            file: FIGHTERS_MANIFEST.to_string(),
            expected: "JSON",
        })?;
    check_manifest(fighters_text, FIGHTERS_MANIFEST, "CC0-1.0")?;
    check_manifest(fighters_text, FIGHTERS_MANIFEST, "hero")?;
    Ok(licensed_sources(
        hero.len(),
        opp.len(),
        walls.len(),
        crate_mesh.len(),
        sand.len(),
        stone.len(),
    ))
}

/// Asset bundle for the run; hero and material lookups prove search answers.
fn build_bundle() -> Result<BundleView, A2Error> {
    let mut bundle = BundleView::new();
    for (kind, name, tags) in [
        (AssetKind::Mesh, "hero_body", &["fighter", "hero"][..]),
        (AssetKind::Mesh, "opp_body", &["fighter", "opponent"][..]),
        (AssetKind::Mesh, "walls", &["ct8", "wall"][..]),
        (AssetKind::Mesh, "crate", &["prop", "ct8"][..]),
        (AssetKind::Material, "hero-mat", &["fighter"][..]),
        (AssetKind::Texture, "sand_albedo", &["albedo", "ct8"][..]),
        (AssetKind::Texture, "stone_albedo", &["albedo", "ct8"][..]),
    ] {
        bundle
            .push(BundleEntry::new(kind, name, tags))
            .map_err(|error| A2Error::Browser(error.to_string()))?;
    }
    let hero_hits = search(&bundle, &SearchQuery::new("hero", None, None))
        .map_err(|error| A2Error::Browser(error.to_string()))?;
    if !hero_hits.iter().any(|hit| hit.name == "hero_body") {
        return Err(A2Error::Check("search `hero` missed hero_body".to_string()));
    }
    let mat_hits = search(
        &bundle,
        &SearchQuery::new("", Some(AssetKind::Material), None),
    )
    .map_err(|error| A2Error::Browser(error.to_string()))?;
    if mat_hits.len() != 1 || mat_hits[0].name != "hero-mat" {
        return Err(A2Error::Check(
            "material search missed hero-mat".to_string(),
        ));
    }
    Ok(bundle)
}

/// Placed nodes plus receipts: hero id, placed lines, preview line and the
/// refusal of an unknown asset, which stages nothing.
struct PlacedScene {
    hero: ObjectId,
    placed: Vec<String>,
    preview_line: String,
    refusal: String,
}

/// Place wall, crate (browser preview next to the wall) and hero at fixed
/// slots, then refuse an unknown asset preview.
fn place_scene(shell: &mut Shell, bundle: &BundleView) -> Result<PlacedScene, A2Error> {
    let wall = deterministic_id(1)?;
    shell
        .insert_node(
            wall,
            ObjectId::NIL,
            Transform::new([-2.0, 1.5, -8.0], [0.0, 0.0, 0.0, 1.0], [8.0, 3.0, 1.0]),
        )
        .map_err(|error| A2Error::Shell(error.to_string()))?;
    let actors = vec![
        BrowserActor::new(2, "wall_north", -2.0, 1.5, -8.0, 8.0, 3.0, 1.0, 0.0),
        BrowserActor::new(3, "crate_proto", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, 0.0),
    ];
    let intent = Intent {
        object: ObjectRef::Id(3),
        wants: vec![Want {
            relation: Relation::NextTo,
            target: Target::Object(2),
            distance_cm: None,
        }],
    };
    let preview = preview_drop(bundle, "crate", &intent, &actors, shell.revision())
        .map_err(|error| A2Error::Browser(error.to_string()))?;
    let preview_line = preview.effects.join("; ");
    let placed_crate = Transform::new(
        [preview.x, preview.y, preview.z],
        yaw_quat(preview.yaw),
        [1.0, 1.0, 1.0],
    );
    let krate = deterministic_id(2)?;
    shell
        .insert_node(krate, ObjectId::NIL, placed_crate)
        .map_err(|error| A2Error::Shell(error.to_string()))?;
    let hero = deterministic_id(3)?;
    shell
        .insert_node(
            hero,
            ObjectId::NIL,
            Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        )
        .map_err(|error| A2Error::Shell(error.to_string()))?;
    let placed = vec![
        format!("{wall} wall_north from walls mesh at (-2.00, 1.50, -8.00)"),
        format!(
            "{krate} crate_a from crate mesh at ({:.2}, {:.2}, {:.2})",
            preview.x, preview.y, preview.z
        ),
        format!("{hero} hero from hero_body mesh at (0.00, 0.90, 0.00)"),
    ];
    let bad_intent = Intent {
        object: ObjectRef::Id(3),
        wants: Vec::new(),
    };
    let refusal = match preview_drop(
        bundle,
        "no-such-mesh",
        &bad_intent,
        &actors,
        shell.revision(),
    ) {
        Ok(_) => {
            return Err(A2Error::Check(
                "unknown asset previewed instead of refusing".to_string(),
            ));
        }
        Err(error) => format!("unknown asset refused: {error}"),
    };
    Ok(PlacedScene {
        hero,
        placed,
        preview_line,
        refusal,
    })
}

/// Inspector receipt plus the refusal of an out-of-range lane.
struct InspectorEdit {
    component_edit: String,
    refusal: String,
}

/// Schema edit of the hero pose lane through the service.
fn inspector_edit(shell: &mut Shell, hero: ObjectId) -> Result<InspectorEdit, A2Error> {
    if !shell.select(hero) {
        return Err(A2Error::Check("hero selection failed".to_string()));
    }
    let before = shell
        .document()
        .hierarchy
        .local(hero)
        .ok_or_else(|| A2Error::Check("hero pose missing".to_string()))?;
    let revision_before_bad = shell.revision();
    let refusal = match shell.auto_edit("pose", "pose.px", "20000") {
        Ok(_) => {
            return Err(A2Error::Check(
                "out-of-range pose.px applied instead of refusing".to_string(),
            ));
        }
        Err(error) => format!("out-of-range pose.px refused: {error}"),
    };
    if shell.revision() != revision_before_bad {
        return Err(A2Error::Check(
            "refused inspector edit moved the revision".to_string(),
        ));
    }
    shell
        .auto_edit("pose", "pose.px", "2.5")
        .map_err(|error| A2Error::Auto(error.to_string()))?;
    let after = shell
        .document()
        .hierarchy
        .local(hero)
        .ok_or_else(|| A2Error::Check("hero pose missing after edit".to_string()))?;
    if (after.position[0] - 2.5).abs() > 1e-6 {
        return Err(A2Error::Check("pose.px edit did not land".to_string()));
    }
    Ok(InspectorEdit {
        component_edit: format!(
            "node {hero} pose.px {:.2} -> {:.2} (inspector schema edit)",
            before.position[0], after.position[0]
        ),
        refusal,
    })
}

/// Material receipt plus the refusal of an unknown material.
struct MaterialEdit {
    material_edit: String,
    refusal: String,
}

/// Journaled material bind through the command service.
fn material_step(
    shell: &mut Shell,
    bundle: &BundleView,
    hero: ObjectId,
) -> Result<MaterialEdit, A2Error> {
    let refusal = match assign_material(bundle, shell, hero, "no-such-mat") {
        Ok(_) => {
            return Err(A2Error::Check(
                "unknown material bound instead of refusing".to_string(),
            ));
        }
        Err(error) => format!("unknown material refused: {error}"),
    };
    let revision_before_mat = shell.revision();
    let material_edit = assign_material(bundle, shell, hero, "hero-mat")?;
    if shell.revision() != revision_before_mat + 1 {
        return Err(A2Error::Check(
            "material edit did not bump the revision".to_string(),
        ));
    }
    if !shell
        .material_of(hero)
        .is_some_and(|name| name.as_str() == "hero-mat")
    {
        return Err(A2Error::Check(
            "material binding missing after the edit".to_string(),
        ));
    }
    Ok(MaterialEdit {
        material_edit,
        refusal,
    })
}

/// Live save outputs: revision, node count, scene bytes, viewport PNG.
struct LiveSave {
    revision: u64,
    nodes: usize,
    bytes: Vec<u8>,
    png: Vec<u8>,
    method: String,
}

/// Write one evidence file or stop with its path.
fn write_out(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), A2Error> {
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|error| A2Error::Io {
        path: path.to_string_lossy().into_owned(),
        reason: error.to_string(),
    })
}

/// Save the live document plus rows, journal and viewport PNG.
fn save_live(
    shell: &Shell,
    out_dir: &Path,
    exe_capture: Option<ExeCapture<'_>>,
) -> Result<LiveSave, A2Error> {
    let revision = shell.revision();
    let nodes = shell.document().hierarchy.len();
    let bytes = save_to_bytes(shell.document());
    write_out(out_dir, "scene.edoc", &bytes)?;
    let mut rows_text = String::new();
    for row in shell.rows() {
        rows_text.push_str(&format!(
            "{} parent={} depth={} pos=({:.2},{:.2},{:.2})\n",
            row.id,
            row.parent,
            row.depth,
            row.local.position[0],
            row.local.position[1],
            row.local.position[2]
        ));
    }
    write_out(out_dir, "rows.txt", rows_text.as_bytes())?;
    let mut journal_text = String::new();
    for entry in &shell.journal().past {
        for line in &entry.effects {
            journal_text.push_str(&format!("r{} {line}\n", entry.result_revision));
        }
    }
    write_out(out_dir, "journal.txt", journal_text.as_bytes())?;
    let (png, method) = match exe_capture.and_then(|capture| capture(shell)) {
        Some(pair) => pair,
        None => (
            capture_viewport_png(shell)?,
            "headless-cpu-raster (library fallback; the editor-exe record \
             overwrites these PNGs with window-path captures)"
                .to_string(),
        ),
    };
    Ok(LiveSave {
        revision,
        nodes,
        bytes,
        png,
        method,
    })
}

/// Reopened save outputs: resaved bytes plus the capture lines.
struct ReopenedSave {
    resaved: Vec<u8>,
    captures: Vec<String>,
}

/// Close, reopen and byte-compare with the material binding intact.
fn reopen_verify(
    out_dir: &Path,
    hero: ObjectId,
    live: &LiveSave,
    exe_capture: Option<ExeCapture<'_>>,
) -> Result<ReopenedSave, A2Error> {
    let saved_back = read_content(out_dir, "scene.edoc")?;
    if saved_back != live.bytes {
        return Err(A2Error::Check(
            "scene file differs from saved bytes".to_string(),
        ));
    }
    let reopened =
        load_from_bytes(&saved_back).map_err(|error| A2Error::Save(error.to_string()))?;
    let resaved = save_to_bytes(&reopened);
    write_out(out_dir, "reopened.edoc", &resaved)?;
    if resaved != live.bytes {
        return Err(A2Error::Check(
            "reopened scene resaved to different bytes (truncation)".to_string(),
        ));
    }
    if reopened.revision != live.revision {
        return Err(A2Error::Check("reopened revision moved".to_string()));
    }
    if reopened.hierarchy.len() != live.nodes {
        return Err(A2Error::Check("reopened node count moved".to_string()));
    }
    if !reopened
        .materials
        .get(&hero.to_string())
        .is_some_and(|name| name.as_str() == "hero-mat")
    {
        return Err(A2Error::Check(
            "reopened document lost the hero-mat binding".to_string(),
        ));
    }
    let needle = b"hero-mat";
    if !live.bytes.windows(needle.len()).any(|w| w == needle) {
        return Err(A2Error::Check(
            "scene bytes do not carry the bound material name".to_string(),
        ));
    }
    let mut reopened_shell = Shell::new();
    reopened_shell.open_document(reopened.clone());
    let png = match exe_capture.and_then(|capture| capture(&reopened_shell)) {
        Some((png, _)) => png,
        None => capture_viewport_png(&reopened_shell)?,
    };
    write_out(out_dir, "viewport.png", &live.png)?;
    write_out(out_dir, "reopened.png", &png)?;
    let captures = vec![
        format!(
            "viewport.png ({} bytes, fnv {})",
            live.png.len(),
            fnv_hex(&live.png)
        ),
        format!("reopened.png ({} bytes, fnv {})", png.len(), fnv_hex(&png)),
    ];
    Ok(ReopenedSave { resaved, captures })
}

/// Run the full recorded session: import, place three objects, edit one
/// component plus one material, save, close, reopen with a byte-compare.
/// Writes `scene.edoc`, `reopened.edoc`, `rows.txt`, `journal.txt` and
/// `report.txt` into `out_dir` and returns the report.
///
/// `exe_capture` supplies the window-path PNGs when the caller is the editor
/// executable (GPU offscreen first, then the live panel); the library
/// fallback rasterizes headless. Either way the same shell is captured, so
/// the PNGs always show the recorded document.
pub fn run_session(
    content_root: &Path,
    out_dir: &Path,
    exe_capture: Option<ExeCapture<'_>>,
) -> Result<SessionReport, A2Error> {
    std::fs::create_dir_all(out_dir).map_err(|error| A2Error::Io {
        path: out_dir.to_string_lossy().into_owned(),
        reason: error.to_string(),
    })?;
    let sources = read_sources(content_root)?;
    let bundle = build_bundle()?;
    let mut shell = Shell::new();
    let placed_scene = place_scene(&mut shell, &bundle)?;
    let hero_id = placed_scene.hero;
    let placed = placed_scene.placed;
    let preview_line = placed_scene.preview_line;
    let mut refusals = vec![placed_scene.refusal];
    let inspector = inspector_edit(&mut shell, hero_id)?;
    refusals.push(inspector.refusal);
    let component_edit = inspector.component_edit;

    let material = material_step(&mut shell, &bundle, hero_id)?;
    refusals.push(material.refusal);
    let material_edit = material.material_edit;
    let live = save_live(&shell, out_dir, exe_capture)?;
    drop(shell);
    let reopened_out = reopen_verify(out_dir, hero_id, &live, exe_capture)?;
    let bytes_match = reopened_out.resaved == live.bytes;
    let report = SessionReport {
        sources,
        placed,
        preview_line,
        component_edit,
        material_edit,
        refusals,
        saved: format!(
            "scene.edoc ({} bytes, fnv {})",
            live.bytes.len(),
            fnv_hex(&live.bytes)
        ),
        reopened: format!(
            "reopened.edoc ({} bytes, fnv {})",
            reopened_out.resaved.len(),
            fnv_hex(&reopened_out.resaved)
        ),
        bytes_match,
        revision: live.revision,
        nodes: live.nodes,
        captures: reopened_out.captures,
        capture_method: live.method,
    };
    write_out(out_dir, "report.txt", report.render().as_bytes())?;
    Ok(report)
}

/// Checkout root holding `content/`, resolved at runtime so the binary
/// bakes no absolute source path. Probes the launch directory then the
/// executable directory upward for `content/`; falls back to the launch
/// directory. Pure besides the directory probes.
pub(crate) fn checkout_root() -> PathBuf {
    fn root_above(dir: &Path) -> Option<PathBuf> {
        let mut dir = dir;
        loop {
            if dir.join("content").is_dir() {
                return Some(dir.to_path_buf());
            }
            dir = dir.parent()?;
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(root) = root_above(&cwd) {
            return root;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(root) = root_above(dir) {
                return root;
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Content root of the lane checkout holding `content/`, resolved at
/// runtime from the launch or executable directory.
pub fn lane_content_root() -> PathBuf {
    checkout_root().join("content")
}

/// Default evidence dir for the recorded run inside the lane checkout.
pub fn lane_evidence_dir() -> PathBuf {
    checkout_root().join("target").join("evidence").join("a2")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_magics_are_refused_with_their_file() {
        let dir = std::env::temp_dir().join("ev7-a2-badmagic");
        let _ = std::fs::create_dir_all(&dir);
        let not_glb = dir.join("x.glb");
        std::fs::write(&not_glb, b"nope").unwrap();
        let bytes = std::fs::read(&not_glb).unwrap();
        assert!(bytes.len() < 4 || bytes[0..4] != GLB_MAGIC);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn material_binding_refuses_unknown_assets_and_plain_meshes() {
        let mut bundle = BundleView::new();
        bundle
            .push(BundleEntry::new(AssetKind::Mesh, "crate", &["prop"]))
            .unwrap();
        bundle
            .push(BundleEntry::new(
                AssetKind::Material,
                "hero-mat",
                &["fighter"],
            ))
            .unwrap();
        let mut shell = Shell::new();
        let node = deterministic_id(11).unwrap();
        shell
            .insert_node(node, ObjectId::NIL, Transform::identity())
            .unwrap();
        assert!(assign_material(&bundle, &mut shell, node, "missing").is_err());
        assert!(assign_material(&bundle, &mut shell, node, "crate").is_err());
        assert!(assign_material(
            &bundle,
            &mut shell,
            deterministic_id(99).unwrap(),
            "hero-mat"
        )
        .is_err());
        // Within-run identity holds: the same slot is the same node.
        assert_eq!(deterministic_id(11).unwrap(), node);
        assert_ne!(deterministic_id(11).unwrap(), deterministic_id(12).unwrap());
        // The known material binds on the known node.
        assert!(assign_material(&bundle, &mut shell, node, "hero-mat").is_ok());
    }

    #[test]
    fn deterministic_ids_are_v4_shaped_and_stable() {
        let wall = deterministic_id(1).unwrap();
        let krate = deterministic_id(2).unwrap();
        let hero = deterministic_id(3).unwrap();
        assert_ne!(wall, krate);
        assert_ne!(krate, hero);
        assert_eq!(deterministic_id(1).unwrap(), wall);
        assert_eq!(wall.as_uuid().get_version_num(), 4);
        assert_eq!(wall.to_string().len(), 36);
    }

    #[test]
    fn headless_png_is_deterministic_and_valid() {
        let mut shell = Shell::new();
        shell
            .insert_node(
                deterministic_id(21).unwrap(),
                ObjectId::NIL,
                Transform::new([0.0, 0.9, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            )
            .unwrap();
        let first = capture_viewport_png(&shell).unwrap();
        let second = capture_viewport_png(&shell).unwrap();
        assert_eq!(first, second);
        assert_eq!(&first[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(
            &first[first.len() - 12..],
            &[0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130]
        );
        // Unknown sizes refuse instead of panicking.
        assert!(encode_png_rgba8(&[], 0, 8).is_err());
        assert!(encode_png_rgba8(&[0u8; 4], 1, 1).is_ok());
    }

    #[test]
    fn full_session_roundtrips_on_a_scratch_dir() {
        let content = lane_content_root();
        if !content.join(HERO_MESH).exists() {
            return;
        }
        let scratch = std::env::temp_dir().join("ev7-a2-scratch");
        let _ = std::fs::remove_dir_all(&scratch);
        let report = run_session(&content, &scratch, None).unwrap();
        assert!(report.bytes_match);
        assert_eq!(report.nodes, 3);
        assert_eq!(report.revision, 5);
        assert_eq!(report.refusals.len(), 3);
        assert_eq!(report.captures.len(), 2);
        let saved = std::fs::read(scratch.join("scene.edoc")).unwrap();
        let reopened = std::fs::read(scratch.join("reopened.edoc")).unwrap();
        assert_eq!(saved, reopened);
        for png in ["viewport.png", "reopened.png"] {
            let bytes = std::fs::read(scratch.join(png)).unwrap();
            assert_eq!(
                &bytes[0..8],
                &[137, 80, 78, 71, 13, 10, 26, 10],
                "{png} is a PNG"
            );
            assert!(!bytes.is_empty(), "{png} holds pixels");
        }
        // Deterministic ids: the placed lines name the fixed evidence slots.
        assert!(report.placed[0].starts_with(&deterministic_id(1).unwrap().to_string()));
        assert!(report.placed[1].starts_with(&deterministic_id(2).unwrap().to_string()));
        assert!(report.placed[2].starts_with(&deterministic_id(3).unwrap().to_string()));
        let _ = std::fs::remove_dir_all(&scratch);
    }

    #[test]
    fn two_runs_write_identical_scene_bytes() {
        let content = lane_content_root();
        if !content.join(HERO_MESH).exists() {
            return;
        }
        let first_dir = std::env::temp_dir().join("ev7-a2-cross-a");
        let second_dir = std::env::temp_dir().join("ev7-a2-cross-b");
        let _ = std::fs::remove_dir_all(&first_dir);
        let _ = std::fs::remove_dir_all(&second_dir);
        let first = run_session(&content, &first_dir, None).unwrap();
        let second = run_session(&content, &second_dir, None).unwrap();
        assert_eq!(first.placed, second.placed);
        assert_eq!(first.saved, second.saved);
        let a = std::fs::read(first_dir.join("scene.edoc")).unwrap();
        let b = std::fs::read(second_dir.join("scene.edoc")).unwrap();
        assert_eq!(a, b, "cross-run scene.edoc bytes match");
        let pa = std::fs::read(first_dir.join("viewport.png")).unwrap();
        let pb = std::fs::read(second_dir.join("viewport.png")).unwrap();
        assert_eq!(pa, pb, "cross-run viewport PNG bytes match");
        let _ = std::fs::remove_dir_all(&first_dir);
        let _ = std::fs::remove_dir_all(&second_dir);
    }

    #[test]
    fn checkout_roots_resolve_relative_without_baked_prefix() {
        assert_eq!(lane_content_root(), checkout_root().join("content"));
        assert_eq!(
            lane_evidence_dir(),
            checkout_root().join("target").join("evidence").join("a2")
        );
        assert_eq!(
            lane_content_root()
                .file_name()
                .expect("content root names content"),
            "content"
        );
    }

    #[test]
    fn missing_content_names_its_first_file() {
        let empty = std::env::temp_dir().join("ev7-a2-missing-content");
        let _ = std::fs::remove_dir_all(&empty);
        std::fs::create_dir_all(&empty).unwrap();
        let out = std::env::temp_dir().join("ev7-a2-missing-out");
        let _ = std::fs::remove_dir_all(&out);
        let error = run_session(&empty, &out, None).unwrap_err();
        assert!(
            error.to_string().contains(HERO_MESH),
            "missing content names {HERO_MESH}: {error}"
        );
        let _ = std::fs::remove_dir_all(&empty);
        let _ = std::fs::remove_dir_all(&out);
    }
}

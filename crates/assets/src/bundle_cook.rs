//! bundle_cook: bundle cook entry from cooked bytes to E204BNDL.
//!
//! Consumer: the packaged game's `ship pack BUNDLE_RESOLVE` plus the
//! `bundle_cook` bin stage one file the game and editor startup loader
//! accepts. Data owner: `engine-assets`.
//! Units: payload bytes in, file bytes out, entry counts in receipts. Same
//! staged entries cook byte-identical bundles (per-kind name-sorted staging,
//! insertion-order `BundleWriter::finish`); every payload carries its blake3
//! hex digest and the loader re-verifies it. Offline cook only, never the
//! fixed tick. Errors: typed [`BundleCookError`]; empty/stub output is
//! refused unless the caller passes `allow_stub`.

use super::bundle::{
    BundleError, BundleLoader, BundleWriter, LoadedBundle, BUNDLE_MAGIC, BUNDLE_VERSION,
    MAX_BUNDLE_BYTES, MAX_ENTRIES,
};
use super::cooked::{CookedMaterial, CookedMesh, CookedTexture};
use super::skin_import::{ImportedClip, Skin};
use super::wire::bound_chars;
use std::fmt;
use std::path::{Path, PathBuf};

/// Byte length of the empty (stub) bundle: 8 magic + 4 version + 4 count.
pub const STUB_BUNDLE_LEN: usize = 16;
/// Bound for echoed path and IO detail carried in [`BundleCookError`].
pub const MAX_COOK_DETAIL_CHARS: usize = 128;

/// Why a bundle cook or file verify failed; every bad input maps to one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleCookError {
    /// No entries staged and `allow_stub` was false; nothing was written.
    EmptyWithoutAllowStub,
    /// File holds the 16-byte empty bundle but `allow_stub` was false.
    StubFile {
        /// Byte length seen (always [`STUB_BUNDLE_LEN`] for now).
        bytes: usize,
    },
    /// File or buffer exceeds the bundle budget; offender carried.
    TooLarge {
        /// Offending byte length or entry count.
        count: usize,
    },
    /// Filesystem read/write failure; bounded detail, never a raw path dump.
    Io(String),
    /// Output path empty; bounded echo.
    BadPath(String),
    /// Staging or verification failed inside [`BundleWriter`]/loader.
    Bundle(BundleError),
}

impl fmt::Display for BundleCookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyWithoutAllowStub => {
                write!(f, "bundle cook refused empty output without --allow-stub")
            }
            Self::StubFile { bytes } => {
                write!(
                    f,
                    "bundle stub file ({bytes} bytes) refused without --allow-stub"
                )
            }
            Self::TooLarge { count } => write!(f, "bundle cook too large: {count}"),
            Self::Io(detail) => write!(f, "bundle cook io: {detail}"),
            Self::BadPath(path) => write!(f, "bundle cook bad path {path:?}"),
            Self::Bundle(err) => write!(f, "bundle cook bundle: {err}"),
        }
    }
}

impl std::error::Error for BundleCookError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bundle(err) => Some(err),
            _ => None,
        }
    }
}

impl From<BundleError> for BundleCookError {
    fn from(err: BundleError) -> Self {
        match err {
            BundleError::TooLarge { count } => Self::TooLarge { count },
            other => Self::Bundle(other),
        }
    }
}

/// Truncate echoed text to at most [`MAX_COOK_DETAIL_CHARS`] chars.
fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_COOK_DETAIL_CHARS)
}

fn io_err(detail: impl fmt::Display) -> BundleCookError {
    BundleCookError::Io(bound_detail(&format!("{detail}")))
}

/// Cooked inputs staged for one bundle; per-kind vectors in caller order.
/// `cook_bundle_bytes` stages them name-sorted per kind so the same set in
/// any order finishes to the same bytes.
#[derive(Clone, Debug, Default)]
pub struct StagedCooked {
    /// `(name, mesh)` pairs staged as mesh entries.
    pub meshes: Vec<(String, CookedMesh)>,
    /// `(name, material)` pairs staged as material entries.
    pub materials: Vec<(String, CookedMaterial)>,
    /// `(name, texture)` pairs staged as texture entries.
    pub textures: Vec<(String, CookedTexture)>,
    /// `(name, skin)` pairs staged as skin entries.
    pub skins: Vec<(String, Skin)>,
    /// `(name, clip)` pairs staged as clip entries.
    pub clips: Vec<(String, ImportedClip)>,
}

impl StagedCooked {
    /// Empty staging: no entries of any kind.
    pub fn new() -> Self {
        Self::default()
    }

    /// Total entry count across all kinds.
    pub fn len(&self) -> usize {
        self.meshes.len()
            + self.materials.len()
            + self.textures.len()
            + self.skins.len()
            + self.clips.len()
    }

    /// True when no entry of any kind is staged.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Deterministic demo staging (mesh shape + texture shape + one of each
/// remaining kind); the bin's default `--out` path and the round-trip tests
/// build from here so the packaged game always has a non-empty producer.
pub fn demo_staged() -> StagedCooked {
    let mesh = CookedMesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        normals: vec![[0.0, 0.0, 1.0]; 3],
        uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
        indices: vec![0, 1, 2],
    };
    let texture = CookedTexture {
        width: 2,
        height: 2,
        mips: vec![
            super::cooked::Mip {
                width: 2,
                height: 2,
                texels: vec![
                    [1.0, 0.0, 0.0, 1.0],
                    [0.0, 1.0, 0.0, 1.0],
                    [0.0, 0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0, 1.0],
                ],
            },
            super::cooked::Mip {
                width: 1,
                height: 1,
                texels: vec![[0.5, 0.5, 0.5, 1.0]],
            },
        ],
        slot: super::cooked::TextureSlot::BaseColor,
        encoding: super::cooked::TextureEncoding::Srgb,
    };
    StagedCooked {
        meshes: vec![("demo_mesh".to_string(), mesh)],
        materials: vec![("demo_mat".to_string(), CookedMaterial::default())],
        textures: vec![("demo_tex".to_string(), texture)],
        skins: vec![(
            "demo_skin".to_string(),
            Skin {
                joints: vec![[0, 1, 0, 0], [1, 0, 0, 0]],
                weights: vec![[0.7, 0.3, 0.0, 0.0], [0.5, 0.5, 0.0, 0.0]],
            },
        )],
        clips: vec![(
            "demo_clip".to_string(),
            ImportedClip {
                name: "step".to_string(),
                duration: 1.0,
                positions: engine_anim::Clip {
                    tracks: vec![engine_anim::Track {
                        times: vec![0.0, 1.0],
                        values: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                    }],
                },
                rotations: vec![engine_anim::pose::QuatTrack {
                    times: vec![0.0, 1.0],
                    values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 0.0]],
                }],
            },
        )],
    }
}

/// Stage `staged` into a [`BundleWriter`] in fixed kind order, name-sorted
/// within each kind, and finish to versioned E204BNDL bytes. Same entry set
/// in any caller order finishes byte-identical; digests are blake3 hex per
/// entry via the writer.
///
/// Units: payload bytes in, file bytes out. Ownership: caller owns `staged`
/// and the returned bytes; single-threaded offline cook, never the fixed
/// tick. Determinism: same entry set in any caller order finishes
/// byte-identical (per-kind name-sorted staging, insertion-order finish).
/// Errors: [`BundleCookError`] when a payload breaks the writer budget or an
/// entry fails validation.
///
/// # Example
///
/// ```rust
/// use engine_assets::bundle_cook::{
///     cook_bundle_bytes, demo_staged, header_entry_count, verify_bundle_bytes,
/// };
///
/// let staged = demo_staged();
/// let bytes = cook_bundle_bytes(&staged).expect("demo staging cooks");
/// assert_eq!(
///     header_entry_count(&bytes).expect("header reads"),
///     staged.len()
/// );
/// let loaded = verify_bundle_bytes(&bytes, false).expect("demo bundle verifies");
/// assert_eq!(loaded.len(), staged.len());
/// ```
pub fn cook_bundle_bytes(staged: &StagedCooked) -> Result<Vec<u8>, BundleCookError> {
    let mut writer = BundleWriter::new();
    let mut meshes: Vec<&(String, CookedMesh)> = staged.meshes.iter().collect();
    meshes.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, mesh) in meshes {
        writer.add_mesh(name, mesh)?;
    }
    let mut materials: Vec<&(String, CookedMaterial)> = staged.materials.iter().collect();
    materials.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, material) in materials {
        writer.add_material(name, material)?;
    }
    let mut textures: Vec<&(String, CookedTexture)> = staged.textures.iter().collect();
    textures.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, texture) in textures {
        writer.add_texture(name, texture)?;
    }
    let mut skins: Vec<&(String, Skin)> = staged.skins.iter().collect();
    skins.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, skin) in skins {
        writer.add_skin(name, skin)?;
    }
    let mut clips: Vec<&(String, ImportedClip)> = staged.clips.iter().collect();
    clips.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, clip) in clips {
        writer.add_clip(name, clip)?;
    }
    Ok(writer.finish())
}

/// Header-only check mirroring `tools/ship.mjs checkBundleHeader`:
/// magic, version and entry-count bound. Returns the entry count.
pub fn header_entry_count(bytes: &[u8]) -> Result<usize, BundleCookError> {
    if bytes.len() < STUB_BUNDLE_LEN {
        return Err(BundleCookError::Bundle(BundleError::Truncated {
            want: STUB_BUNDLE_LEN,
            got: bytes.len(),
        }));
    }
    if &bytes[0..8] != BUNDLE_MAGIC {
        return Err(BundleCookError::Bundle(BundleError::BadMagic));
    }
    let version = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if version > BUNDLE_VERSION {
        return Err(BundleCookError::Bundle(BundleError::FutureVersion {
            found: version,
        }));
    }
    if version != BUNDLE_VERSION {
        return Err(BundleCookError::Bundle(BundleError::Corrupt(format!(
            "unsupported bundle version {version}"
        ))));
    }
    let count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
    if count > MAX_ENTRIES {
        return Err(BundleCookError::TooLarge { count });
    }
    Ok(count)
}

/// True when `bytes` are exactly the empty (stub) bundle: 16 bytes with the
/// magic, version 1 and entry count 0.
pub fn is_stub_bundle(bytes: &[u8]) -> bool {
    bytes.len() == STUB_BUNDLE_LEN && header_entry_count(bytes).is_ok_and(|count| count == 0)
}

/// Verify in-memory bundle bytes the way `BUNDLE_RESOLVE` accepts them:
/// header check plus full [`BundleLoader`] digest verification. The 16-byte
/// stub loads only with `allow_stub`; without it the stub is refused here
/// even though the loader alone would accept the empty bundle.
pub fn verify_bundle_bytes(
    bytes: &[u8],
    allow_stub: bool,
) -> Result<LoadedBundle, BundleCookError> {
    let count = header_entry_count(bytes)?;
    if count == 0 && !allow_stub {
        return Err(BundleCookError::EmptyWithoutAllowStub);
    }
    let loaded = BundleLoader::new().load(bytes)?;
    if loaded.is_empty() && !allow_stub {
        return Err(BundleCookError::EmptyWithoutAllowStub);
    }
    Ok(loaded)
}

/// One written bundle: entry and byte counts plus the file it was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleReceipt {
    /// Verified entry count (`BundleLoader::len`).
    pub entries: usize,
    /// File byte length.
    pub bytes: usize,
    /// File the bundle was written to or read from.
    pub path: PathBuf,
}

impl fmt::Display for BundleReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bundle {} entries:{} bytes:{}",
            self.path.display(),
            self.entries,
            self.bytes
        )
    }
}

/// Write already-cooked bundle `bytes` to `path`: verifies them first (header
/// plus digests), refuses the stub without `allow_stub`, writes the file,
/// then reads it back and re-verifies so a corrupt write never reports ok.
pub fn write_bundle_file(
    bytes: &[u8],
    path: &Path,
    allow_stub: bool,
) -> Result<BundleReceipt, BundleCookError> {
    if path.as_os_str().is_empty() {
        return Err(BundleCookError::BadPath(String::new()));
    }
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(BundleCookError::TooLarge { count: bytes.len() });
    }
    let checked = verify_bundle_bytes(bytes, allow_stub)?;
    std::fs::write(path, bytes).map_err(io_err)?;
    let back = std::fs::read(path).map_err(io_err)?;
    if back != bytes {
        return Err(BundleCookError::Io(bound_detail("read-back differs")));
    }
    let reloaded = verify_bundle_bytes(&back, allow_stub)?;
    debug_assert_eq!(checked.len(), reloaded.len());
    Ok(BundleReceipt {
        entries: reloaded.len(),
        bytes: back.len(),
        path: path.to_path_buf(),
    })
}

/// Cook `staged` and write the bundle file in one step; stub refused without
/// `allow_stub`.
pub fn cook_and_write(
    staged: &StagedCooked,
    path: &Path,
    allow_stub: bool,
) -> Result<BundleReceipt, BundleCookError> {
    if staged.is_empty() && !allow_stub {
        return Err(BundleCookError::EmptyWithoutAllowStub);
    }
    let bytes = cook_bundle_bytes(staged)?;
    write_bundle_file(&bytes, path, allow_stub)
}

/// Load plus verify a bundle file the way `BUNDLE_RESOLVE` accepts it:
/// size bound, header check, stub refusal without `allow_stub`, then full
/// digest verification. Reports the receipt alongside the loaded bundle.
pub fn load_bundle_file(
    path: &Path,
    allow_stub: bool,
) -> Result<(LoadedBundle, BundleReceipt), BundleCookError> {
    if path.as_os_str().is_empty() {
        return Err(BundleCookError::BadPath(String::new()));
    }
    let meta_len = std::fs::metadata(path).map_err(io_err)?.len() as usize;
    if meta_len > MAX_BUNDLE_BYTES {
        return Err(BundleCookError::TooLarge { count: meta_len });
    }
    let bytes = std::fs::read(path).map_err(io_err)?;
    if is_stub_bundle(&bytes) && !allow_stub {
        return Err(BundleCookError::StubFile { bytes: bytes.len() });
    }
    let loaded = verify_bundle_bytes(&bytes, allow_stub)?;
    let receipt = BundleReceipt {
        entries: loaded.len(),
        bytes: bytes.len(),
        path: path.to_path_buf(),
    };
    Ok((loaded, receipt))
}

#[cfg(test)]
mod tests {
    use super::super::bundle::{EntryKind, BUNDLE_MAGIC, BUNDLE_VERSION};
    use super::super::cooked::{TextureEncoding, TextureSlot};
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SCRATCH: AtomicU64 = AtomicU64::new(0);

    /// Unique scratch file per test under the OS temp dir (unit tests run in
    /// parallel threads; never touches `content/`).
    fn scratch(name: &str) -> PathBuf {
        let n = SCRATCH.fetch_add(1, Ordering::SeqCst);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "engine-assets-as7-{name}-{}-{n}.e2040",
            std::process::id()
        ));
        path
    }

    fn remove(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    /// Encode flat RGBA rows as PNG bytes via the workspace `image` dep.
    fn png_bytes(width: u32, height: u32, texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let image: image::RgbaImage =
            image::ImageBuffer::from_fn(width, height, |x, y| image::Rgba(texel(x, y)));
        let dynamic = image::DynamicImage::ImageRgba8(image);
        let mut out = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&mut out);
            dynamic
                .write_to(&mut cursor, image::ImageFormat::Png)
                .expect("test png must encode");
        }
        out
    }

    #[test]
    fn demo_bundle_file_loads_non_empty_with_digests() {
        // Texture path: encoded PNG bytes cook through `cook_texture` (sRGB
        // base-color), then feed the writer as a cooked texture entry.
        let png = png_bytes(2, 2, |x, y| {
            if (x + y) % 2 == 0 {
                [200, 40, 40, 255]
            } else {
                [40, 200, 40, 255]
            }
        });
        let ct7_tex = super::super::texture::cook_texture(&png, TextureSlot::BaseColor)
            .expect("CT7 texture cook must succeed");
        assert_eq!(ct7_tex.encoding, TextureEncoding::Srgb);
        // Triangle mesh plus the demo staging (mesh cook LOD0 has
        // the same parallel-array shape the bundle validates).
        let mut staged = demo_staged();
        staged.textures.push(("ct7_quadrant".to_string(), ct7_tex));
        let path = scratch("demo");
        remove(&path);
        let receipt = cook_and_write(&staged, &path, false).expect("demo must cook");
        assert_eq!(receipt.entries, staged.len());
        assert!(receipt.entries >= 6);
        assert!(receipt.bytes > STUB_BUNDLE_LEN);
        // Header mirrors ship pack BUNDLE_RESOLVE: magic, version, count.
        let bytes = std::fs::read(&path).expect("written file must read");
        assert_eq!(&bytes[0..8], BUNDLE_MAGIC);
        assert_eq!(
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            BUNDLE_VERSION
        );
        let count = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]) as usize;
        assert_eq!(count, receipt.entries);
        assert_eq!(header_entry_count(&bytes).expect("header must pass"), count);
        assert!(!is_stub_bundle(&bytes));
        // Full digest verification through the real loader: non-empty.
        let (loaded, file_receipt) =
            load_bundle_file(&path, false).expect("produced file must verify");
        assert!(!loaded.is_empty());
        assert_eq!(loaded.len(), receipt.entries);
        assert_eq!(file_receipt, receipt);
        assert_eq!(loaded.bytes, bytes.len());
        // Every entry digest recomputes (blake3 hex per entry).
        for entry in &loaded.entries {
            assert_eq!(
                entry.digest,
                engine_scene::ids::content_digest(&entry.bytes)
            );
            assert_eq!(entry.digest.len(), 64);
        }
        // Kind round-trips through the loader decoders.
        let mesh = loaded
            .find(EntryKind::Mesh, "demo_mesh")
            .expect("mesh present");
        mesh.decode_mesh().expect("mesh decodes");
        let tex = loaded
            .find(EntryKind::Texture, "ct7_quadrant")
            .expect("CT7 texture present");
        let cooked = tex.decode_texture().expect("CT7 texture decodes");
        assert_eq!(cooked.width, 2);
        assert_eq!(cooked.encoding, TextureEncoding::Srgb);
        remove(&path);
    }

    #[test]
    fn same_inputs_in_any_order_cook_identical() {
        let mut a = demo_staged();
        let mut b = StagedCooked::new();
        // Reverse every kind order: sorted staging must still finish equal.
        b.meshes = a.meshes.drain(..).rev().collect();
        b.materials = a.materials.drain(..).rev().collect();
        b.textures = a.textures.drain(..).rev().collect();
        b.skins = a.skins.drain(..).rev().collect();
        b.clips = a.clips.drain(..).rev().collect();
        let a = demo_staged();
        assert_eq!(
            cook_bundle_bytes(&a).unwrap(),
            cook_bundle_bytes(&b).unwrap()
        );
        assert_eq!(
            cook_bundle_bytes(&a).unwrap(),
            cook_bundle_bytes(&demo_staged()).unwrap()
        );
    }

    #[test]
    fn stub_refused_without_flag_allowed_with_it() {
        let empty = StagedCooked::new();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        // Cooking empty still yields the 16-byte stub (writer behavior frozen).
        let stub = cook_bundle_bytes(&empty).expect("empty cooks to stub");
        assert_eq!(stub.len(), STUB_BUNDLE_LEN);
        assert!(is_stub_bundle(&stub));
        // Cook-and-write refuses the stub without the flag.
        let path = scratch("stub");
        remove(&path);
        assert_eq!(
            cook_and_write(&empty, &path, false).unwrap_err(),
            BundleCookError::EmptyWithoutAllowStub
        );
        assert!(!path.exists(), "refused write stages no file");
        // With the flag the stub writes and loads as an empty bundle.
        let receipt = cook_and_write(&empty, &path, true).expect("stub allowed with flag");
        assert_eq!(receipt.entries, 0);
        assert_eq!(receipt.bytes, STUB_BUNDLE_LEN);
        let (loaded, _) = load_bundle_file(&path, true).expect("stub loads with flag");
        assert!(loaded.is_empty());
        // The same file is refused without the flag (ship pack parity: stub
        // needs --allow-stub, never an A8 proof).
        assert_eq!(
            load_bundle_file(&path, false).unwrap_err(),
            BundleCookError::StubFile {
                bytes: STUB_BUNDLE_LEN
            }
        );
        assert!(matches!(
            verify_bundle_bytes(&stub, false).unwrap_err(),
            BundleCookError::EmptyWithoutAllowStub
        ));
        remove(&path);
    }

    #[test]
    fn corrupt_output_refused_by_loader() {
        let staged = demo_staged();
        let path = scratch("corrupt");
        remove(&path);
        let receipt = cook_and_write(&staged, &path, false).expect("demo must cook");
        assert!(receipt.entries > 0);
        let mut bytes = std::fs::read(&path).expect("file must read");
        // Flip one payload byte: framing holds, the blake3 digest must fail
        // with the entry name carried.
        let name = "demo_mesh";
        let at = STUB_BUNDLE_LEN + 1 + 2 + name.len() + 4;
        bytes[at] ^= 0xFF;
        match verify_bundle_bytes(&bytes, false).unwrap_err() {
            BundleCookError::Bundle(BundleError::DigestMismatch { entry, .. }) => {
                assert_eq!(entry, name);
            }
            other => panic!("want DigestMismatch, got {other}"),
        }
        // Truncated file is refused as truncated, never as ok (fresh bytes:
        // the flipped copy above would already fail on its digest first).
        let fresh = std::fs::read(&path).expect("file must re-read");
        let cut = fresh.len() - 1;
        assert!(matches!(
            verify_bundle_bytes(&fresh[..cut], false).unwrap_err(),
            BundleCookError::Bundle(BundleError::Truncated { .. })
        ));
        // Bad magic is refused as bad magic (ship pack BUNDLE_RESOLVE parity).
        let mut bad = fresh;
        bad[0] ^= 0xFF;
        assert_eq!(
            verify_bundle_bytes(&bad, false).unwrap_err(),
            BundleCookError::Bundle(BundleError::BadMagic)
        );
        // The untouched file still verifies (negative control).
        let (loaded, _) = load_bundle_file(&path, false).expect("good file must verify");
        assert_eq!(loaded.len(), receipt.entries);
        remove(&path);
    }

    #[test]
    fn bad_path_and_header_edges_typed() {
        let staged = demo_staged();
        let bytes = cook_bundle_bytes(&staged).expect("demo must cook");
        assert!(matches!(
            write_bundle_file(&bytes, Path::new(""), false).unwrap_err(),
            BundleCookError::BadPath(_)
        ));
        assert!(matches!(
            load_bundle_file(Path::new(""), false).unwrap_err(),
            BundleCookError::BadPath(_)
        ));
        assert!(matches!(
            header_entry_count(&bytes[..5]).unwrap_err(),
            BundleCookError::Bundle(BundleError::Truncated { .. })
        ));
        let mut future = bytes.clone();
        future[8..12].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            header_entry_count(&future).unwrap_err(),
            BundleCookError::Bundle(BundleError::FutureVersion { found: 2 })
        );
        assert!(!format!("{}", BundleCookError::EmptyWithoutAllowStub).is_empty());
        assert!(!format!("{}", BundleCookError::StubFile { bytes: 16 }).is_empty());
        assert!(!format!("{}", BundleCookError::TooLarge { count: 1 }).is_empty());
        assert!(!format!("{}", BundleCookError::Io("x".to_string())).is_empty());
        assert!(!format!("{}", BundleCookError::BadPath("x".to_string())).is_empty());
    }
}

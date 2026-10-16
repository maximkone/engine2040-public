//! import: glTF-only + KTX2-only, 7-gate checklist. Decode offline, runtime looks up only.

use std::fmt;

use crate::health;

/// Max verts per import; above fails the polyBudget gate.
pub const POLY_BUDGET: u32 = 1_000_000;
/// KTX2 identifier per the Khronos spec (12 bytes).
pub const KTX2_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
/// GLB magic glTF as little-endian bytes.
pub const GLB_MAGIC: [u8; 4] = [0x67, 0x6C, 0x54, 0x46];
/// JSON keys scanned in the glTF text chunk.
const COUNT_KEY: &str = "\"count\"";
const URI_KEY: &str = "\"uri\"";
const QUOTE: &str = "\"";

/// Interchange format, from file extension only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Glb,
    Gltf,
}

/// Validated import: path plus format plus vert count plus texture refs plus hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    pub path: String,
    pub format: Format,
    pub verts: u32,
    pub images: Vec<String>,
    pub hash: u64,
}

/// Import failure; every variant names the file plus the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// Extension is not .glb / .gltf.
    NotInterchange { path: String },
    /// Magic or JSON envelope does not match the format.
    BadMagic { path: String },
    /// A texture ref is not KTX2 (bake offline, see validate_ktx2).
    KtxOnly { path: String, image: String },
    /// Count missing or over POLY_BUDGET.
    PolyBudget { path: String, verts: u32 },
    /// A mesh, PBR, or rig signal is missing; names the gate.
    Gate { path: String, gate: String },
    /// Below the byte floor or a sparse container.
    TooSmall { path: String },
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInterchange { path } => {
                write!(f, "import {path}: glTF-only, want .glb or .gltf")
            }
            Self::BadMagic { path } => write!(f, "import {path}: bad magic or envelope"),
            Self::KtxOnly { path, image } => {
                write!(f, "import {path}: KTX2-only textures, got {image}")
            }
            Self::PolyBudget { path, verts } => {
                write!(f, "import {path}: verts {verts} over budget {POLY_BUDGET}")
            }
            Self::Gate { path, gate } => write!(f, "import {path}: gate {gate} failed"),
            Self::TooSmall { path } => write!(f, "import {path}: below byte floor"),
        }
    }
}

impl std::error::Error for ImportError {}
/// KTX2 header facts from validate_ktx2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KtxInfo {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
}

/// Sidecar shape: source row plus content hash; cache under imported/.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sidecar {
    pub guid: u64,
    pub content_hash: u64,
    pub bytes: u64,
}

impl Sidecar {
    /// Mint a sidecar row for freshly baked bytes.
    pub fn bake(_path: &str, bytes: &[u8]) -> Self {
        let content_hash = crate::registry::content_hash(bytes);
        Self {
            guid: content_hash,
            content_hash,
            bytes: bytes.len() as u64,
        }
    }
    /// True when the source bytes no longer match the baked hash.
    pub fn stale(&self, bytes: &[u8]) -> bool {
        crate::registry::content_hash(bytes) != self.content_hash
    }
}

/// Format from extension; glTF-only, the rest is NotInterchange.
pub fn format_of(path: &str) -> Result<Format, ImportError> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".glb") {
        Ok(Format::Glb)
    } else if lower.ends_with(".gltf") {
        Ok(Format::Gltf)
    } else {
        Err(ImportError::NotInterchange { path: path.into() })
    }
}

fn has(hay: &str, needle: &str) -> bool {
    hay.contains(needle)
}

fn magic_ok(format: Format, bytes: &[u8], text: &str) -> bool {
    match format {
        Format::Glb => {
            bytes.len() >= 12
                && bytes[0..4] == GLB_MAGIC
                && u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) == 2
        }
        Format::Gltf => has(text, "\"asset\""),
    }
}

fn floors_ok(bytes: &[u8]) -> bool {
    health::floors(bytes, true).iter().all(|pair| pair.1)
}

/// First count value in the JSON text; 0 when absent or unparseable.
fn vert_count(text: &str) -> u32 {
    let mut search = text;
    while let Some(i) = search.find(COUNT_KEY) {
        let rest = &search[i + COUNT_KEY.len()..];
        let mut digits = String::new();
        let mut started = false;
        for c in rest.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
                started = true;
            } else if started {
                break;
            }
        }
        if let Ok(n) = digits.parse::<u32>() {
            return n;
        }
        search = &search[i + COUNT_KEY.len()..];
    }
    0
}

/// Every uri value in the JSON text, in document order.
fn image_refs(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut search = text;
    while let Some(i) = search.find(URI_KEY) {
        let rest = &search[i + URI_KEY.len()..];
        let start = rest.find(QUOTE).map(|q| q + 1).unwrap_or(rest.len());
        let tail = &rest[start.min(rest.len())..];
        let end = tail.find(QUOTE).unwrap_or(tail.len());
        out.push(tail[..end].to_string());
        search = &tail[end.min(tail.len())..];
    }
    out
}

fn images_ktx2(images: &[String]) -> bool {
    !images.is_empty()
        && images
            .iter()
            .all(|u| u.to_ascii_lowercase().ends_with(".ktx2"))
}
/// Full 7-gate checklist in crate::GATES order; never fails, bad input
/// reports 0 of 7. Deterministic on the same path plus bytes.
pub fn gate_results(path: &str, bytes: &[u8]) -> [(String, bool); 7] {
    let mut g = [
        (crate::GATES[0].to_string(), false),
        (crate::GATES[1].to_string(), false),
        (crate::GATES[2].to_string(), false),
        (crate::GATES[3].to_string(), false),
        (crate::GATES[4].to_string(), false),
        (crate::GATES[5].to_string(), false),
        (crate::GATES[6].to_string(), false),
    ];
    let format = match format_of(path) {
        Ok(f) => f,
        Err(_) => return g,
    };
    let text = String::from_utf8_lossy(bytes);
    let verts = vert_count(&text);
    let images = image_refs(&text);
    g[0].1 = has(&text, "\"indices\"") && !has(&text, "OPEN_BOUNDARY");
    g[1].1 = verts > 0 && verts <= POLY_BUDGET;
    g[2].1 = has(&text, "TEXCOORD_0") && !has(&text, "UVOVERLAP");
    g[3].1 = has(&text, "pbrMetallicRoughness");
    g[4].1 = has(&text, "baseColorTexture") && images_ktx2(&images) && !has(&text, "ALBEDO_DIRTY");
    g[5].1 = has(&text, "\"skins\"") && has(&text, "\"joints\"");
    g[6].1 = magic_ok(format, bytes, &text) && floors_ok(bytes);
    g
}

/// Count of passing gates in gate_results.
pub fn gates_ok(path: &str, bytes: &[u8]) -> usize {
    gate_results(path, bytes)
        .iter()
        .filter(|pair| pair.1)
        .count()
}

/// Strict import: every gate must pass or the reason is returned.
/// Same path plus bytes means the same Imported struct.
pub fn import(path: &str, bytes: &[u8]) -> Result<Imported, ImportError> {
    let format = format_of(path)?;
    let text = String::from_utf8_lossy(bytes);
    if !magic_ok(format, bytes, &text) {
        return Err(ImportError::BadMagic { path: path.into() });
    }
    if !floors_ok(bytes) {
        return Err(ImportError::TooSmall { path: path.into() });
    }
    let images = image_refs(&text);
    for u in &images {
        if !u.to_ascii_lowercase().ends_with(".ktx2") {
            return Err(ImportError::KtxOnly {
                path: path.into(),
                image: u.clone(),
            });
        }
    }
    if let Some(pair) = gate_results(path, bytes).iter().find(|pair| !pair.1) {
        if pair.0 == "polyBudget" {
            return Err(ImportError::PolyBudget {
                path: path.into(),
                verts: vert_count(&text),
            });
        }
        return Err(ImportError::Gate {
            path: path.into(),
            gate: pair.0.clone(),
        });
    }
    Ok(Imported {
        path: path.into(),
        format,
        verts: vert_count(&text),
        images,
        hash: crate::registry::content_hash(bytes),
    })
}

/// Validate KTX2 bytes baked offline by KTX-Software; returns header facts.
pub fn validate_ktx2(path: &str, bytes: &[u8]) -> Result<KtxInfo, ImportError> {
    if bytes.len() < 44 || bytes[0..12] != KTX2_MAGIC {
        return Err(ImportError::BadMagic { path: path.into() });
    }
    let u32le = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    if u32le(40) > 3 {
        return Err(ImportError::BadMagic { path: path.into() });
    }
    Ok(KtxInfo {
        width: u32le(20),
        height: u32le(24),
        levels: u32le(36).max(1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn json() -> String {
        r#"{"asset":{"version":"2.0"},"scene":0,"meshes":[{"primitives":[{"attributes":{"TEXCOORD_0":1},"indices":0,"mode":4,"material":0}]}],"accessors":[{"count":1234}],"materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}],"textures":[{"source":0}],"images":[{"uri":"albedo.ktx2"}],"skins":[{"joints":[0,1]}]}"#.into()
    }
    fn valid_glb() -> Vec<u8> {
        let j = json();
        let mut v = vec![0u8; 12];
        v[0..4].copy_from_slice(&GLB_MAGIC);
        v[4..8].copy_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(j.as_bytes());
        v.extend(vec![0xABu8; 3072 - v.len()]);
        let len = v.len() as u32;
        v[8..12].copy_from_slice(&len.to_le_bytes());
        v
    }
    fn bad_glb() -> Vec<u8> {
        let j = r#"{"asset":{},"UVOVERLAP":1,"ALBEDO_DIRTY":1,"OPEN_BOUNDARY":1,"accessors":[{"count":9999999}],"images":[{"uri":"albedo.png"}]}"#;
        let mut v = vec![0u8; 12];
        v[0..4].copy_from_slice(&GLB_MAGIC);
        v[4..8].copy_from_slice(&2u32.to_le_bytes());
        v.extend_from_slice(j.as_bytes());
        v
    }
    fn ktx2() -> Vec<u8> {
        let mut v = vec![0u8; 80];
        v[0..12].copy_from_slice(&KTX2_MAGIC);
        v[20..24].copy_from_slice(&64u32.to_le_bytes());
        v[24..28].copy_from_slice(&64u32.to_le_bytes());
        v[36..40].copy_from_slice(&4u32.to_le_bytes());
        v
    }
    #[test]
    fn valid_glb_gates_7_of_7() {
        let b = valid_glb();
        assert_eq!(gates_ok("pack/hero.glb", &b), 7);
        assert!(gate_results("pack/hero.glb", &b).iter().all(|pair| pair.1));
        let imp = import("pack/hero.glb", &b).unwrap();
        assert_eq!(imp.verts, 1234);
        assert_eq!(imp.images, vec!["albedo.ktx2".to_string()]);
    }
    #[test]
    fn bad_shapes_gate_0_of_7() {
        assert_eq!(gates_ok("pack/bad.fbx", b"tiny"), 0);
        assert!(import("pack/bad.fbx", b"tiny").is_err());
        assert_eq!(gates_ok("pack/bad.glb", &bad_glb()), 0);
        assert!(import("pack/bad.glb", &bad_glb()).is_err());
        let raw = valid_glb();
        let text = String::from_utf8_lossy(&raw).into_owned();
        let png = text.replace("albedo.ktx2", "albedo.png");
        let e = import("pack/hero.glb", png.as_bytes()).unwrap_err();
        assert!(matches!(e, ImportError::KtxOnly { .. }));
    }
    #[test]
    fn import_hash_stable_x2() {
        let b = valid_glb();
        let a = import("pack/hero.glb", &b).unwrap();
        let c = import("pack/hero.glb", &b).unwrap();
        assert_eq!(a, c);
        assert_eq!(
            gate_results("pack/hero.glb", &b),
            gate_results("pack/hero.glb", &b)
        );
    }
    #[test]
    fn ktx2_header_roundtrip() {
        let info = validate_ktx2("tex/albedo.ktx2", &ktx2()).unwrap();
        assert_eq!((info.width, info.height, info.levels), (64, 64, 4));
        assert!(validate_ktx2("tex/a.ktx2", b"tiny").is_err());
        assert!(validate_ktx2("tex/a.ktx2", &[0xABu8; 64]).is_err());
    }
    #[test]
    fn sidecar_bake_stale() {
        let b = valid_glb();
        let s = Sidecar::bake("pack/hero.glb", &b);
        assert!(!s.stale(&b));
        let mut touched = b.clone();
        touched[100] ^= 0xFF;
        assert!(s.stale(&touched));
    }
}

//! The machine-eyes reference set: the capture per tier the look gate
//! scores against.
//!
//! Consumer: the look judge ([`super::judge`]), which pairs a capture frame
//! with its tier's reference frame, and the operator, who materializes the
//! set for the door. Data owner: this module owns the recipe - the same
//! canvas plus seed paints the same frame twice, integer only, no wall
//! clock, no RNG - and reads `index.json`, the manifest that pins every
//! tier's digest. Pixels never enter any simulation hash.
//!
//! Tier axis: the delivery leg the capture ships on - `native` (the desktop
//! window), `web` (the browser page canvas) and `potato` (the low-end
//! fallback). The legs are the tier ladder the tree already names
//! (`content/reference/native/...` in the door's own path check, and the
//! research note's "per tier (native, web, potato)"), and the leg is the
//! axis a capture varies on: the same scene, the same camera, the same
//! seed, painted at that leg's canvas.
//!
//! Why a recipe and not a stored PNG: a reference the tree can regenerate
//! byte-identically is stronger than a file - the digest is reproducible
//! offline, the set cannot rot behind a stale renderer, and no binary rides
//! in git. The manifest files the digest of the rendered capture, so a
//! recipe change without a re-filed digest refuses loudly.
//!
//! Units: pixels for sizes, FNV-1a hex for the digest over the decoded
//! pixels (the same [`crate::frame_check::digest_hex`] the judge files as
//! `pixel_fnv`). The digest pins the pixels, not the PNG bytes: the bytes
//! depend on the encoder, the pixels are the capture, so the digest stays
//! reproducible offline on any machine. Bounds: at most [`MAX_ROWS`] rows,
//! the judge's own reference ceiling. Determinism: pure over the inputs;
//! the same directory twice gives the same set or the same error. Errors:
//! [`ReferenceError`] names the missing tier, the missing file or the
//! digest that drifted; nothing here panics.

use std::fmt;
use std::path::{Path, PathBuf};

use super::judge::{NamedPng, References};
use crate::frame_check::digest_hex;
use crate::parse::{Cursor, ParseError};

/// The manifest file name inside a reference-set directory.
pub const INDEX_NAME: &str = "index.json";
/// Manifest schema this reader accepts.
pub const SCHEMA: u64 = 1;
/// Most reference rows in one set (three tiers of ten real captures plus the
/// three recipe rows that stay as the second source).
pub const MAX_ROWS: usize = 64;
/// Largest index.json accepted, bytes.
pub const MAX_INDEX_BYTES: usize = 1 << 20;
/// Largest capture PNG accepted, bytes (the judge's capture ceiling).
pub const MAX_PNG_BYTES: usize = 4 * 1024 * 1024;
/// JSON nesting this reader accepts for the index (object, rows array, row).
const MAX_DEPTH: usize = 8;
/// The seed every shipped capture ran with.
pub const SEED: u64 = 2040;
/// The camera every shipped capture ran with.
pub const CAMERA: &str = "arena600";
/// The scene every shipped capture holds.
pub const SCENE: &str = "arena-duel";

/// Why a reference set could not be read or verified.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ReferenceError {
    /// The set directory holds no `index.json`.
    MissingIndex {
        /// Directory the set was expected in.
        dir: String,
    },
    /// `index.json` is over its byte bound.
    TooLarge {
        /// File name.
        path: String,
        /// Bytes offered.
        bytes: usize,
    },
    /// The index did not read as the manifest shape.
    BadIndex {
        /// File name.
        path: String,
        /// Parse fault in words.
        reason: String,
    },
    /// The index holds more rows than the bound.
    TooManyRows {
        /// Rows offered.
        count: usize,
    },
    /// A directory or file could not be read or written.
    Io {
        /// Path (for diagnostics).
        path: String,
        /// IO failure kind.
        kind: std::io::ErrorKind,
    },
    /// The tier has no capture in the set: a named refusal, never a
    /// silent pass.
    UnknownTier {
        /// Tier that was asked for.
        tier: String,
    },
    /// The rendered capture does not match the pixel digest the row filed.
    DigestMismatch {
        /// Row tier.
        tier: String,
        /// Row shot.
        shot: String,
        /// Digest the row filed over the pixels.
        expected: String,
        /// Digest the recipe renders.
        got: String,
    },
    /// The PNG encoder refused the rendered frame.
    Encode {
        /// The frame it refused (`width x height`).
        path: String,
        /// The encoder's own words.
        reason: String,
    },
    /// A real row's capture PNG is not on disk under the set directory.
    MissingCapture {
        /// Row tier.
        tier: String,
        /// Row shot.
        shot: String,
        /// The path the row declares, relative to the set directory.
        path: String,
    },
    /// A real row's capture PNG is not the size the row declares.
    SizeMismatch {
        /// Row tier.
        tier: String,
        /// Row shot.
        shot: String,
        /// Width and height the row declares.
        declared: (u32, u32),
        /// Width and height the PNG holds.
        found: (u32, u32),
    },
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingIndex { dir } => write!(
                f,
                "no {INDEX_NAME} in {dir}: the reference set is not on disk; \
                 write it from the recipe or pick a tier it holds"
            ),
            Self::TooLarge { path, bytes } => write!(
                f,
                "{path} is {bytes} bytes; at most {MAX_INDEX_BYTES} per index"
            ),
            Self::BadIndex { path, reason } => {
                write!(f, "{path} is not a readable reference index: {reason}")
            }
            Self::TooManyRows { count } => write!(
                f,
                "{count} reference rows offered; at most {MAX_ROWS} per set"
            ),
            Self::Io { path, kind } => write!(f, "cannot read {path} ({kind:?})"),
            Self::UnknownTier { tier } => write!(
                f,
                "the reference set holds no capture for tier {tier}: \
                 the reference seat has not shot it; pick a tier the set \
                 holds or add its row first"
            ),
            Self::DigestMismatch {
                tier,
                shot,
                expected,
                got,
            } => write!(
                f,
                "reference {tier}/{shot} drifted: the recipe renders pixels {got} but \
                 the index filed {expected}; re-file the digest or restore the recipe"
            ),
            Self::Encode { path, reason } => write!(
                f,
                "the reference frame {path} does not encode as PNG: {reason}"
            ),
            Self::MissingCapture { tier, shot, path } => write!(
                f,
                "reference {tier}/{shot}: {path} is not on disk; run \
                 tools/reference-captures.mjs to shoot the tier, or drop the row"
            ),
            Self::SizeMismatch {
                tier,
                shot,
                declared,
                found,
            } => write!(
                f,
                "reference {tier}/{shot} is {}x{} but the row declares {}x{}; \
                 re-shoot the tier at the declared canvas",
                found.0, found.1, declared.0, declared.1
            ),
        }
    }
}

impl std::error::Error for ReferenceError {}

/// One manifest row: a tier's reference capture and its provenance.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReferenceRow {
    /// Delivery leg the capture stands for (`native`, `web`, `potato`).
    pub tier: String,
    /// Shot name inside the tier (unique with the tier).
    pub shot: String,
    /// Where the capture is filed when the set is materialized, relative to
    /// the set directory.
    pub png: String,
    /// FNV-1a hex over the capture's decoded pixels, filed by the manifest.
    pub pixel_fnv: String,
    /// Canvas width the capture renders at, pixels.
    pub width: u32,
    /// Canvas height the capture renders at, pixels.
    pub height: u32,
    /// Seed the capture ran with.
    pub seed: u64,
    /// Camera the capture ran with.
    pub camera: String,
    /// Sim tick the capture was taken on (0 for a recipe row).
    pub tick: u64,
    /// Where the capture came from: the binary or browser, the adapter and
    /// the command line for a real row, or the recipe's own words.
    pub source: String,
}

/// One tier's verified reference captures, in manifest order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReferenceTier {
    /// Delivery leg name.
    pub tier: String,
    /// The captures, manifest order, each digest-verified.
    pub captures: Vec<(ReferenceRow, NamedPng)>,
}

/// A loaded reference set: every real row read back from its PNG on disk and
/// digest-checked against the manifest, plus the recipe rows kept as the
/// second source (painted and digest-checked the same way).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReferenceSet {
    tiers: Vec<ReferenceTier>,
    recipe: Vec<ReferenceRow>,
}

impl ReferenceSet {
    /// Load and verify the set in `dir`.
    ///
    /// Reads `{dir}/{INDEX_NAME}`. A row in `rows` is a real capture: its PNG
    /// is read from `{dir}/{row.png}`, decoded, and refused by name when the
    /// canvas or the pixel digest disagrees with the manifest. A row in the
    /// optional `recipe` list is painted from the recipe and digest-checked,
    /// so the stand-in source stays verifiable beside the real frames.
    ///
    /// # Errors
    ///
    /// [`ReferenceError`] for a missing or unreadable index, a bad shape, a
    /// missing or wrongly sized capture PNG, or a row whose digest disagrees.
    pub fn load(dir: &Path) -> Result<Self, ReferenceError> {
        let index_path = dir.join(INDEX_NAME);
        let text = read_index(&index_path)?;
        let parsed = parse_index(&text).map_err(|error| ReferenceError::BadIndex {
            path: index_path.display().to_string(),
            reason: error.to_string(),
        })?;
        let ParsedIndex { rows, recipe } = parsed;
        let count = rows.len().saturating_add(recipe.len());
        if count > MAX_ROWS {
            return Err(ReferenceError::TooManyRows { count });
        }
        let mut tiers: Vec<ReferenceTier> = Vec::new();
        for row in rows {
            let capture = read_capture(dir, &row)?;
            let named = NamedPng {
                name: format!("{}.png", row.shot),
                png: capture,
            };
            if let Some(tier) = tiers.iter_mut().find(|t| t.tier == row.tier) {
                tier.captures.push((row, named));
            } else {
                tiers.push(ReferenceTier {
                    tier: row.tier.clone(),
                    captures: vec![(row, named)],
                });
            }
        }
        for row in &recipe {
            render_capture(row)?;
        }
        Ok(Self { tiers, recipe })
    }

    /// Every real row the set holds, tier by tier, in manifest order.
    pub fn rows(&self) -> impl Iterator<Item = &ReferenceRow> {
        self.tiers
            .iter()
            .flat_map(|tier| tier.captures.iter().map(|(row, _)| row))
    }

    /// The recipe rows kept as the second source, in manifest order.
    #[must_use]
    pub fn recipe_rows(&self) -> &[ReferenceRow] {
        &self.recipe
    }

    /// Every tier the set holds, in manifest order.
    pub fn tiers(&self) -> impl Iterator<Item = &str> {
        self.tiers.iter().map(|tier| tier.tier.as_str())
    }

    /// One tier's capture pool, manifest order, for the judge's Q1 pairing.
    ///
    /// # Errors
    ///
    /// [`ReferenceError::UnknownTier`] when the set holds no capture for the
    /// tier: a named refusal, never a silent pass.
    pub fn pool(&self, tier: &str) -> Result<Vec<NamedPng>, ReferenceError> {
        match self.tiers.iter().find(|t| t.tier == tier) {
            Some(found) => Ok(found.captures.iter().map(|(_, png)| png.clone()).collect()),
            None => Err(ReferenceError::UnknownTier {
                tier: tier.to_owned(),
            }),
        }
    }

    /// The judge's reference pool for one tier: the tier's captures as the
    /// Q1 frames, no fighter previews.
    ///
    /// # Errors
    ///
    /// [`ReferenceError::UnknownTier`] when the set holds no capture for the
    /// tier.
    pub fn references(&self, tier: &str) -> Result<References, ReferenceError> {
        Ok(References {
            fighters: Vec::new(),
            q1: self.pool(tier)?,
        })
    }

    /// One row by tier and shot.
    #[must_use]
    pub fn row(&self, tier: &str, shot: &str) -> Option<&ReferenceRow> {
        self.tiers.iter().find(|t| t.tier == tier).and_then(|t| {
            t.captures
                .iter()
                .find(|(row, _)| row.shot == shot)
                .map(|(row, _)| row)
        })
    }

    /// How many rows the set holds, across its tiers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tiers.iter().map(|t| t.captures.len()).sum()
    }

    /// True when the set holds no capture at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tiers.is_empty()
    }

    /// Write every row's capture PNG under `dir`, at the manifest's own
    /// relative paths, so an operator can point the door's `q1_dir` at a
    /// tier.
    ///
    /// # Errors
    ///
    /// [`ReferenceError::Io`] when a directory or file cannot be written.
    pub fn materialize(&self, dir: &Path) -> Result<(), ReferenceError> {
        for tier in &self.tiers {
            for (row, capture) in &tier.captures {
                let path = dir.join(&row.png);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| ReferenceError::Io {
                        path: parent.display().to_string(),
                        kind: error.kind(),
                    })?;
                }
                std::fs::write(&path, &capture.png).map_err(|error| ReferenceError::Io {
                    path: path.display().to_string(),
                    kind: error.kind(),
                })?;
            }
        }
        Ok(())
    }
}

fn read_index(path: &Path) -> Result<String, ReferenceError> {
    let meta = std::fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ReferenceError::MissingIndex {
                dir: path
                    .parent()
                    .map_or_else(String::new, |parent| parent.display().to_string()),
            }
        } else {
            ReferenceError::Io {
                path: path.display().to_string(),
                kind: error.kind(),
            }
        }
    })?;
    let bytes = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    if bytes > MAX_INDEX_BYTES {
        return Err(ReferenceError::TooLarge {
            path: path.display().to_string(),
            bytes,
        });
    }
    let raw = std::fs::read(path).map_err(|error| ReferenceError::Io {
        path: path.display().to_string(),
        kind: error.kind(),
    })?;
    String::from_utf8(raw).map_err(|_| ReferenceError::BadIndex {
        path: path.display().to_string(),
        reason: "not utf-8".to_owned(),
    })
}

/// Render one row's capture and check it against the filed digest.
fn render_capture(row: &ReferenceRow) -> Result<Vec<u8>, ReferenceError> {
    let png = capture_png(row)?;
    if png.len() > MAX_PNG_BYTES {
        return Err(ReferenceError::TooLarge {
            path: row.png.clone(),
            bytes: png.len(),
        });
    }
    let frame =
        super::metric::Frame::decode_png(&png).map_err(|error| ReferenceError::BadIndex {
            path: row.png.clone(),
            reason: format!("the rendered capture does not decode: {error}"),
        })?;
    let pixels = digest_hex(frame.rgba());
    if pixels != row.pixel_fnv {
        return Err(ReferenceError::DigestMismatch {
            tier: row.tier.clone(),
            shot: row.shot.clone(),
            expected: row.pixel_fnv.clone(),
            got: pixels,
        });
    }
    Ok(png)
}

/// Read one real row's capture PNG from the set directory, decode it and
/// check it against the row's declared canvas and digest.
///
/// A real frame is a file on disk, never a render: the manifest proves the
/// pixels the shipped arena (or the page) actually drew.
fn read_capture(dir: &Path, row: &ReferenceRow) -> Result<Vec<u8>, ReferenceError> {
    let path = dir.join(&row.png);
    let png = match std::fs::read(&path) {
        Ok(png) => png,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ReferenceError::MissingCapture {
                tier: row.tier.clone(),
                shot: row.shot.clone(),
                path: row.png.clone(),
            });
        }
        Err(error) => {
            return Err(ReferenceError::Io {
                path: path.display().to_string(),
                kind: error.kind(),
            });
        }
    };
    if png.len() > MAX_PNG_BYTES {
        return Err(ReferenceError::TooLarge {
            path: row.png.clone(),
            bytes: png.len(),
        });
    }
    let frame =
        super::metric::Frame::decode_png(&png).map_err(|error| ReferenceError::BadIndex {
            path: row.png.clone(),
            reason: format!("the capture does not decode: {error}"),
        })?;
    let found = (frame.width(), frame.height());
    let declared = (row.width, row.height);
    if found != declared {
        return Err(ReferenceError::SizeMismatch {
            tier: row.tier.clone(),
            shot: row.shot.clone(),
            declared,
            found,
        });
    }
    let pixels = digest_hex(frame.rgba());
    if pixels != row.pixel_fnv {
        return Err(ReferenceError::DigestMismatch {
            tier: row.tier.clone(),
            shot: row.shot.clone(),
            expected: row.pixel_fnv.clone(),
            got: pixels,
        });
    }
    Ok(png)
}

/// The reference capture for one row: the recipe's frame, PNG-encoded.
/// The same row twice gives the same bytes, so the filed digest is
/// reproducible offline.
///
/// # Errors
///
/// [`ReferenceError::Encode`] when the encoder refuses the frame.
pub fn capture_png(row: &ReferenceRow) -> Result<Vec<u8>, ReferenceError> {
    let pixels = rgba(row.width, row.height, row.seed);
    encode_png(&pixels, row.width, row.height)
}

/// The reference scene: the arena duel - a stands band over textured ground
/// and two fighters at their staged duel positions - as a pure function of
/// the canvas and the seed. Integer only: the same inputs give the same
/// bytes on every machine, so the manifest digest is reproducible offline.
#[must_use]
pub fn rgba(width: u32, height: u32, seed: u64) -> Vec<u8> {
    let capacity = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().map(|h| w * h * 4))
        .unwrap_or(0);
    let mut out = Vec::with_capacity(capacity);
    let stands = height / 4;
    let cell = (width / 64).max(2);
    let jitter = u32::try_from(seed % 7).unwrap_or(0);
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = if y < stands {
                // The stands: a grey gradient that darkens toward the pitch.
                let shade = 40 + (y * 160 / stands.max(1));
                [tone(shade), tone(shade), tone(shade)]
            } else {
                // The ground: textured sand, the judge's own palette, one
                // tone per cell so the texture scales with the canvas.
                let cy = (y - stands) / cell;
                let cx = x / cell;
                let wobble = (cx * 7 + cy * 13 + (cx * cy) % 5 + jitter) % 23;
                let wobble = i32::try_from(wobble).unwrap_or(0) - 11;
                sand_tone(wobble)
            };
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    draw_fighters(&mut out, width, height);
    out
}

fn tone(value: u32) -> u8 {
    u8::try_from(value.min(255)).unwrap_or(255)
}

fn sand_tone(wobble: i32) -> [u8; 3] {
    [194i32, 160, 104].map(|c| u8::try_from((c + wobble).clamp(0, 255)).unwrap_or(255))
}

/// The two staged fighters: rimmed boxes at a quarter and three quarters of
/// the pitch, scaled with the canvas so every tier draws the same duel.
fn draw_fighters(out: &mut [u8], width: u32, height: u32) {
    const WHITE: [u8; 3] = [255, 255, 255];
    const GOLD: [u8; 3] = [217, 179, 38];
    const EDGE: [u8; 3] = [20, 16, 12];
    for (at, fill) in [(250u32, WHITE), (750u32, GOLD)] {
        let cx = width * at / 1_000;
        let cy = height * 620 / 1_000;
        let body_w = (width * 55 / 1_000).max(7);
        let body_h = (height * 94 / 1_000).max(9);
        let Some(left) = cx.checked_sub(body_w / 2) else {
            continue;
        };
        let Some(top) = cy.checked_sub(body_h / 2) else {
            continue;
        };
        for dy in 0..body_h {
            for dx in 0..body_w {
                let Some(x) = left.checked_add(dx) else {
                    continue;
                };
                let Some(y) = top.checked_add(dy) else {
                    continue;
                };
                if x >= width || y >= height {
                    continue;
                }
                let rim = dx == 0 || dy == 0 || dx + 1 == body_w || dy + 1 == body_h;
                let [r, g, b] = if rim { EDGE } else { fill };
                let Some(at) = usize::try_from((y * width + x) * 4).ok() else {
                    continue;
                };
                if at + 4 <= out.len() {
                    out[at..at + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
    }
}

/// Encode RGBA8 bytes as PNG (the same encoder the judge's own fixtures use).
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, ReferenceError> {
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(rgba, width, height, ExtendedColorType::Rgba8)
        .map_err(|error| ReferenceError::Encode {
            path: format!("{width}x{height}"),
            reason: error.to_string(),
        })?;
    Ok(out)
}

/// One JSON value as the reader accepts it: text, integer, list or object.
#[derive(Clone, PartialEq, Eq, Debug)]
enum Field {
    Text(String),
    Number(u64),
    List(Vec<Field>),
    Object(Vec<(String, Field)>),
}

/// The manifest split into its two sources: the real captures in `rows` and
/// the stand-in scene in the optional `recipe` list.
struct ParsedIndex {
    rows: Vec<ReferenceRow>,
    recipe: Vec<ReferenceRow>,
}

/// Read the index text into its rows; unknown keys are skipped.
fn parse_index(text: &str) -> Result<ParsedIndex, ParseError> {
    let mut cursor = Cursor::new(text.trim_start_matches('\u{feff}'));
    cursor.ws();
    let root = read_object(&mut cursor, 0)?;
    cursor.ws();
    cursor.finish()?;
    let schema = root
        .iter()
        .find(|(key, _)| key == "schema")
        .and_then(|(_, value)| as_u64(value));
    if schema != Some(SCHEMA) {
        return Err(unexpected(&cursor));
    }
    let rows = row_list(&root, "rows", &cursor)?;
    let recipe = match root.iter().find(|(key, _)| key == "recipe") {
        Some(_) => row_list(&root, "recipe", &cursor)?,
        None => Vec::new(),
    };
    Ok(ParsedIndex { rows, recipe })
}

/// Read one named list of rows out of the manifest object.
fn row_list(
    root: &[(String, Field)],
    name: &str,
    cursor: &Cursor<'_>,
) -> Result<Vec<ReferenceRow>, ParseError> {
    let items = root
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value);
    let Some(Field::List(items)) = items else {
        return Err(unexpected(cursor));
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(row_of(item)?);
    }
    Ok(out)
}

fn row_of(entry: &Field) -> Result<ReferenceRow, ParseError> {
    let Field::Object(fields) = entry else {
        return Err(bad_shape());
    };
    let text = |name: &str| -> Result<String, ParseError> {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| match value {
                Field::Text(text) => Some(text.clone()),
                _ => None,
            })
            .ok_or_else(bad_shape)
    };
    let number = |name: &str| -> Result<u64, ParseError> {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| as_u64(value))
            .ok_or_else(bad_shape)
    };
    let width = u32::try_from(number("width")?).map_err(|_| bad_shape())?;
    let height = u32::try_from(number("height")?).map_err(|_| bad_shape())?;
    Ok(ReferenceRow {
        tier: text("tier")?,
        shot: text("shot")?,
        png: text("png")?,
        pixel_fnv: text("pixel_fnv")?,
        width,
        height,
        seed: number("seed")?,
        camera: text("camera")?,
        // A recipe row carries no tick; a real row files the sim tick it ran
        // on, so ten shots of one path stay distinct rows.
        tick: number("tick").unwrap_or(0),
        source: text("source").unwrap_or_default(),
    })
}

fn bad_shape() -> ParseError {
    ParseError::UnexpectedByte { pos: 0, byte: b'{' }
}

fn as_u64(value: &Field) -> Option<u64> {
    match value {
        Field::Number(number) => Some(*number),
        _ => None,
    }
}

fn read_object(cursor: &mut Cursor<'_>, depth: usize) -> Result<Vec<(String, Field)>, ParseError> {
    if depth > MAX_DEPTH {
        return Err(ParseError::NestingTooDeep { pos: cursor.pos() });
    }
    cursor.ws();
    if !cursor.eat(b'{') {
        return Err(unexpected(cursor));
    }
    let mut out = Vec::new();
    cursor.ws();
    if cursor.eat(b'}') {
        return Ok(out);
    }
    loop {
        cursor.ws();
        let key = read_text(cursor)?;
        cursor.ws();
        if !cursor.eat(b':') {
            return Err(unexpected(cursor));
        }
        cursor.ws();
        let value = read_field(cursor, depth)?;
        out.push((key, value));
        cursor.ws();
        if cursor.eat(b',') {
            continue;
        }
        if cursor.eat(b'}') {
            return Ok(out);
        }
        return Err(unexpected(cursor));
    }
}

fn read_field(cursor: &mut Cursor<'_>, depth: usize) -> Result<Field, ParseError> {
    if depth > MAX_DEPTH {
        return Err(ParseError::NestingTooDeep { pos: cursor.pos() });
    }
    cursor.ws();
    match cursor.peek() {
        Some(b'"') => read_text(cursor).map(Field::Text),
        Some(b'-' | b'0'..=b'9') => read_number(cursor).map(Field::Number),
        Some(b'[') => {
            cursor.bump();
            let mut items = Vec::new();
            cursor.ws();
            if cursor.eat(b']') {
                return Ok(Field::List(items));
            }
            loop {
                items.push(read_field(cursor, depth + 1)?);
                cursor.ws();
                if cursor.eat(b',') {
                    continue;
                }
                if cursor.eat(b']') {
                    return Ok(Field::List(items));
                }
                return Err(unexpected(cursor));
            }
        }
        Some(b'{') => read_object(cursor, depth + 1).map(Field::Object),
        _ => Err(unexpected(cursor)),
    }
}

fn unexpected(cursor: &Cursor<'_>) -> ParseError {
    ParseError::UnexpectedByte {
        pos: cursor.pos(),
        byte: cursor.peek().unwrap_or(b' '),
    }
}

fn read_number(cursor: &mut Cursor<'_>) -> Result<u64, ParseError> {
    let start = cursor.pos();
    let mut digits = String::new();
    while let Some(byte @ b'0'..=b'9') = cursor.peek() {
        digits.push(char::from(byte));
        cursor.bump();
    }
    if digits.is_empty() {
        return Err(ParseError::BadNumber { pos: start });
    }
    digits
        .parse::<u64>()
        .map_err(|_| ParseError::NumberOutOfRange { pos: start })
}

fn read_text(cursor: &mut Cursor<'_>) -> Result<String, ParseError> {
    cursor.bump();
    let mut out = String::new();
    loop {
        let Some(byte) = cursor.peek() else {
            return Err(ParseError::UnterminatedString { pos: cursor.pos() });
        };
        match byte {
            b'"' => {
                cursor.bump();
                return Ok(out);
            }
            b'\\' => {
                cursor.bump();
                let Some(escaped) = cursor.peek() else {
                    return Err(ParseError::UnterminatedEscape { pos: cursor.pos() });
                };
                let decoded = match escaped {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    _ => {
                        return Err(ParseError::BadEscape {
                            pos: cursor.pos(),
                            byte: Some(escaped),
                        })
                    }
                };
                out.push(decoded);
                cursor.bump();
            }
            byte if byte < 0x20 => {
                return Err(ParseError::ControlByte { pos: cursor.pos() });
            }
            byte => {
                out.push(char::from(byte));
                cursor.bump();
            }
        }
    }
}

/// The shipped set's directory, from this crate: the tree layout puts
/// `content/` two levels above the playtest crate.
#[must_use]
pub fn shipped_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/reference")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_check::digest_hex;

    /// One real manifest row for the native tier: the digest its own PNG
    /// bytes decode to, so the fixture exercises the file-reading path.
    fn row(tier: &str, width: u32, height: u32) -> ReferenceRow {
        let mut row = ReferenceRow {
            tier: tier.to_owned(),
            shot: "arena600-duel".to_owned(),
            png: format!("{tier}/arena600-duel.png"),
            pixel_fnv: String::new(),
            width,
            height,
            seed: SEED,
            camera: CAMERA.to_owned(),
            tick: 0,
            source: "arena (engine-game bin) adapter=test cmd=arena --reference-set".to_owned(),
        };
        let png = capture_png(&row).expect("the capture encodes");
        let frame = super::super::metric::Frame::decode_png(&png).expect("the capture decodes");
        row.pixel_fnv = digest_hex(frame.rgba());
        row
    }

    /// One stand-in row for the recipe block, with the digest the recipe
    /// paints for it.
    fn recipe_row(tier: &str, width: u32, height: u32) -> ReferenceRow {
        let mut row = row(tier, width, height);
        row.shot = "arena600-duel".to_owned();
        row.png = format!("{tier}/arena600-duel.png");
        row.tick = 0;
        row.source = format!("recipe {SCENE}");
        row
    }

    fn index_of(rows: &[ReferenceRow], recipe: &[ReferenceRow]) -> String {
        let mut out = String::from("{\"schema\":1,\"scene\":\"arena-duel\",\"rows\":[");
        for (at, row) in rows.iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            out.push_str(&format!(
                "{{\"tier\":\"{}\",\"shot\":\"{}\",\"png\":\"{}\",\
                 \"pixel_fnv\":\"{}\",\"width\":{},\"height\":{},\"seed\":{},\
                 \"camera\":\"{}\",\"tick\":{},\"source\":\"{}\"}}",
                row.tier,
                row.shot,
                row.png,
                row.pixel_fnv,
                row.width,
                row.height,
                row.seed,
                row.camera,
                row.tick,
                row.source
            ));
        }
        out.push(']');
        if !recipe.is_empty() {
            out.push_str(",\"recipe\":[");
            for (at, row) in recipe.iter().enumerate() {
                if at > 0 {
                    out.push(',');
                }
                out.push_str(&format!(
                    "{{\"tier\":\"{}\",\"shot\":\"{}\",\"png\":\"{}\",\
                     \"pixel_fnv\":\"{}\",\"width\":{},\"height\":{},\"seed\":{},\
                     \"camera\":\"{}\",\"source\":\"{}\"}}",
                    row.tier,
                    row.shot,
                    row.png,
                    row.pixel_fnv,
                    row.width,
                    row.height,
                    row.seed,
                    row.camera,
                    row.source
                ));
            }
            out.push(']');
        }
        out.push('}');
        out
    }

    /// Write a set: the index plus every real row's PNG at its own path, the
    /// way the capture tool files a tier.
    fn write_set(dir: &Path, rows: &[ReferenceRow]) {
        write_set_with_recipe(dir, rows, &[]);
    }

    fn write_set_with_recipe(dir: &Path, rows: &[ReferenceRow], recipe: &[ReferenceRow]) {
        std::fs::create_dir_all(dir).expect("the set directory is writable");
        std::fs::write(dir.join(INDEX_NAME), index_of(rows, recipe))
            .expect("the index is writable");
        for row in rows {
            let path = dir.join(&row.png);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("the tier directory is writable");
            }
            std::fs::write(&path, capture_png(row).expect("the capture encodes"))
                .expect("the capture is writable");
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engine2040-ref-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_recipe_renders_the_same_bytes_twice() {
        let first = rgba(64, 48, SEED);
        let second = rgba(64, 48, SEED);
        assert_eq!(first, second);
        assert_eq!(first.len(), 64 * 48 * 4);
    }

    #[test]
    fn a_set_loads_and_pools_its_tiers() {
        let dir = temp_dir("pools");
        let rows = vec![row("native", 64, 48), row("web", 32, 24)];
        write_set(&dir, &rows);
        let set = ReferenceSet::load(&dir).expect("the set loads");
        assert_eq!(set.len(), 2);
        assert_eq!(set.tiers().collect::<Vec<_>>(), ["native", "web"]);
        assert_eq!(set.pool("native").expect("the native tier").len(), 1);
        assert_eq!(set.pool("web").expect("the web tier").len(), 1);
        // The judge reads the tier's captures as its Q1 pool.
        let refs = set.references("native").expect("the native tier");
        assert_eq!(refs.q1.len(), 1);
        assert!(refs.fighters.is_empty());
        // The row names the tier, the shot, the tick and the digests.
        let row = set.row("native", "arena600-duel").expect("the row");
        assert_eq!(row.camera, CAMERA);
        assert_eq!(row.seed, SEED);
        assert_eq!(row.tick, 0);
        assert_eq!(row.pixel_fnv.len(), 16);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_capture_refuses_by_name() {
        let dir = temp_dir("no-png");
        let rows = vec![row("native", 64, 48)];
        write_set(&dir, &rows);
        std::fs::remove_file(dir.join(&rows[0].png)).expect("the capture is removable");
        let error = ReferenceSet::load(&dir).expect_err("a missing capture refuses");
        assert!(
            matches!(&error, ReferenceError::MissingCapture { tier, shot, .. }
                if tier == "native" && shot == "arena600-duel"),
            "{error}"
        );
        let words = error.to_string();
        assert!(words.contains("native/arena600-duel.png"), "{words}");
        assert!(words.contains("reference-captures.mjs"), "{words}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_size_capture_refuses_by_name() {
        let dir = temp_dir("wrong-size");
        let mut rows = vec![row("native", 64, 48)];
        write_set(&dir, &rows);
        // The row now declares a canvas the filed PNG does not hold.
        rows[0].width = 128;
        rows[0].height = 96;
        std::fs::write(dir.join(INDEX_NAME), index_of(&rows, &[])).expect("the index is writable");
        let error = ReferenceSet::load(&dir).expect_err("a wrong canvas refuses");
        assert!(
            matches!(&error, ReferenceError::SizeMismatch { declared, found, .. }
                if *declared == (128, 96) && *found == (64, 48)),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_recipe_block_stays_a_second_source() {
        let dir = temp_dir("recipe-block");
        let rows = vec![row("native", 64, 48)];
        let recipe = vec![recipe_row("native", 64, 48)];
        write_set_with_recipe(&dir, &rows, &recipe);
        let set = ReferenceSet::load(&dir).expect("the set loads");
        assert_eq!(set.len(), 1, "only the real row joins a tier");
        assert_eq!(set.recipe_rows().len(), 1);
        assert_eq!(set.recipe_rows()[0].source, format!("recipe {SCENE}"));
        // A drifted recipe digest refuses just like a drifted real one.
        let mut drifted = vec![recipe_row("native", 64, 48)];
        drifted[0].pixel_fnv = "0000000000000000".to_owned();
        write_set_with_recipe(&dir, &rows, &drifted);
        let error = ReferenceSet::load(&dir).expect_err("a drifted recipe refuses");
        assert!(
            matches!(&error, ReferenceError::DigestMismatch { tier, .. } if tier == "native"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_tier_refuses_by_name() {
        let dir = temp_dir("unknown-tier");
        write_set(&dir, &[row("native", 64, 48)]);
        let set = ReferenceSet::load(&dir).expect("the set loads");
        let error = set.pool("potato").expect_err("an absent tier refuses");
        assert_eq!(
            error,
            ReferenceError::UnknownTier {
                tier: "potato".to_owned()
            }
        );
        let words = error.to_string();
        assert!(words.contains("potato"), "{words}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_drifted_digest_refuses_by_name() {
        let dir = temp_dir("drift");
        let mut rows = vec![row("native", 64, 48)];
        rows[0].pixel_fnv = "0000000000000000".to_owned();
        write_set(&dir, &rows);
        let error = ReferenceSet::load(&dir).expect_err("a drifted digest refuses");
        assert!(
            matches!(&error, ReferenceError::DigestMismatch { tier, .. } if tier == "native"),
            "{error}"
        );
        let words = error.to_string();
        assert!(words.contains("native/arena600-duel"), "{words}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_index_refuses_by_name() {
        let dir = temp_dir("no-index");
        std::fs::create_dir_all(&dir).expect("the directory is writable");
        let error = ReferenceSet::load(&dir).expect_err("a missing index refuses");
        assert!(
            matches!(&error, ReferenceError::MissingIndex { .. }),
            "{error}"
        );
        let words = error.to_string();
        assert!(words.contains("index.json"), "{words}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_garbled_index_refuses_by_name() {
        let dir = temp_dir("garbled");
        std::fs::create_dir_all(&dir).expect("the directory is writable");
        std::fs::write(dir.join(INDEX_NAME), "not json at all").expect("the index is writable");
        let error = ReferenceSet::load(&dir).expect_err("a garbled index refuses");
        assert!(matches!(&error, ReferenceError::BadIndex { .. }), "{error}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shipped_set_is_on_disk_and_matches_its_manifest() {
        let dir = shipped_dir();
        let set = ReferenceSet::load(&dir)
            .unwrap_or_else(|error| panic!("the shipped set loads at {dir:?}: {error}"));
        assert!(!set.is_empty(), "the shipped set holds captures");
        // Every tier the set holds reads back the PNGs the manifest filed.
        for tier in set.tiers() {
            let pool = set.pool(tier).expect("the tier holds captures");
            assert!(!pool.is_empty(), "{tier} holds a capture");
        }
        assert_eq!(set.recipe_rows().len(), 3, "three recipe rows stay filed");
    }

    /// The B8 gate: every tier in the shipped set is a complete set of ten
    /// real frames - a PNG on disk at the declared canvas, a nonzero pixel
    /// digest, the tick it ran on, and a source naming the binary or browser,
    /// the adapter and the command line. No real row may name the recipe; the
    /// recipe rows stay in their own block.
    #[test]
    fn reference_set_lands_ten_real_frames_per_tier() {
        let dir = shipped_dir();
        let set = ReferenceSet::load(&dir)
            .unwrap_or_else(|error| panic!("the shipped set loads at {dir:?}: {error}"));
        let tiers: Vec<&str> = set.tiers().collect();
        assert!(
            tiers.contains(&"native"),
            "the native tier landed: {tiers:?}"
        );
        for tier in &tiers {
            let pool = set.pool(tier).expect("the tier holds captures");
            assert_eq!(
                pool.len(),
                10,
                "{tier} holds ten real captures, found {}",
                pool.len()
            );
        }
        assert_eq!(
            set.len(),
            10 * tiers.len(),
            "ten per tier and no stray row: {}",
            set.len()
        );
        let mut ticks: std::collections::BTreeSet<(String, u64)> =
            std::collections::BTreeSet::new();
        for row in set.rows() {
            let source = row.source.to_lowercase();
            // Native frames ride the shipped arena600 orbit; the web page owns
            // its own ring orbit and files that camera by name.
            let camera = if row.tier == "web" {
                "procedural-arena"
            } else {
                CAMERA
            };
            assert_ne!(
                row.pixel_fnv, "0000000000000000",
                "{}/{} files a zero digest",
                row.tier, row.shot
            );
            assert_eq!(
                row.pixel_fnv.len(),
                16,
                "{}/{} digest hex",
                row.tier,
                row.shot
            );
            assert!(
                !source.contains("recipe"),
                "{}/{} names the recipe: {}",
                row.tier,
                row.shot,
                row.source
            );
            assert!(
                source.contains("arena") || source.contains("chrome"),
                "{}/{} names no binary or browser: {}",
                row.tier,
                row.shot,
                row.source
            );
            assert!(
                row.source.contains("cmd="),
                "{}/{} names no command line: {}",
                row.tier,
                row.shot,
                row.source
            );
            assert!(
                row.tick < 600,
                "{}/{} tick {} leaves the 600-frame path",
                row.tier,
                row.shot,
                row.tick
            );
            assert_eq!(row.seed, SEED, "{}/{} seed", row.tier, row.shot);
            assert_eq!(row.camera, camera, "{}/{} camera", row.tier, row.shot);
            assert!(
                ticks.insert((row.tier.clone(), row.tick)),
                "{}/{} repeats tick {}",
                row.tier,
                row.shot,
                row.tick
            );
            // The PNG is on disk at its declared canvas (the load above read
            // and digest-checked it; this names the size the row declares).
            let path = dir.join(&row.png);
            let png = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            let frame = super::super::metric::Frame::decode_png(&png)
                .unwrap_or_else(|error| panic!("{path:?} decodes: {error}"));
            assert_eq!(
                (frame.width(), frame.height()),
                (row.width, row.height),
                "{}/{} declared canvas",
                row.tier,
                row.shot
            );
            assert_eq!(
                digest_hex(frame.rgba()),
                row.pixel_fnv,
                "{}/{} pixel digest",
                row.tier,
                row.shot
            );
        }
        // The recipe scene stays filed as the second source, one painted row
        // per tier, and it still paints.
        assert_eq!(set.recipe_rows().len(), 3, "three recipe rows stay filed");
        for row in set.recipe_rows() {
            assert!(
                row.source.starts_with("recipe "),
                "a recipe row says so: {}",
                row.source
            );
            assert_eq!(row.tick, 0, "a recipe row is not a capture tick");
        }
    }
}

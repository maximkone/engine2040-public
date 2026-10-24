//! `asset.search`: ranked lookup over the committed kit report.
//!
//! The index is `content/maps/report.json`, the same kit report `asset.budget`
//! reads: one row per map asset (`gltf`, digest and LOD0 triangles), one per
//! texture (`png`) and one per bundle entry (`mesh`, `material`, ...). Three
//! query modes: `text` matches name and tag tokens (an exact tag beats a word
//! prefix, which beats a substring), `kind` keeps one kind, and `palette` keeps
//! rows whose mean material base colour is within [`PALETTE_MAX_DELTA`] of the
//! asked colour. Rows come back scored, sorted by score then name, at most
//! [`MAX_SEARCH_LIMIT`]; a query with no hit returns an empty list, not an error.
//!
//! Palette colour: the mean of the cooked GLB's material base-colour factors,
//! rounded to 8-bit sRGB. Distance is CIE76 in L*a*b* (sRGB, D65), rounded to an
//! integer: no CIEDE2000 owner exists in this tree, so this is the documented
//! integer distance. `[u8; 6]` carries three 16-bit little-endian channels
//! (red, green, blue), each scaled to `0..=255` by `/ 257`.
//!
//! Determinism: one report and one query give one row order, no clock, no hash
//! map; every cost row is the committed report's own number.

use std::path::{Path, PathBuf};

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use crate::mcp::{McpServer, McpToolError};

use super::TOOL_ASSET_SEARCH;

/// Most rows one call returns.
pub const MAX_SEARCH_LIMIT: usize = 64;
/// Largest CIE76 distance a palette hit may have.
pub const PALETTE_MAX_DELTA: u32 = 12;
/// Kinds a query may name.
pub const SEARCH_KINDS: [&str; 8] = [
    "gltf", "png", "mesh", "material", "clip", "texture", "bundle", "map",
];

/// Args for `asset.search`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchArgs {
    /// Text to match against names and tags (mutually exclusive with `kind` and `palette`).
    #[serde(default)]
    pub text: Option<String>,
    /// Keep one kind: `gltf`, `png`, `mesh`, `material`, `clip`, `texture`, `bundle` or `map`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Mean colour to match, three 16-bit little-endian sRGB channels.
    #[serde(default)]
    pub palette: Option<[u8; 6]>,
    /// Rows to return, 1..=64 (default 16).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Report JSON to index (default `content/maps/report.json`).
    #[serde(default)]
    pub index_json: Option<String>,
}

/// One ranked hit.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchRow {
    /// Entry name as the report spells it.
    pub name: String,
    /// Entry kind, lowercase.
    pub kind: String,
    /// Lowercase hex digest (16 chars).
    pub digest: String,
    /// LOD0 triangles (0 for textures and bundle rows).
    pub triangles: u32,
    /// Bytes the entry costs (cooked GLB size, decoded RGBA texels, or 0).
    pub bytes: u64,
    /// Match score, higher first.
    pub score: u32,
    /// Mean material base colour, sRGB; set by a palette query only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_rgb: Option<[u8; 3]>,
}

/// Ranked hits plus the mode that produced them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchOutput {
    /// Ranked rows, at most `limit`.
    pub rows: Vec<SearchRow>,
    /// Rows that matched before the limit.
    pub total: u64,
    /// Mode used: `text`, `kind` or `palette`.
    pub mode: String,
    /// CIE76 distance a palette hit may have (0 for other modes).
    pub palette_max_delta: u32,
}

/// One index entry before scoring.
struct Entry {
    name: String,
    kind: String,
    digest: String,
    triangles: u32,
    bytes: u64,
    tags: Vec<String>,
    mean_rgb: Option<[u8; 3]>,
}

fn bad(message: String) -> McpToolError {
    McpToolError::new("bad_input", message)
}

fn hash_hex(bytes: &[u8]) -> String {
    format!("{:016x}", engine_core::hash_bytes(bytes))
}

fn content_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content")
}

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn row_tags(kind: &str, name: &str) -> Vec<String> {
    let mut tags = tokens(name);
    tags.push(kind.to_owned());
    tags.push("all".to_owned());
    tags.push(name.to_lowercase());
    tags.sort();
    tags.dedup();
    tags
}

/// Mean base colour of one cooked GLB's materials, sRGB.
fn mean_colour(path: &Path) -> Option<[u8; 3]> {
    let bytes = std::fs::read(path).ok()?;
    let imported = engine_assets::import_gltf_bytes(&bytes).ok()?;
    if imported.materials.is_empty() {
        return None;
    }
    let mut sum = [0.0f64; 3];
    for material in &imported.materials {
        for (axis, channel) in sum.iter_mut().enumerate() {
            *channel += f64::from(material.base_color_factor[axis].clamp(0.0, 1.0));
        }
    }
    let count = f64::from(u32::try_from(imported.materials.len()).unwrap_or(u32::MAX));
    let channel = |axis: usize| {
        let scaled = (sum[axis] / count * 255.0).round().clamp(0.0, 255.0);
        u8::try_from(engine_core::checked::saturating_f64_to_u64(scaled))
            .unwrap_or(255)
            .min(255)
    };
    Some([channel(0), channel(1), channel(2)])
}

fn number(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

/// Build the index from the kit report and the cooked GLBs beside it.
fn load_index(report: &Path) -> Result<Vec<Entry>, McpToolError> {
    let text = std::fs::read_to_string(report).map_err(|error| {
        McpToolError::new(
            "bad_input",
            format!(
                "report `{}` did not read: {error}; pass index_json with a committed kit report",
                report.display()
            ),
        )
    })?;
    let doc: Value = rmcp::serde_json::from_str(&text).map_err(|error| {
        McpToolError::new(
            "bad_input",
            format!(
                "report `{}` is not JSON: {error}; pass a kit report",
                report.display()
            ),
        )
    })?;
    let cooked = report
        .parent()
        .map(|dir| dir.join("cooked"))
        .unwrap_or_default();
    let mut entries = asset_entries(&doc, &cooked);
    entries.extend(texture_entries(&doc));
    entries.extend(bundle_entries(&doc));
    Ok(entries)
}

/// One row per `assets` entry: kind `gltf`, the report's digest and LOD0 triangles.
fn asset_entries(doc: &Value, cooked: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    for asset in doc
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = asset.get("asset").and_then(Value::as_str) else {
            continue;
        };
        let glb = cooked.join(format!("{name}.glb"));
        entries.push(Entry {
            name: name.to_owned(),
            kind: "gltf".to_owned(),
            digest: asset
                .get("digest")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| hash_hex(name.as_bytes())),
            triangles: u32::try_from(number(asset.get("lod0_tris"))).unwrap_or(u32::MAX),
            bytes: std::fs::metadata(&glb).map(|m| m.len()).unwrap_or(0),
            tags: row_tags("gltf", name),
            mean_rgb: None,
        });
    }
    entries
}

/// One row per `textures` entry: kind `png`, decoded RGBA bytes as its cost.
fn texture_entries(doc: &Value) -> Vec<Entry> {
    let mut entries = Vec::new();
    for texture in doc
        .get("textures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = texture.get("texture").and_then(Value::as_str) else {
            continue;
        };
        let size = texture
            .get("size")
            .and_then(Value::as_array)
            .map(|axes| {
                (
                    axes.first().and_then(Value::as_u64).unwrap_or(0),
                    axes.get(1).and_then(Value::as_u64).unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        entries.push(Entry {
            name: name.to_owned(),
            kind: "png".to_owned(),
            digest: hash_hex(name.as_bytes()),
            triangles: 0,
            bytes: size.0.saturating_mul(size.1).saturating_mul(4),
            tags: row_tags("png", name),
            mean_rgb: None,
        });
    }
    entries
}

/// One row per `bundle_entries` entry, kind taken from the bundle.
fn bundle_entries(doc: &Value) -> Vec<Entry> {
    let mut entries = Vec::new();
    for entry in doc
        .get("bundle_entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = entry.get("name").and_then(Value::as_str) else {
            continue;
        };
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("bundle")
            .to_lowercase();
        entries.push(Entry {
            name: name.to_owned(),
            kind: kind.clone(),
            digest: hash_hex(name.as_bytes()),
            triangles: 0,
            bytes: 0,
            tags: row_tags(&kind, name),
            mean_rgb: None,
        });
    }
    entries
}

fn word_score(tags: &[String], word: &str) -> u32 {
    let mut best = 0;
    for tag in tags {
        best = best.max(if tag == word {
            1000
        } else if tag.starts_with(word) {
            700
        } else if tag.contains(word) {
            400
        } else {
            0
        });
    }
    best
}

fn text_score(tags: &[String], words: &[String]) -> u32 {
    let mut total = 0u32;
    for word in words {
        let score = word_score(tags, word);
        if score == 0 {
            return 0;
        }
        total = total.saturating_add(score);
    }
    total
}

fn palette_rgb(raw: [u8; 6]) -> [u8; 3] {
    let channel = |lo: u8, hi: u8| {
        let value = u16::from(lo) | (u16::from(hi) << 8);
        u8::try_from(value / 257).unwrap_or(0)
    };
    [
        channel(raw[0], raw[1]),
        channel(raw[2], raw[3]),
        channel(raw[4], raw[5]),
    ]
}

fn srgb_to_lab(rgb: [u8; 3]) -> [f64; 3] {
    let linear = |byte: u8| {
        let v = f64::from(byte) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (linear(rgb[0]), linear(rgb[1]), linear(rgb[2]));
    let x = (r * 0.4124 + g * 0.3576 + b * 0.1805) / 0.950_47;
    let y = r * 0.2126 + g * 0.7152 + b * 0.0722;
    let z = (r * 0.0193 + g * 0.1192 + b * 0.9505) / 1.088_83;
    let f = |t: f64| {
        if t > 0.008_856 {
            t.cbrt()
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// CIE76 L*a*b* distance, rounded to an integer (documented palette distance).
#[must_use]
pub fn palette_distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    let (la, lb) = (srgb_to_lab(a), srgb_to_lab(b));
    let sum = (la[0] - lb[0]).powi(2) + (la[1] - lb[1]).powi(2) + (la[2] - lb[2]).powi(2);
    u32::try_from(engine_core::checked::saturating_f64_to_u64(
        sum.sqrt().round(),
    ))
    .unwrap_or(u32::MAX)
}

fn search(args: &SearchArgs) -> Result<SearchOutput, McpToolError> {
    let modes = usize::from(args.text.is_some())
        + usize::from(args.kind.is_some())
        + usize::from(args.palette.is_some());
    if modes == 0 {
        return Err(bad(
            "no query: send text, kind or palette (one of the three)".to_owned(),
        ));
    }
    if modes > 1 {
        return Err(bad(
            "more than one query: send text, kind or palette, never two of them".to_owned(),
        ));
    }
    let limit = args.limit.unwrap_or(16);
    if limit == 0 || limit > MAX_SEARCH_LIMIT {
        return Err(bad(format!(
            "limit {limit} is outside 1..={MAX_SEARCH_LIMIT}; send a smaller row count"
        )));
    }
    if let Some(kind) = &args.kind {
        let wanted = kind.to_lowercase();
        if !SEARCH_KINDS.contains(&wanted.as_str()) {
            return Err(bad(format!(
                "kind `{kind}` is not one of {}; send one of them",
                SEARCH_KINDS.join(", ")
            )));
        }
    }
    let report = args
        .index_json
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| content_dir().join("maps/report.json"));
    let mut entries = load_index(&report)?;
    let mode;
    let mut scored: Vec<(u32, Entry)> = Vec::new();
    if let Some(text) = &args.text {
        let words = tokens(text);
        if words.is_empty() {
            return Err(bad(
                "text has no word: send at least one letter or digit".to_owned()
            ));
        }
        mode = "text".to_owned();
        let phrase = text.trim().to_lowercase();
        for entry in entries.drain(..) {
            let score = if !phrase.is_empty() && entry.name.to_lowercase() == phrase {
                3000
            } else if entry.tags.iter().any(|tag| tag == &phrase) {
                2500
            } else {
                text_score(&entry.tags, &words)
            };
            if score > 0 {
                scored.push((score, entry));
            }
        }
    } else if let Some(kind) = &args.kind {
        let wanted = kind.to_lowercase();
        mode = "kind".to_owned();
        for entry in entries.drain(..) {
            if entry.kind == wanted {
                scored.push((1000, entry));
            }
        }
    } else {
        let wanted = palette_rgb(args.palette.unwrap_or([0; 6]));
        let cooked = report
            .parent()
            .map_or_else(|| content_dir().join("maps"), Path::to_path_buf)
            .join("cooked");
        mode = "palette".to_owned();
        scored = palette_rows(entries, wanted, &cooked);
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    Ok(answer(scored, limit, mode, args.palette.is_some()))
}

/// Ranked rows cut to `limit`, plus the match count and the mode.
fn answer(scored: Vec<(u32, Entry)>, limit: usize, mode: String, palette: bool) -> SearchOutput {
    let total = u64::try_from(scored.len()).unwrap_or(u64::MAX);
    let rows = scored
        .into_iter()
        .take(limit)
        .map(|(score, entry)| SearchRow {
            name: entry.name,
            kind: entry.kind,
            digest: entry.digest,
            triangles: entry.triangles,
            bytes: entry.bytes,
            score,
            mean_rgb: entry.mean_rgb,
        })
        .collect();
    SearchOutput {
        rows,
        total,
        mode,
        palette_max_delta: if palette { PALETTE_MAX_DELTA } else { 0 },
    }
}

fn search_verb(_server: &McpServer, args: &SearchArgs) -> Result<SearchOutput, McpToolError> {
    search(args)
}

/// Palette hits: every gltf row whose mean colour is within
/// [`PALETTE_MAX_DELTA`], scored `1000 - 70 x distance`. A row without a colour
/// (texture, bundle entry) cannot match a colour query and is skipped.
fn palette_rows(entries: Vec<Entry>, wanted: [u8; 3], cooked: &Path) -> Vec<(u32, Entry)> {
    let mut scored = Vec::new();
    for mut entry in entries {
        if entry.mean_rgb.is_none() && entry.kind == "gltf" {
            entry.mean_rgb = mean_colour(&cooked.join(format!("{}.glb", entry.name)));
        }
        let Some(colour) = entry.mean_rgb else {
            continue;
        };
        let distance = palette_distance(wanted, colour);
        if distance <= PALETTE_MAX_DELTA {
            scored.push((1000u32.saturating_sub(distance * 70), entry));
        }
    }
    scored
}

crate::e2_tool!(
    /// `asset.search`: ranked lookup over the committed kit report.
    SearchTool,
    name = TOOL_ASSET_SEARCH,
    description = "Search the committed kit report: text matches names and tags (exact tag, then word prefix, then substring), kind keeps one of gltf/png/mesh/material/clip/texture/bundle/map, palette keeps rows whose mean base colour is within CIE76 12 of the asked sRGB colour. Rows carry name, kind, digest, triangles, bytes and score, sorted by score then name; at most 64; no hit is an empty list.",
    readonly = true,
    args = SearchArgs,
    output = SearchOutput,
    invoke = search_verb
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpServer;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn server() -> McpServer {
        McpServer::new(11, Capability::unrestricted(11))
    }

    fn report() -> String {
        content_dir().join("maps/report.json").display().to_string()
    }

    fn run(args: SearchArgs) -> SearchOutput {
        SearchTool::invoke(&server(), args).expect("search runs")
    }

    #[test]
    fn a_text_query_ranks_the_exact_tag_first() {
        let out = run(SearchArgs {
            text: Some("siege_wall".to_owned()),
            index_json: Some(report()),
            limit: Some(8),
            ..SearchArgs::default()
        });
        assert_eq!(out.mode, "text");
        let first = out.rows.first().expect("a hit");
        assert_eq!(first.name, "siege_wall");
        assert_eq!(first.kind, "gltf");
        assert_eq!(first.triangles, 1920, "the report's LOD0 count");
        assert!(first.bytes > 0, "the cooked GLB is on disk");
        assert!(out.rows.len() <= 8);
        let second = run(SearchArgs {
            text: Some("siege_wall".to_owned()),
            index_json: Some(report()),
            limit: Some(8),
            ..SearchArgs::default()
        });
        assert_eq!(out.rows, second.rows, "two runs are equal");
    }

    #[test]
    fn a_kind_query_keeps_one_kind_and_an_unknown_one_is_typed() {
        let out = run(SearchArgs {
            kind: Some("png".to_owned()),
            index_json: Some(report()),
            limit: Some(MAX_SEARCH_LIMIT),
            ..SearchArgs::default()
        });
        assert!(!out.rows.is_empty());
        assert!(
            out.rows.iter().all(|row| row.kind == "png"),
            "{:?}",
            out.rows
        );
        let refused = SearchTool::invoke(
            &server(),
            SearchArgs {
                kind: Some("sound".to_owned()),
                index_json: Some(report()),
                ..SearchArgs::default()
            },
        )
        .expect_err("unknown kind");
        assert_eq!(refused.code, "bad_input");
        assert!(refused.message.contains("gltf"), "{}", refused.message);
    }

    #[test]
    fn a_palette_query_returns_only_close_pieces() {
        // A mid grey: every returned row must sit within the documented distance.
        let out = run(SearchArgs {
            palette: Some([128, 128, 128, 128, 128, 128]),
            index_json: Some(report()),
            limit: Some(MAX_SEARCH_LIMIT),
            ..SearchArgs::default()
        });
        assert_eq!(out.mode, "palette");
        assert_eq!(out.palette_max_delta, PALETTE_MAX_DELTA);
        assert!(!out.rows.is_empty(), "a mid grey matches some piece");
        for row in &out.rows {
            let colour = row.mean_rgb.expect("palette rows carry their colour");
            assert!(
                palette_distance([128, 128, 128], colour) <= PALETTE_MAX_DELTA,
                "{} is too far: {colour:?}",
                row.name
            );
        }
        let again = run(SearchArgs {
            palette: Some([128, 128, 128, 128, 128, 128]),
            index_json: Some(report()),
            limit: Some(MAX_SEARCH_LIMIT),
            ..SearchArgs::default()
        });
        assert_eq!(out.rows, again.rows, "two palette runs are equal");
    }

    #[test]
    fn a_no_hit_query_is_an_empty_list_not_an_error() {
        let out = run(SearchArgs {
            text: Some("zzzznotathing".to_owned()),
            index_json: Some(report()),
            ..SearchArgs::default()
        });
        assert!(out.rows.is_empty());
        assert_eq!(out.total, 0);
    }

    #[test]
    fn a_limit_over_64_or_no_query_is_refused() {
        let over = SearchTool::invoke(
            &server(),
            SearchArgs {
                text: Some("a".to_owned()),
                limit: Some(MAX_SEARCH_LIMIT + 1),
                index_json: Some(report()),
                ..SearchArgs::default()
            },
        )
        .expect_err("limit over 64");
        assert!(over.message.contains("64"), "{}", over.message);
        let none = SearchTool::invoke(
            &server(),
            SearchArgs {
                index_json: Some(report()),
                ..SearchArgs::default()
            },
        )
        .expect_err("no query");
        assert_eq!(none.code, "bad_input");
        assert!(none.message.contains("text, kind or palette"));
    }
}

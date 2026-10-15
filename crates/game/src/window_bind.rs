//! window_bind: staging of the hero + 2 opponents for the window twin.
//!
//! Consumer: the window twin draw list (binds 3 staged fighters by
//! slot, so the owner frame shows fighters instead of the lone cube).
//! Owner: this module owns placement parsing plus the pin checks against
//! the pinned bundle SHAs. Units: meters for `x`/`z`, degrees for `yaw_deg`,
//! SHAs as lowercase hex. Determinism: the same JSON bytes give the same
//! placements in slot order; no wall clock, no RNG. Errors:
//! [`WindowBindError`] at every boundary; a missing file names the row
//! loudly, a torn file fails with the culprit slot, never stages the
//! wrong bodies.
//!
//! Budget: one-shot setup IO (one JSON read); never on the fixed tick.

use std::path::{Path, PathBuf};

use crate::capture::cap_message;
use crate::fighter_bind;

/// Window-bind file name inside the fighters content directory.
pub const WINDOW_BIND_NAME: &str = "window_bind.json";
/// Board row that owns this file.
pub const WINDOW_BIND_ROW: &str = "CT29";
/// Staged fighter count: the hero (slot 0) plus two opponents.
pub const WINDOW_BIND_SLOTS: usize = 3;
/// Arena half-extent in meters (matches the staged-fight receipt).
pub const WINDOW_BIND_HALF_EXTENT_M: f32 = 20.0;
/// Minimum pairwise spawn separation in meters (no shared footing).
pub const WINDOW_BIND_MIN_SEPARATION_M: f32 = 1.5;

/// One staged window fighter: slot identity plus spawn placement.
#[derive(Clone, PartialEq, Debug)]
pub struct WindowBind {
    /// Fighter slot (`0..WINDOW_BIND_SLOTS`).
    pub slot: u8,
    /// Variant name (`fighter_bind::VARIANTS[slot]`).
    pub variant: String,
    /// Bundle SHA-256 (pinned against `fighter_bind`).
    pub bundle_sha256: String,
    /// Spawn feet `x` in meters.
    pub x: f32,
    /// Spawn feet `z` in meters.
    pub z: f32,
    /// Spawn facing in degrees (finite, never hashed).
    pub yaw_deg: f32,
}

/// Window-bind failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WindowBindError {
    /// Filesystem failure; kind carried (paths stay in the caller).
    Io(std::io::ErrorKind),
    /// Missing file or torn placement; reason carries the row marker or the
    /// culprit slot.
    BadBind(String),
}

impl core::fmt::Display for WindowBindError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "window bind io ({kind:?})"),
            Self::BadBind(reason) => write!(f, "bad window bind ({reason})"),
        }
    }
}

impl std::error::Error for WindowBindError {}

/// Extract `"key": "<string>"` from a JSON block.
fn extract_string(block: &str, what: &str, key: &str) -> Result<String, WindowBindError> {
    let marker = format!("\"{key}\"");
    let at = block
        .find(marker.as_str())
        .ok_or_else(|| WindowBindError::BadBind(cap_message(&format!("{what} has no {key}"))))?;
    let after = &block[at + marker.len()..];
    let colon = after.find(':').ok_or_else(|| {
        WindowBindError::BadBind(cap_message(&format!("{what} {key} has no value")))
    })?;
    let value = after[colon + 1..].trim_start();
    let quoted = value.strip_prefix('"').ok_or_else(|| {
        WindowBindError::BadBind(cap_message(&format!("{what} {key} is not a string")))
    })?;
    let end = quoted.find('"').ok_or_else(|| {
        WindowBindError::BadBind(cap_message(&format!("{what} {key} is unterminated")))
    })?;
    Ok(quoted[..end].to_string())
}

/// Extract `"key": <number>` from a JSON block (integer rows).
fn extract_count(block: &str, what: &str, key: &str) -> Result<u32, WindowBindError> {
    let marker = format!("\"{key}\"");
    let at = block
        .find(marker.as_str())
        .ok_or_else(|| WindowBindError::BadBind(cap_message(&format!("{what} has no {key}"))))?;
    let after = &block[at + marker.len()..];
    let colon = after.find(':').ok_or_else(|| {
        WindowBindError::BadBind(cap_message(&format!("{what} {key} has no value")))
    })?;
    let value = after[colon + 1..].trim_start();
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return Err(WindowBindError::BadBind(cap_message(&format!(
            "{what} {key} is not a count"
        ))));
    }
    digits
        .parse::<u32>()
        .map_err(|_| WindowBindError::BadBind(cap_message(&format!("{what} {key} overflows u32"))))
}

/// Extract `"key": <float>` from a JSON block (placement rows).
fn extract_float(block: &str, what: &str, key: &str) -> Result<f32, WindowBindError> {
    let marker = format!("\"{key}\"");
    let at = block
        .find(marker.as_str())
        .ok_or_else(|| WindowBindError::BadBind(cap_message(&format!("{what} has no {key}"))))?;
    let after = &block[at + marker.len()..];
    let colon = after.find(':').ok_or_else(|| {
        WindowBindError::BadBind(cap_message(&format!("{what} {key} has no value")))
    })?;
    let value = after[colon + 1..].trim_start();
    let text: String = value
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect();
    if text.is_empty() {
        return Err(WindowBindError::BadBind(cap_message(&format!(
            "{what} {key} is not a number"
        ))));
    }
    text.parse::<f32>().map_err(|_| {
        WindowBindError::BadBind(cap_message(&format!("{what} {key} is not a number")))
    })
}

/// True for 64 lowercase hex chars (a SHA-256 row).
fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Split the `"slots"` array into one block per `"slot"` entry, so field
/// search never crosses placements.
fn slot_blocks(json: &str) -> Result<Vec<&str>, WindowBindError> {
    let marker = "\"slots\"";
    let at = json
        .find(marker)
        .ok_or_else(|| WindowBindError::BadBind(cap_message("CT29 window bind has no slots")))?;
    let rest = &json[at + marker.len()..];
    let mut starts = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = rest[cursor..].find("\"slot\"") {
        starts.push(cursor + found);
        cursor += found + "\"slot\"".len();
    }
    if starts.is_empty() {
        return Err(WindowBindError::BadBind(cap_message(
            "CT29 window bind slots are empty",
        )));
    }
    let mut blocks = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().enumerate() {
        let end = if index + 1 < starts.len() {
            starts[index + 1]
        } else {
            rest.len()
        };
        blocks.push(&rest[*start..end]);
    }
    Ok(blocks)
}

/// Parse window placements from JSON bytes: require the row marker,
/// exactly 3 placements on slots 0/1/2, each naming the pinned variant
/// plus bundle SHA with a finite in-bounds spawn and >= 1.5 m separation.
pub fn parse(json: &str) -> Result<Vec<WindowBind>, WindowBindError> {
    let row = extract_string(json, WINDOW_BIND_ROW, "row")?;
    if row != WINDOW_BIND_ROW {
        return Err(WindowBindError::BadBind(cap_message(&format!(
            "window bind row {row} is not {WINDOW_BIND_ROW}"
        ))));
    }
    let blocks = slot_blocks(json)?;
    if blocks.len() != WINDOW_BIND_SLOTS {
        return Err(WindowBindError::BadBind(cap_message(&format!(
            "CT29 window bind holds {} slots, want {WINDOW_BIND_SLOTS}",
            blocks.len()
        ))));
    }
    let mut binds = Vec::with_capacity(WINDOW_BIND_SLOTS);
    for block in &blocks {
        let what = "CT29 window slot";
        let slot = extract_count(block, what, "slot")?;
        if usize::try_from(slot).unwrap_or(usize::MAX) >= WINDOW_BIND_SLOTS {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slot {slot} is outside 0..{WINDOW_BIND_SLOTS}"
            ))));
        }
        let index = usize::try_from(slot).unwrap_or(usize::MAX);
        let variant = extract_string(block, what, "variant")?;
        if variant != fighter_bind::VARIANTS[index] {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slot {slot} variant {variant} is not {}",
                fighter_bind::VARIANTS[index]
            ))));
        }
        let bundle = extract_string(block, what, "bundle_sha256")?;
        if !is_hex64(&bundle) {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slot {slot} bundle is not a SHA-256"
            ))));
        }
        if bundle != fighter_bind::EXPECTED_BUNDLE_SHA256[index] {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slot {slot} bundle is not the CT7 bytes"
            ))));
        }
        let x = extract_float(block, what, "x")?;
        let z = extract_float(block, what, "z")?;
        let yaw_deg = extract_float(block, what, "yaw_deg")?;
        if !x.is_finite() || x.abs() > WINDOW_BIND_HALF_EXTENT_M {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slot {slot} x is out of bounds"
            ))));
        }
        if !z.is_finite() || z.abs() > WINDOW_BIND_HALF_EXTENT_M {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slot {slot} z is out of bounds"
            ))));
        }
        if !yaw_deg.is_finite() || yaw_deg.abs() > 360.0 {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slot {slot} yaw is out of bounds"
            ))));
        }
        let slot_u8 = u8::try_from(slot).unwrap_or(u8::MAX);
        binds.push(WindowBind {
            slot: slot_u8,
            variant,
            bundle_sha256: bundle,
            x,
            z,
            yaw_deg,
        });
    }
    binds.sort_by_key(|bind| bind.slot);
    for pair in 0..binds.len() {
        for other in (pair + 1)..binds.len() {
            if binds[pair].slot == binds[other].slot {
                return Err(WindowBindError::BadBind(cap_message(&format!(
                    "CT29 slot {} is staged twice",
                    binds[pair].slot
                ))));
            }
            let dx = binds[pair].x - binds[other].x;
            let dz = binds[pair].z - binds[other].z;
            if dx.hypot(dz) < WINDOW_BIND_MIN_SEPARATION_M {
                return Err(WindowBindError::BadBind(cap_message(&format!(
                    "CT29 slots {} and {} share footing",
                    binds[pair].slot, binds[other].slot
                ))));
            }
        }
    }
    for (index, bind) in binds.iter().enumerate() {
        if usize::from(bind.slot) != index {
            return Err(WindowBindError::BadBind(cap_message(&format!(
                "CT29 slots are not 0..{WINDOW_BIND_SLOTS}"
            ))));
        }
    }
    Ok(binds)
}

/// Load the window placements under `content_dir`
/// (`window_bind.json` beside the fighters `manifest.json`). A missing
/// file fails loudly naming the row, never falls back to the lone cube.
pub fn load(content_dir: &Path) -> Result<Vec<WindowBind>, WindowBindError> {
    let path: PathBuf = content_dir.join(WINDOW_BIND_NAME);
    let json = std::fs::read_to_string(&path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            WindowBindError::BadBind(cap_message(&format!(
                "CT29 {WINDOW_BIND_NAME} missing under {}",
                content_dir.display()
            )))
        } else {
            WindowBindError::Io(err.kind())
        }
    })?;
    parse(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    #[test]
    fn binds_ct29_window_triangle() {
        let binds = load(&content_dir()).expect("CT29 window bind loads");
        assert_eq!(binds.len(), WINDOW_BIND_SLOTS);
        for (index, bind) in binds.iter().enumerate() {
            assert_eq!(usize::from(bind.slot), index);
            assert_eq!(bind.variant, fighter_bind::VARIANTS[index]);
            assert_eq!(
                bind.bundle_sha256,
                fighter_bind::EXPECTED_BUNDLE_SHA256[index]
            );
            assert!(bind.x.abs() <= WINDOW_BIND_HALF_EXTENT_M);
            assert!(bind.z.abs() <= WINDOW_BIND_HALF_EXTENT_M);
            assert!(bind.yaw_deg.is_finite());
        }
        // Slots stage the opening triangle: hero west, opponents east.
        assert!(binds[0].x < 0.0);
        assert!(binds[1].x > 0.0 && binds[2].x > 0.0);
        // Same bytes twice give the same placements.
        let again = load(&content_dir()).expect("CT29 window bind repeats");
        assert_eq!(binds, again);
    }

    /// Minimal placement JSON carrying the pinned SHAs (the parser only needs
    /// the row marker plus slot blocks, so fixtures stay small and fast).
    fn synth_json(hero_bundle: &str, slot_one: u8) -> String {
        let mut out = String::from("{\"row\": \"CT29\", \"slots\": [");
        let slots = [(0u8, 0usize), (slot_one, 1usize), (2u8, 2usize)];
        for (entry, (slot, variant_index)) in slots.iter().enumerate() {
            if entry > 0 {
                out.push(',');
            }
            let bundle = if *variant_index == 0 {
                hero_bundle.to_string()
            } else {
                fighter_bind::EXPECTED_BUNDLE_SHA256[*variant_index].to_string()
            };
            out.push_str(&format!(
                "{{\"slot\": {slot}, \"variant\": \"{}\", \"bundle_sha256\": \"{bundle}\", \"x\": {}, \"z\": 0.0, \"yaw_deg\": 0.0}}",
                fighter_bind::VARIANTS[*variant_index],
                f32::from(*slot) * 2.0 - 2.0,
            ));
        }
        out.push_str("]}");
        out
    }

    #[test]
    fn refuses_tampered_bundle_sha() {
        let err = parse(&synth_json(&"0".repeat(64), 1)).expect_err("zeroed hero bundle must fail");
        assert!(
            matches!(err, WindowBindError::BadBind(_)),
            "wrong error: {err}"
        );
        assert!(err.to_string().contains("slot 0"), "culprit lost: {err}");
    }

    #[test]
    fn missing_file_names_ct29() {
        let dir = std::env::temp_dir().join(format!("e2040-windowbind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let err = load(&dir).expect_err("missing window bind must fail");
        assert!(err.to_string().contains("CT29"), "row unnamed: {err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_shared_footing() {
        // Slots 1 and 2 collapse onto the same spawn: separation rejects.
        let crowded = "{\"row\": \"CT29\", \"slots\": [{\"slot\": 0, \"variant\": \"hero\", \"bundle_sha256\": \"".to_string()
            + fighter_bind::EXPECTED_BUNDLE_SHA256[0]
            + "\", \"x\": -2.0, \"z\": 0.0, \"yaw_deg\": 0.0}, {\"slot\": 1, \"variant\": \"opp_male_light\", \"bundle_sha256\": \""
            + fighter_bind::EXPECTED_BUNDLE_SHA256[1]
            + "\", \"x\": 2.0, \"z\": 0.0, \"yaw_deg\": 0.0}, {\"slot\": 2, \"variant\": \"opp_female_medium\", \"bundle_sha256\": \""
            + fighter_bind::EXPECTED_BUNDLE_SHA256[2]
            + "\", \"x\": 2.0, \"z\": 0.0, \"yaw_deg\": 0.0}]}";
        let err = parse(&crowded).expect_err("shared footing must fail");
        assert!(err.to_string().contains("footing"), "culprit lost: {err}");
    }
}

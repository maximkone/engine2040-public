//! kit_load: arena placement table loading from JSON into actors.
//!
//! Consumer: `arena_kit` re-exports every public item here, so the headless
//! twin (`smoke_presenter`) and the live window keep calling `arena_kit::`
//! paths. Data owner: this module owns the placement parse (JSON text into
//! [`KitActor`]); the glTF files own the source meshes, `arena_kit` owns the
//! AABB fit and the presenter upload. Units: meters, radians (yaw about +Y).
//! Determinism: the same bytes give the same actors; pixels never enter the
//! sim hash. Errors: typed [`KitError`](super::KitError), never panics.
//! Budget: 11 actors parsed once at init; zero per-tick work, so the frame
//! cost stays flat over a match.

use super::{parse_err, KitError, MAX_KIT_POS_M, MAX_KIT_SIZE_M};

/// Embedded placement table (`content/arena/placement.json`).
pub const KIT_JSON: &str = include_str!("../../../content/arena/placement.json");
/// Actors in the placement table (11 over 8 kit files).
pub const KIT_ACTOR_COUNT: usize = 11;

/// One of the 8 kit assets on disk (`content/arena/src/<file>`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KitAsset {
    /// Open sand plane.
    SandFloor,
    /// Tiered spectator stands.
    Stands,
    /// Arena enclosure walls (two actors share it).
    Walls,
    /// Vaulted gate tunnel.
    GateTunnel,
    /// Freestanding torch (two actors share it).
    Torch,
    /// Hanging banner (two actors share it).
    Banner,
    /// Supply crate.
    Crate,
    /// Scattered debris.
    Debris,
}

impl KitAsset {
    /// Parse the placement `"asset"` label; unknown labels fail loudly.
    pub fn parse(label: &str) -> Result<Self, KitError> {
        match label {
            "sand_floor" => Ok(Self::SandFloor),
            "stands" => Ok(Self::Stands),
            "walls" => Ok(Self::Walls),
            "gate_tunnel" => Ok(Self::GateTunnel),
            "torch" => Ok(Self::Torch),
            "banner" => Ok(Self::Banner),
            "crate" => Ok(Self::Crate),
            "debris" => Ok(Self::Debris),
            _ => Err(parse_err("unknown kit asset")),
        }
    }

    /// glTF file for this asset under `content/arena/src/`.
    #[must_use]
    pub fn file_name(self) -> &'static str {
        match self {
            Self::SandFloor => "sand_floor.gltf",
            Self::Stands => "stands.gltf",
            Self::Walls => "walls.gltf",
            Self::GateTunnel => "gate_tunnel.gltf",
            Self::Torch => "torch.gltf",
            Self::Banner => "banner.gltf",
            Self::Crate => "crate.gltf",
            Self::Debris => "debris.gltf",
        }
    }
}

/// One staged kit actor: placement box plus asset.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct KitActor {
    /// Stable actor id from the file (1..=11).
    pub id: u32,
    /// Which kit file this stages.
    pub asset: KitAsset,
    /// Box center in meters `[x, y, z]`.
    pub pos_m: [f32; 3],
    /// Full extents in meters `[sx, sy, sz]`.
    pub size_m: [f32; 3],
    /// Yaw about +Y in radians.
    pub yaw: f32,
    /// Placement `"physical"` flag: true actors collide in the level zones.
    pub physical: bool,
}

/// Raw field slice after `"needle":` in one actor block.
fn kit_field_raw<'a>(block: &'a str, needle: &str) -> Result<&'a str, KitError> {
    let at = block
        .find(needle)
        .ok_or_else(|| parse_err("kit actor field missing"))?;
    let rest = block
        .get(at + needle.len()..)
        .ok_or_else(|| parse_err("kit actor field missing"))?;
    let colon = rest
        .find(':')
        .ok_or_else(|| parse_err("kit actor field missing"))?;
    rest.get(colon + 1..)
        .ok_or_else(|| parse_err("kit actor field missing"))
}

/// Parse one finite `f32` field. Rejects missing, unparsable and non-finite
/// values loudly.
fn kit_parse_f32(block: &str, needle: &str) -> Result<f32, KitError> {
    let rest = kit_field_raw(block, needle)?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let token = rest.get(..end).unwrap_or("").trim();
    let value: f32 = token
        .parse()
        .map_err(|_| parse_err("kit actor number unparsable"))?;
    if !value.is_finite() {
        return Err(parse_err("kit actor number not finite"));
    }
    Ok(value)
}

/// Parse one `u32` id field.
fn kit_parse_id(block: &str) -> Result<u32, KitError> {
    let rest = kit_field_raw(block, "\"id\"")?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let token = rest.get(..end).unwrap_or("").trim();
    token
        .parse::<u32>()
        .map_err(|_| parse_err("kit actor id unparsable"))
}

/// Parse one asset label from its quoted string value.
fn kit_parse_asset(block: &str) -> Result<KitAsset, KitError> {
    let rest = kit_field_raw(block, "\"asset\"")?;
    let quote = rest
        .find('"')
        .ok_or_else(|| parse_err("kit actor asset unparsable"))?;
    let tail = rest.get(quote + 1..).unwrap_or("");
    let end = tail
        .find('"')
        .ok_or_else(|| parse_err("kit actor asset unparsable"))?;
    KitAsset::parse(tail.get(..end).unwrap_or(""))
}

/// Parse one `"physical"` flag. Rejects missing and non-boolean values
/// loudly so an unread flag never silently drops collision.
fn kit_parse_physical(block: &str) -> Result<bool, KitError> {
    let rest = kit_field_raw(block, "\"physical\"")?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let token = rest.get(..end).unwrap_or("").trim();
    match token {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(parse_err("kit actor physical unparsable")),
    }
}

/// Parse one actor object block (no nested objects inside) and validate its
/// ranges: nonzero id, known asset, finite pose, sizes in `(0, 100]`,
/// plus the `"physical"` collision flag.
pub(crate) fn kit_actor_from_block(block: &str) -> Result<KitActor, KitError> {
    let id = kit_parse_id(block)?;
    if id == 0 {
        return Err(parse_err("kit actor id is zero"));
    }
    let asset = kit_parse_asset(block)?;
    let pos_m = [
        kit_parse_f32(block, "\"x\"")?,
        kit_parse_f32(block, "\"y\"")?,
        kit_parse_f32(block, "\"z\"")?,
    ];
    let size_m = [
        kit_parse_f32(block, "\"sx\"")?,
        kit_parse_f32(block, "\"sy\"")?,
        kit_parse_f32(block, "\"sz\"")?,
    ];
    let yaw = kit_parse_f32(block, "\"yaw\"")?;
    let physical = kit_parse_physical(block)?;
    if pos_m[0].abs() > MAX_KIT_POS_M || pos_m[2].abs() > MAX_KIT_POS_M {
        return Err(parse_err("kit actor out of bounds"));
    }
    for size in size_m {
        if !(0.0 < size && size <= MAX_KIT_SIZE_M) {
            return Err(parse_err("kit actor size out of range"));
        }
    }
    Ok(KitActor {
        id,
        asset,
        pos_m,
        size_m,
        yaw,
        physical,
    })
}

/// Parse the kit actor table from JSON text. Expects exactly
/// [`KIT_ACTOR_COUNT`] objects inside the `"actors"` array with unique ids;
/// any shape drift fails loudly so a stale binary never renders a stale kit.
pub fn kit_actors_from_json(text: &str) -> Result<Vec<KitActor>, KitError> {
    let anchor = text
        .find("\"actors\"")
        .ok_or_else(|| parse_err("kit placement misses actors"))?;
    let tail = text
        .get(anchor..)
        .ok_or_else(|| parse_err("kit placement misses actors"))?;
    let open = tail
        .find('[')
        .ok_or_else(|| parse_err("kit placement misses actors"))?;
    let mut cursor = anchor + open + 1;
    let mut out = Vec::with_capacity(KIT_ACTOR_COUNT);
    while out.len() < KIT_ACTOR_COUNT {
        let rest = text
            .get(cursor..)
            .ok_or_else(|| parse_err("kit actor missing"))?;
        let open_rel = rest
            .find('{')
            .ok_or_else(|| parse_err("kit actor missing"))?;
        let body = rest.get(open_rel + 1..).unwrap_or("");
        let close_rel = body
            .find('}')
            .ok_or_else(|| parse_err("kit actor missing"))?;
        let block = body.get(..close_rel).unwrap_or("");
        out.push(kit_actor_from_block(block)?);
        cursor += open_rel + 1 + close_rel + 1;
    }
    let rest = text.get(cursor..).unwrap_or("");
    if let Some(open_pos) = rest.find('{') {
        let extra_before_close = match rest.find(']') {
            None => true,
            Some(close_pos) => open_pos < close_pos,
        };
        if extra_before_close {
            return Err(parse_err("kit actor count mismatch"));
        }
    }
    let mut ids: Vec<u32> = out.iter().map(|actor| actor.id).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.len() != KIT_ACTOR_COUNT {
        return Err(parse_err("kit actor ids are not unique"));
    }
    Ok(out)
}

/// Parse the embedded `content/arena/placement.json` table. Same bytes give
/// the same actors; a drifted file fails loudly at staging time.
pub fn kit_actors() -> Result<Vec<KitActor>, KitError> {
    kit_actors_from_json(KIT_JSON)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_first_actor_is_sand_floor() {
        let actors = kit_actors().expect("embedded placement parses");
        let first = &actors[0];
        assert_eq!(first.id, 1);
        assert_eq!(first.asset, KitAsset::SandFloor);
        assert!(first.physical);
        assert_eq!(first.asset.file_name(), "sand_floor.gltf");
    }

    #[test]
    fn asset_files_follow_first_seen_order() {
        let actors = kit_actors().expect("embedded placement parses");
        let files = crate::arena_kit::kit_asset_files(&actors);
        assert_eq!(files.len(), super::super::KIT_ASSET_COUNT);
        assert_eq!(files[0], "sand_floor.gltf");
        // Shared files stage once: walls dress two actors, torches two.
        let mut sorted = files.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), files.len());
    }

    #[test]
    fn rejects_extra_actor_beyond_eleven() {
        let mut text = String::from("{\"actors\": [");
        for id in 1..=12u32 {
            if id > 1 {
                text.push(',');
            }
            text.push_str(&format!(
                "{{\"id\": {id}, \"asset\": \"crate\", \"x\": 0, \"y\": 1, \"z\": 0, \"sx\": 1, \"sy\": 1, \"sz\": 1, \"yaw\": 0, \"physical\": true}}"
            ));
        }
        text.push_str("]}");
        assert!(
            kit_actors_from_json(&text).is_err(),
            "12 actors must fail, want exactly 11"
        );
    }

    #[test]
    fn placement_parses_11_x2_stable() {
        let first = kit_actors().expect("embedded placement parses");
        let second = kit_actors().expect("embedded placement parses twice");
        assert_eq!(first.len(), KIT_ACTOR_COUNT);
        assert_eq!(second.len(), KIT_ACTOR_COUNT);
        for (index, (a, b)) in first.iter().zip(second.iter()).enumerate() {
            let want = u32::try_from(index + 1).expect("kit id fits u32");
            assert_eq!(a.id, want);
            assert_eq!(b.id, want);
            assert_eq!(a.asset, b.asset);
            assert_eq!(a.pos_m, b.pos_m);
            assert_eq!(a.size_m, b.size_m);
            assert_eq!(a.yaw, b.yaw);
            assert_eq!(a.physical, b.physical);
        }
        let from_json = kit_actors_from_json(KIT_JSON).expect("KIT_JSON parses");
        assert_eq!(from_json, first);
        let end = KIT_JSON.find('}').expect("first actor closes");
        let truncated = KIT_JSON.get(..end).unwrap_or("");
        assert!(
            kit_actors_from_json(truncated).is_err(),
            "truncated placement must fail"
        );
    }
}

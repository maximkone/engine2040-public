//! kit_load: arena placement table loading from JSON into actors.
//!
//! Consumer: `arena_kit` re-exports every public item here, so the headless
//! twin (`smoke_presenter`) and the live window keep calling `arena_kit::`
//! paths. Data owner: this module owns the placement parse (JSON text into
//! [`KitActor`]); the glTF files own the source meshes, `arena_kit` owns the
//! AABB fit and the presenter upload. Units: meters, radians (yaw about +Y).
//! Determinism: the same bytes give the same actors; pixels never enter the
//! sim hash. Errors: typed [`KitError`](super::KitError), never panics.
//! Budget: 14 actors parsed once at init; zero per-tick work, so the frame
//! cost stays flat over a match.

use super::{parse_err, KitError, MAX_KIT_POS_M, MAX_KIT_SIZE_M};

/// Embedded placement table (`content/arena/placement.json`).
pub const KIT_JSON: &str = include_str!("../../../content/arena/placement.json");
/// Actors in the placement table (14 over 11 kit files).
pub const KIT_ACTOR_COUNT: usize = 14;

/// One of the 11 kit assets on disk: 8 under `content/arena/src/<file>`, the
/// three machine props under `content/generated/props/<file>` (SHOW4).
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
    /// Seeded siege ballista, a look-only prop from the props kit (SHOW4).
    Ballista,
    /// Seeded treadwheel crane, a look-only prop from the props kit (SHOW4).
    Crane,
    /// Seeded supply cart, a look-only prop from the props kit (SHOW4).
    Cart,
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
            "machine_ballista" => Ok(Self::Ballista),
            "machine_crane" => Ok(Self::Crane),
            "machine_cart" => Ok(Self::Cart),
            _ => Err(parse_err("unknown kit asset")),
        }
    }

    /// glTF file for this asset: the eight arena files sit under
    /// `content/arena/src/`, the three machine props under
    /// `content/generated/props/` (the props kit's own output).
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
            Self::Ballista => "machine_ballista.glb",
            Self::Crane => "machine_crane.glb",
            Self::Cart => "machine_cart.glb",
        }
    }

    /// True for the props-kit machines: they load from `content/generated/props`
    /// and stand as look-only dressing, never as colliders.
    #[must_use]
    pub const fn is_prop_kit(self) -> bool {
        matches!(self, Self::Ballista | Self::Crane | Self::Cart)
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
    /// Placement `"texture_scale"`: how many times this actor's material tiles
    /// across its own UVs, `1.0` when the file names no scale (SW124).
    pub texture_scale: f32,
}

/// Tiling of an actor that names no `"texture_scale"`: the file's own UVs.
pub const KIT_TEXTURE_SCALE_DEFAULT: f32 = 1.0;
/// Largest tiling one actor may ask for: past this a typo shreds the UVs.
pub const KIT_TEXTURE_SCALE_MAX: f32 = 16.0;

/// Multiply every UV by `scale`: how many times one actor's material tiles
/// across its own UVs (placement `"texture_scale"`). A scale of `1.0` leaves
/// the buffer untouched. Pure, in place; cold init only.
pub fn scale_uvs(uvs: &mut [[f32; 2]], scale: f32) {
    if scale == 1.0 {
        return;
    }
    for uv in uvs.iter_mut() {
        uv[0] *= scale;
        uv[1] *= scale;
    }
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

/// Parse one `"texture_scale"` tiling. An actor that names none keeps
/// [`KIT_TEXTURE_SCALE_DEFAULT`]; a value of zero, a negative or one past
/// [`KIT_TEXTURE_SCALE_MAX`] refuses loudly, so a broken row never silently
/// draws the file's own UVs.
fn kit_parse_texture_scale(block: &str) -> Result<f32, KitError> {
    if !block.contains("\"texture_scale\"") {
        return Ok(KIT_TEXTURE_SCALE_DEFAULT);
    }
    let value = kit_parse_f32(block, "\"texture_scale\"")?;
    if !(0.0 < value && value <= KIT_TEXTURE_SCALE_MAX) {
        return Err(parse_err("kit actor texture_scale out of range"));
    }
    Ok(value)
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
    let texture_scale = kit_parse_texture_scale(block)?;
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
        texture_scale,
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
        // The sand slab is the ground the duel is fought on, never a wall:
        // the shipped flag is false and the collision world skips it.
        assert!(
            !first.physical,
            "the sand floor is dressing, not a collider"
        );
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
    fn rejects_extra_actor_beyond_fourteen() {
        let mut text = String::from("{\"actors\": [");
        for id in 1..=15u32 {
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
            "15 actors must fail, want exactly 14"
        );
    }

    #[test]
    /// SW124: an actor's `"texture_scale"` is read, defaults to the file's own
    /// UVs when absent, and refuses a zero, a negative, a value past the cap or
    /// a token that is not a number.
    #[test]
    fn texture_scale_defaults_to_one_and_refuses_a_broken_row() {
        let block = |tail: &str| {
            format!(
                "\"id\": 1, \"asset\": \"crate\", \"x\": 0, \"y\": 1, \"z\": 0, \"sx\": 1, \"sy\": 1, \"sz\": 1, \"yaw\": 0, \"physical\": false{tail}"
            )
        };
        let plain = kit_actor_from_block(&block("")).expect("no scale parses");
        assert_eq!(plain.texture_scale, KIT_TEXTURE_SCALE_DEFAULT);
        let eight = kit_actor_from_block(&block(", \"texture_scale\": 8")).expect("8x parses");
        assert_eq!(eight.texture_scale, 8.0);
        for bad in [
            ", \"texture_scale\": 0",
            ", \"texture_scale\": -2",
            ", \"texture_scale\": 17",
            ", \"texture_scale\": 1e40",
            ", \"texture_scale\": nope",
        ] {
            assert!(
                kit_actor_from_block(&block(bad)).is_err(),
                "{bad} must refuse"
            );
        }
    }

    /// SW124: the arena floor tiles the sand at 8x its own UVs, so the 2 m set
    /// and its 0.25 m detail are on screen together; every other actor keeps
    /// the file's own UVs. The multiply is the whole edit `fitted_buffers`
    /// makes, so this pins it without a GLB.
    #[test]
    fn the_floor_tiles_its_sand_eight_times_and_one_leaves_uvs_alone() {
        let actors = kit_actors().expect("embedded placement parses");
        assert_eq!(actors[0].asset, KitAsset::SandFloor);
        assert_eq!(actors[0].texture_scale, 8.0, "the floor's fitted detail");
        for actor in &actors[1..] {
            assert_eq!(
                actor.texture_scale, KIT_TEXTURE_SCALE_DEFAULT,
                "actor {} keeps its own UVs",
                actor.id
            );
        }
        let mut uvs = [[0.0f32, 0.0], [0.25, 0.5], [1.0, 1.0]];
        let mut same = uvs;
        scale_uvs(&mut same, 1.0);
        assert_eq!(same, uvs, "1.0 is the file's own UVs, untouched");
        scale_uvs(&mut uvs, 8.0);
        assert_eq!(uvs, [[0.0, 0.0], [2.0, 4.0], [8.0, 8.0]]);
    }

    #[test]
    fn placement_parses_14_x2_stable() {
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

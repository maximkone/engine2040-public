//! schema: the JSON Schema an AI reads before writing a record.
//!
//! Owner: `engine-material`. [`SCHEMA`] is the discoverable contract of
//! [`crate::record::MaterialRecord`] v1: every key, bound and enum name the
//! parser accepts, in under 8 KB so a door verb can hand it over whole. The
//! parser is the law; the test below pins that every key the parser knows
//! appears here, so the two cannot drift apart silently.

/// JSON Schema (draft 2020-12) of material record v1.
pub const SCHEMA: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "engine2040/material/v1",
  "title": "engine2040 material record v1",
  "description": "One record for look, physics, sound and damage. body, sound and fx are the sim half (plain decimals read as Q16 fixed point); look and layers are the frame half. Unknown keys are refused by path. A record may extend another: omitted fields come from the parent.",
  "type": "object",
  "additionalProperties": false,
  "required": ["material", "version"],
  "properties": {
    "material": {"$ref": "#/$defs/name", "description": "Unique name; also the wire id order in a set."},
    "version": {"const": 1},
    "extends": {"$ref": "#/$defs/name", "description": "Parent record. Without it body, sound and fx are required in full."},
    "domain": {"enum": ["surface"], "default": "surface"},
    "look": {"$ref": "#/$defs/look"},
    "layers": {"type": "array", "maxItems": 8, "items": {"$ref": "#/$defs/layer"}, "description": "Gameplay layers applied in order when their channel is high."},
    "body": {
      "type": "object", "additionalProperties": false,
      "properties": {
        "toughness": {"type": "number", "exclusiveMinimum": 0, "maximum": 64, "description": "Bond strain limit; glass 0.5, timber 4, stone 8, iron 32."},
        "brittleness": {"$ref": "#/$defs/unit", "description": "0 dents, 1 shatters fully."},
        "shard": {"enum": ["few_large", "mixed", "splinters", "shards", "dust", "sheets"]},
        "burn": {"type": "number", "minimum": 0, "maximum": 16, "description": "Fuel rate; 0 never burns, thatch 4."}
      }
    },
    "sound": {
      "type": "object", "additionalProperties": false,
      "properties": {"impact": {"enum": ["stone", "wood", "metal", "soft", "glass", "crack"]}}
    },
    "fx": {
      "type": "object", "additionalProperties": false,
      "properties": {
        "dust": {"enum": ["none", "stone", "brick", "wood", "pale", "fiber"]},
        "spark": {"enum": ["none", "few", "shower"]},
        "decal": {"$ref": "#/$defs/decal"}
      }
    }
  },
  "$defs": {
    "name": {"type": "string", "pattern": "^[0-9A-Za-z._-]{1,64}$"},
    "unit": {"type": "number", "minimum": 0, "maximum": 1},
    "rgb": {"type": "array", "minItems": 3, "maxItems": 3, "items": {"$ref": "#/$defs/unit"}, "description": "Linear RGB."},
    "ior": {"type": "number", "minimum": 1, "maximum": 3},
    "ref": {"type": ["string", "null"], "pattern": "^[0-9A-Za-z._:-]{1,128}$", "description": "tex:<name> or an asset id; null clears the slot."},
    "decal": {"enum": ["crack", "chip", "dent", "tear", "shatter", "scorch"]},
    "look": {
      "type": "object", "additionalProperties": false,
      "description": "OpenPBR names. Every field has a default; lobes are absent unless written, null removes an inherited lobe.",
      "properties": {
        "base_color": {"$ref": "#/$defs/rgb", "default": [1, 1, 1]},
        "base_metalness": {"$ref": "#/$defs/unit", "default": 0},
        "specular_roughness": {"$ref": "#/$defs/unit", "default": 0.5},
        "specular_ior": {"$ref": "#/$defs/ior", "default": 1.5},
        "normal_scale": {"type": "number", "minimum": 0, "maximum": 4, "default": 1},
        "occlusion_strength": {"$ref": "#/$defs/unit", "default": 1},
        "coat": {"type": ["object", "null"], "additionalProperties": false, "required": ["weight", "roughness", "ior"],
          "properties": {"weight": {"$ref": "#/$defs/unit"}, "roughness": {"$ref": "#/$defs/unit"}, "ior": {"$ref": "#/$defs/ior"}},
          "description": "Clear coat; a wet film is ior 1.33."},
        "sheen": {"type": ["object", "null"], "additionalProperties": false, "required": ["weight", "color", "roughness"],
          "properties": {"weight": {"$ref": "#/$defs/unit"}, "color": {"$ref": "#/$defs/rgb"}, "roughness": {"$ref": "#/$defs/unit"}}},
        "anisotropy": {"type": ["object", "null"], "additionalProperties": false, "required": ["strength", "rotation"],
          "properties": {"strength": {"$ref": "#/$defs/unit"}, "rotation": {"$ref": "#/$defs/unit", "description": "Tangent rotation in turns."}}},
        "transmission": {"type": ["object", "null"], "additionalProperties": false, "required": ["weight", "thickness", "attenuation_color"],
          "properties": {"weight": {"$ref": "#/$defs/unit"}, "thickness": {"type": "number", "minimum": 0, "maximum": 10, "description": "Meters."}, "attenuation_color": {"$ref": "#/$defs/rgb"}}},
        "subsurface": {"type": ["object", "null"], "additionalProperties": false, "required": ["weight", "radius", "color"],
          "properties": {"weight": {"$ref": "#/$defs/unit"}, "radius": {"$ref": "#/$defs/rgb", "description": "Scatter radius per channel, meters."}, "color": {"$ref": "#/$defs/rgb"}}},
        "emission": {"type": ["object", "null"], "additionalProperties": false, "required": ["color", "luminance"],
          "properties": {"color": {"$ref": "#/$defs/rgb"}, "luminance": {"type": "number", "minimum": 0, "maximum": 100000, "description": "Nits."}}},
        "textures": {"type": "object", "additionalProperties": false,
          "properties": {"base_color": {"$ref": "#/$defs/ref"}, "orm": {"$ref": "#/$defs/ref", "description": "Occlusion R, roughness G, metalness B."}, "normal": {"$ref": "#/$defs/ref"}, "emissive": {"$ref": "#/$defs/ref"}}}
      }
    },
    "layer": {
      "type": "object", "additionalProperties": false, "required": ["kind", "driver"],
      "properties": {
        "kind": {"enum": ["wet", "dirt", "blood", "damage", "burn"]},
        "driver": {"enum": ["wetness", "dirt", "blood", "strain", "heat"], "description": "Per-object coverage channel, 0..1, published by presentation from sim events."},
        "strength": {"$ref": "#/$defs/unit", "default": 1},
        "tint": {"$ref": "#/$defs/rgb", "description": "Mixed over base color at full coverage."},
        "darken": {"$ref": "#/$defs/unit", "description": "Base color darkening at full coverage."},
        "roughness_to": {"$ref": "#/$defs/unit", "description": "Roughness the surface moves toward at full coverage."},
        "decal": {"$ref": "#/$defs/decal", "description": "Decal family laid at full coverage."}
      }
    }
  }
}
"##;

/// Byte cap the door verb hands over whole.
pub const MAX_SCHEMA_BYTES: usize = 8 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use crate::record::{
        ANISOTROPY_KEYS, BODY_KEYS, COAT_KEYS, EMISSION_KEYS, FX_KEYS, LAYER_KEYS, LOOK_KEYS,
        SHEEN_KEYS, SOUND_KEYS, SUBSURFACE_KEYS, TEXTURE_KEYS, TOP_KEYS, TRANSMISSION_KEYS,
    };
    use engine_destruction::material::{DecalKind, DustKind, ShardRule, SoundKind, SparkKind};

    #[test]
    fn schema_is_json_under_the_cap() {
        assert!(SCHEMA.len() <= MAX_SCHEMA_BYTES, "{} bytes", SCHEMA.len());
        let doc = json::parse(SCHEMA).expect("schema is JSON");
        assert_eq!(
            doc.get("$id").and_then(json::Value::as_str),
            Some("engine2040/material/v1")
        );
        assert!(doc.get("properties").is_some());
    }

    #[test]
    fn every_key_the_parser_knows_is_in_the_schema() {
        let tables: [&[&str]; 13] = [
            TOP_KEYS,
            LOOK_KEYS,
            COAT_KEYS,
            SHEEN_KEYS,
            ANISOTROPY_KEYS,
            TRANSMISSION_KEYS,
            SUBSURFACE_KEYS,
            EMISSION_KEYS,
            TEXTURE_KEYS,
            LAYER_KEYS,
            BODY_KEYS,
            SOUND_KEYS,
            FX_KEYS,
        ];
        for table in tables {
            for key in table {
                assert!(SCHEMA.contains(&format!("\"{key}\"")), "schema lacks {key}");
            }
        }
    }

    #[test]
    fn every_enum_name_is_in_the_schema() {
        let shard = [
            ShardRule::FewLarge,
            ShardRule::Mixed,
            ShardRule::Splinters,
            ShardRule::Shards,
            ShardRule::Dust,
            ShardRule::Sheets,
        ];
        for value in shard {
            assert!(SCHEMA.contains(&format!("\"{}\"", value.name())));
        }
        for value in [
            SoundKind::Stone,
            SoundKind::Wood,
            SoundKind::Metal,
            SoundKind::Soft,
            SoundKind::Glass,
            SoundKind::Crack,
        ] {
            assert!(SCHEMA.contains(&format!("\"{}\"", value.name())));
        }
        for value in [
            DustKind::None,
            DustKind::Stone,
            DustKind::Brick,
            DustKind::Wood,
            DustKind::Pale,
            DustKind::Fiber,
        ] {
            assert!(SCHEMA.contains(&format!("\"{}\"", value.name())));
        }
        for value in [SparkKind::None, SparkKind::Few, SparkKind::Shower] {
            assert!(SCHEMA.contains(&format!("\"{}\"", value.name())));
        }
        for value in [
            DecalKind::Crack,
            DecalKind::Chip,
            DecalKind::Dent,
            DecalKind::Tear,
            DecalKind::Shatter,
            DecalKind::Scorch,
        ] {
            assert!(SCHEMA.contains(&format!("\"{}\"", value.name())));
        }
    }
}

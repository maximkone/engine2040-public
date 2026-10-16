//! Scene-component schema gate vectors: 40 JSON cases gate valid/invalid.
//!
//! Consumer: `engine_scene::component::Component::validate` range rules own
//! the truth; this gate mirrors them over `serde_json::Value` so authored
//! JSON refuses before it parses. Units: meters, 0..=1 colors, integer
//! counts. Determinism: same payload gives the same accept/refuse, no IO.
//! Errors: `Err(String)` names the field and the legal shape.
//!
//! PORT(Stranger6667/jsonschema@testsuite:vectors shape) MIT
//! https://github.com/Stranger6667/jsonschema - testsuite vectors shape
//! only (description/data/valid rows); all Rust below is fresh, no new dep.

use engine_scene::component::{MAX_ASSET_NAME_LEN, MAX_CHUNKS, MAX_EXTENT_M, MAX_LIMBS};
use serde_json::Value;

/// One gate vector: JSON payload of one component kind plus the verdict.
#[derive(Clone, Copy, Debug)]
struct Vector {
    name: &'static str,
    kind: &'static str,
    payload: &'static str,
    valid: bool,
    category: &'static str,
}

// 20 valid, 20 invalid (5 types, 5 required, 5 enum, 5 min/max).
const VECTORS: [Vector; 40] = [
    Vector {
        name: "mesh minimal defaults",
        kind: "mesh",
        payload: r#"{"asset":"meshes/wall"}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "mesh full",
        kind: "mesh",
        payload: r#"{"asset":"meshes/wall","cast_shadows":false}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "collider box",
        kind: "collider",
        payload: r#"{"shape":"box","half":[1.0,2.5,0.125]}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "collider sphere sensor",
        kind: "collider",
        payload: r#"{"shape":"sphere","half":[0.5,0.5,0.5],"sensor":true}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "collider capsule",
        kind: "collider",
        payload: r#"{"shape":"capsule","half":[0.25,1.0,0.25]}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "material minimal defaults",
        kind: "material",
        payload: r#"{"asset":"marble"}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "material full tint",
        kind: "material",
        payload: r#"{"asset":"marble","tint":[1.0,0.5,0.25]}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "light point",
        kind: "light",
        payload: r#"{"kind":"point","color":[1.0,1.0,1.0],"intensity":12.5,"range":30.0}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "light spot",
        kind: "light",
        payload: r#"{"kind":"spot","color":[1.0,0.9,0.8],"intensity":12.5,"range":30.0}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "light directional zero range",
        kind: "light",
        payload: r#"{"kind":"directional","color":[1.0,1.0,1.0],"intensity":1.0,"range":0.0}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "spawner minimal edges",
        kind: "spawner",
        payload: r#"{"archetype":"fighter.grunt","count":1,"period_ticks":1}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "spawner typical",
        kind: "spawner",
        payload: r#"{"archetype":"fighter.grunt","count":3,"period_ticks":600}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "destructible typical",
        kind: "destructible",
        payload: r#"{"material":"stone","health":400,"chunks":64}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "destructible min edges",
        kind: "destructible",
        payload: r#"{"material":"stone","health":1,"chunks":1}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "anatomy typical",
        kind: "anatomy",
        payload: r#"{"template":"humanoid","limbs":4}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "anatomy max limbs",
        kind: "anatomy",
        payload: r#"{"template":"humanoid","limbs":32}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "rig typical",
        kind: "rig",
        payload: r#"{"skeleton":"hero_skel","clips":"hero_clips"}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "collider min half edge",
        kind: "collider",
        payload: r#"{"shape":"box","half":[0.0001,0.0001,0.0001]}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "light max edges",
        kind: "light",
        payload: r#"{"kind":"point","color":[1.0,1.0,1.0],"intensity":1000000.0,"range":100000.0}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "destructible max edges",
        kind: "destructible",
        payload: r#"{"material":"stone","health":4294967295,"chunks":4096}"#,
        valid: true,
        category: "valid",
    },
    Vector {
        name: "mesh asset is number",
        kind: "mesh",
        payload: r#"{"asset":123}"#,
        valid: false,
        category: "types",
    },
    Vector {
        name: "collider half is string",
        kind: "collider",
        payload: r#"{"shape":"box","half":"big"}"#,
        valid: false,
        category: "types",
    },
    Vector {
        name: "light intensity is string",
        kind: "light",
        payload: r#"{"kind":"point","color":[1.0,1.0,1.0],"intensity":"bright","range":5.0}"#,
        valid: false,
        category: "types",
    },
    Vector {
        name: "spawner count is float",
        kind: "spawner",
        payload: r#"{"archetype":"a","count":3.5,"period_ticks":600}"#,
        valid: false,
        category: "types",
    },
    Vector {
        name: "rig skeleton is null",
        kind: "rig",
        payload: r#"{"skeleton":null,"clips":"c"}"#,
        valid: false,
        category: "types",
    },
    Vector {
        name: "mesh missing asset",
        kind: "mesh",
        payload: r#"{"cast_shadows":true}"#,
        valid: false,
        category: "required",
    },
    Vector {
        name: "collider missing shape",
        kind: "collider",
        payload: r#"{"half":[1.0,1.0,1.0]}"#,
        valid: false,
        category: "required",
    },
    Vector {
        name: "light missing color",
        kind: "light",
        payload: r#"{"kind":"point","intensity":5.0,"range":10.0}"#,
        valid: false,
        category: "required",
    },
    Vector {
        name: "spawner missing period",
        kind: "spawner",
        payload: r#"{"archetype":"a","count":2}"#,
        valid: false,
        category: "required",
    },
    Vector {
        name: "anatomy missing limbs",
        kind: "anatomy",
        payload: r#"{"template":"humanoid"}"#,
        valid: false,
        category: "required",
    },
    Vector {
        name: "collider shape cone",
        kind: "collider",
        payload: r#"{"shape":"cone","half":[1.0,1.0,1.0]}"#,
        valid: false,
        category: "enum",
    },
    Vector {
        name: "collider shape uppercase",
        kind: "collider",
        payload: r#"{"shape":"BOX","half":[1.0,1.0,1.0]}"#,
        valid: false,
        category: "enum",
    },
    Vector {
        name: "light kind laser",
        kind: "light",
        payload: r#"{"kind":"laser","color":[1.0,1.0,1.0],"intensity":5.0,"range":10.0}"#,
        valid: false,
        category: "enum",
    },
    Vector {
        name: "light kind empty",
        kind: "light",
        payload: r#"{"kind":"","color":[1.0,1.0,1.0],"intensity":5.0,"range":10.0}"#,
        valid: false,
        category: "enum",
    },
    Vector {
        name: "light kind is number",
        kind: "light",
        payload: r#"{"kind":1,"color":[1.0,1.0,1.0],"intensity":5.0,"range":10.0}"#,
        valid: false,
        category: "enum",
    },
    Vector {
        name: "collider half zero",
        kind: "collider",
        payload: r#"{"shape":"box","half":[0.0,1.0,1.0]}"#,
        valid: false,
        category: "minmax",
    },
    Vector {
        name: "material tint over one",
        kind: "material",
        payload: r#"{"asset":"m","tint":[2.0,0.0,0.0]}"#,
        valid: false,
        category: "minmax",
    },
    Vector {
        name: "light intensity negative",
        kind: "light",
        payload: r#"{"kind":"point","color":[1.0,1.0,1.0],"intensity":-1.0,"range":5.0}"#,
        valid: false,
        category: "minmax",
    },
    Vector {
        name: "spawner count zero",
        kind: "spawner",
        payload: r#"{"archetype":"a","count":0,"period_ticks":10}"#,
        valid: false,
        category: "minmax",
    },
    Vector {
        name: "destructible chunks over max",
        kind: "destructible",
        payload: r#"{"material":"stone","health":10,"chunks":5000}"#,
        valid: false,
        category: "minmax",
    },
];

fn as_object(data: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    data.as_object()
        .ok_or_else(|| "payload must be an object".to_string())
}

fn reject_unknown(obj: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("unknown field `{key}`"));
        }
    }
    Ok(())
}

fn required<'a>(obj: &'a serde_json::Map<String, Value>, field: &str) -> Result<&'a Value, String> {
    obj.get(field)
        .ok_or_else(|| format!("missing required `{field}`"))
}

fn check_asset(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("asset name is empty".to_string());
    }
    if name.len() > MAX_ASSET_NAME_LEN {
        return Err("asset name exceeds 64 bytes".to_string());
    }
    let plain = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'));
    if !plain {
        return Err("asset name allows only 0-9 A-Z a-z - _ . /".to_string());
    }
    Ok(())
}

fn asset_field(obj: &serde_json::Map<String, Value>, field: &str) -> Result<(), String> {
    let value = required(obj, field)?;
    let text = value
        .as_str()
        .ok_or_else(|| format!("`{field}` must be a string"))?;
    check_asset(text)
}

fn bool_field(obj: &serde_json::Map<String, Value>, field: &str) -> Result<bool, String> {
    let value = required(obj, field)?;
    value
        .as_bool()
        .ok_or_else(|| format!("`{field}` must be a bool"))
}

fn f64_field(obj: &serde_json::Map<String, Value>, field: &str) -> Result<f64, String> {
    let value = required(obj, field)?;
    value
        .as_f64()
        .ok_or_else(|| format!("`{field}` must be a number"))
}

fn u64_field(obj: &serde_json::Map<String, Value>, field: &str) -> Result<u64, String> {
    let value = required(obj, field)?;
    value
        .as_u64()
        .ok_or_else(|| format!("`{field}` must be an integer 0 or more"))
}

fn triplet_field(obj: &serde_json::Map<String, Value>, field: &str) -> Result<[f64; 3], String> {
    let value = required(obj, field)?;
    let list = value
        .as_array()
        .ok_or_else(|| format!("`{field}` must be an array of 3 numbers"))?;
    if list.len() != 3 {
        return Err(format!("`{field}` must be an array of 3 numbers"));
    }
    let mut out = [0.0_f64; 3];
    for (index, item) in list.iter().enumerate() {
        out[index] = item
            .as_f64()
            .ok_or_else(|| format!("`{field}` must be an array of 3 numbers"))?;
    }
    Ok(out)
}

fn check_f64_range(
    field: &str,
    value: f64,
    low: f64,
    high: f64,
    range: &str,
) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("`{field}` must be finite"));
    }
    if value < low || value > high {
        return Err(format!("`{field}` is outside {range}"));
    }
    Ok(())
}

fn check_u64_range(field: &str, value: u64, low: u64, high: u64) -> Result<(), String> {
    if value < low || value > high {
        return Err(format!("`{field}` is outside {low}..={high}"));
    }
    Ok(())
}

fn validate_mesh(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["asset", "cast_shadows"])?;
    asset_field(obj, "asset")?;
    if let Some(value) = obj.get("cast_shadows") {
        value
            .as_bool()
            .ok_or_else(|| "`cast_shadows` must be a bool".to_string())?;
    }
    Ok(())
}

fn validate_collider(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["shape", "half", "sensor"])?;
    let shape = required(obj, "shape")?;
    let shape = shape
        .as_str()
        .ok_or_else(|| "`shape` must be a string".to_string())?;
    if !matches!(shape, "box" | "sphere" | "capsule") {
        return Err("`shape` must be box, sphere or capsule".to_string());
    }
    let half = triplet_field(obj, "half")?;
    for value in half {
        check_f64_range(
            "half",
            value,
            0.000_1,
            f64::from(MAX_EXTENT_M),
            "0.0001..=100000",
        )?;
    }
    if let Some(value) = obj.get("sensor") {
        value
            .as_bool()
            .ok_or_else(|| "`sensor` must be a bool".to_string())?;
    }
    Ok(())
}

fn validate_material(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["asset", "tint"])?;
    asset_field(obj, "asset")?;
    if obj.contains_key("tint") {
        let tint = triplet_field(obj, "tint")?;
        for value in tint {
            check_f64_range("tint", value, 0.0, 1.0, "0..=1")?;
        }
    }
    Ok(())
}

fn validate_light(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["kind", "color", "intensity", "range"])?;
    let kind = required(obj, "kind")?;
    let kind = kind
        .as_str()
        .ok_or_else(|| "`kind` must be a string".to_string())?;
    if !matches!(kind, "point" | "spot" | "directional") {
        return Err("`kind` must be point, spot or directional".to_string());
    }
    let color = triplet_field(obj, "color")?;
    for value in color {
        check_f64_range("color", value, 0.0, 1.0, "0..=1")?;
    }
    let intensity = f64_field(obj, "intensity")?;
    check_f64_range("intensity", intensity, 0.0, 1_000_000.0, "0..=1000000")?;
    let range = f64_field(obj, "range")?;
    check_f64_range("range", range, 0.0, f64::from(MAX_EXTENT_M), "0..=100000")?;
    Ok(())
}

fn validate_spawner(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["archetype", "count", "period_ticks"])?;
    asset_field(obj, "archetype")?;
    let count = u64_field(obj, "count")?;
    check_u64_range("count", count, 1, 65_535)?;
    let period = u64_field(obj, "period_ticks")?;
    check_u64_range("period_ticks", period, 1, 4_294_967_295)?;
    Ok(())
}

fn validate_destructible(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["material", "health", "chunks"])?;
    asset_field(obj, "material")?;
    let health = u64_field(obj, "health")?;
    check_u64_range("health", health, 1, 4_294_967_295)?;
    let chunks = u64_field(obj, "chunks")?;
    check_u64_range("chunks", chunks, 1, u64::from(MAX_CHUNKS))?;
    Ok(())
}

fn validate_anatomy(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["template", "limbs"])?;
    asset_field(obj, "template")?;
    let limbs = u64_field(obj, "limbs")?;
    check_u64_range("limbs", limbs, 1, u64::from(MAX_LIMBS))?;
    Ok(())
}

fn validate_rig(data: &Value) -> Result<(), String> {
    let obj = as_object(data)?;
    reject_unknown(obj, &["skeleton", "clips"])?;
    asset_field(obj, "skeleton")?;
    asset_field(obj, "clips")?;
    Ok(())
}

/// Gate one authored payload: the JSON-schema half (types, required, enum,
/// min/max) plus unknown-field refusal. Pure, no IO.
fn gate_validate(kind: &str, data: &Value) -> Result<(), String> {
    match kind {
        "mesh" => validate_mesh(data),
        "collider" => validate_collider(data),
        "material" => validate_material(data),
        "light" => validate_light(data),
        "spawner" => validate_spawner(data),
        "destructible" => validate_destructible(data),
        "anatomy" => validate_anatomy(data),
        "rig" => validate_rig(data),
        _ => Err(format!("unknown component `{kind}`")),
    }
}

fn parse(vector: &Vector) -> Value {
    serde_json::from_str(vector.payload).expect("fixture is fixed valid JSON")
}

#[test]
fn valid_vectors_accept_zero_fp() {
    let mut accepted = 0;
    for vector in VECTORS.iter().filter(|v| v.valid) {
        let data = parse(vector);
        assert!(
            gate_validate(vector.kind, &data).is_ok(),
            "valid `{}` refused",
            vector.name
        );
        accepted += 1;
    }
    assert_eq!(accepted, 20, "fixture carries 20 valid vectors");
}

#[test]
fn invalid_vectors_reject_at_least_18_of_20() {
    let mut rejects = 0;
    let mut total = 0;
    for vector in VECTORS.iter().filter(|v| !v.valid) {
        total += 1;
        let data = parse(vector);
        if gate_validate(vector.kind, &data).is_err() {
            rejects += 1;
        }
    }
    assert_eq!(total, 20, "fixture carries 20 invalid vectors");
    assert!(
        rejects >= 18,
        "gate rejects {rejects}/20 invalid, need 18 or more"
    );
}

#[test]
fn categories_cover_types_required_enum_minmax() {
    for category in ["types", "required", "enum", "minmax"] {
        let count = VECTORS.iter().filter(|v| v.category == category).count();
        assert_eq!(count, 5, "category `{category}` carries 5 vectors");
        for vector in VECTORS.iter().filter(|v| v.category == category) {
            let data = parse(vector);
            assert!(
                gate_validate(vector.kind, &data).is_err(),
                "category `{category}` vector `{}` accepted",
                vector.name
            );
        }
    }
}

#[test]
fn gate_agrees_with_typed_validator_on_ranges() {
    use engine_scene::component::{
        Collider, ColliderShape, Component, Destructible, Light, LightKind, MaterialRef, Spawner,
    };
    fn name(text: &str) -> engine_scene::component::AssetName {
        engine_scene::component::AssetName::try_from(text).expect("test name is valid")
    }
    let good = [
        Component::Collider(Collider {
            shape: ColliderShape::Box,
            half: [1.0, 1.0, 1.0],
            sensor: false,
        }),
        Component::Light(Light {
            kind: LightKind::Point,
            color: [1.0, 1.0, 1.0],
            intensity: 5.0,
            range: 10.0,
        }),
        Component::Spawner(Spawner {
            archetype: name("a"),
            count: 2,
            period_ticks: 10,
        }),
    ];
    for value in good {
        assert!(value.validate().is_ok(), "typed valid refuses");
    }
    let bad_collider = Component::Collider(Collider {
        shape: ColliderShape::Box,
        half: [0.0, 1.0, 1.0],
        sensor: false,
    });
    assert!(bad_collider.validate().is_err());
    let bad_json: Value = serde_json::from_str(r#"{"shape":"box","half":[0.0,1.0,1.0]}"#)
        .expect("fixture is fixed valid JSON");
    assert!(gate_validate("collider", &bad_json).is_err());
    let bad_tint = Component::Material(MaterialRef {
        asset: name("m"),
        tint: [2.0, 0.0, 0.0],
    });
    assert!(bad_tint.validate().is_err());
    let bad_chunks = Component::Destructible(Destructible {
        material: name("stone"),
        health: 10,
        chunks: 5000,
    });
    assert!(bad_chunks.validate().is_err());
    let mesh_obj: Value = serde_json::from_str(r#"{"asset":"a","cast_shadows":true}"#)
        .expect("fixture is fixed valid JSON");
    let obj = as_object(&mesh_obj).expect("object parses");
    assert!(bool_field(obj, "cast_shadows").expect("bool reads"));
}

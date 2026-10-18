//! The three CT40 AI props run the real intake path with the provenance each file carries: licence
//! gate, repair, pose, prop cook, bundle and provenance row. Each stays under the prop budget and
//! cooks byte-identical twice; a model licence nobody can read is refused before any cook.
#![cfg(all(feature = "import", feature = "registry"))]

use engine_assets::gen_intake::{
    intake, IntakeCache, IntakeOutput, IntakeRequest, IntakeSettings, ModelRef, Provenance,
};
use engine_assets::mesh_cook::PROP_MAX_TRIS;
use std::path::{Path, PathBuf};

const PROPS: [&str; 3] = ["ai_amphora", "ai_barrel", "ai_crate"];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/props/ai")
}

/// The text between `"key":"` and the next quote; the provenance files are one compact JSON line.
fn quoted<'a>(json: &'a str, key: &str) -> &'a str {
    let at = json.find(&format!("\"{key}\":\"")).expect(key) + key.len() + 4;
    &json[at..at + json[at..].find('"').expect("closing quote")]
}

/// Every `{"id":..,"license":..}` pair in a span of the file.
fn refs(span: &str) -> Vec<ModelRef> {
    span.split("{\"id\":\"")
        .skip(1)
        .map(|part| ModelRef {
            id: part[..part.find('"').expect("id quote")].to_owned(),
            license: quoted(part, "license").to_owned(),
        })
        .collect()
}

/// The provenance a prop file declares, read back into the intake type.
fn provenance(name: &str) -> Provenance {
    let json = std::fs::read_to_string(dir().join(format!("{name}.provenance.json")))
        .expect("provenance file");
    let after = |key: &str| &json[json.find(key).expect(key) + key.len()..];
    let model = after("\"model\":");
    let components = after("\"components\":[");
    let seed = after("\"seed\":");
    Provenance {
        model: refs(&model[..model.find('}').expect("model end")]).remove(0),
        components: refs(&components[..components.find(']').expect("components end")]),
        source_digest: quoted(&json, "source_digest").to_owned(),
        seed: seed[..seed.find(',').expect("seed end")]
            .parse()
            .expect("seed is a number"),
        conditions: Vec::new(),
        code_ref: quoted(&json, "code_ref").to_owned(),
    }
}

fn cook(
    name: &str,
    prov: &Provenance,
) -> Result<IntakeOutput, engine_assets::gen_intake::IntakeError> {
    let glb = std::fs::read(dir().join(format!("{name}.glb"))).expect("prop glb");
    let request = IntakeRequest {
        name,
        glb: &glb,
        provenance: prov,
        accepted_conditions: &[],
        settings: IntakeSettings::default(),
    };
    intake(&request, &mut IntakeCache::memory())
}

#[test]
fn ai_props_cook_under_budget_twice_identical_with_their_own_provenance() {
    for name in PROPS {
        let prov = provenance(name);
        let first = cook(name, &prov).unwrap_or_else(|e| panic!("{name}: {e}"));
        let second = cook(name, &prov).expect("second cook");
        assert!(
            first.tris > 0 && first.tris <= PROP_MAX_TRIS,
            "{name}: {}",
            first.tris
        );
        assert!(
            first.lod_tris.windows(2).all(|w| w[0] >= w[1]),
            "{name}: {:?}",
            first.lod_tris
        );
        assert!(first.bundle == second.bundle, "{name}: cook x2 differs");
        assert_eq!(first.row, second.row, "{name}");
        for needle in [
            format!("\"id\":\"{}\"", prov.model.id),
            format!("\"source_digest\":\"{}\"", prov.source_digest),
            format!("\"seed\":{}", prov.seed),
        ] {
            assert!(
                first.row.contains(&needle),
                "{name}: {needle} missing in {}",
                first.row
            );
        }
    }
}

#[test]
fn ai_prop_with_an_unreadable_model_licence_is_refused_before_any_cook() {
    let mut prov = provenance("ai_crate");
    for (license, code) in [
        ("see the model card!", "license_not_spdx"),
        ("", "bad_provenance"),
        ("CC-BY-NC-4.0", "license_refused"),
    ] {
        prov.model.license = license.to_owned();
        let err = cook("ai_crate", &prov).expect_err(license);
        assert_eq!(err.code(), code, "{license:?}: {err}");
    }
}

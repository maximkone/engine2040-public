//! CT56 urban-war pilot corner: the three corner actors cook through the CT2 prop lane on the
//! frozen CT9 kit (no new geometry, no engine hunk), match the corner cook receipt triangle counts,
//! stage into one AS1 bundle that finishes to the same bytes twice and loads with every LOD, and
//! a bundle with a flipped byte is refused.
#![cfg(all(feature = "import", feature = "registry"))]

use engine_assets::{
    cook_bundle_bytes, cook_gltf_bytes, verify_bundle_bytes, MeshClass, StagedCooked, PROP_MAX_TRIS,
};
use std::fs;
use std::path::{Path, PathBuf};

fn maps() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/maps")
}

/// `(actor name, asset)` pairs of `corner.json`, in file order.
fn corner_actors() -> Vec<(String, String)> {
    let text = fs::read_to_string(maps().join("urban-war-pilot/corner.json")).expect("corner.json");
    let quoted = |part: &str| part[..part.find('"').expect("closing quote")].to_owned();
    text.split("\"name\": \"")
        .skip(1)
        .map(|part| {
            let asset = part
                .split("\"asset\": \"")
                .nth(1)
                .expect("asset after name");
            (quoted(part), quoted(asset))
        })
        .collect()
}

#[test]
fn pilot_corner_cooks_into_one_as1_bundle_twice_identical_and_corrupt_bytes_are_refused() {
    let actors = corner_actors();
    assert_eq!(actors.len(), 3, "one corner, three actors");
    let receipt =
        fs::read_to_string(maps().join("urban-war-pilot/corner-cook.json")).expect("receipt");
    let stage = || {
        let mut staged = StagedCooked::new();
        for (name, asset) in &actors {
            let bytes = fs::read(maps().join(format!("src/{asset}.gltf"))).expect("kit asset");
            let cooked = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("CT2 cook");
            let tris: u32 = cooked.meshes.iter().map(|m| m.base_tris).sum();
            assert!(
                cooked.meshes.iter().all(|m| m.base_tris <= PROP_MAX_TRIS),
                "{asset}"
            );
            assert!(
                receipt.contains(&format!("\"lod0_tris\": {tris}")),
                "{asset}: {tris} LOD0 tris"
            );
            for (m, mesh) in cooked.meshes.iter().enumerate() {
                for (level, lod) in mesh.lods.iter().enumerate() {
                    staged
                        .meshes
                        .push((format!("{name}_m{m}_lod{level}"), lod.clone()));
                }
            }
        }
        staged
    };
    let staged = stage();
    let first = cook_bundle_bytes(&staged).expect("bundle");
    let second = cook_bundle_bytes(&stage()).expect("bundle again");
    assert!(first == second, "AS1 bundle bytes differ between two cooks");
    let loaded = verify_bundle_bytes(&first, false).expect("AS1 loads the bundle");
    assert_eq!(
        loaded.len(),
        staged.len(),
        "every LOD of every mesh is in the bundle"
    );
    let mut corrupt = first.clone();
    *corrupt.last_mut().expect("bundle has bytes") ^= 0xff;
    assert!(
        verify_bundle_bytes(&corrupt, false).is_err(),
        "a flipped byte is refused"
    );
}

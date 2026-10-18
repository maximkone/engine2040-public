//! CT25 crowd impostors: both spectator specimens cook identical twice through the CT2 prop lane,
//! stay under the prop budget with a whole, falling LOD chain and hulls that hold every LOD, and
//! match the committed `content/crowd/report-split1.json` digests and triangle counts.
#![cfg(feature = "import")]

use engine_assets::{asset_digest, cook_gltf_bytes, MeshClass, PROP_MAX_TRIS};
use std::fs;
use std::path::{Path, PathBuf};

const SPECIMENS: [&str; 2] = ["crowd_impostor_01", "crowd_impostor_02"];

fn crowd() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/crowd")
}

#[test]
fn crowd_impostors_cook_twice_within_the_prop_budget_and_match_the_report() {
    let report = fs::read_to_string(crowd().join("report-split1.json")).expect("report");
    for specimen in SPECIMENS {
        let bytes = fs::read(crowd().join(format!("src/{specimen}.gltf"))).expect("specimen");
        let first = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("cook");
        let second = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("recook");
        assert_eq!(first.digest, second.digest, "{specimen}: cook x2 differs");
        assert_eq!(asset_digest(&first), asset_digest(&second), "{specimen}");
        assert_eq!(first.meshes.len(), 2, "{specimen}: torso and head");
        for (i, mesh) in first.meshes.iter().enumerate() {
            assert!(mesh.base_tris <= PROP_MAX_TRIS, "{specimen} mesh {i}");
            assert_eq!(mesh.lods.len(), 4, "{specimen} mesh {i}: whole LOD chain");
            let tris: Vec<usize> = mesh.lods.iter().map(|l| l.indices.len() / 3).collect();
            assert!(tris.iter().all(|&t| t > 0), "{specimen} mesh {i}: {tris:?}");
            assert!(
                tris.windows(2).all(|w| w[0] >= w[1]),
                "{specimen} mesh {i}: {tris:?}"
            );
            for (lod, hull) in mesh.lods.iter().zip(&mesh.hulls) {
                assert!(
                    hull.contains_mesh(lod),
                    "{specimen} mesh {i}: hull misses a LOD"
                );
            }
            let row = format!(
                "{{\"mesh\":{i},\"base_tris\":{},\"lod_tris\":[{},{},{},{}],\"digest\":\"{:016x}\"}}",
                mesh.base_tris, tris[0], tris[1], tris[2], tris[3], mesh.digest
            );
            assert!(
                report.contains(&row),
                "{specimen}: {row} is not in report-split1.json"
            );
        }
        let asset = format!("\"asset_digest\":\"{:016x}\"", first.digest);
        assert!(
            report.contains(&asset),
            "{specimen}: {asset} is not in report-split1.json"
        );
    }
}

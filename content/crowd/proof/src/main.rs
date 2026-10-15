// CT25 proof binary (splitter pieces 1-4): cooks both spectator impostors
// through the engine's own CT2 lane twice and checks budgets, LOD shape and
// hull containment. No new engine code: only calls landed APIs.
// Exit 0 with report-split1.json on green; exit 1 naming the failure.
// Consumer of crowd_impostor_02: this binary reads src/crowd_impostor_02.gltf
// (UNLOCKS: CT50 soak staging over CT25 + CT44).
use engine_assets::{asset_digest, cook_gltf_bytes, MeshClass, PROP_MAX_TRIS};
use std::path::PathBuf;

fn fail(msg: &str) -> ! {
    eprintln!("CT25-PROOF FAIL: {msg}");
    std::process::exit(1);
}

fn cook_one(root: &std::path::Path, file: &str) -> (String, u32, String) {
    let bytes = std::fs::read(root.join(file))
        .unwrap_or_else(|e| fail(&format!("read {file}: {e}")));
    let a = cook_gltf_bytes(&bytes, MeshClass::Prop)
        .unwrap_or_else(|e| fail(&format!("cook {file} run1: {e:?}")));
    let b = cook_gltf_bytes(&bytes, MeshClass::Prop)
        .unwrap_or_else(|e| fail(&format!("cook {file} run2: {e:?}")));
    if a.digest != b.digest || asset_digest(&a) != asset_digest(&b) {
        fail(&format!("cook {file} not identical twice"));
    }
    if a.meshes.len() != 2 {
        fail(&format!("{file}: want 2 meshes, got {}", a.meshes.len()));
    }
    let mut total_lod0 = 0u32;
    let mut meshes_json = String::from("[");
    for (i, m) in a.meshes.iter().enumerate() {
        if m.base_tris > PROP_MAX_TRIS {
            fail(&format!("{file} mesh {i} over prop budget: {}", m.base_tris));
        }
        total_lod0 += m.base_tris;
        if m.lods.len() != 4 {
            fail(&format!("{file} mesh {i} want 4 LODs, got {}", m.lods.len()));
        }
        let mut prev = u32::MAX;
        let mut tris: Vec<u32> = Vec::new();
        for (l, lod) in m.lods.iter().enumerate() {
            let t = (lod.indices.len() / 3) as u32;
            eprintln!("CT25-PROOF {file} mesh {i} lod {l} tris={t}");
            if t == 0 {
                fail(&format!("{file} mesh {i} lod {l} empty"));
            }
            if t > prev {
                fail(&format!("{file} mesh {i} lod {l} not monotonic: {t} > {prev}"));
            }
            prev = t;
            tris.push(t);
            if !m.hulls[l].contains_mesh(lod) {
                fail(&format!("{file} mesh {i} hull {l} does not contain lod"));
            }
        }
        meshes_json.push_str(&format!(
            "{{\"mesh\":{i},\"base_tris\":{},\"lod_tris\":[{},{},{},{}],\"digest\":\"{:016x}\"}}",
            m.base_tris, tris[0], tris[1], tris[2], tris[3], m.digest
        ));
        if i + 1 < a.meshes.len() {
            meshes_json.push(',');
        }
    }
    meshes_json.push(']');
    let specimen = file.trim_start_matches("src/").trim_end_matches(".gltf");
    let entry = format!(
        "{{\"specimen\":\"{specimen}\",\"file\":\"{file}\",\"class\":\"Prop\",\"prop_budget_tris\":{PROP_MAX_TRIS},\"total_lod0_tris\":{total_lod0},\"identical_twice\":true,\"asset_digest\":\"{:016x}\",\"meshes\":{meshes_json}}}",
        a.digest
    );
    (entry, total_lod0, format!("{:016x}", a.digest))
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let files = ["src/crowd_impostor_01.gltf", "src/crowd_impostor_02.gltf"];
    let mut entries = Vec::new();
    let mut grand = 0u32;
    for f in files {
        let (entry, total, digest) = cook_one(&root, f);
        eprintln!("CT25-PROOF {f} GREEN total_lod0={total} digest={digest}");
        grand += total;
        entries.push(entry);
    }
    let report = format!("{{\"kit\":\"ct25-crowd-kit-split1\",\"identical_twice\":true,\"total_lod0_tris\":{grand},\"specimens\":[{}]}}\n", entries.join(","));
    std::fs::write(root.join("report-split1.json"), &report)
        .unwrap_or_else(|e| fail(&format!("write report: {e}")));
    println!("CT25-PROOF GREEN specimens=2 total_lod0={grand}");
}

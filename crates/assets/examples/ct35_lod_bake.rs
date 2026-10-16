//! ct35_lod_bake: bake the CT35 hero LOD chain through the engine cook.
//!
//! Consumer: `content/fighters/lod/hero.json` feeds the CT41 LOD ms/VRAM table
//! and the game/editor LOD picker (`lod_swap` validates every chain this tool
//! writes). Data owner: `engine-assets`; the cooker (`mesh_cook`) owns the
//! chain contents, this tool only drives it. Units: triangles for counts,
//! bytes for payload sizes. Determinism scope: the same `hero_body.glb` bytes
//! always yield the same JSON (the cook is bit-stable; the tool asserts the
//! recook digest before writing).
//! Errors: any refusal (cook failure, digest drift, growing chain, hull miss)
//! fails loudly with a non-zero exit and writes nothing.

use engine_assets::lod_swap::check_chain;
use engine_assets::mesh_cook::{lod_bytes, tri_count};
use engine_assets::{cook_gltf_bytes, MeshClass};
use std::fmt::Write as _;

/// Usage: `ct35_lod_bake <hero_body.glb> <out hero.json>`.
fn main() {
    match bake(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(report) => println!("{report}"),
        Err(message) => {
            eprintln!("ct35_lod_bake FAIL: {message}");
            std::process::exit(1);
        }
    }
}

/// Cook the input file twice, gate the chain, and write the JSON sidecar.
fn bake(args: &[String]) -> Result<String, String> {
    if args.len() != 2 {
        return Err("want <hero_body.glb> <out hero.json>".to_string());
    }
    let bytes = std::fs::read(&args[0]).map_err(|e| format!("read {}: {e}", args[0]))?;
    let first = cook_gltf_bytes(&bytes, MeshClass::Hero).map_err(|e| format!("cook hero: {e}"))?;
    let second =
        cook_gltf_bytes(&bytes, MeshClass::Hero).map_err(|e| format!("recook hero: {e}"))?;
    if first != second {
        return Err("cook x2 drift: identical bytes cooked differently".to_string());
    }
    if first.meshes.is_empty() {
        return Err("cook holds no meshes".to_string());
    }
    // Every mesh chain must pass the runtime swap gate (monotonic LOD1-3 with
    // one containing hull per LOD); the gate names the defect, never a guess.
    for (index, mesh) in first.meshes.iter().enumerate() {
        check_chain(mesh).map_err(|e| format!("mesh {index}: {e}"))?;
    }
    let json = render_json(&args[0], bytes.len(), &first);
    if let Some(parent) = std::path::Path::new(&args[1]).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(&args[1], &json).map_err(|e| format!("write {}: {e}", args[1]))?;
    Ok(report_line(&first))
}

/// One-line bake receipt for the packet proof: mesh, LOD tri counts, digest.
fn report_line(asset: &engine_assets::CookedAsset) -> String {
    let mut parts = Vec::new();
    for (index, mesh) in asset.meshes.iter().enumerate() {
        let tris: Vec<String> = mesh
            .lods
            .iter()
            .map(|lod| tri_count(lod).to_string())
            .collect();
        parts.push(format!(
            "mesh{index} lod_tris=[{}] digest={:016x}",
            tris.join(","),
            mesh.digest
        ));
    }
    format!(
        "CT35-LOD-BAKE meshes={} {} asset_digest={:016x} gate=check_chain:ok",
        asset.meshes.len(),
        parts.join(" "),
        asset.digest
    )
}

/// Escape one JSON string lane: backslashes first, then quotes.
fn escape_json(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Render the `hero.json` sidecar: source, class budget, and per-LOD numbers.
/// The `consumer` lane names the CT41 table plus the picker that read this
/// file; `gate` records the `check_chain` verdict the bake enforced.
fn render_json(source: &str, source_bytes: usize, asset: &engine_assets::CookedAsset) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{{\n  \"row\": \"CT35\",\n  \"source\": \"{}\",\n  \"source_bytes\": {source_bytes},",
        escape_json(source)
    );
    let _ = writeln!(
        out,
        "  \"class\": \"Hero\",\n  \"class_max_tris\": {},\n  \"cook_x2_identical\": true,",
        MeshClass::Hero.max_tris()
    );
    let _ = writeln!(out, "  \"asset_digest\": \"{:016x}\",", asset.digest);
    let _ = writeln!(
        out,
        "  \"consumer\": \"CT41 LOD ms/VRAM table + lod_swap picker\",\n  \"gate\": \"check_chain:ok\","
    );
    let _ = writeln!(out, "  \"meshes\": [");
    for (index, mesh) in asset.meshes.iter().enumerate() {
        let _ = writeln!(
            out,
            "    {{\"index\": {index}, \"base_tris\": {}, \"digest\": \"{:016x}\", \"lods\": [",
            mesh.base_tris, mesh.digest
        );
        for (level, pair) in mesh.lods.iter().zip(mesh.hulls.iter()).enumerate() {
            let (lod, hull) = pair;
            let _ = writeln!(
                out,
                "      {{\"level\": {level}, \"tris\": {}, \"bytes\": {}, \"hull_contains\": {}, \"hull_support\": {}}}{}",
                tri_count(lod),
                lod_bytes(lod),
                hull.contains_mesh(lod),
                hull.support.len(),
                if level + 1 == mesh.lods.len() { "" } else { "," }
            );
        }
        let _ = writeln!(
            out,
            "    ]}}{}",
            if index + 1 == asset.meshes.len() {
                ""
            } else {
                ","
            }
        );
    }
    let _ = writeln!(out, "  ]\n}}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_assets::lod_swap::LodSwapError;
    use engine_assets::{cook_mesh, CookedMesh, MeshCookOutput};

    /// One quad, two triangles, `+Z` normals, `0..=1` UVs.
    fn quad_mesh() -> CookedMesh {
        CookedMesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uv0: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    #[test]
    fn usage_refuses_bad_arg_counts() {
        assert!(bake(&[]).is_err());
        assert!(bake(&["only-one".to_string()]).is_err());
    }

    #[test]
    fn missing_input_fails_without_writing() {
        let out = format!(
            "{}ct35_missing_{}.json",
            std::env::temp_dir().display(),
            std::process::id()
        );
        assert!(bake(&["no-such-file.glb".to_string(), out.clone()]).is_err());
        assert!(!std::path::Path::new(&out).exists());
    }

    #[test]
    fn cook_x2_identical_with_monotonic_lods_and_containing_hulls() {
        let quad = quad_mesh();
        let first = cook_mesh(&quad, MeshClass::Prop).expect("cook");
        let second = cook_mesh(&quad, MeshClass::Prop).expect("recook");
        assert_eq!(first, second);
        assert_eq!(first.digest, second.digest);
        check_chain(&first).expect("gate");
        let mut prev = tri_count(&first.lods[0]);
        for lod in first.lods.iter().skip(1) {
            let tris = tri_count(lod);
            assert!(tris <= prev, "lods grow down the chain");
            prev = tris;
        }
        for pair in first.lods.iter().zip(first.hulls.iter()) {
            assert!(pair.1.contains_mesh(pair.0), "hull misses its lod");
        }
    }

    #[test]
    fn growing_chain_refused_naming_the_level() {
        let quad = quad_mesh();
        let mut growing = cook_mesh(&quad, MeshClass::Prop).expect("cook");
        growing.lods.reverse();
        assert!(matches!(
            check_chain(&growing),
            Err(LodSwapError::GrowingChain { .. })
        ));
    }

    #[test]
    fn hull_mismatch_and_empty_chains_refuse_with_counts() {
        let quad = quad_mesh();
        let mut short = cook_mesh(&quad, MeshClass::Prop).expect("cook");
        short.hulls.pop();
        let lods = short.lods.len();
        assert_eq!(
            check_chain(&short),
            Err(LodSwapError::HullMismatch {
                lods,
                hulls: lods - 1
            })
        );
        let empty = MeshCookOutput {
            class: MeshClass::Prop,
            base_tris: 0,
            lods: Vec::new(),
            hulls: Vec::new(),
            digest: 0,
        };
        assert_eq!(check_chain(&empty), Err(LodSwapError::EmptyChain));
    }

    #[test]
    fn render_json_escapes_source_and_names_consumer_and_gate() {
        let quad = quad_mesh();
        let chain = cook_mesh(&quad, MeshClass::Prop).expect("cook");
        let asset = engine_assets::CookedAsset {
            meshes: vec![chain],
            chunks: Vec::new(),
            digest: 7,
        };
        let json = render_json("weird\"name\\hero.glb", 3, &asset);
        assert!(
            json.contains("weird\\\"name\\\\hero.glb"),
            "source unescaped"
        );
        assert!(json.contains("\"consumer\": \"CT41 LOD ms/VRAM table + lod_swap picker\""));
        assert!(json.contains("\"gate\": \"check_chain:ok\""));
        assert!(json.contains("\"hull_contains\": true"));
        assert!(json.contains("\"cook_x2_identical\": true"));
    }

    #[test]
    fn report_line_starts_with_the_receipt_tag() {
        let quad = quad_mesh();
        let chain = cook_mesh(&quad, MeshClass::Prop).expect("cook");
        let asset = engine_assets::CookedAsset {
            meshes: vec![chain],
            chunks: Vec::new(),
            digest: 9,
        };
        let line = report_line(&asset);
        assert!(line.starts_with("CT35-LOD-BAKE meshes=1 "), "line: {line}");
    }

    #[test]
    fn invalid_bytes_fail_without_writing() {
        let dir = std::env::temp_dir();
        let input = dir.join(format!("ct35_invalid_{}.glb", std::process::id()));
        let out = dir.join(format!("ct35_invalid_{}.json", std::process::id()));
        std::fs::write(&input, b"not a gltf file").expect("write input");
        let args = vec![
            input.to_string_lossy().into_owned(),
            out.to_string_lossy().into_owned(),
        ];
        assert!(bake(&args).is_err());
        assert!(!out.exists(), "refusal must write nothing");
        let _ = std::fs::remove_file(&input);
    }
}

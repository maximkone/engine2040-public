//! CT7 proof harness: import -> CT1 (retarget cook) -> CT2 (mesh cook) ->
//! texture cook -> AS1 bundle -> bundle load (the WIN1/ED1 path).
//! Prints a receipt per variant; exits nonzero on any failure.
use engine_assets::{
    cook_gltf_bytes, cook_mesh, cook_texture, import_clip, import_gltf_bytes, import_skinned_bytes,
    load_bundle, retarget_cook_with_sidecar, retarget_digest, validate_bind, BindPose, BoneMap,
    BundleWriter, CookConfig, EntryKind, MeshClass, TextureSlot,
};
use engine_anim::contract::Skeleton;
use std::fs;

const CLIPS: [(&str, &str); 14] = [
    ("human_male_idle", "idle"),
    ("human_male_run", "run"),
    ("human_male_jump", "jump"),
    ("human_male_death", "death"),
    ("human_male_idle_combat_main_hand_melee", "idle_combat_1h"),
    ("human_male_idle_combat_unarmed", "idle_combat_unarmed"),
    ("human_male_cast_main_hand_melee", "attack_1h_1"),
    ("human_male_cast_main_hand_melee_2", "attack_1h_2"),
    ("human_male_cast_main_hand_melee_3", "attack_1h_3"),
    ("human_male_cast_two_handed_melee", "attack_2h_1"),
    ("human_male_cast_two_handed_melee_2", "attack_2h_2"),
    ("human_male_cast_off_hand_shield", "block_shield"),
    ("human_male_cast_dual_wield_melee", "attack_dual_1"),
    ("human_male_power_up", "taunt"),
];

const VARIANTS: [(&str, MeshClass); 4] = [
    ("hero", MeshClass::Hero),
    ("opp_male_light", MeshClass::Opponent),
    ("opp_female_medium", MeshClass::Opponent),
    ("opp_female_light", MeshClass::Opponent),
];

fn fail(variant: &str, msg: String) -> ! {
    eprintln!("CT7-FAIL {variant}: {msg}");
    std::process::exit(1);
}

/// Parent-first BFS node order, mirroring the engine's skeleton_order.
fn bfs_order(doc: &gltf::Document) -> Vec<usize> {
    let count = doc.nodes().len();
    let mut parent: Vec<Option<usize>> = vec![None; count];
    for node in doc.nodes() {
        for child in node.children() {
            if parent[child.index()].is_none() {
                parent[child.index()] = Some(node.index());
            }
        }
    }
    let mut seen = vec![false; count];
    let mut order = Vec::with_capacity(count);
    let mut queue: Vec<usize> = (0..count).filter(|i| parent[*i].is_none()).collect();
    let mut head = 0;
    while head < queue.len() {
        let cur = queue[head];
        head += 1;
        if seen[cur] {
            continue;
        }
        seen[cur] = true;
        order.push(cur);
        if let Some(node) = doc.nodes().nth(cur) {
            for child in node.children() {
                if !seen[child.index()] {
                    queue.push(child.index());
                }
            }
        }
    }
    for (i, was) in seen.iter().enumerate() {
        if !was {
            order.push(i);
        }
    }
    order
}

fn sanitize(raw: &str, index: usize) -> String {
    let mut base = String::new();
    for c in raw.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
            base.push(c);
        } else {
            base.push('_');
        }
    }
    if base.is_empty() {
        base = format!("joint_{index}");
    } else if base.as_bytes()[0].is_ascii_digit() {
        base.insert(0, '_');
    }
    if base.len() > 64 {
        base.truncate(64);
    }
    base
}

fn joint_idx(skel: &Skeleton, want: &str, variant: &str) -> usize {
    skel.joints
        .iter()
        .position(|j| j.name == want)
        .unwrap_or_else(|| fail(variant, format!("joint {want} missing in skeleton")))
}

fn main() {
    let lane = std::env::args().nth(1).expect("lane fighters dir arg");
    let mut hero_names: Option<Vec<String>> = None;
    for (variant, class) in VARIANTS {
        let body_path = format!("{lane}/glb/{variant}_body.glb");
        let body = fs::read(&body_path)
            .unwrap_or_else(|e| fail(variant, format!("read body: {e}")));
        let imported = import_gltf_bytes(&body)
            .unwrap_or_else(|e| fail(variant, format!("mesh import: {e:?}")));
        // One mesh primitive per quadrant material (glTF primitive<->material
        // is 1:1); the character budget counts materials and total tris.
        if imported.meshes.len() != 4 || imported.materials.len() != 4 {
            fail(
                variant,
                format!(
                    "want 4 meshes + 4 materials, got {} + {}",
                    imported.meshes.len(),
                    imported.materials.len()
                ),
            );
        }
        if imported.materials.len() > 4 {
            fail(variant, "material budget exceeded".to_string());
        }
        let skinned = import_skinned_bytes(&body)
            .unwrap_or_else(|e| fail(variant, format!("skin import: {e:?}")));
        let skel = &skinned.skeleton;
        if skinned.skins.len() != imported.meshes.len() {
            fail(
                variant,
                format!(
                    "want {} skins, got {}",
                    imported.meshes.len(),
                    skinned.skins.len()
                ),
            );
        }
        let names: Vec<String> = skel.joints.iter().map(|j| j.name.clone()).collect();
        match &hero_names {
            None => hero_names = Some(names.clone()),
            Some(h) => {
                if *h != names {
                    fail(variant, "skeleton differs from hero skeleton".to_string());
                }
            }
        }
        // Bind pose from the same node order the engine used.
        let (doc, buffers, _) =
            gltf::import(&body_path).unwrap_or_else(|e| fail(variant, format!("gltf parse: {e}")));
        let order = bfs_order(&doc);
        let mut seen_names = std::collections::BTreeSet::new();
        let mut ordered_names = Vec::with_capacity(order.len());
        for node_index in &order {
            let node = doc.nodes().nth(*node_index).unwrap();
            let mut nm = sanitize(node.name().unwrap_or(""), *node_index);
            let mut n = 2u32;
            while !seen_names.insert(nm.clone()) {
                nm = format!("{nm}_{n}");
                n += 1;
            }
            ordered_names.push(nm);
        }
        if ordered_names != names {
            fail(variant, "harness node order disagrees with engine skeleton".to_string());
        }
        let mut positions = Vec::with_capacity(order.len());
        let mut rotations = Vec::with_capacity(order.len());
        for node_index in &order {
            let node = doc.nodes().nth(*node_index).unwrap();
            let (t, r, s) = node.transform().decomposed();
            if (s[0] - 1.0).abs() > 1e-3 || (s[1] - 1.0).abs() > 1e-3 || (s[2] - 1.0).abs() > 1e-3 {
                fail(variant, format!("non-unit scale on node {}", node.index()));
            }
            positions.push(t);
            rotations.push(r);
        }
        let bind = BindPose { positions, rotations };
        if let Err(e) = validate_bind(skel, &bind) {
            fail(variant, format!("bind invalid: {e:?}"));
        }
        let pairs: Vec<(String, String)> =
            names.iter().map(|n| (n.clone(), n.clone())).collect();
        let pair_refs: Vec<(&str, &str)> =
            pairs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let map = BoneMap::build(skel, skel, &pair_refs)
            .unwrap_or_else(|e| fail(variant, format!("bonemap: {e:?}")));
        let config = CookConfig {
            root_joint: joint_idx(skel, "pelvis", variant),
            left_foot: joint_idx(skel, "foot_l", variant),
            right_foot: joint_idx(skel, "foot_r", variant),
            ground_y: 0.02,
            start_tick: 0,
        };
        // CT1 over every clip, cooked twice.
        let mut cooked_clips = Vec::new();
        for (action, short) in CLIPS {
            let clip_path = format!("{lane}/glb/clips/{action}.glb");
            let (cdoc, cbuffers, _) = gltf::import(&clip_path)
                .unwrap_or_else(|e| fail(variant, format!("clip {short} parse: {e}")));
            let clips = import_clip(&cdoc, &cbuffers, skel)
                .unwrap_or_else(|e| fail(variant, format!("clip {short} import: {e:?}")));
            if clips.len() != 1 {
                fail(variant, format!("clip {short}: want 1 animation, got {}", clips.len()));
            }
            let sidecar_path = format!("{lane}/sidecars/{short}.json");
            let sidecar = fs::read_to_string(&sidecar_path)
                .unwrap_or_else(|e| fail(variant, format!("sidecar {short}: {e}")));
            let plan = engine_assets::RetargetPlan {
                source_clip: &clips[0],
                source: skel,
                source_bind: &bind,
                target: skel,
                target_bind: &bind,
                map: &map,
                config: &config,
            };
            let r1 = retarget_cook_with_sidecar(&plan, &sidecar)
                .unwrap_or_else(|e| fail(variant, format!("CT1 {short}: {e:?}")));
            let r2 = retarget_cook_with_sidecar(&plan, &sidecar)
                .unwrap_or_else(|e| fail(variant, format!("CT1 {short} rerun: {e:?}")));
            if r1 != r2 {
                fail(variant, format!("CT1 {short} not identical twice"));
            }
            println!(
                "CT7-CLIP {variant} {short} dur={:.3}s keys={} digest={:016x} loop={} root={}",
                r1.clip.duration,
                r1.clip.positions.tracks.iter().map(|t| t.times.len()).sum::<usize>(),
                retarget_digest(&r1),
                r1.tags.looped,
                r1.tags.root_motion,
            );
            cooked_clips.push((short, r1));
        }
        // CT2 mesh cook per primitive, twice each; the class budget
        // applies to the summed LOD0 triangles.
        let mut cooked_meshes = Vec::new();
        let mut total_tris = 0u32;
        for (pi, mesh) in imported.meshes.iter().enumerate() {
            let m1 = cook_mesh(mesh, class).unwrap_or_else(|e| {
                fail(variant, format!("CT2 prim {pi}: {e:?}"))
            });
            let m2 = cook_mesh(mesh, class).unwrap_or_else(|e| {
                fail(variant, format!("CT2 prim {pi} rerun: {e:?}"))
            });
            if m1 != m2 {
                fail(variant, format!("CT2 prim {pi} not identical twice"));
            }
            total_tris += m1.base_tris;
            cooked_meshes.push(m1);
        }
        let max = match class {
            MeshClass::Hero => engine_assets::HERO_MAX_TRIS,
            MeshClass::Opponent => engine_assets::OPPONENT_MAX_TRIS,
            MeshClass::Prop => engine_assets::PROP_MAX_TRIS,
        };
        if total_tris > max {
            fail(variant, format!("over budget: {total_tris} > {max}"));
        }
        let g1 = cook_gltf_bytes(&body, class)
            .unwrap_or_else(|e| fail(variant, format!("CT2 file: {e:?}")));
        let g2 = cook_gltf_bytes(&body, class)
            .unwrap_or_else(|e| fail(variant, format!("CT2 file rerun: {e:?}")));
        if g1 != g2 {
            fail(variant, "CT2 file not identical twice".to_string());
        }
        let lod_tris: Vec<Vec<u32>> = cooked_meshes
            .iter()
            .map(|m| {
                m.lods.iter().map(|l| (l.indices.len() / 3) as u32).collect()
            })
            .collect();
        let m1 = &cooked_meshes[0];
        // Texture cook (one 1024 quadrant PNG per material), twice each.
        let mut cooked_tex = Vec::new();
        for qi in 0..4 {
            let atlas = fs::read(format!("{lane}/atlas/{variant}_atlas_q{qi}.png"))
                .unwrap_or_else(|e| fail(variant, format!("atlas q{qi} read: {e}")));
            let t1 = cook_texture(&atlas, TextureSlot::BaseColor)
                .unwrap_or_else(|e| fail(variant, format!("texture cook q{qi}: {e:?}")));
            let t2 = cook_texture(&atlas, TextureSlot::BaseColor)
                .unwrap_or_else(|e| fail(variant, format!("texture cook q{qi} rerun: {e:?}")));
            if t1 != t2 {
                fail(variant, format!("texture cook q{qi} not identical twice"));
            }
            if t1.width != 1024 || t1.height != 1024 {
                fail(variant, format!("quadrant q{qi} not 1024x1024"));
            }
            cooked_tex.push(t1);
        }
        let t1 = &cooked_tex[0];
        // AS1 bundle (the WIN1/ED1 load path), finished twice.
        let mut writer = BundleWriter::new();
        for (pi, m) in cooked_meshes.iter().enumerate() {
            for (i, lod) in m.lods.iter().enumerate() {
                writer
                    .add_mesh(&format!("{variant}_p{pi}_lod{i}"), lod)
                    .unwrap_or_else(|e| fail(variant, format!("bundle mesh: {e}")));
            }
        }
        for (qi, mat) in imported.materials.iter().enumerate() {
            writer
                .add_material(&format!("{variant}_mat_q{qi}"), mat)
                .unwrap_or_else(|e| fail(variant, format!("bundle material: {e}")));
        }
        for (qi, tex) in cooked_tex.iter().enumerate() {
            writer
                .add_texture(&format!("{variant}_atlas_q{qi}"), tex)
                .unwrap_or_else(|e| fail(variant, format!("bundle texture: {e}")));
        }
        for (pi, prim) in skinned.skins.iter().enumerate() {
            writer
                .add_skin(&format!("{variant}_skin_p{pi}"), &prim.skin)
                .unwrap_or_else(|e| fail(variant, format!("bundle skin: {e}")));
        }
        for (short, rc) in &cooked_clips {
            writer
                .add_clip(&format!("{variant}_{short}"), &rc.clip)
                .unwrap_or_else(|e| fail(variant, format!("bundle clip: {e}")));
        }
        let entry_count = writer.len();
        let bytes_a = writer.finish();
        let loaded = load_bundle(&bytes_a)
            .unwrap_or_else(|e| fail(variant, format!("bundle load: {e}")));
        for entry in &loaded.entries {
            match entry.kind {
                EntryKind::Mesh => {
                    entry.decode_mesh().unwrap_or_else(|e| fail(variant, format!("decode: {e}")));
                }
                EntryKind::Material => {
                    entry.decode_material().unwrap_or_else(|e| fail(variant, format!("decode: {e}")));
                }
                EntryKind::Texture => {
                    entry.decode_texture().unwrap_or_else(|e| fail(variant, format!("decode: {e}")));
                }
                EntryKind::Skin => {
                    entry.decode_skin().unwrap_or_else(|e| fail(variant, format!("decode: {e}")));
                }
                EntryKind::Clip => {
                    entry.decode_clip().unwrap_or_else(|e| fail(variant, format!("decode: {e}")));
                }
            }
        }
        // Rebuild + refinish to prove byte-identical bundles.
        let mut writer2 = BundleWriter::new();
        for (pi, m) in cooked_meshes.iter().enumerate() {
            for (i, lod) in m.lods.iter().enumerate() {
                writer2.add_mesh(&format!("{variant}_p{pi}_lod{i}"), lod).unwrap();
            }
        }
        for (qi, mat) in imported.materials.iter().enumerate() {
            writer2.add_material(&format!("{variant}_mat_q{qi}"), mat).unwrap();
        }
        for (qi, tex) in cooked_tex.iter().enumerate() {
            writer2.add_texture(&format!("{variant}_atlas_q{qi}"), tex).unwrap();
        }
        for (pi, prim) in skinned.skins.iter().enumerate() {
            writer2.add_skin(&format!("{variant}_skin_p{pi}"), &prim.skin).unwrap();
        }
        for (short, rc) in &cooked_clips {
            writer2.add_clip(&format!("{variant}_{short}"), &rc.clip).unwrap();
        }
        let bytes_b = writer2.finish();
        if bytes_a != bytes_b {
            fail(variant, "bundle not identical twice".to_string());
        }
        let bundle_path = format!("{lane}/bundle/{variant}.e2040");
        fs::create_dir_all(format!("{lane}/bundle")).ok();
        fs::write(&bundle_path, &bytes_a)
            .unwrap_or_else(|e| fail(variant, format!("bundle write: {e}")));
        let skin_rows: usize = skinned.skins.iter().map(|s| s.skin.joints.len()).sum();
        println!(
            "CT7-BODY {variant} joints={} tris_lod0={total_tris} lods={lod_tris:?} prims={} skin_rows={} tex={}x{}mips={} clips={} entries={} bundle={}B load_ms={}",
            skel.len(),
            cooked_meshes.len(),
            skin_rows,
            t1.width,
            t1.height,
            t1.mips.len(),
            cooked_clips.len(),
            entry_count,
            bytes_a.len(),
            loaded.load_time_ms,
        );
    }
    println!("CT7-OK");
}

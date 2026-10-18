use super::*;
use crate::bundle_cook::verify_bundle_bytes;
use std::sync::atomic::{AtomicU32, Ordering};

fn ref_of(id: &str, license: &str) -> ModelRef {
    ModelRef {
        id: id.to_owned(),
        license: license.to_owned(),
    }
}

fn provenance() -> Provenance {
    Provenance {
        model: ref_of("VAST-AI/TripoSG", "MIT"),
        components: vec![
            ref_of("ZhengPeng7/BiRefNet", "MIT"),
            ref_of("facebook/dinov2-large", "Apache-2.0"),
        ],
        source_digest: digest_text("a wooden barrel with iron bands"),
        seed: 4243,
        conditions: Vec::new(),
        code_ref: "github.com/VAST-AI-Research/TripoSG@fc5c409".to_owned(),
    }
}

fn request<'a>(glb: &'a [u8], prov: &'a Provenance, accepted: &'a [String]) -> IntakeRequest<'a> {
    IntakeRequest {
        name: "barrel",
        glb,
        provenance: prov,
        accepted_conditions: accepted,
        settings: IntakeSettings::default(),
    }
}

/// An icosphere with `4^level * 20` triangles, deterministic.
fn icosphere(level: u32) -> (Vec<[f32; 3]>, Vec<u32>) {
    let t = (1.0f32 + 5.0f32.sqrt()) / 2.0;
    let mut pos: Vec<[f32; 3]> = vec![
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ];
    let mut tris: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for p in &mut pos {
        let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        *p = [p[0] / len, p[1] / len, p[2] / len];
    }
    for _ in 0..level {
        let mut mids: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        let mut next = Vec::new();
        let mut mid = |pos: &mut Vec<[f32; 3]>, a: u32, b: u32| -> u32 {
            let key = (a.min(b), a.max(b));
            *mids.entry(key).or_insert_with(|| {
                let (pa, pb) = (pos[a as usize], pos[b as usize]);
                let m = [pa[0] + pb[0], pa[1] + pb[1], pa[2] + pb[2]];
                let len = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
                pos.push([m[0] / len, m[1] / len, m[2] / len]);
                u32::try_from(pos.len() - 1).unwrap_or(0)
            })
        };
        for tri in &tris {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let (ab, bc, ca) = (
                mid(&mut pos, a, b),
                mid(&mut pos, b, c),
                mid(&mut pos, c, a),
            );
            next.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        tris = next;
    }
    (pos, tris.concat())
}

/// A GLB (positions and indices only, like a marching-cubes export).
fn glb_of(pos: &[[f32; 3]], idx: &[u32]) -> Vec<u8> {
    let mut bin = Vec::new();
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in pos {
        for k in 0..3 {
            bin.extend_from_slice(&p[k].to_le_bytes());
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let pos_len = bin.len();
    for i in idx {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let idx_len = bin.len() - pos_len;
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let json = format!(
        "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"mesh\":0}}],\
\"meshes\":[{{\"primitives\":[{{\"attributes\":{{\"POSITION\":0}},\"indices\":1}}]}}],\
\"buffers\":[{{\"byteLength\":{}}}],\
\"bufferViews\":[{{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{pos_len},\"target\":34962}},{{\"buffer\":0,\"byteOffset\":{pos_len},\"byteLength\":{idx_len},\"target\":34963}}],\
\"accessors\":[{{\"bufferView\":0,\"componentType\":5126,\"count\":{},\"type\":\"VEC3\",\"min\":[{:?},{:?},{:?}],\"max\":[{:?},{:?},{:?}]}},\
{{\"bufferView\":1,\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}]}}",
        bin.len(),
        pos.len(),
        lo[0], lo[1], lo[2], hi[0], hi[1], hi[2],
        idx.len()
    );
    let mut json = json.into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(total).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&u32::try_from(json.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(&u32::try_from(bin.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&bin);
    out
}

/// A generated-looking prop: a 20,480-triangle sphere stretched into a barrel,
/// one duplicate triangle, one degenerate triangle, one floating speck.
fn generated_glb(z_up: bool) -> Vec<u8> {
    let (mut pos, mut idx) = icosphere(5);
    for p in &mut pos {
        *p = [p[0] * 0.6, p[1] * 1.0, p[2] * 0.6];
    }
    let first = [idx[0], idx[1], idx[2]];
    idx.extend_from_slice(&first); // duplicate
    idx.extend_from_slice(&[idx[0], idx[0], idx[1]]); // degenerate
    let base = u32::try_from(pos.len()).unwrap_or(0);
    pos.extend_from_slice(&[
        [5.0, 5.0, 5.0],
        [5.1, 5.0, 5.0],
        [5.0, 5.1, 5.0],
        [5.0, 5.0, 5.1],
    ]);
    idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    if z_up {
        for p in &mut pos {
            *p = [p[0], -p[2], p[1]]; // y-up barrel lying so z is its long axis
        }
    }
    glb_of(&pos, &idx)
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gen_intake_{tag}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn spdx_gate_allows_the_list_and_refuses_the_rest() {
    for ok in [
        "MIT",
        "Apache-2.0",
        "mit",
        "MIT OR Apache-2.0",
        "(MIT AND BSD-3-Clause)",
        "MIT OR GPL-3.0-only",
        "Apache-2.0 WITH LLVM-exception",
    ] {
        assert!(check_license("m", ok).is_ok(), "{ok}");
    }
    for bad in [
        "CC-BY-NC-4.0",
        "MIT AND CC-BY-NC-4.0",
        "GPL-3.0-only",
        "BRIA license",
        "MIT (SD 1.5 base: CreativeML OpenRAIL-M)",
        "tencent-hunyuan-community",
    ] {
        let err = check_license("m", bad).expect_err(bad);
        assert_eq!(err.code(), "license_refused", "{bad}: {err}");
    }
    let nc = check_license("m", "CC-BY-NC-4.0").expect_err("nc");
    assert!(nc.to_string().contains("non-commercial"), "{nc}");
    assert_eq!(
        check_license("m", "see the card!")
            .expect_err("prose")
            .code(),
        "license_not_spdx"
    );
    assert_eq!(
        check_license("m", "  ").expect_err("empty").code(),
        "license_not_spdx"
    );
}

#[test]
fn restricted_models_are_refused_whatever_the_card_says() {
    let mut p = provenance();
    p.model = ref_of("Tencent-Hunyuan/Hunyuan3D-2.1", "MIT");
    assert_eq!(
        check_provenance(&p, &[]).expect_err("hunyuan").code(),
        "model_restricted"
    );
    let mut p = provenance();
    p.components.push(ref_of("briaai/RMBG-1.4", "MIT"));
    let err = check_provenance(&p, &[]).expect_err("rmbg");
    assert_eq!(err.code(), "model_restricted");
    assert!(err.to_string().contains("non-commercial"), "{err}");
    let mut p = provenance();
    p.components.push(ref_of("diso", "MIT"));
    assert_eq!(
        check_provenance(&p, &[]).expect_err("diso").code(),
        "model_restricted"
    );
    let mut p = provenance();
    p.components.push(ref_of("some/segmenter", "CC-BY-NC-4.0"));
    assert_eq!(
        check_provenance(&p, &[]).expect_err("nc part").code(),
        "license_refused"
    );
    assert!(check_provenance(&provenance(), &[]).is_ok());
}

#[test]
fn provenance_needs_a_digest_and_declared_conditions_need_accepting() {
    let mut p = provenance();
    p.source_digest = "not hex".to_owned();
    assert_eq!(
        check_provenance(&p, &[]).expect_err("digest").code(),
        "bad_provenance"
    );
    let mut p = provenance();
    p.model.id.clear();
    assert_eq!(
        check_provenance(&p, &[]).expect_err("id").code(),
        "bad_provenance"
    );
    let mut p = provenance();
    p.conditions = vec!["tencent-hunyuan-community: eu-excluded".to_owned()];
    match check_provenance(&p, &[]).expect_err("unaccepted") {
        IntakeError::ConditionsUnaccepted { missing } => assert_eq!(missing, p.conditions),
        other => panic!("want ConditionsUnaccepted, got {other}"),
    }
    assert!(check_provenance(&p, &p.conditions).is_ok());
}

#[test]
fn a_refused_model_costs_no_cook() {
    let mut p = provenance();
    p.components
        .push(ref_of("briaai/RMBG-1.4", "bria-rmbg-1.4"));
    let mut cache = IntakeCache::memory();
    let err =
        intake(&request(b"this is not a glb", &p, &[]), &mut cache).expect_err("refused first");
    assert_eq!(err.code(), "model_restricted");
    assert!(cache.is_empty());
}

#[test]
fn a_generated_prop_cooks_to_five_thousand_triangles_with_a_hull_and_a_row() {
    let glb = generated_glb(false);
    let p = provenance();
    let mut cache = IntakeCache::memory();
    let out = intake(&request(&glb, &p, &[]), &mut cache).expect("intake");
    assert!(!out.cache_hit);
    assert!(out.tris <= PROP_MAX_TRIS, "tris {}", out.tris);
    assert!(out.tris > 1500, "a barrel keeps its shape: {}", out.tris);
    assert_eq!(
        out.tris_in, 20_480,
        "repair removed the duplicate, the degenerate and the speck"
    );
    assert_eq!(out.repair.duplicate_tris, 1);
    assert_eq!(out.repair.degenerate_tris, 1);
    assert_eq!((out.repair.speck_tris, out.repair.specks), (2, 1));
    assert!(
        out.lod_tris.windows(2).all(|w| w[0] >= w[1]),
        "{:?}",
        out.lod_tris
    );
    assert_eq!(out.lod_tris[0], out.tris);
    // The bundle verifies and holds every LOD plus the material.
    let loaded = verify_bundle_bytes(&out.bundle, false).expect("bundle verifies");
    assert_eq!(loaded.len(), out.lod_tris.len() + 1);
    assert_eq!(out.bundle_hash, content_digest(&out.bundle));
    // Posed: longest side 1 m, centred, standing on the floor, and the hull holds it.
    let h = &out.hull;
    let size = (0..3).map(|k| h.max[k] - h.min[k]).fold(0.0f32, f32::max);
    assert!((size - 1.0).abs() < 1e-4, "size {size}");
    assert!(h.min[1].abs() < 1e-5, "base on the floor: {}", h.min[1]);
    assert!(
        ((h.min[0] + h.max[0]) * 0.5).abs() < 1e-4 && ((h.min[2] + h.max[2]) * 0.5).abs() < 1e-4
    );
    assert!(h.radius > 0.4 && !h.support.is_empty());
    // The provenance row names model, licence, digest, seed.
    for needle in [
        "\"model\":{\"id\":\"VAST-AI/TripoSG\",\"license\":\"MIT\"}",
        "\"seed\":4243",
        &format!("\"source_digest\":\"{}\"", p.source_digest),
        &format!("\"bundle\":\"{}\"", out.bundle_hash),
        "\"id\":\"ZhengPeng7/BiRefNet\"",
        "\"textured\":false",
    ] {
        assert!(out.row.contains(needle), "{needle} missing in {}", out.row);
    }
    assert_eq!(out.row.matches('{').count(), out.row.matches('}').count());
    assert!(!out.row.contains('\n'));
}

#[test]
fn cook_twice_is_byte_identical_and_the_cache_returns_the_same_hash() {
    let glb = generated_glb(false);
    let p = provenance();
    let a = intake(&request(&glb, &p, &[]), &mut IntakeCache::memory()).expect("first");
    let b =
        intake(&request(&glb, &p, &[]), &mut IntakeCache::memory()).expect("second, fresh cache");
    assert_eq!(a.bundle, b.bundle, "cook x2 byte-identical");
    assert_eq!(a.row, b.row);
    assert_eq!(a.bundle_hash, b.bundle_hash);
    let mut cache = IntakeCache::memory();
    let first = intake(&request(&glb, &p, &[]), &mut cache).expect("miss");
    let again = intake(&request(&glb, &p, &[]), &mut cache).expect("hit");
    assert!(!first.cache_hit && again.cache_hit);
    assert_eq!(first.bundle_hash, again.bundle_hash);
    // Without the GLB: the generation inputs alone find it.
    let found = cache
        .lookup(&request(&[], &p, &[]))
        .expect("lookup by inputs");
    assert_eq!(found.bundle_hash, first.bundle_hash);
    // A different seed or model licence is a different key.
    let mut other = p.clone();
    other.seed = 4244;
    assert!(cache.lookup(&request(&[], &other, &[])).is_none());
    // The same inputs but a changed GLB must not return the old bundle.
    let (mut pos, idx) = icosphere(5);
    pos[0] = [pos[0][0] * 1.2, pos[0][1], pos[0][2]];
    let different = glb_of(&pos, &idx);
    let fresh = intake(&request(&different, &p, &[]), &mut cache).expect("changed glb recooks");
    assert!(!fresh.cache_hit);
    assert_ne!(fresh.bundle_hash, first.bundle_hash);
}

#[test]
fn the_folder_cache_survives_a_new_process() {
    let dir = temp_dir("disk");
    let glb = generated_glb(false);
    let p = provenance();
    let made = {
        let mut cache = IntakeCache::open(&dir).expect("open");
        intake(&request(&glb, &p, &[]), &mut cache).expect("cook")
    };
    let cache = IntakeCache::open(&dir).expect("reopen");
    assert!(cache.is_empty(), "nothing in memory yet");
    let hit = cache.lookup(&request(&[], &p, &[])).expect("hit from disk");
    assert!(hit.cache_hit);
    assert_eq!(hit.bundle_hash, made.bundle_hash);
    assert_eq!(hit.bundle, made.bundle);
    assert_eq!(hit.row, made.row);
    assert_eq!(
        (hit.tris, hit.tris_in, &hit.lod_tris),
        (made.tris, made.tris_in, &made.lod_tris)
    );
    assert_eq!(hit.hull, made.hull);
    assert_eq!(hit.cook_digest, made.cook_digest);
    // A bundle damaged on disk is a miss, never a wrong answer.
    let key = cache_key(&request(&[], &p, &[]));
    std::fs::write(dir.join(&key).join("bundle"), b"damaged").expect("damage");
    assert!(IntakeCache::open(&dir)
        .expect("reopen")
        .lookup(&request(&[], &p, &[]))
        .is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cook_full_hash_a_kill_between_files_leaves_exactly_one_entry_to_redo() {
    let dir = temp_dir("kill");
    let glb = generated_glb(false);
    let (p, mut other) = (provenance(), provenance());
    other.seed += 1;
    let (keep, kill) = (request(&glb, &p, &[]), request(&glb, &other, &[]));
    let mut cache = IntakeCache::open(&dir).expect("open");
    intake(&keep, &mut cache).expect("cook");
    let made = intake(&kill, &mut cache).expect("cook");
    // A process killed after the first file of the second entry: its staging
    // folder holds the bundle alone and nothing was committed.
    let (entry, staging) = (
        dir.join(&made.cache_key),
        dir.join(format!("{}.tmp", made.cache_key)),
    );
    std::fs::remove_dir_all(&entry).expect("uncommit");
    std::fs::create_dir(&staging).expect("staging");
    std::fs::write(staging.join("bundle"), &made.bundle).expect("file 1");

    let cache = IntakeCache::open(&dir).expect("reopen");
    assert!(
        cache.lookup(&keep).is_some(),
        "the finished entry stays a hit"
    );
    assert!(
        cache.lookup(&kill).is_none(),
        "the half-written entry is a miss"
    );
    let mut cache = cache;
    let redone = intake(&kill, &mut cache).expect("redo");
    assert!(!redone.cache_hit, "exactly that entry is cooked again");
    assert_eq!(redone.bundle, made.bundle, "bundle bytes x2");
    assert!(
        !staging.exists(),
        "the commit rename consumed the staging folder"
    );
    assert!(IntakeCache::open(&dir)
        .expect("reopen")
        .lookup(&kill)
        .is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn z_up_meshes_turn_to_y_up_and_settings_are_bounded() {
    let p = provenance();
    let mut req = request(&[], &p, &[]);
    let glb = generated_glb(true);
    req.glb = &glb;
    req.settings.up_axis = UpAxis::Z;
    req.settings.size_mm = 2000;
    let out = intake(&req, &mut IntakeCache::memory()).expect("z-up intake");
    let h = &out.hull;
    let ext = |k: usize| h.max[k] - h.min[k];
    assert!(
        (ext(1) - 2.0).abs() < 1e-4,
        "the long axis is up and 2 m: {}",
        ext(1)
    );
    assert!(h.min[1].abs() < 1e-5);
    for bad in [
        IntakeSettings {
            size_mm: 1,
            ..IntakeSettings::default()
        },
        IntakeSettings {
            max_tris: PROP_MAX_TRIS + 1,
            ..IntakeSettings::default()
        },
    ] {
        let mut r = request(&glb, &p, &[]);
        r.settings = bad;
        assert_eq!(
            intake(&r, &mut IntakeCache::memory())
                .expect_err("bounds")
                .code(),
            "bad_settings"
        );
    }
    let mut r = request(&glb, &p, &[]);
    r.name = "Bad Name";
    assert_eq!(
        intake(&r, &mut IntakeCache::memory())
            .expect_err("name")
            .code(),
        "bad_settings"
    );
}

#[test]
fn bad_files_and_empty_meshes_fail_typed() {
    let p = provenance();
    let err = intake(
        &request(b"not a glb at all", &p, &[]),
        &mut IntakeCache::memory(),
    )
    .expect_err("garbage");
    assert_eq!(err.code(), "import_failed");
    let flat = glb_of(
        &[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
        &[0, 1, 2],
    );
    let err = intake(&request(&flat, &p, &[]), &mut IntakeCache::memory()).expect_err("flat");
    assert_eq!(err.code(), "nothing_left", "{err}");
}

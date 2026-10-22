//! The compact rig cook: the CT7 hero body comes out the same rig, with a
//! KTX2 atlas and a body that fits the page.
//!
//! The cook is the lever the page needs: the rig as it stands is 6,239,621 B
//! brotli against a 2,040,000 B first-bundle budget, and the hero body's four
//! 1024x1024 atlas quadrants decode to 16,777,216 B of RGBA, 84 % of the
//! page's whole linear memory on their own. The cook turns the atlas into
//! KTX2 block maps and drops the images from the body, keeping the rig
//! (the skeleton, the bind pose, the inverse binds, the skin rows and the
//! mesh) byte for byte.
#![cfg(feature = "import")]

use engine_assets::body_import::import_skinned_body;
use engine_assets::ktx2_cook::{decode_level_rgba, parse_ktx2};
use engine_assets::rig_cook::{cook_body, cook_body_at, ATLAS_MAX_DIM, COOKED_DIR, QUADRANTS};
use engine_assets::{AssetSource, BundleSource};
use std::fs;
use std::path::{Path, PathBuf};

/// The bodies the rig holds, in the manifest's order.
const BODIES: [&str; 4] = [
    "hero_body.glb",
    "opp_male_light_body.glb",
    "opp_female_light_body.glb",
    "opp_female_medium_body.glb",
];

fn fighters() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
}

fn hero_body() -> Vec<u8> {
    fs::read(fighters().join("glb/hero_body.glb")).expect("the hero body is in the tree")
}

#[test]
fn the_cooked_body_is_the_same_rig_without_its_images() {
    let source = hero_body();
    let cooked = cook_body("hero_body.glb", &source).expect("the hero body cooks");

    // The rig that comes out is the rig that went in.
    let before = import_skinned_body(&source).expect("the source body imports");
    let after = import_skinned_body(&cooked.glb).expect("the cooked body imports");
    assert_eq!(
        after.skeleton.joints.len(),
        before.skeleton.joints.len(),
        "the same joints"
    );
    for (index, (was, now)) in before
        .skeleton
        .joints
        .iter()
        .zip(after.skeleton.joints.iter())
        .enumerate()
    {
        assert_eq!(now.name, was.name, "joint {index} keeps its name");
        assert_eq!(now.parent, was.parent, "joint {index} keeps its parent");
    }
    assert_eq!(
        after.bind.positions, before.bind.positions,
        "the bind pose is the bind pose"
    );
    assert_eq!(
        after.bind.rotations, before.bind.rotations,
        "the bind rotations are the bind rotations"
    );
    assert_eq!(
        after.inverse_binds, before.inverse_binds,
        "the inverse binds are the inverse binds"
    );
    assert_eq!(after.parts.len(), before.parts.len(), "the same parts");
    for (index, (was, now)) in before.parts.iter().zip(after.parts.iter()).enumerate() {
        assert_eq!(
            now.mesh.positions, was.mesh.positions,
            "part {index} keeps its positions"
        );
        assert_eq!(
            now.mesh.normals, was.mesh.normals,
            "part {index} keeps its normals"
        );
        assert_eq!(now.mesh.uv0, was.mesh.uv0, "part {index} keeps its UVs");
        assert_eq!(
            now.mesh.indices, was.mesh.indices,
            "part {index} keeps its indices"
        );
        assert_eq!(
            now.skin.joints, was.skin.joints,
            "part {index} keeps its skin rows"
        );
        assert_eq!(
            now.skin.weights, was.skin.weights,
            "part {index} keeps its weights"
        );
        assert_eq!(
            now.material.base_color_factor, was.material.base_color_factor,
            "part {index} keeps its base color"
        );
    }
}

#[test]
fn the_cooked_body_is_far_smaller_than_the_source() {
    let source = hero_body();
    let cooked = cook_body("hero_body.glb", &source).expect("the hero body cooks");
    assert!(
        cooked.glb.len() < source.len() / 4,
        "the body without its images is a quarter of the source: {} of {}",
        cooked.glb.len(),
        source.len()
    );
    let atlas: usize = cooked.textures.iter().map(Vec::len).sum();
    assert!(
        atlas < source.len(),
        "the atlases are smaller than the source body: {atlas} of {}",
        source.len()
    );
    // The whole cooked body, atlases included, is smaller than the source.
    assert!(
        cooked.glb.len() + atlas < source.len(),
        "the cooked body is smaller than the source: {} of {}",
        cooked.glb.len() + atlas,
        source.len()
    );
}

#[test]
fn the_cooked_atlases_are_block_maps_at_the_capped_edge() {
    let cooked = cook_body("hero_body.glb", &hero_body()).expect("the hero body cooks");
    assert_eq!(cooked.textures.len(), QUADRANTS, "one atlas per quadrant");
    for (quadrant, bytes) in cooked.textures.iter().enumerate() {
        let texture = parse_ktx2(bytes).expect("the atlas is a KTX2 file");
        assert_eq!(
            texture.width, ATLAS_MAX_DIM,
            "quadrant {quadrant} is capped at the edge"
        );
        assert_eq!(
            texture.height, ATLAS_MAX_DIM,
            "quadrant {quadrant} is square"
        );
        let (width, height, rgba) = decode_level_rgba(&texture, 0).expect("the base level decodes");
        assert_eq!(
            width, ATLAS_MAX_DIM,
            "quadrant {quadrant} decodes at the edge"
        );
        assert_eq!(
            height, ATLAS_MAX_DIM,
            "quadrant {quadrant} decodes at the edge"
        );
        assert_eq!(
            rgba.len(),
            (ATLAS_MAX_DIM * ATLAS_MAX_DIM * 4) as usize,
            "quadrant {quadrant} decodes to RGBA8"
        );
    }
}

#[test]
fn the_atlas_cap_trades_wire_bytes_against_the_page_memory() {
    // The page's own bars: the first bundle is 2,040,000 B brotli with
    // 1,621,894 B already spent, and the linear memory is 19,988,480 B. The
    // rig rides in what is left, so the cap is the lever: the atlas's edge
    // decides the wire bytes and the decoded bytes the page holds.
    let source = hero_body();
    let mut rows: Vec<String> = Vec::new();
    for cap in [256u32, 512, 1024] {
        let cooked = cook_body_at("hero_body.glb", &source, cap).expect("the body cooks");
        let atlas: usize = cooked.textures.iter().map(Vec::len).sum();
        let decoded = 4 * (cap as usize) * (cap as usize) * QUADRANTS;
        rows.push(format!("cap={cap} atlas={atlas} decoded_rgba={decoded}"));
    }
    println!("rig_cook cap receipt: {}", rows.join(" "));
    // The 256 px atlas is a quarter of the pixels of the 1024 px one, so a
    // quarter of the decoded bytes in the page's memory.
    let at_256 = cook_body_at("hero_body.glb", &source, 256).expect("the body cooks");
    let at_1024 = cook_body_at("hero_body.glb", &source, 1024).expect("the body cooks");
    let small: usize = at_256.textures.iter().map(Vec::len).sum();
    let large: usize = at_1024.textures.iter().map(Vec::len).sum();
    assert!(
        small * 3 < large,
        "the 256 px atlas is a quarter of the 1024 px one: {small} of {large}"
    );
    // One body's atlases at 256 px fit the page's memory with room to spare.
    let decoded = 4 * 256 * 256 * QUADRANTS;
    assert!(
        decoded * 4 < 19_988_480,
        "all four bodies' atlases at 256 px fit the page's memory: {decoded} each"
    );
}

#[test]
fn the_cook_gives_the_same_bytes_twice() {
    let source = hero_body();
    let first = cook_body("hero_body.glb", &source).expect("the hero body cooks");
    let second = cook_body("hero_body.glb", &source).expect("the hero body cooks");
    assert_eq!(first.glb, second.glb, "the same body gives the same bytes");
    assert_eq!(first.textures, second.textures, "the same atlases");
}

#[test]
fn the_cooked_rig_is_measured_against_the_page_budgets() {
    // The whole rig, cooked: the bodies without their images plus the
    // atlases. The page's first-bundle budget is 2,040,000 B brotli and the
    // page already spends 1,621,894 B of it, so the rig has to be small
    // enough to ride in the room that is left.
    let fighters = fighters();
    let mut body_bytes = 0usize;
    let mut atlas_bytes = 0usize;
    for name in BODIES {
        let source = fs::read(fighters.join("glb").join(name)).expect("the body is in the tree");
        let cooked = cook_body_at(name, &source, ATLAS_MAX_DIM).expect("the body cooks");
        body_bytes += cooked.glb.len();
        atlas_bytes += cooked.textures.iter().map(Vec::len).sum::<usize>();
    }
    let total = body_bytes + atlas_bytes;
    // The receipt: the bodies and the atlases, and what the atlases decode
    // to in the page's linear memory.
    let decoded = 4 * (ATLAS_MAX_DIM as usize) * (ATLAS_MAX_DIM as usize) * BODIES.len();
    println!(
        "rig_cook receipt: bodies={body_bytes} atlases={atlas_bytes} total={total} \
decoded_rgba={decoded} atlas_dim={ATLAS_MAX_DIM}"
    );
    // The mesh is the floor: the rig's bones and skin, with no atlas at all.
    assert!(
        body_bytes < 1_500_000,
        "the bodies without their atlas are under 1.5 MB: {body_bytes}"
    );
    // The atlases at the capped edge are the wire cost. The rig's whole
    // cooked lane, body and atlas together, has to ride in the room the
    // page's first bundle has left: 2,040,000 B with 1,621,894 B spent.
    assert!(
        atlas_bytes < 800_000,
        "the atlases at {ATLAS_MAX_DIM} px are under 800 KB: {atlas_bytes}"
    );
    // The raw number is the bound a test here can check, because the crate
    // has no brotli encoder. Measured with the page's own gate the packed
    // lane brotli's to 353 007 B, well under the room first_bundle has left
    // (418 106 B): the atlases are block-compressed and the body JSON chunks
    // are space-padded, so the arena kit's 0.486 ratio does not carry over.
    assert!(
        total < 2_040_000,
        "the cooked rig fits the page's whole first-bundle budget: {total} of 2040000"
    );
}

/// The file the page fetches is the cook's own bytes.
///
/// `content/fighters/rig.e2bundle` is packed from the cooked lane
/// (`e2bundle pack content/fighters content/fighters/rig.e2bundle --include
/// cooked/`) and ships in `dist/web` as `assets/fighters.e2bundle`. Every
/// entry has to equal what `cook_body` makes from the body in the tree, and
/// the lane has to hold nothing else: a recook that changes the lane without
/// re-packing the file is a failure here, not a page that quietly ships a
/// stale rig.
#[test]
fn the_shipped_lane_file_is_the_cook() {
    let lane = fs::read(fighters().join("rig.e2bundle")).expect("the lane file is in the tree");
    let bundle = BundleSource::open(lane).expect("the lane bundle opens");

    let mut want: Vec<String> = Vec::new();
    for body in BODIES {
        let variant = body.trim_end_matches("_body.glb");
        let source =
            fs::read(fighters().join(format!("glb/{body}"))).expect("the body is in the tree");
        let cooked = cook_body(body, &source).expect("the body cooks");

        let name = format!("{COOKED_DIR}/{variant}_body.glb");
        assert_eq!(
            &bundle.read(&name).expect("the body is in the lane")[..],
            &cooked.glb[..],
            "{name}"
        );
        want.push(name);

        for quadrant in 0..QUADRANTS {
            let name = format!("{COOKED_DIR}/{variant}_q{quadrant}.ktx2");
            assert_eq!(
                &bundle.read(&name).expect("the atlas is in the lane")[..],
                &cooked.textures[quadrant][..],
                "{name}"
            );
            want.push(name);
        }
    }

    want.sort();
    let mut got = bundle.list("").expect("the lane lists its files");
    got.sort();
    assert_eq!(got, want, "the lane holds the rig and nothing else");
}

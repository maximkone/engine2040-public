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
use engine_assets::rig_cook::{cook_body, ATLAS_MAX_DIM, QUADRANTS};
use std::fs;
use std::path::{Path, PathBuf};

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
fn the_cook_gives_the_same_bytes_twice() {
    let source = hero_body();
    let first = cook_body("hero_body.glb", &source).expect("the hero body cooks");
    let second = cook_body("hero_body.glb", &source).expect("the hero body cooks");
    assert_eq!(first.glb, second.glb, "the same body gives the same bytes");
    assert_eq!(first.textures, second.textures, "the same atlases");
}

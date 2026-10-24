//! fighter_kit_body: the hero's Roman kit as body parts of the hero's own body.
//!
//! `fighter_kit` builds the kit as plain arrays. The look draws a body as a list of parts, each
//! with its own mesh, skin rows, factors and base-colour image, so the kit joins that list here
//! and goes through the same upload as every other part. Owner: this module; consumer:
//! `FighterLook::load_rig` (cold path, once per rig). Determinism: the same body gives the same
//! parts.

use engine_assets::body_import::{BodyPart, SkinnedBody};
use engine_assets::cooked::{CookedMesh, MATERIAL_KIND_STONE};
use engine_assets::skin_import::Skin;

use super::fighter_kit::{dress_hero, KitPart, KIT_PARTS};
use super::fighter_kit_tex::Texels;

/// The map as a PNG (the cooked body parts carry their images encoded). The maps are opaque, so
/// dropping alpha loses nothing. `None` when the codec refuses the size.
fn png(texels: &Texels) -> Option<Vec<u8>> {
    let rgb: Vec<u8> = texels
        .rgba
        .chunks_exact(4)
        .flat_map(|texel| [texel[0], texel[1], texel[2]])
        .collect();
    crate::camera_feed::encode_capture_png(texels.width, texels.height, &rgb).ok()
}

/// One kit piece as a body part: the template's material record with the piece's mesh, skin rows,
/// factors and generated map.
fn body_part(template: &BodyPart, piece: KitPart) -> BodyPart {
    let mut part = template.clone();
    let count = piece.positions.len();
    part.mesh = CookedMesh {
        positions: piece.positions,
        normals: piece.normals,
        uv0: piece.uv0,
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; count],
        indices: piece.indices,
    };
    part.skin = Skin {
        joints: piece.joints,
        weights: piece.weights,
    };
    part.material.base_color_factor = [1.0; 4];
    part.material.metallic_factor = piece.metallic;
    part.material.roughness_factor = piece.roughness;
    part.material.kind = MATERIAL_KIND_STONE;
    part.base_color_image = png(&piece.texels);
    part
}

/// Dress the hero: take its plate suit off and add the Roman kit as parts of its own. A kit
/// piece the body already carries by name (the gear table's `galea` or `lorica`) is not added
/// again. Bodies that are not the hero (the opponents) are left as they are. Returns the
/// parts added.
pub fn dress(body: &mut SkinnedBody, variant: &str) -> usize {
    let Some(template) = body.parts.first().cloned() else {
        return 0;
    };
    // The rig holds `MAX_PARTS` parts: a body that cannot take the whole kit is left as it is.
    if body.parts.len() + KIT_PARTS > super::MAX_PARTS {
        return 0;
    }
    let drawn: Vec<String> = body.parts.iter().map(|part| part.name.clone()).collect();
    let kit: Vec<KitPart> = dress_hero(body, variant)
        .into_iter()
        .filter(|piece| !drawn.iter().any(|name| *name == piece.name))
        .collect();
    let added = kit.len();
    body.parts
        .extend(kit.into_iter().map(|piece| body_part(&template, piece)));
    added
}

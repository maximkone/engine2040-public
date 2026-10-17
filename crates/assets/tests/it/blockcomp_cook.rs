//! blockcomp cook spike: BC7 plus KTX2 emit, x2 identical, under budget.

use engine_assets::bc::{encode_image, BcError, BcFormat, BcSlot};
use engine_assets::budget::POTATO;
use engine_assets::import::{validate_ktx2, KTX2_MAGIC};
use engine_assets::texture_cook::cook_image;

fn albedo() -> Vec<u8> {
    let mut out = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256u32 {
        for x in 0..256u32 {
            let cut = x < 64 && y < 64;
            out.extend_from_slice(&[x as u8, y as u8, ((x + y) / 2) as u8, u8::from(!cut) * 255]);
        }
    }
    out
}

fn normal() -> Vec<u8> {
    let mut out = Vec::with_capacity(256 * 256 * 4);
    for y in 0..256u32 {
        for x in 0..256u32 {
            out.extend_from_slice(&[x as u8, y as u8, 128, 255]);
        }
    }
    out
}

#[test]
fn bc7_plus_ktx2_x2_under_budget() {
    let rgba = albedo();
    let first = cook_image(&rgba, 256, 256, BcSlot::BaseColor).expect("albedo cooks");
    assert_eq!(first.format, BcFormat::Bc7);
    assert_eq!(first.blocks.len(), 65_536);
    assert_eq!(&first.ktx2[0..12], &KTX2_MAGIC);
    assert!(first.ktx2.len() - first.blocks.len() < 256);
    let second = cook_image(&rgba, 256, 256, BcSlot::BaseColor).expect("recook");
    assert_eq!(first.blocks, second.blocks);
    assert_eq!(first.ktx2, second.ktx2);
    assert_eq!(first.digest, second.digest);
    let info = validate_ktx2("tex/albedo.ktx2", &first.ktx2).expect("gate passes");
    assert_eq!((info.width, info.height), (256, 256));
    assert_eq!(first.blocks.len(), 262_144 / 4);
    assert!(first.ktx2.len() < 262_144);
    let map = normal();
    let cooked = cook_image(&map, 256, 256, BcSlot::Normal).expect("normal cooks");
    assert_eq!(cooked.format, BcFormat::Bc5);
    assert_eq!(cooked.blocks.len(), 65_536);
    let again = cook_image(&map, 256, 256, BcSlot::Normal).expect("normal x2");
    assert_eq!(cooked.ktx2, again.ktx2);
    validate_ktx2("tex/normal.ktx2", &cooked.ktx2).expect("normal gate passes");
    assert!(cooked.ktx2.len() < 262_144);
    const { assert!((256 * 256) <= POTATO.max_tex_px) };
}

#[test]
fn cook_refuses_loudly() {
    assert!(matches!(
        cook_image(&[0u8; 64], 5, 4, BcSlot::BaseColor),
        Err(BcError::BadDimensions { .. })
    ));
    let mut transparent = vec![200u8; 64];
    transparent[3] = 0;
    assert!(matches!(
        encode_image(&transparent, 4, 4, BcFormat::Bc1, BcSlot::BaseColor),
        Err(BcError::AlphaInBc1 { .. })
    ));
}

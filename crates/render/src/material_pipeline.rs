//! material_pipeline: render-side bins from draws to compiled materials.
//!
//! One draw names one material plus four texture slots. The forward pass
//! builds one pipeline per distinct compiled structure; draws that share
//! a structure share a pipeline. Std-only: no GPU, no file, no clock.

use engine_core::Fnv64;

/// Most pipelines one frame may need: 21 records share at most 12.
pub const MAX_MATERIAL_PIPELINES: usize = 12;

/// Texture slots per draw, in fixed bind order.
pub const MATERIAL_TEXTURE_SLOTS: usize = 4;

/// Slot names in bind order: base color, ORM, normal, emissive.
pub const SLOT_ORDER: [&str; MATERIAL_TEXTURE_SLOTS] = ["base_color", "orm", "normal", "emissive"];

/// Slot bits in bind order. Values mirror the material slot bits.
pub const SLOT_BITS: [u8; MATERIAL_TEXTURE_SLOTS] = [1, 2, 4, 8];

/// Lobe flags that change the compiled structure, hence the pipeline.
/// Values mirror the material compiler lobe bits so a draw bins like the
/// structure it names without a new dependency.
pub const LOBE_COAT: u8 = 1;
/// Charlie sheen lobe flag; pairs with [`LOBE_COAT`].
pub const LOBE_SHEEN: u8 = 2;

/// Stable pipeline key over lobe flags and the four slot bits alone.
/// Same input gives the same key on any host; draws that share lobes and
/// slots share a pipeline even when their material ids differ. Std-only.
#[must_use]
pub fn pipeline_key_for(lobes: u8, slot_mask: u8) -> u64 {
    let mut h = Fnv64::new();
    h.write_u8(lobes);
    h.write_u8(slot_mask);
    h.finish()
}

/// Pipeline bin of one draw: whose material, which lobes, which slots.
/// Draws with equal keys share one pipeline.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MaterialBinKey {
    /// Opaque compiled-material id from the draw.
    pub material_id: u64,
    /// Lobe flags ([`LOBE_COAT`], [`LOBE_SHEEN`]).
    pub lobes: u8,
    /// Bound slots as [`SLOT_BITS`]: 1 is base color only, 15 is all four.
    pub slot_mask: u8,
}

impl MaterialBinKey {
    /// Bin key of a material id, lobe flags, and four slot-bound flags.
    #[must_use]
    pub fn new(material_id: u64, lobes: u8, bound: [bool; MATERIAL_TEXTURE_SLOTS]) -> Self {
        let mut mask = 0u8;
        for (slot, bit) in bound.iter().zip(SLOT_BITS) {
            if *slot {
                mask |= bit;
            }
        }
        Self {
            material_id,
            lobes,
            slot_mask: mask,
        }
    }

    /// Bound texture keys on this draw: 1 for one slot, up to 4 for all four.
    #[must_use]
    pub fn texture_key_count(&self) -> u32 {
        self.slot_mask.count_ones()
    }

    /// Stable key over the bin: material id plus lobes and slots. Same bin
    /// gives the same key on any host. The shared pipeline key is
    /// [`pipeline_key_for`] over lobes and slots alone, so draws that share
    /// a compiled structure share one pipeline even when their ids differ.
    #[must_use]
    pub fn pipeline_key(&self) -> u64 {
        let base = pipeline_key_for(self.lobes, self.slot_mask);
        let mut h = Fnv64::new();
        h.write_u64(self.material_id);
        h.write_u64(base);
        h.finish()
    }
}

/// Bin key of one [`crate::present::DrawItem`]: its material id plus
/// which of its four slots are bound today. Lobes stay 0 until the
/// draw interface names them.
#[cfg(feature = "v1")]
#[must_use]
pub fn bin_key_for_draw(item: &crate::present::DrawItem) -> MaterialBinKey {
    let slots = item.texture_slots();
    MaterialBinKey::new(
        item.material_id(),
        0,
        [
            slots[0].is_some(),
            slots[1].is_some(),
            slots[2].is_some(),
            slots[3].is_some(),
        ],
    )
}

/// Distinct bins of a frame's draws, in stable order. Equal draws share
/// one entry, so the length is the pipeline count the frame needs.
#[must_use]
pub fn bin_draws(keys: &[MaterialBinKey]) -> Vec<MaterialBinKey> {
    let mut bins: Vec<MaterialBinKey> = keys.to_vec();
    bins.sort();
    bins.dedup();
    bins
}

/// True when the frame's draws fit [`MAX_MATERIAL_PIPELINES`].
#[must_use]
pub fn fits_pipeline_budget(keys: &[MaterialBinKey]) -> bool {
    bin_draws(keys).len() <= MAX_MATERIAL_PIPELINES
}

/// Distinct shared pipelines in a frame's draws: one entry per lobes plus
/// slots combination, so draws that share a compiled structure count once
/// even when their material ids differ. The length is the pipeline total
/// the frame needs. Std-only.
#[must_use]
pub fn pipeline_count(keys: &[MaterialBinKey]) -> usize {
    let mut shared: Vec<u64> = keys
        .iter()
        .map(|draw| pipeline_key_for(draw.lobes, draw.slot_mask))
        .collect();
    shared.sort_unstable();
    shared.dedup();
    shared.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(id: u64, bound: [bool; 4]) -> MaterialBinKey {
        MaterialBinKey::new(id, 0, bound)
    }

    #[test]
    fn same_input_gives_the_same_bin_and_pipeline_key() {
        let a = key(11, [true, false, false, false]);
        let b = key(11, [true, false, false, false]);
        assert_eq!(a, b);
        assert_eq!(a.pipeline_key(), b.pipeline_key());
    }

    #[test]
    fn slot_order_is_base_orm_normal_emissive() {
        assert_eq!(SLOT_ORDER, ["base_color", "orm", "normal", "emissive"]);
        assert_eq!(key(1, [true, false, false, false]).slot_mask, 1);
        assert_eq!(key(1, [false, true, false, false]).slot_mask, 2);
        assert_eq!(key(1, [false, false, true, false]).slot_mask, 4);
        assert_eq!(key(1, [false, false, false, true]).slot_mask, 8);
        assert_eq!(key(1, [true, true, true, true]).slot_mask, 15);
    }

    #[test]
    fn texture_keys_climb_one_to_four() {
        assert_eq!(key(1, [true, false, false, false]).texture_key_count(), 1);
        assert_eq!(key(1, [true, true, false, false]).texture_key_count(), 2);
        assert_eq!(key(1, [true, true, true, false]).texture_key_count(), 3);
        assert_eq!(key(1, [true, true, true, true]).texture_key_count(), 4);
        // A new bound slot is a new bin, never a silent merge.
        assert_ne!(
            key(7, [true, false, false, false]),
            key(7, [true, true, false, false])
        );
    }

    #[test]
    fn twenty_one_draws_share_twelve_pipelines() {
        // The shipped shape: 21 draws over 12 distinct bins.
        let mut draws = Vec::new();
        for i in 0..21u64 {
            let bin = i % 12;
            draws.push(key(1000 + bin, [true, bin % 2 == 0, bin % 3 == 0, false]));
        }
        let bins = bin_draws(&draws);
        assert_eq!(bins.len(), 12);
        assert!(fits_pipeline_budget(&draws));
        // A 13th distinct bin still fits; the 13th over budget does not.
        let mut over: Vec<MaterialBinKey> = bins.clone();
        for i in 0..12u64 {
            over.push(key(5000 + i, [true, true, true, true]));
        }
        assert!(!fits_pipeline_budget(&over));
    }

    #[test]
    fn bins_come_back_in_stable_order() {
        let draws = vec![
            key(9, [true, false, false, false]),
            key(3, [true, false, false, false]),
            key(9, [true, false, false, false]),
        ];
        assert_eq!(
            bin_draws(&draws),
            vec![
                key(3, [true, false, false, false]),
                key(9, [true, false, false, false]),
            ]
        );
    }

    #[test]
    fn draw_can_name_sheen_coat_and_normal_map_slot() {
        let sheen = MaterialBinKey::new(21, LOBE_SHEEN, [true, false, true, false]);
        let coat = MaterialBinKey::new(21, LOBE_COAT, [true, false, true, false]);
        assert_ne!((sheen, sheen.pipeline_key()), (coat, coat.pipeline_key()));
        assert_eq!(sheen.texture_key_count(), 2);
        assert_eq!(sheen.slot_mask & SLOT_BITS[2], SLOT_BITS[2]);
    }

    #[cfg(feature = "v1")]
    #[test]
    fn draw_item_bins_by_id_and_slot_zero() {
        use crate::present::{DrawItem, MeshKey};
        let scene = crate::v1::default_scene(160, 120).expect("default scene is valid");
        let plain = DrawItem {
            mesh: MeshKey::new(1, [0x11; 32]),
            material: scene.material.clone(),
            base_color: None,
            normal_map: None,
        };
        let bound = DrawItem {
            mesh: MeshKey::new(1, [0x11; 32]),
            material: scene.material,
            base_color: Some(crate::present::TextureKey::new(7, [0x77; 32])),
            normal_map: None,
        };
        let a = bin_key_for_draw(&plain);
        assert_eq!(a.texture_key_count(), 0);
        let b = bin_key_for_draw(&bound);
        assert_eq!(b.texture_key_count(), 1);
        assert_eq!(b.slot_mask, 1);
        assert_ne!(a, b);
    }

    // Shipped shape read from the material compiler and set: 14 siege
    // records in id order then 7 reference records. Lobes use the file
    // flags plus the four compiler bits below; all shipped records are
    // untextured today so the mask is 0 and the four-slot path is proved
    // by the draw tests above. Layers split ceramic from stone in the
    // full build; this slice keys lobes and slots only.
    const EMISSION_BIT: u8 = 32;
    const ANISOTROPY_BIT: u8 = 4;
    const TRANSMISSION_BIT: u8 = 8;
    const SUBSURFACE_BIT: u8 = 16;

    const SHIPPED: [(&str, u8, u8); 21] = [
        ("stone", LOBE_COAT | EMISSION_BIT, 0),
        ("brick", LOBE_COAT | EMISSION_BIT, 0),
        ("timber", EMISSION_BIT, 0),
        ("thatch", LOBE_SHEEN | EMISSION_BIT, 0),
        ("plaster", LOBE_COAT | EMISSION_BIT, 0),
        ("concrete", LOBE_COAT | EMISSION_BIT, 0),
        ("marble", LOBE_COAT | EMISSION_BIT, 0),
        ("iron", EMISSION_BIT, 0),
        ("bronze", EMISSION_BIT, 0),
        ("leather", LOBE_SHEEN | EMISSION_BIT, 0),
        ("cloth", LOBE_SHEEN | EMISSION_BIT, 0),
        ("rope", EMISSION_BIT, 0),
        ("glass", TRANSMISSION_BIT | EMISSION_BIT, 0),
        ("ceramic", LOBE_COAT | EMISSION_BIT, 0),
        ("sand", LOBE_COAT | EMISSION_BIT, 0),
        ("steel", ANISOTROPY_BIT | EMISSION_BIT, 0),
        ("skin", SUBSURFACE_BIT | LOBE_COAT, 0),
        ("marble", LOBE_COAT | EMISSION_BIT, 0),
        ("wood", EMISSION_BIT, 0),
        ("cloth", LOBE_SHEEN | EMISSION_BIT, 0),
        ("marble_wet", LOBE_COAT | EMISSION_BIT, 0),
    ];

    #[test]
    fn shipped_records_all_build() {
        assert_eq!(SHIPPED.len(), 21);
        for &(name, lobes, slot_mask) in SHIPPED.iter() {
            let first = pipeline_key_for(lobes, slot_mask);
            let again = pipeline_key_for(lobes, slot_mask);
            assert_eq!(first, again, "{name} builds stably");
            let bin = MaterialBinKey::new(1, lobes, [false, false, false, false]);
            assert_eq!(bin.slot_mask, slot_mask, "{name} mask");
            assert_eq!(bin.texture_key_count(), 0, "{name} untextured");
        }
    }

    #[test]
    fn shipped_pipelines_fit_budget() {
        let mut keys: Vec<u64> = SHIPPED
            .iter()
            .map(|&(_, lobes, slot_mask)| pipeline_key_for(lobes, slot_mask))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(SHIPPED.len(), 21);
        assert!(
            keys.len() <= MAX_MATERIAL_PIPELINES,
            "{} pipelines for 21 records",
            keys.len()
        );
    }

    #[test]
    fn texture_keys_per_draw_are_four() {
        assert_eq!(MATERIAL_TEXTURE_SLOTS, 4);
        assert_eq!(SLOT_ORDER.len(), 4);
        assert_eq!(SLOT_BITS.len(), 4);
        let full = MaterialBinKey::new(5, 0, [true, true, true, true]);
        assert_eq!(full.texture_key_count(), 4);
        assert_eq!(full.slot_mask, 15);
    }

    #[test]
    fn shared_pipeline_key_ignores_material_id() {
        let a = MaterialBinKey::new(1, LOBE_COAT, [true, false, true, false]);
        let b = MaterialBinKey::new(2, LOBE_COAT, [true, false, true, false]);
        assert_ne!(a, b);
        assert_ne!(a.pipeline_key(), b.pipeline_key());
        assert_eq!(
            pipeline_key_for(a.lobes, a.slot_mask),
            pipeline_key_for(b.lobes, b.slot_mask)
        );
        assert_eq!(
            pipeline_key_for(LOBE_COAT, 1 | 4),
            pipeline_key_for(a.lobes, a.slot_mask)
        );
        assert_eq!(pipeline_count(&[a, b]), 1);
    }

    #[test]
    fn pipeline_count_shares_across_material_ids() {
        let draws: Vec<MaterialBinKey> = SHIPPED
            .iter()
            .enumerate()
            .map(|(index, &(_, lobes, slot_mask))| {
                let id = u64::try_from(index).expect("21 draws fit in u64");
                let bound = [
                    slot_mask & SLOT_BITS[0] != 0,
                    slot_mask & SLOT_BITS[1] != 0,
                    slot_mask & SLOT_BITS[2] != 0,
                    slot_mask & SLOT_BITS[3] != 0,
                ];
                MaterialBinKey::new(id, lobes, bound)
            })
            .collect();
        assert_eq!(draws.len(), 21);
        assert_eq!(bin_draws(&draws).len(), 21);
        assert!(pipeline_count(&draws) <= MAX_MATERIAL_PIPELINES);
    }
}

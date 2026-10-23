//! material_kind: which shading family one draw takes.
//!
//! The kind rides the material uniform's word at bytes 24..28 and the cook's
//! `CookedMaterial::kind` byte. Stone is word 0, so an older cook with no
//! kind byte, and every path that never sets one, keeps the single-BRDF frame
//! each earlier run drew.
//!
//! Each kind owns one shader branch in `shaders/forward.wgsl` `direct_light`:
//! stone is the Burley plus isotropic GGX pair, skin adds wrap and thickness
//! transmittance to the diffuse lobe, metal swaps in anisotropic GGX over the
//! mesh tangent frame, cloth swaps in Charlie sheen and Neubelt visibility.
//! Shapes are Filament's (`shaders/src/surface_brdf.fs`,
//! `surface_shading_model_subsurface.fs`, `surface_shading_model_cloth.fs`);
//! the skin lobe is a game lobe, never a physically accurate skin model.

/// Shading family of one draw, stored as a kind word in the material uniform.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[repr(u8)]
pub enum MaterialKind {
    /// Today's path: Burley diffuse plus isotropic GGX specular. Word 0.
    #[default]
    Stone = 0,
    /// Wrap plus thickness transmittance on the diffuse lobe. Word 1.
    Skin = 1,
    /// Anisotropic GGX specular over the tangent frame. Word 2.
    Metal = 2,
    /// Charlie sheen specular plus Burley diffuse. Word 3.
    Cloth = 3,
}

impl MaterialKind {
    /// Every kind, stone first, so a caller can walk the set.
    pub const ALL: [Self; 4] = [Self::Stone, Self::Skin, Self::Metal, Self::Cloth];

    /// The uniform word (bytes 24..28) this kind writes.
    pub const fn word(self) -> u32 {
        match self {
            Self::Stone => 0,
            Self::Skin => 1,
            Self::Metal => 2,
            Self::Cloth => 3,
        }
    }

    /// The kind a uniform word names. An unknown word keeps stone rather
    /// than reading a lobe this build does not carry.
    pub const fn from_word(word: u32) -> Self {
        match word {
            1 => Self::Skin,
            2 => Self::Metal,
            3 => Self::Cloth,
            _ => Self::Stone,
        }
    }

    /// The parameter word (bytes 28..32): skin thickness or metal
    /// anisotropy, both in `0..=1`. Stone and cloth read no parameter, so
    /// they write zero and leave the word free for a later kind.
    pub const fn default_param(self) -> f32 {
        match self {
            Self::Skin => 0.35,
            Self::Metal => 0.6,
            _ => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_map_to_their_words_both_ways() {
        for (kind, word) in [
            (MaterialKind::Stone, 0u32),
            (MaterialKind::Skin, 1),
            (MaterialKind::Metal, 2),
            (MaterialKind::Cloth, 3),
        ] {
            assert_eq!(kind.word(), word, "{kind:?} word");
            assert_eq!(MaterialKind::from_word(word), kind, "word {word}");
        }
        assert_eq!(
            MaterialKind::from_word(9),
            MaterialKind::Stone,
            "an unknown word keeps stone"
        );
        assert_eq!(MaterialKind::default(), MaterialKind::Stone);
    }

    #[test]
    fn only_skin_and_metal_carry_a_parameter() {
        assert_eq!(MaterialKind::Stone.default_param(), 0.0);
        assert_eq!(MaterialKind::Cloth.default_param(), 0.0);
        for kind in [MaterialKind::Skin, MaterialKind::Metal] {
            let param = kind.default_param();
            assert!(
                (0.0..=1.0).contains(&param) && param > 0.0,
                "{kind:?} parameter {param}"
            );
        }
    }
}

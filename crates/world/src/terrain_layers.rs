//! terrain_layers: a sidecar layer document over the TVER v1 terrain.
//!
//! TVER v1 stays untouched; this module only decides which heights a map has
//! and then reuses [`write_terrain`](crate::terrain::write_terrain) for bytes
//! and [`terrain_stats`](crate::terrain::terrain_stats) for the door's row.
//! Steal: `xKrvZ/Terra@63e822ad` (MIT) `landscape_blueprint.rs`: one named
//! blueprint of ordered layers, ideas only, zero bytes vendored.
//!
//! Ladder, fixed and integer-only, so two runs agree byte for byte:
//! 1. base: [`base_height`](crate::terrain::base_height) at every texel;
//! 2. layers in document order: each layer blends its field over the current
//!    height by `weight_permille` (see [`Blend`] for the four rules);
//! 3. holes: the seed bake's hole mask, never edited by a layer;
//! 4. splat: the crate's own rule from the final height and its slope.
//!
//! Units: texels for `size`, permille for weights, height steps `0..=255`.
//! Determinism: no clock, no RNG, no float; the same document always resolves
//! to the same [`Terrain`]. Errors: [`LayerErr`], each naming the legal range
//! and one recovery.

use crate::terrain::{
    bake_terrain, base_height, parse_terrain, slope_of, splat_for, write_terrain, Terrain,
    SLOPE_DEFAULT,
};
use crate::{height, world_hash, MAX_BAKE_SIZE};

/// Most layers one document may hold.
pub const MAX_LAYERS: usize = 32;
/// Weight total every document must hit, in permille.
pub const PERMILLE: u32 = 1000;
const RIDGE_TAG: u64 = 0x52_4944_4745;
const VALLEY_TAG: u64 = 0x56_414c_4c59;

/// How one layer folds its field into the current height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blend {
    /// Add the field's difference from the base field, scaled by weight; two
    /// agreeing fields at 1000 leave the map alone.
    Add,
    /// Keep the higher of the current height and the weighted field.
    Max,
    /// Keep the lower of the current height and the weighted field.
    Min,
    /// Move the current height toward the field by weight.
    Mix,
}

impl Blend {
    /// Every blend name a document may use, in contract order.
    pub const NAMES: [&'static str; 4] = ["add", "max", "min", "mix"];

    /// Parse one blend name; `None` names an unknown blend.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "add" => Some(Self::Add),
            "max" => Some(Self::Max),
            "min" => Some(Self::Min),
            "mix" => Some(Self::Mix),
            _ => None,
        }
    }

    /// The name as a document writes it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Max => "max",
            Self::Min => "min",
            Self::Mix => "mix",
        }
    }
}

/// One height field a layer can carry, evaluated from the crate's integer
/// value-noise helpers with the document seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The ladder's own base: the 2-octave bake field.
    Base,
    /// Crests: peaks where the tiered noise reaches an extreme, `127..=255`.
    Ridge,
    /// Floors: low where the tiered noise sits mid-range, `0..=127`.
    Valley,
    /// Terraces of the base field, 32 steps apart.
    Plateau,
    /// Raw value noise under its own tag, so two noise layers differ.
    Noise(u64),
}

impl Field {
    /// Every field name a document may use (`noise:<tag>` carries a `u64` tag).
    pub const NAMES: [&'static str; 5] = ["base", "ridge", "valley", "plateau", "noise:<tag>"];

    /// Parse one field name; `None` names an unknown field or a bad noise tag.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "base" => Some(Self::Base),
            "ridge" => Some(Self::Ridge),
            "valley" => Some(Self::Valley),
            "plateau" => Some(Self::Plateau),
            other => other
                .strip_prefix("noise:")
                .and_then(|tag| tag.parse::<u64>().ok())
                .map(Self::Noise),
        }
    }

    /// The name as a document writes it.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Base => "base".to_owned(),
            Self::Ridge => "ridge".to_owned(),
            Self::Valley => "valley".to_owned(),
            Self::Plateau => "plateau".to_owned(),
            Self::Noise(tag) => format!("noise:{tag}"),
        }
    }
}

/// One ordered layer of a [`TerrainLayers`] document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// Layer name, unique inside one document.
    pub name: String,
    /// Share of the layer, in permille; the document's layers sum to 1000.
    pub weight_permille: u16,
    /// How the field meets the current height.
    pub blend: Blend,
    /// The height field this layer carries.
    pub field: Field,
}

/// One sidecar terrain document: a name, the seed bake it sits on, and layers
/// in order. Resolving it is the only way to get bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainLayers {
    /// Document name, for the door row and the file it writes.
    pub name: String,
    /// Bake seed: the base field, the hole mask and every `Noise` tag ride it.
    pub seed: u64,
    /// Square side in texels, `1..=`[`MAX_BAKE_SIZE`].
    pub size: u16,
    /// Layers in the order the ladder applies them.
    pub layers: Vec<Layer>,
}

/// Why a layer document was refused. Nothing is baked or written on refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerErr {
    /// The document names no layer.
    NoLayers,
    /// The document holds more than [`MAX_LAYERS`] layers.
    TooManyLayers {
        /// The refused layer count.
        layers: usize,
    },
    /// The side exceeds [`MAX_BAKE_SIZE`].
    TooLarge {
        /// The refused side in texels.
        size: u16,
    },
    /// Weights do not sum to [`PERMILLE`].
    WeightsNotPermille {
        /// The sum the document asked for.
        sum: u32,
    },
    /// Two layers share a name.
    DuplicateName {
        /// Index of the first layer with the name.
        first: usize,
        /// Index of the repeated name.
        second: usize,
    },
    /// A layer name is empty.
    EmptyName {
        /// Index of the unnamed layer.
        layer: usize,
    },
}

impl std::fmt::Display for LayerErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLayers => write!(
                f,
                "terrain document holds 0 layers; add 1..={MAX_LAYERS} layers whose weights sum to \
                 {PERMILLE}, for example one base layer at {PERMILLE}"
            ),
            Self::TooManyLayers { layers } => write!(
                f,
                "terrain document holds {layers} layers; keep 1..={MAX_LAYERS}"
            ),
            Self::TooLarge { size } => write!(
                f,
                "terrain size {size} is outside 1..={MAX_BAKE_SIZE}; re-run with a side that fits"
            ),
            Self::WeightsNotPermille { sum } => write!(
                f,
                "layer weights sum to {sum}; they must sum to exactly {PERMILLE} permille"
            ),
            Self::DuplicateName { first, second } => write!(
                f,
                "layer {second} repeats the name of layer {first}; give every layer its own name"
            ),
            Self::EmptyName { layer } => write!(
                f,
                "layer {layer} has an empty name; name it, for example `ridge`"
            ),
        }
    }
}

impl std::error::Error for LayerErr {}

impl TerrainLayers {
    /// Check the document before any bake allocates: a layer count, a side, a
    /// weight total and unique names. Errors: [`LayerErr`].
    pub fn validate(&self) -> Result<(), LayerErr> {
        if self.size > MAX_BAKE_SIZE {
            return Err(LayerErr::TooLarge { size: self.size });
        }
        if self.layers.is_empty() {
            return Err(LayerErr::NoLayers);
        }
        if self.layers.len() > MAX_LAYERS {
            return Err(LayerErr::TooManyLayers {
                layers: self.layers.len(),
            });
        }
        let mut sum = 0u32;
        for (index, layer) in self.layers.iter().enumerate() {
            sum = sum.saturating_add(u32::from(layer.weight_permille));
            if layer.name.trim().is_empty() {
                return Err(LayerErr::EmptyName { layer: index });
            }
            if let Some(first) = self.layers[..index]
                .iter()
                .position(|other| other.name == layer.name)
            {
                return Err(LayerErr::DuplicateName {
                    first,
                    second: index,
                });
            }
        }
        if sum != PERMILLE {
            return Err(LayerErr::WeightsNotPermille { sum });
        }
        Ok(())
    }

    /// Resolve the document to one terrain: the base field, every layer in
    /// order, the seed bake's holes untouched, then the crate's splat rule from
    /// the final height and slope. Errors: [`LayerErr`] before any bake.
    pub fn resolve(&self) -> Result<Terrain, LayerErr> {
        self.validate()?;
        let bytes = bake_terrain(self.seed, self.size)
            .map_err(|_| LayerErr::TooLarge { size: self.size })?;
        let mut terrain = parse_terrain(&bytes).ok_or(LayerErr::TooLarge { size: self.size })?;
        let n = usize::from(self.size);
        let mut heights = vec![0u8; n * n];
        for y in 0..n {
            for x in 0..n {
                heights[y * n + x] = base_height(self.seed, coord(x), coord(y));
            }
        }
        for layer in &self.layers {
            for y in 0..n {
                for x in 0..n {
                    let at = y * n + x;
                    let base = base_height(self.seed, coord(x), coord(y));
                    let field = field_at(layer.field, self.seed, coord(x), coord(y));
                    heights[at] =
                        blend_value(layer.blend, heights[at], field, base, layer.weight_permille);
                }
            }
        }
        for y in 0..n {
            for x in 0..n {
                let at = y * n + x;
                terrain.splat[at] = splat_for(heights[at], slope_of(&heights, n, x, y));
            }
        }
        terrain.heights = heights;
        Ok(terrain)
    }

    /// Canonical TVER bytes of the resolved document. Errors: [`LayerErr`].
    pub fn resolve_bytes(&self) -> Result<Vec<u8>, LayerErr> {
        write_terrain(&self.resolve()?).map_err(|_| LayerErr::TooLarge { size: self.size })
    }

    /// FNV-1a 64 of the resolved bytes: the same document always answers it.
    /// Errors: [`LayerErr`].
    pub fn digest(&self) -> Result<u64, LayerErr> {
        Ok(world_hash(&self.resolve_bytes()?))
    }

    /// Cells [`crate::terrain::blocked`] refuses at [`SLOPE_DEFAULT`] after the
    /// ladder; the door's row reuses this instead of re-deriving it.
    pub fn blocked_cells(&self) -> Result<u32, LayerErr> {
        let terrain = self.resolve()?;
        let count = crate::terrain::blocked(&terrain, SLOPE_DEFAULT)
            .iter()
            .filter(|cell| **cell)
            .count();
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }
}

/// One texel index as the noise helpers take it; a side never exceeds
/// [`MAX_BAKE_SIZE`], so the conversion cannot fail.
fn coord(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// One layer's field at one texel: the crate's integer value noise, folded by
/// the field kind. The base field is the ladder's own start.
fn field_at(field: Field, seed: u64, x: i32, y: i32) -> u8 {
    match field {
        Field::Base => base_height(seed, x, y),
        Field::Ridge => {
            let h = height(seed ^ RIDGE_TAG, x, y);
            if h >= 128 {
                255
            } else {
                h.saturating_mul(2)
            }
        }
        Field::Valley => height(seed ^ VALLEY_TAG, x, y).abs_diff(128),
        Field::Plateau => base_height(seed, x, y) & 0xE0,
        Field::Noise(tag) => height(seed ^ tag, x, y),
    }
}

/// Fold `field` into `current` by `weight` permille. Units: height steps.
fn blend_value(blend: Blend, current: u8, field: u8, base: u8, weight: u16) -> u8 {
    let weight = i32::from(weight);
    let (cur, field, base) = (i32::from(current), i32::from(field), i32::from(base));
    let lerp = cur + (field - cur) * weight / i32::try_from(PERMILLE).unwrap_or(1000);
    let value = match blend {
        Blend::Add => cur + (field - base) * weight / i32::try_from(PERMILLE).unwrap_or(1000),
        Blend::Max => lerp.max(cur),
        Blend::Min => lerp.min(cur),
        Blend::Mix => lerp,
    };
    u8::try_from(value.clamp(0, 255)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(name: &str, weight: u16, blend: Blend, field: Field) -> Layer {
        Layer {
            name: name.to_owned(),
            weight_permille: weight,
            blend,
            field,
        }
    }

    fn doc(layers: Vec<Layer>) -> TerrainLayers {
        TerrainLayers {
            name: "fixture".to_owned(),
            seed: 2040,
            size: 24,
            layers,
        }
    }

    #[test]
    fn empty_and_malformed_documents_are_refused_typed() {
        let empty = doc(Vec::new());
        assert_eq!(empty.resolve().err(), Some(LayerErr::NoLayers));
        assert!(empty
            .resolve()
            .expect_err("empty refuses")
            .to_string()
            .contains("1..=32"));
        // Weights must sum to 1000: name the sum and the rule.
        let light = doc(vec![layer("a", 400, Blend::Mix, Field::Base)]);
        assert_eq!(
            light.validate(),
            Err(LayerErr::WeightsNotPermille { sum: 400 })
        );
        let text = light.validate().expect_err("sum refuses").to_string();
        assert!(text.contains("400") && text.contains("1000"), "{text}");
        // Duplicate names, empty names, too many layers and an oversize side.
        let twice = doc(vec![
            layer("a", 500, Blend::Mix, Field::Base),
            layer("a", 500, Blend::Mix, Field::Ridge),
        ]);
        assert_eq!(
            twice.validate(),
            Err(LayerErr::DuplicateName {
                first: 0,
                second: 1
            })
        );
        let unnamed = doc(vec![layer("", 1000, Blend::Mix, Field::Base)]);
        assert_eq!(unnamed.validate(), Err(LayerErr::EmptyName { layer: 0 }));
        let many = doc(vec![
            layer("a", 1000, Blend::Mix, Field::Base);
            MAX_LAYERS + 1
        ]);
        assert_eq!(
            many.validate(),
            Err(LayerErr::TooManyLayers {
                layers: MAX_LAYERS + 1
            })
        );
        let mut huge = doc(vec![layer("a", 1000, Blend::Mix, Field::Base)]);
        huge.size = MAX_BAKE_SIZE + 1;
        assert_eq!(
            huge.validate(),
            Err(LayerErr::TooLarge {
                size: MAX_BAKE_SIZE + 1
            })
        );
        // Field and blend names parse both ways; an unknown name answers None.
        for name in Field::NAMES {
            assert_eq!(Field::parse(name).is_some(), !name.contains('<'));
        }
        assert_eq!(Field::parse("noise:7"), Some(Field::Noise(7)));
        assert_eq!(Field::parse("noise:x"), None);
        assert_eq!(Field::parse("voronoi"), None);
        for name in Blend::NAMES {
            let blend = Blend::parse(name).expect("every blend parses");
            assert_eq!(blend.name(), name);
        }
        assert_eq!(Blend::parse("overlay"), None);
        assert_eq!(Field::Noise(7).name(), "noise:7");
    }

    #[test]
    fn ridge_layer_at_1000_differs_and_raises_the_max() {
        let base = doc(vec![layer("base", 1000, Blend::Mix, Field::Base)]);
        let ridge = doc(vec![layer("ridge", 1000, Blend::Mix, Field::Ridge)]);
        let base_bytes = base.resolve_bytes().expect("base resolves");
        let ridge_bytes = ridge.resolve_bytes().expect("ridge resolves");
        assert_ne!(base_bytes, ridge_bytes, "ridge differs from base");
        let base_max = parse_terrain(&base_bytes)
            .expect("parses")
            .heights
            .iter()
            .copied()
            .max();
        let ridge_max = parse_terrain(&ridge_bytes)
            .expect("parses")
            .heights
            .iter()
            .copied()
            .max();
        assert!(
            ridge_max > base_max,
            "ridge max {ridge_max:?} beats base max {base_max:?}"
        );
        // The ridge fold halves the tiered noise up to a full crest at 255, so
        // the crest reaches the top of the height range.
        let parsed = parse_terrain(&ridge_bytes).expect("parses");
        assert_eq!(parsed.heights.iter().copied().max(), Some(255));
    }

    #[test]
    fn add_and_mix_agree_when_the_fields_agree() {
        // Two base layers carry the same field as the ladder's start, so Add's
        // difference term is zero and Mix's move is zero: same bytes.
        let add = doc(vec![
            layer("a", 500, Blend::Add, Field::Base),
            layer("b", 500, Blend::Add, Field::Base),
        ]);
        let mix = doc(vec![
            layer("a", 500, Blend::Mix, Field::Base),
            layer("b", 500, Blend::Mix, Field::Base),
        ]);
        assert_eq!(
            add.resolve_bytes().expect("add resolves"),
            mix.resolve_bytes().expect("mix resolves")
        );
    }

    #[test]
    fn resolve_twice_is_byte_equal_and_the_digest_is_stable() {
        let document = doc(vec![
            layer("plateau", 400, Blend::Mix, Field::Plateau),
            layer("ridge", 600, Blend::Mix, Field::Ridge),
        ]);
        let first = document.resolve_bytes().expect("resolves");
        let second = document.resolve_bytes().expect("resolves");
        assert_eq!(first, second, "two resolves agree byte for byte");
        assert_eq!(
            document.digest().expect("digests"),
            document.digest().expect("digests")
        );
        assert_eq!(
            Some(world_hash(&first)),
            document.digest().ok(),
            "digest hashes the written bytes"
        );
        // Holes come from the seed bake and a layer never edits them.
        let terrain = document.resolve().expect("resolves");
        let baked = parse_terrain(&bake_terrain(document.seed, document.size).expect("bakes"))
            .expect("parses");
        assert_eq!(terrain.holes, baked.holes, "holes stay untouched");
        assert_eq!(terrain.splat.len(), terrain.heights.len());
        assert!(terrain.splat.iter().all(|w| {
            let total: u32 = w.iter().map(|part| u32::from(*part)).sum();
            total == 255
        }));
        // A different blend order is a different document.
        let swapped = doc(vec![
            layer("ridge", 600, Blend::Mix, Field::Ridge),
            layer("plateau", 400, Blend::Mix, Field::Plateau),
        ]);
        assert_ne!(
            swapped.digest().expect("digests"),
            document.digest().expect("digests")
        );
        assert!(document.blocked_cells().expect("counts") <= u32::try_from(24 * 24).expect("fits"));
    }
}

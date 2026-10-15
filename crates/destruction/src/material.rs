//! material: the world material table for deterministic breaks.
//!
//! Consumer: the graded-bond solver reads [`MaterialTable::breaking_strain_q16`]
//! for each bond's material instead of keeping its own copy of the numbers;
//! the structure, energy and fire follow-ups read toughness, shard rule and
//! burn rate from the same rows. Bond lists and baked-chunk lists carry a
//! parallel material-id byte array each (same order as the bonds or chunks);
//! [`validate_bond_materials`] and [`validate_chunk_materials`] check those
//! arrays. Data owner: `engine-destruction`. Units: toughness, brittleness
//! and burn rate in Q16 fixed point (`1.0 == 65536`); shard, sound, dust,
//! spark and decal as small enum bytes. Determinism scope: [`validate`] and
//! [`MaterialTable::digest`] are pure over the table bytes in material-id
//! order; the same rows give the same digest on every PC, and validation
//! reports the first failure in material-id order. Errors:
//! [`MaterialError`]; no function panics on user data. Budget: validate and
//! digest are `O(MATERIAL_COUNT)` integer ops with no heap; getters are
//! `O(1)`. Presentation (sound, dust, spark, decal) rides along in the rows
//! so one table versions the whole break look, but tick math reads only
//! toughness, brittleness, shard rule and burn rate: a slow GPU never changes
//! an outcome.
//!
//! ```rust
//! use engine_destruction::material::{
//!     validate_bond_materials, MaterialId, MaterialTable,
//! };
//!
//! let table = MaterialTable::siege_kit();
//! assert!(table.validate().is_ok());
//! assert_eq!(table.digest(), MaterialTable::siege_kit().digest());
//! assert!(validate_bond_materials(&[MaterialId::Stone.as_u8()]).is_ok());
//! assert!(table.is_combustible(MaterialId::Thatch));
//! ```

use engine_core::Fnv64;

/// Table version of the shipped siege row.
pub const MATERIAL_VERSION_V1: u32 = 1;
/// Materials in the shipped table, indexed by [`MaterialId`].
pub const MATERIAL_COUNT: usize = 14;
/// One whole unit in Q16 fixed point.
pub const Q16_ONE: u32 = 65_536;
/// Toughness cap in Q16 (`64.0`); anything tougher is data error. Matches the
/// cut-resolver tissue cap so bonded strain and blade protection agree.
pub const MAX_TOUGHNESS_Q16: u32 = 64 << 16;
/// Brittleness cap in Q16 (`1.0`): a fraction, fully shattering at the cap.
pub const MAX_BRITTLENESS_Q16: u32 = 65_536;
/// Burn-rate cap in Q16 (`16.0` fuel units); anything faster is data error.
pub const MAX_BURN_Q16: u32 = 16 << 16;

/// World material, in canonical digest order.
///
/// The fourteen cover the siege and arena set: quarried stone, fired brick,
/// structural timber, thatch roofing, plaster facing, poured concrete,
/// marble facing, wrought iron, bronze fittings, leather straps, woven
/// cloth, rope rigging, glass panes and ceramic tile.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaterialId {
    /// Quarried stone: hard, brittle, blocky shards, stone dust.
    Stone,
    /// Fired brick: mid-hard, brick dust, mixed chunks.
    Brick,
    /// Structural timber: splinters, burns steadily.
    Timber,
    /// Thatch roofing: weak, tears in sheets, burns fast.
    Thatch,
    /// Plaster facing: weak, falls to dust, never burns.
    Plaster,
    /// Poured concrete: very hard, blocky shards, never burns.
    Concrete,
    /// Marble facing: hard, shatters to shards, never burns.
    Marble,
    /// Wrought iron: toughest row, denting, throws shower sparks.
    Iron,
    /// Bronze fittings: near-iron toughness, denting, shower sparks.
    Bronze,
    /// Leather straps: tough sheets, smoulders slowly.
    Leather,
    /// Woven cloth: weak sheets, burns fast.
    Cloth,
    /// Rope rigging: frays to fibers, burns steadily.
    Rope,
    /// Glass panes: weakest hard row, fully shattering.
    Glass,
    /// Ceramic tile: mid-soft, shatters to shards, never burns.
    Ceramic,
}

impl MaterialId {
    /// Wire value of the material, `0..MATERIAL_COUNT`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Stone => 0,
            Self::Brick => 1,
            Self::Timber => 2,
            Self::Thatch => 3,
            Self::Plaster => 4,
            Self::Concrete => 5,
            Self::Marble => 6,
            Self::Iron => 7,
            Self::Bronze => 8,
            Self::Leather => 9,
            Self::Cloth => 10,
            Self::Rope => 11,
            Self::Glass => 12,
            Self::Ceramic => 13,
        }
    }

    /// Array index of the material; the digest and validation order.
    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.as_u8())
    }

    /// Record name of the material: the text form the material record
    /// (`engine-material`) and the editor binding use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stone => "stone",
            Self::Brick => "brick",
            Self::Timber => "timber",
            Self::Thatch => "thatch",
            Self::Plaster => "plaster",
            Self::Concrete => "concrete",
            Self::Marble => "marble",
            Self::Iron => "iron",
            Self::Bronze => "bronze",
            Self::Leather => "leather",
            Self::Cloth => "cloth",
            Self::Rope => "rope",
            Self::Glass => "glass",
            Self::Ceramic => "ceramic",
        }
    }

    /// Material by record name; `None` for a name no shipped row has.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        Self::all().into_iter().find(|id| id.name() == text)
    }

    /// Every material in canonical order.
    #[must_use]
    pub const fn all() -> [MaterialId; MATERIAL_COUNT] {
        [
            MaterialId::Stone,
            MaterialId::Brick,
            MaterialId::Timber,
            MaterialId::Thatch,
            MaterialId::Plaster,
            MaterialId::Concrete,
            MaterialId::Marble,
            MaterialId::Iron,
            MaterialId::Bronze,
            MaterialId::Leather,
            MaterialId::Cloth,
            MaterialId::Rope,
            MaterialId::Glass,
            MaterialId::Ceramic,
        ]
    }
}

impl TryFrom<u8> for MaterialId {
    type Error = MaterialError;

    /// Wire byte to material; unknown bytes refuse, never wrap.
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Stone),
            1 => Ok(Self::Brick),
            2 => Ok(Self::Timber),
            3 => Ok(Self::Thatch),
            4 => Ok(Self::Plaster),
            5 => Ok(Self::Concrete),
            6 => Ok(Self::Marble),
            7 => Ok(Self::Iron),
            8 => Ok(Self::Bronze),
            9 => Ok(Self::Leather),
            10 => Ok(Self::Cloth),
            11 => Ok(Self::Rope),
            12 => Ok(Self::Glass),
            13 => Ok(Self::Ceramic),
            other => Err(MaterialError::UnknownMaterial(other)),
        }
    }
}

/// How a material comes apart: the fracture follow-up sizes its cells by this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShardRule {
    /// A few large pieces that stay whole (iron, bronze).
    FewLarge,
    /// Mixed chunk sizes (brick).
    Mixed,
    /// Long fiber pieces (timber, rope).
    Splinters,
    /// Sharp small pieces (marble, glass, ceramic).
    Shards,
    /// Falls to powder (plaster).
    Dust,
    /// Tears in whole sheets (thatch, leather, cloth).
    Sheets,
}

impl ShardRule {
    /// Wire value of the shard rule.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::FewLarge => 0,
            Self::Mixed => 1,
            Self::Splinters => 2,
            Self::Shards => 3,
            Self::Dust => 4,
            Self::Sheets => 5,
        }
    }

    /// Record name of the shard rule.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::FewLarge => "few_large",
            Self::Mixed => "mixed",
            Self::Splinters => "splinters",
            Self::Shards => "shards",
            Self::Dust => "dust",
            Self::Sheets => "sheets",
        }
    }

    /// Shard rule by record name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "few_large" => Some(Self::FewLarge),
            "mixed" => Some(Self::Mixed),
            "splinters" => Some(Self::Splinters),
            "shards" => Some(Self::Shards),
            "dust" => Some(Self::Dust),
            "sheets" => Some(Self::Sheets),
            _ => None,
        }
    }
}

/// Impact sound family; presentation only, chosen by the audio handoff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoundKind {
    /// Stone knock (stone, brick, concrete, marble).
    Stone,
    /// Wood knock (timber).
    Wood,
    /// Metal ring (iron, bronze).
    Metal,
    /// Soft thud or creak (thatch, leather, cloth, rope).
    Soft,
    /// Shatter clink (glass, ceramic).
    Glass,
    /// Plaster crack (plaster).
    Crack,
}

impl SoundKind {
    /// Wire value of the sound family.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Stone => 0,
            Self::Wood => 1,
            Self::Metal => 2,
            Self::Soft => 3,
            Self::Glass => 4,
            Self::Crack => 5,
        }
    }

    /// Record name of the sound family.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stone => "stone",
            Self::Wood => "wood",
            Self::Metal => "metal",
            Self::Soft => "soft",
            Self::Glass => "glass",
            Self::Crack => "crack",
        }
    }

    /// Sound family by record name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "stone" => Some(Self::Stone),
            "wood" => Some(Self::Wood),
            "metal" => Some(Self::Metal),
            "soft" => Some(Self::Soft),
            "glass" => Some(Self::Glass),
            "crack" => Some(Self::Crack),
            _ => None,
        }
    }
}

/// Dust puff family; presentation only, chosen by the effects handoff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DustKind {
    /// No dust (iron, bronze, glass).
    None,
    /// Grey stone dust (stone, concrete).
    Stone,
    /// Red brick dust (brick).
    Brick,
    /// Timber chips (timber).
    Wood,
    /// Pale powder (plaster, marble, ceramic).
    Pale,
    /// Loose fibers (thatch, leather, cloth, rope).
    Fiber,
}

impl DustKind {
    /// Wire value of the dust family.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Stone => 1,
            Self::Brick => 2,
            Self::Wood => 3,
            Self::Pale => 4,
            Self::Fiber => 5,
        }
    }

    /// Record name of the dust family.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Stone => "stone",
            Self::Brick => "brick",
            Self::Wood => "wood",
            Self::Pale => "pale",
            Self::Fiber => "fiber",
        }
    }

    /// Dust family by record name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "stone" => Some(Self::Stone),
            "brick" => Some(Self::Brick),
            "wood" => Some(Self::Wood),
            "pale" => Some(Self::Pale),
            "fiber" => Some(Self::Fiber),
            _ => None,
        }
    }
}

/// Strike-spark family; presentation only, chosen by the effects handoff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SparkKind {
    /// No sparks; most rows.
    None,
    /// A few strike sparks (stone, concrete, marble).
    Few,
    /// A full spark shower (iron, bronze).
    Shower,
}

impl SparkKind {
    /// Wire value of the spark family.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Few => 1,
            Self::Shower => 2,
        }
    }

    /// Record name of the spark family.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Few => "few",
            Self::Shower => "shower",
        }
    }

    /// Spark family by record name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "none" => Some(Self::None),
            "few" => Some(Self::Few),
            "shower" => Some(Self::Shower),
            _ => None,
        }
    }
}

/// Left-behind mark family; presentation only, chosen by the decal handoff.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecalKind {
    /// Spreading crack (stone, brick, plaster, concrete).
    Crack,
    /// Surface chip (timber).
    Chip,
    /// Dull dent (iron, bronze).
    Dent,
    /// Torn edge (thatch, leather, cloth, rope).
    Tear,
    /// Shatter star (marble, glass, ceramic).
    Shatter,
    /// Burnt scorch; laid by the fire handoff over any burnt row.
    Scorch,
}

impl DecalKind {
    /// Wire value of the decal family.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Crack => 0,
            Self::Chip => 1,
            Self::Dent => 2,
            Self::Tear => 3,
            Self::Shatter => 4,
            Self::Scorch => 5,
        }
    }

    /// Record name of the decal family.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Crack => "crack",
            Self::Chip => "chip",
            Self::Dent => "dent",
            Self::Tear => "tear",
            Self::Shatter => "shatter",
            Self::Scorch => "scorch",
        }
    }

    /// Decal family by record name.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text {
            "crack" => Some(Self::Crack),
            "chip" => Some(Self::Chip),
            "dent" => Some(Self::Dent),
            "tear" => Some(Self::Tear),
            "shatter" => Some(Self::Shatter),
            "scorch" => Some(Self::Scorch),
            _ => None,
        }
    }
}

/// One material row: tick numbers plus the presentation look.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MaterialRow {
    /// Break resistance in Q16 (`1..=MAX_TOUGHNESS_Q16`); the graded-bond
    /// solver uses this as the bond strain limit.
    pub toughness_q16: u32,
    /// Shatter fraction in Q16 (`0..=MAX_BRITTLENESS_Q16`); `0` dents,
    /// `65536` shatters fully.
    pub brittleness_q16: u32,
    /// How the fracture follow-up sizes this material's cells.
    pub shard: ShardRule,
    /// Impact sound family; presentation only.
    pub sound: SoundKind,
    /// Dust puff family; presentation only.
    pub dust: DustKind,
    /// Strike-spark family; presentation only.
    pub spark: SparkKind,
    /// Left-behind mark family; presentation only.
    pub decal: DecalKind,
    /// Fuel rate in Q16 (`0..=MAX_BURN_Q16`); `0` never burns, higher burns
    /// faster for the fire follow-up.
    pub burn_q16: u32,
}

/// The versioned fourteen-row table, indexed by [`MaterialId`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MaterialTable {
    /// Table version; must be [`MATERIAL_VERSION_V1`].
    pub version: u32,
    /// One row per material in [`MaterialId`] order.
    pub rows: [MaterialRow; MATERIAL_COUNT],
}

impl MaterialTable {
    /// The shipped siege row: quarry, brickyard, timber, forge and cloth
    /// values the collapse and fire follow-ups balance against.
    ///
    /// Toughness runs glass `0.5` to iron `32.0`; thatch and cloth burn
    /// fastest, stone and metal never burn; glass shatters fully while iron
    /// only dents.
    #[must_use]
    pub const fn siege_kit() -> Self {
        Self {
            version: MATERIAL_VERSION_V1,
            rows: SIEGE_ROWS,
        }
    }
}

/// The shipped fourteen rows in [`MaterialId`] order; [`MaterialTable::siege_kit`]
/// wraps them with the version. Kept as data (not built by a fn) so the
/// constructor stays small and each material diffs on its own lines.
const SIEGE_ROWS: [MaterialRow; MATERIAL_COUNT] = [
    // Stone: hard and brittle, blocky shards, stone dust.
    MaterialRow {
        toughness_q16: 524_288,
        brittleness_q16: 55_706,
        shard: ShardRule::FewLarge,
        sound: SoundKind::Stone,
        dust: DustKind::Stone,
        spark: SparkKind::Few,
        decal: DecalKind::Crack,
        burn_q16: 0,
    },
    // Brick: mid-hard, mixed chunks, red dust.
    MaterialRow {
        toughness_q16: 327_680,
        brittleness_q16: 52_429,
        shard: ShardRule::Mixed,
        sound: SoundKind::Stone,
        dust: DustKind::Brick,
        spark: SparkKind::None,
        decal: DecalKind::Crack,
        burn_q16: 0,
    },
    // Timber: structural wood, splinters, steady burn.
    MaterialRow {
        toughness_q16: 262_144,
        brittleness_q16: 16_384,
        shard: ShardRule::Splinters,
        sound: SoundKind::Wood,
        dust: DustKind::Wood,
        spark: SparkKind::None,
        decal: DecalKind::Chip,
        burn_q16: 65_536,
    },
    // Thatch: weak sheets, fastest burn.
    MaterialRow {
        toughness_q16: 32_768,
        brittleness_q16: 6_554,
        shard: ShardRule::Sheets,
        sound: SoundKind::Soft,
        dust: DustKind::Fiber,
        spark: SparkKind::None,
        decal: DecalKind::Tear,
        burn_q16: 262_144,
    },
    // Plaster: weak facing, falls to dust.
    MaterialRow {
        toughness_q16: 98_304,
        brittleness_q16: 45_875,
        shard: ShardRule::Dust,
        sound: SoundKind::Crack,
        dust: DustKind::Pale,
        spark: SparkKind::None,
        decal: DecalKind::Crack,
        burn_q16: 0,
    },
    // Concrete: very hard, blocky shards.
    MaterialRow {
        toughness_q16: 655_360,
        brittleness_q16: 49_152,
        shard: ShardRule::FewLarge,
        sound: SoundKind::Stone,
        dust: DustKind::Stone,
        spark: SparkKind::Few,
        decal: DecalKind::Crack,
        burn_q16: 0,
    },
    // Marble: hard facing, shatters to shards.
    MaterialRow {
        toughness_q16: 458_752,
        brittleness_q16: 58_982,
        shard: ShardRule::Shards,
        sound: SoundKind::Stone,
        dust: DustKind::Pale,
        spark: SparkKind::Few,
        decal: DecalKind::Shatter,
        burn_q16: 0,
    },
    // Iron: toughest row, dents, spark shower.
    MaterialRow {
        toughness_q16: 2_097_152,
        brittleness_q16: 3_277,
        shard: ShardRule::FewLarge,
        sound: SoundKind::Metal,
        dust: DustKind::None,
        spark: SparkKind::Shower,
        decal: DecalKind::Dent,
        burn_q16: 0,
    },
    // Bronze: near-iron, dents, spark shower.
    MaterialRow {
        toughness_q16: 1_572_864,
        brittleness_q16: 5_243,
        shard: ShardRule::FewLarge,
        sound: SoundKind::Metal,
        dust: DustKind::None,
        spark: SparkKind::Shower,
        decal: DecalKind::Dent,
        burn_q16: 0,
    },
    // Leather: tough sheets, slow smoulder.
    MaterialRow {
        toughness_q16: 131_072,
        brittleness_q16: 3_277,
        shard: ShardRule::Sheets,
        sound: SoundKind::Soft,
        dust: DustKind::Fiber,
        spark: SparkKind::None,
        decal: DecalKind::Tear,
        burn_q16: 32_768,
    },
    // Cloth: weak sheets, fast burn.
    MaterialRow {
        toughness_q16: 49_152,
        brittleness_q16: 3_277,
        shard: ShardRule::Sheets,
        sound: SoundKind::Soft,
        dust: DustKind::Fiber,
        spark: SparkKind::None,
        decal: DecalKind::Tear,
        burn_q16: 196_608,
    },
    // Rope: frays to fibers, steady burn.
    MaterialRow {
        toughness_q16: 196_608,
        brittleness_q16: 3_277,
        shard: ShardRule::Splinters,
        sound: SoundKind::Soft,
        dust: DustKind::Fiber,
        spark: SparkKind::None,
        decal: DecalKind::Tear,
        burn_q16: 131_072,
    },
    // Glass: weakest hard row, fully shattering.
    MaterialRow {
        toughness_q16: 32_768,
        brittleness_q16: 65_536,
        shard: ShardRule::Shards,
        sound: SoundKind::Glass,
        dust: DustKind::None,
        spark: SparkKind::None,
        decal: DecalKind::Shatter,
        burn_q16: 0,
    },
    // Ceramic: mid-soft tile, shatters to shards.
    MaterialRow {
        toughness_q16: 163_840,
        brittleness_q16: 62_259,
        shard: ShardRule::Shards,
        sound: SoundKind::Glass,
        dust: DustKind::Pale,
        spark: SparkKind::None,
        decal: DecalKind::Shatter,
        burn_q16: 0,
    },
];

impl MaterialTable {
    /// Row for a material.
    #[must_use]
    pub fn row_of(self, id: MaterialId) -> MaterialRow {
        self.rows[id.index()]
    }

    /// Bond strain limit for a material in Q16: its toughness. The
    /// graded-bond solver stamps one of these per bond from the bond's
    /// material id, so the table stays the single owner of the numbers.
    #[must_use]
    pub fn breaking_strain_q16(self, id: MaterialId) -> u32 {
        self.rows[id.index()].toughness_q16
    }

    /// Whether the fire follow-up can ignite this material.
    #[must_use]
    pub fn is_combustible(self, id: MaterialId) -> bool {
        self.rows[id.index()].burn_q16 > 0
    }

    /// Content digest over the table bytes in canonical material order.
    ///
    /// The break-tool verbs pin this: the same rows give the same digest on
    /// every PC, and any row change flips it. [`digest_rows`] is the one
    /// loop, so a material set of any length folds the same stream.
    #[must_use]
    pub fn digest(self) -> u64 {
        digest_rows(self.version, &self.rows)
    }

    /// Range-check the table; see [`validate`] for the rules.
    pub fn validate(self) -> Result<(), MaterialError> {
        validate(self)
    }
}

/// Content digest over `version` and `rows` in order: the stream
/// [`MaterialTable::digest`] folds, shared with the material record set so
/// the two never disagree. Pure integer math; no heap.
#[must_use]
pub fn digest_rows(version: u32, rows: &[MaterialRow]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(version);
    for row in rows {
        hasher.write_u32(row.toughness_q16);
        hasher.write_u32(row.brittleness_q16);
        hasher.write_u8(row.shard.as_u8());
        hasher.write_u8(row.sound.as_u8());
        hasher.write_u8(row.dust.as_u8());
        hasher.write_u8(row.spark.as_u8());
        hasher.write_u8(row.decal.as_u8());
        hasher.write_u32(row.burn_q16);
    }
    hasher.finish()
}

/// Bounded material failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaterialError {
    /// Table set carries version 0, reserved and never valid anew.
    BadVersion(u32),
    /// Version tag with no known rows.
    UnknownVersion(u32),
    /// Toughness is zero or above the cap; material and culprit carried.
    /// First failure wins in material-id order.
    BadToughness {
        /// Culprit material wire value.
        material: u8,
        /// Culprit Q16 value.
        value: u32,
    },
    /// Brittleness is above the full-shatter cap; material and culprit
    /// carried.
    BadBrittleness {
        /// Culprit material wire value.
        material: u8,
        /// Culprit Q16 value.
        value: u32,
    },
    /// Burn rate is above the cap; material and culprit carried.
    BadBurn {
        /// Culprit material wire value.
        material: u8,
        /// Culprit Q16 value.
        value: u32,
    },
    /// A bond material byte names no material; bond index and culprit
    /// carried.
    BadBondMaterial {
        /// Index into the bond material array.
        index: usize,
        /// Culprit material byte.
        value: u8,
    },
    /// A chunk material byte names no material; chunk index and culprit
    /// carried.
    BadChunkMaterial {
        /// Index into the chunk material array.
        index: usize,
        /// Culprit material byte.
        value: u8,
    },
    /// A material byte names no material.
    UnknownMaterial(u8),
}

impl core::fmt::Display for MaterialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(version) => {
                write!(f, "material table version 0 is reserved ({version})")
            }
            Self::UnknownVersion(version) => {
                write!(f, "unknown material table version ({version})")
            }
            Self::BadToughness { material, value } => {
                write!(f, "material {material} toughness out of range ({value})")
            }
            Self::BadBrittleness { material, value } => {
                write!(f, "material {material} brittleness out of range ({value})")
            }
            Self::BadBurn { material, value } => {
                write!(f, "material {material} burn rate out of range ({value})")
            }
            Self::BadBondMaterial { index, value } => {
                write!(f, "bond {index} names no material ({value})")
            }
            Self::BadChunkMaterial { index, value } => {
                write!(f, "chunk {index} names no material ({value})")
            }
            Self::UnknownMaterial(value) => {
                write!(f, "no material named ({value})")
            }
        }
    }
}

impl std::error::Error for MaterialError {}

/// Range-check a versioned material table. First failure wins, in canonical
/// material order.
///
/// Rules: version is [`MATERIAL_VERSION_V1`]; every toughness stays in
/// `1..=MAX_TOUGHNESS_Q16` (zero would make a bond that breaks untouched);
/// every brittleness stays in `0..=MAX_BRITTLENESS_Q16`; every burn rate
/// stays in `0..=MAX_BURN_Q16`.
pub fn validate(table: MaterialTable) -> Result<(), MaterialError> {
    if table.version == 0 {
        return Err(MaterialError::BadVersion(table.version));
    }
    if table.version != MATERIAL_VERSION_V1 {
        return Err(MaterialError::UnknownVersion(table.version));
    }
    let ids = MaterialId::all();
    let mut step: usize = 0;
    while step < MATERIAL_COUNT {
        let id = ids[step];
        let row = table.rows[step];
        if row.toughness_q16 == 0 || row.toughness_q16 > MAX_TOUGHNESS_Q16 {
            return Err(MaterialError::BadToughness {
                material: id.as_u8(),
                value: row.toughness_q16,
            });
        }
        if row.brittleness_q16 > MAX_BRITTLENESS_Q16 {
            return Err(MaterialError::BadBrittleness {
                material: id.as_u8(),
                value: row.brittleness_q16,
            });
        }
        if row.burn_q16 > MAX_BURN_Q16 {
            return Err(MaterialError::BadBurn {
                material: id.as_u8(),
                value: row.burn_q16,
            });
        }
        step += 1;
    }
    Ok(())
}

/// Check a bond material-id array: one byte per bond in bond order. Every
/// byte must name a [`MaterialId`]; the first unknown byte fails with its
/// bond index. An empty array is valid (a prop with no bonds).
pub fn validate_bond_materials(ids: &[u8]) -> Result<(), MaterialError> {
    check_material_ids(ids, true)
}

/// Check a baked-chunk material-id array: one byte per chunk in chunk order.
/// Every byte must name a [`MaterialId`]; the first unknown byte fails with
/// its chunk index. An empty array is valid (an empty bake).
pub fn validate_chunk_materials(ids: &[u8]) -> Result<(), MaterialError> {
    check_material_ids(ids, false)
}

/// Shared id-array checker behind both validators. `bond_side` selects the
/// error family so bond and chunk arrays report their own culprit without a
/// second copy of the loop.
fn check_material_ids(ids: &[u8], bond_side: bool) -> Result<(), MaterialError> {
    let mut index: usize = 0;
    while index < ids.len() {
        if MaterialId::try_from(ids[index]).is_err() {
            if bond_side {
                return Err(MaterialError::BadBondMaterial {
                    index,
                    value: ids[index],
                });
            }
            return Err(MaterialError::BadChunkMaterial {
                index,
                value: ids[index],
            });
        }
        index += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// First byte past the valid wire range: every id array refuses it.
    const UNKNOWN_WIRE: u8 = 14;

    #[test]
    fn shipped_kit_validates() {
        assert!(MaterialTable::siege_kit().validate().is_ok());
    }

    #[test]
    fn digest_repeats_and_is_nonzero() {
        let table = MaterialTable::siege_kit();
        assert_eq!(table.digest(), table.digest());
        assert_ne!(table.digest(), 0);
    }

    #[test]
    fn digest_sensitive_to_every_field() {
        let table = MaterialTable::siege_kit();
        let stone = MaterialId::Stone.index();
        let mut toughness_flip = table;
        toughness_flip.rows[stone].toughness_q16 += 1;
        assert_ne!(toughness_flip.digest(), table.digest());
        let mut brittle_flip = table;
        brittle_flip.rows[stone].brittleness_q16 += 1;
        assert_ne!(brittle_flip.digest(), table.digest());
        let mut shard_flip = table;
        shard_flip.rows[stone].shard = ShardRule::Dust;
        assert_ne!(shard_flip.digest(), table.digest());
        let mut sound_flip = table;
        sound_flip.rows[stone].sound = SoundKind::Glass;
        assert_ne!(sound_flip.digest(), table.digest());
        let mut dust_flip = table;
        dust_flip.rows[stone].dust = DustKind::Fiber;
        assert_ne!(dust_flip.digest(), table.digest());
        let mut spark_flip = table;
        spark_flip.rows[stone].spark = SparkKind::Shower;
        assert_ne!(spark_flip.digest(), table.digest());
        let mut decal_flip = table;
        decal_flip.rows[stone].decal = DecalKind::Shatter;
        assert_ne!(decal_flip.digest(), table.digest());
        let mut burn_flip = table;
        burn_flip.rows[stone].burn_q16 += 1;
        assert_ne!(burn_flip.digest(), table.digest());
    }

    #[test]
    fn digest_moves_when_rows_swap_places() {
        let table = MaterialTable::siege_kit();
        let mut swapped = table;
        let iron = MaterialId::Iron.index();
        let glass = MaterialId::Glass.index();
        swapped.rows.swap(iron, glass);
        assert_ne!(swapped.digest(), table.digest());
    }

    #[test]
    fn validator_refuses_bad_versions() {
        let mut zero = MaterialTable::siege_kit();
        zero.version = 0;
        assert_eq!(zero.validate(), Err(MaterialError::BadVersion(0)));
        let mut future = MaterialTable::siege_kit();
        future.version = MATERIAL_VERSION_V1 + 1;
        assert_eq!(
            future.validate(),
            Err(MaterialError::UnknownVersion(MATERIAL_VERSION_V1 + 1))
        );
    }

    #[test]
    fn validator_refuses_out_of_range_rows() {
        let mut soft = MaterialTable::siege_kit();
        soft.rows[MaterialId::Brick.index()].toughness_q16 = 0;
        assert_eq!(
            soft.validate(),
            Err(MaterialError::BadToughness {
                material: MaterialId::Brick.as_u8(),
                value: 0,
            })
        );
        let mut hard = MaterialTable::siege_kit();
        hard.rows[MaterialId::Brick.index()].toughness_q16 = MAX_TOUGHNESS_Q16 + 1;
        assert_eq!(
            hard.validate(),
            Err(MaterialError::BadToughness {
                material: MaterialId::Brick.as_u8(),
                value: MAX_TOUGHNESS_Q16 + 1,
            })
        );
        let mut shatter = MaterialTable::siege_kit();
        shatter.rows[MaterialId::Timber.index()].brittleness_q16 = MAX_BRITTLENESS_Q16 + 1;
        assert_eq!(
            shatter.validate(),
            Err(MaterialError::BadBrittleness {
                material: MaterialId::Timber.as_u8(),
                value: MAX_BRITTLENESS_Q16 + 1,
            })
        );
        let mut inferno = MaterialTable::siege_kit();
        inferno.rows[MaterialId::Cloth.index()].burn_q16 = MAX_BURN_Q16 + 1;
        assert_eq!(
            inferno.validate(),
            Err(MaterialError::BadBurn {
                material: MaterialId::Cloth.as_u8(),
                value: MAX_BURN_Q16 + 1,
            })
        );
    }

    #[test]
    fn bond_materials_name_every_bond_or_fail_at_the_culprit() {
        let ok = [
            MaterialId::Stone.as_u8(),
            MaterialId::Timber.as_u8(),
            MaterialId::Iron.as_u8(),
        ];
        assert!(validate_bond_materials(&ok).is_ok());
        assert!(validate_bond_materials(&[]).is_ok());
        let bad = [MaterialId::Stone.as_u8(), UNKNOWN_WIRE];
        assert_eq!(
            validate_bond_materials(&bad),
            Err(MaterialError::BadBondMaterial {
                index: 1,
                value: UNKNOWN_WIRE,
            })
        );
    }

    #[test]
    fn chunk_materials_name_every_chunk_or_fail_at_the_culprit() {
        let ok = [MaterialId::Glass.as_u8(), MaterialId::Ceramic.as_u8()];
        assert!(validate_chunk_materials(&ok).is_ok());
        assert!(validate_chunk_materials(&[]).is_ok());
        let bad = [UNKNOWN_WIRE];
        assert_eq!(
            validate_chunk_materials(&bad),
            Err(MaterialError::BadChunkMaterial {
                index: 0,
                value: UNKNOWN_WIRE,
            })
        );
    }

    #[test]
    fn breaking_strain_orders_glass_below_timber_below_stone_below_iron() {
        let table = MaterialTable::siege_kit();
        let glass = table.breaking_strain_q16(MaterialId::Glass);
        let timber = table.breaking_strain_q16(MaterialId::Timber);
        let stone = table.breaking_strain_q16(MaterialId::Stone);
        let iron = table.breaking_strain_q16(MaterialId::Iron);
        assert!(glass < timber);
        assert!(timber < stone);
        assert!(stone < iron);
        assert_eq!(stone, table.row_of(MaterialId::Stone).toughness_q16);
    }

    #[test]
    fn combustible_set_covers_thatch_cloth_timber_rope_leather_only() {
        let table = MaterialTable::siege_kit();
        for id in [
            MaterialId::Thatch,
            MaterialId::Cloth,
            MaterialId::Timber,
            MaterialId::Rope,
            MaterialId::Leather,
        ] {
            assert!(table.is_combustible(id));
        }
        for id in [
            MaterialId::Stone,
            MaterialId::Brick,
            MaterialId::Plaster,
            MaterialId::Concrete,
            MaterialId::Marble,
            MaterialId::Iron,
            MaterialId::Bronze,
            MaterialId::Glass,
            MaterialId::Ceramic,
        ] {
            assert!(!table.is_combustible(id));
        }
    }

    #[test]
    fn material_ids_roundtrip_and_refuse_unknown_bytes() {
        for id in MaterialId::all() {
            assert_eq!(MaterialId::try_from(id.as_u8()), Ok(id));
        }
        assert_eq!(
            MaterialId::try_from(UNKNOWN_WIRE),
            Err(MaterialError::UnknownMaterial(UNKNOWN_WIRE))
        );
    }

    #[test]
    fn record_names_round_trip_every_variant_and_refuse_strangers() {
        for id in MaterialId::all() {
            assert_eq!(MaterialId::from_name(id.name()), Some(id));
        }
        assert_eq!(MaterialId::from_name("granite"), None);
        for shard in [
            ShardRule::FewLarge,
            ShardRule::Mixed,
            ShardRule::Splinters,
            ShardRule::Shards,
            ShardRule::Dust,
            ShardRule::Sheets,
        ] {
            assert_eq!(ShardRule::from_name(shard.name()), Some(shard));
        }
        for sound in [
            SoundKind::Stone,
            SoundKind::Wood,
            SoundKind::Metal,
            SoundKind::Soft,
            SoundKind::Glass,
            SoundKind::Crack,
        ] {
            assert_eq!(SoundKind::from_name(sound.name()), Some(sound));
        }
        for dust in [
            DustKind::None,
            DustKind::Stone,
            DustKind::Brick,
            DustKind::Wood,
            DustKind::Pale,
            DustKind::Fiber,
        ] {
            assert_eq!(DustKind::from_name(dust.name()), Some(dust));
        }
        for spark in [SparkKind::None, SparkKind::Few, SparkKind::Shower] {
            assert_eq!(SparkKind::from_name(spark.name()), Some(spark));
        }
        for decal in [
            DecalKind::Crack,
            DecalKind::Chip,
            DecalKind::Dent,
            DecalKind::Tear,
            DecalKind::Shatter,
            DecalKind::Scorch,
        ] {
            assert_eq!(DecalKind::from_name(decal.name()), Some(decal));
        }
        assert_eq!(ShardRule::from_name("Shards"), None);
        assert_eq!(DecalKind::from_name(""), None);
    }

    #[test]
    fn digest_rows_is_the_table_digest_for_the_shipped_rows() {
        let table = MaterialTable::siege_kit();
        assert_eq!(digest_rows(table.version, &table.rows), table.digest());
        assert_ne!(
            digest_rows(table.version, &table.rows[..13]),
            table.digest()
        );
        assert_ne!(digest_rows(table.version + 1, &table.rows), table.digest());
        assert_ne!(digest_rows(table.version, &[]), 0);
    }
}

//! anatomy: joint graph plus tissue and armor tables for deterministic cuts.
//!
//! Consumer: the cut resolver reads an [`AnatomyTable`] (joint parents,
//! per-segment tissue toughness, per-joint armor coverage) and compares blade
//! energy against [`AnatomyTable::protection_q16`]; agents author tables
//! through the break-tool verbs and prove them with [`AnatomyTable::digest`]. Data
//! owner: `engine-combat`. Units: toughness and guards in Q16 fixed point
//! (`1.0 == 65536`), joints as [`JointId`] slots, armor as bit flags.
//! Determinism scope: [`validate`] and [`AnatomyTable::digest`] are pure over
//! the table bytes in joint-id order; the same table gives the same digest on
//! every PC, and validation reports the first failure in joint-id order.
//! Errors: [`AnatomyError`] for bad versions, parents, tissue and coverage;
//! no function panics on user data. Budget: validate and digest are
//! `O(JOINT_TOTAL)` integer ops with no heap; queries are `O(1)`.
//! Presentation (stump caps, blood) never reads this module: the cut event
//! carries the joint, so a slow GPU cannot change an outcome.

use engine_core::Fnv64;

/// Table version of the shipped Rome kit row.
pub const ANATOMY_VERSION_V1: u32 = 1;
/// Severable joints in the humanoid graph (every joint but the pelvis root).
pub const SEVERABLE_JOINT_COUNT: usize = 15;
/// Joints in the graph including the non-severable pelvis root.
pub const JOINT_TOTAL: usize = 16;
/// Parent byte marking the pelvis root, which has no parent.
pub const NO_PARENT: u8 = 0xFF;
/// Per-layer toughness cap in Q16 (`64.0`); anything tougher is data error.
pub const MAX_TOUGHNESS_Q16: u32 = 64 << 16;
/// Skin toughness in Q16 (`1.0`): blade meets it first, stops nothing alone.
pub const SKIN_Q16: u32 = 65_536;
/// Flesh toughness in Q16 (`2.0`): muscle slows the edge.
pub const FLESH_Q16: u32 = 131_072;
/// Bone toughness in Q16 (`6.0`): the edge must earn the joint.
pub const BONE_Q16: u32 = 393_216;

/// Galea (helmet) coverage bit: guards the neck.
pub const ARMOR_GALEA: u8 = 0x01;
/// Manica (arm guard) coverage bit: guards the sword-arm joints.
pub const ARMOR_MANICA: u8 = 0x02;
/// Greave (shin guard) coverage bit: guards the leading-leg joints.
pub const ARMOR_GREAVE: u8 = 0x04;
/// Lorica (body armor) coverage bit: guards the torso bisects.
pub const ARMOR_LORICA: u8 = 0x08;
/// Scutum (carried shield) coverage bit: guards the shield-arm joints.
pub const ARMOR_SCUTUM: u8 = 0x10;
/// Every known armor bit; coverage rows must stay inside it.
pub const ARMOR_ALL: u8 = 0x1F;

/// Galea guard in Q16 (`6.0`): a helmet turns a neck cut.
pub const GALEA_GUARD_Q16: u32 = 393_216;
/// Manica guard in Q16 (`5.0`): layered plates over the sword arm.
pub const MANICA_GUARD_Q16: u32 = 327_680;
/// Greave guard in Q16 (`5.0`): bronze over the leading shin.
pub const GREAVE_GUARD_Q16: u32 = 327_680;
/// Lorica guard in Q16 (`4.0`): mail over the torso.
pub const LORICA_GUARD_Q16: u32 = 262_144;
/// Scutum guard in Q16 (`8.0`): a shield wall over the shield arm.
pub const SCUTUM_GUARD_Q16: u32 = 524_288;

/// One joint in the humanoid graph, in canonical digest order.
///
/// The pelvis is the non-severable root anchor; the other fifteen joints are
/// severable, counting the plan's named set (neck, shoulders, elbows,
/// wrists, hips, knees, ankles, waist bisect) plus the chest bisect so the
/// torso cuts in two documented places.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JointId {
    /// Root anchor; never severable, parents the waist and hips.
    Pelvis,
    /// Lower-torso bisect; parents the chest.
    Waist,
    /// Upper-torso bisect; parents the neck and shoulders.
    Chest,
    /// Head joint; covered by the galea.
    Neck,
    /// Shield-arm shoulder; covered by the scutum.
    ShoulderL,
    /// Sword-arm shoulder; covered by the manica.
    ShoulderR,
    /// Shield-arm elbow; covered by the scutum.
    ElbowL,
    /// Sword-arm elbow; covered by the manica.
    ElbowR,
    /// Shield-arm wrist; covered by the scutum.
    WristL,
    /// Sword-arm wrist; covered by the manica.
    WristR,
    /// Bare in the Rome row: a cuttable hip.
    HipL,
    /// Bare in the Rome row: a cuttable hip.
    HipR,
    /// Leading knee; covered by the greave.
    KneeL,
    /// Bare in the Rome row: a cuttable knee.
    KneeR,
    /// Leading ankle; covered by the greave.
    AnkleL,
    /// Bare in the Rome row: a cuttable ankle.
    AnkleR,
}

impl JointId {
    /// Canonical index: discriminant order, the digest and validation order.
    #[must_use]
    pub fn index(self) -> usize {
        usize::from(self.as_u8())
    }

    /// Wire value of the joint, `0..JOINT_TOTAL`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Pelvis => 0,
            Self::Waist => 1,
            Self::Chest => 2,
            Self::Neck => 3,
            Self::ShoulderL => 4,
            Self::ShoulderR => 5,
            Self::ElbowL => 6,
            Self::ElbowR => 7,
            Self::WristL => 8,
            Self::WristR => 9,
            Self::HipL => 10,
            Self::HipR => 11,
            Self::KneeL => 12,
            Self::KneeR => 13,
            Self::AnkleL => 14,
            Self::AnkleR => 15,
        }
    }

    /// Decode a wire value; unknown values are `None`, never a panic.
    #[must_use]
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Pelvis),
            1 => Some(Self::Waist),
            2 => Some(Self::Chest),
            3 => Some(Self::Neck),
            4 => Some(Self::ShoulderL),
            5 => Some(Self::ShoulderR),
            6 => Some(Self::ElbowL),
            7 => Some(Self::ElbowR),
            8 => Some(Self::WristL),
            9 => Some(Self::WristR),
            10 => Some(Self::HipL),
            11 => Some(Self::HipR),
            12 => Some(Self::KneeL),
            13 => Some(Self::KneeR),
            14 => Some(Self::AnkleL),
            15 => Some(Self::AnkleR),
            _ => None,
        }
    }

    /// Every joint in canonical order, for order-proof iteration.
    #[must_use]
    pub const fn all() -> [Self; JOINT_TOTAL] {
        [
            Self::Pelvis,
            Self::Waist,
            Self::Chest,
            Self::Neck,
            Self::ShoulderL,
            Self::ShoulderR,
            Self::ElbowL,
            Self::ElbowR,
            Self::WristL,
            Self::WristR,
            Self::HipL,
            Self::HipR,
            Self::KneeL,
            Self::KneeR,
            Self::AnkleL,
            Self::AnkleR,
        ]
    }

    /// Whether a cut may detach here; only the pelvis root answers false.
    #[must_use]
    pub const fn is_severable(self) -> bool {
        !matches!(self, Self::Pelvis)
    }
}

/// Three tissue layers of one segment, each a Q16 toughness.
///
/// The cut compares blade energy against skin plus flesh plus bone plus
/// armor, so a bare segment still resists by its body while an armored one
/// adds its guard on top.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TissueRow {
    /// Skin toughness in Q16, `0..=MAX_TOUGHNESS_Q16`.
    pub skin_q16: u32,
    /// Flesh toughness in Q16, `0..=MAX_TOUGHNESS_Q16`.
    pub flesh_q16: u32,
    /// Bone toughness in Q16, `0..=MAX_TOUGHNESS_Q16`.
    pub bone_q16: u32,
}

impl TissueRow {
    /// The shipped V1 flesh: skin `1.0`, flesh `2.0`, bone `6.0`.
    pub const fn v1() -> Self {
        Self {
            skin_q16: SKIN_Q16,
            flesh_q16: FLESH_Q16,
            bone_q16: BONE_Q16,
        }
    }

    /// Combined tissue toughness in Q16, widened so the sum cannot wrap.
    #[must_use]
    pub fn total_q16(self) -> u64 {
        u64::from(self.skin_q16) + u64::from(self.flesh_q16) + u64::from(self.bone_q16)
    }
}

/// Versioned anatomy table: joint graph plus tissue plus armor coverage.
///
/// Arrays run in [`JointId`] canonical order; `parents` holds the parent
/// wire value ([`JointId::as_u8`]) or [`NO_PARENT`] for the pelvis root,
/// `cover` holds armor bits from the `ARMOR_*` set. Fixed arrays, `Copy`,
/// no heap: the tick shares this by reference and never allocates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AnatomyTable {
    /// Table version; nonzero, 1 for the shipped Rome row.
    pub version: u32,
    /// Parent per joint in canonical order; root carries [`NO_PARENT`].
    pub parents: [u8; JOINT_TOTAL],
    /// Tissue per joint in canonical order; no row may be all zero.
    pub tissue: [TissueRow; JOINT_TOTAL],
    /// Armor coverage per joint in canonical order, bits of `ARMOR_*`.
    pub cover: [u8; JOINT_TOTAL],
}

impl AnatomyTable {
    /// The shipped Rome kit row: galea on the neck, manica on the
    /// sword arm, single greave on the leading leg, lorica on the torso,
    /// scutum on the shield arm; hips and the off-side knee and ankle stay
    /// bare so a blade earns its cut there.
    #[must_use]
    pub const fn rome_kit() -> Self {
        let flesh = TissueRow::v1();
        Self {
            version: ANATOMY_VERSION_V1,
            parents: [
                NO_PARENT, // Pelvis: root anchor.
                0,         // Waist: pelvis.
                1,         // Chest: waist.
                2,         // Neck: chest.
                2,         // ShoulderL: chest.
                2,         // ShoulderR: chest.
                4,         // ElbowL: shoulder L.
                5,         // ElbowR: shoulder R.
                6,         // WristL: elbow L.
                7,         // WristR: elbow R.
                0,         // HipL: pelvis.
                0,         // HipR: pelvis.
                10,        // KneeL: hip L.
                11,        // KneeR: hip R.
                12,        // AnkleL: knee L.
                13,        // AnkleR: knee R.
            ],
            tissue: [flesh; JOINT_TOTAL],
            cover: [
                0,            // Pelvis: bare anchor under the belt line.
                ARMOR_LORICA, // Waist.
                ARMOR_LORICA, // Chest.
                ARMOR_GALEA,  // Neck.
                ARMOR_SCUTUM, // ShoulderL.
                ARMOR_MANICA, // ShoulderR.
                ARMOR_SCUTUM, // ElbowL.
                ARMOR_MANICA, // ElbowR.
                ARMOR_SCUTUM, // WristL.
                ARMOR_MANICA, // WristR.
                0,            // HipL: bare.
                0,            // HipR: bare.
                ARMOR_GREAVE, // KneeL.
                0,            // KneeR: bare.
                ARMOR_GREAVE, // AnkleL.
                0,            // AnkleR: bare.
            ],
        }
    }

    /// Cut resistance of a joint under a worn set, in Q16.
    ///
    /// Tissue total plus one guard per piece that both covers the joint and
    /// is worn, summed in fixed piece order so the answer is canonical.
    /// `worn` bits outside [`ARMOR_ALL`] are ignored, never an error.
    #[must_use]
    pub fn protection_q16(self, joint: JointId, worn: u8) -> u64 {
        let row = self.tissue[joint.index()];
        let armed = self.cover[joint.index()] & worn;
        let mut guard: u64 = 0;
        if armed & ARMOR_GALEA != 0 {
            guard += u64::from(GALEA_GUARD_Q16);
        }
        if armed & ARMOR_MANICA != 0 {
            guard += u64::from(MANICA_GUARD_Q16);
        }
        if armed & ARMOR_GREAVE != 0 {
            guard += u64::from(GREAVE_GUARD_Q16);
        }
        if armed & ARMOR_LORICA != 0 {
            guard += u64::from(LORICA_GUARD_Q16);
        }
        if armed & ARMOR_SCUTUM != 0 {
            guard += u64::from(SCUTUM_GUARD_Q16);
        }
        row.total_q16() + guard
    }

    /// Whether a worn set carries plate (the lorica segmentata, FEEL4) over
    /// this joint. Only the lorica is plate; the galea, manica, greave and
    /// scutum guard by [`AnatomyTable::protection_q16`] and never deflect.
    #[must_use]
    pub fn plated(self, joint: JointId, worn: u8) -> bool {
        self.cover[joint.index()] & worn & ARMOR_LORICA != 0
    }

    /// Content digest over the table bytes in canonical joint order.
    ///
    /// The break-tool verbs and BreakBench pin this: the same rows give the same
    /// digest on every PC, and any row change flips it.
    #[must_use]
    pub fn digest(self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u32(self.version);
        let mut joint: usize = 0;
        while joint < JOINT_TOTAL {
            hasher.write_u8(self.parents[joint]);
            hasher.write_u32(self.tissue[joint].skin_q16);
            hasher.write_u32(self.tissue[joint].flesh_q16);
            hasher.write_u32(self.tissue[joint].bone_q16);
            hasher.write_u8(self.cover[joint]);
            joint += 1;
        }
        hasher.finish()
    }

    /// Range-check the table; see [`validate`] for the rules.
    pub fn validate(self) -> Result<(), AnatomyError> {
        validate(self)
    }
}

/// Bounded anatomy failures; invalid user data never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnatomyError {
    /// Table set carries version 0, reserved and never valid anew.
    BadVersion(u32),
    /// Version tag with no known rows.
    UnknownVersion(u32),
    /// The pelvis root must carry [`NO_PARENT`]; culprit byte carried.
    RootHasParent(u8),
    /// A non-root joint carries [`NO_PARENT`] and hangs off the graph.
    Detached {
        /// Culprit joint wire value.
        joint: u8,
    },
    /// A parent byte names no joint; joint and culprit carried.
    BadParent {
        /// Culprit joint wire value.
        joint: u8,
        /// Parent byte that names no joint.
        parent: u8,
    },
    /// Parent links loop back before reaching the root; joint carried.
    Cyclic {
        /// Joint whose ancestor walk never reaches the root.
        joint: u8,
    },
    /// A tissue layer exceeds [`MAX_TOUGHNESS_Q16`]; joint, layer and
    /// culprit carried. First failure wins in skin, flesh, bone order.
    BadToughness {
        /// Culprit joint wire value.
        joint: u8,
        /// Layer name, `"skin"`, `"flesh"` or `"bone"`.
        layer: &'static str,
        /// Culprit Q16 value.
        value: u32,
    },
    /// All three layers are zero: a gap no cut can ever resolve.
    GapTissue {
        /// Culprit joint wire value.
        joint: u8,
    },
    /// Coverage names unknown armor bits; joint and culprit carried.
    BadCover {
        /// Culprit joint wire value.
        joint: u8,
        /// Culprit coverage byte.
        value: u8,
    },
}

impl core::fmt::Display for AnatomyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(version) => {
                write!(f, "anatomy table version 0 is reserved ({version})")
            }
            Self::UnknownVersion(version) => {
                write!(f, "unknown anatomy table version ({version})")
            }
            Self::RootHasParent(parent) => {
                write!(f, "anatomy root must have no parent ({parent})")
            }
            Self::Detached { joint } => {
                write!(f, "anatomy joint {joint} hangs off the graph")
            }
            Self::BadParent { joint, parent } => {
                write!(f, "anatomy joint {joint} names no parent ({parent})")
            }
            Self::Cyclic { joint } => {
                write!(f, "anatomy parents loop above joint {joint}")
            }
            Self::BadToughness {
                joint,
                layer,
                value,
            } => {
                write!(
                    f,
                    "anatomy joint {joint} layer {layer} out of range ({value})"
                )
            }
            Self::GapTissue { joint } => {
                write!(f, "anatomy joint {joint} has no tissue to cut")
            }
            Self::BadCover { joint, value } => {
                write!(f, "anatomy joint {joint} names unknown armor ({value})")
            }
        }
    }
}

impl std::error::Error for AnatomyError {}

/// Range-check a versioned anatomy table. First failure wins, in canonical
/// joint order.
///
/// Rules: version is [`ANATOMY_VERSION_V1`]; the pelvis root carries
/// [`NO_PARENT`]; every other joint reaches the root through parent links
/// within `JOINT_TOTAL` steps (a longer walk is a cycle, a [`NO_PARENT`]
/// mid-walk is a detach, a byte past the joint range is a bad parent); each
/// tissue layer stays within [`MAX_TOUGHNESS_Q16`] with at least one layer
/// nonzero (an all-zero row is an unarmored gap, not a bare segment); each
/// coverage byte stays inside [`ARMOR_ALL`].
pub fn validate(table: AnatomyTable) -> Result<(), AnatomyError> {
    if table.version == 0 {
        return Err(AnatomyError::BadVersion(table.version));
    }
    if table.version != ANATOMY_VERSION_V1 {
        return Err(AnatomyError::UnknownVersion(table.version));
    }
    if table.parents[JointId::Pelvis.index()] != NO_PARENT {
        return Err(AnatomyError::RootHasParent(
            table.parents[JointId::Pelvis.index()],
        ));
    }
    let joints = JointId::all();
    let mut step: usize = 0;
    while step < JOINT_TOTAL {
        let joint = joints[step];
        if joint.is_severable() {
            // Walk to the root; a walk longer than the joint count repeats a
            // joint, so it is a cycle by the pigeonhole principle.
            let mut current = joint;
            let mut depth: usize = 0;
            while current != JointId::Pelvis {
                if depth >= JOINT_TOTAL {
                    return Err(AnatomyError::Cyclic {
                        joint: joint.as_u8(),
                    });
                }
                let parent_byte = table.parents[current.index()];
                if parent_byte == NO_PARENT {
                    return Err(AnatomyError::Detached {
                        joint: joint.as_u8(),
                    });
                }
                match JointId::from_u8(parent_byte) {
                    Some(parent) => current = parent,
                    None => {
                        return Err(AnatomyError::BadParent {
                            joint: joint.as_u8(),
                            parent: parent_byte,
                        });
                    }
                }
                depth += 1;
            }
        }
        let row = table.tissue[step];
        if row.skin_q16 > MAX_TOUGHNESS_Q16 {
            return Err(AnatomyError::BadToughness {
                joint: joint.as_u8(),
                layer: "skin",
                value: row.skin_q16,
            });
        }
        if row.flesh_q16 > MAX_TOUGHNESS_Q16 {
            return Err(AnatomyError::BadToughness {
                joint: joint.as_u8(),
                layer: "flesh",
                value: row.flesh_q16,
            });
        }
        if row.bone_q16 > MAX_TOUGHNESS_Q16 {
            return Err(AnatomyError::BadToughness {
                joint: joint.as_u8(),
                layer: "bone",
                value: row.bone_q16,
            });
        }
        if row.skin_q16 == 0 && row.flesh_q16 == 0 && row.bone_q16 == 0 {
            return Err(AnatomyError::GapTissue {
                joint: joint.as_u8(),
            });
        }
        if table.cover[step] & !ARMOR_ALL != 0 {
            return Err(AnatomyError::BadCover {
                joint: joint.as_u8(),
                value: table.cover[step],
            });
        }
        step += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cyclic() -> AnatomyTable {
        let mut table = AnatomyTable::rome_kit();
        // Shoulder R and elbow R parent each other: neither reaches the root.
        table.parents[JointId::ShoulderR.index()] = JointId::ElbowR.as_u8();
        table
    }

    fn gap() -> AnatomyTable {
        let mut table = AnatomyTable::rome_kit();
        table.tissue[JointId::KneeR.index()] = TissueRow {
            skin_q16: 0,
            flesh_q16: 0,
            bone_q16: 0,
        };
        table
    }

    #[test]
    fn rome_row_validates_and_covers_all_five_armor_pieces() {
        let table = AnatomyTable::rome_kit();
        assert_eq!(table.validate(), Ok(()));
        assert_eq!(table.version, ANATOMY_VERSION_V1);
        // Every armor piece guards at least one joint, at its named home.
        assert_eq!(table.cover[JointId::Neck.index()], ARMOR_GALEA);
        assert_eq!(table.cover[JointId::ElbowR.index()], ARMOR_MANICA);
        assert_eq!(table.cover[JointId::WristR.index()], ARMOR_MANICA);
        assert_eq!(table.cover[JointId::KneeL.index()], ARMOR_GREAVE);
        assert_eq!(table.cover[JointId::Waist.index()], ARMOR_LORICA);
        assert_eq!(table.cover[JointId::Chest.index()], ARMOR_LORICA);
        assert_eq!(table.cover[JointId::ElbowL.index()], ARMOR_SCUTUM);
        let mut seen: u8 = 0;
        for joint in JointId::all() {
            seen |= table.cover[joint.index()];
        }
        assert_eq!(seen, ARMOR_ALL);
        // Hips and the off-side knee and ankle stay bare for honest cuts.
        assert_eq!(table.cover[JointId::HipL.index()], 0);
        assert_eq!(table.cover[JointId::KneeR.index()], 0);
        assert_eq!(table.cover[JointId::AnkleR.index()], 0);
    }

    #[test]
    fn fifteen_severable_joints_over_one_root() {
        let mut severable = 0;
        for joint in JointId::all() {
            if joint.is_severable() {
                severable += 1;
            }
        }
        assert_eq!(severable, SEVERABLE_JOINT_COUNT);
        assert!(!JointId::Pelvis.is_severable());
        // Wire round-trip: every canonical value decodes, the rest refuse.
        for joint in JointId::all() {
            assert_eq!(JointId::from_u8(joint.as_u8()), Some(joint));
        }
        assert_eq!(JointId::from_u8(16), None);
        assert_eq!(JointId::from_u8(NO_PARENT), None);
    }

    #[test]
    fn digest_stable_and_sensitive_to_every_field() {
        let table = AnatomyTable::rome_kit();
        assert_eq!(table.digest(), table.digest());
        assert_ne!(table.digest(), 0);
        // One changed byte anywhere flips the digest: tissue, parent, cover.
        let mut tissue_flip = table;
        tissue_flip.tissue[JointId::Neck.index()].bone_q16 += 1;
        assert_ne!(tissue_flip.digest(), table.digest());
        let mut parent_flip = table;
        parent_flip.parents[JointId::Neck.index()] = JointId::Waist.as_u8();
        assert_ne!(parent_flip.digest(), table.digest());
        let mut cover_flip = table;
        cover_flip.cover[JointId::Neck.index()] = ARMOR_LORICA;
        assert_ne!(cover_flip.digest(), table.digest());
        // Joint order is canonical: moving a row moves the digest, so two
        // tables with swapped rows never share one.
        let mut swapped = table;
        let wrist_l = JointId::WristL.index();
        let wrist_r = JointId::WristR.index();
        swapped.tissue.swap(wrist_l, wrist_r);
        swapped.tissue[wrist_l].bone_q16 += 1;
        swapped.tissue[wrist_r].bone_q16 -= 1;
        assert_ne!(swapped.digest(), table.digest());
    }

    #[test]
    fn validator_rejects_cycle_detach_gap_and_bad_rows() {
        assert_eq!(cyclic().validate(), Err(AnatomyError::Cyclic { joint: 5 }));
        assert_eq!(gap().validate(), Err(AnatomyError::GapTissue { joint: 13 }));
        // A mid-graph NO_PARENT detaches the whole subtree below it.
        let mut detached = AnatomyTable::rome_kit();
        detached.parents[JointId::ElbowR.index()] = NO_PARENT;
        assert_eq!(
            detached.validate(),
            Err(AnatomyError::Detached { joint: 7 })
        );
        // A parent byte past the joint range names nothing.
        let mut orphan = AnatomyTable::rome_kit();
        orphan.parents[JointId::WristL.index()] = 42;
        assert_eq!(
            orphan.validate(),
            Err(AnatomyError::BadParent {
                joint: 8,
                parent: 42
            })
        );
        // The root must stay parentless.
        let mut rooted = AnatomyTable::rome_kit();
        rooted.parents[JointId::Pelvis.index()] = JointId::Waist.as_u8();
        assert_eq!(rooted.validate(), Err(AnatomyError::RootHasParent(1)));
        // Toughness past the cap and unknown armor bits both refuse.
        let mut tough = AnatomyTable::rome_kit();
        tough.tissue[JointId::Chest.index()].bone_q16 = MAX_TOUGHNESS_Q16 + 1;
        assert_eq!(
            tough.validate(),
            Err(AnatomyError::BadToughness {
                joint: 2,
                layer: "bone",
                value: MAX_TOUGHNESS_Q16 + 1
            })
        );
        let mut covered = AnatomyTable::rome_kit();
        covered.cover[JointId::Chest.index()] = ARMOR_ALL + 1;
        assert_eq!(
            covered.validate(),
            Err(AnatomyError::BadCover {
                joint: 2,
                value: ARMOR_ALL + 1
            })
        );
        // Versions: 0 is reserved, anything past V1 is unknown.
        let mut legacy = AnatomyTable::rome_kit();
        legacy.version = 0;
        assert_eq!(legacy.validate(), Err(AnatomyError::BadVersion(0)));
        let mut future = AnatomyTable::rome_kit();
        future.version = ANATOMY_VERSION_V1 + 1;
        assert_eq!(future.validate(), Err(AnatomyError::UnknownVersion(2)));
    }

    #[test]
    fn manica_stops_a_swing_a_bare_forearm_loses() {
        let table = AnatomyTable::rome_kit();
        let bare = table.protection_q16(JointId::ElbowR, 0);
        let guarded = table.protection_q16(JointId::ElbowR, ARMOR_MANICA);
        // Tissue alone resists; the manica adds exactly its guard.
        assert_eq!(bare, TissueRow::v1().total_q16());
        assert_eq!(guarded, bare + u64::from(MANICA_GUARD_Q16));
        assert!(guarded > bare);
        // The wrong piece on the wrong joint adds nothing.
        assert_eq!(table.protection_q16(JointId::ElbowR, ARMOR_GREAVE), bare);
        // Bare joints stand on tissue alone, never zero.
        assert!(table.protection_q16(JointId::KneeR, ARMOR_ALL) > 0);
        // Unknown worn bits are ignored, so a future piece cannot shift cuts.
        assert_eq!(
            table.protection_q16(JointId::Neck, ARMOR_GALEA | !ARMOR_ALL),
            table.protection_q16(JointId::Neck, ARMOR_GALEA)
        );
    }

    #[test]
    fn protection_grows_monotonically_with_armor() {
        let table = AnatomyTable::rome_kit();
        // Every joint, every worn combination: adding worn pieces never
        // lowers resistance, and full plate tops every partial set.
        for joint in JointId::all() {
            let bare = table.protection_q16(joint, 0);
            let full = table.protection_q16(joint, ARMOR_ALL);
            let mut worn: u16 = 0;
            while worn <= u16::from(ARMOR_ALL) {
                // `worn` fits in u8: the loop bound is ARMOR_ALL (31).
                let set = worn.to_le_bytes()[0];
                let current = table.protection_q16(joint, set);
                assert!(current >= bare, "{joint:?} below bare under {set}");
                assert!(full >= current, "{joint:?} full below {set}");
                // Donning the joint's own cover can only help.
                let dressed = table.protection_q16(joint, set | table.cover[joint.index()]);
                assert!(dressed >= current, "{joint:?} regressed under {set}");
                worn += 1;
            }
        }
    }

    #[test]
    fn queries_touch_no_heap_and_read_canonical_rows() {
        // Fixed arrays behind shared references: validate, digest and
        // protection keep no state and expose no allocation path; the
        // module holds no Vec, String or Box by construction.
        let table = AnatomyTable::rome_kit();
        assert_eq!(table.validate(), Ok(()));
        let digest = table.digest();
        assert_eq!(AnatomyTable::rome_kit().digest(), digest);
        assert_eq!(
            table.tissue[JointId::Neck.index()],
            TissueRow {
                skin_q16: SKIN_Q16,
                flesh_q16: FLESH_Q16,
                bone_q16: BONE_Q16,
            }
        );
        assert_eq!(
            TissueRow::v1().total_q16(),
            u64::from(SKIN_Q16) + u64::from(FLESH_Q16) + u64::from(BONE_Q16)
        );
        assert_eq!(
            table.protection_q16(JointId::ElbowL, ARMOR_SCUTUM)
                - table.protection_q16(JointId::ElbowL, 0),
            u64::from(SCUTUM_GUARD_Q16)
        );
        assert_eq!(
            table.protection_q16(JointId::ElbowL, !ARMOR_ALL),
            table.protection_q16(JointId::ElbowL, 0)
        );
    }
}

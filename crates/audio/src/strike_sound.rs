//! strike_sound: cut, break and collapse events to material and tissue cue plays.
//!
//! Consumer: the arena's presentation side (the gore and break look) turns a
//! BX2 `Cut`, a BX4 limb body and a BX13 structure event into one
//! [`SoundEvent`] and queues it through [`queue_sound`] (or reads
//! [`cue_for`] when it drives a [`Mixer`](crate::Mixer) directly). The table
//! is pure data: a blade on bone, flesh, wood, stone or armor picks a
//! different cue, and the stump, the landing limb, the groaning beam and the
//! collapse each have their own.
//!
//! Rules this module keeps:
//!
//! - **Every material pair sounds.** [`CUT_TABLE`] holds one cell per
//!   [`Contact`] x [`Struck`] pair and every cell names a bank cue with a
//!   positive gain; [`SEVER_TABLE`], [`LANDING_TABLE`], [`GROAN_TABLE`] and
//!   [`COLLAPSE_TABLE`] hold one row per struck material. Nothing falls back
//!   to silence.
//! - **No sound without an event.** [`cue_for`] is the only door and it takes
//!   a [`SoundEvent`]; there is no ambient, loop or "idle" sound here, so a
//!   voice only ever starts on the authority event that caused it.
//! - **Sim never reads this.** Cues, gains and positions are presentation;
//!   the hash is untouched by any value in this module.
//!
//! Material names mirror the `engine-destruction` table's record names
//! (`stone`, `brick`, ... `ceramic`), so a material pack resolves a row by
//! name ([`Struck::from_name`]) and audio keeps no dependency on the data
//! crate; the three tissue layers and the armor coverage come from the
//! `engine-combat` anatomy (`skin`, `flesh`, `bone`, and `armor` for a joint
//! under a galea, manica, greave, lorica or scutum).
//!
//! Units: gain is a linear `0.0..=1.0` play level times the cue's own trim;
//! position is `[east, up, south]` in centimeters. Determinism scope: every
//! function is a pure match over enum bytes, so the same event always picks
//! the same cue and gain, and a replayed script sounds identical.

use crate::combat_cues::CuePlay;
use crate::cue::names;
use crate::feed::{FeedError, FeedTx};

/// What struck: the edge of a blade, or the mass behind a blunt blow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Contact {
    /// A blade edge (sword, axe, spear point).
    Edge,
    /// A blunt mass (fist, club, shield rim, a falling body).
    Blunt,
}

impl Contact {
    /// Number of contacts.
    pub const COUNT: usize = 2;
    /// Every contact, in [`Self::index`] order.
    pub const ALL: [Contact; Contact::COUNT] = [Self::Edge, Self::Blunt];

    /// Dense index `0..COUNT` for the pair table.
    pub fn index(self) -> usize {
        match self {
            Self::Edge => 0,
            Self::Blunt => 1,
        }
    }
}

/// What was struck: the three body tissue layers, a joint's armor, and the
/// fourteen world materials of the destruction table in its canonical order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Struck {
    /// Skin: the outer layer, stops nothing alone.
    Skin,
    /// Flesh: muscle, a wet thud.
    Flesh,
    /// Bone: hard, rings, cracks on a sever.
    Bone,
    /// A joint under armor plate or mail (galea, manica, greave, lorica,
    /// scutum): a metal ring over a body thud.
    Armor,
    /// Quarried stone.
    Stone,
    /// Fired brick.
    Brick,
    /// Structural timber.
    Timber,
    /// Thatch roofing.
    Thatch,
    /// Plaster facing.
    Plaster,
    /// Poured concrete.
    Concrete,
    /// Marble facing.
    Marble,
    /// Wrought iron.
    Iron,
    /// Bronze fittings.
    Bronze,
    /// Leather straps.
    Leather,
    /// Woven cloth.
    Cloth,
    /// Rope rigging.
    Rope,
    /// Glass panes.
    Glass,
    /// Ceramic tile.
    Ceramic,
}

impl Struck {
    /// Number of struck materials.
    pub const COUNT: usize = 18;
    /// Every struck material, in [`Self::index`] order (tissue first, then
    /// the destruction table's row order).
    pub const ALL: [Struck; Struck::COUNT] = [
        Self::Skin,
        Self::Flesh,
        Self::Bone,
        Self::Armor,
        Self::Stone,
        Self::Brick,
        Self::Timber,
        Self::Thatch,
        Self::Plaster,
        Self::Concrete,
        Self::Marble,
        Self::Iron,
        Self::Bronze,
        Self::Leather,
        Self::Cloth,
        Self::Rope,
        Self::Glass,
        Self::Ceramic,
    ];

    /// Dense index `0..COUNT` for the per-material tables.
    pub fn index(self) -> usize {
        match self {
            Self::Skin => 0,
            Self::Flesh => 1,
            Self::Bone => 2,
            Self::Armor => 3,
            Self::Stone => 4,
            Self::Brick => 5,
            Self::Timber => 6,
            Self::Thatch => 7,
            Self::Plaster => 8,
            Self::Concrete => 9,
            Self::Marble => 10,
            Self::Iron => 11,
            Self::Bronze => 12,
            Self::Leather => 13,
            Self::Cloth => 14,
            Self::Rope => 15,
            Self::Glass => 16,
            Self::Ceramic => 17,
        }
    }

    /// Record name of the material: the destruction row name for the fourteen
    /// world materials, and `skin`/`flesh`/`bone`/`armor` for the body.
    pub fn name(self) -> &'static str {
        match self {
            Self::Skin => "skin",
            Self::Flesh => "flesh",
            Self::Bone => "bone",
            Self::Armor => "armor",
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

    /// Material by record name; an unknown name is `None`, never a panic.
    pub fn from_name(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == text)
    }
}

/// One table cell: which cue plays and at what level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CueRow {
    /// Cue name in the arena bank (one of [`names`]).
    pub name: &'static str,
    /// Play level `0.0..=1.0`, times the cue's own trim.
    pub gain: f32,
}

/// Table cell shorthand.
const fn row(name: &'static str, gain: f32) -> CueRow {
    CueRow { name, gain }
}

/// Cut table: one row per [`Contact`], one column per [`Struck`], so every
/// material pair has its own cell. A blade on bone rings, on flesh thuds, on
/// timber knocks, on stone cracks and on armor rings over a body thud; a
/// blunt blow reads the same families a little softer.
pub const CUT_TABLE: [[CueRow; Struck::COUNT]; Contact::COUNT] = [
    // Edge: skin, flesh, bone, armor, stone, brick, timber, thatch, plaster,
    // concrete, marble, iron, bronze, leather, cloth, rope, glass, ceramic.
    [
        row(names::HIT, 0.85),
        row(names::HIT, 1.0),
        row(names::IMPACT_BONE, 1.0),
        row(names::IMPACT_ARMOR, 0.95),
        row(names::IMPACT_STONE, 0.9),
        row(names::IMPACT_STONE, 0.9),
        row(names::BLOCK, 0.9),
        row(names::BLOCK, 0.6),
        row(names::IMPACT_STONE, 0.7),
        row(names::IMPACT_STONE, 1.0),
        row(names::IMPACT_STONE, 0.95),
        row(names::PARRY, 0.95),
        row(names::PARRY, 0.9),
        row(names::HIT, 0.7),
        row(names::HIT, 0.55),
        row(names::HIT, 0.6),
        row(names::PARRY, 0.85),
        row(names::IMPACT_STONE, 0.85),
    ],
    // Blunt: the same materials, a touch softer on the hard rows.
    [
        row(names::HIT, 0.8),
        row(names::HIT, 0.95),
        row(names::IMPACT_BONE, 0.9),
        row(names::IMPACT_ARMOR, 0.9),
        row(names::IMPACT_STONE, 0.8),
        row(names::IMPACT_STONE, 0.85),
        row(names::BLOCK, 0.85),
        row(names::BLOCK, 0.5),
        row(names::IMPACT_STONE, 0.65),
        row(names::IMPACT_STONE, 0.95),
        row(names::IMPACT_STONE, 0.9),
        row(names::PARRY, 0.9),
        row(names::PARRY, 0.85),
        row(names::HIT, 0.65),
        row(names::HIT, 0.5),
        row(names::HIT, 0.55),
        row(names::PARRY, 0.75),
        row(names::IMPACT_STONE, 0.8),
    ],
];

/// Sever table: the stump opens. One row per [`Struck`], in its order.
pub const SEVER_TABLE: [CueRow; Struck::COUNT] = [
    row(names::STUMP, 0.95),
    row(names::STUMP, 1.0),
    row(names::STUMP, 1.0),
    row(names::STUMP, 1.0),
    row(names::STUMP, 0.95),
    row(names::STUMP, 0.95),
    row(names::STUMP, 0.9),
    row(names::STUMP, 0.75),
    row(names::STUMP, 0.8),
    row(names::STUMP, 0.95),
    row(names::STUMP, 0.9),
    row(names::STUMP, 0.9),
    row(names::STUMP, 0.9),
    row(names::STUMP, 0.85),
    row(names::STUMP, 0.8),
    row(names::STUMP, 0.85),
    row(names::STUMP, 0.85),
    row(names::STUMP, 0.85),
];

/// Limb landing: a severed mass hits the floor. One row per [`Struck`].
pub const LANDING_TABLE: [CueRow; Struck::COUNT] = [
    row(names::KNOCKDOWN, 0.85),
    row(names::KNOCKDOWN, 0.9),
    row(names::KNOCKDOWN, 0.9),
    row(names::KNOCKDOWN, 0.95),
    row(names::KNOCKDOWN, 0.85),
    row(names::KNOCKDOWN, 0.85),
    row(names::KNOCKDOWN, 0.85),
    row(names::KNOCKDOWN, 0.7),
    row(names::KNOCKDOWN, 0.75),
    row(names::KNOCKDOWN, 0.9),
    row(names::KNOCKDOWN, 0.9),
    row(names::KNOCKDOWN, 0.95),
    row(names::KNOCKDOWN, 0.9),
    row(names::KNOCKDOWN, 0.75),
    row(names::KNOCKDOWN, 0.7),
    row(names::KNOCKDOWN, 0.7),
    row(names::KNOCKDOWN, 0.8),
    row(names::KNOCKDOWN, 0.8),
];

/// Structure groan: a loaded part creaks before it fails. One row per
/// [`Struck`]; a body row is the soft joint creak of a falling pile.
pub const GROAN_TABLE: [CueRow; Struck::COUNT] = [
    row(names::STRUCTURE_GROAN, 0.5),
    row(names::STRUCTURE_GROAN, 0.5),
    row(names::STRUCTURE_GROAN, 0.55),
    row(names::STRUCTURE_GROAN, 0.6),
    row(names::STRUCTURE_GROAN, 0.85),
    row(names::STRUCTURE_GROAN, 0.8),
    row(names::STRUCTURE_GROAN, 0.9),
    row(names::STRUCTURE_GROAN, 0.6),
    row(names::STRUCTURE_GROAN, 0.65),
    row(names::STRUCTURE_GROAN, 0.85),
    row(names::STRUCTURE_GROAN, 0.8),
    row(names::STRUCTURE_GROAN, 0.8),
    row(names::STRUCTURE_GROAN, 0.75),
    row(names::STRUCTURE_GROAN, 0.55),
    row(names::STRUCTURE_GROAN, 0.5),
    row(names::STRUCTURE_GROAN, 0.6),
    row(names::STRUCTURE_GROAN, 0.5),
    row(names::STRUCTURE_GROAN, 0.55),
];

/// Structure collapse: the part lets go. One row per [`Struck`].
pub const COLLAPSE_TABLE: [CueRow; Struck::COUNT] = [
    row(names::KNOCKDOWN, 0.8),
    row(names::KNOCKDOWN, 0.85),
    row(names::KNOCKDOWN, 0.85),
    row(names::KNOCKDOWN, 0.9),
    row(names::IMPACT_STONE, 1.0),
    row(names::IMPACT_STONE, 0.95),
    row(names::KNOCKDOWN, 0.95),
    row(names::KNOCKDOWN, 0.7),
    row(names::IMPACT_STONE, 0.8),
    row(names::IMPACT_STONE, 1.0),
    row(names::IMPACT_STONE, 1.0),
    row(names::PARRY, 0.95),
    row(names::PARRY, 0.9),
    row(names::KNOCKDOWN, 0.75),
    row(names::KNOCKDOWN, 0.7),
    row(names::KNOCKDOWN, 0.7),
    row(names::PARRY, 0.8),
    row(names::IMPACT_STONE, 0.85),
];

/// The authority event a sound hangs on. Presentation builds one of these
/// from a cut, a break or a collapse; there is no other way in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundEvent {
    /// A blade or blunt blow met one material pair.
    Cut {
        /// Edge or mass.
        contact: Contact,
        /// What it met.
        struck: Struck,
    },
    /// A joint severed: the stump opens (BX2 `Cut` at a joint).
    Sever {
        /// Tissue or armor the edge left through.
        struck: Struck,
    },
    /// A severed limb or chunk landed (BX4 limb body).
    LimbLanding {
        /// What landed: flesh, bone or armor.
        struck: Struck,
    },
    /// A loaded structure part creaked before failing (BX13).
    Groan {
        /// Material of the creaking part.
        struck: Struck,
    },
    /// A structure part let go and fell (BX13 / DX1 `Collapse`).
    Collapse {
        /// Material of the falling part.
        struck: Struck,
    },
}

/// Cue play for one event at `pos_cm` (`[east, up, south]` in centimeters).
///
/// Total: every event names a cue with a positive gain, so a cut, a sever, a
/// landing, a groan or a collapse is never silent. Pure data; never panics.
pub fn cue_for(event: SoundEvent, pos_cm: [i32; 3]) -> CuePlay {
    let cell = match event {
        SoundEvent::Cut { contact, struck } => CUT_TABLE[contact.index()][struck.index()],
        SoundEvent::Sever { struck } => SEVER_TABLE[struck.index()],
        SoundEvent::LimbLanding { struck } => LANDING_TABLE[struck.index()],
        SoundEvent::Groan { struck } => GROAN_TABLE[struck.index()],
        SoundEvent::Collapse { struck } => COLLAPSE_TABLE[struck.index()],
    };
    CuePlay {
        name: cell.name,
        gain: cell.gain,
        pos_cm: Some(pos_cm),
    }
}

/// Queue `event` at `pos_cm` on the feed for `tick`: the cue [`cue_for`]
/// names, played at its source. Never blocks and never allocates.
///
/// # Errors
///
/// As [`FeedTx::play`]: [`FeedError::Full`] (counted) or
/// [`FeedError::Closed`].
pub fn queue_sound(
    tx: &mut FeedTx,
    tick: u32,
    event: SoundEvent,
    pos_cm: [i32; 3],
) -> Result<(), FeedError> {
    let play = cue_for(event, pos_cm);
    tx.play(tick, play.name, play.gain, play.pos_cm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contact_and_material_index_is_dense_and_stable() {
        for (i, contact) in Contact::ALL.iter().enumerate() {
            assert_eq!(contact.index(), i, "{contact:?}");
        }
        for (i, struck) in Struck::ALL.iter().enumerate() {
            assert_eq!(struck.index(), i, "{struck:?}");
        }
        assert_eq!(Contact::ALL.len(), Contact::COUNT);
        assert_eq!(Struck::ALL.len(), Struck::COUNT);
    }

    #[test]
    fn materials_round_trip_by_record_name() {
        for struck in Struck::ALL {
            assert_eq!(Struck::from_name(struck.name()), Some(struck));
        }
        assert_eq!(Struck::from_name("unobtainium"), None);
        assert_eq!(Struck::from_name(""), None);
    }

    /// The fourteen destruction row names all resolve, so a material pack can
    /// hand its row over by name without audio knowing the data crate.
    #[test]
    fn the_fourteen_world_materials_resolve_by_name() {
        let world = [
            "stone", "brick", "timber", "thatch", "plaster", "concrete", "marble", "iron",
            "bronze", "leather", "cloth", "rope", "glass", "ceramic",
        ];
        let mut found = 0;
        for name in world {
            let struck = Struck::from_name(name).unwrap_or_else(|| panic!("{name} missing"));
            assert!(
                matches!(
                    struck,
                    Struck::Stone
                        | Struck::Brick
                        | Struck::Timber
                        | Struck::Thatch
                        | Struck::Plaster
                        | Struck::Concrete
                        | Struck::Marble
                        | Struck::Iron
                        | Struck::Bronze
                        | Struck::Leather
                        | Struck::Cloth
                        | Struck::Rope
                        | Struck::Glass
                        | Struck::Ceramic
                ),
                "{name} is no world material"
            );
            found += 1;
        }
        assert_eq!(found, 14, "the destruction table has fourteen rows");
    }

    /// Every table cell names a cue and a level the mixer keeps.
    #[test]
    fn every_table_cell_is_a_real_play() {
        let cells = CUT_TABLE
            .iter()
            .flatten()
            .chain(SEVER_TABLE.iter())
            .chain(LANDING_TABLE.iter())
            .chain(GROAN_TABLE.iter())
            .chain(COLLAPSE_TABLE.iter());
        for cell in cells {
            assert!(!cell.name.is_empty());
            assert!(cell.gain > 0.0 && cell.gain <= 1.0, "{}", cell.name);
        }
    }

    /// A cut of a blade on bone is not the same sound as flesh, wood, stone
    /// or armor: the cells the row names are five different cues.
    #[test]
    fn the_named_struck_materials_keep_their_own_cue() {
        let edge = |struck| {
            cue_for(
                SoundEvent::Cut {
                    contact: Contact::Edge,
                    struck,
                },
                [0, 0, 0],
            )
        };
        let flesh = edge(Struck::Flesh).name;
        let mut names = vec![flesh];
        for struck in [Struck::Bone, Struck::Timber, Struck::Stone, Struck::Armor] {
            let name = edge(struck).name;
            assert!(!names.contains(&name), "{struck:?} shares `{name}`");
            names.push(name);
        }
    }

    #[test]
    fn every_event_kind_reads_its_own_table_row() {
        let at = [10, 20, 30];
        for struck in Struck::ALL {
            let sever = cue_for(SoundEvent::Sever { struck }, at);
            assert_eq!(
                (sever.name, sever.gain),
                (
                    SEVER_TABLE[struck.index()].name,
                    SEVER_TABLE[struck.index()].gain
                )
            );
            let landing = cue_for(SoundEvent::LimbLanding { struck }, at);
            assert_eq!(
                (landing.name, landing.gain),
                (
                    LANDING_TABLE[struck.index()].name,
                    LANDING_TABLE[struck.index()].gain
                )
            );
            let groan = cue_for(SoundEvent::Groan { struck }, at);
            assert_eq!(
                (groan.name, groan.gain),
                (
                    GROAN_TABLE[struck.index()].name,
                    GROAN_TABLE[struck.index()].gain
                )
            );
            let collapse = cue_for(SoundEvent::Collapse { struck }, at);
            assert_eq!(
                (collapse.name, collapse.gain),
                (
                    COLLAPSE_TABLE[struck.index()].name,
                    COLLAPSE_TABLE[struck.index()].gain
                )
            );
            assert_eq!(groan.pos_cm, Some(at));
        }
    }
}

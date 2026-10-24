//! limbloss: what a body does after a cut or a heavy wound: stump states,
//! venous and arterial bleeding, shock, faint, surrender and exsanguination.
//!
//! Consumer: the arena match step feeds each [`Cut`] from `cut.rs` and each
//! non-severing hit to [`BodyState`](crate::limbloss::BodyState) and steps it once per tick; AI reads
//! the joint mask and the surrender flag; the limb body (`physics::limb`)
//! reads the severed set. Presentation (blood spray, stump caps) reads the
//! events and never this state. Data owner: `engine-combat`. Units: blood
//! and shock in Q16 (`1.0 == 65536`, one blood unit is 1 ml), time in
//! fixed 60 Hz ticks. Determinism scope: every function is pure over the
//! table, the body state and the cut bytes; integer math only, joints in
//! canonical [`JointId`] order, so the same tape gives the same bodies on
//! every PC. Errors: [`LimbLossError`](crate::limbloss::LimbLossError) for a bad table; user data never
//! panics. Budget: [`BodyState::step`](crate::limbloss::BodyState::step) is `O(1)` plus one pass over
//! [`JOINT_TOTAL`] joints on a change only, no heap; the state is `Copy`.
//!
//! Extends the anatomy table and the cut event; it forks neither: joint ids
//! and parents come from [`AnatomyTable`], the sever event is [`Cut`].

use crate::anatomy::{AnatomyTable, JointId, JOINT_TOTAL};
use crate::cut::Cut;
use crate::{trauma_shake, Trauma};
use engine_core::Fnv64;

/// Wire version of the shipped wound table.
pub const WOUND_VERSION_V1: u32 = 1;
/// Most blood a table may carry, in Q16 (60,000 units).
pub const MAX_BLOOD_Q16: u32 = 60_000 << 16;
/// Longest bleed-out a table may promise; ticks (one hour at 60 Hz).
pub const MAX_BLEED_OUT_TICKS: u32 = 216_000;

/// How one wound bleeds, slowest first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BleedClass {
    /// Slow seep from a wounded joint.
    Ooze,
    /// Steady flow from a cut limb stump.
    Venous,
    /// Pulsing flow from a cut at a main artery (neck, hip, shoulder).
    Arterial,
}

impl BleedClass {
    /// Wire value, canonical table order.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ooze => 0,
            Self::Venous => 1,
            Self::Arterial => 2,
        }
    }

    const fn slot(self) -> usize {
        match self {
            Self::Ooze => 0,
            Self::Venous => 1,
            Self::Arterial => 2,
        }
    }
}

/// State of one joint's limb segment, worst last.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StumpState {
    /// Whole and working.
    Intact,
    /// Hurt: bleeds as an ooze but still works.
    Wounded,
    /// Cannot be used (hangs, grips nothing); still attached.
    Disabled,
    /// Gone: cut here, or below a cut.
    Severed,
}

impl StumpState {
    /// Wire value.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Intact => 0,
            Self::Wounded => 1,
            Self::Disabled => 2,
            Self::Severed => 3,
        }
    }
}

/// What the body did this tick; each is raised once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LimbEvent {
    /// Shock or blood loss broke the will to fight.
    Surrender,
    /// Blood fell under the faint line: the body drops.
    Faint,
    /// Blood ran out: the fighter is dead.
    Exsanguinated,
}

/// Why a wound table was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LimbLossError {
    /// Version 0 is reserved.
    BadVersion(u32),
    /// Blood is zero or past [`MAX_BLOOD_Q16`].
    BadBlood(u32),
    /// A bleed-out time is zero or past [`MAX_BLEED_OUT_TICKS`], or the
    /// classes do not run slowest to fastest; class carried.
    BadBleedOut(u8),
    /// A threshold line is zero, or the lines are out of order.
    BadThreshold(&'static str),
    /// The pelvis root carries a wound line that could never apply.
    BadJointRow(u8),
}

impl core::fmt::Display for LimbLossError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(v) => write!(f, "wound table version {v} is not valid"),
            Self::BadBlood(v) => write!(f, "wound table blood {v} is out of range"),
            Self::BadBleedOut(c) => write!(f, "wound table bleed-out of class {c} is out of range"),
            Self::BadThreshold(name) => write!(f, "wound table threshold {name} is out of range"),
            Self::BadJointRow(j) => write!(f, "wound table joint row {j} is out of range"),
        }
    }
}

impl std::error::Error for LimbLossError {}

/// Versioned wound, bleed and surrender table. Fixed arrays, `Copy`, no heap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WoundTable {
    /// Table version, [`WOUND_VERSION_V1`] for the shipped row.
    pub version: u32,
    /// Blood of a whole fighter, Q16.
    pub blood_full_q16: u32,
    /// Ticks from full blood to none for one open wound of each class
    /// ([`BleedClass`] order: ooze, venous, arterial).
    pub bleed_out_ticks: [u32; 3],
    /// Class of the stump a cut at each joint leaves, canonical joint order.
    pub stump_class: [BleedClass; JOINT_TOTAL],
    /// Shock a cut at each joint adds at once, Q16.
    pub sever_shock_q16: [u32; JOINT_TOTAL],
    /// Damage a non-severing hit must reach, summed, to wound the joint, Q16.
    pub wound_at_q16: [u32; JOINT_TOTAL],
    /// Summed damage that disables the joint, Q16; above `wound_at_q16`.
    pub disable_at_q16: [u32; JOINT_TOTAL],
    /// Shock lost per tick, Q16.
    pub shock_decay_q16: u32,
    /// Shock gained per tick for each open bleed class, Q16.
    pub bleed_shock_q16: [u32; 3],
    /// Shock at which the fighter surrenders, Q16.
    pub surrender_shock_q16: u32,
    /// Blood at or under which the fighter surrenders, Q16.
    pub surrender_blood_q16: u32,
    /// Blood at or under which the fighter faints, Q16; under the surrender
    /// blood line.
    pub faint_blood_q16: u32,
}

impl WoundTable {
    /// The shipped V1 row: 5,000 ml of blood; an arterial stump empties it
    /// in 20 s, a venous one in 90 s, an ooze in 5 min; arteries at the
    /// neck, shoulders and hips; every other cut is venous.
    #[must_use]
    pub const fn v1() -> Self {
        let blood = 5_000u32 << 16;
        use BleedClass::{Arterial, Venous};
        Self {
            version: WOUND_VERSION_V1,
            blood_full_q16: blood,
            bleed_out_ticks: [18_000, 5_400, 1_200],
            stump_class: [
                Venous,   // Pelvis: never cut.
                Arterial, // Waist: torso bisect.
                Arterial, // Chest: torso bisect.
                Arterial, // Neck.
                Arterial, // ShoulderL.
                Arterial, // ShoulderR.
                Venous,   // ElbowL.
                Venous,   // ElbowR.
                Venous,   // WristL.
                Venous,   // WristR.
                Arterial, // HipL.
                Arterial, // HipR.
                Venous,   // KneeL.
                Venous,   // KneeR.
                Venous,   // AnkleL.
                Venous,   // AnkleR.
            ],
            sever_shock_q16: [
                0,
                60 << 16,
                60 << 16,
                80 << 16,
                40 << 16,
                40 << 16,
                30 << 16,
                30 << 16,
                20 << 16,
                20 << 16,
                45 << 16,
                45 << 16,
                30 << 16,
                30 << 16,
                20 << 16,
                20 << 16,
            ],
            wound_at_q16: [
                0,
                8 << 16,
                8 << 16,
                6 << 16,
                5 << 16,
                5 << 16,
                4 << 16,
                4 << 16,
                3 << 16,
                3 << 16,
                5 << 16,
                5 << 16,
                4 << 16,
                4 << 16,
                3 << 16,
                3 << 16,
            ],
            disable_at_q16: [
                0,
                20 << 16,
                20 << 16,
                16 << 16,
                12 << 16,
                12 << 16,
                10 << 16,
                10 << 16,
                8 << 16,
                8 << 16,
                12 << 16,
                12 << 16,
                10 << 16,
                10 << 16,
                8 << 16,
                8 << 16,
            ],
            shock_decay_q16: 1 << 14,
            bleed_shock_q16: [1 << 12, 1 << 13, 1 << 15],
            surrender_shock_q16: 100 << 16,
            surrender_blood_q16: 3_000 << 16,
            faint_blood_q16: 1_500 << 16,
        }
    }

    /// Range-checks the table; first failure wins in field order.
    pub fn validate(&self) -> Result<(), LimbLossError> {
        if self.version == 0 {
            return Err(LimbLossError::BadVersion(self.version));
        }
        if self.blood_full_q16 == 0 || self.blood_full_q16 > MAX_BLOOD_Q16 {
            return Err(LimbLossError::BadBlood(self.blood_full_q16));
        }
        let mut class = 0usize;
        while class < 3 {
            let ticks = self.bleed_out_ticks[class];
            let slower_ok = class == 0 || self.bleed_out_ticks[class - 1] > ticks;
            if ticks == 0 || ticks > MAX_BLEED_OUT_TICKS || !slower_ok {
                return Err(LimbLossError::BadBleedOut(
                    u8::try_from(class).unwrap_or(u8::MAX),
                ));
            }
            class += 1;
        }
        if self.surrender_shock_q16 == 0 {
            return Err(LimbLossError::BadThreshold("surrender_shock"));
        }
        if self.faint_blood_q16 == 0
            || self.faint_blood_q16 >= self.surrender_blood_q16
            || self.surrender_blood_q16 >= self.blood_full_q16
        {
            return Err(LimbLossError::BadThreshold("blood lines"));
        }
        let mut joint = 1usize;
        while joint < JOINT_TOTAL {
            if self.wound_at_q16[joint] == 0
                || self.disable_at_q16[joint] <= self.wound_at_q16[joint]
            {
                return Err(LimbLossError::BadJointRow(
                    u8::try_from(joint).unwrap_or(u8::MAX),
                ));
            }
            joint += 1;
        }
        Ok(())
    }

    /// Blood lost per tick by one open wound of a class, Q16, rounded up so
    /// the promised bleed-out time is never overshot by more than a tick.
    #[must_use]
    pub fn rate_q16(&self, class: BleedClass) -> u64 {
        let ticks = u64::from(self.bleed_out_ticks[class.slot()]).max(1);
        u64::from(self.blood_full_q16).div_ceil(ticks)
    }

    /// Ticks one open wound of a class needs to empty a whole body: the
    /// bleed-out table entry the stepper must hit within one tick.
    #[must_use]
    pub fn bleed_out_of(&self, class: BleedClass) -> u32 {
        self.bleed_out_ticks[class.slot()]
    }

    /// Digest over every field in order.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(self.version);
        h.write_u32(self.blood_full_q16);
        for t in self.bleed_out_ticks {
            h.write_u32(t);
        }
        for joint in 0..JOINT_TOTAL {
            h.write_u8(self.stump_class[joint].as_u8());
            h.write_u32(self.sever_shock_q16[joint]);
            h.write_u32(self.wound_at_q16[joint]);
            h.write_u32(self.disable_at_q16[joint]);
        }
        h.write_u32(self.shock_decay_q16);
        for s in self.bleed_shock_q16 {
            h.write_u32(s);
        }
        h.write_u32(self.surrender_shock_q16);
        h.write_u32(self.surrender_blood_q16);
        h.write_u32(self.faint_blood_q16);
        h.finish()
    }
}

/// One fighter's body after hits: joint states, blood, shock and the three
/// one-shot flags. `Copy`, no heap, hashed in order by [`Self::digest`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BodyState {
    /// Segment state per joint, canonical order.
    pub stump: [StumpState; JOINT_TOTAL],
    /// Summed non-severing damage per joint, Q16.
    pub damage_q16: [u32; JOINT_TOTAL],
    /// Joints that bleed from a cut, one bit per joint (bit = wire value).
    pub stump_bleed: u16,
    /// Blood left, Q16.
    pub blood_q16: u32,
    /// Shock, Q16.
    pub shock_q16: u32,
    /// Tick counter of this body, for event ordering.
    pub age: u32,
    /// Bit 0 surrendered, bit 1 fainted, bit 2 dead.
    pub flags: u8,
}

const FLAG_SURRENDERED: u8 = 1;
const FLAG_FAINTED: u8 = 2;
const FLAG_DEAD: u8 = 4;

impl BodyState {
    /// A whole fighter with full blood.
    #[must_use]
    pub fn new(table: &WoundTable) -> Self {
        Self {
            stump: [StumpState::Intact; JOINT_TOTAL],
            damage_q16: [0; JOINT_TOTAL],
            stump_bleed: 0,
            blood_q16: table.blood_full_q16,
            shock_q16: 0,
            age: 0,
            flags: 0,
        }
    }

    /// True once the fighter has surrendered.
    #[must_use]
    pub fn surrendered(&self) -> bool {
        self.flags & FLAG_SURRENDERED != 0
    }

    /// True once the fighter has fainted.
    #[must_use]
    pub fn fainted(&self) -> bool {
        self.flags & FLAG_FAINTED != 0
    }

    /// True once the fighter has bled out.
    #[must_use]
    pub fn dead(&self) -> bool {
        self.flags & FLAG_DEAD != 0
    }

    /// State of one joint's segment.
    #[must_use]
    pub fn state_of(&self, joint: JointId) -> StumpState {
        self.stump[joint.index()]
    }

    /// Joints that still work: bit per joint, set when intact or wounded.
    #[must_use]
    pub fn working_mask(&self) -> u16 {
        let mut mask = 0u16;
        for joint in 0..JOINT_TOTAL {
            if matches!(self.stump[joint], StumpState::Intact | StumpState::Wounded) {
                mask |= 1 << joint;
            }
        }
        mask
    }

    /// Joints that are gone: bit per joint, set when severed.
    #[must_use]
    pub fn severed_mask(&self) -> u16 {
        let mut mask = 0u16;
        for joint in 0..JOINT_TOTAL {
            if self.stump[joint] == StumpState::Severed {
                mask |= 1 << joint;
            }
        }
        mask
    }

    /// Open wounds per class (ooze, venous, arterial) now bleeding.
    #[must_use]
    pub fn open_wounds(&self, table: &WoundTable) -> [u32; 3] {
        let mut open = [0u32; 3];
        for joint in 0..JOINT_TOTAL {
            if let Some(class) = self.bleed_class(table, joint) {
                open[class.slot()] += 1;
            }
        }
        open
    }

    /// Class a joint bleeds as right now: its stump class when cut, an ooze
    /// when wounded or disabled, nothing otherwise. A severed joint below a
    /// cut does not bleed; only the cut root does.
    fn bleed_class(&self, table: &WoundTable, joint: usize) -> Option<BleedClass> {
        if self.stump_bleed & (1 << joint) != 0 {
            return Some(table.stump_class[joint]);
        }
        match self.stump[joint] {
            StumpState::Wounded | StumpState::Disabled => Some(BleedClass::Ooze),
            StumpState::Intact | StumpState::Severed => None,
        }
    }

    /// Applies one cut: the joint and everything below it become severed,
    /// the cut root starts bleeding, shock jumps by the joint's row. A cut
    /// at a joint that is already gone changes nothing and returns false.
    pub fn apply_cut(&mut self, table: &WoundTable, anatomy: &AnatomyTable, cut: &Cut) -> bool {
        let Some(joint) = JointId::from_u8(cut.joint) else {
            return false;
        };
        let at = joint.index();
        if !joint.is_severable() || self.stump[at] == StumpState::Severed || self.dead() {
            return false;
        }
        self.stump[at] = StumpState::Severed;
        self.stump_bleed |= 1 << at;
        // Parents run below children in canonical order: one pass reaches
        // every descendant.
        for child in (at + 1)..JOINT_TOTAL {
            let parent = usize::from(anatomy.parents[child]);
            if parent < JOINT_TOTAL && self.stump[parent] == StumpState::Severed {
                self.stump[child] = StumpState::Severed;
                // A deeper stump that already bled no longer does: its limb
                // left with the new cut.
                self.stump_bleed &= !(1 << child);
            }
        }
        self.shock_q16 = self.shock_q16.saturating_add(table.sever_shock_q16[at]);
        true
    }

    /// Applies one non-severing hit of `damage_q16` to a joint: damage sums,
    /// and the joint steps to wounded then disabled at the table lines.
    /// Returns the new state; severed, dead and root joints do not change.
    pub fn apply_wound(
        &mut self,
        table: &WoundTable,
        joint: JointId,
        damage_q16: u32,
    ) -> StumpState {
        let at = joint.index();
        if !joint.is_severable() || self.stump[at] == StumpState::Severed || self.dead() {
            return self.stump[at];
        }
        self.damage_q16[at] = self.damage_q16[at].saturating_add(damage_q16);
        let total = self.damage_q16[at];
        let state = if total >= table.disable_at_q16[at] {
            StumpState::Disabled
        } else if total >= table.wound_at_q16[at] {
            StumpState::Wounded
        } else {
            StumpState::Intact
        };
        // Never heal: a state only moves toward worse.
        if state.as_u8() > self.stump[at].as_u8() {
            self.stump[at] = state;
        }
        self.stump[at]
    }

    /// One tick: bleed, shock rise and decay, then the three lines checked
    /// in order (exsanguination, surrender, faint). Each event is raised
    /// once; at most one comes back per tick, so two lines crossed together
    /// show on consecutive ticks.
    pub fn step(&mut self, table: &WoundTable) -> Option<LimbEvent> {
        self.age = self.age.saturating_add(1);
        if self.dead() {
            return None;
        }
        let mut bleed = 0u64;
        let mut shock_up = 0u32;
        for joint in 0..JOINT_TOTAL {
            if let Some(class) = self.bleed_class(table, joint) {
                bleed = bleed.saturating_add(table.rate_q16(class));
                shock_up = shock_up.saturating_add(table.bleed_shock_q16[class.slot()]);
            }
        }
        let bleed = u32::try_from(bleed).unwrap_or(u32::MAX);
        self.blood_q16 = self.blood_q16.saturating_sub(bleed);
        self.shock_q16 = self
            .shock_q16
            .saturating_add(shock_up)
            .saturating_sub(table.shock_decay_q16);
        if self.blood_q16 == 0 {
            self.flags |= FLAG_DEAD | FLAG_FAINTED;
            return Some(LimbEvent::Exsanguinated);
        }
        if !self.surrendered()
            && (self.shock_q16 >= table.surrender_shock_q16
                || self.blood_q16 <= table.surrender_blood_q16)
        {
            self.flags |= FLAG_SURRENDERED;
            return Some(LimbEvent::Surrender);
        }
        if !self.fainted() && self.blood_q16 <= table.faint_blood_q16 {
            self.flags |= FLAG_FAINTED;
            return Some(LimbEvent::Faint);
        }
        None
    }

    /// Presentation trauma from this body's shock plus blood loss: the
    /// production consumer of [`Trauma`]. Shock fraction and blood-loss
    /// fraction each map to permille, the worse wins, clamped at full shake.
    /// Integer math only, same body plus table always gives the same trauma.
    /// Presentation-only: never hashed, never stepped. No heap.
    #[must_use]
    pub fn impact_trauma(&self, table: &WoundTable) -> Trauma {
        let shock = u64::from(self.shock_q16) * 1000 / u64::from(table.surrender_shock_q16.max(1));
        let lost = table.blood_full_q16.saturating_sub(self.blood_q16);
        let blood = u64::from(lost) * 1000 / u64::from(table.blood_full_q16.max(1));
        Trauma::from_permille(u16::try_from(shock.max(blood).min(1000)).unwrap_or(u16::MAX))
    }

    /// Presentation shake offsets for this body at a tick: trauma from
    /// [`Self::impact_trauma`] through [`trauma_shake`]. Same body plus
    /// table plus seed plus tick always gives the same offsets. No heap.
    #[must_use]
    pub fn impact_shake(&self, table: &WoundTable, seed: u64, tick: u64) -> (f32, f32) {
        trauma_shake(seed, self.impact_trauma(table), tick)
    }

    /// Digest over every field in order: the receipt a replay compares.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        for joint in 0..JOINT_TOTAL {
            h.write_u8(self.stump[joint].as_u8());
            h.write_u32(self.damage_q16[joint]);
        }
        h.write_u16(self.stump_bleed);
        h.write_u32(self.blood_q16);
        h.write_u32(self.shock_q16);
        h.write_u32(self.age);
        h.write_u8(self.flags);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anatomy::AnatomyTable;

    fn cut(joint: JointId, swing: u32) -> Cut {
        Cut {
            swing,
            attacker: 0,
            victim: 1,
            joint: joint.as_u8(),
            flags: 0,
            energy_q16: 0,
            protection_q16: 0,
        }
    }

    /// Ticks until the first `Exsanguinated` for one cut joint.
    fn bleed_out_ticks_of(joint: JointId) -> u32 {
        let table = WoundTable::v1();
        let anatomy = AnatomyTable::rome_kit();
        let mut body = BodyState::new(&table);
        assert!(body.apply_cut(&table, &anatomy, &cut(joint, 1)));
        for tick in 1..=MAX_BLEED_OUT_TICKS {
            if body.step(&table) == Some(LimbEvent::Exsanguinated) {
                return tick;
            }
        }
        panic!("never bled out");
    }

    #[test]
    fn the_shipped_table_validates_and_digests_stably() {
        let table = WoundTable::v1();
        assert_eq!(table.validate(), Ok(()));
        assert_eq!(table.digest(), WoundTable::v1().digest());
        let mut other = table;
        other.faint_blood_q16 += 1;
        assert_ne!(table.digest(), other.digest());
    }

    #[test]
    fn bad_tables_refuse_with_the_first_failure() {
        let good = WoundTable::v1();
        let cases = [
            (
                WoundTable { version: 0, ..good },
                LimbLossError::BadVersion(0),
            ),
            (
                WoundTable {
                    blood_full_q16: 0,
                    ..good
                },
                LimbLossError::BadBlood(0),
            ),
            (
                WoundTable {
                    blood_full_q16: u32::MAX,
                    ..good
                },
                LimbLossError::BadBlood(u32::MAX),
            ),
            (
                WoundTable {
                    bleed_out_ticks: [100, 100, 50],
                    ..good
                },
                LimbLossError::BadBleedOut(1),
            ),
            (
                WoundTable {
                    bleed_out_ticks: [0, 0, 0],
                    ..good
                },
                LimbLossError::BadBleedOut(0),
            ),
            (
                WoundTable {
                    surrender_shock_q16: 0,
                    ..good
                },
                LimbLossError::BadThreshold("surrender_shock"),
            ),
            (
                WoundTable {
                    faint_blood_q16: good.surrender_blood_q16,
                    ..good
                },
                LimbLossError::BadThreshold("blood lines"),
            ),
        ];
        for (table, want) in cases {
            assert_eq!(table.validate(), Err(want));
        }
        let mut row = good;
        row.disable_at_q16[5] = row.wound_at_q16[5];
        assert_eq!(row.validate(), Err(LimbLossError::BadJointRow(5)));
    }

    #[test]
    fn bleed_out_times_hit_the_table_within_one_tick_twice() {
        let table = WoundTable::v1();
        for (joint, class) in [
            (JointId::ElbowR, BleedClass::Venous),
            (JointId::ShoulderL, BleedClass::Arterial),
            (JointId::Neck, BleedClass::Arterial),
            (JointId::KneeR, BleedClass::Venous),
        ] {
            assert_eq!(table.stump_class[joint.index()], class);
            let want = table.bleed_out_of(class);
            let first = bleed_out_ticks_of(joint);
            let second = bleed_out_ticks_of(joint);
            assert_eq!(first, second, "{joint:?} replays");
            assert!(
                first.abs_diff(want) <= 1,
                "{joint:?}: {first} ticks vs table {want}"
            );
        }
        // Ooze: a wounded joint, no cut.
        let mut body = BodyState::new(&table);
        body.apply_wound(
            &table,
            JointId::KneeL,
            table.wound_at_q16[JointId::KneeL.index()],
        );
        let mut ticks = 0u32;
        while ticks < MAX_BLEED_OUT_TICKS {
            ticks += 1;
            if body.step(&table) == Some(LimbEvent::Exsanguinated) {
                break;
            }
        }
        assert!(
            ticks.abs_diff(table.bleed_out_of(BleedClass::Ooze)) <= 1,
            "ooze {ticks}"
        );
    }

    #[test]
    fn a_cut_severs_everything_below_and_only_the_root_bleeds() {
        let table = WoundTable::v1();
        let anatomy = AnatomyTable::rome_kit();
        let mut body = BodyState::new(&table);
        // Wrist first, then the shoulder above it takes the whole arm.
        assert!(body.apply_cut(&table, &anatomy, &cut(JointId::WristR, 1)));
        assert_eq!(body.open_wounds(&table), [0, 1, 0]);
        assert!(body.apply_cut(&table, &anatomy, &cut(JointId::ShoulderR, 2)));
        for joint in [JointId::ShoulderR, JointId::ElbowR, JointId::WristR] {
            assert_eq!(body.state_of(joint), StumpState::Severed, "{joint:?}");
        }
        assert_eq!(body.state_of(JointId::ShoulderL), StumpState::Intact);
        assert_eq!(
            body.open_wounds(&table),
            [0, 0, 1],
            "one arterial stump only"
        );
        // A cut at a joint already gone changes nothing.
        let before = body.digest();
        assert!(!body.apply_cut(&table, &anatomy, &cut(JointId::ElbowR, 3)));
        assert_eq!(body.digest(), before);
        // The pelvis never cuts; an unknown joint is ignored, never a panic.
        assert!(!body.apply_cut(&table, &anatomy, &cut(JointId::Pelvis, 4)));
        let mut bad = cut(JointId::Neck, 5);
        bad.joint = 200;
        assert!(!body.apply_cut(&table, &anatomy, &bad));
        assert_eq!(body.severed_mask().count_ones(), 3);
        assert_eq!(body.working_mask() & body.severed_mask(), 0);
    }

    #[test]
    fn wounds_step_toward_worse_only() {
        let table = WoundTable::v1();
        let mut body = BodyState::new(&table);
        let joint = JointId::ElbowL;
        let (w, d) = (
            table.wound_at_q16[joint.index()],
            table.disable_at_q16[joint.index()],
        );
        assert_eq!(body.apply_wound(&table, joint, w - 1), StumpState::Intact);
        assert_eq!(body.apply_wound(&table, joint, 1), StumpState::Wounded);
        assert_eq!(body.apply_wound(&table, joint, d - w), StumpState::Disabled);
        assert_eq!(body.apply_wound(&table, joint, 0), StumpState::Disabled);
        assert_eq!(
            body.apply_wound(&table, JointId::Pelvis, d),
            StumpState::Intact
        );
    }

    #[test]
    fn the_wound_matrix_replays_twice() {
        let table = WoundTable::v1();
        let anatomy = AnatomyTable::rome_kit();
        let run = || {
            let mut digests = Vec::new();
            for joint in JointId::all() {
                for step in 0..9u32 {
                    let mut body = BodyState::new(&table);
                    body.apply_wound(&table, joint, step * (3 << 16));
                    if step % 3 == 2 {
                        body.apply_cut(&table, &anatomy, &cut(joint, step));
                    }
                    for _ in 0..300 {
                        let _ = body.step(&table);
                    }
                    digests.push(body.digest());
                }
            }
            digests
        };
        let a = run();
        assert_eq!(a, run());
        assert_eq!(a.len(), JOINT_TOTAL * 9);
        let mut h = Fnv64::new();
        for d in &a {
            h.write_u64(*d);
        }
        println!("wound matrix digest={:016x} cells={}", h.finish(), a.len());
    }

    #[test]
    fn shock_and_blood_loss_drive_surrender_faint_and_death_in_order() {
        let table = WoundTable::v1();
        let anatomy = AnatomyTable::rome_kit();
        // A neck cut: shock 80 then arterial shock climbs past 100 fast.
        let mut body = BodyState::new(&table);
        body.apply_cut(&table, &anatomy, &cut(JointId::Neck, 1));
        let mut events = Vec::new();
        for tick in 1..=2_000u32 {
            if let Some(e) = body.step(&table) {
                events.push((tick, e));
            }
        }
        let kinds: Vec<LimbEvent> = events.iter().map(|(_, e)| *e).collect();
        assert_eq!(
            kinds,
            vec![
                LimbEvent::Surrender,
                LimbEvent::Faint,
                LimbEvent::Exsanguinated
            ]
        );
        assert!(events[0].0 < events[1].0 && events[1].0 < events[2].0);
        assert!(body.dead() && body.surrendered() && body.fainted());
        // Dead bodies stay put: no more events, no more changes.
        let frozen = body.blood_q16;
        assert_eq!(body.step(&table), None);
        assert_eq!(body.blood_q16, frozen);
        assert!(!body.apply_cut(&table, &anatomy, &cut(JointId::ElbowL, 9)));
    }

    #[test]
    fn a_hand_cut_makes_the_fighter_yield_before_he_bleeds_out() {
        let table = WoundTable::v1();
        let anatomy = AnatomyTable::rome_kit();
        let mut body = BodyState::new(&table);
        body.apply_cut(&table, &anatomy, &cut(JointId::WristR, 1));
        let mut yielded = None;
        let mut dead = None;
        for tick in 1..=MAX_BLEED_OUT_TICKS {
            match body.step(&table) {
                Some(LimbEvent::Surrender) => yielded = Some(tick),
                Some(LimbEvent::Exsanguinated) => {
                    dead = Some(tick);
                    break;
                }
                _ => {}
            }
        }
        let (y, d) = (yielded.expect("yields"), dead.expect("dies"));
        assert!(y < d, "yields at {y}, dies at {d}");
    }

    #[test]
    fn impact_trauma_tracks_shock_and_blood() {
        let table = WoundTable::v1();
        let anatomy = AnatomyTable::rome_kit();
        let fresh = BodyState::new(&table);
        assert_eq!(fresh.impact_trauma(&table), Trauma::from_permille(0));
        assert_eq!(fresh.impact_shake(&table, 7, 3), (0.0, 0.0));
        let mut hurt = BodyState::new(&table);
        assert!(hurt.apply_cut(&table, &anatomy, &cut(JointId::Neck, 1)));
        for _ in 0..60 {
            let _ = hurt.step(&table);
        }
        let first = hurt.impact_trauma(&table);
        assert!(first.amplitude() > 0.0, "a neck cut shakes");
        assert_eq!(first, hurt.impact_trauma(&table), "same body replays");
        assert_eq!(
            hurt.impact_shake(&table, 7, 3),
            crate::trauma_shake(7, first, 3)
        );
    }

    #[test]
    fn body_state_is_copy_and_the_step_allocates_nothing() {
        let table = WoundTable::v1();
        let anatomy = AnatomyTable::rome_kit();
        let mut body = BodyState::new(&table);
        body.apply_cut(&table, &anatomy, &cut(JointId::KneeR, 1));
        let ((), allocs) = engine_core::alloc_probe::measure(|| {
            for _ in 0..600 {
                let _ = body.step(&table);
            }
        });
        assert_eq!(allocs.calls, 0);
    }
}

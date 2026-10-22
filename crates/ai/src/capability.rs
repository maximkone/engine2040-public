//! capability: the typed capability observation a game brain reads after a
//! cut, and the reaction rule over it: hands, legs, blood, morale.
//!
//! Consumer: `arena` builds one [`CapabilityView`] per brain per tick from the
//! fighter's joint mask, blood and morale (`engine-combat`'s
//! `limbloss::BodyState` is the body owner); a planner calls [`react`] with
//! the shipped [`CapabilityTable`] and acts on the [`Plan`]. Data owner: the
//! arena owns the body; a view is a plain copy rebuilt every tick. Units:
//! permille for blood and morale (`0..=1000`), counts for hands and legs.
//! Determinism: integer-only, no clock, no RNG, total over every input, so
//! the same view and table give the same plan and replay byte-equal. Errors:
//! [`CapabilityError`] for a table outside its fences; user data never
//! panics. Budget: constant work per call, `Copy` in and out, zero
//! allocation.
//!
//! The whole view ([`CapabilityView::WHOLE`]) plans [`Choice::PressOn`] with
//! the two-hand set, so a brain that lost nothing decides as it did before
//! this module existed.
//!
//! The proof reads two one-armed populations: an exhaustive grid over every
//! reading, and a seeded set drawn from the keyed counter stream, so the same
//! seeds give the same fixtures on every platform.

use engine_core::Fnv64;

/// Wire version of the shipped capability table.
pub const CAPABILITY_VERSION_V1: u32 = 1;
/// Highest legal blood reading, permille.
pub const BLOOD_MAX: u16 = 1_000;
/// Highest legal morale reading, permille.
pub const MORALE_MAX: u16 = 1_000;
/// Most morale one witnessed cut may cost, permille.
pub const WITNESS_COST_MAX: u16 = 400;

/// Usable limbs of one kind, most first.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Limbs {
    /// Both limbs work.
    #[default]
    Two,
    /// One limb works.
    One,
    /// None work.
    None,
}

impl Limbs {
    /// Quantize a raw limb count: two or more reads as [`Limbs::Two`].
    #[must_use]
    pub const fn from_count(count: u8) -> Self {
        match count {
            0 => Self::None,
            1 => Self::One,
            _ => Self::Two,
        }
    }

    /// Raw count, `0..=2`.
    #[must_use]
    pub const fn count(self) -> u8 {
        match self {
            Self::Two => 2,
            Self::One => 1,
            Self::None => 0,
        }
    }
}

/// What the remaining arms can hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveSet {
    /// Both hands on the two-handed set.
    TwoHand,
    /// One hand: the one-hand set.
    OneHand,
    /// No hand holds a weapon: shield, kicks and head only.
    Unarmed,
}

/// What the planner does this tick, worst last.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    /// Nothing lost: keep fighting.
    PressOn,
    /// A two-handed weapon no longer fits the hands: drop it.
    DropWeapon,
    /// Fight the one-hand set with the hand that is left.
    OneHand,
    /// Give ground and keep the guard.
    Retreat,
    /// Stop fighting.
    Yield,
}

/// Why a plan looks like it does; every plan carries one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reason {
    /// Nothing lost.
    Whole,
    /// One leg left: standing is possible but slow.
    OneLeg,
    /// One hand left: the one-hand set.
    LostHand,
    /// Both hands gone: no weapon can be held.
    NoHands,
    /// Both legs gone: the body cannot stand.
    NoLegs,
    /// Blood at or under the retreat line.
    Bleeding,
    /// Morale at or under the break line.
    MoraleBroken,
}

impl Reason {
    /// Short, stable debug text: never empty.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Whole => "whole: press on",
            Self::OneLeg => "one leg: press on, lame",
            Self::LostHand => "hand lost: one-hand set",
            Self::NoHands => "no hands: yield",
            Self::NoLegs => "no legs: retreat",
            Self::Bleeding => "blood low: retreat",
            Self::MoraleBroken => "morale broken: yield",
        }
    }
}

/// One tick's decision with its debug reason.
///
/// `move_set` says what the body can still use; `choice` says what the will
/// does. A yielding one-armed fighter still reports [`MoveSet::OneHand`], and
/// `drop_weapon` stays set until the arena reports the weapon gone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Plan {
    /// The verb the planner runs this tick.
    pub choice: Choice,
    /// The set the body can still fight with.
    pub move_set: MoveSet,
    /// True while a two-handed weapon no longer fits the hands.
    pub drop_weapon: bool,
    /// Why this plan; never absent.
    pub reason: Reason,
}

/// One fighter's capability as a brain sees it, quantized once at the edge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CapabilityView {
    hands: Limbs,
    legs: Limbs,
    blood_permille: u16,
    morale_permille: u16,
    weapon_hands: u8,
}

impl CapabilityView {
    /// Nothing lost: two hands, two legs, full blood and morale, a
    /// two-handed weapon held.
    pub const WHOLE: Self = Self {
        hands: Limbs::Two,
        legs: Limbs::Two,
        blood_permille: BLOOD_MAX,
        morale_permille: MORALE_MAX,
        weapon_hands: 2,
    };

    /// Quantize raw readings. Limb counts above two read as two, blood and
    /// morale cap at 1000, and a weapon needs at least one hand. Total over
    /// all inputs.
    #[must_use]
    pub fn quantize(
        hands: u8,
        legs: u8,
        blood_permille: u16,
        morale_permille: u16,
        weapon_hands: u8,
    ) -> Self {
        Self {
            hands: Limbs::from_count(hands),
            legs: Limbs::from_count(legs),
            blood_permille: blood_permille.min(BLOOD_MAX),
            morale_permille: morale_permille.min(MORALE_MAX),
            weapon_hands: weapon_hands.clamp(1, 2),
        }
    }

    /// Hands that can still hold a weapon.
    #[must_use]
    pub const fn hands(&self) -> Limbs {
        self.hands
    }

    /// Legs that can still stand.
    #[must_use]
    pub const fn legs(&self) -> Limbs {
        self.legs
    }

    /// Blood left, permille of a whole body.
    #[must_use]
    pub const fn blood_permille(&self) -> u16 {
        self.blood_permille
    }

    /// Morale, permille.
    #[must_use]
    pub const fn morale_permille(&self) -> u16 {
        self.morale_permille
    }

    /// Hands the held weapon needs.
    #[must_use]
    pub const fn weapon_hands(&self) -> u8 {
        self.weapon_hands
    }
}

/// Versioned lines and costs of the reaction rule. Fixed, `Copy`, no heap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CapabilityTable {
    /// Table version, [`CAPABILITY_VERSION_V1`] for the shipped row.
    pub version: u32,
    /// Morale at or under this line breaks the will to fight.
    pub morale_break: u16,
    /// Blood at or under this line (permille) calls a retreat.
    pub blood_retreat: u16,
    /// Morale a full-severity witnessed cut takes, permille.
    pub witness_cut_cost: u16,
}

/// Why a capability table was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CapabilityError {
    /// Version 0 is reserved.
    BadVersion(u32),
    /// The morale break line is past [`MORALE_MAX`].
    BadMoraleBreak(u16),
    /// The blood retreat line is past [`BLOOD_MAX`].
    BadBloodRetreat(u16),
    /// The witnessed-cut cost is past [`WITNESS_COST_MAX`].
    BadWitnessCost(u16),
}

impl core::fmt::Display for CapabilityError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadVersion(v) => write!(f, "capability table version {v} is not valid"),
            Self::BadMoraleBreak(v) => write!(f, "morale break {v} is outside 0..=1000"),
            Self::BadBloodRetreat(v) => write!(f, "blood retreat {v} is outside 0..=1000"),
            Self::BadWitnessCost(v) => write!(f, "witness cut cost {v} is outside 0..=400"),
        }
    }
}

impl std::error::Error for CapabilityError {}

impl CapabilityTable {
    /// Shipped table: yields at or under 250 morale, retreats at or under
    /// 300 blood permille, loses up to 120 morale for a full witnessed cut.
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            version: CAPABILITY_VERSION_V1,
            morale_break: 250,
            blood_retreat: 300,
            witness_cut_cost: 120,
        }
    }

    /// Refuse a table outside its fences. The first bad field wins.
    pub fn validate(&self) -> Result<(), CapabilityError> {
        if self.version == 0 {
            return Err(CapabilityError::BadVersion(self.version));
        }
        if self.morale_break > MORALE_MAX {
            return Err(CapabilityError::BadMoraleBreak(self.morale_break));
        }
        if self.blood_retreat > BLOOD_MAX {
            return Err(CapabilityError::BadBloodRetreat(self.blood_retreat));
        }
        if self.witness_cut_cost > WITNESS_COST_MAX {
            return Err(CapabilityError::BadWitnessCost(self.witness_cut_cost));
        }
        Ok(())
    }

    /// Digest over every field in order.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u32(self.version);
        h.write_u16(self.morale_break);
        h.write_u16(self.blood_retreat);
        h.write_u16(self.witness_cut_cost);
        h.finish()
    }
}

/// Morale after witnessing one cut of `severity_permille` (`0` a scratch,
/// `1000` a limb taken): the table cost scaled by severity, floored at zero.
/// Pure, integer, total; the same tape gives the same morale on every tick.
#[must_use]
pub fn witness_cut(table: &CapabilityTable, morale_permille: u16, severity_permille: u16) -> u16 {
    let severity = u32::from(severity_permille.min(MORALE_MAX));
    let cost = u32::from(table.witness_cut_cost) * severity / u32::from(MORALE_MAX);
    let cost = u16::try_from(cost).unwrap_or(u16::MAX);
    morale_permille.min(MORALE_MAX).saturating_sub(cost)
}

/// The plan for this view. Total over every input; checks run worst first:
/// no hands, broken morale, no legs, low blood, then one hand, then press on.
#[must_use]
pub fn react(table: &CapabilityTable, view: &CapabilityView) -> Plan {
    let move_set = move_set_for(view);
    if view.hands == Limbs::None {
        return Plan {
            choice: Choice::Yield,
            move_set,
            drop_weapon: false,
            reason: Reason::NoHands,
        };
    }
    if view.morale_permille <= table.morale_break {
        return Plan {
            choice: Choice::Yield,
            move_set,
            drop_weapon: false,
            reason: Reason::MoraleBroken,
        };
    }
    if view.legs == Limbs::None {
        return Plan {
            choice: Choice::Retreat,
            move_set,
            drop_weapon: false,
            reason: Reason::NoLegs,
        };
    }
    if view.blood_permille <= table.blood_retreat {
        return Plan {
            choice: Choice::Retreat,
            move_set,
            drop_weapon: false,
            reason: Reason::Bleeding,
        };
    }
    if view.hands == Limbs::One {
        let drop_weapon = view.weapon_hands == 2;
        return Plan {
            choice: if drop_weapon {
                Choice::DropWeapon
            } else {
                Choice::OneHand
            },
            move_set,
            drop_weapon,
            reason: Reason::LostHand,
        };
    }
    Plan {
        choice: Choice::PressOn,
        move_set,
        drop_weapon: false,
        reason: if view.legs == Limbs::One {
            Reason::OneLeg
        } else {
            Reason::Whole
        },
    }
}

/// The set the body can still fight with: no hand means unarmed, one hand or
/// a one-hand weapon means the one-hand set, else the two-hand set.
fn move_set_for(view: &CapabilityView) -> MoveSet {
    if view.hands == Limbs::None {
        MoveSet::Unarmed
    } else if view.hands == Limbs::One || view.weapon_hands == 1 {
        MoveSet::OneHand
    } else {
        MoveSet::TwoHand
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;
    use engine_core::{CounterCursor, CounterRng};

    /// Seeds of the seeded one-armed fixture sweep.
    const FIXTURE_SEEDS: [u64; 4] = [2040, 2041, 2042, 2043];
    /// One-armed readings drawn per seed.
    const FIXTURES_PER_SEED: usize = 512;
    /// Lane the witnessed-cut severities come from, so the tape stream never
    /// shadows the fixture stream.
    const TAPE_LANE: u64 = 1;
    /// Cuts in the witnessed-cut tape.
    const TAPE_CUTS: usize = 16;

    /// One seeded one-armed reading: what a brain sees after it lost a hand.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Fixture {
        legs: u8,
        blood_permille: u16,
        morale_permille: u16,
        weapon_hands: u8,
    }

    impl Fixture {
        /// The capability view this reading is.
        fn view(self) -> CapabilityView {
            CapabilityView::quantize(
                1,
                self.legs,
                self.blood_permille,
                self.morale_permille,
                self.weapon_hands,
            )
        }
    }

    /// The seeded one-armed readings of one seed, in draw order.
    fn one_armed_fixtures(seed: u64) -> Vec<Fixture> {
        let mut cursor = CounterCursor::new(seed);
        (0..FIXTURES_PER_SEED)
            .map(|_| {
                let word = cursor.next_u64();
                Fixture {
                    legs: limb_count(word & 0xff),
                    blood_permille: permille((word >> 8) & 0xffff),
                    morale_permille: permille((word >> 24) & 0xffff),
                    weapon_hands: 1 + u8::try_from((word >> 40) & 1).expect("one bit fits in u8"),
                }
            })
            .collect()
    }

    /// The seeded severities of one witnessed-cut tape, in draw order.
    fn severity_tape(seed: u64) -> Vec<u16> {
        let mut cursor = CounterCursor::from_rng(CounterRng::new(seed).fork(TAPE_LANE), 0);
        (0..TAPE_CUTS)
            .map(|_| permille((cursor.next_u64() >> 8) & 0xffff))
            .collect()
    }

    /// Legs that still stand from a drawn byte: 0, 1 or 2.
    fn limb_count(byte: u64) -> u8 {
        u8::try_from(byte % 3).expect("a byte under three fits in u8")
    }

    /// A permille reading from a drawn word: 0..=1000.
    fn permille(word: u64) -> u16 {
        u16::try_from(word % 1_001).expect("a word under 1001 fits in u16")
    }

    /// The reasons each verb may carry; a plan outside the table is a bug.
    fn legal_reasons(choice: Choice) -> &'static [Reason] {
        match choice {
            Choice::PressOn => &[Reason::Whole, Reason::OneLeg],
            Choice::DropWeapon => &[Reason::LostHand],
            Choice::OneHand => &[Reason::LostHand],
            Choice::Retreat => &[Reason::NoLegs, Reason::Bleeding],
            Choice::Yield => &[Reason::NoHands, Reason::MoraleBroken],
        }
    }

    #[test]
    fn shipped_table_validates_and_digests_stably() {
        let table = CapabilityTable::v1();
        assert_eq!(table.validate(), Ok(()));
        assert_eq!(table.digest(), CapabilityTable::v1().digest());
        let other = CapabilityTable {
            witness_cut_cost: 121,
            ..table
        };
        assert_ne!(table.digest(), other.digest());
    }

    #[test]
    fn table_fences_refuse_each_bad_field() {
        let good = CapabilityTable::v1();
        assert_eq!(
            CapabilityTable { version: 0, ..good }.validate(),
            Err(CapabilityError::BadVersion(0))
        );
        assert_eq!(
            CapabilityTable {
                morale_break: 1_001,
                ..good
            }
            .validate(),
            Err(CapabilityError::BadMoraleBreak(1_001))
        );
        assert_eq!(
            CapabilityTable {
                blood_retreat: 1_001,
                ..good
            }
            .validate(),
            Err(CapabilityError::BadBloodRetreat(1_001))
        );
        assert_eq!(
            CapabilityTable {
                witness_cut_cost: 401,
                ..good
            }
            .validate(),
            Err(CapabilityError::BadWitnessCost(401))
        );
    }

    #[test]
    fn view_quantizes_every_reading_into_range() {
        let view = CapabilityView::quantize(7, 2, 5_000, 4_000, 9);
        assert_eq!(view.hands(), Limbs::Two);
        assert_eq!(view.legs(), Limbs::Two);
        assert_eq!(view.blood_permille(), BLOOD_MAX);
        assert_eq!(view.morale_permille(), MORALE_MAX);
        assert_eq!(view.weapon_hands(), 2);
        let view = CapabilityView::quantize(0, 0, 0, 0, 0);
        assert_eq!(view.hands(), Limbs::None);
        assert_eq!(view.legs(), Limbs::None);
        assert_eq!(view.weapon_hands(), 1);
        assert_eq!(Limbs::from_count(1).count(), 1);
        assert_eq!(
            CapabilityView::WHOLE,
            CapabilityView::quantize(2, 2, 1_000, 1_000, 2)
        );
    }

    #[test]
    fn whole_fighter_presses_on_with_the_two_hand_set() {
        let plan = react(&CapabilityTable::v1(), &CapabilityView::WHOLE);
        assert_eq!(plan.choice, Choice::PressOn);
        assert_eq!(plan.move_set, MoveSet::TwoHand);
        assert!(!plan.drop_weapon);
        assert_eq!(plan.reason, Reason::Whole);
    }

    /// Accept: 99% of one-armed fixtures switch to a one-hand move set.
    #[test]
    fn one_armed_fixtures_switch_to_one_hand_moves() {
        let table = CapabilityTable::v1();
        let mut armed = 0u32;
        let mut one_hand = 0u32;
        let mut fighting = 0u32;
        let mut drop = 0u32;
        for morale in (0..=1_000u16).step_by(50) {
            for blood in (0..=1_000u16).step_by(50) {
                for legs in 0..=2u8 {
                    for weapon_hands in 1..=2u8 {
                        let view = CapabilityView::quantize(1, legs, blood, morale, weapon_hands);
                        let plan = react(&table, &view);
                        armed += 1;
                        if plan.move_set == MoveSet::OneHand {
                            one_hand += 1;
                        }
                        if plan.reason == Reason::LostHand {
                            fighting += 1;
                            assert!(
                                matches!(plan.choice, Choice::DropWeapon | Choice::OneHand),
                                "{view:?} fights one-handed with {:?}",
                                plan.choice
                            );
                            if plan.drop_weapon {
                                drop += 1;
                            }
                        }
                        assert!(
                            matches!(
                                plan.reason,
                                Reason::LostHand
                                    | Reason::MoraleBroken
                                    | Reason::NoLegs
                                    | Reason::Bleeding
                            ),
                            "{view:?} kept a stale reason: {:?}",
                            plan.reason
                        );
                    }
                }
            }
        }
        let permille = one_hand * 1_000 / armed;
        println!(
            "one-armed fixtures={armed} one-hand={one_hand} ratio={permille} permille \
             fighting={fighting} drop={drop}"
        );
        assert!(
            permille >= 990,
            "one-hand ratio {permille} permille is under 99%"
        );
        assert_eq!(one_hand, armed, "every one-armed body fights one-handed");
        assert!(fighting > 0, "some one-armed fixtures still fight");
    }

    /// Accept, seeded: the same 99% bar over a seeded one-armed population,
    /// drawn from the keyed counter stream so the set is reproducible.
    #[test]
    fn seeded_one_armed_fixtures_switch_to_one_hand_moves() {
        let table = CapabilityTable::v1();
        let mut armed = 0u32;
        let mut one_hand = 0u32;
        let mut fighting = 0u32;
        let mut drop = 0u32;
        let mut yielded = 0u32;
        let mut retreated = 0u32;
        let mut digest = Fnv64::new();
        for seed in FIXTURE_SEEDS {
            for fixture in one_armed_fixtures(seed) {
                let view = fixture.view();
                let plan = react(&table, &view);
                armed += 1;
                // The reading itself, in draw order: same set, same digest.
                digest.write_u8(1);
                digest.write_u8(fixture.legs);
                digest.write_u16(fixture.blood_permille);
                digest.write_u16(fixture.morale_permille);
                digest.write_u8(fixture.weapon_hands);
                // The accept: a one-armed body moves to the one-hand set.
                if plan.move_set == MoveSet::OneHand {
                    one_hand += 1;
                }
                match plan.choice {
                    Choice::DropWeapon | Choice::OneHand => {
                        fighting += 1;
                        if plan.drop_weapon {
                            drop += 1;
                        }
                    }
                    Choice::Yield => yielded += 1,
                    Choice::Retreat => retreated += 1,
                    Choice::PressOn => panic!("a one-armed body pressed on with both hands"),
                }
                assert!(
                    legal_reasons(plan.choice).contains(&plan.reason),
                    "{view:?} planned {:?} with the reason {:?}",
                    plan.choice,
                    plan.reason
                );
            }
        }
        let permille = one_hand * 1_000 / armed;
        println!(
            "seeded one-armed fixtures={armed} one-hand move set={one_hand} \
             ratio={permille} permille fighting={fighting} drop={drop} \
             yielded={yielded} retreated={retreated} digest={:016x}",
            digest.finish()
        );
        assert!(
            permille >= 990,
            "one-hand move set ratio {permille} permille is under 99%"
        );
        assert_eq!(
            one_hand, armed,
            "every one-armed body moves to the one-hand set"
        );
        assert!(fighting > 0, "some seeded one-armed fixtures still fight");
        assert!(
            yielded > 0,
            "the seeded set reaches the broken-morale yield"
        );
        assert!(retreated > 0, "the seeded set reaches the wounded retreat");
    }

    /// The second arm of the accept: among the one-armed fixtures still in
    /// the fight, every one fights the one-hand set.
    #[test]
    fn seeded_one_armed_fixtures_still_fighting_use_the_one_hand_set() {
        let table = CapabilityTable::v1();
        let mut in_fight = 0u32;
        let mut one_hand = 0u32;
        let mut drop = 0u32;
        for seed in FIXTURE_SEEDS {
            for fixture in one_armed_fixtures(seed) {
                let view = fixture.view();
                let plan = react(&table, &view);
                // "Still in the fight": the body can stand, still has blood
                // and its morale is not broken, so the switch is the live
                // question for the planner.
                if fixture.morale_permille <= table.morale_break
                    || fixture.legs == 0
                    || fixture.blood_permille <= table.blood_retreat
                {
                    continue;
                }
                in_fight += 1;
                assert_eq!(
                    plan.move_set,
                    MoveSet::OneHand,
                    "{view:?} kept the two-hand set"
                );
                assert_eq!(plan.reason, Reason::LostHand);
                assert!(
                    matches!(plan.choice, Choice::DropWeapon | Choice::OneHand),
                    "{view:?} planned {:?} instead of a one-hand move",
                    plan.choice
                );
                one_hand += 1;
                if plan.drop_weapon {
                    drop += 1;
                }
            }
        }
        let permille = one_hand * 1_000 / in_fight;
        println!(
            "seeded one-armed still fighting={in_fight} one-hand={one_hand} \
             ratio={permille} permille drop={drop}"
        );
        assert!(
            in_fight > 0,
            "the seeded set has no fighter still in the fight"
        );
        assert!(
            permille >= 990,
            "one-hand ratio {permille} permille is under 99%"
        );
        assert_eq!(one_hand, in_fight);
        assert!(
            drop > 0,
            "some still-fighting fixtures drop the two-hand weapon"
        );
    }

    /// The seeded set is the same set on every run, and a different seed
    /// gives a different set.
    #[test]
    fn seeded_fixtures_replay_identically() {
        for seed in FIXTURE_SEEDS {
            let first = one_armed_fixtures(seed);
            let second = one_armed_fixtures(seed);
            assert_eq!(first, second, "seed {seed} drew two different sets");
        }
        let a = one_armed_fixtures(FIXTURE_SEEDS[0]);
        let b = one_armed_fixtures(FIXTURE_SEEDS[1]);
        assert_ne!(a, b, "two seeds drew the same set");
        // The plan over a set is the same plan on every run.
        let table = CapabilityTable::v1();
        let plans = |fixtures: &[Fixture]| {
            fixtures
                .iter()
                .map(|fixture| react(&table, &fixture.view()))
                .collect::<Vec<_>>()
        };
        assert_eq!(plans(&a), plans(&one_armed_fixtures(FIXTURE_SEEDS[0])));
    }

    /// Accept: the morale rule replays x2 and a witnessed cut can break it.
    #[test]
    fn morale_rule_replays_exactly() {
        let table = CapabilityTable::v1();
        let tape = [1_000u16, 250, 900, 0, 1_000, 400, 750, 1_000];
        let run = || {
            let mut morale = 600u16;
            let mut h = Fnv64::new();
            for severity in tape {
                morale = witness_cut(&table, morale, severity);
                h.write_u16(morale);
            }
            (
                morale,
                h.finish(),
                react(&table, &CapabilityView::quantize(2, 2, 1_000, morale, 2)),
            )
        };
        let (morale_a, digest_a, plan_a) = run();
        let (morale_b, digest_b, plan_b) = run();
        assert_eq!((morale_a, digest_a, plan_a), (morale_b, digest_b, plan_b));
        println!("morale tape end={morale_a} digest={digest_a:016x}");
        assert_eq!(morale_a, 0, "eight witnessed cuts empty neutral morale");
        assert_eq!(plan_a.choice, Choice::Yield);
        assert_eq!(plan_a.reason, Reason::MoraleBroken);
        // One full cut at 300 morale sits under the 250 line.
        assert_eq!(witness_cut(&table, 300, 1_000), 180);
        assert!(witness_cut(&table, 300, 1_000) <= table.morale_break);
        // A scratch costs nothing and zero morale cannot go negative.
        assert_eq!(witness_cut(&table, 600, 0), 600);
        assert_eq!(witness_cut(&table, 0, 1_000), 0);
    }

    /// Accept, seeded: the morale rule over a seeded witnessed-cut tape
    /// replays x2 to the same hash, and a witnessed cut can break it.
    #[test]
    fn seeded_morale_tape_replays_exactly() {
        let table = CapabilityTable::v1();
        let tape = severity_tape(FIXTURE_SEEDS[0]);
        let run = || {
            let mut morale = 600u16;
            let mut h = Fnv64::new();
            for severity in &tape {
                morale = witness_cut(&table, morale, *severity);
                h.write_u16(morale);
            }
            (
                morale,
                h.finish(),
                react(&table, &CapabilityView::quantize(2, 2, 1_000, morale, 2)),
            )
        };
        let (morale_a, digest_a, plan_a) = run();
        let (morale_b, digest_b, plan_b) = run();
        assert_eq!((morale_a, digest_a, plan_a), (morale_b, digest_b, plan_b));
        println!(
            "seeded morale tape cuts={} end={morale_a} digest={digest_a:016x}",
            tape.len()
        );
        // The tape breaks the will: the fighter yields at the end.
        assert_eq!(morale_a, 0, "the seeded tape empties neutral morale");
        assert_eq!(plan_a.choice, Choice::Yield);
        assert_eq!(plan_a.reason, Reason::MoraleBroken);
        // The tape is the same tape on every run.
        assert_eq!(tape, severity_tape(FIXTURE_SEEDS[0]));
        assert_ne!(tape, severity_tape(FIXTURE_SEEDS[1]));
        // A witnessed cut at the break line breaks it outright.
        let broken = witness_cut(&table, table.morale_break + 100, 1_000);
        assert!(
            broken <= table.morale_break,
            "a full witnessed cut must break the morale"
        );
        assert_eq!(
            react(&table, &CapabilityView::quantize(2, 2, 1_000, broken, 2)).reason,
            Reason::MoraleBroken
        );
    }

    /// Accept: every choice carries a debug reason, and each verb is reachable.
    #[test]
    fn every_choice_carries_a_debug_reason() {
        let table = CapabilityTable::v1();
        let mut seen = [false; 5];
        let mut reasons = Vec::new();
        for hands in 0..=2u8 {
            for legs in 0..=2u8 {
                for morale in (0..=1_000u16).step_by(100) {
                    for blood in (0..=1_000u16).step_by(100) {
                        for weapon_hands in 1..=2u8 {
                            let view =
                                CapabilityView::quantize(hands, legs, blood, morale, weapon_hands);
                            let plan = react(&table, &view);
                            let text = plan.reason.as_str();
                            assert!(!text.is_empty(), "{view:?} planned without a reason");
                            assert!(text.len() <= 32, "{view:?} carries a long reason: {text}");
                            assert!(
                                legal_reasons(plan.choice).contains(&plan.reason),
                                "{view:?} planned {:?} with the reason {:?}",
                                plan.choice,
                                plan.reason
                            );
                            seen[choice_slot(plan.choice)] = true;
                            if !reasons.contains(&text) {
                                reasons.push(text);
                            }
                        }
                    }
                }
            }
        }
        assert!(seen.iter().all(|&hit| hit), "an unreachable verb: {seen:?}");
        assert_eq!(
            reasons.len(),
            7,
            "one distinct reason per rule: {reasons:?}"
        );
    }

    fn choice_slot(choice: Choice) -> usize {
        match choice {
            Choice::PressOn => 0,
            Choice::DropWeapon => 1,
            Choice::OneHand => 2,
            Choice::Retreat => 3,
            Choice::Yield => 4,
        }
    }

    #[test]
    fn reactions_follow_the_worst_first_order() {
        let table = CapabilityTable::v1();
        // No hands beats broken morale.
        let view = CapabilityView::quantize(0, 2, 1_000, 0, 2);
        assert_eq!(react(&table, &view).reason, Reason::NoHands);
        // Broken morale beats lost legs.
        let view = CapabilityView::quantize(2, 0, 1_000, 250, 2);
        assert_eq!(react(&table, &view).reason, Reason::MoraleBroken);
        // Lost legs beat low blood.
        let view = CapabilityView::quantize(2, 0, 100, 600, 2);
        let plan = react(&table, &view);
        assert_eq!(
            (plan.choice, plan.reason),
            (Choice::Retreat, Reason::NoLegs)
        );
        assert_eq!(plan.move_set, MoveSet::TwoHand);
        // Low blood retreats with the guard up.
        let view = CapabilityView::quantize(2, 2, 300, 600, 2);
        let plan = react(&table, &view);
        assert_eq!(
            (plan.choice, plan.reason),
            (Choice::Retreat, Reason::Bleeding)
        );
        // One hand and a two-handed weapon: drop it, keep the one-hand set.
        let view = CapabilityView::quantize(1, 2, 1_000, 600, 2);
        let plan = react(&table, &view);
        assert_eq!((plan.choice, plan.drop_weapon), (Choice::DropWeapon, true));
        // One hand and a one-hand weapon: fight on as it is.
        let view = CapabilityView::quantize(1, 2, 1_000, 600, 1);
        let plan = react(&table, &view);
        assert_eq!((plan.choice, plan.drop_weapon), (Choice::OneHand, false));
        // One leg: press on, marked lame.
        let view = CapabilityView::quantize(2, 1, 1_000, 600, 2);
        assert_eq!(react(&table, &view).reason, Reason::OneLeg);
        // Tunable: the same fighter yields once the table's break line moves.
        let view = CapabilityView::quantize(2, 2, 1_000, 350, 2);
        assert_eq!(react(&table, &view).choice, Choice::PressOn);
        let strict = CapabilityTable {
            morale_break: 400,
            ..table
        };
        assert_eq!(react(&strict, &view).reason, Reason::MoraleBroken);
    }

    #[test]
    fn react_and_witness_cut_never_allocate() {
        let table = CapabilityTable::v1();
        let ((plan, morale), heap) = measure(|| {
            let mut last = react(&table, &CapabilityView::WHOLE);
            let mut morale = 1_000u16;
            for tick_morale in (0..=1_000u16).step_by(25) {
                for hands in 0..=2u8 {
                    let view = CapabilityView::quantize(hands, 2, 900, tick_morale, 1);
                    last = react(&table, &view);
                    morale = witness_cut(&table, morale, 500);
                }
            }
            (last, morale)
        });
        assert_eq!(
            heap.calls, 0,
            "capability plan allocated {} times",
            heap.calls
        );
        assert!(!plan.reason.as_str().is_empty());
        assert_eq!(morale, 0, "the tape bleeds morale to the floor");
    }
}

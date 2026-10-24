//! opponent: a four-bucket foe model plus the action lease it feeds.
//!
//! Consumer: [`DepthBrain`](crate::depth::DepthBrain), which re-exports the
//! lease as `depth::ActionLease` and reads the table only when its opponent
//! read is armed. Both are opt-in: the default brain leaves the lease at
//! `N = 0` and the table idle, so every decision stays byte-identical.
//! Data owner: this module owns the lease timer, the bucket counts and the
//! counter-lane bias. Units: ticks, millimetres, counts.
//! Determinism: integer only, no clock, no RNG; counts saturate and ties
//! keep the earlier [`Intent`] lane, so the same observations always give
//! the same prediction. Errors: none, every method is total.

use crate::depth::ActionLease;
use crate::melee::{MeleeView, WOUNDED_PERMILLE};
use crate::perception::{Sight, StagedAction};
use crate::{Brain, Intent, Percept};
use engine_input::ParticipantInput as BodyInput;

/// Range bands in the foe table: inside [`Brain::CLOSE_MM`] and beyond it.
pub const BANDS: usize = 2;
/// Foe action classes per bucket: pressing (attack/advance) and yielding
/// (block/dodge/retreat/reposition).
pub const CLASSES: usize = 2;
/// Buckets in the foe table: one per (band, last action class).
pub const BUCKETS: usize = BANDS * CLASSES;
/// Intent lanes a bucket counts, in [`Intent`] table order.
pub const ACTIONS: usize = 6;
/// Separation change (mm) that reads as the foe closing or opening.
pub const FOE_STEP_MM: i32 = 100;

/// One range band: close enough to trade blows, or farther out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RangeBand {
    /// At or inside [`Brain::CLOSE_MM`].
    Close,
    /// Beyond [`Brain::CLOSE_MM`].
    Far,
}

impl RangeBand {
    /// Band for a sanitized separation in millimetres. Negative values read
    /// as close, so a garbage reading never wraps into the far bucket.
    pub const fn of(dist_mm: i32) -> Self {
        if dist_mm <= Brain::CLOSE_MM {
            RangeBand::Close
        } else {
            RangeBand::Far
        }
    }
}

/// Bucket index for one key; total over every band and intent.
const fn bucket(band: RangeBand, last: Intent) -> usize {
    let row = match band {
        RangeBand::Close => 0,
        RangeBand::Far => 1,
    };
    row * CLASSES + class_of(last)
}

/// Action class: 0 pressing, 1 yielding. The two classes are the compression
/// that makes four buckets enough.
const fn class_of(intent: Intent) -> usize {
    match intent {
        Intent::Attack | Intent::Advance => 0,
        Intent::Block | Intent::Dodge | Intent::Retreat | Intent::Reposition => 1,
    }
}

/// Lane index in [`Intent`] table order, 0..=5. The one table the decision
/// ring (`crate::gladiator`) and this counter both read.
pub(crate) const fn lane_index(intent: Intent) -> u8 {
    match intent {
        Intent::Attack => 0,
        Intent::Block => 1,
        Intent::Dodge => 2,
        Intent::Advance => 3,
        Intent::Retreat => 4,
        Intent::Reposition => 5,
    }
}

/// Intent for one lane index; out-of-range lanes read as [`Intent::Advance`].
const fn intent_of(lane: usize) -> Intent {
    match lane {
        0 => Intent::Attack,
        1 => Intent::Block,
        2 => Intent::Dodge,
        3 => Intent::Advance,
        4 => Intent::Retreat,
        _ => Intent::Reposition,
    }
}

/// Four-bucket foe model: for each (range band, last action class) it counts
/// the intents the foe took next and predicts the most likely one.
///
/// `observe` is the pure update; `observe_foe` is the brain-facing read that
/// classifies the separation trend, updates the previous bucket and returns
/// the prediction for the current bucket. Cold counters only: no allocation,
/// no floating point. Counts saturate at `u32::MAX` so a long bout cannot
/// wrap a bucket. Ties keep the earlier [`Intent`] lane (attack first), and
/// an empty bucket also predicts [`Intent::Attack`], so the read is total.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OpponentTable {
    counts: [[u32; ACTIONS]; BUCKETS],
    last: Option<(RangeBand, Intent)>,
    last_dist_mm: Option<i32>,
}

impl OpponentTable {
    /// Empty table: no observations, every prediction is [`Intent::Attack`].
    pub const fn new() -> Self {
        Self {
            counts: [[0; ACTIONS]; BUCKETS],
            last: None,
            last_dist_mm: None,
        }
    }

    /// Drop every count and the last read (a new bout with a new foe).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Drop the last read at a round boundary but keep the learned counts.
    pub fn clear_read(&mut self) {
        self.last = None;
        self.last_dist_mm = None;
    }

    /// Count one observation: in `band`, after `last`, the foe did `next`.
    pub fn observe(&mut self, band: RangeBand, last: Intent, next: Intent) {
        let cell = &mut self.counts[bucket(band, last)][usize::from(lane_index(next))];
        *cell = cell.saturating_add(1);
    }

    /// Most likely next intent in `band` after `last`; ties keep the earlier
    /// lane, an empty bucket predicts [`Intent::Attack`].
    pub fn predict(&self, band: RangeBand, last: Intent) -> Intent {
        let row = &self.counts[bucket(band, last)];
        let mut best = 0usize;
        for lane in 1..ACTIONS {
            if row[lane] > row[best] {
                best = lane;
            }
        }
        intent_of(best)
    }

    /// Count held on one observation cell.
    pub fn count(&self, band: RangeBand, last: Intent, next: Intent) -> u32 {
        self.counts[bucket(band, last)][usize::from(lane_index(next))]
    }

    /// Observations counted in one bucket.
    pub fn observed(&self, band: RangeBand, last: Intent) -> u32 {
        self.counts[bucket(band, last)]
            .iter()
            .fold(0u32, |sum, n| sum.saturating_add(*n))
    }

    /// Read the foe this tick, learn from the previous read and predict the
    /// next action. `foe_attacking` is the brain's noticed attack bit; the
    /// separation trend fills in advance/retreat and standing still reads
    /// as guard. Total: the first read only seeds the trend.
    pub fn observe_foe(&mut self, foe_attacking: bool, dist_mm: i32) -> Intent {
        let action = read_foe_action(foe_attacking, self.last_dist_mm, dist_mm);
        if let Some((band, last)) = self.last {
            self.observe(band, last, action);
        }
        let band = RangeBand::of(dist_mm);
        self.last = Some((band, action));
        self.last_dist_mm = Some(dist_mm);
        self.predict(band, action)
    }

    /// Read the foe through the perception ring instead of the live frame
    /// (SW85): the sight is one reaction beat old and carries no intent, so a
    /// press that started inside that beat is not answered. `None` while the
    /// ring is still filling, so an opt-in caller keeps its current read.
    pub fn observe_sighted(&mut self, sight: Option<Sight>) -> Option<Intent> {
        let sight = sight?;
        let attacking = sight.action == StagedAction::Strike;
        Some(self.observe_foe(attacking, i32::from(sight.dist_mm)))
    }
}

impl Default for OpponentTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Coarse foe action from the noticed attack bit and the separation trend.
fn read_foe_action(foe_attacking: bool, last_dist_mm: Option<i32>, dist_mm: i32) -> Intent {
    if foe_attacking {
        return Intent::Attack;
    }
    match last_dist_mm {
        Some(prev) if dist_mm.saturating_add(FOE_STEP_MM) < prev => Intent::Advance,
        Some(prev) if dist_mm > prev.saturating_add(FOE_STEP_MM) => Intent::Retreat,
        _ => Intent::Block,
    }
}

/// Counter-lane bias in scripted table order for one predicted foe action.
/// Bounded to `+/-80`; one predicted lane never outweighs a curve swing.
pub const fn counter_bias(predicted: Intent) -> [i32; 5] {
    match predicted {
        Intent::Attack => [0, 40, 40, 0, 0],
        Intent::Block => [40, 0, 40, 0, 0],
        Intent::Dodge => [40, 40, 0, 0, 0],
        Intent::Advance => [30, 40, 30, 0, 0],
        Intent::Retreat => [70, 0, 0, 80, 0],
        Intent::Reposition => [30, 20, 20, 20, 0],
    }
}

/// Abort bits for an armed [`ActionLease`]. Any bit clears the hold and the
/// brain takes the fresh choice that tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LeaseThreats(u8);

impl LeaseThreats {
    /// No threat: the hold survives.
    pub const NONE: Self = Self(0);
    /// The foe's blow is in windup or on its impact tick.
    pub const INCOMING_ATTACK: Self = Self(1 << 0);
    /// Own hp sits below [`WOUNDED_PERMILLE`].
    pub const LOW_HP: Self = Self(1 << 1);
    /// No foe left in view (or no usable separation).
    pub const FOE_GONE: Self = Self(1 << 2);

    /// Bits for one observation. `foe_visible` false raises [`Self::FOE_GONE`].
    pub const fn observe(incoming: bool, hp_permille: u16, foe_visible: bool) -> Self {
        let mut bits = 0u8;
        if incoming {
            bits |= Self::INCOMING_ATTACK.0;
        }
        if hp_permille < WOUNDED_PERMILLE {
            bits |= Self::LOW_HP.0;
        }
        if !foe_visible {
            bits |= Self::FOE_GONE.0;
        }
        Self(bits)
    }

    /// True when any abort bit is set.
    pub const fn any(self) -> bool {
        self.0 != 0
    }

    /// True when `bit` is set (`NONE` is always contained).
    pub const fn contains(self, bit: Self) -> bool {
        self.0 & bit.0 == bit.0
    }
}

/// True while the percept still carries a foe: a legacy percept always does;
/// an attached view must list one. A non-finite separation reads as gone.
fn foe_visible(percept: &Percept) -> bool {
    if !percept.dist_m.is_finite() {
        return false;
    }
    if percept.melee == MeleeView::EMPTY {
        return true;
    }
    percept.melee.is_live()
}

/// Commitment lease: hold the chosen intent for `N` ticks, `N = 0` is off.
impl ActionLease {
    /// A lease holding for `len` ticks; `0` is off.
    pub const fn new(len: u32) -> Self {
        Self {
            len,
            intent: Intent::Advance,
            ticks_left: 0,
            held: None,
            last: None,
        }
    }

    /// Configured hold length in ticks (`0` is off).
    pub const fn hold_ticks(self) -> u32 {
        self.len
    }

    /// Install a hold length; any current hold is dropped.
    pub fn set_len(&mut self, len: u32) {
        self.len = len;
        self.abort();
    }

    /// True when the lease is configured on.
    pub const fn is_on(self) -> bool {
        self.len > 0
    }

    /// Ticks left on the current hold.
    pub const fn ticks_left(self) -> u32 {
        self.ticks_left
    }

    /// The lane held by the most recent [`ActionLease::decide`], if any.
    pub const fn lane(self) -> Option<Intent> {
        self.last
    }

    /// Drop the current hold; the next decide takes the fresh lane.
    pub fn abort(&mut self) {
        self.ticks_left = 0;
        self.held = None;
        self.last = None;
    }

    /// Decide one tick: `fresh`/`fresh_intent` are the core's choice, and
    /// `percept` supplies the abort bits. Aborts on any threat bit, and
    /// reaction lanes (block, dodge) plus the anti-stuck reposition lane are
    /// never leased: they stay responsive. Otherwise the armed input is
    /// re-emitted with a refreshed tick. Returns the lane and input to emit.
    pub fn decide(
        &mut self,
        fresh: BodyInput,
        fresh_intent: Intent,
        percept: &Percept,
    ) -> (Intent, BodyInput) {
        self.last = None;
        if self.len == 0 {
            return (fresh_intent, fresh);
        }
        let threats = LeaseThreats::observe(
            percept.foe_attacking,
            percept.melee.own_hp_permille(),
            foe_visible(percept),
        );
        if threats.any()
            || matches!(
                fresh_intent,
                Intent::Block | Intent::Dodge | Intent::Reposition
            )
        {
            self.abort();
            return (fresh_intent, fresh);
        }
        if self.ticks_left == 0 {
            self.intent = fresh_intent;
            self.ticks_left = self.len;
            self.held = Some(fresh);
            self.last = Some(fresh_intent);
            self.ticks_left -= 1;
            return (fresh_intent, fresh);
        }
        self.ticks_left -= 1;
        let mut input = self.held.unwrap_or(fresh);
        input.tick = percept.tick;
        self.last = Some(self.intent);
        (self.intent, input)
    }
}

impl Default for ActionLease {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::melee::{FoeView, StrikePhase};
    use engine_input::ParticipantId as BodyId;

    fn fighter(id: u8) -> BodyId {
        BodyId::new(id).expect("slot is legal")
    }

    fn calm(tick: u32, dist_m: f32) -> Percept {
        Percept::new(
            [0.0, 0.0, 0.0],
            0.0,
            [dist_m, 0.0, 0.0],
            false,
            dist_m,
            tick,
        )
    }

    fn input(tick: u32, intent: Intent) -> BodyInput {
        let (mx, buttons) = match intent {
            Intent::Attack => (30, 1),
            Intent::Block => (0, 2),
            Intent::Dodge => (0, 4),
            Intent::Advance => (100, 0),
            Intent::Retreat => (-80, 0),
            Intent::Reposition => (60, 0),
        };
        BodyInput {
            tick,
            fighter: fighter(0),
            mx,
            mz: 0,
            facing: 0,
            buttons: engine_input::Buttons(buttons),
        }
    }

    /// One close live view at `dist_cm`, own hp in permille.
    fn view(dist_cm: u16, hp: u16) -> MeleeView {
        MeleeView::nearest(
            hp,
            1_000,
            &[FoeView {
                slot: 1,
                dist_cm,
                hp_permille: 1_000,
                phase: StrikePhase::Idle,
                frames_to_active: 0,
            }],
        )
    }

    #[test]
    fn predicts_attack_after_retreat() {
        let mut table = OpponentTable::new();
        // A close foe that backs off then lunges: the punish read.
        for _ in 0..3 {
            table.observe(RangeBand::Close, Intent::Retreat, Intent::Attack);
        }
        table.observe(RangeBand::Close, Intent::Retreat, Intent::Block);
        assert_eq!(
            table.count(RangeBand::Close, Intent::Retreat, Intent::Attack),
            3
        );
        assert_eq!(
            table.predict(RangeBand::Close, Intent::Retreat),
            Intent::Attack
        );
        // The far band is a separate bucket and learns a different answer.
        for _ in 0..3 {
            table.observe(RangeBand::Far, Intent::Retreat, Intent::Dodge);
        }
        assert_eq!(
            table.predict(RangeBand::Far, Intent::Retreat),
            Intent::Dodge
        );
        assert_eq!(
            table.predict(RangeBand::Close, Intent::Retreat),
            Intent::Attack
        );
        // The last action class also keys the bucket: after a pressing
        // attack the same band reads its own counts.
        for _ in 0..2 {
            table.observe(RangeBand::Close, Intent::Attack, Intent::Retreat);
        }
        assert_eq!(
            table.predict(RangeBand::Close, Intent::Attack),
            Intent::Retreat
        );
    }

    #[test]
    fn opponent_table_repeats_the_same_prediction() {
        let sequence = [
            (RangeBand::Close, Intent::Attack, Intent::Attack),
            (RangeBand::Close, Intent::Attack, Intent::Attack),
            (RangeBand::Close, Intent::Attack, Intent::Block),
            (RangeBand::Far, Intent::Retreat, Intent::Advance),
            (RangeBand::Far, Intent::Retreat, Intent::Advance),
            (RangeBand::Far, Intent::Retreat, Intent::Advance),
        ];
        let mut first = OpponentTable::new();
        let mut second = OpponentTable::new();
        for (band, last, next) in sequence {
            first.observe(band, last, next);
            second.observe(band, last, next);
        }
        for band in [RangeBand::Close, RangeBand::Far] {
            for last in [
                Intent::Attack,
                Intent::Block,
                Intent::Dodge,
                Intent::Advance,
                Intent::Retreat,
                Intent::Reposition,
            ] {
                assert_eq!(first.predict(band, last), second.predict(band, last));
                assert_eq!(first.predict(band, last), first.predict(band, last));
                assert_eq!(first.observed(band, last), second.observed(band, last));
            }
        }
        assert_eq!(first, second);
        // Ties keep the earlier lane, deterministically.
        let mut tied = OpponentTable::new();
        tied.observe(RangeBand::Close, Intent::Retreat, Intent::Dodge);
        tied.observe(RangeBand::Close, Intent::Retreat, Intent::Attack);
        assert_eq!(
            tied.predict(RangeBand::Close, Intent::Retreat),
            Intent::Attack
        );
        let empty = OpponentTable::new();
        assert_eq!(empty.predict(RangeBand::Far, Intent::Block), Intent::Attack);
    }

    #[test]
    fn opponent_table_reads_the_separation_trend() {
        let mut table = OpponentTable::new();
        // Tick one seeds the trend; the foe opens 500 mm, then closes 500.
        assert_eq!(table.observe_foe(false, 2_000), Intent::Attack);
        let after_open = table.observe_foe(false, 2_500);
        assert_eq!(
            table.count(RangeBand::Close, Intent::Block, Intent::Retreat),
            1
        );
        assert_eq!(after_open, table.predict(RangeBand::Close, Intent::Retreat));
        let _ = table.observe_foe(false, 2_000);
        assert_eq!(
            table.count(RangeBand::Close, Intent::Retreat, Intent::Advance),
            1
        );
        // A noticed attack is its own action class.
        let _ = table.observe_foe(true, 2_000);
        assert_eq!(
            table.count(RangeBand::Close, Intent::Advance, Intent::Attack),
            1
        );
        let _ = table.observe_foe(false, 2_000);
        assert_eq!(
            table.count(RangeBand::Close, Intent::Attack, Intent::Block),
            1
        );
        assert!(counter_bias(Intent::Retreat)[3] > 0, "chase a retreat");
    }

    #[test]
    fn lease_holds_lane_across_perception_ticks() {
        let mut lease = ActionLease::new(4);
        assert!(lease.is_on());
        let m = calm(0, 1.0);
        assert_eq!(
            lease.decide(input(0, Intent::Attack), Intent::Attack, &m),
            (Intent::Attack, input(0, Intent::Attack))
        );
        assert_eq!(lease.lane(), Some(Intent::Attack));
        // Three more ticks score Advance (distance drift), but the armed
        // lane keeps winning.
        for tick in 1..4 {
            let out = lease.decide(
                input(tick, Intent::Advance),
                Intent::Advance,
                &calm(tick, 1.0),
            );
            assert_eq!(out.0, Intent::Attack, "tick {tick}");
            assert_eq!(out.1.tick, tick, "the held input refreshes its tick");
            assert_eq!(out.1.facing, 0);
        }
        assert_eq!(lease.ticks_left(), 0);
        // The hold expired: the fresh lane decides and arms the next hold.
        let after = lease.decide(input(4, Intent::Advance), Intent::Advance, &calm(4, 1.0));
        assert_eq!(after.0, Intent::Advance);
        assert_eq!(lease.lane(), Some(Intent::Advance));
        // A reaction lane never leases: it takes the fresh guard and drops
        // the current hold.
        let guard = lease.decide(input(5, Intent::Block), Intent::Block, &calm(5, 1.0));
        assert_eq!(guard.0, Intent::Block);
        assert_eq!(lease.lane(), None);
        assert_eq!(lease.ticks_left(), 0);
        // N = 0 is off: every tick takes the fresh lane.
        let mut off = ActionLease::new(0);
        assert!(!off.is_on());
        assert_eq!(
            off.decide(input(0, Intent::Attack), Intent::Attack, &calm(0, 1.0))
                .0,
            Intent::Attack
        );
        assert_eq!(
            off.decide(input(1, Intent::Dodge), Intent::Dodge, &calm(1, 1.0))
                .0,
            Intent::Dodge
        );
        assert_eq!(off.lane(), None);
        assert_eq!(off, ActionLease::default());
    }

    #[test]
    fn lease_aborts_on_threat() {
        // Incoming attack.
        let mut lease = ActionLease::new(9);
        let _ = lease.decide(input(0, Intent::Attack), Intent::Attack, &calm(0, 1.0));
        let attack = Percept::new([0.0, 0.0, 0.0], 0.0, [1.0, 0.0, 0.0], true, 1.0, 1);
        let out = lease.decide(input(1, Intent::Dodge), Intent::Dodge, &attack);
        assert_eq!(out.0, Intent::Dodge, "the blow aborts the hold");
        assert_eq!(lease.lane(), None);
        assert_eq!(lease.ticks_left(), 0);
        // Low hp.
        let mut wounded = ActionLease::new(9);
        let _ = wounded.decide(input(0, Intent::Attack), Intent::Attack, &calm(0, 1.0));
        let low = calm(1, 1.0).with_melee(view(100, 200));
        assert_eq!(
            wounded
                .decide(input(1, Intent::Block), Intent::Block, &low)
                .0,
            Intent::Block
        );
        assert_eq!(wounded.lane(), None);
        // Foe gone: a non-finite separation aborts.
        let mut gone = ActionLease::new(9);
        let _ = gone.decide(input(0, Intent::Attack), Intent::Attack, &calm(0, 1.0));
        let lost = Percept::new([0.0; 3], 0.0, [0.0; 3], false, f32::NAN, 1);
        assert_eq!(
            gone.decide(input(1, Intent::Advance), Intent::Advance, &lost)
                .0,
            Intent::Advance
        );
        assert_eq!(gone.lane(), None);
        // The bits themselves stay addressable for a caller that has them.
        let bits = LeaseThreats::observe(true, 200, false);
        assert!(bits.any());
        assert!(bits.contains(LeaseThreats::INCOMING_ATTACK));
        assert!(bits.contains(LeaseThreats::LOW_HP));
        assert!(bits.contains(LeaseThreats::FOE_GONE));
        assert!(!LeaseThreats::NONE.any());
    }
}

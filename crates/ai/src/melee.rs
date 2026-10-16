//! melee: quantized close-quarters perception for a brain.
//!
//! Consumer: `arena` quantizes each fighter's surroundings into one
//! [`MeleeView`] per brain per tick under rules v2 and stores it on the
//! [`Percept`](crate::gladiator::Percept); [`DepthBrain`](crate::depth::DepthBrain)
//! reads it for the threat window, the punish window, the stamina and hp
//! pacing and the wait-for-a-token ring. Data owner: the arena owns match
//! state; a view is a plain copy rebuilt every tick. Units: centimetres for
//! separation, permille for hp and stamina, ticks for frame counts.
//! Determinism: integer-only; foes sort by `(dist_cm, slot)`, so equal
//! separations keep the lowest slot. Errors: none, every constructor
//! saturates. Budget: three compares per foe, zero allocation.
//!
//! The empty view ([`MeleeView::EMPTY`]) is what the legacy rules build:
//! it adds no bias, so a brain fed only legacy percepts decides exactly as it
//! did before this module existed.

/// Foes a view keeps, nearest first.
pub const NEAR_FOES: usize = 3;
/// [`NEAR_FOES`] as the view's count type.
const NEAR_FOES_U8: u8 = 3;

/// Separation (mm) a brain without an attack token holds outside of.
pub const WAIT_RING_MM: i32 = 3_000;
/// Width (mm) of the guard band just outside the ring.
pub const WAIT_EDGE_MM: i32 = 500;
/// Own stamina (permille) below which a swing is unaffordable.
pub const SWING_STAMINA_PERMILLE: u16 = 250;
/// Own hp (permille) below which a brain turns defensive.
pub const WOUNDED_PERMILLE: u16 = 300;

/// Where a foe stands in its swing, read from the combat clocks at the edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StrikePhase {
    /// Not swinging.
    #[default]
    Idle,
    /// Startup: the blow has not landed yet.
    Windup,
    /// The impact tick.
    Active,
    /// The blow has landed; the foe is locked in recovery.
    Recovery,
}

impl StrikePhase {
    /// True while the blow can still land (windup or the impact tick).
    pub const fn is_threat(self) -> bool {
        matches!(self, StrikePhase::Windup | StrikePhase::Active)
    }
}

/// One foe as a brain sees it, quantized once at the edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FoeView {
    /// Fighter slot of the foe.
    pub slot: u8,
    /// Planar separation in whole centimetres, saturating at `u16::MAX`.
    pub dist_cm: u16,
    /// Foe hp in permille, `0..=1000`.
    pub hp_permille: u16,
    /// Where the foe stands in its swing.
    pub phase: StrikePhase,
    /// Ticks until the foe's blow lands (zero outside windup), saturating.
    pub frames_to_active: u8,
}

impl FoeView {
    /// Quantize raw foe readings. Negative separations read as zero, hp caps
    /// at 1000, separations round down to a centimetre and frame counts
    /// saturate at `u8::MAX`. Total over all inputs.
    pub fn quantize(
        slot: u8,
        dist_mm: i32,
        hp_permille: u16,
        phase: StrikePhase,
        frames_to_active: u32,
    ) -> Self {
        let cm = u32::try_from(dist_mm.max(0)).unwrap_or(0) / 10;
        Self {
            slot,
            dist_cm: u16::try_from(cm).unwrap_or(u16::MAX),
            hp_permille: hp_permille.min(1_000),
            phase,
            frames_to_active: u8::try_from(frames_to_active).unwrap_or(u8::MAX),
        }
    }

    /// Separation in millimetres (centimetre resolution).
    pub fn dist_mm(&self) -> i32 {
        i32::from(self.dist_cm) * 10
    }

    /// Sort key: nearest first, ties keep the lowest slot.
    fn key(&self) -> (u16, u8) {
        (self.dist_cm, self.slot)
    }
}

/// Quantized close-quarters view: own hp and stamina, the nearest
/// [`NEAR_FOES`] foes in distance order, the foe being fought and whether
/// this brain holds an attack token on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeleeView {
    own_hp_permille: u16,
    own_stamina_permille: u16,
    foes: [FoeView; NEAR_FOES],
    count: u8,
    target: FoeView,
    engage: bool,
}

impl MeleeView {
    /// The legacy view: no foes listed, full vitals, free to engage.
    pub const EMPTY: Self = Self {
        own_hp_permille: 1_000,
        own_stamina_permille: 1_000,
        foes: [FoeView {
            slot: 0,
            dist_cm: 0,
            hp_permille: 0,
            phase: StrikePhase::Idle,
            frames_to_active: 0,
        }; NEAR_FOES],
        count: 0,
        target: FoeView {
            slot: 0,
            dist_cm: 0,
            hp_permille: 0,
            phase: StrikePhase::Idle,
            frames_to_active: 0,
        },
        engage: true,
    };

    /// Build the view from every living foe in any order: keeps the nearest
    /// [`NEAR_FOES`] sorted by `(dist_cm, slot)`, targets the nearest and
    /// leaves the brain free to engage. Own vitals cap at 1000. No foes gives
    /// [`MeleeView::EMPTY`] with the vitals applied.
    pub fn nearest(own_hp_permille: u16, own_stamina_permille: u16, foes: &[FoeView]) -> Self {
        let mut view = Self {
            own_hp_permille: own_hp_permille.min(1_000),
            own_stamina_permille: own_stamina_permille.min(1_000),
            ..Self::EMPTY
        };
        for foe in foes {
            view.insert(*foe);
        }
        view.target = view.foes[0];
        view
    }

    /// Insert one foe into the sorted top list, dropping the farthest.
    fn insert(&mut self, foe: FoeView) {
        let used = usize::from(self.count);
        let mut at = used.min(NEAR_FOES);
        for (index, held) in self.foes[..used].iter().enumerate() {
            if foe.key() < held.key() {
                at = index;
                break;
            }
        }
        if at >= NEAR_FOES {
            return;
        }
        let last = used.min(NEAR_FOES - 1);
        for index in (at..last).rev() {
            self.foes[index + 1] = self.foes[index];
        }
        self.foes[at] = foe;
        self.count = self.count.saturating_add(1).min(NEAR_FOES_U8);
    }

    /// Fight the listed foe `slot` (kept as-is when it is not listed) and
    /// record whether this brain holds the attack token on it.
    #[must_use]
    pub fn targeting(mut self, slot: u8, engage: bool) -> Self {
        if let Some(foe) = self.foes().iter().find(|foe| foe.slot == slot) {
            self.target = *foe;
        }
        self.engage = engage;
        self
    }

    /// The listed foes, nearest first (at most [`NEAR_FOES`]).
    pub fn foes(&self) -> &[FoeView] {
        &self.foes[..usize::from(self.count)]
    }

    /// The foe being fought (the nearest unless retargeted).
    pub const fn target(&self) -> FoeView {
        self.target
    }

    /// True once at least one foe is listed (the view carries information).
    pub const fn is_live(&self) -> bool {
        self.count > 0
    }

    /// Own hp in permille.
    pub const fn own_hp_permille(&self) -> u16 {
        self.own_hp_permille
    }

    /// Own stamina in permille of the combat maximum.
    pub const fn own_stamina_permille(&self) -> u16 {
        self.own_stamina_permille
    }

    /// False while this brain waits for an attack token on its target.
    pub const fn engage(&self) -> bool {
        self.engage
    }

    /// Additive score bias in utility order (attack, block, dodge, advance,
    /// retreat) from what the view shows: the wait ring without a token, the
    /// stamina and hp pacing and the punish window on a recovering target.
    /// Zero for an empty view. Bounded to `+/-1000`.
    pub fn score_bias(&self, dist_mm: i32) -> [i32; 5] {
        if !self.is_live() {
            return [0; 5];
        }
        let mut bias = [0i32; 5];
        if !self.engage {
            add_bias(&mut bias, wait_bias(dist_mm));
            return bias;
        }
        if self.own_stamina_permille < SWING_STAMINA_PERMILLE {
            add_bias(&mut bias, [-400, 100, -200, -50, 150]);
        }
        if self.own_hp_permille < WOUNDED_PERMILLE {
            add_bias(&mut bias, [-80, 80, 40, -40, 60]);
        }
        if self.target.phase == StrikePhase::Recovery {
            add_bias(&mut bias, [250, -150, -400, 0, -100]);
        }
        bias
    }
}

/// Saturating lane-wise add of one bias row into another.
pub(crate) fn add_bias(into: &mut [i32; 5], more: [i32; 5]) {
    for (lane, extra) in into.iter_mut().zip(more) {
        *lane = lane.saturating_add(extra);
    }
}

/// Waiting for a token: no swing, give ground inside the ring, stand guard on
/// its edge and walk in from farther out (the swing is still held). Scores
/// are tuned against the scripted bands so the wait lane wins in each zone.
fn wait_bias(dist_mm: i32) -> [i32; 5] {
    if dist_mm < WAIT_RING_MM {
        [-900, 0, 0, -600, 200]
    } else if dist_mm < WAIT_RING_MM + WAIT_EDGE_MM {
        [-900, 200, 0, -600, 0]
    } else {
        [-900, 0, 0, 0, 0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foe(slot: u8, dist_cm: u16) -> FoeView {
        FoeView {
            slot,
            dist_cm,
            hp_permille: 1_000,
            phase: StrikePhase::Idle,
            frames_to_active: 0,
        }
    }

    #[test]
    fn melee_percept_lists_the_nearest_three_of_four_in_order() {
        let view = MeleeView::nearest(
            900,
            800,
            &[foe(3, 700), foe(1, 300), foe(2, 500), foe(0, 900)],
        );
        let slots: Vec<u8> = view.foes().iter().map(|f| f.slot).collect();
        assert_eq!(slots, vec![1, 2, 3], "nearest three, distance order");
        assert_eq!(view.target().slot, 1, "targets the nearest");
        assert_eq!(view.own_hp_permille(), 900);
        assert_eq!(view.own_stamina_permille(), 800);
        assert!(view.engage());
    }

    #[test]
    fn melee_percept_equal_separations_keep_the_lowest_slot() {
        let view = MeleeView::nearest(
            1_000,
            1_000,
            &[foe(2, 400), foe(1, 400), foe(3, 400), foe(0, 400)],
        );
        let slots: Vec<u8> = view.foes().iter().map(|f| f.slot).collect();
        assert_eq!(slots, vec![0, 1, 2]);
    }

    #[test]
    fn melee_percept_order_does_not_depend_on_input_order() {
        let base = [foe(0, 900), foe(1, 300), foe(2, 500), foe(3, 700)];
        let want = MeleeView::nearest(1_000, 1_000, &base);
        for rotate in 0..base.len() {
            let mut shuffled = base;
            shuffled.rotate_left(rotate);
            assert_eq!(MeleeView::nearest(1_000, 1_000, &shuffled), want);
        }
    }

    #[test]
    fn melee_percept_with_fewer_than_three_foes_lists_what_exists() {
        assert_eq!(MeleeView::nearest(1_000, 1_000, &[]).foes().len(), 0);
        assert!(!MeleeView::nearest(1_000, 1_000, &[]).is_live());
        let one = MeleeView::nearest(1_000, 1_000, &[foe(5, 100)]);
        assert_eq!(one.foes().len(), 1);
        assert!(one.is_live());
        let two = MeleeView::nearest(1_000, 1_000, &[foe(5, 100), foe(4, 50)]);
        assert_eq!(two.foes()[0].slot, 4);
        assert_eq!(two.foes()[1].slot, 5);
    }

    #[test]
    fn melee_percept_quantizes_at_the_edge_without_wrapping() {
        let far = FoeView::quantize(1, i32::MAX, 5_000, StrikePhase::Windup, 9_999);
        assert_eq!(far.dist_cm, u16::MAX, "separation saturates");
        assert_eq!(far.hp_permille, 1_000, "hp caps");
        assert_eq!(far.frames_to_active, u8::MAX, "frames saturate");
        let behind = FoeView::quantize(1, -40, 10, StrikePhase::Idle, 0);
        assert_eq!(behind.dist_cm, 0, "negative separation reads zero");
        assert_eq!(
            FoeView::quantize(1, 1_999, 10, StrikePhase::Idle, 0).dist_cm,
            199
        );
        assert_eq!(
            FoeView::quantize(1, 1_999, 10, StrikePhase::Idle, 0).dist_mm(),
            1_990
        );
    }

    #[test]
    fn melee_percept_targeting_picks_a_listed_foe_and_ignores_others() {
        let view = MeleeView::nearest(1_000, 1_000, &[foe(1, 300), foe(2, 500)]);
        let second = view.targeting(2, false);
        assert_eq!(second.target().slot, 2);
        assert!(!second.engage());
        let unlisted = view.targeting(7, true);
        assert_eq!(
            unlisted.target().slot,
            1,
            "an unlisted slot keeps the target"
        );
    }

    #[test]
    fn melee_percept_empty_view_adds_no_bias() {
        assert_eq!(MeleeView::EMPTY.score_bias(1_000), [0; 5]);
        assert_eq!(MeleeView::EMPTY.foes().len(), 0);
    }

    #[test]
    fn melee_percept_waiting_brain_does_not_swing_or_close_in() {
        let near = MeleeView::nearest(1_000, 1_000, &[foe(1, 150)]).targeting(1, false);
        let bias = near.score_bias(1_500);
        assert!(bias[0] <= -900 && bias[3] <= -600, "no swing, no advance");
        assert!(bias[4] > 0, "inside the ring it gives ground");
        let edge = near.score_bias(WAIT_RING_MM);
        assert!(edge[1] > 0 && edge[4] == 0, "on the ring it stands guard");
        let far = near.score_bias(WAIT_RING_MM + WAIT_EDGE_MM);
        assert!(
            far[0] <= -900 && far[3] == 0,
            "farther out it still walks in"
        );
    }

    #[test]
    fn melee_percept_recovering_foe_is_punished_and_winded_brain_holds_back() {
        let mut recovering = foe(1, 150);
        recovering.phase = StrikePhase::Recovery;
        let punish = MeleeView::nearest(1_000, 1_000, &[recovering]).score_bias(1_500);
        assert!(punish[0] > 0 && punish[2] < 0, "attack up, dodge down");
        let winded = MeleeView::nearest(1_000, 100, &[foe(1, 150)]).score_bias(1_500);
        assert!(winded[0] < 0 && winded[1] > 0, "cannot afford a swing");
    }
}

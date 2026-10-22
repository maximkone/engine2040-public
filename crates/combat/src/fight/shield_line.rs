//! Rules v2, the shield line: weapon rows, their reach and damage, and the
//! boss push with the scutum. A child of `fight`, which steps them.

use crate::sweep::SweepParticipant;

/// The weapon row of a fighter: the bare hand holds no steel.
pub(super) const ROW_BARE: u8 = 0;
// The `type` column of `content/design/weapons.rows` says what a weapon
// does. The fight crate fights with steel only: 1 thrust, 2 cut.
pub(super) const TYPE_NONE: u8 = 0;
pub(super) const TYPE_THRUST: u8 = 1;
pub(super) const TYPE_CUT: u8 = 2;
/// Attack kind of the boss push: a shove with the scutum, not a strike of
/// the steel. It sits above the depth kinds, so the code spaces stay apart.
pub(super) const KIND_PUSH: u8 = 3;

/// The weapon type of each shield-line weapon row: bare hands, pugio,
/// gladius, spatha. The rows mirror `content/design/weapons.rows`.
pub(super) const SHIELD_KINDS: [u8; 4] = [TYPE_NONE, TYPE_THRUST, TYPE_THRUST, TYPE_CUT];
/// The damage of each shield-line weapon row, in permille.
pub(super) const SHIELD_DAMAGE: [u16; 4] = [0, 80, 120, 150];
/// Reach of a thrust under the shield rim, in millimetres.
pub(super) const THRUST_REACH_MM: u32 = 900;
/// Reach of a cut, in millimetres.
pub(super) const CUT_REACH_MM: u32 = 600;
/// Reach of the boss push, in millimetres: a shove with the scutum is short.
pub(super) const PUSH_REACH_MM: u32 = 700;
/// The cut is weak: it does half the row's damage.
pub(super) const CUT_DAMAGE_DEN: u16 = 2;
/// Startup ticks of the boss push, at 60 Hz: a short shove.
pub(super) const PUSH_STARTUP_TICKS: u32 = 6;
/// Recovery ticks after the push lands.
pub(super) const PUSH_RECOVERY_TICKS: u32 = 10;
/// Ticks the pushed fighter cannot act: the shove staggers him.
pub(super) const PUSH_STAGGER_TICKS: u32 = 12;
/// The balance impulse one push feeds, in Q8 units.
pub(super) const PUSH_IMPULSE_Q8: u16 = 256;
/// Freeze ticks of one push landing, at 60 Hz.
pub(super) const PUSH_HITSTOP_TICKS: u32 = 3;

/// The damage of one strike of the weapon row, in permille. The bare hand
/// keeps `bare`, the swinger's own damage (its armed [`crate::heft`] row or
/// the table's), a thrust does the row's damage and a cut is weak: half of it.
pub(super) fn strike_damage(bare: u16, weapon: u8) -> u16 {
    let row = usize::from(weapon);
    if row >= SHIELD_DAMAGE.len() {
        return bare;
    }
    match SHIELD_KINDS[row] {
        TYPE_THRUST => SHIELD_DAMAGE[row],
        TYPE_CUT => SHIELD_DAMAGE[row] / CUT_DAMAGE_DEN,
        _ => bare,
    }
}

/// The reach of one strike in millimetres, or `None` when the arena's own
/// blade sweep decides (the bare hand). A shove with the scutum is short.
pub(super) fn strike_reach(weapon: u8, kind: u8) -> Option<u32> {
    if kind == KIND_PUSH {
        return Some(PUSH_REACH_MM);
    }
    match SHIELD_KINDS.get(usize::from(weapon)) {
        Some(&TYPE_THRUST) => Some(THRUST_REACH_MM),
        Some(&TYPE_CUT) => Some(CUT_REACH_MM),
        _ => None,
    }
}

/// True when the victim stands inside the strike's reach. The reach counts
/// from the body centre to the skin, so a wide body is easier to reach.
pub(super) fn within_reach(
    attacker: &SweepParticipant,
    victim: &SweepParticipant,
    reach: Option<u32>,
) -> bool {
    let Some(limit) = reach else {
        return true;
    };
    let dx = i64::from(victim.pos_mm[0]) - i64::from(attacker.pos_mm[0]);
    let dz = i64::from(victim.pos_mm[2]) - i64::from(attacker.pos_mm[2]);
    let reach = i64::from(limit) + i64::from(victim.hurt_radius_mm);
    dx * dx + dz * dz <= reach * reach
}

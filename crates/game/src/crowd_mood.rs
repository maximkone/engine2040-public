//! crowd_mood: the stands answer the fight as a heat gauge, one beat late.
//!
//! Consumer: [`crowd_instances_mooded`](crate::fight_ring::crowd_stands::crowd_instances_mooded),
//! which lifts and turns each spectator by [`instance_cheer`]. Presentation
//! only: no match state, no hash path, no tick cost beyond a few integers.
//!
//! The stands do not read the fight. They read the director's heat: blows and
//! near misses raise it, a peak floods it, and quiet decays it by
//! [`DECAY_PER_TICK`] a tick down to the floor the director's rung sets
//! ([`engine_ai::crowd_volume`]). The seated crowd shows [`CrowdMood::lagged`],
//! the heat of the previous beat, so a roar reaches the far stand after the
//! blow it answers.
//!
//! Units: heat and the director floor are permille 0..=1000; a cheer is
//! micrometres of rise and milliradians of turn, so nothing here is a float.
//! Determinism: integer only, no clock, no RNG, no heap; the same event tape
//! and floor read the same heat, and the same heat the same seat digest.

use engine_ai::crowd_volume;
use engine_core::Fnv64;
use engine_render::draw::crowd_seats::crowd_total;

/// Full heat, permille.
pub const HEAT_MAX: u32 = 1000;
/// Heat one landed blow adds.
pub const BLOW_GAIN: u32 = 180;
/// Heat one near miss adds.
pub const NEAR_MISS_GAIN: u32 = 60;
/// Heat one director peak adds.
pub const PEAK_GAIN: u32 = 300;
/// Heat a quiet tick takes away, down to the director's floor.
pub const DECAY_PER_TICK: u32 = 8;
/// A seated spectator's largest rise, micrometres (60 mm).
const CHEER_RISE_UM: u32 = 60_000;
/// A seated spectator's largest turn, milliradians (80 mrad).
const CHEER_YAW_MRAD: i32 = 80;

/// What the stands heard this beat.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CrowdEvent {
    /// Nothing landed: the heat decays toward its floor.
    Nothing,
    /// A landed blow.
    Blow,
    /// A blow that missed close enough to feel.
    NearMiss,
    /// The director reached its peak.
    Peak,
}

impl CrowdEvent {
    /// Heat this event adds before the cap.
    pub const fn gain(self) -> u32 {
        match self {
            Self::Nothing => 0,
            Self::Blow => BLOW_GAIN,
            Self::NearMiss => NEAR_MISS_GAIN,
            Self::Peak => PEAK_GAIN,
        }
    }
}

/// One bout's crowd: the live heat and the heat the stands show.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct CrowdMood {
    /// Heat now, permille.
    pub heat_permille: u32,
    /// The heat the stands showed last beat, permille.
    pub lagged_permille: u32,
}

impl CrowdMood {
    /// A silent arena: no heat, no floor heard yet.
    pub const fn new() -> Self {
        Self {
            heat_permille: 0,
            lagged_permille: 0,
        }
    }

    /// The stands' heat, one beat behind the live heat.
    pub const fn lagged(&self) -> u32 {
        self.lagged_permille
    }

    /// Advance one beat: the stands take last beat's heat, the live heat moves
    /// by `event` and decays toward `floor_permille`.
    pub fn step(&mut self, event: CrowdEvent, floor_permille: u32) {
        self.lagged_permille = self.heat_permille;
        self.heat_permille = heat_step(self.heat_permille, event, floor_permille);
    }

    /// The floor one director rung sets, permille: the rung's crowd volume.
    pub fn floor_for(rung: u8) -> u32 {
        crowd_volume(rung).map_or(0, u32::from)
    }
}

/// One beat of heat: the event's gain capped at [`HEAT_MAX`], never below the
/// director's floor; a quiet beat then decays by [`DECAY_PER_TICK`] but never
/// through the floor.
pub fn heat_step(heat_permille: u32, event: CrowdEvent, floor_permille: u32) -> u32 {
    let floor = floor_permille.min(HEAT_MAX);
    let raised = heat_permille
        .max(floor)
        .saturating_add(event.gain())
        .min(HEAT_MAX);
    match event {
        CrowdEvent::Nothing => raised.saturating_sub(DECAY_PER_TICK).max(floor),
        _ => raised.max(floor),
    }
}

/// One spectator's answer to the heat: a rise and a small turn, integers.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Cheer {
    /// Rise above the seat, micrometres.
    pub rise_um: u32,
    /// Turn off the seat's facing, milliradians, either way.
    pub yaw_millirad: i32,
}

/// The cheer of one seat at a heat: cold stands sit still, a hot stand rises
/// and rocks. Seats differ by a seeded phase so the crowd is not one body.
pub fn instance_cheer(seat: usize, heat_permille: u32) -> Cheer {
    let heat = heat_permille.min(HEAT_MAX);
    if heat == 0 {
        return Cheer::default();
    }
    let mut hash = Fnv64::new();
    hash.write_u32(u32::try_from(seat).unwrap_or(0));
    let bytes = hash.finish().to_le_bytes();
    // Phase in permille of the heat: two seats never move as one.
    let phase = u32::from(u16::from_le_bytes([bytes[0], bytes[1]])) % 1000;
    let scale = heat.saturating_mul(phase.max(250)) / 1000;
    let rise_um = u64::from(CHEER_RISE_UM) * u64::from(scale) / u64::from(HEAT_MAX);
    let sign = if bytes[2] & 1 == 0 { 1 } else { -1 };
    let turn = u64::from(80u32) * u64::from(scale) / u64::from(HEAT_MAX);
    Cheer {
        rise_um: u32::try_from(rise_um).unwrap_or(CHEER_RISE_UM),
        yaw_millirad: sign * i32::try_from(turn).unwrap_or(CHEER_YAW_MRAD),
    }
}

/// Digest of every seat's cheer pose at this heat: the stands' answer as one
/// word, so a roar and a hush are two different crowds.
pub fn stands_digest(heat_permille: u32) -> u64 {
    let mut hash = Fnv64::new();
    for seat in 0..crowd_total() {
        let cheer = instance_cheer(seat, heat_permille);
        hash.write_u32(u32::try_from(seat).unwrap_or(0));
        hash.write_u32(cheer.rise_um);
        hash.write_u32(cheer.yaw_millirad.unsigned_abs());
    }
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heat_rises_on_a_blow_and_decays_to_the_director_floor() {
        let floor = CrowdMood::floor_for(1);
        assert!(floor > 0, "a director rung sets a real floor: {floor}");
        let mut mood = CrowdMood::new();
        assert_eq!(mood.heat_permille, 0, "a bout opens silent");

        mood.step(CrowdEvent::Blow, floor);
        assert_eq!(mood.heat_permille, BLOW_GAIN, "a blow lifts the heat");
        mood.step(CrowdEvent::NearMiss, floor);
        assert_eq!(mood.heat_permille, BLOW_GAIN + NEAR_MISS_GAIN);
        mood.step(CrowdEvent::Peak, floor);
        assert_eq!(mood.heat_permille, HEAT_MAX, "a peak floods the cap");

        // Quiet beats decay, never through the director's floor.
        let mut quiet = 0;
        while quiet < 400 {
            mood.step(CrowdEvent::Nothing, floor);
            quiet += 1;
        }
        assert_eq!(mood.heat_permille, floor, "the decay stops at the floor");
        assert!(mood.heat_permille >= floor, "the floor is never crossed");
        // A floor above the live heat lifts it, never pulls it down.
        assert_eq!(heat_step(100, CrowdEvent::Nothing, 400), 400);
        // One quiet beat from the floor stays at the floor.
        assert_eq!(heat_step(floor, CrowdEvent::Nothing, floor), floor);
    }

    #[test]
    fn stands_lag_one_beat_behind_the_heat() {
        let floor = CrowdMood::floor_for(1);
        let mut mood = CrowdMood::new();
        assert_eq!(mood.lagged(), 0);
        mood.step(CrowdEvent::Blow, floor);
        assert_eq!(mood.heat_permille, BLOW_GAIN);
        assert_eq!(mood.lagged(), 0, "the stands have not heard it yet");
        mood.step(CrowdEvent::Nothing, floor);
        assert_eq!(
            mood.lagged(),
            BLOW_GAIN,
            "the far stand roars one beat after the blow"
        );
        mood.step(CrowdEvent::Nothing, floor);
        assert_eq!(mood.lagged(), BLOW_GAIN - DECAY_PER_TICK);

        // The stands' pose really differs between a hush and a roar.
        assert_ne!(
            stands_digest(100),
            stands_digest(900),
            "a hush and a roar are two different crowds"
        );
        // Same heat twice: the same crowd bytes twice.
        assert_eq!(stands_digest(900), stands_digest(900));
        // Cold stands sit still.
        assert_eq!(instance_cheer(7, 0), Cheer::default());
        // A hot crowd moves every seat, and never above the caps.
        for seat in 0..crowd_total() {
            let cheer = instance_cheer(seat, HEAT_MAX);
            assert!(cheer.rise_um <= CHEER_RISE_UM, "seat {seat} rises too far");
            assert!(
                cheer.yaw_millirad.unsigned_abs() <= u32::try_from(CHEER_YAW_MRAD).unwrap_or(80),
                "seat {seat} turns too far"
            );
        }
    }
}

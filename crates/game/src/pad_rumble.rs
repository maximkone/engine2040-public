//! pad_rumble: the pad rumbles on a hit. A pure mapping from the eye trauma a tick's cues gave
//! the player to a rumble strength and length, plus one thin gilrs call that plays it on each
//! connected pad with force feedback. No pad, or a pad without force feedback, is a no-op.
//! Owner: engine-game. Units: strength in permille (0 silent, 1000 full), length in ms.
//! Presentation only, never hashed.
use crate::camera::TRAUMA_SCALE;
use crate::fps_view::{EyeTrauma, EYE_TRAUMA_DECAY_PER_TICK};
use engine_audio::FightCue;

#[cfg(feature = "app")]
use crate::{TickOutcome, HUMAN_SLOT};

/// Simulation ticks per second: the fixed tick the eye's decay runs on.
const TICKS_PER_SECOND: u32 = 60;

/// Whether the pad rumbles on a hit. The settings file has no rumble field yet, so the window
/// reads this switch; set it to false to silence the pad.
pub const PAD_RUMBLE_ON: bool = true;

/// One rumble: strength in permille (0 is silent) and how long it plays, in ms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rumble {
    pub strength_permille: u32,
    pub duration_ms: u32,
}

/// The rumble for an eye trauma level. The strength is the trauma itself: a hit taken (180)
/// and a knockdown (350) rumble at those strengths. The length is the ticks the eye's own decay
/// takes to fade that trauma to nothing, so the pad stops as the shake does (150 ms and 300 ms
/// for those two). Zero trauma, or `on == false`, gives silence.
#[must_use]
pub fn rumble_for_trauma(trauma_permille: u32, on: bool) -> Rumble {
    let strength = trauma_permille.min(TRAUMA_SCALE);
    if !on || strength == 0 {
        return Rumble::default();
    }
    let fade_ticks = strength.div_ceil(EYE_TRAUMA_DECAY_PER_TICK);
    Rumble {
        strength_permille: strength,
        duration_ms: fade_ticks * 1000 / TICKS_PER_SECOND,
    }
}

/// The rumble the cues of one tick give the fighter in `slot`, through the eye's own trauma
/// rule: a blow taken counts in full, a blow landed counts a third, a blow between rivals counts
/// nothing. A fresh eye, so only this tick's cues count.
#[must_use]
pub fn rumble_for_cues(cues: &[FightCue], slot: u8, on: bool) -> Rumble {
    let mut eye = EyeTrauma::default();
    rumble_for_trauma(eye.feed(0, cues, slot), on)
}

#[cfg(feature = "app")]
thread_local! {
    /// The effect now playing. Holding it keeps it alive; the next rumble replaces it.
    static PLAYING: std::cell::RefCell<Option<gilrs::ff::Effect>> =
        const { std::cell::RefCell::new(None) };
}

/// Plays `rumble` on every connected pad that has force feedback. Silence, no backend, no pad,
/// a pad without force feedback and a refused effect are all quiet no-ops, never an error.
#[cfg(feature = "app")]
pub fn play_on_pads(backend: Option<&mut gilrs::Gilrs>, rumble: Rumble) {
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Envelope, Replay, Ticks};
    if rumble.strength_permille == 0 {
        return;
    }
    let Some(gilrs) = backend else {
        return;
    };
    let pads: Vec<gilrs::GamepadId> = gilrs
        .gamepads()
        .filter(|(_, pad)| pad.is_ff_supported())
        .map(|(id, _)| id)
        .collect();
    if pads.is_empty() {
        return;
    }
    // gilrs takes a u16 motor magnitude; full trauma is the full motor.
    let magnitude = u16::try_from(rumble.strength_permille * u32::from(u16::MAX) / TRAUMA_SCALE)
        .unwrap_or(u16::MAX);
    let effect = EffectBuilder::new()
        .add_effect(BaseEffect {
            kind: BaseEffectType::Strong { magnitude },
            scheduling: Replay {
                after: Ticks::from_ms(0),
                play_for: Ticks::from_ms(rumble.duration_ms),
                with_delay: Ticks::from_ms(0),
            },
            envelope: Envelope::default(),
        })
        .gamepads(&pads)
        .finish(gilrs);
    let Ok(effect) = effect else {
        return;
    };
    let _ = effect.play();
    PLAYING.with(|slot| *slot.borrow_mut() = Some(effect));
}

/// The windowed tick's hook: rumble for the cues the human fighter took or dealt this tick, then
/// hand the outcome back unchanged so the loop reads it as before.
#[cfg(feature = "app")]
pub fn pulse(outcome: TickOutcome, backend: Option<&mut gilrs::Gilrs>) -> TickOutcome {
    let rumble = rumble_for_cues(&outcome.cues, HUMAN_SLOT, PAD_RUMBLE_ON);
    play_on_pads(backend, rumble);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_feed::{TRAUMA_HIT, TRAUMA_KNOCKDOWN};

    #[test]
    fn pad_rumble_hit_taken_and_knockdown_map_to_two_strengths() {
        let hit = [FightCue::Hit {
            attacker: 1,
            victim: 0,
            damage: 200,
        }];
        let down = [FightCue::Knockdown {
            attacker: 1,
            victim: 0,
        }];
        let hit_r = rumble_for_cues(&hit, 0, true);
        let down_r = rumble_for_cues(&down, 0, true);
        assert_eq!(hit_r.strength_permille, TRAUMA_HIT);
        assert_eq!(down_r.strength_permille, TRAUMA_KNOCKDOWN);
        assert!(
            down_r.strength_permille > hit_r.strength_permille,
            "the knockdown rumbles harder than the hit"
        );
        assert_eq!(hit_r.duration_ms, 150, "a hit fades in nine ticks");
        assert!(down_r.duration_ms > hit_r.duration_ms);
    }

    #[test]
    fn pad_rumble_is_silent_when_off_rivals_or_calm() {
        let down = [FightCue::Knockdown {
            attacker: 1,
            victim: 0,
        }];
        assert_eq!(rumble_for_cues(&down, 0, false), Rumble::default());
        let rivals = [FightCue::Hit {
            attacker: 3,
            victim: 4,
            damage: 200,
        }];
        assert_eq!(rumble_for_cues(&rivals, 0, true), Rumble::default());
        assert_eq!(rumble_for_cues(&[], 0, true), Rumble::default());
        assert_eq!(rumble_for_trauma(0, true), Rumble::default());
    }

    #[cfg(feature = "app")]
    #[test]
    fn pad_rumble_without_a_pad_is_a_quiet_no_op() {
        play_on_pads(None, rumble_for_trauma(TRAUMA_KNOCKDOWN, true));
        let out = pulse(TickOutcome::default(), None);
        assert!(out.cues.is_empty());
    }
}

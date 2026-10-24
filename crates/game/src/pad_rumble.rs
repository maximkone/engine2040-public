//! pad_rumble: the pad rumbles on a hit. A pure mapping from the eye trauma a tick's cues gave
//! the player to a rumble strength and length, plus one thin gilrs call that plays it on each
//! connected pad with force feedback. No pad, or a pad without force feedback, is a no-op.
//! Owner: engine-game. Units: strength in permille (0 silent, 1000 full), length in ms.
//! Presentation only, never hashed.
use crate::camera::TRAUMA_SCALE;
use crate::fps_view::{EyeTrauma, EYE_TRAUMA_DECAY_PER_TICK};
use engine_audio::FightCue;

#[cfg(feature = "app")]
use crate::TickOutcome;

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

/// Damage in permille at which a landed hit reads heavy: a fifth of the health bar, so a
/// jab and a committed arc feel different on the pad, not just in the hitstop.
pub const HEAVY_DAMAGE_PERMILLE: u16 = 200;

/// Milliseconds focus loss has to cut both motors: the stop deadline the window promises when
/// the player alt-tabs away, so no motor keeps buzzing behind a lost window.
pub const MOTORS_OFF_MS: u32 = 10;

/// One cue's rumble on the pad's four motors, in the raw `u16` magnitude gilrs takes
/// (`0` silent, `u16::MAX` full). `low` and `high` are the two body motors, `trigger_*` the
/// impulse triggers. `ms` is how long the row plays. Presentation only, never hashed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RumbleRow {
    /// Low-frequency (left) body motor.
    pub low: u16,
    /// High-frequency (right) body motor.
    pub high: u16,
    /// Left impulse trigger.
    pub trigger_l: u16,
    /// Right impulse trigger.
    pub trigger_r: u16,
    /// Play length in milliseconds; `0` on a silent row.
    pub ms: u32,
}

/// The silent row: every motor off, nothing to play.
const SILENT_ROW: RumbleRow = RumbleRow {
    low: 0,
    high: 0,
    trigger_l: 0,
    trigger_r: 0,
    ms: 0,
};

/// Cue rows in [`RUMBLE_TABLE`]: a landed hit splits light/heavy on
/// [`HEAVY_DAMAGE_PERMILLE`], then block, parry, dodge, stamina out and knockdown.
pub const LIGHT_HIT_ROW: usize = 0;
/// Heavy landed hit row.
pub const HEAVY_HIT_ROW: usize = 1;
/// Held block row.
pub const BLOCKED_ROW: usize = 2;
/// Parry row.
pub const PARRIED_ROW: usize = 3;
/// Dodge row.
pub const DODGED_ROW: usize = 4;
/// Stamina-out row: the only silent row, because a failed input is not a hit.
pub const STAMINA_OUT_ROW: usize = 5;
/// Knockdown row, the longest and heaviest.
pub const KNOCKDOWN_ROW: usize = 6;

/// One row per fight cue, in [`LIGHT_HIT_ROW`]..[`KNOCKDOWN_ROW`] order. Magnitudes are a
/// fraction of the motor, not of the trauma: a block is a short blip, a parry a sharp answer on
/// the left trigger, a dodge the lightest tick, a knockdown the only row that fills the body
/// motors and runs past 200 ms. Every row but stamina out moves a motor and stays at or under
/// 300 ms, so no cue leaves the pad running.
pub const RUMBLE_TABLE: [RumbleRow; 7] = [
    RumbleRow {
        low: 18_000,
        high: 24_000,
        trigger_l: 0,
        trigger_r: 0,
        ms: 90,
    },
    RumbleRow {
        low: 30_000,
        high: 40_000,
        trigger_l: 0,
        trigger_r: 12_000,
        ms: 180,
    },
    RumbleRow {
        low: 12_000,
        high: 8_000,
        trigger_l: 6_000,
        trigger_r: 0,
        ms: 70,
    },
    RumbleRow {
        low: 20_000,
        high: 16_000,
        trigger_l: 14_000,
        trigger_r: 0,
        ms: 110,
    },
    RumbleRow {
        low: 8_000,
        high: 6_000,
        trigger_l: 0,
        trigger_r: 0,
        ms: 60,
    },
    SILENT_ROW,
    RumbleRow {
        low: 40_000,
        high: 50_000,
        trigger_l: 8_000,
        trigger_r: 8_000,
        ms: 260,
    },
];

/// The row for one cue. `on == false` is silence for every cue. A landed hit picks the light or
/// heavy row by its damage; a failed input (stamina out) is the silent row.
#[must_use]
pub fn rumble_for_cue(cue: FightCue, on: bool) -> RumbleRow {
    if !on {
        return SILENT_ROW;
    }
    match cue {
        FightCue::Hit { damage, .. } => {
            if damage >= HEAVY_DAMAGE_PERMILLE {
                RUMBLE_TABLE[HEAVY_HIT_ROW]
            } else {
                RUMBLE_TABLE[LIGHT_HIT_ROW]
            }
        }
        FightCue::Blocked { .. } => RUMBLE_TABLE[BLOCKED_ROW],
        FightCue::Parried { .. } => RUMBLE_TABLE[PARRIED_ROW],
        FightCue::Dodged { .. } => RUMBLE_TABLE[DODGED_ROW],
        FightCue::StaminaOut { .. } => RUMBLE_TABLE[STAMINA_OUT_ROW],
        FightCue::Knockdown { .. } => RUMBLE_TABLE[KNOCKDOWN_ROW],
    }
}

/// The row the window plays when focus leaves: every motor off, due within [`MOTORS_OFF_MS`].
/// The winit `Focused(false)` hook that calls this is the window loop's, not this module's.
#[must_use]
pub fn focus_lost() -> RumbleRow {
    RumbleRow {
        ms: MOTORS_OFF_MS,
        ..SILENT_ROW
    }
}

/// The one row a tick's cues play: the strongest value per motor and the longest length, so two
/// cues in a tick read as one buzz instead of a queue of overlapping effects. `on == false` or no
/// cue is silence.
#[must_use]
pub fn merged_cue_row(cues: &[FightCue], on: bool) -> RumbleRow {
    let mut merged = SILENT_ROW;
    if !on {
        return merged;
    }
    for cue in cues {
        let row = rumble_for_cue(*cue, true);
        merged.low = merged.low.max(row.low);
        merged.high = merged.high.max(row.high);
        merged.trigger_l = merged.trigger_l.max(row.trigger_l);
        merged.trigger_r = merged.trigger_r.max(row.trigger_r);
        merged.ms = merged.ms.max(row.ms);
    }
    merged
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

/// Play one cue row on every connected pad that has force feedback: the row's `low` motor drives
/// the strong actuator and `high` the weak one, for the row's own `ms`. Trigger magnitudes have no
/// gilrs channel in 0.11, so they stay data in the row (a later backend exposes them). Silence, no
/// backend, no pad, a pad without force feedback and a refused effect are quiet no-ops.
#[cfg(feature = "app")]
pub fn play_row_on_pads(backend: Option<&mut gilrs::Gilrs>, row: RumbleRow) {
    use gilrs::ff::{BaseEffect, BaseEffectType, EffectBuilder, Envelope, Replay, Ticks};
    if row.low == 0 && row.high == 0 {
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
    let scheduling = Replay {
        after: Ticks::from_ms(0),
        play_for: Ticks::from_ms(row.ms),
        with_delay: Ticks::from_ms(0),
    };
    let mut builder = EffectBuilder::new();
    for (magnitude, kind) in [
        (row.low, BaseEffectType::Strong { magnitude: row.low }),
        (
            row.high,
            BaseEffectType::Weak {
                magnitude: row.high,
            },
        ),
    ] {
        if magnitude > 0 {
            builder.add_effect(BaseEffect {
                kind,
                scheduling,
                envelope: Envelope::default(),
            });
        }
    }
    let effect = builder.gamepads(&pads).finish(gilrs);
    let Ok(effect) = effect else {
        return;
    };
    let _ = effect.play();
    PLAYING.with(|slot| *slot.borrow_mut() = Some(effect));
}

/// Cut every motor now: the focus-loss path (`WindowEvent::Focused(false)`) and the disconnect
/// path call this, so no effect keeps playing behind a lost window or a pulled pad. No effect
/// playing is a quiet no-op.
#[cfg(feature = "app")]
pub fn stop_effects() {
    PLAYING.with(|slot| {
        if let Some(effect) = slot.borrow_mut().take() {
            let _ = effect.stop();
        }
    });
}

/// The windowed tick's hook: one merged row for the cues the human fighter took or dealt this
/// tick, played on the pad's two body motors, then the outcome back unchanged so the loop reads it
/// as before.
#[cfg(feature = "app")]
pub fn pulse(outcome: TickOutcome, backend: Option<&mut gilrs::Gilrs>) -> TickOutcome {
    let row = merged_cue_row(&outcome.cues, PAD_RUMBLE_ON);
    play_row_on_pads(backend, row);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera_feed::{TRAUMA_HIT, TRAUMA_KNOCKDOWN};

    #[test]
    fn merged_cue_row_keeps_the_strongest_motor_and_longest_length() {
        let cues = [
            FightCue::Hit {
                attacker: 1,
                victim: 0,
                damage: 1,
            },
            FightCue::Knockdown {
                attacker: 1,
                victim: 0,
            },
            FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 1,
            },
        ];
        let merged = merged_cue_row(&cues, true);
        let light = RUMBLE_TABLE[LIGHT_HIT_ROW];
        let down = RUMBLE_TABLE[KNOCKDOWN_ROW];
        assert_eq!(merged.low, light.low.max(down.low));
        assert_eq!(merged.high, light.high.max(down.high));
        assert_eq!(merged.ms, down.ms, "the longest row wins the length");
        assert_eq!(merged_cue_row(&cues, false), SILENT_ROW);
        assert_eq!(merged_cue_row(&[], true), SILENT_ROW);
        assert_eq!(merged_cue_row(&[], false), SILENT_ROW);
    }

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

    /// Every cue at its own damage so each row gets read: a light hit, a heavy hit, then one
    /// each of the rest.
    fn one_cue_per_row() -> [FightCue; 7] {
        [
            FightCue::Hit {
                attacker: 1,
                victim: 0,
                damage: HEAVY_DAMAGE_PERMILLE - 1,
            },
            FightCue::Hit {
                attacker: 1,
                victim: 0,
                damage: HEAVY_DAMAGE_PERMILLE,
            },
            FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 20,
            },
            FightCue::Parried {
                attacker: 1,
                victim: 0,
            },
            FightCue::Dodged {
                attacker: 1,
                victim: 0,
            },
            FightCue::StaminaOut { fighter: 0 },
            FightCue::Knockdown {
                attacker: 1,
                victim: 0,
            },
        ]
    }

    #[test]
    fn rumble_merge_maps_seven_cues_and_cuts_motors_on_focus_loss() {
        let cues = one_cue_per_row();
        assert_eq!(cues.len(), RUMBLE_TABLE.len(), "one row per cue");
        let rows: Vec<RumbleRow> = cues.iter().map(|cue| rumble_for_cue(*cue, true)).collect();
        // The two landed hits split on the damage threshold; the other five map straight.
        assert_eq!(rows[LIGHT_HIT_ROW], RUMBLE_TABLE[LIGHT_HIT_ROW]);
        assert_eq!(rows[HEAVY_HIT_ROW], RUMBLE_TABLE[HEAVY_HIT_ROW]);
        assert_ne!(
            rows[LIGHT_HIT_ROW], rows[HEAVY_HIT_ROW],
            "a heavy hit is not a jab"
        );
        for row in [BLOCKED_ROW, PARRIED_ROW, DODGED_ROW, KNOCKDOWN_ROW] {
            assert_eq!(rows[row], RUMBLE_TABLE[row], "row {row} maps its own cue");
        }
        // Exactly one silent row (stamina out): a failed input never buzzes.
        let silent = |row: &RumbleRow| {
            row.low == 0 && row.high == 0 && row.trigger_l == 0 && row.trigger_r == 0
        };
        assert_eq!(
            rows.iter().filter(|row| silent(row)).count(),
            1,
            "one silent row"
        );
        assert!(silent(&rows[STAMINA_OUT_ROW]));
        assert_eq!(rows[STAMINA_OUT_ROW].ms, 0, "a silent row does not play");
        // Every other row moves a motor and stops within the feel budget.
        for (index, row) in rows.iter().enumerate() {
            if index == STAMINA_OUT_ROW {
                continue;
            }
            assert!(
                row.low > 0 || row.high > 0,
                "row {index} must move a body motor"
            );
            assert!(
                row.ms > 0 && row.ms <= 300,
                "row {index} lasts {} ms",
                row.ms
            );
        }
        // The switch off is silence for every cue, and focus loss cuts all four motors inside
        // the deadline the window promises.
        for cue in cues {
            assert_eq!(rumble_for_cue(cue, false), SILENT_ROW, "{cue:?}");
        }
        let lost = focus_lost();
        assert!(silent(&lost), "focus loss leaves no motor on");
        assert!(
            lost.ms <= MOTORS_OFF_MS,
            "motors must be off within {MOTORS_OFF_MS} ms, row says {}",
            lost.ms
        );
        assert_eq!(MOTORS_OFF_MS, 10, "the stop deadline is 10 ms");
    }
}

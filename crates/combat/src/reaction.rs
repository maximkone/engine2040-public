//! reaction: visible hit reactions beside the fight step.
//!
//! Owner: engine-combat. Consumer: presentation — the tick folds one
//! [`Reaction`] per tick with [`reaction_for_tick`] (or steps and folds with
//! [`reaction_step_into`]), then draws freeze, flash and damage number on its
//! own clock. Units: ticks at 60 Hz, damage numbers in permille. Determinism:
//! integer math only; same event plus same rows always give the same reaction.
//! Errors: none — every event maps, quiet ones to [`Reaction::ZERO`]. Budget:
//! const construction per event, O(events) fold over a caller slice, no heap.
//! The contact-frame layers of the feel module stay frozen: this module reads
//! the same fight events and rows through its own visible triple, and the
//! shared [`contact_sim_digest`] proves the
//! sim never moves under presentation.

use crate::feel::{capture_input, contact_sim_digest, FeelPreset};
use crate::fight::{step_with_tables_into, FightEvent, FightState, Tables};
use engine_input::fighter::{ATTACK, BLOCK};
use engine_input::ParticipantInput;

/// Cap for the visible freeze channel, in ticks at 60 Hz.
pub const MAX_REACTION_FREEZE_TICKS: u32 = 60;
/// Cap for the visible flash channel, in frames.
pub const MAX_REACTION_FLASH_TICKS: u32 = 30;
/// Cap for the damage number channel, in permille.
pub const MAX_REACTION_NUMBER_PERMILLE: u16 = 1000;

/// Visible reaction for one contact frame: the three channels the captures
/// must show together.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Reaction {
    /// Ticks presentation holds the freeze, mirroring the sim row.
    pub freeze_ticks: u32,
    /// Frames presentation flashes the struck fighter.
    pub flash_ticks: u32,
    /// Damage number presentation prints (`0` prints nothing).
    pub damage_number: u16,
}

impl Reaction {
    /// Quiet frame: no freeze, no flash, no number.
    pub const ZERO: Self = Self {
        freeze_ticks: 0,
        flash_ticks: 0,
        damage_number: 0,
    };

    /// True while every channel is quiet.
    pub const fn is_quiet(self) -> bool {
        self.freeze_ticks == 0 && self.flash_ticks == 0 && self.damage_number == 0
    }

    /// Merge two frames: the strongest freeze, flash and number win, so a
    /// hit plus its knockdown still prints the hit damage.
    pub const fn merge(self, other: Self) -> Self {
        Self {
            freeze_ticks: if self.freeze_ticks > other.freeze_ticks {
                self.freeze_ticks
            } else {
                other.freeze_ticks
            },
            flash_ticks: if self.flash_ticks > other.flash_ticks {
                self.flash_ticks
            } else {
                other.flash_ticks
            },
            damage_number: if self.damage_number > other.damage_number {
                self.damage_number
            } else {
                other.damage_number
            },
        }
    }
}

/// Clamp a sim freeze row to the presentation cap; unvalidated rows stay safe.
const fn clamped_freeze(hitstop_ticks: u32) -> u32 {
    if hitstop_ticks > MAX_REACTION_FREEZE_TICKS {
        MAX_REACTION_FREEZE_TICKS
    } else {
        hitstop_ticks
    }
}

/// Clamp a juice flash row to the presentation cap; unvalidated rows stay safe.
const fn clamped_flash(flash_ticks: u32) -> u32 {
    if flash_ticks > MAX_REACTION_FLASH_TICKS {
        MAX_REACTION_FLASH_TICKS
    } else {
        flash_ticks
    }
}

/// Clamp a damage reading to the number cap; unvalidated rows stay safe.
const fn clamped_number(damage_permille: u16) -> u16 {
    if damage_permille > MAX_REACTION_NUMBER_PERMILLE {
        MAX_REACTION_NUMBER_PERMILLE
    } else {
        damage_permille
    }
}

/// Visible reaction for one resolved event under a preset: hits carry the sim
/// freeze plus the juice flash and print their damage, blocked chips skip the
/// freeze the sim never takes but still print the chip, parries hold the
/// freeze with a flash and print nothing, knockdowns hold the freeze with a
/// flash and print nothing (the precipitating hit already printed), dodges
/// stay quiet, and stamina-outs flash the exhausted fighter with no freeze
/// and no number (the sim never freezes on a refused attack and no damage
/// changed hands). Integer only; presentation-side, never read
/// back by the sim.
///
/// Pass the resolved rows the tick stepped (`preset.tables(base)`), so the
/// reaction can never drift from the fight it describes.
pub const fn reaction_for_event(event: FightEvent, preset: FeelPreset, tables: Tables) -> Reaction {
    match event {
        FightEvent::Hit { damage, .. } => Reaction {
            freeze_ticks: clamped_freeze(tables.hitstop_ticks),
            flash_ticks: clamped_flash(preset.juicing.flash_ticks),
            damage_number: clamped_number(damage),
        },
        FightEvent::Blocked { chip, .. } => Reaction {
            freeze_ticks: 0,
            flash_ticks: clamped_flash(preset.juicing.flash_ticks),
            damage_number: clamped_number(chip),
        },
        FightEvent::Parried { .. } => Reaction {
            freeze_ticks: clamped_freeze(tables.hitstop_ticks),
            flash_ticks: clamped_flash(preset.juicing.flash_ticks),
            damage_number: 0,
        },
        FightEvent::Knockdown { .. } => Reaction {
            freeze_ticks: clamped_freeze(tables.hitstop_ticks),
            flash_ticks: clamped_flash(preset.juicing.flash_ticks),
            damage_number: 0,
        },
        FightEvent::Dodged { .. } => Reaction::ZERO,
        FightEvent::StaminaOut { .. } => Reaction {
            freeze_ticks: 0,
            flash_ticks: clamped_flash(preset.juicing.flash_ticks),
            damage_number: 0,
        },
    }
}

/// Fold one tick's events into a single visible reaction.
///
/// Consumer wiring: the tick steps into its reused event buffer, then folds
/// per-event reactions into one summary with [`Reaction::merge`]. Slices in,
/// value out — no heap, no floats, no panics on user data.
pub fn reaction_for_tick(events: &[FightEvent], preset: FeelPreset, tables: Tables) -> Reaction {
    let mut reaction = Reaction::ZERO;
    let mut i = 0;
    while i < events.len() {
        reaction = reaction.merge(reaction_for_event(events[i], preset, tables));
        i += 1;
    }
    reaction
}

/// Tuned step plus visible reaction, into a reused event buffer.
///
/// Consumer wiring: resolves [`FeelPreset::tables`] over `base`, steps the
/// shared fight core (same events and end state as stepping those tables
/// directly), then folds the tick's visible reaction for presentation. `out`
/// is cleared first, so a buffer reserved once never reallocates and the tick
/// holds no heap of its own. Never panics on user data.
pub fn reaction_step_into(
    inputs: &[ParticipantInput],
    state: &mut FightState,
    preset: &FeelPreset,
    base: &Tables,
    out: &mut Vec<FightEvent>,
) -> Reaction {
    let tables = preset.tables(*base);
    step_with_tables_into(inputs, state, &tables, out);
    reaction_for_tick(out, *preset, tables)
}

/// Button cadence driving one reaction capture: the jab lands a clean hit
/// (kp01), the chip lands through a held block past the parry window (kp05
/// hit-stop context), the parry answers on the impact tick (kp06).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReactionScript {
    /// Clean hit: freeze plus flash plus damage number.
    Jab,
    /// Held block past the window: chip number with no freeze.
    Chip,
    /// Fresh block on the impact tick: freeze plus flash, no number.
    Parry,
}

/// Attacker button bit for one bout step: tap once at the press tick.
const fn attacker_buttons(step: u32, press_at: u32) -> u8 {
    if step == press_at {
        ATTACK
    } else {
        0
    }
}

/// Defender button bits for one bout step under a script: idle for the jab,
/// held from the first tick for the chip (so the block ages out before
/// impact), pressed only on the impact tick for the parry.
const fn defender_buttons(script: ReactionScript, step: u32, clock: u32) -> u8 {
    match script {
        ReactionScript::Jab => 0,
        ReactionScript::Chip => BLOCK,
        ReactionScript::Parry => {
            if step == clock {
                BLOCK
            } else {
                0
            }
        }
    }
}

/// Data capture for one scripted reaction: the clock tick, the script, the
/// captured event, its visible triple and the sim digest at the window end.
/// Presentation reads the reaction; the digest proves the sim never moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReactionCapture {
    /// Bout tick of the captured contact.
    pub clock: u32,
    /// Button cadence that drove it.
    pub script: ReactionScript,
    /// The contact event at the clock tick.
    pub event: FightEvent,
    /// Its visible freeze, flash and damage number.
    pub reaction: Reaction,
    /// Sim digest at the window end (see [`contact_sim_digest`]).
    pub sim_digest: u64,
}

/// Script one reaction at an exact bout tick and capture its visible triple.
///
/// The bout idles until `clock` minus the resolved startup, presses per
/// `script` (jab idle, chip held, parry fresh), then drains past the freeze
/// so the sim digest covers the same window twice. Returns `None` when no
/// contact lands exactly at `clock` — a clock before the resolved startup, or
/// rows whose press never affords an attack. Harness only: it owns scratch
/// event buffers, never the tick.
pub fn reaction_capture(
    script: ReactionScript,
    clock: u32,
    preset: &FeelPreset,
    base: &Tables,
) -> Option<ReactionCapture> {
    let tables = preset.tables(*base);
    let press_at = clock.saturating_sub(tables.attack_startup_ticks);
    if press_at.saturating_add(tables.attack_startup_ticks) != clock {
        return None;
    }
    let end = clock
        .saturating_add(tables.attack_recovery_ticks)
        .saturating_add(tables.hitstop_ticks)
        .saturating_add(2);
    let mut state = FightState::new(2).ok()?;
    let mut tick_events = Vec::new();
    let mut window: Vec<FightEvent> = Vec::new();
    let mut first: Option<FightEvent> = None;
    let mut step = 0u32;
    loop {
        let attacker = attacker_buttons(step, press_at);
        let defender = defender_buttons(script, step, clock);
        let inputs = [
            capture_input(0, attacker, step)?,
            capture_input(1, defender, step)?,
        ];
        step_with_tables_into(&inputs, &mut state, &tables, &mut tick_events);
        let mut i = 0;
        while i < tick_events.len() {
            let event = tick_events[i];
            window.push(event);
            if step == clock && first.is_none() && is_contact(script, event) {
                first = Some(event);
            }
            i += 1;
        }
        if step == end {
            break;
        }
        step = step.saturating_add(1);
    }
    let event = first?;
    let reaction = reaction_for_event(event, *preset, tables);
    let sim_digest = contact_sim_digest(&state, &window);
    Some(ReactionCapture {
        clock,
        script,
        event,
        reaction,
        sim_digest,
    })
}

/// True when the event is the script's awaited contact: the jab waits for a
/// clean hit, the chip for a blocked chip, the parry for a parry answer.
const fn is_contact(script: ReactionScript, event: FightEvent) -> bool {
    matches!(
        (script, event),
        (ReactionScript::Jab, FightEvent::Hit { .. })
            | (ReactionScript::Chip, FightEvent::Blocked { .. })
            | (ReactionScript::Parry, FightEvent::Parried { .. })
    )
}

/// Reaction capture at tick 30 under the shipped rows: the early-overlap
/// frame, one per script.
pub fn reaction_capture_t30(script: ReactionScript) -> Option<ReactionCapture> {
    use crate::fight::TABLES_V1;
    reaction_capture(script, 30, &FeelPreset::v1(), &TABLES_V1)
}

/// Reaction capture at tick 180 under the shipped rows: the late-overlap
/// frame, one per script.
pub fn reaction_capture_t180(script: ReactionScript) -> Option<ReactionCapture> {
    use crate::fight::TABLES_V1;
    reaction_capture(script, 180, &FeelPreset::v1(), &TABLES_V1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fight::{step_with_tables, step_with_tables_into, TABLES_V1};
    use engine_input::{Buttons, ParticipantId};

    fn input(fighter: u8, buttons: u8) -> ParticipantInput {
        ParticipantInput {
            tick: 0,
            fighter: ParticipantId::new(fighter).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons(buttons),
        }
    }

    fn none(fighter: u8) -> ParticipantInput {
        input(fighter, 0)
    }

    #[test]
    fn kp01_jab_shows_freeze_flash_and_number() {
        // kp01 (jab lands): the clean hit carries all three visible channels.
        let preset = FeelPreset::v1();
        let tables = preset.tables(TABLES_V1);
        let event = FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: tables.attack_damage,
        };
        let reaction = reaction_for_event(event, preset, tables);
        assert!(!reaction.is_quiet());
        assert_eq!(reaction.freeze_ticks, tables.hitstop_ticks);
        assert_eq!(reaction.flash_ticks, preset.juicing.flash_ticks);
        assert_eq!(reaction.damage_number, tables.attack_damage);
        assert!(Reaction::ZERO.is_quiet());
        assert_eq!(
            Reaction::ZERO,
            reaction_for_event(
                FightEvent::Dodged {
                    attacker: 0,
                    victim: 1
                },
                preset,
                tables,
            )
        );
        // Stamina-outs flash the exhausted fighter (no freeze, no number):
        // the refused attack must read differently from silence.
        assert_eq!(
            Reaction {
                freeze_ticks: 0,
                flash_ticks: preset.juicing.flash_ticks,
                damage_number: 0,
            },
            reaction_for_event(FightEvent::StaminaOut { fighter: 0 }, preset, tables,)
        );
    }

    #[test]
    fn stamina_out_flashes_the_exhausted_fighter() {
        // C5 spam-attack oracle: an attack refused for empty stamina must
        // show an exhausted cue instead of silence, so the player can tell
        // a refused attack from a started one.
        let preset = FeelPreset::v1();
        let tables = preset.tables(TABLES_V1);
        let reaction = reaction_for_event(FightEvent::StaminaOut { fighter: 0 }, preset, tables);
        assert!(
            !reaction.is_quiet(),
            "stamina-out must show an exhausted cue"
        );
        assert_eq!(reaction.freeze_ticks, 0);
        assert_eq!(reaction.flash_ticks, preset.juicing.flash_ticks);
        assert_eq!(reaction.damage_number, 0);
    }

    #[test]
    fn kp05_chip_shows_number_without_freeze() {
        // kp05 (hit-stop context): a chip past the window prints its number
        // but never freezes, because the sim never freezes on a chip.
        let preset = FeelPreset::v1();
        let tables = preset.tables(TABLES_V1);
        let chip = tables.chip(tables.attack_damage);
        assert!(chip > 0, "fixture damage must chip or the case is void");
        let reaction = reaction_for_event(
            FightEvent::Blocked {
                attacker: 0,
                victim: 1,
                chip,
            },
            preset,
            tables,
        );
        assert_eq!(reaction.freeze_ticks, 0);
        assert_eq!(reaction.flash_ticks, preset.juicing.flash_ticks);
        assert_eq!(reaction.damage_number, chip);
        assert!(!reaction.is_quiet());
    }

    #[test]
    fn kp06_parry_shows_freeze_and_flash_without_number() {
        // kp06 (parry riposte): the parry answer freezes and flashes but
        // prints no number — no damage changed hands.
        let preset = FeelPreset::v1();
        let tables = preset.tables(TABLES_V1);
        let reaction = reaction_for_event(
            FightEvent::Parried {
                attacker: 0,
                victim: 1,
            },
            preset,
            tables,
        );
        assert_eq!(reaction.freeze_ticks, tables.hitstop_ticks);
        assert_eq!(reaction.flash_ticks, preset.juicing.flash_ticks);
        assert_eq!(reaction.damage_number, 0);
        assert!(!reaction.is_quiet());
        // The knockdown alone prints nothing: the precipitating hit on the
        // same tick already carries the number, and the fold keeps it.
        let down = reaction_for_event(
            FightEvent::Knockdown {
                attacker: 0,
                victim: 1,
            },
            preset,
            tables,
        );
        assert_eq!(down.damage_number, 0);
        let hit = reaction_for_event(
            FightEvent::Hit {
                attacker: 0,
                victim: 1,
                damage: tables.attack_damage,
            },
            preset,
            tables,
        );
        let folded = reaction_for_tick(
            &[
                FightEvent::Hit {
                    attacker: 0,
                    victim: 1,
                    damage: tables.attack_damage,
                },
                FightEvent::Knockdown {
                    attacker: 0,
                    victim: 1,
                },
            ],
            preset,
            tables,
        );
        assert_eq!(folded, hit.merge(down));
        assert_eq!(folded.damage_number, tables.attack_damage);
        assert_eq!(folded.freeze_ticks, tables.hitstop_ticks);
    }

    #[test]
    fn unvalidated_rows_clamp_without_panicking() {
        // Out-of-range rows clamp to the presentation caps instead of
        // panicking, so a hot tuning row can never break presentation.
        let mut tables = TABLES_V1;
        tables.hitstop_ticks = MAX_REACTION_FREEZE_TICKS + 9;
        let preset = FeelPreset::v1();
        let reaction = reaction_for_event(
            FightEvent::Hit {
                attacker: 0,
                victim: 1,
                damage: MAX_REACTION_NUMBER_PERMILLE + 40,
            },
            preset,
            tables,
        );
        assert_eq!(reaction.freeze_ticks, MAX_REACTION_FREEZE_TICKS);
        assert_eq!(reaction.flash_ticks, preset.juicing.flash_ticks);
        assert_eq!(reaction.damage_number, MAX_REACTION_NUMBER_PERMILLE);
    }

    #[test]
    fn captures_land_at_t30_and_t180_with_sim_bytes_twice() {
        // Every script lands on its clock at both overlap frames, shows its
        // channels, and replays byte-identically (sim bytes x2).
        for script in [
            ReactionScript::Jab,
            ReactionScript::Chip,
            ReactionScript::Parry,
        ] {
            for clock in [30u32, 180u32] {
                let capture = reaction_capture(script, clock, &FeelPreset::v1(), &TABLES_V1)
                    .expect("scripted reaction lands on its clock");
                assert_eq!(capture.clock, clock);
                assert_eq!(capture.script, script);
                assert!(is_contact(script, capture.event));
                match script {
                    ReactionScript::Jab => {
                        assert_ne!(capture.reaction.freeze_ticks, 0);
                        assert_ne!(capture.reaction.flash_ticks, 0);
                        assert_ne!(capture.reaction.damage_number, 0);
                    }
                    ReactionScript::Chip => {
                        assert_eq!(capture.reaction.freeze_ticks, 0);
                        assert_ne!(capture.reaction.damage_number, 0);
                    }
                    ReactionScript::Parry => {
                        assert_ne!(capture.reaction.freeze_ticks, 0);
                        assert_eq!(capture.reaction.damage_number, 0);
                    }
                }
                let again = reaction_capture(script, clock, &FeelPreset::v1(), &TABLES_V1)
                    .expect("replay lands too");
                assert_eq!(capture, again);
                assert_eq!(capture.sim_digest, again.sim_digest);
            }
        }
        // The pinned overlap frames agree with the generic runner, and the
        // digest separates the two clocks.
        let early = reaction_capture_t30(ReactionScript::Jab).expect("t30 lands");
        let late = reaction_capture_t180(ReactionScript::Jab).expect("t180 lands");
        assert_eq!(
            early,
            reaction_capture(ReactionScript::Jab, 30, &FeelPreset::v1(), &TABLES_V1)
                .expect("generic runner agrees")
        );
        assert_ne!(early.sim_digest, late.sim_digest);
    }

    #[test]
    fn capture_refuses_off_clock_ticks() {
        // A clock before the resolved startup cannot hold its own contact:
        // the press slides to zero and the impact lands at startup instead.
        assert_eq!(
            reaction_capture(ReactionScript::Jab, 0, &FeelPreset::v1(), &TABLES_V1),
            None
        );
        assert_eq!(
            reaction_capture(ReactionScript::Parry, 2, &FeelPreset::v1(), &TABLES_V1),
            None
        );
        assert!(reaction_capture_t30(ReactionScript::Chip).is_some());
    }

    #[test]
    fn step_matches_tables_step_on_the_same_tick_without_growing_buffers() {
        // The visible triple rides the same tick as the impact ( P2
        // same-tick window): quiet ticks fold quiet, the impact tick folds
        // its reaction at once, and state plus events match the tables step.
        let preset = FeelPreset::v1();
        let base = TABLES_V1;
        let tables = preset.tables(base);
        let mut state = FightState::new(2).unwrap();
        let mut direct = FightState::new(2).unwrap();
        let mut events = Vec::with_capacity(crate::fight::MAX_EVENTS_PER_TICK);
        let mut direct_events = Vec::with_capacity(crate::fight::MAX_EVENTS_PER_TICK);
        let cap = events.capacity();
        for _ in 0..4 {
            let reaction =
                reaction_step_into(&[none(0), none(1)], &mut state, &preset, &base, &mut events);
            assert_eq!(reaction, Reaction::ZERO);
            step_with_tables_into(
                &[none(0), none(1)],
                &mut direct,
                &tables,
                &mut direct_events,
            );
        }
        assert_eq!(state, direct);
        // Direct bout proves the wiring: the same press through the bare
        // tables entry lands the same hit the reaction folds.
        let mut bare = FightState::new(2).unwrap();
        let bare_events = step_with_tables(&[input(0, ATTACK), none(1)], &mut bare, &tables);
        assert!(bare_events.is_empty(), "press tick carries no impact yet");
        let reaction = reaction_step_into(
            &[input(0, ATTACK), none(1)],
            &mut state,
            &preset,
            &base,
            &mut events,
        );
        step_with_tables_into(
            &[input(0, ATTACK), none(1)],
            &mut direct,
            &tables,
            &mut direct_events,
        );
        assert_eq!(reaction, Reaction::ZERO);
        let mut hit_tick: Option<Reaction> = None;
        for _ in 0..12 {
            let reaction =
                reaction_step_into(&[none(0), none(1)], &mut state, &preset, &base, &mut events);
            step_with_tables_into(
                &[none(0), none(1)],
                &mut direct,
                &tables,
                &mut direct_events,
            );
            if events.iter().any(|e| matches!(e, FightEvent::Hit { .. })) {
                hit_tick = Some(reaction);
            } else if hit_tick.is_none() {
                assert_eq!(reaction, Reaction::ZERO);
            }
            assert_eq!(state, direct);
            assert_eq!(events, direct_events);
        }
        let hit = hit_tick.expect("the scripted press must land its hit");
        assert_eq!(hit.freeze_ticks, tables.hitstop_ticks);
        assert_eq!(hit.flash_ticks, preset.juicing.flash_ticks);
        assert_eq!(hit.damage_number, tables.attack_damage);
        assert_eq!(events.capacity(), cap);
    }
}

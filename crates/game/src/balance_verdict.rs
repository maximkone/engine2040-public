//! balance_verdict: window text for the integer balance verdicts.
//!
//! Consumer: the window HUD (one short line per slot, e.g. `slot 1: fallen`)
//! and its offscreen captures. Data owner: this module owns only the text
//! mapping; `engine-arena` owns the verdict state (`Sim::balance`), and
//! `fighter_look` owns the pose (lean, flat, kneel). Units: slot index.
//! Determinism scope: pure text in, same text out; never hashed, never read
//! by the sim. Errors: none; every gait maps, every slot number formats.

use engine_arena::GaitPhase;
use engine_audio::FightCue;

/// Short verdict word for one gait phase, for the window HUD.
/// Pure; same gait gives the same word twice.
#[must_use]
pub fn verdict_label(gait: GaitPhase) -> &'static str {
    match gait {
        GaitPhase::Steady => "steady",
        GaitPhase::Stumbling => "stumbling",
        GaitPhase::Fallen => "fallen",
        GaitPhase::Catching => "catching",
    }
}

/// True when the slot is down (fallen or catching): the window draws the
/// down marker for these. Pure.
#[must_use]
pub fn is_down(gait: GaitPhase) -> bool {
    matches!(gait, GaitPhase::Fallen | GaitPhase::Catching)
}

/// Append one window line (`slot {slot}: {label}`) to `out`, reusing its
/// buffer. The caller clears once per frame and pushes one line per slot,
/// so a frame costs no new allocation after warmup. Pure text.
pub fn push_verdict_line(slot: u8, gait: GaitPhase, out: &mut String) {
    out.push_str("slot ");
    push_slot(slot, out);
    out.push_str(": ");
    out.push_str(verdict_label(gait));
}

/// Count down slots in `gaits`: the window summary number. Pure.
#[must_use]
pub fn count_down(gaits: &[GaitPhase]) -> usize {
    gaits.iter().filter(|gait| is_down(**gait)).count()
}

/// Fold one tick's fight cues into live verdict gaits (EG45): a `Hit`
/// stumbles its victim, a `Knockdown` floors them; anything else leaves the
/// gaits alone and a fall never downgrades to a stumble. Out-of-range victims
/// are ignored. Pure over the slice; presentation only, never hashed. Mirrors
/// the look's cue mirror (`fighter_look` stumbles and floors the same cues
/// for the pose); this copy serves the text while the look serves the body.
///
/// Caller: `crate::windowed` smoke verdict lines (the headless smokes fold
/// each tick's [`TickOutcome`](crate::TickOutcome) cues and count the text
/// through the HUD path, so this text has a live caller outside its tests).
pub fn fold_verdict_gaits(gaits: &mut [GaitPhase], cues: &[FightCue]) {
    for cue in cues {
        match *cue {
            FightCue::Hit { victim, .. } => {
                if let Some(gait) = gaits.get_mut(usize::from(victim)) {
                    match *gait {
                        GaitPhase::Fallen | GaitPhase::Catching => {}
                        GaitPhase::Steady | GaitPhase::Stumbling => {
                            *gait = GaitPhase::Stumbling;
                        }
                    }
                }
            }
            FightCue::Knockdown { victim, .. } => {
                if let Some(gait) = gaits.get_mut(usize::from(victim)) {
                    *gait = GaitPhase::Fallen;
                }
            }
            FightCue::Blocked { .. }
            | FightCue::Parried { .. }
            | FightCue::Dodged { .. }
            | FightCue::StaminaOut { .. } => {}
        }
    }
}

/// Slot number as decimal text, pushed digit by digit: `u8` is at most three
/// digits, so no temp string is built. Pure.
fn push_slot(slot: u8, out: &mut String) {
    let hundreds = slot / 100;
    let tens = (slot / 10) % 10;
    let ones = slot % 10;
    if hundreds > 0 {
        out.push(digit_char(hundreds));
    }
    if hundreds > 0 || tens > 0 {
        out.push(digit_char(tens));
    }
    out.push(digit_char(ones));
}

/// One decimal digit (`0..=9`) as its ASCII char. The caller only passes
/// remainders of `/ 100`, `/ 10 % 10`, `% 10`, so the sum stays on `0..=9`.
fn digit_char(digit: u8) -> char {
    (b'0' + digit) as char
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gait_names_its_word() {
        assert_eq!(verdict_label(GaitPhase::Steady), "steady");
        assert_eq!(verdict_label(GaitPhase::Stumbling), "stumbling");
        assert_eq!(verdict_label(GaitPhase::Fallen), "fallen");
        assert_eq!(verdict_label(GaitPhase::Catching), "catching");
    }

    #[test]
    fn only_the_down_gaits_read_down() {
        assert!(!is_down(GaitPhase::Steady));
        assert!(!is_down(GaitPhase::Stumbling));
        assert!(is_down(GaitPhase::Fallen));
        assert!(is_down(GaitPhase::Catching));
    }

    #[test]
    fn window_line_names_slot_and_verdict() {
        let mut first = String::new();
        push_verdict_line(1, GaitPhase::Fallen, &mut first);
        assert_eq!(first, "slot 1: fallen");
        let mut zeroth = String::new();
        push_verdict_line(0, GaitPhase::Steady, &mut zeroth);
        assert_eq!(zeroth, "slot 0: steady");
        let mut high = String::new();
        push_verdict_line(255, GaitPhase::Catching, &mut high);
        assert_eq!(high, "slot 255: catching");
    }

    #[test]
    fn window_line_reuses_the_buffer() {
        let mut buf = String::with_capacity(64);
        let ptr = buf.as_ptr();
        push_verdict_line(0, GaitPhase::Steady, &mut buf);
        buf.clear();
        push_verdict_line(1, GaitPhase::Stumbling, &mut buf);
        assert_eq!(buf, "slot 1: stumbling");
        assert_eq!(buf.as_ptr(), ptr, "no regrow after warmup");
    }

    #[test]
    fn down_count_matches_the_down_slots() {
        let gaits = [
            GaitPhase::Steady,
            GaitPhase::Fallen,
            GaitPhase::Stumbling,
            GaitPhase::Catching,
        ];
        assert_eq!(count_down(&gaits), 2);
        assert_eq!(count_down(&[]), 0);
    }

    #[test]
    fn same_verdict_twice_reads_twice() {
        assert_eq!(
            verdict_label(GaitPhase::Fallen),
            verdict_label(GaitPhase::Fallen)
        );
        let gaits = [GaitPhase::Stumbling, GaitPhase::Fallen];
        assert_eq!(count_down(&gaits), count_down(&gaits));
    }

    #[test]
    fn verdict_fold_tracks_hits_and_falls() {
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        };
        let kd = FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        };
        let mut gaits = [GaitPhase::Steady, GaitPhase::Steady];
        fold_verdict_gaits(&mut gaits, &[]);
        assert_eq!(gaits, [GaitPhase::Steady, GaitPhase::Steady]);
        fold_verdict_gaits(&mut gaits, &[hit]);
        assert_eq!(gaits[1], GaitPhase::Stumbling);
        fold_verdict_gaits(&mut gaits, &[hit]);
        assert_eq!(
            gaits[1],
            GaitPhase::Stumbling,
            "repeat hits hold the stumble"
        );
        fold_verdict_gaits(&mut gaits, &[kd]);
        assert_eq!(gaits[1], GaitPhase::Fallen);
        fold_verdict_gaits(&mut gaits, &[hit]);
        assert_eq!(gaits[1], GaitPhase::Fallen, "a fall never downgrades");
        assert_eq!(gaits[0], GaitPhase::Steady, "attacker untouched");
    }

    #[test]
    fn verdict_fold_ignores_bad_slots() {
        let far = FightCue::Knockdown {
            attacker: 0,
            victim: 9,
        };
        let mut gaits = [GaitPhase::Steady];
        fold_verdict_gaits(&mut gaits, &[far]);
        assert_eq!(gaits, [GaitPhase::Steady], "out-of-range victim ignored");
        let mut empty: [GaitPhase; 0] = [];
        fold_verdict_gaits(&mut empty, &[far]);
        assert!(empty.is_empty(), "empty gaits never panic");
    }
}

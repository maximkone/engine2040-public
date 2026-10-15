//! audio_director: arena-mix bridge into the tension director.
//!
//! Consumer: [`App`](crate::App) drives [`Director::update`] once per accepted
//! fixed tick with the tick's fight cues stamped at ringside distance, and
//! serves the returned [`Layers`] on every [`TickOutcome`](crate::TickOutcome)
//! for the window mix (crowd beds + music scalar); headless tests assert the
//! numbers. Data owner: `engine-audio` owns the director brain; this module
//! only maps cues and re-exports its types. Units: integer cm for distance,
//! intensity and gains in `0..1` at 60 Hz. Determinism: same cue script gives
//! the same intensity curve twice (presentation only, never hashed; the golden
//! `d57b5c8ae3096614` stands). Errors: none on the per-tick path (the
//! director clamps; bad tables refuse at construction inside `engine-audio`).
//! Budget: [`Director::update`] costs at most 0.05 ms per tick (the audio
//! `budget_probe_per_update` probe); this bridge adds one pass over at most
//! [`MAX_EVENTS_PER_UPDATE`] cues on a stack buffer, with no heap per tick.

use engine_audio::crowd::FightCue;

pub use engine_audio::director::{
    Director, DirectorError, DirectorState, Layers, PlacedCue, TensionTable, BUDGET_MS_PER_UPDATE,
    MAX_EVENTS_PER_UPDATE, TICK_HZ,
};

/// Listener seat for the mix: 2.5 m ringside, inside the scripted full-weight
/// radius (400 cm), so every heard exchange lands at full excitation and the
/// room heat tracks the fight, not the seat. One fixed seat keeps the curve a
/// pure function of the cue script (deterministic replay x2).
pub const LISTENER_DISTANCE_CM: u32 = 250;

/// Padding cue for the stack buffer: a stamina whiff carries zero excitation
/// at any distance, so truncated slots outside `..n` never affect the update.
const PAD: PlacedCue = PlacedCue {
    cue: FightCue::StaminaOut { fighter: 0 },
    distance_cm: 0,
};

/// Stamp one fight cue at the ringside seat.
#[must_use]
pub fn place(cue: FightCue) -> PlacedCue {
    PlacedCue::new(cue, LISTENER_DISTANCE_CM)
}

/// Drive the director for one accepted tick: map at most
/// [`MAX_EVENTS_PER_UPDATE`] cues (head of the burst, in order) onto the
/// stack buffer and run [`Director::update`] exactly once. Borrows only; no
/// heap. Returns the mix answer for the tick.
pub fn drive(director: &mut Director, cues: &[FightCue]) -> Layers {
    let mut buf = [PAD; MAX_EVENTS_PER_UPDATE];
    let n = cues.len().min(MAX_EVENTS_PER_UPDATE);
    for (slot, cue) in buf.iter_mut().zip(cues.iter()).take(n) {
        *slot = place(*cue);
    }
    director.update(&buf[..n])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, GameConfig};
    use engine_input::{Buttons, FighterId, FighterInput};

    use crate::HUMAN_SLOT;

    const TICKS: u32 = 600;

    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: TICKS,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    fn human(tick: u32, mx: i8, mz: i8, buttons: Buttons) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).unwrap(),
            mx,
            mz,
            facing: 0,
            buttons,
        }
    }

    /// Circle-strafe with an attack every 5 ticks: stays in range and trades
    /// steadily so the room reads hotter than an idle room.
    fn script(tick: u32) -> FighterInput {
        let phase = (tick / 15) % 4;
        let (mx, mz) = match phase {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        let buttons = if tick.is_multiple_of(5) {
            Buttons(engine_input::fighter::ATTACK)
        } else {
            Buttons::NONE
        };
        human(tick, mx, mz, buttons)
    }

    /// Short script for every gate here: fewer ticks keep the O(tick^2) prefix
    /// reruns fast. The full 600-tick replay (now including layers) lives in
    /// the crate-root session tests.
    const SHORT_TICKS: u32 = 200;

    fn run_short(inputs: impl Fn(u32) -> FighterInput) -> (App, Vec<crate::TickOutcome>) {
        let short = GameConfig {
            ticks: SHORT_TICKS,
            ..config()
        };
        let mut app = App::new(short).unwrap();
        let mut outcomes = Vec::with_capacity(SHORT_TICKS as usize);
        for tick in 0..SHORT_TICKS {
            outcomes.push(app.tick(inputs(tick)));
        }
        (app, outcomes)
    }

    fn idle(tick: u32) -> FighterInput {
        human(tick, 0, 0, Buttons::NONE)
    }

    fn peak_intensity(outcomes: &[crate::TickOutcome]) -> f32 {
        outcomes
            .iter()
            .map(|o| o.layers.intensity)
            .fold(0.0f32, f32::max)
    }

    #[test]
    fn director_reached_every_accepted_tick() {
        let (app, outcomes) = run_short(script);
        assert_eq!(outcomes.len(), SHORT_TICKS as usize);
        // Every accepted tick drives exactly one update: the director clock
        // matches the session cursor, and every outcome carries its layers.
        assert_eq!(app.director_updates(), u64::from(SHORT_TICKS));
        assert_eq!(app.cursor(), SHORT_TICKS);
        for (tick, outcome) in outcomes.iter().enumerate() {
            assert!(outcome.input_accepted, "script inputs validate");
            assert_eq!(outcome.tick, tick as u32);
            assert!((0.0..=1.0).contains(&outcome.layers.intensity));
            assert_eq!(outcome.layers.music, outcome.layers.intensity);
            assert!((0.0..=1.0).contains(&outcome.layers.cheer));
            assert!((0.0..=1.0).contains(&outcome.layers.gasp));
            assert!((0.0..=1.0).contains(&outcome.layers.boo));
            assert!(
                outcome.layers.murmur >= engine_audio::director::MURMUR_FLOOR - 1e-6,
                "bed never cuts to air"
            );
        }
        assert_eq!(app.director_layers(), outcomes.last().unwrap().layers);
    }

    #[test]
    fn close_fight_hotter_than_idle() {
        let (_, fight) = run_short(script);
        let (_, rest) = run_short(idle);
        let fight_cues: usize = fight.iter().map(|o| o.cues.len()).sum();
        let fight_peak = peak_intensity(&fight);
        let rest_peak = peak_intensity(&rest);
        assert!(
            fight_cues > 0,
            "scripted fight must land cues (got none in {SHORT_TICKS} ticks)"
        );
        assert!(
            fight_peak > rest_peak + 0.05,
            "close fight {fight_peak:.3} must read clearly hotter than idle {rest_peak:.3}"
        );
    }

    #[test]
    fn sustained_flurry_traverses_the_arc() {
        // Tuning probe through the real per-tick call path (tuning hook): with
        // cooling disabled the same scripted flurry must ratchet through
        // rising into climax and open the cheer bed; a bad table refuses.
        let short = GameConfig {
            ticks: SHORT_TICKS,
            ..config()
        };
        let mut app = App::new(short).unwrap();
        let mut tuned = app.director_table();
        tuned.decay_per_tick = 0.0;
        app.set_director_table(tuned)
            .expect("zero decay is a legal table");
        let mut peak = 0.0f32;
        let mut cheer = 0.0f32;
        let (mut saw_rising, mut saw_climax) = (false, false);
        for tick in 0..SHORT_TICKS {
            let outcome = app.tick(script(tick));
            peak = peak.max(outcome.layers.intensity);
            cheer = cheer.max(outcome.layers.cheer);
            match app.director_state() {
                DirectorState::Rising => saw_rising = true,
                DirectorState::Climax => saw_climax = true,
                DirectorState::Murmur | DirectorState::Release => {}
            }
        }
        assert!(saw_rising, "flurry must pass rising (peak {peak:.3})");
        assert!(saw_climax, "sustained flurry must climax (peak {peak:.3})");
        assert!(
            cheer > 0.5,
            "climax must open the cheer bed (cheer {cheer:.3})"
        );
        assert_eq!(app.director_updates(), u64::from(SHORT_TICKS));
        // A bad table refuses and keeps the tuned one.
        let mut bad = tuned;
        bad.hit = f32::NAN;
        assert!(app.set_director_table(bad).is_err(), "NaN heat refuses");
        assert_eq!(app.director_table(), tuned);
    }

    #[test]
    fn scripted_fight_replays_exactly() {
        let (first_app, first) = run_short(script);
        let (second_app, second) = run_short(script);
        assert_eq!(first_app.director_updates(), second_app.director_updates());
        assert_eq!(
            first_app.director_layers(),
            second_app.director_layers(),
            "tail layers must match"
        );
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.layers, b.layers, "tick {} layers differ", a.tick);
            assert_eq!(a, b, "tick {} outcomes differ", a.tick);
        }
    }

    #[test]
    fn rejected_input_never_drives_the_director() {
        let mut app = App::new(config()).unwrap();
        assert_eq!(app.director_updates(), 0);
        // Wrong slot: rejected, director clock and layers untouched.
        let bad = FighterInput {
            fighter: FighterId::new(1).unwrap(),
            ..idle(0)
        };
        let before = app.director_layers();
        let rejected = app.tick(bad);
        assert!(!rejected.input_accepted);
        assert_eq!(app.director_updates(), 0);
        assert_eq!(app.director_layers(), before);
        assert_eq!(rejected.layers, before);
        // Accepted tick drives exactly one update.
        assert!(app.tick(idle(999)).input_accepted);
        assert_eq!(app.director_updates(), 1);
    }

    #[test]
    fn burst_past_the_cap_keeps_deterministic_head() {
        // More cues than one update integrates: head wins, order kept.
        let cues = vec![
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120
            };
            MAX_EVENTS_PER_UPDATE + 9
        ];
        let mut first = Director::scripted();
        let mut second = Director::scripted();
        let a = drive(&mut first, &cues);
        let b = drive(&mut second, &cues[..MAX_EVENTS_PER_UPDATE]);
        assert_eq!(a, b, "cap keeps the head of the burst");
        assert_eq!(first.tick_count(), 1);
    }

    #[test]
    #[ignore = "timing: bridge plus update budget probe, run under --perf in release"]
    fn budget_probe_drive_plus_update() {
        // Ringside burst per tick: mapping plus update must stay under budget.
        // Wall clock lives only in this ignored probe, never in `drive`.
        let cues = vec![
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120
            };
            MAX_EVENTS_PER_UPDATE
        ];
        let mut director = Director::scripted();
        drive(&mut director, &cues);
        let rounds = 4000u32;
        let start = std::time::Instant::now();
        let mut folded = 0u32;
        for _ in 0..rounds {
            let layers = drive(&mut director, &cues);
            folded ^= layers.intensity.to_bits();
        }
        std::hint::black_box(folded);
        let mean_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
        assert!(
            mean_ms < BUDGET_MS_PER_UPDATE as f64,
            "mean {mean_ms:.4} ms per drive+update over budget"
        );
    }
}

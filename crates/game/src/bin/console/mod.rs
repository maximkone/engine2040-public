//! arena console wiring: the window's door, key routing and effects.
//!
//! Consumer: `bin/arena.rs`. The parser and pause/step state live in
//! `engine_game::console`; this module binds them to the live App, the winit
//! key event, the virtual clock and the presenter. Presentation only, never
//! hashed: a console step owes one whole fixed tick.
//!
//! Errors: key routing never fails; a refused line is printed and the console
//! stays open.

use engine_game::console::{ConsoleDoor, ConsoleEffect, ConsoleKnob};
use engine_game::App;
use winit::event::ElementState;

use super::ArenaApp;

/// The window's read-only console door over the live App authority. The
/// endpoint crate can implement [`ConsoleDoor`] over the net session later;
/// the window never needs that dependency.
struct WindowDoor<'a> {
    app: &'a App,
}

impl ConsoleDoor for WindowDoor<'_> {
    fn hash(&self) -> u64 {
        self.app.checkpoints().last().map_or(0, |point| point.hash)
    }

    fn tick(&self) -> u32 {
        self.app.cursor()
    }

    fn stats(&self) -> String {
        format!(
            "fighters={} sim_steps={} hp={:?}",
            self.app.fighter_views().len(),
            self.app.sim_steps(),
            self.app.hp()
        )
    }
}

impl ArenaApp {
    /// True when the console owns this key. While open it hands every key to
    /// [`Self::console_key`]; while closed Backquote opens it and clears held
    /// fight buttons so the key that opened it cannot move the fighter.
    pub(super) fn console_owns_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        if self.console.is_open() {
            self.console_key(event);
            return true;
        }
        if event.state == ElementState::Pressed
            && !event.repeat
            && matches!(
                event.physical_key,
                winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Backquote)
            )
        {
            self.console.set_open(true);
            self.pressed.clear();
            return true;
        }
        false
    }

    /// Route one key to the open console: Backquote/Escape close, Enter runs
    /// the line, Backspace edits, printable text appends. Every key is
    /// consumed, so typing never moves the fighter.
    fn console_key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::{KeyCode, PhysicalKey};
        let pressed = event.state == ElementState::Pressed;
        let struck = pressed && !event.repeat;
        match event.physical_key {
            PhysicalKey::Code(KeyCode::Backquote | KeyCode::Escape) => {
                if struck {
                    self.console.set_open(false);
                }
            }
            PhysicalKey::Code(KeyCode::Enter) => {
                if struck {
                    self.run_console_line();
                }
            }
            PhysicalKey::Code(KeyCode::Backspace) => {
                if pressed {
                    self.console.backspace();
                }
            }
            _ => {
                if pressed {
                    if let Some(text) = event.text.as_ref() {
                        for ch in text.chars() {
                            self.console.push_char(ch);
                        }
                    }
                }
            }
        }
    }

    /// Execute the typed line against the live authority door, print its
    /// reply, and apply its presentation/time effects outside the tick.
    fn run_console_line(&mut self) {
        let line = self.console.take_line();
        if line.trim().is_empty() {
            return;
        }
        let outcome = {
            let door = WindowDoor { app: &self.app };
            self.console.execute(&line, &door)
        };
        match outcome {
            Ok(reply) => {
                for line in &reply.lines {
                    say!("console: {line}");
                }
                self.apply_console_effects(reply.effects);
            }
            Err(err) => tell!("arena console: {err}"),
        }
    }

    /// Apply console effects after the tick: slow changes the virtual clock's
    /// target, knobs change presenter state. Neither reaches the sim.
    fn apply_console_effects(&mut self, effects: Vec<ConsoleEffect>) {
        for effect in effects {
            match effect {
                ConsoleEffect::Slow { permille } => {
                    if let Err(err) = self.clock.set_speed(permille, 0) {
                        tell!("arena console: slow {permille}: {err}");
                    }
                }
                ConsoleEffect::SetKnob { knob, value } => {
                    self.apply_console_knob(knob, value);
                }
            }
        }
    }

    /// Apply one presentation knob through the existing presenter setters.
    fn apply_console_knob(&mut self, knob: ConsoleKnob, value: i64) {
        let Some(presenter) = self.presenter.as_mut() else {
            tell!(
                "arena console: set-knob {} {value}: no presenter yet",
                knob.name()
            );
            return;
        };
        match knob {
            ConsoleKnob::Msaa => presenter.set_msaa_enabled(value != 0),
            ConsoleKnob::ContactShadow => {
                let permille = u16::try_from(value).unwrap_or(0);
                presenter.set_contact_shadow_strength(f32::from(permille) / 1000.0);
            }
            ConsoleKnob::AutoExposure => presenter.set_auto_exposure_enabled(value != 0),
        }
    }
}

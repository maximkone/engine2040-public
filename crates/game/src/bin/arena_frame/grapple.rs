//! grapple: the spear keys of the window and their end-to-end test.
//!
//! Owner: game lane, input mapping. `Q` throws at the rival the soft lock-on
//! aims at; `Q` with `E` also sets `THROW_SPEAR_WORLD`, so the spear targets no
//! fighter and anchors arena geometry (`docs/plan/spear-grapple.md`). `Ctrl`
//! throws the laquearius noose (row SG11): a loop that snares the rival. The
//! sim masks these bits when the match was built without the spear or the
//! noose, so this never changes an old match. The test lives here because `bin/arena.rs` is a
//! pinned file that may not grow (`docs/RUST-QUALITY.md`).

use crate::ArenaApp;
use engine_input::{FighterInput, PhysicalButton};

impl ArenaApp {
    /// The input with the throw bits set while the spear keys are held and the
    /// grapple is on. Q throws at the fighters; Q with E throws at the world,
    /// so the spear passes them and sticks in arena geometry (the wall swing);
    /// Ctrl throws the noose.
    pub(crate) fn with_spear_key(&self, mut input: FighterInput) -> FighterInput {
        let ctrl = PhysicalButton::Key(engine_input::map::Key::CTRL.0);
        if self.spear.on && self.pressed.contains(&ctrl) {
            input.buttons =
                engine_input::Buttons(input.buttons.0 | engine_input::fighter::THROW_NOOSE);
        }
        let q = PhysicalButton::Key(engine_input::map::Key::Q.0);
        if self.spear.on && self.pressed.contains(&q) {
            let e = PhysicalButton::Key(engine_input::map::Key::E.0);
            let world = if self.pressed.contains(&e) {
                engine_input::fighter::THROW_SPEAR_WORLD
            } else {
                0
            };
            input.buttons = engine_input::Buttons(
                input.buttons.0 | engine_input::fighter::THROW_SPEAR | world,
            );
        }
        input
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{key, walking_session};

    #[test]
    fn q_throws_at_the_rival_and_q_with_e_aims_the_world() {
        // The aim: a duel can reach a wall behind the enemy. Q keeps the
        // lock-on throw, Q+E sets the world bit (the sim flies past fighters),
        // and without the grapple neither bit leaves the window.
        use engine_input::fighter::{THROW_SPEAR, THROW_SPEAR_WORLD};
        use engine_input::map::Key;
        let mut app = walking_session();
        app.enable_spear(true, false, false)
            .expect("the grapple turns on");
        let human = app.human().expect("human slot");
        app.set_pressed(key(Key::Q), true);
        let plain = app.human_input(0, human);
        assert_eq!(plain.buttons.0 & THROW_SPEAR, THROW_SPEAR, "Q throws");
        assert_eq!(
            plain.buttons.0 & THROW_SPEAR_WORLD,
            0,
            "Q alone aims the foe"
        );
        app.set_pressed(key(Key::E), true);
        let aimed = app.human_input(1, human);
        assert_eq!(
            aimed.buttons.0 & (THROW_SPEAR | THROW_SPEAR_WORLD),
            THROW_SPEAR | THROW_SPEAR_WORLD,
            "Q+E carries both bits"
        );
        let mut off = walking_session();
        off.set_pressed(key(Key::Q), true);
        off.set_pressed(key(Key::E), true);
        let none = off.human_input(0, human);
        assert_eq!(
            none.buttons.0 & (THROW_SPEAR | THROW_SPEAR_WORLD),
            0,
            "no grapple, no spear bits"
        );
    }

    #[test]
    fn ctrl_throws_the_noose_and_only_with_the_grapple_on() {
        use engine_input::fighter::{THROW_NOOSE, THROW_SPEAR};
        use engine_input::map::Key;
        let mut app = walking_session();
        app.enable_spear(true, false, false)
            .expect("the grapple turns on");
        assert!(
            app.app.noose(),
            "the window's grapple session carries the noose"
        );
        let human = app.human().expect("human slot");
        let idle = app.human_input(0, human);
        assert_eq!(
            idle.buttons.0 & THROW_NOOSE,
            0,
            "nothing held, nothing thrown"
        );
        app.set_pressed(key(Key::CTRL), true);
        let cord = app.human_input(1, human);
        assert_eq!(
            cord.buttons.0 & THROW_NOOSE,
            THROW_NOOSE,
            "Ctrl throws the noose"
        );
        assert_eq!(cord.buttons.0 & THROW_SPEAR, 0, "and not the spear");
        let mut off = walking_session();
        off.set_pressed(key(Key::CTRL), true);
        assert_eq!(
            off.human_input(0, human).buttons.0 & THROW_NOOSE,
            0,
            "no grapple, no noose bit"
        );
    }
}

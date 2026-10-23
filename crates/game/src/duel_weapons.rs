//! FEEL1b: the duel's per-slot weapon arming, kept out of `lib.rs` so the
//! pinned file does not grow.
//!
//! Consumer: the window (`App::into_duel`) and every rebuild or net peer
//! through `App::spec`. Data owner: `engine-arena`'s `MatchSpec` and
//! `Sim::arm_weapons`. Presentation reads `App::weapon`/`App::weapon_name`;
//! the armed rows fold into the checkpoint hash, the look times each swing by
//! the row's startup and never writes match state.

use engine_arena::MatchSpec;
use engine_combat::heft::Heft;

use crate::{App, GameError, DUEL_KIT_WEAPON};

impl App {
    /// Move a fresh session to the live duel's combat: tables v2 under rules
    /// v2 and the kit's gladius armed on every slot (FEEL1b: the row equals
    /// the shipped table, so the bout is unchanged and only the armed-weapon
    /// fold moves the checkpoint hash). Only before the first tick; later
    /// calls refuse with `BadConfig("duel_tables")`. The proof scene and LAN
    /// play never call this and stay on v1.
    ///
    /// # Errors
    ///
    /// `BadConfig("duel_tables")` after the first tick, `BadConfig("arena")`
    /// when the tables v2 match does not build.
    pub fn into_duel(self) -> Result<Self, GameError> {
        let weapons = vec![DUEL_KIT_WEAPON; usize::from(self.config.fighters)];
        self.into_duel_with_weapons(&weapons)
    }

    /// [`App::into_duel`] armed with one explicit weapon row per slot, in
    /// slot order (FEEL1b): `["pugio", "clava"]` gives the human slot the
    /// dagger and slot 1 the club. The ids ride [`App::spec`], so every peer
    /// of the session arms the same list. Only before the first tick.
    ///
    /// # Errors
    ///
    /// `BadConfig("duel_tables")` after the first tick, a refused row list
    /// (`BadConfig("duel_weapons")`) or a match that does not build.
    pub fn into_duel_with_weapons(mut self, weapons: &[&'static str]) -> Result<Self, GameError> {
        if self.cursor != 0 || self.retired_steps != 0 || self.live.steps() != 0 {
            return Err(GameError::BadConfig("duel_tables"));
        }
        let mut spec = MatchSpec::duel(self.arena, self.world.clone());
        spec.spear = self.spear;
        spec.height = self.height;
        if !weapons.is_empty() {
            spec = spec.with_weapons(weapons);
        }
        let refusal = if weapons.is_empty() {
            GameError::BadConfig("arena")
        } else {
            GameError::BadConfig("duel_weapons")
        };
        self.weapons = weapons.to_vec();
        self.live = spec.live().ok_or(refusal)?;
        self.arena = spec.config;
        self.rules = spec.rules;
        Ok(self)
    }

    /// The weapon slot `slot` swings in the live duel (FEEL1b): its row
    /// numbers, or `None` when the slot is bare. Presentation reads the
    /// swing startup here.
    #[must_use]
    pub fn weapon(&self, slot: u8) -> Option<Heft> {
        self.live.weapon(slot)
    }

    /// The armed weapon row id slot `slot` swings (FEEL1b), or `None` when
    /// the slot is bare. The HUD names the weapon from this.
    #[must_use]
    pub fn weapon_name(&self, slot: u8) -> Option<&'static str> {
        self.weapons.get(usize::from(slot)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameConfig;
    use engine_input::{fighter::ATTACK, Buttons, FighterId, FighterInput};

    fn config() -> GameConfig {
        GameConfig {
            seed: 7,
            ticks: 64,
            fighters: 2,
            half_extent_m: 12.0,
        }
    }

    fn script(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(0).expect("slot 0 is legal"),
            mx: 90,
            mz: 30,
            facing: 0,
            buttons: if tick.is_multiple_of(25) {
                Buttons(ATTACK)
            } else {
                Buttons::NONE
            },
        }
    }

    /// The default duel arms the kit's gladius on every slot. The row equals
    /// the shipped table, so the bout's cues are the unarmed duel's and only
    /// the armed-weapon fold moves the checkpoint hash.
    #[test]
    fn feel1b_default_duel_arms_the_kit_and_only_the_fold_moves() {
        let mut armed = App::new(config())
            .expect("session")
            .into_duel()
            .expect("duel");
        let mut bare = App::new(config())
            .expect("session")
            .into_duel_with_weapons(&[])
            .expect("duel");
        assert_eq!(armed.weapon_name(0), Some(DUEL_KIT_WEAPON));
        assert_eq!(armed.weapon_name(1), Some(DUEL_KIT_WEAPON));
        assert_eq!(bare.weapon_name(0), None);
        assert_eq!(
            armed.weapon(0).map(|weapon| weapon.startup_ticks),
            Some(15),
            "the gladius row"
        );
        for tick in 0..24 {
            let a = armed.tick(script(tick));
            let b = bare.tick(script(tick));
            assert_eq!(a.cues, b.cues, "the shipped table at tick {tick}");
            assert_ne!(
                a.checkpoint.map(|cp| cp.hash),
                b.checkpoint.map(|cp| cp.hash),
                "the armed fold reaches the hash at tick {tick}"
            );
        }
    }

    /// An explicit list arms slot by slot, and a refused row builds no
    /// session.
    #[test]
    fn feel1b_duel_arms_pugio_against_clava() {
        let app = App::new(config())
            .expect("session")
            .into_duel_with_weapons(&["pugio", "clava"])
            .expect("duel");
        assert_eq!(app.weapon_name(0), Some("pugio"));
        assert_eq!(app.weapon_name(1), Some("clava"));
        assert_eq!(app.weapon(0).map(|weapon| weapon.startup_ticks), Some(12));
        assert_eq!(app.weapon(1).map(|weapon| weapon.startup_ticks), Some(21));
        assert_eq!(app.spec().weapons, vec!["pugio", "clava"]);
        assert!(matches!(
            App::new(config())
                .expect("session")
                .into_duel_with_weapons(&["nope", "clava"]),
            Err(GameError::BadConfig("duel_weapons"))
        ));
    }
}

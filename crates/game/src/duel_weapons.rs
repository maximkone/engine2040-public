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
        spec.noose = self.noose;
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
            Some(30),
            "the gladius row"
        );
        for tick in 0..40 {
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
        assert_eq!(app.weapon(0).map(|weapon| weapon.startup_ticks), Some(24));
        assert_eq!(app.weapon(1).map(|weapon| weapon.startup_ticks), Some(42));
        assert_eq!(app.spec().weapons, vec!["pugio", "clava"]);
        assert!(matches!(
            App::new(config())
                .expect("session")
                .into_duel_with_weapons(&["nope", "clava"]),
            Err(GameError::BadConfig("duel_weapons"))
        ));
    }

    /// Distance at which the approach stops and swings: inside the shipped
    /// 1.5 m blade, so the blow lands on the row's startup tick.
    const NEAR_M: f32 = 1.3;

    /// A rules v2 duel armed pugio against clava (FEEL1b), long enough for the
    /// walk in plus the clava's 42-tick windup.
    fn armed_duel() -> App {
        let config = GameConfig {
            seed: 7,
            ticks: 900,
            fighters: 2,
            half_extent_m: 12.0,
        };
        App::new(config)
            .expect("session")
            .into_duel_with_weapons(&["pugio", "clava"])
            .expect("duel")
    }

    fn slot_input(
        tick: u32,
        slot: u8,
        mx: i8,
        mz: i8,
        facing: i16,
        buttons: Buttons,
    ) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(slot).expect("slots 0 and 1 are legal"),
            mx,
            mz,
            facing,
            buttons,
        }
    }

    /// One slot's feet for the latest tick, or `None` before the first views.
    fn feet(views: &[crate::FighterView], slot: u8) -> Option<[f32; 3]> {
        views
            .iter()
            .find(|view| view.slot == slot)
            .map(|view| view.pos_m)
    }

    /// Slot 0's input for one tick of the approach: walk at slot 1 facing it
    /// (wish 0 in reach) and press `ATTACK` while `press` and in reach.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a bearing is within +-18000 centidegrees and a wish within +-100"
    )]
    fn walker(tick: u32, views: &[crate::FighterView], press: bool) -> FighterInput {
        let (Some(me), Some(foe)) = (feet(views, 0), feet(views, 1)) else {
            return slot_input(tick, 0, 0, 0, 0, Buttons::NONE);
        };
        let to = [foe[0] - me[0], foe[2] - me[2]];
        let dist = to[0].hypot(to[1]).max(1.0e-3);
        let near = dist <= NEAR_M;
        let wish = |component: f32| {
            if near {
                0
            } else {
                (component / dist * 100.0).round() as i8
            }
        };
        let buttons = if press && near {
            Buttons(ATTACK)
        } else {
            Buttons::NONE
        };
        slot_input(
            tick,
            0,
            wish(to[0]),
            wish(to[1]),
            (to[0].atan2(to[1]).to_degrees() * 100.0).round() as i16,
            buttons,
        )
    }

    /// Slot 1's input: stand still, face slot 0, and press `ATTACK` while
    /// `press` and in reach.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a bearing is within +-18000 centidegrees"
    )]
    fn stander(tick: u32, views: &[crate::FighterView], press: bool) -> FighterInput {
        let (Some(me), Some(foe)) = (feet(views, 1), feet(views, 0)) else {
            return slot_input(tick, 1, 0, 0, 0, Buttons::NONE);
        };
        let to = [foe[0] - me[0], foe[2] - me[2]];
        let buttons = if press && to[0].hypot(to[1]) <= NEAR_M {
            Buttons(ATTACK)
        } else {
            Buttons::NONE
        };
        slot_input(
            tick,
            1,
            0,
            0,
            (to[0].atan2(to[1]).to_degrees() * 100.0).round() as i16,
            buttons,
        )
    }

    /// Run one armed bout until `attacker`'s slot lands its first blow: the
    /// press tick of that swing and the blow tick. Only `attacker` ever
    /// presses, and only once (a held press restarts the swing inside the v2
    /// feint window), because a landed blow cancels the other slot's swing.
    fn first_blow(app: &mut App, attacker: u8) -> (u32, u32) {
        let (mut press, mut swung) = (None, false);
        for tick in 0..app.config.ticks {
            let views = app.fighter_views();
            let (mine, foe) = if attacker == 0 {
                (walker(tick, views, !swung), stander(tick, views, false))
            } else {
                (walker(tick, views, false), stander(tick, views, !swung))
            };
            app.offer_remote(foe).expect("the foe slot stages");
            let out = app.tick(mine);
            if out.started.contains(&attacker) {
                press = Some(out.tick);
                swung = true;
            }
            let hit = out.cues.iter().find_map(|cue| match *cue {
                crate::FightCue::Hit { attacker: who, .. } if who == attacker => Some(out.tick),
                _ => None,
            });
            if let Some(hit) = hit {
                return (press.expect("the swing was pressed before it landed"), hit);
            }
        }
        panic!("slot {attacker}'s swing never landed");
    }

    /// FEEL1b: the armed slots swing their own startup, read through the game
    /// and the shipped rows. Slot 0 walks in and swings the pugio; a second
    /// bout of the same arm list stands slot 1 and swings the clava, so the
    /// pugio's blow cannot cancel the clava's windup. Both blows land on their
    /// row's startup tick, and the clava's first hit follows the pugio's by
    /// the rows' startup gap (42 - 24 = 18 ticks today: the row text's older
    /// 9 predates OPT59's doubled startups). The gap is read from
    /// `App::weapon`, never typed here.
    #[test]
    fn feel1b_duel_lands_the_clava_after_the_pugio_by_the_rows() {
        let mut pugio_app = armed_duel();
        let pugio_startup = pugio_app.weapon(0).expect("pugio row").startup_ticks;
        let clava_startup = pugio_app.weapon(1).expect("clava row").startup_ticks;
        assert!(
            clava_startup > pugio_startup,
            "the rows say the club is slower: {clava_startup} vs {pugio_startup}"
        );
        let (pugio_press, pugio_hit) = first_blow(&mut pugio_app, 0);
        let mut clava_app = armed_duel();
        let (clava_press, clava_hit) = first_blow(&mut clava_app, 1);
        assert_eq!(
            pugio_press, clava_press,
            "the same walk reaches reach on the same tick"
        );
        assert_eq!(
            pugio_hit - pugio_press,
            pugio_startup,
            "the pugio lands on its row's startup"
        );
        assert_eq!(
            clava_hit - clava_press,
            clava_startup,
            "the clava lands on its row's startup"
        );
        assert_eq!(
            clava_hit - pugio_hit,
            clava_startup - pugio_startup,
            "the clava's first hit follows the pugio's by the rows' startup gap"
        );
    }
}

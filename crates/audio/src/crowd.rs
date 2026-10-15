//! crowd: deterministic fight-night reactions plus the ambience sidechain.
//!
//! The game audio layer maps each combat [`FightEvent`](https://docs.rs)
//! 1:1 into [`FightCue`] and calls [`Crowd::on_fight_event`], then advances
//! [`Crowd::tick`] once per fixed tick (60 Hz) and multiplies the crowd-bed
//! loop by [`Crowd::bed_gain`]. Units: cooldowns and duck recovery are tick
//! counts, reaction levels are `1..=3`. Determinism scope: same cue script
//! with the same [`tick`](Crowd::tick) cadence gives the same reactions and
//! the same bed schedule; no wall clock, no RNG.
//!
//! Dependency note (contract choice): this module deliberately does **not**
//! import `engine-combat`. Audio stays a leaf crate with no new dependency
//! (the packet's `--locked` build forbids a lockfile change); the caller maps
//! each `FightEvent` onto the same-named [`FightCue`] field for field, and each
//! combat `AttackStarted { attacker }` press cue onto [`Crowd::on_attack_started`].
//! Never import `arena`/`game`/`combat` here either.

/// Cooldown after an `ooh` before another `ooh` may fire (ticks).
pub const OOH_COOLDOWN_TICKS: u64 = 20;
/// Cooldown after a `cheer` before another `cheer` may fire (ticks).
pub const CHEER_COOLDOWN_TICKS: u64 = 45;
/// Cooldown after a `gasp` before another `gasp` may fire (ticks).
pub const GASP_COOLDOWN_TICKS: u64 = 20;
/// Global floor between any two reactions; a louder reaction may still
/// escalate inside the window, an equal-or-quieter one is suppressed.
pub const GLOBAL_COOLDOWN_TICKS: u64 = 6;
/// Bed recovery length after a hit (ticks).
pub const HIT_DUCK_TICKS: u32 = 20;
/// Bed recovery length after a knockdown (ticks).
pub const KNOCKDOWN_DUCK_TICKS: u32 = 45;
/// Bed depth removed by a hit (`bed_gain` floor `1 - depth`).
pub const HIT_DUCK_DEPTH: f32 = 0.5;
/// Bed depth removed by a knockdown (`bed_gain` floor `1 - depth`).
pub const KNOCKDOWN_DUCK_DEPTH: f32 = 0.65;
/// Damage at or above which a hit earns an `ooh` level 2 (combat table value).
pub const HIT_LOUD_DAMAGE: u16 = 120;

/// Plain caller-mapped fight cue. Field-for-field compatible with the combat
/// `FightEvent` shapes in `crates/combat/src/fight.rs` (read-only
/// reference); the game audio layer matches each `FightEvent` variant into
/// the same-named variant here. `Copy` so scripts replay by value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FightCue {
    /// Full damage landed.
    Hit {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
        /// Damage in permille.
        damage: u16,
    },
    /// Held block soaked the hit down to chip damage.
    Blocked {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot.
        victim: u8,
        /// Chip damage in permille.
        chip: u16,
    },
    /// Block inside the parry window: no damage, attacker staggered.
    Parried {
        /// Attacker slot (staggered).
        attacker: u8,
        /// Victim slot (parried).
        victim: u8,
    },
    /// Dodge i-frames negated the hit.
    Dodged {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot (dodged).
        victim: u8,
    },
    /// Attack/dodge pressed without enough stamina; nothing started.
    StaminaOut {
        /// Fighter slot.
        fighter: u8,
    },
    /// Victim dropped to zero hp.
    Knockdown {
        /// Attacker slot.
        attacker: u8,
        /// Victim slot (now at zero hp).
        victim: u8,
    },
}

/// One crowd answer for a cue. Levels run `1..=3` (audible effort);
/// [`Reaction::Quiet`] means the crowd stays on the bed (cooldown held,
/// or the cue is too minor to notice).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reaction {
    /// No audible reaction this cue.
    Quiet,
    /// Impact groan (`Hit`: level 2 at/above [`HIT_LOUD_DAMAGE`], else 1).
    Ooh {
        /// Effort level `1..=2`.
        level: u8,
    },
    /// Roar (`Parried`: 2, `Knockdown`: 3).
    Cheer {
        /// Effort level `2..=3`.
        level: u8,
    },
    /// Sharp intake (`Dodged`: 1).
    Gasp {
        /// Effort level, always 1.
        level: u8,
    },
}

impl Reaction {
    /// True for [`Reaction::Quiet`].
    pub fn is_quiet(self) -> bool {
        matches!(self, Self::Quiet)
    }

    /// Effort level (`0` when quiet).
    pub fn level(self) -> u8 {
        match self {
            Self::Quiet => 0,
            Self::Ooh { level } | Self::Cheer { level } | Self::Gasp { level } => level,
        }
    }
}

/// Fixed-tick crowd state: per-kind reaction cooldowns plus the bed-duck
/// sidechain. Cooldowns stop machine-gun chants; the bed dips under every
/// `Hit`/`Knockdown` (even a cooldown-suppressed one still hits the room)
/// and recovers linearly on [`tick`](Crowd::tick).
#[derive(Clone, Debug)]
pub struct Crowd {
    tick: u64,
    ooh_until: u64,
    cheer_until: u64,
    gasp_until: u64,
    last: Option<(u64, u8)>,
    duck_remaining: u32,
    duck_total: u32,
    duck_depth: f32,
}

impl Crowd {
    /// New crowd at tick 0: bed at full gain, no cooldowns held.
    pub fn new() -> Self {
        Self {
            tick: 0,
            ooh_until: 0,
            cheer_until: 0,
            gasp_until: 0,
            last: None,
            duck_remaining: 0,
            duck_total: 0,
            duck_depth: 0.0,
        }
    }

    /// Fixed-tick clock (the game audio layer calls it once per tick).
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Advance one fixed tick: the clock runs and the bed recovers one
    /// step toward full gain. Never panics.
    pub fn tick(&mut self) {
        self.tick = self.tick.saturating_add(1);
        if self.duck_remaining > 0 {
            self.duck_remaining -= 1;
        }
    }

    /// Bed-loop gain multiplier `0.0..=1.0` for the game audio layer to
    /// apply to the crowd-bed voice. Exact `1.0` when no duck is active; linear ramp
    /// `1 - depth * remaining / total` while recovering, so a dip lands
    /// back on `1.0` exactly on schedule. Never panics.
    pub fn bed_gain(&self) -> f32 {
        if self.duck_remaining == 0 || self.duck_total == 0 {
            return 1.0;
        }
        (1.0 - self.duck_depth * (self.duck_remaining as f32 / self.duck_total as f32))
            .clamp(0.0, 1.0)
    }

    /// React to one fight cue. Mapping: `Hit` -> `Ooh`, `Parried` ->
    /// `Cheer(2)`, `Dodged` -> `Gasp(1)`, `Knockdown` -> `Cheer(3)`,
    /// `Blocked`/`StaminaOut` -> [`Reaction::Quiet`] via
    /// [`on_attack_started`](Self::on_attack_started) (the shared quiet
    /// whiff path). A reaction whose kind cooldown is held, or that is not
    /// louder than a reaction inside [`GLOBAL_COOLDOWN_TICKS`], comes back
    /// [`Reaction::Quiet`] so the crowd cannot machine-gun. Never panics.
    pub fn on_fight_event(&mut self, event: FightCue) -> Reaction {
        enum Kind {
            Ooh,
            Cheer,
            Gasp,
        }
        let (kind, level) = match event {
            FightCue::Hit { damage, .. } => {
                self.duck(HIT_DUCK_DEPTH, HIT_DUCK_TICKS);
                let level = if damage >= HIT_LOUD_DAMAGE { 2 } else { 1 };
                (Some(Kind::Ooh), level)
            }
            FightCue::Blocked { attacker, .. } | FightCue::StaminaOut { fighter: attacker } => {
                return self.on_attack_started(attacker);
            }
            FightCue::Parried { .. } => (Some(Kind::Cheer), 2),
            FightCue::Dodged { .. } => (Some(Kind::Gasp), 1),
            FightCue::Knockdown { .. } => {
                self.duck(KNOCKDOWN_DUCK_DEPTH, KNOCKDOWN_DUCK_TICKS);
                (Some(Kind::Cheer), 3)
            }
        };
        let (kind_cooldown, reaction) = match kind {
            Some(Kind::Ooh) => (OOH_COOLDOWN_TICKS, Reaction::Ooh { level }),
            Some(Kind::Cheer) => (CHEER_COOLDOWN_TICKS, Reaction::Cheer { level }),
            Some(Kind::Gasp) => (GASP_COOLDOWN_TICKS, Reaction::Gasp { level }),
            None => return Reaction::Quiet,
        };
        let until = match kind {
            Some(Kind::Ooh) => &mut self.ooh_until,
            Some(Kind::Cheer) => &mut self.cheer_until,
            Some(Kind::Gasp) => &mut self.gasp_until,
            None => return Reaction::Quiet,
        };
        if self.tick < *until {
            return Reaction::Quiet;
        }
        if let Some((at, last_level)) = self.last {
            if self.tick.wrapping_sub(at) < GLOBAL_COOLDOWN_TICKS && level <= last_level {
                return Reaction::Quiet;
            }
        }
        *until = self.tick.saturating_add(kind_cooldown);
        self.last = Some((self.tick, level));
        reaction
    }

    /// Answer one press-tick attack tell. Mapping: combat
    /// `AttackStarted { attacker }` (TABLES_V2 press tick only) ->
    /// [`Reaction::Quiet`]. The wind-up is too minor to notice: no audible
    /// reaction, no bed dip, no cooldown armed, so contact cues on the same
    /// tick still fire. Units: `attacker` is a fighter slot. Determinism
    /// scope: touches no state, so any cue order replays. Never panics.
    /// Live caller: [`on_fight_event`](Self::on_fight_event) (the whiff
    /// path), reached once per tick from the game live audio path
    /// (`crates/game/src/lib.rs::App::observe_into`).
    pub fn on_attack_started(&self, attacker: u8) -> Reaction {
        let _ = attacker;
        Reaction::Quiet
    }

    /// Start (or deepen) a bed dip: a strictly deeper floor replaces the
    /// running duck, otherwise the running duck keeps its schedule. Pure
    /// fixed-tick state, deterministic on the cue order.
    fn duck(&mut self, depth: f32, ticks: u32) {
        if ticks == 0 {
            return;
        }
        let depth = depth.clamp(0.0, 1.0);
        if self.duck_remaining == 0 || 1.0 - depth < self.bed_gain() {
            self.duck_remaining = ticks;
            self.duck_total = ticks;
            self.duck_depth = depth;
        }
    }
}

impl Default for Crowd {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit() -> FightCue {
        FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        }
    }

    #[test]
    fn reacts_per_event_kind() {
        // Fresh crowd per cue so no cooldown masks the mapping.
        assert_eq!(
            Crowd::new().on_fight_event(hit()),
            Reaction::Ooh { level: 2 }
        );
        assert_eq!(
            Crowd::new().on_fight_event(FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 24,
            }),
            Reaction::Ooh { level: 1 }
        );
        assert_eq!(
            Crowd::new().on_fight_event(FightCue::Parried {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Cheer { level: 2 }
        );
        assert_eq!(
            Crowd::new().on_fight_event(FightCue::Dodged {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Gasp { level: 1 }
        );
        assert_eq!(
            Crowd::new().on_fight_event(FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Cheer { level: 3 }
        );
        assert_eq!(
            Crowd::new().on_fight_event(FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 24,
            }),
            Reaction::Quiet
        );
        assert_eq!(
            Crowd::new().on_fight_event(FightCue::StaminaOut { fighter: 0 }),
            Reaction::Quiet
        );
    }

    #[test]
    fn attack_started_is_quiet_and_holds_no_cooldown_or_bed() {
        // Press-tick tell (combat `AttackStarted { attacker }` under TABLES_V2):
        // the crowd stays on the bed. No audible reaction, no duck, and no
        // cooldown armed, so contact cues on the same tick still fire.
        assert_eq!(Crowd::new().on_attack_started(0), Reaction::Quiet);
        assert!(Crowd::new().on_attack_started(1).is_quiet());
        assert_eq!(Crowd::new().on_attack_started(0).level(), 0);
        let mut crowd = Crowd::new();
        assert_eq!(crowd.on_attack_started(0), Reaction::Quiet);
        assert_eq!(crowd.bed_gain(), 1.0, "press tell never dips the bed");
        assert_eq!(
            crowd.on_fight_event(hit()),
            Reaction::Ooh { level: 2 },
            "press tell arms no cooldown"
        );
        assert_eq!(crowd.on_attack_started(1), Reaction::Quiet);
        assert_eq!(
            crowd.bed_gain(),
            1.0 - HIT_DUCK_DEPTH,
            "only the contact cue dips the bed"
        );
    }

    #[test]
    fn cooldowns_stop_machine_gun_fire() {
        let mut crowd = Crowd::new();
        assert_eq!(crowd.on_fight_event(hit()), Reaction::Ooh { level: 2 });
        // Same-tick repeat and mid-cooldown repeats stay quiet.
        assert_eq!(crowd.on_fight_event(hit()), Reaction::Quiet);
        for _ in 0..OOH_COOLDOWN_TICKS - 1 {
            crowd.tick();
            assert_eq!(crowd.on_fight_event(hit()), Reaction::Quiet);
        }
        crowd.tick();
        assert_eq!(crowd.on_fight_event(hit()), Reaction::Ooh { level: 2 });

        // Cheer holds its longer cooldown; other kinds are unaffected.
        let mut crowd = Crowd::new();
        assert_eq!(
            crowd.on_fight_event(FightCue::Parried {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Cheer { level: 2 }
        );
        assert_eq!(
            crowd.on_fight_event(FightCue::Parried {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Quiet
        );
        // A gasp is a different kind: no cheer cooldown on it (only the
        // short global floor from the same tick applies, so step one tick).
        crowd.tick();
        crowd.tick();
        crowd.tick();
        crowd.tick();
        crowd.tick();
        crowd.tick();
        assert_eq!(
            crowd.on_fight_event(FightCue::Dodged {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Gasp { level: 1 }
        );
    }

    #[test]
    fn louder_reaction_escalates_inside_global_window() {
        let mut crowd = Crowd::new();
        // Hit (level 2) then knockdown (level 3) on the same tick: the
        // knockdown is louder, so it still fires.
        assert_eq!(crowd.on_fight_event(hit()), Reaction::Ooh { level: 2 });
        assert_eq!(
            crowd.on_fight_event(FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Cheer { level: 3 }
        );
        // Equal-or-quieter inside the window stays quiet.
        let mut crowd = Crowd::new();
        assert_eq!(crowd.on_fight_event(hit()), Reaction::Ooh { level: 2 });
        assert_eq!(
            crowd.on_fight_event(FightCue::Dodged {
                attacker: 1,
                victim: 0,
            }),
            Reaction::Quiet
        );
    }

    #[test]
    fn bed_dips_and_recovers_on_schedule() {
        let mut crowd = Crowd::new();
        assert_eq!(crowd.bed_gain(), 1.0);
        assert_eq!(crowd.on_fight_event(hit()), Reaction::Ooh { level: 2 });
        assert_eq!(crowd.bed_gain(), 1.0 - HIT_DUCK_DEPTH);
        for _ in 0..HIT_DUCK_TICKS / 2 {
            crowd.tick();
        }
        assert_eq!(crowd.bed_gain(), 1.0 - HIT_DUCK_DEPTH / 2.0);
        for _ in 0..HIT_DUCK_TICKS / 2 {
            crowd.tick();
        }
        assert_eq!(crowd.bed_gain(), 1.0);

        // Knockdown dips deeper and longer.
        let mut crowd = Crowd::new();
        assert_eq!(
            crowd.on_fight_event(FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            }),
            Reaction::Cheer { level: 3 }
        );
        assert_eq!(crowd.bed_gain(), 1.0 - KNOCKDOWN_DUCK_DEPTH);
        for _ in 0..KNOCKDOWN_DUCK_TICKS {
            assert!(crowd.bed_gain() < 1.0);
            crowd.tick();
        }
        assert_eq!(crowd.bed_gain(), 1.0);

        // Quiet cues never touch the bed.
        let mut crowd = Crowd::new();
        assert_eq!(
            crowd.on_fight_event(FightCue::StaminaOut { fighter: 1 }),
            Reaction::Quiet
        );
        assert_eq!(
            crowd.on_fight_event(FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 24,
            }),
            Reaction::Quiet
        );
        assert_eq!(crowd.bed_gain(), 1.0);
    }

    #[test]
    fn same_script_twice_same_reactions_and_bed() {
        let script = vec![
            hit(),
            FightCue::Parried {
                attacker: 1,
                victim: 0,
            },
            hit(),
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
            FightCue::StaminaOut { fighter: 1 },
        ];
        let run = || {
            let mut crowd = Crowd::new();
            let mut out = Vec::new();
            for cue in &script {
                out.push(crowd.on_fight_event(*cue));
                out.push(if crowd.bed_gain() < 1.0 {
                    Reaction::Ooh { level: 0 }
                } else {
                    Reaction::Quiet
                });
                crowd.tick();
            }
            (out, crowd.bed_gain(), crowd.tick_count())
        };
        assert_eq!(run(), run());
    }
}

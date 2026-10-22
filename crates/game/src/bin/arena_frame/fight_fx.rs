//! FEEL5: a hit throws sparks, blood and dust where it lands.
//!
//! The tick's fight cues name who hit whom, not where: the contact point is put
//! on the victim's chest, [`REACH_M`] toward the attacker, from the fighters'
//! feet positions. A flesh hit sprays blood away from the attacker, a block or a
//! parry throws sparks back toward them, a knockdown raises dust at the feet.
//! The presenter's fight pool draws them (render `fight_fx.rs`); it steps once
//! per sim tick, so a capture repeats. Presentation only: nothing here reads
//! back into the sim or the hash.

use super::ArenaApp;
use engine_audio::FightCue;
use engine_game::FighterView;
use engine_render::particles_gpu::EmitterKind;
use engine_render::present::Presenter;

/// Height of the contact above the feet, meters: a torso blow.
pub(super) const CHEST_M: f32 = 1.25;
/// Contact point distance from the victim toward the attacker, meters: the
/// near side of the torso, where the blade meets it.
pub(super) const REACH_M: f32 = 0.3;
/// Blood particles for a hit: a base plus one per [`BLOOD_PER`] permille of
/// damage, never over [`BLOOD_MAX`].
const BLOOD_BASE: u32 = 8;
const BLOOD_PER: u32 = 25;
const BLOOD_MAX: u32 = 48;
/// Sparks for a held block (plus the chip) and for a parry (the louder one).
const SPARKS_BLOCK: u32 = 14;
const SPARKS_PARRY: u32 = 30;
/// Dust where a downed body lands, and the blood of the finishing blow.
const DUST_KNOCKDOWN: u32 = 24;
const BLOOD_KNOCKDOWN: u32 = 12;

/// One burst a cue asks for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Burst {
    pub kind: EmitterKind,
    pub at_m: [f32; 3],
    pub dir: [f32; 3],
    pub count: u32,
}

/// Every burst the tick's cues ask for, in cue order, handed to `emit` one by
/// one (no per-tick allocation). A cue whose fighters are not in `views` (a
/// slot that left) throws nothing.
pub(super) fn bursts_for(cues: &[FightCue], views: &[FighterView], mut emit: impl FnMut(Burst)) {
    for cue in cues {
        match *cue {
            FightCue::Hit {
                attacker,
                victim,
                damage,
            } => {
                if let Some((at_m, toward)) = contact(views, attacker, victim) {
                    let count = (BLOOD_BASE + u32::from(damage) / BLOOD_PER).min(BLOOD_MAX);
                    let dir = lift(neg(toward), 0.5);
                    emit(Burst {
                        kind: EmitterKind::Blood,
                        at_m,
                        dir,
                        count,
                    });
                }
            }
            FightCue::Blocked {
                attacker,
                victim,
                chip,
            } => {
                if let Some((at_m, toward)) = contact(views, attacker, victim) {
                    let count = SPARKS_BLOCK + u32::from(chip) / BLOOD_PER;
                    emit(Burst {
                        kind: EmitterKind::Sparks,
                        at_m,
                        dir: lift(toward, 0.6),
                        count,
                    });
                }
            }
            FightCue::Parried { attacker, victim } => {
                if let Some((at_m, toward)) = contact(views, attacker, victim) {
                    let dir = lift(toward, 0.8);
                    emit(Burst {
                        kind: EmitterKind::Sparks,
                        at_m,
                        dir,
                        count: SPARKS_PARRY,
                    });
                }
            }
            FightCue::Knockdown { attacker, victim } => {
                if let Some((at_m, toward)) = contact(views, attacker, victim) {
                    let dir = lift(neg(toward), 0.5);
                    emit(Burst {
                        kind: EmitterKind::Blood,
                        at_m,
                        dir,
                        count: BLOOD_KNOCKDOWN,
                    });
                    let feet = [at_m[0], at_m[1] - CHEST_M + 0.05, at_m[2]];
                    let up = [0.0, 1.0, 0.0];
                    emit(Burst {
                        kind: EmitterKind::Dust,
                        at_m: feet,
                        dir: up,
                        count: DUST_KNOCKDOWN,
                    });
                }
            }
            FightCue::Dodged { .. } | FightCue::StaminaOut { .. } => {}
        }
    }
}

/// The contact point on the victim's chest toward the attacker, and the unit
/// ground direction from the victim to the attacker. Fighters on the same spot
/// face +z.
fn contact(views: &[FighterView], attacker: u8, victim: u8) -> Option<([f32; 3], [f32; 3])> {
    let from = views.iter().find(|v| v.slot == attacker)?.pos_m;
    let to = views.iter().find(|v| v.slot == victim)?.pos_m;
    let (dx, dz) = (from[0] - to[0], from[2] - to[2]);
    let len = (dx * dx + dz * dz).sqrt();
    let toward = if len.is_finite() && len > 1e-3 {
        [dx / len, 0.0, dz / len]
    } else {
        [0.0, 0.0, 1.0]
    };
    let at = [
        to[0] + toward[0] * REACH_M,
        to[1] + CHEST_M,
        to[2] + toward[2] * REACH_M,
    ];
    Some((at, toward))
}

fn neg(v: [f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

/// A ground direction tipped up by `up` (the emitter normalizes it).
fn lift(v: [f32; 3], up: f32) -> [f32; 3] {
    [v[0], v[1] + up, v[2]]
}

impl ArenaApp {
    /// Throw the tick's fight bursts into the presenter's fight pool and step it
    /// one sim tick. No presenter (headless smoke) throws nothing.
    pub(super) fn throw_fight_fx(&mut self) {
        let Some(presenter) = self.presenter.as_mut() else {
            return;
        };
        bursts_for(&self.tick_scratch.cues, self.app.fighter_views(), |b| {
            presenter.fight_burst(b.kind, b.at_m, b.dir, b.count);
        });
        presenter.step_fight_fx(engine_core::DT);
    }

    /// The exit receipt: `arena fight fx: spawned=<n> alive=<n>` — how many
    /// particles the fight threw over the run and how many still fly. A duel
    /// with hits and `spawned=0` means the cues never reached the picture.
    pub(crate) fn fight_fx_line(&self) -> String {
        let (alive, spawned) = self
            .presenter
            .as_ref()
            .map_or((0, 0), Presenter::fight_fx_counts);
        format!("arena fight fx: spawned={spawned} alive={alive}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn views() -> Vec<FighterView> {
        vec![
            FighterView {
                slot: 0,
                pos_m: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterView {
                slot: 1,
                pos_m: [2.0, 0.0, 0.0],
                yaw: 0.0,
            },
        ]
    }

    fn collect(cues: &[FightCue]) -> Vec<Burst> {
        let mut out = Vec::new();
        bursts_for(cues, &views(), |b| out.push(b));
        out
    }

    /// A cut bleeds on the victim's chest, away from the attacker, more for a
    /// harder blow; a block and a parry throw sparks back at the attacker.
    #[test]
    fn a_hit_bleeds_where_it_lands_and_a_block_sparks() {
        let light = collect(&[FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 50,
        }]);
        let heavy = collect(&[FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 900,
        }]);
        assert_eq!(light.len(), 1);
        let cut = light[0];
        assert_eq!(cut.kind, EmitterKind::Blood);
        assert_eq!(
            cut.at_m,
            [2.0 - REACH_M, CHEST_M, 0.0],
            "the near side of the victim's chest"
        );
        assert!(cut.dir[0] > 0.0, "blood sprays away from the attacker");
        assert!(heavy[0].count > cut.count, "a harder blow bleeds more");
        assert!(heavy[0].count <= BLOOD_MAX);
        let block = collect(&[FightCue::Blocked {
            attacker: 0,
            victim: 1,
            chip: 10,
        }]);
        assert_eq!(block[0].kind, EmitterKind::Sparks);
        assert!(block[0].dir[0] < 0.0, "sparks fly back at the attacker");
        let parry = collect(&[FightCue::Parried {
            attacker: 0,
            victim: 1,
        }]);
        assert!(
            parry[0].count > block[0].count,
            "a parry rings louder than a block"
        );
    }

    /// A knockdown bleeds and raises dust at the feet; a dodge, a stamina miss
    /// and a slot that left throw nothing.
    #[test]
    fn a_knockdown_raises_dust_and_misses_throw_nothing() {
        let down = collect(&[FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        }]);
        assert_eq!(down.len(), 2);
        assert_eq!(down[1].kind, EmitterKind::Dust);
        assert!(
            down[1].at_m[1] < 0.2,
            "the dust is at the feet, got {:?}",
            down[1].at_m
        );
        assert!(collect(&[FightCue::Dodged {
            attacker: 0,
            victim: 1
        }])
        .is_empty());
        assert!(collect(&[FightCue::StaminaOut { fighter: 0 }]).is_empty());
        assert!(collect(&[FightCue::Hit {
            attacker: 0,
            victim: 7,
            damage: 100
        }])
        .is_empty());
    }
}

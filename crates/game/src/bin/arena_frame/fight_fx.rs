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
/// Sparks for a parry, the loudest ring (a block's ring is
/// [`engine_game::shield_spark`]'s 6..=10 at the shield rim).
const SPARKS_PARRY: u32 = 30;
/// Dust where a downed body lands, and the blood of the finishing blow.
const DUST_KNOCKDOWN: u32 = 24;
const BLOOD_KNOCKDOWN: u32 = 12;
/// Blood splat on the floor under the victim: the near blade width for a cut,
/// a step wider for a heavy blow ([`SPLAT_HEAVY_PERMILLE`] damage or more) and
/// wider again where a body lands.
const SPLAT_RADIUS_HIT_M: f32 = 0.26;
const SPLAT_RADIUS_HEAVY_M: f32 = 0.40;
const SPLAT_HEAVY_PERMILLE: u16 = 400;
const SPLAT_RADIUS_DOWN_M: f32 = 0.5;

/// One blood splat a cue asks for: where it landed on the floor, how wide.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Splat {
    pub at_m: [f32; 3],
    pub radius_m: f32,
}

/// One burst a cue asks for, and the blood it leaves on the floor.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Burst {
    pub kind: EmitterKind,
    pub at_m: [f32; 3],
    pub dir: [f32; 3],
    pub count: u32,
    pub splat: Option<Splat>,
}

/// The contact of one cue: the chest point the particles fly from, the unit
/// ground direction from the victim to the attacker, and the victim's feet
/// where the blood lands.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Contact {
    chest_m: [f32; 3],
    toward: [f32; 3],
    feet_m: [f32; 3],
}

/// Every burst the tick's cues ask for, in cue order, handed to `emit` one by
/// one (no per-tick allocation). `tick` stamps a block's rim sparks with the
/// tick that resolved it. A cue whose fighters are not in `views` (a slot that
/// left) throws nothing.
pub(super) fn bursts_for(
    cues: &[FightCue],
    views: &[FighterView],
    tick: u32,
    mut emit: impl FnMut(Burst),
) {
    for cue in cues {
        match *cue {
            FightCue::Hit {
                attacker,
                victim,
                damage,
            } => {
                if let Some(at) = contact(views, attacker, victim) {
                    let count = (BLOOD_BASE + u32::from(damage) / BLOOD_PER).min(BLOOD_MAX);
                    let dir = lift(neg(at.toward), 0.5);
                    let radius_m = if damage >= SPLAT_HEAVY_PERMILLE {
                        SPLAT_RADIUS_HEAVY_M
                    } else {
                        SPLAT_RADIUS_HIT_M
                    };
                    emit(Burst {
                        kind: EmitterKind::Blood,
                        at_m: at.chest_m,
                        dir,
                        count,
                        splat: Some(Splat {
                            at_m: at.feet_m,
                            radius_m,
                        }),
                    });
                }
            }
            FightCue::Blocked { .. } => {
                // SW172: the ring leaves the shield rim facing the attacker,
                // stamped with this tick: not a chest hit.
                if let Some(spark) = engine_game::shield_spark::rim_spark_for(cue, tick, views) {
                    emit(Burst {
                        kind: EmitterKind::Sparks,
                        at_m: spark.at_m,
                        dir: spark.dir,
                        count: spark.count,
                        splat: None,
                    });
                }
            }
            FightCue::Parried { attacker, victim } => {
                if let Some(at) = contact(views, attacker, victim) {
                    let dir = lift(at.toward, 0.8);
                    emit(Burst {
                        kind: EmitterKind::Sparks,
                        at_m: at.chest_m,
                        dir,
                        count: SPARKS_PARRY,
                        splat: None,
                    });
                }
            }
            FightCue::Knockdown { attacker, victim } => {
                if let Some(at) = contact(views, attacker, victim) {
                    let dir = lift(neg(at.toward), 0.5);
                    emit(Burst {
                        kind: EmitterKind::Blood,
                        at_m: at.chest_m,
                        dir,
                        count: BLOOD_KNOCKDOWN,
                        splat: Some(Splat {
                            at_m: at.feet_m,
                            radius_m: SPLAT_RADIUS_DOWN_M,
                        }),
                    });
                    let feet = [at.chest_m[0], at.chest_m[1] - CHEST_M + 0.05, at.chest_m[2]];
                    let up = [0.0, 1.0, 0.0];
                    emit(Burst {
                        kind: EmitterKind::Dust,
                        at_m: feet,
                        dir: up,
                        count: DUST_KNOCKDOWN,
                        splat: None,
                    });
                }
            }
            FightCue::Dodged { .. } | FightCue::StaminaOut { .. } => {}
        }
    }
}

/// The contact point on the victim's chest toward the attacker, the unit
/// ground direction from the victim to the attacker, and the victim's feet.
/// Fighters on the same spot face +z.
fn contact(views: &[FighterView], attacker: u8, victim: u8) -> Option<Contact> {
    let from = views.iter().find(|v| v.slot == attacker)?.pos_m;
    let to = views.iter().find(|v| v.slot == victim)?.pos_m;
    let (dx, dz) = (from[0] - to[0], from[2] - to[2]);
    let len = (dx * dx + dz * dz).sqrt();
    let toward = if len.is_finite() && len > 1e-3 {
        [dx / len, 0.0, dz / len]
    } else {
        [0.0, 0.0, 1.0]
    };
    Some(Contact {
        chest_m: [
            to[0] + toward[0] * REACH_M,
            to[1] + CHEST_M,
            to[2] + toward[2] * REACH_M,
        ],
        toward,
        feet_m: to,
    })
}

fn neg(v: [f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

/// A ground direction tipped up by `up` (the emitter normalizes it).
fn lift(v: [f32; 3], up: f32) -> [f32; 3] {
    [v[0], v[1] + up, v[2]]
}

impl ArenaApp {
    /// Throw the tick's fight bursts into the presenter's fight pool and their
    /// ground splats into the decal pool, then step both one sim tick. No
    /// presenter (headless smoke) throws nothing.
    pub(super) fn throw_fight_fx(&mut self) {
        let Some(presenter) = self.presenter.as_mut() else {
            return;
        };
        bursts_for(
            &self.tick_scratch.cues,
            self.app.fighter_views(),
            self.tick_scratch.tick,
            |b| {
                presenter.fight_burst(b.kind, b.at_m, b.dir, b.count);
                if let Some(splat) = b.splat {
                    presenter.fight_splat(splat.at_m, splat.radius_m);
                }
            },
        );
        presenter.step_fight_fx(engine_core::DT);
    }

    /// The exit receipt: per cue kind how many particles the fight threw over
    /// the run, how many still fly, and the ground splat counts under the
    /// tier's cap. A duel with hits and `spawned=0` means the cues never
    /// reached the picture; `splats=0` with hits means no blood hit the floor.
    pub(crate) fn fight_fx_line(&self) -> String {
        let Some(presenter) = self.presenter.as_ref() else {
            return "arena fight fx: spawned=0 alive=0 blood=0 sparks=0 dust=0 \
                    flame=0 splats=0 splats_placed=0 splats_evicted=0 splat_cap=0"
                .to_owned();
        };
        let (alive, spawned) = presenter.fight_fx_counts();
        let by_kind = presenter.fight_fx_by_kind();
        let (splats, placed, evicted, cap) = presenter.fight_splat_counts();
        let blood = by_kind[EmitterKind::Blood.index()];
        let sparks = by_kind[EmitterKind::Sparks.index()];
        let dust = by_kind[EmitterKind::Dust.index()];
        let flame = by_kind[EmitterKind::Flame.index()];
        format!(
            "arena fight fx: spawned={spawned} alive={alive} blood={blood} \
             sparks={sparks} dust={dust} flame={flame} splats={splats} \
             splats_placed={placed} splats_evicted={evicted} splat_cap={cap}"
        )
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
        bursts_for(cues, &views(), 0, |b| out.push(b));
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

    /// Blood reaches the floor: a cut splats under the victim, a heavy blow
    /// wider, a knockdown widest, and only the cues that bleed.
    #[test]
    fn a_bleeding_cue_leaves_blood_under_the_victim() {
        let cut = collect(&[FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 50,
        }])[0];
        let splat = cut.splat.expect("a cut bleeds on the floor");
        assert_eq!(splat.at_m, [2.0, 0.0, 0.0], "at the victim's feet");
        assert!((splat.radius_m - SPLAT_RADIUS_HIT_M).abs() < 1e-6);
        let heavy = collect(&[FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: SPLAT_HEAVY_PERMILLE,
        }])[0];
        assert_eq!(
            heavy.splat.map(|s| s.radius_m),
            Some(SPLAT_RADIUS_HEAVY_M),
            "a heavy blow leaves a wider pool"
        );
        let down = collect(&[FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        }]);
        let blood = down
            .iter()
            .find(|b| b.kind == EmitterKind::Blood)
            .expect("a knockdown bleeds");
        assert_eq!(
            blood.splat.map(|s| s.radius_m),
            Some(SPLAT_RADIUS_DOWN_M),
            "a body leaves the widest pool"
        );
        assert!(
            down.iter()
                .find(|b| b.kind == EmitterKind::Dust)
                .is_some_and(|b| b.splat.is_none()),
            "dust leaves no blood"
        );
        for cue in [
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 10,
            },
            FightCue::Parried {
                attacker: 0,
                victim: 1,
            },
        ] {
            assert!(
                collect(&[cue]).iter().all(|b| b.splat.is_none()),
                "sparks leave no blood on the floor"
            );
        }
    }
}

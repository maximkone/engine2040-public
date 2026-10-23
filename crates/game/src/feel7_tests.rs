//! FEEL7 landing tests: the scripted jump in the default duel and the HUD
//! health mirror. Test-only.

use engine_arena::Checkpoint;
use engine_input::Buttons;

use crate::tests::{human, small_config};
use crate::{App, FallLanding, TickOutcome, HUMAN_SLOT, MAX_HP_PERMILLE};

#[test]
fn feel7_scripted_jump_lands_once_with_the_fall_row() {
    let mut app = App::new(small_config()).unwrap().into_duel().unwrap();
    let mut landing = None;
    for tick in 0..small_config().ticks {
        // Hold the jump through the rise, then let go: a tap cuts the
        // jump at the release tick and never reaches the jump's height.
        let buttons = if tick < 8 {
            Buttons(engine_input::fighter::JUMP)
        } else {
            Buttons::NONE
        };
        let out = app.tick(human(tick, 0, 0, buttons));
        if let Some(land) = out.lands.first().copied() {
            landing = Some((tick, land, out.hud.hp));
            break;
        }
    }
    let (tick, land, hp) = landing.expect("the scripted jump lands");
    assert_eq!(land.slot, HUMAN_SLOT);
    assert!(
        land.fall_mm > 200,
        "a jump carries height: {}",
        land.fall_mm
    );
    assert_eq!(land.damage, 0, "a jump is below the first fall row");
    assert_eq!(hp, u32::from(MAX_HP_PERMILLE));
    assert!(tick > 4, "the landing is not the press tick");
}

/// One landing tick carries the weight through the whole presentation: the
/// feet fold it into the pelvis, the camera dips, and the window audio plays
/// the footstep. The default duel's scripted jump supplies the tick; the same
/// tick's views go to the look and the same outcome to the audio.
#[cfg(feature = "app")]
#[test]
fn feel7_a_landing_tick_squashes_dips_and_plays_its_cue() {
    use std::sync::Arc;

    use engine_audio::feed::{feed, Command};
    use engine_audio::{cue_names, CueBank};

    use crate::fighter_look::FighterLook;
    use crate::window_audio_space::{AudioSpace, SpaceParts};

    let mut app = App::new(small_config()).unwrap().into_duel().unwrap();
    let mut look = FighterLook::feel7_look_two();
    let bank = Arc::new(CueBank::placeholder(8_000).expect("the shipped table"));
    let footstep = bank.id(cue_names::FOOTSTEP).expect("the shipped cue");
    let (mut tx, mut rx) = feed(16, bank);
    let mut space = AudioSpace::with_parts(SpaceParts::EVENTS_ONLY);
    let mut landed = None;
    for tick in 0..small_config().ticks {
        let buttons = if tick < 8 {
            Buttons(engine_input::fighter::JUMP)
        } else {
            Buttons::NONE
        };
        let out = app.tick(human(tick, 0, 0, buttons));
        let views = app.fighter_views();
        // The window's order: the feet see the fall, then the authority row
        // names the landing tick, then the tick's acts fold.
        look.observe_landings(tick, views);
        look.observe_landing_rows(tick, &out.lands);
        if !out.lands.is_empty() {
            space.feed(&mut tx, tick, &out, views);
            landed = Some((tick, out));
            break;
        }
    }
    let (tick, out) = landed.expect("the scripted jump lands");
    assert_eq!(out.lands.first().expect("a landing row").slot, HUMAN_SLOT);
    let (squash_tick, squash_m) = look
        .feel7_landing_squash(tick)
        .expect("the look saw the landing");
    assert_eq!(squash_tick, tick, "the squash starts on the landing tick");
    assert!(squash_m > 0.0, "the pelvis drops by {squash_m} m");
    let dip_m = look
        .feel7_landing_dip_m(tick)
        .expect("the camera holds the landing");
    assert!(dip_m > 0.0, "the camera dips by {dip_m} m");
    let mut plays = Vec::new();
    while let Some(command) = rx.pop() {
        if let Command::Play(play) = command {
            plays.push(play);
        }
    }
    assert!(
        plays
            .iter()
            .any(|play| play.tick == tick && play.cue == footstep),
        "the landing tick {tick} plays its footstep cue"
    );
}

#[test]
fn feel7_app_mirrors_the_fall_damage_in_its_hud_health() {
    let mut app = App::new(small_config()).unwrap();
    app.hp[0] = 1_000;
    app.tick_lands.push(FallLanding {
        slot: 0,
        fall_mm: 4_000,
        damage: 250,
    });
    let mut out = TickOutcome::default();
    app.observe_into(0, &[], &[], Checkpoint { tick: 0, hash: 0 }, &mut out);
    assert_eq!(out.lands.len(), 1, "the tick names the landing");
    assert_eq!(app.hp[0], 750, "the HUD health mirrors the fall table");
}

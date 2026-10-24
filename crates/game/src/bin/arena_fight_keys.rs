//! The window's movement frame in a fight (owner 2026-10-24: "controls inverted, left is
//! right"): with a living rival inside lock-on range the keys turn by the line to that rival,
//! not by the director's shot. Tests only; the code is `ArenaApp::camera_relative`.
#![cfg(test)]

use super::*;
use engine_game::HUMAN_SLOT;
use engine_input::map::Key;

fn standing(app: &ArenaApp, slot: u8) -> [f32; 3] {
    app.app.fighter_views()[usize::from(slot)].pos_m
}

fn held(app: &mut ArenaApp, code: Key, down: bool) {
    app.set_pressed(PhysicalButton::Key(code.0), down);
}

/// A seed-2040 bout with the host's pad out of the picture, walked up with W and the camera
/// behind the hero until the rival stands under 6 m away. The spawn is over 30 m across the
/// pit, so the bout runs 900 ticks.
fn closed_in() -> ArenaApp {
    let args = WinArgs::parse(&["arena", "--frames", "900", "--exit"].map(String::from))
        .expect("smoke args parse");
    let config = GameConfig {
        seed: 2040,
        ticks: 900,
        fighters: 2,
        half_extent_m: engine_game::windowed::ARENA_HALF_EXTENT_M,
    };
    let mut app = ArenaApp::new(config, args, None).expect("session builds");
    app.pad = WindowPad::absent();
    let human = app.human().expect("human slot");
    app.run_sim_ticks(1, human);
    held(&mut app, Key::W, true);
    while app.ticks_done < 700 {
        let (hero, rival) = (standing(&app, HUMAN_SLOT), standing(&app, 1));
        if (rival[0] - hero[0]).hypot(rival[2] - hero[2]) < 6.0 {
            break;
        }
        app.note_view_forward(hero, [rival[0], 1.0, rival[2]]);
        app.run_sim_ticks(1, human);
    }
    held(&mut app, Key::W, false);
    app
}

/// Hold `code` for 15 ticks with the camera looking back at the hero from beyond the rival,
/// where camera-relative keys flip both W and D. Returns the move along the line toward the
/// rival and along the hero's right of it (`[-z, x]`).
fn hold_facing_the_hero(code: Key) -> (f32, f32) {
    let mut app = closed_in();
    let (hero, rival) = (standing(&app, HUMAN_SLOT), standing(&app, 1));
    let line = [rival[0] - hero[0], rival[2] - hero[2]];
    let len = line[0].hypot(line[1]);
    assert!(
        len < LOCK_ON_RANGE_M,
        "the walk reached lock-on range: {len} m"
    );
    let dir = [line[0] / len, line[1] / len];
    let behind_rival = [rival[0] + dir[0] * 6.0, 1.8, rival[2] + dir[1] * 6.0];
    app.note_view_forward(behind_rival, [hero[0], 1.0, hero[2]]);
    held(&mut app, code, true);
    let human = app.human().expect("human slot");
    app.run_sim_ticks(15, human);
    let end = standing(&app, HUMAN_SLOT);
    let moved = [end[0] - hero[0], end[2] - hero[2]];
    (
        moved[0] * dir[0] + moved[1] * dir[1],
        moved[0] * -dir[1] + moved[1] * dir[0],
    )
}

#[test]
fn in_a_fight_the_keys_follow_the_rival_line_whatever_the_camera_shows() {
    let (ahead, _) = hold_facing_the_hero(Key::W);
    assert!(
        ahead > 0.2,
        "W closes on the rival whatever the shot: {ahead}"
    );
    let (_, right) = hold_facing_the_hero(Key::D);
    assert!(
        right > 0.2,
        "D strafes to the hero's right of the rival line: {right}"
    );
}
